# Land/art rendering: open items from the July 2026 investigation

Consolidated on 2026-10-25 from three session logs, now removed:
`land_shader_and_placement_issues.md`, `water_animation_wave_tile_handoff.md`,
`flat_quad_water_wave_followups_handoff.md`. Their full history (hypotheses,
ruled-out leads, derivations) is in git: `git log --all -- docs/agent_notes/<name>`.

## Resolved

All user-verified in game and committed on `prereleases`:

- A: CC land textures missing; texmap cap replaced by `texidx.element_count()` (`ec4d353`).
- C: CC statics floated by their own height; draw offset double count (`2b25879`).
- G: CC land rendered black; graphic-id vs texture-id lookup in `draw_mesh.rs` (`e66a0bb`).
- H: shoreline water statics drawn as upright billboards. Now flat XZ quads with per-fragment
  depth and a height-aware wave anchor (`68bf045`). The coast seams were closed later; see
  `water_terrain_static_resolution.md`.
- E: cursor inspect overlay wrap (`bf1bdc0`).
- D: Basic Controls "Render Style" now also sets `post_process_profile` (`b29004a`).
- F: shader dialog layout and categories (`8b32930`, `032557a`).
- Art projected shadows made visible (`528237d`).

## Still open (none blocking)

- **Issue B, land height scale (`0.1`).** The geometric derivation found it plausible, not
  wrong. This is a design decision between classic-faithful and exaggerated relief. The user
  wanted to set direction from screenshots first.
- **Grunge.** Still needs source/preset/strength-specific visual checks. Explicit
  non-KR art-normal activation was reviewed later on stone/timber fixtures; see
  `art_fake_normal_validation.md` for the result and its limits.
- **EC `lib/uocf/src/enhanced/tileart.rs` offsets.** The parser produced garbage offsets on
  the real EC client. Optional, low priority: its motivating CC caveat was closed as
  by-design (comment above `apply_slot_aliases` in `lib/udd-conv/src/tex_art_cc.rs`).
- **CC billboard anchor.** Targets the near vertex `(1.0, 1.0)`. If walls look aligned
  sideways but pushed slightly forward off their tile, try the centre `(0.5, 0.5)`.
- **`try_load_cc_land_tile`** (`dynamapper/src/core/texture_cache/land/cache.rs`). Implemented
  but deliberately not wired in; a future switch enables it. The assumptions its author must
  revisit are documented at the function.

## Gotchas worth keeping

- "Render Style" drives **land** shading only; art has a separate shading path.
- Land normal maps only apply to EC atlas land (`texture_size == 2`). They are a structural
  no-op on CC sources. The dialog can't grey them out because it has no global "source is
  EC" signal (source is per tile), so EC-only controls carry a static label.
- `just` 1.51 conditions accept a single comparison only: no `&&` / `||`, even in
  parentheses. Use nested `if`s.
- Ground conclusions in code, git history and real client data under
  `/mnt/dati/_proj_local/_uo_clients/`. Visual acceptance comes from the user.
