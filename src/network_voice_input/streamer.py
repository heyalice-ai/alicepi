import socket
import logging
import threading
import config
from alicepi_proto.vad import encode_packet

logger = logging.getLogger(__name__)

class AudioStreamer:
    def __init__(self):
        self.server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server_socket.bind((config.HOST, config.VOICE_INPUT_PORT))
        self.server_socket.listen(1) 
        self.client_socket = None
        self.running = False
        self.lock = threading.Lock()
        
    def start(self):
        self.running = True
        logger.info(f"Streamer listening on {config.HOST}:{config.VOICE_INPUT_PORT}")
        threading.Thread(target=self._accept_loop, daemon=True).start()

    def _accept_loop(self):
        while self.running:
            try:
                client, addr = self.server_socket.accept()
                logger.info(f"Accepted connection from {addr}")
                with self.lock:
                    if self.client_socket:
                        try:
                            self.client_socket.close()
                        except:
                            pass
                    self.client_socket = client
            except Exception as e:
                if self.running:
                    logger.error(f"Error accepting connection: {e}")

    def send_packet(self, packet):
        """Encode and send a single VadPacket."""
        payload = encode_packet(packet)
        with self.lock:
            if self.client_socket:
                try:
                    self.client_socket.sendall(payload)
                except BrokenPipeError:
                    logger.warning("Client disconnected")
                    self.client_socket.close()
                    self.client_socket = None
                except Exception as e:
                    logger.error(f"Error sending data: {e}")
                    self.client_socket = None

    def stop(self):
        self.running = False
        if self.client_socket:
            self.client_socket.close()
        self.server_socket.close()
