# Project Status

Tunic is rebuilding around a portable Rust core and a shared GPUI desktop
interface. The previous engine, DSP, CLI, and Core Audio prototype crates were
removed rather than carried into the new design; the new GPUI spike calls the
simplified core directly.

This file is the source of truth for implemented product capabilities.

## In

- A side-effect-free `tunic-core` containing validated profile, chain, and
  device-selection domain types.
- Complete backend command handling with revision-checked profile updates,
  atomic whole-state persistence through a `Store` contract, and rejection of
  corrupt restored state.
- An offline `tunic-presets` catalog with build-validated JSON, static brand/model
  indexes, and on-demand payload decoding. Initial oratory1990 presets cover
  Sennheiser HD650 and Sony MDR-7506.
- Stable filter identities and source-provided adjustment mappings. Profiles copy
  preset attribution, revision, and adjustments; edits to other filter parameters
  remove obsolete adjustments without changing the source preset.
- Sample-rate-specific stereo DSP with preamp gain, ordered peaking and shelf
  filters, post-quantization stability validation, and allocation-free audio
  processing.
- A cloneable controller that publishes latest-value chain replacements and
  atomic bypass changes to the processor. Chain replacements use a
  five-millisecond crossfade.
- Demand-driven, allocation-free post-output telemetry with stereo peak/RMS
  levels and a 256-point spectrum exposed through nonblocking latest-value
  subscriptions.
- A minimal GPUI-CE application split into shared `tunic-ui` presentation and
  `tunic-app` lifecycle/platform composition. It browses both bundled presets,
  creates and selects real core profiles, and clears selection.
- A `tunic-macos` route for the current default output using a Core Audio process
  tap, private aggregate device, callback-owned core `Processor`, native-buffer
  normalization, and ordered teardown. Profile clicks publish live chain changes
  through the core `Controller`.
- An app-owned `Platform` contract with an Apple implementation that observes
  Core Audio default-output notifications and immediately rebuilds the complete route.
- macOS GPUI development through `mise run build-app` and `mise run run-app`.
- Automated formatting, Clippy, and workspace tests through `mise run check`.

## Not In Yet

- Product-complete UI, explicit permission UX, recovery beyond retrying the
  current default output, bypass, and telemetry presentation; GPUI builds and platform
  integrations for Linux and Windows.
- Durable storage. A future crate such as `tunic-sqlite` can implement the
  core's `Store` contract.
- Bulk preset ingestion, AutoEq data, and catalog updates independent of app releases.
- Filter types other than peaking, low-shelf, and high-shelf EQ.
- Profile import/export.
- Packaging, signing, release automation, or end-user installation.

## Current Non-Goals

- Preserving API or data-format compatibility before the first release.
- Putting platform hardware or UI concerns into `tunic-core`.
- Putting a durable storage implementation into `tunic-core`.

## Verification

Run the complete Rust suite with:

```console
mise run check
```

Build and launch the GPUI application on macOS with:

```console
mise run run-app
```
