import socket
import threading
import time
import sys
import json

from alicepi_proto import vad_pb2
from alicepi_proto.vad import encode_packet, make_audio_packet, make_status_packet

# Configuration matches interfaces.py
SR_CONTROL_PORT = 5001
SR_AUDIO_INPUT_PORT = 5002
SR_TEXT_OUTPUT_PORT = 5003

CMD_START = "START"
CMD_STOP = "STOP"

def listen_for_text():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.connect(('localhost', SR_TEXT_OUTPUT_PORT))
        print(f"Connected to Text Output on {SR_TEXT_OUTPUT_PORT}")
        while True:
            data = s.recv(1024)
            if not data: break
            print(f"RECEIVED TEXT: {data.decode('utf-8').strip()}")
    except ConnectionRefusedError:
        print("Text Output Connection Refused (Service not ready?)")
    except Exception as e:
        print(f"Text Listener Error: {e}")

def send_dummy_audio():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.connect(('localhost', SR_AUDIO_INPUT_PORT))
        print(f"Connected to Audio Input on {SR_AUDIO_INPUT_PORT}")

        now_ms = lambda: int(time.time() * 1000)

        # Emit a status transition into speech to match the new protocol
        s.sendall(encode_packet(make_status_packet(vad_pb2.Status.SPEECH_DETECTED, timestamp_ms=now_ms())))

        # Send silence (zeros) as audio payloads
        # 16000 Hz * 2 bytes/sample = 32000 bytes/sec
        data = b'\x00' * 32000
        for sequence in range(1, 4):
            packet = make_audio_packet(
                sample_rate=16000,
                channels=1,
                sequence=sequence,
                data=data,
                timestamp_ms=now_ms(),
            )
            s.sendall(encode_packet(packet))
            time.sleep(1)
            print(f"Sent 1s of silence (packet {sequence})")

        s.sendall(encode_packet(make_status_packet(vad_pb2.Status.SILENCE, timestamp_ms=now_ms())))
        s.close()
    except ConnectionRefusedError:
        print("Audio Input Connection Refused")
    except Exception as e:
        print(f"Audio Sender Error: {e}")

def control_session():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.connect(('localhost', SR_CONTROL_PORT))
        print(f"Connected to Control on {SR_CONTROL_PORT}")
        
        print("Sending START")
        s.sendall(CMD_START.encode('utf-8'))
        
        time.sleep(1)
        send_dummy_audio()
        
        time.sleep(1)
        print("Sending STOP")
        s.sendall(CMD_STOP.encode('utf-8'))
        s.close()
    except ConnectionRefusedError:
        print("Control Connection Refused")

if __name__ == "__main__":
    t_text = threading.Thread(target=listen_for_text, daemon=True)
    t_text.start()
    
    time.sleep(2) # Give it a moment to startup
    
    control_session()
    
    time.sleep(2)
    print("Test finished")
