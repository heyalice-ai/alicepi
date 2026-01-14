use std::env;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::protocol::{AudioOutput, AudioStreamFormat};

mod cloud;
mod local;
mod session;

pub use cloud::{CloudEngine, CloudEngineConfig};
pub use local::{LocalEngine, LocalEngineConfig};
pub use session::{ChatMessage, SessionManager};

const MAX_RETRY_ATTEMPTS: usize = 5;
const RETRY_BACKOFF_BASE_MS: u64 = 200;

fn retry_backoff_duration(attempt: usize) -> Duration {
    let factor = 1u64 << attempt.saturating_sub(1);
    Duration::from_millis(RETRY_BACKOFF_BASE_MS.saturating_mul(factor))
}

fn escape_single_quotes(value: &str) -> String {
    value.replace('\'', r"'\''")
}

fn curl_equivalent(request: &reqwest::Request) -> String {
    let mut parts = Vec::new();
    parts.push("curl".to_string());
    parts.push("-X".to_string());
    parts.push(request.method().to_string());
    parts.push(format!("'{}'", escape_single_quotes(request.url().as_str())));
    for (name, value) in request.headers().iter() {
        if let Ok(value) = value.to_str() {
            parts.push("-H".to_string());
            parts.push(format!(
                "'{}: {}'",
                escape_single_quotes(name.as_str()),
                escape_single_quotes(value)
            ));
        }
    }
    if let Some(body) = request.body().and_then(|body| body.as_bytes()) {
        let data = match std::str::from_utf8(body) {
            Ok(value) => escape_single_quotes(value),
            Err(_) => format!("<{} bytes>", body.len()),
        };
        parts.push("-d".to_string());
        parts.push(format!("'{}'", data));
    }
    parts.join(" ")
}

fn debug_urls_enabled() -> bool {
    env::var("DEBUG_URLS")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

pub(crate) async fn send_with_retry<F>(mut build: F) -> Result<reqwest::Response, reqwest::Error>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        let builder = build();
        if debug_urls_enabled() {
            if let Some(clone) = builder.try_clone() {
                if let Ok(request) = clone.build() {
                    let curl = curl_equivalent(&request);
                    info!(curl = %curl, "sending request");
                }
            }
        }
        let response = builder.send().await?;
        if response.status().is_server_error() && attempt < MAX_RETRY_ATTEMPTS {
            warn!(
                status = %response.status(),
                attempt,
                max_attempts = MAX_RETRY_ATTEMPTS,
                "request failed with 5xx, retrying"
            );
            sleep(retry_backoff_duration(attempt)).await;
            continue;
        }
        return Ok(response);
    }
}

#[derive(Debug, Clone)]
pub struct EngineRequest<'a> {
    pub text: &'a str,
    pub history: &'a [ChatMessage],
    pub session_id: &'a str,
}

#[derive(Debug)]
pub struct EngineResponse {
    pub assistant_text: Option<String>,
    pub audio: EngineAudio,
}

pub struct AudioStream {
    pub format: AudioStreamFormat,
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, EngineError>> + Send>>,
}

#[derive(Debug)]
pub enum EngineAudio {
    Full(AudioOutput),
    Stream(AudioStream),
}

impl std::fmt::Debug for AudioStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioStream")
            .field("format", &self.format)
            .field("stream", &"<stream>")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("llm request failed: {0}")]
    LlmRequest(String),
    #[error("vibevoice request failed: {0}")]
    Vibevoice(String),
    #[error("cloud request failed: {0}")]
    CloudRequest(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait Engine: Send + Sync {
    async fn process(&self, request: EngineRequest<'_>) -> Result<EngineResponse, EngineError>;
}

#[derive(Debug, Clone)]
pub enum EngineConfig {
    Local(LocalEngineConfig),
    Cloud(CloudEngineConfig),
}

impl EngineConfig {
    pub fn from_env() -> Self {
        let mode = env::var("ORCHESTRATOR_MODE")
            .unwrap_or_else(|_| "local".to_string())
            .to_lowercase();
        if mode == "cloud" {
            EngineConfig::Cloud(CloudEngineConfig::from_env())
        } else {
            EngineConfig::Local(LocalEngineConfig::from_env())
        }
    }
}

pub fn build_engine(config: EngineConfig) -> Result<Arc<dyn Engine>, EngineError> {
    match config {
        EngineConfig::Local(config) => Ok(Arc::new(LocalEngine::new(config)?)),
        EngineConfig::Cloud(config) => Ok(Arc::new(CloudEngine::new(config)?)),
    }
}

fn env_string(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_optional_string(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
}

fn env_optional_f32(key: &str) -> Option<f32> {
    env::var(key).ok().and_then(|value| value.parse::<f32>().ok())
}

fn env_optional_u32(key: &str) -> Option<u32> {
    env::var(key).ok().and_then(|value| value.parse::<u32>().ok())
}

fn env_duration_seconds(key: &str, default_secs: f32) -> Duration {
    let value = env_optional_f32(key).unwrap_or(default_secs);
    Duration::from_secs_f32(value.max(0.0))
}
