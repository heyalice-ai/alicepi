mod cli;
mod config;
mod engine;
mod orchestrator;
mod protocol;
mod tasks;
mod watchdog;

use std::time::Duration;

use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;

use crate::cli::{ClientAction, Command, Cli};
use crate::config::ServerConfig;
use crate::protocol::{ClientCommand, ServerReply};

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
            gpio_button,
            gpio_lid,
        } => {
            let config = ServerConfig {
                bind_addr: bind,
                watchdog_timeout: Duration::from_millis(watchdog_ms),
                gpio_button_pin: gpio_button,
                gpio_lid_pin: gpio_lid,
            };
            orchestrator::run_server(config).await.map_err(|err| err.into())
        }
        Command::Client { addr, action } => {
            let command = match action {
    ClientAction::Ping => ClientCommand::Ping,
    ClientAction::Status => ClientCommand::Status,
                ClientAction::Text { text } => ClientCommand::Text { text },
                ClientAction::Voice { path } => ClientCommand::VoiceFile { path },
                ClientAction::Audio { path } => ClientCommand::AudioFile { path },
                ClientAction::Button => ClientCommand::ButtonPress,
                ClientAction::LidOpen => ClientCommand::LidOpen,
                ClientAction::LidClose => ClientCommand::LidClose,
            };

            let reply = send_command(&addr, command).await?;
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
    }
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
