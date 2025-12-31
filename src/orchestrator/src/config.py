import os

# Orchestrator
ORCHESTRATOR_QUEUE_SIZE = 10

# Voice Output (Downstream) - ZMQ PUB
VOICE_OUTPUT_HOST = os.environ.get("VOICE_OUTPUT_HOST", "voice-output")
VOICE_OUTPUT_PORT = 5557
ZMQ_TOPIC_AUDIO = "voice_output_audio"
ZMQ_TOPIC_CONTROL = "voice_output_control"

# Speech Recognition (Upstream) - Raw Socket
# Note: speech-rec internal ports are 5001 (Control), 5002 (Audio In), 5003 (Text Out)
SPEECH_REC_HOST = os.environ.get("SPEECH_REC_HOST", "speech-rec")
SPEECH_REC_CONTROL_PORT = 5001
SPEECH_REC_TEXT_PORT = 5003

# Buttons (Upstream) - ZMQ PUB (we SUB)
BUTTONS_HOST = os.environ.get("BUTTONS_HOST", "buttons")
BUTTONS_PORT = 5558

# LLM Configuration
LLM_API_URL = os.environ.get("LLM_API_URL", "http://spark7:11434/v1/chat/completions")
SYSTEM_PROMPT = os.environ.get("SYSTEM_PROMPT", "You are Alice, a helpful AI assistant for the AlicePi smart speaker. Keep your responses concise and friendly.")
SESSION_TIMEOUT_SECONDS = 5.0

# Logging Configuration
ENABLE_SESSION_LOGGING = os.environ.get("ENABLE_SESSION_LOGGING", "false").lower() == "true"
SESSION_LOG_PATH = os.environ.get("SESSION_LOG_PATH", "/app/logs/sessions.jsonl")
