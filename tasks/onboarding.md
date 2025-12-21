# Task: Implement Onboarding Service

## Overview
The Onboarding service ensures the device has network connectivity and can reach the Orchestrator/Cloud.

## Responsibilities
*   **Network Check**: Verify internet/LAN access.
*   **Auto-Connect**: Attempt to connect to known Spark Hotspot.
*   **Discovery**: Find the Spark server IP on the network.

## Interfaces
*   **Outputs**:
    *   `onboarding_status` (Socket): Updates on connectivity.

## Implementation Steps
1.  **Scaffold**: Create `src/onboarding/`.
2.  **Dependencies**: System calls to `nmcli` or python `networkmanager` libs.
3.  **Dockerfile**: Needs dbus/network access.
4.  **Core Logic**: State loop checking connection -> retry strategies.
5.  **CI**: GitHub Action for build/push to GHCR.
