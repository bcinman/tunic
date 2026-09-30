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
- Publishes the selected profile chain through a core `Controller`.
- Does not own platform devices, audio callbacks, or native resources.

## `tunic-app`

**Responsibility:** GPUI application lifecycle and composition.

- Selects GPUI's native platform backend and opens the application window.
- Starts and retains the platform audio session.
- Passes the platform's core `Controller` to the shared `tunic-ui` root view.
- Is currently built and visually verified on macOS only.

## `tunic-macos`

**Responsibility:** Core Audio system-output processing.

- Opens the current default output only; device-change recovery is intentionally deferred.
- Owns the process tap, private aggregate device, IOProc, and ordered teardown.
- Normalizes native buffers to interleaved stereo for a callback-owned core `Processor`.
- Returns a core `Controller` for non-real-time chain publication.
- Contains no profile, persistence, or UI policy.

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

## Application

`tunic-app` is the active application. Its current macOS build processes system
output through `tunic-core`; selecting a bundled or flat profile publishes that
chain to the live processor.

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
                   ┌──────────┐────▶ tunic-presets
┌───────────┐─────▶│ tunic-ui │
│ tunic-app │      └────┬─────┘
└─────┬─────┘           └─────────▶ tunic-core
      │                              ▲
      └────────▶ tunic-macos ────────┘

Legacy SwiftUI spike ──▶ tunic-ffi ──┬──▶ tunic-presets
                                     └──▶ tunic-core
```
