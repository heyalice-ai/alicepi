# Task: Implement Speech Recognition Service

## Overview
The Speech Recognition (SR) service runs a local inference engine (Whisper) to transcribe audio into text in real-time.

## Responsibilities
*   **Inference**: Run OpenAI Whisper (likely `faster-whisper` for Pi optimization).
*   **Streaming**: Accept PCM stream, output partial and final text.
*   **Control**: Start/Stop listening based on external commands. Needs to accept both STOP and RESET commands. STOP will emit the final text, but RESET will not, it will kill the process as fast as it can before restarting.
*   **Tokenization**: Output special `STOP` tokens when a phrase ends.

## Interfaces
*   **Inputs**:
    *   `mic_pcm_stream` (Socket): Audio data.
    *   `sr_control` (Socket): Start/Stop.
*   **Outputs**:
    *   `sr_text` (Socket): JSON stream `{ "text": "...", "is_final": true/false }`.

## Implementation Steps
1.  **Scaffold**: Create `src/speech-rec/`. [x]
2.  **Dependencies**: `faster-whisper`, `torch` (CPU), Hailo 8L SDK for NPU acceleration (no CUDA). [x]
3.  **Dockerfile**: Heavy image, needs careful optimization for size and build time. [x]
4.  **Core Logic**: Audio buffer -> Whisper segment -> Text emit. [x]
5.  **CI**: GitHub Action for build/push to GHCR. [x]
