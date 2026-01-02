# AlicePi

AlicePi is the edge computing component of the HeyAlice smart speaker ecosystem, running on Raspberry Pi. It interacts with the cloud services (hosted on an Nvidia DGX Spark) to provide voice assistance.

## Architecture

The system is designed as a microservices architecture. A central service orchestrates the communication between various functional modules.

### Service Modules

*   **Orchestrator**: The primary control service. It manages the application state machine, handles prioritization logic (e.g., button interrupts vs. voice activation), and coordinates inter-service communication. It also manages session state with the remote inference engine.
*   **Voice Input**: A service responsible for capturing raw audio from the microphone. It implements Voice Activity Detection (VAD) to filter silence and streams PCM data to the Speech Recognition service. It can also ingest audio from files for testing.
*   **Voice Output**: A dedicated service for audio playback. It runs in an isolated container with direct hardware access (ALSA) to play audio buffers received from the Orchestrator.
*   **Speech Recognition (SR)**: A local inference service running OpenAI Whisper. It consumes the PCM stream from the Voice Input service and outputs transcribed text streams with delimiters.
*   **Buttons**: A hardware interface service. It monitors GPIO pins for physical button presses, debounces the signals, and transmits semantic events (e.g., `RESET_ACTION`) to the Orchestrator.
*   **Onboarding**: A network management service. It handles the initial connection to the network and discovery of the upstream Spark service.
*   **Updater**: A lifecycle management service. It monitors the Docker containers and performs updates by pulling new images from the registry.

## Development

The project uses Docker for containerization and microservices isolation.

### Prerequisites

*   Docker & Docker Compose
*   Raspberry Pi 4 or 5 (target hardware)

### Building and Running
1. Make sure you have Docker installed on your Raspberry Pi 5.
When using raspbian, you can install it via:
    ```bash
    sudo apt-get update
    sudo apt-get install docker.io docker-cli docker-compose
    ```
2. Clone the repository:
    ```bash
    git clone https://github.com/heyalice-ai/alicepi.git
    cd alicepi
    ```
3. Start the services using Docker Compose:
    ```bash
    docker compose up orchestrator voice-input voice-output speech-rec
    ```

4. To bring down the services, run:
    ```bash
    docker compose down
    ```

If you want to make any changes to the codebase when developing, you can rebuild the images with:
```bash
docker compose build
```

Or directly using the --build flag when bringing up the services:
```bash
docker compose up --build orchestrator
```

### Deployment

*   **Container Registry**: GitHub Container Registry (GHCR).
*   **CI/CD**: GitHub Actions are used to build and push container images.
