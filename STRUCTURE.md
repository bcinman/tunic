# Workspace Outline

- `tunic-app`
- `tunic-engine`
- `tunic-dsp`
- `tunic-macos`
- `tunic-cli` (optional diagnostic binary)

## `tunic-app`

**Responsibility:** GPUI application lifecycle and presentation.

- Owns the application-scoped engine handle.
- Creates, closes, and reopens the normal application window.
- Keeps the engine alive while no window is open.
- Renders snapshots, profiles, editor state, failures, and meters.
- Owns transient UI state and UI-only preferences.
- Performs orderly engine shutdown only on explicit Quit.

### Rough public interface

```rust
pub struct AppConfig {
    pub data_directory: PathBuf,
}

pub fn run(config: AppConfig) -> Result<(), AppError>;
```

Everything else can remain internal GPUI entities, views, and actions.

## `tunic-engine`

**Responsibility:** Authoritative product state and non-real-time coordination.

- Owns profiles and per-device preferences.
- Owns SQLite persistence.
- Tracks desired state, observed output, and active processing state.
- Publishes bypass through a shared atomic control so the real-time route does
  not wait on engine-thread work.
- Serializes non-real-time user requests and platform events.
- Coordinates route preparation, activation, retirement, retries, and shutdown.
- Publishes immutable state snapshots and latest telemetry.
- Defines the platform-audio contract implemented by macOS and future platforms.

### Rough public interface

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
    pub fn toggle_bypass(&self) -> bool;
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
    pub fn subscribe_snapshots(&self) -> SnapshotReceiver;
    pub fn subscribe_telemetry(&self) -> TelemetryReader;

    pub fn shutdown(self) -> Result<(), EngineError>;
}
```

### Platform contract

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

## `tunic-dsp`

**Responsibility:** Portable audio-processing definitions and algorithms.

- Defines the versioned equalizer.
- Validates and canonicalizes equalizers.
- Prepares processing graphs for a specific sample rate.
- Processes arbitrary stereo frame counts without allocation.
- Supports reset, bypass integration, and latency reporting.
- Contains mathematical, impulse-response, and frequency-response tests.
- Knows nothing about devices, GPUI, profiles, persistence, or Core Audio.

### Rough public interface

```rust
pub struct Equalizer;
pub struct PreparedGraph;

impl Equalizer {
    pub fn parse_json(input: &str) -> Result<Self, EqualizerError>;
    pub fn to_canonical_json(&self) -> Result<String, EqualizerError>;
}

impl PreparedGraph {
    pub fn prepare(
        equalizer: &Equalizer,
        sample_rate_hz: f64,
    ) -> Result<Self, EqualizerError>;

    pub fn process(&mut self, frames: &mut [[f32; 2]]);

    pub fn reset(&mut self);
    pub fn latency_frames(&self) -> usize;
}
```

## `tunic-macos`

**Responsibility:** Core Audio integration and macOS real-time execution.

- Implements AudioPlatform.
- Observes the system default output.
- Manages system-audio permissions.
- Owns process taps, aggregate devices, IOProcs, and callback contexts.
- Negotiates stream formats.
- Owns capture/render transport, resampling, graph publication, and crossfades.
- Publishes callback-safe telemetry.
- Guarantees ordered activation, handoff, retirement, and teardown.
- Contains no profile, persistence, or UI policy.

### Rough public interface

```rust
pub struct CoreAudioPlatform;

impl CoreAudioPlatform {
    pub fn new(
        options: CoreAudioOptions,
    ) -> Result<Self, CoreAudioError>;
}

impl AudioPlatform for CoreAudioPlatform {
    // Platform contract implementation
}
```

The app wires it directly into the engine:

```rust
let platform = CoreAudioPlatform::new(audio_options)?;
let engine = Engine::start(engine_options, platform)?;
tunic_app::run_with_engine(engine)?;
```

## `tunic-cli` (optional)

**Responsibility:** Headless diagnostics and deterministic testing.

- Runs the engine with either a fake platform or CoreAudioPlatform.
- Injects commands and platform failures.
- Emits snapshots and telemetry as structured output.
- Exercises restart, routing, and shutdown without GPUI.
- Reuses production engine and audio code without going through the app.

### Rough interface

```console
tunic-cli fake
tunic-cli macos
tunic-cli inspect
```

It likely needs no public Rust API; it is an executable consumer of the other crates.

## Dependency Direction

```text
┌───────────┐
│ tunic-app │──────────────┐
└─────┬─────┘              │
      ▼                    ▼
┌──────────────┐     ┌─────────────┐
│ tunic-engine │◀────│ tunic-macos │
└──────┬───────┘     └──────┬──────┘
       │                    │
       └─────────┬──────────┘
                 ▼
          ┌───────────┐
          │ tunic-dsp │
          └───────────┘

tunic-cli ──▶ tunic-engine
          └─▶ tunic-macos
```
