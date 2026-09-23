# `tunic-cli`

Optional diagnostic binary.

**Responsibility:** Headless diagnostics and deterministic testing.

- Runs the engine with either a fake platform or `CoreAudioPlatform`.
- Injects commands and platform failures.
- Emits snapshots and telemetry as structured output.
- Exercises restart, routing, and shutdown without GPUI.
- Reuses production engine and audio code without going through the app.

## Rough interface

```console
tunic-cli fake
tunic-cli macos
tunic-cli inspect
```

It likely needs no public Rust API; it is an executable consumer of the other crates.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
