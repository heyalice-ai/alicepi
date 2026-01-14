use std::env;
use std::path::Path;
use std::time::{Duration, Instant};

use bytemuck::cast_slice;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time;

use crate::model_download;
use crate::protocol::{SpeechRecCommand, SpeechRecEvent};
use crate::watchdog::Heartbeat;

#[derive(Debug, Clone)]
struct SpeechRecConfig {
    sample_rate: u32,
    channels: u16,
    model: String,
    compute_type: String,
    backend: String,
    threads: usize,
}

impl SpeechRecConfig {
    fn from_env() -> Self {
        let sample_rate = env_u32("STREAM_SAMPLE_RATE", 16_000);
        let channels = env_u16("STREAM_CHANNELS", 1);
        let model = env::var("SR_WHISPER_MODEL").unwrap_or_else(|_| "base.en".to_string());
        let compute_type = env::var("SR_COMPUTE_TYPE").unwrap_or_else(|_| "int8".to_string());
        let backend = env::var("SR_BACKEND").unwrap_or_else(|_| "cpu".to_string());
        let threads = env_usize(
            "SR_THREADS",
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1),
        );
        Self {
            sample_rate,
            channels,
            model,
            compute_type,
            backend,
            threads,
        }
    }
}

#[derive(Debug)]
struct TranscribeRequest {
    request_id: u64,
    generation: u64,
    audio: Vec<i16>,
    sample_rate: u32,
    channels: u16,
}

#[derive(Debug)]
struct TranscribeResponse {
    generation: u64,
    text: Result<String, String>,
}

struct WhisperBackend {
    context: WhisperContext,
    threads: usize,
}

impl WhisperBackend {
    fn transcribe(&self, audio: &[i16], sample_rate: u32, channels: u16) -> Result<String, String> {
        if sample_rate != 16_000 {
            return Err(format!(
                "unsupported sample rate {}; whisper-rs expects 16000Hz",
                sample_rate
            ));
        }

        let mut float_audio = vec![0.0f32; audio.len()];
        whisper_rs::convert_integer_to_float_audio(audio, &mut float_audio)
            .map_err(|err| err.to_string())?;

        let mono_audio = match channels {
            1 => float_audio,
            2 => whisper_rs::convert_stereo_to_mono_audio(&float_audio)
                .map_err(|err| err.to_string())?,
            _ => {
                return Err(format!(
                    "unsupported channel count {}; whisper-rs expects mono audio",
                    channels
                ))
            }
        };

        if mono_audio.is_empty() {
            return Err("no audio samples to transcribe".to_string());
        }

        let mut state = self
            .context
            .create_state()
            .map_err(|err| err.to_string())?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 0 });
        params.set_n_threads(self.threads as i32);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, &mono_audio)
            .map_err(|err| err.to_string())?;

        let mut text = String::new();
        for segment in state.as_iter() {
            let segment_text = segment.to_string();
            if segment_text.trim().is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(segment_text.trim());
        }

        Ok(text)
    }
}

enum Backend {
    Cpu(WhisperBackend),
    #[cfg(feature = "hailo")]
    HailoStub,
}

impl Backend {
    fn transcribe(
        &self,
        audio: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<String, String> {
        match self {
            Backend::Cpu(model) => model.transcribe(audio, sample_rate, channels),
            #[cfg(feature = "hailo")]
            Backend::HailoStub => Err("hailo backend not implemented".to_string()),
        }
    }
}

pub async fn run(
    mut rx: mpsc::Receiver<SpeechRecCommand>,
    events: broadcast::Sender<SpeechRecEvent>,
    heartbeat: Heartbeat,
    mut shutdown: watch::Receiver<bool>,
) {
    let config = SpeechRecConfig::from_env();
    if let Err(err) = model_download::ensure_whisper_model(&config.model).await {
        tracing::warn!("whisper model download failed: {}", err);
    }
    let (req_tx, mut resp_rx) = spawn_transcriber(config.clone());
    let mut buffer: Vec<u8> = Vec::new();
    let mut tick = time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut chunk_count: u64 = 0;
    let mut last_log = Instant::now();
    let mut generation: u64 = 0;
    let mut request_id: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                break;
            }
            _ = tick.tick() => {
                heartbeat.tick();
                if last_log.elapsed() >= Duration::from_secs(5) {
                    let elapsed = last_log.elapsed().as_secs_f64().max(0.001);
                    let rate = chunk_count as f64 / elapsed;
                    tracing::debug!(
                        "speech_rec audio chunks: {} in {:.1}s ({:.1} chunks/sec)",
                        chunk_count,
                        elapsed,
                        rate
                    );
                    chunk_count = 0;
                    last_log = Instant::now();
                }
            }
            response = resp_rx.recv() => {
                match response {
                    Some(response) => {
                        if response.generation != generation {
                            continue;
                        }
                        match response.text {
                            Ok(text) => {
                                if !text.trim().is_empty() {
                                    let _ = events.send(SpeechRecEvent::Text { text, is_final: true });
                                }
                            }
                            Err(err) => {
                                tracing::warn!("speech rec transcription failed: {}", err);
                            }
                        }
                    }
                    None => {
                        tracing::warn!("speech rec worker channel closed; restarting task");
                        break;
                    }
                }
            }
            command = rx.recv() => {
                match command {
                    Some(SpeechRecCommand::AudioChunk(chunk)) => {
                        buffer.extend_from_slice(&chunk);
                        chunk_count = chunk_count.saturating_add(1);
                    }
                    Some(SpeechRecCommand::AudioEnded) => {
                        let aligned_len = buffer.len() - (buffer.len() % 2);
                        if aligned_len == 0 {
                            buffer.clear();
                            continue;
                        }
                        let audio: Vec<i16> = cast_slice(&buffer[..aligned_len]).to_vec();
                        buffer.clear();
                        request_id = request_id.wrapping_add(1);
                        let request = TranscribeRequest {
                            request_id,
                            generation,
                            audio,
                            sample_rate: config.sample_rate,
                            channels: config.channels,
                        };
                        if req_tx.send(request).await.is_err() {
                            tracing::warn!("speech rec worker unavailable");
                        }
                    }
                    Some(SpeechRecCommand::Reset) => {
                        generation = generation.wrapping_add(1);
                        buffer.clear();
                    }
                    Some(SpeechRecCommand::Shutdown) | None => {
                        break;
                    }
                }
            }
        }
    }
}

fn spawn_transcriber(
    config: SpeechRecConfig,
) -> (mpsc::Sender<TranscribeRequest>, mpsc::Receiver<TranscribeResponse>) {
    let (req_tx, mut req_rx) = mpsc::channel::<TranscribeRequest>(4);
    let (resp_tx, resp_rx) = mpsc::channel::<TranscribeResponse>(4);

    std::thread::spawn(move || {
        let backend = match init_backend(&config) {
            Ok(backend) => backend,
            Err(err) => {
                tracing::error!("speech rec backend init failed: {}", err);
                return;
            }
        };

        while let Some(request) = req_rx.blocking_recv() {
            let result = backend.transcribe(
                &request.audio,
                request.sample_rate,
                request.channels,
            );
            let _ = resp_tx.blocking_send(TranscribeResponse {
                generation: request.generation,
                text: result,
            });
        }
    });

    (req_tx, resp_rx)
}

fn init_backend(config: &SpeechRecConfig) -> Result<Backend, String> {
    if config.backend == "hailo" {
        #[cfg(feature = "hailo")]
        {
            return Ok(Backend::HailoStub);
        }
        #[cfg(not(feature = "hailo"))]
        {
            tracing::warn!("hailo backend requested but feature disabled; using cpu");
        }
    }

    let model_path = resolve_model_path(&config.model)?;
    if !Path::new(&model_path).exists() {
        return Err(format!(
            "whisper model path '{}' does not exist",
            model_path
        ));
    }

    let context = WhisperContext::new_with_params(
        &model_path,
        WhisperContextParameters::default(),
    )
    .map_err(|err| err.to_string())?;
    Ok(Backend::Cpu(WhisperBackend {
        context,
        threads: config.threads,
    }))
}

fn resolve_model_path(spec: &str) -> Result<String, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("SR_WHISPER_MODEL is empty".to_string());
    }

    let chosen = trimmed
        .split(',')
        .find(|value| !value.trim().is_empty())
        .unwrap_or(trimmed)
        .trim();

    if chosen.is_empty() {
        return Err("SR_WHISPER_MODEL does not contain a usable model path".to_string());
    }

    if Path::new(chosen).exists() {
        return Ok(chosen.to_string());
    }

    let fallback = model_download::default_models_path(&format!("ggml-{}.bin", chosen));
    if fallback.exists() {
        return Ok(fallback.to_string_lossy().to_string());
    }

    Ok(chosen.to_string())
}

fn env_u32(key: &str, default: u32) -> u32 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u16(key: &str, default: u16) -> u16 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
