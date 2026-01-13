from abc import ABC, abstractmethod
from typing import Callable, List, Dict

class BaseEngine(ABC):
    @abstractmethod
    def process(self, text: str, history: List[Dict[str, str]], on_audio_chunk: Callable[[bytes, int, int, str], None]) -> str:
        """
        Process user text and conversation history.
        Call on_audio_chunk with PCM data and its format metadata.
        Returns the full text response from the assistant.
        """
        pass
