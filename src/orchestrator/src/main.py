import logging
import time
import sys
import os
import zmq
import threading
import json

# Add src to path if needed
sys.path.append(os.path.join(os.path.dirname(__file__), '..'))

try:
    from src import config
    from src.state import State
    from src.llm import LLMClient
    from src.session import SessionManager
    from src.sr_client import SRClient
except ImportError:
    import config
    from state import State
    from llm import LLMClient
    from session import SessionManager
    from sr_client import SRClient

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("Orchestrator")

class Orchestrator:
    def __init__(self):
        self.running = False
        self.state = State.IDLE
        self.zmq_ctx = None
        self.pub_socket = None
        self.buttons_sub_socket = None
        
        # Delegated Responsibilities
        self.session = SessionManager()
        self.llm = LLMClient()
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
        bind_addr = f"tcp://0.0.0.0:{config.VOICE_OUTPUT_PORT}"
        self.pub_socket.bind(bind_addr)
        logger.info(f"ZMQ audio/control PUB bound to {bind_addr}")

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
            except zmq.ZMQError as e:
                if self.running:
                    logger.error(f"ZMQ Error in buttons loop: {e}")
                time.sleep(1)

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

    def _process_text(self, text):
        self.session.add_user_message(text)
        
        response_text = self.llm.call(self.session.history)
        
        self.state = State.SPEAKING
        
        if response_text:
            self.session.add_assistant_message(response_text)
            
            ctrl_msg = {
                "type": "speak",
                "text": response_text
            }
            logger.info(f"Processing complete. Response: '{response_text}'")
            
            self.pub_socket.send_multipart([
                config.ZMQ_TOPIC_CONTROL.encode('utf-8'),
                json.dumps(ctrl_msg).encode('utf-8')
            ])
            
            # Simulate speaking delay
            time.sleep(0.5)
            
        self.session.update_tts_end()
        self.state = State.LISTENING

    def _main_loop(self):
        while self.running:
            time.sleep(1)

if __name__ == "__main__":
    service = Orchestrator()
    service.start()
