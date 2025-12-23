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

    # Hangover state management
    # When speech ends, we continue streaming for a "hangover" period
    # to avoid cutting off trailing sounds and provide context to the recognizer
    last_speech_time = None
    
    # Validate configuration
    if config.SILENCE_DURATION_MS <= 0:
        raise ValueError(f"SILENCE_DURATION_MS must be positive, got {config.SILENCE_DURATION_MS}")
    
    hangover_duration = config.SILENCE_DURATION_MS / 1000.0  # Convert to seconds
    
    try:
        capture.start()
        streamer.start()

        logger.info("Voice Input Service Started")
        logger.info(f"VAD Hangover Duration: {config.SILENCE_DURATION_MS}ms")
        
        while True:
            chunk = capture.read_chunk()
            if chunk is None:
                # Should not happen in current loop implementation
                break

            current_time = time.time()
            is_speech = vad.process(chunk)
            
            if is_speech:
                # Speech detected, stream it and update last speech time
                streamer.send_chunk(chunk)
                last_speech_time = current_time
            else:
                # Silence detected
                # Check if we're in hangover period (recently had speech)
                if last_speech_time is not None:
                    time_since_speech = current_time - last_speech_time
                    
                    if time_since_speech < hangover_duration:
                        # Still in hangover period, continue streaming silence
                        streamer.send_chunk(chunk)
                    else:
                        # Hangover period ended, stop streaming
                        last_speech_time = None
                # else: No recent speech, don't stream
                
    except KeyboardInterrupt:
        logger.info("Stopping...")
    except Exception as e:
        logger.exception("Generla Error")
    finally:
        capture.stop()
        streamer.stop()

if __name__ == "__main__":
    main()
