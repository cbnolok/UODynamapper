# Land shader controls + land/art placement issues — investigation log

Session-resumable tracking doc for a batch of 6 issues reported by the user
(2026-07-02). Explorative phase: map issues to code, rank severity, investigate
the top one first. Update this file as work continues across sessions —
check off TODOs, append new evidence, don't delete superseded hypotheses
(mark them ruled out instead, so we don't re-walk the same dead end).

## Issue list & severity ranking

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| A | Part 2.3 — some classic (CC) land textures render black/missing | **Critical** | **FIXED** (2026-07-02), verified against real client data |
| B | Part 2.1 — world height/Z space looks compressed (mountains, shores) | High → **downgraded, see below** | Geometric derivation done — `0.1` is NOT clearly wrong; original hypothesis weakened |
| C | Part 2.2 — CC art tiles render higher/misplaced vs correct pos (KR fine) | High | **FIXED** (2026-07-02) — root cause confirmed algebraically, not the same root as B |
| D | Part 1.3 — several shader-dialog effects have no perceivable effect | Medium | Root cause CONFIRMED for 2 of 5 sub-effects |
| E | Part 1.4 — cursor tile inspect overlay strings unreadably long | Low | Confirmed, trivial to fix, not started |
| F | Part 1.1 / 1.2 — Land Shader Controls dialog layout too packed / uncategorized | Low (UX polish) | Confirmed, not started |
| G | Some CC land textures still render black even after fix A, in a real client screenshot | High — new, found 2026-07-02/03 | **FIXED** (2026-07-03) — graphic-id vs. texture-id lookup bug in `draw_mesh.rs`, unrelated to fix A |
| H | CC static "surface-like" tiles (shoreline water) render as vertical billboards instead of flat tiles — wrong shading/animation (issue "2") AND a seam vs. terrain water (issue "3") | High — new, found 2026-07-03 | **FIXED** (2026-07-03) — flat-quad fix (Option A-b) implemented and verified |

Severity rationale: A is a hard correctness bug (visibly broken/missing
content, quantified below). B/C degrade visual fidelity globally but nothing
is missing. D breaks part of the tuning UI/workflow but the renderer itself
is fine. E/F are cosmetic/dev-tool ergonomics only.

**Working rule given by the user:** the oldest, most structural `const`
values in this codebase are the most battle-tested and are probably *not*
the bug — prefer suspecting newer/less-tested code first. This held up
for issue A's *packing pipeline* (new code, checked out, looks correct) but
the *actual* bug turned out to be an old constant whose assumption (client
data size) was outgrown by newer client data — see below. So: don't discard
old constants outright, but verify their assumptions against real current
data before trusting them.

---

## A (CRITICAL) — Missing classic land textures — CONFIRMED root cause

### Root cause
`lib/uocf/src/classic/land_texture.rs:184`:
```rust
const TEXMAP_MAX_ID: u32 = 0x1388; // 5000
```
`TexMap::load_with_verdata` (same file, ~line 156) only reads index slots
`0..TEXMAP_MAX_ID` from `texidx.mul` (loop at line 209), silently
`continue`-ing/never allocating anything past that. This constant dates to
the very first commit of the file (`b5076c1`, 2025-06-28) and has never
changed — it's old/structural, exactly the kind of thing the "probably
fine" heuristic points away from. It turned out to be the bug anyway,
because the *client data* it was written against has since been outgrown.

This value flows straight through the newer (2026-05-13) `tex_land_cc`
packaging pipeline:
- `lib/udd-conv/src/tex_land_cc.rs::convert_texmaps_mul_to_tex_land_cc_uddp_with_patches_and_progress`
  sets `slot_count = texmap_source.len()` (line 196), which is
  `TexMap::len()` = `file_data.len()` = capped at 5000.
- `pack_tiles_into_pages` (line 547) allocates exactly `slot_count` slot
  records, `TexLandCcSlotRecord::absent` for ids `0..slot_count`.
- At runtime, `lib/udd-assets/src/tex_land_cc.rs::TexLandCcPackage::present_slot`
  (line 196) does `self.slots.get(id as usize)` — any `id >= 5000` is out of
  bounds and returns `None`, i.e. "texture not present" → black fallback
  layer (`dynamapper/src/core/texture_cache/land/cache.rs:85`, "Reserve
  layer 0 as a permanent black fallback tile").

I verified the packing/atlas code itself (new, 2026-05-13, less tested) is
**not** the bug — every decoded tile does get placed into a page and does
get a slot record; nothing is silently dropped there. The cap happens purely
at the MUL-index-read stage, before any of that newer code runs.

### Verified against real client files
Local machine has real UO client trees under
`/mnt/dati/_proj_local/_uo_clients/`. Checked `texidx.mul` (12 bytes/entry:
u32 lookup, u32 size, u32 extra) directly with a throwaway Python parse
(valid = `lookup != 0xFFFFFFFF && extra != 0xFFFFFFFF && size in {0x2000 (64×64), 0x8000 (128×128)}`):

| Client | total index entries | valid land textures | valid entries with id ≥ 5000 (dropped today) |
|---|---|---|---|
| `Ultima Online Classic_7_0_20_0` | 16384 (0x4000) | 2647 | **702 (~27% of all valid textures)** |
| `Ultima Online Mondain's Legacy` | 16383 | 1870 | 1 |
| `Ultima Online Gold` (old client) | 4096 | 1869 | 0 |

This is exactly why "a previous version had that working" — early/older
client data (e.g. `Ultima Online Gold`, only 4096 index slots total) never
exceeded the 5000 cap, so the bug was invisible. The modern
`Classic_7_0_20_0` tree (most likely what's actively being tested against
now) loses **702 real textures**, rendered black.

### Fix applied (2026-07-02)
Replaced the hardcoded `TEXMAP_MAX_ID` with the real index file size in
`lib/uocf/src/classic/land_texture.rs::TexMap::load_with_verdata`:
```rust
let texmap_max_id = texidx.element_count() as u32; // was: const TEXMAP_MAX_ID: u32 = 0x1388;
let mut texmap = TexMap {
    file_data: vec![TexMapElement::default(); texmap_max_id as usize],
    ...
};
...
for i_idx_raw in 0..texmap_max_id { ... }
```
`IndexFile::element_count()` (`lib/uocf/src/classic/generic_index.rs:52`)
returns the true entry count from the file's actual size, and was already
computed via `texidx` right before this point (previously only used for a
log message). The old commented-out `// 0..texidx.element_count()` next to
the loop (visible in `git log -p` on older revisions) confirms this exact
fix was considered once and abandoned in favor of the hardcoded guess.

Downstream check done: `TEXMAP_MAX_ID` was only referenced in these 3 spots
in `land_texture.rs` (grepped workspace-wide) — narrow, self-contained
change, no other file depended on the literal value or on `TexMap`'s array
being exactly 5000 long.

**Verification:**
- `cargo check -p uocf` and `cargo check --workspace` both pass clean.
- Ran the real converter end-to-end against the modern client:
  `cargo run -p udd-conv-cli --bin udd-cli -- pack pack-texmaps --ccdir
  "/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0" --raw
  --output <scratch>/tex_land_cc.uddp`. Output now reads:
  `uocf: Loaded 16384 (0x4000) Index Elements from 'texidx.mul'.` /
  `Parsed 16384 (0x4000) Map Tile texture slots, loaded 2647 (0xa57) valid.` /
  `Wrote 4 pages ... for 2647 populated slots out of 16384 total slots.`
  This matches exactly the 2647 valid entries found by the earlier direct
  Python parse of the same `texidx.mul` — confirming all entries (including
  the 702 previously beyond the 5000 cap) are now read and packed.

**Real asset regenerated (2026-07-02):** re-ran `pack-texmaps` against the
user's actual runtime package at `/home/claudio/test/uddp/tex_land_cc.uddp`
(referenced by `dynamapper/assets/settings/runtime_assets.toml`'s
`udd_path`), source `Ultima Online Classic_7_0_20_0` (user-confirmed via
`AskUserQuestion`), flags `--raw --zstd` (RGBA8888 + zstd, no BC7, no
upscale — user-confirmed workflow default, see memory
`project_uodynamapper_uddp_cc_conversion_defaults`). Old file backed up
alongside it as `tex_land_cc.uddp.bak-pre-texmap-max-id-fix` before
overwriting. New package: `Wrote 4 pages (RGBA8888 Zstd) for 2647 populated
slots out of 16384 total slots`; `tool info` confirms 6 files, self-consistent
package hash, metadata size grew from ~100KB to ~327KB (matches the
5000→16384 slot count scale-up almost exactly). Structurally verified; **not
yet visually confirmed in-game** by the user.

### Open items for next session
- [ ] User to launch dynamapper and confirm previously-black classic land
      tiles now render correctly with the regenerated package.
- [ ] Once confirmed good, delete the `.bak-pre-texmap-max-id-fix` backup.
- [ ] Check `try_load_cc_land_tile` in
      `dynamapper/src/core/texture_cache/land/cache.rs:141` — dead stub,
      always returns `None`, never called, present unchanged since its
      introduction. Looked like a possible `art.mul` fallback for CC land
      textures never implemented. Probably unrelated to the 702-texture gap
      (that's a MUL-index-cap issue, not a "missing from texmaps.mul
      entirely" issue) — but worth a decision: implement it, or delete the
      dead code, once A is confirmed fixed and if any textures are *still*
      missing after the cap fix.

---

## B + C (HIGH) — Height/Z compression & CC art vertical misplacement (unconfirmed, likely linked)

### B — height/Z looks compressed (mountains, shores)
`dynamapper/assets/shaders/world/land/atlas.wgsl:56-58`:
```wgsl
let z_i32 = i32(height_biased) - 128;
let tile_height = f32(z_i32) * 0.1;
```
Flat, undocumented `0.1` world-Y-per-Z-unit constant. No derivation/comment,
unlike the camera's horizontal calibration in
`dynamapper/src/core/render/scene/camera.rs:30-46`
(`ORTHO_WIDTH_SCALE_FACTOR = 1.79`, explicitly commented as "found through
trial and error... to render tiles of same width and height"). There is no
equivalent measured/derived vertical constant. Also flagged by a pre-existing
TODO at `dynamapper/src/core/render/scene/world/art/statics_collect.rs:2425`:
`let height_scale = 0.1; // TODO: no magic numbers. also, don't we already have a fn for this?`

The camera is a true 3D isometric rig (`PlayerCamera::BASE_OFFSET_FROM_PLAYER
= Vec3::new(5,5,5)`, ~35.26° tilt) — different from the classic 2D client's
flat 2:1 pixel-shift dimetric projection. A `0.1` world-Y-per-Z-unit that
"looks right" needs to be derived from *this* camera's actual projection
geometry, the same way `TILE_SIZE_FACTOR` was for the horizontal axis — not
guessed.

### C — CC art appears higher/misplaced, EC (KR) fine
`dynamapper/src/core/render/scene/world/art/statics_collect.rs:27-31`:
```rust
const STATIC_WORLD_Y_PER_XZ_PIXEL: f32 = 7.5 * 0.1;   // embeds the same 0.1
const CC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / CC_TILE_PIXEL_WIDTH; // /44
const CC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * CC_WORLD_XZ_PER_PIXEL;
const EC_WORLD_XZ_PER_PIXEL: f32 = ISO_TILE_SCREEN_DIAGONAL_WORLD_UNITS / EC_TILE_PIXEL_WIDTH; // /64
const EC_WORLD_Y_PER_PIXEL: f32 = STATIC_WORLD_Y_PER_XZ_PIXEL * EC_WORLD_XZ_PER_PIXEL;
```
Used by `resolve_static_billboard_bounds` (line 73) to convert a sprite's
pixel draw-offset into world-space billboard bounds. Both CC and EC share
the same `STATIC_WORLD_Y_PER_XZ_PIXEL` (embedding the same suspect `0.1`),
but CC divides by the smaller `CC_TILE_PIXEL_WIDTH = 44` vs EC's `64`. A
smaller denominator means the *same* absolute pixel-offset error produces a
*proportionally larger* world-space displacement for CC than for EC — which
would explain "CC looks off, KR looks fine" from a single shared
miscalibration rather than two independent bugs. Elevation itself
(`tile.z * height_scale` + `depth_class_y_bias`, lines ~254-262/283-293) is
source-independent (same formula for CC/EC), so the CC-specific symptom has
to come from the billboard/offset math, not the elevation math — consistent
with this theory.

**Not yet verified** — this is architecture-level reasoning from reading the
constants, not confirmed against real client data or an in-game screenshot.

### Update (2026-07-02, subagent investigation) — B and C are NOT the same root cause after all

Dispatched a focused subagent to derive the geometrically-correct
world-Y-per-Z-unit from the camera's actual projection and dig further into
the CC/EC offset pipelines. Full findings below; this substantially revises
the original theory that B and C share one root cause.

**B (height compression) — weakened, likely not a simple constant bug:**
Camera basis derivation (from `Vec3::new(5,5,5)` offset +
`looking_at`/`Vec3::Y`, matches `STATIC_BILLBOARD_RIGHT_XZ` exactly,
cross-validating the method) plus the existing `ORTHO_SIZE_FACTOR` /
`ORTHO_WIDTH_SCALE_FACTOR` horizontal calibration gives a plausible target
range for world-Y-per-Z-unit of **~0.088–0.129** (using the standard
UO-community classic screen-shift formula `screenY = (x+y)*22 - z*4`, i.e.
4px/Z-unit against a 44px tile, as the un-documented-in-repo assumption —
flagged explicitly as an assumption, not a repo fact). **The current `0.1`
falls inside that range**, and the central estimate (0.088) is actually
*smaller* than 0.1 — i.e. if anything the current value is slightly more
generous/exaggerated than a strict classic-faithful reproduction, not more
compressed. This derivation does not support "0.1 is a clear bug." The
reported visual compression may be a legitimate design-taste question
(faithful-to-classic vs. deliberately exaggerated relief) rather than a
calibration error — **that's a decision for the user, not something further
code archaeology will resolve.**

**`STATIC_WORLD_Y_PER_XZ_PIXEL = 7.5 * 0.1` (`statics_collect.rs:27`)** —
confirmed structurally independent from the land-elevation `0.1` (converts
sprite pixel-offsets to world Y, not map Z-units; introduced standalone in
commit `56444143`, no comment then or since, `git log -p --follow`
checked). Its own theoretically-derived target is `≈0.684`; current
`0.75` is only ~10% off — same rough magnitude of looseness as
`ORTHO_WIDTH_SCALE_FACTOR`'s own ~3.7% empirical imprecision, not the kind
of gross error that would read as "misplaced." **Separately**,
`statics_collect.rs:2425`'s `height_scale = 0.1` (the TODO one) IS a literal
mirror of the land-elevation constant by its own comment
(`// The land shader does: world.y = z * 0.1`) and would need to change in
lockstep with `atlas.wgsl:58` if that one ever changes — but per the above,
there's no confirmed reason to change either right now.

**C (CC misplacement) — new, more concrete, independent lead found:**
CC and EC compute static-art anchor offsets via two *structurally different
algorithms*, not the same formula scaled by a different tile-width
denominator (the original "smaller denominator → proportionally bigger
error" theory is now considered less likely than this):
- CC's primary path (`lib/udd-conv/src/tex_art_cc.rs:920-928`,
  `classic_static_draw_offset`, called from `tex_art_cc.rs:763-772`)
  synthesizes the anchor purely geometrically from the decoded bitmap's own
  dimensions: `draw_offset_x = -(width/2)`, `draw_offset_y = -height`. No
  embedded per-sprite metadata is consulted for the primary (non-alias)
  path.
- EC's path (`lib/udd-conv/src/tex_art_ec.rs:753-780`) takes the offset
  directly from `tileart.uop`'s embedded per-entry metadata
  (`ec_img_offset.x_off/y_off`, `lib/uocf/src/enhanced/tileart.rs:265-272,356-361,486-491`)
  — trusted, not derived.
- `tileart.uop` *also* carries a parallel `cc_img_offset` with the same
  struct shape (`tileart.rs:303-304,363-368`), but it's only consulted by
  CC's alias path for IDs missing a direct `.mul` entry
  (`tex_art_cc.rs:945-951`), never by the primary path.
- **Testable, concrete hypothesis:** if CC static art bitmaps have
  transparent padding around their visible content within the sprite's
  bounding box (common in `art.mul`), then `draw_offset_y = -height` (full
  bitmap height, including any transparent padding) anchors higher than
  the actual visible pixel content warrants — sprite would render too high,
  exactly matching the reported symptom. EC sidesteps this entirely by
  trusting embedded, presumably content-aware, offsets instead of deriving
  from raw bitmap bounds.

### Update (2026-07-02, second subagent pass) — C root cause CONFIRMED, crop hypothesis refuted

Dispatched a follow-up subagent to test the transparent-padding/crop
hypothesis with real decoded data. Result: **refuted**, but it found the
actual bug while tracing offset consumption precisely. Full detail:

**Crop hypothesis refuted:** `TexArtCcAtlasOptions`
(`lib/udd-conv/src/tex_art_cc.rs:126-138`) has no crop/trim option at all —
that only exists on the EC side (`TexArtEcAtlasOptions`,
`lib/udd-conv/src/tex_art_ec.rs:89`, wired to `--art-crop-transparent-bounds`
on the `pack-ec-textures` CLI command only). `rgba_bounds.rs` (the trimmer)
is never imported by `tex_art_cc.rs`. Decoded ~490 real CC static sprites
from the actual client on disk
(`/mnt/dati/_proj_local/_uo_clients/Ultima Online Classic_7_0_20_0/art.mul`)
via a throwaway (deleted afterward, repo left clean) test: top padding is
0-1px on virtually all of them — classic `art.mul` art is already tightly
bound at the top by the original format. So there's no cropping happening
for CC today, and even if there were, there's essentially no top padding to
cause the kind of mismatch hypothesized. Side note: the user's "waste the
least space" goal is **currently only implemented for EC, not CC** — a
separate, smaller potential follow-up if wanted later.

**Actual bug found, CONFIRMED algebraically:** tracing exactly how
`draw_offset_y` is consumed in
`dynamapper/src/core/render/scene/world/art/statics_collect.rs:82-93`
(`resolve_static_billboard_bounds`):
```rust
let local_min_y = -(offset_y_pixels as f32 * world_y_per_pixel);
let local_max_y = local_min_y + logical_height * world_y_per_pixel;
```
CC's primary path sets `draw_offset_y = -height` (full sprite height,
negative) — `lib/udd-conv/src/tex_art_cc.rs:925-928`,
`classic_static_draw_offset`. Substituting `offset_y_pixels = -height`,
`logical_height = height`:
- `local_min_y = -( -height · wy ) = height · wy` (should be `0`)
- `local_max_y = height·wy + height·wy = 2·height·wy` (should be `height·wy`)

**Both bounds are shifted upward by exactly one sprite-height.** Every CC
static sprite floats above the ground by an amount equal to its own
on-screen height — proportionally worse for tall objects (trees,
buildings), which matches the reported symptom (and its severity pattern)
far better than the padding-crop theory did. Vertex consumption confirmed
in `dynamapper/assets/shaders/world/art/main.wgsl:82-92` (top-UV vertex ←
`local_max_y`, bottom-UV vertex ← `local_min_y`, nothing downstream
compensates). The X-axis formula (`local_min_x = offset_x_pixels *
world_xz_per_pixel`, no negation) IS self-consistent with CC's
`-(width/2)` and correctly centers sprites — this bug is Y-axis only.

The formula's implicit contract only comes out correct when
`offset_y_pixels ≈ 0` ("no nudge, bottom-anchored by `logical_height`
alone") — structurally consistent with how EC's `texture.offset_y` is used
(a raw untouched metadata value, `tex_art_ec.rs:753-780`), which is
presumably why EC renders fine while CC (which deliberately sets
`offset_y = -height`, violating that assumption) doesn't. **Caveat:** the
subagent could not empirically confirm EC's real offset magnitudes from
`tileart.uop` on the real EC client (`_Ultima Online Enhanced`) — parsed
values were garbage (e.g. `50331648`), likely a struct/version mismatch for
that client's `tileart.uop` layout — so the "EC offsets are near-zero"
half of this reasoning is structural/algebraic, not empirically confirmed
with real numbers. Worth resolving that parsing issue separately at some
point (`lib/uocf/src/enhanced/tileart.rs`), but doesn't block confidence in
the CC-side finding, which stands on its own from the formula/consumption
trace.

**Fix applied (2026-07-02):** `lib/udd-conv/src/tex_art_cc.rs`, in
`classic_static_draw_offset` — changed the Y draw-offset from `-height` to
`0` (with an explanatory comment), and dropped the now-unused `height`
parameter from the function signature and its one call site:
```rust
// Y is left at 0 (no per-sprite nudge): resolve_static_billboard_bounds already
// anchors the sprite's bottom edge at ground level and extends it upward by the
// full decoded height, so a nonzero offset here (e.g. -height) double-counts the
// height and floats the sprite above the ground by its own on-screen height.
Ok((
    i16_draw_offset(art_id, "x", -(i32::from(width) / 2))?,
    i16_draw_offset(art_id, "y", 0)?,
))
```
Isolated to CC's own offset computation; `resolve_static_billboard_bounds`
and the EC path are untouched (no regression risk to EC).

**Verification:**
- `cargo check -p udd-conv` and `cargo check --workspace` both pass clean.
- `cargo test -p udd-conv --lib`: all 88 tests pass (no existing test
  exercised `classic_static_draw_offset`'s own formula directly, only
  storage/packing preservation of already-computed offsets, so nothing
  needed updating).
- Re-converted the real runtime asset,
  `/home/claudio/test/uddp/tex_art_cc.uddp` (source
  `Ultima Online Classic_7_0_20_0`, flags `--raw --zstd`, matching the
  confirmed CC workflow defaults). Old file backed up as
  `tex_art_cc.uddp.bak-pre-y-offset-fix`. New package:
  `Wrote 40 pages (RGBA8888 Zstd) for 34503 populated slots out of 81884
  total slots`; `tool info` recognizes it as `tex_art_cc`, self-consistent
  package hash, `Rgba8888` page format, atlas 2048x2048/40 pages/34503
  populated slots — structurally sound. **Not yet visually confirmed
  in-game** by the user.

### TODO for next session
- [ ] User to launch dynamapper (CC art source mode) and confirm static
      objects now sit on the ground correctly instead of floating.
- [ ] Once confirmed good, delete both `.bak-pre-*` backup files
      (`tex_land_cc.uddp.bak-pre-texmap-max-id-fix` and
      `tex_art_cc.uddp.bak-pre-y-offset-fix`).
- [ ] Optional, lower priority: investigate the `tileart.rs` EC offset
      parsing issue found as a side effect (garbage values on the real `_Ultima
      Online Enhanced` client) — separate from this bug, but worth a look
      since it blocked full empirical confirmation of the EC side.
- [ ] Optional, lower priority: CC's static-art atlas packing doesn't crop
      transparent bounds today (only EC's does) — revisit if atlas size
      becomes a concern; not related to the misplacement bug.
- [ ] **Decision still needed from the user, not further code reading:** for B (height/Z compression),
      is the goal "reproduce classic client's on-screen height ratio
      exactly" or "deliberately exaggerate relief for clarity/legibility"?
      The derivation above can't answer this — it's a taste call. If
      "exactly," get one real in-game screenshot of a known steep
      transition (there's already an F3 cursor-inspect overlay for
      measuring exact Z at a location, `cursor_behavior.rs`) to compute the
      real on-screen ratio and compare against classic's 4px/Z-unit; only
      then would a specific corrected constant make sense.
- [ ] For C, the more promising next step (read-only, no screenshot
      needed, uses the real client trees already on disk under
      `/mnt/dati/_proj_local/_uo_clients/`): decode a handful of specific
      CC static art IDs directly and check whether the decoded RGBA has
      transparent padding above the visible content that `-height` doesn't
      account for, compared against `tiledata.mul`'s `height` field for the
      same IDs. This is the strongest concrete lead in the whole batch and
      is fully testable without running the app.
- [ ] Once either is confirmed, this is still a value-affecting rendering
      change — propose the specific fix and get user sign-off before
      editing code (per collaboration rules; especially true for B since
      the "4px/Z-unit" target is an assumption sourced from general UO
      community knowledge, not from anything in this repo).

---

## D (MEDIUM) — Shader dialog effects with no visible effect — 2 of 5 CONFIRMED

Basic Controls block: `dynamapper/src/core/render/dialogs/land_shader.rs`,
`land_ui_system`, lines 96-180.

- **CONFIRMED — KR Transition Grime / KR Color Wash inert from Basic Controls:**
  Gated in the shader by `visual_profile == 2u`
  (`effects.post_process_profile`, `main.wgsl:523`) — e.g.
  `kr_color_wash.wgsl:16` (`if profile != 2u || ... { return rgb; }`),
  `main.wgsl:749` (`apply_kr_transition_grime`), `main.wgsl:290`
  (`apply_kr_land_atmosphere_depth`). Basic Controls' "Render Style"
  selector (land_shader.rs:101-110) only ever writes `shading_mode`, never
  `post_process_profile` — that field is only reachable via the hidden
  "Advanced → Effects → Visual Profile" section (lines 189-199). So a user
  can select "KR" in Basic Controls and toggle these checkboxes and see
  nothing change, because `post_process_profile` is a silently-independent
  field. Corroborated by the user's own uncommitted edit to
  `dynamapper/assets/settings/shaders/land.toml`: `classic.morning` preset
  has `shading_mode = 2` but `post_process_profile` left at `0` — exactly
  this mismatch, caught live.
- **CONFIRMED — Land Normal Maps only apply to EC-sourced land textures:**
  `main.wgsl:731`: `if (effects.enable_normal_maps == 1u && shading_mode != 0u && tile.texture_size == 2u)`.
  `texture_size == 2u` means "EC atlas" specifically
  (`land_bindings.wgsl:11`); CC textures are `texture_size` 0/1 and never
  enter this branch. If tested with CC art source, it's a structural no-op,
  not a subtlety issue.
- **Unconfirmed / lower confidence:** Grunge, Art Projected Shadows, Art
  Fake Normals — no hard gate found; may just be genuinely subtle
  (low default strength / small magnitude). Needs an in-app A/B check, not
  further static analysis.

### TODO for next session
- [ ] Decide the intended UX: should Basic Controls' "Render Style" also
      set `post_process_profile` (probably yes, they're conceptually the
      same "mode" to a user), or should Basic Controls expose Visual
      Profile as its own explicit control? This needs a design decision
      before writing code (per collaboration rules).
- [ ] Once decided, wire it up and re-verify KR Transition Grime / KR Color
      Wash actually change the render in KR mode.
- [ ] Spend a short in-app session toggling Grunge / Art Projected Shadows /
      Art Fake Normals at max strength to confirm whether they're "broken"
      or just "subtle."

---

## E (LOW) — Cursor tile inspect overlay strings too long

`dynamapper/src/core/render/overlays/cursor_behavior.rs`:
`build_cursor_inspect_label` (line 537) and `describe_static_tile_details`
(line 1211, the actual format string at line 1301) each emit a single
~15-field line per static
(`id=... z=... hue=... name=... surf=... cc_tex=... ec_tex=... ec_tex_branch=... cc_slot=... tex_art_ec_slot=... tex_land_ec_slot=... tex_land_ec_direct=... ...`),
and up to 3 are joined with `" | "` (`describe_static_tiles`, line 1186).
Rendered as a plain Bevy UI `Text` node (`OverlayCursorInspectText`, spawned
at line 191) with no width constraint/wrap. This is a debug dump, not
scaled for on-screen use. Confirmed, not started.

### TODO for next session
- [ ] Decide the fix shape: shorten field labels, add a verbosity toggle,
      or reflow into a multi-line/table layout. Needs a quick design call,
      not exploration.

---

## F (LOW) — Land Shader Controls dialog layout

`dynamapper/src/core/render/dialogs/land_shader.rs`, `land_ui_system`,
"Basic Controls" block lines 96-180. 25+ controls (mode selector, output
toggles, brightness/sun/shadow sliders, 3 composite sliders, 15 Art/KR
sliders at 146-160, 8 mixed-category checkboxes in one `ui.horizontal` at
162-171, 3 more sliders) all stacked in one linear column, no
`ui.columns`/`egui::Grid` anywhere in the file.

Recommendation (not yet implemented): group by category first (full-screen
/ terrain-land / art), *then* lay out each category group in 2 columns
(`ui.columns(2, ...)` or `egui::Grid`) — grouping-then-columning reads
better than raw alternating left/right of unrelated controls.

### TODO for next session
- [ ] Implement categorized grouping + 2-column layout once D's design
      decision (Visual Profile control placement) is settled, since that
      may add/move a control in this same panel.

---

## G (HIGH) — Some CC land textures still black after fix A — root cause not yet found

User provided a real in-game screenshot (2026-07-02/03) after fix A was
applied, showing a large contiguous black region in loaded terrain (not an
unloaded-chunk area — confirmed by the user). Cursor-inspect overlay was
hovering directly on a black tile: `land: id=15856 z=0 tex=2016
name=NoName`.

**Checked directly:**
- `texture id 2016` in the real `texidx.mul`
  (`Ultima Online Classic_7_0_20_0`) is valid: `lookup=0x127c000 size=0x2000
  (64x64) extra=0`. Not a source-data gap.
- Checked the actual regenerated runtime package,
  `/home/claudio/test/uddp/tex_land_cc.uddp` (the one fix A produced),
  directly via a throwaway reverted test using
  `udd_assets::tex_land_cc::TexLandCcPackage`: **slot 2016 is present** —
  `page_index=2 x=593 y=1977 w=64 h=64`. So the packaging fix (A) is not
  the cause here; the texture is correctly packaged.
- Ruled out texture-array capacity as a systemic cause:
  `TEXARRAY_SMALL_MAX_TILE_LAYERS`/`TEXARRAY_BIG_MAX_TILE_LAYERS` are 2048
  each (`dynamapper/src/core/texture_cache/land/texture_array.rs:40-41`) —
  far more than a normal viewport would need, unlikely to explain a large
  contiguous black area via eviction/exhaustion.

**CONFIRMED by user (2026-07-03): this is a genuine runtime bug, not a
stale build.** User confirmed the app was relaunched with the regenerated
package and re-tested — still black.

### Root cause CONFIRMED (very high confidence, verified against real data)

**The renderer indexes `tex_land_cc.uddp` by the map cell's raw graphic id,
but the package is keyed by tiledata's `texture_id` field — a completely
different bug from fix A (the packaging cap).** Most tiles happen to have
`texture_id == graphic_id` (coincidence/convention), so it "works" for the
common case; a small set of tiles where they differ renders black (slot
absent) or, worse, silently shows the *wrong* texture (slot present but for
an unrelated texmap).

- `dynamapper/src/core/render/scene/world/land/draw_mesh.rs`, per-id loop
  ~lines 760-806 (`sys_draw_spawned_land_chunks`): for the CC page-atlas
  branch, `package.present_slot(id as u32)` is called with `id = cell.id`
  (the raw graphic id straight from `map*.mul`, `lib/uocf/src/classic/map.rs:47`) —
  never resolved to the tile's actual `texture_id` first. If absent,
  `texture_lookup_cache[id] = 3` (mode 3 = "missing", forces black per
  `dynamapper/assets/shaders/world/land/atlas.wgsl:61,84`).
- But `tex_land_cc.uddp`'s slots are keyed by **texmap id**
  (`lib/udd-conv/src/tex_land_cc.rs:452,479,583`; confirmed by
  `present_slot`'s simple `self.slots[id]` indexing,
  `lib/udd-assets/src/tex_land_cc.rs:196`) — i.e. tiledata's `texture_id`
  field, NOT the land graphic id.
- **The EC arm of this same loop already does the correct indirection**
  (`resolve_effective_runtime_slot_id(id)`, draw_mesh.rs:772) — the CC arm
  is simply missing the equivalent step. Not a design gap, a straightforward
  parity gap between the two arms.
- Verified against real `tiledata.mul`/`texidx.mul`
  (`Ultima Online Classic_7_0_20_0`, 16384 land tiles): graphic 15856 →
  `texture_id`=2016 (matches the inspector reading exactly); `texidx` slot
  15856 is invalid, slot 2016 is valid. Renderer asks for slot 15856 (never
  asks for 2016) → black.

**Scope — a specific, predictable class, not random:** of 16384 land
tiles, 2611 have `texture_id == graphic_id` (unaffected/accidentally
correct), 13687 have `texture_id == 0` (untextured, separate/unrelated,
unaffected by this fix either way), and **86 have `texture_id != 0` and
`!= graphic_id`** — the broken class. Of those 86: **80 render solid
black** (including a contiguous run, graphics 77-90+, all sharing
`texture_id`=76 — explaining why the user sees one *contiguous* black
region rather than scattered single tiles), and **6 render a silently
wrong texture** (e.g. graphics 2028-2033 → texmap 2241-2246, a subtler bug
worth telling the user about even though it wasn't visually reported yet).
This count (86) is specific to this client's `tiledata.mul`; other clients
will have a different count but the same failure shape.

### Proposed fix — NOT YET IMPLEMENTED, awaiting sign-off

`dynamapper/src/core/render/scene/world/land/draw_mesh.rs`, CC arm of the
per-id loop (~lines 763-766): resolve the graphic id to tiledata's
`texture_id` before the slot lookup (falling back to the graphic id itself
when `texture_id == 0` or tilemeta is unavailable, to preserve current
behavior for untextured tiles):
```rust
// Resolve graphic id -> tiledata texture id (tex_land_cc slots are keyed by texmap id).
// Fall back to the graphic id itself when texture_id is 0 or tilemeta is unavailable,
// preserving current behavior for untextured tiles.
let cc_lookup_id = frame_pacing.tilemeta.as_ref()
    .and_then(|m| m.0.land_tiles().get(id as usize))
    .map(|t| t.texture_id as u32)
    .filter(|&t| t != 0)
    .unwrap_or(id as u32);
```
then use `cc_lookup_id` (instead of raw `id`) in
`package.present_slot(cc_lookup_id).map(|_| cc_lookup_id)`, so the stored
payload becomes the texmap id. `texture_lookup_cache` stays indexed by `id`
(graphic id) — only the resolved payload changes. Mirrors what the EC arm
already does. Optional follow-up: the legacy (non-page-atlas) branch,
~lines 789-795 (`get_texture_size_layer(..., id, ...)`), has the identical
conflation and should get the same fix for consistency, though it's not
the path exercised by the user's reported scenario.

### Fix applied (2026-07-03) — user-approved and implemented

`dynamapper/src/core/render/scene/world/land/draw_mesh.rs`, CC arm of the
per-id loop (~lines 763-778): now resolves `cc_lookup_id` from
`frame_pacing.tilemeta`'s `land_tiles()[id].texture_id`, falling back to
the raw graphic `id` when `texture_id == 0` or tilemeta is unavailable, and
uses `cc_lookup_id` for both `present_slot(...)` and the payload stored in
`texture_lookup_cache` — exactly the fix proposed above, mirroring the
EC arm's existing `resolve_effective_runtime_slot_id`.

**Note on process:** the implementing subagent's final report was lost to
a connection error mid-response (only a token-count/error stub came back,
no actual summary). Rather than treat that as "done," re-verified
everything directly and independently: read the actual diff via `git diff`
(not a summary), confirmed it matches the proposed fix exactly, confirmed
the EC arm (lines 779-786) is untouched, confirmed the legacy
non-page-atlas branch (~line 802, `get_texture_size_layer(..., id, ...)`)
was intentionally left as-is (still using raw `id`) — checked why this is
fine: `use_land_page_atlas` (line 745-748) is true whenever
`cache_r.tex_land_cc` is loaded, which is the actual current
architecture ("fully dependent on uddp asset files" per project history);
that legacy branch is only reachable if the CC package failed to load at
all, effectively vestigial fallback code, not part of the reported bug's
path — reasonable to leave alone.

**Independently re-ran verification** (not just trusting the lost
report): `cargo check --workspace` clean; `cargo test -p dynamapper`: 88
passed, 0 failed; `git status` confirms no stray scratch files left behind
and no unexpected files touched beyond the intended one.

**Not yet visually confirmed in-game** — needs the user to check that
land graphic 15856 (and the contiguous ~77-90 run sharing texture_id 76)
now render correctly, and ideally that the 6 "silently wrong texture"
tiles (graphics 2028-2033) now show the correct texmap too.

### TODO for next session
- [ ] User to confirm in-game.
- [ ] No package regeneration needed — this was a pure runtime Rust bug,
      not a conversion/asset bug.

**User confirmed in-game (2026-07-03): CC land textures now load correctly.
Issue G is fully closed.**

## H (HIGH) — CC surface-like (shoreline water) statics: vertical billboard causes wrong shading + seam vs. terrain

User's issues "2" and "3" from the 2026-07-02/03 screenshot: open-water
terrain tiles render correctly (checkered blue wave pattern, via the land
render path); the static water tiles at the shore edge (placed as
individual "items"/statics in classic UO map design, used to get finer
coastline detail than the terrain grid allows — normal, expected map
design, not a bug) render very differently — a dark line with black
dash/tick marks — and don't seamlessly meet the terrain water at the
boundary.

### Root cause — CONFIRMED, one shared cause for both symptoms

**CC never gets a flat "ground quad" treatment for surface-like statics —
only EC does.** `resolve_static_visual_kind_cached`
(`dynamapper/src/core/render/scene/world/art/statics_collect.rs:662-683`):
the `Cc` arm unconditionally returns `StaticVisualKind::CcRegular`
(billboard). Only the `Ec` arm can produce `StaticVisualKind::TexLandEc`
(flat ground quad, via `resolve_ec_static_visual_kind`). Same asymmetry
already visible in the debug overlay
(`cursor_behavior.rs:830` Cc→Sprite vs `:866-882` Ec→Ground). Downstream,
`emit_art_static_tile` (`statics_collect.rs:2005-2062`) only routes
`TexLandEc` to `emit_ground_static_instance` (flat quad); every CC static
goes to `emit_billboard_static_instance` (vertical card).

**Issue "2" mechanism:** the water art (~44×44 diamond) is rendered as an
**upright** vertical card facing the camera
(`dynamapper/assets/shaders/world/art/main.wgsl:87-91`, `local.y` maps to
world Y) instead of flat in the XZ plane like `ground.wgsl:159-163` (and
like terrain water). Effects meant for upright objects then get applied
inappropriately: water UV "breathing" animation
(`apply_water_animation`, gated on `is_wet==1`,
`statics_collect.rs:1885-1907`) jitters the sprite's alpha-cut edges on a
vertical card; object surface shading
(`apply_art_surface_shading` with `is_ground_art=false`,
`shading.wgsl:112-118`) darkens the lower part of the sprite
(`lower_occlusion`/`vertical_light` terms) plus mottle noise — producing
the dark waterline + dash/tick look. **Projected shadows are not the
cause** — real water statics are classified `Background`-depth (see
below) and the shadow pass discards for that class
(`main.wgsl:156`).

**Issue "3" mechanism:** direct consequence of the same vertical geometry.
The card's bottom edge is a single diagonal line at ground level, rising
~1 world unit up (`logical_height * CC_WORLD_Y_PER_PIXEL ≈ 44*0.024 ≈
1.06`), vs. the adjacent terrain water which is a filled flat quad at the
tile's Z — footprints don't match at the boundary. Compounding it: CC
billboards get `apply_static_world_anchor_translation` = `(+0.5, +1.5)`
(`statics_collect.rs:104-122`, meant for centering **upright** objects)
while EC's flat path uses no translation (`(world_x, world_z)`, line 117)
— so the CC water card's footprint is also offset from the terrain cell it
should align with. **No separate missing-texture bug needed to explain
the seam** — it falls directly out of billboard-vs-flat geometry.

**Important classification gotcha found (matters for any fix):** real
classic shoreline water item IDs (confirmed against real
`tiledata.mul` — `0x1796-0x17B2`, `0x346E+`, `0x3494+`, `0x55F0+`, all
named "water") carry `flags = 0x800C1` = `WET (0x80) | Background (0x1)`.
`static_tile_is_surface_like()` (`statics_collect.rs:212-220`) correctly
returns true for them (WET is set) — that's what routes EC to its ground
quad. **But their `StaticDepthClass` comes out as `Background`, not
`SurfaceLikeFloor`**, because `resolve_static_depth_class`
(`statics_collect.rs:228-252`) checks `TILE_FLAG_BACKGROUND` *before* the
surface-like check. **Any fix must key off `static_tile_is_surface_like()`
directly, not `StaticDepthClass::SurfaceLikeFloor`** — the latter would
silently miss every real water static.

### Proposed fix — NOT YET IMPLEMENTED, awaiting user sign-off

**Option A-b (recommended):** teach the existing CC **sprite** pipeline to
render a flat quad for `static_tile_is_surface_like` CC tiles specifically
(keeps CC art in the sprite atlas — avoids re-routing into the EC
land/ground atlas that `emit_ground_static_instance`/`ground.wgsl`
currently expect). Concretely: (1) at collect time
(`statics_collect.rs`), when `art_source==Cc &&
static_tile_is_surface_like(tilemeta)`, emit with flat `local_min/max =
0..1`, anchor `(world_x, world_z)` (no `+0.5/+1.5`), plus a new
per-instance "flat" flag; (2) `main.wgsl` vertex shader branches on that
flag to build a flat XZ quad (like `ground.wgsl:159-163`) instead of the
vertical billboard; (3) `main.wgsl:243-261` passes `is_ground_art=true`
for those tiles so shading uses the flat-bevel normal and skips the
vertical bottom-occlusion darkening. Blast radius: `statics_collect.rs`
(emit path + one new instance flag) and `main.wgsl` (vertex + one shading
arg). CC-only — EC and non-surface CC statics untouched. Needs
re-verification across all three rendering presets after the change.

**Option A-a (more invasive, not recommended):** route CC surface-like
statics through the existing EC `emit_ground_static_instance` +
`ground.wgsl` path instead. Conceptually cleaner (one flat path shared by
both clients) but requires making that pipeline sample CC art from the
sprite atlas — more binding/atlas plumbing, higher regression risk to the
already-working EC ground path.

**Option B (cheap stopgap, does NOT fix the seam — partial fix only):**
keep the billboard geometry, but for `static_tile_is_surface_like` CC
tiles suppress the object-shading effects (mottle/highlight/shadow, treat
as `is_ground_art=true`) and skip `apply_water_animation`. Removes most of
the "dark dashes" (issue 2) but the vertical-card footprint mismatch
(issue 3) remains. Not recommended as a standalone fix.

**Recommendation: Option A-b.** Minimal, CC-scoped, addresses both symptoms
at the shared root cause, reuses the flat-quad concept EC already proves
out.

### Fix applied (2026-07-03) — Option A-b, user-approved and implemented

Implemented via a dedicated subagent, then independently reviewed
(diff read in full, not just the agent's summary trusted) and
re-verified directly. Four files touched:

- `dynamapper/assets/shaders/world/art/art_bindings.wgsl` — repurposed the
  existing unused `_pad_inst: u32` padding field in `SpriteInstance` into a
  semantic `flat_quad: u32` flag (same type/offset, zero layout/binding
  change — lower-risk than adding a new field).
- `dynamapper/src/core/render/scene/world/art/statics_collect.rs` —
  `SpriteInstance.flat_quad` (renamed from `_pad_inst`); in
  `emit_art_static_tile`, added `cc_surface_like_flat = art_source==Cc &&
  static_tile_is_surface_like(info.tilemeta)` (correctly gated on the
  surface-like predicate directly, NOT `StaticDepthClass::SurfaceLikeFloor`
  — verified this matches the documented Background-flags gotcha above).
  When true: anchor uses `(world_x, world_z)` with no translation, and a
  new `emit_cc_surface_like_flat_static_instance` pushes a flat-quad
  instance (`resolve_surface_like_ground_quad_bounds()` for local bounds,
  ground-style local-light sampling, `flat_quad: 1`, `is_wet_flags`
  preserved so water animation still runs). The existing
  `emit_billboard_static_instance` is untouched — verified byte-for-byte
  identical in the diff, only reached when `cc_surface_like_flat` is false.
- `dynamapper/assets/shaders/world/art/main.wgsl` — vertex shader branches
  on `inst.flat_quad`: flat instances build `(world_x+local.x, world_y,
  world_z+local.y)` — confirmed to exactly match `ground.wgsl`'s existing
  flat-quad construction (`ground.wgsl:157-163`) by direct comparison, not
  just by the implementing agent's say-so. Billboard branch (`else`) is
  the original, unmodified code. Fragment shading's `is_ground_art`
  parameter to `apply_art_surface_shading` changed from the constant
  `false` to `inst.flat_quad != 0u`.
- `dynamapper/src/core/render/scene/world/art/statics_draw.rs` — one
  `_pad_inst: 0` → `flat_quad: 0` rename in an initial-buffer initializer
  (the `TextureFormat` hunk in this same file's diff is unrelated
  pre-existing uncommitted user work, not part of this fix).

**Verification (re-checked independently, not just trusting the
implementing agent's report):**
- Read the full diff directly (`git diff`) for all 4 files — confirmed
  minimal, matches the plan, EC/billboard paths structurally untouched.
- `cargo check --workspace`: clean.
- `cargo test -p dynamapper`: 88 passed, 0 failed.
- `bevy-lint` could not be run in this environment (pre-existing justfile
  parse error unrelated to this change, and the lint driver isn't
  installed here) — not a blocker since the change is data-layout + shader
  logic, no new Bevy systems/queries.

**Not yet visually confirmed in-game** — needs the user's own check with
CC art source selected, across all three rendering presets, confirming
shoreline water now lies flat/seams with terrain and the dark-dash
artifacts are gone while animation still works.

**User tested in-game (2026-07-03): positioning/flatness/seam is fixed,
but the tiles render visibly too small.** Root cause found immediately on
inspection: `emit_cc_surface_like_flat_static_instance` uses
`resolve_surface_like_ground_quad_bounds()` — a hardcoded `0..1` unit
square designed for EC's plain axis-aligned ground textures. CC sprite art
is fundamentally different: it's 2D pixel art from `art.mul` with the
isometric look baked into the pixels (designed for camera-facing
billboards, sized via `resolve_static_billboard_bounds`'s
`CC_WORLD_XZ_PER_PIXEL = sqrt(2)/44` scale, i.e. a 44px sprite gets a
√2≈1.414 world-unit footprint, not 1.0). Reusing EC's fixed 1×1 square
for CC content is a real geometric mismatch (not just "needs to be
bigger"), and there's an open question of whether a rotation (matching
`BILLBOARD_RIGHT_XZ`'s diagonal axes) is *also* needed, or whether scale
alone fixes it. Dispatched a subagent (opus) to work out the correct
transform rigorously (grounded in real decoded sprite pixel data, not
guessing) and implement it.

### Size/orientation fix applied (2026-07-03) — verified independently

Root cause confirmed by decoding real water sprites from `art.mul`
(`0x1797`, `0x346E`, `0x3494`, `0x55F0`): they're 44×44 axis-aligned
diamonds (~52% fill, transparent corners) — the isometric look baked
directly into the pixels, same convention as the billboard path. Scale
alone would not have fixed it — the flat quad also needed the diagonal
`BILLBOARD_RIGHT_XZ`/new `FLAT_QUAD_FORWARD_XZ` basis (not raw world X/Z)
to "undo" that baked rotation, or the diamond content renders doubly-
rotated/collapsed to a small inscribed blob (the reported "too small").

Changes: `art_bindings.wgsl` adds `FLAT_QUAD_FORWARD_XZ = (1/√2, 1/√2)`
(perpendicular to `BILLBOARD_RIGHT_XZ`); `statics_collect.rs` adds
`resolve_cc_surface_like_flat_quad_bounds` (centered bounds scaled by the
existing `CC_WORLD_XZ_PER_PIXEL`, replacing the reused
`resolve_surface_like_ground_quad_bounds()`) and centers the instance on
the cell (`+ HALF_MAP_CELL_WORLD` on both axes); `main.wgsl`'s `flat_quad`
vertex branch now builds the quad on the diagonal basis instead of raw
X/Z.

**Verified independently (traced the math by hand, not just trusting the
report):** with bounds half-extent `h = 44 * 0.5 * CC_WORLD_XZ_PER_PIXEL =
√2/2` and the diagonal basis, the quad *mesh*'s own corners land at
distance 1 from the cell center (extending into transparent padding,
same as ordinary billboards already do) — but the sprite's *visible
diamond content* (inscribed at the UV square's edge midpoints) maps
exactly onto the four corners of the 1×1 map cell. Confirms the fix is
geometrically correct, not just plausible-sounding.

`cargo check --workspace`: clean. `cargo test -p dynamapper`: 88/88 pass.
`git status`: no stray files, only the intended 3 files touched (plus the
already-covered `flat_quad` field rename from the prior H fix).

**Not yet visually confirmed in-game** — needs the user's re-check.

### TODO for next session
- [ ] User to re-confirm in-game (CC art source, all 3 presets): shoreline
      water fills its cell at correct size, seams with terrain, still
      animates.

**User tested in-game again (2026-07-03) and reported 5 new observations
from a fresh screenshot** (a courtyard/shoreline scene). Progress
confirmed (size/positioning look better), but new/residual problems
found:

1. Cursor-inspect overlay text still ran off the edge of the screen in
   some cases (the per-static diagnostic line was a single very long
   line). **Fixed directly** (no subagent needed, small/well-understood):
   `dynamapper/src/core/render/overlays/cursor_behavior.rs`,
   `describe_static_tile_details`'s format string split across 5 shorter
   lines (still every field, just wrapped with `\n` instead of one long
   line), and `describe_static_tiles`'s join separator between multiple
   stacked statics changed from `" | "` to `"\n"` to match. Verified:
   `cargo check -p dynamapper` clean, `cargo test -p dynamapper` 88/88.
2. "statics coords are unaligned with map coords."
3. "static land like tiles do not have 'precedence' on standard static
   tiles, you see that they are z fighting" — the inspector reading in
   the screenshot showed a `surf=true` **roof** tile at `z=60` (elevated),
   which raises a real possibility that the H fix's flat-quad path (meant
   for ground-level water) is incorrectly also catching elevated
   surface-like tiles like roofs/floors.
4. "static water still has the 'boxed shadow' artifact."
5. "water waves static tiles aren't fully aligned with the static water
   and there are some empty spaces between them" — implies a distinct
   "wave"/foam decorative static category that doesn't align with the
   now-flat-quad water tiles.

**Dispatched two parallel subagents** (opus) to investigate/fix, since 2/3/5
look related (anchor-convention and depth-sort consistency across static
categories: billboard vs. the new H flat-quad path) while 4 looks like a
distinct shader/shadow-pass mechanism:
- One covering 2, 3, 5 together, with an explicit instruction to check
  whether the flat-quad predicate (`static_tile_is_surface_like`) is
  over-broad (catching elevated roofs/floors, not just water) as the
  likely root of 3, and whether the flat-quad anchor (`+0.5,+0.5`) vs. the
  pre-existing billboard anchor (`+0.5,+1.5`,
  `EC_STATIC_TILE_TRANSLATION_X/Z`) is a real asymmetry or whether 5 is
  actually a *classification* gap (wave tiles never entering the flat-quad
  path at all).
- One covering 4, focused on whether the shadow-pass depth-class exclusion
  actually covers `Background`-classified water (not just
  `SurfaceLikeFloor`), and/or whether the shadow or static-light-decal
  system uses the flat-quad's full bounding-box geometry (which extends
  into transparent padding, per the H size fix) rather than the alpha-cut
  silhouette, producing a boxy artifact.

Investigation in progress as of this writing — results not yet in.

### Fixes applied (2026-07-03) — both verified independently, not just trusting the reports

**Issues 2 & 3 (coordinate misalignment / z-fighting) — root cause: the
flat-quad gate was over-broad.** Confirmed with real `tiledata.mul` data:
the "slate roof" from the screenshot has flags `SURFACE | Background`
(authored `SurfaceLike`, but **not** `WET`) — it satisfied the old gate
(`static_tile_is_surface_like`, reused from the EC land-redirection path)
and was being incorrectly flattened at its own elevation (z=60) with the
flat-quad's cell-center anchor, instead of staying an upright billboard —
directly causing both the mis-anchoring and the z-fighting against other
statics stacked in the same cell. Real water and "wave" statics (verified:
`0x1796-0x17B2`, `0x346E+`, `0x3494+`, `0x55F0+`, and wave range
`0x1fa3-0x1fca`) **all** carry the `WET` flag, so it's a complete, correct
discriminator. Fix: new CC-specific predicate
`cc_static_tile_renders_flat` (`statics_collect.rs`), gated purely on
`TILE_FLAG_WET`, replacing the reused `static_tile_is_surface_like` at the
CC flat-quad gate only (the EC land-redirection path, which legitimately
needs the broader predicate, is untouched). New unit test
`cc_flat_render_gate_targets_wet_water_not_surface_floors` added and
passing. **Judgment call flagged by the implementing agent, not assumed:**
this scopes flat rendering to water-only — non-wet walkable floors revert
to the pre-H billboard behavior (never reported broken, flat geometry
never validated for them). If flat floors are wanted too, that's a
separate, deliberate follow-up, not implied by this fix.

Issue 2's "wave gap" concern and the `+0.5,+1.5` (billboard) vs `+0.5,+0.5`
(flat-quad) anchor difference were investigated and found **not** to be a
bug needing reconciliation — they're legitimately different conventions
for legitimately different geometry (upright card vs. cell-centered flat
quad), and wave tiles were already going through the flat path (all
carry WET) both before and after this fix, so any *residual* gap after
this fix is more likely per-sprite art sizing, not a classification or
anchor-math bug — flagged as needing the user's visual re-check rather
than guessed at further.

**Issue 4 (boxed shadow) — root cause: unconditional upright-object
shading applied to flat water.** Projected shadows and the static-light-
decal system were both checked and ruled out (shadow pass already
correctly discards for `Background`-classified statics; the decal system
is purely additive point-light, can't produce a dark box). The actual
cause: `apply_art_surface_shading` (in `main.wgsl`'s fragment shader) was
called **unconditionally** for every sprite instance — its `is_ground_art`
argument only gates the fake-normal branch, not the base
lower-occlusion/side-contact-darkening/top-highlight terms, which paint a
bright-top/dark-edge gradient onto any object, flat water included.
**Confirmed this exact class of bug already has an established fix
pattern**: the sibling EC ground path (`ground.wgsl:234`) already wraps
the identical call in `if (!use_ec_water_material)` for exactly this
reason — CC flat water had no equivalent guard. Fix: added
`is_cc_flat_water = inst.flat_quad != 0u && in.is_wet == 1u` and skip the
`apply_art_surface_shading` call when true, mirroring the EC pattern.
Global lighting, atmosphere, hue, water animation, grunge, and color-wash
still apply to flat water afterward, unchanged.

**Verified independently for both fixes** (not just trusting the
subagent reports): read both diffs directly via `git diff`; confirmed
`in.is_wet` and `flat_quad` are real, correctly-used existing fields (not
hallucinated); confirmed the `!use_ec_water_material` EC precedent exists
exactly as claimed. `cargo check --workspace`: clean. `cargo test -p
dynamapper`: 89/89 pass (was 88, +1 new test). `git status`: only the
expected files touched, no stray scratch files, no conflicts between the
two subagents' concurrent edits to `statics_collect.rs`/`main.wgsl`
(different hunks).

Note: WGSL isn't validated by `cargo check`/`cargo test` (naga-oil
validates it at runtime) — the shading-guard fix's syntax was checked by
hand against existing, working uses of the same fields
(`inst.flat_quad`, `in.is_wet`) elsewhere in the same file, but has not
been runtime-compiled in this environment.

### TODO for next session
- [ ] User to re-confirm in-game (CC art source, all 3 presets — including
      at least one fake-normals-on preset like KR to check the shading
      guard doesn't regress there): roofs/floors render as normal
      billboards again (no z-fighting), water/waves still lie flat and
      seam with terrain, no boxed shadow artifact, and check whether any
      wave/water gap remains (would point to per-sprite art sizing, a
      separate follow-up, not a code-logic bug).
- [ ] First real runtime shader compile check for the shading-guard fix
      specifically (confirm no naga-oil validation error).

**User tested again (2026-07-03) — confirmed "way better" overall, but 4
more issues found from a fresh screenshot:**
1. A residual **slight** (not gross) misalignment between ordinary CC
   billboard static art and the terrain grid beneath it.
2. **"Fake normals are creating the 'boxed shadow' artifact on the
   bridge"** — a wooden bridge/dock, classified `Background` via tiledata
   flags but NOT `WET` (so it's an ordinary billboard, never entered the
   H flat-quad path, and is unaffected by the water-only shading-skip
   fix already applied) — shows the same kind of boxed-shadow look the
   water fix addressed, but on a billboard this time.
3. **Water wave animation**: at certain animation frames, part of the
   water texture becomes transparent enough to reveal the land/terrain
   mesh beneath ("brown" showing through) — likely the `apply_water_animation`
   UV-zoom effect (`dynamapper/assets/shaders/world/effects/surface_effects.wgsl`)
   interacting badly with the tightly-fitted flat-quad diamond geometry
   from the H fix (the zoom could push the diamond's alpha silhouette
   outside the fixed quad footprint, or pull in a transparent corner from
   the source sprite).
4. **Fake normals also implicated on plaster walls** — "highlighting the
   vertical margins of the sprite," a different visual symptom from the
   bridge's boxed look but the user attributes both to the same
   `art_fake_normal`/`enable_art_fake_normals` mechanism
   (`dynamapper/assets/shaders/world/art/shading.wgsl`). Own quick read
   found a likely culprit: `side_plane = smoothstep(0.18, 0.5,
   abs(uv_in_tile.x - 0.5))`, which ramps up specifically near the
   horizontal UV edges of a sprite — a strong candidate for "vertical
   margin" highlighting on rectangular wall art, feeding into
   `edge_profile`/`side_falloff_profile` per depth class inside
   `apply_art_surface_shading`.

**Dispatched three parallel subagents** (opus), each scoped to avoid file
collisions:
- One for issue 1 (residual anchor misalignment) — re-examining
  `EC_STATIC_TILE_TRANSLATION_X/Z = 0.5/1.5` from first principles (don't
  assume old-and-untouched means correct, per this thread's own
  established lesson from issue C), plus checking for a possible
  land-grid-vs-static-grid origin disagreement as an alternate cause.
- One for issues 2 & 4 (fake-normal artifacts on billboards) — reading
  `art_fake_normal`/`apply_art_surface_shading` in full, determining why
  `Background` (bridge) and `Regular` (wall) depth classes produce
  different-looking but likely same-mechanism artifacts, and fixing
  without regressing legitimate shading on other object types. Flagged as
  partly a look-dev judgment call — authorized to implement the version
  that most clearly removes the artifact even if conservative, noting if
  further visual tuning may be wanted later.
- One for issue 3 (water animation revealing land beneath) — checking
  whether `apply_water_animation`'s `centre` argument is correctly wired
  for the new flat-quad UV parametrization (most likely direct cause) vs.
  whether the effect's built-in amplitude is simply too strong for the
  now-tightly-fitted diamond geometry; also checking whether water
  animation is exclusively flat-quad now (safe to adjust globally) or
  still shared with a billboard path (would need scoping to flat_quad
  only).

Investigation in progress as of this writing — results not yet in.

### Update (2026-07-04) — session-limit interruption, one fix survived

All three subagents dispatched above hit an API session-limit cutoff
before reporting back (their "results" were just rate-limit stubs, not
findings). Checked the repo directly to see what actually happened:

- **Issues 2 & 4 (fake-normal artifacts) — a real, complete fix landed**
  before the cutoff, in `dynamapper/assets/shaders/world/art/shading.wgsl`:
  two new named constants `ART_UPRIGHT_BILLBOARD_EDGE_PROFILE = 0.0` and
  `ART_UPRIGHT_BILLBOARD_SIDE_FALLOFF_PROFILE = 0.0`, replacing the
  previous `edge_profile = side_plane * 0.08` /
  `side_falloff_profile = 0.52` in the `!is_ground_art` branch of
  `apply_art_surface_shading` (the branch covering ordinary upright
  Regular/Background billboards — walls, bridges, docks, furniture).
  Well-commented, explains that both the bridge's boxed-shadow and the
  wall's vertical-margin highlight trace to the same smoothstep-edge terms
  (any nonzero value produces a visible frame around the sprite
  silhouette, so removed structurally rather than tuned down), and
  explicitly notes it erred conservative (edge polish fully removed,
  re-tune upward later if wanted) — Lambertian directional shading is kept
  intact. Verified independently: `cargo check --workspace` clean,
  `cargo test -p dynamapper` 89/89 (unchanged, this is WGSL-only). WGSL
  syntax checked by hand against surrounding working code — looks sound,
  not runtime-compiled in this environment.
- **Issue 1 (coordinate misalignment) — zero progress**, no code touched
  (`EC_STATIC_TILE_TRANSLATION_X/Z` unchanged). Re-dispatched fresh.
- **Issue 3 (water animation revealing land) — zero progress**, no code
  touched (`surface_effects.wgsl` unchanged). Re-dispatched fresh.
- No stray scratch files left behind by any of the three; `git status`
  clean apart from the one real fix above.

### Update (2026-07-04) — issue 3 (water animation) investigated, ruled out, no fix applied

Re-dispatched subagent reached high confidence the water-UV animation
mechanism is **not** the cause, and correctly made **no code change**
rather than fabricate one. Verified independently: `git diff` on
`surface_effects.wgsl` and the water-animation lines of `art/main.wgsl`
is empty — confirmed clean, matches the claim.

Evidence: decoded the real water sprites from `art.mul` — each is a
diamond centered exactly at (21.5, 21.5) in its 44×44 box, tips at the
exact UV edge-midpoints (matches issue H's original find). Confirmed
`get_water_animation_scale()`'s components are provably always ≥1 (a pure
zoom-toward-center, never zoom-out), and ran a full 360° numerical
simulation against the real decoded alpha: **0.00% in-cell transparent
fragments at every animation frame** — the opaque diamond always fully
covers the 1×1 cell with margin to spare, both with nearest and bilinear
sampling at the real alpha cutoff. Also confirmed water animation is
*shared* across CC billboard, CC flat-quad, EC ground, and land/terrain
water paths (not flat-quad-exclusive), so a global amplitude change would
have been risky regardless — moot since no change is warranted.

**Real suggested culprit, out of this subagent's scope, worth
investigating next:** depth/z-fighting between the flat water quad
(`Background` depth class, small negative logical-depth offset) and the
terrain water mesh beneath it — a per-fragment depth tie could let the
brown terrain win through at certain camera/animation phases, which would
look exactly like "a specific frame reveals brown" even though the actual
mechanism is a depth-sort interaction, not a UV/alpha hole. This has
notable overlap with issue 1's investigation (still in progress) and with
the general z-fighting theme from the previous round (issue 3 there,
already fixed for the roof/billboard case) — worth checking whether the
*water quad itself* vs. *terrain mesh* (not object vs. object) has a
similar insufficient-separation problem.

### Update (2026-07-04) — issue 1 (coordinate misalignment) fixed, verified independently

Root cause confirmed and fixed: ordinary CC billboard statics used the
same anchor translation `(+0.5, +1.5)` (`EC_STATIC_TILE_TRANSLATION_X/Z`)
as EC billboards, but CC sprites are force-centered in their bounding box
(`draw_offset_x = -(width/2)`, from issue C) while EC sprites carry their
own real per-sprite offsets — the two need different, non-interchangeable
anchor conventions. Derivation (independently re-verified by hand): in the
billboard's diagonal screen basis, a world translation `(tx, tz)` shifts
the sprite's on-screen horizontal position by `(tx - tz)/√2`. For the old
`(0.5, 1.5)`: `(0.5-1.5)/√2 ≈ -0.707` — a half-tile horizontal drift,
confirmed mathematically. A translation is only horizontally centered
when `tx == tz`; the specific value `(1.0, 1.0)` was chosen because it
preserves the *same* forward/depth component as the old value
(`(tx+tz)/√2 = √2` in both cases), so nothing that already looked
correct vertically/depth-wise regresses — only the horizontal drift is
removed. Verified this decomposition arithmetic myself independently, it
checks out exactly.

Fix: new CC-specific `CC_STATIC_TILE_TRANSLATION_X = CC_STATIC_TILE_TRANSLATION_Z
= 1.0` and `apply_cc_static_world_anchor_translation`, with
`surface_like_static_world_anchor` now branching `CcRegular` (new,
symmetric anchor) vs. `EcRegular` (unchanged `0.5/1.5`, left alone since
EC billboards were reported fine and use a different offset convention
entirely) — scoped correctly, doesn't touch EC.

**Verified independently:** re-read the diff directly (only the new
constants/function/branch, everything else in the file's diff is the
already-reviewed prior H-fix work); confirmed the existing anchor test
was updated to assert the new CC constants (not a fabricated pass);
`cargo check --workspace` clean; `cargo test -p dynamapper` 89/89
(unchanged count — existing test updated in place, not duplicated).

Second possible source (land-grid vs. static-grid origin disagreement)
was checked and ruled out — both systems already agree on integer
cell-origin convention; independently corroborated by the fact that the
flat-quad water fix (issue H) already seams correctly with terrain using
a `(+0.5,+0.5)` cell-center anchor.

**One open uncertainty flagged by the implementing agent, worth watching
for on re-test:** the choice of `(1.0, 1.0)` (tile's *near vertex* as the
sprite's feet-anchor) versus the alternative `(0.5, 0.5)` (tile *center*,
matching the flat-quad convention) rests on an assumption not verifiable
from code alone — if walls now look correctly aligned sideways but appear
pushed slightly forward/down-screen off their tile (feet past the near
edge), that would indicate `(0.5, 0.5)` is actually the better target, a
small follow-up fix.

### Consolidated status after this round of fixes (2026-07-04)

Three things fixed and independently verified this round: fake-normal
edge artifacts (bridges/walls), the CC billboard anchor misalignment, and
a negative-but-valuable result ruling out water-animation UV as the cause
of the brown-through-water report (redirects that investigation toward a
water-quad-vs-terrain depth-sort interaction instead, not yet chased
further — worth doing only if the user confirms it's still visible after
this round).

### Update (2026-07-04) — completeness gap found in the fake-normal fix, fixed

User caught this by re-inspecting a fresh screenshot: "persisting shadow
issue for floor/background flat (not necessarily diamond shaped
land-like) static tiles on the bridge." The earlier fake-normal fix
(above) only zeroed the edge-frame terms in the generic `!is_ground_art`
branch of `apply_art_surface_shading` (`shading.wgsl`) — but there's a
**separate, earlier-checked branch** for `depth_class ==
ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR` that has its own untouched
`edge_profile = side_plane * 0.35`. A tile like a bridge deck (SURFACE
tiledata flag, but not WET, so it never enters the flat-quad path and
`is_ground_art` is false) classifies `SurfaceLikeFloor` and hits this
branch as an ordinary upright billboard — same spurious edge-frame
artifact, missed by the first fix. Fixed: that branch's `edge_profile` is
now `select(side_plane * 0.35, ART_UPRIGHT_BILLBOARD_EDGE_PROFILE,
!is_ground_art)` — zeroed for the billboard case, left as originally
designed for the (untested, likely-never-hit-in-practice) case of a truly
flat SurfaceLikeFloor instance, since real CC water classifies
`Background` (checked earlier in `resolve_static_depth_class`), not
`SurfaceLikeFloor`. `side_falloff_profile` didn't need touching — this
branch never set it away from its `0.0` default. Verified: `cargo check
--workspace` clean, `cargo test -p dynamapper` 89/89 (WGSL-only change,
Rust side unaffected).

### New issue found (2026-07-04) — sawtooth/z-fighting at static-static and static-map boundaries

User also reports: **"sawtooth artifacts when statics collide (water and
bridge wall) and map and statics colliding (bottom right)."** Two
distinct collision types, both showing a jagged/sawtooth pattern — the
classic visual signature of insufficient depth separation (z-fighting)
between two surfaces at nearly the same depth:
1. Static-vs-static: the flat water quad meeting the bridge's stone wall
   (an upright billboard) at their shared boundary.
2. Static-vs-map: a static (likely the flat water quad again, or a
   shoreline static) meeting the terrain/land mesh itself, visible in the
   bottom-right of the screenshot.

**This was not chased further in this session** — it's a real depth-sort
investigation (similar magnitude to the earlier successful "roof z-fights
with billboards" fix, but for a different pairing: flat water quad vs.
adjacent billboard walls, and flat water quad vs. terrain mesh) and has
been added to the dedicated handoff document
(`docs/agent_notes/water_animation_wave_tile_handoff.md`) as a strong
**candidate unifying cause** for the still-unresolved "brown showing
through water on some animation frames" report — a previous subagent
speculated exactly this (water-quad-vs-terrain depth interaction) as an
alternative to the UV-animation theory, and this fresh sawtooth
observation is independent supporting evidence for that same theory. See
that handoff doc for full detail; this is the strongest lead for a new
session to start with.

### TODO for next session
- [ ] Hand off to a new session (user's request) using
      `docs/agent_notes/water_animation_wave_tile_handoff.md`, now updated
      to include the sawtooth/z-fighting findings as a likely-unifying
      lead for the water animation issue.
- [ ] User to re-confirm in-game (CC art source, all 3 presets) once that
      new session's work lands:
      - Walls/docks/bridges: base edge sits centered on its cell, not
        shoved sideways (watch for the near-vertex-vs-center uncertainty
        noted above).
      - No boxed shadow on bridges, no vertical-margin highlight on walls,
        and now also no shadow artifact on bridge decks/floors specifically
        (today's completeness-gap fix).
      - Sawtooth/z-fighting at water-vs-wall and water-vs-terrain
        boundaries resolved.
      - Whether "brown showing through water on some frames" persists.

### Update (2026-07-04, follow-up session) — sawtooth/brown-bleed AND wave misplacement both fixed

Both remaining items from `water_animation_wave_tile_handoff.md` were
resolved in one session (two parallel subagents, both outputs verified
independently — diffs read directly, math re-checked, tests re-run). Full
technical detail is in that handoff doc's two new RESOLVED sections; short
version:

1. **Sawtooth + "brown showing through water on specific frames" — one
   depth-sort bug, fixed in `art/main.wgsl`.** Every sprite wrote a
   per-instance *constant* `frag_depth` (fine for upright billboards,
   wrong for a flat quad coplanar with the terrain: the terrain's real
   per-fragment depth varies ~2000× more across a tile than the
   Background-class logical offset, so terrain won the depth test over a
   moving diagonal band — the sawtooth line and the sweeping brown band).
   Flat quads now write their own rasterized per-fragment depth
   (`out.depth = in.position.z`); since the quad sits `STATIC_ART_Y_BIAS`
   (0.002 world-Y) above the terrain, it now beats it by a uniform,
   zoom-independent margin at every fragment. Confirms the earlier
   0%-alpha-hole simulation was right: the UV animation was never the
   cause, and the inflated-amplitude lead is moot.
2. **Wave-tile misplacement — flat-quad anchor silently assumed h == 44,
   fixed in `statics_collect.rs`.** Decoded all 40 real `wave` sprites
   (`0x1fa3-0x1fca`): none is a centered diamond; the ~89px breaking-wave
   set was displaced ~one cell diagonally by the hardcoded cell-center
   anchor. New height-aware `cc_surface_like_flat_quad_center_offsets`
   (per-axis `1 - h/88`, feet at the near vertex like the validated CC
   billboard convention; identical to the old behavior at h=44, so plain
   water is unaffected by construction) + unit test.

Verified: `cargo check --workspace` clean; `cargo test -p dynamapper`
**90/90** (was 89, +1 new test). WGSL hand-checked (naga-oil validates at
runtime only).

### TODO for next session
- [ ] User in-game re-check (CC art source, all 3 presets): no sawtooth at
      water/wall and water/terrain boundaries; no brown bleed-through at
      any animation frame/camera position; breaking waves sit on the
      shoreline (not one cell up-screen); plain water still seams and
      animates as before.
- [ ] Everything else (commit plan, draw-offset caveats study, smaller
      leftovers) is consolidated in the dedicated handoff:
      `docs/agent_notes/flat_quad_water_wave_followups_handoff.md`
      (2026-07-06).

## Session log

- **2026-07-02**: Explored all 6 issues, mapped each to files/functions,
  ranked severity. Investigated A in depth: ruled out the newer atlas-packing
  code, confirmed the root cause is the old `TEXMAP_MAX_ID = 0x1388` cap in
  `lib/uocf/src/classic/land_texture.rs`, verified against real client data
  on disk (`/mnt/dati/_proj_local/_uo_clients/`) — 702 valid land textures
  (~27%) silently dropped for the modern `Classic_7_0_20_0` client.
- **2026-07-02 (same session, continued)**: Applied the fix — swapped the
  hardcoded cap for `texidx.element_count()`. Verified with
  `cargo check --workspace` and a real end-to-end run of `udd-cli pack
  pack-texmaps` against the modern client tree, confirming all 2647 valid
  textures (including the 702 previously dropped) are now read and packed.
  Issue A is resolved pending the user's own in-game re-conversion/check.
- **2026-07-02 (continued)**: Dispatched two sequential focused subagents
  for B/C. First derived camera projection geometry — found `0.1` (land
  height scale) is NOT clearly wrong (falls inside a derived plausible
  range), downgrading B to a user taste/design question rather than a bug.
  Second tested a transparent-padding hypothesis for C with real decoded
  client data — refuted it, but found the actual bug while tracing offset
  consumption: CC's `draw_offset_y = -height` double-counts sprite height
  against `resolve_static_billboard_bounds`, floating every CC static
  object above ground by its own height. User confirmed the fix; applied,
  tested (88/88 unit tests pass, workspace check clean), and the real
  `tex_art_cc.uddp` asset regenerated. Also recorded canonical CC/EC client
  source directories to memory (see reference_uodynamapper_client_source_dirs).
  Pending: user's in-game visual confirmation of the C fix. B still needs a
  user decision (classic-faithful vs. exaggerated height) before any further
  action — not a code task.
