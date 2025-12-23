FROM ghcr.io/heyalice-ai/alicepi-base

WORKDIR /app

# Install dependencies using uv
RUN --mount=type=bind,source=src/orchestrator/pyproject.toml,target=pyproject.toml \
    --mount=type=bind,source=src/orchestrator/uv.lock,target=uv.lock \
    --mount=type=bind,source=src/proto_defs,target=/proto_defs \
    uv sync --frozen --no-install-project

COPY src/orchestrator/src/ src/

CMD ["uv", "run", "python", "-u", "src/main.py"]
