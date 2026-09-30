# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built as a portable, side-effect-free Rust core with a shared GPUI
desktop interface. Platform integrations own device discovery, audio wiring,
permissions, and lifecycle.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Domain state, business rules, DSP, real-time processing, telemetry, and storage contracts. |
| [`tunic-ui`](crates/tunic-ui) | Shared GPUI presentation and direct integration with the core backend. |
| [`tunic-desktop`](crates/tunic-desktop) | Thin GPUI desktop executable, currently verified on macOS. |
| [`tunic-presets`](crates/tunic-presets) | Embedded headphone presets, validated JSON, and indexed brand/model queries. |
| [`tunic-ffi`](crates/tunic-ffi) | Native bindings, including the zero-copy real-time audio entry point. |

Run the GPUI application on macOS with `mise run run-desktop`. The earlier
SwiftUI/FFI integration remains available while the GPUI spike is evaluated.
