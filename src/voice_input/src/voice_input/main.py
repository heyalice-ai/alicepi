import logging
import time
import os
import argparse
from . import config
from .audio_capture import AudioCapture
from .vad import VADWrapper
from .streamer import AudioStreamer
from alicepi_proto import vad_pb2
from alicepi_proto.vad import make_audio_packet, make_status_packet

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
    last_status = vad_pb2.VadPacket.Status.SILENCE
    sequence = 0
    
    # Validate configuration
    if config.SILENCE_DURATION_MS <= 0:
        raise ValueError(f"SILENCE_DURATION_MS must be positive, got {config.SILENCE_DURATION_MS}")
    
    hangover_duration = config.SILENCE_DURATION_MS / 1000.0  # Convert to seconds
    
    try:
        capture.start()
        streamer.start()

        logger.info("Voice Input Service Started")
        logger.info(
            f"VadPacket Output -> {config.SPEECH_REC_HOST}:{config.SPEECH_REC_AUDIO_PORT}"
        )
        logger.info(f"VAD Hangover Duration: {config.SILENCE_DURATION_MS}ms")
        
        while True:
            chunk = capture.read_chunk()
            if chunk is None:
                # Should not happen in current loop implementation
                break

            current_time = time.time()
            now_ms = int(current_time * 1000)
            is_speech = vad.process(chunk)
            
            if is_speech:
                # Speech detected, stream it and update last speech time
                sequence += 1
                packet = make_audio_packet(
                    sample_rate=config.SAMPLE_RATE,
                    channels=config.CHANNELS,
                    sequence=sequence,
                    data=chunk,
                    timestamp_ms=now_ms,
                )
                streamer.send_packet(packet)
                if last_status != vad_pb2.VadPacket.Status.SPEECH_DETECTED:
                    status_packet = make_status_packet(
                        vad_pb2.VadPacket.Status.SPEECH_DETECTED, timestamp_ms=now_ms
                    )
                    streamer.send_packet(status_packet)
                    last_status = vad_pb2.VadPacket.Status.SPEECH_DETECTED
                last_speech_time = current_time
            else:
                # Silence detected
                # Check if we're in hangover period (recently had speech)
                if last_speech_time is not None:
                    time_since_speech = current_time - last_speech_time
                    
                    if time_since_speech < hangover_duration:
                        # Still in hangover period, continue streaming silence
                        sequence += 1
                        packet = make_audio_packet(
                            sample_rate=config.SAMPLE_RATE,
                            channels=config.CHANNELS,
                            sequence=sequence,
                            data=chunk,
                            timestamp_ms=now_ms,
                        )
                        streamer.send_packet(packet)
                        if last_status != vad_pb2.VadPacket.Status.SPEECH_HANGOVER:
                            status_packet = make_status_packet(
                                vad_pb2.VadPacket.Status.SPEECH_HANGOVER, timestamp_ms=now_ms
                            )
                            streamer.send_packet(status_packet)
                            last_status = vad_pb2.VadPacket.Status.SPEECH_HANGOVER
                    else:
                        # Hangover period ended, stop streaming
                        if last_status != vad_pb2.VadPacket.Status.SILENCE:
                            status_packet = make_status_packet(
                                vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                            )
                            streamer.send_packet(status_packet)
                            last_status = vad_pb2.VadPacket.Status.SILENCE
                        last_speech_time = None
                else:
                    # No recent speech, just emit silence status transitions
                    if last_status != vad_pb2.VadPacket.Status.SILENCE:
                        status_packet = make_status_packet(
                            vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                        )
                        streamer.send_packet(status_packet)
                        last_status = vad_pb2.VadPacket.Status.SILENCE
                
    except KeyboardInterrupt:
        logger.info("Stopping...")
    except Exception as e:
        logger.exception("Generla Error")
    finally:
        capture.stop()
        streamer.stop()

if __name__ == "__main__":
    main()
