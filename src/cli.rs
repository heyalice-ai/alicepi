use clap::{Parser, Subcommand};

use crate::config::ServerConfig;

#[derive(Parser, Debug)]
#[command(name = "alicepi", version, about = "AlicePi Rust runtime")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Server {
        #[arg(long, default_value_t = ServerConfig::default_bind())]
        bind: String,
        #[arg(long, default_value_t = 3000)]
        watchdog_ms: u64,
        #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "no_stream")]
        stream: bool,
        #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "stream")]
        no_stream: bool,
        #[arg(long)]
        gpio_button: Option<u8>,
        #[arg(long)]
        gpio_lid: Option<u8>,
        #[arg(long, value_name = "DIR")]
        save_request_wavs: Option<String>,
    },
    Client {
        #[arg(long, default_value_t = ServerConfig::default_bind())]
        addr: String,
        #[command(subcommand)]
        action: ClientAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum ClientAction {
    Ping,
    #[command(about = "Fetch current runtime status (state, mic, lid)")]
    Status,
    Text { text: String },
    #[command(about = "Inject an audio file into the voice input pipeline (VAD -> SR -> response)")]
    Voice { path: String },
    #[command(about = "Play an audio file directly through voice output (no recognition)")]
    Audio { path: String },
    #[command(about = "Stream an MP3 file through voice output in chunks")]
    AudioStream {
        path: String,
        #[arg(long, default_value_t = 8192)]
        chunk_bytes: usize,
        #[arg(long, default_value_t = 0)]
        delay_after_bytes: usize,
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    Button,
    LidOpen,
    LidClose,
}
