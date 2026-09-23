# `tunic-engine`

**Responsibility:** Authoritative product state and non-real-time coordination.

- Owns profiles and per-device preferences.
- Owns SQLite persistence.
- Tracks desired state, observed output, and active processing state.
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
    pub fn set_bypassed(&self, value: bool) -> Result<(), SubmitError>;
    pub fn select_profile(&self, id: ProfileId) -> Result<(), SubmitError>;

    pub fn create_profile(
        &self,
        name: String,
        configuration: Configuration,
    ) -> Result<(), SubmitError>;

    pub fn save_configuration(
        &self,
        configuration: Configuration,
        expected_revision: ConfigurationRevision,
    ) -> Result<(), SubmitError>;

    pub fn preview_configuration(
        &self,
        configuration: Configuration,
        edit_revision: EditRevision,
    ) -> Result<(), SubmitError>;

    pub fn discard_preview(&self) -> Result<(), SubmitError>;

    pub fn snapshots(&self) -> SnapshotReceiver;
    pub fn telemetry(&self) -> TelemetryReader;

    pub fn shutdown(&self) -> Result<(), ShutdownError>;
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

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
