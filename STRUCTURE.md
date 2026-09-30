# Workspace Structure

Tunic separates portable product behavior from native platform integration.

## `tunic-core`

**Responsibility:** Side-effect-free domain logic and real-time audio
processing.

- Defines profiles, chains, filters, commands, and backend state.
- Applies commands as pure state transitions.
- Persists snapshots through an abstract `Store`; it does not implement durable
  storage.
- Compiles chains into sample-rate-specific DSP.
- Processes interleaved stereo without allocation or locking.
- Publishes live chain and bypass changes through a callback-safe controller.
- Produces demand-driven peak/RMS and spectrum telemetry.
- Knows nothing about platform APIs, hardware, UI frameworks, or databases.

## `tunic-presets`

**Responsibility:** The bundled, read-only headphone preset catalog.

- Stores one JSON file per preset under `data/<source>/<brand>/`.
- Shares strict Serde decoding between build-time validation and runtime lookup.
- Generates static brand/model and preset-ID indexes at build time.
- Decodes only the requested payload; browsing never parses JSON or scans files.
- Implements the core's `PresetCatalog` interface. The backend copies selected
  preset chains, attribution, and adjustment mappings into independent profiles.
- Depends on `tunic-core`; the core does not depend on the bundled catalog.

## `tunic-ui`

**Responsibility:** Shared GPUI presentation.

- Owns GPUI entities, rendering, click handlers, and transient presentation state.
- Calls `tunic-core` and `tunic-presets` directly without an FFI conversion layer.
- Renders backend state but does not own product rules, devices, or audio callbacks.

## `tunic-desktop`

**Responsibility:** GPUI application lifecycle and composition.

- Selects GPUI's native platform backend and opens the application window.
- Constructs the shared `tunic-ui` root view.
- Is currently built and visually verified on macOS only.

## `tunic-ffi`

**Responsibility:** Expose the core to native applications without weakening
the real-time boundary.

- Uses BoltFFI for processor construction, domain conversion, errors, and
  non-real-time controller methods.
- Exposes a handwritten C ABI that processes a caller-owned mutable audio
  buffer directly.
- Packages the bindings as an XCFramework and Swift package.
- Constructs the bundled catalog and exposes typed brand/model queries and preset
  lookup. Native apps never parse catalog JSON.
- Wraps the core backend with native methods that translate values and issue one
  command each. Profile rules and authoritative state remain in core; the current
  native constructor uses an in-memory store.
- Does not own devices, audio callbacks, UI, or backend-to-processor coordination.

## Native applications

`tunic-desktop` is the active cross-platform UI spike. Its current macOS build
shows the bundled presets and executes profile selection directly through the
core backend. Audio is not connected yet.

`native/macos` retains the earlier SwiftUI/Xcode integration for comparison. It
depends on the generated local Swift package in `dist/apple` and remains a
placeholder rather than the intended application architecture.

The desktop application and future platform integrations own all side effects:

- UI and application lifecycle;
- device discovery and permissions;
- native audio capture and playback wiring;
- processor creation and callback ownership;
- coordinating backend state with transient processor previews;
- loading and saving state through a platform storage adapter.

## Dependency Direction

```text
┌───────────────┐     ┌──────────┐
│ tunic-desktop │────▶│ tunic-ui │
└───────────────┘     └────┬─────┘
                           ├──────▶ tunic-presets
                           └──────▶ tunic-core

Legacy SwiftUI spike ──▶ tunic-ffi ──┬──▶ tunic-presets
                                     └──▶ tunic-core
```
