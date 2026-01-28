mod cli;
mod config;
mod engine;
mod model_download;
mod orchestrator;
mod protocol;
mod tasks;
mod watchdog;

use std::io::Read;
use std::time::Duration;

use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;

use crate::cli::{ClientAction, Command, Cli};
use crate::config::ServerConfig;
use crate::protocol::{AudioStreamFormat, ClientCommand, RuntimeState, ServerReply, StatusSnapshot};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
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
            led_status_gpio,
            save_request_wavs,
        } => {
            if download_models {
                let model = std::env::var("SR_WHISPER_MODEL")
                    .unwrap_or_else(|_| "base.en".to_string());
                let vad_path = model_download::default_assets_path("silero_vad.onnx");
                model_download::ensure_models_with_progress(&model, &vad_path)
                    .await
                    .map_err(|err| format!("model download failed: {}", err))?;

                let assets_dir = model_download::assets_dir_path();
                model_download::ensure_moonshine_english_models_with_progress(&assets_dir)
                    .await
                    .map_err(|err| format!("moonshine model download failed: {}", err))?;

                println!("Model download completed. Moving forward.");
            }
            
            let stream_audio = if stream {
                true
            } else if no_stream {
                false
            } else {
                false
            };
            let gpio_status_led_pin =
                led_status_gpio.or_else(|| parse_env_u8("GPIO_STATUS_LED"));
            let config = ServerConfig {
                bind_addr: bind,
                watchdog_timeout: Duration::from_millis(watchdog_ms),
                gpio_button_pin: gpio_button,
                gpio_lid_pin: gpio_lid,
                gpio_status_led_pin,
                stream_audio,
                save_request_wavs_dir: save_request_wavs.map(std::path::PathBuf::from),
            };
            orchestrator::run_server(config).await.map_err(|err| err.into())
        }
        Command::LedTest { led_status_gpio } => {
            let gpio_status_led_pin =
                led_status_gpio.or_else(|| parse_env_u8("GPIO_STATUS_LED"));
            let Some(pin) = gpio_status_led_pin else {
                return Err("led-test requires --led-status-gpio or GPIO_STATUS_LED".into());
            };
            run_led_test(pin).await
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

#[cfg(feature = "gpio")]
async fn run_led_test(pin: u8) -> Result<(), Box<dyn std::error::Error>> {
    use rppal::gpio::Gpio;
    use tokio::io::AsyncBufReadExt;

    let gpio = Gpio::new().map_err(|err| format!("gpio unavailable: {}", err))?;
    let pin = gpio
        .get(pin)
        .map_err(|err| format!("failed to init status led pin {}: {}", pin, err))?
        .into_output_low();

    let (status_tx, status_rx) = tokio::sync::watch::channel(StatusSnapshot {
        state: RuntimeState::Idle,
        mic_muted: false,
        lid_open: true,
    });
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let led_task = tokio::spawn(tasks::gpio::run_status_led_with_env(
        pin,
        status_rx,
        shutdown_rx,
    ));

    println!("Status LED test mode");
    println!("Commands: I=Idle, L=Listening, T=Transcribing, P=Processing, S=Speaking, Q=Quit");

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let line = line?;
                let Some(line) = line else {
                    break;
                };
                let cmd = line.trim();
                if cmd.is_empty() {
                    continue;
                }
                let state = match cmd.to_ascii_lowercase().as_str() {
                    "i" | "idle" => Some(RuntimeState::Idle),
                    "l" | "listening" => Some(RuntimeState::Listening),
                    "t" | "transcribing" => Some(RuntimeState::Transcribing),
                    "p" | "processing" => Some(RuntimeState::Processing),
                    "s" | "speaking" => Some(RuntimeState::Speaking),
                    "q" | "quit" | "exit" => break,
                    _ => {
                        println!("Unknown command '{}'. Use I/L/P/S or Q to quit.", cmd);
                        None
                    }
                };
                if let Some(state) = state {
                    let _ = status_tx.send(StatusSnapshot {
                        state,
                        mic_muted: false,
                        lid_open: true,
                    });
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    let _ = shutdown_tx.send(true);
    let _ = led_task.await;
    Ok(())
}

#[cfg(not(feature = "gpio"))]
async fn run_led_test(_pin: u8) -> Result<(), Box<dyn std::error::Error>> {
    println!("led-test requires the 'gpio' feature to be enabled.");
    Ok(())
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

fn parse_env_u8(name: &str) -> Option<u8> {
    let value = std::env::var(name).ok()?;
    match value.trim().parse::<u8>() {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::warn!("invalid {} value '{}': {}", name, value, err);
            None
        }
    }
}
