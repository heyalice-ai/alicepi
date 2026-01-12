import os
import sys
import zmq
import sounddevice as sd
import numpy as np
import json
import logging

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(name)s - %(levelname)s - %(message)s')
logger = logging.getLogger("voice_output")

# Configuration
ZMQ_PUB_URL = os.environ.get("ZMQ_PUB_URL", "tcp://0.0.0.0:5557") # Voice output owns the bind endpoint
ZMQ_SUB_TOPIC_AUDIO = os.environ.get("ZMQ_TOPIC_AUDIO", "voice_output_audio")
ZMQ_SUB_TOPIC_CONTROL = os.environ.get("ZMQ_TOPIC_CONTROL", "voice_output_control")

SAMPLE_RATE = int(os.environ.get("SAMPLE_RATE", 24000))
CHANNELS = int(os.environ.get("CHANNELS", 1))
DTYPE = 'int16' # Assumed PCM 16-bit
PLAYBACK_DEVICE = os.environ.get("PLAYBACK_DEVICE")

def main():
    logger.info("Starting Voice Output Service...")
    logger.info(f"Connecting to ZMQ at {ZMQ_PUB_URL}")
    logger.info(f"Audio Config: {SAMPLE_RATE}Hz, {CHANNELS}ch, {DTYPE}")
    if PLAYBACK_DEVICE:
        logger.info(f"Playback device: {PLAYBACK_DEVICE}")

    # ZMQ Setup
    ctx = zmq.Context()
    socket = ctx.socket(zmq.SUB)
    socket.bind(ZMQ_PUB_URL)
    socket.setsockopt_string(zmq.SUBSCRIBE, ZMQ_SUB_TOPIC_AUDIO)
    socket.setsockopt_string(zmq.SUBSCRIBE, ZMQ_SUB_TOPIC_CONTROL)

    # Audio Setup
    try:
        # We use a RawOutputStream to write bytes directly
        stream = sd.RawOutputStream(
            samplerate=SAMPLE_RATE,
            channels=CHANNELS,
            dtype=DTYPE,
            blocksize=1024, # tuning might be needed
            device=PLAYBACK_DEVICE,
        )
        stream.start()
        logger.info("Audio stream started.")
    except Exception as e:
        logger.error(f"Failed to open audio device: {e}")
        sys.exit(1)

    try:
        while True:
            # Receive multipart: [topic, payload]
            try:
                topic, payload = socket.recv_multipart()
                topic = topic.decode('utf-8')
            except ValueError:
                continue # Ignore malformed messages

            if topic == ZMQ_SUB_TOPIC_AUDIO:
                # payload is raw PCM bytes
                if stream.active:
                    stream.write(payload)
                else:
                    logger.warning("Received audio but stream is inactive.")

            elif topic == ZMQ_SUB_TOPIC_CONTROL:
                try:
                    command = json.loads(payload.decode('utf-8'))
                    handle_command(command, stream)
                except Exception as e:
                    logger.error(f"Error handling control command: {e}")

    except KeyboardInterrupt:
        logger.info("Stopping service...")
    finally:
        stream.stop()
        stream.close()
        socket.close()
        ctx.term()

def handle_command(command, stream):
    cmd_type = command.get("type")
    if cmd_type == "stop":
        # For RawOutputStream, proper "stopping" usually means aborting current buffer or closing.
        # But here we might just want to silence it or stop accepting new data for a moment.
        # sd.RawOutputStream 'stop' closes the stream which is heavy. 
        # Ideally we just drop buffers if we want to silence.
        # But if the user means "stop playing current file", we don't have a file buffer, we just stream what we get.
        # So "stop" essentially means "reset" here from the sender side, but maybe we can clear local latency buffers if any.
        # sounddevice doesn't expose a 'flush' easily on RawOutputStream without stopping.
        pass
    elif cmd_type == "pause":
        # Maybe toggle a flag in main loop to not write to stream
        pass
    
    logger.info(f"Received control command: {cmd_type} (Not fully implemented)")

if __name__ == "__main__":
    main()
