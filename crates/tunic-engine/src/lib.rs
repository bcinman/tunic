//! Authoritative product state and non-real-time coordination over portable
//! processing provided by `tunic-dsp`.

mod persistence;
mod telemetry;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tunic_dsp::Equalizer;

use crate::persistence::{ProfileStore, StoredCatalog, StoredProfile};
pub use crate::telemetry::{
    TelemetryFrame, TelemetryGeneration, TelemetryPublisher, TelemetryReader,
};
pub use tunic_dsp::{
    ChannelLevels, SPECTRUM_MAX_FREQUENCY_HZ, SPECTRUM_MIN_FREQUENCY_HZ, SPECTRUM_POINT_COUNT,
    Spectrum, StereoLevels, spectrum_frequency_hz,
};

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
    pub equalizer: Equalizer,
    pub database_path: Option<PathBuf>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            processed_output_sink: None,
            equalizer: Equalizer::identity(),
            database_path: None,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileId(String);

impl ProfileId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub equalizer: Equalizer,
    pub revision: EqualizerRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceProfileAssignment {
    pub device_id: DeviceId,
    pub profile_id: ProfileId,
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
    Running(ActiveRoute),
    Failed(String),
    Stopped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    pub status: EngineStatus,
    pub bypassed: bool,
    pub devices: Vec<OutputDevice>,
    pub equalizer: Equalizer,
    pub equalizer_revision: EqualizerRevision,
    pub edit_revision: EditRevision,
    pub has_unsaved_changes: bool,
    pub profiles: Vec<Profile>,
    pub default_profile_id: Option<ProfileId>,
    pub active_profile_id: Option<ProfileId>,
    pub device_profile_assignments: Vec<DeviceProfileAssignment>,
}

pub struct SnapshotReceiver {
    receiver: Receiver<EngineSnapshot>,
}

impl SnapshotReceiver {
    pub fn recv(&self) -> Result<EngineSnapshot, RecvError> {
        self.receiver.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<EngineSnapshot, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Result<EngineSnapshot, TryRecvError> {
        self.receiver.try_recv()
    }
}

struct SnapshotState {
    current: RwLock<EngineSnapshot>,
    subscribers: Mutex<Vec<Sender<EngineSnapshot>>>,
    bypass: BypassControl,
}

impl SnapshotState {
    fn new(snapshot: EngineSnapshot) -> Self {
        Self {
            current: RwLock::new(snapshot),
            subscribers: Mutex::new(Vec::new()),
            bypass: BypassControl::default(),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, EngineSnapshot> {
        self.current.read().expect("engine snapshot lock poisoned")
    }

    fn snapshot(&self) -> EngineSnapshot {
        self.read().clone()
    }

    fn publish(&self, mut snapshot: EngineSnapshot) {
        let mut subscribers = self
            .subscribers
            .lock()
            .expect("engine snapshot subscriber lock poisoned");
        let changed = {
            let mut current = self.current.write().expect("engine snapshot lock poisoned");
            snapshot.bypassed = current.bypassed;
            if *current == snapshot {
                false
            } else {
                *current = snapshot.clone();
                true
            }
        };
        if changed {
            subscribers.retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
        }
    }

    fn toggle_bypass(&self) -> bool {
        let toggled_to = self.bypass.toggle();
        let mut subscribers = self
            .subscribers
            .lock()
            .expect("engine snapshot subscriber lock poisoned");
        let snapshot = {
            let mut current = self.current.write().expect("engine snapshot lock poisoned");
            let bypassed = self.bypass.is_bypassed();
            if current.bypassed == bypassed {
                return toggled_to;
            }
            current.bypassed = bypassed;
            current.clone()
        };
        subscribers.retain(|subscriber| subscriber.send(snapshot.clone()).is_ok());
        toggled_to
    }

    fn subscribe(&self) -> SnapshotReceiver {
        let mut subscribers = self
            .subscribers
            .lock()
            .expect("engine snapshot subscriber lock poisoned");
        let (sender, receiver) = mpsc::channel();
        sender
            .send(self.snapshot())
            .expect("new snapshot receiver must be connected");
        subscribers.push(sender);
        SnapshotReceiver { receiver }
    }
}

impl EngineSnapshot {
    #[must_use]
    pub fn active_route(&self) -> Option<&ActiveRoute> {
        match &self.status {
            EngineStatus::Running(route) => Some(route),
            EngineStatus::Starting | EngineStatus::Failed(_) | EngineStatus::Stopped => None,
        }
    }

    fn starting(equalizer: Equalizer) -> Self {
        Self {
            status: EngineStatus::Starting,
            bypassed: false,
            devices: Vec::new(),
            equalizer,
            equalizer_revision: EqualizerRevision::INITIAL,
            edit_revision: EditRevision::INITIAL,
            has_unsaved_changes: false,
            profiles: Vec::new(),
            default_profile_id: None,
            active_profile_id: None,
            device_profile_assignments: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EqualizerRevision(u64);

impl EqualizerRevision {
    const INITIAL: Self = Self(0);

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("equalizer revision overflow"))
    }

    fn from_persisted(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditRevision(u64);

impl EditRevision {
    const INITIAL: Self = Self(0);

    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("edit revision overflow"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformEvent {
    DefaultOutputChanged,
    OutputSampleRateChanged,
}

pub type PlatformEventSink = Sender<PlatformEvent>;

#[derive(Clone, Default)]
pub struct BypassControl(Arc<AtomicBool>);

impl BypassControl {
    #[must_use]
    pub fn is_bypassed(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn toggle(&self) -> bool {
        !self.0.fetch_xor(true, Ordering::Relaxed)
    }
}

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
        telemetry: TelemetryPublisher,
        equalizer: &Equalizer,
        bypass: BypassControl,
    ) -> Result<PlatformState, PlatformError>;
    fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError>;
    fn set_equalizer(&mut self, equalizer: &Equalizer) -> Result<(), PlatformError>;
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
    PreviewEqualizer(
        Equalizer,
        EditRevision,
        Sender<Result<EditRevision, EngineError>>,
    ),
    SaveEqualizer(EditRevision, Sender<Result<EqualizerRevision, EngineError>>),
    DiscardPreview(EditRevision, Sender<Result<EditRevision, EngineError>>),
    CreateProfile(String, Sender<Result<ProfileId, EngineError>>),
    RenameProfile(ProfileId, String, Sender<Result<(), EngineError>>),
    DeleteProfile(ProfileId, Sender<Result<(), EngineError>>),
    SelectProfile(ProfileId, Sender<Result<(), EngineError>>),
    AssignProfile(DeviceId, ProfileId, Sender<Result<(), EngineError>>),
    Shutdown(Sender<Result<(), PlatformError>>),
}

pub struct Engine;

pub struct EngineHandle {
    commands: Sender<Command>,
    snapshot: Arc<SnapshotState>,
    telemetry: telemetry::TelemetrySource,
    worker: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start<P, F>(options: EngineOptions, platform: F) -> Result<EngineHandle, EngineError>
    where
        P: AudioPlatform,
        F: FnOnce() -> P + Send + 'static,
    {
        let snapshot = Arc::new(SnapshotState::new(EngineSnapshot::starting(
            options.equalizer.clone(),
        )));
        let worker_snapshot = Arc::clone(&snapshot);
        let worker_bypass = snapshot.bypass.clone();
        let (telemetry_publisher, telemetry) = telemetry::channel();
        let (commands, command_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);

        let worker = thread::Builder::new()
            .name("tunic-engine".into())
            .spawn(move || {
                run_engine(
                    platform(),
                    options,
                    telemetry_publisher,
                    command_rx,
                    worker_snapshot,
                    worker_bypass,
                    startup_tx,
                );
            })
            .map_err(|error| EngineError(format!("failed to start engine thread: {error}")))?;

        match startup_rx.recv() {
            Ok(Ok(())) => Ok(EngineHandle {
                commands,
                snapshot,
                telemetry,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
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
        self.snapshot.snapshot()
    }

    #[must_use]
    pub fn subscribe_snapshots(&self) -> SnapshotReceiver {
        self.snapshot.subscribe()
    }

    #[must_use]
    pub fn subscribe_telemetry(&self) -> TelemetryReader {
        self.telemetry.subscribe()
    }

    pub fn toggle_bypass(&self) -> bool {
        self.snapshot.toggle_bypass()
    }

    pub fn preview_equalizer(
        &self,
        equalizer: Equalizer,
        expected_revision: EditRevision,
    ) -> Result<EditRevision, EngineError> {
        self.request(
            |reply| Command::PreviewEqualizer(equalizer, expected_revision, reply),
            "previewing equalizer",
        )?
    }

    pub fn save_equalizer(
        &self,
        expected_revision: EditRevision,
    ) -> Result<EqualizerRevision, EngineError> {
        self.request(
            |reply| Command::SaveEqualizer(expected_revision, reply),
            "saving equalizer",
        )?
    }

    pub fn discard_preview(
        &self,
        expected_revision: EditRevision,
    ) -> Result<EditRevision, EngineError> {
        self.request(
            |reply| Command::DiscardPreview(expected_revision, reply),
            "discarding equalizer preview",
        )?
    }

    pub fn create_profile(&self, name: String) -> Result<ProfileId, EngineError> {
        self.request(
            |reply| Command::CreateProfile(name, reply),
            "creating profile",
        )?
    }

    pub fn rename_profile(&self, id: ProfileId, name: String) -> Result<(), EngineError> {
        self.request(
            |reply| Command::RenameProfile(id, name, reply),
            "renaming profile",
        )?
    }

    pub fn delete_profile(&self, id: ProfileId) -> Result<(), EngineError> {
        self.request(
            |reply| Command::DeleteProfile(id, reply),
            "deleting profile",
        )?
    }

    pub fn select_profile(&self, id: ProfileId) -> Result<(), EngineError> {
        self.request(
            |reply| Command::SelectProfile(id, reply),
            "selecting profile",
        )?
    }

    pub fn assign_profile(
        &self,
        device_id: DeviceId,
        profile_id: ProfileId,
    ) -> Result<(), EngineError> {
        self.request(
            |reply| Command::AssignProfile(device_id, profile_id, reply),
            "assigning profile",
        )?
    }

    pub fn shutdown(mut self) -> Result<(), EngineError> {
        self.shutdown_inner()
    }

    fn request<R>(
        &self,
        command: impl FnOnce(Sender<R>) -> Command,
        operation: &'static str,
    ) -> Result<R, EngineError> {
        let (reply, response) = mpsc::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| EngineError("engine is not running".into()))?;
        response
            .recv()
            .map_err(|_| EngineError(format!("engine stopped before {operation}")))
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
    telemetry: TelemetryPublisher,
    commands: Receiver<Command>,
    snapshot: Arc<SnapshotState>,
    bypass: BypassControl,
    startup: mpsc::SyncSender<Result<(), EngineError>>,
) {
    let mut store = match options.database_path.as_deref() {
        Some(path) => match ProfileStore::open(path) {
            Ok(store) => Some(store),
            Err(error) => {
                let _ = startup.send(Err(EngineError(error.to_string())));
                return;
            }
        },
        None => None,
    };
    let initial_profile = match &store {
        Some(store) => match store.load_default_profile() {
            Ok(profile) => SavedEqualizer::from(profile),
            Err(error) => {
                let _ = startup.send(Err(EngineError(error.to_string())));
                return;
            }
        },
        None => SavedEqualizer {
            profile_id: None,
            equalizer: options.equalizer,
            revision: EqualizerRevision::INITIAL,
        },
    };
    let (events, event_rx) = mpsc::channel();
    let initial_equalizer = initial_profile.equalizer.clone();
    match platform.start(
        events,
        options.processed_output_sink,
        telemetry,
        &initial_profile.equalizer,
        bypass.clone(),
    ) {
        Ok(state) => {
            let selected_profile = match resolve_profile(store.as_ref(), &state, &initial_profile) {
                Ok(profile) => profile,
                Err(error) => {
                    let _ = platform.shutdown();
                    let _ = startup.send(Err(error));
                    return;
                }
            };
            if selected_profile.equalizer != initial_equalizer
                && let Err(error) = platform.set_equalizer(&selected_profile.equalizer)
            {
                let _ = platform.shutdown();
                let _ = startup.send(Err(EngineError(error.to_string())));
                return;
            }
            let catalog = if let Some(store) = store.as_ref() {
                match store.catalog() {
                    Ok(catalog) => Some(catalog),
                    Err(error) => {
                        let _ = platform.shutdown();
                        let _ = startup.send(Err(EngineError(error.to_string())));
                        return;
                    }
                }
            } else {
                None
            };
            let mut current = EngineSnapshot::starting(selected_profile.equalizer.clone());
            update_running_snapshot(
                &mut current,
                state,
                bypass.is_bypassed(),
                selected_profile.equalizer.clone(),
                selected_profile.revision,
                EditRevision::INITIAL,
                false,
            );
            if let Some(catalog) = catalog {
                apply_profile_catalog(&mut current, catalog);
            }
            current.active_profile_id = selected_profile.profile_id.clone();
            snapshot.publish(current.clone());
            let _ = startup.send(Ok(()));
            run_commands(
                platform,
                store.as_mut(),
                selected_profile,
                current,
                commands,
                event_rx,
                snapshot,
            );
        }
        Err(error) => {
            let _ = startup.send(Err(EngineError(error.to_string())));
        }
    }
}

fn run_commands(
    mut platform: impl AudioPlatform,
    mut store: Option<&mut ProfileStore>,
    mut saved_equalizer: SavedEqualizer,
    mut current: EngineSnapshot,
    commands: Receiver<Command>,
    event_rx: Receiver<PlatformEvent>,
    snapshot: Arc<SnapshotState>,
) {
    let bypass = snapshot.bypass.clone();
    loop {
        while let Ok(event) = event_rx.try_recv() {
            let state = match platform.rebuild_default_route() {
                Ok(state) => state,
                Err(error) => {
                    fail_snapshot(&mut current, &snapshot, error.to_string());
                    continue;
                }
            };
            match event {
                PlatformEvent::DefaultOutputChanged => {
                    let bypassed = bypass.is_bypassed();
                    let current_equalizer = current.equalizer.clone();
                    let mut edit_revision = current.edit_revision;
                    let has_unsaved_changes = current.has_unsaved_changes;
                    match resolve_profile(store.as_deref(), &state, &saved_equalizer) {
                        Ok(profile) => {
                            let profile_changed = saved_equalizer.profile_id != profile.profile_id;
                            if current_equalizer != profile.equalizer
                                && let Err(error) = platform.set_equalizer(&profile.equalizer)
                            {
                                fail_snapshot(&mut current, &snapshot, error.to_string());
                                continue;
                            }
                            if profile_changed
                                || has_unsaved_changes
                                || current_equalizer != profile.equalizer
                            {
                                edit_revision = edit_revision.next();
                            }
                            update_running_snapshot(
                                &mut current,
                                state,
                                bypassed,
                                profile.equalizer.clone(),
                                profile.revision,
                                edit_revision,
                                false,
                            );
                            current.active_profile_id = profile.profile_id.clone();
                            snapshot.publish(current.clone());
                            saved_equalizer = profile;
                        }
                        Err(error) => {
                            fail_snapshot(&mut current, &snapshot, error.to_string());
                        }
                    }
                }
                PlatformEvent::OutputSampleRateChanged => {
                    let bypassed = bypass.is_bypassed();
                    let equalizer = current.equalizer.clone();
                    let equalizer_revision = current.equalizer_revision;
                    let edit_revision = current.edit_revision;
                    let has_unsaved_changes = current.has_unsaved_changes;
                    update_running_snapshot(
                        &mut current,
                        state,
                        bypassed,
                        equalizer,
                        equalizer_revision,
                        edit_revision,
                        has_unsaved_changes,
                    );
                    snapshot.publish(current.clone());
                }
            }
        }

        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(Command::PreviewEqualizer(equalizer, expected_revision, result)) => {
                let current_revision = current.edit_revision;
                let previewed = if current_revision != expected_revision {
                    Err(stale_edit_revision(expected_revision, current_revision))
                } else if current.equalizer == equalizer {
                    Ok(current_revision)
                } else {
                    platform
                        .set_equalizer(&equalizer)
                        .map_err(|error| EngineError(error.to_string()))
                        .map(|()| {
                            current.equalizer = equalizer;
                            current.edit_revision = current.edit_revision.next();
                            current.has_unsaved_changes =
                                current.equalizer != saved_equalizer.equalizer;
                            snapshot.publish(current.clone());
                            current.edit_revision
                        })
                };
                let _ = result.send(previewed);
            }
            Ok(Command::SaveEqualizer(expected_revision, result)) => {
                let current_revision = current.edit_revision;
                let has_unsaved_changes = current.has_unsaved_changes;
                let equalizer = current.equalizer.clone();
                let equalizer_revision = current.equalizer_revision;
                let saved = if current_revision != expected_revision {
                    Err(stale_edit_revision(expected_revision, current_revision))
                } else if !has_unsaved_changes {
                    Ok(equalizer_revision)
                } else {
                    let persisted = match (&mut store, &saved_equalizer.profile_id) {
                        (Some(store), Some(profile_id)) => store
                            .save_profile(profile_id.as_str(), &equalizer, equalizer_revision.get())
                            .map(|(revision, catalog)| {
                                (EqualizerRevision::from_persisted(revision), Some(catalog))
                            })
                            .map_err(|error| EngineError(error.to_string())),
                        _ => Ok((equalizer_revision.next(), None)),
                    };
                    persisted.map(|(revision, catalog)| {
                        saved_equalizer.equalizer = equalizer;
                        saved_equalizer.revision = revision;
                        current.equalizer_revision = revision;
                        current.edit_revision = current.edit_revision.next();
                        current.has_unsaved_changes = false;
                        if let Some(catalog) = catalog {
                            apply_profile_catalog(&mut current, catalog);
                        }
                        snapshot.publish(current.clone());
                        revision
                    })
                };
                let _ = result.send(saved);
            }
            Ok(Command::DiscardPreview(expected_revision, result)) => {
                let current_revision = current.edit_revision;
                let has_unsaved_changes = current.has_unsaved_changes;
                let discarded = if current_revision != expected_revision {
                    Err(stale_edit_revision(expected_revision, current_revision))
                } else if !has_unsaved_changes {
                    Ok(current_revision)
                } else {
                    platform
                        .set_equalizer(&saved_equalizer.equalizer)
                        .map_err(|error| EngineError(error.to_string()))
                        .map(|()| {
                            current.equalizer = saved_equalizer.equalizer.clone();
                            current.edit_revision = current.edit_revision.next();
                            current.has_unsaved_changes = false;
                            snapshot.publish(current.clone());
                            current.edit_revision
                        })
                };
                let _ = result.send(discarded);
            }
            Ok(Command::CreateProfile(name, result)) => {
                let equalizer = current.equalizer.clone();
                let active_device_id = current
                    .active_route()
                    .map(|route| route.device_id.as_str().to_owned());
                let created = match (&mut store, active_device_id) {
                    (Some(store), Some(active_device_id)) => store
                        .create_and_select_profile(&name, &equalizer, &active_device_id)
                        .map_err(|error| EngineError(error.to_string()))
                        .map(|(profile, catalog)| {
                            let saved = SavedEqualizer::from(profile);
                            let id = saved.profile_id.clone().expect("stored profile has an id");
                            saved_equalizer = saved;
                            current.equalizer_revision = saved_equalizer.revision;
                            current.edit_revision = current.edit_revision.next();
                            current.has_unsaved_changes = false;
                            current.active_profile_id = Some(id.clone());
                            apply_profile_catalog(&mut current, catalog);
                            snapshot.publish(current.clone());
                            id
                        }),
                    (None, _) => Err(profile_store_required()),
                    (_, None) => Err(EngineError("no active output device".into())),
                };
                let _ = result.send(created);
            }
            Ok(Command::RenameProfile(profile_id, name, result)) => {
                let renamed = match &mut store {
                    Some(store) => store
                        .rename_profile(profile_id.as_str(), &name)
                        .map_err(|error| EngineError(error.to_string()))
                        .map(|catalog| {
                            apply_profile_catalog(&mut current, catalog);
                            snapshot.publish(current.clone());
                        }),
                    None => Err(profile_store_required()),
                };
                let _ = result.send(renamed);
            }
            Ok(Command::SelectProfile(profile_id, result)) => {
                let has_unsaved_changes = current.has_unsaved_changes;
                let current_equalizer = current.equalizer.clone();
                let active_device_id = current
                    .active_route()
                    .map(|route| route.device_id.as_str().to_owned());
                let selected = if has_unsaved_changes {
                    Err(unsaved_profile_change())
                } else {
                    match (&mut store, active_device_id) {
                        (Some(store), Some(active_device_id)) => store
                            .load_profile_by_id(profile_id.as_str())
                            .map_err(|error| EngineError(error.to_string()))
                            .and_then(|profile| {
                                let changed = current_equalizer != profile.equalizer;
                                if changed {
                                    platform
                                        .set_equalizer(&profile.equalizer)
                                        .map_err(|error| EngineError(error.to_string()))?;
                                }
                                let catalog = store
                                    .select_default_profile(profile_id.as_str(), &active_device_id)
                                    .map_err(|error| {
                                        if changed {
                                            let _ = platform.set_equalizer(&current_equalizer);
                                        }
                                        EngineError(error.to_string())
                                    })?;
                                saved_equalizer = SavedEqualizer::from(profile);
                                publish_selected_profile(&mut current, &saved_equalizer);
                                apply_profile_catalog(&mut current, catalog);
                                snapshot.publish(current.clone());
                                Ok(())
                            }),
                        (None, _) => Err(profile_store_required()),
                        (_, None) => Err(EngineError("no active output device".into())),
                    }
                };
                let _ = result.send(selected);
            }
            Ok(Command::AssignProfile(device_id, profile_id, result)) => {
                let is_active = current
                    .active_route()
                    .is_some_and(|route| route.device_id == device_id);
                let has_unsaved_changes = current.has_unsaved_changes;
                let current_equalizer = current.equalizer.clone();
                let assigned = if is_active && has_unsaved_changes {
                    Err(unsaved_profile_change())
                } else {
                    match &mut store {
                        Some(store) => store
                            .load_profile_by_id(profile_id.as_str())
                            .map_err(|error| EngineError(error.to_string()))
                            .and_then(|profile| {
                                let changed = is_active && current_equalizer != profile.equalizer;
                                if changed {
                                    platform
                                        .set_equalizer(&profile.equalizer)
                                        .map_err(|error| EngineError(error.to_string()))?;
                                }
                                let catalog = store
                                    .assign_profile(device_id.as_str(), profile_id.as_str())
                                    .map_err(|error| {
                                        if changed {
                                            let _ = platform.set_equalizer(&current_equalizer);
                                        }
                                        EngineError(error.to_string())
                                    })?;
                                if is_active {
                                    saved_equalizer = SavedEqualizer::from(profile);
                                }
                                if is_active {
                                    publish_selected_profile(&mut current, &saved_equalizer);
                                }
                                apply_profile_catalog(&mut current, catalog);
                                snapshot.publish(current.clone());
                                Ok(())
                            }),
                        None => Err(profile_store_required()),
                    }
                };
                let _ = result.send(assigned);
            }
            Ok(Command::DeleteProfile(profile_id, result)) => {
                let is_active = current.active_profile_id.as_ref() == Some(&profile_id);
                let has_unsaved_changes = current.has_unsaved_changes;
                let current_equalizer = current.equalizer.clone();
                let deleted = if is_active && has_unsaved_changes {
                    Err(unsaved_profile_change())
                } else {
                    match &mut store {
                        Some(store) => {
                            let fallback = if is_active {
                                Some(
                                    store
                                        .load_default_profile()
                                        .map_err(|error| EngineError(error.to_string())),
                                )
                            } else {
                                None
                            };
                            fallback
                                .transpose()
                                .and_then(|fallback| {
                                    if fallback
                                        .as_ref()
                                        .is_some_and(|profile| profile.id == profile_id.as_str())
                                    {
                                        return Err(EngineError(
                                            "cannot delete the default profile; select another profile first"
                                                .into(),
                                        ));
                                    }
                                    let changed = fallback
                                        .as_ref()
                                        .is_some_and(|profile| profile.equalizer != current_equalizer);
                                    if let Some(fallback) = &fallback
                                        && changed
                                    {
                                        platform
                                            .set_equalizer(&fallback.equalizer)
                                            .map_err(|error| EngineError(error.to_string()))?;
                                    }
                                    let catalog = store
                                        .delete_profile(profile_id.as_str())
                                        .map_err(|error| {
                                            if changed {
                                                let _ = platform.set_equalizer(&current_equalizer);
                                            }
                                            EngineError(error.to_string())
                                        })?;
                                    if let Some(fallback) = fallback {
                                        saved_equalizer = SavedEqualizer::from(fallback);
                                    }
                                    if is_active {
                                        publish_selected_profile(&mut current, &saved_equalizer);
                                    }
                                    apply_profile_catalog(&mut current, catalog);
                                    snapshot.publish(current.clone());
                                    Ok(())
                                })
                        }
                        None => Err(profile_store_required()),
                    }
                };
                let _ = result.send(deleted);
            }
            Ok(Command::Shutdown(result)) => {
                let shutdown = platform.shutdown();
                current.status = EngineStatus::Stopped;
                snapshot.publish(current.clone());
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

#[derive(Clone)]
struct SavedEqualizer {
    profile_id: Option<ProfileId>,
    equalizer: Equalizer,
    revision: EqualizerRevision,
}

impl From<StoredProfile> for SavedEqualizer {
    fn from(profile: StoredProfile) -> Self {
        Self {
            profile_id: Some(ProfileId::new(profile.id)),
            equalizer: profile.equalizer,
            revision: EqualizerRevision::from_persisted(profile.revision),
        }
    }
}

fn resolve_profile(
    store: Option<&ProfileStore>,
    state: &PlatformState,
    fallback: &SavedEqualizer,
) -> Result<SavedEqualizer, EngineError> {
    store.map_or_else(
        || Ok(fallback.clone()),
        |store| {
            store
                .load_profile_for_device(state.route.device_id.as_str())
                .map(SavedEqualizer::from)
                .map_err(|error| EngineError(error.to_string()))
        },
    )
}

fn fail_snapshot(current: &mut EngineSnapshot, snapshots: &SnapshotState, error: String) {
    current.status = EngineStatus::Failed(error);
    snapshots.publish(current.clone());
}

fn update_running_snapshot(
    snapshot: &mut EngineSnapshot,
    state: PlatformState,
    bypassed: bool,
    equalizer: Equalizer,
    equalizer_revision: EqualizerRevision,
    edit_revision: EditRevision,
    has_unsaved_changes: bool,
) {
    snapshot.status = EngineStatus::Running(state.route);
    snapshot.bypassed = bypassed;
    snapshot.devices = state.devices;
    snapshot.equalizer = equalizer;
    snapshot.equalizer_revision = equalizer_revision;
    snapshot.edit_revision = edit_revision;
    snapshot.has_unsaved_changes = has_unsaved_changes;
}

fn apply_profile_catalog(snapshot: &mut EngineSnapshot, catalog: StoredCatalog) {
    snapshot.profiles = catalog
        .profiles
        .into_iter()
        .map(|profile| Profile {
            id: ProfileId::new(profile.id),
            name: profile.name,
            equalizer: profile.equalizer,
            revision: EqualizerRevision::from_persisted(profile.revision),
        })
        .collect();
    snapshot.default_profile_id = Some(ProfileId::new(catalog.default_profile_id));
    snapshot.device_profile_assignments = catalog
        .assignments
        .into_iter()
        .map(|assignment| DeviceProfileAssignment {
            device_id: DeviceId::new(assignment.device_id),
            profile_id: ProfileId::new(assignment.profile_id),
        })
        .collect();
}

fn publish_selected_profile(snapshot: &mut EngineSnapshot, profile: &SavedEqualizer) {
    let selection_changed = snapshot.active_profile_id != profile.profile_id
        || snapshot.equalizer != profile.equalizer
        || snapshot.equalizer_revision != profile.revision
        || snapshot.has_unsaved_changes;
    snapshot.equalizer = profile.equalizer.clone();
    snapshot.equalizer_revision = profile.revision;
    if selection_changed {
        snapshot.edit_revision = snapshot.edit_revision.next();
    }
    snapshot.has_unsaved_changes = false;
    snapshot.active_profile_id = profile.profile_id.clone();
}

fn profile_store_required() -> EngineError {
    EngineError("profile management requires persistent storage".into())
}

fn unsaved_profile_change() -> EngineError {
    EngineError("save or discard equalizer changes before changing profiles".into())
}

fn stale_edit_revision(expected: EditRevision, current: EditRevision) -> EngineError {
    EngineError(format!(
        "equalizer edit is stale: expected revision {}, current revision is {}",
        expected.get(),
        current.get()
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, QualityFactor};

    use super::{
        ActiveRoute, AudioPlatform, BypassControl, DeviceId, Engine, EngineOptions, EngineStatus,
        PlatformError, PlatformEvent, PlatformEventSink, PlatformState, ProcessedOutputSink,
        ProfileId, TelemetryPublisher,
    };

    #[test]
    fn snapshot_subscription_delivers_initial_and_changed_state_in_order() {
        let engine = Engine::start(EngineOptions::default(), || FakePlatform {
            applied: Arc::new(Mutex::new(Vec::new())),
            reject_equalizer: false,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let snapshots = engine.subscribe_snapshots();

        assert_eq!(snapshots.recv().unwrap(), engine.snapshot());
        engine
            .preview_equalizer(Equalizer::identity(), engine.snapshot().edit_revision)
            .unwrap();
        assert_eq!(
            snapshots.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        assert!(engine.toggle_bypass());
        assert!(snapshots.recv().unwrap().bypassed);
        assert!(!engine.toggle_bypass());
        assert!(!snapshots.recv().unwrap().bypassed);

        engine.shutdown().unwrap();
        let stopped = snapshots.recv().unwrap();
        assert_eq!(stopped.status, EngineStatus::Stopped);
        assert!(stopped.active_route().is_none());
        assert!(snapshots.recv().is_err());
    }

    #[test]
    fn bypass_does_not_wait_for_a_route_rebuild() {
        let (event_sink_tx, event_sink_rx) = std::sync::mpsc::sync_channel(1);
        let (rebuild_entered_tx, rebuild_entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_rebuild_tx, release_rebuild_rx) = std::sync::mpsc::sync_channel(1);
        let engine = Engine::start(EngineOptions::default(), move || BlockingRebuildPlatform {
            event_sink: event_sink_tx,
            rebuild_entered: rebuild_entered_tx,
            release_rebuild: release_rebuild_rx,
        })
        .unwrap();
        let (event_sink, platform_bypass) = event_sink_rx.recv().unwrap();
        event_sink
            .send(PlatformEvent::DefaultOutputChanged)
            .unwrap();
        rebuild_entered_rx.recv().unwrap();
        let (toggle_done_tx, toggle_done_rx) = std::sync::mpsc::sync_channel(1);
        let toggle = thread::spawn(move || {
            let bypassed = engine.toggle_bypass();
            toggle_done_tx.send((engine, bypassed)).unwrap();
        });

        let completed = toggle_done_rx.recv_timeout(Duration::from_secs(1));
        if completed.is_err() {
            release_rebuild_tx.send(()).unwrap();
            let (engine, _) = toggle_done_rx.recv().unwrap();
            toggle.join().unwrap();
            engine.shutdown().unwrap();
            panic!("bypass waited for the route rebuild");
        }
        let (engine, bypassed) = completed.unwrap();
        assert!(bypassed);
        assert!(platform_bypass.is_bypassed());

        release_rebuild_tx.send(()).unwrap();
        toggle.join().unwrap();
        engine.shutdown().unwrap();
    }

    #[test]
    fn repeated_profile_commands_do_not_publish_or_advance_the_edit_revision() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tunic-snapshot-noop-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let engine = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();
        let snapshots = engine.subscribe_snapshots();
        let initial = snapshots.recv().unwrap();
        let initial_revision = initial.edit_revision;

        engine.select_profile(ProfileId::new("default")).unwrap();
        assert_eq!(engine.snapshot().edit_revision, initial_revision);
        assert_eq!(
            snapshots.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        engine
            .assign_profile(DeviceId::new("fake"), ProfileId::new("default"))
            .unwrap();
        let assigned = snapshots.recv().unwrap();
        assert_eq!(assigned.edit_revision, initial_revision);
        engine
            .assign_profile(DeviceId::new("fake"), ProfileId::new("default"))
            .unwrap();
        assert_eq!(
            snapshots.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        );

        let other = engine.create_profile("Other".into()).unwrap();
        let created = snapshots.recv().unwrap();
        assert_eq!(created.equalizer, initial.equalizer);
        engine.select_profile(ProfileId::new("default")).unwrap();
        let selected = snapshots.recv().unwrap();
        assert_eq!(selected.equalizer, initial.equalizer);
        assert_eq!(selected.active_profile_id, Some(ProfileId::new("default")));
        assert_eq!(
            selected.edit_revision.get(),
            created.edit_revision.get() + 1
        );
        assert_ne!(selected.active_profile_id, Some(other));

        engine.shutdown().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn publishes_preview_only_after_the_platform_accepts_it() {
        let applied = Arc::new(Mutex::new(Vec::new()));
        let platform_applied = Arc::clone(&applied);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: platform_applied,
            reject_equalizer: false,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let equalizer = Equalizer::with_filters(vec![
            Filter::low_shelf(
                FrequencyHz::new(100.0).unwrap(),
                GainDb::new(3.0).unwrap(),
                QualityFactor::new(0.7).unwrap(),
            ),
            Filter::high_shelf(
                FrequencyHz::new(1_000.0).unwrap(),
                GainDb::new(-6.0).unwrap(),
                QualityFactor::new(1.5).unwrap(),
            ),
        ]);

        let revision = engine
            .preview_equalizer(equalizer.clone(), engine.snapshot().edit_revision)
            .unwrap();

        assert_eq!(revision.get(), 1);
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.equalizer, equalizer);
        assert_eq!(snapshot.equalizer_revision.get(), 0);
        assert!(snapshot.has_unsaved_changes);
        assert_eq!(applied.lock().unwrap().as_slice(), &[equalizer]);
        engine.shutdown().unwrap();
    }

    #[test]
    fn save_commits_the_preview_and_discard_restores_it() {
        let applied = Arc::new(Mutex::new(Vec::new()));
        let platform_applied = Arc::clone(&applied);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: platform_applied,
            reject_equalizer: false,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let saved = Equalizer::with_filter(Filter::low_shelf(
            FrequencyHz::new(100.0).unwrap(),
            GainDb::new(3.0).unwrap(),
            QualityFactor::new(0.7).unwrap(),
        ));
        let discarded = Equalizer::with_filter(Filter::high_shelf(
            FrequencyHz::new(8_000.0).unwrap(),
            GainDb::new(-4.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));

        let preview_revision = engine
            .preview_equalizer(saved.clone(), engine.snapshot().edit_revision)
            .unwrap();
        let equalizer_revision = engine.save_equalizer(preview_revision).unwrap();

        let snapshot = engine.snapshot();
        assert_eq!(equalizer_revision.get(), 1);
        assert_eq!(snapshot.edit_revision.get(), 2);
        assert!(!snapshot.has_unsaved_changes);

        let preview_revision = engine
            .preview_equalizer(discarded.clone(), snapshot.edit_revision)
            .unwrap();
        let discarded_revision = engine.discard_preview(preview_revision).unwrap();

        let snapshot = engine.snapshot();
        assert_eq!(discarded_revision.get(), 4);
        assert_eq!(snapshot.equalizer, saved);
        assert_eq!(snapshot.equalizer_revision.get(), 1);
        assert!(!snapshot.has_unsaved_changes);
        assert_eq!(
            applied.lock().unwrap().as_slice(),
            &[saved.clone(), discarded, saved]
        );
        engine.shutdown().unwrap();
    }

    #[test]
    fn rejects_an_edit_based_on_a_stale_revision() {
        let applied = Arc::new(Mutex::new(Vec::new()));
        let platform_applied = Arc::clone(&applied);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: platform_applied,
            reject_equalizer: false,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let original_revision = engine.snapshot().edit_revision;
        let first = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(500.0).unwrap(),
            GainDb::new(3.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));
        let stale = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(2_000.0).unwrap(),
            GainDb::new(-3.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));

        engine
            .preview_equalizer(first.clone(), original_revision)
            .unwrap();
        let error = engine
            .preview_equalizer(stale, original_revision)
            .unwrap_err();

        assert!(error.to_string().contains("edit is stale"));
        assert_eq!(engine.snapshot().equalizer, first.clone());
        assert_eq!(applied.lock().unwrap().as_slice(), &[first]);
        engine.shutdown().unwrap();
    }

    #[test]
    fn preserves_the_snapshot_when_the_platform_rejects_a_preview() {
        let engine = Engine::start(EngineOptions::default(), || FakePlatform {
            applied: Arc::new(Mutex::new(Vec::new())),
            reject_equalizer: true,
            events: None,
            rebuild_sample_rate_hz: 48_000.0,
        })
        .unwrap();
        let equalizer = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(6.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));

        assert!(
            engine
                .preview_equalizer(equalizer, engine.snapshot().edit_revision)
                .is_err()
        );
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.equalizer, Equalizer::identity());
        assert_eq!(snapshot.equalizer_revision.get(), 0);
        assert_eq!(snapshot.edit_revision.get(), 0);
        assert!(!snapshot.has_unsaved_changes);
        engine.shutdown().unwrap();
    }

    #[test]
    fn sample_rate_event_rebuilds_the_route() {
        let events = Arc::new(Mutex::new(None));
        let platform_events = Arc::clone(&events);
        let engine = Engine::start(EngineOptions::default(), move || FakePlatform {
            applied: Arc::new(Mutex::new(Vec::new())),
            reject_equalizer: false,
            events: Some(platform_events),
            rebuild_sample_rate_hz: 44_100.0,
        })
        .unwrap();
        let preview = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(750.0).unwrap(),
            GainDb::new(2.0).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));
        let preview_revision = engine
            .preview_equalizer(preview.clone(), engine.snapshot().edit_revision)
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
            .active_route()
            .is_none_or(|route| route.sample_rate_hz != 44_100.0)
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.active_route().unwrap().sample_rate_hz, 44_100.0);
        assert_eq!(snapshot.equalizer, preview);
        assert_eq!(snapshot.edit_revision, preview_revision);
        assert!(snapshot.has_unsaved_changes);
        engine.shutdown().unwrap();
    }

    #[test]
    fn profile_database_restores_saves_and_resolves_device_assignments() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tunic-engine-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let equalizer = Equalizer::with_filter(Filter::high_shelf(
            FrequencyHz::new(6_000.0).unwrap(),
            GainDb::new(-2.5).unwrap(),
            QualityFactor::new(0.8).unwrap(),
        ));
        let engine = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();
        let edit_revision = engine
            .preview_equalizer(equalizer.clone(), engine.snapshot().edit_revision)
            .unwrap();
        engine.save_equalizer(edit_revision).unwrap();
        engine.shutdown().unwrap();

        let restored = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();

        let snapshot = restored.snapshot();
        assert_eq!(snapshot.equalizer, equalizer);
        assert_eq!(snapshot.equalizer_revision.get(), 1);
        assert!(!snapshot.has_unsaved_changes);
        restored.shutdown().unwrap();

        let assigned_equalizer = Equalizer::with_filter(Filter::low_shelf(
            FrequencyHz::new(120.0).unwrap(),
            GainDb::new(4.0).unwrap(),
            QualityFactor::new(0.7).unwrap(),
        ));
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO profiles (id, name, equalizer_json, revision)
                 VALUES ('assigned', 'Assigned', ?1, 3)",
                [assigned_equalizer.to_canonical_json().unwrap()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO device_profile_assignments (device_id, profile_id)
                 VALUES ('fake', 'assigned')",
                [],
            )
            .unwrap();
        drop(connection);

        let assigned = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();

        let snapshot = assigned.snapshot();
        assert_eq!(snapshot.equalizer, assigned_equalizer);
        assert_eq!(snapshot.equalizer_revision.get(), 3);
        assigned.shutdown().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn manages_profiles_and_applies_active_device_assignments() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tunic-profile-crud-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let applied = Arc::new(Mutex::new(Vec::new()));
        let platform_applied = Arc::clone(&applied);
        let engine = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            move || FakePlatform {
                applied: platform_applied,
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();
        let headphone_equalizer = Equalizer::with_filter(Filter::low_shelf(
            FrequencyHz::new(90.0).unwrap(),
            GainDb::new(5.0).unwrap(),
            QualityFactor::new(0.8).unwrap(),
        ));
        engine
            .assign_profile(DeviceId::new("fake"), ProfileId::new("default"))
            .unwrap();
        engine
            .assign_profile(DeviceId::new("other-device"), ProfileId::new("default"))
            .unwrap();
        let edit_revision = engine.snapshot().edit_revision;
        let revision = engine
            .preview_equalizer(headphone_equalizer.clone(), edit_revision)
            .unwrap();
        assert_eq!(revision.get(), edit_revision.get() + 1);

        let headphones = engine.create_profile("Headphones".into()).unwrap();
        assert_eq!(headphones.as_str(), "headphones");
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.active_profile_id.as_ref(), Some(&headphones));
        assert_eq!(snapshot.default_profile_id.as_ref(), Some(&headphones));
        assert!(!snapshot.has_unsaved_changes);
        assert_eq!(snapshot.profiles.len(), 2);
        assert_eq!(snapshot.device_profile_assignments.len(), 1);
        assert_eq!(
            snapshot.device_profile_assignments[0].device_id,
            DeviceId::new("other-device")
        );

        engine
            .rename_profile(headphones.clone(), "Studio Headphones".into())
            .unwrap();
        assert_eq!(
            engine
                .snapshot()
                .profiles
                .iter()
                .find(|profile| profile.id == headphones)
                .unwrap()
                .name,
            "Studio Headphones"
        );

        engine
            .assign_profile(DeviceId::new("fake"), headphones.clone())
            .unwrap();
        engine
            .assign_profile(DeviceId::new("other-device"), headphones.clone())
            .unwrap();
        assert_eq!(engine.snapshot().device_profile_assignments.len(), 2);

        engine.select_profile(ProfileId::new("default")).unwrap();
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.equalizer, Equalizer::identity());
        assert_eq!(snapshot.active_profile_id, Some(ProfileId::new("default")));
        assert_eq!(snapshot.device_profile_assignments.len(), 1);
        assert_eq!(
            snapshot.device_profile_assignments[0].device_id,
            DeviceId::new("other-device")
        );
        engine
            .assign_profile(DeviceId::new("fake"), headphones.clone())
            .unwrap();
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.equalizer, headphone_equalizer);
        assert_eq!(snapshot.active_profile_id.as_ref(), Some(&headphones));
        assert_eq!(snapshot.device_profile_assignments.len(), 2);

        engine.delete_profile(headphones).unwrap();
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.equalizer, Equalizer::identity());
        assert_eq!(snapshot.profiles.len(), 1);
        assert!(snapshot.device_profile_assignments.is_empty());
        assert_eq!(snapshot.active_profile_id, Some(ProfileId::new("default")));
        assert_eq!(
            applied.lock().unwrap().as_slice(),
            &[
                headphone_equalizer.clone(),
                Equalizer::identity(),
                headphone_equalizer,
                Equalizer::identity(),
            ]
        );

        engine.shutdown().unwrap();
        let restored = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.equalizer, Equalizer::identity());
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.default_profile_id, Some(ProfileId::new("default")));
        assert_eq!(snapshot.active_profile_id, Some(ProfileId::new("default")));
        assert!(snapshot.device_profile_assignments.is_empty());
        restored.shutdown().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_profile_selection_with_an_unsaved_preview() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tunic-profile-unsaved-{}-{nonce}.sqlite3",
            std::process::id()
        ));
        let engine = Engine::start(
            EngineOptions {
                database_path: Some(path.clone()),
                ..EngineOptions::default()
            },
            || FakePlatform {
                applied: Arc::new(Mutex::new(Vec::new())),
                reject_equalizer: false,
                events: None,
                rebuild_sample_rate_hz: 48_000.0,
            },
        )
        .unwrap();
        let profile = engine.create_profile("Other".into()).unwrap();
        engine.select_profile(ProfileId::new("default")).unwrap();
        let preview = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_500.0).unwrap(),
            GainDb::new(-4.0).unwrap(),
            QualityFactor::new(1.2).unwrap(),
        ));
        engine
            .preview_equalizer(preview.clone(), engine.snapshot().edit_revision)
            .unwrap();

        let error = engine.select_profile(profile).unwrap_err();
        assert!(error.to_string().contains("save or discard"));
        assert_eq!(engine.snapshot().equalizer, preview);

        engine.shutdown().unwrap();
        fs::remove_file(path).unwrap();
    }

    struct FakePlatform {
        applied: Arc<Mutex<Vec<Equalizer>>>,
        reject_equalizer: bool,
        events: Option<Arc<Mutex<Option<PlatformEventSink>>>>,
        rebuild_sample_rate_hz: f64,
    }

    impl AudioPlatform for FakePlatform {
        fn start(
            &mut self,
            events: PlatformEventSink,
            _output_sink: Option<Arc<dyn ProcessedOutputSink>>,
            _telemetry: TelemetryPublisher,
            _equalizer: &Equalizer,
            _bypass: BypassControl,
        ) -> Result<PlatformState, PlatformError> {
            if let Some(target) = &self.events {
                *target.lock().unwrap() = Some(events);
            }
            Ok(platform_state(48_000.0))
        }

        fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError> {
            Ok(platform_state(self.rebuild_sample_rate_hz))
        }

        fn set_equalizer(&mut self, equalizer: &Equalizer) -> Result<(), PlatformError> {
            if self.reject_equalizer {
                return Err(PlatformError::new("equalizer rejected"));
            }
            self.applied.lock().unwrap().push(equalizer.clone());
            Ok(())
        }

        fn shutdown(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    struct BlockingRebuildPlatform {
        event_sink: std::sync::mpsc::SyncSender<(PlatformEventSink, BypassControl)>,
        rebuild_entered: std::sync::mpsc::SyncSender<()>,
        release_rebuild: std::sync::mpsc::Receiver<()>,
    }

    impl AudioPlatform for BlockingRebuildPlatform {
        fn start(
            &mut self,
            events: PlatformEventSink,
            _output_sink: Option<Arc<dyn ProcessedOutputSink>>,
            _telemetry: TelemetryPublisher,
            _equalizer: &Equalizer,
            bypass: BypassControl,
        ) -> Result<PlatformState, PlatformError> {
            self.event_sink.send((events, bypass)).unwrap();
            Ok(platform_state(48_000.0))
        }

        fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError> {
            self.rebuild_entered.send(()).unwrap();
            self.release_rebuild.recv().unwrap();
            Ok(platform_state(48_000.0))
        }

        fn set_equalizer(&mut self, _equalizer: &Equalizer) -> Result<(), PlatformError> {
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
