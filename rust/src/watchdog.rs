use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch, RwLock};
use tokio::task::JoinHandle;
use tokio::time;

#[derive(Clone, Debug)]
pub struct CommandHandle<T> {
    sender: Arc<RwLock<mpsc::Sender<T>>>,
}

impl<T> CommandHandle<T> {
    pub fn new(sender: mpsc::Sender<T>) -> Self {
        Self {
            sender: Arc::new(RwLock::new(sender)),
        }
    }

    pub async fn send(&self, command: T) -> Result<(), mpsc::error::SendError<T>> {
        let sender = self.sender.read().await;
        sender.send(command).await
    }

    pub async fn replace(&self, sender: mpsc::Sender<T>) {
        let mut guard = self.sender.write().await;
        *guard = sender;
    }
}

#[derive(Clone)]
pub struct Heartbeat {
    sender: watch::Sender<Instant>,
}

impl Heartbeat {
    pub fn new() -> (Self, watch::Receiver<Instant>) {
        let (sender, receiver) = watch::channel(Instant::now());
        (Self { sender }, receiver)
    }

    pub fn tick(&self) {
        let _ = self.sender.send(Instant::now());
    }
}

pub async fn supervise<T, F, Fut>(
    name: &'static str,
    handle: CommandHandle<T>,
    mut initial: Option<(mpsc::Sender<T>, mpsc::Receiver<T>)>,
    buffer: usize,
    heartbeat_timeout: Duration,
    mut shutdown: watch::Receiver<bool>,
    mut spawn: F,
) where
    T: Send + 'static,
    F: FnMut(mpsc::Receiver<T>, Heartbeat, watch::Receiver<bool>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        if *shutdown.borrow() {
            break;
        }

        let (tx, rx) = if let Some((tx, rx)) = initial.take() {
            (tx, rx)
        } else {
            mpsc::channel(buffer)
        };
        handle.replace(tx).await;

        let (heartbeat, mut heartbeat_rx) = Heartbeat::new();
        let task_shutdown = shutdown.clone();
        let mut join = tokio::spawn(spawn(rx, heartbeat, task_shutdown));

        let mut interval = time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    join.abort();
                    return;
                }
                _ = interval.tick() => {
                    let last = *heartbeat_rx.borrow();
                    if last.elapsed() > heartbeat_timeout {
                        tracing::warn!(task = name, "watchdog timeout, restarting task");
                        join.abort();
                        break;
                    }
                }
                result = &mut join => {
                    if result.is_err() {
                        tracing::warn!(task = name, "task aborted, restarting");
                    } else {
                        tracing::warn!(task = name, "task exited, restarting");
                    }
                    break;
                }
                _ = heartbeat_rx.changed() => {
                    // heartbeat updated
                }
            }
        }
    }
}

pub async fn spawn_task<T, F, Fut>(
    buffer: usize,
    mut spawn: F,
    shutdown: watch::Receiver<bool>,
) -> (CommandHandle<T>, JoinHandle<()>)
where
    T: Send + 'static,
    F: FnMut(mpsc::Receiver<T>, watch::Receiver<bool>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::channel(buffer);
    let handle = CommandHandle::new(tx);
    let join = tokio::spawn(spawn(rx, shutdown));
    (handle, join)
}
