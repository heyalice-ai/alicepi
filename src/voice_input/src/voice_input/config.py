import os

# Audio Configuration (stream/processing)
SAMPLE_RATE = int(os.environ.get("STREAM_SAMPLE_RATE", 16000))
CHANNELS = int(os.environ.get("STREAM_CHANNELS", 1))
CHUNK_SIZE = int(os.environ.get("CHUNK_SIZE", 512))

# Audio Capture Configuration (device/hardware)
CAPTURE_DEVICE = os.environ.get("AUDIO_CARD", os.environ.get("CAPTURE_DEVICE"))
CAPTURE_FORMAT = os.environ.get("SAMPLE_FORMAT", os.environ.get("CAPTURE_FORMAT", "S16_LE")).upper()
CAPTURE_RATE = int(os.environ.get("RATE", os.environ.get("CAPTURE_RATE", SAMPLE_RATE)))
CAPTURE_CHANNELS = int(os.environ.get("CHANNELS", os.environ.get("CAPTURE_CHANNELS", CHANNELS)))

# VAD Configuration
VAD_THRESHOLD = 0.5
SILENCE_DURATION_MS = 500  # Hangover duration: continue streaming for this many ms after speech ends

# Networking Configuration
SPEECH_REC_HOST = os.environ.get("SPEECH_REC_HOST", "speech-rec")
SPEECH_REC_AUDIO_PORT = int(os.environ.get("SPEECH_REC_AUDIO_PORT", 5002))
