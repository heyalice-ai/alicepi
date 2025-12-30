import os

# Audio Configuration
SAMPLE_RATE = 16000
CHANNELS = 1
CHUNK_SIZE = 512

# VAD Configuration
VAD_THRESHOLD = 0.5
SILENCE_DURATION_MS = 500  # Hangover duration

# Networking Configuration
HOST = "0.0.0.0"
# Port where we receive raw PCM from audio_sender.py
NETWORK_INPUT_PORT = int(os.environ.get("NETWORK_INPUT_PORT", 5558))
# Port where we stream VadPackets to speech_rec (drop-in for voice_input)
VOICE_INPUT_PORT = int(os.environ.get("VOICE_INPUT_PORT", 6000))
