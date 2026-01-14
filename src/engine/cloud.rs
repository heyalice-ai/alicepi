use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::ACCEPT;
use serde::Serialize;

use crate::engine::{
    env_duration_seconds, env_optional_string, env_string, send_with_retry, AudioStream, Engine,
    EngineAudio, EngineError, EngineRequest, EngineResponse,
};
use crate::protocol::{AudioOutput, AudioStreamFormat};

#[derive(Debug, Clone)]
pub struct CloudEngineConfig {
    pub api_url: String,
    pub voice_id: String,
    pub tenant_id: Option<String>,
    pub timeout: Duration,
    pub stream_audio: bool,
}

impl CloudEngineConfig {
    pub fn from_env() -> Self {
        Self {
            api_url: env_string("CLOUD_API_URL", "http://localhost:8080/api/voice/chat"),
            voice_id: env_string("CLOUD_VOICE_ID", "af_alloy"),
            tenant_id: env_optional_string("CLOUD_TENANT_ID"),
            timeout: env_duration_seconds("CLOUD_TIMEOUT_SECONDS", 30.0),
            stream_audio: true,
        }
    }
}

#[derive(Debug)]
pub struct CloudEngine {
    client: reqwest::Client,
    config: CloudEngineConfig,
}

impl CloudEngine {
    pub fn new(config: CloudEngineConfig) -> Result<Self, EngineError> {
        let client = reqwest::Client::builder()
            .user_agent("BookOfBooks/1.0")
            .timeout(config.timeout)
            .build()
            .map_err(|err| EngineError::CloudRequest(err.to_string()))?;
        Ok(Self { client, config })
    }
}

#[async_trait]
impl Engine for CloudEngine {
    async fn process(&self, request: EngineRequest<'_>) -> Result<EngineResponse, EngineError> {
        let payload = CloudRequest {
            query: request.text,
            voice_id: &self.config.voice_id,
            conversation_id: request.session_id,
            tenant_id: self.config.tenant_id.as_deref(),
        };

        let response = send_with_retry(|| {
            self.client
                .post(&self.config.api_url)
                .header(ACCEPT, "audio/mpeg")
                .json(&payload)
        })
        .await
        .map_err(|err| EngineError::CloudRequest(err.to_string()))?;

        let response = response
            .error_for_status()
            .map_err(|err| EngineError::CloudRequest(err.to_string()))?;

        if self.config.stream_audio {
            Ok(EngineResponse {
                assistant_text: None,
                audio: EngineAudio::Stream(AudioStream {
                    format: AudioStreamFormat::Mp3,
                    stream: Box::pin(response.bytes_stream().map(|chunk| {
                        chunk.map_err(|err| EngineError::CloudRequest(err.to_string()))
                    })),
                }),
            })
        } else {
            let data = response
                .bytes()
                .await
                .map_err(|err| EngineError::CloudRequest(err.to_string()))?;
            Ok(EngineResponse {
                assistant_text: None,
                audio: EngineAudio::Full(AudioOutput::Mp3 {
                    data: data.to_vec(),
                }),
            })
        }
    }
}

#[derive(Debug, Serialize)]
struct CloudRequest<'a> {
    #[serde(rename = "query")]
    query: &'a str,
    #[serde(rename = "voiceId")]
    voice_id: &'a str,
    #[serde(rename = "conversationId")]
    conversation_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none", rename = "tenantId")]
    tenant_id: Option<&'a str>,
}
