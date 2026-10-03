# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built as a portable, side-effect-free Rust core with a shared GPUI
desktop interface. Platform integrations own device discovery, audio wiring,
permissions, and lifecycle.

An independent native macOS menu-bar prototype lives in
[`apps/tunic-native`](apps/tunic-native). It is currently static SwiftUI with no
Rust integration. Run it with `mise run run-native`.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Domain state, business rules, DSP, real-time processing, telemetry, and storage contracts. |
| [`tunic-ui`](crates/tunic-ui) | Shared GPUI presentation and direct backend/controller integration. |
| [`tunic-app`](crates/tunic-app) | GPUI application lifecycle and platform composition. |
| [`tunic-macos`](crates/tunic-macos) | Core Audio system-output route and callback ownership. |
| [`tunic-presets`](crates/tunic-presets) | Embedded headphone presets, validated JSON, and indexed brand/model queries. |

Run the GPUI application on macOS with `mise run run-app`.
