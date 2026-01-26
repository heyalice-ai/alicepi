use std::env;
use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{env_usize, SpeechRecStrategy};
use crate::model_download;

#[derive(Debug, Clone)]
pub struct WhisperConfig {
    pub model: String,
    pub backend: String,
    pub threads: usize,
}

impl WhisperConfig {
    pub fn from_env() -> Self {
        let model = env::var("SR_WHISPER_MODEL").unwrap_or_else(|_| "base.en".to_string());
        let backend = env::var("SR_BACKEND").unwrap_or_else(|_| "cpu".to_string());
        let threads = env_usize(
            "SR_THREADS",
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1),
        );
        Self {
            model,
            backend,
            threads,
        }
    }
}

pub struct WhisperBackend {
    context: WhisperContext,
    threads: usize,
    buffer: Vec<i16>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
}

impl SpeechRecStrategy for WhisperBackend {
    fn on_audio_chunk(
        &mut self,
        audio: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Option<String>, String> {
        self.ensure_format(sample_rate, channels)?;
        self.buffer.extend_from_slice(audio);
        Ok(None)
    }

    fn on_audio_end(&mut self) -> Result<Option<String>, String> {
        let sample_rate = match self.sample_rate {
            Some(rate) => rate,
            None => return Ok(None),
        };
        let channels = match self.channels {
            Some(channels) => channels,
            None => return Ok(None),
        };
        if self.buffer.is_empty() {
            return Ok(None);
        }

        let text = self.transcribe(&self.buffer, sample_rate, channels)?;
        self.buffer.clear();
        Ok(Some(text))
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.sample_rate = None;
        self.channels = None;
    }
}

pub fn init_whisper_backend(config: &WhisperConfig) -> Result<WhisperBackend, String> {
    if config.backend == "hailo" {
        #[cfg(feature = "hailo")]
        {
            return Err("hailo backend not implemented".to_string());
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
    Ok(WhisperBackend {
        context,
        threads: config.threads,
        buffer: Vec::new(),
        sample_rate: None,
        channels: None,
    })
}

impl WhisperBackend {
    fn ensure_format(&mut self, sample_rate: u32, channels: u16) -> Result<(), String> {
        if sample_rate != 16_000 {
            return Err(format!(
                "unsupported sample rate {}; whisper-rs expects 16000Hz",
                sample_rate
            ));
        }
        if let Some(existing) = self.sample_rate {
            if existing != sample_rate {
                return Err(format!(
                    "sample rate changed from {} to {}",
                    existing, sample_rate
                ));
            }
        } else {
            self.sample_rate = Some(sample_rate);
        }
        if let Some(existing) = self.channels {
            if existing != channels {
                return Err(format!(
                    "channel count changed from {} to {}",
                    existing, channels
                ));
            }
        } else {
            self.channels = Some(channels);
        }
        Ok(())
    }

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
