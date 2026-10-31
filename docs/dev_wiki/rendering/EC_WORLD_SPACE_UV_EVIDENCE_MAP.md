# EC World-Space UV — Evidence Map and Data-Model Clarification

**Phase 11 artifact.** Purpose: prevent future implementation phases from treating the
third-party proof-of-concept renderer as original EC source, and make explicit what is
proven, what is conceptually aligned, and what remains open.

---

## What the third-party report is and is not

The shader files at
`UOClient-stefanomerotta/UOClient/Content/shaders/{single-terrain,solid-terrain,liquid-terrain,statics}.fx`
are a **third-party best-guess reconstruction**, not original Ultima Online Enhanced Client
source code.  They are useful design references because they encode plausible architectural
decisions inferred from EC behaviour, but they carry no authority over original EC shader
semantics.

**Do not cite these files as proof of original EC behaviour.**  When they are used in this
project, they must be explicitly labelled as third-party reconstruction evidence.

The evidence tiers in force for this codebase are:

| Tier | Source | Authority |
|------|--------|-----------|
| 1 — Primary | Original UOP package contents, string_dictionary-resolved paths | Authoritative for what data exists |
| 2 — Secondary | Extracted original shader text from Shaders.uop (partial) | Authoritative for confirmed shader families |
| 3 — Reconstruction reference | Third-party FX files | Design clue only; not original source |
| 4 — Current engine | UODynamapper Rust parsers, WGSL shaders | Authoritative for current behaviour, not original EC intent |

---

## Concept mapping into the current data model

### The third-party "Stretch" parameter

The solid-terrain and liquid-terrain FX files use an explicit integer `Texture0Stretch`,
`Texture1Stretch`, `AlphaMaskStretch`, and `NormalStretch` parameter passed per draw call.
The vertex shader divides world-position XZ by this integer to produce UV coordinates.

```hlsl
// solid-terrain.fx — illustrative excerpt (third-party reconstruction)
float2 xy = position.xz / Texture0Stretch;
float2 zw = position.xz / Texture1Stretch;
```

This concept maps onto two separate signals in UODynamapper's data model:

| Third-party concept | UODynamapper field | Location | Status |
|---------------------|--------------------|----------|--------|
| `Texture0Stretch` (per draw call) | `texture_repetition` | `TerrainDefinition::TerrainMaterialLayer` in `lib/uocf/src/enhanced/terrain_definition.rs:46` | **Decoded and preserved** |
| `texture_stretch` (per tileart ref) | `texture_stretch` | `tileart::TextureItem` in `lib/uocf/src/enhanced/tileart.rs:117,246` | **Decoded and preserved** |
| Derived from texture pixel width | `texture_extent.x / CC_TILE_PX` | WGSL in `dynamapper/assets/shaders/world/land/sampling.wgsl` | **Current runtime behaviour** |

**Conceptual alignment status:** The Stretch concept in the third-party reconstruction
*aligns conceptually* with both `texture_repetition` (terrain) and `texture_stretch`
(tileart).  However, it is **not yet proven** that these values map one-to-one to the
third-party parameter, or that the unit conventions are identical.

---

## Current UODynamapper behaviour

### How stretch is derived at runtime

UODynamapper currently derives the world-space stretch value from the texture's pixel
width, **not** from the decoded `texture_repetition` or `texture_stretch` metadata fields
(except when the latter is explicitly non-zero):

```wgsl
// sampling.wgsl — actual current behaviour
fn ec_world_uv(world_xz: vec2<f32>, tile: TileUniform) -> vec2<f32> {
  let tile_w = max(f32(tile.texture_extent.x), 1.0);
  // Uses tile.texture_stretch only when it is > 0; otherwise falls back to extent-derived.
  let stretch = select(tile_w / CC_TILE_PX, tile.texture_stretch, tile.texture_stretch > 0.0);
  return fract(world_xz / stretch);
}
```

The constant `CC_TILE_PX = 44.0` is the Classic Client isometric tile width in pixels,
used as the world-unit denominator.  A 44-px EC texture maps to stretch = 1 (one
repetition per tile); a 176-px texture maps to stretch = 4 (one repetition per four
tiles), and so on.

The same fallback-first pattern applies to the aux-lookup slot reader for secondary-base
and ripple layers:

```wgsl
fn ec_slot_world_uv(world_xz: vec2<f32>, slot: LandLookupSlot) -> vec2<f32> {
  let tile_w = max(f32(slot.texture_extent.x), 1.0);
  let stretch = select(tile_w / CC_TILE_PX, slot.texture_stretch, slot.texture_stretch > 0.0);
  return fract(world_xz / stretch);
}
```

**This is a current behaviour fact, not a confirmed bug.**  Visuals may look correct for
textures whose physical size matches the intended world-space repetition.  However, the
current behaviour diverges from the third-party model in a measurable way: the FX files
use an explicit per-material integer parameter, not a pixel-width heuristic.

### Dual-signal inventory

Both signal families are already partially present in the pipeline:

| Signal | Where preserved | Notes |
|--------|-----------------|-------|
| `texture_repetition` (decoded) | `TerrainMaterialLayer.texture_repetition: f32` | Parsed from TerrainDefinition.uop layer records |
| `texture_stretch` (decoded) | `TextureItem.texture_stretch: f32` in tileart | Parsed from tileart.uop linked texture blocks |
| Texture pixel extent | `TileUniform.texture_extent` / `LandLookupSlot.texture_extent` | Stored in packed UDDP lookup textures |
| `texture_stretch_q8` | `LandLookupSlot.texture_stretch` (fixed-point) | Packed into lookup texture; used by `ec_slot_world_uv` |

**Rule for future phases:** preserve both decoded repetition values and actual texture
extents as separate, comparable signals.  Do not discard one in favour of the other before
a systematic comparison against client visuals or stronger evidence establishes the correct
precedence rule.

---

## What the third-party shaders suggest architecturally

The following points are *reconstruction inferences*, not proven original EC facts:

| Observation from FX files | Inference status |
|---------------------------|-----------------|
| `single-terrain.fx` passes world-space XZ directly as UV (`input.Position.xz`) | Conceptually aligned — UODynamapper uses the same approach |
| `solid-terrain.fx` uses explicit integer Stretch per texture, not pixel-size heuristic | **Diverges from current UODynamapper behaviour** — worth investigating |
| `solid-terrain.fx` uses Texture0 + Texture1 + AlphaMask (three inputs) | Already partially implemented in UODynamapper (Phase 10D); stretch handling unverified |
| `liquid-terrain.fx` uses `Texture0Stretch` and `NormalStretch` as separate integer parameters | Decoded TerrainDefinition `texture_repetition` per layer is a plausible match; not proven identical |
| `liquid-terrain.fx` `WaveHeight`, `WindDirection`, `WindForce`, `Time`, `Center` parameters | Superseded: UOSA.exe `UOWaterTerrainLayer::Draw` scrolls item 0 by `time × tweaks 0/1` (seconds), which the renderer follows; the FX parameters are not read |
| `liquid-terrain.fx` `FollowCenter` vs `Main` technique | Superseded: the client has no follow-center flag, and the old `TERRAIN_FLAG_FOLLOW_CENTER` bit is removed |

---

## What remains unproven

The following questions are explicitly open.  Future implementation phases must not assume
they are resolved unless new evidence is found:

1. **One-to-one stretch equivalence**: Whether `TerrainDefinition.texture_repetition` and
   `tileart.texture_stretch` map directly to the FX `*Stretch` integer parameters in the
   same units.  The FX files use world-position XZ directly; UODynamapper uses a
   CC_TILE_PX denominator.  The unit system may differ.

2. **Terrain vertical scaling**: The FX files use `position.xz` (horizontal plane) only.
   UODynamapper's world-position model already uses `world_position.xz` in the same way,
   but the exact height-to-world-unit scale has not been cross-validated against EC
   visuals.

3. **Pixel-width heuristic vs explicit repetition**: Whether the extent-derived stretch
   (`texture_extent.x / 44.0`) is a correct approximation, an overfit coincidence, or
   systematically wrong for some material families.  Visual comparison against a set of
   known-correct EC screenshots is needed.

4. **Art texture_stretch semantics**: `tileart.texture_stretch` is parsed and threaded
   into `TileMetaItemTextureRef.texture_stretch`, but its exact intended semantics in the
   context of art shaders (billboard art vs surface-like art) has not been validated.

5. **NormalStretch vs Texture0Stretch in liquid**: Whether the EC always used different
   Stretch values for the base and normal layers, or whether they typically shared one
   value.  TerrainDefinition stores one `texture_repetition` per layer record, but it is
   unclear how those map to multi-input liquid materials.

---

## Rules for future implementation phases

1. **Cite evidence tiers explicitly** when claiming a parameter or behaviour matches the
   original EC.  "The FX file does X" is tier-3 evidence; "the UOP data contains Y" is
   tier-1 evidence.

2. **Do not collapse the dual-signal model** before the comparison is done.  Keep decoded
   `texture_repetition`/`texture_stretch` and texture pixel extents separately available.

3. **If changing stretch derivation**, justify it against both the decoded metadata and
   known-correct visual families, not only against the FX files.

4. **FollowCenter** has no client counterpart and its terrain flag was removed. Water
   scroll follows `UOWaterTerrainLayer::Draw` (item 0 translated by time × tweaks 0/1).

5. **Liquid NormalStretch** may eventually need to be a separate per-layer stored value
   rather than shared with the base stretch.  Design for that possibility before hardcoding
   a single stretch into multi-layer liquid materials.

6. **Art-owned liquid** (Phase 10F domain) uses billboard geometry, not terrain quads.
   World-space UV derivation for art shaders is a different problem from terrain UV
   derivation, even if the "Stretch" concept looks similar.

---

## Summary

| Claim | Status |
|-------|--------|
| Third-party FX files are original EC source | **False** — reconstruction only |
| World-space UV with stretch ≈ extent / CC_TILE_PX is current UODynamapper behaviour | **True** |
| Decoded `texture_repetition` and `texture_stretch` are available in the data model | **True** |
| Those decoded values map one-to-one to the FX `*Stretch` integer parameters | **Not proven** |
| `FollowCenter` terrain variant is already tracked | **True** |
| Terrain vertical scaling is validated against client visuals | **Not proven** |
| Pixel-width heuristic is always correct | **Not proven** |
