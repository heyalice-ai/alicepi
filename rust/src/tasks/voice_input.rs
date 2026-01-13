use std::time::Duration;

use tokio::fs;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time;

use crate::protocol::{VoiceInputCommand, VoiceInputEvent};
use crate::watchdog::Heartbeat;

pub async fn run(
    mut rx: mpsc::Receiver<VoiceInputCommand>,
    events: broadcast::Sender<VoiceInputEvent>,
    heartbeat: Heartbeat,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut listening = false;

    let mut tick = time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                break;
            }
            _ = tick.tick() => {
                heartbeat.tick();
            }
            command = rx.recv() => {
                match command {
                    Some(VoiceInputCommand::StartListening) => {
                        listening = true;
                        let _ = events.send(VoiceInputEvent::VadSpeech);
                    }
                    Some(VoiceInputCommand::StopListening) => {
                        listening = false;
                    }
                    Some(VoiceInputCommand::InjectAudioFile { path }) => {
                        if listening {
                            if let Ok(bytes) = fs::read(&path).await {
                                let _ = events.send(VoiceInputEvent::AudioChunk(bytes));
                                let _ = events.send(VoiceInputEvent::AudioEnded);
                                let _ = events.send(VoiceInputEvent::VadSilence);
                            } else {
                                let _ = events.send(VoiceInputEvent::VadSilence);
                            }
                        }
                    }
                    Some(VoiceInputCommand::Shutdown) | None => {
                        break;
                    }
                }
            }
        }
    }
}
