import os
import time
import json
import logging
import zmq
from datetime import datetime

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("Buttons")

# Try to import gpiozero, fallback to mock if not available
try:
    from gpiozero import Button
    HAS_GPIO = True
except (ImportError, RuntimeError):
    logger.warning("GPIO not available. Running in MOCK mode.")
    HAS_GPIO = False

# Configuration
ZMQ_PORT = int(os.environ.get("BUTTONS_PORT", 5558))
# Mapping GPIO pins to events
# GPIO 17: RESET
# GPIO 27: VOLUME_UP
# GPIO 22: VOLUME_DOWN
PIN_MAPPING = {
    17: "RESET",
    27: "VOLUME_UP",
    22: "VOLUME_DOWN"
}

class ButtonsService:
    def __init__(self):
        self.running = False
        self.zmq_ctx = zmq.Context()
        self.pub_socket = self.zmq_ctx.socket(zmq.PUB)
        self.pub_socket.bind(f"tcp://0.0.0.0:{ZMQ_PORT}")
        logger.info(f"ZMQ PUB socket bound to port {ZMQ_PORT}")
        
        self.buttons = []
        if HAS_GPIO:
            self._setup_gpio()

    def _setup_gpio(self):
        for pin, event_name in PIN_MAPPING.items():
            try:
                btn = Button(pin, hold_time=2.0)
                btn.when_pressed = lambda p=pin, e=event_name: self._handle_event(e)
                btn.when_held = lambda p=pin, e=event_name: self._handle_event(f"LONG_{e}")
                self.buttons.append(btn)
                logger.info(f"Configured GPIO {pin} for event {event_name}")
            except Exception as e:
                logger.error(f"Failed to configure GPIO {pin}: {e}")

    def _handle_event(self, event_name):
        payload = {
            "event": event_name,
            "timestamp": datetime.utcnow().isoformat()
        }
        msg = json.dumps(payload)
        logger.info(f"Publishing event: {msg}")
        self.pub_socket.send_string(msg)

    def start(self):
        self.running = True
        logger.info("Buttons service started.")
        
        try:
            while self.running:
                # If mock mode, we could listen for terminal input or just sleep
                if not HAS_GPIO:
                    time.sleep(10)
                else:
                    time.sleep(1)
        except KeyboardInterrupt:
            self.stop()

    def stop(self):
        self.running = False
        logger.info("Buttons service stopping...")
        self.pub_socket.close()
        self.zmq_ctx.term()

if __name__ == "__main__":
    service = ButtonsService()
    service.start()
