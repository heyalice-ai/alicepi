use std::env;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use bytemuck::cast_slice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::fs;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time;

use crate::protocol::{VoiceInputCommand, VoiceInputEvent};
use crate::watchdog::Heartbeat;

#[derive(Debug, Clone)]
struct VoiceInputConfig {
    stream_sample_rate: u32,
    stream_channels: usize,
    chunk_size: usize,
    capture_device: Option<String>,
    mock_file: Option<String>,
}

impl VoiceInputConfig {
    fn from_env() -> Self {
        let stream_sample_rate = env_u32("STREAM_SAMPLE_RATE", 48_000);
        let stream_channels = env_usize("STREAM_CHANNELS", 2);
        let chunk_size = env_usize("CHUNK_SIZE", 512);
        let capture_device = env::var("CAPTURE_DEVICE")
            .ok()
            .or_else(|| env::var("AUDIO_CARD").ok());
        let mock_file = env::var("MOCK_AUDIO_FILE").ok();

        Self {
            stream_sample_rate,
            stream_channels,
            chunk_size,
            capture_device,
            mock_file,
        }
    }
}

struct CaptureStream {
    receiver: mpsc::Receiver<Vec<f32>>,
    sample_rate: u32,
    channels: usize,
    shutdown: Option<std_mpsc::Sender<()>>,
}

impl CaptureStream {
    async fn next(&mut self) -> Option<Vec<f32>> {
        self.receiver.recv().await
    }
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

struct AudioPipeline {
    target_rate: u32,
    target_channels: usize,
    chunk_size: usize,
    pending: Vec<f32>,
    resampler: Option<LinearResampler>,
}

impl AudioPipeline {
    fn new(input_rate: u32, _input_channels: usize, target_rate: u32, target_channels: usize, chunk_size: usize) -> Self {
        let resampler = if input_rate != target_rate {
            Some(LinearResampler::new(input_rate, target_rate, target_channels))
        } else {
            None
        };
        Self {
            target_rate,
            target_channels,
            chunk_size,
            pending: Vec::new(),
            resampler,
        }
    }

    fn push_samples(&mut self, input: &[f32], input_channels: usize) -> Vec<Vec<i16>> {
        if input.is_empty() {
            return Vec::new();
        }

        let mut output = convert_channels(input, input_channels, self.target_channels);
        if let Some(resampler) = &mut self.resampler {
            output = resampler.process(&output);
        }

        self.pending.extend_from_slice(&output);
        let mut chunks = Vec::new();
        let frame_size = self.target_channels;
        let target_samples = self.chunk_size * frame_size;

        while self.pending.len() >= target_samples {
            let chunk: Vec<f32> = self.pending.drain(..target_samples).collect();
            chunks.push(f32_to_i16(&chunk));
        }

        chunks
    }

    fn finish(&mut self) -> Option<Vec<i16>> {
        if self.pending.is_empty() {
            return None;
        }
        let leftover = std::mem::take(&mut self.pending);
        Some(f32_to_i16(&leftover))
    }
}

struct LinearResampler {
    input_rate: u32,
    output_rate: u32,
    channels: usize,
    pos: f32,
    carry: Vec<f32>,
}

impl LinearResampler {
    fn new(input_rate: u32, output_rate: u32, channels: usize) -> Self {
        Self {
            input_rate,
            output_rate,
            channels,
            pos: 0.0,
            carry: Vec::new(),
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }

        let mut combined = Vec::with_capacity(self.carry.len() + input.len());
        combined.extend_from_slice(&self.carry);
        combined.extend_from_slice(input);

        let total_frames = combined.len() / self.channels;
        if total_frames < 2 {
            self.carry = combined;
            return Vec::new();
        }

        let step = self.input_rate as f32 / self.output_rate as f32;
        let mut output = Vec::new();
        let mut pos = self.pos;

        while pos + 1.0 < total_frames as f32 {
            let base = pos.floor() as usize;
            let frac = pos - base as f32;
            let idx0 = base * self.channels;
            let idx1 = (base + 1) * self.channels;
            for ch in 0..self.channels {
                let s0 = combined[idx0 + ch];
                let s1 = combined[idx1 + ch];
                output.push(s0 + (s1 - s0) * frac);
            }
            pos += step;
        }

        let keep_frame = pos.floor() as usize;
        let keep_index = keep_frame * self.channels;
        self.carry = combined[keep_index..].to_vec();
        self.pos = pos - keep_frame as f32;
        output
    }
}

fn convert_channels(input: &[f32], input_channels: usize, output_channels: usize) -> Vec<f32> {
    if input_channels == output_channels {
        return input.to_vec();
    }

    let frames = input.len() / input_channels;
    let mut output = Vec::with_capacity(frames * output_channels);

    for frame in 0..frames {
        let start = frame * input_channels;
        let frame_slice = &input[start..start + input_channels];
        if output_channels == 1 {
            let sum: f32 = frame_slice.iter().copied().sum();
            output.push(sum / input_channels as f32);
        } else if output_channels == 2 {
            match input_channels {
                1 => {
                    output.push(frame_slice[0]);
                    output.push(frame_slice[0]);
                }
                _ => {
                    output.push(frame_slice[0]);
                    output.push(frame_slice[1]);
                }
            }
        } else {
            output.extend_from_slice(&frame_slice[..output_channels.min(input_channels)]);
        }
    }

    output
}

fn f32_to_i16(input: &[f32]) -> Vec<i16> {
    input
        .iter()
        .map(|sample| {
            let scaled = (sample * i16::MAX as f32).round();
            scaled.clamp(i16::MIN as f32, i16::MAX as f32) as i16
        })
        .collect()
}

pub async fn run(
    mut rx: mpsc::Receiver<VoiceInputCommand>,
    events: broadcast::Sender<VoiceInputEvent>,
    heartbeat: Heartbeat,
    mut shutdown: watch::Receiver<bool>,
) {
    let config = VoiceInputConfig::from_env();
    let (mut capture, mut pipeline) = match start_capture(&config) {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("voice input failed to start capture: {}", err);
            return;
        }
    };

    let mut listening = false;
    let mut tick = time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if listening {
                    flush_audio(&mut pipeline, &events);
                }
                break;
            }
            _ = tick.tick() => {
                heartbeat.tick();
            }
            command = rx.recv() => {
                match command {
                    Some(VoiceInputCommand::StartListening) => {
                        listening = true;
                        pipeline.pending.clear();
                    }
                    Some(VoiceInputCommand::StopListening) => {
                        if listening {
                            flush_audio(&mut pipeline, &events);
                        }
                        listening = false;
                        pipeline.pending.clear();
                    }
                    Some(VoiceInputCommand::InjectAudioFile { path }) => {
                        if listening {
                            if let Err(err) = inject_audio_file(&config, &events, &path).await {
                                tracing::warn!("voice input inject failed: {}", err);
                            }
                        }
                    }
                    Some(VoiceInputCommand::Shutdown) | None => {
                        if listening {
                            flush_audio(&mut pipeline, &events);
                        }
                        break;
                    }
                }
            }
            samples = capture.next() => {
                if let Some(samples) = samples {
                    if listening {
                        let chunks = pipeline.push_samples(&samples, capture.channels);
                        for chunk in chunks {
                            send_chunk(&events, &chunk);
                        }
                    }
                } else {
                    if listening {
                        flush_audio(&mut pipeline, &events);
                    }
                    break;
                }
            }
        }
    }
}

fn start_capture(
    config: &VoiceInputConfig,
) -> Result<(CaptureStream, AudioPipeline), String> {
    if let Some(mock_file) = &config.mock_file {
        let (tx, rx) = mpsc::channel(8);
        let path = mock_file.clone();
        let chunk_frames = config.chunk_size;
        tokio::spawn(async move {
            if let Err(err) = stream_mock_audio(&path, chunk_frames, tx).await {
                tracing::warn!("mock audio stream error: {}", err);
            }
        });
        let reader = hound::WavReader::open(mock_file)
            .map_err(|err| format!("failed to open mock wav: {}", err))?;
        let spec = reader.spec();
        let pipeline = AudioPipeline::new(
            spec.sample_rate,
            spec.channels as usize,
            config.stream_sample_rate,
            config.stream_channels,
            config.chunk_size,
        );
        Ok((
            CaptureStream {
                receiver: rx,
                sample_rate: spec.sample_rate,
                channels: spec.channels as usize,
                shutdown: None,
            },
            pipeline,
        ))
    } else {
        let (capture, pipeline) = start_live_capture(config)?;
        Ok((capture, pipeline))
    }
}

fn start_live_capture(
    config: &VoiceInputConfig,
) -> Result<(CaptureStream, AudioPipeline), String> {
    let (tx, rx) = mpsc::channel(8);
    let (info_tx, info_rx) = std_mpsc::channel();
    let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
    let thread_config = config.clone();

    std::thread::spawn(move || {
        match build_input_stream(&thread_config, tx) {
            Ok((stream, info)) => {
                if let Err(err) = stream.play() {
                    let _ = info_tx.send(Err(format!("failed to start input stream: {}", err)));
                    return;
                }
                let _ = info_tx.send(Ok(info));
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                drop(stream);
            }
            Err(err) => {
                let _ = info_tx.send(Err(err));
            }
        }
    });

    let info = info_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "timed out starting input stream".to_string())??;

    let pipeline = AudioPipeline::new(
        info.sample_rate,
        info.channels,
        config.stream_sample_rate,
        config.stream_channels,
        config.chunk_size,
    );

    Ok((
        CaptureStream {
            receiver: rx,
            sample_rate: info.sample_rate,
            channels: info.channels,
            shutdown: Some(shutdown_tx),
        },
        pipeline,
    ))
}

struct CaptureInfo {
    sample_rate: u32,
    channels: usize,
}

fn build_input_stream(
    config: &VoiceInputConfig,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<(cpal::Stream, CaptureInfo), String> {
    let host = cpal::default_host();

    let available_devices = host
        .input_devices()
        .map_err(|err| format!("failed to list input devices: {}", err))?
        .map(|d| d.name().unwrap_or("unknown".to_string()))
        .collect::<Vec<_>>();
    tracing::info!("available input devices: {:?}", available_devices);
    
    let device = match &config.capture_device {
        Some(name) => host
            .input_devices()
            .map_err(|err| format!("failed to list input devices: {}", err))?
            .find(|device| device.name().map(|n| n.contains(name)).unwrap_or(false))
            .ok_or_else(|| format!("input device '{}' not found.", name))?,
        None => host
            .default_input_device()
            .ok_or_else(|| "no default input device available".to_string())?,
    };

    let default_config = device
        .default_input_config()
        .map_err(|err| format!("failed to get default input config: {}", err))?;

    let input_config = pick_input_config(&device, config.stream_sample_rate)
        .unwrap_or_else(|| default_config.clone());
    let sample_format = input_config.sample_format();
    let stream_config: cpal::StreamConfig = input_config.into();
    let channels = stream_config.channels as usize;
    let sample_rate = stream_config.sample_rate.0;

    // Print chosen device and config
    tracing::info!(
        "using input device: '{}' with config: {:?}",
        device.name().unwrap_or("unknown".to_string()),
        stream_config
    );

    let err_fn = |err| tracing::warn!("audio capture error: {}", err);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let _ = tx.try_send(data.to_vec());
                },
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build input stream: {}", err))?,
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> =
                        data.iter().map(|sample| *sample as f32 / i16::MAX as f32).collect();
                    let _ = tx.try_send(converted);
                },
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build input stream: {}", err))?,
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect();
                    let _ = tx.try_send(converted);
                },
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build input stream: {}", err))?,
        cpal::SampleFormat::I32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[i32], _| {
                    let converted: Vec<f32> =
                        data.iter().map(|sample| *sample as f32 / i32::MAX as f32).collect();
                    let _ = tx.try_send(converted);
                },
                err_fn,
                None,
            )
            .map_err(|err| format!("failed to build input stream: {}", err))?,
        _ => {
            return Err(format!("unsupported input sample format {:?}", sample_format));
        }
    };

    Ok((
        stream,
        CaptureInfo {
            sample_rate,
            channels,
        },
    ))
}

fn pick_input_config(device: &cpal::Device, target_rate: u32) -> Option<cpal::SupportedStreamConfig> {
    let mut configs = device.supported_input_configs().ok()?;
    configs.find_map(|config| {
        let min = config.min_sample_rate().0;
        let max = config.max_sample_rate().0;
        if min <= target_rate && target_rate <= max && [
            cpal::SampleFormat::F32,
            cpal::SampleFormat::I16,
            cpal::SampleFormat::U16,
            cpal::SampleFormat::I32
        ].contains(&config.sample_format()) {
            Some(config.with_sample_rate(cpal::SampleRate(target_rate)))
        } else {
            None
        }
    })
}

async fn inject_audio_file(
    config: &VoiceInputConfig,
    events: &broadcast::Sender<VoiceInputEvent>,
    path: &str,
) -> Result<(), String> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| format!("failed to read {}: {}", path, err))?;
    let mut reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).map_err(|err| err.to_string())?;
    let spec = reader.spec();

    let mut pipeline = AudioPipeline::new(
        spec.sample_rate,
        spec.channels as usize,
        config.stream_sample_rate,
        config.stream_channels,
        config.chunk_size,
    );
    let mut scratch = Vec::new();
    let sleep_ms = ((config.chunk_size as f32 / spec.sample_rate as f32) * 1000.0).max(1.0);
    for sample in reader.samples::<i16>() {
        let sample = sample.map_err(|err| err.to_string())?;
        scratch.push(sample as f32 / i16::MAX as f32);
        if scratch.len() >= config.chunk_size * spec.channels as usize {
            let chunks = pipeline.push_samples(&scratch, spec.channels as usize);
            for chunk in chunks {
                send_chunk(events, &chunk);
            }
            scratch.clear();
            tokio::time::sleep(Duration::from_millis(sleep_ms as u64)).await;
        }
    }

    if !scratch.is_empty() {
        let chunks = pipeline.push_samples(&scratch, spec.channels as usize);
        for chunk in chunks {
            send_chunk(events, &chunk);
        }
    }

    if let Some(leftover) = pipeline.finish() {
        send_chunk(events, &leftover);
    }
    let _ = events.send(VoiceInputEvent::AudioEnded);
    Ok(())
}

async fn stream_mock_audio(
    path: &str,
    chunk_frames: usize,
    sender: mpsc::Sender<Vec<f32>>,
) -> Result<(), String> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| format!("failed to read {}: {}", path, err))?;
    let mut reader =
        hound::WavReader::new(std::io::Cursor::new(bytes)).map_err(|err| err.to_string())?;
    let spec = reader.spec();
    let sleep_ms = ((chunk_frames as f32 / spec.sample_rate as f32) * 1000.0).max(1.0);

    let mut samples = Vec::new();
    for sample in reader.samples::<i16>() {
        let sample = sample.map_err(|err| err.to_string())?;
        samples.push(sample as f32 / i16::MAX as f32);
    }

    if samples.is_empty() {
        return Ok(());
    }

    loop {
        for chunk in samples.chunks(chunk_frames * spec.channels as usize) {
            match sender.try_send(chunk.to_vec()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Ok(());
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::debug!("mock audio frame dropped");
                }
            }
            tokio::time::sleep(Duration::from_millis(sleep_ms as u64)).await;
        }
    }
}

fn env_u32(key: &str, default: u32) -> u32 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn send_chunk(events: &broadcast::Sender<VoiceInputEvent>, chunk: &[i16]) {
    if chunk.is_empty() {
        return;
    }
    let _ = events.send(VoiceInputEvent::AudioChunk(cast_slice(chunk).to_vec()));
}

fn flush_audio(pipeline: &mut AudioPipeline, events: &broadcast::Sender<VoiceInputEvent>) {
    if let Some(leftover) = pipeline.finish() {
        send_chunk(events, &leftover);
    }
    let _ = events.send(VoiceInputEvent::AudioEnded);
}
