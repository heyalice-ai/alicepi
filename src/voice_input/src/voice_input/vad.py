import torch
import numpy as np
import logging
from . import config

logger = logging.getLogger(__name__)

class VADWrapper:
    def __init__(self):
        logger.info("Initializing Silero VAD...")
        self.model, utils = torch.hub.load(repo_or_dir='snakers4/silero-vad',
                                           model='silero_vad',
                                           force_reload=False,
                                           trust_repo=True)
        (self.get_speech_timestamps,
         self.save_audio,
         self.read_audio,
         self.VADIterator,
         self.collect_chunks) = utils
        
        self.model.eval()
        logger.info("Silero VAD loaded.")

    def process(self, audio_chunk_bytes):
        """
        Process a chunk of audio bytes (PCM 16-bit).
        Returns boolean is_speech.
        """
        # Convert bytes to numpy float32 array normalized to [-1, 1]
        # PyAudio gives int16 bytes
        audio_int16 = np.frombuffer(audio_chunk_bytes, dtype=np.int16)
        audio_float32 = audio_int16.astype(np.float32) / 32768.0
        
        # Determine if speech is present
        # Silero expects a tensor of shape (1, N) or just (N)
        wav_tensor = torch.from_numpy(audio_float32)
        
        # We can use the model directly to get a probability for the chunk
        # But Silero is better with windowing context.
        # Ideally we should use VADIterator if we want stateful processing.
        # Let's start with simple probability check for this 'Step 1' implementation.
        # However, checking context is safer.
        
        # model(x, sr) returns probability
        with torch.no_grad():
             speech_prob = self.model(wav_tensor, config.SAMPLE_RATE).item()
        
        return speech_prob > config.VAD_THRESHOLD
