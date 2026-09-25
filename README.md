# Tunic

An opinionated cross-platform parametric equalizer.

## Features

- System-wide audio capture, equalization, and playback on macOS.
- Live peaking, low-shelf, and high-shelf filters with configurable frequency, gain, and Q.
- Persistent profiles with defaults and per-output-device assignments.
- Live equalizer previews with save and discard workflows.
- A graphical response editor for adding, adjusting, removing, saving, and
  reverting filters.
- Processing bypass plus stereo peak, RMS, and real-time spectrum metering.
- An interactive CLI for filter editing, profile management, device inspection, and WAV capture.
- Automatic adaptation to default-output and sample-rate changes.

## Coming Soon

- Additional equalizer filter types.
- Profile import and export.
- Smoother route handoff, recovery, and sample-rate conversion.
- Linux and Windows audio backends.
- Signed, installable releases.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-app`](crates/tunic-app) | GPUI application lifecycle and graphical equalizer editor. |
| [`tunic-cli`](crates/tunic-cli) | Headless diagnostics and interactive control of the equalizer. |
| [`tunic-dsp`](crates/tunic-dsp) | Portable equalizer definitions, validation, and real-time audio processing. |
| [`tunic-engine`](crates/tunic-engine) | Product state, profile persistence, and non-real-time audio coordination. |
| [`tunic-macos`](crates/tunic-macos) | Core Audio integration for system-audio capture, processing, and playback. |
