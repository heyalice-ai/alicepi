import struct
import time
from typing import List

from .vad_pb2 import VadPacket

_LENGTH_PREFIX_BYTES = 4


def encode_packet(packet: VadPacket) -> bytes:
    """Length-prefix encode a VadPacket for streaming transport."""
    payload = packet.SerializeToString()
    return struct.pack(">I", len(payload)) + payload


class VadPacketFramer:
    """Framer that handles length-prefixed VadPacket streams."""

    def __init__(self):
        self._buffer = bytearray()

    def decode(self, data: bytes) -> List[VadPacket]:
        """Feed raw bytes, returning any complete packets decoded."""
        packets: List[VadPacket] = []
        self._buffer.extend(data)

        while len(self._buffer) >= _LENGTH_PREFIX_BYTES:
            message_len = struct.unpack(">I", self._buffer[:_LENGTH_PREFIX_BYTES])[0]
            if len(self._buffer) < _LENGTH_PREFIX_BYTES + message_len:
                break

            start = _LENGTH_PREFIX_BYTES
            end = _LENGTH_PREFIX_BYTES + message_len
            payload = bytes(self._buffer[start:end])
            del self._buffer[:end]

            packet = VadPacket()
            packet.ParseFromString(payload)
            packets.append(packet)

        return packets


def make_audio_packet(
    *,
    sample_rate: int,
    channels: int,
    sequence: int,
    data: bytes,
    timestamp_ms: int | None = None,
) -> VadPacket:
    packet = VadPacket(timestamp_ms=timestamp_ms or _now_ms())
    audio = packet.audio
    audio.sample_rate = sample_rate
    audio.channels = channels
    audio.sequence = sequence
    audio.data = data
    return packet


def make_status_packet(
    status: int,
    *,
    timestamp_ms: int | None = None,
) -> VadPacket:
    packet = VadPacket(timestamp_ms=timestamp_ms or _now_ms())
    packet.status = status
    return packet


def _now_ms() -> int:
    return int(time.time() * 1000)
