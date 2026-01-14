use std::env;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use rodio::source::SineWave;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::{mpsc, watch};

use crate::protocol::{AudioOutput, VoiceOutputCommand};

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

    while let Ok(command) = rx.recv() {
        match command {
            VoiceOutputCommand::PlayText { text } => {
                stop_sink(&mut current_sink);
                play_beep(&handle, &mut current_sink);
                tracing::info!("voice output: {}", text);
            }
            VoiceOutputCommand::PlayAudioFile { path } => {
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
            VoiceOutputCommand::Stop => {
                stop_sink(&mut current_sink);
                tracing::info!("voice output stop");
            }
            VoiceOutputCommand::Shutdown => {
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
