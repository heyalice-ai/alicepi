FROM ghcr.io/alice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
# We use --system because we want to leverage the pre-installed packages in the base image.
RUN --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --frozen --no-install-project --system

COPY src/ src/

# GPIO access often requires root or being in the 'gpio' group
# For simplicity in Docker, we run as root, butprivileged: true in docker-compose is usually needed
CMD ["uv", "run", "python", "-u", "src/main.py"]
