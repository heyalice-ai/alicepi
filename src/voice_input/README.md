# Voice Input Service

The Voice Input service captures raw audio from the microphone, performs Voice Activity Detection (VAD), and streams relevant audio to the speech recognition service.

## Features

- **Audio Capture**: Interfaces with hardware (PyAudio/ALSA) to read microphone data
- **Voice Activity Detection (VAD)**: Uses Silero VAD to detect speech vs silence
- **Hangover Mechanism**: Continues streaming for 500ms after speech ends to prevent choppy transcriptions
- **Mock Mode**: Can read audio from a file or network stream for testing

## Configuration

Configuration is managed in `src/config.py`:

- `STREAM_SAMPLE_RATE`: Output sample rate for VAD + speech-rec (default: 16000 Hz)
- `STREAM_CHANNELS`: Output channel count (default: 1, mono)
- `CHUNK_SIZE`: Size of audio chunks to process (default: 512 samples)
- `AUDIO_CARD`: ALSA input device name or index (e.g., `hw:2,0`)
- `SAMPLE_FORMAT`: Input sample format (`S16_LE`, `S24_LE`, `S32_LE`)
- `RATE`: Input device sample rate (e.g., 48000)
- `CHANNELS`: Input device channel count (e.g., 2)
- `VAD_THRESHOLD`: Probability threshold for speech detection (default: 0.5)
- `SILENCE_DURATION_MS`: Hangover duration - continues streaming for this many milliseconds after speech ends (default: 500ms)
- `SPEECH_REC_HOST`: Hostname for speech recognition (default: `speech-rec`)
- `SPEECH_REC_AUDIO_PORT`: Port for speech recognition audio input (default: 5002)

Input audio is converted to the stream format (mono, `STREAM_SAMPLE_RATE`, int16)
before VAD and streaming.

## How It Works

### VAD with Hangover

The service implements a stateful VAD system with a "hangover" period:

1. **Idle State**: Not streaming, waiting for speech
2. **Speech Detected**: Begin streaming audio chunks
3. **Silence After Speech**: Continue streaming for `SILENCE_DURATION_MS` (hangover period)
4. **Hangover Expired**: Stop streaming, return to idle

This hangover mechanism is crucial for speech recognition quality because:
- It prevents cutting off trailing sounds at the end of utterances
- It provides context to the speech recognizer for better accuracy
- It handles natural pauses within sentences without interrupting the audio stream

### Streaming

Audio is streamed via TCP socket to the Speech Recognition service. The service connects to `speech_rec` and sends protobuf-encoded audio/status packets when speech is detected (including the hangover period).

## Usage

### Running with Real Microphone

```bash
python -m src.main
```

### Running with Mock Audio File

```bash
python -m src.main --mock-file /path/to/audio.wav
```

Or set the environment variable:

```bash
MOCK_AUDIO_FILE=/path/to/audio.wav python -m src.main
```

## Docker

The service is containerized with audio device access:

```bash
docker build -t voice-input .
docker run --device /dev/snd voice-input
```

## Dependencies

- `pyaudio`: Audio capture
- `torch`: Required for Silero VAD
- `numpy`: Audio data processing
