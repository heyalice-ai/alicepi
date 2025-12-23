"""
Shared protobuf definitions and helpers for AlicePi audio/VAD messaging.
"""

from .vad_pb2 import VadPacket  # noqa: F401
from . import vad  # noqa: F401

__all__ = ["VadPacket", "vad"]
