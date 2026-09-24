# Project Status

Tunic is currently a working macOS headless multi-band equalizer prototype. It
can capture system audio, apply live peaking and shelving filters, and render
it to the current default output device. It is not yet a graphical application
or a complete parametric equalizer.

This file is the source of truth for implemented product capabilities. The
crate READMEs and `STRUCTURE.md` also describe intended interfaces and future
architecture.

## In

- A five-crate Rust workspace separating the app, CLI, DSP, engine, and macOS
  platform layers.
- A Core Audio route built from a process tap, aggregate device, and IOProc.
- System-audio capture and playback through the current default macOS output.
- Default-output and nominal sample-rate change observation with route
  rebuilding and graph preparation for the new rate.
- Float32 input validation, stereo normalization, and support for output
  devices with additional channels.
- A non-real-time engine thread with immutable snapshots, bypass toggling, and
  orderly shutdown.
- An allocation-free stereo DSP graph with zero reported latency.
- An ordered cascade of validated peaking, low-shelf, and high-shelf filters
  with configurable frequency, gain, and Q, including post-quantization
  stability checks.
- Live, real-time-safe graph replacement from the engine to the audio callback.
- Versioned JSON equalizer documents with strict validation on load.
- A SQLite profile store with a seeded fallback profile, per-device assignment
  schema, optimistic saved revisions, and automatic schema versioning.
- Profile creation from the current equalizer, listing, inspection, renaming,
  deletion, default selection, and persistent per-device assignment.
- In-memory previews with live save and discard workflows. Saved equalizers are
  restored across restarts, while previews remain ephemeral.
- Saved equalizer and edit revisions published in engine snapshots.
- Ordered engine snapshot subscriptions with immediate initial-state delivery.
- Real-time-safe post-DSP stereo peak/RMS and 28-band RTA measurement, with
  attack smoothing and controlled dB decay, published at 30 Hz through a
  lock-free latest-value engine telemetry reader.
- A headless interactive CLI with:
  - engine and route status;
  - continuous engine snapshot watching;
  - output-device listing and inspection;
  - profile CRUD, selection, and per-device assignment commands;
  - processing bypass;
  - live stereo peak/RMS meters and a 31.5 Hz–16 kHz spectrum in dBFS;
  - live typed-filter add, set, remove, and reset commands;
  - persistent equalizer save and discard commands;
  - optional post-DSP Float32 stereo WAV capture;
  - graceful quit and interrupt handling.
- Filter history is cleared when bypass begins so stale ringing is not replayed
  when processing resumes.
- Automated formatting, Clippy, and workspace test tasks through `mise`.
- A Criterion DSP processing benchmark covering 0, 5, 10, and 20 filters at
  common buffer lengths and sample rates.
- Unit coverage for identity, peaking, and shelving DSP; equalizer validation
  and publication; audio buffer handling; telemetry calculation and
  publication; command parsing; and WAV capture.
- An ignored hardware test that applies live low- and high-shelf filters, plays
  a stereo probe through the production route, and verifies telemetry plus the
  combined post-DSP low/high gain contrast.

## Not In Yet

- Filter types other than peaking, low-shelf, and high-shelf EQ.
- Profile import/export.
- Automatic recovery or retry after route rebuild failures.
- Route handoff, crossfading, or sample-rate conversion.
- A GPUI application; `tunic-app` is currently a placeholder crate.
- Linux or Windows audio backends.
- Packaging, signing, release automation, or end-user installation.
- Most of the command surface documented in `crates/tunic-cli/README.md`; only
  the interactive commands listed above are implemented.

## Current Non-Goals

These are scope boundaries for the current audio-plumbing milestone, not
permanent exclusions from the product roadmap.

- Preserving API or data-format compatibility before the first release.
- Building the graphical application before the engine and DSP contracts are
  proven through the headless client.
- Supporting platforms other than macOS before the Core Audio path is stable.
- Automatic route-rebuild recovery or retry until a rebuild failure is
  reproduced through Core Audio. Fault injection confirms that the engine does
  not retry, but normal and rapid output changes have not produced a platform
  failure.
- Optimizing or polishing distribution before the end-to-end product behavior
  is complete.

## Verification

Run the complete automated suite with:

```console
mise run check
```

Run the opt-in production audio-path test on a Mac with System Sound Recording
permission using:

```console
mise run verify-audio
```

The hardware test uses the current output device and is intentionally excluded
from the normal test suite.
