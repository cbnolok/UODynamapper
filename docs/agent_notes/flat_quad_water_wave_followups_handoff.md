# Handoff: flat-quad water/wave follow-ups (verification, commit, offset-caveats study)

## SESSION UPDATE 2026-07-07 — steps 1–3 DONE

- **Step 1 (in-game verification): PASSED.** User confirmed all checks
  good on CC art across the 3 presets (shaders compile, no sawtooth /
  brown bleed-through, breaking waves on the shoreline, earlier
  uncommitted fixes still correct).
- **Step 2 (commit + cleanup): DONE.** The rendering-thread work was
  committed on `prereleases` in 6 focused batches, WITHOUT absorbing the
  unrelated pre-existing user edits (the wgpu→bevy import migration in
  `core.rs`/`tile_atlas.rs`/`texture_cache/art.rs`/`statics_draw.rs`
  TextureFormat hunk/`rgba_bounds.rs`, settings TOMLs, tools/, scripts/,
  Cargo files — all still uncommitted by design):
  `ec4d353` texmap cap fix (uocf) · `e66a0bb` CC land slot resolution
  (draw_mesh) · `2b25879` CC static draw-offset fix + caveat-2 comment
  (tex_art_cc) · `68bf045` flat-quad water rework (art_bindings/main/
  shading wgsl + statics_collect + statics_draw flat_quad hunk only) ·
  `bf1bdc0` overlay wrap (cursor_behavior) · `d466ea9` docs. The two
  `.bak-pre-*` runtime-asset backups were deleted.
  `statics_draw.rs` was staged partially: only the `flat_quad` field-init
  hunk; its unrelated `TextureFormat` migration hunk stays uncommitted
  with the rest of the wgpu→bevy migration.
- **Step 3 (offset caveats): CLOSED.** User confirmed the tileart.uop
  fallback packing path (`TexArtCcMetadataSourceKind::TileartUop`) is NOT
  a supported CC workflow (CC art is always packed from a full classic
  tree with `tiledata.mul` ⇒ empty alias list ⇒ synthetic offsets only).
  Caveat 2 closed as by-design with a documenting comment above
  `apply_slot_aliases` in `lib/udd-conv/src/tex_art_cc.rs`. Caveat 1
  (odd-width 0.5px sub-pixel polish) intentionally left un-done — plain
  water is byte-identical regardless; revisit only if going for exactness.

**Still open: only "Smaller leftovers" (§4 below).** Most need a user
design decision or a display; none are blocking. The original body below
is retained for context.

### 2026-07-07 (later) — two §4 leftovers landed

Picked up the two self-contained §4 items (user steer: "easier tasks
first"). Both committed on `prereleases`, validated (`cargo check
--workspace` clean, `cargo test -p dynamapper` 90/90). Did NOT touch the
user's still-uncommitted local edits (`.cargo`, settings TOMLs).

- **Dead stub `try_load_cc_land_tile` → implemented, kept UNWIRED**
  (`36ddbd1`). User decision: implement it fully but do NOT invoke it yet
  (easier to debug without it; a switch will enable it later). It now
  extracts a land tile from the CC **art** package (`tex_art_cc`, art.mul
  land art) and crop/pads it into a texmap-sized buffer. Added
  `TexArtCcPackage::get_pixel_data` (mirrors `TexLandCcPackage`) so the
  slot crop is correct for both RGBA8888 and BC7 pages — do NOT hand-roll
  an atlas-stride crop, BC7 pages decode to `used_w×used_h`, not atlas
  dims. Assumptions the future switch's author must revisit are documented
  at the fn: (a) `texture_id` is treated as a CC **art** slot id
  (`art_id`), not a texmap id — different id spaces; (b) top-left crop/pad,
  not centered/scaled onto the cell diamond (the stretched land mesh will
  want that). The live loader
  (`try_load_tile_from_packages_with_sources`) still ignores `tex_art_cc`.
- **Per-overlay overlay toggles** (`d674ac7`). "Show Performance Overlay"
  was a single master gating all three overlays (perf stats,
  player-position, cursor/tile-inspector). Kept it as master, added three
  per-overlay flags on `SectPerformance` (`show_perf_stats_overlay`,
  `show_player_position_overlay`, `show_cursor_inspect_overlay`,
  `#[serde(default = "default_emit_true")]` for back-compat). Effective
  visibility = master AND flag, via helper methods
  (`perf_stats_overlay_visible()` etc.) used at every gate site.
  Preferences dialog shows the three as indented sub-checkboxes, greyed
  out when the master is off. Runtime visual confirmation is the user's
  (no display for agents).

**`justfile:155` parse error — FIXED** (`daf6e82`). Root cause: `just` 1.51
conditions accept only a *single* comparison — no `&&`/`||` chaining, even
parenthesized (verified empirically). `build_features` was rewritten with
nested `if`s (all 4 feature combos verified equal to the old intent). `just`
now parses; the `bevy-lint` recipe is reachable, but **`bevy_lint` itself is
not installed** (`bevy lint` → "install via `bevy lint install`", needs a
nightly toolchain fetch — left for the user).

### 2026-07-07 (later 2) — Issue D LANDED (`b29004a`)

User picked all three recommended options (coupling, scope-grouped sections,
tag+remove-dead). Implemented in
`dynamapper/src/core/render/dialogs/land_shader.rs`, Basic Controls block:

- **Render Style now couples** `shading_mode` + `post_process_profile` via a
  named `RENDER_STYLE_PRESETS` table (Classic→0/Neutral, Enhanced→1/EC,
  KR→2/KR). Fixes the confirmed Issue-D bug: KR Transition Grime / KR Color
  Wash were gated behind `post_process_profile==2`, only reachable via the
  hidden Advanced "Visual Profile", so picking "KR" in Basic did nothing.
  Advanced Visual Profile kept as a post-hoc override (not removed).
- **Scope sections** via new `basic_scope_header(ui, title, caption)`:
  *Terrain (land)* / *Objects (art / statics)* / *Everything (whole scene)*.
  Full per-control scope audit (which land/art/postprocess shaders read each
  uniform field) is the basis; Render Style itself only drives LAND shading
  (art has a separate shading path) — worth remembering.
- **EC-only knobs tagged** "(EC land only)" — Normal Maps + Land Normal Maps
  only affect `texture_size==2` (EC atlas) land; no-op on CC art source. NOT
  dynamically greyed: there is no global "source is EC" signal in the dialog's
  scope (source is per-tile), so a static label states the scope instead.
- **Dead knob removed**: "Static Light Decals" — `static_light_decal_visibility`
  has no shader reader (only the uniform struct decl). UI control dropped; the
  struct field left in place (GPU layout).

Validated: `cargo check -p dynamapper` clean, `cargo test -p dynamapper` 90/90.
Did NOT touch the user's still-uncommitted local edits (`.cargo`, settings TOMLs).

**Still open in §4:** Issue B + A/B tuning (deferred — user has screenshots to
set direction first), EC `tileart.rs` offset parsing (optional/low priority;
its motivating caveat-2 was already closed as by-design). Issue D re-verify:
user to confirm in-game that picking KR in Basic Controls now visibly activates
KR Transition Grime / Color Wash, across the 3 presets.

---

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
