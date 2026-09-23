# `tunic-macos`

**Responsibility:** Core Audio integration and macOS real-time execution.

- Implements `AudioPlatform`.
- Observes the system default output.
- Manages system-audio permissions.
- Owns process taps, aggregate devices, IOProcs, and callback contexts.
- Negotiates stream formats.
- Owns capture/render transport, resampling, graph publication, and crossfades.
- Publishes callback-safe telemetry.
- Guarantees ordered activation, handoff, retirement, and teardown.
- Contains no profile, persistence, or UI policy.

## Rough public interface

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

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
