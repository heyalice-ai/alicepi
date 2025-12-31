import pyaudio
import socket
import sys
import curses
import threading
import time

# Audio configuration
CHUNK = 1024
FORMAT = pyaudio.paInt16
CHANNELS = 1
RATE = 16000


class AudioSender:
    def __init__(self, host, port):
        self.host = host
        self.port = port
        self.p = pyaudio.PyAudio()
        self.stream = None
        self.socket = None
        self.muted = False
        self.running = True
        self.device_index = None

    def get_input_devices(self):
        devices = []
        info = self.p.get_host_api_info_by_index(0)
        numdevices = info.get("deviceCount")
        for i in range(0, numdevices):
            if (
                self.p.get_device_info_by_host_api_device_index(0, i).get(
                    "maxInputChannels"
                )
            ) > 0:
                devices.append(
                    (
                        i,
                        self.p.get_device_info_by_host_api_device_index(0, i).get(
                            "name"
                        ),
                    )
                )
        return devices

    def select_device(self, stdscr):
        devices = self.get_input_devices()
        if not devices:
            return None

        current_row = 0
        while True:
            stdscr.clear()
            stdscr.addstr(
                0, 0, "Select Input Device (Use Arrow Keys, Enter to Select):"
            )

            for idx, (dev_idx, name) in enumerate(devices):
                if idx == current_row:
                    stdscr.addstr(idx + 2, 0, f"> {name}", curses.A_REVERSE)
                else:
                    stdscr.addstr(idx + 2, 0, f"  {name}")

            key = stdscr.getch()

            if key == curses.KEY_UP and current_row > 0:
                current_row -= 1
            elif key == curses.KEY_DOWN and current_row < len(devices) - 1:
                current_row += 1
            elif key == curses.KEY_ENTER or key in [10, 13]:
                return devices[current_row][0]

    def stream_audio(self):
        try:
            self.stream = self.p.open(
                format=FORMAT,
                channels=CHANNELS,
                rate=RATE,
                input=True,
                input_device_index=self.device_index,
                frames_per_buffer=CHUNK,
            )

            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.connect((self.host, self.port))

            while self.running:
                if not self.muted:
                    data = self.stream.read(CHUNK, exception_on_overflow=False)
                    self.socket.sendall(data)
                else:
                    # Send silence or just skip sending?
                    # Sending silence keeps the connection alive and timing consistent.
                    data = b"\x00" * (CHUNK * 2)  # 16-bit audio = 2 bytes per sample
                    self.socket.sendall(data)
                    # Sleep to simulate audio rate
                    time.sleep(CHUNK / RATE)

        except Exception as e:
            pass  # Handle errors gracefully in the UI loop
        finally:
            if self.socket:
                self.socket.close()
            if self.stream:
                self.stream.stop_stream()
                self.stream.close()

    def run(self, stdscr):
        curses.curs_set(0)
        self.device_index = self.select_device(stdscr)

        if self.device_index is None:
            return

        # Start streaming in a separate thread
        stream_thread = threading.Thread(target=self.stream_audio)
        stream_thread.start()

        stdscr.nodelay(True)

        try:
            while self.running:
                stdscr.clear()
                stdscr.addstr(0, 0, f"Streaming to {self.host}:{self.port}")
                stdscr.addstr(2, 0, "Controls:")
                stdscr.addstr(3, 0, "  [M] Toggle Mute")
                stdscr.addstr(4, 0, "  [Q] Quit")

                status = "MUTED" if self.muted else "LIVE"
                color = curses.A_DIM if self.muted else curses.A_BOLD
                stdscr.addstr(6, 0, f"Status: {status}", color)

                key = stdscr.getch()

                if key == ord("m") or key == ord("M"):
                    self.muted = not self.muted
                elif key == ord("q") or key == ord("Q"):
                    self.running = False

                time.sleep(0.1)

        finally:
            self.running = False
            stream_thread.join()
            self.p.terminate()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <host> <port>")
        sys.exit(1)

    host = sys.argv[1]
    port = int(sys.argv[2])

    sender = AudioSender(host, port)
    curses.wrapper(sender.run)
