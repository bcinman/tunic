# Tunic

An opinionated cross-platform parametric equalizer.

Tunic is built around a portable Rust `Session` with pure domain rules and
GPUI and SwiftUI interfaces. Session coordinates persistence and audio through
interfaces; platform integrations implement device discovery and audio wiring.

A native macOS menu-bar app lives in [`apps/tunic-native`](apps/tunic-native).
It uses `tunic-ffi` for Rust-owned state and audio processing. Run it with
`mise run run-native`; bindings are generated automatically.

For Now Playing, use `mise run run-native-bundle` instead. Open Music or Spotify,
then choose **Settings → Now Playing → Connect** for that player. macOS calls
this Automation access even though Tunic only reads metadata. Denied access can
be restored under **System Settings → Privacy & Security → Automation**.
The development bundle is built at `target/native/Tunic.app`; it defaults to
ad-hoc signing, which can require renewed consent after rebuilding. Set
`CODE_SIGN_IDENTITY` to your signing identity for a consistent signed identity.
This is a local development bundle, not a notarized distribution build.

## Crates

| Crate | Provides |
| --- | --- |
| [`tunic-core`](crates/tunic-core) | Session commands and state, business rules, DSP, telemetry, and persistence/platform contracts. |
| [`tunic-ffi`](crates/tunic-ffi) | BoltFFI engine handle, snapshots, commands, DSP analysis, and Swift package generation. |
| [`tunic-ui`](crates/tunic-ui) | Shared GPUI presentation; sends Session commands and reads Session state. |
| [`tunic-app`](crates/tunic-app) | GPUI application lifecycle and platform composition. |
| [`tunic-macos`](crates/tunic-macos) | Core Audio system-output route and callback ownership. |
| [`tunic-presets`](crates/tunic-presets) | Embedded headphone presets, validated JSON, and indexed brand/model queries. |

Run the GPUI application on macOS with `mise run run-app`.

## Development checks

Install the pinned tools with `mise install`, then run `mise run check` for
Rust formatting, Clippy, tests, and the native Swift checks.

Run `mise run lint-native` for SwiftLint alone. It checks the native package
manifest, sources, and tests in strict mode (warnings fail the check), excluding
generated bindings and build output. Rules live in `.swiftlint.yml`, with
test-only size and complexity exceptions under `apps/tunic-native/Tests/`.
Use `mise exec -- swiftlint lint --fix` to apply supported automatic corrections.

The synthetic profile-menu interaction test is disabled by default: running it
can leave the macOS Dock unable to reveal itself, requiring a Dock restart.
To investigate explicitly, run `TUNIC_MENU_TESTS=1 mise exec -- swift test
--package-path apps/tunic-native --filter profilePickerOpensFromPaddedSurface`.
The cause is unresolved; other native window tests still run normally.

## Audio fidelity checks

`mise run check-audio-fidelity` exercises the public processor with impulses at
44.1, 48, 96, and 192 kHz. It compares measured magnitude and phase with the W3C
Audio EQ Cookbook's analog prototypes using frequency prewarping, independently
of Tunic's coefficient and graph code. The matrix covers all three filter types,
boosts/cuts, two Q values, bass/mid/near-Nyquist centers, and preamp gain. Acceptance
limits are 0.05 dB magnitude error and 1% relative complex error. Separate checks
require exact bypass nulling and identical output across callback block sizes.
These tests also run in `mise run check`; they are not proof of end-to-end hardware fidelity.

`mise run audit-audio-rate` creates an **unmuted**, private macOS tap/aggregate,
checks its input/output virtual stream rates against the output rate used by DSP,
then destroys it. It requires System Audio Recording permission, but does not
play sound, change the default output, or write profiles. Route startup enforces
the same agreement. This does not yet cover rate changes while a route is running.

For a bypass loopback comparison:

1. Use an audio interface's output-loopback input or a verified virtual loopback
   device. Record **after Tunic's output**, not its input tap or a room microphone.
   Keep the route, volume, sample rate, recorder gain, and all other effects fixed.
2. Generate a six-second 24-bit stereo probe (peak below −20 dBFS; turn headphones
   down before playing noise):
   `mise run audio-null generate /tmp/tunic-probe.wav --rate 48000`.
3. Record it with Tunic quit as `baseline.wav`. Then start Tunic, disable
   **Processing**, and record the same file as `bypass.wav`. Play using an external
   process such as `afplay /tmp/tunic-probe.wav`; Tunic excludes its own process.
   Start recording before playback and retain the entire signal plus trailing silence.
   Export both as stereo signed integer PCM WAV, preferably 24-bit, at the original
   sample rate. Do not normalize, trim active audio, or resample the recordings.
4. Run `mise run audio-null compare baseline.wav bypass.wav`.

The comparator aligns fixed integer-frame latency, checks drift at the middle/end,
and reports each channel's active-signal residual, gain, peak error, and clipping.
It does **not** fit away gain changes, channel swaps, dropouts, or clock drift.
Its strict digital acceptance is ≤−80 dB residual relative to signal, peak error
≤0.0001 full scale, no clipping, and no measured integer-frame drift. JSON `null`
for a residual means exact zero. Exit 0 passes, 1 fails, and 2 means invalid input.
Use the generated broadband probe: correlation alignment is ambiguous for a pure tone.
Analog loopbacks, fractional delays, or asynchronous resampling can fail these
digital limits without establishing an audible defect; investigate the metrics.
This is not a THD/noise-floor measurement, and a synthetic self-null is not a live
loopback result. `mise run test-audio-null` verifies the analyzer against deliberate
gain changes, channel swaps, and a dropped frame.
