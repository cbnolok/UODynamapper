# Handoff: flat-quad water/wave follow-ups (verification, commit, offset-caveats study)

**Prepared 2026-07-06 for a fresh session.** Continuation of the thread in
`docs/agent_notes/land_shader_and_placement_issues.md` (issues A–H + rounds
of follow-ups) and `docs/agent_notes/water_animation_wave_tile_handoff.md`
(both RESOLVED sections at the end). Read both before touching code — this
file only covers what is *left to do*; don't re-derive what those already
establish. Line numbers in older notes are stale; re-locate by symbol name.

## Current state (as of 2026-07-04 session end)

Two fixes landed, **verified statically but NOT yet confirmed in-game and
NOT committed**:

1. **Depth fix** (`dynamapper/assets/shaders/world/art/main.wgsl`,
   fragment shader): flat quads (`inst.flat_quad != 0u`) now write their
   own rasterized per-fragment depth (`out.depth = in.position.z`) instead
   of the per-instance constant logical depth. Root cause and the numeric
   derivation are in the RESOLVED section of the water handoff doc
   (constant-vs-gradient depth mismatch ⇒ sawtooth + sweeping "brown
   bleed-through" band; ~2000× NDC mismatch; quad wins uniformly now via
   the existing `STATIC_ART_Y_BIAS` 0.002 world-Y separation).
2. **Wave anchor fix**
   (`dynamapper/src/core/render/scene/world/art/statics_collect.rs`): new
   `cc_surface_like_flat_quad_center_offsets(logical_height)` — per-axis
   flat-quad center offset `CC_STATIC_TILE_TRANSLATION_* − h·CC_WORLD_XZ_PER_PIXEL·(1/√2)·0.5`
   (= `1 − h/88`), replacing the old hardcoded cell-center
   (`HALF_MAP_CELL_WORLD`, constant removed). Identical to old behavior at
   h=44 (plain water regression-free by construction); fixes the ~89px
   breaking waves (`0x1fb7-0x1fca`) that were displaced ~one cell
   up-screen. Unit test `cc_flat_quad_center_offsets_track_sprite_height`
   added.

Validation done: `cargo check --workspace` clean;
`cargo test -p dynamapper` **90/90**. WGSL is NOT validated by cargo —
naga-oil compiles it at first app launch; the depth hunk's syntax was
hand-checked against existing uses of `in.position` / `out.depth` in the
same file but has never been runtime-compiled.

The working tree also carries several *earlier* sessions of related
uncommitted work (flat-quad rework, anchor fixes, shading fixes, texmap
cap fix, cursor-overlay wrap, plus unrelated pre-existing user edits in
tools/, lib/, settings TOMLs). Nothing from this thread has been
committed yet.

## What needs to be done (in order)

### 1. Gate: user in-game verification (blocks everything else)

CC art source, all 3 rendering presets (per `dynamapper/CLAUDE.md`), user
checks:
- First launch = first real naga-oil compile of the new WGSL. If the app
  fails shader compilation, the depth hunk in `art/main.wgsl` (fragment,
  `if (inst.flat_quad != 0u) { out.depth = in.position.z; }`) is the
  newest suspect.
- No sawtooth at water↔wall and water↔terrain boundaries; no brown
  terrain bleeding through water at any animation frame / camera
  position / zoom.
- Breaking waves (`0x1fb7-0x1fca`, ~89px) sit on the shoreline instead of
  ~one cell up-screen; small waves (`0x1fa3-0x1fb6`) and plain water
  unchanged.
- Regression sweep of the earlier uncommitted fixes listed in the main
  tracking doc's last TODO (billboard anchors, fake-normal artifacts,
  bridge decks).

### 2. After confirmation: commit the batch(es)

Per root `CLAUDE.md`: stage only files relevant to each coherent batch;
do NOT absorb the unrelated pre-existing user edits (settings TOMLs,
tools/, .cargo, justfile, workflow files…). The rendering-thread work
spans: `statics_collect.rs`, `statics_draw.rs`, `art_bindings.wgsl`,
`art/main.wgsl`, `shading.wgsl`, `draw_mesh.rs`,
`land_texture.rs` (uocf), `tex_art_cc.rs` (udd-conv),
`cursor_behavior.rs`, and the three `docs/agent_notes/*.md` files.
Suggest splitting into reviewable commits (e.g. texmap cap fix; CC
draw-offset fix; flat-quad water rework + follow-ups; docs). Check
`git diff` per file — some files may mix thread work with unrelated
user edits (historically `statics_draw.rs` had an unrelated
`TextureFormat` hunk; verify before staging).

Also after confirmation: delete the two runtime-asset backups
`/home/claudio/test/uddp/tex_land_cc.uddp.bak-pre-texmap-max-id-fix` and
`tex_art_cc.uddp.bak-pre-y-offset-fix` (see main tracking doc TODOs).

### 3. Caveats study — flat-quad path vs. stored draw offsets

Context (verified 2026-07-06, current code): the flat-quad emit path
(`emit_cc_surface_like_flat_static_instance`) consumes ONLY
`resolved.logical_width/logical_height`; it ignores
`resolved.offset_x/offset_y` (the per-slot `draw_offset_x/y` stored in
`tex_art_cc.uddp`, surfaced by `dynamapper/src/core/texture_cache/art.rs`).
The billboard path consumes them (`resolve_static_billboard_bounds`).
`tilemeta.uddp` carries NO image offsets at all (only tiledata-style
fields: flags, height, `stacking_offset`, …) — its role here is just the
WET flag that routes tiles into the flat path.

Ignoring offsets is currently justified because on the CC **primary**
packing path they're synthetic, derived purely from the bitmap dims by
`classic_static_draw_offset` (`lib/udd-conv/src/tex_art_cc.rs`):
`x = -(width/2)`, `y = 0` — information the flat path already re-derives
from width/height. But two caveats deserve a focused pass:

**Caveat 1 — odd-width sub-pixel mismatch (small, optional polish).**
`-(width/2)` is floored integer division; the flat path splits `±w/2`
exactly. For odd widths (waves are 45px and ~89px wide) the two
conventions differ by 0.5px ≈ 0.016 world units horizontally.
Study/decide: honor `resolved.offset_x` on the RIGHT axis of the flat
quad (i.e. derive the horizontal centering from the stored offset instead
of symmetric `±w/2`) so flat and billboard placement are bit-identical in
convention. Pure sub-pixel effect; implement only if going for exactness,
and keep plain water byte-identical (44 is even ⇒ unaffected).

**Caveat 2 — alias-path REAL offsets are ignored by the flat path
(needs study; scope depends on packing workflow).** Verified mechanics in
`lib/udd-conv/src/tex_art_cc.rs`:
- Slot aliases are built ONLY when the CC art metadata source is
  `TexArtCcMetadataSourceKind::TileartUop` (i.e. packing CC art from a
  source tree WITHOUT `tiledata.mul`, using EC's `tileart.uop` as
  fallback — see the `match metadata_source.kind` near the top of the
  convert fn, ~line 390). With the normal `TiledataMul` workflow (the
  user's standard: `Ultima Online Classic_7_0_20_0`, see memory notes)
  the alias list is **empty** and every slot offset in the package is
  synthetic ⇒ caveat 2 has zero exposure today.
- When `TileartUop` IS the source, `load_tileart_cc_slot_aliases`
  (~line 933) creates an alias for EVERY tileart entry with a
  `cc_texture`, carrying **real authored offsets**
  (`texture.offset_x/offset_y` from `tileart.uop`), and
  `apply_slot_aliases` (~line 964) **overwrites the target slot record**
  with the canonical slot's atlas rect + the alias's real offsets. These
  can be arbitrary (not `-(w/2),0`), including nonzero Y.
- Consequence: for a package built via the tileart fallback, a WET tile
  rendered by the flat-quad path would have its real X *and* Y offsets
  silently dropped (billboards would honor them). The `1 − h/88`
  feet-anchor assumes the synthetic bottom-anchored convention
  (`offset_y == 0`); a real nonzero `offset_y` breaks that assumption.

Study plan for caveat 2:
1. Determine whether the tileart-fallback packing path is actually used
   in any supported workflow (ask the user if unclear — per memory, CC
   packing is done from a full classic tree with tiledata.mul; the
   fallback may be for EC-only trees).
2. If it is (or should be supported): decide the correct flat-quad
   placement formula when offsets are real. Natural generalization: the
   billboard convention places the box left edge at
   `offset_x` px from the anchor and the bottom at `-offset_y` px; laying
   that flat means the RIGHT-axis center comes from
   `offset_x + w/2` and the FORWARD-axis feet from `offset_y` (replacing
   the assumed 0) before the `h/2` pull-back — i.e. fold both offsets
   into `cc_surface_like_flat_quad_center_offsets` and derive today's
   behavior as the synthetic-offset special case. Verify against real
   decoded `tileart.uop` data before implementing (note: the EC
   `tileart.rs` parser produced garbage offsets on the real
   `_Ultima Online Enhanced` client once before — see issue C notes —
   fix/verify parsing first or the study data will be junk).
3. If the fallback path is NOT a supported workflow: document that
   explicitly in `tex_art_cc.rs` near `apply_slot_aliases` (one comment)
   and close the caveat as "by design; flat path assumes synthetic
   offsets."

### 4. Smaller leftovers from the main tracking doc (unchanged priority)

Still open there (see its TODO sections; none blocking):
- Issue D design decision (Basic Controls "Render Style" vs.
  `post_process_profile`), then wire-up + F's dialog re-layout.
- In-app A/B of Grunge / Art Projected Shadows / Art Fake Normals
  (broken vs. just subtle).
- Issue B user decision (classic-faithful vs. exaggerated height scale).
- EC `tileart.rs` offset-parsing bug on the real EC client (also feeds
  caveat 2's study, step 2).
- Dead stub `try_load_cc_land_tile` (implement or delete).
- Billboard anchor open question: if walls look aligned sideways but
  pushed slightly forward off their tile, switch CC billboard anchor
  target from near-vertex `(1.0,1.0)` toward center `(0.5,0.5)` — watch
  for this in the in-game check.

## Working rules (unchanged from prior handoffs)

- No display available to agents: ground conclusions in code, git
  history, and real client data
  (`/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/`);
  visual confirmation comes from the user.
- Minimal surgical diffs; named+commented constants only; don't touch EC
  paths or already-fixed areas without cause.
- Validate with `cargo check --workspace` + `cargo test -p dynamapper`
  (90 passing as of this handoff). WGSL: hand-check syntax; real
  validation happens at app launch.
- Update this file / the main tracking doc before ending the session.
