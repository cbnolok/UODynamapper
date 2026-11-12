# ENGINEERING.md

Shared reference for all AI agents working in this repository. **Read this before starting any task.**

Before diving into source code for any non-trivial task, also read relevant files in `docs/agent_notes/` — they contain durable discoveries from prior sessions. Reading existing documentation beats re-deriving knowledge from scratch. Writing discoveries back beats holding them in session context.

---

## Project

UODynamapper is a 2D client/dynamic map renderer for Ultima Online, emulating Classic 2D, Enhanced Classic, and Kingdom Reborn (KR-like) visuals.

**Tech Stack**: Rust, Bevy Engine v0.20.0, WESL/WGSL shaders (Bevy's `wesl` composer), wgpu.

---

## Workspace Layout

- `dynamapper/` — Main Bevy application (rendering, UI, controls)
- `lib/uocf/` — Ultima Online client file readers
- `lib/udd-assets/`, `lib/udd-container/` — UODynamapper asset packaging and runtime readers
- `lib/support/` — shared tooling plumbing: `uot-texture-codecs` (BC1/BC7 and KTX2 texture codecs), `uot-image-postprocess` (upscalers and filters), `uot-kdl` (strict KDL decoding), `uot-logging` (terminal logging for the tools), `uot-gui` (eframe helpers for the GUI tools)
- `tools/udd-conv-cli/` — CLI for building/inspecting converted packages (binary: `udd-cli tool|pack`)
- `tools/uoppackage-cli/` — Content-agnostic UOP package operations (binary: `uoppackage-cli`)
- `tools/uocf-cli/` — UO client file content operations: anim-patch, sound, multimap, MUL↔UOP (binary: `uocf-cli`)
- `tools/webmap-export-cli/` — Static web map export: tile plan, spool consumer, WebP pyramid and Leaflet viewer (binary: `webmap-export-cli`)
- `tools/uocf-devtools-cli/` — Research/diagnostic tools (feature flag: `dev-tool-tests`)
- `tools/udd-devtools-cli/` — Converted-package comparisons, gump scoring, paperdoll migration checks and optional GPU/codec experiments
- `tools/uddp-inspector-gui/`, `tools/udd-conv-gui/`, `tools/uocf-editor-gui/` — GUI tools

Crate name prefixes (2026-11-06): `uocf` is the UO client files (`lib/uocf` and the tools that work on client files),
`udd` is UO Dynamapper data (the converted packages, their conversion and their readers), and `uot` is shared tooling
plumbing with no tie to either, under `lib/support/`. Client-file tools don't depend on `udd*` crates.

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

**Feature unification (`lib/workspace-hack`)**: every member depends on the `workspace-hack` crate that cargo-hakari generates. It
turns on the union of the third-party features, so the workspace build, `-p dynamapper` and each tool share one copy of each
dependency instead of rebuilding it per feature set. Members name it as `workspace-hack.workspace = true`, the empty stub
on crates.io; the root `[patch.crates-io]` swaps in `lib/workspace-hack`, but only inside this workspace, so projects that
use our crates as path or git dependencies don't inherit the unified features. After adding, removing or changing a dependency or its features, run
`cargo hakari generate`; a new member also needs `cargo hakari manage-deps`. `.config/hakari.toml` lists the platforms, and the
crates kept out because only opt-in features would pull them in (tracy, GPU profiling). Because of those
excludes `cargo hakari verify` reports differences by design; the check is `cargo hakari generate --diff`. Never set
compile-time log level features (`log`/`tracing` `max_level_*`): unification would apply them to every tool.

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

### Configuration files
Every TOML or KDL file a user or developer writes by hand (bundled settings, shader presets, conversion tables, profiles, tool configs) is read strictly: an unknown key, node or property is an error, and so is a missing required key. Use `#[serde(deny_unknown_fields)]` without `#[serde(default)]` (unless the format makes a key optional by design, then prefer `Option<T>` and say why), read TOML through `uocf::utils::config_toml::parse_strict`, and decode KDL through `uot_kdl::strict` (a `NodeSchema` names the allowed nodes and properties for the error message). Errors name the file, the line and column, the offending name and the allowed names; apps print them and exit with a non-zero code instead of panicking.

Only files an app writes itself and nobody edits (dynamapper's cache `settings/*.toml`, the udd-conv-gui config) are lenient, through `config_toml::parse_app_state`: a missing key is filled from the defaults and an unknown key is dropped, each with a logged warning, and the app rewrites the file complete on its next save, so a file from another version still loads.

Dynamapper's hand-written specs under `assets/runtime_specs/` (KDL) have user copies at the same path under `cache/runtime_specs/`, read in place of the bundled file when they exist (`configs::runtime_specs::SpecFiles`): the bundled file is required, the copy is strict too, and a copy that does not read leaves the bundled file in force with a warning in the log and in game; whatever writes the copy (the paperdoll menu) reads it again first and leaves an invalid one as it is. Use `SpecFiles` for every runtime spec rather than reading `assets/runtime_specs/` directly.

### Dynamapper rendering and UI

- Preserve logical map and object coordinates, raw height, tile heights and movement flags separately from visual offsets, relief and depth ordering. Rendering must not replace world data needed for future movement and surface selection.
- Keep primary actions configurable through the keybindings settings rather than hardcoding keys.
- Schedule egui drawing systems in `EguiPrimaryContextPass`. Internal Bevy plugins follow the `TrackedPlugin` and build-logging convention; register third-party plugins as such.
- Avoid mutably accessing materials in hot paths just to read them: mutation triggers re-extraction and rebinding.
- Keep Rust material uniform bindings and shader declarations in agreement. When changing a uniform, update its Rust definition, populated value and shader declaration together.
- Use the logging facade for diagnostics rather than printing from the application.

There are three shading modes and four render styles: Classic, Enhanced, KR-like and UOD-Painterly. KR-like and UOD-Painterly share a shading mode but have separate presets. Test visual changes across all four as required by `AGENTS.md`. The bundled shader preset comments and `docs/agent_notes/shader_settings_audit.md` provide additional context; check current settings and code when using the dated audit.

For missing Enhanced terrain textures, retain the explicit missing-texture behavior rather than silently substituting Classic texmaps. See `docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md` for the terrain and transition constraints.

### Shell scripts
Standalone shell scripts are POSIX sh (`#!/bin/sh`, no bashisms: no `[[ ]]`, arrays, `BASH_SOURCE` or `pipefail`), so they behave the same whether run directly or as `sh`, `dash`, `bash` or `zsh` followed by the script path. Find the script's own folder with `$(dirname "$0")`, quote every expansion, and call `python3`, not `python`. `just check-shell` (run in CI) checks them with `shellcheck -s sh`. Justfile recipes are exempt: just runs them with bash.

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

For EC renderer/UDDP work, start with `docs/dev_wiki/UDDP_FORMATS.md`,
`docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md` and the executable
research linked from `docs/dev_wiki/rendering/EC_WORLD_SPACE_UV_EVIDENCE_MAP.md`.
The former root `plan_ec_textures.md` and the old lighting/land-shader overview
were retired after the executable-derived rework; retrieve them from git only
for history. Dated reviews and `docs/agent_notes/ec_material_audit/` inventories
record their original package revisions and hypotheses, not current contracts.
Do not restore filename-based roles or reopen completed work from those records.

---

## Further Reference

- `docs/dev_wiki/` — authoritative deeper technical notes
- `docs/agent_notes/` — agent-discovered notes, lower confidence, periodically reviewed
- `docs/TODO.md` — roadmap
