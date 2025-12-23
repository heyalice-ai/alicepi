FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=bind,source=src/voice_input/pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=src/voice_input/uv.lock,target=uv.lock \
    --mount=type=bind,source=src/proto_defs,target=/proto_defs \
    uv sync --frozen --no-install-project

# Copy source code
COPY src/voice_input/src/ ./src/

# Set entrypoint
CMD ["uv", "run", "python", "-m", "src.main"]
