use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytemuck::cast_slice;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time;

use crate::model_download;
use crate::protocol::{SpeechRecCommand, SpeechRecEvent};
use crate::watchdog::Heartbeat;

mod whisper;
#[cfg(feature = "sherpa")]
mod sherpa;

pub trait SpeechRecStrategy: Send {
    fn on_audio_chunk(
        &mut self,
        audio: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Option<String>, String>;
    fn on_audio_end(&mut self) -> Result<Option<String>, String>;
    fn reset(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechRecEngine {
    Whisper,
    SherpaZipformer,
}

impl SpeechRecEngine {
    fn from_env() -> Self {
        let raw = env::var("SR_ENGINE").unwrap_or_else(|_| "whisper".to_string());
        match raw.trim().to_lowercase().as_str() {
            "sherpa" | "sherpa-zipformer" | "zipformer" | "sherpa_zipformer" => {
                SpeechRecEngine::SherpaZipformer
            }
            _ => SpeechRecEngine::Whisper,
        }
    }
}

#[derive(Debug, Clone)]
struct SpeechRecConfig {
    sample_rate: u32,
    channels: u16,
    engine: SpeechRecEngine,
    whisper: whisper::WhisperConfig,
    #[allow(dead_code)]
    sherpa: SherpaConfig,
    hangover_silence: Duration,
}

impl SpeechRecConfig {
    fn from_env() -> Self {
        let sample_rate = env_u32("STREAM_SAMPLE_RATE", 16_000);
        let channels = env_u16("STREAM_CHANNELS", 1);
        let hangover_ms = env_u64("SILENCE_DURATION_MS", 500);
        let engine = SpeechRecEngine::from_env();

        Self {
            sample_rate,
            channels,
            engine,
            whisper: whisper::WhisperConfig::from_env(),
            sherpa: SherpaConfig::from_env(sample_rate),
            hangover_silence: Duration::from_millis(hangover_ms),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct SherpaConfig {
    encoder: String,
    decoder: String,
    joiner: String,
    tokens: String,
    model_name: String,
    model_variant: String,
    model_dir: String,
    provider: String,
    decoding_method: String,
    hotwords_file: String,
    modeling_unit: String,
    bpe_vocab: String,
    model_type: String,
    num_threads: i32,
    sample_rate: u32,
    feature_dim: i32,
    blank_penalty: f32,
    hotwords_score: f32,
}

impl SherpaConfig {
    fn from_env(default_sample_rate: u32) -> Self {
        let num_threads = env_i32(
            "SR_SHERPA_NUM_THREADS",
            std::thread::available_parallelism()
                .map(|count| count.get() as i32)
                .unwrap_or(1),
        );
        let mut config = Self {
            encoder: env::var("SR_SHERPA_ENCODER").unwrap_or_default(),
            decoder: env::var("SR_SHERPA_DECODER").unwrap_or_default(),
            joiner: env::var("SR_SHERPA_JOINER").unwrap_or_default(),
            tokens: env::var("SR_SHERPA_TOKENS").unwrap_or_default(),
            model_name: env::var("SR_SHERPA_MODEL")
                .unwrap_or_else(|_| "zipformer-en-2023-06-26".to_string()),
            model_variant: env::var("SR_SHERPA_MODEL_VARIANT")
                .unwrap_or_else(|_| "fp32".to_string()),
            model_dir: env::var("SR_SHERPA_MODEL_DIR").unwrap_or_default(),
            provider: env::var("SR_SHERPA_PROVIDER").unwrap_or_else(|_| "cpu".to_string()),
            decoding_method: env::var("SR_SHERPA_DECODING_METHOD")
                .unwrap_or_else(|_| "greedy_search".to_string()),
            hotwords_file: env::var("SR_SHERPA_HOTWORDS_FILE").unwrap_or_default(),
            modeling_unit: env::var("SR_SHERPA_MODELING_UNIT").unwrap_or_default(),
            bpe_vocab: env::var("SR_SHERPA_BPE_VOCAB").unwrap_or_default(),
            model_type: env::var("SR_SHERPA_MODEL_TYPE")
                .unwrap_or_else(|_| "zipformer".to_string()),
            num_threads,
            sample_rate: env_u32("SR_SHERPA_SAMPLE_RATE", default_sample_rate),
            feature_dim: env_i32("SR_SHERPA_FEATURE_DIM", 80),
            blank_penalty: env_f32("SR_SHERPA_BLANK_PENALTY", 0.0),
            hotwords_score: env_f32("SR_SHERPA_HOTWORDS_SCORE", 1.5),
        };

        let model_dir = if config.model_dir.trim().is_empty() {
            None
        } else {
            Some(Path::new(&config.model_dir))
        };
        if let Ok(Some(paths)) = model_download::sherpa_zipformer_paths(
            &config.model_name,
            &config.model_variant,
            model_dir,
        ) {
            config.apply_defaults_from_paths(paths);
        }
        config
    }

    fn apply_defaults_from_paths(&mut self, paths: model_download::SherpaZipformerPaths) {
        if self.encoder.is_empty() {
            self.encoder = paths.encoder.to_string_lossy().to_string();
        }
        if self.decoder.is_empty() {
            self.decoder = paths.decoder.to_string_lossy().to_string();
        }
        if self.joiner.is_empty() {
            self.joiner = paths.joiner.to_string_lossy().to_string();
        }
        if self.tokens.is_empty() {
            self.tokens = paths.tokens.to_string_lossy().to_string();
        }
        if self.bpe_vocab.is_empty() {
            if let Some(path) = paths.bpe_vocab {
                self.bpe_vocab = path.to_string_lossy().to_string();
            }
        }
        if self.modeling_unit.is_empty() {
            if let Some(unit) = paths.modeling_unit {
                self.modeling_unit = unit.to_string();
            }
        }
    }
}

#[derive(Debug)]
enum TranscribeRequest {
    AudioChunk {
        generation: u64,
        audio: Vec<i16>,
        sample_rate: u32,
        channels: u16,
    },
    End {
        generation: u64,
    },
    Reset,
}

#[derive(Debug)]
struct TranscribeResponse {
    generation: u64,
    text: Result<String, String>,
    is_final: bool,
}

pub async fn run(
    mut rx: mpsc::Receiver<SpeechRecCommand>,
    events: broadcast::Sender<SpeechRecEvent>,
    heartbeat: Heartbeat,
    mut shutdown: watch::Receiver<bool>,
    save_request_wavs_dir: Option<PathBuf>,
) {
    let mut config = SpeechRecConfig::from_env();
    if config.engine == SpeechRecEngine::Whisper {
        let result = run_with_heartbeat(&heartbeat, model_download::ensure_whisper_model(&config.whisper.model)).await;
        if let Err(err) = result {
            tracing::warn!("whisper model download failed: {}", err);
        }
    }
    if config.engine == SpeechRecEngine::SherpaZipformer {
        let model_dir = if config.sherpa.model_dir.trim().is_empty() {
            None
        } else {
            Some(Path::new(&config.sherpa.model_dir))
        };
        let result = run_with_heartbeat(
            &heartbeat,
            model_download::ensure_sherpa_zipformer_model(
                &config.sherpa.model_name,
                &config.sherpa.model_variant,
                model_dir,
            ),
        )
        .await;
        match result {
            Ok(Some(paths)) => {
                config.sherpa.apply_defaults_from_paths(paths);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("sherpa model download failed: {}", err);
            }
        }
    }

    let (req_tx, mut resp_rx) = spawn_transcriber(config.clone());
    let mut buffer: Vec<u8> = Vec::new();
    let mut request_audio: Vec<i16> = Vec::new();
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
                                    tracing::info!(
                                        "speech_rec result before orchestrator: is_final={} text={}",
                                        response.is_final,
                                        text
                                    );
                                    let _ = events.send(SpeechRecEvent::Text {
                                        text,
                                        is_final: response.is_final,
                                    });
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

                        let aligned_len = buffer.len() - (buffer.len() % 2);
                        if aligned_len == 0 {
                            continue;
                        }
                        let audio: Vec<i16> = cast_slice(&buffer[..aligned_len]).to_vec();
                        buffer.drain(..aligned_len);
                        request_audio.extend_from_slice(&audio);

                        let request = TranscribeRequest::AudioChunk {
                            generation,
                            audio,
                            sample_rate: config.sample_rate,
                            channels: config.channels,
                        };
                        if req_tx.send(request).await.is_err() {
                            tracing::warn!("speech rec worker unavailable");
                        }
                    }
                    Some(SpeechRecCommand::AudioEnded) => {
                        if !buffer.is_empty() {
                            buffer.clear();
                        }
                        if let Some(silence) = build_hangover_silence(
                            config.sample_rate,
                            config.channels,
                            config.hangover_silence,
                        ) {
                            if !silence.is_empty() {
                                request_audio.extend_from_slice(&silence);
                                let request = TranscribeRequest::AudioChunk {
                                    generation,
                                    audio: silence,
                                    sample_rate: config.sample_rate,
                                    channels: config.channels,
                                };
                                if req_tx.send(request).await.is_err() {
                                    tracing::warn!("speech rec worker unavailable");
                                }
                            }
                        }
                        request_id = request_id.wrapping_add(1);
                        if let Some(save_dir) = save_request_wavs_dir.clone() {
                            let audio_copy = request_audio.clone();
                            spawn_request_wav_save(
                                save_dir,
                                request_id,
                                config.sample_rate,
                                config.channels,
                                audio_copy,
                            );
                        }
                        request_audio.clear();
                        if req_tx.send(TranscribeRequest::End { generation }).await.is_err() {
                            tracing::warn!("speech rec worker unavailable");
                        }
                    }
                    Some(SpeechRecCommand::Reset) => {
                        generation = generation.wrapping_add(1);
                        buffer.clear();
                        request_audio.clear();
                        let _ = req_tx.send(TranscribeRequest::Reset).await;
                    }
                    Some(SpeechRecCommand::Shutdown) | None => {
                        break;
                    }
                }
            }
        }
    }
}

async fn run_with_heartbeat<F, T>(heartbeat: &Heartbeat, future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let heartbeat = heartbeat.clone();
    let ticker = tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    heartbeat.tick();
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });

    let result = future.await;
    let _ = stop_tx.send(());
    ticker.abort();
    result
}

fn spawn_transcriber(
    config: SpeechRecConfig,
) -> (mpsc::Sender<TranscribeRequest>, mpsc::Receiver<TranscribeResponse>) {
    let (req_tx, mut req_rx) = mpsc::channel::<TranscribeRequest>(4);
    let (resp_tx, resp_rx) = mpsc::channel::<TranscribeResponse>(4);

    std::thread::spawn(move || {
        let mut backend = match init_backend(&config) {
            Ok(backend) => backend,
            Err(err) => {
                tracing::error!("speech rec backend init failed: {}", err);
                return;
            }
        };

        while let Some(request) = req_rx.blocking_recv() {
            match request {
                TranscribeRequest::AudioChunk {
                    generation,
                    audio,
                    sample_rate,
                    channels,
                } => {
                    let result = backend.on_audio_chunk(&audio, sample_rate, channels);
                    if let Err(err) = &result {
                        let _ = resp_tx.blocking_send(TranscribeResponse {
                            generation,
                            text: Err(err.clone()),
                            is_final: false,
                        });
                        continue;
                    }
                    if let Ok(Some(text)) = result {
                        let _ = resp_tx.blocking_send(TranscribeResponse {
                            generation,
                            text: Ok(text),
                            is_final: false,
                        });
                    }
                }
                TranscribeRequest::End { generation } => {
                    let result = backend.on_audio_end();
                    if let Err(err) = &result {
                        let _ = resp_tx.blocking_send(TranscribeResponse {
                            generation,
                            text: Err(err.clone()),
                            is_final: true,
                        });
                        continue;
                    }
                    if let Ok(Some(text)) = result {
                        let _ = resp_tx.blocking_send(TranscribeResponse {
                            generation,
                            text: Ok(text),
                            is_final: true,
                        });
                    }
                }
                TranscribeRequest::Reset => {
                    backend.reset();
                }
            }
        }
    });

    (req_tx, resp_rx)
}

fn init_backend(config: &SpeechRecConfig) -> Result<Box<dyn SpeechRecStrategy>, String> {
    match config.engine {
        SpeechRecEngine::Whisper => {
            let backend = whisper::init_whisper_backend(&config.whisper)?;
            Ok(Box::new(backend))
        }
        SpeechRecEngine::SherpaZipformer => {
            #[cfg(feature = "sherpa")]
            {
                let backend = sherpa::init_zipformer_backend(&config.sherpa)?;
                Ok(Box::new(backend))
            }
            #[cfg(not(feature = "sherpa"))]
            {
                Err("sherpa engine requested but 'sherpa' feature disabled".to_string())
            }
        }
    }
}

fn spawn_request_wav_save(
    save_dir: PathBuf,
    request_id: u64,
    sample_rate: u32,
    channels: u16,
    audio: Vec<i16>,
) {
    tokio::task::spawn_blocking(move || {
        if let Err(err) =
            write_request_wav(&save_dir, request_id, sample_rate, channels, &audio)
        {
            tracing::warn!("failed to save request wav {}: {}", request_id, err);
        }
    });
}

fn write_request_wav(
    save_dir: &Path,
    request_id: u64,
    sample_rate: u32,
    channels: u16,
    audio: &[i16],
) -> Result<(), String> {
    if audio.is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(save_dir)
        .map_err(|err| format!("create dir {} failed: {}", save_dir.display(), err))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("request_{:06}_{}.wav", request_id, timestamp);
    let path = save_dir.join(filename);
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)
        .map_err(|err| format!("open wav {} failed: {}", path.display(), err))?;
    for sample in audio {
        writer
            .write_sample(*sample)
            .map_err(|err| format!("write wav {} failed: {}", path.display(), err))?;
    }
    writer
        .finalize()
        .map_err(|err| format!("finalize wav {} failed: {}", path.display(), err))?;
    Ok(())
}

fn env_u32(key: &str, default: u32) -> u32 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u16(key: &str, default: u16) -> u16 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_i32(key: &str, default: i32) -> i32 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_f32(key: &str, default: f32) -> f32 {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn build_hangover_silence(
    sample_rate: u32,
    channels: u16,
    hangover: Duration,
) -> Option<Vec<i16>> {
    if hangover.is_zero() {
        return None;
    }
    let samples = (sample_rate as u64)
        .saturating_mul(hangover.as_millis() as u64)
        .saturating_div(1000)
        .saturating_mul(channels as u64);
    if samples == 0 {
        return None;
    }
    Some(vec![0i16; samples as usize])
}
