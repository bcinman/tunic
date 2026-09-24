//! Authoritative product state and non-real-time coordination over portable
//! processing provided by `tunic-dsp`.

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tunic_dsp::Configuration;

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

pub struct EngineOptions {
    pub processed_output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    pub configuration: Configuration,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            processed_output_sink: None,
            configuration: Configuration::identity(),
        }
    }
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
    pub configuration: Configuration,
    pub configuration_revision: ConfigurationRevision,
}

impl EngineSnapshot {
    fn starting(configuration: Configuration) -> Self {
        Self {
            status: EngineStatus::Starting,
            bypassed: false,
            route: None,
            devices: Vec::new(),
            configuration,
            configuration_revision: ConfigurationRevision::INITIAL,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationRevision(u64);

impl ConfigurationRevision {
    const INITIAL: Self = Self(0);

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("configuration revision overflow"),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEvent {
    DefaultOutputChanged,
    OutputSampleRateChanged,
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
        configuration: &Configuration,
    ) -> Result<PlatformState, PlatformError>;
    fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError>;
    fn set_bypassed(&mut self, bypassed: bool);
    fn set_configuration(&mut self, configuration: &Configuration) -> Result<(), PlatformError>;
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
    SetConfiguration(
        Configuration,
        Sender<Result<ConfigurationRevision, PlatformError>>,
    ),
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
        let snapshot = Arc::new(RwLock::new(EngineSnapshot::starting(
            options.configuration.clone(),
        )));
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

    pub fn set_configuration(
        &self,
        configuration: Configuration,
    ) -> Result<ConfigurationRevision, EngineError> {
        let (result_tx, result_rx) = mpsc::channel();
        self.commands
            .send(Command::SetConfiguration(configuration, result_tx))
            .map_err(|_| EngineError("engine is not running".into()))?;
        result_rx
            .recv()
            .map_err(|_| EngineError("engine stopped before applying configuration".into()))?
            .map_err(|error| EngineError(error.to_string()))
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
    match platform.start(
        events,
        options.processed_output_sink,
        &options.configuration,
    ) {
        Ok(state) => {
            update_running_snapshot(
                &snapshot,
                state,
                false,
                options.configuration,
                ConfigurationRevision::INITIAL,
            );
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
                PlatformEvent::DefaultOutputChanged | PlatformEvent::OutputSampleRateChanged => {
                    match platform.rebuild_default_route() {
                        Ok(state) => {
                            let current = read_snapshot(&snapshot);
                            let bypassed = current.bypassed;
                            let configuration = current.configuration.clone();
                            let configuration_revision = current.configuration_revision;
                            drop(current);
                            update_running_snapshot(
                                &snapshot,
                                state,
                                bypassed,
                                configuration,
                                configuration_revision,
                            );
                        }
                        Err(error) => {
                            let mut current = write_snapshot(&snapshot);
                            current.status = EngineStatus::Failed(error.to_string());
                            current.route = None;
                        }
                    }
                }
            }
        }

        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(Command::ToggleBypass(result)) => {
                let bypassed = !read_snapshot(&snapshot).bypassed;
                platform.set_bypassed(bypassed);
                write_snapshot(&snapshot).bypassed = bypassed;
                let _ = result.send(bypassed);
            }
            Ok(Command::SetConfiguration(configuration, result)) => {
                let applied = platform.set_configuration(&configuration).map(|()| {
                    let mut current = write_snapshot(&snapshot);
                    current.configuration = configuration;
                    current.configuration_revision = current.configuration_revision.next();
                    current.configuration_revision
                });
                let _ = result.send(applied);
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
    configuration: Configuration,
    configuration_revision: ConfigurationRevision,
) {
    *write_snapshot(snapshot) = EngineSnapshot {
        status: EngineStatus::Running,
        bypassed,
        route: Some(state.route),
        devices: state.devices,
        configuration,
        configuration_revision,
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use tunic_dsp::{Configuration, FrequencyHz, GainDb, PeakingFilter, QualityFactor};

    use super::{
        ActiveRoute, AudioPlatform, DeviceId, Engine, EngineOptions, PlatformError, PlatformEvent,
        PlatformEventSink, PlatformState, ProcessedOutputSink,
    };

    #[test]
    fn publishes_configuration_only_after_the_platform_accepts_it() {
        let applied = Arc::new(Mutex::new(Vec::new()));
        let platform_applied = Arc::clone(&applied);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: platform_applied,
            reject_configuration: false,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let configuration = Configuration::with_peaking_filter(PeakingFilter::new(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));

        let revision = engine.set_configuration(configuration.clone()).unwrap();

        assert_eq!(revision.get(), 1);
        assert_eq!(engine.snapshot().configuration, configuration);
        assert_eq!(applied.lock().unwrap().as_slice(), &[configuration]);
        engine.shutdown().unwrap();
    }

    #[test]
    fn preserves_the_snapshot_when_the_platform_rejects_a_configuration() {
        let engine = Engine::start(EngineOptions::default(), || FakePlatform {
            applied: Arc::new(Mutex::new(Vec::new())),
            reject_configuration: true,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let configuration = Configuration::with_peaking_filter(PeakingFilter::new(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));

        assert!(engine.set_configuration(configuration).is_err());
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.configuration, Configuration::identity());
        assert_eq!(snapshot.configuration_revision.get(), 0);
        engine.shutdown().unwrap();
    }

    #[test]
    fn sample_rate_event_rebuilds_the_route() {
        let events = Arc::new(Mutex::new(None));
        let platform_events = Arc::clone(&events);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: Arc::new(Mutex::new(Vec::new())),
            reject_configuration: false,
            events: Some(platform_events),
            rebuild_sample_rate_hz: 44_100.0,
        })
        .unwrap();
        events
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .send(PlatformEvent::OutputSampleRateChanged)
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while engine
            .snapshot()
            .route
            .is_none_or(|route| route.sample_rate_hz != 44_100.0)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(engine.snapshot().route.unwrap().sample_rate_hz, 44_100.0);
        engine.shutdown().unwrap();
    }

    struct FakePlatform {
        applied: Arc<Mutex<Vec<Configuration>>>,
        reject_configuration: bool,
        events: Option<Arc<Mutex<Option<PlatformEventSink>>>>,
        rebuild_sample_rate_hz: f64,
    }

    impl AudioPlatform for FakePlatform {
        fn start(
            &mut self,
            events: PlatformEventSink,
            _output_sink: Option<Arc<dyn ProcessedOutputSink>>,
            _configuration: &Configuration,
        ) -> Result<PlatformState, PlatformError> {
            if let Some(target) = &self.events {
                *target.lock().unwrap() = Some(events);
            }
            Ok(platform_state(48_000.0))
        }

        fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError> {
            Ok(platform_state(self.rebuild_sample_rate_hz))
        }

        fn set_bypassed(&mut self, _bypassed: bool) {}

        fn set_configuration(
            &mut self,
            configuration: &Configuration,
        ) -> Result<(), PlatformError> {
            if self.reject_configuration {
                return Err(PlatformError::new("configuration rejected"));
            }
            self.applied.lock().unwrap().push(configuration.clone());
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    fn platform_state(sample_rate_hz: f64) -> PlatformState {
        PlatformState {
            route: ActiveRoute {
                device_id: DeviceId::new("fake"),
                device_name: "Fake Output".into(),
                sample_rate_hz,
                channels: 2,
            },
            devices: Vec::new(),
        }
    }
}
