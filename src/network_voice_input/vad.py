import torch
import numpy as np
import logging
import config

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
        if not audio_chunk_bytes:
            return False
            
        audio_int16 = np.frombuffer(audio_chunk_bytes, dtype=np.int16)
        audio_float32 = audio_int16.astype(np.float32) / 32768.0
        
        wav_tensor = torch.from_numpy(audio_float32)
        
        with torch.no_grad():
             speech_prob = self.model(wav_tensor, config.SAMPLE_RATE).item()
        
        return speech_prob > config.VAD_THRESHOLD
