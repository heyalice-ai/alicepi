use std::ffi::{CStr, CString};
use std::ptr;

use sherpa_rs_sys as sys;

use super::{SherpaConfig, SpeechRecStrategy};

pub struct SherpaZipformerBackend {
    recognizer: *const sys::SherpaOnnxOnlineRecognizer,
    stream: *const sys::SherpaOnnxOnlineStream,
    sample_rate: u32,
    last_partial: String,
}

impl SherpaZipformerBackend {
    fn new(config: &SherpaConfig) -> Result<Self, String> {
        let encoder = to_cstring("SR_SHERPA_ENCODER", &config.encoder)?;
        let decoder = to_cstring("SR_SHERPA_DECODER", &config.decoder)?;
        let joiner = to_cstring("SR_SHERPA_JOINER", &config.joiner)?;
        let tokens = to_cstring("SR_SHERPA_TOKENS", &config.tokens)?;
        let provider = CString::new(config.provider.clone())
            .map_err(|_| "SR_SHERPA_PROVIDER contains an interior NUL byte".to_string())?;
        let decoding_method = CString::new(config.decoding_method.clone()).map_err(|_| {
            "SR_SHERPA_DECODING_METHOD contains an interior NUL byte".to_string()
        })?;
        let model_type = CString::new(config.model_type.clone()).map_err(|_| {
            "SR_SHERPA_MODEL_TYPE contains an interior NUL byte".to_string()
        })?;
        let modeling_unit = CString::new(config.modeling_unit.clone()).map_err(|_| {
            "SR_SHERPA_MODELING_UNIT contains an interior NUL byte".to_string()
        })?;
        let bpe_vocab = CString::new(config.bpe_vocab.clone()).map_err(|_| {
            "SR_SHERPA_BPE_VOCAB contains an interior NUL byte".to_string()
        })?;
        let hotwords_file = to_optional_cstring(&config.hotwords_file)?;

        let transducer_config = sys::SherpaOnnxOnlineTransducerModelConfig {
            encoder: encoder.as_ptr(),
            decoder: decoder.as_ptr(),
            joiner: joiner.as_ptr(),
        };

        let model_config = sys::SherpaOnnxOnlineModelConfig {
            transducer: transducer_config,
            paraformer: unsafe { std::mem::zeroed() },
            zipformer2_ctc: unsafe { std::mem::zeroed() },
            tokens: tokens.as_ptr(),
            num_threads: config.num_threads,
            provider: provider.as_ptr(),
            debug: 0,
            model_type: model_type.as_ptr(),
            modeling_unit: modeling_unit.as_ptr(),
            bpe_vocab: bpe_vocab.as_ptr(),
            tokens_buf: ptr::null(),
            tokens_buf_size: 0,
            nemo_ctc: unsafe { std::mem::zeroed() },
        };

        let recognizer_config = sys::SherpaOnnxOnlineRecognizerConfig {
            feat_config: sys::SherpaOnnxFeatureConfig {
                sample_rate: config.sample_rate as i32,
                feature_dim: config.feature_dim,
            },
            model_config,
            decoding_method: decoding_method.as_ptr(),
            max_active_paths: 0,
            enable_endpoint: 0,
            rule1_min_trailing_silence: 0.0,
            rule2_min_trailing_silence: 0.0,
            rule3_min_utterance_length: 0.0,
            hotwords_file: hotwords_file.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
            hotwords_score: config.hotwords_score,
            ctc_fst_decoder_config: sys::SherpaOnnxOnlineCtcFstDecoderConfig {
                graph: ptr::null(),
                max_active: 0,
            },
            rule_fsts: ptr::null(),
            rule_fars: ptr::null(),
            blank_penalty: config.blank_penalty,
            hotwords_buf: ptr::null(),
            hotwords_buf_size: 0,
            hr: sys::SherpaOnnxHomophoneReplacerConfig {
                dict_dir: ptr::null(),
                lexicon: ptr::null(),
                rule_fsts: ptr::null(),
            },
        };

        // Safety: All C strings live until CreateOnlineRecognizer returns.
        let recognizer = unsafe { sys::SherpaOnnxCreateOnlineRecognizer(&recognizer_config) };
        if recognizer.is_null() {
            return Err("sherpa-onnx failed to create online recognizer".to_string());
        }

        // Safety: recognizer is valid and managed by this struct.
        let stream = unsafe { sys::SherpaOnnxCreateOnlineStream(recognizer) };
        if stream.is_null() {
            unsafe {
                sys::SherpaOnnxDestroyOnlineRecognizer(recognizer);
            }
            return Err("sherpa-onnx failed to create online stream".to_string());
        }

        Ok(Self {
            recognizer,
            stream,
            sample_rate: config.sample_rate,
            last_partial: String::new(),
        })
    }

    fn decode_ready(&self) {
        // Safety: recognizer/stream pointers are valid for lifetime of self.
        unsafe {
            while sys::SherpaOnnxIsOnlineStreamReady(self.recognizer, self.stream) != 0 {
                sys::SherpaOnnxDecodeOnlineStream(self.recognizer, self.stream);
            }
        }
    }

    fn get_result_text(&self) -> Result<String, String> {
        // Safety: recognizer/stream pointers are valid.
        let result_ptr = unsafe {
            sys::SherpaOnnxGetOnlineStreamResult(self.recognizer, self.stream)
        };
        if result_ptr.is_null() {
            return Err("sherpa-onnx returned a null result".to_string());
        }

        let text = unsafe { online_result_to_string(result_ptr) };
        unsafe {
            sys::SherpaOnnxDestroyOnlineRecognizerResult(result_ptr);
        }
        Ok(text)
    }

    fn accept_waveform(&self, audio: &[i16], sample_rate: u32, channels: u16) -> Result<(), String> {
        if sample_rate != self.sample_rate {
            return Err(format!(
                "unsupported sample rate {}; sherpa-onnx expects {}Hz",
                sample_rate, self.sample_rate
            ));
        }
        let samples = to_mono_f32(audio, channels)?;
        if samples.is_empty() {
            return Ok(());
        }

        // Safety: stream pointer is valid and samples slice is valid.
        unsafe {
            sys::SherpaOnnxOnlineStreamAcceptWaveform(
                self.stream,
                self.sample_rate as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
        }

        Ok(())
    }
}

impl SpeechRecStrategy for SherpaZipformerBackend {
    fn on_audio_chunk(
        &mut self,
        audio: &[i16],
        sample_rate: u32,
        channels: u16,
    ) -> Result<Option<String>, String> {
        self.accept_waveform(audio, sample_rate, channels)?;
        self.decode_ready();
        let text = self.get_result_text()?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text == self.last_partial {
            return Ok(None);
        }
        self.last_partial = text.clone();
        Ok(Some(text))
    }

    fn on_audio_end(&mut self) -> Result<Option<String>, String> {
        // Safety: stream pointer is valid.
        unsafe {
            sys::SherpaOnnxOnlineStreamInputFinished(self.stream);
        }
        self.decode_ready();
        let text = self.get_result_text()?;
        self.last_partial.clear();

        // Safety: recognizer/stream pointers are valid.
        unsafe {
            sys::SherpaOnnxOnlineStreamReset(self.recognizer, self.stream);
        }

        if text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    fn reset(&mut self) {
        self.last_partial.clear();
        if !self.recognizer.is_null() && !self.stream.is_null() {
            unsafe {
                sys::SherpaOnnxOnlineStreamReset(self.recognizer, self.stream);
            }
        }
    }
}

impl Drop for SherpaZipformerBackend {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                sys::SherpaOnnxDestroyOnlineStream(self.stream);
            }
            if !self.recognizer.is_null() {
                sys::SherpaOnnxDestroyOnlineRecognizer(self.recognizer);
            }
        }
    }
}

pub fn init_zipformer_backend(config: &SherpaConfig) -> Result<SherpaZipformerBackend, String> {
    validate_path("SR_SHERPA_ENCODER", &config.encoder)?;
    validate_path("SR_SHERPA_DECODER", &config.decoder)?;
    validate_path("SR_SHERPA_JOINER", &config.joiner)?;
    validate_path("SR_SHERPA_TOKENS", &config.tokens)?;

    SherpaZipformerBackend::new(config)
}

fn validate_path(name: &str, path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err(format!("{} is required for sherpa-onnx", name));
    }
    if !std::path::Path::new(path).exists() {
        return Err(format!("{} path '{}' does not exist", name, path));
    }
    Ok(())
}

fn to_cstring(name: &str, value: &str) -> Result<CString, String> {
    if value.trim().is_empty() {
        return Err(format!("{} is required for sherpa-onnx", name));
    }
    CString::new(value).map_err(|_| format!("{} contains an interior NUL byte", name))
}

fn to_optional_cstring(value: &str) -> Result<Option<CString>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    CString::new(value)
        .map(Some)
        .map_err(|_| "SR_SHERPA_HOTWORDS_FILE contains an interior NUL byte".to_string())
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
            "unsupported channel count {}; sherpa-onnx expects mono audio",
            channels
        )),
    }
}

unsafe fn online_result_to_string(
    result: *const sys::SherpaOnnxOnlineRecognizerResult,
) -> String {
    if result.is_null() {
        return String::new();
    }
    let raw = result.read();
    if raw.text.is_null() {
        return String::new();
    }
    CStr::from_ptr(raw.text).to_string_lossy().into_owned()
}

// Safety: sherpa-onnx recognizer APIs are thread-safe for separate streams.
unsafe impl Send for SherpaZipformerBackend {}
unsafe impl Sync for SherpaZipformerBackend {}
