import logging
import time
import sys
import os
import zmq
import threading
import json
import re

# Add src to path if needed
sys.path.append(os.path.join(os.path.dirname(__file__), '..'))

try:
    from src import config
    from src.state import State
    from src.llm import LLMClient
    from src.session import SessionManager
    from src.sr_client import SRClient
    from src.vibevoice_client import VibeVoiceClient
    from src.engines.local import LocalEngine
    from src.engines.cloud import CloudEngine
    from src.audio_processor import AudioProcessor
except ImportError:
    import config
    from state import State
    from llm import LLMClient
    from session import SessionManager
    from sr_client import SRClient
    from vibevoice_client import VibeVoiceClient
    from engines.local import LocalEngine
    from engines.cloud import CloudEngine
    from audio_processor import AudioProcessor

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("Orchestrator")

VOICE_OUTPUT_PATTERN = re.compile(
    r"\[VOICE OUTPUT\](.*?)\[/VOICE OUTPUT\]", re.IGNORECASE | re.DOTALL
)

class Orchestrator:
    def __init__(self):
        self.running = False
        self.state = State.IDLE
        self.zmq_ctx = None
        self.pub_socket: zmq.SyncSocket = None
        self.buttons_sub_socket: zmq.SyncSocket = None
        
        # Delegated Responsibilities
        self.session = SessionManager()
        self.audio_processor = AudioProcessor(
            target_rate=config.TARGET_SAMPLE_RATE,
            target_channels=config.TARGET_CHANNELS,
            target_dtype=config.TARGET_DTYPE
        )
        
        if config.ORCHESTRATOR_MODE == "cloud":
            logger.info("Initializing Cloud Engine")
            self.engine = CloudEngine()
        else:
            logger.info("Initializing Local Engine")
            self.engine = LocalEngine()

        self.sr = SRClient(
            on_text_callback=self._handle_sr_text,
            on_connect_callback=self._on_sr_connect
        )
        
    def start(self):
        self.running = True
        logger.info(f"Orchestrator service started. State: {self.state.name}")
        
        self._setup_zmq()
        self.sr.start()
        
        # Start Buttons sub thread
        threading.Thread(target=self._buttons_listener_loop, daemon=True).start()

        try:
            self._main_loop()
        except KeyboardInterrupt:
            self.stop()
            
    def stop(self):
        self.running = False
        logger.info("Orchestrator stopping...")
        
        # Log final session
        self.session.log_session()
        
        # Stop SR Client
        self.sr.stop()
        
        # Close ZMQ sockets
        if self.pub_socket:
            logger.info("Closing Voice Output PUB socket...")
            self.pub_socket.close(linger=0)
        if self.buttons_sub_socket:
            logger.info("Closing Buttons SUB socket...")
            self.buttons_sub_socket.close(linger=0)
            
        if self.zmq_ctx:
            logger.info("Terminating ZMQ context...")
            self.zmq_ctx.term()

    def _setup_zmq(self):
        self.zmq_ctx = zmq.Context()
        
        # Publisher for Voice Output
        self.pub_socket = self.zmq_ctx.socket(zmq.PUB)
        connect_addr = f"tcp://{config.VOICE_OUTPUT_HOST}:{config.VOICE_OUTPUT_PORT}"
        self.pub_socket.connect(connect_addr)
        logger.info(f"ZMQ audio/control PUB connected to {connect_addr}")

        # Subscriber for Buttons
        self.buttons_sub_socket = self.zmq_ctx.socket(zmq.SUB)
        btn_addr = f"tcp://{config.BUTTONS_HOST}:{config.BUTTONS_PORT}"
        self.buttons_sub_socket.connect(btn_addr)
        self.buttons_sub_socket.subscribe("") 
        logger.info(f"ZMQ buttons SUB connected to {btn_addr}")

    def _on_sr_connect(self):
        self.sr.send_command("START")
        self.state = State.LISTENING

    def _buttons_listener_loop(self):
        while self.running:
            try:
                if self.buttons_sub_socket.poll(100): 
                    msg = self.buttons_sub_socket.recv_string()
                    logger.info(f"Received Button Event: {msg}")
                    try:
                        payload = json.loads(msg)
                        event = payload.get("event")
                        if event:
                            self._handle_button_event(event)
                    except json.JSONDecodeError:
                        logger.error(f"Failed to decode button event: {msg}")
            except zmq.ZMQError as e:
                if self.running:
                    logger.error(f"ZMQ Error in buttons loop: {e}")
                time.sleep(1)

    def _handle_button_event(self, event):
        logger.info(f"Handling button event: {event}")
        if event == "RESET":
            logger.info("Resetting session...")
            self.session.log_session()
            self.session.clear()
            
            # Send stop command to voice output if speaking
            stop_msg = {
                "type": "control",
                "command": "stop"
            }
            self.pub_socket.send_multipart([
                config.ZMQ_TOPIC_CONTROL.encode('utf-8'),
                json.dumps(stop_msg).encode('utf-8')
            ])
            
            # Also reset SR (Speech Recognition) state if possible
            self.sr.send_command("RESET")
            self.state = State.LISTENING
        elif event == "LONG_RESET":
            logger.warning("Factory reset requested (MOCK)")
            # In a real system, this might trigger a system reboot or config wipe
        elif "VOLUME" in event:
            logger.info(f"Volume change requested: {event}")
            # Could send a control message to Voice Output to adjust ALSA volume

    def _handle_sr_text(self, line):
        try:
            payload = json.loads(line)
            text = payload.get("text", "")
            is_final = payload.get("is_final", False)
            
            if text:
                logger.info(f"Hearing: {text} (Final: {is_final})")
                
                if is_final:
                    self.session.check_timeout()
                    self.state = State.PROCESSING
                    self._process_text(text)
        except json.JSONDecodeError:
            logger.error(f"Failed to decode JSON from SR: {line}")

    def _extract_voice_output(self, response_text):
        matches = VOICE_OUTPUT_PATTERN.findall(response_text or "")
        segments = [match.strip() for match in matches if match.strip()]
        if segments:
            return " ".join(segments)
        return None

    def _publish_audio_chunk(self, pcm_bytes):
        if not self.pub_socket:
            logger.warning("Voice output PUB socket is not ready.")
            return
        try:
            self.pub_socket.send_multipart([
                config.ZMQ_TOPIC_AUDIO.encode('utf-8'),
                pcm_bytes
            ])
        except zmq.ZMQError as e:
            logger.error(f"Failed to publish audio chunk: {e}")

    def _on_audio_chunk(self, chunk, rate, channels, dtype):
        processed_chunk = self.audio_processor.process_chunk(chunk, rate, channels, dtype)
        self._publish_audio_chunk(processed_chunk)

    def _process_text(self, text):
        self.session.add_user_message(text)
        self.session.update_tts_end() 

        self.state = State.SPEAKING
        self.audio_processor.reset()

        response_text = self.engine.process(
            text, 
            self.session.history, 
            self._on_audio_chunk
        )
        
        if response_text:
            self.session.add_assistant_message(response_text)
            logger.info(f"Processing complete. Response: {response_text}")

            # Send a control message if we have text to display/log
            voice_text = self._extract_voice_output(response_text)
            if not voice_text:
                voice_text = response_text.strip()
            
            ctrl_msg = {
                "type": "speak",
                "text": voice_text
            }
            self.pub_socket.send_multipart([
                config.ZMQ_TOPIC_CONTROL.encode('utf-8'),
                json.dumps(ctrl_msg).encode('utf-8')
            ])
            
        self.session.update_tts_end()
        self.state = State.LISTENING

    def _main_loop(self):
        while self.running:
            time.sleep(1)

if __name__ == "__main__":
    service = Orchestrator()
    service.start()
