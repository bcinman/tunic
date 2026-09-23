# `tunic-cli`

Tunic's headless reference client.

**Responsibility:** Exercise every durable product capability through the same engine API used by the app.

- Runs the engine against the real platform and processes system audio.
- Manages profiles, live graph edits, bypass, and per-device settings.
- Exposes engine snapshots, failures, and telemetry as human-readable or structured output.
- Exercises routing, output-device changes, recovery, persistence, and shutdown without GPUI.

Product policy belongs in the engine. The CLI owns command input and output formatting; the app later presents the same commands and snapshots graphically.

## Command surface

```console
tunic run                            # Process audio and accept live commands
tunic status                         # Print the current engine snapshot
tunic watch                          # Stream snapshots, failures, and telemetry
tunic profile list                   # List profiles
tunic profile create|rename|delete   # Manage profiles
tunic profile select                 # Activate a profile
tunic filter add|set|remove          # Preview live graph changes
tunic configuration save|discard     # Commit or discard the preview
tunic bypass on|off                  # Toggle processing
tunic device list|show               # Inspect output devices and their settings
tunic device set-profile             # Associate a profile with an output device
```

The executable selects the platform automatically. It needs no public Rust API; it is a consumer of the engine and platform crates.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
