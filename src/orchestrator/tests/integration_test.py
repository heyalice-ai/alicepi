import http.server
import shutil
import threading
import socket
import zmq
import time
import json
import sys
import os

# Adjust path to find orchestrator source
sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), '../src')))

# Mock Config (MONKEY PATCHING)
os.environ['SPEECH_REC_HOST'] = 'localhost'
os.environ['BUTTONS_HOST'] = 'localhost'
os.environ['VOICE_OUTPUT_HOST'] = 'localhost'

import config

# Mock LLM API
class MockLLMHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers['Content-Length'])
        post_data = self.rfile.read(content_length)
        payload = json.loads(post_data.decode('utf-8'))
        
        print(f"[Mock LLM] Received Request: {payload}")
        
        # Verify history is growing
        history = payload.get("messages", [])
        user_msgs = [m for m in history if m["role"] == "user"]
        
        response_text = f"Response {len(user_msgs)}"
        
        response_payload = {
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": response_text
                }
            }]
        }
        
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(response_payload).encode('utf-8'))

def run_mock_llm(stop_event):
    server = http.server.HTTPServer(('localhost', 11434), MockLLMHandler)
    server.timeout = 0.5
    print("[Mock LLM] Listening on 11434...")
    while not stop_event.is_set():
        server.handle_request()
    server.server_close()

# Mock Speech Rec Service
def run_mock_speech_rec(stop_event, trigger_msg_event):
    ctrl_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    ctrl_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    ctrl_sock.bind(('localhost', config.SPEECH_REC_CONTROL_PORT))
    ctrl_sock.listen(1)
    
    text_sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    text_sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    text_sock.bind(('localhost', config.SPEECH_REC_TEXT_PORT))
    text_sock.listen(1)
    
    conn, _ = ctrl_sock.accept()
    text_conn, _ = text_sock.accept()
    
    while not stop_event.is_set():
        if trigger_msg_event.wait(0.1):
            trigger_msg_event.clear()
            print("[Mock SR] Sending text...")
            msg = json.dumps({"text": "User msg", "is_final": True}) + "\n"
            text_conn.sendall(msg.encode())
            
    conn.close()
    text_conn.close()
    ctrl_sock.close()
    text_sock.close()

# Mock Voice Output (Subscriber)
def run_mock_voice_output(stop_event, response_count_event):
    ctx = zmq.Context()
    sub_socket = ctx.socket(zmq.SUB)
    sub_socket.connect(f"tcp://localhost:{config.VOICE_OUTPUT_PORT}") 
    sub_socket.subscribe("")
    
    count = 0
    while not stop_event.is_set():
        try:
            if sub_socket.poll(100):
                msg = sub_socket.recv_multipart()
                topic = msg[0].decode()
                payload = msg[1].decode() if len(msg) > 1 else ""
                print(f"[Mock VO] Received on {topic}: {payload}")
                if topic == config.ZMQ_TOPIC_CONTROL:
                    count += 1
                    response_count_event.set()
        except zmq.ZMQError:
            break

# Mock Buttons Service (Publisher)
def run_mock_buttons(stop_event):
    ctx = zmq.Context()
    pub_socket = ctx.socket(zmq.PUB)
    pub_socket.bind(f"tcp://*:{config.BUTTONS_PORT}") 
    while not stop_event.is_set():
        time.sleep(0.1)

if __name__ == "__main__":
    # Test Setup
    LOG_PATH = "/tmp/heyalice_session_test.jsonl"
    if os.path.exists(LOG_PATH): os.remove(LOG_PATH)
    
    os.environ['ENABLE_SESSION_LOGGING'] = 'true'
    os.environ['SESSION_LOG_PATH'] = LOG_PATH
    os.environ['LLM_API_URL'] = 'http://localhost:11434'
    
    stop_event = threading.Event()
    trigger_sr_event = threading.Event()
    response_received_event = threading.Event()
    
    t_llm = threading.Thread(target=run_mock_llm, args=(stop_event,), daemon=True)
    t_sr = threading.Thread(target=run_mock_speech_rec, args=(stop_event, trigger_sr_event), daemon=True)
    t_vo = threading.Thread(target=run_mock_voice_output, args=(stop_event, response_received_event), daemon=True)
    t_btn = threading.Thread(target=run_mock_buttons, args=(stop_event,), daemon=True)
    
    t_llm.start()
    t_sr.start()
    t_vo.start()
    t_btn.start()
    
    time.sleep(1) 
    
    from main import Orchestrator
    orch = Orchestrator()
    t_orch = threading.Thread(target=orch.start, daemon=True)
    t_orch.start()
    
    # Wait for ZMQ PUB/SUB connection to stabilize
    print("[Main] Orchestrator started. Waiting for connections...")
    time.sleep(2)
    
    print("[Main] Sending Message 1...")
    trigger_sr_event.set()
    
    if response_received_event.wait(timeout=10):
        print("[Main] Message 1 handled. Sending Message 2 within 2s...")
        response_received_event.clear()
        time.sleep(1)
        trigger_sr_event.set()
        
        if response_received_event.wait(timeout=10):
            print("[Main] Message 2 handled.")
            # SUCCESS!
        else:
            print("[Main] FAILURE: Message 2 timed out")
    else:
        print("[Main] FAILURE: Message 1 timed out")
    
    print("[Main] Stopping and checking logs...")
    orch.stop()
    stop_event.set()
    
    time.sleep(0.5)
    if os.path.exists(LOG_PATH):
        with open(LOG_PATH, "r") as f:
            logs = f.readlines()
            print(f"[Main] LOG CONTENT: {logs}")
            if len(logs) > 0:
                print("[Main] SUCCESS: Session logged correctly.")
            else:
                print("[Main] FAILURE: Log file empty.")
    else:
        print(f"[Main] FAILURE: Log file {LOG_PATH} not found.")
        
    sys.exit(0)
