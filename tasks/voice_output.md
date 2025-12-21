# Task: Implement Voice Output Service

## Overview
The Voice Output service is a minimal, isolated service responsible for playing audio on the hardware speakers.

## Responsibilities
*   **Playback**: Receive audio chunks or file paths and play them via ALSA.
*   **Isolation**: Keep hardware access restricted to this container.

## Interfaces
*   **Inputs (Subscribes)**:
    *   `voice_output_control` (Socket): Audio data stream or commands.

## Implementation Steps
1.  **Scaffold**: Create `src/voice_output/`. Minimal Python or even a bash script with `aplay`/`mpv` might suffice, but Python `sounddevice` or `pyaudio` preferred for control. [x]
2.  **Dockerfile**: Alpine-based or slim Debian. Needs `alsa-utils` and device access. [x]
3.  **Core Logic**: Listen on socket -> buffer -> write to audio device. [x]
4.  **CI**: GitHub Action for build/push to GHCR. [x]
