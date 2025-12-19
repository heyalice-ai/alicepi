import logging
import numpy as np
import threading
import time

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
        
        # Confg
        self.sample_rate = 16000 # Whisper expects 16k
        
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
        
        self.audio_buffer = np.concatenate((self.audio_buffer, float32_data))

    def transcribe(self):
        """
        Attempt to transcribe the current buffer.
        Returns a generator yielding text segments.
        This is a simplified implementation. Real real-time streaming with VAD-triggering is more complex.
        For now, we will perform transcription on available buffer if it meets a minimum length.
        """
        if self.model is None:
            return

        # Simple threshold for demo: if buffer > 1 second
        if len(self.audio_buffer) < self.sample_rate * 1.0: 
            return

        # In a real VAD-based streaming setup, we'd wait for a VAD "speech end" signal 
        # or process sliding windows. 
        # Here we will just process the whole buffer and clear it, which is closer to "phrase-based" than "streaming"
        
        segments, info = self.model.transcribe(
            self.audio_buffer, 
            beam_size=5,
            language="en",
            vad_filter=True  # faster-whisper has built-in Silero VAD
        )

        text_output = []
        for segment in segments:
            text_output.append(segment.text)
        
        # Clear buffer after transcription (naive approach - potentially cutting off words)
        # Improved approach: keep last N seconds or wait for Silence.
        # However, for this task Step 1, this logic suffices to prove connectivity.
        self.audio_buffer = np.array([], dtype=np.float32)
        
        return " ".join(text_output).strip()

    def reset(self):
        self.audio_buffer = np.array([], dtype=np.float32)

