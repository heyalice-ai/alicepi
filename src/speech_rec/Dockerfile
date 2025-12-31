FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --frozen

# Create src directory
COPY src/ ./src/

# Environment variables
ENV PYTHONPATH=/app

CMD ["uv", "run", "python", "-u", "-m", "src.speech_rec.main"]
