use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

use ort::tensor::TensorElementType;
use ort::value::Tensor;
use ort::session::Session;
use tokenizers::Tokenizer;

use super::SpeechRecStrategy;
use crate::model_download;

#[derive(Debug, Clone)]
pub struct MoonshineConfig {
    pub model: String,
    pub precision: String,
    pub model_dir: Option<PathBuf>,
    pub encoder: String,
    pub decoder: String,
    pub tokenizer: PathBuf,
    pub max_tokens: usize,
    pub min_audio_secs: f32,
    pub max_audio_secs: f32,
    pub partial_secs: f32,
    pub partial_window_secs: f32,
    pub num_threads: usize,
}

impl MoonshineConfig {
    pub fn from_env() -> Self {
        let model = env::var("SR_MOONSHINE_MODEL").unwrap_or_else(|_| "moonshine/tiny".to_string());
        let model_name = model.split('/').last().unwrap_or(model.as_str());
        let precision = env::var("SR_MOONSHINE_PRECISION").unwrap_or_else(|_| "float".to_string());
        let model_dir = env::var("SR_MOONSHINE_MODEL_DIR").ok().map(PathBuf::from);
        let encoder = env::var("SR_MOONSHINE_ENCODER").unwrap_or_default();
        let decoder = env::var("SR_MOONSHINE_DECODER").unwrap_or_default();
        let tokenizer = env::var("SR_MOONSHINE_TOKENIZER")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(format!("assets/moonshine_{}_tokenizer.json", model_name))
            });
        let max_tokens = env_usize("SR_MOONSHINE_MAX_TOKENS", 192);
        let min_audio_secs = env_f32("SR_MOONSHINE_MIN_AUDIO_SECS", 0.1);
        let max_audio_secs = env_f32("SR_MOONSHINE_MAX_AUDIO_SECS", 64.0);
        let partial_secs = env_f32("SR_MOONSHINE_PARTIAL_SECS", 0.0);
        let partial_window_secs = env_f32("SR_MOONSHINE_PARTIAL_WINDOW_SECS", 0.0);
        let num_threads = env_usize(
            "SR_MOONSHINE_NUM_THREADS",
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1),
        );
        Self {
            model,
            precision,
            model_dir,
            encoder,
            decoder,
            tokenizer,
            max_tokens,
            min_audio_secs,
            max_audio_secs,
            partial_secs,
            partial_window_secs,
            num_threads,
        }
    }
}

pub struct MoonshineBackend {
    config: MoonshineConfig,
    encoder: Session,
    decoder: Session,
    encoder_inputs: HashSet<String>,
    decoder_inputs: HashSet<String>,
    decoder_input_types: HashMap<String, TensorElementType>,
    tokenizer: Tokenizer,
    buffer: Vec<i16>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    last_partial: String,
    last_partial_samples: usize,
    model_spec: MoonshineModelSpec,
}

#[derive(Debug, Clone, Copy)]
struct MoonshineModelSpec {
    num_layers: usize,
    num_key_value_heads: usize,
    head_dim: usize,
    decoder_start_token_id: i64,
    eos_token_id: i64,
}

impl MoonshineModelSpec {
    fn from_model_name(model: &str) -> Result<Self, String> {
        let model = model.to_lowercase();
        if model.contains("tiny") {
            Ok(Self {
                num_layers: 6,
                num_key_value_heads: 8,
                head_dim: 36,
                decoder_start_token_id: 1,
                eos_token_id: 2,
            })
        } else if model.contains("base") {
            Ok(Self {
                num_layers: 8,
                num_key_value_heads: 8,
                head_dim: 52,
                decoder_start_token_id: 1,
                eos_token_id: 2,
            })
        } else {
            Err(format!("unknown Moonshine model '{}'; expected 'tiny' or 'base'", model))
        }
    }
}

impl SpeechRecStrategy for MoonshineBackend {
    fn on_audio_chunk(
        &mut self,
        audio: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Option<String>, String> {
        self.ensure_format(sample_rate, channels)?;
        self.buffer.extend_from_slice(audio);

        if self.config.partial_secs <= 0.0 {
            return Ok(None);
        }

        let mono_samples = self.buffer.len() / channels as usize;
        if mono_samples == 0 {
            return Ok(None);
        }

        let secs = mono_samples as f32 / sample_rate as f32;
        if secs < self.config.partial_secs || secs < self.config.min_audio_secs {
            return Ok(None);
        }

        let stride_samples = (self.config.partial_secs * sample_rate as f32) as usize;
        if self.last_partial_samples > 0 && mono_samples < self.last_partial_samples + stride_samples {
            return Ok(None);
        }

        let window_secs = if self.config.partial_window_secs > 0.0 {
            self.config.partial_window_secs
        } else {
            secs
        }
        .min(self.config.max_audio_secs);
        let window_samples = (window_secs * sample_rate as f32) as usize;
        let window_samples = window_samples.min(mono_samples);
        if window_samples == 0 {
            return Ok(None);
        }

        let start_sample = mono_samples.saturating_sub(window_samples);
        let mono_audio = to_mono_f32(&self.buffer, channels)?;
        let window = &mono_audio[start_sample..];
        let text = self.transcribe_audio(window, sample_rate)?;
        self.last_partial_samples = mono_samples;
        if text.trim().is_empty() || text == self.last_partial {
            return Ok(None);
        }
        self.last_partial = text.clone();
        Ok(Some(text))
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

        let mono_audio = to_mono_f32(&self.buffer, channels)?;
        let text = self.transcribe_segments(&mono_audio, sample_rate)?;

        self.buffer.clear();
        self.last_partial.clear();
        self.last_partial_samples = 0;

        Ok(if text.trim().is_empty() { None } else { Some(text) })
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.sample_rate = None;
        self.channels = None;
        self.last_partial.clear();
        self.last_partial_samples = 0;
    }
}

pub fn init_moonshine_backend(config: &MoonshineConfig) -> Result<MoonshineBackend, String> {
    let model_spec = MoonshineModelSpec::from_model_name(&config.model)?;
    let (encoder_path, decoder_path) = resolve_model_paths(config)?;

    if !encoder_path.exists() {
        return Err(format!(
            "moonshine encoder model path '{}' does not exist",
            encoder_path.display()
        ));
    }
    if !decoder_path.exists() {
        return Err(format!(
            "moonshine decoder model path '{}' does not exist",
            decoder_path.display()
        ));
    }
    if !config.tokenizer.exists() {
        return Err(format!(
            "moonshine tokenizer '{}' does not exist",
            config.tokenizer.display()
        ));
    }

    let encoder = Session::builder()
        .map_err(|err| err.to_string())?
        .with_intra_threads(config.num_threads)
        .map_err(|err| err.to_string())?
        .commit_from_file(&encoder_path)
        .map_err(|err| err.to_string())?;

    let decoder = Session::builder()
        .map_err(|err| err.to_string())?
        .with_intra_threads(config.num_threads)
        .map_err(|err| err.to_string())?
        .commit_from_file(&decoder_path)
        .map_err(|err| err.to_string())?;

    let encoder_inputs = session_input_names(&encoder);
    let decoder_inputs = session_input_names(&decoder);
    let decoder_input_types = session_input_types(&decoder);
    let tokenizer = Tokenizer::from_file(&config.tokenizer).map_err(|err| err.to_string())?;

    // Touch sessions to get shapes resolved early.
    let _ = encoder.inputs();
    let _ = decoder.inputs();

    Ok(MoonshineBackend {
        config: config.clone(),
        encoder,
        decoder,
        encoder_inputs,
        decoder_inputs,
        decoder_input_types,
        tokenizer,
        buffer: Vec::new(),
        sample_rate: None,
        channels: None,
        last_partial: String::new(),
        last_partial_samples: 0,
        model_spec,
    })
}

impl MoonshineBackend {
    fn ensure_format(&mut self, sample_rate: u32, channels: u16) -> Result<(), String> {
        if sample_rate != 16_000 {
            return Err(format!(
                "unsupported sample rate {}; moonshine expects 16000Hz",
                sample_rate
            ));
        }
        if channels != 1 && channels != 2 {
            return Err(format!(
                "unsupported channel count {}; moonshine expects mono or stereo",
                channels
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

    fn transcribe_segments(&mut self, audio: &[f32], sample_rate: u32) -> Result<String, String> {
        let max_samples = (self.config.max_audio_secs * sample_rate as f32) as usize;
        if max_samples == 0 {
            return Err("SR_MOONSHINE_MAX_AUDIO_SECS is too small".to_string());
        }

        let min_samples = (self.config.min_audio_secs * sample_rate as f32) as usize;
        if audio.len() < min_samples {
            return Ok(String::new());
        }

        let mut parts = Vec::new();
        for chunk in audio.chunks(max_samples) {
            if chunk.len() < min_samples {
                continue;
            }
            let text = self.transcribe_audio(chunk, sample_rate)?;
            if !text.trim().is_empty() {
                parts.push(text);
            }
        }

        Ok(parts.join(" "))
    }

    fn transcribe_audio(&mut self, audio: &[f32], sample_rate: u32) -> Result<String, String> {
        let num_seconds = audio.len() as f32 / sample_rate as f32;
        if num_seconds < self.config.min_audio_secs {
            return Ok(String::new());
        }
        if num_seconds > self.config.max_audio_secs {
            return Err(format!(
                "audio segment is {:.2}s, exceeds max {:.2}s; split audio or raise SR_MOONSHINE_MAX_AUDIO_SECS",
                num_seconds, self.config.max_audio_secs
            ));
        }

        let tokens = self.generate_tokens(audio)?;
        let token_ids: Vec<u32> = tokens
            .into_iter()
            .filter_map(|token| u32::try_from(token).ok())
            .collect();
        self.tokenizer
            .decode(&token_ids, true)
            .map_err(|err| err.to_string())
    }

    fn generate_tokens(&mut self, audio: &[f32]) -> Result<Vec<i64>, String> {
        let audio_tensor = Tensor::from_array(([1usize, audio.len()], audio.to_vec()))
            .map_err(|err| err.to_string())?;
        let mut encoder_inputs = ort::inputs! { "input_values" => audio_tensor };

        if self.encoder_inputs.contains("attention_mask") {
            let mask = build_attention_mask(audio.len(), self.encoder_input_type("attention_mask"))?;
            encoder_inputs.push(("attention_mask".into(), mask.into()));
        }
        
        let dtype = self.decoder_input_type("encoder_attention_mask");
        let dtype_cache = self.decoder_input_type("use_cache_branch");
        let encoder_outputs = self.encoder
            .run(encoder_inputs)
            .map_err(|err| err.to_string())?;
        let mut encoder_iter = encoder_outputs.into_iter();
        let encoder_hidden = match encoder_iter.next() {
            Some((_, value)) => value,
            None => return Err("moonshine encoder returned no outputs".to_string()),
        };

        let audio_attention_mask = if self.decoder_inputs.contains("encoder_attention_mask") {
            Some(build_attention_mask(
                audio.len(),
                dtype,
            )?)
        } else {
            None
        };

        let past_keys = build_past_key_names(self.model_spec.num_layers);
        let mut past_values = build_empty_past_values(
            &past_keys,
            self.model_spec.num_key_value_heads,
            self.model_spec.head_dim,
        )?;

        let mut tokens = vec![self.model_spec.decoder_start_token_id];
        let mut input_ids = vec![self.model_spec.decoder_start_token_id];

        for step in 0..self.config.max_tokens {
            let use_cache = step > 0;
            let input_ids_tensor =
                Tensor::from_array(([1usize, input_ids.len()], input_ids.clone()))
                    .map_err(|err| err.to_string())?;

            let mut decoder_inputs = ort::inputs! {
                "input_ids" => input_ids_tensor,
                "encoder_hidden_states" => &encoder_hidden,
            };

            if let Some(ref mask) = audio_attention_mask {
                decoder_inputs.push(("encoder_attention_mask".into(), mask.into()));
            }

            if self.decoder_inputs.contains("use_cache_branch") {
                let use_cache_tensor = build_use_cache_tensor(
                    use_cache,
                    dtype_cache,
                )?;
                decoder_inputs.push(("use_cache_branch".into(), use_cache_tensor.into()));
            }

            for (name, value) in &past_values {
                decoder_inputs.push((name.clone().into(), value.into()));
            }

            let decoder_outputs = self
                .decoder
                .run(decoder_inputs)
                .map_err(|err| err.to_string())?;

            let (logits, present_values) = split_decoder_outputs(decoder_outputs)?;
            let next_token = pick_next_token(&logits)?;
            tokens.push(next_token);
            if next_token == self.model_spec.eos_token_id {
                break;
            }

            if present_values.len() != past_values.len() {
                return Err(format!(
                    "moonshine decoder returned {} cache tensors, expected {}",
                    present_values.len(),
                    past_values.len()
                ));
            }

            for (idx, value) in present_values.into_iter().enumerate() {
                let update = !use_cache || past_values[idx].0.contains(".decoder.");
                if update {
                    past_values[idx].1 = value;
                }
            }

            input_ids = vec![next_token];
        }

        Ok(tokens)
    }

    fn encoder_input_type(&self, name: &str) -> Option<TensorElementType> {
        self.encoder
            .inputs()
            .iter()
            .find(|input| input.name() == name)
            .and_then(|input| input.dtype().tensor_type())
    }

    fn decoder_input_type(&self, name: &str) -> Option<TensorElementType> {
        self.decoder_input_types.get(name).copied()
    }
}

fn resolve_model_paths(config: &MoonshineConfig) -> Result<(PathBuf, PathBuf), String> {
    let model_name = config
        .model
        .split('/')
        .last()
        .unwrap_or(&config.model)
        .to_string();

    if !config.encoder.is_empty() || !config.decoder.is_empty() {
        if config.encoder.is_empty() || config.decoder.is_empty() {
            return Err("SR_MOONSHINE_ENCODER and SR_MOONSHINE_DECODER must both be set".to_string());
        }
        return Ok((PathBuf::from(&config.encoder), PathBuf::from(&config.decoder)));
    }

    let base_dir = if let Some(ref dir) = config.model_dir {
        dir.clone()
    } else {
        model_download::models_dir()
            .join("moonshine")
            .join(model_name)
            .join(&config.precision)
    };

    Ok((
        base_dir.join("encoder_model.onnx"),
        base_dir.join("decoder_model_merged.onnx"),
    ))
}

fn build_past_key_names(num_layers: usize) -> Vec<String> {
    let mut keys = Vec::new();
    for layer in 0..num_layers {
        for side in ["decoder", "encoder"] {
            for kind in ["key", "value"] {
                keys.push(format!("past_key_values.{layer}.{side}.{kind}"));
            }
        }
    }
    keys
}

fn build_empty_past_values(
    keys: &[String],
    num_heads: usize,
    head_dim: usize,
) -> Result<Vec<(String, ort::value::DynValue)>, String> {
    let mut values = Vec::with_capacity(keys.len());
    for name in keys {
        let tensor = Tensor::from_array((
            [0usize, num_heads, 1, head_dim],
            Vec::<f32>::new(),
        ))
        .map_err(|err| err.to_string())?
        .into_dyn();
        values.push((name.clone(), tensor));
    }
    Ok(values)
}

fn split_decoder_outputs(
    outputs: ort::session::SessionOutputs<'_>,
) -> Result<(ort::value::DynValue, Vec<ort::value::DynValue>), String> {
    let mut outputs: Vec<(String, ort::value::DynValue)> = outputs
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();

    if outputs.is_empty() {
        return Err("moonshine decoder returned no outputs".to_string());
    }

    let logits_index = outputs
        .iter()
        .position(|(name, _)| name.contains("logits"))
        .unwrap_or(0);
    let logits = outputs.remove(logits_index).1;

    let present = outputs.into_iter().map(|(_, value)| value).collect();
    Ok((logits, present))
}

fn pick_next_token(logits: &ort::value::DynValue) -> Result<i64, String> {
    let (shape, data) = logits
        .try_extract_tensor::<f32>()
        .map_err(|err| err.to_string())?;
    if shape.len() != 3 {
        return Err(format!(
            "unexpected logits shape {:?}; expected [1, seq, vocab]",
            shape
        ));
    }

    let seq_len = shape[1] as usize;
    let vocab_size = shape[2] as usize;
    if seq_len == 0 || vocab_size == 0 {
        return Err("logits tensor is empty".to_string());
    }

    let offset = (seq_len - 1) * vocab_size;
    let slice = &data[offset..offset + vocab_size];

    let mut best_idx = 0usize;
    let mut best_val = f32::NEG_INFINITY;
    for (idx, value) in slice.iter().enumerate() {
        if *value > best_val {
            best_val = *value;
            best_idx = idx;
        }
    }

    Ok(best_idx as i64)
}

fn build_attention_mask(
    len: usize,
    dtype: Option<TensorElementType>,
) -> Result<ort::value::DynValue, String> {
    let dtype = dtype.unwrap_or(TensorElementType::Int64);
    match dtype {
        TensorElementType::Int32 => Tensor::from_array(([1usize, len], vec![1i32; len]))
            .map_err(|err| err.to_string())
            .map(|v| v.into_dyn()),
        TensorElementType::Int64 => Tensor::from_array(([1usize, len], vec![1i64; len]))
            .map_err(|err| err.to_string())
            .map(|v| v.into_dyn()),
        _ => Err(format!("unsupported attention mask type {dtype}")),
    }
}

fn build_use_cache_tensor(
    enabled: bool,
    dtype: Option<TensorElementType>,
) -> Result<ort::value::DynValue, String> {
    let dtype = dtype.unwrap_or(TensorElementType::Int64);
    match dtype {
        TensorElementType::Bool => Tensor::from_array(([1usize], vec![enabled]))
            .map_err(|err| err.to_string())
            .map(|v| v.into_dyn()),
        TensorElementType::Int32 => Tensor::from_array(([1usize], vec![if enabled { 1i32 } else { 0i32 }]))
            .map_err(|err| err.to_string())
            .map(|v| v.into_dyn()),
        TensorElementType::Int64 => Tensor::from_array(([1usize], vec![if enabled { 1i64 } else { 0i64 }]))
            .map_err(|err| err.to_string())
            .map(|v| v.into_dyn()),
        _ => Err(format!("unsupported use_cache_branch type {dtype}")),
    }
}

fn session_input_names(session: &Session) -> HashSet<String> {
    session
        .inputs()
        .iter()
        .map(|input| input.name().to_string())
        .collect()
}

fn session_input_types(session: &Session) -> HashMap<String, TensorElementType> {
    session
        .inputs()
        .iter()
        .filter_map(|input| input.dtype().tensor_type().map(|ty| (input.name().to_string(), ty)))
        .collect()
}

fn to_mono_f32(audio: &[i16], channels: u16) -> Result<Vec<f32>, String> {
    match channels {
        1 => Ok(audio
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect()),
        2 => {
            let mut mono = Vec::with_capacity(audio.len() / 2);
            for frame in audio.chunks_exact(2) {
                let left = frame[0] as f32 / i16::MAX as f32;
                let right = frame[1] as f32 / i16::MAX as f32;
                mono.push((left + right) * 0.5);
            }
            Ok(mono)
        }
        _ => Err(format!(
            "unsupported channel count {}; moonshine expects mono audio",
            channels
        )),
    }
}

fn env_f32(key: &str, default: f32) -> f32 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
