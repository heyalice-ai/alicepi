import logging
import time
import os
import argparse
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
    
    hangover_duration = config.SILENCE_DURATION_MS / 1000.0
    
    try:
        receiver.start()
        streamer.start()

        logger.info("Network Voice Input Service Started")
        logger.info(f"Port 5558 -> Raw PCM Input")
        logger.info(
            f"VadPacket Output -> {config.SPEECH_REC_HOST}:{config.SPEECH_REC_AUDIO_PORT}"
        )
        
        while True:
            chunk = receiver.read_chunk()
            if chunk is None:
                # Idle wait for network data
                continue

            current_time = time.time()
            now_ms = int(current_time * 1000)
            is_speech = vad.process(chunk)
            
            if is_speech:
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
                if last_speech_time is not None:
                    time_since_speech = current_time - last_speech_time
                    
                    if time_since_speech < hangover_duration:
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
                        if last_status != vad_pb2.VadPacket.Status.SILENCE:
                            status_packet = make_status_packet(
                                vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                            )
                            streamer.send_packet(status_packet)
                            last_status = vad_pb2.VadPacket.Status.SILENCE
                        last_speech_time = None
                else:
                    if last_status != vad_pb2.VadPacket.Status.SILENCE:
                        status_packet = make_status_packet(
                            vad_pb2.VadPacket.Status.SILENCE, timestamp_ms=now_ms
                        )
                        streamer.send_packet(status_packet)
                        last_status = vad_pb2.VadPacket.Status.SILENCE
                
    except KeyboardInterrupt:
        logger.info("Stopping...")
    except Exception as e:
        logger.exception("General Error in NetworkVoiceInputService")
    finally:
        receiver.stop()
        streamer.stop()

if __name__ == "__main__":
    main()
