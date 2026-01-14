# AlicePi Project Context

## Overview
AlicePi is a smart speaker runtime for Raspberry Pi. It is a single Rust binary that runs
the orchestrator, engine backends, and audio tasks inside one Tokio runtime.

## Architecture
- **Control Plane**: `orchestrator` owns the state machine (`Idle`, `Listening`, `Processing`, `Speaking`).
- **Execution Model**: Tokio tasks handle voice input, speech recognition, voice output, and GPIO events.
- **Engines**: Local/cloud backends live under `src/engine/` and share session logic.
- **RPC**: A TCP JSON command endpoint is exposed for client commands and status.
- **Supervision**: A watchdog monitors task heartbeats and restarts stalled workers.

## Directory Structure
- `src/`: Rust sources for CLI, orchestrator, protocol, and tasks.
- `src/engine/`: Local/cloud engine implementations and session handling.
- `src/tasks/`: Voice input, speech recognition, voice output, GPIO tasks.
- `assets/` and `models/`: Runtime assets and model data.
- `tasks/`: Markdown files tracking task progress.

## Coding Conventions
- **Rust**: Keep modules small and focused; prefer explicit types in protocol structs.
- **Async**: Use Tokio tasks for concurrency; avoid blocking calls in async paths.
- **GPIO**: Use feature flags (`gpio`, `lid_control`) for hardware-specific behavior.


## Key Runtime Behaviors

### 1. Orchestrator (`src/orchestrator.rs`)
- **Role**: Primary control unit and state machine.
- **Responsibilities**: 
  - Manages states (Idle, Listening, Processing, Speaking).
  - Handles prioritization (e.g., button events > SR text).
  - Manages session state and forwards events to engine backends.

### 2. Speech Recognition Task (`src/tasks/speech_rec.rs`)
- **Logic**:
  - Streams transcription results to the orchestrator.
  - Supports `STOP` vs `RESET` style control semantics.

### 3. Voice Input Task (`src/tasks/voice_input.rs`)
- **Role**: Audio capture & VAD gating.
- **Features**:
  - Optional mock/file injection for tests.
  - Emits PCM frames to internal channels.

### 4. Voice Output Task (`src/tasks/voice_output.rs`)
- **Role**: Audio playback (ALSA integration pending).

### 5. GPIO (`src/tasks/gpio.rs`)
- **Role**: GPIO event listener and debouncing.
- **Features**: Maps pin events to semantic commands (e.g., `RESET_REQUEST`).

### 6. Engine Backends (`src/engine/`)
- **Role**: Local/cloud inference implementations with shared session logic.
