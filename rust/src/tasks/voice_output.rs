use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::{self, Duration};

use crate::protocol::VoiceOutputCommand;

pub async fn run(mut rx: mpsc::Receiver<VoiceOutputCommand>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                break;
            }
            command = rx.recv() => {
                match command {
                    Some(VoiceOutputCommand::PlayText { text }) => {
                        tracing::info!("voice output: {}", text);
                    }
                    Some(VoiceOutputCommand::PlayAudioFile { path }) => {
                        tracing::info!("voice output audio file: {}", path);
                        let _ = time::sleep(Duration::from_millis(100)).await;
                    }
                    Some(VoiceOutputCommand::Stop) => {
                        tracing::info!("voice output stop");
                    }
                    Some(VoiceOutputCommand::Shutdown) | None => {
                        break;
                    }
                }
            }
        }
    }
}
