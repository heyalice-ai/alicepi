import logging
import time
import json
import socket
import threading
import sys
import os

# Create src/speech_rec/src if running from root, or src if running from inside
# This path hack is for local dev vs docker
sys.path.append(os.path.join(os.path.dirname(__file__), '..'))

try:
    from . import interfaces
    from .audio_stream import AudioStreamServer
    from .transcriber import AudioTranscriber
except ImportError:
    import interfaces
    from audio_stream import AudioStreamServer
    from transcriber import AudioTranscriber
from alicepi_proto import vad_pb2

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("SpeechRecService")

def _select_whisper_model(model_spec: str) -> str:
    # Allow comma-separated model lists for build-time prefetching.
    models = [model.strip() for model in model_spec.split(",") if model.strip()]
    return models[0] if models else "tiny.en"

SR_WHISPER_MODEL = _select_whisper_model(os.environ.get("SR_WHISPER_MODEL", "tiny.en"))

class SpeechRecService:
    def __init__(self):
        self.running = False
        self.control_socket = None
        self.text_socket = None
        
        self.audio_server = AudioStreamServer(port=interfaces.SR_AUDIO_INPUT_PORT)
        self.transcriber = AudioTranscriber(model_size=SR_WHISPER_MODEL)
        self.transcription_thread = None
        self.transcription_cancel = threading.Event()
        
        self.control_sock_server = None
        self.text_sock_server = None
        
        # State
        self.is_listening = False
        self._last_vad_status = vad_pb2.VadPacket.Status.SILENCE
        self._pending_transcription = False

    def start(self):
        self.running = True
        
        # Start Audio Server
        self.audio_server.start()
        
        # Start Control Socket (Server)
        self._start_control_socket()
        
        # Start Text Output Socket (Server)
        self._start_text_socket()
        
        logger.info("Speech Recognition Service Started")
        
        try:
            self._main_loop()
        except KeyboardInterrupt:
            self.stop()

    def stop(self):
        logger.info("Stopping service...")
        self.running = False
        self._cancel_transcription()
        self.audio_server.stop()
        if self.control_sock_server:
            self.control_sock_server.close()
        if self.text_sock_server:
            self.text_sock_server.close()

    def _start_control_socket(self):
        # Creates a socket that listens for commands
        self.control_sock_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.control_sock_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.control_sock_server.bind(('0.0.0.0', interfaces.SR_CONTROL_PORT))
        self.control_sock_server.listen(1)
        threading.Thread(target=self._control_accept_loop, daemon=True).start()
        logger.info(f"Control socket listening on port {interfaces.SR_CONTROL_PORT}")

    def _start_text_socket(self):
        # Creates a socket where we *push* text
        self.text_sock_server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.text_sock_server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.text_sock_server.bind(('0.0.0.0', interfaces.SR_TEXT_OUTPUT_PORT))
        self.text_sock_server.listen(1)
        logger.info(f"Text output socket listening on port {interfaces.SR_TEXT_OUTPUT_PORT}")
        # We accept one client for text output (The Orchestrator)
        threading.Thread(target=self._text_accept_loop, daemon=True).start()

    def _control_accept_loop(self):
        while self.running:
            try:
                client, _ = self.control_sock_server.accept()
                threading.Thread(target=self._handle_control_client, args=(client,), daemon=True).start()
            except OSError:
                break

    def _handle_control_client(self, client):
        with client:
            while self.running:
                data = client.recv(1024)
                if not data: break
                command = data.decode('utf-8').strip()
                self._process_command(command)

    def _text_accept_loop(self):
        while self.running:
            try:
                client, addr = self.text_sock_server.accept()
                logger.info(f"Text output client connected: {addr}")
                self.text_socket = client
            except OSError:
                break

    def _process_command(self, command):
        logger.info(f"Received command: {command}")
        if command == interfaces.CMD_START:
            self.is_listening = True
            self._last_vad_status = vad_pb2.VadPacket.Status.SILENCE
            self._pending_transcription = False
            self.transcriber.reset()
            self.audio_server.clear_queue()
            logger.info("Listening started")
        elif command == interfaces.CMD_STOP:
            self.is_listening = False
            self._pending_transcription = True
            logger.info("Listening stopped")
        elif command == interfaces.CMD_RESET:
            self.is_listening = False
            self.transcriber.reset()
            self.audio_server.clear_queue()
            self._cancel_transcription()
            logger.info("Reset requested")

    def _emit_text(self, text, is_final=False):
        if not self.text_socket:
            return
        
        payload = {
            interfaces.KEY_TEXT: text,
            interfaces.KEY_IS_FINAL: is_final
        }
        try:
            msg = json.dumps(payload) + "\n"
            self.text_socket.sendall(msg.encode('utf-8'))
        except (BrokenPipeError, OSError):
            logger.error("Text socket broken")
            self.text_socket = None

    def _main_loop(self):
        while self.running:
            # 1. Ingest Audio/Status packets from the VAD stream
            while True:
                packet = self.audio_server.get_packet()
                if packet is None:
                    break

                payload_type = packet.WhichOneof("payload")
                if payload_type == "audio":
                    if self.is_listening:
                        self.transcriber.process_vad_packet(packet)
                elif payload_type == "status":
                    self._handle_vad_status(packet.status)
            
            # 2. Transcribe when an utterance is ready
            if self._pending_transcription:
                self._maybe_start_transcription()
            
            time.sleep(0.01)

    def _maybe_start_transcription(self):
        if not self._pending_transcription:
            return
        if self.transcription_thread and self.transcription_thread.is_alive():
            return

        audio = self.transcriber.drain_audio()
        self._pending_transcription = False
        if audio is None:
            return

        logger.info("Starting new transcription task")

        # Clear stale cancel flags from prior runs
        if self.transcription_cancel.is_set():
            self.transcription_cancel = threading.Event()

        def _worker():
            try:
                logger.info("Starting transcription worker")
                text = self.transcriber.transcribe(audio=audio, cancel_event=self.transcription_cancel)
                if text and not self.transcription_cancel.is_set():
                    logger.info(f"Transcribed: {text}")
                    self._emit_text(text, is_final=True)  # Treating all as final for this simple chunks version
            except Exception:
                logger.exception("Transcription worker failed")

        self.transcription_thread = threading.Thread(target=_worker, daemon=True)
        self.transcription_thread.start()

    def _handle_vad_status(self, status: int):
        logger.info(f"VAD status received: {vad_pb2.VadPacket.Status.Name(status)}")
        if not self.is_listening:
            self._last_vad_status = status
            return

        was_speaking = self._last_vad_status in (
            vad_pb2.VadPacket.Status.SPEECH_DETECTED,
            vad_pb2.VadPacket.Status.SPEECH_HANGOVER,
        )
        if was_speaking and status == vad_pb2.VadPacket.Status.SILENCE:
            logger.info("Speech ended; scheduling transcription")
            self._pending_transcription = True

        self._last_vad_status = status

    def _cancel_transcription(self):
        if self.transcription_thread and self.transcription_thread.is_alive():
            self.transcription_cancel.set()
            self.transcription_thread.join(timeout=0.5)
            if self.transcription_thread.is_alive():
                logger.warning("Transcription thread did not stop after cancel; it will be abandoned")
        self.transcription_thread = None

if __name__ == "__main__":
    service = SpeechRecService()
    service.start()
