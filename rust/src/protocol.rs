use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientCommand {
    Ping,
    Text { text: String },
    VoiceFile { path: String },
    AudioFile { path: String },
    ButtonPress,
    LidOpen,
    LidClose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerReply {
    Ok { message: String },
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
    Stop,
    Shutdown,
}
