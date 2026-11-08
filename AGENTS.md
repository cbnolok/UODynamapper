# AGENTS.md

This is the shared instruction file for all harnesses. Root `CLAUDE.md`, `CODEX.md`, and `GEMINI.md`, plus `dynamapper/CLAUDE.md`, are relative symlinks to this file. Edit `AGENTS.md` to update their guidance.

Read **[ENGINEERING.md](ENGINEERING.md)** before starting any task. It contains workspace layout, build commands, architecture rules, and engineering discipline shared across all agents.

---

## Collaboration

- Break complex tasks into focused subtasks. When sub-agents are available, use them for code generation, file reads, test runs, and searches; coordinate their work and review their outputs before accepting them.
- Prefer delegating implementation when a suitable sub-agent is available. If an output is wrong, correct the instructions and retry; fix minor formatting issues directly. Work directly when delegation is unavailable or the harness disallows it.
- When a task requires design decisions, present an implementation-ready proposal and obtain confirmation before writing code, unless the user has already authorized that approach. Resolve the design before asking so implementation can proceed without intermediate stops.
- Use named constants (`const` or `static`) for bounds, limits, version numbers, and other fixed values; do not introduce magic numbers.

## Validation

- Plain `cargo` picks up `.cargo/config.toml` automatically; no special invocation is needed.
- Use `cargo check -p <crate>` for fast feedback. Use `cargo check --workspace` as final validation when touching cross-crate interfaces.
- Do not run `cargo fmt`, `rustfmt`, or `cargo clippy` automatically; follow the maintainer-only policy in `ENGINEERING.md`.
- After visual changes in `dynamapper/`, test all four render styles: **Classic**, **Enhanced**, **KR-like**, and **UOD-Painterly**. There are three shading modes but four preset groups; testing only three styles is incomplete.
- When touching Bevy plugin or system code, run `just bevy-lint`.

## Development Skills

Project skills live in [`skills/`](skills/). Read the relevant `SKILL.md` before using its workflow. Harness discovery directories link to this shared folder so skills and their supporting files have one maintained copy.

- [`kde-x11-preview`](skills/kde-x11-preview/SKILL.md): launch an owned X11/XWayland window on a named KDE virtual desktop without changing the user's active desktop; verify placement and clean up only that run's child and KWin script.
- [`renderer-reference-validation`](skills/renderer-reference-validation/SKILL.md): isolate settings, drive the renderer, capture complete native screenshots on inactive desktops, and compare actual pixels against original-client references. Prefer the existing renderer control socket over temporary capture instrumentation.

## Context Preservation

When compacting or handing off a conversation, preserve API changes and their rationale, exact error messages and their fixes, validation results, and remaining work. Summarize exploration attempts briefly.

## Completed Phases and Review

### Phase-based commits

Work in completed, validated phases. A phase is complete when the intended subtask is implemented, validated, and the diff is coherent and minimal. Commit only at phase boundaries, staging only files relevant to the completed phase.

After each completed, validated implementation phase, commit the relevant files unless the user requested otherwise.

Before committing, verify:
- the implementation matches the original intent,
- no exploration or debug clutter remains,
- no unauthorized formatting or linting was performed,
- the change is minimal and reviewable.

Use concise commit messages describing the completed phase, not the mechanism. Do not commit formatting-only changes, linter-only changes, or unvalidated work.

### Post-change review

After implementing any change, explicitly check:

1. Does it implement the intended behavior?
2. Is the approach consistent with the repository's established patterns?
3. Did I introduce unnecessary complexity?
4. Did I leave behind any exploration or audit clutter?
5. Was it validated with a narrowly scoped check?

If validation fails, diagnose carefully once before retrying. Do not enter a blind fix-and-rerun loop.

### Git

Inspect the working tree before editing when appropriate. Do not revert, reformat, or absorb pre-existing uncommitted user changes. Never use `git reset --hard`, `git clean`, force pushes, or history rewrites unless explicitly instructed.
