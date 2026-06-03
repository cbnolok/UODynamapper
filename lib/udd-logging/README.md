# udd-logging

`udd-logging` is a thin logging setup helper used by the CLI and GUI tools in
the workspace. It initializes the `log` facade backend with a colored, formatted
console output.

## Responsibilities

- Initializes a `paris`-backed log subscriber with `env_filter` for log-level
  filtering via the `RUST_LOG` environment variable.
- Provides a single call-site setup used consistently across all tools.

## Notes

- `dynamapper` uses Bevy's built-in logging plugin instead of this crate. See
  [/memories/repo/bevy-logging.md] for details.
- This crate is not used by `uocf`, which emits log events without coupling to
  any backend.
