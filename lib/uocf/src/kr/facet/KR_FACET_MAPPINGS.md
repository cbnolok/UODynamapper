# Kingdom Reborn Facet Mappings

A Kingdom Reborn (KR) facet square stores its material (a `TerrainDefinition` id), its `subtype` (which alias of
that material: 0/1/2) and its legacy (Classic) land id. The KR client renders from material and subtype and does
not read the legacy id (`scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md`). The mapping
therefore runs one way only:

- **Decoding (KR -> Classic) needs no mapping.** `facet_decoder` takes the land from the stored `classic_land`
  and keeps every static (the sectors store Classic static graphics directly).
- **Encoding (Classic -> KR) needs `classic -> (material, subtype)`.** `facet_encoder` takes it from a
  `kr_land_map::KrLandMap`.

## `kr_land_map.rs`

- `SHIPPED_KR_LAND_MAP`: generated from the shipped KR facets 0-4 (69,746,240 on-map squares, 1556 Classic ids):
  the most frequent `material` and `subtype` of each Classic id. `material` is a function of the Classic id
  except for 2444 (KR 58 or 46) and 2446 (KR 60 or 47): exact on 99.9995% of the squares. `subtype` takes more
  than one value for 64 ids: exact on 99.967%. The ignored test `kr_facet_sectors_reencode_identically_and_follow_the_rules`
  (`lib/uocf/tests/real_files/kr_facet_client_files.rs`) derives the table again with `KrLandCounter` and fails when it differs.
- `BUILDER_KR_LAND_MAP_SUPPLEMENT`: the 1133 Classic ids the KR facets never use, taken from `get_tile_map` in
  `tile_mappings_builder.rs` (Manawydan's KR converter table). Unverified. The builder table agrees with the
  facets on most shipped ids, but disagrees on 36 and lacks 19, so the generated table wins where both have an id.
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
