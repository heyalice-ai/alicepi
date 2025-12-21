# Task: Implement Updater Service

## Overview
The Updater service is responsible for keeping the system up to date by pulling new Docker images.

## Responsibilities
*   **Self-Reflection**: Check running container versions.
*   **Registry Check**: Poll GHCR for new tags/hashes.
*   **Update**: `docker pull` and restart services.

## Interfaces
*   **Interaction**: Direct Docker Socket access (`/var/run/docker.sock`).

## Implementation Steps
1.  **Scaffold**: Create `src/updater/`.
2.  **Dependencies**: `docker` python client.
3.  **Dockerfile**: Volume mount docker socket.
4.  **Core Logic**: Cron-like loop or webhook listener (if applicable) to trigger updates.
5.  **CI**: GitHub Action for build/push to GHCR.
