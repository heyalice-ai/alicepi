use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub watchdog_timeout: Duration,
    pub gpio_button_pin: Option<u8>,
    pub gpio_lid_pin: Option<u8>,
    pub stream_audio: bool,
    pub save_request_wavs_dir: Option<PathBuf>,
}

impl ServerConfig {
    pub fn default_bind() -> String {
        "127.0.0.1:7878".to_string()
    }
}
