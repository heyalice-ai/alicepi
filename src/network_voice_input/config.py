import os

# Audio Configuration
SAMPLE_RATE = 16000
CHANNELS = 1
CHUNK_SIZE = 512

# VAD Configuration
VAD_THRESHOLD = 0.5
SILENCE_DURATION_MS = 2000  # Hangover duration

# Networking Configuration
HOST = "0.0.0.0"
# Port where we receive raw PCM from audio_sender.py
NETWORK_INPUT_PORT = int(os.environ.get("NETWORK_INPUT_PORT", 5558))
# Speech Rec audio input (VadPacket stream destination)
SPEECH_REC_HOST = os.environ.get("SPEECH_REC_HOST", "speech-rec")
SPEECH_REC_AUDIO_PORT = int(os.environ.get("SPEECH_REC_AUDIO_PORT", 5002))
