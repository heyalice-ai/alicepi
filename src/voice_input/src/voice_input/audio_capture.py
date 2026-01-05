import audioop
import pyaudio
import wave
import time
import logging
from . import config

logger = logging.getLogger(__name__)

FORMAT_MAP = {
    "S16_LE": (pyaudio.paInt16, 2),
    "S24_LE": (pyaudio.paInt24, 3),
    "S32_LE": (pyaudio.paInt32, 4),
}

TARGET_SAMPLE_WIDTH = 2  # int16 for VAD + speech-rec

class AudioCapture:
    def __init__(self, mock_file=None):
        self.mock_file = mock_file
        self.p = pyaudio.PyAudio()
        self.stream = None
        self.wf = None
        self.capture_rate = None
        self.capture_channels = None
        self.capture_width = None
        self.capture_chunk_size = None
        self._rate_state = None

    def _resolve_input_device_index(self, device_name):
        if not device_name:
            return None
        try:
            return int(device_name)
        except ValueError:
            pass

        for index in range(self.p.get_device_count()):
            info = self.p.get_device_info_by_index(index)
            if info.get("maxInputChannels", 0) <= 0:
                continue
            name = info.get("name", "")
            if device_name in name:
                logger.info(f"Using input device '{name}' (index {index})")
                return index

        logger.warning(f"Requested input device '{device_name}' not found; using default.")
        return None

    def _resolve_format(self, sample_format):
        fmt = (sample_format or "").upper()
        if fmt in FORMAT_MAP:
            return FORMAT_MAP[fmt]
        logger.warning(f"Unknown SAMPLE_FORMAT '{sample_format}', defaulting to S16_LE.")
        return FORMAT_MAP["S16_LE"]

    def _calc_capture_chunk_size(self, capture_rate):
        if capture_rate <= 0:
            return config.CHUNK_SIZE
        return max(1, int(config.CHUNK_SIZE * (capture_rate / config.SAMPLE_RATE)))

    def _convert_to_stream(self, data):
        if not data:
            return data

        width = self.capture_width
        channels = self.capture_channels
        rate = self.capture_rate

        if channels != config.CHANNELS:
            if channels == 2 and config.CHANNELS == 1:
                data = audioop.tomono(data, width, 0.5, 0.5)
                channels = 1
            else:
                logger.warning(
                    f"Unsupported channel conversion {channels} -> {config.CHANNELS}; "
                    "streaming raw audio."
                )

        if width != TARGET_SAMPLE_WIDTH:
            data = audioop.lin2lin(data, width, TARGET_SAMPLE_WIDTH)
            width = TARGET_SAMPLE_WIDTH

        if rate != config.SAMPLE_RATE:
            data, self._rate_state = audioop.ratecv(
                data, width, channels, rate, config.SAMPLE_RATE, self._rate_state
            )

        return data

    def start(self):
        if self.mock_file:
            logger.info(f"Starting AudioCapture in MOCK mode with file: {self.mock_file}")
            self.wf = wave.open(self.mock_file, 'rb')
            self.capture_rate = self.wf.getframerate()
            self.capture_channels = self.wf.getnchannels()
            self.capture_width = self.wf.getsampwidth()
            self.capture_chunk_size = self._calc_capture_chunk_size(self.capture_rate)
            self._rate_state = None
            logger.info(
                "Mock capture: %sHz, %sch, %s-byte samples -> stream %sHz, %sch",
                self.capture_rate,
                self.capture_channels,
                self.capture_width,
                config.SAMPLE_RATE,
                config.CHANNELS,
            )
        else:
            logger.info("Starting AudioCapture in LIVE mode")
            capture_format, capture_width = self._resolve_format(config.CAPTURE_FORMAT)
            self.capture_rate = config.CAPTURE_RATE
            self.capture_channels = config.CAPTURE_CHANNELS
            self.capture_width = capture_width
            self.capture_chunk_size = self._calc_capture_chunk_size(self.capture_rate)
            self._rate_state = None

            input_device_index = self._resolve_input_device_index(config.CAPTURE_DEVICE)
            logger.info(
                "Live capture: %sHz, %sch, %s -> stream %sHz, %sch",
                self.capture_rate,
                self.capture_channels,
                config.CAPTURE_FORMAT,
                config.SAMPLE_RATE,
                config.CHANNELS,
            )
            self.stream = self.p.open(
                format=capture_format,
                channels=self.capture_channels,
                rate=self.capture_rate,
                input=True,
                input_device_index=input_device_index,
                frames_per_buffer=self.capture_chunk_size,
            )

    def read_chunk(self):
        """Returns a chunk of bytes or None if stream ended."""
        if self.mock_file:
            data = self.wf.readframes(self.capture_chunk_size)
            if len(data) == 0:
                # End of file, loop or stop? Let's stop/return None for now or loop.
                # Ideally for testing let's loop to simulate continuous stream or just return None.
                # Returning None might kill the server process in main loop,
                # let's seek to start to loop forever for stress testing?
                # For now, let's just loop it.
                logger.debug("Mock file EOF, rewinding.")
                self.wf.rewind()
                data = self.wf.readframes(self.capture_chunk_size)
            
            # Simulate real-time delay
            # Chunk duration = chunk_size / sample_rate
            time.sleep(config.CHUNK_SIZE / config.SAMPLE_RATE)
            return self._convert_to_stream(data)
        else:
            try:
                # exception_on_overflow=False prevents crashes on heavy load
                data = self.stream.read(self.capture_chunk_size, exception_on_overflow=False)
                return self._convert_to_stream(data)
            except IOError as e:
                logger.error(f"Error recording: {e}")
                return b'\x00' * (config.CHUNK_SIZE * TARGET_SAMPLE_WIDTH * config.CHANNELS) # Return silence on error

    def stop(self):
        if self.stream:
            self.stream.stop_stream()
            self.stream.close()
        if self.wf:
            self.wf.close()
        self.p.terminate()
