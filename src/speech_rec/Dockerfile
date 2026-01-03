FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=cache,target=/root/.cache/uv \
    --mount=type=bind,source=pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=uv.lock,target=uv.lock \
    uv sync --frozen

ARG SR_WHISPER_MODEL="base.en"
ENV SR_WHISPER_MODEL=${SR_WHISPER_MODEL}

RUN uv run python - <<'PY'
import os

from faster_whisper import WhisperModel

models = [model.strip() for model in os.environ.get("SR_WHISPER_MODEL", "").split(",") if model.strip()]
if not models:
    raise SystemExit("SR_WHISPER_MODEL is empty; nothing to download")

for model in models:
    WhisperModel(model, device="cpu", compute_type="int8")
PY

# Create src directory
COPY src/ ./src/

# Environment variables
ENV PYTHONPATH=/app

CMD ["uv", "run", "python", "-u", "-m", "src.speech_rec.main"]
