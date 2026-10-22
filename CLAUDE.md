# sndbx

- Bump `version` in `Cargo.toml` on every code change to sndbx (`src`, `completions/_sndbx`): patch for fixes, minor for new features or options.
- After changing `src` or `completions/_sndbx`, run `bundle exec rake` only once the user confirms the task is finished, not mid-task. Mid-task, run targeted checks like `cargo test <name>`.
