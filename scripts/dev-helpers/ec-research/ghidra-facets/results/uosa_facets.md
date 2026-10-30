# EC facets: sector packages, the 8-byte header entry, facetNN.dds and FacetDefinition

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`. Data:
`_Ultima Online Enhanced` (EC), `UO - Old KR` (KR), and `_Ultima Online Classic/radarcol.mul` for the
radar colours. The names below are the ones `NameFacetCode.java` applies (namespace `UOFacet`, tag
`UO_FACET`); [uosa_facets_names.md](uosa_facets_names.md) is its report.

Confidence: **high** means read from the code and checked against all shipped files, **medium** means
read from the code only, **low** means inferred.

## Packages and mounting

- `MountPackages` (0x00a6bb5a) mounts `facet0.uop` .. `facet6.uop` (strings at 0x00cfda48; `facet6.uop`,
  the Test facet, is not shipped) and `facets.uop` with every other package. Lookups are global by hash.
  High.
- `UOFacet::SwapFacetPackageVariants` (0x00a6c9fc) unmounts `facetN.uop` and mounts `facetNx.uop` for
  N = 0, 1, 2, 5 (and the reverse). `UOFacet::ApplyFacetVariantFlag` (0x00615730), called from
  `Handle_0xA9_CitiesAndChars`, does this when bit 15 (0x8000) of the 0xA9 character-list flags differs
  from the mounted state. Emulators call that bit "new Felucca areas" (naming not verified here). High for
  the swap, medium for the meaning.
- Shipped variants: `facet2x` and `facet5x` are identical to the base packages (every sector byte-equal).
  `facet0x` differs from `facet0` in 191 of 7168 sectors, and `facet1x` from `facet1` in 209 of 7168. They
  use the same entry names, so a variant package must use the base package's names and sector ids.

## Q1: the 8-byte entry (`data.map`)

- Name: `build/sectors/facet_%02d/data.map`. lookup3 `hashlittle2` of the lowercased path, initial
  values 0, entry hash `(b << 32) | c`. That is the same as every UOP name (`UOP::HashFileName` 0x0098d487
  → `HashString` 0x0098ed3d). It was matched by brute force over path templates. High.

  | Package | Hash |
  |---|---|
  | facet0.uop, facet0x.uop | `FB6AC9BE9FE7BD97` |
  | facet1.uop, facet1x.uop | `CA032948BBC09C73` |
  | facet2.uop, facet2x.uop | `ADCD1190E8AEB4B9` |
  | facet3.uop | `1EE6CCF2ACFB0236` |
  | facet4.uop | `BE6A25CFE7C54D6D` |
  | facet5.uop, facet5x.uop | `45B0389074FCF0FF` |

- Payload: `u32 height, u32 width`, in tiles (for example 4096, 7168 for Felucca), stored uncompressed
  (flag 0) while every sector is zlib (flag 1). High.
- **The client never reads it.** Neither `data.map` nor `.map` occurs in `UOSA.exe` (ASCII or UTF-16), and
  nothing in the exe holds a precomputed hash. The only sector request is `UOFacet::RequestSector`.
  Dimensions come from the hard-coded table `UOFacet::g_FacetDimensions` (0x00da72a8, 7 records
  `{char *name, u32 width, u32 height}`: Felucca and Trammel 7168×4096, Ilshenar 2304×1600, Malas
  2560×2048, Tokuno 1448×1448, TerMur 1280×4096, Test 256×256) and from FacetDefinition. Only the width
  (+4) and height (+8) fields are read. High.
- A writer should still emit it, matching the shipped packages and the patcher. Its only consumer is
  offline tools.
- KR: `facet0..4.uop` (version 4) carry the same entry with the same names and hashes.

## Q3: sector entries, the client's constraints

Asset key `(sector << 6 | facet & 0x3f, 0x09000000)`, which assetmap type 9 resolves to
`build/sectors/facet_%02d/%08d.bin` with the sector id in decimal. Read by `UOFacet::RequestSector`
(0x005df3b0), the only builder of this key.

Numbering (`UOFacet::SectorIndexFromXY` 0x005df210, high): column-major,
`sector = ceil(height / 64) * (x >> 6) + (y >> 6)`, with the height taken from `g_FacetDimensions`. Facets
whose size is not a multiple of 64 (Tokuno, 1448) get a partial last row and column. The shipped data
pads those with 64×64 sectors whose out-of-map cells are `z 0, land 0`, with no statics and no
delimiters.

Payload (`UOFacet::Sector_Read` 0x0083d19a and its helpers, high; every shipped sector is consumed
exactly):

```
u8  facet
u16 sector id
4096 squares, x 0..63 outer, y 0..63 inner (square index = x * 64 + y):
    i8  z            (land)
    u16 land id      (TerrainDefinition key, 0x03000000)
    u8  delimiter count, then per delimiter:
        u8  direction   0 (-1,0) 1 (-1,-1) 2 (0,-1) 3 (1,0) 4 (1,1) 5 (0,1) 6 (-1,1) 7 (1,-1)
        i8  z
        u32 land id
    u8  static count, then per static:
        u32 graphic     (tileart id, no 0x4000 offset)
        i8  z
        u32 hue
```

Constraints a writer must respect:

| Rule | Client behaviour | Address | Confidence |
|---|---|---|---|
| Every read must stay inside the blob | a `StreamRead` overrun returns null (no sector) | `Sector_Read` | high |
| The blob must end exactly after square 4095 | leftover bytes set ok = false; the factory then tries other resolvers and the sector is not used | `Sector_Read`, `SectorFactory_Create` 0x0083d094 | high |
| Delimiter direction ≤ 7 | a larger value sets ok = false: **the whole sector is rejected** | `Sector_ReadDelimiter` 0x0083d3a0 | high |
| Header facet and sector id equal the requested ones | on a mismatch the 32×32 chunk is not built (a hole) | `BuildChunkFromSector` 0x005e2f00 (0x005e2fa4..0x005e2fc2) | high |
| A missing sector entry | the chunk is not built (a hole); no fallback | `BuildChunkFromSector` | high |
| At most 255 delimiters or statics per square | u8 counts | `Sector_ReadSquare` 0x0083d29f | high |
| Entry order inside the package | irrelevant (hash lookup) | `FindEntryIndex` 0x0098d460 | high |

Delimiters as shipped (all 6 facets): they appear only on the sector's edge squares, one per direction
that leaves the sector (edges L 0, T 2, R 3, B 5; corners LT 0 1 2, RT 2 3 7, LB 0 5 6, RB 3 4 5). Each
holds the neighbour's land z and id, and there are none for neighbours outside the map. The client reads
neighbours through `SectorClientInstance_GetNeighbourZAndLand` (slot 4, 0x00847d24): inside 0..63 it uses
the square, outside it clamps to the border square and asks that square's delimiter list for (dx, dy).
Without a matching delimiter the caller's default is left unchanged (medium). A writer should emit the
shipped set.

Statics as shipped: at most 16 per square (Ilshenar), 11 (Tokuno).

## Q2: facetNN.dds (radar)

The files are loose (not in a UOP): `facet00.dds` .. `facet05.dds` in the client directory. They are
DXT1 (NVTT-style header flags 0xa1007), 1 pixel per tile, full size, with
`mips = floor(log2(min(w, h))) + 1`. High.

| File | Size | Mips |
|---|---|---|
| facet00.dds, facet01.dds | 7168×4096 | 13 |
| facet02.dds | 2304×1600 | 11 |
| facet03.dds | 2560×2048 | 12 |
| facet04.dds | 1448×1448 | 11 |
| facet05.dds | 1280×4096 | 11 |

How the client uses it (high):

- `UOFacet::FacetDds_Open` (0x00534b60) memory-maps `<dir>facet%02d.dds` (`"%s%s%02d%s"`, `"facet"`,
  `".dds"`; zero-padded, so `facet0.dds` is never found). `FacetDds_GetMipCount` (0x00534d10) reads the
  header +0x1c (mipMapCount) and returns 0 when the file is missing.
- `Radar_Update` (0x005118c0) maps the zoom to a mip level, clamped below the mip count.
  `Radar_RebuildTexture` (0x00511b10) then calls `FacetDds_CopyRegionBlocks` (0x00534d90). That function
  fills the `radar_texture` NiPixelData with the FacetDefinition region colour (encoded by
  `Dxt1_EncodeBlock` 0x00487410). It then copies the file's 8-byte DXT1 blocks of that mip (data at +0x80)
  only inside the region rects, so the image must be DXT1 with the standard mip chain layout. The "CourseMap"
  texture (0x00537c90) uses the same copy.
- Live overlay: `Radar_OverlayStaticsInRect` (0x00535310) → `Radar_RecolorBlock` (0x00535500) decodes a
  block (`Dxt1_DecodeBlock` 0x00487970) and repaints pixels before re-encoding. The colours come from
  objects seen by `Radar_CollectObjectColors` (0x005110c0). Within 24 tiles of the player it also uses
  statics: it skips NoDraw and HoverOver, and takes Wall or Roof when z > best, or Surface when z ≥ best,
  starting from best = -128. The colour is the tileart radar colour (RGBA after the sitting block).
- **The client never recomputes land colours**. Land and far statics exist only in the DDS. Without the
  file the radar shows only the region background colours and the nearby overlay.

### Regeneration recipe (fitted offline; the generator is not in the client)

For every tile (x, y) of facet N, using the facet's own sectors:

1. `land = radarcol.mul[land id]` (classic `radarcol.mul`, RGB555, 5-bit channels).
2. Static choice: among the square's statics with `static z >= land z`, take the highest z; on equal z,
   take the later one in sector order. If there is one, `colour = radarcol.mul[0x4000 + graphic]`. Hue is
   ignored, and so is the tileart height.
3. Shading, applied only when no static was chosen:
   `D = z(x+1, y) - z(x, y+1)` on the land z, clamping at the map edge. If `D <= -3` multiply each 5-bit
   channel by 0.6; if `D >= 3` multiply by 1.4; take the floor and clamp to 31.
4. Expand to 8 bits (`c << 3 | c >> 2`), DXT1-encode with the full mip chain above, and write the loose
   `facet%02d.dds`.

Match against the shipped mip 0. "Index" is the fraction of pixels whose nearest colour in the shipped
block's 4-colour palette is the shipped pixel's colour, which does not depend on the encoder; "≤16" is the
fraction within 16 per channel. Byte-identical regeneration is not possible, because the shipped encoder
is unknown and its block endpoints differ.

| Facet | Recipe: index | Recipe: ≤16 | Land only, flat: index | Repo rule (top z incl. height above land), shaded: index |
|---|---|---|---|---|
| 0 | 0.9916 | 0.9678 | 0.8985 | 0.9724 |
| 1 | 0.9914 | 0.9679 | 0.8964 | 0.9724 |
| 2 | 0.9773 | 0.9611 | 0.7874 | 0.9625 |
| 3 | 0.9916 | 0.9841 | 0.8991 | 0.9745 |
| 4 | 0.9916 | 0.9665 | 0.9073 | 0.9773 |
| 5 | 0.9822 | 0.9709 | 0.8930 | 0.9701 |

Variants tried and rejected:

- Shading statics as well scores 0.0005–0.004 lower.
- Highest static z regardless of land (A) scores 0.002 lower; first-of-equal (B) 0.006 lower.
- Using the top z including the tileart height (C, the repo encoder's rule) scores 0.01–0.02 lower.

The residuals are mostly water statics (6064–6066, 8612) and some foliage, then DXT1 noise. Confidence:
high for the format and how the client uses it, medium for the recipe (an empirical fit, about 99 % on
5 of 6 facets).

KR ships `facet00..04.dds` with the same sizes and mips. They are not byte-identical to the EC files.

## Q4: FacetDefinition (facets.uop)

Asset key `(facet id, 0x17000000)` (built by `ResourceRequest_AddFacetDefinitionKey` 0x004cb9a0),
which resolves to `build/facetdefinition/%08d.bin`, ids 0..6. Parsed by `FacetDefinitionFactory_Create`
(0x0083c82f, factory RTTI 0x00de5738), `FacetDefinition_Read` (0x0083c933), `_ReadRegions` (0x0083ca5f)
and `_ReadRegionRects` (0x0083cb53). The only accessor is `GetFacetDefinition` (0x005a5290, 14 callers, all
radar or map code). Little endian, no padding.

| Offset | Type | Field | Object, getter (UOFacetDefinition slot) | Consumers | Confidence |
|---|---|---|---|---|---|
| 0 | u8 | facet id | +4, slot 3 | | high |
| 1 | u32 | name cliloc | +8, slot 4 | UORadarGetFacetLabel | high |
| 5 | u32 | width (tiles) | +0xc, slot 5 | UORadarGetFacetDimensions, `GetFacetDimensions` → `FacetDds_CopyRegionBlocks` | high |
| 9 | u32 | height (tiles) | +0x10, slot 6 | same | high |
| 13 | u8 | unknown, 0x40 in all 7 | +0x14, slot 7 | none found | high (unread); low (meaning: sector side) |
| 14 | u8 | unknown, 0x40 in all 7 | +0x15, slot 8 | none found | high (unread); low (meaning: sector side) |
| 15 | u8 | unknown, 0x01 in all 7 | +0x16, slot 9 | none found | high (unread); low (meaning: version) |
| 16 | u32 | region count | slot 10 | UORadarGetAreaCount | high |

Each region (a 0x1c-byte object; `FacetDefinition_AddRegion` 0x00846a0d):

| Type | Field | Getter | Consumers | Confidence |
|---|---|---|---|---|
| u32 | index; equals the region's position in all 7 records | none (stored at +0) | none | high |
| u32 | name cliloc; region 0 is always 1112058 (the whole facet) | slot 11 | UORadarGetAreaLabel | high |
| u8 ×3 | r, g, b: the radar background for the region (region 0, for example, 0, 56, 80) | slot 12 | `Radar_GetAreaColor` → `FacetDds_CopyRegionBlocks` | high |
| u8 | physical facet: 0xff means this facet, otherwise the facet whose coordinates the rects use | slot 13 | `Radar_SetView` (switches the radar facet), `Radar_FindAreaAtLocation` | high |
| u32 | rect count | slot 15 | `Radar_GetAreaRectCount` | high |
| rect count × 4×u32 | x1, y1, x2, y2, inclusive | slot 16; bounds over all rects, slot 14 | `Radar_GetAreaRect`, `Radar_IsLocationInArea` (`x1 <= x <= x2`, `y1 <= y <= y2`), `Radar_GetMaxZoomForArea` (slot 14), UORadarGetAreaDimensions | high |

All 7 records parse with every byte accounted for:

| Id | Size | Name cliloc | Size (tiles) | Regions | Rects | Physical facets used |
|---|---|---|---|---|---|---|
| 0 | 1284 | 1012001 | 7168×4096 | 31 | 48 | 0xff |
| 1 | 1380 | 1012000 | 7168×4096 | 34 | 51 | 0xff |
| 2 | 900 | 1012002 | 2304×1600 | 20 | 35 | 0xff |
| 3 | 148 | 1060643 | 2560×2048 | 4 | 4 | 0xff |
| 4 | 148 | 1063258 | 1448×1448 | 4 | 4 | 3, 0xff |
| 5 | 836 | 1112178 | 1280×4096 | 24 | 27 | 0, 0xff |
| 6 | 52 | 1072851 | 256×256 | 1 | 1 | 0xff |

Anomalies in the shipped records:

- Facet 2, region 5 has an inverted rect (165, 1080, 180, 1040). It matches nothing.
- Facet 4's physical-facet-3 regions and facet 5's regions 2 and 7 (physical facet 0) use Malas and
  Felucca coordinates, up to x2 = 7168.

KR has no `facets.uop` and no FacetDefinition.

## KR differences (`UO - Old KR`)

- `facet0..4.uop`, version 4. They have the same `data.map` entry, names and hashes.
- Sector payloads are larger: a different, KR-era square format (according to the converter below, a KR
  tile id plus the classic id). Not decoded here, so whether the EC parser rejects them is not verified.
- `facet00..04.dds` have the same sizes and mips as the EC files but different bytes.
- No `facets.uop`.

## ManawydanMapConverter (third-party, read only)

It reads the classic map as `TileMatrix(6144, 4096)` (`Form1.cs`) and writes loose KR-era
`facet{N}-XX_YY.dat` files plus `facet{N}-00_00.raw`. Point by point:

| Point | Verdict |
|---|---|
| Header entry = `u32 height, u32 width` (`WriteMapResolution`) | confirmed: same content and order as `data.map`; the client does not read it |
| Sector header `u8 facet, u16 id` | layout confirmed; but it always writes facet 0, and writes id `sum - 1` while naming the file `sum`, which is off by one, so the client would reject it |
| Column-major numbering `blocksX * 64 + blocksY` | confirmed for 64-sector-high facets only (64 is hard-coded; the client uses `ceil(height / 64)`) |
| Delimiter directions Left 0, Top 2, Right 3, Bottom 5 (and diagonals) | confirmed (`Sector_ReadDelimiter` table) |
| Square layout (extra zero bytes, KR id + classic id, u16 delimiter id) | refuted for EC: EC is `i8 z, u16 land`, delimiter `i8 z, u32 land`, statics `u32, i8, u32` |
| Cell coordinates | wrong: the x/y loops start at `ceil(sum / 64)` instead of `blocksX * 64`, and every block starts with the global (0, 0) square |
| Block (0, 0) | replaced by the `.raw` header, so sector 0 is never written |
| Statics | not written (`WriteStatics` commented out) |
| Edge tests `x - floor(x / 64) == 63` | wrong (not `x % 64`) |
| UOP packaging, DDS | not produced |
| Written but not needed by the EC client | the header (`.raw` / `data.map`) |

## Repo code against these findings

- `lib/uocf/src/enhanced/facet_decoder.rs` skips a delimiter with direction > 7 and reads on. The client
  rejects the whole sector instead.
- `lib/uocf/src/enhanced/facet_encoder.rs`:
  - Sector ids are numbered row-major (`chunk_y` outer, `file_id += 1`). The client is column-major, so
    every sector except 0 lands in the wrong place, unless the facet has a single row or column.
  - The DDS is written as `facet{N}.dds`, but the client opens `facet%02d.dds`.
  - It puts uncompressed RGBA8 data into a DXT1-format `ddsfile` container with no mips. The client copies
    raw 8-byte DXT1 blocks, so this file would render as garbage.
  - Its radar colour uses rule C, with no shading and a hue lookup that indexes the hue table by
    `radarcol.r()`. The shipped images ignore hue.
  - The u8 static count wraps above 255 statics per square.
  - It writes no `data.map`, which is harmless for the client.
