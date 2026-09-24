# `tunic-engine`

**Responsibility:** Authoritative product state and non-real-time coordination.

- Owns profiles and per-device preferences.
- Owns SQLite persistence.
- Tracks desired state, observed output, and active processing state.
- Owns the persisted equalizer and in-memory live preview.
- Serializes user requests and platform events.
- Coordinates route preparation, activation, retirement, retries, and shutdown.
- Publishes immutable state snapshots and latest telemetry.
- Defines the platform-audio contract implemented by macOS and future platforms.

## Rough public interface

```rust
pub struct Engine;
pub struct EngineHandle;

impl Engine {
    pub fn start(
        options: EngineOptions,
        platform: impl AudioPlatform,
    ) -> Result<EngineHandle, EngineError>;
}

impl EngineHandle {
    pub fn toggle_bypass(&self) -> Result<bool, EngineError>;
    pub fn create_profile(&self, name: String) -> Result<ProfileId, EngineError>;
    pub fn rename_profile(&self, id: ProfileId, name: String) -> Result<(), EngineError>;
    pub fn delete_profile(&self, id: ProfileId) -> Result<(), EngineError>;
    pub fn select_profile(&self, id: ProfileId) -> Result<(), EngineError>;
    pub fn assign_profile(
        &self,
        device_id: DeviceId,
        profile_id: ProfileId,
    ) -> Result<(), EngineError>;

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

    pub fn snapshot(&self) -> EngineSnapshot;
    pub fn telemetry(&self) -> TelemetryReader;

    pub fn shutdown(self) -> Result<(), EngineError>;
}
```

## Platform contract

```rust
pub trait AudioPlatform: Send + 'static {
    fn start(&mut self, events: PlatformEventSink)
        -> Result<(), PlatformError>;

    fn prepare_route(
        &mut self,
        request: PrepareRouteRequest,
    ) -> Result<(), PlatformError>;

    fn activate_route(
        &mut self,
        request: ActivateRouteRequest,
    ) -> Result<(), PlatformError>;

    fn retire_route(&mut self, route: RouteId);
    fn shutdown(&mut self) -> Result<(), PlatformError>;
}
```

Commands, events, effects, and reducer details can remain private to the engine.

## Persistence

When `EngineOptions::database_path` is set, the engine creates a versioned
SQLite database containing profiles, the fallback profile selection, and
per-device profile assignments. A fresh database contains one `Default`
profile. Equalizers are stored as versioned JSON documents, while profile
revisions and relationships remain relational. The CLI stores this database at
`~/Library/Application Support/Tunic/tunic.sqlite3` by default.

Creating a profile saves the current live equalizer into it and selects it as
the default. Selecting a profile clears an assignment for the active device so
the selected default takes effect immediately. Assignment commands override the
default for their device. Commands that would replace the active equalizer
require any preview to be saved or discarded first.

## Telemetry

The engine owns a lock-free latest-value channel for post-DSP stereo peak and
RMS levels. The platform publishes from its real-time audio callback, while
clients obtain an independent `TelemetryReader` from `EngineHandle::telemetry`.
Telemetry is intentionally separate from engine snapshots and commands so a
meter can poll it without waking or blocking the engine thread.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
