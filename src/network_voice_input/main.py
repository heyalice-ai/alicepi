import logging
import time
import config
from receiver import NetworkReceiver
from vad import VADWrapper
from streamer import AudioStreamer
from alicepi_proto import vad_pb2
from alicepi_proto.vad import make_audio_packet, make_status_packet

logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(name)s - %(levelname)s - %(message)s')
logger = logging.getLogger("NetworkVoiceInputService")

def main():
    receiver = NetworkReceiver()
    vad = VADWrapper()
    streamer = AudioStreamer()

    last_speech_time = None
    last_status = vad_pb2.VadPacket.Status.SILENCE
    sequence = 0
    expected_chunk_bytes = config.CHUNK_SIZE * 2
    stats_interval_s = 5.0
    last_stats_log = time.monotonic()
    last_chunk_at = time.monotonic()
    last_no_data_log = time.monotonic()
    stats = {
        "chunks": 0,
        "speech": 0,
        "hangover": 0,
        "silence": 0,
    }
    
    if config.SILENCE_DURATION_MS <= 0:
        raise ValueError(f"SILENCE_DURATION_MS must be positive, got {config.SILENCE_DURATION_MS}")

    hangover_duration = config.SILENCE_DURATION_MS / 1000.0
    
    try:
        receiver.start()
        streamer.start()

        logger.info("Network Voice Input Service Started")
        logger.info(f"Port 5558 -> Raw PCM Input")
        logger.info(
            f"VadPacket Output -> {config.SPEECH_REC_HOST}:{config.SPEECH_REC_AUDIO_PORT}"
        )
        logger.info(f"VAD Hangover Duration: {config.SILENCE_DURATION_MS}ms")
        
        while True:
            chunk = receiver.read_chunk()
            if chunk is None:
                # Idle wait for network data
                now = time.monotonic()
                if now - last_chunk_at >= stats_interval_s and now - last_no_data_log >= stats_interval_s:
                    logger.warning("No network audio received for %.1fs", now - last_chunk_at)
                    last_no_data_log = now
                continue
            last_chunk_at = time.monotonic()
            stats["chunks"] += 1

            if len(chunk) != expected_chunk_bytes:
                logger.warning(
                    "Unexpected chunk size: got=%d expected=%d",
                    len(chunk),
                    expected_chunk_bytes,
                )

            current_time = time.time()
            now_ms = int(current_time * 1000)
            is_speech = vad.process(chunk)
            
            if is_speech:
                stats["speech"] += 1
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
                    logger.info("VAD status -> SPEECH_DETECTED")
                    last_status = vad_pb2.VadPacket.Status.SPEECH_DETECTED
                last_speech_time = current_time
            else:
                if last_speech_time is not None:
                    time_since_speech = current_time - last_speech_time
                    
                    if time_since_speech < hangover_duration:
                        stats["hangover"] += 1
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
                            logger.info("VAD status -> SPEECH_HANGOVER")
                            last_status = vad_pb2.VadPacket.Status.SPEECH_HANGOVER
                    else:
                        stats["silence"] += 1
                        if last_status != vad_pb2.VadPacket.Status.SILENCE:
                            status_packet = make_status_packet(
                                vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                            )
                            streamer.send_packet(status_packet)
                            logger.info("VAD status -> SILENCE")
                            last_status = vad_pb2.VadPacket.Status.SILENCE
                        last_speech_time = None
                else:
                    stats["silence"] += 1
                    if last_status != vad_pb2.VadPacket.Status.SILENCE:
                        status_packet = make_status_packet(
                            vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                        )
                        streamer.send_packet(status_packet)
                        logger.info("VAD status -> SILENCE")
                        last_status = vad_pb2.VadPacket.Status.SILENCE

            now = time.monotonic()
            if now - last_stats_log >= stats_interval_s:
                logger.info(
                    "VAD stats: chunks=%d speech=%d hangover=%d silence=%d last_status=%s",
                    stats["chunks"],
                    stats["speech"],
                    stats["hangover"],
                    stats["silence"],
                    vad_pb2.VadPacket.Status.Name(last_status),
                )
                stats = {"chunks": 0, "speech": 0, "hangover": 0, "silence": 0}
                last_stats_log = now
                
    except KeyboardInterrupt:
        logger.info("Stopping...")
    except Exception as e:
        logger.exception("General Error in NetworkVoiceInputService")
    finally:
        receiver.stop()
        streamer.stop()

if __name__ == "__main__":
    main()
