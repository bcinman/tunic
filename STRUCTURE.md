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
- Implements the core `Platform` contract as `MacosPlatform`, shared by both hosts.
- Exposes only `MacosPlatform`; route sessions, watchers, and native errors are private.
- Contains no profile, persistence, or UI policy.

## `tunic-ffi`

**Responsibility:** Foreign-language application composition and binding conversion.

- Wraps one Session with a thread-safe BoltFFI `Engine` handle. A standard-library
  channel serializes commands on a Rust worker; Session and native resources are
  constructed, used, and destroyed on that worker without requiring `Send`.
- Composes `MemoryPersistence`, `BundledCatalog`, and (on macOS) `MacosPlatform`.
  Offline construction supports tests and previews without opening an audio route.
- Validates foreign values into core's branded types and exposes owned snapshots.
  `enqueue` accepts commands without waiting for execution. Validation/enqueue
  errors throw immediately; execution errors appear in the latest snapshot and
  can be superseded by a later successful command before observation.
- Exposes independent invalidation streams. Consumers reread the latest snapshot
  rather than treating notifications as a history of state changes. Rust buffers
  one invalidation per subscriber; BoltFFI's Swift AsyncStream is unbounded, so
  consumers should keep draining it (the native model does, even with the popup closed).
- Owns route notifications, delayed retries, and telemetry subscription replacement.
  Frontends enable telemetry while visible and poll bounded frame batches.
- Exposes core frequency-response calculations in batches; no DSP is rewritten in
  Swift, and no real-time audio buffers cross the language boundary.
- Generates a macOS arm64 Swift package/XCFramework through `mise run build-ffi`.
  Neither core nor the platform crate depends on BoltFFI.

## Application

`tunic-app` is the GPUI application. Its current macOS build processes system
output through `tunic-core`; selecting a bundled or flat profile publishes that
chain to the live processor.

`apps/tunic-native` is a thin SwiftUI menu-bar frontend using `tunic-ffi`. An
application-owned observable model projects Rust snapshots into views and owns
the update task. Popup visibility controls telemetry demand, not engine lifetime.
The application delegate shuts the engine down before quitting. Save remains
in-memory in both applications; neither implements durable storage yet.

The native `TunicUI` target groups views by feature under `Features/`:
audio status, visualizer, profile editor, and levels. `ContentView` composes the
popup and owns its visibility-driven telemetry task. The visualizer owns local
style, demo, hover, and lab presentation state. Editor views receive snapshots
and command closures; they do not duplicate Rust's draft state. Live spectrum
and level views observe measurements separately from the editing controls.

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

SwiftUI ──▶ tunic-ffi ──▶ tunic-presets ──▶ tunic-core
                │                              ▲
                ├──────────────────────────────┤
                └─────▶ tunic-macos ────────────┘
```
