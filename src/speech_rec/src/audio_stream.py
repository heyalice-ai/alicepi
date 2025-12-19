import socket
import threading
import queue
import logging

try:
    from . import interfaces
except ImportError:
    import interfaces

logger = logging.getLogger(__name__)

class AudioStreamServer:
    def __init__(self, host='0.0.0.0', port=interfaces.SR_AUDIO_INPUT_PORT):
        self.host = host
        self.port = port
        self.server_socket = None
        self.client_socket = None
        self.running = False
        self.audio_queue = queue.Queue()
        self.thread = None

    def start(self):
        self.running = True
        self.server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server_socket.bind((self.host, self.port))
        self.server_socket.listen(1)
        self.thread = threading.Thread(target=self._accept_loop, daemon=True)
        self.thread.start()
        logger.info(f"AudioStreamServer listening on {self.host}:{self.port}")

    def stop(self):
        self.running = False
        if self.client_socket:
            try:
                self.client_socket.close()
            except:
                pass
        if self.server_socket:
            try:
                self.server_socket.close()
            except:
                pass
        
    def _accept_loop(self):
        while self.running:
            try:
                client, addr = self.server_socket.accept()
                logger.info(f"Accepted audio connection from {addr}")
                self.client_socket = client
                self._receive_loop(client)
            except OSError:
                if self.running:
                    logger.error("Socket error in accept loop", exc_info=True)
                break
            except Exception as e:
                logger.error(f"Unexpected error in accept loop: {e}", exc_info=True)

    def _receive_loop(self, client_sock):
        buffer_size = 4096 
        while self.running:
            try:
                data = client_sock.recv(buffer_size)
                if not data:
                    logger.info("Audio client disconnected")
                    break
                
                # In a real scenario, you'd likely want to handle framing or just raw stream 
                # For faster-whisper we usually feed numpy arrays. 
                # We will just put raw bytes in queue and let the consumer convert.
                self.audio_queue.put(data)
                
            except OSError:
                break
            except Exception as e:
                logger.error(f"Error receiving audio: {e}")
                break
        
        if self.client_socket == client_sock:
            self.client_socket = None

    def get_audio_chunk(self):
        """Non-blocking get from queue"""
        try:
            return self.audio_queue.get_nowait()
        except queue.Empty:
            return None
    
    def clear_queue(self):
        with self.audio_queue.mutex:
            self.audio_queue.queue.clear()
