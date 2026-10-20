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

## Releases and updates

Tunic ships directly for Apple Silicon on macOS 26+. Sparkle 2 provides the
gear menu's update controls and standard update dialogs. Development builds
cannot check for updates. Production uses the public GitHub latest-release
`appcast.xml` asset; each feed points to its version-specific `Tunic.zip` asset.
No separate update server is needed. Never replace a published ZIP or reuse a version.

GitHub Actions **Check** runs `mise run check` and builds the development bundle
on pull requests and main. Both workflows use macOS 26 ARM runners, Xcode 26.6,
and the tools pinned in mise. Hardware/audio-permission tests remain opt-in.

### One-time release setup

Create a GitHub environment named **release**, restrict it to the main branch,
and require your approval. Add these environment values through GitHub Settings
(never put credentials in source control or chat):

| Kind | Name | Value |
| --- | --- | --- |
| Secret | `CERTIFICATE_P12` | Base64-encoded Developer ID Application certificate **with its private key**, exported from Keychain Access |
| Secret | `CERTIFICATE_PASSWORD` | Password used to protect that export |
| Secret | `APPLE_ID` | Apple Developer account email |
| Secret | `APPLE_APP_PASSWORD` | Apple app-specific password for notarization |
| Secret | `SPARKLE_PRIVATE_KEY` | Contents of the exported Sparkle private-key file |
| Variable | `CODE_SIGN_IDENTITY` | Full `Developer ID Application: … (TEAMID)` identity |
| Variable | `APPLE_TEAM_ID` | Apple Developer team ID |
| Variable | `SPARKLE_PUBLIC_KEY` | Sparkle's base64 public key |

After `mise run build-native-bundle`, generate Sparkle keys once:

```sh
tools=apps/tunic-native/.build/artifacts/sparkle/Sparkle/bin
"$tools/generate_keys" --account tunic
"$tools/generate_keys" --account tunic -x "$HOME/tunic-sparkle-private.key"
```

The first command prints the public key and stores the private key in your login
Keychain. Store an encrypted backup of the export, copy its contents into the
GitHub secret, then remove the plaintext export. Keep the same public key across
releases; do not casually regenerate it or change the app's bundle identifier.

### Publish a version

1. Push a new `vX.Y.Z` tag pointing at a tested commit on main.
2. Run **Release** from main in GitHub Actions, supplying that tag, and approve
   the protected environment. It rejects existing/older releases, reruns checks,
   builds release-mode Rust and Swift, signs the app and Sparkle helpers,
   notarizes and staples the app, then signs the final ZIP and generates the feed.
   `CFBundleVersion` is the release workflow run number; do not reset that counter.
3. Download the ZIP from the resulting **draft release**, install it in Applications,
   and smoke-test audio, permissions, login startup, and the update menu. Edit the
   generated GitHub release notes as needed.
4. Publish the draft as the **latest release**. Publish in version/build order;
   draft assets are not accessible to Sparkle. The same action exposes both assets.

Before the first public launch, test an actual older → newer signed/notarized
update using a temporary test feed URL. Draft creation alone does not verify
installation or permission continuity. If a published version is bad, release
a higher version containing the fix/revert; do not downgrade the feed. Keep the
macOS minimum and architecture unchanged until the feed also retains updates
for users on older supported systems.

Signing and notarization use a temporary CI keychain, removed on exit.
`mise run release-native` owns packaging; Actions only manages credentials and
uploads the draft. CI needs no signing secrets, and merging does not publish.

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
