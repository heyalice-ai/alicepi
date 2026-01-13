import logging
import re
from typing import Callable, List, Dict
from src.llm import LLMClient
from src.vibevoice_client import VibeVoiceClient
from .base import BaseEngine

logger = logging.getLogger("Orchestrator.LocalEngine")

VOICE_OUTPUT_PATTERN = re.compile(
    r"\[VOICE OUTPUT\](.*?)\[/VOICE OUTPUT\]", re.IGNORECASE | re.DOTALL
)

class LocalEngine(BaseEngine):
    def __init__(self):
        self.llm = LLMClient()
        self.vibevoice = VibeVoiceClient()

    def process(self, text: str, history: List[Dict[str, str]], on_audio_chunk: Callable[[bytes, int, int, str], None]) -> str:
        response_text = self.llm.call(history)
        
        if response_text:
            voice_text = self._extract_voice_output(response_text)
            if not voice_text:
                logger.warning("LLM response missing [VOICE OUTPUT]; using raw response.")
                voice_text = response_text.strip()

            if voice_text:
                logger.info(f"Streaming voice output. Length: {len(voice_text)} characters.")
                
                # VibeVoice returns 22050Hz, 1ch, 16bit PCM by default
                def _vv_callback(chunk):
                    on_audio_chunk(chunk, 22050, 1, "int16")

                try:
                    self.vibevoice.stream(voice_text, _vv_callback)
                except Exception as e:
                    logger.error(f"VibeVoice stream error: {e}")
        
        return response_text

    def _extract_voice_output(self, response_text: str) -> str:
        matches = VOICE_OUTPUT_PATTERN.findall(response_text or "")
        segments = [match.strip() for match in matches if match.strip()]
        if segments:
            return " ".join(segments)
        return None
