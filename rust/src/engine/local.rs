use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use crate::engine::{
    env_duration_seconds, env_optional_f32, env_optional_string, env_optional_u32, env_string,
    Engine, EngineError, EngineRequest, EngineResponse,
};
use crate::protocol::AudioOutput;

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are Alice, a helpful AI assistant for the AlicePi smart speaker. Keep your responses concise and friendly.

You are an incarnation of "Alice" from Alice in Wonderland, so use a whimsical and imaginative tone in your replies.

If you are asked about your identity, always say the following exactly:
I am Alice, a Language Model harnessed in a book, designed to help children learn and have fun.

Identify yourself as Alice in your replies. Use a warm and engaging tone, and avoid overly technical language.

The person listening to you is a child. Keep explanations simple and small.

You are speaking to a child through an ORCHESTRATOR.

You have access to the following tools:
- Voice Output: You can send audio responses to be spoken aloud.
To use this tool, preceed your message with [VOICE OUTPUT] and end it with [/VOICE OUTPUT].
Always ensure your responses are appropriate for a young audience.

Example:
User: "What's the weather like today?"
Alice: [VOICE OUTPUT]The weather today is sunny with a high of 75 degrees.[/VOICE OUTPUT]

- Memory: You can remember important details about the user to make interactions more personal.
To use this tool, preceed your message with [MEMORY] and end it with [/MEMORY].
Only use this tool to store information that will help you assist the user better in future interactions.
Example:
User: "My favorite color is blue."
Alice: [MEMORY]User's favorite color is blue.[/MEMORY]
Always ensure your responses are appropriate for a young audience.
When responding, consider the context of previous messages in the conversation history.

From previous sessions, you have the following memories:
[MEMORIES]
{memories}
[/MEMORIES]
- Book: You can ask the harness to retrieve information from the Alice in Wonderland book.
To use this tool, preceed your message with [BOOK] and end it with [/BOOK]. When you use this tool, you should expect
a response that includes relevant excerpts from the book. We will use a vector database to find the most relevant sections.
Example:
User: "Who is the Mad Hatter?"
Alice: [BOOK]red hatter character[/BOOK]
Harness Response: "- The Mad Hatter is a whimsical character. \n- The Mad Hatter hosts eccentric tea parties.\n- The Mad Hatter loves riddles and wordplay."
Alice: [VOICE OUTPUT]The Mad Hatter is a whimsical character known for his eccentric tea parties and riddles.[/VOICE OUTPUT]
Always ensure your responses are appropriate for a young audience.

END OF TOOLS DESCRIPTION.

When generating responses, always follow these guidelines:
1. Be concise and to the point.
2. Use simple language suitable for children.
3. Maintain a friendly and engaging tone.
4. Always identify yourself as Alice.
5. If you have access to your LLM underlying identity, you can mention it only if you are asked directly.


The user will now speak to you. Respond appropriately and helpfully.
"#;

#[derive(Debug, Clone)]
pub struct LocalEngineConfig {
    pub llm_api_url: String,
    pub llm_model: String,
    pub system_prompt: String,
    pub vibevoice_ws_url: String,
    pub vibevoice_cfg_scale: Option<f32>,
    pub vibevoice_inference_steps: Option<u32>,
    pub vibevoice_voice: Option<String>,
    pub vibevoice_connect_timeout: Duration,
    pub vibevoice_sample_rate: u32,
    pub vibevoice_channels: u16,
    pub llm_timeout: Duration,
}

impl LocalEngineConfig {
    pub fn from_env() -> Self {
        Self {
            llm_api_url: env_string("LLM_API_URL", "http://ollama:11434/v1/chat/completions"),
            llm_model: env_string("LLM_MODEL_NAME", "gemma3:270m"),
            system_prompt: env_string("SYSTEM_PROMPT", DEFAULT_SYSTEM_PROMPT),
            vibevoice_ws_url: env_string("VIBEVOICE_WS_URL", "ws://vibevoice:8000/stream"),
            vibevoice_cfg_scale: env_optional_f32("VIBEVOICE_CFG_SCALE"),
            vibevoice_inference_steps: env_optional_u32("VIBEVOICE_INFERENCE_STEPS"),
            vibevoice_voice: env_optional_string("VIBEVOICE_VOICE"),
            vibevoice_connect_timeout: env_duration_seconds("VIBEVOICE_CONNECT_TIMEOUT", 10.0),
            vibevoice_sample_rate: env_optional_u32("VIBEVOICE_SAMPLE_RATE").unwrap_or(22_050),
            vibevoice_channels: env_optional_u32("VIBEVOICE_CHANNELS")
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(1),
            llm_timeout: env_duration_seconds("LLM_TIMEOUT_SECONDS", 15.0),
        }
    }
}

#[derive(Clone)]
struct LlmClient {
    client: reqwest::Client,
    api_url: String,
    model: String,
    system_prompt: String,
}

impl LlmClient {
    fn new(config: &LocalEngineConfig) -> Result<Self, EngineError> {
        let client = reqwest::Client::builder()
            .timeout(config.llm_timeout)
            .build()
            .map_err(|err| EngineError::LlmRequest(err.to_string()))?;
        Ok(Self {
            client,
            api_url: config.llm_api_url.clone(),
            model: config.llm_model.clone(),
            system_prompt: config.system_prompt.clone(),
        })
    }

    async fn call(&self, history: &[crate::engine::ChatMessage]) -> Result<String, EngineError> {
        let mut messages = Vec::with_capacity(history.len() + 1);
        if !self.system_prompt.trim().is_empty() {
            messages.push(LlmMessage {
                role: "system".to_string(),
                content: self.system_prompt.clone(),
            });
        }
        for message in history {
            messages.push(LlmMessage {
                role: message.role.as_str().to_string(),
                content: message.content.clone(),
            });
        }

        let payload = LlmRequest {
            model: self.model.clone(),
            messages,
            stream: false,
        };

        let response = self
            .client
            .post(&self.api_url)
            .header(CONTENT_TYPE, "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|err| EngineError::LlmRequest(err.to_string()))?;

        let response = response
            .error_for_status()
            .map_err(|err| EngineError::LlmRequest(err.to_string()))?;

        let body: LlmResponse = response
            .json()
            .await
            .map_err(|err| EngineError::LlmRequest(err.to_string()))?;

        if let Some(content) = body.content() {
            return Ok(content.to_string());
        }

        Err(EngineError::InvalidResponse(
            "missing LLM response content".to_string(),
        ))
    }
}

#[derive(Clone)]
struct VibevoiceClient {
    ws_url: String,
    cfg_scale: Option<f32>,
    inference_steps: Option<u32>,
    voice: Option<String>,
    connect_timeout: Duration,
    sample_rate: u32,
    channels: u16,
}

impl VibevoiceClient {
    fn new(config: &LocalEngineConfig) -> Self {
        Self {
            ws_url: config.vibevoice_ws_url.clone(),
            cfg_scale: config.vibevoice_cfg_scale,
            inference_steps: config.vibevoice_inference_steps,
            voice: config.vibevoice_voice.clone(),
            connect_timeout: config.vibevoice_connect_timeout,
            sample_rate: config.vibevoice_sample_rate,
            channels: config.vibevoice_channels,
        }
    }

    async fn synthesize(&self, text: &str) -> Result<AudioOutput, EngineError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(EngineError::Vibevoice("empty voice output".to_string()));
        }

        let url = self
            .build_url(trimmed)
            .map_err(|err| EngineError::Vibevoice(err.to_string()))?;

        let connect = tokio_tungstenite::connect_async(url.as_str());
        let (stream, _response) = tokio::time::timeout(self.connect_timeout, connect)
            .await
            .map_err(|_| EngineError::Vibevoice("connection timeout".to_string()))?
            .map_err(|err| EngineError::Vibevoice(err.to_string()))?;

        let (_write, mut read) = stream.split();
        let mut audio = Vec::new();
        while let Some(message) = read.next().await {
            match message {
                Ok(Message::Binary(chunk)) => audio.extend_from_slice(&chunk),
                Ok(Message::Text(text)) => {
                    tracing::debug!("vibevoice message: {}", text);
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(err) => {
                    return Err(EngineError::Vibevoice(err.to_string()));
                }
            }
        }

        if audio.is_empty() {
            return Err(EngineError::Vibevoice(
                "no audio received from vibevoice".to_string(),
            ));
        }

        Ok(AudioOutput::Pcm {
            data: audio,
            sample_rate: self.sample_rate,
            channels: self.channels,
        })
    }

    fn build_url(&self, text: &str) -> Result<Url, url::ParseError> {
        let mut url = Url::parse(&self.ws_url)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("text", text);
            if let Some(cfg) = self.cfg_scale {
                pairs.append_pair("cfg", &cfg.to_string());
            }
            if let Some(steps) = self.inference_steps {
                pairs.append_pair("steps", &steps.to_string());
            }
            if let Some(voice) = &self.voice {
                pairs.append_pair("voice", voice);
            }
        }
        Ok(url)
    }
}

pub struct LocalEngine {
    llm: LlmClient,
    vibevoice: VibevoiceClient,
}

impl LocalEngine {
    pub fn new(config: LocalEngineConfig) -> Result<Self, EngineError> {
        Ok(Self {
            llm: LlmClient::new(&config)?,
            vibevoice: VibevoiceClient::new(&config),
        })
    }
}

#[async_trait]
impl Engine for LocalEngine {
    async fn process(&self, request: EngineRequest<'_>) -> Result<EngineResponse, EngineError> {
        let response_text = self.llm.call(request.history).await?;
        let voice_text = extract_voice_output(&response_text)
            .unwrap_or_else(|| response_text.trim().to_string());
        let audio = self.vibevoice.synthesize(&voice_text).await?;

        Ok(EngineResponse {
            assistant_text: Some(response_text),
            audio,
        })
    }

}

fn extract_voice_output(text: &str) -> Option<String> {
    static VOICE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    let regex = VOICE_OUTPUT_RE
        .get_or_init(|| Regex::new(r"(?is)\[VOICE OUTPUT\](.*?)\[/VOICE OUTPUT\]").unwrap());

    let segments: Vec<String> = regex
        .captures_iter(text)
        .filter_map(|cap| cap.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect();

    if segments.is_empty() {
        None
    } else {
        Some(segments.join(" "))
    }
}

#[derive(Debug, Serialize)]
struct LlmRequest {
    model: String,
    messages: Vec<LlmMessage>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct LlmMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LlmResponse {
    choices: Option<Vec<LlmChoice>>,
    message: Option<LlmAssistantMessage>,
}

impl LlmResponse {
    fn content(&self) -> Option<&str> {
        if let Some(choices) = &self.choices {
            return choices
                .iter()
                .find_map(|choice| choice.message.content.as_deref());
        }
        self.message.as_ref()?.content.as_deref()
    }
}

#[derive(Debug, Deserialize)]
struct LlmChoice {
    message: LlmAssistantMessage,
}

#[derive(Debug, Deserialize)]
struct LlmAssistantMessage {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::extract_voice_output;

    #[test]
    fn extracts_voice_output_segments() {
        let input = "Hello [VOICE OUTPUT]Hi![/VOICE OUTPUT] [VOICE OUTPUT]Bye.[/VOICE OUTPUT]";
        let output = extract_voice_output(input);
        assert_eq!(output.as_deref(), Some("Hi! Bye."));
    }

    #[test]
    fn returns_none_when_no_voice_output_tags() {
        let input = "Just text.";
        assert!(extract_voice_output(input).is_none());
    }

    #[test]
    fn ignores_empty_voice_output_segments() {
        let input = "[VOICE OUTPUT]  [/VOICE OUTPUT] [VOICE OUTPUT]Hello[/VOICE OUTPUT]";
        let output = extract_voice_output(input);
        assert_eq!(output.as_deref(), Some("Hello"));
    }
}
