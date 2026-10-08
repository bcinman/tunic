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
- A narrow command API for flat/preset selection, filter/control edits,
  save/reset, clear selection, and audio refresh. Session owns the only draft;
  callers cannot submit replacement profiles, and profiles need no revision counters.
- Atomic whole-state persistence through a `Persistence` contract and rejection
  of corrupt restored state. Failed saves retain drafts. DSP publication errors
  remain visible across no-op commands until resolved; refresh cannot bypass a
  failed output-watcher installation.
- An offline `tunic-presets` catalog with build-validated JSON, static brand/model
  indexes, and on-demand payload decoding. Initial oratory1990 presets cover
  Sennheiser HD650 and Sony MDR-7506.
- Profiles own an editable base chain plus named controls targeting stable filter
  identities. Each control holds a relative gain adjustment; effective DSP chains
  add those adjustments without changing base values.
- Sample-rate-specific stereo DSP with preamp gain, ordered peaking and shelf
  filters, post-quantization stability validation, and allocation-free audio
  processing.
- A cloneable controller that publishes latest-value chain replacements and
  atomic bypass changes to the processor. Chain replacements use a
  five-millisecond crossfade.
- Demand-driven, allocation-free post-output telemetry with a lock-free history
  of sequenced raw stereo peak/RMS measurements and 256-point spectra.
- A minimal GPUI-CE application split into shared `tunic-ui` presentation and
  `tunic-app` lifecycle/platform composition. It browses both bundled presets,
  creates and selects real core profiles, and clears selection.
- A `tunic-macos` route for the current default output using a Core Audio process
  tap, private aggregate device, callback-owned core `Processor`, native-buffer
  normalization, and ordered teardown. Profile clicks publish live chain changes
  through the core `Controller`.
- A Session-owned `Platform` contract with a shared `tunic-macos::MacosPlatform`
  that observes Core Audio default-output notifications and rebuilds the complete
  route with the current draft. Each host schedules refresh commands and retries.
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
  status, preset selection, named gain controls, save/reset, and live RMS readings.
  A transparent Metal `MTKView` visualizer replaces the Canvas spectrum and EQ
  response curve. An icon-only tuning button over the visualizer's top-right opens
  a scrollable lab popover; its settings remain live and survive dismissal.
  The button is hidden until hovering the spectrum, fades over 200 ms, and remains
  visible while the popover is open; Reduce Motion disables the fade.
  The FPS readout sits at the top-left. The lab switches between white points
  and a signed distance field of the spectrum curve, with line, glow, contour-band,
  and distance-debug mappings; width, spread, and hue update live. Halftone shading
  fills beneath the curve with white or horizontal-gradient grid/hexagonal dots, with
  four editable color stops at 0%, 33%, 67%, and 100% of the frequency axis.
  Gradient defaults use bright cyan, mint, lavender, and pink, interpolated between stops.
  Halftone's gradient replaces its hue control. Dot diameter and spacing are adjustable,
  with a response control that scales diameter by displayed spectrum
  height at each dot's frequency. Diameters above the spacing overlap into connected
  areas; an Invert option fills the graph around transparent dot-shaped holes.
  Boundary dots shrink to fit their full circles
  inside the spectrum and viewport, including their antialiased edges, rather than
  being sliced by the fill mask. An independent signed vertical response tapers
  dots toward the top or bottom of each frequency's filled area. Optional bass-hit
  chromatic aberration splits color channels across the entire halftone image,
  with adjustable strength, 20–180 Hz trigger threshold, and time-based decay.
  Extra redraws stop when the pulse settles or the view is hidden. A static demo
  input supports tuning without audio. Visual settings are ephemeral and separate
  from audio settings. Distance and color are separate shader functions in one
  pass, not a general layer compositor. Rendering follows ~30 Hz telemetry updates,
  with adjustable spectrum attack (0–500 ms) and decay (0–1500 ms) smoothing of
  displayed dB heights across all styles, independent of bass-hit detection.
  A Shape smoothing control (0–1, default 0.5) applies a Gaussian blur to dB
  heights before temporal smoothing, strongest at bass frequencies and tapering
  toward treble. It reuses scratch storage and runs on input updates, not per pixel;
  dots stay crisp and raw bass-hit detection is unchanged. Zero restores raw geometry.
  Startup and Reset default to 0 ms attack, 100 ms decay, white grid halftone with
  3 pt dots, 4 pt spacing, 0.5 amplitude response, and chromatic aberration
  enabled. Continuous redraws run only during spectrum settling or
  chromatic pulses; numeric RMS readings are
  limited to 5 Hz.
  A visualizer FPS overlay reports GPU-completed frames over elapsed wall time,
  updated once per second without driving additional Metal frames. Idle reads zero;
  hiding the window stops the counter timer and resets the reading.
  Visualization settings, pure envelope/pulse dynamics, SwiftUI lifecycle,
  and Metal resource ownership are separate components. Shader composition applies
  chromatic offsets, then halftone coverage/color in one pass.
  Optional Deep glow follows chromatic aberration for all visualization styles,
  combining five octave-spaced Gaussian scales to approximate inverse-square
  falloff (not the proprietary plugin's exact kernel). It uses linear-light
  blending, half-float downsampled intermediates, and exposure/radius controls.
  The crisp source and transparent background are preserved; reusable render
  targets are resized on demand and released when bypassed. Disabled by default.
  Spectrum target storage is reused, envelope coefficients are computed once per
  frame, and GPU uniforms use a fixed-size value rather than a temporary array.
  Halftone clearance scans only nearby spectrum segments within the dot radius;
  full-distance SDF styles retain the complete scan. GPU differential tests compare
  bounded clearance against the original full scan on steep and cropped spectra.
  Telemetry observation is isolated from profile controls. Closing the popup
  stops telemetry demand, not processing.
  Run it with `mise run run-native`.
- Reproducible macOS arm64 XCFramework/Swift package generation under `target/`
  through `mise run build-ffi`, with matching pinned BoltFFI CLI/library versions.
- Automated formatting, Clippy, Rust tests, binding generation, SwiftLint, and
  Swift integration tests through `mise run check`. `mise run lint-native` runs
  strict SwiftLint checks on the native sources, tests, and package manifest.

## Not In Yet

- Product-complete UI, explicit permission UX, recovery beyond retrying the
  current default output, bypass, and full equalizer controls; GPUI builds and
  platform integrations for Linux and Windows.
- Production-ready native menu-bar UI and editable graph gestures.
  Non-Swift bindings and non-macOS audio
  adapters are not yet integrated or verified.
- Durable storage. A future crate such as `tunic-sqlite` can implement the
  core's `Persistence` contract; the app currently uses `MemoryPersistence`.
- Bulk preset ingestion, AutoEq data, and catalog updates independent of app releases.
- Filter types other than peaking, low-shelf, and high-shelf EQ.
- Profile import/export.
- Profile rename, delete, and copy workflows; independent concurrent editors.
- Application packaging, signing, release automation, or end-user installation.

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
