//! Authoritative product state and non-real-time coordination over portable
//! processing provided by `tunic-dsp`.

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessedOutputFormat {
    pub sample_rate_hz: f64,
    pub channels: u32,
}

#[derive(Debug)]
pub struct OutputSinkError(String);

impl OutputSinkError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for OutputSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for OutputSinkError {}

/// Receives interleaved post-DSP samples from the active platform route.
///
/// `write` runs on the real-time audio thread and must not block or allocate.
pub trait ProcessedOutputSink: Send + Sync + 'static {
    fn configure(&self, format: ProcessedOutputFormat) -> Result<(), OutputSinkError>;
    fn write(&self, interleaved_samples: &[f32]);
}

#[derive(Default)]
pub struct EngineOptions {
    pub processed_output_sink: Option<Arc<dyn ProcessedOutputSink>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceId(String);

impl DeviceId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDevice {
    pub id: DeviceId,
    pub name: String,
    pub sample_rate_hz: f64,
    pub channels: u32,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveRoute {
    pub device_id: DeviceId,
    pub device_name: String,
    pub sample_rate_hz: f64,
    pub channels: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineStatus {
    Starting,
    Running,
    Failed(String),
    Stopped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    pub status: EngineStatus,
    pub bypassed: bool,
    pub route: Option<ActiveRoute>,
    pub devices: Vec<OutputDevice>,
}

impl EngineSnapshot {
    fn starting() -> Self {
        Self {
            status: EngineStatus::Starting,
            bypassed: false,
            route: None,
            devices: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEvent {
    DefaultOutputChanged,
}

pub type PlatformEventSink = Sender<PlatformEvent>;

#[derive(Debug)]
pub struct PlatformError(String);

impl PlatformError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for PlatformError {}

pub trait AudioPlatform: 'static {
    fn start(
        &mut self,
        events: PlatformEventSink,
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    ) -> Result<PlatformState, PlatformError>;
    fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError>;
    fn set_bypassed(&mut self, bypassed: bool);
    fn shutdown(&mut self) -> Result<(), PlatformError>;
}

#[derive(Debug)]
pub struct PlatformState {
    pub route: ActiveRoute,
    pub devices: Vec<OutputDevice>,
}

#[derive(Debug)]
pub struct EngineError(String);

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for EngineError {}

enum Command {
    ToggleBypass(Sender<bool>),
    Shutdown(Sender<Result<(), PlatformError>>),
}

pub struct Engine;

pub struct EngineHandle {
    commands: Sender<Command>,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    worker: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start<P, F>(options: EngineOptions, platform: F) -> Result<EngineHandle, EngineError>
    where
        P: AudioPlatform,
        F: FnOnce() -> P + Send + 'static,
    {
        let snapshot = Arc::new(RwLock::new(EngineSnapshot::starting()));
        let worker_snapshot = Arc::clone(&snapshot);
        let (commands, command_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);

        let worker = thread::Builder::new()
            .name("tunic-engine".into())
            .spawn(move || {
                run_engine(platform(), options, command_rx, worker_snapshot, startup_tx);
            })
            .map_err(|error| EngineError(format!("failed to start engine thread: {error}")))?;

        match startup_rx.recv() {
            Ok(Ok(())) => Ok(EngineHandle {
                commands,
                snapshot,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(EngineError(error.to_string()))
            }
            Err(error) => {
                let _ = worker.join();
                Err(EngineError(format!(
                    "engine stopped during startup: {error}"
                )))
            }
        }
    }
}

impl EngineHandle {
    #[must_use]
    pub fn snapshot(&self) -> EngineSnapshot {
        self.snapshot
            .read()
            .expect("engine snapshot lock poisoned")
            .clone()
    }

    pub fn toggle_bypass(&self) -> Result<bool, EngineError> {
        let (result_tx, result_rx) = mpsc::channel();
        self.commands
            .send(Command::ToggleBypass(result_tx))
            .map_err(|_| EngineError("engine is not running".into()))?;
        result_rx
            .recv()
            .map_err(|_| EngineError("engine stopped before applying bypass".into()))
    }

    pub fn shutdown(mut self) -> Result<(), EngineError> {
        self.shutdown_inner()
    }

    fn shutdown_inner(&mut self) -> Result<(), EngineError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        let (result_tx, result_rx) = mpsc::channel();
        let result = if self.commands.send(Command::Shutdown(result_tx)).is_err() {
            Err(EngineError("engine is not running".into()))
        } else {
            match result_rx.recv() {
                Ok(result) => result.map_err(|error| EngineError(error.to_string())),
                Err(_) => Err(EngineError(
                    "engine stopped before acknowledging shutdown".into(),
                )),
            }
        };
        let joined = worker
            .join()
            .map_err(|_| EngineError("engine thread panicked".into()));
        joined?;
        result
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

fn run_engine(
    mut platform: impl AudioPlatform,
    options: EngineOptions,
    commands: Receiver<Command>,
    snapshot: Arc<RwLock<EngineSnapshot>>,
    startup: mpsc::SyncSender<Result<(), PlatformError>>,
) {
    let (events, event_rx) = mpsc::channel();
    match platform.start(events, options.processed_output_sink) {
        Ok(state) => {
            update_running_snapshot(&snapshot, state, false);
            let _ = startup.send(Ok(()));
        }
        Err(error) => {
            let _ = startup.send(Err(error));
            return;
        }
    }

    loop {
        while let Ok(event) = event_rx.try_recv() {
            match event {
                PlatformEvent::DefaultOutputChanged => match platform.rebuild_default_route() {
                    Ok(state) => {
                        let bypassed = read_snapshot(&snapshot).bypassed;
                        update_running_snapshot(&snapshot, state, bypassed);
                    }
                    Err(error) => {
                        let mut current = write_snapshot(&snapshot);
                        current.status = EngineStatus::Failed(error.to_string());
                        current.route = None;
                    }
                },
            }
        }

        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(Command::ToggleBypass(result)) => {
                let bypassed = !read_snapshot(&snapshot).bypassed;
                platform.set_bypassed(bypassed);
                write_snapshot(&snapshot).bypassed = bypassed;
                let _ = result.send(bypassed);
            }
            Ok(Command::Shutdown(result)) => {
                let shutdown = platform.shutdown();
                let mut current = write_snapshot(&snapshot);
                current.status = EngineStatus::Stopped;
                current.route = None;
                let _ = result.send(shutdown);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = platform.shutdown();
                return;
            }
        }
    }
}

fn update_running_snapshot(
    snapshot: &RwLock<EngineSnapshot>,
    state: PlatformState,
    bypassed: bool,
) {
    *write_snapshot(snapshot) = EngineSnapshot {
        status: EngineStatus::Running,
        bypassed,
        route: Some(state.route),
        devices: state.devices,
    };
}

fn read_snapshot(
    snapshot: &RwLock<EngineSnapshot>,
) -> std::sync::RwLockReadGuard<'_, EngineSnapshot> {
    snapshot.read().expect("engine snapshot lock poisoned")
}

fn write_snapshot(
    snapshot: &RwLock<EngineSnapshot>,
) -> std::sync::RwLockWriteGuard<'_, EngineSnapshot> {
    snapshot.write().expect("engine snapshot lock poisoned")
}
