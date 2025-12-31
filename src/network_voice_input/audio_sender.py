import argparse
import curses
import socket
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
    def __init__(self, host, port, playback_path=None):
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
        self.playback_path = Path(playback_path).expanduser() if playback_path else None
        self.playback_rate = RATE
        self.playback_channels = CHANNELS

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
        wave_reader = None
        try:
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.connect((self.host, self.port))

            if self.playback_path:
                wave_reader = wave.open(str(self.playback_path), "rb")
                self.configure_playback_source(wave_reader)
                self.stream_file_audio(wave_reader)
            else:
                self.stream_microphone_audio()

        except Exception as e:
            self.status_message = f"Stream error: {e}"
        finally:
            if self.socket:
                self.socket.close()
            if self.stream:
                self.stream.stop_stream()
                self.stream.close()
            if wave_reader:
                wave_reader.close()

    def configure_playback_source(self, wave_reader):
        channels = wave_reader.getnchannels()
        sample_width = wave_reader.getsampwidth()
        rate = wave_reader.getframerate()

        if channels != CHANNELS:
            raise ValueError(
                f"Playback file must be mono ({CHANNELS} channel), got {channels}"
            )
        if sample_width != self.sample_width:
            self.sample_width = sample_width
        if rate != RATE:
            raise ValueError(f"Playback file must be {RATE} Hz, got {rate}")

        self.playback_rate = rate
        self.playback_channels = channels

    def stream_microphone_audio(self):
        if self.socket is None:
            raise RuntimeError("Socket not initialized")
        sock = self.socket

        self.stream = self.p.open(
            format=FORMAT,
            channels=CHANNELS,
            rate=RATE,
            input=True,
            input_device_index=self.device_index,
            frames_per_buffer=CHUNK,
        )

        silence_chunk = b"\x00" * (CHUNK * self.sample_width)

        while self.running:
            audio_chunk = None

            if not self.muted or self.recording:
                audio_chunk = self.stream.read(CHUNK, exception_on_overflow=False)

            if not self.muted and audio_chunk is not None:
                payload = audio_chunk
            else:
                payload = silence_chunk

            sock.sendall(payload)

            if self.muted and not self.recording:
                time.sleep(CHUNK / RATE)

            if self.recording and audio_chunk is not None:
                with self.record_lock:
                    self.record_frames.append(audio_chunk)

    def stream_file_audio(self, wave_reader):
        if self.socket is None:
            raise RuntimeError("Socket not initialized")
        sock = self.socket

        chunk_frames = CHUNK
        silence_chunk = b"\x00" * (chunk_frames * self.sample_width)

        while self.running:
            if self.muted:
                sock.sendall(silence_chunk)
                time.sleep(chunk_frames / self.playback_rate)
                continue

            data = wave_reader.readframes(chunk_frames)
            if not data:
                self.status_message = "Playback finished"
                self.running = False
                break

            sock.sendall(data)
            frames_sent = len(data) // (self.sample_width * self.playback_channels)
            if frames_sent == 0:
                time.sleep(chunk_frames / self.playback_rate)
            else:
                time.sleep(frames_sent / self.playback_rate)

    def run(self, stdscr):
        curses.curs_set(0)
        if self.playback_path:
            self.status_message = f"Playback file: {self.playback_path}"
        else:
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
                if self.playback_path:
                    stdscr.addstr(clamp(1), 0, f"Source: {self.playback_path}")
                stdscr.addstr(clamp(2), 0, "Controls:")
                stdscr.addstr(clamp(3), 0, "  [M] Toggle Mute")
                stdscr.addstr(
                    clamp(4),
                    0,
                    "  [R] Toggle Record" + (" (N/A)" if self.playback_path else ""),
                )
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
                    if self.playback_path:
                        self.status_message = "Recording disabled in file mode"
                    elif not self.recording:
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
    parser = argparse.ArgumentParser(
        description="Stream live microphone audio or a WAV file to a TCP endpoint."
    )
    parser.add_argument("host", help="Destination host")
    parser.add_argument("port", type=int, help="Destination port")
    parser.add_argument(
        "--file",
        dest="playback_file",
        help="Path to a WAV file to stream instead of the microphone",
    )

    args = parser.parse_args()

    sender = AudioSender(args.host, args.port, playback_path=args.playback_file)
    curses.wrapper(sender.run)
