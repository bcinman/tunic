# `tunic-engine`

**Responsibility:** Authoritative product state and non-real-time coordination.

- Owns profiles and per-device preferences.
- Owns SQLite persistence.
- Tracks desired state, observed output, and active processing state.
- Owns the persisted equalizer and in-memory live preview.
- Serializes user requests and platform events.
- Requests route rebuilds after default-output and sample-rate changes.
- Coordinates bypass, equalizer updates, and orderly shutdown with the platform.
- Publishes immutable state snapshots and latest telemetry.
- Defines the platform-audio contract implemented by macOS and future platforms.

## Public interface

`Engine::start` is the crate's entry point:

```rust
pub struct Engine;
pub struct EngineHandle;

impl Engine {
    pub fn start<P, F>(
        options: EngineOptions,
        platform: F,
    ) -> Result<EngineHandle, EngineError>
    where
        P: AudioPlatform,
        F: FnOnce() -> P + Send + 'static;
}
```

The platform is supplied as a factory because it is constructed and owned on
the engine thread. `start` blocks until persistence is open, the platform has
started, and the first running snapshot is available. Startup failures are
returned as `EngineError`; after success, `EngineHandle` is the only command
and observation surface.

`EngineOptions` configures the boundaries owned by the engine:

```rust
pub struct EngineOptions {
    pub processed_output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    pub equalizer: Equalizer,
    pub database_path: Option<PathBuf>,
}
```

- `equalizer` is the initial equalizer when persistence is disabled.
- `database_path` enables profile persistence and takes precedence over the
  initial equalizer.
- `processed_output_sink` receives post-DSP stereo samples, for example for WAV
  capture.

### Commands and observations

The handle serializes every mutation through the engine thread:

```rust
impl EngineHandle {
    pub fn snapshot(&self) -> EngineSnapshot;
    pub fn subscribe_snapshots(&self) -> SnapshotReceiver;
    pub fn subscribe_telemetry(&self) -> TelemetryReader;

    pub fn toggle_bypass(&self) -> bool;

    pub fn preview_equalizer(
        &self,
        equalizer: Equalizer,
        expected_revision: EditRevision,
    ) -> Result<EditRevision, EngineError>;
    pub fn save_equalizer(
        &self,
        expected_revision: EditRevision,
    ) -> Result<EqualizerRevision, EngineError>;
    pub fn discard_preview(
        &self,
        expected_revision: EditRevision,
    ) -> Result<EditRevision, EngineError>;

    pub fn create_profile(&self, name: String) -> Result<ProfileId, EngineError>;
    pub fn rename_profile(&self, id: ProfileId, name: String) -> Result<(), EngineError>;
    pub fn delete_profile(&self, id: ProfileId) -> Result<(), EngineError>;
    pub fn select_profile(&self, id: ProfileId) -> Result<(), EngineError>;
    pub fn assign_profile(
        &self,
        device_id: DeviceId,
        profile_id: ProfileId,
    ) -> Result<(), EngineError>;

    pub fn shutdown(self) -> Result<(), EngineError>;
}
```

Dropping the handle also shuts the engine down, but explicit `shutdown` reports
platform teardown errors. Command calls are synchronous: they return only after
the engine has accepted or rejected the operation.

### Snapshots and revisions

`EngineSnapshot` is the immutable read model for lifecycle, bypass, devices,
the live equalizer, profile catalog, profile selections, and revisions.
`EngineStatus::Running(ActiveRoute)` carries the active route, so a running
snapshot cannot exist without one. `EngineSnapshot::active_route()` provides a
uniform optional lookup across all lifecycle states. `Starting` is used during
synchronous startup, `Failed` records a route-rebuild failure, and `Stopped` is
the final state published during orderly shutdown.

`snapshot()` returns the latest value immediately. `subscribe_snapshots()`
returns a `SnapshotReceiver` whose `recv`, `recv_timeout`, and `try_recv`
methods first yield the current snapshot and then every changed snapshot in
engine order. No-op commands do not publish.

The two revision types have separate meanings:

- `EditRevision` changes whenever the live editor state changes. Callers pass
  the revision they observed to preview, save, and discard commands; stale
  writes are rejected.
- `EqualizerRevision` identifies the committed profile contents. Saving
  advances it in memory, and also in SQLite when persistence is enabled.

`DeviceId` and `ProfileId` are distinct identifier types. Both provide `new`,
`as_str`, and `Display`, preventing device and profile identifiers from being
mixed accidentally. Snapshot data uses `OutputDevice`, `ActiveRoute`, `Profile`,
and `DeviceProfileAssignment`; these are plain immutable values with public
fields for presentation and inspection.

## Platform interface

Platform crates implement `AudioPlatform`. The engine owns the implementation
and invokes it only from the engine thread:

```rust
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
```

`PlatformState` reports the active route and available output devices after a
successful start or rebuild. The platform sends `DefaultOutputChanged` and
`OutputSampleRateChanged` through `PlatformEventSink`; the engine then requests
a route rebuild and updates its state. `BypassControl` is shared directly with
the real-time route so bypass never waits behind engine-thread work.
`PlatformError::new` lets platform implementations preserve contextual error
messages without exposing their internal error types.

## Processed output

`ProcessedOutputSink` is an optional post-DSP capture boundary. The platform
calls `configure(ProcessedOutputFormat)` when creating a route and calls
`write(&[f32])` with interleaved stereo samples from the real-time audio thread.
`write` must not block or allocate. Sink setup failures use
`OutputSinkError::new` and fail route construction.

## Persistence

When `EngineOptions::database_path` is set, the engine creates a versioned
SQLite database containing profiles, the fallback profile selection, and
per-device profile assignments. A fresh database contains one `Default`
profile. Equalizers are stored as versioned JSON documents, while profile
revisions and relationships remain relational. The CLI stores this database at
`~/Library/Application Support/Tunic/tunic.sqlite3` by default.

Profile create, rename, delete, select, and assignment commands require
persistence. Without `database_path`, the engine still supports live preview,
in-memory save and discard, bypass, snapshots, telemetry, and shutdown.

Creating a profile saves the current live equalizer into it and selects it as
the default. Selecting a profile clears an assignment for the active device so
the selected default takes effect immediately. Assignment commands override the
default for their device. Commands that would replace the active equalizer
require any preview to be saved or discarded first.

## Telemetry

The engine owns a lock-free latest-value channel for post-DSP stereo peak/RMS
levels and a fixed 28-band spectrum from 31.5 Hz through 16 kHz. The platform
publishes smoothed, decaying measurements from its real-time audio callback
only while at least one `TelemetryReader` exists. Readers act as meter
subscriptions: cloning one keeps the same subscription alive, and dropping the
last clone disables measurement when no other reader exists. Telemetry is
intentionally separate from engine snapshots and commands so a meter can poll
it without waking or blocking the engine thread.

Consumers obtain `TelemetryReader` from `EngineHandle::subscribe_telemetry()`
and poll `try_latest()`. The platform receives `TelemetryPublisher` during
startup. It calls `active_generation()` before doing measurement work and
publishes a `TelemetryFrame` for that generation; publications for inactive or
superseded generations are ignored. `TelemetryFrame` contains `StereoLevels`
and `Spectrum`, while `SPECTRUM_FREQUENCIES_HZ` defines the center frequency of
each of the `SPECTRUM_BAND_COUNT` bands.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
