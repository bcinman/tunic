# `tunic-core`

Tunic's portable, platform-independent product core. Side-effect
implementations live outside this crate.

It defines profiles, commands, processing chains, backend state, real-time
processing, telemetry, and the `Store` contract. Every chain has one equalizer
and can gain optional spatial processing later. Native applications own UI,
device discovery, audio wiring, permissions, and lifecycle. Separate storage
crates, such as a future `tunic-sqlite`, implement durable persistence.

Profiles are global and reusable. The core remembers which profile each device
has selected without making that device the profile's owner.

This crate contains no platform APIs, hardware integration, UI framework, or
storage implementation.
