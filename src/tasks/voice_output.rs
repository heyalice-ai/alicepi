use std::env;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait};
use rodio::buffer::SamplesBuffer;
use rodio::mixer::Mixer;
use rodio::source::{SineWave, Zero};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use tokio::sync::{broadcast, mpsc, watch};

use crate::protocol::{AudioOutput, AudioStreamFormat, VoiceOutputCommand, VoiceOutputEvent};

const START_SILENCE_MS: u64 = 50;

pub async fn run(
    mut rx: mpsc::Receiver<VoiceOutputCommand>,
    events: broadcast::Sender<VoiceOutputEvent>,
    mut shutdown: watch::Receiver<bool>,
) {
    let playback_generation = Arc::new(AtomicU64::new(0));
    let (tx, thread_rx) = std_mpsc::channel();
    let thread_events = events.clone();
    let thread_generation = Arc::clone(&playback_generation);
    std::thread::spawn(move || output_loop(thread_rx, thread_events, thread_generation));

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

fn output_loop(
    rx: std_mpsc::Receiver<VoiceOutputCommand>,
    events: broadcast::Sender<VoiceOutputEvent>,
    playback_generation: Arc<AtomicU64>,
) {
    let stream = match open_output_stream() {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("voice output failed to open device: {}", err);
            return;
        }
    };
    let handle = stream.mixer();
    let mut current_sink: Option<Arc<Sink>> = None;
    let mut current_stream: Option<StreamState> = None;

    while let Ok(command) = rx.recv() {
        match command {
                VoiceOutputCommand::PlayText { text } => {
                    stop_stream(&mut current_stream);
                    stop_sink(&mut current_sink);
                    let generation = next_generation(&playback_generation);
                    play_beep(&handle, &mut current_sink);
                    if let Some(sink) = current_sink.as_ref() {
                        spawn_sink_finish_thread(
                            Arc::clone(sink),
                            events.clone(),
                            Arc::clone(&playback_generation),
                            generation,
                        );
                    }
                    tracing::info!("voice output: {}", text);
                }
                VoiceOutputCommand::PlayAudioFile { path } => {
                    stop_stream(&mut current_stream);
                    stop_sink(&mut current_sink);
                    let generation = next_generation(&playback_generation);
                    match play_audio_file(&handle, &path) {
                        Ok(sink) => {
                            let sink = Arc::new(sink);
                            spawn_sink_finish_thread(
                                Arc::clone(&sink),
                                events.clone(),
                                Arc::clone(&playback_generation),
                                generation,
                            );
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
                    let generation = next_generation(&playback_generation);
                    match play_audio(&handle, audio) {
                        Ok(sink) => {
                            let sink = Arc::new(sink);
                            spawn_sink_finish_thread(
                                Arc::clone(&sink),
                                events.clone(),
                                Arc::clone(&playback_generation),
                                generation,
                            );
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
                    let generation = next_generation(&playback_generation);
                    match start_stream(
                        &handle,
                        format,
                        events.clone(),
                        Arc::clone(&playback_generation),
                        generation,
                    ) {
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
                    next_generation(&playback_generation);
                    tracing::info!("voice output stop");
                }
                VoiceOutputCommand::Shutdown => {
                    stop_stream(&mut current_stream);
                    stop_sink(&mut current_sink);
                    next_generation(&playback_generation);
                    break;
                }
        }
    }
}

fn next_generation(playback_generation: &Arc<AtomicU64>) -> u64 {
    playback_generation.fetch_add(1, Ordering::SeqCst) + 1
}

fn spawn_sink_finish_thread(
    sink: Arc<Sink>,
    events: broadcast::Sender<VoiceOutputEvent>,
    playback_generation: Arc<AtomicU64>,
    generation: u64,
) {
    std::thread::spawn(move || {
        sink.sleep_until_end();
        if playback_generation.load(Ordering::SeqCst) == generation {
            let _ = events.send(VoiceOutputEvent::Finished);
        }
    });
}
fn play_audio_file(handle: &Mixer, path: &str) -> Result<Sink, String> {
    let file = File::open(path).map_err(|err| format!("open failed: {}", err))?;
    let reader = BufReader::new(file);
    let decoder = Decoder::new(reader).map_err(|err| format!("decode failed: {}", err))?;
    let sink = Sink::connect_new(handle);
    sink.append(decoder);
    sink.play();
    Ok(sink)
}

fn play_audio(handle: &Mixer, audio: AudioOutput) -> Result<Sink, String> {
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
            let samples: Vec<f32> = bytemuck::cast_slice(&data)
                .iter()
                .map(|&s: &f32| s as f32 / 32768.0)
                .collect();
            let source = SamplesBuffer::new(channels, sample_rate, samples);
            let sink = Sink::connect_new(handle);
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
            let stereo_samples: Vec<f32> = mono_then_stereo(decoder, channels)?.collect();
            let source = SamplesBuffer::new(2, sample_rate, stereo_samples);
            let sink = Sink::connect_new(handle);
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
        sink: Arc<Sink>,
        sample_rate: u32,
        channels: u16,
        /// Accumulates early chunks until `min_bytes` is reached to avoid underflow.
        pending: Vec<u8>,
        /// Minimum buffered bytes before pushing PCM to the sink.
        min_bytes: usize,
        timings: Arc<Mutex<StreamTimings>>,
        events: broadcast::Sender<VoiceOutputEvent>,
        playback_generation: Arc<AtomicU64>,
        generation: u64,
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
                sink.as_ref(),
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
            StreamState::Mp3 {
                tx, stop, timings, ..
            } => {
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
                events,
                playback_generation,
                generation,
                ..
            } => {
                if !pending.is_empty() {
                    let chunk = std::mem::take(pending);
                    let _ = push_pcm_chunk(sink.as_ref(), chunk, *sample_rate, *channels);
                }
                log_total_playback("pcm", Arc::clone(timings), false);
                spawn_sink_finish_thread(
                    Arc::clone(sink),
                    events.clone(),
                    Arc::clone(playback_generation),
                    *generation,
                );
            }
            StreamState::Mp3 { tx, pending, .. } => {
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
    handle: &Mixer,
    format: AudioStreamFormat,
    events: broadcast::Sender<VoiceOutputEvent>,
    playback_generation: Arc<AtomicU64>,
    generation: u64,
) -> Result<StreamState, String> {
    match format {
        AudioStreamFormat::Pcm {
            sample_rate,
            channels,
        } => {
            let sink = Arc::new(Sink::connect_new(handle));
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
                    sample_rate,
                    channels,
                ))))),
                events,
                playback_generation,
                generation,
            })
        }
        AudioStreamFormat::Mp3 => {
            let (tx, rx) = std_mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let timings = Arc::new(Mutex::new(StreamTimings::new(None)));
            let thread_handle = handle.clone();
            let thread_stop = Arc::clone(&stop);
            let thread_timings = Arc::clone(&timings);
            let thread_events = events.clone();
            let thread_generation = Arc::clone(&playback_generation);
            std::thread::spawn(move || {
                run_mp3_stream(
                    &thread_handle,
                    rx,
                    thread_stop,
                    thread_timings,
                    thread_events,
                    thread_generation,
                    generation,
                )
            });
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
    let min = bytes_per_ms
        .saturating_mul(ms)
        .max(bytes_per_sample * channels);
    min as usize
}

fn min_mp3_chunk_bytes() -> usize {
    4
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
    let samples: Vec<f32> = bytemuck::cast_slice(&data)
        .iter()
        .map(|&s: &f32| s as f32 / 32768.0)
        .collect();
    let source = SamplesBuffer::new(channels, sample_rate, samples);
    sink.append(source);
    Ok(())
}

struct Mp3StreamReader {
    rx: Arc<Mutex<std_mpsc::Receiver<StreamMessage>>>,
    cursor: Cursor<Vec<u8>>,
    ended: bool,
}

impl Mp3StreamReader {
    fn new(
        rx: Arc<Mutex<std_mpsc::Receiver<StreamMessage>>>,
        buffer: Vec<u8>,
        ended: bool,
    ) -> Self {
        Self {
            rx,
            cursor: Cursor::new(buffer),
            ended,
        }
    }
}

impl Read for Mp3StreamReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.cursor.position() as usize >= self.cursor.get_ref().len() && !self.ended {
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
                        self.cursor.get_mut().extend_from_slice(&data);
                    }
                }
                Ok(StreamMessage::End) | Err(_) => {
                    self.ended = true;
                }
            }
        }

        let pos = self.cursor.position() as usize;
        let len = self.cursor.get_ref().len();
        if pos >= len {
            return Ok(0);
        }

        let available = len - pos;
        if available < out.len() && !self.ended {
            tracing::debug!(
                "streaming reader underflow: requested {}, available {} (continues)",
                out.len(),
                available
            );
        }

        let to_copy = available.min(out.len());
        out[..to_copy].copy_from_slice(&self.cursor.get_ref()[pos..pos + to_copy]);
        self.cursor.set_position((pos + to_copy) as u64);
        Ok(to_copy)
    }
}

impl Seek for Mp3StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let len = self.cursor.get_ref().len();
        let current = self.cursor.position() as i64;
        let next = match pos {
            SeekFrom::Start(value) => value as i64,
            SeekFrom::Current(offset) => current + offset,
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
        self.cursor.set_position(next as u64);
        Ok(self.cursor.position())
    }
}

fn run_mp3_stream(
    handle: &Mixer,
    rx: std_mpsc::Receiver<StreamMessage>,
    stop: Arc<AtomicBool>,
    timings: Arc<Mutex<StreamTimings>>,
    events: broadcast::Sender<VoiceOutputEvent>,
    playback_generation: Arc<AtomicU64>,
    generation: u64,
) {
    let rx = Arc::new(Mutex::new(rx));
    let reader = Mp3StreamReader::new(rx, vec![], false);
    let reader = BufReader::new(reader);
    let decoder = match Decoder::new(reader) {
        Ok(decoder) => decoder,
        Err(err) => {
            tracing::warn!("mp3 decode failed: {}", err);
            return;
        }
    };
    let sink = Sink::connect_new(&handle);
    sink.play();

    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    if sample_rate > 0 && channels > 0 {
        let silence = Zero::new(channels, sample_rate)
            .take_duration(Duration::from_millis(START_SILENCE_MS));
        sink.append(silence);
    }
    tracing::info!(
        "mp3 decoded sample_rate: {}, channels: {}",
        sample_rate,
        channels
    );
    let stereo_iter = match mono_then_stereo(decoder, channels) {
        Ok(iter) => iter,
        Err(err) => {
            tracing::warn!("mp3 stereo conversion failed: {}", err);
            return;
        }
    };

    sink.append(stereo_iter);

    loop {
        if stop.load(Ordering::SeqCst) {
            sink.stop();
            log_total_playback("mp3", timings, true);
            break;
        }
        if sink.empty() {
            log_total_playback("mp3", timings, false);
            if playback_generation.load(Ordering::SeqCst) == generation {
                let _ = events.send(VoiceOutputEvent::Finished);
            }
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

fn open_output_stream() -> Result<OutputStream, String> {
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
        
        let stream = OutputStreamBuilder::from_device(device)
            .map_err(|err| format!("output device '{}' failed: {}", device_name, err))?;
        tracing::info!("using output device: '{}'", device_name);
        return stream.open_stream().map_err(|err| format!("output stream failed: {}", err));
    }

    OutputStreamBuilder::from_default_device().map_err(|err| format!("default output device failed: {}", err))?.open_stream().map_err(|err| format!("output stream failed: {}", err))
}

fn mono_then_stereo<I>(
    mut samples: rodio::Decoder<I>,
    channels: u16,
) -> Result<MonoThenStereo<I>, String>
where
    I: Read + Seek,
{
    let channel_count = channels as usize;
    if channel_count == 0 {
        return Err("mp3 reported zero channels".to_string());
    }
    let mut frame = Vec::with_capacity(channel_count);
    while frame.len() < channel_count {
        match samples.next() {
            Some(sample) => frame.push(sample),
            None => {
                if frame.is_empty() {
                    return Err("mp3 decoded to empty buffer".to_string());
                }
                return Err("mp3 decoded buffer is empty".to_string());
            }
        }
    }

    let mono = mono_from_frame(&frame, channel_count);
    Ok(MonoThenStereo::new(samples, channel_count, mono))
}

struct MonoThenStereo<I: Read + Seek> {
    iter: rodio::Decoder<I>,
    channel_count: usize,
    frame: Vec<f32>,
    pending: [f32; 2],
    pending_idx: usize,
}

impl<I> MonoThenStereo<I>
where
    I: Read + Seek,
{
    fn new(iter: rodio::Decoder<I>, channel_count: usize, mono: f32) -> Self {
        Self {
            iter,
            channel_count,
            frame: Vec::with_capacity(channel_count),
            pending: [mono, mono],
            pending_idx: 0,
        }
    }

    fn next_frame_mono(&mut self) -> Option<f32> {
        self.frame.clear();
        while self.frame.len() < self.channel_count {
            match self.iter.next() {
                Some(sample) => self.frame.push(sample),
                None => return None,
            }
        }
        Some(mono_from_frame(&self.frame, self.channel_count))
    }
}

impl<I> Iterator for MonoThenStereo<I>
where
    I: Read + Seek,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pending_idx < self.pending.len() {
            let sample = self.pending[self.pending_idx];
            self.pending_idx += 1;
            return Some(sample);
        }

        let mono = self.next_frame_mono()?;
        self.pending = [mono, mono];
        self.pending_idx = 1;
        Some(mono)
    }
}

impl<I> Source for MonoThenStereo<I>
where
    I: Read + Seek + Send + 'static,
{
    fn current_span_len(&self) -> Option<usize> {
        self.iter.current_span_len()
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        self.iter.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.iter.total_duration()
    }
}

fn mono_from_frame(frame: &[f32], channel_count: usize) -> f32 {
    let sum: f32 = frame.iter().sum();
    sum / channel_count as f32
}

fn play_beep(handle: &Mixer, current_sink: &mut Option<Arc<Sink>>) {
    let sink = Arc::new(Sink::connect_new(handle));
    let source = SineWave::new(440.0)
        .take_duration(Duration::from_millis(250))
        .amplify(0.15);
    sink.append(source);
    sink.play();
    *current_sink = Some(sink);
}

fn stop_sink(current_sink: &mut Option<Arc<Sink>>) {
    if let Some(sink) = current_sink.take() {
        sink.stop();
    }
}

fn stop_stream(current_stream: &mut Option<StreamState>) {
    if let Some(stream) = current_stream.take() {
        stream.stop();
    }
}
