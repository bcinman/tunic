# Repository guidance
- Lint and check rust code with clippy
- Use the tool versions managed by `mise.toml`; add new project tools and common commands to mise when appropriate.
- Run `mise run check` after cross-component changes
- The terminal being used for development has System Sound Recording permissions on mac.
- Do not preserve backward compatibility when changing existing code; nothing has been released yet.
- Funcitonal core, imperative shell.
- Use branded types when it makes sense to prevent invalid state.
- You can add new dependencies to make your life easier, but ask the user before continuing. serde, itertools, are blessed. Use them when it makes sense.
