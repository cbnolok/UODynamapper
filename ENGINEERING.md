# ENGINEERING.md

Shared reference for all AI agents working in this repository. **Read this before starting any task.**

Before diving into source code for any non-trivial task, also read relevant files in `docs/agent_notes/` — they contain durable discoveries from prior sessions. Reading existing documentation beats re-deriving knowledge from scratch. Writing discoveries back beats holding them in session context.

---

## Project

UODynamapper is a 2D client/dynamic map renderer for Ultima Online, emulating Classic 2D, Enhanced Classic, and Kingdom Reborn (KR-like) visuals.

**Tech Stack**: Rust, Bevy Engine v0.18.1, WGSL shaders (naga-oil), wgpu.

---

## Workspace Layout

- `dynamapper/` — Main Bevy application (rendering, UI, controls)
- `lib/uocf/` — Ultima Online client file readers
- `lib/udd-assets/`, `lib/udd-container/` — UODynamapper asset packaging and runtime readers
- `lib/udd-image-codecs/` — KTX2 / image codec handling
- `tools/udd-conv-cli/` — CLI for building/inspecting converted packages (binary: `udd-cli tool|pack`)
- `tools/uoppackage-cli/` — Content-agnostic UOP package operations (binary: `uoppackage-cli`)
- `tools/uocf-cli/` — UO client file content operations: anim-patch, sound, multimap, MUL↔UOP (binary: `uocf-cli`)
- `tools/uocf-devtools-cli/` — Research/diagnostic tools (feature flag: `dev-tool-tests`)
- `tools/uddp-inspector-gui/`, `tools/udd-conv-gui/`, `tools/uop-inspector-gui/` — GUI tools

---

## Build Commands

```sh
cargo build -p dynamapper                        # debug build
cargo build --workspace                          # full workspace debug
cargo check -p <crate>                           # fast type check
cargo test --workspace                           # all tests
just build-local-workspace-release               # release (stable)
just build-local-workspace-release-nightly       # release (nightly, best optimization)
just bevy-lint                                   # Bevy-specific lints
```

**Cargo target directory**: keep the one configured in `.cargo/config.toml`. Never pass `CARGO_TARGET_DIR` or `--target-dir`.

**Do not run `cargo fmt`, `rustfmt`, or `cargo clippy` automatically.** These are privileged maintainer actions. Maintain local style manually in the lines you touch. If a validation command would implicitly reformat code, do not run it.

---

## Engineering Discipline

### Minimal changes
Modify only what is necessary. Preserve existing style, architecture, naming conventions, and formatting in lines you don't touch. Do not opportunistically refactor or clean up surrounding code. Do not remove pre-existing comments.

### High-confidence execution
Form one well-reasoned approach before editing. Inspect relevant source, find the established pattern, implement the minimal consistent change. Do not try multiple approaches and keep whichever happens to work. Trial-and-error is acceptable only when behavior cannot be determined statically and the experiment is narrowly scoped.

### No clutter
Do not leave behind debug prints, scratch functions, commented-out experiments, dead branches, or temporary code. Production code contains only code that serves the final validated behavior.

### Validation
Use the narrowest meaningful check: `cargo check -p <crate>`, a targeted test, or a type check. After any change, verify: does it implement the intended behavior? Is it minimal and consistent with the repository? Did it introduce unnecessary complexity?

### Git
Do not revert, reformat, or absorb pre-existing uncommitted user changes unless directly required. Stage only files relevant to the completed work. Never use `git reset --hard`, `git clean`, force pushes, or history rewrites unless explicitly instructed.

---

## Agent Knowledge Base

`docs/agent_notes/` holds persistent notes written by AI agents across sessions. It is the right place to record durable discoveries that are not yet in `docs/dev_wiki/`.

**Read** relevant files there at the start of any non-trivial task — before reading source.

**Write** to it before ending a session if you discovered something that would have saved time to know upfront:
- Non-obvious module responsibilities or cross-crate coupling
- Gotchas and pitfalls encountered during the session
- Established patterns not evident from a single file
- Debugging findings not yet captured in this file

Only record repo-level facts. Do not write session context, task-specific notes, or anything that will be stale within weeks. The test: *"would this have saved me an hour at the start of the session?"*

Files in `docs/agent_notes/` are lower-confidence than `docs/dev_wiki/`. A human reviewer may promote, correct, or remove entries at any time.

---

## Further Reference

- `docs/agents/dynamapper_architecture.md` — dynamapper-specific architecture rules, source structure, rendering presets, debugging
- `docs/dev_wiki/` — authoritative deeper technical notes
- `docs/agent_notes/` — agent-discovered notes, lower confidence, periodically reviewed
- `docs/TODO.md` — roadmap
