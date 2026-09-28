# `tunic-core`

Tunic's portable, side-effect-free product core.

It defines profiles, commands, processing chains, backend state, real-time
processing, telemetry, and the `Store` contract. Native applications own UI,
device discovery, audio wiring, permissions, and lifecycle. Separate storage
crates, such as a future `tunic-sqlite`, implement durable persistence.

This crate contains no platform APIs, hardware integration, UI framework, or
storage implementation.
