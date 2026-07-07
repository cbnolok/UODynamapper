# Handoff: water animation "brown bleed-through" + wave-angle tile misplacement

**Prepared 2026-07-04 for a fresh session, recommended model: Opus, high
effort.** This is a focused sub-thread spun out of the larger investigation
in `docs/agent_notes/land_shader_and_placement_issues.md` (read that file
too, especially issue H and its several follow-ups — CC surface-like
shoreline water statics were reworked from an upright billboard to a flat
XZ-plane quad; this handoff covers two remaining problems in that same
area that need a clean, focused pass). Don't re-derive what's already
established in that doc or here — cross-check line numbers, code has
moved around across many recent edits.

## Problem 1: water animation reveals land ("brown") on some frames — STILL BROKEN, prior investigation's conclusion contradicted by reality

### What the user reports (2026-07-04, real in-game testing, CC art source)
> "the brown bits are still showing when the animation reached specific
> frames and it was working months ago. it was a direct port of classicuo
> water animation, maybe there's still a reference in our md files about
> the algorithm."

This is **after** the flat-quad geometry fix (issue H) and **after** a
prior subagent investigation in this same thread that concluded, via a
full 360°-cycle numerical simulation against real decoded sprite alpha,
that the water-UV animation mechanism provably **never** opens a hole
(0.00% transparent fragments at every frame) and made no code change. The
user's fresh in-game report directly contradicts that conclusion — the
bug is real and still present. **Do not re-trust that simulation's
conclusion; find out what it missed.**

### Critical lead found by the orchestrator (2026-07-04), not yet acted on

`dynamapper/assets/shaders/world/effects/surface_effects.wgsl` contains,
in its own header comment, the **original ClassicUO reference algorithm**
this was ported from:
```
// From ClassicUO (View.cs / LandView.cs):
//   var sin = (float)Math.Sin(Time.Ticks / 1000f);
//   var cos = (float)Math.Cos(Time.Ticks / 1000f);
//   scale = new Vector2(1.1f + sin * 0.1f, 1.1f + cos * 0.5f * 0.1f);
```
i.e. base **1.1**, amplitude **0.1** (and 0.05 for the second axis:
`0.5*0.1`). But the actual shipped implementation right below that
comment, in `get_water_animation_scale()`, uses:
```rust
return vec2<f32>(
    1.3 + s * 0.28,
    1.3 + c * 0.14 // 0.5 * 0.28 = 0.14
);
```
base **1.3**, amplitude **0.28** (and 0.14) — a comment right there
admits this: `"Scale factors oscillate around ~1.3 to allow stronger
amplitude without dropping below 1.0."` **These numbers do not match the
"faithful port" the file claims to be**, and the amplitude is nearly 3×
larger than the original ClassicUO values.

`git log -p --follow` on this file shows the comment and the inflated
values were introduced **together, in the same commit** (`e3e6789b`,
2026-05-16) — so this isn't a later regression edit to a once-faithful
port; the amplitude has been inflated since the feature's introduction.
That said, this doesn't rule it out as the cause: it's plausible the
inflated amplitude was always slightly "too strong" but harmless under
the old **upright billboard** water rendering (where any resulting
texture bleed just looked like normal shimmer, with nothing precise to
seam against), and only became a *visible* defect once water became a
**tightly-fitted flat quad** (issue H, this past week) that must seam
pixel-perfectly with the terrain mesh beneath it — i.e. "it was working
months ago" may really mean "the whole rendering context was different
months ago," not that this specific animation code regressed on its own.

**Recommended first step for the new session:** try reverting
`get_water_animation_scale()` to the documented original ClassicUO values
(`1.1 + sin*0.1`, `1.1 + cos*0.05`) and see if that alone resolves the
report, OR — better — re-run/extend the prior subagent's simulation
methodology using the *actual* original values to see if the hole rate is
still 0% (if the original gentler amplitude also simulates to 0% holes,
that's consistent with "it was fine before, inflated amplitude is the
bug"; if even the original amplitude shows >0% holes under the new
flat-quad geometry, the bug is elsewhere and amplitude is a red herring).

### Why the prior simulation's "0% holes" conclusion might be wrong or incomplete

The user's real observation contradicts it, so something in that
simulation didn't match reality. Possible gaps to check (not yet
verified, just hypotheses for the new session to test):
- **Texture filtering / mipmapping at atlas boundaries.** The simulation
  likely treated texture sampling as ideal nearest/bilinear point-sampling
  against the *decoded* alpha array in isolation. Real GPU sampling of an
  *atlas-packed* sprite can bleed across tile boundaries via bilinear
  filtering or mip-level selection near the edge of the packed region,
  especially if the atlas gutter/extrusion (`extrude_rgba_rect_edges`,
  mentioned elsewhere in this codebase's atlas-packing code) doesn't fully
  cover the range the animated UV can reach. This is a real-GPU-sampling
  effect the abstract simulation may not have modeled.
- **Wrong assumption about which `centre` is actually used at runtime.**
  The prior investigation traced call sites in `art/main.wgsl`,
  `ground.wgsl`, `land/main.wgsl` and concluded `centre = vec2(0.5, 0.5)`
  tile-local is correct and matches the real sprite's decoded diamond
  center — but double-check this against the **current** code (things
  may have shifted since), and specifically check whether the flat-quad
  path's fragment shader converts atlas UV to tile-local UV the exact same
  way as the billboard path, or whether there's a subtle difference (e.g.
  aspect ratio, `uv_min`/`uv_max` extent handling) introduced by the H fix
  work that the simulation didn't account for.
- **`globals.time` value range / precision.** The ClassicUO original uses
  `Time.Ticks / 1000f` (seconds, presumably a `double`-precision
  millisecond tick counter divided down). Check what `globals.time` (Bevy)
  actually is — if it's a large, ever-increasing f32 (not wrapped/modulo'd
  ever), `sin(globals.time)`/`cos(globals.time)` on a large f32 value can
  lose precision (f32 has ~7 significant decimal digits; after a long
  session, `globals.time` could be large enough that `sin`/`cos` of it
  become numerically unstable, producing values slightly outside `[-1,1]`
  or otherwise wrong) — this would explain "only at specific frames" /
  after running "months" of engine-time-equivalent... actually more likely
  "after running for a while in a single session," check if the user's
  report correlates with how long the session had been running, or if it
  really is periodic/frame-phase-dependent regardless of session length.
- **Whether animation is really shared identically across ALL water
  render paths** (flat-quad CC, EC ground, land/terrain) as the prior
  session claimed, or whether the *fragment shader* consuming the animated
  UV for the CC flat-quad path specifically has a bug in how it combines
  the animated UV with the flat-quad's own `uv_min`/`uv_max` remapping
  (double-transformation bugs — e.g. applying the atlas-to-local
  conversion twice, or in the wrong order relative to the animation
  scale — are a classic source of "sometimes it's fine, sometimes it
  clips wildly depending on the current animation phase" bugs).

## Problem 1a (found 2026-07-04, after this handoff was first drafted): sawtooth/z-fighting at water boundaries — likely the REAL cause of Problem 1, start here

The user, on the same screenshot, separately reported: **"sawtooth
artifacts when statics collide (water and bridge wall) and map and
statics colliding (bottom right)."** Two distinct boundary types, both
showing a jagged/sawtooth pattern — the classic visual signature of
insufficient depth separation (z-fighting) between two surfaces at nearly
the same depth:
1. **Static-vs-static**: the flat water quad meeting the bridge's stone
   wall (an upright billboard) at their shared edge.
2. **Static-vs-map**: a static (the flat water quad, or a shoreline
   static) meeting the terrain/land mesh itself, visible bottom-right of
   the screenshot.

**This strongly corroborates, independently, the alternative theory the
prior water-animation subagent raised but didn't chase** (see Problem 1's
"why the prior simulation might be wrong" section above): a depth/z-
fighting interaction between the flat water quad (classified `Background`
depth class, small negative logical-depth offset per
`depth_class_logical_offset` in `statics_collect.rs`) and (a) the terrain
water mesh beneath it, and (b) adjacent upright billboards at the shore.
If the flat water quad's depth sometimes loses/wins the depth test against
the terrain mesh in a camera-angle- or precision-dependent way, that would
produce **both** symptoms the user has now reported independently: the
sawtooth pattern (classic z-fighting) at the boundary, and "brown showing
through on specific frames" (the terrain winning the depth test in a
thin strip or at specific viewing angles, unrelated to the water UV
animation at all — consistent with the prior subagent's simulation
correctly finding zero UV/alpha holes, because the real defect isn't in
the texture sampling, it's in which of the two full-opaque surfaces the
GPU decides is "in front").

**Recommended: investigate this before spending more time on the UV
animation amplitude theory.** Check:
- The exact depth/priority formula used for the flat water quad
  (`priority_z_units`, `depth_class_logical_offset`,
  `logical_depth_from_projected_priority` or equivalent in `main.wgsl`) —
  compare its actual computed depth value, for a water tile at world Z=0,
  against the terrain mesh's own depth at the exact same world position.
  Is the separation between them large enough to survive floating-point
  depth-buffer precision at typical camera distances/zoom levels, or is it
  a very small epsilon that can flip sign due to precision or a slightly
  wrong sign convention?
- Whether this is specifically about the *water quad vs. terrain* pairing
  (a new pairing introduced by issue H's flat-quad work — previously
  water was a billboard, never coplanar with the terrain mesh, so this
  z-fighting risk didn't exist before) vs. the *water quad vs. adjacent
  wall billboard* pairing (also new, since the water quad's geometry now
  extends into the same screen-space region a nearby wall's billboard
  card occupies, in a way the old vertical water billboard didn't).
- Whether the fix is a depth-bias/offset adjustment (give the flat water
  quad a slightly larger separation from the terrain, or ensure the sign
  is correct so it's *never* ambiguous which one is in front) or something
  structural (e.g. the terrain mesh and the flat water quad should not
  both be trying to occupy exactly the same world-Y at the same XZ
  position in the first place — check if there's a principled small
  Y-offset that should exist between "terrain water surface" and "static
  water overlay" that got lost or was never established when the flat-quad
  work landed).

If this pans out, it may make the whole "revert to original ClassicUO
1.1/0.1 amplitude" experiment in Problem 1 above moot — the amplitude
might have been fine all along, and this depth-sort bug is the actual
root cause "since months ago [when water was rendered differently, before
the flat-quad geometry existed at all]." Still worth quickly trying the
amplitude revert too since it's cheap, but don't let it substitute for
investigating this depth-sort angle.

### RESOLVED (2026-07-04) — depth-sort theory CONFIRMED and fixed (flat quad now uses its own geometry depth)

The depth-sort theory is correct and objective; the water UV amplitude was a
red herring (not touched). Mechanism, verified from code + math:

- The land/terrain shader (`land/main.wgsl`) writes NO `@builtin(frag_depth)`
  — terrain uses true, per-fragment geometric depth. The art shader
  (`art/main.wgsl`) OVERRIDES `frag_depth` with a per-instance CONSTANT
  "logical depth" (`logical_depth_from_projected_priority`): it projects a
  single point `(inst.tile_x, priority_z*0.1 + depth_class_offset,
  inst.tile_y)` — the cell ORIGIN — so the whole sprite writes ONE depth.
- Terrain material = StandardMaterial default `Opaque` → Opaque3d (drawn
  first, writes depth). Opaque static water = `AlphaMode::Mask(0.5)` →
  AlphaMask3d (drawn after, depth test+write). Both are
  `ExtendedMaterial<StandardMaterial,_>` sharing the one Camera3d depth
  buffer. Bevy 0.18.1 = reverse-Z (near=1/far=0, GreaterEqual): nearer =
  larger depth. Iso camera (offset (5,5,5)) → reverse-Z depth is monotonic
  increasing in world (x+y+z).
- A constant per-sprite depth is fine for upright billboards (thin, stand
  well above the ground plane, tuned to sort in painter order). It is
  fundamentally wrong for the H flat quad: that quad is coplanar with the
  terrain and spans a whole cell in the ground plane, where the terrain's
  own depth VARIES by ~2 world-units of (x+z) across the tile (~5.8e-4 NDC),
  vs. the Background class' 0.001 world-Y logical offset (~3e-7 NDC) — a
  ~2000x mismatch. So terrain wins the depth test over a moving diagonal
  band of each water tile: exactly the sawtooth boundary AND the intermittent
  "brown on specific frames" (the band sweeps as the camera follows the
  player). Same cause covers water-vs-terrain and water-vs-adjacent-wall.
  Not a UV/alpha hole — consistent with the earlier 0%-hole simulation.
- Fix (one WGSL branch, `art/main.wgsl` fragment, flat_quad only): write the
  quad's own rasterized per-fragment depth `out.depth = in.position.z`
  instead of the constant. The quad geometry is already placed
  `STATIC_ART_Y_BIAS` (=0.002 world-Y, depth_class_y_bias for Background)
  ABOVE the terrain, so as parallel planes it beats terrain by a uniform
  margin at every fragment (~1.7e-6 NDC ≈ tens of f32-depth ULP — plane
  separation along the view ray = 0.002·√3, /2000 depth range), no gradient
  mismatch → no sawtooth, no bleed. Margin is zoom-independent (ortho
  near/far fixed at ±1000, unaffected by ortho.scale). Billboards, EC,
  shading, anchors, offsets untouched. `SURFACE_LIKE_DEPTH_CLASS_OFFSET`/
  Background `-0.001` left as-is (only fed the constant path we now bypass
  for flat water; still used by billboards/other classes).
- Verified: `cargo check --workspace` clean; `cargo test -p dynamapper`
  89/89. WGSL not cargo-validated — syntax hand-checked against the same
  file's existing `in.position`/`inst.flat_quad`/`out.depth` usage. Not yet
  runtime-compiled or visually confirmed in-game (needs the user).

## Problem 2: "wave angle" tiles now misplaced (new report, 2026-07-04, screenshot provided)

The user's latest screenshot (a wooden bridge over water) shows small
individual "wave" decorative statics near the shore that are now visibly
misplaced/offset — a new observation, immediately after the CC billboard
anchor fix (`CC_STATIC_TILE_TRANSLATION_X/Z = 1.0/1.0`, see the main
tracking doc's most recent entries) and the WET-only flat-quad gating fix
(`cc_static_tile_renders_flat`) both landed in this same thread.

### What's already known (from the main tracking doc, don't re-derive)
- Real classic "wave" static item graphics are in range **`0x1fa3-0x1fca`**
  (distinct from plain water/shoreline graphics like `0x1796-0x17B2`,
  `0x346E+`, `0x3494+`, `0x55F0+`), and **all carry the `WET` tiledata
  flag** — confirmed against real `tiledata.mul`
  (`/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/`).
  This means wave tiles **do** go through the flat-quad path
  (`cc_static_tile_renders_flat`/`emit_cc_surface_like_flat_static_instance`),
  same as plain water, both before and after the WET-only gating fix — so
  this is very unlikely to be a *classification* regression (they were
  already being flattened).
- The plain water sprites (`0x1797`, `0x346E`, `0x3494`, `0x55F0`) were
  decoded and confirmed to be **perfectly centered, symmetric** 44×44
  diamonds (centroid exactly at 21.5,21.5, tips at exact UV edge
  midpoints) — this is the assumption `resolve_cc_surface_like_flat_quad_bounds`
  and the diagonal `BILLBOARD_RIGHT_XZ`/`FLAT_QUAD_FORWARD_XZ` transform
  in `main.wgsl` are built on (center the quad, size from
  `logical_width`/`logical_height` only, assume the diamond is centered in
  its own bounding box).

### What's NOT yet checked — the likely real cause

**Nobody has yet decoded the actual `0x1fa3-0x1fca` "wave" sprites from
`art.mul` to check whether their diamond/art content is ALSO centered and
symmetric in their bounding box, the same way plain water is.** Given
their name ("wave *angle*" — the user's own word choice, suggesting a
tilted/diagonal/asymmetric visual, e.g. a wave crest breaking at an angle
rather than a plain symmetric diamond) and that they're presumably meant
to render at shoreline edges/corners (where a symmetric water diamond
wouldn't look right, hence a special "angled" art variant), it's very
plausible these sprites are **not** centered/symmetric within their
bounding box the way plain water is. If so,
`resolve_cc_surface_like_flat_quad_bounds`'s centering assumption
(everything is centered, size purely from width/height) would misplace
them — exactly matching "misplacement of wave angle tiles."

**Recommended investigation for the new session:**
1. Decode several real sprites from the `0x1fa3-0x1fca` range (and any
   other ranges tiledata/naming suggests are "wave" variants — search
   `tiledata.mul` names near these IDs for "wave"/"foam"/similar) from
   `/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/art.mul`,
   the same way the plain water sprites were decoded in the H
   investigation. Check: are they centered/symmetric diamonds like plain
   water, or asymmetric/offset shapes?
2. If asymmetric: the flat-quad path needs to account for the sprite's
   real `draw_offset_x`/`draw_offset_y` (or equivalent) the same way the
   billboard path's `resolve_static_billboard_bounds` does (recall issue C:
   CC statics get `draw_offset_x = -(width/2)`, `draw_offset_y = 0` from
   `classic_static_draw_offset` — check whether this generic offset
   convention is even appropriate for "wave" art, or whether wave sprites
   need their own handling to end up correctly placed relative to the
   cell when flattened).
3. Consider whether "wave angle" tiles are conceptually a *third*
   category, distinct from both plain flat water and upright billboards —
   e.g. a foam/splash decoration meant to sit at a specific corner/edge
   orientation of the cell rather than centered — which might mean the
   uniform flat-quad treatment isn't even the right model for them, and
   they may need their own placement logic (or might legitimately need to
   stay closer to a billboard-like placement despite carrying `WET`). Don't
   assume the fix is purely numerical; the right *model* for these tiles
   might differ from plain water.

### RESOLVED (2026-07-04) — hypothesis confirmed by decoding the real sprites; height-aware anchor fix applied

Decoded all 40 sprites in `0x1fa3-0x1fca` from the real client's `art.mul`
(decoder validated against the known-good plain-water references, which
re-confirmed as perfectly centered 44×44 diamonds, centroid 21.50/21.50).
Findings:
- All 40 are named `wave`, all carry `WET` (flat-quad path is the right
  classification), tiledata `height`=0 for all.
- **None is a centered symmetric diamond.** Two sub-sets: `0x1fa3-0x1fb6`
  (20 sprites, **45px** bitmaps, thin crescents hugging an edge/corner,
  opaque-centroid offsets up to ±8px — a rotational/directional series)
  and `0x1fb7-0x1fca` (20 sprites, **~89px** bitmaps, large breaking-wave
  arcs spanning ~2 cells, centroid offsets up to ±20px).
- Root cause of the misplacement: `emit_cc_surface_like_flat_static_instance`
  centered every flat quad on the cell (`+0.5, +0.5`) — a rule that is only
  correct because plain water happens to be 44px tall. Deriving the anchor
  from the validated CC billboard convention (box feet at the cell's near
  vertex `(wx+1, wz+1)`, box laid flat extending up-screen along
  `-FLAT_QUAD_FORWARD_XZ`), the correct per-axis center offset is
  `1 - h/88` — which equals exactly `0.5` at `h == 44` (hence water was
  verified correct) but ≈ `0.0` for the ~89px waves. The hardcoded
  cell-center therefore displaced the large breaking waves by ~0.5/axis,
  **~0.72 world units ≈ one full cell diagonally up-screen** — the reported
  gross misplacement. The small 45px set was within ~2px of correct (their
  shore-hugging look is baked into the art and is preserved by design).
- Name sweep for other wave-like art: only these 40 `wave` entries are
  affected; `ripples` (`0x34d1-0x34d5`, `0x5670-0x5674`, WET) and `splash`
  (mostly non-WET) are separate sets, not part of this report.

**Fix applied** (`statics_collect.rs`): new
`cc_surface_like_flat_quad_center_offsets(logical_height)` — per-axis
center offset `CC_STATIC_TILE_TRANSLATION_* - h·CC_WORLD_XZ_PER_PIXEL·(1/√2)·0.5`
(= `1 - h/88`), replacing the hardcoded `+ HALF_MAP_CELL_WORLD` (constant
removed, now unused). Zero regression by construction on plain water
(h=44 ⇒ 0.5, byte-identical placement) and on non-flat statics (they don't
take this path). Bounds stay symmetric; depth-sort keys (`tile_x/tile_y`)
stay at the cell origin. New unit test
`cc_flat_quad_center_offsets_track_sprite_height` (44 ⇒ 0.5, 88 ⇒ 0.0).
Verified: `cargo check --workspace` clean, `cargo test -p dynamapper`
**90/90** (89 + the new test). Awaiting the user's in-game visual
confirmation (CC art source): breaking waves sit on/along the shoreline
instead of one cell up-screen; plain water unchanged.

## General guidance for the new session

- Read `docs/agent_notes/land_shader_and_placement_issues.md` in full
  first (long document, but has the full history — issues A through H and
  several follow-up rounds, all in this same rendering area).
- This project's established pattern in this whole thread: objective
  correctness bugs with a clear, well-evidenced fix can be implemented
  directly without asking the user first; if the right answer requires a
  judgment call or an assumption that can't be verified, stop and report
  rather than guess.
- No display is available to any agent in this environment — ground every
  conclusion in code reading, git history, and real decoded client data
  (`/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/` has
  the real `.mul` files), not visual guessing. Real in-game visual
  confirmation always has to come from the user afterward.
- Minimal, surgical diffs; no magic numbers without a named, commented
  constant; don't touch unrelated paths (EC, non-water statics, the
  already-fixed fake-normal shading work in `shading.wgsl`, the already-
  fixed CC billboard anchor in `statics_collect.rs`).
- Verify with `cargo check --workspace` and `cargo test -p dynamapper`
  (currently 89 passing) after any change. WGSL isn't validated by cargo —
  double-check shader syntax by hand against existing working patterns in
  the same file.
- Update `docs/agent_notes/land_shader_and_placement_issues.md` (or this
  file) with findings before ending the session, so a further follow-up
  session (or the user) can pick up cleanly — this thread has been going
  for a while and cross-session continuity has been valuable throughout.
