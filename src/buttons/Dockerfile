FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --frozen --no-install-project

COPY src/ src/

# GPIO access often requires root or being in the 'gpio' group
# For simplicity in Docker, we run as root, butprivileged: true in docker-compose is usually needed
CMD ["uv", "run", "python", "-u", "src/main.py"]
