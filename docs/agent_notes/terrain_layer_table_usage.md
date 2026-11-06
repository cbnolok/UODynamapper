# Enhanced terrain layer table: what the renderer actually uses

Investigated on 2026-11-06 at f369f149, with the canonical packages in
`/home/claudio/test/uddp/` (`tex_land_ec.uddp`, `tex_aux_ec.uddp`). Line numbers
refer to that commit.

## Summary

The terrain layer table (`metadata/terrain_layers.bin` in `tex_land_ec.uddp`,
`TexLandEcPackage::terrain_layers()`) feeds the land material's bindings 113
`ec_aux_atlas` and 114 `ec_aux_lookup`. **No tile reaches the shader code that
reads them.** The two-base blend runs on 0 tiles and the ripple on 0 tiles. The
packed land-page roles (BASE, DETAIL, MASK, NORMAL, ANIMATED, LIQUID_PARAMS)
draw what the client draws instead, so the four Enhanced views render the same
pixels with or without the table.

| Kind | Verdict | Reached |
|---|---|---|
| Secondary base (ColourB) | Correctly redundant; dead in practice | 0 tiles |
| Blend mask (BlendAlpha) | Correctly redundant; dead in practice | 0 tiles |
| Ripple (Distortion) | Dead code, unreachable by construction | 0 tiles |
| Liquid flag (aux `w`) | Same value as `REVIEWED_LIQUID`; read only by the dead ripple | — |
| Liquid parameters | Not table-driven: land-page role 5 comes from the client's techniques and tweaks, and is used | 209 liquid tiles |

None of these is a live Dynamapper bug. The dead paths contain guessed numbers
and latent bugs (listed below); they would be wrong if they ever ran. The
recommendation is to remove them (last section).

## The table and its key

`udd-conv` builds the table (`lib/udd-conv/src/metadata/terrain_meta.rs:188-248`)
keyed by `TerrainDefinitionEntry::runtime_slot_ids()`
(`lib/uocf/src/enhanced/land/terrain_definition.rs:516-529`): each alias's nonzero
`legacy_land_id`, or the material id when every alias has 0. The reader's
`layers_for_tile` returns the first span for a key (`lib/udd-assets/src/metadata/terrain_meta.rs`).

The client never reads `legacy_land_id`
(`scripts/dev-helpers/ec-research/ghidra-terrain/results/uosa_terrain.md:86-88`,
high confidence for "unused"). It routes a classic land id through
`legacyterrainmap.csv` to `(material, subtype)`. Dynamapper's runtime does the same
(`TexLandEcPackage::resolve_material_decision`, `lib/udd-assets/src/textures/tex_land_ec.rs:812`).
So the table can name a different material from the one the client draws:

- 239 keys have layers (245 spans; 6 keys are owned by more than one entry).
- The table's first owner equals the runtime material on 182 keys and differs on 57.
  Those 57 are exactly the keys whose ColourB differs from the runtime
  material's item 1 (45 keys, mask the same) or whose ColourB and BlendAlpha
  both differ (12 keys: 72, 74, 75, 103, 109-112, 125-128).

## Code paths per kind

Rust builds the aux lookup in `create_aux_atlas_images`
(`dynamapper/src/core/texture_cache/land.rs:245`). The table loop runs over tile
ids 0..16383 (land.rs:271-350) and needs both the table and `tex_land_ec`. One
`Rgba32Uint` texel per tile id:

- `x` = secondary layer | stretch_q8 << 16, from `secondary_base_texture_id`
  (land.rs:132, role ColourB, item 1 of a default or bump layer);
- `y` = mask layer, from `alpha_mask_texture_id` (land.rs:137, role BlendAlpha,
  item 2). Its stretch is computed and dropped (`_mask_rep_q8`, land.rs:307);
- `z` = ripple layer | stretch_q8 << 16, from `ripple_texture_id` (land.rs:142,
  role Distortion, water item 0);
- `w` = is_liquid, from the runtime material's layer class, **not** from the
  table (land.rs:279-284).

All three role functions go through `terrain_layer_texture_with_role`
(land.rs:119). A texture absent from `tex_aux_ec` gets layer 0
(`.map_or(0, ...)`, land.rs:290-293). `tex_aux_ec` packs only the
tileart-referenced WorldArt textures, so most ColourB textures get layer 0.
The bindings are declared in `dynamapper/src/core/render/scene/world/land/mesh_material.rs:75-82`
and `dynamapper/assets/shaders/world/land/land_bindings.wesl:87-89`, and assigned at
land.rs:1606-1607. The comment at land_bindings.wesl:88 is stale: it says `z`
and `w` are 0.

### Secondary base and blend mask

`apply_ec_aux_two_base_blend` (`dynamapper/assets/shaders/world/land/sampling.wesl:264-294`),
called from `shade_land` (`main.wesl:628-631`) only when
`texture_size == 2 && !use_ec_liquid`:

1. returns unchanged for a reviewed liquid (`terrain_flags & 0x4`) or when the
   tile has packed DETAIL **and** MASK roles (sampling.wesl:271-275);
2. returns unchanged when `aux.secondary_layer == 0` (sampling.wesl:277);
3. otherwise samples the secondary and the mask from the aux atlas and returns
   `mix(secondary, base, mask_alpha)`.

The client-faithful replacement is `sample_ec_material_albedo`
(sampling.wesl:296-311): `mix(detail, base, mask.a)`, each item at its own
stretch (`ec_slot_world_uv`). The DETAIL/MASK roles are written by
`create_land_shader_images` from `resolve_material_render_layers`, and only
when render layer 0 has a runtime slot (`has_base_layer`, land.rs:1026-1050).

### Ripple

`main.wesl:646-659`. The outer guard is
`texture_size == 2 && enable_water == 1 && !use_ec_liquid`. Inside,
`tile_is_liquid = reviewed_liquid || aux.is_liquid == 1`, then the ripple needs
`tile_is_liquid && ripple_layer != 0 && !ec_liquid_normal`. It re-samples the
solid albedo through `sample_ec_ripple_normal` (sampling.wesl:224-248) with
`RIPPLE_UV_STRENGTH` 0.08.

`use_ec_liquid = texture_size == 2 && (is_wet || reviewed_liquid)` (main.wesl:514).
`aux.is_liquid` (land.rs:279) and `REVIEWED_LIQUID`
(`coverage::ec_land_shader_flags`, `dynamapper/src/core/texture_cache/land/coverage.rs:51-62`)
are the same computation: the runtime material is the water layer class.
Inside the outer guard `reviewed_liquid` is false, so `tile_is_liquid` is false
too, and the ripple cannot run. This holds for any data, not only these
packages. The composition path builds its layer tiles with the same flags
(`ec_layer_tile`, `coverage.wesl:37-46`: wet = water class), so it does not
change this.

The client-faithful water is `sample_ec_liquid` (`dynamapper/assets/shaders/world/ec_liquid.wesl:98`):
packed roles BASE (item 1), NORMAL (item 0, du/dv), ANIMATED (item 2), and role
5 parameters.

### Liquid parameters

Land-page role 5 (`LAND_PAGE_LOOKUP_ROLE_LIQUID_PARAMS`, land.rs:1011-1024):
technique 0 and tweaks 0 and 1 of the runtime material from
`source_materials.json`, `w = LIQUID_PARAMS_FROM_SOURCE`. The table is not
involved. All 209 liquid tiles have NORMAL, ANIMATED and parameters.

## Census

Method: a temporary `#[ignore]` test (not committed) called the real
`create_land_shader_images` and `create_aux_atlas_images` on the packages above,
read the lookup images back and evaluated the shader predicates for every tile
id 0..16383. A second pass compared the packed roles of every Enhanced tile with
its runtime material's TerrainDefinition items.

Aux lookup: 434 tiles with an entry: 57 secondary, 225 mask, 8 ripple,
209 `is_liquid`. `is_liquid` equals `REVIEWED_LIQUID` on all 16384 tile ids.

| Path | Tiles reached |
|---|---|
| Two-base blend (past both early returns) | **0** |
| Ripple | **0** |

- The 57 secondary tiles all have packed DETAIL and MASK, so step 1 returns.
  Their packed DETAIL is the same texture and stretch as the table's ColourB on
  all 57 (for example 3, 22, 25, 117, 119, 268, 282, 433, 561, 581, 586, 1078,
  1082, 1086, 1150, 1152, 1158, 1257, 2023, 10069, 10171-10189, 15809-15848, 16376).
- The 8 ripple tiles (168, 500, 10010, 11790, 11834, 11835, 11860, 16368) are
  all reviewed water, so they take `sample_ec_liquid`. Their packed NORMAL is the
  table's Distortion texture.
- The other 168 mask-only tiles have no secondary, so step 2 returns.

### Packed roles against the table (question 3)

Over the 226 solid Enhanced tiles that the table or the aux lookup covers:

| | Same texture and stretch | Different | Only in the table |
|---|---|---|---|
| ColourB vs packed DETAIL | 169 | 51 | 6 |
| BlendAlpha vs packed MASK | 214 | 6 (same id, other stretch) | 6 |

All 57 mismatches are the 57 keys whose table owner is not the runtime material.
Against the runtime material's own items, the packed DETAIL and MASK match on
all 226 tiles (181 with the pair; 45 where the pair is absent, see below). For
the 13 liquid keys in the table, Distortion = NORMAL, FallbackColour = BASE and
MainColour = ANIMATED on all 13.

So where the table and the packed roles disagree, the packed roles follow the
client's routing and the table does not.

### Tiles the client blends without packed roles (question 4)

There are 2758 Enhanced tiles: 209 liquid and 2549 solid (2462 default, 77 bump,
10 without a material). 2456 solid tiles have the packed pair. The 93 without it:

- 10 have no material and a direct slot: 103, 109-112, 128, 16380-16383.
  There are no client items to blend. (Their routing is outside this question.)
- 83 belong to 19 materials whose item 0 and item 1 are the same texture at the
  same stretch: 15 (604, 616, 702, 710, 10015), 44 (10044), 45 (467-474,
  2028-2033, 2100-2105, 2241-2246, 10045), 52 (76-102, 10052), 154 (475, 10154),
  156-159 (10156-10159), 164-167 (1297-1300, 10164-10167), 170 (430, 580, 10170),
  196 (504), 197-200 (16109, 16110, 16111, 16113). The client's
  `item0 · a + item1 · (1 − a)` is then item 0 alone, so drawing the base alone
  is equivalent.

No tile the client blends from two different textures lacks the packed pair.

## Client or guessed, per parameter

Client facts are from
`scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md`
(verified in Ghidra): roles by position, `SetupFourStages` 0x005974f0 with
`a` = item 2's sampled alpha, UV scale `1 / stretch` per item (0x004613b0),
`UOWaterTerrainLayer::Draw` 0x005988a0 scrolling item 0 by `t · tweak0, t · tweak1`
in seconds with an identity BUMPENVMAP matrix.

| Parameter | Source | Confidence |
|---|---|---|
| Secondary texture (ColourB, item 1) | Client role, but keyed by `legacy_land_id`: wrong material on 57 of 239 keys | High |
| Secondary stretch | Client item stretch, but sampled over the whole 256-px aux layer, ignoring the texture's own extent (`sample_ec_aux_atlas_layer`, sampling.wesl:250) | High |
| Mask texture (BlendAlpha, item 2) | Client role; factor = alpha (`ec_material_mask_value`) matches the client | High |
| Mask stretch | **Wrong**: uses the secondary's stretch (sampling.wesl:287); the client uses the mask item's own (16 or 32 here) | High |
| 50/50 blend without a mask (sampling.wesl:283-289) | **Guessed**; nothing like it in the client | High |
| Blend formula `mix(secondary, base, a)` | Client (`SetupFourStages`) | High |
| Ripple texture (Distortion, item 0) | Client role | High |
| `RIPPLE_SCROLL_A/B` (0.03, 0.01, −0.02, 0.025), sampling.wesl:172-175 | **Guessed**; the client uses one pass scrolled by the material's tweaks 0 and 1 | High |
| Two-pass average, `rg − 0.5` (sampling.wesl:246-247) | **Guessed**; the client uses du/dv through BUMPENVMAP | High |
| `RIPPLE_UV_STRENGTH` 0.08 (main.wesl:654) | **Guessed** | High |
| Ripple target: the solid albedo | **Wrong**: the client perturbs item 2 (main colour) | High |
| `is_liquid` (aux `w`) | Client data (water layer class), same as `REVIEWED_LIQUID` | High |
| Liquid parameters (role 5) | Client (technique 0, tweaks 0 and 1); not from the table | High |

No real tile reaches any guessed or wrong row, so none of them changes the
picture today.

## History

- 14885d10 (2026-06-24): the two-base blend from `ec_tex_aux` and the table.
- 0b7a5404 (2026-06-24): the ripple from TerrainDefinition water layers.
- 096c9ab5 (2026-10-25): the packed-pair and reviewed-liquid early return in
  `apply_ec_aux_two_base_blend` ("must not be blended twice").
- 845ee761 (2026-10-25): `use_ec_liquid`, and the `!use_ec_liquid` ripple guard.
- 26f752d7 (2026-10-31): water scrolls by the client's tweaks only.

The later commits switched the old paths off with guards instead of removing
them. The ripple has been unreachable since 845ee761, and the blend has
reached no tile since 096c9ab5 (with these packages).

## Recommendation: remove

The table-driven code and bindings 113/114 can be removed from the land
material with no change in output:

1. `sampling.wesl`: `apply_ec_aux_two_base_blend`, `read_aux_lookup_slot`,
   `AuxLookupSlot`, `sample_ec_ripple_normal`, `sample_ec_aux_atlas_layer`,
   `RIPPLE_SCROLL_*` and the aux lookup constants. `main.wesl`: the blend call
   (628-631) and the ripple block (633-659), and their imports.
2. `land_bindings.wesl:87-89` and `mesh_material.rs:75-82`: bindings 113 and 114;
   land.rs:1606-1607.
3. land.rs: the table loop in `create_aux_atlas_images` (271-350),
   `terrain_layer_texture_with_role` and the three role functions (119-145),
   the `terrain_meta` parameter, and the aux lookup image. Keep the aux atlas
   image: `EcItemLiquidImages` and `EcItemSurfaceImages` use it (land.rs:1508-1515).
   Check that their layers are still assigned once the table loop is gone.
4. The shader-compile test `land_sampling_shader_composes` (land.rs:1963) names the removed functions; update it.
5. Docs: `UDDP_FORMATS.md:121` says the renderer reads the table for the
   secondary base, blend alpha and ripple; `LAND_TEXTURES_AND_TRANSITIONS.md:64`
   says `tex_aux_ec` supplies the secondary bases and masks of the two-base blends.
   Both become false.

After that, nothing in Dynamapper reads the table. Its other readers are tools
and tests: `udd-conv-cli` `package_info.rs:281` and `package_validate.rs:563`
(tile count), `uddp-inspector-gui` `app.rs:1944` (test bytes), the example
`inspect_ec_tile_surface_signals.rs`, and the `udd-conv`/`udd-assets` tests.
`source_materials.json` already carries the same items, keyed correctly by
material. So the table can be dropped from `tex_land_ec` in a later format
bump, or kept for inspection; nothing at runtime needs it.

## Related finding outside the table: base stretch of the 83 duplicate-item tiles

For the 83 tiles above (19 materials with item 0 = item 1), the provenance has
no record for layer 0, only for layers 1 and 2. `resolve_material_render_layers`
then returns no layer 0, `has_base_layer` is false, and the BASE role is
written by the fallback branch (land.rs:1050-1065) with the stretch of
`resolve_material_layer_slot(tile, primary_layer_index = 0)`, which is also
`None`, so the stretch is 0 (land.rs:1059). The shader then uses
`extent / 44` (`ec_tile_stretch`, sampling.wesl:50-53): 5.82 cells for a
256-px texture, 11.6 for material 15's 512-px one. The client repeats item 0
every 1, 4, 5 or 6 cells (material 170 "Black": 1; 156-159 and 164-167: 4;
15, 44, 45, 52: 5; 154, 196, 197-200: 6).

Data: high confidence. Visual effect: not rendered or measured, and how often
these tiles occur on the maps was not counted. Possible fixes: have the packer
record layer 0 when item 0 repeats item 1, or take the fallback stretch from the
material's item 0 in `source_materials.json`. Not changed here.
