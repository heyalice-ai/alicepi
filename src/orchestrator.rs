use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use crate::config::ServerConfig;
use crate::engine::{
    build_engine, Engine, EngineAudio, EngineConfig, EngineError, EngineRequest, EngineResponse,
    SessionManager,
};
use crate::protocol::{
    ClientCommand, ServerReply, SpeechRecCommand, SpeechRecEvent, StatusSnapshot, VoiceInputCommand,
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

struct Orchestrator {
    state: State,
    mic_muted: bool,
    lid_open: bool,
    generation: Arc<AtomicU64>,
    engine: Arc<dyn Engine>,
    session: SessionManager,
    session_timeout: Duration,
    voice_input: CommandHandle<VoiceInputCommand>,
    speech_rec: CommandHandle<SpeechRecCommand>,
    voice_output: CommandHandle<VoiceOutputCommand>,
    internal_tx: mpsc::Sender<OrchestratorEvent>,
    status_tx: watch::Sender<StatusSnapshot>,
}

#[derive(Debug)]
enum OrchestratorEvent {
    EngineResponse {
        generation: u64,
        result: Result<EngineResponse, EngineError>,
        started_at: Instant,
    },
}

impl Orchestrator {
    fn new(
        engine: Arc<dyn Engine>,
        session_timeout: Duration,
        voice_input: CommandHandle<VoiceInputCommand>,
        speech_rec: CommandHandle<SpeechRecCommand>,
        voice_output: CommandHandle<VoiceOutputCommand>,
        internal_tx: mpsc::Sender<OrchestratorEvent>,
        status_tx: watch::Sender<StatusSnapshot>,
    ) -> Self {
        Self {
            state: State::Idle,
            mic_muted: true,
            lid_open: true,
            generation: Arc::new(AtomicU64::new(0)),
            engine,
            session: SessionManager::new(),
            session_timeout,
            voice_input,
            speech_rec,
            voice_output,
            internal_tx,
            status_tx,
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
            ClientCommand::Status => {}
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
                self.set_state(State::Speaking);
                let _ = self.voice_output.send(VoiceOutputCommand::PlayAudioFile { path }).await;
            }
            ClientCommand::AudioStreamStart { format } => {
                self.set_state(State::Speaking);
                let _ = self
                    .voice_output
                    .send(VoiceOutputCommand::StartStream { format })
                    .await;
            }
            ClientCommand::AudioStreamChunk { data } => {
                let _ = self.voice_output.send(VoiceOutputCommand::StreamChunk { data }).await;
            }
            ClientCommand::AudioStreamEnd => {
                let _ = self.voice_output.send(VoiceOutputCommand::EndStream).await;
            }
            ClientCommand::ButtonPress => {
                self.handle_button_press().await;
            }
            ClientCommand::ButtonRelease => {
                self.handle_button_release().await;
            }
            ClientCommand::LidOpen => {
                self.set_lid_open(true);
                self.session.start_new();
            }
            ClientCommand::LidClose => {
                self.set_lid_open(false);
                self.cancel_session().await;
                self.set_mic_muted(true);
                self.set_state(State::Idle);
            }
        }
    }

    async fn handle_button_press(&mut self) {
        self.cancel_session().await;
        self.set_mic_muted(false);
        self.set_state(State::Listening);
        let _ = self.voice_input.send(VoiceInputCommand::StartListening).await;
    }

    async fn handle_button_release(&mut self) {
        self.set_mic_muted(true);
        self.set_state(State::Idle);
        let _ = self.voice_input.send(VoiceInputCommand::StopListening).await;
    }

    async fn handle_voice_event(&mut self, event: VoiceInputEvent) {
        match event {
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
            OrchestratorEvent::EngineResponse {
                generation,
                result,
                started_at,
            } => {
                if self.generation.load(Ordering::SeqCst) == generation {
                    match result {
                        Ok(response) => {
                            if let Some(text) = response.assistant_text {
                                self.session.add_assistant_message(text);
                            } else {
                                self.session.add_assistant_placeholder();
                            }

                            self.set_state(State::Speaking);
                            match response.audio {
                                EngineAudio::Full(audio) => {
                                    let _ = self
                                        .voice_output
                                        .send(VoiceOutputCommand::PlayAudio { audio })
                                        .await;
                                }
                                EngineAudio::Stream(mut audio) => {
                                    let voice_output = self.voice_output.clone();
                                    let generation_ref = self.generation.clone();
                                    let started_at = started_at;
                                    tokio::spawn(async move {
                                        let mut logged_first_chunk = false;
                                        let mut chunk_count: u64 = 0;
                                        if voice_output
                                            .send(VoiceOutputCommand::StartStream {
                                                format: audio.format,
                                            })
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                        while let Some(chunk) = audio.stream.as_mut().next().await {
                                            if generation_ref.load(Ordering::SeqCst) != generation {
                                                let _ = voice_output
                                                    .send(VoiceOutputCommand::Stop)
                                                    .await;
                                                return;
                                            }
                                            match chunk {
                                                Ok(bytes) => {
                                                    chunk_count += 1;
                                                    if !logged_first_chunk {
                                                        let wait = started_at.elapsed();
                                                        tracing::info!(
                                                            "engine stream first chunk after {:.0}ms ({} bytes)",
                                                            wait.as_secs_f64() * 1000.0,
                                                            bytes.len()
                                                        );
                                                        logged_first_chunk = true;
                                                    }
                                                    
                                                    if voice_output
                                                        .send(VoiceOutputCommand::StreamChunk {
                                                            data: bytes.to_vec(),
                                                        })
                                                        .await
                                                        .is_err()
                                                    {
                                                        tracing::warn!(
                                                            "voice output stream closed unexpectedly"
                                                        );
                                                        return;
                                                    }
                                                }
                                                Err(err) => {
                                                    tracing::warn!(
                                                        "engine stream failed: {}",
                                                        err
                                                    );
                                                    let _ = voice_output
                                                        .send(VoiceOutputCommand::Stop)
                                                        .await;
                                                    return;
                                                }
                                            }
                                        }
                                        let _ = voice_output
                                            .send(VoiceOutputCommand::EndStream)
                                            .await;
                                    });
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!("engine request failed: {}", err);
                            self.set_state(State::Idle);
                        }
                    }
                } else {
                    tracing::info!("dropping stale engine response");
                }
            }
        }
    }

    async fn process_text(&mut self, text: String) {
        if !self.lid_open {
            tracing::info!("lid closed; ignoring text input");
            return;
        }

        if self.session.maybe_rollover(self.session_timeout) {
            tracing::info!("session timed out; starting new session");
        }

        self.session.add_user_message(&text);
        self.set_state(State::Processing);
        let generation = self.generation.load(Ordering::SeqCst);
        let started_at = Instant::now();
        let tx = self.internal_tx.clone();
        let engine = self.engine.clone();
        let history = self.session.history().to_vec();
        let session_id = self.session.id().to_string();
        tokio::spawn(async move {
            let request = EngineRequest {
                text: &text,
                history: &history,
                session_id: &session_id,
            };
            let result = engine.process(request).await;
            let _ = tx
                .send(OrchestratorEvent::EngineResponse {
                    generation,
                    result,
                    started_at,
                })
                .await;
        });
    }

    async fn cancel_session(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let _ = self.voice_output.send(VoiceOutputCommand::Stop).await;
        let _ = self.speech_rec.send(SpeechRecCommand::Reset).await;
        let _ = self.voice_input.send(VoiceInputCommand::StopListening).await;
    }

    fn set_state(&mut self, next: State) {
        if self.state != next {
            self.state = next;
            self.publish_status();
            tracing::info!(state = ?self.state, "state changed");
        }
    }

    fn set_mic_muted(&mut self, muted: bool) {
        if self.mic_muted != muted {
            self.mic_muted = muted;
            self.publish_status();
            tracing::info!(mic_muted = self.mic_muted, "mic state changed");
        }
    }

    fn set_lid_open(&mut self, open: bool) {
        if self.lid_open != open {
            self.lid_open = open;
            self.publish_status();
            tracing::info!(lid_open = self.lid_open, "lid state changed");
        }
    }

    fn publish_status(&self) {
        let _ = self.status_tx.send(StatusSnapshot {
            state: format!("{:?}", self.state),
            mic_muted: self.mic_muted,
            lid_open: self.lid_open,
        });
    }
}

pub async fn run_server(config: ServerConfig) -> Result<(), String> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (client_tx, client_rx) = mpsc::channel(64);
    let (internal_tx, internal_rx) = mpsc::channel(16);
    let (status_tx, status_rx) = watch::channel(StatusSnapshot {
        state: format!("{:?}", State::Idle),
        mic_muted: true,
        lid_open: true,
    });

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

    let save_request_wavs_dir = config.save_request_wavs_dir.clone();
    let speech_rec_supervisor = watchdog::supervise(
        "speech_rec",
        speech_rec_handle.clone(),
        Some((speech_rec_tx, speech_rec_rx)),
        32,
        config.watchdog_timeout,
        shutdown_rx.clone(),
        move |rx, heartbeat, shutdown| {
            let events = sr_events_tx.clone();
            let save_request_wavs_dir = save_request_wavs_dir.clone();
            async move {
                tasks::speech_rec::run(rx, events, heartbeat, shutdown, save_request_wavs_dir).await
            }
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

    let server_task = tcp_server(
        config.bind_addr.clone(),
        client_tx.clone(),
        status_rx.clone(),
        shutdown_rx.clone(),
    );

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

    let engine = build_engine(EngineConfig::from_env(config.stream_audio))
        .map_err(|err| format!("engine init failed: {}", err))?;
    let session_timeout = session_timeout_from_env();
    let mut orchestrator = Orchestrator::new(
        engine,
        session_timeout,
        voice_input_handle,
        speech_rec_handle,
        voice_output_handle,
        internal_tx,
        status_tx,
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

fn session_timeout_from_env() -> Duration {
    let value = env::var("SESSION_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(60.0);
    Duration::from_secs_f32(value.max(0.0))
}

async fn tcp_server(
    bind_addr: String,
    client_tx: mpsc::Sender<ClientCommand>,
    status_rx: watch::Receiver<StatusSnapshot>,
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
                        let status = status_rx.clone();
                        tokio::spawn(async move { handle_connection(stream, tx, status).await; });
                    }
                    Err(err) => {
                        tracing::warn!("accept error: {}", err);
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    client_tx: mpsc::Sender<ClientCommand>,
    status_rx: watch::Receiver<StatusSnapshot>,
) {
    let (reader, mut writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let reply = match serde_json::from_str::<ClientCommand>(&line) {
            Ok(command) => {
                if let ClientCommand::Status = command {
                    let status = status_rx.borrow().clone();
                    ServerReply::Status { status }
                } else {
                    let _ = client_tx.send(command).await;
                    ServerReply::Ok {
                        message: "accepted".to_string(),
                    }
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
