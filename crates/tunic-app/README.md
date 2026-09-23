# `tunic-app`

**Responsibility:** GPUI application lifecycle and presentation.

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
