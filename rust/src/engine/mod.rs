use std::env;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::protocol::AudioOutput;

mod cloud;
mod local;
mod session;

pub use cloud::{CloudEngine, CloudEngineConfig};
pub use local::{LocalEngine, LocalEngineConfig};
pub use session::{ChatMessage, SessionManager};

#[derive(Debug, Clone)]
pub struct EngineRequest<'a> {
    pub text: &'a str,
    pub history: &'a [ChatMessage],
    pub session_id: &'a str,
}

#[derive(Debug, Clone)]
pub struct EngineResponse {
    pub assistant_text: Option<String>,
    pub audio: AudioOutput,
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
