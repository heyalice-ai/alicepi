import numpy as np
import audioop
import logging

logger = logging.getLogger("Orchestrator.AudioProcessor")

class AudioProcessor:
    def __init__(self, target_rate=48000, target_channels=2, target_dtype='int32'):
        self.target_rate = target_rate
        self.target_channels = target_channels
        self.target_dtype = np.dtype(target_dtype)
        self.rate_state = None

    def process_chunk(self, pcm_bytes: bytes, source_rate: int, source_channels: int, source_dtype: str) -> bytes:
        """
        Converts input PCM bytes to the target format.
        Assumes input is raw PCM.
        """
        if not pcm_bytes:
            return b""

        src_dtype = np.dtype(source_dtype)
        
        # 1. Convert DType to float32 first for easier processing if it's not already
        # Or just use numpy to do everything.
        
        audio = np.frombuffer(pcm_bytes, dtype=src_dtype)
        
        # Reshape to channels
        if source_channels > 1:
            audio = audio.reshape(-1, source_channels)
        else:
            audio = audio.reshape(-1, 1)

        # 2. Channel conversion (to target_channels)
        if source_channels != self.target_channels:
            if source_channels == 1 and self.target_channels == 2:
                audio = np.repeat(audio, 2, axis=1)
            elif source_channels == 2 and self.target_channels == 1:
                audio = audio.mean(axis=1).reshape(-1, 1)
            else:
                # Fallback / Error
                logger.warning(f"Unsupported channel conversion: {source_channels} -> {self.target_channels}")
        
        # 3. Resampling
        intermediate_bytes = audio.astype(src_dtype).tobytes()
        
        if source_rate != self.target_rate:
            intermediate_bytes, self.rate_state = audioop.ratecv(
                intermediate_bytes,
                src_dtype.itemsize,
                self.target_channels,
                source_rate,
                self.target_rate,
                self.rate_state
            )
        
        # 4. Final DType conversion (to target_dtype, e.g. int32)
        # Re-parse to target dtype if needed
        final_audio = np.frombuffer(intermediate_bytes, dtype=src_dtype)
        
        if src_dtype != self.target_dtype:
            # Scale if needed
            if src_dtype.kind == 'i' and self.target_dtype.kind == 'i':
                if src_dtype.itemsize == 2 and self.target_dtype.itemsize == 4:
                    # 16-bit to 32-bit
                    final_audio = (final_audio.astype(np.int32) << 16)
                elif src_dtype.itemsize == 4 and self.target_dtype.itemsize == 2:
                    # 32-bit to 16-bit
                    final_audio = (final_audio.astype(np.int32) >> 16).clip(-32768, 32767).astype(np.int16)
            
            final_audio = final_audio.astype(self.target_dtype)

        return final_audio.tobytes()

    def reset(self):
        self.rate_state = None
