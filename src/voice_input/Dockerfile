FROM ghcr.io/alice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --frozen --no-install-project

# Copy source code
COPY src/ ./src/

# Set entrypoint
CMD ["uv", "run", "python", "-m", "src.main"]
