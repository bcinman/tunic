# Workspace Structure

Tunic separates portable product behavior from native platform integration.

## `tunic-core`

**Responsibility:** Pure domain logic, application orchestration, and real-time
audio processing.

- Defines profiles, chains, filters, Session commands, and application state.
- Keeps profile rules and durable state transitions pure. Session owns saved
  state, drafts, selection, errors, and desired versus controller-accepted chains.
- Commands describe the active editing workflow, not arbitrary profile replacement.
  Only Session's draft can be saved; there are no profile revision counters.
  Source preset revisions remain provenance metadata.
- Persists snapshots through `Persistence`; it does not implement durable storage.
- Coordinates previews and route lifecycle through the controller and an abstract
  `Platform`. Native notifications enter as `RefreshAudio` commands.
- Retains a platform only after watcher installation succeeds. Route failures,
  unresolved desired-chain publication failures, and action errors have separate
  lifetimes, so an unrelated successful action cannot hide an audio failure.
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
- Implements the core's `PresetCatalog` interface. Session copies selected
  preset base chains, named controls, and attribution into independent profiles.
- Depends on `tunic-core`; the core does not depend on the bundled catalog.

## `tunic-ui`

**Responsibility:** Shared GPUI presentation.

- Owns GPUI entities, rendering, click handlers, and transient presentation state.
- Receives a Session from the app; sends commands and reads state through
  read-only methods. It has no mutable access to Session internals.
- Converts pointer coordinates to frequency/gain commands. Drafts, save/reset,
  profile creation, catalog access, and controller publication belong to Session.
- Animates telemetry supplied by Session; route changes replace the subscription.
- Does not own platform devices, audio callbacks, or native resources.

## `tunic-app`

**Responsibility:** GPUI application lifecycle and composition.

- Selects GPUI's native platform backend and opens the application window.
- Constructs Session with `MemoryPersistence`, the bundled catalog, and Apple’s
  implementation of the core `Platform` contract.
- Schedules Session refresh commands after native notifications and retries when
  Session reports a retryable failure.
- Passes Session to the shared `tunic-ui` root view.
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

`apps/tunic-native` is a separate SwiftUI menu-bar prototype. It currently owns
only a static status item and application lifecycle, and does not depend on or
link to the Rust workspace.

The desktop host and platform integrations implement native side effects:

- UI and application lifecycle;
- device discovery and permissions;
- native audio capture and playback wiring;
- processor creation and callback ownership;

Session coordinates those interfaces with drafts and saved state, including
loading and saving through an injected persistence adapter. It does not depend
on GPUI or native platform APIs.

## Dependency Direction

```text
┌───────────┐────▶ tunic-ui ─────────┐
│ tunic-app │────▶ tunic-presets ────┤
└─────┬─────┘                       ▼
      └─────────▶ tunic-macos ──▶ tunic-core
```
