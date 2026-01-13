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
        #[arg(long)]
        gpio_button: Option<u8>,
        #[arg(long)]
        gpio_lid: Option<u8>,
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
    Text { text: String },
    Voice { path: String },
    Audio { path: String },
    Button,
    LidOpen,
    LidClose,
}
