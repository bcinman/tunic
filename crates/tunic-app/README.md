# `tunic-app`

**Responsibility:** GPUI application lifecycle and presentation.

The GPUI-CE application starts the production engine and provides a graphical
parametric equalizer editor. Its response graph supports direct frequency/gain
dragging and overlays the live post-EQ spectrum behind the response curve; band
rows provide filter-type, frequency, gain, Q, and removal controls. Drag
previews are coalesced while preserving the final pointer value. Changes
preview live and can be saved or reverted. Run it with:

```console
mise run app
```

- Owns the application-scoped engine handle.
- Creates, closes, and reopens the normal application window.
- Keeps the engine alive while no window is open.
- Renders snapshots, profiles, editor state, failures, and meters.
- Owns transient UI state and UI-only preferences.
- Performs orderly engine shutdown only on explicit Quit.

## Rough public interface

```rust
pub struct AppConfig {
    pub data_directory: PathBuf,
}

pub fn run(config: AppConfig) -> Result<(), AppError>;
```

Everything else can remain internal GPUI entities, views, and actions.

See the [workspace structure](../../STRUCTURE.md) for the complete crate layout and dependency direction.
