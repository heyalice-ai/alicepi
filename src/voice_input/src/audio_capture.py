import pyaudio
import wave
import time
import logging
from . import config

logger = logging.getLogger(__name__)

class AudioCapture:
    def __init__(self, mock_file=None):
        self.mock_file = mock_file
        self.p = pyaudio.PyAudio()
        self.stream = None
        self.wf = None

    def start(self):
        if self.mock_file:
            logger.info(f"Starting AudioCapture in MOCK mode with file: {self.mock_file}")
            self.wf = wave.open(self.mock_file, 'rb')
            # Verify sample rate match (optional but good practice)
            if self.wf.getframerate() != config.SAMPLE_RATE:
                logger.warning(f"Mock file sample rate {self.wf.getframerate()} != {config.SAMPLE_RATE}")
        else:
            logger.info("Starting AudioCapture in LIVE mode")
            self.stream = self.p.open(format=pyaudio.paInt16,
                                      channels=config.CHANNELS,
                                      rate=config.SAMPLE_RATE,
                                      input=True,
                                      frames_per_buffer=config.CHUNK_SIZE)

    def read_chunk(self):
        """Returns a chunk of bytes or None if stream ended."""
        if self.mock_file:
            data = self.wf.readframes(config.CHUNK_SIZE)
            if len(data) == 0:
                # End of file, loop or stop? Let's stop/return None for now or loop.
                # Ideally for testing let's loop to simulate continuous stream or just return None.
                # Returning None might kill the server process in main loop,
                # let's seek to start to loop forever for stress testing?
                # For now, let's just loop it.
                logger.debug("Mock file EOF, rewinding.")
                self.wf.rewind()
                data = self.wf.readframes(config.CHUNK_SIZE)
            
            # Simulate real-time delay
            # Chunk duration = chunk_size / sample_rate
            time.sleep(config.CHUNK_SIZE / config.SAMPLE_RATE)
            return data
        else:
            try:
                # exception_on_overflow=False prevents crashes on heavy load
                return self.stream.read(config.CHUNK_SIZE, exception_on_overflow=False)
            except IOError as e:
                logger.error(f"Error recording: {e}")
                return b'\x00' * (config.CHUNK_SIZE * 2) # Return silence on error

    def stop(self):
        if self.stream:
            self.stream.stop_stream()
            self.stream.close()
        if self.wf:
            self.wf.close()
        self.p.terminate()
