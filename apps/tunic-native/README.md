# Tunic Native

A thin macOS menu-bar frontend built with SwiftUI. `TunicModel` observes Rust
snapshots and sends commands through BoltFFI; profiles, drafts, save/reset,
route recovery, and audio processing stay in Rust.

Run it from the repository root with:

```console
mise run run-native
```

This builds the macOS arm64 XCFramework and generated Swift package under
`target/tunic-ffi/apple` before launching the app. Run `mise run build-ffi` before
opening the Swift package in Xcode. Rust changes require regenerating bindings
and rebuilding the native app. Tool versions are pinned in `mise.toml` and Cargo.

The app owns one engine for its entire lifetime. Closing the popup stops meter
polling and FFT demand, not audio processing. Quit tears down the Core Audio
route. Save currently uses in-memory persistence and does not survive quitting.
System Audio Recording permission is required; development from the permitted
terminal follows the same path as the GPUI app. Do not run both audio hosts at once.

`mise run check-native` regenerates bindings and runs the Swift integration tests
without accessing audio devices. The opt-in live audio smoke test opens a route
and plays a brief system sound at 1% gain to verify nonzero telemetry:

```console
TUNIC_TEST_AUDIO=1 mise run check-native
```

To render representative popup states from real offline Rust sessions, point
`TUNIC_SCREENSHOTS` at an existing directory when running `mise run check-native`.
