use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    Ping,
    Status,
    Text { text: String },
    VoiceFile { path: String },
    AudioFile { path: String },
    ButtonPress,
    LidOpen,
    LidClose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub state: String,
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
    VadSpeech,
    VadSilence,
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
    Stop,
    Shutdown,
}
