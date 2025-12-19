FROM ghcr.io/astral-sh/uv:python3.11-bookworm-slim

# Install system dependencies (PortAudio for PyAudio)
RUN apt-get update && apt-get install -y \
    portaudio19-dev \
    gcc \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Enable bytecode compilation
ENV UV_COMPILE_BYTECODE=1

# Install dependencies using uv
# Use cache mount for faster rebuilds
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project

# Copy source code
COPY src/ ./src/

# Set entrypoint
CMD ["uv", "run", "python", "-m", "src.main"]

