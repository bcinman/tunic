# `tunic-cli`

Tunic's headless reference client.

**Responsibility:** Exercise every durable product capability through the same engine API used by the app.

- Runs the engine against the real platform and processes system audio.
- Manages profiles, live equalizer edits, bypass, and per-device settings.
- Exposes engine snapshots, failures, and telemetry as human-readable or structured output.
- Exercises routing, output-device changes, recovery, persistence, and shutdown without GPUI.

Product policy belongs in the engine. The CLI owns command input and output formatting; the app later presents the same commands and snapshots graphically.

## Command surface

The currently implemented surface is:

```console
tunic start [--capture <PATH>] [--data-directory <DIRECTORY>]
                                      # Process audio; optionally override storage location

# Inside the interactive session:
status
device list
device show [DEVICE]
bypass
filter add <peaking|low-shelf|high-shelf> --frequency <HZ> --gain <DB> --q <Q>
filter set <BAND> <peaking|low-shelf|high-shelf> --frequency <HZ> --gain <DB> --q <Q>
filter remove <BAND>
filter reset
save                                  # Persist the preview
discard                               # Restore the last persisted equalizer
help
quit
```

The planned product surface also includes:

```console
tunic status                         # Print the current engine snapshot
tunic watch                          # Stream snapshots, failures, and telemetry

tunic profile list                   # List profiles
tunic profile info                   # Show profile details
tunic profile import                 # Import a profile from a file in APO format
tunic profile export                 # Export a profile to a file in APO format
tunic profile create|rename|delete   # Manage profiles
tunic profile select                 # Activate a profile

tunic bypass                         # Toggle processing
tunic device list|show               # Inspect output devices and their settings
tunic device set-profile             # Associate a profile with an output device

tunic telemetry                      # Stream telemetry data
```

The executable selects the platform automatically. By default it stores state
in `~/Library/Application Support/Tunic/tunic.sqlite3`. It needs no public Rust
API; it is a consumer of the engine and platform crates.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
