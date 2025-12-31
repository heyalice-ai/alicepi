import curses
import socket
import sys
import threading
import time
import wave
from pathlib import Path

import pyaudio

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
        self.sample_width = self.p.get_sample_size(FORMAT)
        self.recording = False
        self.record_frames = []
        self.record_lock = threading.Lock()
        self.status_message = ""

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
            silence_chunk = b"\x00" * (CHUNK * self.sample_width)

            while self.running:
                audio_chunk = None

                if not self.muted or self.recording:
                    audio_chunk = self.stream.read(
                        CHUNK, exception_on_overflow=False
                    )

                if not self.muted and audio_chunk is not None:
                    payload = audio_chunk
                else:
                    payload = silence_chunk

                self.socket.sendall(payload)

                if self.muted and not self.recording:
                    time.sleep(CHUNK / RATE)

                if self.recording and audio_chunk is not None:
                    with self.record_lock:
                        self.record_frames.append(audio_chunk)

        except Exception as e:
            self.status_message = f"Stream error: {e}"
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
                max_row = stdscr.getmaxyx()[0] - 1

                def clamp(row):
                    return max(0, min(row, max_row))

                stdscr.addstr(clamp(0), 0, f"Streaming to {self.host}:{self.port}")
                stdscr.addstr(clamp(2), 0, "Controls:")
                stdscr.addstr(clamp(3), 0, "  [M] Toggle Mute")
                stdscr.addstr(clamp(4), 0, "  [R] Toggle Record")
                stdscr.addstr(clamp(5), 0, "  [Q] Quit")

                status = "MUTED" if self.muted else "LIVE"
                color = curses.A_DIM if self.muted else curses.A_BOLD
                stdscr.addstr(clamp(7), 0, f"Stream: {status}", color)

                rec_status = "ACTIVE" if self.recording else "OFF"
                rec_color = curses.A_BLINK if self.recording else curses.A_NORMAL
                stdscr.addstr(clamp(8), 0, f"Recording: {rec_status}", rec_color)

                if self.status_message:
                    stdscr.addstr(
                        clamp(10),
                        0,
                        f"Message: {self.status_message}"[: max(0, stdscr.getmaxyx()[1] - 1)],
                    )

                key = stdscr.getch()

                if key == ord("m") or key == ord("M"):
                    self.muted = not self.muted
                elif key == ord("r") or key == ord("R"):
                    if not self.recording:
                        self.start_recording()
                    else:
                        self.stop_recording(stdscr)
                elif key == ord("q") or key == ord("Q"):
                    self.running = False

                time.sleep(0.1)

        finally:
            self.running = False
            stream_thread.join()
            self.p.terminate()

    def start_recording(self):
        with self.record_lock:
            self.record_frames = []
        self.recording = True
        self.status_message = "Recording started"

    def stop_recording(self, stdscr):
        self.recording = False
        with self.record_lock:
            frames = list(self.record_frames)
            self.record_frames = []

        if not frames:
            self.status_message = "No audio captured"
            return

        filename = self.prompt_filename(stdscr)
        try:
            filepath = self.prepare_filename(filename)
            self.write_wave_file(filepath, frames)
            self.status_message = f"Saved recording to {filepath}"
        except Exception as exc:
            self.status_message = f"Save failed: {exc}"

    def prompt_filename(self, stdscr):
        default_name = self.generate_default_filename()
        prompt = f"Enter filename [{default_name}]: "
        prompt_row = max(0, stdscr.getmaxyx()[0] - 2)

        stdscr.nodelay(False)
        curses.echo()
        stdscr.move(prompt_row, 0)
        stdscr.clrtoeol()
        stdscr.addstr(prompt_row, 0, prompt)
        user_input = stdscr.getstr()
        curses.noecho()
        stdscr.nodelay(True)

        try:
            value = user_input.decode("utf-8").strip()
        except UnicodeDecodeError:
            value = ""

        return value or default_name

    def generate_default_filename(self):
        base = Path.cwd()
        for number in range(10000):
            candidate = base / f"output-{number:04d}.wav"
            if not candidate.exists():
                return str(candidate)
        return str(base / "output.wav")

    def prepare_filename(self, filename):
        path = Path(filename).expanduser()
        if not path.suffix:
            path = path.with_suffix(".wav")
        return path

    def write_wave_file(self, filepath, frames):
        filepath.parent.mkdir(parents=True, exist_ok=True)
        with wave.open(str(filepath), "wb") as wav_file:
            wav_file.setnchannels(CHANNELS)
            wav_file.setsampwidth(self.sample_width)
            wav_file.setframerate(RATE)
            wav_file.writeframes(b"".join(frames))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <host> <port>")
        sys.exit(1)

    host = sys.argv[1]
    port = int(sys.argv[2])

    sender = AudioSender(host, port)
    curses.wrapper(sender.run)
