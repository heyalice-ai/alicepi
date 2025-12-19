FROM ghcr.io/astral-sh/uv:python3.10-bookworm-slim

WORKDIR /app

# Install system dependencies
# ffmpeg is often required for audio processing libraries
RUN apt-get update && apt-get install -y \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

# Enable bytecode compilation
ENV UV_COMPILE_BYTECODE=1

# Install dependencies using uv
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    --mount=type=cache,target=/root/.cache/uv \
    uv sync --frozen --no-install-project

# Create src directory
COPY src/ ./src/

# Environment variables
ENV PYTHONPATH=/app

CMD ["uv", "run", "python", "-u", "src/main.py"]

