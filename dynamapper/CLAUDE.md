# CLAUDE.md

Read **[../ENGINEERING.md](../ENGINEERING.md)** before starting any task. It contains workspace layout, build commands, architecture rules, and engineering discipline.

---

## Claude Code Notes

- Plain `cargo` picks up `.cargo/config.toml` automatically; no special invocation needed.
- `cargo check -p <crate>` for fast feedback; `cargo check --workspace` as final validation when touching cross-crate interfaces.
- After visual changes in `dynamapper/`, test all three rendering presets (Classic, Enhanced, KR-like) before reporting done.
- When touching Bevy plugin or system code, run `just bevy-lint`.
- Do not run `cargo fmt` or `cargo clippy` — see ENGINEERING.md.

## Commits

After each completed, validated coding batch, commit the relevant files. A batch is complete when the intended subtask is implemented, validated, and the diff is coherent and minimal.

Before committing, verify:
- the implementation matches the original intent,
- no exploration or debug clutter remains,
- no unauthorized formatting or linting was performed,
- the change is minimal and reviewable.

Stage only files relevant to the completed batch. Use concise commit messages describing what was done, not how. Do not commit formatting-only changes, linter-only changes, or unvalidated work.
