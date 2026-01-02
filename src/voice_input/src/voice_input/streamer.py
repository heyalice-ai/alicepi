import socket
import logging
import threading
import time
from . import config
from alicepi_proto.vad import encode_packet

logger = logging.getLogger(__name__)

class AudioStreamer:
    def __init__(self):
        self.socket = None
        self.running = False
        self.lock = threading.Lock()
        self.thread = None
        self._addr = (config.SPEECH_REC_HOST, config.SPEECH_REC_AUDIO_PORT)
        
    def start(self):
        self.running = True
        logger.info(f"Streamer connecting to {self._addr[0]}:{self._addr[1]}")
        self.thread = threading.Thread(target=self._connect_loop, daemon=True)
        self.thread.start()

    def _connect_loop(self):
        while self.running:
            if self.socket is None:
                try:
                    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    sock.settimeout(5)
                    sock.connect(self._addr)
                    sock.settimeout(None)
                    with self.lock:
                        self.socket = sock
                    logger.info(f"Connected to Speech Rec at {self._addr}")
                except Exception as e:
                    logger.debug(f"Speech Rec connection pending: {e}")
                    time.sleep(2)
                    continue
            time.sleep(1)

    def send_packet(self, packet):
        """Encode and send a single VadPacket."""
        payload = encode_packet(packet)
        with self.lock:
            if self.socket:
                try:
                    self.socket.sendall(payload)
                except BrokenPipeError:
                    logger.warning("Speech Rec disconnected")
                    try:
                        self.socket.close()
                    except Exception:
                        pass
                    self.socket = None
                except Exception as e:
                    logger.error(f"Error sending data: {e}")
                    try:
                        self.socket.close()
                    except Exception:
                        pass
                    self.socket = None

    def stop(self):
        self.running = False
        if self.thread:
            self.thread.join(timeout=1)
        with self.lock:
            if self.socket:
                try:
                    self.socket.close()
                except Exception:
                    pass
                self.socket = None
