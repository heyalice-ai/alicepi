import os

# Audio Configuration
SAMPLE_RATE = 16000
CHANNELS = 1
CHUNK_SIZE = 512

# VAD Configuration
VAD_THRESHOLD = 0.5
SILENCE_DURATION_MS = 500  # Hangover duration: continue streaming for this many ms after speech ends

# Networking Configuration
SPEECH_REC_HOST = os.environ.get("SPEECH_REC_HOST", "speech-rec")
SPEECH_REC_AUDIO_PORT = int(os.environ.get("SPEECH_REC_AUDIO_PORT", 5002))
