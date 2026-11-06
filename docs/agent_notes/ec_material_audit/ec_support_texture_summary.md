# EC Support Texture Inventory Summary

Historical inventory from before the executable-derived role rework. Counts and
image-decoding results describe that audit's source and decoder, not guaranteed
current coverage. The sibling CSVs retain provisional filename-based role labels;
none of those labels establish client shader roles or current package ownership.
Use [current auxiliary coverage](../tex_aux_ec_coverage.md),
[source routing](../ec_source_terrain_routing.md) and the
[format contracts](../../dev_wiki/UDDP_FORMATS.md) for implementation.
Regenerate with `udd-cli pack audit-ec-material-refs` and
`udd-cli pack inventory-ec-support-textures` against an explicitly identified
client when a fresh census is needed; do not overwrite the historical snapshot
as if the source and semantics were unchanged.

Source: `EC client`

## TerrainTexture.uop

| Metric | Count |
|--------|-------|
| Total resources | 37 |
| Decodable images | 0 |
| Directly owner-linked | 29 |
| Image, no owner link | 0 |
| Owner-linked, no image | 29 |

### File Kinds

| Kind | Count |
|------|-------|
| unknown_binary | 37 |

### Stable Role Guesses

| Role | Count |
|------|-------|
| EffectOnlyMetadata | 37 |

### Speculative Role Guesses

| Role | Count |
|------|-------|
| PostBlendSupport | 5 |
| UnknownSupport | 32 |

### Keyword Frequency

| Keyword | Count |
|---------|-------|
| glow | 5 |
| light | 4 |

## EffectTexture.uop

| Metric | Count |
|--------|-------|
| Total resources | 829 |
| Decodable images | 238 |
| Directly owner-linked | 0 |
| Image, no owner link | 238 |
| Owner-linked, no image | 0 |

### File Kinds

| Kind | Count |
|------|-------|
| image | 238 |
| nif | 466 |
| text | 63 |
| unknown_binary | 62 |

### Stable Role Guesses

| Role | Count |
|------|-------|
| EffectOnlyMetadata | 591 |
| ImageSupport | 238 |

### Speculative Role Guesses

| Role | Count |
|------|-------|
| FoamHighlight | 42 |
| LavaBubbleSupport | 11 |
| LiquidEnvProbe | 51 |
| LiquidReflectionSupport | 2 |
| LiquidRipple | 1 |
| PostBlendSupport | 77 |
| UnknownSupport | 635 |
| WaterfallSupport | 10 |

### Keyword Frequency

| Keyword | Count |
|---------|-------|
| alpha | 518 |
| light | 69 |
| flare | 62 |
| glow | 49 |
| sphere | 47 |
| splash | 43 |
| bubble | 28 |
| lava | 27 |
| water | 26 |
| blur | 21 |
| waterfall | 10 |
| detail | 9 |
| env | 5 |
| flow | 3 |
| mask | 3 |
| reflect | 2 |
| noise | 1 |
| ripple | 1 |

## Required Special Cases

| Name pattern | Occurs in |
|--------------|-----------|
| ripple | EffectTexture.uop only |
| cube | neither |
| env | EffectTexture.uop only |
| sphere | EffectTexture.uop only |
| water | EffectTexture.uop only |
| mask | EffectTexture.uop only |
| alpha | EffectTexture.uop only |
| noise | EffectTexture.uop only |
| blur | EffectTexture.uop only |
| lava | EffectTexture.uop only |
| splash | EffectTexture.uop only |
| waterfall | EffectTexture.uop only |
| flare | EffectTexture.uop only |
| glow | both |
| foam | neither |

**EffectTexture.uop images directly owner-referenced:** 0

**Images with no direct owner link (may be only indirect/NIF/EMS):**
- TerrainTexture.uop: 0
- EffectTexture.uop: 238

**Owner-linked resources with no decodable image (NIF/EMS/text/binary):**
- TerrainTexture.uop: 29
- EffectTexture.uop: 0

## Open Points

- Resources with no direct owner link and no decodable image may be reachable only through NIF or EMS indirection.
- No TerrainShaders-like or shader-resource package families are distinguished from this inventory alone; check the owner-reference census CSV for shader_name columns.
- Speculative roles are provisional guesses from name heuristics only — not proven original-EC intent.
