import os
import time
import json
import logging
from . import config

logger = logging.getLogger("Orchestrator.Session")

class SessionManager:
    def __init__(self):
        self.history = []
        self.last_tts_end_time = 0

    def add_user_message(self, text):
        self.history.append({"role": "user", "content": text})

    def add_assistant_message(self, text):
        self.history.append({"role": "assistant", "content": text})

    def check_timeout(self):
        """Check if the session has timed out and clear history if needed."""
        now = time.time()
        if self.history and (now - self.last_tts_end_time > config.SESSION_TIMEOUT_SECONDS):
            logger.info("Session timed out. Logging and clearing history.")
            self.log_session()
            self.history = []
            return True
        return False

    def log_session(self):
        """Save the current session history to a file if enabled."""
        if not config.ENABLE_SESSION_LOGGING or not self.history:
            return
            
        try:
            os.makedirs(os.path.dirname(config.SESSION_LOG_PATH), exist_ok=True)
            session_data = {
                "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
                "history": self.history
            }
            with open(config.SESSION_LOG_PATH, "a") as f:
                f.write(json.dumps(session_data) + "\n")
            logger.info(f"Session logged to {config.SESSION_LOG_PATH}")
        except Exception as e:
            logger.error(f"Failed to log session: {e}")

    def update_tts_end(self):
        self.last_tts_end_time = time.time()
        
    def clear(self):
        self.history = []
