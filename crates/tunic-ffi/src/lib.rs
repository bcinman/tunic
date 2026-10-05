//! Foreign-language application boundary. Only commands, snapshots, and analysis
//! cross FFI; the real-time processor remains entirely inside Rust/Core Audio.

mod types;
pub use types::*;

use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use boltffi::{EventSubscription, export};
use tunic_core::{Command, MemoryPersistence, Session, Telemetry};
use tunic_presets::BundledCatalog;

enum Message {
    Execute(Command),
    Refresh,
    Telemetry(bool),
    Shutdown,
}

#[derive(Default)]
struct Updates {
    closed: bool,
    listeners: Vec<Weak<EventSubscription<u64>>>,
}

#[derive(Default)]
struct Measurements {
    generation: u64,
    subscription: Option<Telemetry>,
}

#[derive(Default)]
struct Shared {
    snapshot: Mutex<Option<StateSnapshot>>,
    updates: Mutex<Updates>,
    measurements: Mutex<Measurements>,
}

impl Shared {
    fn publish(&self, session: &Session, revision: u64) {
        *self.snapshot.lock().unwrap() = Some(StateSnapshot::read(session, revision));
        let listeners = {
            let mut updates = self.updates.lock().unwrap();
            updates
                .listeners
                .retain(|listener| listener.strong_count() > 0);
            updates
                .listeners
                .iter()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        // Waking foreign continuations can run code inline. Hold no locks here.
        for listener in listeners {
            listener.push_event(revision);
        }
    }

    fn close(&self) {
        *self.snapshot.lock().unwrap() = None;
        self.measurements.lock().unwrap().subscription = None;
        let listeners = {
            let mut updates = self.updates.lock().unwrap();
            updates.closed = true;
            std::mem::take(&mut updates.listeners)
        };
        for listener in listeners.into_iter().filter_map(|s| s.upgrade()) {
            listener.unsubscribe();
        }
    }
}

/// One application session, shared by all views. Native resources never leave
/// its owner thread. Dropping the handle or calling shutdown joins that thread.
pub struct Engine {
    sender: mpsc::Sender<Message>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[export]
impl Engine {
    /// Offline mode is useful for previews/tests and hosts without an audio adapter.
    pub fn new(connect_audio: bool) -> Result<Self, EngineError> {
        #[cfg(not(target_os = "macos"))]
        if connect_audio {
            return Err(EngineError::Unavailable {
                message: "No audio adapter for this platform".into(),
            });
        }
        let (sender, receiver) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let notify_sender = sender.clone();
        let worker = thread::Builder::new()
            .name("tunic-session".into())
            .spawn(move || {
                let session = Session::new(MemoryPersistence::default(), BundledCatalog)
                    .expect("the in-memory store starts with valid state");
                #[cfg(target_os = "macos")]
                let session = if connect_audio {
                    session.with_audio(
                        tunic_macos::MacosPlatform::default(),
                        Arc::new(move || {
                            let _ = notify_sender.send(Message::Refresh);
                        }),
                    )
                } else {
                    session
                };
                #[cfg(not(target_os = "macos"))]
                let _ = notify_sender;
                run(session, receiver, &worker_shared);
            })
            .map_err(|e| EngineError::Unavailable {
                message: e.to_string(),
            })?;
        Ok(Self {
            sender,
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }

    /// Success means queued, not executed. Input validation errors throw immediately.
    /// Execution errors appear in the latest snapshot and may be superseded by a
    /// subsequent successful command before observation. Revisions are not acknowledgements.
    pub fn enqueue(&self, command: EngineCommand) -> Result<(), EngineError> {
        self.sender
            .send(Message::Execute(command.try_into()?))
            .map_err(|_| EngineError::Closed)
    }

    /// None while initializing or after shutdown. Owned values cannot mutate Session.
    pub fn snapshot(&self) -> Option<StateSnapshot> {
        self.shared.snapshot.lock().unwrap().clone()
    }

    /// Invalidations, not a history of snapshots. Always reread snapshot on receipt.
    /// The Rust buffer coalesces at capacity one; generated bindings choose their
    /// own delivery buffering (BoltFFI 0.30.1 uses an unbounded Swift AsyncStream).
    #[ffi_stream(item = u64)]
    pub fn updates(&self) -> Arc<EventSubscription<u64>> {
        let subscription = Arc::new(EventSubscription::new(1));
        let mut updates = self.shared.updates.lock().unwrap();
        if updates.closed {
            subscription.unsubscribe();
        } else {
            subscription.push_event(0);
            updates.listeners.push(Arc::downgrade(&subscription));
        }
        subscription
    }

    /// One telemetry demand per engine. Enable while the analysis UI is visible;
    /// disabling drops the core subscription and stops unnecessary FFT work.
    pub fn set_telemetry_enabled(&self, enabled: bool) -> Result<(), EngineError> {
        self.sender
            .send(Message::Telemetry(enabled))
            .map_err(|_| EngineError::Closed)
    }

    /// Drains at most core's retained history (16 frames). Reset animation when
    /// audio_generation changes. Never touches the Session or audio callback.
    pub fn poll_telemetry(&self) -> TelemetryBatch {
        let measurements = self.shared.measurements.lock().unwrap();
        let mut frames = Vec::new();
        if let Some(subscription) = &measurements.subscription {
            subscription.for_each_unseen(|frame| frames.push(frame.into()));
        }
        TelemetryBatch {
            audio_generation: measurements.generation,
            frames,
        }
    }

    /// Idempotent. Returns only after the owner has torn down the native route.
    pub fn shutdown(&self) {
        let mut worker = self.worker.lock().unwrap();
        if let Some(worker) = worker.take() {
            let _ = self.sender.send(Message::Shutdown);
            let _ = worker.join();
            self.shared.close();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(mut session: Session, receiver: mpsc::Receiver<Message>, shared: &Shared) {
    let mut revision = 1;
    let mut telemetry_enabled = false;
    let mut retry_at = session
        .audio_retry_needed()
        .then(|| Instant::now() + Duration::from_secs(1));
    shared.measurements.lock().unwrap().generation = session.audio_generation();
    shared.publish(&session, revision);
    loop {
        let message = match retry_at {
            Some(deadline) => {
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => Message::Refresh,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match receiver.recv() {
                Ok(message) => message,
                Err(_) => break,
            },
        };
        match message {
            Message::Shutdown => break,
            Message::Execute(command) => {
                let _ = session.execute(command);
            }
            Message::Refresh => {
                let _ = session.execute(Command::RefreshAudio);
                retry_at = session
                    .audio_retry_needed()
                    .then(|| Instant::now() + Duration::from_secs(1));
            }
            Message::Telemetry(enabled) => {
                telemetry_enabled = enabled;
            }
        }
        // Do not let continuous UI commands postpone a due audio retry.
        if retry_at.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = session.execute(Command::RefreshAudio);
            retry_at = session
                .audio_retry_needed()
                .then(|| Instant::now() + Duration::from_secs(1));
        }
        {
            let mut measurements = shared.measurements.lock().unwrap();
            if !telemetry_enabled || measurements.generation != session.audio_generation() {
                measurements.subscription = None;
            }
            measurements.generation = session.audio_generation();
            if telemetry_enabled && measurements.subscription.is_none() {
                measurements.subscription = session.subscribe_telemetry();
            }
        }
        revision += 1;
        shared.publish(&session, revision);
    }
    drop(session);
    shared.close();
}

#[cfg(test)]
mod tests;
