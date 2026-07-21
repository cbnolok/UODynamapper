# Handoff: water seam artifact — static water vs land water

> **Current-state warning (2026-07-19 13:27):** the implementation and visual
> status in §0.5-§0.7 were superseded by two failed compositor experiments.
> New agents must start with
> [`water_seam_terrain_vs_static_next_handoff.md`](water_seam_terrain_vs_static_next_handoff.md),
> then use this longer document only for the evidence archive.

**Last updated 2026-07-19 (third update: IMPLEMENTED, VISUAL CONFIRMATION
PENDING).** Supersedes the
2026-07-11 version of this file (which was written into a session scratchpad,
nearly lost, and recovered from a transcript on 2026-07-18 — write handoffs
*here*, never to a scratchpad).

Two prior sessions have burned confident hypotheses on this artifact. **Read
§0 first — both problems are now diagnosed with hard evidence**; the "ruled
out" section and the old leads below are kept for history.

---

## 0. DIAGNOSIS (2026-07-19) — read this, the rest is history

### 0.1 Problem B (the strokes) — SOLVED: they are the sprite art of tile 0x1796

The seam is built in the map data like this (verified by decoding `map0.mul` /
`statics0.mul` at the real capture site, a north–south coast at
**x ≈ 5786-5796, y ≈ 3038-3077, map0** — found via `session_state.toml`
`last_p` + a wide scan for water-land tiles adjacent to water statics):

- interior static water: `0x1797-0x179C` @ z=−5 over *sunken dry* land
  (`0x0064` "forest" @ z=−15);
- **the outermost seam column is always tile `0x1796` @ z=−5, placed over
  coplanar wet land (`0x00A8-0x00AB` @ z=−5)** — one per row along the coast;
- at staircase steps there are dry `0x0064` @ z=−5 transition cells.

Decoding `art.mul`: **0x1796 is not a water diamond. It is a hollow "V" —
just the two upper diamond edges, 2 texels wide, 86 opaque texels** (vs 1012
for the full diamonds 0x1797-0x179C). §2.1's mask verification covered
0x1797-0x179A and friends but **never 0x1796**. Companion accent sprites at
the seam are partial too: `0x17AD`/`0x17AE` (single 109-texel diagonal leg),
`0x17A5` (216-texel half-leg). These are the classic client's shoreline "wave
edge" accents; in the original CC they blend into equally-lit water and read
as subtle wave lines.

The flat-quad path renders 0x1796 faithfully: terrain water shows through the
transparent interior (so the seam cell *looks* like land water), and only the
thin V outline paints — dark, because of problem A. Screen-space verification
(zoom ≈ 1.01, `ORTHO_WIDTH_SCALE_FACTOR = 1.79` vertical stretch):

- +X cell edge projects at atan2(0.408·1.79, 0.707) = **45.94°**; measured
  stroke angle across 9 strokes: **45.9° ± 0.5**. +Z edge → 134.1°; the one
  mirrored stroke measured 134.2°.
- One cell edge ≈ 31.7 px; each V leg is exactly one cell edge long.
- Stroke pixel colour equals the static-water colour ([0,12,26] vs static
  median [0,12,29]) — they *are* art-pipeline water pixels.
- The NE (+X) leg of the V juts against light terrain water → the visible
  stroke. The NW (+Z) leg lies on the static/land colour boundary → the faint
  1-px dark fringe also present in the capture.
- A stroke is prominent only where the seam cell's north neighbour `(X, Z−1)`
  renders light (i.e. at staircase steps); elsewhere the leg sits dark-on-dark
  → "one stroke per staircase step", spacing 2·e_z in the capture.

This is why every depth/coverage/animation experiment changed nothing: the
strokes are **alpha coverage of a sprite whose bitmap IS the stroke**. There
is no depth bug, no coverage void, no mesh-diagonal artifact here. Lead #1
(v0-v3 diagonal) is dead: that diagonal projects screen-vertical (90°), and
the measured strokes are cell-edge-aligned.

### 0.2 Problem A (colour mismatch) — albedo theory REFUTED; it is shading

Decoded both albedos (§6's recommended first step): land art `0x00A8-0x00AB`
and static art `0x1796-0x179C` are **the same water bitmaps** (median RGB
[0,56,72] vs [0,48-56,72]; 0x1797 is pixel-equivalent to 0x00A8's diamond).
The ~2× on-screen gap is created by the render pipelines, not the source art.

Static analysis of both chains under Classic (profile 0) says the *intended*
terms now match with the uncommitted work: same baked light vector on both
sides (`statics_draw.rs:1680`, `texture_cache/land.rs:1163` — both
`BAKED_GLOBAL_LIGHT`), same `water_flat_wrapped_lambert`, same
ambient/diffuse (via the `_pad2_.x`/`land_diffuse_strength` plumbing), same
per-frame `global_lighting` (`land.rs:262`), KR wash / atmosphere stages
no-op at profile 0, both atlases Rgba8UnormSrgb. Therefore:

1. **First action next session: have the user re-capture with the current
   working tree.** The 2026-07-19 capture may predate the uncommitted
   lighting-match work ("never visually confirmed"); the 2× measurement may
   be stale.
2. If the gap persists, the remaining asymmetric stages are the suspects:
   land-only blur/sharpen and `apply_static_light_decals`; **different grunge
   functions** (land: `apply_texture_shadow_aware_land_grunge` with a
   shadow term; art: `apply_texture_visual_grunge`); `pixel_art_atlas_uv_iq`
   on the art side. Note the measured darkening is **tinted, not scalar**
   (G ratio 12/34 ≈ 0.35 vs B ratio 29/57 ≈ 0.51) — look for a
   tinting/desaturating stage, not the energy term.
3. The §4 red/green debug paint remains the right instrument if runtime
   confirmation is wanted (strokes would paint red = art pipeline).

### 0.3 Consequence for the fix

Fixing (A) largely dissolves (B): the 0x1796/0x17AD/0x17AE accents are
water-coloured pixels drawn over water — with matched shading they
self-camouflage exactly as in the original client. Whether to additionally
suppress or restyle the accent sprites is a design decision for the user.
Everything below this section is prior-session context.

### 0.4 Investigation record after the diagnosis

The implementation was deliberately advanced through falsifiable stages. Keep
this sequence: it explains why the final solution is material composition, not
geometry surgery.

1. A temporary ownership mask rendered static flat water pure red and land
   water pure green, with lighting/texture disabled. It showed continuous
   ownership and no voids or intrusive quads; the strokes disappeared. The
   debug paint was removed. This confirmed a texture/shading artifact.
2. Moving the wet Lambert response per-fragment and synchronizing the light
   direction did not visibly change the capture. Lighting consistency was still
   correct cleanup, but not sufficient to solve the source-pipeline mismatch.
3. Dense 44×44 wet static fills were routed to a 1×1 XZ ground instance using
   live CC terrain-water tile `168`. This dramatically reduced the large dark
   water region and confirmed that terrain water must be the canonical base.
4. Classifying all 44×44 art as dense water was wrong: `0x1796` and companion
   shoreline accents share small extents but are sparse V/leg masks. Alpha
   coverage, not dimensions alone, is the discriminant. Full diamonds have
   1012/1936 opaque pixels; `0x1796` has 86/1936.
5. Keeping sparse accents on the art pipeline with their raw RGB restored the
   dark sawtooth and darkened the full coastline. This was the decisive proof
   that their alpha is useful but their absolute base color cannot remain a
   second independently shaded water material.
6. Replacing sparse accent RGB completely with terrain water removed the seam,
   but also removed the authored foam and land-impact variation. That version
   was conceptually correct but visually incomplete.

### 0.5 Implemented material model (current working tree)

The current implementation uses **canonical base + normalized authored
detail**:

```text
CC wet static placement
  ├─ small art is decoded once and cached by graphic ID
  │    ├─ alpha coverage >= 0.45 and exactly 44×44 -> dense fill
  │    └─ otherwise <= 44×44 -> sparse coastline accent
  ├─ dense fill -> GroundTileInstance sampling terrain water land ID 168
  └─ sparse accent -> SpriteInstance
       ├─ preserve authored alpha mask
       ├─ carry median opaque source RGB (linear) as its reference payload
       └─ shader: terrain RGB × bounded authored luma gain
                  + bounded authored chroma residual
```

The CPU profile is in `statics_collect.rs`:

- transparent padding is ignored;
- an alpha threshold of 128 defines opaque pixels;
- median R/G/B is used rather than the mean so minority bright foam does not
  shift the water baseline upward;
- median sRGB bytes are converted to linear RGB because sampling an sRGB atlas
  gives the shader linear values;
- the result and failed decode (`Option`) are cached per graphic, so each art
  tile is decoded/sorted at most once;
- `SpriteInstance.color_rgba` is reused as the reference payload only when
  `SPRITE_FLAG_CC_CANONICAL_WATER_MASK` is set. It remains the normal instance
  tint everywhere else, so the GPU instance stride stays 128 bytes.

The GPU compositor is in `art/main.wgsl`:

```text
authored_gain = clamp(authored_luma / reference_luma, 0.86, 1.80)
detail_chroma = (authored_chroma - reference_chroma) * 0.35
result = max(terrain_water_rgb * authored_gain + detail_chroma, 0)
```

The conservative lower bound prevents dark V/leg pixels from rebuilding the
Night sawtooth; the upper bound retains foam without letting a few source
pixels blow out. These are named shader constants because they are visual
policy and may need final tuning after screenshots. Art-only grunge remains
disabled for the canonical mask, otherwise it would immediately create a
pipeline-specific seam again. Hue, flat-water lighting, global lighting,
grading, wash, and atmosphere still execute after composition.

The canonical terrain sample uses `fract(world_pos.xz)`, the live land page
atlas/lookup, the current water-animation setting, and the same IQ pixel-art UV
filter used by the land path when linear filtering is enabled. It therefore
tracks the active terrain material rather than a copied/hardcoded color.

Decorative wave sprites larger than 44×44 (notably 45×45 and 89×89) stay on
their authored sprite path. They need their full RGB artwork and are not
mistaken for cell-fill or small coastline-mask tiles.

### 0.6 Why this remains perspective-camera compatible

All routing and sampling decisions are expressed in world XZ space. Dense
fills are real XZ cells; sparse accents are real flat XZ quads; canonical UVs
come from world XZ; and flat quads write rasterized geometric fragment depth.
No step depends on the fixed military projection or on ClassicUO's synthetic
`(x+y) + PriorityZ` sprite depth. Camera projection can therefore change to
perspective without replacing the material model.

ClassicUO remains useful only for intent comparisons. The local reference
checkout and relevant paths are:

- `/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/src/ClassicUO.Client/Game/Scenes/GameSceneDrawingSorting.cs`
- `/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/src/ClassicUO.Client/Game/Map/Chunk.cs`
- `/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/src/ClassicUO.Client/Game/GameObjects/Views/View.cs`
- `/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/src/ClassicUO.Client/Game/GameObjects/Views/LandView.cs`

ClassicUO has no equivalent flat-ground static path; its wet statics are
ordinary ordered sprites. Do not transplant that depth model.

### 0.7 Current validation and remaining visual gate

As of this update:

- `cargo check -p dynamapper` passes;
- focused water-profile tests pass (2/2);
- runtime reached `InGame`, uploaded sprite/ground instances, and reported no
  naga-oil/wgpu shader errors;
- full `cargo test -p dynamapper` and user screenshots across the problematic
  angles/presets remain the final gates after this documentation update.

The next screenshot should answer two separate questions: (1) is the dark
sawtooth still suppressed under Classic + Night, and (2) are the foam/impact
details visible but subordinate to the continuous terrain-water base? Tune
only the three named detail constants if (1) passes but (2) needs adjustment.

---

## 1. The artifact

Preset **Classic + Night**, CC art source, at a diagonal shoreline where CC
"surface-like" static water tiles (flat quads, art pipeline) meet terrain water
(land pipeline).

Two distinct problems, which must not be conflated:

**(A) Colour mismatch.** Static water renders markedly darker than land water.
Measured from the user's capture — static median RGB `[0,12,29]`, land median
`[3,34,57]`; roughly a 2× luminance gap. In the clean screenshot the luma
histogram is cleanly bimodal (dark population peaks ~34-49, light ~85-107).
The user wants these to match.

**(B) Dark strokes at the seam.** Thin, straight, dark strokes emanating from
the static/land boundary **into the land-water region**, roughly one per
staircase step, all mutually parallel and consistently angled. This is what the
user calls the "sawtooth"/ticks. **This is the open problem.**

Because of (A), the tile-quantized boundary staircase between the two water
types is plainly visible. That staircase is *expected geometry* (statics are
placed per tile) — it only reads as an artifact because the two surfaces differ
in colour. Do not chase the staircase itself; chase (A) and (B).

Reference captures:
- `/home/claudio/Immagini/Schermate/Schermata_20260719_000527.png` — **best
  data**: clean PNG, water animation OFF, strokes clearly visible
- `/home/claudio/Video/Screencast/Screencast_20260718_234546.webm` — 243×204,
  8.4 s, VP9; strokes visible but colour-distorted by compression
- Older: `Schermata_20260710_190929.png`, `Schermata_20260711_073334.png`

**You cannot screenshot yourself** — `DISPLAY=:1` returns black / hangs. The
user is the visual oracle. Reason from code and from the captures above.

---

## 2. RULED OUT — do not re-derive, do not re-propose

### 2.1 Coverage gaps / T-junctions / float-ulp mismatch — ruled out numerically

- Adjacent flat quads do **not** share an edge. Each quad's bbox is a
  45°-rotated square of **twice the cell area**, with corners at the *centres*
  of the four edge-adjacent cells. They overlap by a full cell; a T-junction is
  structurally impossible.
- The diamond↔cell mapping is **bit-exact**: sweeping every tile column
  X ∈ [0,7200), `max |diamond_tip.x − (X+1)| = 0.0`. Verified by evaluating the
  WGSL operation order in f32 (`0.70710678118` and Rust `FRAC_1_SQRT_2` round to
  the same bits, `0x3F3504F3`).
- Sampler is **nearest**, ClampToEdge, single mip (`core.rs`,
  `ImagePlugin::default_nearest()`). No bilinear filtering, no mip bleed.
- Sprite alpha is strictly **binary**. The 44×44 water masks (`0x1797`,
  `0x1798`, `0x1799`, `0x179A`, `0x346E`, `0x3494`, `0x55F0`) decode from real
  `art.mul` to 1012 opaque texels matching the exact diamond predicate
  `|px−22| + |py−22| ≤ 22` with **0 mismatches**. Boundary texels are
  *double*-covered — the tiling is overlap-biased, never gap-biased.
- Atlas gutter is 1 texel; edge extrusion is applied to Land art only
  (`tex_art_cc.rs`, `filtering_ready = matches!(tile.kind, ArtTileKind::Land)`),
  but with a nearest sampler this is harmless.

### 2.2 Terrain corners poking through the flat quad ("sand wedges") — REFUTED

A prior subagent argued at length that the strokes were land corners winning
the depth test over the constant-Y water plane, exposing near-vertical **sand**
faces, and backed it with a real `map0.mul`/`statics0.mul` scan (6072 alleged
poke-through cases, all water-static-over-sand). **It is wrong.** Disproven by:

1. **Depth probe.** A temporary `out.depth = in.position.z + 2.0e-3` on the
   flat-quad branch — ~69 UO z units of headroom, where 1 z unit ≈ 2.9e-5 NDC —
   changed nothing. With that much margin no terrain geometry can beat the quad.
2. **Colour.** Stroke pixels measure `R = 0`, pure blue-black. Sand is brown
   (R > G > B).
3. **The user confirms it directly**: "it's not sand. it's the seam between
   static and land water."

Do not resurrect this. The map-data scan was real but the conclusion drawn
from it was not what is on screen.

### 2.3 Water-animation coverage overspill — REFUTED

`apply_water_animation` divides UV by `scale = (1.3 + sin·0.28, 1.3 + cos·0.14)`
(x ∈ [1.02,1.58]). Substituted into the diamond predicate, the opaque footprint
becomes `|u−0.5|/sx + |v−0.5|/sy ≤ 0.5` — the cell **dilated by up to 58%**,
spilling into neighbouring cells. The maths is correct and the overspill is
real. **But it is not this artifact**: the user toggled "Water Animation" off
(live UI control, land shader dialog) and **the strokes persist unchanged**.

The overspill remains a genuine latent defect worth fixing separately — it
paints static water up to ~0.15-0.29 of a cell outside its own tile.

(Note: an even earlier session also listed "skip the zoom on flat quads — did
nothing" as a dead end. That is now independently corroborated.)

---

## 3. Established facts (verified this session)

**Flat quad geometry.** With `corner ∈ {(0,0),(1,0),(0,1),(1,1)}` and
`local = mix(inst.local_min, inst.local_max, corner)`, the transform reduces to
`world.x = X + u + v − 0.5`, `world.z = Z + v − u + 0.5`. Diamond tips
`(0.5,0),(1,0.5),(0,0.5),(0.5,1)` map to cell corners `(X,Z),(X+1,Z),(X,Z+1),
(X+1,Z+1)` — the opaque diamond exactly covers cell `[X,X+1]×[Z,Z+1]`.

**Land geometry.** Axis-aligned unit grid (`land/setup_base_mesh.rs`), displaced
per vertex from the height atlas; `tile_height = f32(z_i32) * 0.1`. Each cell
has **four independent corner heights**, split into two triangles along the
`v0(X,Z)–v3(X+1,Z+1)` diagonal (`[v0, v3, v1, v0, v2, v3]`).

**Render state.**

| | LAND | ART opaque (flat water) |
|---|---|---|
| phase | `Opaque3d` (first) | `AlphaMask3d` (second, same pass, shared depth) |
| material | `StandardMaterial`, unlit | `AlphaMode::Mask(0.5)`, `cull_mode: None`, unlit |
| depth write / compare | on / `GreaterEqual` (reverse-Z) | on / `GreaterEqual` |
| writes `frag_depth` | no | **yes** — `out.depth = in.position.z` for flat quads |

**Depth scale.** Ortho, near/far ±1000, camera offset (5,5,5) → a world-Y offset
`d` maps to `d/(√3·2000)`. So `STATIC_ART_Y_BIAS = 0.002` → **5.8e-7 NDC ≈ 10
ulp of Depth32Float** — right at the precision floor. One UO z unit (0.1 world)
→ 2.9e-5 NDC, ~50× the bias.

**Draw order.** Within a chunk, stable sort on
`(tile_x + tile_y) + (127 + priority_z)*0.01 + depth_class_offset + tie*STEP`.
Adjacent water tiles differ by 1.0 in `tile_x+tile_y`, so intra-chunk order is
deterministic. **Between** chunks: `AlphaMask3d` is a *binned* phase in Bevy
0.18, so batch order follows render-world entity iteration and **can change
between frames** as chunks enter/leave view.

**Latent defect, unrelated to the visible artifact.** The 20 small waves
`0x1FA3-0x1FB6` lack the Background flag → classify `SurfaceLikeFloor` → get
`GROUND_ART_Y_BIAS = 0.0` → exactly coplanar with terrain → guaranteed
z-fighting. Only 1 instance exists in all of `map0` (2.8 M statics), so it is
not what the user is seeing, but it is a real bug. `statics_collect.rs`,
`resolve_static_depth_class` / `depth_class_y_bias`.

---

## 4. START HERE — the diagnostic that settles it

Every hypothesis so far died because nobody established **what the strokes
actually are**. Settle that first, in one screenshot, before theorising.

Add a temporary debug mode that paints, with **no lighting, no animation, no
texture**:
- static flat-quad water → flat pure **red**
- land water → flat pure **green**
- everything else unchanged

Then have the user capture the same shoreline. The strokes resolve immediately:

| strokes appear | meaning |
|---|---|
| **green** inside the red region | land water winning through the static layer |
| **red** inside the green region | static quads drawing outside their cell |
| **black / background** | a true coverage void — neither surface draws |
| **gone entirely** | the strokes are a *shading* artifact, not geometry/depth |

That last outcome is live and would redirect the whole investigation — which is
why this must precede any fix attempt.

---

## 5. Leads, ranked (all UNVERIFIED)

1. **Land mesh triangle diagonal.** The strokes are regularly spaced at roughly
   one per tile and mutually parallel — the signature of a per-cell geometric
   feature. The land mesh splits each cell along the `v0–v3` diagonal
   (`setup_base_mesh.rs`). A shading or interpolation discontinuity across that
   diagonal produces exactly one line per cell. **Strongest lead.** Measure the
   stroke angle in `Schermata_20260719_000527.png` and compare it against the
   screen-space direction of the `v0–v3` diagonal — that is a cheap, decisive
   check and it was *not* completed before this handoff was written.
2. **Near-coplanar z-fighting at the shared boundary.** `STATIC_ART_Y_BIAS`
   buys only ~10 ulp. Note the depth probe raised only the **art** side, so if
   the strokes live in the land region it would not have moved them — the probe
   does *not* rule this out. But the strokes' apparent frame-to-frame variation
   in the video is confounded by camera motion and is **not** established as
   true temporal instability. Verify with a stationary camera first.
3. **Inter-chunk batch order instability** (`AlphaMask3d` binned phase). Would
   predict flicker that changes with camera position, not a stable pattern.
4. **Land-side wet-tile branch.** `land/main.wgsl` gives wet tiles
   `uv_b.x = water_flat_wrapped_lambert(...)`; check whether the first row of
   land-water tiles adjacent to statics is treated differently (neighbour
   height reads, atlas edge handling).

---

## 6. The colour mismatch (problem A) — likely not purely a shader issue

Static water and land water are **different source artwork**: statics come from
`art.mul` (`0x1797`, `0x17A4`, `0x346E`…), land water from land art / texmaps
(`0x00A8-0x00AB`, `0x0136-0x0137`). Even with byte-identical shading they
differ by whatever the two bitmaps differ by, and the measured gap (~2×
luminance) is larger than a shading-term discrepancy usually explains.

**Recommended first step: decode both albedos from the client files and compare
them directly** before touching shader maths. If the source art already differs
by ~2×, the fix is a content/normalisation decision (which one is canonical?),
not a lighting tweak — and that decision belongs to the user.

The current uncommitted work already routes both paths through the same
`water_flat_wrapped_lambert` term with the same `land_diffuse_strength` uniform,
so the *lighting* is already nominally matched. That it still doesn't match is
evidence for the albedo theory.

---

## 7. Hard constraints

- **Do NOT port ClassicUO's depth model.** The user ruled this out on
  2026-07-18: CUO gives every sprite a single constant depth from the synthetic
  key `(x+y) + (127+PriorityZ)*0.01`, which makes land-vs-static occlusion a
  per-tile decision and structurally prevents this bug class — but it is a 2D
  sprite ordering scheme, and **UODynamapper intends to support a perspective
  camera**. Keep true geometric depth. Use CUO as a reference for ordering
  *intent* (e.g. land is biased 2 z units below same-cell statics), not as a
  model to port.
- Per `ENGINEERING.md`: minimal surgical diffs; named constants, no magic
  numbers; do not run `cargo fmt` / `rustfmt` / `cargo clippy`; never pass
  `--target-dir`; do not remove existing comments; do not opportunistically
  refactor.
- **Do not absorb the user's unrelated uncommitted edits** (`.cargo/config.toml`,
  settings TOMLs, tools/, scripts/). Stage only files relevant to the batch.

## 8. Working tree state

The complete water-seam implementation remains uncommitted pending the visual
gate. Relevant files are:

- `art/statics_collect.rs`: content profiling, routing, instance payloads;
- `art/statics_draw.rs`: live land-atlas bindings and matching light uniforms;
- `art/art_bindings.wgsl`, `art/main.wgsl`: sprite payload/bindings and detail
  compositor;
- `art/art_ground_bindings.wgsl`, `art/ground.wgsl`: canonical dense-fill
  ground material;
- `common_bindings.wgsl`, `effects/surface_effects.wgsl`, `land/main.wgsl`:
  shared flat-water light response.

The user-owned `.cargo/config.toml` and `assets/settings/runtime_assets.toml`
changes are unrelated and must not be staged or reverted. Re-check status
before any phase commit because running the GUI can save user settings.

The 2026-07-18 depth probe was **reverted**; no diagnostic code remains.

## 9. Validation loop

- `cargo check -p dynamapper` must pass.
- **WGSL is only validated at runtime by naga-oil:**
  `export DISPLAY=:1; timeout 40 cargo run -p dynamapper >/tmp/w.out 2>&1;
  grep -c "failed to create shader module" /tmp/w.out` → must be **0**.
  (WGSL-only edits skip the Rust rebuild, so shader iterations are fast.)
- `cargo test -p dynamapper` must stay **93/93** with the current three added
  water-routing/profile tests.
- Test all three presets (Classic / Enhanced / KR) per `dynamapper/CLAUDE.md`.

## 10. Prior context

- `docs/agent_notes/land_shader_and_placement_issues.md` (issues A-H)
- `docs/agent_notes/water_animation_wave_tile_handoff.md`
- `docs/agent_notes/flat_quad_water_wave_followups_handoff.md`
- `docs/agent_notes/wgsl_uniform_layout_gotcha.md` — naga-oil breaks on a
  trailing `scalar + vec3` in a shared uniform struct; use vec4 slots or match
  an existing pad's offset
- ClassicUO reference: `/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/` —
  `Game/Scenes/GameSceneDrawingSorting.cs`, `Game/GameObjects/Views/`,
  `Game/Map/Chunk.cs` (`AddGameObject` = the PriorityZ table).
  Note CUO has **no flat-on-ground static path at all** — wet statics are
  ordinary upright billboards. Our flat-quad path is a deliberate divergence
  and is where this bug class lives.
