# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built around a portable Rust `Session` with pure domain rules and a
shared GPUI desktop interface. Session coordinates persistence and audio through
interfaces; platform integrations implement device discovery and audio wiring.

An independent native macOS menu-bar prototype lives in
[`apps/tunic-native`](apps/tunic-native). It is currently static SwiftUI with no
Rust integration. Run it with `mise run run-native`.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Session commands and state, business rules, DSP, telemetry, and persistence/platform contracts. |
| [`tunic-ui`](crates/tunic-ui) | Shared GPUI presentation; sends Session commands and reads Session state. |
| [`tunic-app`](crates/tunic-app) | GPUI application lifecycle and platform composition. |
| [`tunic-macos`](crates/tunic-macos) | Core Audio system-output route and callback ownership. |
| [`tunic-presets`](crates/tunic-presets) | Embedded headphone presets, validated JSON, and indexed brand/model queries. |

Run the GPUI application on macOS with `mise run run-app`.
