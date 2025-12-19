import logging
import time
import os
import argparse
from . import config
from .audio_capture import AudioCapture
from .vad import VADWrapper
from .streamer import AudioStreamer

logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(name)s - %(levelname)s - %(message)s')
logger = logging.getLogger("VoiceInputService")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mock-file", help="Path to a wav file to mock microphone input")
    args = parser.parse_args()

    # Determine mock file from args or env var
    mock_file = args.mock_file or os.environ.get("MOCK_AUDIO_FILE")

    capture = AudioCapture(mock_file=mock_file)
    vad = VADWrapper()
    streamer = AudioStreamer()

    try:
        capture.start()
        streamer.start()

        logger.info("Voice Input Service Started")
        
        while True:
            chunk = capture.read_chunk()
            if chunk is None:
                # Should not happen in current loop implementation
                break

            if vad.process(chunk):
                # Speech detected, stream it
                # logger.debug("Speech detected")
                streamer.send_chunk(chunk)
            else:
                # Silence
                # We might want to stream a bit of silence context or just nothing
                # For now, simplistic VAD: no speech, no stream.
                # NOTE: A real system needs "hangover" (keep sending for N ms after speech ends)
                # to prevent choppy sentences.
                # However, task just says "Output raw PCM data when speech is detected."
                pass
                
    except KeyboardInterrupt:
        logger.info("Stopping...")
    except Exception as e:
        logger.exception("Generla Error")
    finally:
        capture.stop()
        streamer.stop()

if __name__ == "__main__":
    main()
