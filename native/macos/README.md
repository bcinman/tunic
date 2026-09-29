# Tunic for macOS

A minimal SwiftUI app with a static device label and placeholder profile dropdown that links the generated
`Tunic` FFI package. Requires Xcode and an Apple Silicon Mac running macOS 13 or newer.

From the repository root:

```console
mise run run-macos
```

This regenerates the bindings, builds an ad-hoc-signed development app, and
launches it. Use `mise run build-macos` to build without launching.

To work in Xcode, first run `mise run ffi-apple`, then open
`native/macos/Tunic.xcodeproj` and select the `Tunic` scheme. Regenerate the
bindings after changing the Rust API; generated files stay in `dist/apple`.

The app does not initialize a backend, request permissions, or start audio yet.
