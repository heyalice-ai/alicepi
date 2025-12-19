# AlicePi Project Context

## Overview
AlicePi is a smart speaker project running on Raspberry Pi hardware. It is built as a set of loose-coupled microservices running in Docker containers, communicating via ZeroMQ.

## Architecture
- **Control Plane**: An `orchestrator` service manages state and policy.
- **Communication**: Services use ZMQ (PUB/SUB for streams, REQ/REP for RPC) to communicate.
- **Deployment**: Docker Compose (locally) and GitHub Actions -> GHCR for distribution.

## Directory Structure
- `src/`: Contains source code for all services.
- `tasks/`: Markdown files tracking task progress.
- `.github/workflows/`: CI/CD definitions.

## Coding Conventions
- **Service Naming**: Use `snake_case` for service directory names (e.g., `src/voice_output`, `src/speech_rec`). **Do not use hyphens (kebab-case).**
- **Docker**: Each service in `src/` should have its own `Dockerfile`.
- **Python**: Preferred language for services. Use `pyproject.toml` and `uv` for project management. **Do not use `requirements.txt`.**
- **Project Management (`uv`)**:
  - Run scripts with `uv run script.py`.
  - Add dependencies with `uv add package_name`.
  - Synchronize environments with `uv sync`.
  - Projects must have a `pyproject.toml` and `uv.lock`.


## Key Services & Requirements

### 1. Orchestrator (`src/orchestrator`)
- **Role**: Primary control unit and state machine.
- **Responsibilities**: 
  - Manages states (Idle, Listening, Processing, Speaking).
  - Handles prioritization (e.g., Button `RESET` > SR Text).
  - **Power Management**: Sends `SIGKILL` to heavy processes (LLM/SR) if starving/idle.
  - **Session Management**: Syncs context with Cloud (DGX Spark).

### 2. Speech Recognition (`src/speech_rec`)
- **Engine**: `faster-whisper` + **Hailo 8L SDK** for NPU acceleration (No CUDA).
- **Logic**: 
  - Real-time transcription from PCM stream.
  - **Control**: Handles `STOP` (emit final text) vs `RESET` (kill process immediately).
  - Emits special `STOP` tokens.

### 3. Voice Input (`src/voice_input`)
- **Role**: Audio capture & VAD.
- **Features**: 
  - `silero-vad` or similar for gating.
  - **Mock Mode**: Must support reading from file/network for testing.
  - Streams raw PCM via ZMQ.

### 4. Voice Output (`src/voice_output`)
- **Role**: Audio playback (ALSA).
- **Isolation**: Runs in a privileged container for minimal hardware access.

### 5. Buttons (`src/buttons`)
- **Role**: GPIO Event Listener.
- **Features**: Debouncing, translating pin events to semantic events (e.g., `RESET_REQUEST`).

### 6. Onboarding (`src/onboarding`)
- **Role**: Network & Discovery.
- **Features**: Auto-connect to Spark Hotspot, Internet connectivity checks.

### 7. Updater (`src/updater`)
- **Role**: Lifecycle Management.
- **Features**: Watches GHCR for new images, pulls updates, restarts containers.

