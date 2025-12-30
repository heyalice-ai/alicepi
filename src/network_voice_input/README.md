# Network Voice Input Service

A drop-in replacement for the standard `voice_input` service. Instead of capturing audio from a local microphone, it listens for raw PCM audio from the network (port 5558) and bridges it to the standard `VadPacket` interface (port 6000) used by the rest of the Hey Alice system.

## Features

- **Network Audio Sink (Port 5558)**: Receives raw PCM 1kHz Mono 16-bit audio (e.g., from `audio_sender.py`).
- **Standard VAD (Silero)**: Perfroms voice activity detection on the incoming stream.
- **VadPacket Source (Port 6000)**: Streams protobuf-encoded audio and status packets to clients like `speech_rec`.

## Configuration

- `NETWORK_INPUT_PORT`: Port for raw PCM input (default: 5558).
- `VOICE_INPUT_PORT`: Port for VadPacket output (default: 6000).
- `VAD_THRESHOLD`: Probability threshold for speech detection (default: 0.5).

## Usage

### Local Development

```bash
uv run network-voice-input
```

### Docker

```bash
docker build -t network-voice-input .
docker run -p 5558:5558 -p 6000:6000 network-voice-input
```
