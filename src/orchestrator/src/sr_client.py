import socket
import threading
import time
import logging
from . import config

logger = logging.getLogger("Orchestrator.SRClient")

class SRClient:
    def __init__(self, on_text_callback, on_connect_callback):
        self.on_text_callback = on_text_callback
        self.on_connect_callback = on_connect_callback
        self.running = False
        self.control_socket = None
        self.text_socket = None

    def start(self):
        self.running = True
        threading.Thread(target=self._maintain_control_connection, daemon=True).start()
        threading.Thread(target=self._maintain_text_connection, daemon=True).start()

    def stop(self):
        self.running = False
        if self.control_socket:
            try: self.control_socket.close()
            except: pass
        if self.text_socket:
            try: self.text_socket.close()
            except: pass

    def send_command(self, cmd):
        if self.control_socket:
            try:
                self.control_socket.sendall(cmd.encode('utf-8'))
                logger.info(f"Sent SR Control command: {cmd}")
            except Exception as e:
                logger.error(f"Failed to send SR command: {e}")
                self.control_socket = None

    def _maintain_control_connection(self):
        addr = (config.SPEECH_REC_HOST, config.SPEECH_REC_CONTROL_PORT)
        while self.running:
            try:
                if self.control_socket is None:
                    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    sock.settimeout(5)
                    sock.connect(addr)
                    self.control_socket = sock
                    logger.info(f"Connected to SR Control at {addr}")
                    self.on_connect_callback()
                time.sleep(1)
            except (socket.error, OSError) as e:
                logger.debug(f"SR Control connection pending...")
                self.control_socket = None
                time.sleep(5)

    def _maintain_text_connection(self):
        addr = (config.SPEECH_REC_HOST, config.SPEECH_REC_TEXT_PORT)
        while self.running:
            try:
                if self.text_socket is None:
                    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    sock.connect(addr)
                    self.text_socket = sock
                    logger.info(f"Connected to SR Text Output at {addr}")
                    self._read_text_loop()
            except (socket.error, OSError) as e:
                self.text_socket = None
                time.sleep(5)

    def _read_text_loop(self):
        buffer = ""
        while self.running and self.text_socket:
            try:
                data = self.text_socket.recv(1024)
                if not data:
                    logger.warning("SR Text socket closed remotely.")
                    self.text_socket = None
                    break
                
                buffer += data.decode('utf-8')
                while "\n" in buffer:
                    line, buffer = buffer.split("\n", 1)
                    if line.strip():
                        self.on_text_callback(line)
            except Exception as e:
                logger.error(f"Error reading SR text: {e}")
                self.text_socket = None
                break
