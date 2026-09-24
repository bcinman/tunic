# Project Status

Tunic is currently a working macOS headless single-band equalizer prototype. It
can capture system audio, apply a live peaking filter, and render it to the
current default output device. It is not yet a graphical application or a
complete multi-band equalizer.

This file is the source of truth for implemented product capabilities. The
crate READMEs and `STRUCTURE.md` also describe intended interfaces and future
architecture.

## In

- A five-crate Rust workspace separating the app, CLI, DSP, engine, and macOS
  platform layers.
- A Core Audio route built from a process tap, aggregate device, and IOProc.
- System-audio capture and playback through the current default macOS output.
- Default-output change observation and route rebuilding.
- Float32 input validation, stereo normalization, and support for output
  devices with additional channels.
- A non-real-time engine thread with immutable snapshots, bypass toggling, and
  orderly shutdown.
- An allocation-free stereo DSP graph with zero reported latency.
- One validated peaking filter with configurable frequency, gain, and Q.
- Live, real-time-safe graph replacement from the engine to the audio callback.
- Configuration revisions published in engine snapshots.
- A headless interactive CLI with:
  - engine and route status;
  - output-device listing and inspection;
  - processing bypass;
  - live peaking-filter set and clear commands;
  - optional post-DSP Float32 stereo WAV capture;
  - graceful quit and interrupt handling.
- Automated formatting, Clippy, and workspace test tasks through `mise`.
- Unit coverage for identity and peaking DSP, configuration validation and
  publication, audio buffer handling, command parsing, and WAV capture.
- An ignored hardware test that applies a live filter, plays a stereo probe
  through the production route, and verifies its post-DSP gain.

## Not In Yet

- Multiple simultaneous filters or filter types other than peaking EQ.
- A versioned or serialized processing-configuration format.
- Profiles, profile import/export, or per-device profile selection.
- Persistent state or SQLite storage.
- Live graph preview, save, and discard workflows.
- Telemetry, meters, or snapshot streaming.
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
- Adding product-state features such as profiles and persistence before the
  single-band processing path is proven and expanded.
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
