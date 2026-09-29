# `tunic-ffi`

Native-language bindings for `tunic-core`. BoltFFI generates the control API
and Apple package, while `tunic_processor_process_realtime` provides the
allocation-free audio path as a handwritten C ABI over a processor token and
caller-owned mutable `f32` buffer.

The generated Apple package includes a handwritten `RealtimeProcessor` Swift
wrapper over the raw C symbol. It accepts the native callback's existing
`UnsafeMutablePointer<Float>` rather than a Swift array. `RealtimeProcessor`
retains its BoltFFI `Processor`; construct and release both away from the audio
callback.

This spike covers processor construction, live chain and bypass control,
the explicit render entry point, and typed preset catalog access through
`PresetCatalog.brands`, `models`, `list_presets`, and `preset`. Catalog queries
use exact, case-sensitive brand/model filters; an unknown ID returns no preset.
Filters carry stable positive IDs scoped to their chain, including across FFI.
Backend and telemetry bindings are intentionally
deferred until this boundary is proven in a native app.

Generate the macOS XCFramework and Swift package with:

```sh
mise run ffi-apple
```
