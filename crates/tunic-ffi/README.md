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

## Profile backend

`Backend()` starts an empty, non-persistent session (the Rust constructor is
`new_in_memory`; BoltFFI emits it as a Swift initializer). State is lost
when the backend is released. Durable backend construction and telemetry bindings
are not implemented yet.

The backend must be created, called, and released on one owning thread (for
example the main thread), never from an audio callback or concurrently.

Native methods each translate inputs into one core command:

- `createProfileFromPreset`, `createFlatProfile`, `copyProfile`;
- `renameProfile`, `deleteProfile`;
- `selectProfile`, `clearProfile`;
- `updateProfile`, including `expectedRevision`.

The app supplies stable profile/device IDs. Every successful mutation returns
a complete `State` snapshot containing profiles and device selections; `state()`
reads the current snapshot. Returned values are independent copies. Core owns
name validation, revision checks, adjustment retention, and save-before-publish.
Typed errors retain conflict revisions, relevant IDs, and storage-failure messages.

The wrapper never changes a processor. The app coordinates accepted state with
its UI and processor, and handles transient audio previews separately.

Generate the macOS XCFramework and Swift package with:

```sh
mise run ffi-apple
```
