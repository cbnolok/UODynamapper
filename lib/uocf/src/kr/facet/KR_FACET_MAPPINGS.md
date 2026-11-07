# Kingdom Reborn Facet Mappings

A Kingdom Reborn (KR) facet square stores its material (a `TerrainDefinition` id), its `subtype` (which alias of
that material: 0/1/2) and its legacy (Classic) land id. The KR client renders from material and subtype and does
not read the legacy id (`scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md`). The mapping
therefore runs one way only:

- **Decoding (KR -> Classic) needs no mapping.** `facet_decoder` takes the land from the stored `classic_land`
  and keeps every static (the sectors store Classic static graphics directly).
- **Encoding (Classic -> KR) needs `classic -> (material, subtype)`.** `facet_encoder` takes it from a
  `kr_land_map::KrLandMap`.

## Conversion coverage and generated binary mapping

CC (Classic), EC (Enhanced) and KR facet conversion does not require the removed
dictionary files. CC <-> EC needs no KR mapping, and KR -> CC/EC reads the stored
Classic land ids directly. CC/EC -> KR still needs a mapping to choose material
and subtype; today that comes from the compiled `KrLandMap` tables below.
Conversion through CC or EC cannot always recover the original KR material and
subtype, because several authored pairs can share one Classic land id. The
converter reports these losses and requires explicit permission for lossy conversion.

The generated [mappings.bin](../../../assets/kr/mappings.bin) is a candidate replacement for the compiled mapping.
**Generate it from every shipped KR facet available in the source bundle (0-4
in the audited client), not only map0.** Aggregate the on-map squares across all
those facets, excluding sector padding, before deriving the Classic-id routes.
The existing `KrLandCounter` and ignored regeneration test already use facets
0-4 to reproduce the 1,556 verified rows. Preserve the current deterministic
selection policy: choose the most frequent material and subtype separately for
each Classic id, breaking ties with the smaller value.

Record the source bundle and facet coverage, and retain evidence of ambiguous
routes. Unobserved ids need an explicitly reviewed override or an explicit
fallback; the unverified supplement must not become verified merely by being
written into the binary. The [format and regeneration command](../../../assets/kr/README.md)
are documented with the artifact and its evidence. Generation is implemented;
the converter still uses the compiled tables and has no binary loader yet.

## `kr_land_map.rs`

- `SHIPPED_KR_LAND_MAP`: generated from the shipped KR facets 0-4 (69,746,240 on-map squares, 1556 Classic ids):
  the most frequent `material` and `subtype` of each Classic id. `material` is a function of the Classic id
  except for 2444 (KR 58 or 46) and 2446 (KR 60 or 47): exact on 99.9995% of the squares. `subtype` takes more
  than one value for 64 ids: exact on 99.967%. The ignored test `kr_facet_sectors_reencode_identically_and_follow_the_rules`
  (`lib/uocf/tests/real_files/kr_facet_client_files.rs`) derives the table again with `KrLandCounter` and fails when it differs.
- `BUILDER_KR_LAND_MAP_SUPPLEMENT`: the 1133 Classic ids the KR facets never use, taken from `get_tile_map` in
  `tile_mappings_builder.rs` (Manawydan's KR converter table). Unverified. The builder table agrees with the
  facets on most shipped ids, but disagrees on 37 and lacks 19, so the generated table wins where both have an id.
- `KrLandMap::shipped()` is both tables; `shipped_verified()` the first only. A Classic id outside the map fails
  the build, unless `with_fallback` names the land to store (the Classic clients after KR add land ids the map
  lacks).

## The removed dictionaries

The former `TileDictionary.bin` (20,250 bytes) and `StaticDictionary.bin` (9,343 bytes) were .NET
BinaryFormatter blobs of `System.Collections.Generic.Dictionary<Int16, Int16>` (Manawydan's tool), which the former
`tile_mappings_loader.rs` read as raw `(u16, u16, u8)` / `u16` records, yielding garbage. They are gone rather than
parsed: an `Int16 -> Int16` dictionary cannot hold `subtype`, decoding needs no dictionary at all, and the
static "whitelist" would only drop valid statics (the sectors use graphics 3674 and 6345, which it lacks).

## `tile_mappings_builder.rs` (feature `tile_mappings_builder`)

The converter's hardcoded tables: `get_tile_map` (Classic -> `(material, subtype)`, source of the supplement
above) and `get_statics_table` (the static whitelist, unused).

## Can the original client replace these tables?

An executable and asset audit on 2026-11-07 confirmed the following for the
original KR bundle described in the [KR facet research](../../../../../scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md)
(`UOKR.exe` SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`).

- `Sector_ReadSquare` (`0x007b6994`) reads material, subtype and Classic land id
  from each sector record. `BuildChunkFromSector` (`0x0058d890`) uses the stored
  material and subtype to select a TerrainDefinition alias. This path does not
  reconstruct the material from the Classic id.
- `TerrainDefinition_Read` (`0x007bf15a`, alias reader `0x007bf2db`) reads aliases
  from an external stream. Its aliases are source data, not a compiled mapping.
- The many data references to the string `LegacyTerrainDefinition` are copies
  of resource-type name arrays, not a land translation table.
- A search of initialized executable memory for nine 16-row fingerprints of
  `get_tile_map` found no matches: rows starting at positions 0, 100 and 500,
  each encoded as packed 5-byte triples, padded 8-byte triples and three u32s.
  This rules out those representations of those sequences, not every possible
  representation of a table elsewhere in the executable.

The original assets provide useful but different sources:

| Source | Coverage and limits |
| --- | --- |
| KR facets 0-4 | Regenerate all 1,556 verified rows exactly. Over 69,746,240 on-map squares, the table matches 69,745,894 materials and 69,723,228 subtypes. A single Classic id can have multiple authored results, so this is a canonical conversion choice, not an exact inverse of every square. |
| KR `GameData.uop` | All 64 entries checked; no `legacyterrainmap.csv` entry or equivalent land-route table was found. The non-text workbook holds housing authoring sheets. |
| KR `TerrainDefinition.uop` | 191 definitions, 227 aliases, 217 distinct nonzero Classic ids. Among the verified table's ids, 182 occur as alias ids and 179 agree; 139, 2444 and 2446 differ. It cannot replace the full conversion map. |
| EC `legacyterrainmap.csv` | The standard EC table overlaps 1,533 verified KR ids but conflicts on 250. It overlaps 416 supplemental ids but conflicts on 329. Using EC routes for KR would change authored material choices. |

The existing ignored test
`kr_facet_client_files::kr_facet_sectors_reencode_identically_and_follow_the_rules`
reconstructs the verified rows with `KrLandCounter` and compares them with the
compiled table. A future data-driven converter can use the same derivation from
its target KR bundle. No original source was established for the 1,133
supplemental routes; they must remain explicitly unverified unless replaced by
source evidence or reviewed overrides.

The 13,053-id static whitelist is not required by the decoder. `Sector_ReadStatic`
(`0x007b6ba6`) passes every stored graphic, z and hue to the instance constructor
(`0x007c0120`) without consulting such a list. The whitelist excludes graphics
3674 and 6345 present in four facet records, and 195, 196 and 643 present in three
`StaticChunk.uop` records. All observed facet static graphics have tileart records.
It must not filter a conversion. This audit changes no runtime or conversion policy.
