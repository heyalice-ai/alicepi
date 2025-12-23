FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=bind,source=src/speech_rec/pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=src/speech_rec/uv.lock,target=uv.lock \
    uv sync --frozen --no-install-project

# Create src directory
COPY src/speech_rec/src/ ./src/

# Environment variables
ENV PYTHONPATH=/app

CMD ["uv", "run", "python", "-u", "src/main.py"]
