# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built around a portable Rust `Session` with pure domain rules and
GPUI and SwiftUI interfaces. Session coordinates persistence and audio through
interfaces; platform integrations implement device discovery and audio wiring.

A native macOS menu-bar app lives in [`apps/tunic-native`](apps/tunic-native).
It uses `tunic-ffi` for Rust-owned state and audio processing. Run it with
`mise run run-native`; bindings are generated automatically.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Session commands and state, business rules, DSP, telemetry, and persistence/platform contracts. |
| [`tunic-ffi`](crates/tunic-ffi) | BoltFFI engine handle, snapshots, commands, DSP analysis, and Swift package generation. |
| [`tunic-ui`](crates/tunic-ui) | Shared GPUI presentation; sends Session commands and reads Session state. |
| [`tunic-app`](crates/tunic-app) | GPUI application lifecycle and platform composition. |
| [`tunic-macos`](crates/tunic-macos) | Core Audio system-output route and callback ownership. |
| [`tunic-presets`](crates/tunic-presets) | Embedded headphone presets, validated JSON, and indexed brand/model queries. |

Run the GPUI application on macOS with `mise run run-app`.

## Development checks

Install the pinned tools with `mise install`, then run `mise run check` for
Rust formatting, Clippy, tests, and the native Swift checks.

Run `mise run lint-native` for SwiftLint alone. It checks the native package
manifest, sources, and tests in strict mode (warnings fail the check), excluding
generated bindings and build output. Rules live in `.swiftlint.yml`, with
test-only size and complexity exceptions under `apps/tunic-native/Tests/`.
Use `mise exec -- swiftlint lint --fix` to apply supported automatic corrections.
