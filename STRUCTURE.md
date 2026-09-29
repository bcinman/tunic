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
- Does not own devices, audio callbacks, application state, or UI.

## Native applications

Native applications live outside the Rust workspace and own all side effects:

- UI and application lifecycle;
- device discovery and permissions;
- native audio capture and playback wiring;
- processor creation and callback ownership;
- coordinating backend state with transient processor previews;
- loading and saving state through a platform storage adapter.

## Dependency Direction

```text
┌────────────────────┐
│ Native application │
└─────────┬──────────┘
          │ generated API + direct audio buffer
          ▼
     ┌───────────┐      ┌───────────────┐
     │ tunic-ffi │─────▶│ tunic-presets │
     └─────┬─────┘      └───────┬───────┘
           ▼                    │
     ┌────────────┐             │
     │ tunic-core │◀────────────┘
     └────────────┘
```
