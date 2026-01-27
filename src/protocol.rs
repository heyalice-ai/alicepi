use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    Listening,
    Processing,
    Speaking,
}

impl RuntimeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeState::Idle => "Idle",
            RuntimeState::Listening => "Listening",
            RuntimeState::Processing => "Processing",
            RuntimeState::Speaking => "Speaking",
        }
    }
}

impl std::fmt::Display for RuntimeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    Ping,
    Status,
    Text { text: String },
    VoiceFile { path: String },
    AudioFile { path: String },
    AudioStreamStart { format: AudioStreamFormat },
    AudioStreamChunk { data: Vec<u8> },
    AudioStreamEnd,
    ButtonPress,
    ButtonRelease,
    LidOpen,
    LidClose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub state: RuntimeState,
    pub mic_muted: bool,
    pub lid_open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerReply {
    Ok { message: String },
    Status { status: StatusSnapshot },
    Error { message: String },
}

#[derive(Debug, Clone)]
pub enum VoiceInputEvent {
    AudioChunk(Vec<u8>),
    AudioEnded,
}

#[derive(Debug, Clone)]
pub enum SpeechRecEvent {
    Text { text: String, is_final: bool },
}

#[derive(Debug, Clone)]
pub enum AudioOutput {
    Pcm {
        data: Vec<u8>,
        sample_rate: u32,
        channels: u16,
    },
    Mp3 {
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioStreamFormat {
    Pcm {
        sample_rate: u32,
        channels: u16,
    },
    Mp3,
}

#[derive(Debug, Clone)]
pub enum VoiceInputCommand {
    StartListening,
    StopListening,
    InjectAudioFile { path: String },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum SpeechRecCommand {
    AudioChunk(Vec<u8>),
    AudioEnded,
    Reset,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum VoiceOutputCommand {
    PlayText { text: String },
    PlayAudioFile { path: String },
    PlayAudio { audio: AudioOutput },
    StartStream { format: AudioStreamFormat },
    StreamChunk { data: Vec<u8> },
    EndStream,
    Stop,
    Shutdown,
}
