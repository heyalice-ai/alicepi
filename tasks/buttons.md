# Task: Implement Buttons Service

## Overview
The Buttons service manages physical user interactions via GPIO.

## Responsibilities
*   **GPIO Monitoring**: Listen to physical pins.
*   **Debouncing**: Prevent false triggers.
*   **Mapping**: Convert raw pin events to semantic actions (e.g., Pin 5 High -> `RESET_REQUEST`).

## Interfaces
*   **Outputs**:
    *   `buttons_events` (Socket): JSON stream `{ "event": "RESET", "timestamp": ... }`.

## Implementation Steps
1.  **Scaffold**: Create `src/buttons/`.
2.  **Dependencies**: `gpiozero` or `RPi.GPIO`.
3.  **Dockerfile**: Needs privileged access or GPIO mapping.
4.  **Core Logic**: Async loop monitoring pins.
5.  **CI**: GitHub Action for build/push to GHCR.
