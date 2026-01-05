import asyncio
import logging
from typing import Callable
from urllib.parse import urlencode, urlsplit, urlunsplit, parse_qsl, quote

import websockets
from websockets.exceptions import ConnectionClosedError, ConnectionClosedOK

from . import config

logger = logging.getLogger("Orchestrator.VibeVoice")


class VibeVoiceClient:
    def __init__(
        self,
        ws_url=None,
        cfg_scale=None,
        inference_steps=None,
        voice=None,
        connect_timeout=None,
        ping_interval=None,
        ping_timeout=None,
    ):
        self.ws_url = ws_url or config.VIBEVOICE_WS_URL
        self.cfg_scale = config.VIBEVOICE_CFG_SCALE if cfg_scale is None else cfg_scale
        self.inference_steps = (
            config.VIBEVOICE_INFERENCE_STEPS if inference_steps is None else inference_steps
        )
        self.voice = config.VIBEVOICE_VOICE if voice is None else voice
        self.connect_timeout = (
            config.VIBEVOICE_CONNECT_TIMEOUT if connect_timeout is None else connect_timeout
        )
        self.ping_interval = config.VIBEVOICE_PING_INTERVAL if ping_interval is None else ping_interval
        self.ping_timeout = config.VIBEVOICE_PING_TIMEOUT if ping_timeout is None else ping_timeout

    def stream(self, text: str, on_audio_chunk: Callable[[bytes], None]) -> None:
        if not text or not text.strip():
            logger.warning("Skipping VibeVoice request because text is empty.")
            return
        asyncio.run(self._stream_async(text, on_audio_chunk))

    def _build_url(self, text: str) -> str:
        params = {"text": text}
        if self.cfg_scale is not None:
            params["cfg"] = str(self.cfg_scale)
        if self.inference_steps is not None:
            params["steps"] = str(self.inference_steps)
        if self.voice:
            params["voice"] = self.voice

        parts = urlsplit(self.ws_url)
        query_params = dict(parse_qsl(parts.query))
        query_params.update(params)
        query = urlencode(query_params, quote_via=quote)
        return urlunsplit((parts.scheme, parts.netloc, parts.path, query, parts.fragment))

    async def _stream_async(self, text: str, on_audio_chunk: Callable[[bytes], None]) -> None:
        url = self._build_url(text)
        logger.info("Connecting to VibeVoice at %s (text length: %d)", self.ws_url, len(text))
        try:
            async with websockets.connect(
                url,
                open_timeout=self.connect_timeout,
                ping_interval=self.ping_interval,
                ping_timeout=self.ping_timeout,
                max_size=None,
            ) as ws:
                while True:
                    message = await ws.recv()
                    if isinstance(message, bytes):
                        on_audio_chunk(message)
                    else:
                        logger.debug("VibeVoice log: %s", message)
        except ConnectionClosedOK:
            logger.info("VibeVoice stream completed.")
        except ConnectionClosedError as exc:
            logger.error("VibeVoice connection closed with error: %s", exc)
        except Exception as exc:
            logger.error("VibeVoice stream failed: %s", exc)
