#[cfg(feature = "gpio")]
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use crate::protocol::{ClientCommand, RuntimeState, StatusSnapshot};

#[derive(Debug, Clone)]
pub struct GpioConfig {
    pub button_pin: Option<u8>,
    pub lid_pin: Option<u8>,
    pub status_led_pin: Option<u8>,
}

pub async fn run(
    config: GpioConfig,
    sender: mpsc::Sender<ClientCommand>,
    shutdown: watch::Receiver<bool>,
    status_rx: watch::Receiver<StatusSnapshot>,
) {
    if config.button_pin.is_none() && config.lid_pin.is_none() && config.status_led_pin.is_none() {
        return;
    }

    #[cfg(feature = "gpio")]
    {
        use std::time::{Duration, Instant};

        use rppal::gpio::{Gpio, Level, OutputPin};
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

        let status_led: Option<OutputPin> = match config.status_led_pin {
            Some(pin) => match gpio.get(pin).map(|p| p.into_output_low()) {
                Ok(pin) => Some(pin),
                Err(err) => {
                    tracing::warn!("failed to init status led pin {}: {}", pin, err);
                    None
                }
            },
            None => None,
        };

        if let Some(pin) = status_led {
            let led_config = StatusLedConfig::from_env();
            let led_shutdown = shutdown.clone();
            let led_status = status_rx.clone();
            tokio::spawn(async move {
                run_status_led(pin, led_status, led_shutdown, led_config).await;
            });
        }

        if button.is_none() && lid.is_none() {
            let _ = shutdown.changed().await;
            return;
        }

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
        let _ = status_rx;
        tracing::info!("gpio feature disabled; skipping gpio watcher");
    }
}

#[cfg(feature = "gpio")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedMode {
    Fixed,
    Pulse,
}

#[cfg(feature = "gpio")]
#[derive(Debug, Clone, Copy)]
struct StatusLedConfig {
    idle_brightness: f32,
    listening_brightness: f32,
    max_brightness: f32,
    processing_cycle: Duration,
    transition_time: Duration,
    pwm_hz: u32,
    pulse_while_speaking: bool,
}

#[cfg(feature = "gpio")]
impl StatusLedConfig {
    fn from_env() -> Self {
        let idle_brightness = env_brightness("GPIO_STATUS_LED_IDLE_BRIGHT", 0.2);
        let listening_brightness = env_brightness("GPIO_STATUS_LED_LISTENING_BRIGHT", 0.3);
        let max_brightness = env_brightness("GPIO_STATUS_LED_MAX_BRIGHT", 0.8);
        let mut processing_cycle =
            env_duration_seconds("GPIO_STATUS_LED_PROCESSING_CYCLE_TIME", 3.0);
        if processing_cycle.is_zero() {
            processing_cycle = Duration::from_secs_f32(3.0);
        }
        let transition_time = env_duration_seconds("GPIO_STATUS_LED_TRANSITION_TIME", 0.5);
        let pwm_hz = env_u32("GPIO_STATUS_LED_PWM_HZ", 800).max(1);
        let pulse_while_speaking = env_bool("GPIO_STATUS_LED_PULSE_WHILE_SPEAKING", false);
        Self {
            idle_brightness,
            listening_brightness,
            max_brightness,
            processing_cycle,
            transition_time,
            pwm_hz,
            pulse_while_speaking,
        }
    }
}

#[cfg(feature = "gpio")]
async fn run_status_led(
    mut pin: rppal::gpio::OutputPin,
    mut status_rx: watch::Receiver<StatusSnapshot>,
    mut shutdown: watch::Receiver<bool>,
    config: StatusLedConfig,
) {
    use std::time::Instant;

    let pwm_period = Duration::from_secs_f32(1.0 / config.pwm_hz as f32);
    let mut current = 0.0f32;
    let mut target = 0.0f32;
    let mut last_logged_target = -1.0f32;
    let mut last_logged_mode = LedMode::Fixed;
    let mut mode = LedMode::Fixed;
    let mut pulse_high = false;
    let mut next_pulse_switch = Instant::now() + config.processing_cycle;
    let mut last_update = Instant::now();

    let initial_status = status_rx.borrow().clone();
    apply_state_target(
        &initial_status,
        &config,
        &mut mode,
        &mut target,
        &mut pulse_high,
        &mut next_pulse_switch,
    );
    current = target;
    last_logged_target = target;
    last_logged_mode = mode;
    tracing::trace!(
        duty = current,
        target = target,
        mode = ?mode,
        "status led initialized"
    );

    loop {
        if shutdown.has_changed().unwrap_or(false) {
            if *shutdown.borrow_and_update() {
                break;
            }
        }

        if status_rx.has_changed().unwrap_or(false) {
            let status = status_rx.borrow_and_update().clone();
            apply_state_target(
                &status,
                &config,
                &mut mode,
                &mut target,
                &mut pulse_high,
                &mut next_pulse_switch,
            );
            if (target - last_logged_target).abs() > f32::EPSILON || mode != last_logged_mode {
                tracing::trace!(
                    state = %status.state,
                    duty = current,
                    target = target,
                    mode = ?mode,
                    "status led target updated"
                );
                last_logged_target = target;
                last_logged_mode = mode;
            }
        }

        let now = Instant::now();
        if mode == LedMode::Pulse && now >= next_pulse_switch {
            while next_pulse_switch <= now {
                pulse_high = !pulse_high;
                next_pulse_switch += config.processing_cycle;
            }
            target = if pulse_high {
                config.max_brightness
            } else {
                config.idle_brightness
            };
            if (target - last_logged_target).abs() > f32::EPSILON {
                tracing::trace!(
                    duty = current,
                    target = target,
                    mode = ?mode,
                    "status led pulse target updated"
                );
                last_logged_target = target;
            }
        }

        let dt = now.saturating_duration_since(last_update);
        current = step_toward(current, target, dt, config.transition_time);
        last_update = now;
        let duty = current.clamp(0.0, 1.0);
        if duty <= 0.0 {
            tracing::trace!(
                duty = duty,
                target = target,
                mode = ?mode,
                "status led pwm cycle (off)"
            );
            pin.set_low();
            tokio::time::sleep(pwm_period).await;
            continue;
        }
        if duty >= 1.0 {
            tracing::trace!(
                duty = duty,
                target = target,
                mode = ?mode,
                "status led pwm cycle (on)"
            );
            pin.set_high();
            tokio::time::sleep(pwm_period).await;
            continue;
        }

        let on_time = pwm_period.mul_f32(duty);
        let off_time = pwm_period.saturating_sub(on_time);
        tracing::trace!(
            duty = duty,
            target = target,
            mode = ?mode,
            on_ms = on_time.as_secs_f32() * 1000.0,
            off_ms = off_time.as_secs_f32() * 1000.0,
            "status led pwm cycle"
        );
        pin.set_high();
        tokio::time::sleep(on_time).await;
        pin.set_low();
        tokio::time::sleep(off_time).await;
    }

    pin.set_low();
}

#[cfg(feature = "gpio")]
fn apply_state_target(
    status: &StatusSnapshot,
    config: &StatusLedConfig,
    mode: &mut LedMode,
    target: &mut f32,
    pulse_high: &mut bool,
    next_pulse_switch: &mut Instant,
) {
    let desired_mode = desired_led_mode(status.state, config.pulse_while_speaking);
    match desired_mode {
        LedMode::Pulse => {
            if *mode != LedMode::Pulse {
                *mode = LedMode::Pulse;
                *pulse_high = false;
                *target = config.idle_brightness;
                *next_pulse_switch = Instant::now() + config.processing_cycle;
            }
        }
        LedMode::Fixed => {
            *mode = LedMode::Fixed;
            *target = fixed_target_for_state(status.state, config);
        }
    }
}

#[cfg(feature = "gpio")]
fn desired_led_mode(state: RuntimeState, pulse_while_speaking: bool) -> LedMode {
    match state {
        RuntimeState::Processing => LedMode::Pulse,
        RuntimeState::Speaking if pulse_while_speaking => LedMode::Pulse,
        _ => LedMode::Fixed,
    }
}

#[cfg(feature = "gpio")]
fn fixed_target_for_state(state: RuntimeState, config: &StatusLedConfig) -> f32 {
    match state {
        RuntimeState::Listening => config.listening_brightness,
        RuntimeState::Speaking => config.max_brightness,
        RuntimeState::Processing => config.idle_brightness,
        _ => config.idle_brightness,
    }
}

#[cfg(feature = "gpio")]
fn step_toward(current: f32, target: f32, dt: Duration, transition_time: Duration) -> f32 {
    let transition_secs = transition_time.as_secs_f32();
    if transition_secs <= 0.0 {
        return target;
    }
    let step = (dt.as_secs_f32() / transition_secs).min(1.0);
    current + (target - current) * step
}

#[cfg(feature = "gpio")]
fn env_brightness(name: &str, default: f32) -> f32 {
    let mut value = env_f32(name, default);
    if value > 1.0 {
        value /= 100.0;
    }
    value.clamp(0.0, 1.0)
}

#[cfg(feature = "gpio")]
fn env_duration_seconds(name: &str, default: f32) -> Duration {
    use std::time::Duration;

    let value = env_f32(name, default).max(0.0);
    Duration::from_secs_f32(value)
}

#[cfg(feature = "gpio")]
fn env_u32(name: &str, default: u32) -> u32 {
    let value = std::env::var(name).ok();
    match value.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => match raw.parse::<u32>() {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("invalid {} value '{}': {}", name, raw, err);
                default
            }
        },
        _ => default,
    }
}

#[cfg(feature = "gpio")]
fn env_bool(name: &str, default: bool) -> bool {
    let value = std::env::var(name).ok();
    match value.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                tracing::warn!("invalid {} value '{}': expected bool", name, raw);
                default
            }
        },
        _ => default,
    }
}

#[cfg(feature = "gpio")]
fn env_f32(name: &str, default: f32) -> f32 {
    let value = std::env::var(name).ok();
    match value.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => match raw.parse::<f32>() {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!("invalid {} value '{}': {}", name, raw, err);
                default
            }
        },
        _ => default,
    }
}
