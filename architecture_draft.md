# AlicePi Software Architecture Plan (Draft)

## Overview
AlicePi is a single-process Rust runtime for the smart speaker. The Raspberry Pi hosts a Tokio-based
binary that runs the orchestrator, audio tasks, and local/cloud engine backends.

## System Architecture

The system is organized as async tasks within one process, coordinated by the orchestrator state
machine.

### 1. Orchestrator
*   **Role**: Primary control unit.
*   **Responsibilities**:
    *   **State Management**: Controls `Idle`, `Listening`, `Processing`, `Speaking`.
    *   **Prioritization**: Button/lid events override audio/text flows.
    *   **Session Management**: Coordinates local/cloud engine backends.
    *   **Policy**: Applies timeouts and stop/reset semantics for speech flows.
*   **Interface**:
    *   TCP JSON command endpoint for client commands and status.
    *   Internal channels to tasks and engine backends.

### 2. Voice Input Task
*   **Role**: Audio capture and preprocessing.
*   **Responsibilities**:
    *   Capture raw audio from hardware.
    *   VAD gating (optional/placeholder).
    *   Mock file/network injection for testing.

### 3. Speech Recognition Task
*   **Role**: Local speech-to-text.
*   **Responsibilities**:
    *   Ingest PCM frames and emit transcription results.
    *   Emit stop tokens and respect reset semantics.

### 4. Voice Output Task
*   **Role**: Audio playback.
*   **Responsibilities**:
    *   Play audio buffers (ALSA integration pending).

### 5. GPIO Task
*   **Role**: Hardware input abstraction.
*   **Responsibilities**:
    *   Debounce GPIO input and emit semantic events.

### 6. Engine Backends
*   **Role**: Local/cloud inference implementations.
*   **Responsibilities**:
    *   Handle session context and inference requests.

## Infrastructure & Interaction
*   **Host**: Raspberry Pi runs a single Rust binary.
*   **Cloud (DGX Spark)**:
    *   Hosts LLM inference and shared memory services.
    *   Interacts via engine backends when enabled.

## Implementation Notes
*   `src/orchestrator.rs` is the control plane for runtime state.
*   `src/tasks/` hosts the async task implementations.
*   `src/engine/` contains local/cloud engine logic and session handling.
