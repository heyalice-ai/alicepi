use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::time;

use crate::protocol::{SpeechRecCommand, SpeechRecEvent};
use crate::watchdog::Heartbeat;

pub async fn run(
    mut rx: mpsc::Receiver<SpeechRecCommand>,
    events: broadcast::Sender<SpeechRecEvent>,
    heartbeat: Heartbeat,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut buffer: Vec<u8> = Vec::new();
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
                    Some(SpeechRecCommand::AudioChunk(chunk)) => {
                        buffer.extend_from_slice(&chunk);
                    }
                    Some(SpeechRecCommand::AudioEnded) => {
                        if !buffer.is_empty() {
                            let size = buffer.len();
                            buffer.clear();
                            let text = format!("transcript({} bytes)", size);
                            let _ = events.send(SpeechRecEvent::Text { text, is_final: true });
                        }
                    }
                    Some(SpeechRecCommand::Reset) => {
                        buffer.clear();
                    }
                    Some(SpeechRecCommand::Shutdown) | None => {
                        break;
                    }
                }
            }
        }
    }
}
