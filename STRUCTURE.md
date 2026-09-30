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
- Calls `tunic-core` and `tunic-presets` directly.
- Publishes the selected profile chain through a core `Controller`.
- Does not own platform devices, audio callbacks, or native resources.

## `tunic-app`

**Responsibility:** GPUI application lifecycle and composition.

- Selects GPUI's native platform backend and opens the application window.
- Defines the small `Platform`/`Connection` contract used by the application.
- Responds to native default-output notifications and replaces complete audio routes.
- Passes each route's core `Controller` to the shared `tunic-ui` root view.
- Is currently built and visually verified on macOS only.

## `tunic-macos`

**Responsibility:** Core Audio system-output processing.

- Opens the current default output and observes native default-output changes.
- Owns the process tap, private aggregate device, IOProc, and ordered teardown.
- Normalizes native buffers to interleaved stereo for a callback-owned core `Processor`.
- Returns a core `Controller` for non-real-time chain publication.
- Contains no profile, persistence, or UI policy.

## Application

`tunic-app` is the active application. Its current macOS build processes system
output through `tunic-core`; selecting a bundled or flat profile publishes that
chain to the live processor.

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
```
