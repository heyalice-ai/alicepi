import socket
import logging
import threading
import queue
import config

logger = logging.getLogger(__name__)

class NetworkReceiver:
    def __init__(self):
        self.server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server_socket.bind((config.HOST, config.NETWORK_INPUT_PORT))
        self.server_socket.listen(1)
        self.client_socket = None
        self.running = False
        self.audio_queue = queue.Queue(maxsize=100)
        
    def start(self):
        self.running = True
        logger.info(f"NetworkReceiver listening for raw PCM on {config.HOST}:{config.NETWORK_INPUT_PORT}")
        threading.Thread(target=self._accept_loop, daemon=True).start()

    def _accept_loop(self):
        while self.running:
            try:
                client, addr = self.server_socket.accept()
                logger.info(f"Accepted raw audio connection from {addr}")
                self._handle_client(client)
            except Exception as e:
                if self.running:
                    logger.error(f"Error accepting connection: {e}")

    def _handle_client(self, client_sock):
        with client_sock:
            while self.running:
                try:
                    # We expect chunks of raw PCM
                    # audio_sender.py sends CHUNK * 2 bytes
                    data = client_sock.recv(config.CHUNK_SIZE * 2)
                    if not data:
                        logger.info("Raw audio client disconnected")
                        break
                    
                    try:
                        self.audio_queue.put(data, block=False)
                    except queue.Full:
                        # Drop old data if queue is full to avoid latency build-up
                        try:
                            self.audio_queue.get_nowait()
                            self.audio_queue.put(data, block=False)
                        except:
                            pass
                except Exception as e:
                    logger.error(f"Error receiving raw audio: {e}")
                    break

    def read_chunk(self):
        """Returns a chunk of bytes or None if no data available."""
        try:
            return self.audio_queue.get(timeout=0.1)
        except queue.Empty:
            return None

    def stop(self):
        self.running = False
        self.server_socket.close()
        if self.client_socket:
            self.client_socket.close()
