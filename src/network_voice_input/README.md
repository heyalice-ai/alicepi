# Network Voice Input Service

A drop-in replacement for the standard `voice_input` service. Instead of capturing audio from a local microphone, it listens for raw PCM audio from the network (port 5558) and bridges it to the standard `VadPacket` interface by connecting to `speech_rec` (port 5002) used by the rest of the Hey Alice system.

## Features

- **Network Audio Sink (Port 5558)**: Receives raw PCM 16kHz mono 16-bit audio (e.g., from `audio_sender.py`).
- **Standard VAD (Silero)**: Perfroms voice activity detection on the incoming stream.
- **VadPacket Sink (Speech Rec, Port 5002)**: Streams protobuf-encoded audio and status packets to `speech_rec`.

## Configuration

- `NETWORK_INPUT_PORT`: Port for raw PCM input (default: 5558).
- `SPEECH_REC_HOST`: Hostname for speech recognition (default: `speech-rec`).
- `SPEECH_REC_AUDIO_PORT`: Port for speech recognition audio input (default: 5002).
- `VAD_THRESHOLD`: Probability threshold for speech detection (default: 0.5).

## Usage

### Local Development

```bash
uv run network-voice-input
```

### Docker

```bash
docker build -t network-voice-input .
docker run -p 5558:5558 network-voice-input
```
