use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use rodio::source::SineWave;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::{mpsc, watch};

use crate::protocol::VoiceOutputCommand;

pub async fn run(mut rx: mpsc::Receiver<VoiceOutputCommand>, mut shutdown: watch::Receiver<bool>) {
    let (tx, thread_rx) = std_mpsc::channel();
    std::thread::spawn(move || output_loop(thread_rx));

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = tx.send(VoiceOutputCommand::Shutdown);
                break;
            }
            command = rx.recv() => {
                match command {
                    Some(command) => {
                        if tx.send(command).is_err() {
                            break;
                        }
                    }
                    None => {
                        let _ = tx.send(VoiceOutputCommand::Shutdown);
                        break;
                    }
                }
            }
        }
    }
}

fn output_loop(rx: std_mpsc::Receiver<VoiceOutputCommand>) {
    let (stream, handle) = match OutputStream::try_default() {
        Ok(value) => value,
        Err(err) => {
            tracing::error!("voice output failed to open device: {}", err);
            return;
        }
    };
    let _stream = stream;
    let mut current_sink: Option<Sink> = None;

    while let Ok(command) = rx.recv() {
        match command {
            VoiceOutputCommand::PlayText { text } => {
                stop_sink(&mut current_sink);
                play_beep(&handle, &mut current_sink);
                tracing::info!("voice output: {}", text);
            }
            VoiceOutputCommand::PlayAudioFile { path } => {
                stop_sink(&mut current_sink);
                match play_audio_file(&handle, &path) {
                    Ok(sink) => {
                        current_sink = Some(sink);
                        tracing::info!("voice output audio file: {}", path);
                    }
                    Err(err) => {
                        tracing::warn!("voice output failed to play {}: {}", path, err);
                    }
                }
            }
            VoiceOutputCommand::Stop => {
                stop_sink(&mut current_sink);
                tracing::info!("voice output stop");
            }
            VoiceOutputCommand::Shutdown => {
                stop_sink(&mut current_sink);
                break;
            }
        }
    }
}

fn play_audio_file(handle: &OutputStreamHandle, path: &str) -> Result<Sink, String> {
    let file = File::open(path).map_err(|err| format!("open failed: {}", err))?;
    let reader = BufReader::new(file);
    let decoder = Decoder::new(reader).map_err(|err| format!("decode failed: {}", err))?;
    let sink = Sink::try_new(handle).map_err(|err| format!("sink failed: {}", err))?;
    sink.append(decoder);
    sink.play();
    Ok(sink)
}

fn play_beep(handle: &OutputStreamHandle, current_sink: &mut Option<Sink>) {
    if let Ok(sink) = Sink::try_new(handle) {
        let source = SineWave::new(440.0)
            .take_duration(Duration::from_millis(250))
            .amplify(0.15);
        sink.append(source);
        sink.play();
        *current_sink = Some(sink);
    }
}

fn stop_sink(current_sink: &mut Option<Sink>) {
    if let Some(sink) = current_sink.take() {
        sink.stop();
    }
}
