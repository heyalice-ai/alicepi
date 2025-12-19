# AlicePi Software Architecture Plan (Draft)

## Overview
This document outlines the software architecture for the **AlicePi** smart speaker. The system is distributed between a Raspberry Pi (Edge) and an Nvidia DGX Spark ("Cloud"). The initial deployment uses Docker containers on the Pi.

## System Architecture

The system is composed of several independent services coordinated by a central Orchestrator. Communication is primarily handled via sockets and defined interfaces.

### 1. Orchestrator Service
*   **Role**: Primary control unit.
*   **Responsibilities**:
    *   **State Management**: Controls the flow of the application (Listening, Processing, Speaking, Idle).
    *   **Prioritization**: Handles priority interrupts (e.g., "Reset" button press overrides Speech Recognition).
    *   **Power Management**: Monitors resource usage. Sends `SIGKILL` to LLM/heavy processes if starvation occurs.
    *   **Session Management**: Manages session state with the Cloud (DGX Spark), including context and user memory.
    *   **Logic**: specifically, implementing policies like "Timeout SR after 10s of silence".
    *   **Storage**: Optionally writes session logs to local SD card.
*   **Interface**:
    *   Subscribes to: Buttons, SR Text, Onboarding Status.
    *   Publishes to: Voice Output, SR Control (Start/Stop).

### 2. Onboarding Service
*   **Role**: Connectivity manager.
*   **Responsibilities**:
    *   Manage network connections.
    *   Auto-discovery of Spark Hotspot.
    *   Future: User WiFi setup.

### 3. Voice Input Service
*   **Role**: Audio capture and preprocessing.
*   **Responsibilities**:
    *   Capture raw audio from hardware (Microphone).
    *   **VAD (Voice Activity Detection)**: Only transmit frames containing speech.
    *   **Mock Capability**: Can accept input from file or network for automated testing.
*   **Output**: Raw PCM stream via socket.

### 4. Buttons Service
*   **Role**: Hardware input abstraction.
*   **Responsibilities**:
    *   Listen to GPIO pins for physical button presses.
    *   Debounce and interpret signals.
    *   Translate physical events into semantic events (e.g., `ACTION_RESET`).
*   **Output**: Events to Orchestrator.

### 5. Speech Recognition (SR) Service
*   **Role**: Local speech-to-text.
*   **Engine**: OpenAI Whisper (Real-time).
*   **Responsibilities**:
    *   Ingest PCM stream from a socket.
    *   Perform real-time transcription.
    *   Output text + special `STOP` tokens when phrases end.
*   **Input**: PCM Socket.
*   **Output**: Text stream (Socket).

### 6. Voice Output Service
*   **Role**: Audio playback.
*   **Responsibilities**:
    *   Isolated environment for speaker hardware access.
    *   Play audio buffers received from Orchestrator.
*   **Container**: Small, privileged Docker container for hardware access.

### 7. Updater Service
*   **Role**: Lifecycle management.
*   **Responsibilities**:
    *   Self-reflection and health checks.
    *   Upgrade docker images (Watchtower-style).
    *   Future: OTA updates for Yocto.

## Infrastructure & interaction
*   **Host**: Raspberry Pi.
*   **Cloud (DGX Spark)**:
    *   Hosts the LLM inference engine.
    *   Hosts MCP (Model Context Protocol) servers.
    *   Maintains User Memory/Database.
*   **Deployment (Current)**:
    *   **Registry**: GitHub Container Registry (GHCR).
    *   **CI/CD**: GitHub Actions for building and pushing images.
    *   **Updates**: The Spark or Pi will pull from GHCR for now.

## Implementation Roadmap (Autonomous Agent Tasks)

Each of the following can be implemented as a separate module/repo structure:

1.  **`src/orchestrator/`**: Python/Rust service. Implements state machine and socket servers.
2.  **`src/voice-input/`**: Python/C++ service. PortAudio/PyAudio + Silero VAD.
3.  **`src/speech-rec/`**: Python service. Faster-Whisper implementation or similar.
4.  **`src/voice-output/`**: Minimal Alpine/Python container with ALSA support.
5.  **`src/buttons/`**: GPIO Zero (Python) or similar.
6.  **`src/onboarding/`**: NetworkManager wrapper or similar.
7.  **`src/updater/`**: Docker socket interacting service.

## Next Steps for Agents
1.  Define the exact specific socket protocols (ports, message formats).
2.  Scaffold the directory structure.
3.  Create Docker compose file for local orchestration.
4.  Set up GitHub Actions workflows for container builds.
