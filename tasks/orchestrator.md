# Task: Implement Orchestrator Service

## Overview
The Orchestrator is the primary control unit of the AlicePi system. It manages state, prioritization, and communication between all other services.

## Responsibilities
*   **State Machine**: Manage states (Idle, Listening, Processing, Speaking).
*   **Prioritization**: Handle interrupts (e.g., Button press > SR Text).
*   **Power Management**: Monitor resources, `SIGKILL` low-priority/starving processes.
*   **Session Management**: Maintain state with the Cloud (DGX Spark).
*   **Policy Enforcement**: E.g., timeout SR after 10s silence.

## Interfaces
*   **Inputs (Subscribes)**:
    *   `buttons_events` (Socket/ZMQ): Physical button presses.
    *   `sr_text` (Socket/ZMQ): Transcribed text stream.
    *   `onboarding_status` (Socket/ZMQ): Network status.
*   **Outputs (Publishes)**:
    *   `voice_output_control` (Socket/ZMQ): Audio data/commands to play.
    *   `sr_control` (Socket/ZMQ): Commands to Start/Stop listening.

## Implementation Steps
1.  **Scaffold**: Create `src/orchestrator/` with Python.
2.  **Dockerfile**: Create a Dockerfile for the service.
3.  **Core Logic**: Implement the async event loop and state machine.
4.  **Sockets**: Implement the ZMQ (or raw socket) listeners and publishers.
5.  **Tests**: Unit tests for state transitions.
6.  **CI**: GitHub Action for build/push to GHCR.
