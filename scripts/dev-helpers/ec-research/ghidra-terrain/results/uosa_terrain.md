# EC TerrainDefinition.uop: the record format, client use and an encoder recipe

Sources:

- `UOSA.exe` SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, in Ghidra;
- `_Ultima Online Enhanced/TerrainDefinition.uop`, `string_dictionary.uop`, and
  `GameData.uop:data/gamedata/legacyterrainmap.csv`;
- KR: `UOKR.exe` `1b802e5c...348a43` and `UO - Old KR/TerrainDefinition.uop`.

The offline checks are scratch tools: `terrain_def.py` for the parser, encoder and statistics, and
`kr_td.py` for KR. They use the Python stdlib and run with `-I`.

Function names are those applied by [`NameTerrainCode.java`](../NameTerrainCode.java), namespace
`UOTerrain`.

## Package

- 201 entries, `build/terraindefinition/%d.bin` with `%d` = the record id (0..200, decimal, no padding).
  Every entry hash resolves, and entry name == record `id` in 201/201. High.
- All entries are zlib (flag 1). Asset key `(id, 0x03000000)`: `UOTerrain::GetTerrainDefinition` 0x004cc990
  goes through `UOTerrain::ResourceRequest_AddTerrainDefinitionKey` 0x004cb460. High.
- The id is the **material** that sector squares and `legacyterrainmap.csv` (`newId`) refer to.

## Record layout (all little-endian; every byte of all 201 entries accounted for)

Parser `UOTerrain::TerrainDefinition_Read` 0x00845c25. It is called by
`UOTerrain::TerrainDefinitionFactory_Create` 0x00845b04, `UOTerrainDefinitionBinaryFactory` slot 1
(vftable 0x00ca5e14). The in-memory object is 0x48 bytes:

- vftable `UOTerrainDefinitionBase` 0x00cde940;
- constructor `UOTerrain::TerrainDefinition_ctor` 0x0083ac8f;
- `UOTerrain::TerrainDefinition_MemSize` 0x0083ac78 returns 0x48 + 32 × alias count + the name size.

| Offset | Type | Field | Object | Client use | Conf. |
|---|---|---|---|---|---|
| 0x00 | u32 | name: string_dictionary index (`void_no_draw`, `grass`, `dirt`, `water`, ...) | +0x14 std::string | resolved at parse time; no rendering reader found | high (layout) |
| 0x04 | u32 | id (material) | +0x30 | lookup key; == entry name | high |
| 0x08 | f32 | `field_34` | +0x34 | **no reader**; 0.0 in 201/201 | high (unused) |
| 0x0c | f32 | `coverage_exponent` | +0x38 | see below; constructor default 1.0 (DAT_00c981c8) | high |
| 0x10 | f32 | `field_3c` | +0x3c | **no reader**; 0.0 in 201/201 | high (unused) |
| 0x14 | u32 | alias count n (1×117, 2×80, 3×4) | vector +8/+0xc | | high |
| 0x18 | n × 16 | alias `{u32 subtype, u32 legacy_land_id, u64 flags}` | 0x20-byte records `{+0 subtype, +4 legacy id, +8 flags lo, +0xc flags hi}` (`UOTerrain::TerrainDefinition_AddAlias` 0x0084596f, via `UOTerrain::TerrainDefinition_ReadAliases` 0x00845da6) | below | high |
| +0 | u8 | shader block present (1 in 201/201) | | `UOTileArt::ParseShaderBlock` 0x008456a3, the parser shared with tileart | high |
| +1 | u8 | `field_04` (0 in all) | shader +0x04 | no terrain reader | high (unused) |
| +2 | u32 | shader name (string index): `UODefaultTerrainLayer` 192, `UOWaterTerrainLayer` 8, `UOBumpMapTerrainLayer` 1 | shader +0x08 | selects the layer shader | high |
| +6 | u8 | texture item count k (3×148, 4×52, 5×1) | | | high |
| +7 | k × 17 | item `{u32 path index, u8 source_kind, f32 stretch, u32 sampler_index, u32 uv_set}` | `UOTileArtTextureItem` | below | high |
| | u32 + 4·m | technique count m (4 in all), m × i32 (`0,0,0,0` in 193) | shader int[4] +0x34 | shader parameters | high (layout) |
| | u32 + 4·m | tweak count m (4 in all), m × f32 (`0,0,0,0` in 193; small values such as -0.1, 0.01 elsewhere) | shader float[4] +0x44 | shader parameters | high (layout) |

`UOTerrain::TerrainDefinition_SetShader` 0x008459c0 stores the block as a shared_ptr at +0x40/+0x44. Nothing
follows the block. Size = 20 + 4 + 16n + 1 + 1 + 4 + 1 + 17k + 4 + 16 + 4 + 16 bytes. Validation:
**201/201 consumed exactly, and 201/201 re-encode byte-identical** (`terrain_def.py`).

Texture items:

- The path is, for example, `02000010_Grass_C.tga`, `02000011_Grass_B.tga`, `01000003_noise_alpha.tga`.
- `UOTerrain::LoadTerrainLayerTexture` 0x004613b0 reads `source_kind` (+0x1c; 0 in all items) and `stretch`
  (+0x20). It sets the UV scale to 1 / stretch, so stretch is the tile repeat (4, 16, 5, 1, 6, ...). High.
- `sampler_index` equals the position in 625/657 items. The 32 exceptions are 4-item blocks with
  `(0, 1, 2, 4)`. No terrain reader was found, which matches tileart, where it is not read. Medium.
- `uv_set` is 0 in all items.

## The three floats

- `field_34` (+0x34) and `field_3c` (+0x3c) are 0.0 in every record. No reference to these object offsets
  exists in the terrain code: the readers of the definition are the mesh builder, the layer loader, the
  flag and material accessors and MemSize. Write them as 0.0. High.
- `coverage_exponent` (+0x38) has these values: 0 ×92, 1 ×77, 5 ×22, 0.25 ×4, 0.1 ×3, 0.5 ×2, 0.15 ×1.
  `UOTerrain::LoadTerrainLayer` 0x00461800 reads it at 0x00461831 and passes it to
  `UOTerrain::BuildTerrainCoverage` 0x00461060. That function builds the 32×32 coverage NiPixelData of a
  layer from neighbour material equality. When the exponent is > 0, it shapes the falloff through the
  pow-like FUN_00bd6860 times DAT_00d04ef0. Otherwise the mask stays binary. At 0x004618d7 it is compared
  with 0.0 (DAT_00c98578): equal clears bit 0x100 of the layer's 0x10-byte property (vtable 0x00cbf528),
  otherwise sets it. So 0 means a hard edge with no blending, and larger values give a softer, wider
  blend. High for the reads, medium for the visual wording.

## Aliases (subtypes)

- A sector square or `legacyterrainmap.csv` row names `(material, subtype)`. The alias whose `subtype`
  equals it supplies the cell flags. `UOFacet::BuildChunkFromSector` 0x005e2f00 copies `flags` into the
  0x58-byte cell (+0x10/+0x14). When no alias matches, the flags are 0. High.
- `UOTerrain::BuildTerrainMesh` 0x00461ae0 looks up the **subtype-0** alias of each cell's material
  (0x00461b92) and dereferences it with no null check. A definition without subtype 0 crashes the
  client. High.
- `legacy_land_id` (+4) has no reader. It is 0 in 104 aliases. In 176 aliases it equals a classic land id
  mapped to `(material, subtype)` by the csv, and in 134 of those it is the smallest such id. So it is a
  representative classic land id, kept for tools. Low for the purpose, high for "unused".
- Data: subtype 0 in 201/201 definitions, subtypes unique and **equal to the position** in 201/201, values
  0..2. All 2705 csv rows resolve to an existing alias. 44 aliases are not referenced by the csv (for
  example 172..187 subtype 0).

## Flag bits (alias `flags`, u64)

The bit positions follow the tiledata / `TaeFlag` layout (`lib/uocf/src/enhanced/tileart.rs`).

| Bit | Mask | `TaeFlag` | Aliases | Client reader | Conf. |
|---|---|---|---|---|---|
| 4 | 0x10 | Wall | 25 | none found | medium (unread) |
| 6 | 0x40 | Impassable | 72 | `UOTerrain::CollectSurfaces` 0x005f3b20 (via `UOTerrain::GetLandFlags` 0x005de730): land is impassable | high |
| 7 | 0x80 | Wet | 6 | none found | medium (unread) |
| 8 | 0x100 | Ignored | 9 | `CollectSurfaces`: the land square adds no walk surface | high |
| 9 | 0x200 | Surface | 10 | none found | medium (unread) |
| 16 | 0x10000 | Mongen | 2 | none found | medium (unread) |
| 25 | 0x2000000 | HoverOver | 0 | `CollectSurfaces` ORs it in for its mode 3; never in the data | high |
| 32 | 0x100000000 | NoHouse | 12 | none found (house placement is server side) | medium (unread) |
| 33 | 0x200000000 | NoDraw | 2 (`void_no_draw`) | `BuildTerrainMesh` 0x00461bdb: subtype-0 flags hi & 2 -> `UOTerrain::AddNoDrawTerrain` 0x00463b60, the cell is not drawn | high |

"Unread" means no `TEST`/`AND` on these bits was found in the readers of cell +0x10/+0x14 or of the alias
records. The values are still worth keeping exactly, as tiledata-style metadata.

## Other readers

- `UOTerrain::GetLandMaterial` 0x005de840 (cell +0x18) feeds `UOLua::LuaGetTerrainType` 0x005dc160.
- `UOTerrain::LoadLegacyTerrainMap` 0x004027b0 and `UOTerrain::LegacyTerrainMap_Lookup` 0x00403410 hold the
  csv map that `BuildChunkFromSector` uses.
- `UOTerrain::TerrainDefinitionFactory_CreateDefault` 0x004cc5f0 (factory slot 4) builds an empty
  definition. It has no aliases, so `BuildTerrainMesh` would crash on it: a referenced material must
  exist.

## Recipe for the repo (model, encoder, invariants)

Model (rename the `lib/uocf/src/enhanced/terrain_definition.rs` fields; keep serde aliases if they are
serialized):

```text
TerrainDefinition {
  name_index: u32            // was name_id: string_dictionary index (check the repo's index base: the scratch
                             // list keeps the empty string 0, so the raw index resolves directly)
  id: u32                    // material; entry name build/terraindefinition/{id}.bin
  field_34: f32              // was unk;  write 0.0
  coverage_exponent: f32     // was unk2
  field_3c: f32              // was unk3; write 0.0
  aliases: Vec<{ subtype: u32 /* was count_index */, legacy_land_id: u32 /* was alias */, flags: u64 }>
  shader: Option<ShaderBlock> // present byte; Some in every shipped record
}
ShaderBlock { field_04: u8, shader_name_index: u32, items: Vec<TextureItem> (u8 count),
              techniques: Vec<i32> (u32 count), tweaks: Vec<f32> (u32 count) }
TextureItem { path_index: u32, source_kind: u8, stretch: f32, sampler_index: u32, uv_set: u32 }
```

Encoder, in this order, all little-endian:

1. `u32 name_index`, `u32 id`, `f32 field_34`, `f32 coverage_exponent`, `f32 field_3c`.
2. `u32 aliases.len()`, then per alias `u32 subtype, u32 legacy_land_id, u64 flags`.
3. `u8 1` (present), `u8 field_04`, `u32 shader_name_index`, `u8 items.len()`, then per item
   `u32 path_index, u8 source_kind, f32 stretch, u32 sampler_index, u32 uv_set`.
4. `u32 techniques.len()` + i32s, then `u32 tweaks.len()` + f32s.
5. Nothing after. Write `u8 0` alone for `shader: None`, but the shipped data never does that.

The parser should read the shader block unconditionally (present byte), not "if bytes remain". It should
reject trailing bytes. Pack each record as zlib (flag 1) under `build/terraindefinition/{id}.bin`, using the
classic-zlib UOP writer the repo uses for the other packages.

Invariants to enforce on encode:

- every definition has an alias with `subtype == 0` (the client crashes otherwise);
- subtypes are unique within a definition, and the shipped data uses subtype == position;
- every `(material, subtype)` used by a sector square or by `legacyterrainmap.csv` exists. A missing
  subtype gives flags 0 (walkable, drawn); a missing material crashes the mesh builder;
- `items.len() <= 255` and `aliases.len()` fits in u32. Keep the technique and tweak counts at 4, as
  shipped (the shader arrays are int[4] and float[4]; `SetTechniqueValue` ignores an index >= 4, so extra
  values are lost);
- the string indices resolve in `string_dictionary.uop`. Shader names are one of
  `UODefaultTerrainLayer`, `UOWaterTerrainLayer`, `UOBumpMapTerrainLayer`.

Round trip: parse -> encode reproduces all 201 shipped payloads byte for byte. The scratch reference
encoder is `terrain_def.py:encode`.

## KR (`UO - Old KR/TerrainDefinition.uop`, `UOKR.exe`)

- 191 entries. Parser `UOTerrain::TerrainDefinition_Read` 0x007bf15a (factory
  `UOTerrain::TerrainDefinitionFactory_Create` 0x007bf039, `UOTerrainDefinitionBinaryFactory` vftable
  0x00b574ac slot 1).
- The layout is the EC one except that **aliases are 12 bytes**: `{u32 subtype, u32 legacy_land_id, u32
  flags}`. 191/191 consumed exactly (`kr_td.py`). High.
- Floats: `field_34` = `field_3c` = 0 in all. Coverage exponent: 0×94, 1×65, 2×21, 3×4, 0.25×4, 0.5×2,
  4×1.
- Flag bits: 4×25, 6×66, 7×5, 8×3, 19×11, 26×2. The void definition carries `0x4000100` in KR and
  `0x200000100` in EC. So KR bit 26 is the EC no-draw bit 33 (high), and KR bit 19 (11 aliases) is
  probably EC bit 32, NoHouse (medium). A KR to EC converter must remap these two bits.
- Subtype 0 is present and subtype == position in 191/191. Every `(material, subtype)` pair used by the
  KR sectors resolves (see `../../ghidra-facets/results/uokr_facets.md`).
