use std::env;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait};
use rodio::source::{SineWave, Zero};
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::{mpsc, watch};

use crate::protocol::{AudioOutput, AudioStreamFormat, VoiceOutputCommand};

const START_SILENCE_MS: u64 = 50;

pub async fn run(mut rx: mpsc::Receiver<VoiceOutputCommand>, mut shutdown: watch::Receiver<bool>) {
    let (tx, thread_rx) = std_mpsc::channel();
    std::thread::spawn(move || output_loop(thread_rx));

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = tx.send(VoiceOutputCommand::Shutdown);
                break;
            }
            command = rx.recv() => {
                match command {
                    Some(command) => {
                        if tx.send(command).is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = tx.send(VoiceOutputCommand::Shutdown);
                        break;
                    }
                }
            }
        }
    }
}

fn output_loop(rx: std_mpsc::Receiver<VoiceOutputCommand>) {
    let (stream, handle) = match open_output_stream() {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("voice output failed to open device: {}", err);
            return;
        }
    };
    let _stream = stream;
    let mut current_sink: Option<Sink> = None;
    let mut current_stream: Option<StreamState> = None;

    while let Ok(command) = rx.recv() {
        match command {
            VoiceOutputCommand::PlayText { text } => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                play_beep(&handle, &mut current_sink);
                tracing::info!("voice output: {}", text);
            }
            VoiceOutputCommand::PlayAudioFile { path } => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                match play_audio_file(&handle, &path) {
                    Ok(sink) => {
                        current_sink = Some(sink);
                        tracing::info!("voice output audio file: {}", path);
                    }
                    Err(err) => {
                        tracing::warn!("voice output failed to play {}: {}", path, err);
                    }
                }
            }
            VoiceOutputCommand::PlayAudio { audio } => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                match play_audio(&handle, audio) {
                    Ok(sink) => {
                        current_sink = Some(sink);
                        tracing::info!("voice output: audio buffer");
                    }
                    Err(err) => {
                        tracing::warn!("voice output failed to play buffer: {}", err);
                    }
                }
            }
            VoiceOutputCommand::StartStream { format } => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                match start_stream(&handle, format) {
                    Ok(stream) => {
                        current_stream = Some(stream);
                        tracing::info!("voice output: audio stream started");
                    }
                    Err(err) => {
                        tracing::warn!("voice output failed to start stream: {}", err);
                    }
                }
            }
            VoiceOutputCommand::StreamChunk { data } => {
                if let Some(stream) = &mut current_stream {
                    if let Err(err) = stream.push(data) {
                        tracing::warn!("voice output stream error: {}", err);
                        stop_stream(&mut current_stream);
                    }
                }
            }
            VoiceOutputCommand::EndStream => {
                if let Some(stream) = current_stream.as_mut() {
                    stream.end();
                }
                tracing::info!("voice output: audio stream ended");
            }
            VoiceOutputCommand::Stop => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                tracing::info!("voice output stop");
            }
            VoiceOutputCommand::Shutdown => {
                stop_stream(&mut current_stream);
                stop_sink(&mut current_sink);
                break;
            }
        }
    }
}

fn play_audio_file(handle: &OutputStreamHandle, path: &str) -> Result<Sink, String> {
    let file = File::open(path).map_err(|err| format!("open failed: {}", err))?;
    let reader = BufReader::new(file);
    let decoder = Decoder::new(reader).map_err(|err| format!("decode failed: {}", err))?;
    let sink = Sink::try_new(handle).map_err(|err| format!("sink failed: {}", err))?;
    sink.append(decoder);
    sink.play();
    Ok(sink)
}

fn play_audio(handle: &OutputStreamHandle, audio: AudioOutput) -> Result<Sink, String> {
    match audio {
        AudioOutput::Pcm {
            mut data,
            sample_rate,
            channels,
        } => {
            let aligned_len = data.len() - (data.len() % 2);
            data.truncate(aligned_len);
            if data.is_empty() {
                return Err("pcm buffer is empty".to_string());
            }
            let samples: Vec<i16> = bytemuck::cast_slice(&data).to_vec();
            let source = SamplesBuffer::new(channels, sample_rate, samples);
            let sink = Sink::try_new(handle).map_err(|err| format!("sink failed: {}", err))?;
            sink.append(source);
            sink.play();
            Ok(sink)
        }
        AudioOutput::Mp3 { data } => {
            if data.is_empty() {
                return Err("mp3 buffer is empty".to_string());
            }
            let reader = BufReader::new(Cursor::new(data));
            let decoder = Decoder::new(reader).map_err(|err| format!("decode failed: {}", err))?;
            let sample_rate = decoder.sample_rate();
            let channels = decoder.channels();
            tracing::info!(
                "mp3 decoded sample_rate: {}, channels: {}",
                sample_rate,
                channels
            );
            let samples: Vec<i16> = decoder.collect();
            let stereo_samples = mono_then_stereo(samples, channels)?;
            let source = SamplesBuffer::new(2, sample_rate, stereo_samples);
            let sink = Sink::try_new(handle).map_err(|err| format!("sink failed: {}", err))?;
            sink.append(source);
            sink.play();
            Ok(sink)
        }
    }
}

enum StreamMessage {
    Data(Vec<u8>),
    End,
}

enum StreamState {
    Pcm {
        sink: Sink,
        sample_rate: u32,
        channels: u16,
        /// Accumulates early chunks until `min_bytes` is reached to avoid underflow.
        pending: Vec<u8>,
        /// Minimum buffered bytes before pushing PCM to the sink.
        min_bytes: usize,
        timings: Arc<Mutex<StreamTimings>>,
    },
    Mp3 {
        tx: std_mpsc::Sender<StreamMessage>,
        stop: Arc<AtomicBool>,
        /// Accumulates early chunks until `min_bytes` is reached to prime the decoder.
        pending: Vec<u8>,
        /// Minimum buffered bytes before sending MP3 data into the decoder.
        min_bytes: usize,
        timings: Arc<Mutex<StreamTimings>>,
    },
}

impl StreamState {
    fn push(&mut self, data: Vec<u8>) -> Result<(), String> {
        mark_chunk(&self.timings(), data.len())?;
        match self {
            StreamState::Pcm {
                sink,
                sample_rate,
                channels,
                pending,
                min_bytes,
                ..
            } => push_pcm_buffered(
                sink,
                pending,
                data,
                *min_bytes,
                *sample_rate,
                *channels,
            ),
            StreamState::Mp3 {
                tx,
                pending,
                min_bytes,
                ..
            } => push_mp3_buffered(tx, pending, data, *min_bytes),
        }
    }

    fn stop(self) {
        match self {
            StreamState::Pcm { sink, timings, .. } => {
                sink.stop();
                log_total_playback("pcm", timings, true);
            }
            StreamState::Mp3 { tx, stop, timings, .. } => {
                stop.store(true, Ordering::SeqCst);
                let _ = tx.send(StreamMessage::End);
                log_total_playback("mp3", timings, true);
            }
        }
    }

    fn end(&mut self) {
        match self {
            StreamState::Pcm {
                sink,
                sample_rate,
                channels,
                pending,
                timings,
                ..
            } => {
                if !pending.is_empty() {
                    let chunk = std::mem::take(pending);
                    let _ = push_pcm_chunk(sink, chunk, *sample_rate, *channels);
                }
                log_total_playback("pcm", Arc::clone(timings), false);
            }
            StreamState::Mp3 {
                tx, pending, ..
            } => {
                if !pending.is_empty() {
                    let _ = tx.send(StreamMessage::Data(std::mem::take(pending)));
                }
                let _ = tx.send(StreamMessage::End);
            }
        }
    }

    fn timings(&self) -> Arc<Mutex<StreamTimings>> {
        match self {
            StreamState::Pcm { timings, .. } => Arc::clone(timings),
            StreamState::Mp3 { timings, .. } => Arc::clone(timings),
        }
    }
}

fn start_stream(
    handle: &OutputStreamHandle,
    format: AudioStreamFormat,
) -> Result<StreamState, String> {
    match format {
        AudioStreamFormat::Pcm {
            sample_rate,
            channels,
        } => {
            let sink = Sink::try_new(handle).map_err(|err| format!("sink failed: {}", err))?;
            sink.play();
            let min_bytes = min_pcm_chunk_bytes(sample_rate, channels);
            let pending = silence_pcm_bytes(sample_rate, channels, START_SILENCE_MS);
            Ok(StreamState::Pcm {
                sink,
                sample_rate,
                channels,
                pending,
                min_bytes,
                timings: Arc::new(Mutex::new(StreamTimings::new(Some((
                    sample_rate, channels,
                ))))),
            })
        }
        AudioStreamFormat::Mp3 => {
            let (tx, rx) = std_mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let timings = Arc::new(Mutex::new(StreamTimings::new(None)));
            let thread_handle = handle.clone();
            let thread_stop = Arc::clone(&stop);
            let thread_timings = Arc::clone(&timings);
            std::thread::spawn(move || run_mp3_stream(thread_handle, rx, thread_stop, thread_timings));
            Ok(StreamState::Mp3 {
                tx,
                stop,
                pending: Vec::new(),
                min_bytes: min_mp3_chunk_bytes(),
                timings,
            })
        }
    }
}

fn min_pcm_chunk_bytes(sample_rate: u32, channels: u16) -> usize {
    let ms = 40u64;
    let bytes_per_sample = 2u64;
    let channels = channels.max(1) as u64;
    let sample_rate = sample_rate.max(1) as u64;
    let bytes_per_ms = sample_rate * channels * bytes_per_sample / 1000;
    let min = bytes_per_ms.saturating_mul(ms).max(bytes_per_sample * channels);
    min as usize
}

fn min_mp3_chunk_bytes() -> usize {
    4096
}

fn silence_pcm_bytes(sample_rate: u32, channels: u16, ms: u64) -> Vec<u8> {
    let bytes_per_sample = 2u64;
    let channels = channels.max(1) as u64;
    let sample_rate = sample_rate.max(1) as u64;
    let bytes_per_ms = sample_rate * channels * bytes_per_sample / 1000;
    let len = bytes_per_ms.saturating_mul(ms);
    vec![0u8; len as usize]
}

fn push_pcm_buffered(
    sink: &Sink,
    pending: &mut Vec<u8>,
    data: Vec<u8>,
    min_bytes: usize,
    sample_rate: u32,
    channels: u16,
) -> Result<(), String> {
    if !data.is_empty() {
        pending.extend_from_slice(&data);
    }
    if pending.len() < min_bytes {
        return Ok(());
    }
    let frame_bytes = 2usize.saturating_mul(channels.max(1) as usize);
    if frame_bytes == 0 {
        return Ok(());
    }
    let aligned_len = pending.len() - (pending.len() % frame_bytes);
    if aligned_len == 0 {
        return Ok(());
    }
    let chunk: Vec<u8> = pending.drain(..aligned_len).collect();
    push_pcm_chunk(sink, chunk, sample_rate, channels)
}

fn push_mp3_buffered(
    tx: &std_mpsc::Sender<StreamMessage>,
    pending: &mut Vec<u8>,
    data: Vec<u8>,
    min_bytes: usize,
) -> Result<(), String> {
    if !data.is_empty() {
        pending.extend_from_slice(&data);
    }
    if pending.len() < min_bytes {
        return Ok(());
    }
    let chunk = std::mem::take(pending);
    tx.send(StreamMessage::Data(chunk))
        .map_err(|_| "mp3 stream closed".to_string())
}

fn push_pcm_chunk(
    sink: &Sink,
    mut data: Vec<u8>,
    sample_rate: u32,
    channels: u16,
) -> Result<(), String> {
    let aligned_len = data.len() - (data.len() % 2);
    data.truncate(aligned_len);
    if data.is_empty() {
        return Ok(());
    }
    let samples: Vec<i16> = bytemuck::cast_slice(&data).to_vec();
    let source = SamplesBuffer::new(channels, sample_rate, samples);
    sink.append(source);
    Ok(())
}

struct StreamingReader {
    rx: Arc<Mutex<std_mpsc::Receiver<StreamMessage>>>,
    buffer: Vec<u8>,
    pos: usize,
    ended: bool,
}

impl StreamingReader {
    fn new(rx: std_mpsc::Receiver<StreamMessage>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
            buffer: Vec::new(),
            pos: 0,
            ended: false,
        }
    }
}

impl Read for StreamingReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buffer.len() && !self.ended {
            let message = {
                let rx = self
                    .rx
                    .lock()
                    .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "rx poisoned"))?;
                rx.recv()
            };
            match message {
                Ok(StreamMessage::Data(data)) => {
                    if !data.is_empty() {
                        self.buffer.extend_from_slice(&data);
                    }
                }
                Ok(StreamMessage::End) | Err(_) => {
                    self.ended = true;
                }
            }
        }

        if self.pos >= self.buffer.len() {
            return Ok(0);
        }

        let available = self.buffer.len() - self.pos;
        let to_copy = available.min(out.len());
        out[..to_copy].copy_from_slice(&self.buffer[self.pos..self.pos + to_copy]);
        self.pos += to_copy;
        Ok(to_copy)
    }
}

impl Seek for StreamingReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let len = if self.ended {
            self.buffer.len()
        } else {
            self.buffer.len()
        };
        let next = match pos {
            SeekFrom::Start(value) => value as i64,
            SeekFrom::Current(offset) => self.pos as i64 + offset,
            SeekFrom::End(offset) => {
                if !self.ended {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "stream end unknown",
                    ));
                }
                len as i64 + offset
            }
        };

        if next < 0 || next as usize > len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek",
            ));
        }
        self.pos = next as usize;
        Ok(self.pos as u64)
    }
}

fn run_mp3_stream(
    handle: OutputStreamHandle,
    rx: std_mpsc::Receiver<StreamMessage>,
    stop: Arc<AtomicBool>,
    timings: Arc<Mutex<StreamTimings>>,
) {
    let reader = StreamingReader::new(rx);
    let reader = BufReader::new(reader);
    let decoder = match Decoder::new(reader) {
        Ok(decoder) => decoder,
        Err(err) => {
            tracing::warn!("mp3 decode failed: {}", err);
            return;
        }
    };
    let sink = match Sink::try_new(&handle) {
        Ok(sink) => sink,
        Err(err) => {
            tracing::warn!("mp3 sink failed: {}", err);
            return;
        }
    };
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    if sample_rate > 0 && channels > 0 {
        let silence = Zero::<f32>::new(channels, sample_rate)
            .take_duration(Duration::from_millis(START_SILENCE_MS));
        sink.append(silence);
    }
    sink.append(decoder);
    sink.play();
    loop {
        if stop.load(Ordering::SeqCst) {
            sink.stop();
            log_total_playback("mp3", timings, true);
            break;
        }
        if sink.empty() {
            log_total_playback("mp3", timings, false);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct StreamTimings {
    started_at: Instant,
    first_chunk_at: Option<Instant>,
    chunk_count: u64,
    total_bytes: u64,
    pcm_info: Option<(u32, u16)>,
    total_logged: bool,
}

impl StreamTimings {
    fn new(pcm_info: Option<(u32, u16)>) -> Self {
        Self {
            started_at: Instant::now(),
            first_chunk_at: None,
            chunk_count: 0,
            total_bytes: 0,
            pcm_info,
            total_logged: false,
        }
    }
}

fn mark_chunk(timings: &Arc<Mutex<StreamTimings>>, bytes: usize) -> Result<(), String> {
    let now = Instant::now();
    let mut guard = timings
        .lock()
        .map_err(|_| "stream timings lock poisoned".to_string())?;
    guard.chunk_count += 1;
    guard.total_bytes = guard.total_bytes.saturating_add(bytes as u64);
    if guard.first_chunk_at.is_none() {
        guard.first_chunk_at = Some(now);
        let wait = now.duration_since(guard.started_at);
        tracing::info!(
            "voice output first audio chunk after {:.0}ms ({} bytes)",
            wait.as_secs_f64() * 1000.0,
            bytes
        );
    } else {
        tracing::debug!(
            "voice output audio chunk {} ({} bytes)",
            guard.chunk_count,
            bytes
        );
    }
    Ok(())
}

fn log_total_playback(kind: &str, timings: Arc<Mutex<StreamTimings>>, stopped: bool) {
    let mut guard = match timings.lock() {
        Ok(guard) => guard,
        Err(_) => {
            tracing::warn!("voice output {} timing lock poisoned", kind);
            return;
        }
    };

    if guard.total_logged && !stopped {
        return;
    }

    let (total_secs, label) = if let Some((sample_rate, channels)) = guard.pcm_info {
        let bytes_per_sample = 2u64;
        let channels = channels.max(1) as u64;
        let sample_rate = sample_rate.max(1) as u64;
        let total_samples = guard.total_bytes / (bytes_per_sample * channels);
        let total_secs = total_samples as f64 / sample_rate as f64;
        (total_secs, "estimated")
    } else {
        let start = guard.first_chunk_at.unwrap_or(guard.started_at);
        (start.elapsed().as_secs_f64(), "measured")
    };

    guard.total_logged = true;
    if stopped {
        tracing::info!(
            "voice output {} stream stopped after {:.2}s ({})",
            kind,
            total_secs,
            label
        );
    } else {
        tracing::info!(
            "voice output {} stream finished after {:.2}s ({})",
            kind,
            total_secs,
            label
        );
    }
}

fn open_output_stream() -> Result<(OutputStream, OutputStreamHandle), String> {
    let host = cpal::default_host();
    let requested_device = env::var("PLAYBACK_DEVICE")
        .ok()
        .or_else(|| env::var("AUDIO_CARD").ok())
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    if let Some(name) = requested_device {
        let devices: Vec<cpal::Device> = host
            .output_devices()
            .map_err(|err| format!("failed to list output devices: {}", err))?
            .collect();
        let available: Vec<String> = devices
            .iter()
            .filter_map(|device| device.name().ok())
            .collect();
        tracing::info!("available output devices: {:?}", available);
        let device = devices
            .into_iter()
            .find(|device| device.name().map(|n| n.contains(&name)).unwrap_or(false))
            .ok_or_else(|| format!("output device '{}' not found", name))?;
        let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
        let stream = OutputStream::try_from_device(&device)
            .map_err(|err| format!("output device '{}' failed: {}", device_name, err))?;
        tracing::info!("using output device: '{}'", device_name);
        return Ok(stream);
    }

    OutputStream::try_default().map_err(|err| format!("default output device failed: {}", err))
}

fn mono_then_stereo(samples: Vec<i16>, channels: u16) -> Result<Vec<i16>, String> {
    if samples.is_empty() {
        return Err("mp3 decoded to empty buffer".to_string());
    }
    let channel_count = channels as usize;
    if channel_count == 0 {
        return Err("mp3 reported zero channels".to_string());
    }
    let aligned_len = samples.len() - (samples.len() % channel_count);
    if aligned_len == 0 {
        return Err("mp3 decoded buffer is empty".to_string());
    }
    let samples = &samples[..aligned_len];
    let frames = aligned_len / channel_count;
    let mut stereo = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let start = frame * channel_count;
        let end = start + channel_count;
        let sum: i32 = samples[start..end].iter().map(|sample| *sample as i32).sum();
        let mono = (sum / channel_count as i32) as i16;
        stereo.push(mono);
        stereo.push(mono);
    }
    Ok(stereo)
}

fn play_beep(handle: &OutputStreamHandle, current_sink: &mut Option<Sink>) {
    if let Ok(sink) = Sink::try_new(handle) {
        let source = SineWave::new(440.0)
            .take_duration(Duration::from_millis(250))
            .amplify(0.15);
        sink.append(source);
        sink.play();
        *current_sink = Some(sink);
    }
}

fn stop_sink(current_sink: &mut Option<Sink>) {
    if let Some(sink) = current_sink.take() {
        sink.stop();
    }
}

fn stop_stream(current_stream: &mut Option<StreamState>) {
    if let Some(stream) = current_stream.take() {
        stream.stop();
    }
}
