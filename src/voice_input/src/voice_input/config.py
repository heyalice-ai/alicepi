import os

# Audio Configuration
SAMPLE_RATE = 16000
CHANNELS = 1
CHUNK_SIZE = 512

# VAD Configuration
VAD_THRESHOLD = 0.5
SILENCE_DURATION_MS = 500  # Hangover duration: continue streaming for this many ms after speech ends

# Networking Configuration
# We bind to all interfaces to allow other containers to connect
HOST = "0.0.0.0"
PORT = int(os.environ.get("VOICE_INPUT_PORT", 6000))
