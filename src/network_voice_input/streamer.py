import socket
import logging
import threading
import time
import config
from alicepi_proto.vad import encode_packet

logger = logging.getLogger(__name__)

class AudioStreamer:
    def __init__(self):
        self.socket = None
        self.running = False
        self.lock = threading.Lock()
        self.thread = None
        self._addr = (config.SPEECH_REC_HOST, config.SPEECH_REC_AUDIO_PORT)
        self._send_packets = 0
        self._send_bytes = 0
        self._drop_packets = 0
        self._last_stats_log = time.monotonic()
        self._last_drop_log = time.monotonic()
        
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
        now = time.monotonic()
        with self.lock:
            if not self.socket:
                self._drop_packets += 1
                if now - self._last_drop_log >= 5:
                    logger.warning(
                        "Dropping VadPacket: no Speech Rec connection (dropped=%d)",
                        self._drop_packets,
                    )
                    self._last_drop_log = now
                return

            try:
                self.socket.sendall(payload)
                self._send_packets += 1
                self._send_bytes += len(payload)
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

        if now - self._last_stats_log >= 5:
            logger.info(
                "SR stream stats: sent_packets=%d sent_bytes=%d dropped=%d",
                self._send_packets,
                self._send_bytes,
                self._drop_packets,
            )
            self._send_packets = 0
            self._send_bytes = 0
            self._last_stats_log = now

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
