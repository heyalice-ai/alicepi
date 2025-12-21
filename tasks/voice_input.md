# Task: Implement Voice Input Service

## Overview
The Voice Input service is responsible for capturing raw audio from the microphone, performing Voice Activity Detection (VAD), and streaming relevant audio to the speech recognition service.

## Responsibilities
*   **Audio Capture**: Interface with hardware (ALSA/PortAudio) to read microphone data.
*   **VAD**: Use a VAD library (e.g., Silero VAD) to detect speech vs silence.
*   **Mock Mode**: Ability to read audio from a file or network stream for testing.
*   **Streaming**: Output raw PCM data when speech is detected.

## Interfaces
*   **Outputs (Publishes)**:
    *   `mic_pcm_stream` (Socket): Raw PCM chunks containing speech.

## Implementation Steps
1.  **Scaffold**: Create `src/voice-input/` (Python/C++). [x]
2.  **Dependencies**: `pyaudio`, `silero-vad` (or comparable). [x]
3.  **Dockerfile**: Dockerfile with audio device access (`--device /dev/snd`). [x]
4.  **Core Logic**: Loop capturing audio -> VAD check -> Send if speech. [x]
5.  **Mocking**: Implement file reader override. [x]
6.  **CI**: GitHub Action for build/push to GHCR. [x]
