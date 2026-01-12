import argparse
import audioop
import time
import wave

import zmq


DEFAULT_TOPIC = "voice_output_audio"
DEFAULT_RATE = 48000
DEFAULT_CHANNELS = 2
DEFAULT_SAMPLE_WIDTH = 4


def _convert_chunk(
    data,
    in_width,
    in_channels,
    in_rate,
    target_width,
    target_channels,
    target_rate,
    rate_state,
):
    if in_width != target_width:
        data = audioop.lin2lin(data, in_width, target_width)
        in_width = target_width

    if in_channels != target_channels:
        if in_channels == 2 and target_channels == 1:
            data = audioop.tomono(data, in_width, 0.5, 0.5)
        elif in_channels == 1 and target_channels == 2:
            data = audioop.tostereo(data, in_width, 1.0, 1.0)
        else:
            raise ValueError(
                f"Unsupported channel conversion: {in_channels} -> {target_channels}"
            )
        in_channels = target_channels

    if in_rate != target_rate:
        data, rate_state = audioop.ratecv(
            data,
            in_width,
            in_channels,
            in_rate,
            target_rate,
            rate_state,
        )

    return data, rate_state


def main():
    parser = argparse.ArgumentParser(
        description="Publish WAV audio to the voice_output ZMQ topic."
    )
    parser.add_argument("wav_path", help="Path to a PCM WAV file.")
    parser.add_argument(
        "--zmq-url",
        default="tcp://voice-output:5557",
        help="ZMQ endpoint for PUB socket.",
    )
    parser.add_argument(
        "--topic",
        default=DEFAULT_TOPIC,
        help="ZMQ topic for audio frames.",
    )
    parser.add_argument(
        "--rate",
        type=int,
        default=DEFAULT_RATE,
        help="Target sample rate (Hz).",
    )
    parser.add_argument(
        "--channels",
        type=int,
        default=DEFAULT_CHANNELS,
        help="Target channel count.",
    )
    parser.add_argument(
        "--sample-width",
        type=int,
        default=DEFAULT_SAMPLE_WIDTH,
        help="Target sample width in bytes (S32_LE = 4).",
    )
    parser.add_argument(
        "--chunk-frames",
        type=int,
        default=1024,
        help="Input frames per chunk before conversion.",
    )
    parser.add_argument(
        "--no-realtime",
        action="store_true",
        help="Send as fast as possible instead of realtime pacing.",
    )
    parser.add_argument(
        "--bind",
        action="store_true",
        help="Bind instead of connect (default is connect).",
    )
    args = parser.parse_args()

    ctx = zmq.Context()
    socket = ctx.socket(zmq.PUB)
    if args.bind:
        socket.bind(args.zmq_url)
    else:
        socket.connect(args.zmq_url)

    topic_bytes = args.topic.encode("utf-8")
    rate_state = None
    time.sleep(0.2)

    with wave.open(args.wav_path, "rb") as wav_file:
        in_channels = wav_file.getnchannels()
        in_rate = wav_file.getframerate()
        in_width = wav_file.getsampwidth()

        print(f"Input WAV: {in_rate}Hz, {in_channels}ch, {in_width * 8}bit")

        while True:
            chunk = wav_file.readframes(args.chunk_frames)
            if not chunk:
                break

            out_chunk, rate_state = _convert_chunk(
                chunk,
                in_width,
                in_channels,
                in_rate,
                args.sample_width,
                args.channels,
                args.rate,
                rate_state,
            )

            if not out_chunk:
                continue

            socket.send_multipart([topic_bytes, out_chunk])

            if not args.no_realtime:
                frame_count = len(out_chunk) // (args.sample_width * args.channels)
                time.sleep(frame_count / float(args.rate))

    socket.close(linger=0)
    ctx.term()


if __name__ == "__main__":
    main()
