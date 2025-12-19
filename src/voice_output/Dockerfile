FROM ghcr.io/astral-sh/uv:python3.11-bookworm-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    alsa-utils \
    libasound2-dev \
    libportaudio2 \
    build-essential \
    python3-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Enable bytecode compilation
ENV UV_COMPILE_BYTECODE=1

# Install dependencies using uv
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project

# Copy source code
COPY . .

# Run the application
CMD ["uv", "run", "python", "main.py"]

