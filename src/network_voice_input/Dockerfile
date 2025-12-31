FROM ghcr.io/heyalice-ai/alicepi-base

# Install dependencies for Silero VAD and Audio
RUN apt-get update && apt-get install -y \
    libasound2-dev \
    libportaudio2 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --no-install-project


# Copy source code and proto (proto is in ../proto_defs)
# However, Docker build context is usually at service root.
# For local dev we might need to copy proto_defs specifically or build from alicepi root.
# Assuming build from alicepi root or that uv handles the path.
# In Hey Alice project, we usually build from the service directory.
# This means we need to handle the relative path for alicepi-proto.
# I'll update the Dockerfile to include the proto if it's copied in.

COPY . .

# Run the application
CMD ["uv", "run", "python", "main.py"]
