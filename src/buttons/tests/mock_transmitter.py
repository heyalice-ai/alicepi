import zmq
import json
import time
import sys
from datetime import datetime

def send_event(event_name, port=5558):
    ctx = zmq.Context()
    pub = ctx.socket(zmq.PUB)
    pub.bind(f"tcp://0.0.0.0:{port}") # We bind because Orchestrator connects
    
    print(f"Mock Button Service started on port {port}")
    print(f"Sending event: {event_name}")
    
    # Wait for Orchestrator to connect (ZMQ is async, so we might need a small delay or just wait)
    time.sleep(1)
    
    payload = {
        "event": event_name,
        "timestamp": datetime.utcnow().isoformat()
    }
    msg = json.dumps(payload)
    pub.send_string(msg)
    print("Done.")
    
    pub.close()
    ctx.term()

if __name__ == "__main__":
    event = sys.argv[1] if len(sys.argv) > 1 else "RESET"
    send_event(event)
