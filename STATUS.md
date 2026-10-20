# Project Status

Tunic has a portable Rust core with GPUI and native SwiftUI frontends. GPUI calls
Session directly; SwiftUI uses the BoltFFI application boundary in `tunic-ffi`.
Both frontends use the same macOS audio adapter and Rust DSP.

This file is the source of truth for implemented product capabilities.

## In

- A portable `tunic-core` containing pure profile, chain, and device-selection
  rules, with a GPUI-independent `Session` application boundary.
- Session commands own profile selection, draft editing, save/reset, audio
  previews, and route refresh. Read-only accessors expose application state;
  GPUI retains only gestures, presentation, and spectrum animation.
- A narrow command API for creating named profiles from presets or a neutral
  five-band EQ, selecting saved profiles, filter/control edits, save/reset,
  and audio refresh. Session owns the only draft;
  callers cannot submit replacement profiles, and profiles need no revision counters.
- Devices own independent profile collections and remember their last selection.
  Creation atomically saves and selects a fresh profile; names are unique per device.
  Stale-device commands and selecting another device's profile are rejected.
  All edits carry the originating profile ID; stale slider updates, completion,
  cancellation, and draft commands cannot affect a newly selected profile.
  Audio-backed sessions reject profile creation while no route is available;
  the native Add button waits for a real device rather than saving to a placeholder.
  Explicit offline test/preview sessions still support profile creation.
  Flat starts with neutral Bass, Warmth, Mids, Presence, and Treble controls.
- Atomic whole-state persistence through a `Persistence` contract and rejection
  of corrupt restored state. Failed saves retain drafts. DSP publication errors
  remain visible across no-op commands until resolved; refresh cannot bypass a
  failed output-watcher installation.
- A `tunic-sqlite` persistence adapter using bundled SQLite, atomic JSON snapshots,
  and `synchronous=EXTRA` commits with macOS full-drive flushing. Versioned SQL
  migrations run transactionally; newer schemas are rejected without resetting
  saved data. Both macOS hosts use
  `~/Library/Application Support/Tunic/session.sqlite3`; tests and previews retain
  memory storage. Startup storage errors surface rather than falling back to memory.
  The device-owned snapshot format is incompatible with earlier development data;
  old snapshots are rejected without silently resetting or modifying their payload.
  Saved profiles, adjustments, attribution, and selections survive relaunch;
  named gain adjustments preview in audio without disk writes during a drag and
  persist once when the interaction ends, without saving pending base EQ edits.
  Cancellation or a failed gain save restores the committed adjustment. Profile
  changes finish pending gain previews before switching; native panel dismissal
  and orderly engine shutdown also finish previews. Unfinished drags are not crash-durable.
  Only base EQ edits require Save/Reset; unsaved base drafts and runtime/visualizer
  state do not survive relaunch. Failed adjustment saves preserve prior saved state
  and base drafts and restore audio to the committed adjustment.
- An offline `tunic-presets` catalog with build-validated JSON, static brand/model
  indexes, and on-demand payload decoding. The oratory1990 full-list import bundles
  1,062 of 1,070 index entries; eight unsupported/ambiguous sources are blocked.
  `crates/tunic-presets/ISSUES.md` tracks blockers and 138 imported entries needing
  review. A repeatable macOS PDFKit/Python importer preserves source hashes and
  full extracted text in a manifest. Clear single-band gain recommendations become
  controls; other guidance remains source metadata, not in-app instructions.
  Multi-band, frequency/Q, and conditional recommendations are not yet modeled.
- Profiles own an editable base chain plus named controls targeting stable filter
  identities. Each control holds a relative gain adjustment; effective DSP chains
  add those adjustments without changing base values.
- Sample-rate-specific stereo DSP with preamp gain, ordered peaking and shelf
  filters, float64 coefficients/history, stability validation, float32 audio buffers,
  and allocation-free processing. Independent impulse/FFT comparisons against
  prewarped analog prototypes cover 144 filter configurations at 44.1–192 kHz;
  block-size invariance and exact bypass nulling run in CI. This caught a float32
  precision error of 2.51 dB in the tested bass case at 192 kHz; the same matrix
  measures under 0.000003 dB worst error with float64 internals.
- Audio verification commands: `check-audio-fidelity`, an opt-in unmuted
  `audit-audio-rate`, and `audio-null` for generating a stereo probe and comparing
  PCM WAV loopback captures without fitting away gain or drift. The comparator
  is tested against latency, gain changes, channel swaps, and dropped samples.
  The live aggregate stream-rate audit passed at 48 kHz on the development machine;
  physical/virtual output loopback fidelity has not yet been measured.
- A cloneable controller that publishes latest-value chain replacements and
  atomic bypass changes to the processor. Chain replacements use a
  five-millisecond crossfade.
- A Processing toggle in the native gear menu bypasses processing without muting audio or changing
  profiles. Session retains the setting across route changes; launch defaults to on.
- Demand-driven, allocation-free post-output telemetry with a lock-free history
  of sequenced raw stereo peak/RMS measurements and 256-point spectra.
- A minimal GPUI-CE application split into shared `tunic-ui` presentation and
  `tunic-app` lifecycle/platform composition. It browses bundled presets,
  creates independent profiles, and selects saved profiles for the current device.
- A `tunic-macos` route for the current default output using a Core Audio process
  tap, private aggregate device, callback-owned core `Processor`, native-buffer
  normalization, and ordered teardown. Profile clicks publish live chain changes
  through the core `Controller`. Startup rejects aggregate virtual stream rates
  that differ from the DSP rate, rather than silently processing at the wrong rate.
- A Session-owned `Platform` contract with a shared `tunic-macos::MacosPlatform`
  that observes Core Audio default-output notifications and rebuilds the complete
  route using stable Core Audio device UIDs. Same-device reconnections retain the
  current draft; device switches start with that device's saved selection and
  discard the previous base draft. Each host schedules refresh commands and retries.
- A polished 640-pixel desktop shell with native traffic lights, current device
  and profile context, and a low-opacity real-time spectrum behind the equalizer.
- An editable logarithmic equalizer graph with the exact digital filter response,
  live drag previews through the processor controller, endpoint frequency labels,
  and explicit save/reset. The graph edits the base EQ while centered named-control
  sliders apply ±12 dB gain adjustments.
- macOS GPUI development through `mise run build-app` and `mise run run-app`.
- A `tunic-ffi` crate exporting a thread-safe engine handle, validated commands,
  owned snapshots, state invalidations, batched EQ response queries, and
  demand-driven peak/RMS/spectrum telemetry. One Rust worker owns Session and
  native resources, retries route failures, and tears down on explicit shutdown
  or handle drop. No audio buffers or real-time callbacks cross into Swift.
- A SwiftUI menu-bar app linked to generated BoltFFI bindings, with real device
  status, device-scoped profile selection, and automatically saved named gain controls.
  Hovering a nonzero gain row shows a reset icon after its control name;
  clicking it saves a return to 0 dB.
  An unboxed top-left device/profile picker places its chevron beside the profile name.
  A top-right unboxed gear brightens on hover, with a 24 pt hit area and
  a menu offering Quit Tunic and Open at Login through macOS
  ServiceManagement; login startup requires a signed app bundle and is disabled
  in the `swift run` development build. `mise run build-native-bundle` produces a
  locally signed development app at `target/native/Tunic.app` with permission metadata.
  Devices without profiles open headphone search with live, variant-aware results,
  editable profile names, and a secondary flat-EQ starting point. The normal picker
  lists only the current device's profiles and reuses that form for later additions.
  Browsing does not change audio; failed additions retain the form for correction.
  The picker shows the current output above the selected profile. Its menu selects
  profiles for that output; macOS owns device selection. Level meters are not shown.
  A transparent Metal `MTKView` visualizer renders the spectrum. Hovering a gain
  control keeps the affected spectrum at full opacity, fading unaffected frequencies
  to 25% using the filter's gain sensitivity, and fades in the combined EQ response: a subtle white
  stroke with a brighter 2-point white highlight and white fill weighted by that filter's gain
  sensitivity, including at zero gain. Sliders keep their normal appearance.
  During a drag, the curve stays pinned to that control even outside its hover
  area; release or cancellation returns to normal pointer-based highlighting.
  Hover fades respect Reduce Motion. A native disclosure row beneath the spectrum opens
  a scrollable settings area in the same panel; visual settings survive collapse.
  Disclosure uses standard SwiftUI behavior with no custom expansion or window animation.
  The visualizer uses only SDF geometry with grid fill shading. Grid fill
  fills beneath the curve with circle or square cells on a regular grid (no hex layout),
  using a selectable solid color or horizontal gradient, with
  four editable color stops at 0%, 33%, 67%, and 100% of the frequency axis.
  Gradient defaults use bright cyan, mint, lavender, and pink, interpolated between stops.
  Maximum cell size and center spacing are adjustable. Independent size and opacity
  responses map bottom-to-curve position to cell appearance; zero keeps size constant
  or opacity full. Level influence additionally scales that response by displayed
  spectrum height. Sizes above the spacing overlap into connected
  areas. Boundary cells shrink to fit their full shapes
  inside the spectrum and viewport, including their antialiased edges, rather than
  being sliced by the fill mask. Square fitting accounts for corners on slopes and in valleys.
  The default preserves the halftone taper: smaller at the bottom and larger toward
  the top of each frequency's filled area, with size response 1 and opacity response 0.
  Extra redraws stop when the spectrum settles or the view is hidden. A static demo
  input supports tuning without audio. Visual settings are ephemeral and separate
  from audio settings. Cell clearance and color are separate shader functions in one
  pass, not a general layer compositor. Rendering follows ~30 Hz telemetry updates,
  with fixed 10 ms attack and 100 ms decay smoothing of displayed dB heights.
  Fixed shape smoothing of 0.5 applies a Gaussian blur to dB
  heights before temporal smoothing, strongest at bass frequencies and tapering
  toward treble. It reuses scratch storage and runs on input updates, not per pixel;
  dots stay crisp. These smoothing settings are not exposed in the lab.
  Startup and Reset default to gradient grid halftone with
  3 pt circles, 4 pt spacing, and 0.5 level influence.
  Visualization settings, pure envelope dynamics, SwiftUI lifecycle,
  and Metal resource ownership are separate components. The shader renders
  halftone coverage and color in one pass, without glow or chromatic effects.
  Spectrum target storage is reused, envelope coefficients are computed once per
  frame, and GPU uniforms use a fixed-size value rather than a temporary array.
  Halftone clearance scans only nearby spectrum segments within the dot radius.
  GPU differential tests compare bounded clearance against a test-only full scan
  on steep and cropped spectra.
  Telemetry observation is isolated from profile controls. Closing the popup
  stops telemetry demand, not processing.
  Run it with `mise run run-native`.
- Native-only Now Playing for the Music and Spotify desktop apps: a compact
  title/artist/artwork footer with 12 pt edge and divider padding, 32 pt artwork with 8 pt corners,
  and 13 pt title/artist typography matching the profile picker, retained with a Paused label and
  hidden when neither player has a track. Swift owns this ephemeral state outside
  the Rust engine. Distributed playback and workspace notifications trigger
  permission-gated, read-only Apple Events addressed to running process IDs;
  opening the popup and waking also reconcile state. Playing wins over paused,
  a newly playing source takes selection, and existing selection wins other ties.
  Initial ties prefer Music. Late reads and artwork cannot restore obsolete state.
  Artwork loads only while the popup is visible, with a placeholder on failure;
  Music supplies bytes and Spotify artwork downloads use an ephemeral URL session.
  The gear menu's Now Playing submenu offers explicit per-player authorization
  and links denied access to Automation settings. Use `mise run run-native-bundle`
  for consent attributed to Tunic; command-line builds cannot initiate consent.
  No transport controls, browser playback, private MediaRemote APIs, accounts,
  or listening-history persistence. Live operation was confirmed by the user;
  exhaustive coverage of both players' streaming metadata/artwork remains unverified.
- Reproducible macOS arm64 XCFramework/Swift package generation under `target/`
  through `mise run build-ffi`, with matching pinned BoltFFI CLI/library versions.
- Automated formatting, Clippy, Rust tests, binding generation, SwiftLint, and
  Swift integration tests through `mise run check`. `mise run lint-native` runs
  strict SwiftLint checks on the native sources, tests, and package manifest.
- Live software-output validation on External Headphones at 48 kHz passed on
  2026-10-20: bypass and flat output matched tap input exactly; −6 dB preamp
  residual was −144.1 dB relative to expected output. Temporary, allocation-free
  callback instrumentation inspected final Core Audio output buffers during a
  quiet stereo probe, then was removed. This does not measure the DAC/jack,
  downstream resampling, or scheduling dropouts.

## Not In Yet

- End-to-end loopback fidelity measurements, same-device sample-rate-change
  observation/recovery, callback deadline/dropout telemetry, and an explicit
  clipping/headroom policy. The new sample-rate guard applies at route startup.
- Product-complete UI, explicit permission UX, recovery beyond retrying the
  current default output, and full equalizer controls; GPUI builds and
  platform integrations for Linux and Windows.
- Production-ready native menu-bar UI. A native equalizer-curve editor is out of scope for v1.
  Non-Swift bindings and non-macOS audio
  adapters are not yet integrated or verified.
- Bulk preset ingestion, AutoEq data, and catalog updates independent of app releases.
- Filter types other than peaking, low-shelf, and high-shelf EQ.
- Profile import/export.
- Profile rename, delete, and copy workflows; independent concurrent editors.
- Distribution signing/notarization, release automation, or end-user installation.
  The local native bundle defaults to ad-hoc signing; stable permission identity
  across rebuilds requires a signing identity (`CODE_SIGN_IDENTITY`).

## Current Non-Goals

- Preserving API or data-format compatibility before the first release.
- Putting platform hardware or UI concerns into `tunic-core`.
- Putting a durable storage implementation into `tunic-core`.

## Verification

Run the Rust and Swift checks with:

```console
mise run check
```

Build and launch the GPUI application on macOS with:

```console
mise run run-app
```
