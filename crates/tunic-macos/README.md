# `tunic-macos`

**Responsibility:** Core Audio integration and macOS real-time execution.

- Implements `AudioPlatform`.
- Observes the system default output.
- Manages system-audio permissions.
- Owns process taps, aggregate devices, IOProcs, and callback contexts.
- Negotiates stream formats.
- Owns native capture/render transport and buffer normalization.
- Invokes the engine's portable real-time shell and writes processed stereo
  samples back to Core Audio.
- Guarantees ordered activation, handoff, retirement, and teardown.
- Contains no profile, persistence, or UI policy.

## Rough public interface

```rust
pub struct CoreAudioPlatform;

impl CoreAudioPlatform {
    pub fn new() -> Self;
}

impl AudioPlatform for CoreAudioPlatform {
    // Platform contract implementation
}
```

The app wires it directly into the engine:

```rust
let engine = Engine::start(engine_options, CoreAudioPlatform::new)?;
```

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
