# EC Material Baseline And Metadata Axes

This report freezes current UODynamapper EC material behavior before role-aware
selection changes. It is a developer review artifact, not runtime pack data.

This is a historical baseline. The heuristic and inclusion descriptions below
record the pre-role implementation, not the current runtime. For the current
source graph, ownership rules, and executable-derived liquid roles, read
[ec_source_terrain_routing.md](../ec_source_terrain_routing.md) and
[ec_original_water_code.md](../ec_original_water_code.md).

Original shader bindings are stronger evidence than filename-based role labels.
In particular, `water_alpha` (1000013) is the original bump-environment input,
and `cube3` (1000017) is a 2D animated surface. The runtime uses the preserved
shader graph for these roles; a speculative inventory label alone still must
not determine visibility or ownership.

## Current Heuristics

### Item main EC texture chooser

Source: `lib/udd-assets/src/metadata/tilemeta.rs`.

Current order:
1. non-aux WorldArt ref matching the tile id
2. non-aux primary-selected WorldArt ref
3. any non-aux WorldArt ref
4. any non-aux primary-selected ref
5. any non-aux ref
6. legacy fallback to `TileMetaItemTile.ec_texture_id`

Current limitation: this is package/family biased and does not yet use stable
roles, role confidence, or explicit rejection reasons.

### TerrainDefinition primary layer chooser

Source: `lib/uocf/src/enhanced/land/terrain_definition.rs`.

Current order ranks layers by:
1. support-layer name heuristic
2. preferred repetition range
3. unknown field ordering
4. name string offset ordering

Current limitation: this guesses base/support from names and repetition but does
not preserve a role graph for later authored blend or liquid paths.

### `tex_art_ec` inclusion

Source: `lib/udd-conv/src/textures/tex_art_ec.rs`.

Current behavior: `TileType::Liquid` is excluded wholesale.

Current limitation: art-owned liquid entries with visible bases are skipped
before base eligibility can be evaluated.

## Metadata Axes To Preserve

The following axes must stay independent:

- `physical_package`: `Texture.uop`, `LegacyTexture.uop`, `TerrainTexture.uop`,
  `EffectTexture.uop`, `SystemTextures`, `ShaderResources`, `Unknown`
- `logical_family`: `WorldArt`, `TileArtLegacy`, `TileArtEnhanced`, `Textures`,
  `Effects`, `SystemTextures`, `ShaderResources`, `Unknown`
- `stable_role`: `Base`, `SecondaryBase`, `AlphaMask`, `GenericMask`, `Noise`,
  `Detail`, `Overlay`, `NormalLike`, `ImageSupport`, `EffectOnlyMetadata`,
  `UnknownSupport`
- `speculative_role`: `LiquidRipple`, `LiquidReflectionSupport`,
  `LiquidEnvProbe`, `FoamHighlight`, `FlowMapLike`,
  `RefractionDistortionLike`, `WaterfallSupport`, `LavaBubbleSupport`,
  `PostBlendSupport`, `UnknownSupport`
- `role_reason`
- `confidence`
- `metadata_only`
- `decodable_image`
- `owner_kind`
- `owner_id`
- `source_path_raw`
- `source_path_normalized`
- `block_index`
- `item_or_layer_index`
- `stretch_or_repetition`
- `unknown_fields`

## Policy Frozen For Next Changes

- Direct UOP owner evidence is stronger than current Rust heuristics.
- Package membership is not ownership.
- `TerrainTexture.uop` resources may be art-owned support.
- `EffectTexture.uop` resources are currently inventory evidence unless a direct
  owner reference or indirect NIF/EMS link proves more.
- Speculative roles must not drive visibility or routing.
- First-pass base selection must use stable roles only and log/report fallback
  reasons explicitly.
