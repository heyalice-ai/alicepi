import logging
import numpy as np
import threading
import time
from typing import Optional

from alicepi_proto import vad_pb2

# Guard against missing dependency during development/testing outside container
try:
    from faster_whisper import WhisperModel
except ImportError:
    WhisperModel = None

logger = logging.getLogger(__name__)

class AudioTranscriber:
    def __init__(self, model_size="tiny", device="cpu", compute_type="int8"):
        self.model_size = model_size
        self.device = device
        self.compute_type = compute_type
        self.model = None
        self._load_model()
        
        # Buffer for accumulating audio chunks until we have enough for a segment
        self.audio_buffer = np.array([], dtype=np.float32)
        self._buffer_lock = threading.Lock()
        
        # Confg
        self.sample_rate = 16000 # Whisper expects 16k
        self._warned_sample_rate = False
        
    def _load_model(self):
        if WhisperModel is None:
            logger.warning("faster-whisper not installed. Transcriber will be dummy.")
            return

        logger.info(f"Loading Whisper model: {self.model_size} on {self.device}...")
        try:
            self.model = WhisperModel(self.model_size, device=self.device, compute_type=self.compute_type)
            logger.info("Whisper model loaded successfully.")
        except Exception as e:
            logger.error(f"Failed to load Whisper model: {e}")
            self.model = None

    def process_raw_bytes(self, raw_bytes):
        """
        Convert raw PCM bytes (assuming 16-bit mono 16kHz) to float32 numpy array
        and append to buffer.
        """
        # Assume input is 16-bit integer PCM. 
        # OpenAI Whisper expects float32 in range [-1, 1], sample rate 16000.
        
        # Convert bytes to int16
        int16_data = np.frombuffer(raw_bytes, dtype=np.int16)
        
        # Convert to float32 and normalize
        float32_data = int16_data.astype(np.float32) / 32768.0
        
        with self._buffer_lock:
            self.audio_buffer = np.concatenate((self.audio_buffer, float32_data))

    def drain_audio(self, min_seconds: float = 0.0):
        min_samples = int(self.sample_rate * min_seconds)
        with self._buffer_lock:
            if self.audio_buffer.size == 0:
                return None
            if min_samples > 0 and self.audio_buffer.size < min_samples:
                return None
            audio = self.audio_buffer.copy()
            self.audio_buffer = np.array([], dtype=np.float32)
        return audio

    def process_vad_packet(self, packet: vad_pb2.VadPacket):
        """Convert audio payloads from the VAD proto into the internal buffer."""
        payload_type = packet.WhichOneof("payload")
        if payload_type != "audio":
            return

        audio = packet.audio

        if audio.channels != 1:
            logger.warning(f"Unsupported channel count ({audio.channels}); expected mono")
            return

        if audio.sample_rate != self.sample_rate and not self._warned_sample_rate:
            logger.warning(
                f"Incoming sample_rate {audio.sample_rate} does not match expected {self.sample_rate}"
            )
            self._warned_sample_rate = True

        self.process_raw_bytes(audio.data)

    def transcribe(self, audio=None, cancel_event: Optional[threading.Event] = None):
        """Transcribe the provided audio or drain from buffer if audio is None."""
        if cancel_event and cancel_event.is_set():
            return
        if self.model is None:
            return

        if audio is None:
            audio = self.drain_audio(min_seconds=1.0)

        if audio is None or audio.size == 0:
            logger.info("Not enough audio data to transcribe yet.")
            return

        # In a real VAD-based streaming setup, we'd wait for a VAD "speech end" signal 
        # or process sliding windows. 
        # Here we will just process the whole buffer and clear it, which is closer to "phrase-based" than "streaming"
        
        if cancel_event and cancel_event.is_set():
            return

        segments, info = self.model.transcribe(
            audio,
            beam_size=5,
            language="en",
            vad_filter=True,  # faster-whisper has built-in Silero VAD
        )

        text_output = []
        for segment in segments:
            if cancel_event and cancel_event.is_set():
                return
            text_output.append(segment.text)
        
        return " ".join(text_output).strip()

    def reset(self):
        with self._buffer_lock:
            self.audio_buffer = np.array([], dtype=np.float32)
        self._warned_sample_rate = False
