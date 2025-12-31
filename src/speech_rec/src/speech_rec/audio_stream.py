import socket
import threading
import queue
import logging

try:
    from . import interfaces
except ImportError:
    import interfaces
from alicepi_proto.vad import VadPacketFramer
from alicepi_proto import vad_pb2

logger = logging.getLogger(__name__)

class AudioStreamServer:
    def __init__(self, host='0.0.0.0', port=interfaces.SR_AUDIO_INPUT_PORT):
        self.host = host
        self.port = port
        self.server_socket = None
        self.client_socket = None
        self.running = False
        self.packet_queue = queue.Queue()
        self.thread = None
        self.framer = VadPacketFramer()
        self.last_status = vad_pb2.VadPacket.Status.UNKNOWN

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
                self.framer = VadPacketFramer()
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
                
                packets = self.framer.decode(data)
                for packet in packets:
                    payload_type = packet.WhichOneof("payload")
                    if payload_type == "status":
                        self.last_status = packet.status
                        logger.debug(f"VAD status: {vad_pb2.VadPacket.Status.Name(packet.status)}")
                    if payload_type:
                        self.packet_queue.put(packet)
                
            except OSError:
                break
            except Exception as e:
                logger.error(f"Error receiving audio: {e}")
                break
        
        if self.client_socket == client_sock:
            self.client_socket = None

    def get_packet(self):
        """Non-blocking get of the next VadPacket."""
        try:
            return self.packet_queue.get_nowait()
        except queue.Empty:
            return None
    
    def clear_queue(self):
        with self.packet_queue.mutex:
            self.packet_queue.queue.clear()
