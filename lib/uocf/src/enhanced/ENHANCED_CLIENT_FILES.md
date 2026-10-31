# Enhanced Client Asset Reference

This document contains informations about Enhanced Client data files and
explains the EC map/static material pipeline as implemented in
this repo. It is written for contributors and agents who need to change the
pipeline without reintroducing the earlier texture-selection bugs.

The main rule is: keep source facts, derived metadata, runtime policy, and
manual overrides separate. A numeric texture id, file path, or UOP package name
is evidence, not by itself a rendering decision.

## 1. Evidence Order

Use this order when deciding how EC terrain or statics should behave:

1. Original EC UOP files are primary evidence.
2. Packed UDDP metadata is derived runtime evidence.
3. Active KDL files are reviewed integration or override data.
4. Legacy KDL files are review history only.
5. Current Rust/WGSL behavior is implementation, not source truth.

Exploration and audit outputs belong in tooling paths, not in normal pack code,
unless the data becomes a stable package contract.

## 2. EC Source Files

Editing `.uop` packages (`uop_container::package`):

- `UopPackage` replaces (`replace_file`), adds (`add_entry`,
  `add_entry_by_name`), removes (`remove_entry`) and lists (`list_entries`)
  entries. Edited and added entries get a fresh timestamp data block header.
- `finalize_and_save` keeps the source file's layout: an unchanged package
  re-saves byte for byte (all 40 client packages), and an edit rewrites only
  the edited entry's record and bytes. An entry that no longer fits moves to the
  end of the file. The module documentation gives the full policy;
  `SaveLayout::Compact` writes the old gap-free layout.
- **Patcher:** the official patcher re-downloads edited entries and deletes
  added ones. Keep edited packages in a client copy the patcher never runs on.
- Compression (`uop_container::codec::encode_payload`): the client stores
  classic zlib level-6 streams (`78 9C`). `encode_payload` compresses zlib
  entries with classic zlib's `compress2` at level 6 (`libz-sys` with the
  `static` feature, built from its bundled zlib 1.3.2 source, so the system
  zlib never decides the bytes). It reproduces every zlib entry byte for byte,
  so a re-compressed unchanged entry always keeps its slot:

  | Client | Packages | zlib entries | Exact (classic 6) | Fit, old encoder |
  | --- | ---: | ---: | ---: | ---: |
  | Enhanced | 40 | 185,253 | 185,253 | 158,228 |
  | Kingdom Reborn | 34 | 104,911 | 104,911 | 91,479 |
  | Classic (UOP) | 9 with zlib | 48,604 | 48,604 | 40,452 |

  The old encoder (flate2's zlib-ng backend, smallest of levels 6..=9) matched
  485 Enhanced entries exactly. The smallest of classic levels 6..=9 matches
  only 63,442, so there is no level fallback. No client package uses the
  Mythic or zlib+BWT flags. Decompression, and every other flate2 user, keeps
  the zlib-ng backend: it is native zlib-ng (`zng_`-prefixed symbols) and
  shares no global symbol with classic zlib. An edited v5 entry also trades its
  ~134-byte signature header for the 12-byte timestamp header, so it nearly
  always fits.

`string_dictionary.uop`:

- Stores strings used by many EC records.
- Most source records store 1-based string offsets. The parser resolves offset
  `n` through dictionary index `n - 1`.
- Typical values are virtual paths such as `Data\WorldArt\02000130_...tga`,
  shader names such as `UOStaticTerrainShader`, or material names.
- One entry, `build/stringdictionary/string_dictionary.bin`: `u64` (looks like a
  FILETIME), `u32` declared count, `u32`, then `u16` length + UTF-8 strings. The
  client file declares 110856 strings and stores 110855. `UoStringDictionary`
  keeps both counts (`declared_count`, `len`), and `to_payload_bytes` writes the
  client payload back byte for byte.

`facet*.uop`:

- Stores KR/EC map chunks under paths like `build/sectors/facet_0M/XXXXXXXX.bin`.
- EC (client trace and measurements:
  [uosa_facets.md](../../../../scripts/dev-helpers/ec-research/ghidra-facets/results/uosa_facets.md)):
  - Each decompressed `.bin` is a 64x64 tile chunk in column-major order.
  - Header: facet id byte and file id word.
  - Each cell stores land z, land graphic id, delimiter records, and static
    records. Static records store graphic id, z, and hue.
  - The decoder converts this to classic-style 8x8 map blocks plus statics. The
    land graphic id from the facet/map is the terrain query id used by runtime.
  - Entries `build/sectors/facet_%02d/%08d.bin` (zlib), sector id column-major:
    `ceil(h / 64) * (x >> 6) + (y >> 6)`; Tokuno's last sector row and column
    are partial, padded with land 0 at z 0. `build/sectors/facet_%02d/data.map`
    (uncompressed) holds `u32 height, u32 width`. Packages are UOP version 5,
    1000 entries per block. The variants `facet0x/1x/2x/5x.uop` use the base
    facet's names, ids and header byte.
  - Square: `i8 z, u16 land, u8 n, n x {u8 dir, i8 z, u32 land}, u8 m,
    m x {u32 graphic, i8 z, u32 hue}`. The client rejects a whole sector for a
    direction above 7 or for bytes left after the last square; a missing
    sector is a hole.
  - Delimiters copy the neighbour's land z and id: a direction is present
    when every nonzero component of its offset leaves the sector and the
    neighbour is on the map, in ascending direction order (offsets
    `(-1,0) (-1,-1) (0,-1) (1,0) (1,1) (0,1) (-1,1) (1,-1)`). All 34,841 shipped
    sectors (10 packages) re-encode byte for byte; some delimiters are stale
    (facets 0-4: 0.004-0.08% of squares; TerMur: 2.0%, including off-map ones).
    A facet built from the Classic Tokuno files matches the shipped one in
    every land, z and static, and in all but those stale delimiters.
  - UOCF: `facet_sector` (geometry, numbering, encoder, validation, the
    delimiter rule), `facet_package` (`EcFacetPackage`: edit land and
    statics by map coordinates, neighbour delimiters kept in step across
    sectors, layout-preserving save, from-scratch packages with `data.map`),
    `facet_encoder` (Classic map + statics to `facetN.uop` + `facetNN.dds`),
    and `uocf-cli facet` (`dump-cell`, `patch-cell`, `rebuild-radar`,
    `from-classic`, `dump-def`, `patch-def`).
  - Runtime static art collection preserves placed static hue ids. Art shaders
    apply those ids through the preconverted `hues.uddp` lookup texture
    (`textures/hues.rgba8888`), where each hue occupies a 256-pixel strip
    containing the 32 classic hue entries expanded to 8 pixels each (linear
    ramp between entries by default, as EA's atlas, or flat blocks).
- KR (`UO - Old KR`, `facet0..4.uop`, 7168/7168/900/1280/529 sectors):
  - The EC sector with 3 more bytes per square after the land id:
    `i8 z, u16 material, u8 subtype, u16 classic_land`, then the EC
    delimiters (holding the neighbour's z and material) and statics.
    The material is a `TerrainDefinition` id and `subtype` picks its alias
    (0/1/2); the client renders from them and does not read the legacy
    (Classic) id ([uokr_facets.md](../../../../scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md)).
    The KR reader only logs leftover bytes, a direction above 7 and a
    mismatched header; UOCF rejects them. Same entry names, header,
    column-major numbering over the ceil size, `data.map` (first entry,
    uncompressed) and delimiter rule as EC; off-map squares are all zero bytes.
    Packages are UOP version 4, 100 entries per block, zlib sectors, timestamp
    data block headers.
  - All sectors re-encode byte for byte; delimiter presence and order follow
    the EC rule on every square (23/14/0/4/0 squares hold stale values).
  - Decoding needs no dictionary (the square stores its Classic land id).
    Encoding maps Classic -> `(material, subtype)` with
    `kr::facet::kr_land_map` (generated from the shipped facets; see
    [KR_FACET_MAPPINGS.md](../kr/facet/KR_FACET_MAPPINGS.md)). The former
    `TileDictionary.bin`/`StaticDictionary.bin` were .NET BinaryFormatter blobs
    and are removed.
  - UOCF: the EC `facet_sector`/`facet_package` code with
    `ClientDialect::KingdomReborn`, `kr::facet::facet_decoder`,
    `kr::facet::facet_encoder` (Classic map + statics to KR `facetN.uop` +
    `facetNN.dds`), and `uocf-cli facet` (`--kr`, or detected).
  - `StaticChunk.uop` (`kr::facet::static_chunk`): 145 entries (names
    unresolved) of `u16 count` + Classic static records with x, y in 0..32:
    32x32-square chunks keyed `cx << 10 | cy` in the client, which builds
    its static objects from them in memory and has no writer of the file.
    All re-encode byte for byte; map decoding does not need it.
    `TerrainChunk.uop` is mounted but not shipped.

`facetNN.dds` (loose files, `facet%02d.dds`):

- The radar image: DXT1, one pixel per square, full mip chain of
  `floor(log2(min(w, h))) + 1` levels, NVTT header. The client copies the raw
  blocks and never recomputes land colours.
- Regeneration recipe (fitted offline, 0.977-0.992 palette-index agreement):
  colour `radarcol.mul[land]`, or `radarcol.mul[0x4000 + graphic]` of the
  highest static at or above the land (the later on equal z; hue ignored);
  land only is shaded by `D = z(x+1, y) - z(x, y+1)`: x0.6 for `D <= -3`, x1.4
  for `D >= 3` on the 5-bit channels; `c << 3 | c >> 2`; mips from 2x2 averages
  of the decoded level above. `facet_radar` implements it (with the `dds`
  crate's BC1 encoder), writes the shipped header, parses and writes the shipped
  files back byte for byte, and updates only the pixels around edited squares
  (identical to a full rebuild at every level).

`facets.uop`:

- `build/facetdefinition/%08d.bin`, ids 0..6: `u8 id, u32 name cliloc,
  u32 width, u32 height, u8[3] unknown (0x40 0x40 0x01), u32 n`, then regions
  `{u32 index, u32 name cliloc, u8 r, g, b, u8 physical facet (0xff = self),
  u32 m, m x {u32 x1, y1, x2, y2} inclusive}`. The unknown bytes and the region
  index are stored but never read by the client; `facet_definition` keeps them
  verbatim and re-encodes all 7 records byte for byte. Facet 2 region 5 ships an
  inverted rect.

Facet sizes in the executable (`client_exe::facet_dimensions`, `uocf-cli exe`):

- The client sizes each facet from `UOFacet::g_FacetDimensions`, 7 records
  `{char* name, u32 width, u32 height}` in `.data` (EC `UOSA.exe` file
  0x9a72a8; located in any build by the 7 name pointers). Changing a map size
  means patching a copy of the executable: `uocf-cli exe patch <exe> --output
  <copy> --facet N --width W --height H [--split X | --no-split]` writes the
  copy (a new file outside the client folder) and `<copy>.facet-dims.json`
  (original, source and patched SHA-256, the values); a copy is patched again
  only with its manifest. Sizes: at most 32704, multiples of 8 (64 avoids
  padded sectors), at most 65536 sectors. The PE checksum is recomputed
  (`UOSA_eclo.exe` ships a wrong one); signed builds are refused. `exe show`
  prints the table, split and checksum of a build (5 traced builds by SHA-256,
  any other by signature).
- The x = 5120 seam: `IsSameFeluccaTrammelHalf` makes facets 0 and 1 not
  build a chunk across x = 5120 (three imm32 0x1400 in UOSA, none in KR).
  A new layout must move it (`--split X`) or disable it (`--no-split`,
  0x8000).
- The executable alone is not enough. The data must have the new size too:
  `facetN.uop` (and `facetNx.uop`) with ceil(w/64)·ceil(h/64) sectors in
  column-major order (a new height renumbers every sector), `data.map`,
  the FacetDefinition width/height (and region rects) in `facets.uop` (EC; it
  sizes the radar), and `facetNN.dds` at 1 pixel per square with the full
  mip chain; plus the server's map. `uocf-cli exe check-data <exe> <client
  dir>` compares them with the table (all shipped EC and KR data agree).


`TerrainDefinition.uop` (`enhanced::terrain_definition`):

- Owns EC terrain/material semantics. One zlib entry
  `build/terraindefinition/{id}.bin` (decimal, no padding) per material: 201
  in the Enhanced Client, 191 in Kingdom Reborn. Layout traced in
  `scripts/dev-helpers/ec-research/ghidra-terrain/results/uosa_terrain.md`
  (`TerrainDefinition_Read` 0x00845c25): u32 name (string index), u32 id,
  f32 `field_34` (no reader, always 0), f32 `coverage_exponent` (stored as
  the compatibility field `unk2`), f32 `field_3c` (no reader, always 0), u32
  alias count + aliases `{u32 subtype, u32 legacy_land_id, u64 flags}`
  (Kingdom Reborn: u32 flags), u8 shader block present (1 in every record),
  u8 `field_04`, u32 shader name index, u8 texture item count + items
  `{u32 path index, u8 source_kind, f32 stretch, u32 sampler_index, u32
  uv_set}`, u32 + i32 techniques, u32 + f32 tweaks (4 each in every record;
  compatibility fields `unk8`, `unk9`, accessors `techniques()`,
  `tweaks()`). Nothing follows; trailing bytes are rejected.
- String indices are 1-based: index N is the dictionary's string N - 1
  (record 0 names index 110548, string 110547 `void_no_draw`).
- A square names `(material, subtype)` (Kingdom Reborn sectors directly, EC
  sectors through `legacyterrainmap.csv`); the alias with that subtype
  supplies the cell flags. Subtype equals the alias position in every
  record. `BuildTerrainMesh` dereferences the subtype-0 alias of every drawn
  material without a null check, so a definition without one crashes the
  client. `legacy_land_id` is a representative Classic land id (0 in 104 EC
  aliases) with no client reader; non-zero values are the runtime terrain
  slot ids of the tools, and the material id is the fallback slot.
- Alias flags use the tiledata / `TaeFlag` bit positions. The client reads
  impassable (bit 6), ignored (8), hover-over (25) and no-draw (33); wall
  (4), wet (7), surface (9), mongen (16) and no-house (32) are kept but
  unread. Kingdom Reborn flags are u32: KR bit 26 is EC bit 33 no-draw (the
  void definition: KR `0x4000100`, EC `0x200000100`) and KR bit 19 is
  probably EC bit 32 no-house (medium confidence);
  `kr_terrain_flags_to_enhanced` and `TerrainDefinitionEntry::kr_to_enhanced`
  convert.
- `texture_repetition` is the authored stretch (UV scale 1 / stretch); losing
  it shrinks large authored textures into a tiny tile. `sampler_index` equals
  the item position in 625 of 657 items and is only a deterministic final
  tie-breaker of the primary layer choice; `source_kind`, `uv_set`,
  `field_04` are 0 everywhere.
- Writing: `to_bytes_dialect` is the exact inverse of `from_bytes_dialect`
  (201/201 EC and 191/191 KR records byte for byte);
  `detect_terrain_definition_dialect` picks the layout of a package.
  `apply_terrain_definition_edits` and `write_terrain_definition_package`
  replace, add and remove entries in a copy of the package (unchanged
  definitions skipped, classic zlib, timestamp data block header).
- Checks: `TerrainDefinitionEntry::validate` (subtype-0 alias, unique
  subtypes at their positions, shader block, 4 techniques and 4 tweaks) and
  `check_terrain_references` against the `(material, subtype)` pairs of the
  sectors and the csv (`TerrainReferences`): the shipped EC (245 pairs) and
  KR (203 pairs) data pass. EC land id 0 (70,081 squares, the padding of
  partial sectors and void land) has no csv route.
- Editing: the inspector's UOP browser, on a `TerrainDefinition.uop` entry,
  has an **Edit TerrainDefinition.uop** button: a table of every definition
  (coverage exponent, shader from the package's shaders, duplicate, delete)
  and, for the selected one, its aliases (legacy land id, named flag
  checkboxes in the package's dialect, add / remove last), layer stretch and
  sampler, techniques and tweaks, with undo and the checks; a definition
  that would crash the client is not saved.
- Tests: unit tests in `terrain_definition.rs`; the ignored
  `terrain_definition_client_files` checks every EC and KR record byte for
  byte, the checks, the sector and csv references, and that a rewritten
  package changes only the edited entry.

`tileart.uop`:

- Owns EC item/static semantics, including flat surface-like statics.
- Record layout and field meanings follow the client parser
  (`UOTileArt::ParseObjectDefinition` in UOSA.exe), documented in
  `scripts/dev-helpers/ec-research/ghidra-tileart/results/uosa_tileart.md`
  (sections 2, 5, 6 and 8) and verified on all 37756 client records.
- Records are version 4; the client and `TileArtEntry::parse_raw` reject any
  other version.
- The fixed part holds the name string index, tile id, `has_world_art`,
  `uses_enhanced_tileart`, world/legacy art scales, `field_60` (always 0, not
  read by the client), `old_id` (the classic tile id the art replaces, default
  0xffffff; not read by the client), `rotation_degrees`,
  `body_type` (default 400; an equipment appearance is only worn by a mobile
  with this body type), `alternate_animation` (formerly `field_e8`),
  `light_scale`, `light_field_70` (always 0, not read), light x/y offsets,
  `house_join_bits`, two 64-bit flag fields, facing, and the EC and legacy
  image windows. `facing` is copied to the sprite component; no client reader
  was found, so it is probably unused (medium confidence).
- `alternate_animation`: when the item is equipped in layer 2 (two-handed),
  the wearer's stand, walk and run actions use the alternate entry of the
  animation action map. The client copies the raw byte and tests it for
  non-zero (the parser reads any non-zero byte as true); client data holds
  only 0 and 1, set on 30 records (gargoyle two-handed weapons, halberds,
  creature placeholders).
- Light fields: `light_scale` scales the light sprite; `light_offset_x/y` are
  rotated by the view and added to the light sprite position. The client only
  creates the light component when the light shader block (block 3) is set.
- `house_join_bits` (`HOUSE_JOIN_*` constants): bits 1, 2 and 3 are required on
  the north-west corner cell, the west edge (x = 0) and the north edge (y = 0)
  of a house; bit 5 (`HOUSE_JOIN_RESTS_ON_FLOOR`) lets the piece stand
  directly on a floor or on a load-bearing piece below, so the client refuses
  it on stairs and teleporter cells and above stairs (pieces without it are
  supported only by a connected supported neighbour); bit 14
  (`HOUSE_JOIN_SUPPORTS_ABOVE`) makes a supported piece support the cell above
  it. The client draws unsupported pieces with hue 43
  (`HOUSE_UNSUPPORTED_HUE`) but still commits the design. Bits 6..13 are the
  west, east, north and south connectors (two kinds per side, matched with the
  same kind on the facing side). Bits 0, 4 and 15 are not read by the client.
- `flags2` is not a copy of `flags1`; the client tests only bits 18 (paperdoll
  partial hue), 19 (`UseNewArt`) and 24 (`Animation`) in it. Bit 38
  (`TaeFlag::Unused2`) is never set and never tested.
- Property keys (`PropertyKey`): 0 `Weight` (the client tests `weight > 90`),
  3 `Height` (copied to the world object), 6 `Layer` (equipment layer,
  formerly `Slot`), 8 `Appearance`, 9 `Race`, 10 `Gender`, 11 `Paperdoll`
  (appearance override: the definition of `value + 10000000`, used before
  `Appearance`). Keys 1, 2, 4, 5 and 7 (`Key7`, formerly `OffC8`; set on beds,
  curtains, banners and cloth) have no constant-key reader. props2 mostly
  mirrors props1; the client reads it only for the paperdoll gump (keys 11 and
  8, before props1).
- The variable part holds two property vectors, stack aliases, appearance
  filters, an optional sitting block, radar color, four shader blocks and the
  effects tail:
  - appearance filter subtypes: 0 = appearance by owner body type, 1 = by
    owner gender, 2 = animation appearance by owner body type; any other
    subtype is an error.
  - sitting: a 0/1 flag (other values are errors), then 6 32-bit words:
    `direction_mask` (allowed seated facings, `SITTING_DIRECTION_*`: 1 north,
    2 east, 4 south, 8 west; the client snaps the facing to the nearest allowed
    one), signed `seat_offset_x` and `seat_offset_y` (formerly `field_04` and
    `field_08`; the client draws the seated mobile at minus these values),
    `window_shift` (moves the split column of the chair-back overlay for north
    and west facings), and `field_10`, `field_14` (not read by the client).
    Before this was fixed the repo read 4 words, which shifted the
    rest of all 170 sitting records: their radar color was garbage and all four
    shader blocks were lost.
  - effects: u8 count, then that many effect definitions (see
    `EffectDefinitionCollection.uop`), decoded into `TileArtEntry::effects`
    and `ArtData::effects`. They end the record: trailing bytes are an error.
    Client records hold particles (572 at all nesting levels), 23
    PropertyFlag selectors keyed by object hue and 1 LOD selector; 313 records
    have 1 effect and 16 have 2.
- Shader blocks: 0 = WorldArt (present exactly when `has_world_art` is set),
  1 = TileArtLegacy, 2 = TileArtEnhanced, 3 = light texture
  (`*_light_window.tga`). Block 0 is used as the primary EC visual block and
  block 1 as the preferred classic/legacy visual block. Each block stores
  `field_04` (formerly `unk1`), the shader name index, texture items, Technique
  (i32 values, formerly `unk8`) and Tweaks (f32 values, formerly `unk9`).
- Each texture item stores string offset, `source_kind` (formerly `unk4`),
  `texture_stretch`, `sampler_index` (formerly `unk6`) and `uv_set` (formerly
  `unk7`). These are copied into `tilemeta.uddp` texture-ref metadata.
- Flag bit 34 (`TaeFlag::Rotated`, formerly `Unused1`) marks rotated tile art:
  the client applies `rotation_degrees` and uses the full texture size in the
  UI.
- Current tile type classification is render-oriented and derived from the
  block 0 shader name, not stored in the file:

  - `UOWaterShader` -> liquid
  - `UOStaticTerrainShader` -> solid/surface-like
  - `UOSpriteShader` + `Rotated` flag -> solid/surface-like
  - `UOSpriteShader` + first texture stretch not equal to `1.0` ->
    solid/surface-like
  - otherwise regular static art
- Current code does not use texture-block `field_04` (formerly `unk1`) for
  routing. It is 0 in every client record (with the old or the fixed parser),
  so it cannot distinguish surface-like tileart.
- Current interpretation of tileart texture fields (client code evidence):
  - texture-block `field_04`: stored at `UOShaderDefinition` +0x04, meaning
    unknown, always 0.
  - item `source_kind`: 0 (`TEXTURE_SOURCE_NORMAL`) loads the texture, 2
    (`TEXTURE_SOURCE_WRAP`) loads it and forces wrap addressing, any other
    value makes the client load no texture; always 0 in client data.
  - item `texture_stretch`: values other than 1.0 switch the sampler to wrap.
  - item `sampler_index`: not read by the client, which binds the items by
    their position in the block (item 0 the base map, the rest the shader
    maps); usually equal to the item position. The renderer still matches item
    water and surface materials by it, which gives the same result on current
    client data.
  - item `uv_set`: the texture coordinate set index (low byte of the Gamebryo
    texture map flags), 0 or 1.
- Writing (`tileart` module): `TileArtEntry::to_bytes` is the exact inverse of
  `parse_raw`; all 37,756 client records re-encode byte for byte (ignored test
  `tileart_client_records`). The counts are written from the vectors
  (`sync_counts` updates the stored count fields), booleans as 0/1, unknown
  flag bits kept. `apply_tileart_edits` / `write_tileart_package(source,
  output, records, removed)` replace, add and remove records in a copy of the
  package; untouched records keep their bytes, edited ones keep their
  compression flag and get a timestamp data block header.
- Entry names: every record is `build/tileart/%08d.bin`
  (`tileart_entry_path`), and the client looks a definition up by that id; the
  stored tile id is only copied (`GetTileId`). 25 client records store another
  tile's id than their entry name (names 43957..43963 store 43956, 43965..43974
  store 43964; 39777 -> 39676, 39854 -> 39852, 40146 -> 40147, 42091 -> 41516,
  42092 -> 41517, 10000344 -> 10000332, 10000584 -> 100000584, 10000945 ->
  10000877), so edits are keyed by entry hash, not by stored id. A missing id
  falls back to definition 99999999.
- Adding a tile id: add `build/tileart/%08d.bin` with the stored tile id set
  to it (the inspector's "Duplicate selected" does this). Nothing else is
  required: no other file lists the tile ids, and `precachelist.dat` only
  preloads.
  A new name needs a `string_dictionary.uop` string (the record stores its
  1-based index) and new art needs `Texture.uop` / `LegacyTexture.uop`
  entries for the texture paths; a copy can keep the source's name and
  textures. The client also reads a loose
  `Data/Definitions/TileArt/%08d.tileart` override. Servers send item graphics
  as u16, so ids above 0xFFFF are client-side only (paperdoll overrides,
  placeholders).
- Kingdom Reborn records are version 3 (`TILEART_KR_RECORD_VERSION`; the
  version picks the layout in `parse_bytes` / `to_bytes`): one flag byte
  (`has_world_art`; no `uses_enhanced_tileart`), no `alternate_animation`,
  u32 flags, a 4-word sitting block, no radar colour, 3 shader blocks (EC
  blocks 0, 1 and 3; no TileArtEnhanced) and the KR effect grammar
  ([uokr_facets.md](../../../../scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md), answer 9). All 17,086 KR records re-encode
  byte for byte; writing fails on a value v3 cannot hold.

`EffectDefinitionCollection.uop`:

- One entry per server effect id (`build/effectdefinitioncollection/%08d.bin`);
  the payload is a u32 count, then that many effect definitions, ending the
  entry exactly. `effect_definition::parse_effect_definition_collection`
  decodes it.
- Entry id = 0xC7 packet effect - 1 (0x70/0xC0 carry none) (`collection_id_from_packet_effect`);
  all top-level definitions play together; particle files are in
  `EffectTexture.uop` under `data/effects/`.
- Effect definitions (`effect_definition.rs`) are a u32 type key and a body:
  0 Particle, 1 Sound, 2 RoundRobin, 7 PositionOffset, 10 Wait, 11 LOD,
  12 PropertyFlag, 15 Earthquake, 16 Random, 17 UserPreference. Lists and maps
  nest further definitions. Unknown keys and Wait `aligned` / PropertyFlag
  `mode` values other than 0/1 are errors, as in the client; nesting deeper
  than `MAX_EFFECT_NESTING_DEPTH` is our own limit (the client has none).
- Particle names are client string dictionary indices; `ParticleEffect::name`
  resolves them.
- UserPreference `mask` bit 0 (`USER_PREFERENCE_FOOTSTEPS`) is set when the
  user disabled footsteps; the other bits are unknown.
- Particle `attach_mode` (formerly `field_30`): with
  `PARTICLE_ATTACH_PROJECTILE_ONLY` (2) the client does not attach the effect
  to the world object; it plays only while the item is shown in flight
  (thrown models). Client data: 2 on thrown models, 0 on arrows, 1 elsewhere;
  the client treats 0 and 1 alike. Particle `field_3c` must be 0 (the client
  refuses a Nif effect with 1 and a Chaos effect with any non-zero value).
  Sound `value_08` is copied into the sound instance but never read (1.0 in
  the data, probably an intended gain).
- Formats, field meanings, confidence and verification against the client data
  are in `scripts/dev-helpers/ec-research/ghidra-effects/results/uosa_effects.md`.
- `effect_definition::EffectDefinitionCollection` holds every entry in package
  order. Entries are keyed by collection id, or by name hash when no id below
  `EFFECT_COLLECTION_ID_SCAN_END` hashes to them. Client data has 269 entries,
  with ids up to 70006; 2 entries hash to no id.
- `encode_effect_definition_collection` is the exact inverse of the parser.
  All 269 client payloads re-serialize byte for byte. `encode_tileart_effects`
  writes the tileart effect tail (u8 count); `TileArtEntry::to_bytes` uses it.
- `write_effect_definition_collection(source, output)` writes a copy of the
  package and refuses to overwrite `source`. In the copy:
  - unchanged entries keep their compressed bytes and data block header;
  - edited entries keep their compression flag and get a fresh 12-byte data
    block header (`u16 3`, `u16 8`, FILETIME of the current time;
    `data_block_hash` is its Adler-32);
  - new entries get the same fresh header;
  - removed entries leave an empty slot.

  The copy keeps the source layout, so without edits it is byte-identical to
  the source.
- KDL text form (`to_kdl_string`, `from_kdl_str`, `load_kdl`, `write_kdl`):
  - one `entry <id>` (or `entry hash=`) node per entry;
  - one node per definition (`particle`, `sound`, `round-robin`,
    `position-offset`, `wait`, `lod`, `property-flag`, `earthquake`,
    `random`, `user-preference`);
  - properties at their default value are omitted;
  - NaN and the infinities are strings;
  - with a string dictionary, particles carry an informational `name=`, which
    is checked on reading and can replace the index.

  The schema is in `effect_definition/kdl_text.rs`. The KDL round trip
  reproduces the client collection exactly.
- `EffectDefinitionCollection::validate` reports per-entry issues. With the
  string dictionary and `EffectTexture.uop` given, the client data has only 4
  errors: missing particle files in entries 9011, 9500, 9501 and 9533.
  - Errors:
    - unresolved particle names and missing particle files;
    - non-zero `field_3c`;
    - non-finite floats;
    - Wait min above max;
    - nesting too deep.
  - Warnings:
    - empty entries;
    - unknown attach modes;
    - `value_08` other than 1.0;
    - Random weights not summing to `RANDOM_WEIGHT_TOTAL` (within
      `RANDOM_WEIGHT_TOLERANCE`), or negative;
    - unreachable LOD levels and flag-mask cases;
    - duplicate map keys;
    - unused flag-mask defaults;
    - inert position offsets;
    - UserPreference zero or unknown mask bits.

  Unknown definition keys never reach the tree: the parser rejects them.
- Kingdom Reborn (`ClientDialect::KingdomReborn`, the `*_dialect` functions):
  KR type keys are the EC keys minus one from KR 5 (container) on, because EC
  inserted the tile effect at 5 (and appended fade at 18); particles have no
  `hued` byte (27-byte body); the property flag mask is a u32. All 258 KR
  entries re-encode byte for byte ([uokr_facets.md](../../../../scripts/dev-helpers/ec-research/ghidra-facets/results/uokr_facets.md), answers 3 and 4).

`EffectTexture.uop`:

- Version 5, one block of 1000 slots. It holds 829 entries, all zlib-compressed;
  the remaining 171 slots are empty (all-zero records). The package writer
  skips empty slots (`UopFile::is_empty_slot`) and does not count them in
  `file_count`.
- Entries are named `data/effects/<lowercase file name>`. The file kinds in
  client data: 466 `.nif`, 62 `.dds`, 61 `.ems` (Chaos scripts), 2 `.psd`, 238
  other.
- `effect_texture::EffectTexturePackage` gives names to the entries in three
  ways:
  - from the string dictionary;
  - from names harvested from the payloads, where NIF and Chaos files reference
    their textures and meshes;
  - optionally from a hash dictionary.

  Without a hash dictionary, 704 of the 829 names resolve. With
  `tools/_shared_assets/Dictionary.dic`, 721 resolve.
- Reading and editing:
  - `read` reads an entry by file name;
  - `put` replaces an entry or adds one, filling an empty slot first;
  - `remove` leaves an empty slot;
  - `save_as` writes a new package, never the source.
- Data block headers are signature records, as in every version-5 package
  (`uop_container::data_block_header`): `u16 4`, `u16 length`, then big-endian
  `u16` key id 20 and `u16` signature length and a 126- to 129-byte integer, so
  134 to 137 bytes. The 12-byte timestamp headers belong to version-4 packages.
  - `data_block_hash` is the lookup3 `hashlittle` (initial value 0) of the
    header; 829 of 829 entries match. It covers the header, not the payload.
  - The signature is RSA (e = 65537, key id 20, 1025-bit modulus) over the
    SHA-256 of the decompressed payload, padded as `0xFF × 32 ‖ digest ‖
    0xFF × 32`. The public keys are in the patcher (`uopatch.exe`), not in
    the client; without the private key a signature cannot be made.
  - The client never checks either: it skips `data_block_length` bytes to the
    payload, never reads the signature, and reads `data_block_hash` only as a
    cache stamp for string table and audio caches that the shipped client
    never uses. A missing header works.
  - The patcher does check them: a normal run compares each entry's header
    length and `data_block_hash` with its manifest and downloads entries that
    differ; a full file check verifies every signature. Edited packages do not
    survive a normal or full patcher run.
  - Replaced and added entries get a fresh timestamp header instead (type 3,
    the current FILETIME, 12 bytes; `data_block_hash` its Adler-32), in this
    version-5 package as in version-4 ones: a signature cannot be made for a
    new payload, and the new stamp lets the client's caches see the change.
    `UopPackage::set_edit_filetime` (or `EffectTexturePackage::set_edit_filetime`)
    fixes the FILETIME for reproducible output. An entry replaced by the same
    payload, and every untouched entry, keeps its header and hash byte for
    byte. See
    `scripts/dev-helpers/ec-research/ghidra-uop-header/results/uosa_uop_header.md`
    for the evidence (option 4 of its writer policy).

`.ems` Chaos particle scripts (in `EffectTexture.uop`):

- Played by the Chaos `ParticleSystemManager` for particle names containing
  `.ems`. `chaos_script::ChaosScript` parses them into a typed tree. The
  grammar (keywords from UOSA.exe strings, forms from the 61 client scripts):

  ```text
  script    := "particlesystem" STRING NUMBER "{" item* "}"
  item      := statement | "eventsequence" STRING "{" statement* "}"
  statement := lhs "=" value
  lhs       := PROPERTY | "initial" PROPERTY
             | ["fade" "so"] "final" PROPERTY | ["fade" "so"] "at" scalar PROPERTY
  value     := scalar | "XYZ(" scalar x 3 ")" | "RGBA(" scalar x 4 ")"
             | "CelSetup(" NUMBER x 7 ")" | IDENT | STRING
  scalar    := NUMBER | "random(" NUMBER "," NUMBER ")"
  ```

- Keywords, properties and identifiers are case-insensitive. `//` comments run
  to the end of the line. Client files use CRLF.
- Statements and sequences interleave at system level.
- Identifier values are:
  - booleans (`TRUE`/`FALSE`/`YES`/`NO`; 0 and 1 are also accepted);
  - the `ALPHA_*` blend modes;
  - the particle types.
- Strings are file names: `texture` and `meshName` name `EffectTexture.uop`
  entries.
- The parser keeps unknown statements and lines verbatim and reports errors
  with line and column.
- `to_lossless_string` reproduces the parsed text byte for byte, regenerating
  only the edited elements. All 61 client scripts round-trip.
- `to_canonical_string` pretty-prints the script; parsing it back gives an equal
  tree.
- `validate` reports issues in client data:
  - the malformed number `0.0.5` in two scripts. It is read as its longest valid
    prefix, as C `atof` would; this is assumed, not confirmed in the client;
  - one missing mesh file: `00005045_mark.ems` names `00005054_mark.nif`.
- What the properties do, and the `COLLISIONCHECK`/`INHERITED` syntax, are not
  established.

`AnimationSequence.uop`:

- One entry per mobile body type (`build/animationsequence/%08d.bin`);
  `animation_sequence::AnimationSequence::parse` decodes it and rejects
  trailing bytes. The same format serves the Classic and the Enhanced Client.
- Layout: u32 `stored_id`, 8 i8 and 8 i16 per-direction offset pairs, the
  action table, then the action groups. An action record is u32 action, u32
  frame count, i32 alias (-1 = own frames), f32 fps, its own 8 + 8 offset
  pairs, frame effects (`{u32 frame, effect list}`, the
  `EffectDefinitionCollection` format) and skipped frames (u32 list).
- `stored_id` is the sequence's own body identity: the client checks
  equipment body types against it (wear check) and logs it. It is not a frame
  key.
- Server body to sequence and frames (`enhanced::mobile_body`): the client
  draws server body `S` through the `tileart.uop` creature placeholder
  `10000000 + S`. The sequence is the placeholder's body type; the frame body
  is its props1 key 8 appearance (else `S`, minus 10000000 when above it), a
  non-zero animation appearance (filter subtype 2) replaces it, and body 0 is
  used when the frame body has no frames (401 -> sequence 400, frames 401;
  402 -> 400, 400; 146 -> 124, 124). Ghost bodies 402, 403, 607, 608, 694, 695
  are drawn with hue 0x3e3, and so is a mobile whose 0x77/0x78 flags have bit
  0x80. No `bodyconv`, `mobtypes` or `body.def` is loaded.
- Placeholder race (props1 key 9, default 1) and gender (key 10, default 0)
  order the equipment layers and filter item appearances by the owner's
  gender; `MobileBodyDefinition::race` / `gender` keep them.
  Reference client: 1143 placeholders, 52 falling back to body 0; frame
  bodies 344 and 827 have no placeholder.
- Frames: each plain action owns `5 * frame_count` frames for the stored
  directions 3..7; directions 0..2 are horizontal mirrors of 6..4
  (`direction_frames`). The `AnimationFrame` entry of (frame body, action)
  numbers its frames `1..5 * frame_count` (`FrameEntry::frame_number`); the
  number, not the entry position, gives direction and frame (some files have
  gaps).
- Writer: `AnimationSequence::to_bytes` re-encodes all 370 EC sequences byte
  for byte. Kingdom Reborn (`parse_dialect` / `to_bytes_dialect`): actions have
  no i16 offsets and no skipped-frame list, but an i32 after the alias
  (`kr_extra`, stored at action +0x28 and never read; -1 in all data), and use
  the KR effect grammar. Ten KR sequences repeat action id 0
  (`repeated_actions`). All 252 re-encode byte for byte.

`AnimationFrame*.uop` pixels (`enhanced::animationframe`):

- Header +0x0c is the body of the entry name (`animation_id`); +0x10..+0x17
  (`bounds`) are not read by the client. Frame entry box `x0, y0, x1, y1`:
  size `x1 - x0` by `y1 - y0`, anchor `(-x0, -y0)`, mirrored anchor x `x1`.
- Palette entries are BGRA-ordered RGB plus byte 3, a keep-colour flag: 0
  marks a hueable colour; it is never alpha.
- RLE: a byte `c < 0x80` skips `c` pixels (across rows); otherwise a flag byte
  `F` follows, a leading edge pixel when `F >> 4` is non-zero, `c - 0x80`
  solid pixels, a trailing edge pixel when `F & 15` is non-zero. Solid pixels
  are opaque; an edge pixel keeps its palette RGB with alpha `255 * f >> 4`
  (straight alpha). A coded run past the box, a truncated stream or a palette
  index past the palette is an error.
- Hues (`enhanced::hues`): a hueable pixel takes `table[red]` of the hue
  table (`hue{N:04}.bmp`, 256x2, top row, key `hue - 1`, partial flag 0x8000
  masks to 0x3fff) and keeps its edge alpha. The same rows form
  `build/hues/hues.dds` in `Hues.uop`: 1024 x 1024 uncompressed A8R8G8B8, no
  mipmaps, four 256-texel strips, hue H at strip `H / 1024`, row `H % 1024`
  (strip 0 starts at row 1, strip 3 is empty). The client's 1024 width is
  hard-coded in the hue UV math and the shaders, not read from the texture.
- `Hues.uop` from and to `hues.mul` (`enhanced::hues_mul_uop`): a table row
  puts colour i at x = 8i as `c << 3` and ramps linearly to colour i + 1
  (colour 31 held to the end); bitmap row 1 and the 64 x 64 swatch texture
  `0xFA5C6A1BC0D8B01B` (hue h at x = h / 64, y = h % 64) hold the larger of
  colour 0 and colour 31. This reproduces every shipped bitmap whose colours
  still match the Classic file of the same release. EA's format drops bit 15,
  table start/end, raw names and group headers; the converter keeps them in
  `build/hues/huesmul_metadata.bin`, which the client never reads.
- Hue 0x4001 (shadow; the server sets it for a dead bonded pet through 0xBF
  sub-command 0x19): every pixel is black. Its alpha comes from the inverse
  luminance `inv = 255 - trunc(b0 * .34 + b2 * .07 + b1 * .59)` and the
  palette's min, integer mean and max of `inv`
  (`hues::ShadowPaletteStats`): a level x of 0.5 at the mean, 0..0.4 below
  it, 0.6..1 above it, 0 when the mean equals the min or the max; alpha
  `trunc(x * 130 + 60)`, so 60..190, evaluated in f32 like the client. The
  `mobile_anim_ec.uddp` frame records (version 5) carry the statistics of
  their source file's palette.
- Aliases borrow another action's frames, one level only; action 19 is an
  alias of 0 when flying animations are off; a broken alias falls back to the
  lowest plain action (`resolve_action`). Missing actions play at 16 fps.
- Groups (0 war, 1 peace, 2 mounted, 3 flying, 4/5 state, 6 seated) map
  stand/walk/run, 0xE2 type/sub-type/variant and state transitions to actions
  along a parent chain (`move_action`, `typed_action`, `transition_action`);
  0x6E actions 200..269 go through type 11 of group 1
  (`map_character_animation`).

`AnimationDefinition.uop`:

- One entry per animated item id (`build/animationdefinition/%08d.bin`),
  decoded by `animation_definition::AnimationDefinition::parse`. It has
  nothing to do with mobile bodies.
- Layout: u32 `stored_id`, u32 start set, u32 second set, f32 value (>= 0),
  then frame sets `{u32 key, i32 next (-1 none), f32 frame interval (s),
  u32 count, count x {u32 tile id, u8 effect count, effects}}`.
- Client data: one set (key 0) per entry, no effects; 3 entries store an id
  other than their name (7905 -> 7897, 7923 -> 7916, 18153 -> 18150), with no
  known use.
- Layout, run-time semantics, confidence and verification against the client
  data for both files are in
  `scripts/dev-helpers/ec-research/ghidra-animation/results/uosa_animation.md`.

`MultiCollection.uop`:

- Holds two asset types: one entry per multi (`build/multicollection/%06d.bin`,
  `multis::MultiItem::from_payload_bytes`) and the house customization piece
  tables (`build/multicollection/housing.bin`,
  `housing::HousingPieces::from_payload_bytes`). The Classic Client package
  holds the same payloads. Both parsers, like the client, reject trailing
  bytes.
- Multi entry: u32 multi id (equal to the file id in all 862 client entries;
  the loader uses the file id), u32 part count, then parts `{u16 tile,
  i16 x, i16 y, i16 z, u8 not_drawn, u8 flag2, u32 n, n x u32 string id}`.
  - z is a full signed 16-bit value (client data: -8..123).
  - `not_drawn` (part +0xa, high confidence): the client does not show the
    part (multi renderer, house customization multi appearances, placement
    tests), except for multis whose collection field +0x30 is 24..99,
    probably the new ships. Classic `multi.mul` has flags = 0 (not visible)
    for these parts. 4,026 of 178,808 parts.
  - `flag2` (part +0xb, low-medium confidence): with 1 the renderer uses hue 0
    instead of the multi hue; equals the extra dword of classic `multi.mul`
    records. 1,359 parts.
  - string ids: `string_dictionary.uop` indices of part tags such as
    `house_door`, `hueshadow`, `shipaccess`; the client drops empty strings.
  - `MultiItem::bounds` covers x, y and z of all parts, drawn or not, like
    the client's bounds.
- `housing.bin` (all u32): tile type count, then per tile type the tile type,
  a piece count and the pieces `{category, group, feature_mask,
  tooltip_cliloc, solid list, multi list, window list}`, each list a count of
  `{slot, id}`.
  - Tile types (`HOUSING_TILE_TYPE_*`): 1 stairs, 2 roof, 3 door, 4 floor,
    5 wall, 6 teleporter, 7 misc. Category is the window row (style), group a
    sub-row (walls and misc only); Lua publishes both 1-based.
  - Solid ids are the tile variants, window ids the wall variants with
    windows, multi ids the stair blocks 7600..7675 (slots 1..4 = N/E/S/W).
  - `feature_mask` (`HOUSING_FEATURE_*`): the mask of the last piece of a
    category decides its entitlement against the 0xB9 feature flags, with
    cliloc 1062930 ("upgrade to the latest Expansion Pack") or the Gothic /
    Rustic / Time of Legends clilocs (`HOUSING_ENTITLEMENT_CLILOC_*`).
  - `tooltip_cliloc` is the category name.
  - Client data: 7 tile types, 424 pieces, 3,567 tile references, 74 multi
    references. Misc category 25 group 1 lists solid slot 2 twice (tiles
    17340 and 17342; the client keeps the last).
- Editing `housing.bin` (`housing` module): `to_payload_bytes` re-serializes
  byte for byte; `validate` reports what the client drops or misreads
  (repeated tile types and slots, lists outside their tile type, category
  mask and tooltip conflicts, masks outside the expansion filter, category
  gaps, and, through optional lookups, missing tiles, multis and clilocs);
  `to_kdl_string` / `from_kdl_str` give a lossless KDL text form (schema in
  `housing/kdl_text.rs`); `load_housing_bin` / `write_housing_bin` handle a
  loose file and `write_multi_collection` writes a copy of a package with
  only `housing.bin` replaced (the other entries keep payload, compression
  flag and data block header; `housing.bin` keeps its compression flag and
  gets a fresh timestamp data block header).
- Editing multis (`multis` module): `MultiItem::to_payload_bytes` re-encodes
  all 862 client multis byte for byte (stored id equals the path id in all of
  them; ignored test `multi_collection_client_entries`).
  `MultiCollection::from_items`, `set`, `remove` and `parts_mut` edit the
  collection; `apply_to_package` writes the changed, added and removed multis
  into a package (unchanged multis and `housing.bin` keep their bytes, new
  entries take the compression of the existing multis, ids must be below
  `MAX_MULTI_COLLECTION_ID`) and `MultiCollection::write_multi_collection`
  writes a copy of a package.
- Layouts, field meanings, confidence and verification against the client
  data are in
  [`uosa_housing.md`](../../../../scripts/dev-helpers/ec-research/ghidra-housing/results/uosa_housing.md)
  (decoder: `verify_housing.py` next to it).
- Kingdom Reborn parts have one flag (`not_drawn`, by the data and its place)
  and no `flag2` (`MultiCollection::from_package_dialect`,
  `MultiItem::*_dialect`); all 241 KR multis re-encode byte for byte.

`waypoint.uop`:

- Stores waypoint definition tables and compact waypoint records under
  `build/sectors/waypoint.bin`.
- The waypoint record section has verified `x`, `y`, signed `z`, `facet`, and
  `name_cliloc` fields. EC `mapcommon.lua` confirms the record `waypoint_type`
  is the client `wtype` used for default display behavior. The remaining small
  field is preserved as `value_6`: it is 1 on every waypoint except the type 13
  (Shops) ones, which hold 1 or one of 24 values in 10..=59, probably the shop
  kind. Its client reader was not located.
- Client data: 645 waypoints, 16 type definitions (among them 1 Corpse, 2 Party Members,
  3 Rally Point, 4 Quest Giver, 5 Quest Destination, 6 Resurrection, 7 Points
  of Interest, 8 Landmarks, 9 Towns, 10 Dungeons, 11 Shrines, 12 Moongates,
  13 Shops (499 waypoints), 14 Player, 15 Custom), 106 icons, 12 effects, no
  trailing bytes.
- Writing: `WaypointsPackage::to_payload_bytes` re-encodes the client payload
  byte for byte (ignored test `waypoints_client_file`), keeping `value_6` and
  any trailing bytes; `replace_in_package` and `write_waypoints_package(source,
  output)` replace `waypoint.bin` in a copy of the package (compression flag
  kept, timestamp data block header).
- The extracted EC `waypoints.lua` table exposes the client-facing schema as
  facet-local `x`, `y`, `z`, `type`, `Name`, `Icon`, `Scale`, and optional
  `flag`, but it is not a byte-for-byte copy of the UOP payload.
- Icon and scale values are display-layer data. Default icons come from
  `WindowData.WaypointDisplay.displayTypes`; custom static Lua waypoints store
  explicit `Icon` and `Scale`, and custom user waypoints may encode
  `_ICON_..._SCALE_...` in the waypoint name string.
- `userwaypointwindow.lua` owns the custom waypoint icon palette shown to the
  player. It maps UI selections to icon ids such as `100022` custom,
  `100000` danger, `100042` city, `100045` dungeon, `100053` moongate,
  `100060` shrine, and the shop/service icon range. These icon ids are display
  choices, not proven fields in the compact `waypoint.uop` records.

`LegacyTerrain.uop` (`enhanced::legacy_terrain`):

- 2,723 zlib entries `data/definitions/legacyterrain/%08d.legacyterrain`, one
  UTF-8 XML document per Classic land tile, numbered by the land tile id (the
  `id` attribute on every entry): `<legacyTerrain><terrainType id name
  texMap><typeFlags>` with empty flag elements `impassable` (863), `surface`
  (117), `nohouse` (94), `wet` (67), `wall` (39), `mongen` (3) and
  `no_diagonal` (1). No BOM, XML declaration, comment or entity; CRLF line
  breaks; 21 whitespace shapes (four-space or tab indentation,
  `<typeFlags />` or `<typeFlags/>`, stray tabs and trailing spaces, a few
  blank last lines). The client's reader of these files was not traced.
- Against the Classic `tiledata.mul` (HS layout, 64-bit flags) of the same
  land tile: `name` is the land tile name and `texMap` its texture id on all
  2,723 entries, and six flags match a land flag bit on all of them:
  impassable `0x40`, surface `0x200`, wet `0x80`, wall `0x10`, mongen
  `0x10000`, nohouse `0x1_0000_0000`. `no_diagonal` matches no bit.
- Model: `LegacyTerrainType` (id, name, tex_map, flags in document order) is
  the typed definition. `LegacyTerrainDocument` pairs it with a private
  formatting layer that holds every byte outside the typed values: the
  prolog, whitespace and comments between elements, attribute order, spacing
  around `=`, quote character, the stored spelling of each value, the
  self-closing style and the epilog. A schema-specific scanner fills both; a
  generic XML event round trip was not used because the edits need
  formatting rules, and a typed model plus trivia keeps them in one place.
- Writing: `to_bytes` rebuilds the document from the two, so every client
  entry writes back byte for byte (2,723 of 2,723). An edited document keeps
  its style: a value keeps its stored spelling while it still means the typed
  value, else it is written fresh and escaped for its quote. A removed flag
  drops its line. An added flag copies the formatting of a stored flag, or,
  without one, goes one indentation level below `<typeFlags>`, which opens.
  Removing every flag gives back `<typeFlags/>` in the flags' closing style.
  `LegacyTerrainDocument::new` formats like the most common client shape,
  and `with_terrain` copies another document's formatting.
- Package: `read_legacy_terrain` and `read_legacy_terrain_documents` (fails
  on an entry that is not the entry of its id), `apply_legacy_terrain_edits`
  and `write_legacy_terrain_package(source, output, documents, removed)`
  replace, add and remove entries in a copy of the package. Unchanged
  documents are skipped. A new entry takes the package's compression, and
  edited entries get a timestamp data block header.
- Editing: the inspector's UOP browser shows the definition and XML of a
  selected `LegacyTerrain.uop` entry. Its **Edit LegacyTerrain.uop** button
  opens a table of every definition (name, texMap, flag checkboxes;
  add, duplicate, delete; undo) that saves a copy of the package.
- Tests: unit tests in `legacy_terrain.rs`. The ignored
  `legacy_terrain_client_file` checks every client entry byte for byte,
  re-parses every entry after each flag toggle, and checks that a rewritten
  package changes only the edited entries.

`Texture.uop` and `LegacyTexture.uop`:

- Shared image pools, not ownership boundaries.
- `Texture.uop` usually resolves `build/worldart/{:08}.dds`.
- `LegacyTexture.uop` usually resolves `build/tileartlegacy/{:08}.dds`.
- Payloads are raw DDS or TGA image data. Tooling decodes them for packing, but
  the original payload is the source evidence.
- Kingdom Reborn `TileArtLegacy.uop` holds no tileart records: it is the TGA
  source of the legacy art (`data/tileartlegacy/%d.tga`, 16,384 entries, 3 of
  them unnamed), from which `LegacyTexture.uop`'s DDS were packed. Its last
  entry is a header-only 0x0 TGA, which `textures::decode_tga_rgba8` reads as
  an empty image. Four KR `Interface.uop` images are X1R5G5B5 DDS, which
  `textures::decode_dds_rgba8` decodes as opaque B5G5R5A1.

## 3. Ownership Model

Ownership and render role are separate axes.

- `TerrainDefinition.uop` owns terrain/material semantics.
- `tileart.uop` owns item/static semantics.
- `facet*.uop` owns placed map terrain ids, heights, statics, and static hues.
- `Texture.uop`, `LegacyTexture.uop`, `TerrainTexture.uop`, and
  `EffectTexture.uop` are physical package pools, not semantic owners.

Consequences:

- A low texture id does not prove terrain ownership.
- A `Data\WorldArt\...` path does not prove terrain ownership.
- A `Data\Textures\...` path can be an art-owned support reference.
- The same decoded texture id can appear in both art and land packages when
  different semantic owners need it.

## 4. Terrain Texture Selection

Terrain packing starts from `TerrainDefinition.uop`, not from classic tile ids
or raw texture package membership.

The current primary visible texture selector ranks layers in this order:

1. Layers with a resolved texture id.
2. Non-support names before support names.
3. Preferred repetition before non-preferred repetition.
4. Lower `unk6`.
5. Lower `name_string_off`.

Current support-name heuristic:

- Support for primary exclusion: path contains `noise`, `normal`, `mask`, or
  `_alpha`.
- Wider support-like diagnostic clue: path contains `alpha`, `mask`, `noise`,
  `normal`, `bump`, `ripple`, `flow`, or `distort`.

Preferred repetition is `4.0..=8.0`. This matches the observed EC terrain
albedo/detail layers better than choosing the first layer blindly. It prevents
the old failures where an alpha mask, normal map, displacement-like texture, or
red diagnostic-looking support texture became the visible terrain base.

`unk6` is only a deterministic final tie-breaker after the support and
repetition rules. It is not treated as semantic policy. The packer marks
`TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER` when multiple layers have the
same current rank, so those cases stay visible for audit. Do not promote `unk6`
unless an audit proves it works in at least 3/4 of reviewed cases.

Reason codes written to metadata:

- `non_support_preferred_repetition`
- `non_support_repetition_fallback`
- `support_preferred_repetition_fallback`
- `support_repetition_fallback`

Diagnostic flags written to metadata include:

- selected current support layer
- selected support-like layer
- selected preferred repetition
- fallback reason
- multiple preferred non-support layers
- support-like clue outside current heuristic
- opaque `unk6` tie-breaker

## 5. Layer Roles

For EC terrain runtime blending, layer index currently matters:

- Layer 0 is treated as base/albedo.
- Layer 1 is treated as detail/grunge/secondary albedo.
- Layer 2 is treated as mask/alpha blend control.

This is a runtime interpretation of the layer graph preserved from
`TerrainDefinition.uop`. The packer does not flatten layers into one image. It
packs all source texture ids it can decode, then writes provenance so runtime
can sample base/detail/mask with the authored stretch.

Normals, noise, masks, ripple/flow/distortion clues, and other support textures
are preserved as metadata and packed texture refs when available. They are not
all active shader inputs yet. Do not discard them just because the current
shader only uses base/detail/mask.

For EC tileart texture refs, stable roles are inferred from paths and selection
state:

- `normal` or `_n` -> normal-like
- `alpha` -> alpha mask
- `mask` -> generic mask
- `noise` -> noise
- `detail` -> detail
- `light` or `glow` -> overlay
- primary selected ref -> base
- auxiliary ref or `Data\Textures\...` -> image support
- otherwise unknown support

Tileart auxiliary detection currently treats names containing `noise`,
`normal`, or token segments `n`, `nm`, `nrm`, `norm` as auxiliary.

## 6. Surface-Like Statics

Surface-like statics are still tileart-owned. They render on the ground, but
that does not make them terrain materials.

Build-time flow:

- `tileart.rs` classifies EC tileart entries into regular static, solid
  surface-like, or liquid-like.
- `tilemeta.uddp` stores `TileMetaItemVisualKind::SurfaceLike` for non-static
  tileart classifications.
- It also stores all tileart texture refs, including logical family, physical
  package, stable role, speculative role, `texture_stretch`, `source_kind`,
  `sampler_index`, `uv_set`, block index, item index, and primary/auxiliary
  flags.

Runtime flow for EC surface-like statics:

- Prefer a land-atlas path over the regular EC art-atlas path.
- Ask `tilemeta` for the main EC texture id.
- For surface-like tiles, exact `texture_id == tile_id` matches are disabled
  during main texture selection. This avoids treating a CC art id as if it were
  an EC WorldArt texture id.
- If the selected texture id is directly present in `tex_land_ec.uddp`, use it.
- Otherwise, search terrain provenance for records with that selected texture
  id. If there is exactly one canonical slot, use it. If there is no canonical
  slot and exactly one alias slot, use that. Otherwise fall back through
  `resolve_runtime_slot_id(meta.cc_texture_id)`.
- If nothing resolves, skip the surface-like static and log samples.

This is why flat tileart entries such as marble floor corners can correctly use
the land-style texture from `Data\WorldArt\...` without hardcoding their ids.

### Static Water Base Texture Selection

Previously, some surface-like static water tiles routed to an EC
normal/displacement-looking texture instead of the real visible water
base/albedo texture. In those cases the chosen texture refs were support normal
maps, not the water base texture that should be rendered. This is separate from
regular map water and from regular static art. The affected path is the
surface-like static land-atlas path:

- `statics_collect.rs::resolve_static_visual_kind`
- `resolve_surface_like_tex_land_ec_slot_id`
- `StaticVisualKind::TexLandEcArt`
- `ground_atlas.resolve_tex_land_ec`
- `assets/shaders/world/art/ground.wgsl`

Do not fix this by special-casing water ids in the shader. Texture identity is
selected before the shader samples it. The shader only receives the resolved
atlas layer and UVs.

Current runtime policy:

- Wet/liquid surface-like statics must choose explicit visible roles
  (`Base`/`SecondaryBase`) from tileart texture refs.
- Normal maps, masks, noise, alpha, ripple, flow, distortion, auxiliary refs,
  and unknown support refs cannot become the selected water base texture.
- If no explicit visible ref exists, the resolver falls back through the
  existing terrain/static fallback path rather than rendering a support normal
  map as albedo.

What to verify first:

- Inspect the tileart entry for the affected static id in `tileart.uop`.
- List all texture refs in its texture blocks, including `shader_name`,
  `texture_stretch`, path, extracted `texture_id`, stable role, block index,
  item index, and primary/auxiliary flags.
- Confirm whether the visible water texture and the normal/displacement texture
  are both present in tileart metadata. If only the support texture is present,
  the missing albedo may have to be resolved through TerrainDefinition
  provenance or a reviewed override.
- Check whether the current `tilemeta.main_ec_texture_id` points at a ref whose
  stable role is `normal`, `mask`, `noise`, `alpha`, `ripple`, `flow`, or
  `distort`. If yes, the bug is in tileart main-texture selection.
- Check whether `resolve_surface_like_tex_land_ec_slot_id` reroutes a correct
  tileart texture id to a terrain provenance slot whose selected primary layer
  is support-like. If yes, the bug is in terrain primary selection or in the
  surface-like redirection resolver.
- Preserve `is_wet_flags`: the flag is for water animation and should not
  decide the visible albedo by itself.

Likely correct policy:

- For liquid surface-like statics, prefer tileart refs classified as visible
  albedo/base over support refs, exactly as terrain primary selection does.
- Treat names containing `normal`, `bump`, `mask`, `_alpha`, `noise`, `ripple`,
  `flow`, or `distort` as support for visible-primary selection unless a manual
  review proves otherwise.
- Keep `texture_stretch` from the selected visible ref or resolved terrain
  provenance slot; otherwise water scale and motion will look wrong even after
  the texture id is fixed.
- If the source data does not expose a visible albedo automatically, add a KDL
  override entry for that tile id rather than weakening the global heuristic.

Useful diagnostics to add before changing policy:

- Extend `audit-ec-surface-redirection` or add a targeted report for liquid
  surface-like statics that writes one row per candidate texture ref and marks
  the selected runtime slot.
- Include: `tile_id`, tile type, `is_wet`, shader name, ref path, role,
  support-like reason, texture id, physical package, block/item index,
  stretch/repetition, selected/unselected reason, resolved runtime slot, and
  whether the final slot came from direct land presence, canonical terrain
  provenance, alias terrain provenance, or fallback slot resolution.
- Test against the current failing water tiles and at least one known-good
  surface-like non-water tile, so the fix does not regress marble/floor routing.

## 7. Classic Id Ranges

Classic art ids below `0x4000` are land diamonds in classic art data. Classic
static art ids are stored at `item_id + 0x4000`.

Current handling:

- `tex_art_cc.uddp` decodes art ids `< 0x4000` as 44x44 land diamonds and ids
  `>= 0x4000` as statics.
- Runtime CC static rendering adds `0x4000` to the item graphic id before
  looking up the static art atlas.
- `tilemeta` radar color lookup also uses `id + 0x4000` for item/static radar
  entries.
- EC standard static art often keeps the same id as the CC item slot. Surface
  land-like art is the exception: it often points through tileart metadata to a
  separate EC WorldArt texture id.

Do not assume that a CC item id directly equals an EC terrain material id.
For regular static art it can often match the EC tileart id. For surface-like
art, use the tileart texture refs and terrain provenance resolver.

## 8. End-To-End Workflow

Terrain package build:

1. Load `TerrainDefinition.uop` and neighboring `string_dictionary.uop`.
2. Resolve material aliases, shader names, layer paths, texture ids, layer
   repetition, and preserved unknown fields.
3. Load `Texture.uop` and `LegacyTexture.uop`.
4. Collect every terrain source texture id, plus texture ids referenced by
   `EcTerrainOverrides.kdl`.
5. Decode those texture ids from the EC texture pools.
6. Pack decoded textures into atlas pages.
7. Build sparse slot records and alias records.
8. Write terrain provenance for every material alias and selected/available
   layer.
9. Embed KDL-derived override metadata and transcode metadata when present.

Tile metadata build:

1. Load classic `tiledata.mul` and optional classic radar colors.
2. Load EC `tileart.uop` definitions.
3. Build dense land metadata from classic land records.
4. Build dense item metadata from classic item records plus EC tileart records.
5. Store EC/CC texture ids, crop starts, offsets, visual kind, radar color,
   flags, and full EC texture-ref side tables.
6. Compute tileart texture roles from path names, selected primary refs,
   auxiliary clues, and package/family classification.

Runtime:

1. Load UDDP map/statics, `tilemeta.uddp`, `tex_art_cc.uddp`,
   `tex_art_ec.uddp`, and `tex_land_ec.uddp`.
2. Load loose `TerrainTranscode.kdl` and apply it over embedded transcode data.
3. Load loose `EcTerrainOverrides.kdl` and apply it over embedded terrain
   override defaults.
4. Build land shader lookup entries from terrain provenance and override refs.
5. For statics, choose regular art, CC art, or EC land-atlas rendering from
   source mode and `tilemeta` visual kind.

## 9. UDDP Packages

`tilemeta.uddp`:

- `metadata/land.bin`: dense `TileMetaLandTile` table from classic land
  metadata, mapped into EC-compatible flags.
- `metadata/items.bin`: dense `TileMetaItemTile` table from classic item
  metadata plus EC tileart facts.
- `metadata/item_texture_refs_index.bin`: per-item span table into refs.
- `metadata/item_texture_refs.bin`: all EC tileart texture refs.
- Item records store EC and CC texture ids, crop starts, offsets, radar color,
  flags, height, visual kind, and names.
- Texture refs store texture id, logical family, physical package, stable role,
  speculative role, block/item index, primary/auxiliary flags, stretch,
  `source_kind`, `sampler_index`, and `uv_set` (formerly `unk4`, `unk6` and
  `unk7`; the binary layout is unchanged).
- `metadata/item_materials.json`: per-item shader block material (block index,
  shader name, Technique, Tweaks, `field_04`). Packages that still use the old
  `unk1` key load through a serde alias.

`tex_art_ec.uddp`:

- Contains tileart-owned EC static/art images.
- It must not drop a tileart-owned surface entry just because its visual role is
  land-like. Runtime may choose the land path for that tile, but tileart remains
  the semantic owner.

`tex_land_ec.uddp`:

- Contains terrain material images derived from `TerrainDefinition.uop` plus
  reviewed override texture refs.
- Package layout:
  - `pages/*`: atlas page payloads
  - `metadata/pages.bin`: page dimensions, count, pixel format, used extents
  - `metadata/slots.bin`: sparse slot table keyed by runtime land/art slot id
  - `metadata/terrain_provenance.bin`: material-to-texture provenance
  - optional `metadata/terrain_overrides.json`: embedded KDL-derived defaults
  - optional `metadata/transcode.bin`: embedded transcode table
- `terrain_provenance.bin` v3 stores:
  - material id
  - material name id
  - alias count index
  - alias slot id
  - alias tile flags
  - selected texture id
  - canonical packed slot id
  - selected layer index
  - selected texture repetition/stretch
  - primary texture id
  - primary layer index
  - primary selection reason
  - primary selection flags
- Runtime uses provenance to resolve direct terrain, transcoded terrain, and
  surface-like static texture refs, and to fill base/detail/mask lookup entries
  for the land shader.

`tex_art_cc.uddp`:

- Contains Classic Client land and static art.
- Land diamonds and static art live in the classic `0x4000` split described
  above.

## 10. KDL Files

Active KDL files live in `dynamapper/assets/cc_ec_convtables/`.

`EcTerrainOverrides.kdl`:

- Reviewed manual integration table for facts not currently derivable from UOP
  data.
- Supported actions:
  - `policy`: shader/runtime policy such as smoothing or follow-center
  - `liquid`: reviewed liquid speed/wave parameters
  - `layer`: ordered texture layer substitution with optional stretch
  - `texture`: extra texture id reference with role
  - `ignore`: intentionally hidden/transparent terrain
- The packer embeds it into `tex_land_ec.uddp` as
  `metadata/terrain_overrides.json` and also packs referenced override textures.
- Runtime also loads the loose file and applies it over the embedded defaults.
  Loose overrides are fast for local experiments such as dark/light mountains
  or snow variants. A loose override can only use texture pixels already packed
  into `tex_land_ec.uddp`; missing refs are warned and fall back.
- Prefer reason-code values over prose in machine-readable fields.

`TerrainTranscode.kdl`:

- Loose fallback table mapping CC land ids to EC material ids.
- Keep it to id routing. Do not add visual policy here.
- It remains active for ids not fully resolved through direct
  `TerrainDefinition.uop` aliases.

Legacy KDL files:

- Live under `dynamapper/assets/cc_ec_convtables/legacy/`.
- They are review history and migration reference.
- Runtime paths must not load them.

Modify active KDL only when the source-derived pipeline is wrong or incomplete
and the reason has been inspected. If a rule can be derived from UOP data, fix
the parser or metadata builder instead of adding a broad manual table.

## 11. Conflict Handling

Diagnostics are not a goal by themselves. Add or run them only for targeted
questions: a suspicious tile, a candidate weak signal, an override review, or a
regression in the rendered map. If routing is visually correct and metadata
contains the decision reasons, prefer keeping the runtime path simple.

When a case is unclear:

1. Preserve every source reference first.
2. Record the current automatic decision and reason flags.
3. Prefer narrow source-derived criteria over id hardcoding.
4. Use `EcTerrainOverrides.kdl` for reviewed exceptions.
5. Warn and fall back when loose overrides point to unpacked texture ids.
6. Do not change runtime rendering policy until the metadata phase supports the
   decision.

Known weak signals:

- `unk6` is only a tie-breaker and audit flag today.
- Tileart texture-block `field_04` (formerly `unk1`) is 0 in every client
  record; it carries no routing signal.
- Physical package names are not semantic ownership.
- `EffectTexture.uop` evidence is metadata/support evidence unless linked by a
  real owner; do not infer particle-only or terrain-only behavior from the file
  name alone.
