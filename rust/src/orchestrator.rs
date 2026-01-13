use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use crate::config::ServerConfig;
use crate::protocol::{
    ClientCommand, ServerReply, SpeechRecCommand, SpeechRecEvent, VoiceInputCommand,
    VoiceInputEvent, VoiceOutputCommand,
};
use crate::tasks;
use crate::watchdog::{self, CommandHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Listening,
    Processing,
    Speaking,
}

#[derive(Debug)]
struct Orchestrator {
    state: State,
    mic_muted: bool,
    lid_open: bool,
    generation: Arc<AtomicU64>,
    voice_input: CommandHandle<VoiceInputCommand>,
    speech_rec: CommandHandle<SpeechRecCommand>,
    voice_output: CommandHandle<VoiceOutputCommand>,
    internal_tx: mpsc::Sender<OrchestratorEvent>,
}

#[derive(Debug)]
enum OrchestratorEvent {
    EngineResponse { generation: u64, response: String },
}

impl Orchestrator {
    fn new(
        voice_input: CommandHandle<VoiceInputCommand>,
        speech_rec: CommandHandle<SpeechRecCommand>,
        voice_output: CommandHandle<VoiceOutputCommand>,
        internal_tx: mpsc::Sender<OrchestratorEvent>,
    ) -> Self {
        Self {
            state: State::Idle,
            mic_muted: true,
            lid_open: true,
            generation: Arc::new(AtomicU64::new(0)),
            voice_input,
            speech_rec,
            voice_output,
            internal_tx,
        }
    }

    async fn run(
        &mut self,
        mut client_rx: mpsc::Receiver<ClientCommand>,
        mut voice_events: broadcast::Receiver<VoiceInputEvent>,
        mut sr_events: broadcast::Receiver<SpeechRecEvent>,
        mut internal_rx: mpsc::Receiver<OrchestratorEvent>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    break;
                }
                command = client_rx.recv() => {
                    if let Some(command) = command {
                        self.handle_client_command(command).await;
                    } else {
                        break;
                    }
                }
                event = voice_events.recv() => {
                    match event {
                        Ok(event) => self.handle_voice_event(event).await,
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            tracing::warn!("voice input events lagged by {}", count);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                event = sr_events.recv() => {
                    match event {
                        Ok(event) => self.handle_speech_event(event).await,
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            tracing::warn!("speech rec events lagged by {}", count);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                event = internal_rx.recv() => {
                    if let Some(event) = event {
                        self.handle_internal_event(event).await;
                    }
                }
            }
        }
    }

    async fn handle_client_command(&mut self, command: ClientCommand) {
        match command {
            ClientCommand::Ping => {
                tracing::info!("client ping");
            }
            ClientCommand::Text { text } => {
                self.process_text(text).await;
            }
            ClientCommand::VoiceFile { path } => {
                if !self.mic_muted {
                    let _ = self
                        .voice_input
                        .send(VoiceInputCommand::InjectAudioFile { path })
                        .await;
                } else {
                    tracing::info!("ignoring voice input while mic muted");
                }
            }
            ClientCommand::AudioFile { path } => {
                self.state = State::Speaking;
                let _ = self.voice_output.send(VoiceOutputCommand::PlayAudioFile { path }).await;
            }
            ClientCommand::ButtonPress => {
                self.handle_button_press().await;
            }
            ClientCommand::LidOpen => {
                self.lid_open = true;
                tracing::info!("lid open");
            }
            ClientCommand::LidClose => {
                self.lid_open = false;
                tracing::info!("lid closed");
                #[cfg(feature = "lid_control")]
                {
                    self.cancel_session().await;
                    self.mic_muted = true;
                    self.state = State::Idle;
                }
            }
        }
    }

    async fn handle_button_press(&mut self) {
        self.cancel_session().await;
        self.mic_muted = false;
        self.state = State::Listening;
        let _ = self.voice_input.send(VoiceInputCommand::StartListening).await;
    }

    async fn handle_voice_event(&mut self, event: VoiceInputEvent) {
        match event {
            VoiceInputEvent::VadSpeech => {
                if self.state == State::Idle {
                    self.state = State::Listening;
                }
            }
            VoiceInputEvent::VadSilence => {
                self.mic_muted = true;
                self.state = State::Idle;
                let _ = self.voice_input.send(VoiceInputCommand::StopListening).await;
            }
            VoiceInputEvent::AudioChunk(chunk) => {
                let _ = self.speech_rec.send(SpeechRecCommand::AudioChunk(chunk)).await;
            }
            VoiceInputEvent::AudioEnded => {
                let _ = self.speech_rec.send(SpeechRecCommand::AudioEnded).await;
            }
        }
    }

    async fn handle_speech_event(&mut self, event: SpeechRecEvent) {
        match event {
            SpeechRecEvent::Text { text, is_final } => {
                if is_final {
                    self.process_text(text).await;
                }
            }
        }
    }

    async fn handle_internal_event(&mut self, event: OrchestratorEvent) {
        match event {
            OrchestratorEvent::EngineResponse { generation, response } => {
                if self.generation.load(Ordering::SeqCst) == generation {
                    self.state = State::Speaking;
                    let _ = self
                        .voice_output
                        .send(VoiceOutputCommand::PlayText { text: response })
                        .await;
                } else {
                    tracing::info!("dropping stale engine response");
                }
            }
        }
    }

    async fn process_text(&mut self, text: String) {
        if cfg!(feature = "lid_control") && !self.lid_open {
            tracing::info!("lid closed; ignoring text input");
            return;
        }

        self.state = State::Processing;
        let generation = self.generation.load(Ordering::SeqCst);
        let tx = self.internal_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let response = format!("You said: {}", text.trim());
            let _ = tx
                .send(OrchestratorEvent::EngineResponse { generation, response })
                .await;
        });
    }

    async fn cancel_session(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.voice_output.send(VoiceOutputCommand::Stop).await;
        let _ = self.speech_rec.send(SpeechRecCommand::Reset).await;
        let _ = self.voice_input.send(VoiceInputCommand::StopListening).await;
    }
}

pub async fn run_server(config: ServerConfig) -> Result<(), String> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (client_tx, client_rx) = mpsc::channel(64);
    let (internal_tx, internal_rx) = mpsc::channel(16);

    let (voice_events_tx, voice_events_rx) = broadcast::channel(32);
    let (sr_events_tx, sr_events_rx) = broadcast::channel(32);

    let (voice_input_tx, voice_input_rx) = mpsc::channel(32);
    let voice_input_handle = CommandHandle::new(voice_input_tx.clone());

    let voice_input_supervisor = watchdog::supervise(
        "voice_input",
        voice_input_handle.clone(),
        Some((voice_input_tx, voice_input_rx)),
        32,
        config.watchdog_timeout,
        shutdown_rx.clone(),
        move |rx, heartbeat, shutdown| {
            let events = voice_events_tx.clone();
            async move { tasks::voice_input::run(rx, events, heartbeat, shutdown).await }
        },
    );

    let (speech_rec_tx, speech_rec_rx) = mpsc::channel(32);
    let speech_rec_handle = CommandHandle::new(speech_rec_tx.clone());

    let speech_rec_supervisor = watchdog::supervise(
        "speech_rec",
        speech_rec_handle.clone(),
        Some((speech_rec_tx, speech_rec_rx)),
        32,
        config.watchdog_timeout,
        shutdown_rx.clone(),
        move |rx, heartbeat, shutdown| {
            let events = sr_events_tx.clone();
            async move { tasks::speech_rec::run(rx, events, heartbeat, shutdown).await }
        },
    );

    let (voice_output_handle, _voice_output_join) = watchdog::spawn_task(
        32,
        |rx, shutdown| async move { tasks::voice_output::run(rx, shutdown).await },
        shutdown_rx.clone(),
    )
    .await;

    let gpio_task = tasks::gpio::run(
        tasks::gpio::GpioConfig {
            button_pin: config.gpio_button_pin,
            lid_pin: config.gpio_lid_pin,
        },
        client_tx.clone(),
        shutdown_rx.clone(),
    );

    let server_task = tcp_server(config.bind_addr.clone(), client_tx.clone(), shutdown_rx.clone());

    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to listen for ctrl-c: {}", err);
        }
        let _ = shutdown_tx.send(true);
    });

    tokio::spawn(voice_input_supervisor);
    tokio::spawn(speech_rec_supervisor);
    let _ = _voice_output_join;
    tokio::spawn(gpio_task);
    tokio::spawn(server_task);

    let mut orchestrator = Orchestrator::new(
        voice_input_handle,
        speech_rec_handle,
        voice_output_handle,
        internal_tx,
    );

    orchestrator
        .run(
            client_rx,
            voice_events_rx,
            sr_events_rx,
            internal_rx,
            shutdown_rx.clone(),
        )
        .await;

    Ok(())
}

async fn tcp_server(
    bind_addr: String,
    client_tx: mpsc::Sender<ClientCommand>,
    mut shutdown: watch::Receiver<bool>,
) {
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("failed to bind {}: {}", bind_addr, err);
            return;
        }
    };

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _)) => {
                        let tx = client_tx.clone();
                        tokio::spawn(async move { handle_connection(stream, tx).await; });
                    }
                    Err(err) => {
                        tracing::warn!("accept error: {}", err);
                    }
                }
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, client_tx: mpsc::Sender<ClientCommand>) {
    let (reader, mut writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match serde_json::from_str::<ClientCommand>(&line) {
            Ok(command) => {
                let _ = client_tx.send(command).await;
                ServerReply::Ok {
                    message: "accepted".to_string(),
                }
            }
            Err(err) => ServerReply::Error {
                message: format!("invalid command: {}", err),
            },
        };

        let payload = match serde_json::to_string(&reply) {
            Ok(payload) => payload,
            Err(err) => format!("{{\"type\":\"error\",\"message\":\"{}\"}}", err),
        };

        if writer.write_all(payload.as_bytes()).await.is_err() {
            break;
        }
        if writer.write_all(b"\n").await.is_err() {
            break;
        }
    }
}
