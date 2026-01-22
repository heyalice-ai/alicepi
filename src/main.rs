mod cli;
mod config;
mod engine;
mod model_download;
mod orchestrator;
mod protocol;
mod tasks;
mod watchdog;

use std::io::Read;
use std::process::exit;
use std::time::Duration;

use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;

use crate::cli::{ClientAction, Command, Cli};
use crate::config::ServerConfig;
use crate::protocol::{AudioStreamFormat, ClientCommand, ServerReply};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("alicepi=debug".parse()?))
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Server {
            bind,
            watchdog_ms,
            stream,
            no_stream,
            download_models,
            gpio_button,
            gpio_lid,
            save_request_wavs,
        } => {
            if download_models {
                let model = std::env::var("SR_WHISPER_MODEL")
                    .unwrap_or_else(|_| "base.en".to_string());
                model_download::ensure_whisper_model(&model)
                    .await
                    .map_err(|err| format!("model download failed: {}", err))?;
                let vad_path = model_download::default_assets_path("silero_vad.onnx");
                model_download::ensure_silero_vad(&vad_path)
                    .await
                    .map_err(|err| format!("VAD model download failed: {}", err))?;

                println!("Model download completed. Moving forward.");
            }
            
            let stream_audio = if stream {
                true
            } else if no_stream {
                false
            } else {
                false
            };
            let config = ServerConfig {
                bind_addr: bind,
                watchdog_timeout: Duration::from_millis(watchdog_ms),
                gpio_button_pin: gpio_button,
                gpio_lid_pin: gpio_lid,
                stream_audio,
                save_request_wavs_dir: save_request_wavs.map(std::path::PathBuf::from),
            };
            orchestrator::run_server(config).await.map_err(|err| err.into())
        }
        Command::Client { addr, action } => {
            match action {
                ClientAction::Ping => send_simple_command(&addr, ClientCommand::Ping).await,
                ClientAction::Status => send_simple_command(&addr, ClientCommand::Status).await,
                ClientAction::Text { text } => {
                    send_simple_command(&addr, ClientCommand::Text { text }).await
                }
                ClientAction::Voice { path } => {
                    send_simple_command(&addr, ClientCommand::VoiceFile { path }).await
                }
                ClientAction::Audio { path } => {
                    send_simple_command(&addr, ClientCommand::AudioFile { path }).await
                }
                ClientAction::AudioStream {
                    path,
                    chunk_bytes,
                    delay_after_bytes,
                    delay_ms,
                } => {
                    send_audio_stream(&addr, &path, chunk_bytes, delay_after_bytes, delay_ms).await
                }
                ClientAction::ButtonPress => {
                    send_simple_command(&addr, ClientCommand::ButtonPress).await
                }
                ClientAction::ButtonRelease => {
                    send_simple_command(&addr, ClientCommand::ButtonRelease).await
                }
                ClientAction::LidOpen => {
                    send_simple_command(&addr, ClientCommand::LidOpen).await
                }
                ClientAction::LidClose => {
                    send_simple_command(&addr, ClientCommand::LidClose).await
                }
            }
        }
    }
}

async fn send_simple_command(
    addr: &str,
    command: ClientCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let reply = send_command(addr, command).await?;
    match reply {
        ServerReply::Ok { message } => {
            println!("ok: {}", message);
        }
        ServerReply::Status { status } => {
            println!(
                "state: {}, mic_muted: {}, lid_open: {}",
                status.state, status.mic_muted, status.lid_open
            );
        }
        ServerReply::Error { message } => {
            println!("error: {}", message);
        }
    }
    Ok(())
}

async fn send_command(addr: &str, command: ClientCommand) -> Result<ServerReply, String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|err| format!("connect failed: {}", err))?;

    let payload = serde_json::to_string(&command)
        .map_err(|err| format!("serialize failed: {}", err))?;
    stream
        .write_all(payload.as_bytes())
        .await
        .map_err(|err| format!("write failed: {}", err))?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|err| format!("write failed: {}", err))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|err| format!("read failed: {}", err))?;

    serde_json::from_str(&line).map_err(|err| format!("invalid reply: {}", err))
}

async fn send_audio_stream(
    addr: &str,
    path: &str,
    chunk_bytes: usize,
    delay_after_bytes: usize,
    delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if chunk_bytes == 0 {
        return Err("chunk_bytes must be > 0".into());
    }
    if delay_after_bytes == 0 && delay_ms > 0 {
        return Err("delay_after_bytes must be > 0 when delay_ms is set".into());
    }

    let mut file = std::fs::File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    let stream = TcpStream::connect(addr).await?;
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    send_stream_command(
        &mut writer,
        &mut lines,
        ClientCommand::AudioStreamStart {
            format: AudioStreamFormat::Mp3,
        },
    )
    .await?;

    let mut sent_bytes = 0usize;
    let mut delayed = false;
    for chunk in data.chunks(chunk_bytes) {
        if delay_after_bytes > 0 && !delayed && sent_bytes >= delay_after_bytes {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delayed = true;
        }
        send_stream_command(
            &mut writer,
            &mut lines,
            ClientCommand::AudioStreamChunk {
                data: chunk.to_vec(),
            },
        )
        .await?;
        sent_bytes = sent_bytes.saturating_add(chunk.len());
    }

    send_stream_command(
        &mut writer,
        &mut lines,
        ClientCommand::AudioStreamEnd,
    )
    .await?;

    Ok(())
}

async fn send_stream_command(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
    command: ClientCommand,
) -> Result<ServerReply, String> {
    let payload = serde_json::to_string(&command)
        .map_err(|err| format!("serialize failed: {}", err))?;
    writer
        .write_all(payload.as_bytes())
        .await
        .map_err(|err| format!("write failed: {}", err))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|err| format!("write failed: {}", err))?;

    let line = lines
        .next_line()
        .await
        .map_err(|err| format!("read failed: {}", err))?;
    let line = line.ok_or_else(|| "server closed connection".to_string())?;
    serde_json::from_str(&line).map_err(|err| format!("invalid reply: {}", err))
}
