# Resume handoff: CC water statics must preserve authored variants

**Prepared 2026-07-19 13:27 and updated after the 17:31 visual rejection for a
fresh GPT-5.6-sol instance.** This is the current operational handoff. Read
[`water_seam_terrain_vs_static_handoff.md`](water_seam_terrain_vs_static_handoff.md)
after this file for the full evidence archive; its §0.5-§0.7 describe an older
intermediate implementation and are not current.

## 0. Current resumed investigation

### 0.-1 Status update, 2026-07-19 ~17:50: rollback done, diagnostic armed

The §0.0 instruction has been executed:

- All four second-iteration edits are removed:
  `flat_water_mapping.wgsl` deleted; `art/main.wgsl` restored to
  diamond-art-UV animation + IQ `textureSample` (the `is_cc_flat_water`
  definition returned to the shading branch); `land/sampling.wgsl` keeps a
  local `classic_flat_art_uv_from_world_cell_uv` helper again; the module was
  dropped from the `land.rs` composition-test dependencies. The compact
  land-art atlas (first iteration, the `170546` baseline) is intact.
- The temporary runtime selector is in place, gated by
  `const WATER_PATH_DIAGNOSTIC: u32 = 1u` in BOTH
  `land/main.wgsl` and `art/main.wgsl` (set both to `0u` to disable; remove
  entirely before commit). Legend, all unlit flat colors:
  - terrain (IsWet only): **green** = compact land-art path live;
    **cyan** = wet but `texture_size != 4` (not CC page atlas);
    **magenta** = wet CC but no compact land-art slot (absent art id,
    non-identity texmap mapping, or over-capacity);
    **yellow** = wet CC with slot but four-corner flatness guard failed;
    water-looking terrain with NO tint is not IsWet at all.
  - statics (flat CC water only, discards still apply): **red** = cell-sized
    44px diamond quad; **blue** = non-cell-sized (45/89px waves).
- Validation: `cargo check -p dynamapper` clean; 93/93 tests pass including
  the naga-oil composition test; 60 s live desktop/Vulkan run reached
  `InGame` at the coast with Classic art and no shader/pipeline errors.
- Noted for the later parity analysis: the terrain land-art path samples
  `sample_classic_land_art(uv_in_tile, …)` with `uv_in_tile` already
  water-animated in the axis-aligned cell basis (land/main.wgsl), while the
  restored static path animates in the rotated diamond-art basis — the known
  non-commuting pair. Do not re-attempt alignment until the capture proves
  live ownership.
- Next action: user captures the real coast (session already starts at
  [5722, 2991]) under `classic.night` with water animation OFF, then the
  colors decide which premise in §0.0 holds.

### 0.-2 Diagnostic capture result, 2026-07-19 19:49: ownership PROVEN

Capture `/home/claudio/Immagini/Schermate/Schermata_20260719_194926.png`
(diagnostic on, animation off) settles §0.0's open question:

- The entire large water field renders **red** — cell-sized flat CC static
  quads, everywhere.
- The open-water side renders **green** — compact land-art terrain with ALL
  eligibility guards passed. The red/green boundary is exactly the familiar
  hard staircase.
- Shore edge cells render **blue** (45/89px wave overlays), plus thin blue
  dashes over both fields.
- **No yellow, no magenta, no cyan, and no untinted water anywhere at the
  site.** There is no texmap fallback: every wet terrain pixel near the coast
  is live on the compact land-art path.

So §0.0's suspected falsified premise is itself disproven: the smooth terrain
beside the statics IS the compact land-art sample. Both sides use their
intended live sources; the 170546 residual mismatch is a stage asymmetry
between two live paths.

Retrospective mechanism for the 173158 chevron regression (recorded so it is
not re-attempted the same way): the second iteration derived
`world_cell_uv = clamp(world_pos.xz - (tile_x, tile_y), 0, 1)`, but a flat
quad is a 45°-rotated square whose corners sit at the four neighbor-cell
centers — half of each quad's area lies OUTSIDE its central cell. On those
overlap flaps the clamp saturated, mapping to opaque diamond-edge texels
instead of the transparent bitmap corners, so the flaps stopped discarding and
overdrew the neighbors' correct pixels with repeated edge colors → the
per-cell chevrons, and the static field bled ~half a cell outward → the
enlarged staircase. The cell-basis-animation premise was NOT falsified; the
derived-UV implementation broke discard geometry. Any retry must keep
coverage/discard keyed to the authored interpolated `in.uv` and use an
unclamped, continuous cell-basis transform only for the sampled color.

Lighting parity re-verified statically: land classic wet cells shade as
`shade_mode0_classic_vertex(albedo, water_flat_wrapped_lambert(L), ambient,
diffuse)` = `albedo * (ambient + diffuse * lam)`, the identical equation and
shared helper used by the art flat-water branch.

Next bisect (diagnostic consts flipped back to `0u`, code still in place):
user captures the same coast at the restored baseline, `classic.night`,
water animation OFF. If the seam/color mismatch disappears without animation,
the divergence is the animation-basis asymmetry (terrain animates in the
axis-aligned cell basis before the diamond mapping; statics animate in the
rotated art basis) and the fix is a corrected cell-basis static animation per
the discard-safe rules above. If the mismatch persists, proceed to Phase B
stage-checkpoint comparison (sampling/filter conventions, atlas decode, or a
stage not actually disabled under classic.night).

### 0.-3 Baseline animation-off captures, 2026-07-19 20:00: animation exonerated

Captures at the restored baseline, animation OFF:

- `Schermata_20260719_200007.png` (classic.night): "way better", but the
  terrain/static tonal gap persists (terrain water grayer/smoother), plus
  light bands in the terrain-water area and one stray wave sprite in open
  water.
- `Schermata_20260719_200039.png` (classic.morning): the same boundary is far
  subtler — the divergence scales with the preset.

Animation is therefore NOT the residual cause. Static audit of every named
stage under the classic profile (shading_mode 0, post_process_profile 0):

- atmosphere depth: both land and art versions early-return unless
  `visual_profile == 2u` — no-op in classic. Exonerated.
- fog/gloom: `enable_fog = 0`, `enable_gloom = 0` in classic.night/morning
  presets; gloom only feeds modes 1/2 anyway. Exonerated.
- grunge: OFF at night (where the gap is larger), ON at morning — wrong sign
  to explain the gap. Exonerated as the driver.
- `light_color` ([0.8,0.88,1.05] at night): consumed only by shade modes 1/2
  and by `apply_art_surface_shading` (which flat water skips). Exonerated.
- `in.color` instance tint: `color_rgba = [1,1,1,1]` for sprites. Exonerated.
- land cheap-path/blur/sharpen: the land-art override reassigns `base_albedo`
  after them, so eligible wet cells are unaffected. Exonerated.
- lighting equation and inputs: identical formula and shared helper;
  statics_draw copies `land_lighting.diffuse_strength` into the art uniform's
  `land_diffuse_strength` slot, both sides read the same `uniform_state`.
- grading off; tonemap on with same flags/exposure via copied uniform.

Remaining suspects, in current order:

1. the sampled albedo itself — art: `pixel_art_atlas_uv_iq` + sampler over the
   sprite atlas vs terrain: integer `textureLoad` over the one-off compact
   atlas (`enable_linear_filtering = 0` in all classic presets); includes
   possible decode/content differences between the two atlas builders and
   authored variant differences (0x1798-0x179C vs land 169-171 art);
2. transparent-pass `shaded.rgb * a * 2.0` premultiply, if flat water rides
   the transparent pass rather than the opaque one;
3. art-side uniform sync staleness (statics_draw change-detection gates);
4. `apply_static_light_decals` (land-only; unlikely on open sea).

The ownership diagnostic was therefore extended into checkpoint modes
(`WATER_PATH_DIAGNOSTIC` in both shaders, kept in sync): 0 off, 1 ownership
colors, 2 raw sampled albedo only, 3 albedo × classic flat-water energy.
Both consts are currently set to mode 2. A boundary visible at mode 2 means
the sources/sampling differ; first visible at mode 3 means lighting inputs
differ; only at mode 0 means a later stage. Validated by a clean 45 s live
run. Next: user captures night + animation off at mode 2, then mode 3.

**Mode-2 first captures (20:19/20:12) were contaminated by a checkpoint bug.**
`201935` (no animation) and `201234` (animation) showed the static field as a
black checker/diamond lattice. Cause: modes 2/3 forced `alpha = 1` in the
TRANSPARENT pass too, where the surviving fragments are precisely the
transparent texels — including each flat quad's corner flaps — so overlapping
quads painted each other solid black. Fixed: modes 2/3 now `discard` outside
the opaque pass. Mode 0 rendering was never affected. Valid readings from the
contaminated captures:

- the user confirms NO preset dependence of water color at mode 2 — the
  checkpoint bypass works on both paths;
- flat-water quads are enqueued in BOTH the opaque and transparent passes
  (the black could only come from the transparent pass), and for binary-alpha
  water art the transparent pass contributes nothing at mode 0
  (`rgb * a * 2.0` with a ≈ 0) — suspect 2 (premultiply) is effectively
  exonerated for the tonal gap;
- the terrain-side checkpoint renders clean raw albedo.

Compact-atlas builder audit: it copies raw RGBA rows from the same
`read_page_rgba` decode the sprite cache uses, into the same
`Rgba8UnormSrgb` format, no per-pixel transform — decode parity holds
statically. If the clean mode-2 capture still shows a boundary, the remaining
candidates are the filter convention (`pixel_art_atlas_uv_iq` + sampler vs
integer `textureLoad`) and genuinely different authored pixels
(static 0x1798-0x179C vs land 169-171 art; archive medians differ slightly:
G 48-56 vs 56).

### 0.-4 Mode-2 fixed capture, 2026-07-20: albedo parity confirmed

After the transparent-pass discard fix (§0.-3), the user recaptured mode 2
(night vs. morning vs. other presets, animation on and off): the terrain and
static water regions are now visually identical, with no artifacts. This is
the expected mode-2 result — the checkpoint deliberately bypasses the entire
lighting stage on both paths, so preset-independence and the lack of light
response are correct diagnostic behavior, not a new bug. **Conclusion: raw
sampled albedo already matches between the two paths.** Suspect 1 (sampling/
filter convention, atlas decode) is resolved — the compact land-art atlas and
the sprite atlas agree pixel-for-pixel at the checkpoint. Do not re-open the
filter-convention question without new evidence.

Since the lighting formula/inputs were already statically confirmed identical
(§0.-3) and albedo now empirically matches, mode 3 (albedo × lighting energy,
still skipping post) is very likely to also match and is optional to capture.
**Immediate next action: turn the diagnostic OFF** — set `WATER_PATH_DIAGNOSTIC
= 0u` in both `art/main.wgsl` and `land/main.wgsl` — and have the user
recapture the real coast across all three presets (Classic/Enhanced/KR per
dynamapper/CLAUDE.md) with normal rendering restored, to confirm color parity
survives lighting/postprocess and that light response returns. If that gate
passes, remove all `WATER_PATH_DIAGNOSTIC` code (both consts, both checkpoint
branches, the mode-1 ownership branch, and the now-unneeded diagnostic
comments) before the phase commit — it must not ship, per ENGINEERING.md's
no-clutter rule and this handoff's repeated "remove diagnostics before commit"
instruction.

### 0.-5 User questions answered, 2026-07-20 — read before extending scope

**Q: why doesn't water respond to light/presets?** Answered above (§0.-4): a
diagnostic artifact, not a defect. Resolved once the diagnostic is removed and
re-verified.

**Q: tilemeta flags or raw IDs?** Hybrid, by design:
- *Wetness* is the tiledata `IsWet` flag (`TILEMETA_FLAG_WET = 0x80`,
  `land.rs:466`) — universal, works for any land tile ID.
- *Which art.mul image to substitute* uses the land tile's own numeric ID
  directly (`classic_land_art_payload_id` returns `tile.tile_id` as the
  `art_id`) — this is standard CC layout, not an ID hack: land tiles 0-0x3FFF
  own a 44×44 diamond at that same ID in `art.mul`, a distinct slot range from
  statics (0x4000+).
- *Eligibility* is the one real ID-comparison guard:
  `texture_id == 0 || texture_id == tile.tile_id`, reading tiledata's own
  `texture_id` field (the field the ordinary texmap terrain path already uses
  to pick a texmaps.mul slot — see §0.1's tile-168 example) to confirm the
  texmap the renderer would otherwise show is the SAME graphic as the land
  tile's own ID before substituting. Sourced from tiledata, not a hardcoded ID
  list — this is what keeps `create_classic_land_art_images` general instead
  of tile-168-specific.

**Q: will this work with Enhanced Client textures?** No — currently Classic-only
by construction, gated on BOTH sides:
- Terrain: `use_classic_flat_wet_land_art` requires `tile.texture_size == 4u`
  (CC page-atlas format). EC land is `texture_size == 2u` and takes the
  unrelated EC world-space/two-base-blend/liquid-normal path
  (`land/main.wgsl` texture_size==2 branches), completely untouched by this
  investigation.
- Statics: flat-quad classification requires
  `art_source == ClientTextureSource::Cc`
  (`statics_collect.rs:2129`, `cc_surface_like_flat`). EC statics never
  become flat quads and render exactly as before this investigation.
- Extending to EC is explicitly OUT OF SCOPE for the current fix and would be
  new work: EC water textures tile across multiple world cells via a stretch
  factor rather than mapping one diamond per cell (see the
  `ec_world_uv`/`texture_size == 2u` comments in `land/main.wgsl`), so the
  flat-quad-per-cell geometry model does not carry over directly. Decide
  deliberately later; do not fold silently into this fix.

**Q: transparent/translucent water compatibility?** NOT yet handled — this is
a real, separate defect surfaced by the user's capture
`/home/claudio/Immagini/Schermate/1768955859790.png`: a partially-transparent
CC water static (murky/see-through shallow water showing rocks and the seabed
through it) instead renders as a solid-looking flat overlay, losing the
authored partial alpha / see-through quality. This must be investigated and
fixed — across BOTH Classic and Enhanced/KR textures and view modes — as its
own follow-up phase after color-parity closes and the diagnostic is removed.
Not yet root-caused; likely candidates to check first: whether flat-quad
alpha reaches the blend correctly (the `shaded.rgb * shaded.a * 2.0`
transparent-pass premultiply noted in §0.-3, `alpha_cutoff` routing between
opaque/transparent pass_mode for partial-alpha texels, and whether the
flat-water shading branch's `shaded = vec4(shaded.rgb * flat_water_energy,
shaded.a)` preserves the source alpha through to the framebuffer blend).

Open user-reported issues tracked for after color parity:

- light bands in the terrain-water area at night (may be the wave overlays
  rendered lighter than tinted water, or a sampling artifact — mode 2 will
  tell);
- one stray static wave sprite in open water (the ownership capture showed
  many non-cell-sized wave quads (blue dashes) scattered across open water;
  compare against ClassicUO/map data — possibly an animation-frame static the
  CC client cycles or hides, or a draw-order difference against coplanar
  fills).

### 0.-6 Diagnostic flipped OFF, 2026-07-20: awaiting the final visual gate

`WATER_PATH_DIAGNOSTIC` is now `0u` in both
`dynamapper/assets/shaders/world/art/main.wgsl` (line 39) and
`dynamapper/assets/shaders/world/land/main.wgsl` (line 89). Normal rendering
is restored end-to-end: full lighting, hue, grunge, grading, tonemap, and
every other stage run unconditionally again, on both the terrain compact
land-art path and the static flat-quad path. Validated:

- `cargo check -p dynamapper` clean;
- `cargo test -p dynamapper`: 93/93 passed, including the naga-oil
  `land_sampling_shader_composes` composition test (unaffected by the
  diagnostic, which lives in `land/main.wgsl` / `art/main.wgsl`, not
  `land/sampling.wgsl`);
- a 45 s live desktop/Vulkan run reached steady state with no
  shader-module/pipeline/binding errors.

**No visual capture has been taken yet at this state.** The diagnostic code
itself is still present in both files (dead when the const is `0u`, but not
yet deleted) — this is intentional per ENGINEERING.md/this handoff's
established pattern of proving the fix live before doing cleanup, not a step
that was forgotten.

**Immediate next action for whoever resumes:** ask the user to capture the
same real coast (`[5722, 2991]`) across Classic (morning + night), Enhanced
Classic, and KR-like — per `dynamapper/CLAUDE.md`'s "test all three rendering
presets" rule — with the diagnostic off. Confirm two things simultaneously:
(a) the terrain/static color parity from §0.-4 survives with full
lighting/postprocess back on, and (b) water now visibly responds to light
direction/preset again (the §0.-5 diagnostic-artifact explanation predicts
this; it has not yet been empirically confirmed in this state).

**If that gate passes:** delete all diagnostic code before any commit — do
not just leave the const at `0u`, per ENGINEERING.md's no-clutter rule and
this handoff's repeated instruction. Exact removal list (line numbers as of
this writing, re-grep `WATER_PATH_DIAGNOSTIC` and `TEMPORARY DIAGNOSTIC` to
confirm current positions before editing):
- `art/main.wgsl`: the `WATER_PATH_DIAGNOSTIC` / `_CELL_DIAGONAL` const block
  and its comment (~lines 27-42), and the whole
  `if (WATER_PATH_DIAGNOSTIC != 0u && …) { … }` block inside `fragment()`
  (~lines 291-320, sits right after the opaque/transparent alpha-cutoff
  discard and before `var shaded = color * in.color;`).
- `land/main.wgsl`: the const block and its comment (~lines 73-89), the
  ownership-mode `if (WATER_PATH_DIAGNOSTIC == 1u …) { … }` block right after
  `use_classic_flat_wet_land_art` is computed (~lines 611-624), and the
  checkpoint-mode `if (WATER_PATH_DIAGNOSTIC == 2u …) / (== 3u …)` blocks
  right after the `sample_classic_land_art` override
  (~lines 729-738).
- After deletion, re-run `cargo check -p dynamapper` and
  `cargo test -p dynamapper` (both must stay clean/93-93), and do one more
  short live run to confirm the shaders still compose without the removed
  branches.

**If the gate fails** (parity breaks once lighting/postprocess is back on, or
light response still doesn't return): do not re-open the sampling/albedo
question — §0.-4 closed that empirically. Instead suspect a stage this
session did NOT audit under full rendering: `apply_kr_color_wash` (called
unconditionally in both shaders regardless of `enable_kr_color_wash`, since
the function itself gates on the flag — verify it actually no-ops under
classic profiles the same way on both paths), `apply_static_light_decals`
(land-only, not mirrored on the art side — check whether it's visibly active
at the capture site), or a real uniform staleness bug in the
`statics_draw.rs` change-detection gate that only manifests once full
lighting is reading live preset values again (§0.-3 confirmed the *copy*
mechanism exists, not that it always re-fires promptly on preset switch).

### 0.-7 Gate FAILED, 2026-07-20: gap reappears with full rendering; grunge implicated

The §0.-6 gate did not pass. With the diagnostic off (full normal rendering
restored) the user reports the terrain/static color difference is back, and
that disabling `enable_grunge` in the UI **minimizes but does not nullify**
it. This directly contradicts the earlier §0.-3 "grunge exonerated" call —
that conclusion was reached by comparing gap SIZE across two different
presets (night vs. morning) with many confounding variables, not by toggling
grunge alone as a single variable at fixed preset. **§0.-3's grunge
exoneration is retracted; treat grunge as an active, not-yet-explained
suspect,** alongside whatever explains the residual that persists even with
grunge off.

Investigation done this session before adding a new checkpoint:

- **Uniform sync is NOT the cause.** Grepped both sync sites:
  `land.rs:259` (`mat.extension.effects_uniform = uniform_state.effects;`)
  and `statics_draw.rs:1682`/`1702` (`update_art_material_uniforms` /
  `_ground`, same literal assignment). Both land and art materials copy
  `enable_grunge`/`grunge_strength` from the exact same `uniform_state.effects`
  source, unconditionally, whenever the change-detection gate
  (`uniforms_changed`, `statics_draw.rs:1578`) fires. No divergence possible
  here for a global toggle — ruled out as an explanation for the grunge
  finding specifically. (Uniform-sync *staleness/lag* on preset switch, listed
  as a suspect in §0.-6, is still separately unconfirmed either way and worth
  a quick check if mode 0 keeps failing after the mode-4 result below.)
- **Under the classic profile, land's and art's grunge call the SAME
  formula.** `apply_texture_shadow_aware_land_grunge` (land) immediately
  delegates to `apply_texture_visual_grunge` (art's function) whenever
  `profile != 2u` (`grunge.wgsl:129-139`) — i.e. for Classic and Enhanced,
  they are litarally the same code, not just visually similar.
- **Application order is mathematically commutative, so it should not matter.**
  Land: `base_albedo → ×grunge_factor(world_xz) → ×lighting_energy(scalar)`
  (grunge in `land/main.wgsl` runs before the "Shade" section that computes
  `hdr_rgb`). Art: `albedo → ×lighting_energy(scalar) → ×grunge_factor(world_xz)`
  (grunge in `art/main.wgsl` runs after the `is_cc_flat_water` energy
  multiply). `grunge_factor` is a `vec3` tint, `lighting_energy` is a plain
  `f32` scalar (`ambient_strength + diffuse_strength * lam`) on both paths —
  `(a*s)*f == (a*f)*s` per-channel, so the two orders are algebraically
  identical GIVEN identical `world_xz`, `profile`, `strength`, and albedo
  inputs. This is why grunge being unconditionally applied "shouldn't" produce
  an asymmetry — and why its involvement is a real anomaly worth chasing, not
  a red herring to dismiss again.
- **Leading hypothesis, unconfirmed:** grunge is a purely spatial (world_xz)
  procedural pattern (`visual_grunge_uv` tiling texture blended with FBM
  noise, `grunge.wgsl:58-95`). The terrain-water region and the static-water
  region are DIFFERENT, non-overlapping physical map areas (open water vs.
  coastal fringe — confirmed by the §0.-2 ownership capture). Different world
  positions naturally sample different points of the same noise field, so
  even a perfectly symmetric grunge implementation produces locally different
  instantaneous tint on each side. If a small residual base-color/lighting
  mismatch still exists beneath the noise (too subtle to see at mode 2/3
  alone), the added local contrast from grunge could make that boundary far
  more perceptually salient without grunge itself being "buggy" — simultaneous
  contrast at a texture-type edge. This would explain "minimized, not
  nullified" by disabling grunge: removing grunge removes the contrast that
  reveals a smaller underlying gap, it doesn't remove the gap's root cause.
  **This is a hypothesis, not a confirmed conclusion — do not accept it
  without the mode-4 evidence below.**

**Additional user discriminator, 2026-07-20:** the broad bright banding visible
in `Schermata_20260719_200007.png` occurs under `classic.night` but not under
`classic.cave`. That capture predates the mode-4 checkpoint and shows normal
mode-0 rendering, so it is evidence about the original artifact but is NOT the
pending mode-4 result. The factory Night and Cave presets both disable grunge,
fog, gloom, grading, fake art normals, and KR color wash; they also use the same
light-decal intensity/visibility. They differ mainly in numeric global light,
ambient/diffuse energy, and exposure. Therefore the preset observation does not
by itself implicate grunge or decals: the bands may be created by a later stage,
or merely cross a visibility threshold under Night's brighter output. Mode 4 is
still the correct split because it stops before land-only static-light decals,
global lighting, and tonemap. Also correct the capture setup below: grunge is
OFF in the factory `classic.night` preset, so `grunge ON` must be enabled
deliberately in the UI rather than assumed to be the default.

**First mode-4 capture set rejected as a setup mismatch, 2026-07-20:**
`classic_cave_w_grunge.png`, `classic_cave_no_grunge.png`,
`classic_night_no_grunge.png`, and `classic_night_w_grunge.png` all visibly
show `Active: kr.cave` or `Active: kr.night` in the F3 panel. The filenames do
not reflect the live shader preset. These cannot answer the Classic checkpoint:
at mode 4, KR terrain goes through `shade_mode2_kr_fragment`, KR material micro
contrast, and `apply_kr_liquid_material_response`, while flat CC water statics
still use the shared Classic ambient + diffuse flat-water energy. The large
terrain/static boundary in those captures is therefore an expected mismatch
between two different diagnostic stages, not evidence about Classic color
parity. Grunge on/off makes no material difference to that KR-mode boundary,
which exonerates grunge only for that invalid KR comparison. The user also
corrected the operational capture location to **map 0, `[1445, 1790, 0]`**;
use that location for all subsequent visual gates instead of the older saved
location mentioned in the archive.

**New checkpoint added: mode 4 = albedo × lighting-energy × grunge, still
skipping color wash / global lighting / atmosphere / grading / tonemap.**
Both `WATER_PATH_DIAGNOSTIC` consts are now set to `4u` (previously `0u`
after §0.-6; the const value itself is the resume marker — if you find it at
`4u`, the mode-4 capture below is what's pending):

- `land/main.wgsl`: returns `vec4(hdr_rgb, 1.0)` for `tile.is_wet == 1u`
  immediately after `hdr_rgb` is first computed by the shading_mode branch
  (right before `apply_static_light_decals`), i.e. after grunge (which runs
  earlier, pre-shade) and after the lighting-energy multiply.
- `art/main.wgsl`: returns `vec4(shaded.rgb, 1.0)` for
  `is_cc_flat_water && pass_mode == PASS_MODE_OPAQUE`, immediately after the
  grunge block (right before `color_wash_shadow` is computed). The transparent
  pass is deliberately left untouched at mode 4 (no discard) since it isn't
  forced to `alpha = 1` this time — the §0.-3 black-lattice bug doesn't apply
  here, so no repeat of that fix was needed.
- Validated: `cargo check` clean, 93/93 tests pass, 45 s live run reached
  steady state with no shader/pipeline errors.

**Next action: user captures mode 4** at map 0 `[1445, 1790, 0]`, after first
selecting the **Classic 2D render style** and confirming the F3 footer reads
`Active: classic.night`
(the preset where the gap read largest), animation off, with grunge deliberately
enabled in the UI (the factory preset has it off). A second capture at the same
checkpoint with grunge disabled gives the mode-3-equivalent comparison for the
known zero-hue/white-instance water at this site without another shader edit.
If practical, repeat the grunge-off checkpoint under `classic.cave` to preserve
the newly reported Night-only banding discriminator, confirming the footer has
changed to `Active: classic.cave`. Read the result as:

- **Gap visible at mode 4** → grunge (or something in how the two paths reach
  it, e.g. a subtle world_xz mismatch between the flat quad's interpolated
  position and the terrain mesh's) is implicated, OR the noise-locality
  hypothesis above is correct and a real (smaller) gap already existed at
  mode 3 that grunge is merely making visible — **capture mode 3 as well**
  (flip both consts to `3u`, no rebuild needed, just relaunk) to disambiguate:
  if mode 3 is ALSO visibly different, the root cause is upstream of grunge
  (lighting energy or a `world_xz`/`world_pos` mismatch) and grunge is
  innocent; if mode 3 is clean and only mode 4 shows a gap, grunge/noise-
  locality is the real story and the fix is either an explicit grunge
  exemption for `is_cc_flat_water`/wet terrain (mirroring how
  `apply_art_surface_shading` is already skipped for flat water) or accepting
  it as inherent texture variation the user may simply want toned down via
  `grunge_strength`.
- **Gap gone at mode 4** → grunge is fully exonerated after all (the user's
  UI toggle observation would then need a different explanation — e.g. toggling
  grunge might correlate with a preset/effects change that also touches
  something else, or the perceived "minimization" was the noise-locality
  effect in reverse: grunge OFF removes texture-edge contrast that was making
  an unrelated, still-present, later-stage gap merely easier to spot). In that
  case move to the untested downstream stages: `apply_static_light_decals`
  (land-only, not mirrored on art — check activity at the capture site),
  `apply_kr_color_wash` (called unconditionally on both paths but gates
  internally on `enable_kr_color_wash`; verify it truly no-ops identically
  under classic on both shaders), global lighting
  (`apply_global_lighting_rgb`, same function/uniform on both — should be
  fine but not yet checkpoint-isolated), atmosphere (already established as
  a no-op outside KR, §0.-3), and grading/tonemap (same functions/uniform on
  both, also not yet checkpoint-isolated — could add mode 5/6 following the
  same pattern if needed).

Do not skip straight to a code fix from a hunch — every prior shortcut in
this investigation (the additive-sRGB compositor, the luma/chroma compositor,
the second UV-alignment iteration) was rejected by the user's eye once
captured. Get the mode-4 (and if needed mode-3) capture first.

### 0.-8 True Classic mode 4 passed; preset bug fixed; normal rendering restored

The user subsequently selected the real Classic render style and reports that
the water is visually good at mode 4 with grunge both enabled and disabled.
This is the missing checkpoint result: sampled albedo, shared Classic flat-water
lighting energy, and grunge all match. Grunge is exonerated. The user's report
that scene brightness, grading, and tonemap did not affect water was an expected
mode-4 diagnostic artifact, not intended production behavior: both water paths
returned before those stages while land and ordinary ground statics continued.

This retest exposed an independent F3 preset defect. Selecting Classic and then
Morning could switch back to KR, and every time-of-day button restored water
animation to ON. Root cause was confirmed in the live cache: the saved
`classic.morning` slot contained `shading_mode = 2`. Render-style selection did
not update `active_preset`, so closing the panel could save one style's live
values into another style's slot; later preset application trusted the corrupted
stored mode/profile. Commit `f3d581a` fixes this as a completed UI phase:

- selecting a render style applies that style's preset for the currently active
  time period and updates the active key;
- style and time buttons enforce their selected mode/profile even if a cached
  slot was previously contaminated;
- both operations preserve the current water-animation toggle rather than
  treating animation playback as a time/style preset value;
- the misleading `Shadow Brightness` label is now `Terrain Ambient Light`,
  matching the actual ambient term that raises all terrain shading, including
  wet terrain.

The checkpoint consts are now back at `WATER_PATH_DIAGNOSTIC = 0u` in both
shaders. Normal rendering again includes static-light decals, color wash,
global lighting, atmosphere, grading, and tonemap. Validation after the UI fix
and diagnostic flip: `cargo check -p dynamapper` clean; all Dynamapper tests
pass; a 30 s live Vulkan run reached `InGame` at map 0 `[1445, 1790, 0]` with
no shader/pipeline error (the timeout exit is expected).

**Next visual gate:** use the patched executable to verify (1) Classic remains
selected across Morning/Night/Cave and animation stays at the user's current
setting; (2) under full normal `classic.night`, whole-scene brightness/tonemap
again affect both water paths and the terrain/static seam remains good; and
(3) under full normal KR, whether the previously captured KR boundary remains.
The earlier KR screenshots were taken at mode 4, where land used the full KR
liquid response but flat CC statics used the Classic checkpoint energy, so they
cannot establish the full-rendering KR defect by themselves. If the boundary
does persist at mode 0, the next phase is mode-aware flat-CC-water shading parity
for Enhanced/KR, not another Classic albedo/grunge investigation.

### 0.-9 KR mode-0 mismatch confirmed; shared flat-water candidate awaiting capture

The user confirmed the preset fix and restored whole-scene response are correct,
and that full normal KR still has both the patterned artifact and a terrain/art
water color step. Static inspection locates the first production divergence:
eligible flat CC terrain went through slope-aware KR grunge, terrain lighting,
relief/liquid response, land-only static-light decals, terrain color-wash shadow,
and land atmosphere, while adjoining flat CC art water used the level-liquid
energy plus art-side wash/atmosphere inputs. This is the same water material with
two geometry owners, so those terrain-vs-art stages are structurally invalid.

The current uncommitted candidate keeps the already accepted Classic behavior
and aligns only the flat CC material under Enhanced/KR:

- eligible flat wet CC terrain uses the same level-water ambient + diffuse
  energy as authored flat CC water statics in every render style; KR styling
  remains in the shared downstream grunge/wash/global/grading/tonemap stages;
- terrain-owned CC flat water uses the same visual grunge as art-owned water and
  skips KR transition grime; it no longer uses seabed normals to create grunge
  deposits on only one side of the boundary;
- Enhanced/KR land-only static-light decals are skipped for that flat-water
  material (Classic retains its accepted existing decal behavior);
- flat art water no longer feeds its per-sprite vertical UV shadow or local-light
  ownership into KR color wash; both owners use shadow/local-light zero;
- a shared flat-water atmosphere helper removes the previous alpha-vs-normal
  fog asymmetry at distance;
- sloped wet CC terrain, non-water terrain/statics, and every EC texture path are
  unchanged.

Validation: `cargo check -p dynamapper` clean; 95/95 Dynamapper tests pass; a
30 s live desktop/Vulkan run loaded the current KR preset, reached `InGame` at
map 0 `[1445, 1790, 0]`, and produced no shader/module/pipeline errors. No new
binding or Rust uniform layout was needed. **Next action:** capture the same
coast under normal `kr.night` with animation off, first grunge off and then on.
Also make one quick `classic.night` regression check. Do not commit the water
phase until both visual gates pass.

### 0.-10 KR mismatch isolated to duplicate fog; single-owner candidate awaiting capture

The user visually accepted both Classic and Enhanced with the §0.-9 candidate.
Under KR, the terrain/art water color difference disappears when Fog is
disabled; toggling Grunge does not affect it. This supersedes the §0.-9
atmosphere hypothesis: grunge is exonerated again, and fog is the remaining KR
discriminator.

Static inspection found two simultaneous fog implementations driven by the
same `enable_fog` flag:

- per-material KR atmosphere functions ran in `land/main.wgsl`,
  `art/main.wgsl`, and `art/ground.wgsl` before grading/tonemap, with different
  terrain-normal, sprite-alpha, and height inputs;
- `effects/fog_pass.wgsl` then fogged the fully composed frame again after
  tonemapping.

This was stale double ownership. Both `effects/fog_pass.wgsl` and
`legacy/world/fog.wgsl` explicitly document that active fog belongs to the
fullscreen pass so land and art receive uniform coverage; the legacy
world-space material helper is intentionally inactive for that reason.

The current uncommitted candidate restores that architecture:

- removed `apply_kr_land_atmosphere_depth`, `apply_art_atmosphere_depth`, and
  the interim `apply_flat_water_atmosphere_depth`, plus every active call/import;
- removed the now-dead land-only volumetric-fog constant and zoom LOD variables;
- retained `FogPostProcessPlugin` and `effects/fog_pass.wgsl` unchanged as the
  sole fog implementation, still controlled by the same preset/UI flag and fog
  parameters;
- added short comments at the land, sprite-art, and ground-art post-lighting
  sites to prevent per-material fog from being reintroduced.

This changes overall fog strength from two layers to the single intended layer,
and prevents fog from creating a material seam because it now samples the
already composed screen color. Classic and Enhanced are otherwise unchanged;
their accepted flat-water/grunge/light behavior remains intact.

Validation: `cargo check -p dynamapper` clean; 95/95 Dynamapper tests pass; a
30 s live desktop/Vulkan run reached `InGame` at map 0 `[1445, 1790, 0]` with
no shader/module/pipeline errors. The timeout exit is expected.

**Next visual gate:** at the same coast, select normal `kr.night`, leave Fog
enabled, and compare with the just-reported Fog-off baseline. Grunge may be in
either state because it has been visually exonerated. Confirm both (1) the
terrain/art water color step is gone and (2) the previous bright patterned
artifact is gone. Then make one quick Classic and Enhanced regression check.
Do not commit or remove the disabled diagnostic blocks until this gate passes.

### 0.-11 Visual gate passed; diagnostics removed; production phase committed

The user confirmed the §0.-10 single-owner fog result works as intended. The
full-rendering water seam is accepted under Classic, Enhanced, and KR; KR with
Fog enabled no longer has the terrain/art color step or the bright patterned
artifact. Grunge remains unrelated to the resolved defect.

Final cleanup and review are complete:

- removed every `WATER_PATH_DIAGNOSTIC` constant, branch, checkpoint return,
  ownership color, and temporary diagnostic comment from the land and art
  shaders;
- retained the accepted compact Classic land-art atlas, shared flat-water
  lighting, KR flat-water parity rules, and fullscreen-only fog ownership;
- removed a test-only static-art routing abstraction whose behavior was
  identical to the established route and therefore added unnecessary
  production complexity;
- removed investigation-specific references from production shader comments;
- preserved the user-owned `.cargo/config.toml` and
  `dynamapper/assets/settings/runtime_assets.toml` changes without staging or
  reverting them.

Final validation after cleanup: `cargo check -p dynamapper` clean;
`cargo test -p dynamapper` passes 94/94; `git diff --check` passes for the
production scope; a diagnostic-free 30 s desktop/Vulkan run reached `InGame`
without shader/module/pipeline errors. The test count decreased from 95 only
because the redundant routing test was removed during review, not because a
coverage failure was skipped.

Production commit: `8475ddc fix(water): eliminate terrain-static render seams`.
The earlier independent preset/UI phase remains commit `f3d581a`.

This terrain/static seam investigation is closed. The translucent/see-through
CC water-static alpha issue and any stray authored wave sprite noted in §0.-5
remain separate follow-up work; neither was folded into this accepted phase.

### 0.0 Stop condition: the second sampling-alignment iteration is rejected

**Latest user verdict, 2026-07-19:** "it's worse."

Rejected capture:

`/home/claudio/Immagini/Schermate/Schermata_20260719_173158.png`

The current renderer is **not acceptable and must not be committed**. Compared
with the earlier `170546` capture, the second iteration produces a severe new
regression:

- the large authored-static water field is covered by repeated high-contrast
  chevron/diamond strokes;
- individual 44×44 cells become conspicuous instead of reading as water;
- the terrain/static boundary becomes a much larger hard staircase;
- the broad color mismatch remains, and the intended shoreline/foam detail is
  overwhelmed by the repeated raw pattern;
- the adjacent terrain region remains comparatively smooth, so the two sides
  plainly are not following the same effective source/sample path.

The regression was introduced by the **second** iteration after `170546`, not
by the earlier compact terrain land-art atlas alone. That iteration:

1. added `assets/shaders/world/flat_water_mapping.wgsl`;
2. changed cell-sized flat CC statics in `art/main.wgsl` to derive art UV from
   world-cell position and to bypass the atlas IQ sampler with direct
   `textureLoad` when nearest filtering is selected;
3. imported the shared transform into `land/sampling.wgsl` instead of keeping
   the local terrain mapping helper;
4. added that module to the naga-oil composition-test dependencies in
   `texture_cache/land.rs`.

Start the next session by **surgically removing only those second-iteration
edits**, returning to the user-confirmed-better `170546` baseline. Do not revert
the whole water investigation, unrelated work, flat XZ geometry, geometric
depth, authored static routing, or the first compact-atlas experiment.

The likely falsified premise is that the visible terrain beside every
cell-sized water static is actually using the new compact land-art sample. The
terrain guard also requires four equal corner heights; the latest image is
consistent with many real coastline cells retaining the smooth texmap path
while statics alone were forced onto raw integer land-art texels. Do not accept
that inference without a runtime selector. After restoring `170546`, add a
temporary diagnostic that distinguishes:

- compact land-art terrain cells;
- fallback texmap terrain cells, including which eligibility guard failed;
- cell-sized flat statics.

Capture the real coast with animation off first. Only attempt another shared
UV/filter change after proving both pixels use the same live source and branch.
Remove the diagnostic afterward.

### 0.1 First resumed diagnosis: live terrain and static sources do not match

**Updated 2026-07-19 after resuming this handoff.** The source premise in §4
below was incomplete: land art ID `0x00A8` is pixel-equivalent to static art
`0x1797`, but the live terrain renderer does not sample that land art. For CC
land tile 168, tile metadata has `texture_id = 0`, so
`land/draw_mesh.rs` falls back to ID 168 in `tex_land_cc.uddp`. That package is
built from `texmaps.mul`; its slot 168 is a brighter 64×64 square texmap. The
static path samples the darker 44×44 isometric diamond from `tex_art_cc.uddp`
(`art.mul`). Therefore the first real divergent checkpoint is decoded/sample
albedo, before hue, lighting, grunge, or global effects.

Additional source evidence:

- masking land art 168 by sparse static `0x1796`'s alpha produces a byte-exact
  match for every opaque V pixel (`compare -metric AE` = 0);
- land art 168 and the full static `0x1797` are also byte-exact matches;
- the inverse of the existing flat-static diagonal transform maps an
  axis-aligned land cell back to those same 44×44 art coordinates;
- all relevant real-site static hues are zero.

The rejected canonical substitution/compositor has now been removed. Every
wet CC static, including dense variants, remains its original flat sprite with
its original atlas layer and UVs. A focused routing test proves two dense
variants remain distinct.

The current bounded fix builds a compact startup atlas from wet CC land-art
slots and uses it only when all of the following are true: CC page-atlas
terrain, `IsWet`, an identity/zero tiledata texture mapping, a present land-art
slot, and four equal cell-corner heights. The shader applies the inverse
diamond mapping on the real XZ terrain mesh. Sloped wet terrain keeps its
texmap, all static graphics remain authored, and EC terrain is unchanged.

The user's first capture of that fix,
`/home/claudio/Immagini/Schermate/Schermata_20260719_170546.png`, was explicitly
"better" but still showed both a broad color boundary and a weaker sawtooth.
Under the tested `classic.night` preset, grunge, fog, grading, blur, KR wash,
and art fake normals are disabled, while the flat-water lighting equations are
already identical. The next divergent sample stage was water animation and
nearest filtering:

- terrain used world-cell UV → animation → diamond mapping → `textureLoad`;
- flat statics used diamond-art UV → animation → atlas IQ filter →
  `textureSample`.

Those anisotropic animation transforms do not commute. The rejected second
iteration therefore tried a dependency-free shared world-cell-to-art transform
and the terrain's exact nearest integer-texel convention for cell-sized flat CC
water. It applied to both dense 44×44 fills and sparse 44×44 V/leg accents while
leaving larger waves and source content intact, but `173158` proves that this
was not the live-path match it claimed to be. Remove it as described in §0.0.

Current validation:

- `cargo check -p dynamapper` passes;
- full `cargo test -p dynamapper` passes: 93/93;
- a focused naga-oil composition test validates the modified land sampling
  module and its bindings;
- the rebuilt application ran for 60 seconds on the live desktop/Vulkan path
  without shader-module, binding-layout, or render-pipeline errors;
- an isolated live-display launch at `[5722, 2991]` confirms Classic art,
  Classic terrain, and the `classic.night` preset are active at the target
  coast;
- automated validation of the second iteration passed, but the user's
  `173158` visual gate decisively failed;
- no phase commit exists; the user visual gate remains mandatory.

The remaining sections retain the rejected-compositor evidence and constraints
from the original handoff. Section 0 above is authoritative for current code,
validation, and next action.

## 1. Archived stop condition before the resumed fix

At the time of the original handoff, the latest renderer output was **not
accepted**. Its color-transfer compositors remain falsified models and must not
be restored or tuned.

The user ended the session because the context had become too large and asked
for a clean handoff. No renderer code was changed after the final screenshot;
only this documentation is being prepared.

There is no water-related commit. The worktree contains multiple related
uncommitted phases plus unrelated user changes. Preserve all unrelated changes.

## 2. The visual truth — inspect these captures first

### Latest bad state: additive sRGB translation

`/home/claudio/Immagini/Schermate/Schermata_20260719_132710.png`

Symptoms reported by the user:

- coastline static art remains darker;
- the large static-water area repeats one terrain-water sprite instead of
  preserving the different static graphics stored in `statics.mul` / the UDDP
  statics package;
- the long inward sawtooth/V strokes are back;
- there are black missing-looking wedges along the coastline.

The current code produced this capture. It is not a viable base for tuning.

### Previous bad state: normalized luma/chroma detail

`/home/claudio/Immagini/Schermate/Schermata_20260719_132203.png`

This version used `terrain * clamp(authored_luma / median_luma) + chroma
residual`. It turned the wavefront into dotted bright/dark diagonal strokes and
introduced a blank-looking discontinuity. It was immediately rejected.

### Best seam-continuity reference, but not a complete solution

`/home/claudio/Immagini/Schermate/Schermata_20260719_103106.png`

This capture had no long sawtooth artifact and the broad water field was much
more continuous. It came from the early canonical-terrain routing phase, before
sparse shoreline art was restored correctly. Treat it only as the visual
reference for **seam continuity**. It is not final because substituting terrain
water for dense static art discards the map-authored water variants, and the
coastline/foam treatment was incomplete.

Other useful regression captures:

- `Schermata_20260719_130151.png`: authored sparse RGB restored; dark coastline
  and sawtooth returned.
- `Schermata_20260719_103106.png`: no long sawtooth, but canonical substitution.
- `Schermata_20260719_011201.png`: original dark static/terrain mismatch and
  edge strokes.

## 3. Non-negotiable requirements clarified by the user

1. **Preserve the exact static graphic chosen by map/static data.** Full water
   placements `0x1797-0x179C` and other variants must keep their own authored
   atlas content. Do not replace every dense static with land tile ID `168`.
2. **Preserve the complete shoreline/wavefront artwork**, including foam and
   the water/land impact area. Do not reduce the art to alpha-only masking.
3. Remove the unintended high-contrast sawtooth while retaining the intended
   subtle wavefront.
4. Static water and terrain water must belong to one coherent material/lighting
   response, especially under Classic + Night.
5. Keep true world geometry and rasterized geometric depth so the design works
   with a future perspective camera. Do not port ClassicUO's synthetic 2D
   sprite depth.
6. Test visual changes across Classic, Enhanced Classic, and KR-like presets.

These requirements mean the solution must fix the **pipeline mismatch while
sampling each original art sprite**, not replace source content.

## 4. Older diagnosis archive — apply the §0 correction

The geometry/ownership findings below remain valid. The source-albedo
conclusion does not describe the live terrain sample; §0 supersedes it.

At map0 x≈5786-5796, y≈3038-3077:

- interior water statics are `0x1797-0x179C` at z=-5 over sunken dry land;
- outer seam cells contain `0x1796` at z=-5 over coplanar wet land;
- `0x1796` is a sparse hollow V: 86 opaque pixels, not a full diamond;
- `0x17AD`/`0x17AE` are sparse diagonal legs and `0x17A5` is a half-leg;
- full 44×44 diamonds have 1012 opaque pixels.

The visible strokes align exactly with the authored V/leg sprite edges. They
are not land triangle diagonals, holes, T-junctions, or sand poking through.

A temporary unlit ownership debug pass (static red, land green) showed no
coverage void and made the strokes disappear. Therefore geometry ownership is
continuous; the unintended contrast is a material/shading problem.

Decoded source art also refuted the idea that the large color gap is intrinsic
to the bitmaps:

- land `0x00A8-0x00AB` median ≈ `[0,56,72]`;
- static `0x1796-0x179C` median ≈ `[0,48-56,72]`;
- `0x1797` is pixel-equivalent to the corresponding land-water diamond.

The approximately 2× on-screen mismatch is produced by asymmetric render
pipelines, not by fundamentally different source albedos.

Already ruled out:

- alpha/coverage gaps and T-junctions;
- float-ULP diamond placement mismatch;
- terrain/sand depth intrusion (large depth probe changed nothing);
- water animation as the cause (artifact persists with animation off);
- land mesh triangle diagonal (wrong screen angle);
- ClassicUO synthetic depth as an acceptable fix.

## 5. What the current code does — and why it is wrong

Current uncommitted routing in
`dynamapper/src/core/render/scene/world/art/statics_collect.rs`:

1. Small wet CC art is decoded and cached by graphic ID.
2. Exactly 44×44 art with alpha coverage ≥0.45 is treated as a dense fill.
3. Dense fills are emitted as `GroundTileInstance` sampling **land tile ID
   168**, so different static graphics collapse to the same repeated terrain
   texture. This directly explains the user's variant-loss report.
4. Sparse art ≤44×44 remains a sprite and carries a median opaque RGB payload.
5. 45×45 and 89×89 decorative waves remain regular authored sprites.

Current uncommitted compositor in `art/main.wgsl`:

```text
canonical_srgb = linear_to_srgb(terrain_water_sample)
authored_srgb  = linear_to_srgb(static_art_sample)
translated     = clamp(canonical_srgb + authored_srgb - sprite_median, 0, 1)
```

This additive translation was intended to retain every authored RGB delta, but
the `132710` capture rejects it. Dark residuals clamp to black, reproducing the
V/leg strokes and black coastline wedges. Do not keep or tune it.

The immediately preceding luma/chroma ratio compositor is no longer in code;
it produced `132203` and is also rejected. The older alpha-only canonical
replacement removed foam/impact art. All three content-replacement/transfer
approaches are dead ends.

## 6. Recommended next approach

### Phase A — restore source ownership without reverting unrelated work

Surgically remove the experimental canonical-content substitution while
preserving the valid geometry work:

Keep:

- WET-only CC flat-quad classification;
- height-aware flat-quad placement for 44/45/89px art;
- real XZ geometry and `out.depth = in.position.z` for flat quads;
- the future-perspective constraint;
- any proven shared flat-water lighting helper, subject to the parity audit
  below.

Remove/rework:

- dense-fill routing to `emit_cc_plain_water_ground_instance` / land ID 168;
- `GROUND_FLAG_CC_CANONICAL_WATER_MATERIAL` and its ground-shader branch if no
  longer used;
- sprite canonical-water mask RGB replacement/translation;
- cached median RGB payload and sRGB conversion helpers if no longer needed;
- live land-atlas bindings on the sprite material if no longer needed.

After Phase A, every placement must still emit/sample its original static
graphic, layer, and UVs. Add a focused routing test proving two dense static
graphics remain distinct sprite atlas selections rather than both becoming
material payload 168.

Do **not** use `git restore` on the whole files: the current diff also contains
valid flat-geometry and lighting changes from earlier phases and shares files
with pre-existing user work. Review hunks individually.

### Phase B — locate the exact asymmetric color stage

The correct fix is stage parity, not source substitution. Add a temporary,
non-committed diagnostic selector that can compare the land-water and flat-art
paths at matching checkpoints:

1. decoded/sample albedo only, no hue/light/effects;
2. after hue only;
3. after the shared flat ambient + diffuse term;
4. after grunge;
5. after global lighting;
6. after grading / KR wash / atmosphere;
7. after local-light/decal contributions.

Use the real `0x1797` ↔ `0x00A8` pixel-equivalent pair. The first checkpoint at
which they diverge identifies the actual fix. Remove the selector after the
comparison.

Known remaining asymmetries worth checking first:

- land uses `apply_texture_shadow_aware_land_grunge`; art uses
  `apply_texture_visual_grunge`;
- land has blur/sharpen and `apply_static_light_decals` stages not mirrored in
  art;
- art samples through `pixel_art_atlas_uv_iq`; land uses its own atlas sampler;
- verify both actual GPU images remain `Rgba8UnormSrgb` at runtime;
- verify global lighting, grading, wash, atmosphere, and hue are applied once
  and in the same order on both paths;
- compare transparent versus alpha-mask handling, but do not alter alpha
  coverage merely to hide the line.

Once the first divergent stage is found, make the smallest shared-material fix
there. The expected outcome is that authored `0x1796` pixels naturally become
subtle against equally processed terrain water, as they are in the classic
client.

### Phase C — visual gate before commit

First reproduce the `103106` seam continuity **while retaining all static
variants and foam**. Then test the same coast under:

- Classic + Night, animation off and on;
- Classic daylight;
- Enhanced Classic;
- KR-like;
- at least two camera angles/zooms that previously exposed the strokes.

Do not commit until the user accepts the captures.

## 7. Final production and validation state

Production files committed in `8475ddc` include:

- `dynamapper/assets/shaders/world/art/ground.wgsl`;
- `dynamapper/assets/shaders/world/art/main.wgsl`;
- `dynamapper/assets/shaders/world/art/shading.wgsl`;
- `dynamapper/assets/shaders/world/common_bindings.wgsl`;
- `dynamapper/assets/shaders/world/effects/surface_effects.wgsl`;
- `dynamapper/assets/shaders/world/land/land_bindings.wgsl`;
- `dynamapper/assets/shaders/world/land/main.wgsl`;
- `dynamapper/assets/shaders/world/land/sampling.wgsl`;
- `dynamapper/src/core/render/scene/world/art/statics_draw.rs`;
- `dynamapper/src/core/render/scene/world/land/mesh_material.rs`;
- `dynamapper/src/core/texture_cache/land.rs`;

The two water-seam handoff documents are the evidence archive and final
operational record, committed separately from production code.

Unrelated user-owned changes that must not be staged/reverted:

- `.cargo/config.toml`;
- `dynamapper/assets/settings/runtime_assets.toml`;
- any settings saved by GUI interaction.

Latest checks are the final checks listed in section 0.-11: `cargo check` and
all 94 tests pass, naga-oil composes the changed land module, and the live
desktop/Vulkan path reaches `InGame` without shader or pipeline errors.

No `cargo fmt`, `rustfmt`, or `cargo clippy` was run. The completed production
phase is commit `8475ddc`.

## 8. ClassicUO references and perspective constraint

Local ClassicUO checkout:

`/mnt/dati/__cloud/mega/_proj/__nonmio/ClassicUO/`

Relevant files:

- `src/ClassicUO.Client/Game/Scenes/GameSceneDrawingSorting.cs`
- `src/ClassicUO.Client/Game/Map/Chunk.cs`
- `src/ClassicUO.Client/Game/GameObjects/Views/View.cs`
- `src/ClassicUO.Client/Game/GameObjects/Views/LandView.cs`

ClassicUO has no flat-ground static path: wet statics are ordinary ordered
sprites. Use it to understand art and ordering intent, not as the depth model.
UODynamapper must keep geometric depth and world-XZ placement for a future
perspective camera.

## 9. Follow-up resume prompt

> Read `ENGINEERING.md`, `docs/agents/dynamapper_architecture.md`, and §0.-11
> of `docs/agent_notes/water_seam_terrain_vs_static_next_handoff.md`. The
> terrain/static water seam is resolved and committed in `8475ddc`; the preset
> isolation fix is `f3d581a`. Do not reopen sampling, grunge, lighting, or fog
> ownership without new evidence: Classic, Enhanced, and KR passed the visual
> gate, diagnostics were removed, 94/94 tests pass, and the live shaders load.
>
> If continuing water work, take one explicitly separate issue: either the
> translucent/see-through CC water-static alpha defect from capture
> `1768955859790.png`, or the stray authored wave sprite noted in §0.-5.
> Preserve `.cargo/config.toml`, `runtime_assets.toml`, and GUI-saved settings.
> The compact land-art substitution is Classic-only by design; do not extend
> its one-diamond-per-cell mapping to EC textures without a separate design.
