# KR (UOKR.exe): sectors, map sizes and the format deltas the KR inventory asked about

Program: `UO - Old KR/UOKR.exe`, SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.
It is imported into the Ghidra project as `/UOKR.exe` and auto-analysed (40683 functions). It was never
executed. The names below are those applied by [`NameFacetCode.java`](../NameFacetCode.java) and
[`NameTerrainCode.java`](../../ghidra-terrain/NameTerrainCode.java) (namespaces `UOFacet`, `UOTerrain`).
The reports are [uokr_facets_names.md](uokr_facets_names.md) and
[uokr_terrain_names.md](../../ghidra-terrain/results/uokr_terrain_names.md).

Conventions:

- `StreamRead` is FUN_0060cf18. The read size is pushed as `PUSH n; POP EDI` or `XOR EDI,EDI; INC EDI`.
- FUN_00408ba0 is a no-op log stub: a "logged" condition has no effect.
- File offset = VA − 0x401000 + 0x400 for `.text`.

Confidence: **high** = read from the code and checked against all shipped KR files; **medium** = read from
the code only, or the code path is partly traced; **low** = inferred.

The questions are those of the KR inventory (sections 3–5, Ghidra questions 1–10).

## Sector format (from the parser)

`UOFacet::SectorFactory_Create` 0x007b6789 (`UOSectorInstanceBinaryFactory` vftable 0x00b83810, slot 1)
calls `UOFacet::Sector_Read` 0x007b688f:

```
u8  facet
u16 sector id
SectorClientInstance_ctor 0x007b663f
4096 squares, x 0..63 outer, y 0..63 inner      (UOFacet::Sector_ReadSquare 0x007b6994)
    i8  z
    u16 material      (TerrainDefinition id; inventory "kr_land")
    u8  subtype       (alias selector in that TerrainDefinition; inventory "b")
    u16 legacy land id (classic land id; inventory "classic_land")
    -> SectorClientInstance_AddSquare 0x007bfeae (0x44-byte UOSectorClientInstanceSquare, vftable 0x00b84228;
       getters slot 5 z, 6 material, 7 subtype, 8 legacy: 0x007bfe06/0a/0e/12/16)
    u8  delimiter count, then per delimiter       (UOFacet::Sector_ReadDelimiter 0x007b6ac9)
        u8  direction; if < 8: i8 z, u32 land, (dx, dy) from the EC table
            0 (-1,0) 1 (-1,-1) 2 (0,-1) 3 (1,0) 4 (1,1) 5 (0,1) 6 (-1,1) 7 (1,-1) -> FUN_007c032b
    u8  static count, then per static             (UOFacet::Sector_ReadStatic 0x007b6ba6)
        u32 graphic, u8 z, u32 hue -> FUN_007c0120
```

- The square is EC's square with 3 more bytes. EC has `i8 z, u16 land`; KR has `i8 z, u16 material,
  u8 subtype, u16 legacy`.
- Validated offline on every sector of facet0..4 (7168, 7168, 900, 1280, 529): each is consumed
  exactly. Numbering is column-major, `ceil(h/64)·(x>>6) + (y>>6)`, as in EC (`UOFacet::SectorIndexFromXY`
  0x00589ef0). High.
- The request is `UOFacet::RequestSector` 0x0058a080, asset `(sector << 6 | facet & 0x3f, 0x09000000)`, as
  in EC. High.

Client constraints, which differ from EC:

| Rule | KR behaviour | EC behaviour | Conf. |
|---|---|---|---|
| A read overrun | `Sector_Read` deletes the sector and returns null (a hole) | the same | high |
| Leftover bytes after square 4095 | only logged; the sector is **used** | the sector is rejected | high |
| Delimiter direction ≥ 8 | only logged; no z/land is read, so the stream **desyncs** and the sector is not rejected | the sector is rejected | high |
| Delimiter order and presence | no check | no check | high |
| Header facet / sector id ≠ requested | `UOFacet::BuildChunkFromSector` 0x0058d890 compares them (square getters 0x14/0x18) and only logs; the chunk is built | the chunk is not built | medium |
| Missing square | material 0, legacy 1, flags 0 | n/a | high |

## Answers

### 1. The square byte `b` (0/1/2)

It is the **subtype**: it selects the alias inside the material's TerrainDefinition. It is not a separate
definition or a variant index into something else.

- `BuildChunkFromSector` requests the TerrainDefinition of the cell's material.
- It then searches its aliases (16-byte memory stride) for `alias.subtype == b`, and stores that alias's
  u32 flags in the cell (+0x14).
- It does not null-check the result, so the subtype must exist.

Data: in all 5 facets every `(material, subtype)` pair resolves to an alias of the KR
`TerrainDefinition.uop` (0 missing definitions, 0 missing subtypes). Subtype == alias position in 191/191
definitions, so the byte is also the alias index in practice. It is the EC `newSubType` of
`legacyterrainmap.csv`. **High.**

### 2. Delimiter rules and header ids

- The reader enforces **no** delimiter order or presence rule. **High.**
- Direction ≥ 8 is only logged, and the stream desyncs. **High.**
- The header facet and sector id are compared in `BuildChunkFromSector`, but a mismatch is only logged:
  "Fetched incorrect sector ID (bad .sector file?)", "Fetched sector from a foreign facet". The strings
  exist; the compare is at the getters, and FUN_00408ba0 is a no-op. **Medium.**

### 3. Particle byte missing in KR EffectDefinitionCollection

KR lacks **`hued`**. Particle Create 0x007b879d reads:

- u32 name, 4 × f32, u8 attach, f32 duration;
- u8 loop (normalized `!= 0`, stored at +0x38);
- u8 → +0x3c (`field_3c`), via Init 0x004b1320.

The definition is 0x40 bytes (NewInstance 0x004b0e30 allocates 0x40; EC 0x44), and KR has no
HuedParticle class. **High.**

### 4. Effect-type enum

From the `CanHandle` and `GetType` slots of each KR factory.

| KR key | KR type | EC key |
|---|---|---|
| 0 | Particle | 0 |
| 1 | Sound | 1 |
| 2 | RoundRobin | 2 |
| 3 | Death | 3 |
| 4 | Projectile | 4 |
| - | (EC 5 `UOTileEffectDefinition`, **inserted** in EC) | 5 |
| 5 | Container | 6 |
| 6 | PositionOffset | 7 |
| 7 | Stencil | 8 |
| 8 | Collection | 9 |
| 9 | Wait | 10 |
| 10 | LOD | 11 |
| 11 | PropertyFlag | 12 |
| 12 | Rain | 13 |
| 13 | Snow | 14 |
| 14 | Earthquake | 15 |
| 15 | RandomCollection | 16 |
| 16 | UserPrefFilter | 17 |
| - | (EC 18 Fade / FadeToDeath, new) | 18 |

So EC key = KR key + 1 for KR keys ≥ 5.

- KR `CanHandle` accepts 0, 1, 2, 6, 9, 10, 11, 14, 15, 16. The Generic factory always returns false.
- KR key 11, PropertyFlag (Create 0x007b8c99, Init 0x0048cc70, definition 0x2c bytes; EC 0x30): u8 mode
  (values other than 0/1 are logged), **u32 mask** (+8; the 32-bit form of EC's u64 mask) and
  **u32 property selector** (+0xc; EC +0x10), then the list (FUN_007bb949) and the map (FUN_007bb9e2).
  **High.**

### 5. StaticChunk.uop

- **Chunk index.** Chunks are 32 × 32 tiles with the key `cx << 10 | cy`.
  `UOFacet::TerrainChunkBuildRequest_Execute` 0x00458890 decodes `x = (key >> 10 & 0x3ff) << 5`,
  `y = (key & 0x3ff) << 5`. The facet is not in the key; the chunk manager holds the current facet ("Chunk
  isn't in current facet"). **High** for the key, medium for how it maps onto a file entry.
- **Entry name pattern.** Not found.
  - `UOKR.exe` contains no `build/...` path for chunks, sectors or `data.map`: a byte search for
    `sectors`, `data.map` and `facet_` finds nothing.
  - KR resolves `(id, type)` asset keys through a list of locator objects (FUN_00611d75, vtable slot 0 of
    each; then "packed" or "xml" sources at 0x0060a6d8), so the name templates are not in the binary.
  - Guessed names (`build/staticchunk/%08d.bin` and 76 other patterns, each with 13 key formulas such as `cx<<10|cy`, `cx*rows+cy`,
    `x<<16|y` and others for two known chunks) do not match.

  **Unresolved (low).**
- **Runtime writes.** No sign of any. `StaticChunkBuildRequest` (ctor `UOFacet::StaticChunkBuildRequest_ctor`
  0x00457f60, Execute slot 1 `UOFacet::StaticChunkBuildRequest_Execute` 0x004580c0) is a `WorkRequest`. It
  finds the loaded chunk by key (FUN_0058e890) and builds its static objects in memory (FUN_00458160).
  It does no file IO. No writer of `StaticChunk.uop` was found. **Medium.**
- **TerrainChunk.uop.** It does **not** exist in the shipped client. `UOFacet::MountPackages` 0x006095b6
  mounts both names in its list (0x00609ae5 `StaticChunk.uop`, 0x00609b31 `TerrainChunk.uop`), and the
  client runs without the second. **High** for the absence.

### 6. The KR facet 8-byte entry

Yes: it is `build/sectors/facet_%02d/data.map`. The hash matches all 5 KR facets:

| Facet | Hash | u32 height, u32 width |
|---|---|---|
| 0 | `FB6AC9BE9FE7BD97` | 4096, 7168 |
| 1 | `CA032948BBC09C73` | 4096, 7168 |
| 2 | `ADCD1190E8AEB4B9` | 1600, 2304 |
| 3 | `1EE6CCF2ACFB0236` | 2048, 2560 |
| 4 | `BE6A25CFE7C54D6D` | 1448, 1448 |

These match the EC names and hashes. **High.** As in EC, the KR client does not read it: the string is
absent, and the sizes come from `UOFacet::g_FacetDimensions` 0x00bfcc88. **Medium.**

### 7. Land rendering: kr_land (material) or classic_land (legacy id)

KR renders from the **material** (kr_land) plus the subtype.

- The KR cell (stride 0x54, base +4) holds +0xc..+0xf z corners, +0x14 flags, +0x18 material,
  +0x1c subtype and +0x20 legacy. The coverage builder 0x0044ae00 compares cell materials with their
  neighbours. The mesh builder 0x00448de0 reads z. Flags come from the `(material, subtype)` alias.
- The legacy id is stored, but no reader was found among the 0x54-stride users. Of the accessors
  (0x00589560 flags, called by 0x00597ce0; 0x00589660 material, 0x00589760 subtype, 0x00589860 legacy),
  the last three have no direct caller.

So a material/legacy mismatch does not change KR rendering or walkability. It matters only to tools, or to
a server or classic client that use the classic id. **Medium.**

### 8. AnimationSequence: the i32 after alias

Reader 0x004a6a10 (`UOAnimationSequenceBinaryFactory` vftable 0x00b56ddc, slot 1 0x004a6860):

```
u32 id, 8 x (i8, i8)
u32 n actions, each:
    u32 action, u32 frame_count, u32 alias (== -1 -> AddAction 0x004a2480, else the alias path 0x004a23d0),
    u32 extra, f32 fps, 8 x (i8, i8)
    u32 effect-frame count, each {u32 frame, effect list via 0x004b1af0}
u32 group count, groups as in EC
```

- The inventory's "i32 alias, i32 unknown (always -1)": the third u32 is the alias, tested against -1.
  The fourth is `extra`.
- AddAction stores `extra` in the 0x80-byte action record (array at sequence +0x74) at +0x28. The other
  fields are +0x20 frame_count, +0x24 alias, +0x2c base frame, +0x30 fps, +0x40 x offsets, +0x60 y offsets.
- No reader of +0x28 was found in 0x004a1000..0x004a8000. GetFrame 0x004a26f0 uses +0x20/+0x2c. So the
  field is stored but unused. **Medium.**
- The EC short offsets (8 × (i16, i16)) and the EC skipped list are **absent** in KR. **High.**

### 9. tileart v3

Reader 0x007bdeb2 (`UOObjectDefinitionBinaryFactory` vftable 0x00b57338, slot 1 0x007bdd91).

Fixed part: u16 version, u32 name, u32 tile_id (+0x20), then **one** u8, then 14 four-byte fields, then
2 × i32[6] windows (+0x24, +0x3c).

| File order | KR field | KR object | EC equivalent | Conf. |
|---|---|---|---|---|
| u8 | has_world_art | bool +0x54 | `has_world_art` (+0x54); equals "block 0 present" in 17086/17086 records | high |
| (none) | - | - | `uses_enhanced_tileart` (EC +0x55), **dropped** | high |
| f32 | world_art_scale | +0x58 | same | high |
| f32 | legacy_art_scale | +0x5c | same | high |
| u32 | field_60 | +0x60 | same | high |
| u32 | old_id | +0x64 | same | high |
| f32 | rotation | +0x68 | same | high |
| u32 | body_type | +0xd4 | EC +0xe4 | high |
| (none) | - | - | `alternate_animation` (EC u8 after body_type), **dropped** | high |
| f32 | light_scale | +0x6c | same | high |
| u32 | light_field_70 | +0x70 | same | high |
| f32 | light_offset_x | +0x74 | same; non-zero only with KR block 2 (590/590) | high |
| f32 | light_offset_y | +0x78 | same | high |
| u32 | house_join_bits | setter vtable +0x128 | EC setter +0x14c | high |
| u32 | flags1 | +0x80 | EC **u64** | high |
| u32 | flags2 | +0x84 | EC **u64** | high |
| u32 | facing | +0x88 | EC +0x90 | high |

Variable part:

- props1, props2, stack aliases and appearance filters, as in EC (sub-readers 0x007be49b, 0x007be535,
  0x007be40a, 0x007be690).
- Sitting (0x007be787): u8 flag (0/1, other values logged), then **4 u32**. These are EC fields 0..3:
  direction mask, seat offset x, seat offset y, chair-back shift. Field 3 values are 0, 1, 3 and -4, like
  EC's. EC fields 4 and 5 (unread in EC) are dropped. **High** for the count, medium for the meanings.
- No radar colour reader between sitting and the shaders, so EC's 4 × u8 radar colour is **absent**.
  **High.**
- **3 shader blocks** (three calls of 0x007bebf1, setters vtable +0x9c, +0xa0, +0xa4):

  | KR block | Present in | Shader | Texture paths | EC equivalent |
  |---|---|---|---|---|
  | 0 | 13053 records | `UOSpriteShader` / `UOStaticTerrainShader` / `UOWaterShader` | WorldArt | EC block 0, WorldArt |
  | 1 | 17086 records | | TileArtLegacy | EC block 1, TileArtLegacy |
  | 2 | 599 records | | `Textures\...`; 504 paths contain `light`, and the light offsets occur only here | EC block **3**, light |

  EC block 2 (TileArtEnhanced) is the one missing. **High** for 3 blocks, medium-high for block 2 = light.
- Effects follow (0x007be5cf).

### 10. MultiCollection part flag

Reader 0x007bb4c3 (`UOMultiCollectionBinaryFactory` vftable 0x00b83bd4, slot 1 0x007bb3ce):

- u32 id, u32 count;
- per part: u16 item, i16 x, y, z, **u8 flag**, u32 n, n × u32 string ids (resolved through the string
  dictionary).

AddPart 0x007b5d50 stores a 0x1c-byte part `{+0 item, +2 x, +4 y, +6 z, +8 bool flag}`. There is a single
flag, where EC has `not_drawn` (+0x0a) and `flag2` (+0x0b). Leftover bytes are only logged.

It is `not_drawn` by the data (the values match EC's `not_drawn` in 48428/48428 parts) and by being EC's
first flag. The KR readers of +8 were not traced, so the code evidence is **medium**.

## KR TerrainDefinition

This is the EC layout with 12-byte aliases `{u32 subtype, u32 legacy_land_id, u32 flags}`; 191/191 entries
are consumed exactly. KR bit 26 = EC bit 33 (NoDraw), and KR bit 19 is probably EC bit 32 (NoHouse).
Details: [../../ghidra-terrain/results/uosa_terrain.md](../../ghidra-terrain/results/uosa_terrain.md#kr-uo---old-krterraindefinitionuop-uokrexe).

## Map sizes in KR

- **Table.** `UOFacet::g_FacetDimensions`, VA 0x00bfcc88, file 0x007fb688. It has the same 7 records, names
  and sizes as EC. Widths are at +4 (0x00bfcc8c + 12·facet), heights at +8 (0x00bfcc90 + 12·facet).
- **Readers.** Each tests facet < 7 first.
  - `UOFacet::FacetDds_GetMipOffset` 0x004f8b30: 0x004f8bb9 width, 0x004f8bc0 height, `CMP EDI,7` at
    0x004f8baf. Callers 0x004dede0 and 0x004f9fc0.
  - `UOFacet::LoadWaypoints` 0x0057eed0: 0x0057f194, 0x0057f1a0, test at 0x0057f184.
  - `UOFacet::SetPlayerChunkPosition` 0x00589bd0: 0x00589c13, 0x00589c1f and neighbours 0x00589cd5,
    0x00589ce1; tests at 0x00589c03 and 0x00589cc5.
  - `UOFacet::SectorIndexFromXY` 0x00589ef0: 0x00589f03, test at 0x00589efb.
  - `UOFacet::QueueSectorChunks` 0x0058a860: 0x0058a876, test at 0x0058a86e.
  - The land accessors in 0x005879a0..0x0058a740 carry the same x, y ≤ 0x7fe0 guards as EC.
- **No Felucca/Trammel split** and no FacetDefinition.
- **Radar DDS.** `UOFacet::FacetDds_Open` 0x004f88e0 opens `facet%02d.dds` (`"%s%s%02d%s"`). The mip
  offsets come from the table: header 0x80, then DXT1 mips of 8-byte 4×4 blocks. So a patched size
  requires a matching `facetNN.dds` with the full mip chain, or the radar reads the wrong blocks.
- **Patch.** The table only (no split), plus the PE checksum at file offset 0x168. The cross-build recipe
  is in [uosa_facets.md](uosa_facets.md#builds-and-the-cross-build-patch-recipe).

## Ghidra state

- `/UOKR.exe` was imported from `UO - Old KR/UOKR.exe` (read only, never executed), auto-analysed, named
  by both scripts with `--apply` (facets: 1 label, 21 functions; terrain: 2 functions), and saved.
- The d3d9 `.gdt` was not applied: no KR question needed Direct3D types.
