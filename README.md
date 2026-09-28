# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built as a portable, side-effect-free Rust core embedded in native
platform applications. Native apps own their UI, device integration, audio
wiring, permissions, and lifecycle.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Domain state, business rules, DSP, real-time processing, telemetry, and storage contracts. |
| [`tunic-ffi`](crates/tunic-ffi) | Native bindings, including the zero-copy real-time audio entry point. |
