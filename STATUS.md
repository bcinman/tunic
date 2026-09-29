# Project Status

Tunic is rebuilding around a portable Rust core embedded in native platform
applications. The previous GPUI application, CLI, engine, DSP, and Core Audio
prototype crates have been removed rather than carried into the new design.

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
- Typed preset browsing and lookup through BoltFFI.
- Sample-rate-specific stereo DSP with preamp gain, ordered peaking and shelf
  filters, post-quantization stability validation, and allocation-free audio
  processing.
- A cloneable controller that publishes latest-value chain replacements and
  atomic bypass changes to the processor. Chain replacements use a
  five-millisecond crossfade.
- Demand-driven, allocation-free post-output telemetry with stereo peak/RMS
  levels and a 256-point spectrum exposed through nonblocking latest-value
  subscriptions.
- A `tunic-ffi` integration spike using BoltFFI for native processor creation
  and control, plus a handwritten zero-copy real-time audio entry point.
- macOS arm64 XCFramework and Swift package generation through
  `mise run ffi-apple`.
- Automated formatting, Clippy, and workspace tests through `mise run check`.

## Not In Yet

- Native platform applications and their UI, device discovery, permissions,
  audio wiring, and lifecycle.
- Durable storage. A future crate such as `tunic-sqlite` can implement the
  core's `Store` contract.
- Backend and telemetry FFI bindings.
- Bulk preset ingestion, AutoEq data, and catalog updates independent of app releases.
- Apple targets beyond macOS arm64, or bindings for other platforms.
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

Generate and compile the Apple package with:

```console
mise run ffi-apple
```
