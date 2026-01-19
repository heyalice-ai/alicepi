use tokio::sync::{mpsc, watch};

use crate::protocol::ClientCommand;

#[derive(Debug, Clone)]
pub struct GpioConfig {
    pub button_pin: Option<u8>,
    pub lid_pin: Option<u8>,
}

pub async fn run(
    config: GpioConfig,
    sender: mpsc::Sender<ClientCommand>,
    shutdown: watch::Receiver<bool>,
) {
    if config.button_pin.is_none() && config.lid_pin.is_none() {
        return;
    }

    #[cfg(feature = "gpio")]
    {
        use std::time::Duration;

        use rppal::gpio::{Gpio, Level};
        use tokio::time;

        let mut shutdown = shutdown;
        let gpio = match Gpio::new() {
            Ok(gpio) => gpio,
            Err(err) => {
                tracing::warn!("gpio unavailable: {}", err);
                return;
            }
        };

        let button: Option<rppal::gpio::InputPin> = match config.button_pin {
            Some(pin) => match gpio.get(pin).map(|p| p.into_input_pullup()) {
                Ok(pin) => Some(pin),
                Err(err) => {
                    tracing::warn!("failed to init button pin {}: {}", pin, err);
                    None
                }
            },
            None => None,
        };

        let lid: Option<rppal::gpio::InputPin> = match config.lid_pin {
            Some(pin) => match gpio.get(pin).map(|p| p.into_input_pullup()) {
                Ok(pin) => Some(pin),
                Err(err) => {
                    tracing::warn!("failed to init lid pin {}: {}", pin, err);
                    None
                }
            },
            None => None,
        };

        let mut last_button_level = button.as_ref().map(|p| p.read());
        let mut last_lid_level = lid.as_ref().map(|p| p.read());

        let mut tick = time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    break;
                }
                _ = tick.tick() => {
                    if let Some(pin) = button.as_ref() {
                        let level = pin.read();
                        if Some(level) != last_button_level {
                            last_button_level = Some(level);
                            if level == Level::Low {
                                let _ = sender.send(ClientCommand::ButtonPress).await;
                            } else {
                                let _ = sender.send(ClientCommand::ButtonRelease).await;
                            }
                        }
                    }

                    if let Some(pin) = lid.as_ref() {
                        let level = pin.read();
                        if Some(level) != last_lid_level {
                            last_lid_level = Some(level);
                            match level {
                                Level::Low => {
                                    let _ = sender.send(ClientCommand::LidClose).await;
                                }
                                Level::High => {
                                    let _ = sender.send(ClientCommand::LidOpen).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "gpio"))]
    {
        let _ = sender;
        let _ = config;
        let _ = shutdown;
        tracing::info!("gpio feature disabled; skipping gpio watcher");
    }
}
