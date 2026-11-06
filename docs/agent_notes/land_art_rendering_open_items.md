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

## Superseded investigation directions

The July height, offset and anchor hypotheses are not current open tasks:

- Raw Z now renders at `4/44` world Y per unit; see
  [the executable-derived calibration and native checks](static_height_calibration.md).
- EC windows, descriptor scales and offsets have decoded contracts; see
  [source geometry](ec_sprite_descriptor_geometry.md). Do not treat the old
  garbage-offset observation as the current parser's behavior.
- Classic sprites retain their south-vertex anchor. Do not move all sprites to
  the cell center to compensate for an individual artwork or depth mismatch.
- Grunge and optional normals have receiver-specific reachability checks in
  [effect visibility](effect_visibility_validation.md) and
  [art normal validation](art_fake_normal_validation.md).

Current work is tracked in [the canonical TODO](../TODO.md). The old session
hypotheses remain available in git history, not as implementation instructions.

## Remaining implementation caveat

`try_load_cc_land_tile` in `dynamapper/src/core/texture_cache/land/cache.rs`
is deliberately unwired. Revisit its documented assumptions before enabling it;
this is separate from the EC geometry and height repairs.
