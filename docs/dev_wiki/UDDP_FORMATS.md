# UODynamapper Package (UDDP) Custom Formats

This document describes the structure of custom binary `.uddp` formats used by UODynamapper for fast parsing/ram/vram upload and fast rendering. All `.uddp` files are packaged using the core `UddpBuilder` and compressed (typically with `ZstdNoDict`), containing multiple virtual files (metadata tables and texture payloads).

The container is laid out as the header, the dictionary table and the index, back to back, then the blob region: the dictionaries (by data type) and every payload in key order, with no padding, to the end of the file (`lib/udd-container/tests/uddp_layout.rs`). Payloads are not aligned, so a reader that needs aligned records copies them; the `*_cow` reads borrow uncompressed payloads at any offset. `udd-cli tool validate` decodes every entry and runs each package's own reader.

The container header (version 1.0) and every binary metadata manifest carry a version. Readers accept only the current one and reject any other with "package version N is not supported; repack with the current udd-cli": there are no readers for older layouts.

The current implementation still uses a unified `flags` field in `tilemeta.uddp`, but the active design direction is to separate CC and EC metadata fields more explicitly when the schema is bumped. Treat the tables below as the current wire format, not the final semantic model.

For EC-specific content, remember that the UOP source set is mixed and semantic:

- `Texture.uop` and `LegacyTexture.uop` are shared texture pools.
- `tileart.uop` carries static-art ownership, per-entry clip windows, shader/type hints, and EC item/static metadata.
- `TerrainDefinition.uop` carries land/material ownership, aliases, selected textures, and runtime slot relationships.
- `string_dictionary.uop` is a string resource package and are not visual art sources.

---

## 1. `mapN.uddp`

Classic map packages use dense ids for 32x32-tile map chunks. New packages append one final dense-id metadata record after the chunk records.

### 1.1 Map Metadata Record

The final dense id has data type `Metadata` and stores:

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `magic` | ASCII `UMAP`. |
| 0x04 | `u16` | `version` | Current value: `1`. |
| 0x06 | `u16` | `_reserved` | Zero. |
| 0x08 | `u32` | `map_id` | Facet id, matching `mapN.uddp`. |
| 0x0C | `u32` | `width_tiles` | Map width in tiles. |
| 0x10 | `u32` | `height_tiles` | Map height in tiles. |
| 0x14 | `u32` | `chunk_count` | Number of 32x32 chunk records before the metadata record. |

Runtime loading requires this metadata; map dimensions are no longer configured through TOML.

---

## 2. `tilemeta.uddp`

This package merges Classic Client (CC) physical tile properties and Enhanced Client (EC) rendering definitions into a zero-copy, tightly packed binary.

The current wire format stores CC and EC texture-window fields side by side, but CC and EC behavior flags are still collapsed into one unified bitmask. Future work may split those fields so classic behavior and EC behavior can be preserved independently.
The long-term schema direction is to keep CC and EC ownership/behavior separate in the wire format instead of relying on runtime inference.

**Virtual Files:**

- `metadata/land.bin`: Dense array of `TileMetaLandTile` structs.
- `metadata/items.bin`: Dense array of `TileMetaItemTile` structs.
- `metadata/item_texture_refs_index.bin` and `metadata/item_texture_refs.v4.bin`: the per-item span table and the EC tileart texture refs (`TileMetaItemTextureRef`, 24 bytes). The `.v4` in the path is `TILEMETA_FORMAT_VERSION` 4. Since format 2, `stable_role` is the shader input the item feeds, from the block's shader name and the item position (`EcMaterialStableRole::for_texture_position`; the table is in `lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md` §5). Format 3 adds `TileMetaItemTile::ec_art_flags`; format 4 adds `metadata/item_definitions.bin`. The loader rejects format 1 (`metadata/item_texture_refs.bin`, roles guessed from file names), 2 (`.v2`, no `ec_art_flags`) and 3 (`.v3`, no item definitions) and asks for a repack. Byte 0x0F, format 1's file-name `speculative_role`, is reserved (zero). Flags: `AUXILIARY` (1) = the item is not at its block's primary position (1 for `UOWaterShader`, else 0); `PRIMARY_SELECTED` (2) = the block-0 item `ec_texture_id` comes from, or a block-1/2 item with the same id.
- `metadata/item_materials.json`: per-item shader block material. `TileMetaPackage::uses_water_shader(art_id)` reads it: the block-0 material is `UOWaterShader` (322 items in the shipped client), which makes the item wet in Enhanced mode.
- `metadata/item_definitions.bin` (format 4): the `tileart.uop` records the Enhanced paperdoll reads (`TileMetaItemDefinitions`, read by `uocf::enhanced::paperdoll`): every record from 10,000,000 and every item record with a tileart property or appearance filter the paperdoll reads, or a tiledata layer. Little-endian: a `u32` count, then per record, ascending by entry id: `u32` entry id (the id of its `build/tileart/%08d.bin` name, by which the client finds it), `u8` presence mask, `u8` filter count, the stored tile id (`u32`, only when presence bit 7 says it differs from the entry id: 25 shipped records), one `u32` per present property (bits 0..6: props1 Layer 6, Appearance 8, Race 9, Gender 10, Paperdoll 11; props2 Appearance 8, Paperdoll 11), then the filters: `u8` kind 0 (by owner body type: `u16` row count, rows of `u32` body type and `u32` appearance) or 1 (by owner gender: `u8` gender, `u32` appearance). Animation filters are not stored. The shipped client gives 4,022 records, 46 KB raw, 12 KB stored.

### 2.1 `TileMetaLandTile` Struct (48 Bytes, 8-Byte Aligned)

Represents terrain data.

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `tile_id` | The graphic ID of the land tile. |
| 0x04 | `u16` | `texture_id` | The texture index. |
| 0x06 | `u8` | `tile_type` | Routing type (0=Standard, 1=Solid, 2=Liquid). |
| 0x07 | `u8` | `_pad1` | Padding to maintain alignment. |
| 0x08 | `u64` | `flags` | The 64-bit unified `TaeFlag` bitmask. This is the current wire format, but CC and EC flag semantics are treated separately in the planned schema. |
| 0x10 | `[u8; 4]` | `radar_color` | RGBA values representing minimap colors. |
| 0x14 | `[u8; 20]`| `name` | Null-terminated classic ASCII name. |

### 2.2 `TileMetaItemTile` Struct (64 Bytes, 8-Byte Aligned)

Represents static map items and artwork.

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `tile_id` | The graphic ID of the item. |
| 0x04 | `u8` | `weight` | Item weight. |
| 0x05 | `u8` | `quality` | Item quality (or Layer / Light ID). |
| 0x06 | `u8` | `quantity` | Stack amount or equipment struct ID. |
| 0x07 | `u8` | `hue_extra` | Supplementary hue data. |
| 0x08 | `u64` | `flags` | The 64-bit unified `TaeFlag` bitmask. This is the current wire format, but CC and EC flag semantics are treated separately in the planned schema. |
| 0x10 | `u16` | `anim_id` | Animation mapping ID. |
| 0x12 | `u8` | `stacking_offset` | Visual Z-offset when stacked. |
| 0x13 | `u8` | `value` | Item value. |
| 0x14 | `i8` | `height` | Z-buffer physical height. |
| 0x15 | `u8` | `_pad1` | Padding. |
| 0x16 | `u16` | `ec_art_flags` | `TILEMETA_ITEM_EC_ART_FLAG_*` from the tileart record. Bit 0 (`USE_NEW_ART`): `flags2` has `UseNewArt` (bit 19), so the official EC draws the block-0 KR art (`tex_art_ec`); otherwise it draws classic-style art. 795 items in the shipped client. Read it with `TileMetaItemTile::uses_new_art` or `TileMetaPackage::uses_new_art(art_id)`. Informational client data: the renderer deliberately ignores it. The flag is off for almost every item, including items whose KR art the client still packs, so the client disabled that art on purpose; Enhanced mode draws KR art wherever `tex_art_ec` has a slot, then the Classic art, then magenta. The `UseNewArt`-named bit in `flags` is not it: `flags` merges the Classic tiledata flags and the tileart `flags1`. |
| 0x18 | `[u8; 4]` | `radar_color` | RGBA minimap colors. |
| 0x1C | `[u8; 20]`| `name` | Null-terminated classic ASCII name. |
| 0x30 | `u32` | `ec_texture_id` | EC Texture slot ID. |
| 0x34 | `i16` | `ec_start_x` | X sampling-window start for EC: the start of the trimmed `tex_art_ec` image in the source texture (the tileart start shifted by the left trim). |
| 0x36 | `i16` | `ec_start_y` | Y sampling-window start for EC: the start of the trimmed `tex_art_ec` image in the source texture (the tileart start shifted by the top trim). |
| 0x38 | `u32` | `cc_texture_id` | CC Fallback texture slot ID. |
| 0x3C | `i16` | `cc_start_x` | X bounding box start for CC. |
| 0x3E | `i16` | `cc_start_y` | Y bounding box start for CC. |

Art draw offsets are package-local metadata in `tex_art_cc.uddp` and `tex_art_ec.uddp`. EC art is always alpha-trimmed: the `tex_art_ec` draw offsets keep placement, and `ec_start_x`/`ec_start_y` locate the trimmed image in its source texture, so a source-space crop at that start with the slot's extent gives the slot's pixels (the EC sprite hue masks rely on it). Tiles not packed from their `Texture.uop` texture keep the tileart start. See `docs/dev_wiki/rendering/tex_art_ec_trimming.md`.

Future work is expected to keep the original source texture intact for EC art and let runtime sampling windows handle subrect selection, so cropped packing here should be understood as the current behavior, not the final target.
When the schema is eventually widened, the CC texture coordinates, EC texture coordinates, and the CC/EC flags should be split into explicit fields rather than compressed into one mixed record layout.

Runtime static lights use this item metadata: `flags & 0x00800000` marks a light-source static, `quality` is interpreted as the world light id, and the placed static hue id can tint the light mask through the configured runtime hue source.

---

## 2. `tex_art_cc.uddp`, `tex_art_ec.uddp`, `tex_land_ec.uddp`, and related EC texture packages

These are GPU texture atlases packed into fixed-size pages to avoid texture array limits.
`tex_art_cc` packages Classic Art sprites, `tex_art_ec` packages Enhanced Client Art sprites from the shared EC texture pass, and `tex_land_ec` packages Enhanced Client Terrain Textures from the same pass.

EC textures are split three ways: land (`tex_land_ec`), art (`tex_art_ec`), and auxiliary layers and masks (`tex_aux_ec`, section 2.5).
Avoid BC7 RDO for art and mobile-animation sprites unless a specific asset class has been visually validated. Plain BC7 is a GPU-ready storage format; RDO is a separate lossy entropy-reduction pass that permanently changes BC7 block bytes and cannot be undone by the runtime.

Current EC packing semantics:

- `tex_art_ec.uddp` is keyed by tileart/static ownership and uses per-entry sampling windows. It holds only block-0 `WorldArt` images (`Texture.uop`): the KR art. An item without one has no slot; its Classic art is in `tex_art_cc.uddp`. `LegacyTexture.uop` (the EC legacy-art set) is not read. `metadata/sprite_geometry.json` therefore has no source flag any more; packages that still carry `source_is_world_art: true` parse, and the field is ignored.
- `tex_land_ec.uddp` reads terrain layer images from `Texture.uop` only. On the shipped client every TerrainDefinition and override layer id (207) resolves there; classic land comes from `tex_land_cc.uddp`.
- `tex_land_ec.uddp` is keyed by TerrainDefinition semantics and may pack sparse slot ids plus alias/provenance metadata.
- `tex_land_ec.uddp` also carries EC land material roles in the runtime lookup table: role `0` is base/diffuse, role `1` is detail, role `2` is mask/alpha, and role `3` is normal/normal_map. These role entries point into the same page atlas and are selected by semantic material id, not by raw texture-id ranges. Which TerrainDefinition item fills a role depends on the material's layer class (default/bump: items 0, 1, 2 and, for bump, 3; water: item 1 is role 0, item 0 role 3, item 2 the animated main colour); see `docs/dev_wiki/ASSET_PIPELINE.md`.
- `metadata/terrain_provenance.bin` (magic `ELTP`, version 4): u32 version, u32 record count, then 50-byte records: u32 material id, i32 material name id, u32 alias index, u32 alias slot id, u64 alias tile flags, u32 selected texture id, u32 canonical slot id, u32 selected layer index, f32 selected stretch, u32 primary texture id, u32 primary layer index (item 0, or 1 for water), u8 layer class (`TerrainLayerClass`: 0 default, 1 bump, 2 water), u8 selected layer role (`TerrainLayerRole`, 255 for an override or a missing layer). There is one record per TerrainDefinition item that names a texture: a material that repeats a texture (item 0 and item 1 the same, as on 19 shipped materials such as Black and the cave exits) has a record for each item, last item first, so item 0 resolves; an override replaces every record of its texture. Version 3 stored a primary selection reason and flags from a name/stretch heuristic instead; it is rejected, so repack.
- `tex_land_ec` metadata version 5 (the `pages.bin`/`slots.bin` version) drops the terrain layer table, `metadata/terrain_layers.bin` (magic `TMLY`), that version 4 carried (before that it was a loose `terrain_meta.bin` written by `pack-terrain-meta`). Only a table-driven two-base blend and water ripple in the land shader read it, and both were removed: the detail and mask roles above draw a material's items 1 and 2, and `metadata/source_materials.json` keeps every TerrainDefinition item for the tools. A version 4 `tex_land_ec.uddp` is rejected, so repack.
- `tex_aux_ec.uddp` holds the support images (masks, liquid and surface inputs) that tileart refs name but that do not belong to the art or land atlases (section 2.5). `ec_textures_layers.uddp` was the planned name for that third split.
- If a source texture is claimed by both art and land semantics, duplication across outputs is valid and should be decided by ownership, not by avoiding repeated ids.

Normal-map format note:

- Original EC/KR DDS normal maps are source evidence for channel layout, not the runtime storage format. A known KR source sample, `/home/claudio/test/unpacks/kr_squared/01000013.dds`, is `256x256` DXT1, which implies RGB tangent-space normals rather than a special two-channel normal format. UDDP packages repack those pixels into the land page atlas as RGBA8888 or BC7/BC7-RDO payloads. Runtime shader decode samples the UDDP atlas RGB, maps it from `[0, 1]` to `[-1, 1]`, uses blue as the up component, and may need a green-channel flip only if visual validation shows inverted relief.

**Virtual Files:**

- `metadata/pages.bin`: Binary array of `PageRecord` structs representing page dimensions and occupancy.
- `metadata/slots.bin`: Binary array of `SlotRecord` structs indexed by `art_id` containing the UV mapping.
- `pages/{page_id}.rgba8888`: Raw RGBA8888 pixel payloads (compressed by Zstd via UDDP).

EC art is always cropped, per art entry, not per decoded source texture. The order is:

1. apply the tileart `start_x/start_y/end_x/end_y` clip rect for that art entry
2. alpha-trim inside that clipped rectangle

This rule matters because different art ids can reference the same source texture while sampling different sub-rectangles of it.
It also explains why `tileart.uop` must be treated as the source of entry-local EC art semantics rather than as a blunt texture-id list.
When the cropped payload is consumed as an atlas slot by the renderer, the `tex_art_ec` slot record preserves the original on-screen placement by applying the visual trim to EC draw offsets. BC7 output still keeps atlas allocation and stored page extents aligned to 4x4 blocks.

### 2.1 Atlas Gutters, Extrusion, and BC7 Alignment

Atlas packers distinguish three rectangles:

- **content rectangle**: the source pixels addressed by slot or frame metadata
- **allocation rectangle**: content plus padding/gutter reserved in the atlas allocator
- **stored page bounds**: the cropped `used_width x used_height` payload written into the package

The gutter is not part of logical art placement. It exists to keep texture filtering and block compression from seeing unrelated texels. If the runtime samples with linear filtering, applies UV perturbation, or the asset is a terrain/material surface, the packer must fill the allocation gutter by edge extrusion and include the extruded allocation in stored page bounds. Otherwise the stored page can crop back to transparent black and linear previewers or BC7 blocks can blend against black padding.

BC7-oriented atlas packing uses 4x4 alignment because BC7 encodes independent 4x4 pixel blocks. Content extents are rounded up to 4 pixels and the leading gutter is rounded up to 4 pixels. This prevents two unrelated assets from sharing one BC7 block and keeps row byte calculations valid for direct GPU upload. A greater gutter should be chosen only when the sampler can reach farther than one texel, such as mipmapping, strong UV animation, or post-upscale linear minification. For nearest-sampled masked sprites, a reserved but unextruded gutter can be valid because no filter footprint should cross the content edge.

Current package policy:

- Land/material atlases are filter-ready and edge-extrude allocation gutters.
- Mobile animation atlases edge-extrude allocation gutters, including BC7 alignment padding, because they are inspected and previewed as atlas textures and frequently have transparent silhouettes near frame edges.
- Static art sprite atlases reserve gutters for packing and BC7 block separation, but ordinary static sprite gutters are not a promise of filter-ready extrusion. Runtime world art uses nearest sampling for these masked sprites; a renderer that switches them to linear sampling must also make the package filter-ready or clamp sampling inside the content rectangle.

### 2.2 Metadata Structures

**Page Record (16 Bytes)**

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `page_index` | The index of the atlas page. |
| 0x04 | `u32` | `tile_count` | Number of tiles packed into this page. |
| 0x08 | `u32` | `used_width` | Maximum X extent used in the page. |
| 0x0C | `u32` | `used_height`| Maximum Y extent used in the page. |

**Slot Record (28 Bytes)**

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `art_id` | The graphic ID of the tile. |
| 0x04 | `u32` | `page_index` | The atlas page this tile resides in. `u32::MAX` if missing. |
| 0x08 | `u16` | `page_tile_idx`| The visual tile index on the page. |
| 0x0A | `u16` | `flags` | Bitmask (1=Present, 2=Land, 4=Static). |
| 0x0C | `u16` | `x` | X coordinate in the atlas page. |
| 0x0E | `u16` | `y` | Y coordinate in the atlas page. |
| 0x10 | `u16` | `width` | Width of the tile. |
| 0x12 | `u16` | `height` | Height of the tile. |
| 0x14 | `u16` | `upscale_factor` | Per-slot pixel upscale factor. Atlas coordinates and dimensions are physical pixels; renderer placement uses `width / upscale_factor` and `height / upscale_factor` in source-logical pixels. |
| 0x16 | `u16` | `upscale_algorithm` | Per-slot upscale algorithm code. `0` means none/native; nonzero values identify the build-time upscaler family. |
| 0x18 | `i16` | `draw_offset_x` | Source-specific X draw offset in logical pixels. |
| 0x1A | `i16` | `draw_offset_y` | Source-specific Y draw offset in logical pixels. |

`upscale_factor`, `upscale_algorithm`, and draw offsets are intentionally stored per slot because art packages may mix native, 2x, 3x, and 4x assets and aliased art ids may share pixels while keeping distinct placement. Missing or non-upscaled slots use factor `1` and algorithm `0`.

`tex_art_cc.uddp` may include tileart-derived owner aliases when built with EC metadata available. Those alias slots reuse the packed Classic source rectangle selected by tileart `cc_texture_id`, but keep the owning item id and its own `draw_offset_x/draw_offset_y`. Runtime CC placement resolves the owning item slot first and falls back to the raw Classic texture slot only for packages built without those aliases.

### 2.3 Gump Atlas Metadata

`gumps_cc.uddp` and `gumps_ec.uddp` can store paperdoll equipment gumps in atlas pages. Their atlas slot records also carry a per-slot `upscale_factor`. The stored page rectangle remains physical pixels; UI and paperdoll placement use logical dimensions derived by dividing physical width/height by the factor. Single-gump payloads without atlas slot metadata are treated as `upscale_factor = 1`.

An atlas gump is stored trimmed to its drawn texels (the user, 2026-11-06): the packer cuts the upscaled image to the bounding box of its texels with alpha above 0, widened to whole logical texels, and the slot keeps where that box starts in the original canvas and the canvas size, both in logical texels. A gump with no drawn texel keeps one transparent logical texel at 0,0. The slot manifest (`GASL`, id `0xE000_0001`) is a header of `u32` version, atlas width, atlas height, gutter and slot count, then 28 bytes per slot:

| Offset | Type | Field | Notes |
|---|---|---|---|
| 0x00 | `u32` | `gump_id` | |
| 0x04 | `u32` | `page_index` | |
| 0x08 | `u16` | `page_gump_index` | |
| 0x0A | `u16` x 4 | `x`, `y`, `width`, `height` | The stored texels on the page, physical; width and height are whole logical texels. |
| 0x12 | `u16` | `upscale_factor` | |
| 0x14 | `u16` x 2 | `offset_x`, `offset_y` | Where the stored texels start in the original canvas, logical. |
| 0x18 | `u16` x 2 | `canvas_width`, `canvas_height` | The original canvas, logical. |

The reader refuses a slot whose stored size is not whole logical texels or that runs past its canvas. `udd_assets::gumps::GumpImage` gives the stored texels (`stored_rect`, `stored_rgba`), the canvas size logical and physical, and the whole canvas padded with transparent texels (`canvas_rgba`, `into_whole`); a single gump is whole. Dynamapper's paperdoll composes each layer's stored texels at their offset, and every view of one gump on its own (Dynamapper's Open Gump, the inspector's preview, `read_gump_rgba`) shows its whole canvas.

In `gumps_ec` the Enhanced paperdoll gumps, the equipment gumps 50001..=69999 and the body gumps 12-15, 665 and 666, carry the Enhanced client's drawn rule in their alpha, baked at packing time before the upscale passes (`bake_enhanced_paperdoll_rule` in `lib/udd-conv/src/gumps/ec_gumps.rs`). `UOPaperdoll::DrawGumpLayer` (UOSA 0x00509b30) never reads alpha: it draws every texel whose R, G, B differ from texel 0's, and `UOPaperdoll::BuildImage` (0x0050c790) then makes pure black transparent. So a texel is stored as its colour with alpha 255 when its R, G, B differ from texel 0's and are not pure black, whatever alpha the art gave it (the clear handle of 50669, item 0x4067, and the trident of 60675 are drawn, and body 665's soft texels are opaque), and as transparent black otherwise. Two departures from the client: a pure black texel leaves the layers under it showing, where the client's canvas pass clears them too, and a glow region is left out: an 8-connected region of at least 1024 texels with alpha 0, not the key colour and no channel above 32, which only gumps 50322 and 60322 have (10,844 texels each) and which the client draws as a dark patch. A 256x256 gump, which `BuildImage` skips, is not baked, and Dynamapper still skips it; `gumps_cc` and the single gumps that are not paperdoll gumps are stored as decoded. On the shipped client (checked 2026-11-06 against the version 3 packages), each of the 1019 Enhanced paperdoll equipment gumps composed over body 12 or 13, and each of the 6 body gumps alone, comes out the same by the client's rule and by alpha blending, except the glow texels of 50322 and 60322 (21,688 in all); no drawn texel is pure black.

The page and slot manifests are version 4 (`GUMP_ATLAS_METADATA_VERSION`) and every gump package carries them, even one without atlas pages. A reader refuses an older or missing manifest with a message to repack with the current `udd-cli`: version 2 single gumps have no pixel format byte, and version 3 atlas gumps are neither trimmed nor baked.

A single gump (data type `Gump`, sparse id = gump id) is:

| Offset | Type | Field | Notes |
|---|---|---|---|
| 0x00 | `u32` | `width` | |
| 0x04 | `u32` | `height` | |
| 0x08 | `u8` | `pixel_format` | 0 = RGBA8888, 1 = BC1, 2 = BC7 (`GumpPixelFormat`); any other value is an error. |
| 0x09 | bytes | pixels | RGBA8888 rows, BC1 blocks of `ceil(w/4) * ceil(h/4) * 8` bytes, or BC7 blocks of `ceil(w/4) * ceil(h/4) * 16` bytes. A 0x0 gump has no pixel bytes. |

Pixel formats can mix in one package, and each gump decodes by its own format byte. The lossy packs (the user, 2026-11-06): `pack-gumps --bc1` writes BC1 for every single Classic gump, and `pack-ec-gumps --bc7` writes BC7 for the single Enhanced gumps except those in the Classic colour space, which take BC1 as in `gumps_cc` (on the shipped clients, byte for byte the same payloads). A gump is in the Classic colour space when its alpha is only 0 or 255 and every opaque texel's R, G and B have their low 3 bits clear, as the Classic decoder's `c << 3` makes them (`is_in_classic_colour_space` in `lib/udd-conv/src/gumps/single_gumps.rs`); on the shipped client that is exactly the 2216 Classic gumps Interface.uop repeats, so a lossy `gumps_ec` mixes BC1, BC7 and RGBA8888 with no Classic file read. The test runs on the final pixels, so an interpolating upscale pass sends a gump to BC7. Each packer refuses the other's flag and lossy format. Paperdoll atlas pages stay lossless RGBA8888 in both, since both clients hue exact greys only, and so do the paperdoll body gumps, single gumps 12-15, 665 and 666 (`PAPERDOLL_BODY_GUMP_BY_BODY` in `lib/udd-conv/src/gumps/single_gumps.rs`, the clients' body-to-gump table); the packers log how many stayed RGBA8888. BC1 is straight colour with punch-through alpha at 0.5 (Classic gumps have no soft alpha). BC7 holds premultiplied colour, so soft alpha is kept. Readers decode either to straight RGBA8888 on the CPU, un-premultiplying BC7.

#### Shared Classic gump table

A `gumps_ec` can be packed standalone or sharing the Classic gumps with a `gumps_cc` (the user, 2026-11-06: "i'd keep the ability to both share cc gumps and pack them into ec gumps file"). `pack-ec-gumps --share-with <gumps_cc.uddp>` decodes, for each single Enhanced gump, the `gumps_cc` single gump of the same id and leaves the Enhanced gump out when the `gumps_cc` image draws the same as either the image this package would store (encoded and decoded) or the exact source, both after the same upscale passes. Drawing the same means the same width, height and upscale factor, the same alpha everywhere, and the same R, G and B wherever alpha is not 0; the paperdoll body gumps (12-15, 665, 666) must match byte for byte (`gump_images_draw_the_same` in `lib/udd-conv/src/gumps/single_gumps.rs`), and the check runs on the baked body gump this package would store. On the shipped clients that leaves out all 2216 Interface.uop repeats of Classic gumps: body gumps 12-15, whose colour under clear texels differs in the source, match once baked.

The left-out gumps are listed in one metadata entry, sparse id `0xE000_0002` (`SHARED_CLASSIC_GUMP_TABLE_ID`), which only a sharing `gumps_ec` carries:

| Offset | Type | Field | Notes |
|---|---|---|---|
| 0x00 | `[u8; 4]` | magic | `GSHC` |
| 0x04 | `u32` | `version` | 1 (`SHARED_CLASSIC_GUMP_TABLE_VERSION`); another version is refused with the message to repack. |
| 0x08 | `u32` | `count` | |
| 0x0C | `count` x 12 bytes | records | `u32 gump_id`, `u64 image_hash`, sorted by id with no repeats. |

`image_hash` is xxh64 with seed 0 (`gump_image_hash` in `lib/udd-assets/src/gumps.rs`) over the `gumps_cc` image the gump was checked against, decoded to straight RGBA8888: `u32 physical_width`, `u32 physical_height`, `u16 upscale_factor`, little endian, then every RGBA byte, colour under clear texels included. The reader reads a listed id as absent, and refuses a package that both stores a gump (single or atlas) and lists it, or whose table is out of order or doesn't match its count. Dynamapper draws the listed gumps from the Classic source under the Enhanced and KR-like presets and checks each one's hash the first time it serves it; `udd-cli tool validate` checks every hash against the `gumps_cc.uddp` beside the package.

### 2.4 Texture Compression Strategies (BC1 vs BC7 and Supercompression)

The `tex_land_ec.uddp` pipeline commonly utilizes **BC7** block compression. The original EC `Texture.uop` generally contains textures encoded in **DXT1 (BC1)** (4 bits per pixel), with some relying on **DXT5 (BC3)** (8 bits per pixel) for alpha transparency.

Storing the converted textures as BC7 within the UDDP package creates a file that is typically larger than the raw source UOP file. This quality/space trade-off is made for three critical reasons:

1. **Alpha Transparency**: DXT1 only supports 1-bit alpha. Standardizing on BC1 would destroy the smooth translucency of textures that originally used DXT5 (like water or foliage). BC7 supports high-quality, full 8-bit alpha.
2. **Upscaling Fidelity**: When using FSR or xBRZ upscaling options, the smooth details generated by the upscaler are far better preserved by BC7 than by re-compressing them back into the highly artifacted DXT1 format.
3. **Texture Array Uniformity**: Modern GPU renderers use `Texture2DArray` to pack multiple pages into a single draw-call pipeline. All layers in a texture array must share the exact same format. BC7 provides the best "highest common denominator" for mixing opaque and translucent terrain tiles.

To mitigate the inherent size increase of the 8bpp BC7 format, UODynamapper uses a **Supercompression** pass (applying `ZstdNoDict` compression on top of the BC7 payload). Even though BC7 is a fixed-rate GPU format, Zstd is highly effective at compressing the repeating, identical BC7 16-byte blocks that are generated for transparent or flat-color "empty space" regions within the atlas pages.

Packages built with `--bc7` before 2026-11-05 keep blocks from an older encoder until rebuilt: see [`bc7_alpha_plane_search.md`](../agent_notes/bc7_alpha_plane_search.md).

The lossy Enhanced land build (`pack-ec-textures --land-bc7`) is BC7 with the alpha-plane search, no RDO and straight colour, with page Zstd at level 19 (`BC7_LAND_PAGE_ZSTD_LEVEL`) unless `--zstd` names a level. The converter GUI's BC7 + Zstd takes the level of its Zstd setting, and its plain BC7 stores pages uncompressed. Pages are stored at the full atlas extent. The packer logs, per render role (base, detail, mask, normal), how many texels of the referenced textures have alpha below 255. Dynamapper uploads BC7 land pages as their blocks (`Bc7RgbaUnorm`, linear like the RGBA Enhanced pages) instead of decoding them; the format is read once from the first page record.

Classic land (`tex_land_cc.uddp`) stays lossless: `pack-texmaps` and the converter GUI refuse BC7 with the measured reason (`CLASSIC_LAND_LOSSY_REFUSAL`).

Lossless pages are RGBA8888 with plain Zstd 19 per payload (`LOSSLESS_PAGE_COMPRESSION`) unless the packer is given another level (`--zstd=LEVEL`): the land and gump paperdoll atlas pages, the `tex_art_cc` and `tex_art_ec` pages and the `tex_aux_ec` textures. Gumps and land took it on 2026-11-06, art and auxiliary textures the same day (the user: "Yes switch to zstd 19"); the measurements are in [`uddp_anim_format_requirements.md`](../agent_notes/uddp_anim_format_requirements.md). The builder compresses payloads of 10 MiB or more side by side, one zstd job each.

### 2.5 `tex_aux_ec.uddp`

Holds the non-base support images (hue masks, terrain-surface masks, liquid inputs) that runtime consumers read through tilemeta texture refs. These are images that are not atlased. Each image is keyed by its source texture id, with one image per id; the first ref seen wins.

Contents:

- **Only the WorldArt family** (`EcMaterialLogicalFamily` 1, from `Texture.uop`). Every consumer reads block-0 WorldArt refs.
- **No TileArtLegacy or TileArtEnhanced images.** Those belong to the EC's legacy-art mode, which is not converted; Classic art comes from the Classic Client files (`tex_art_cc.uddp`). The packer counts the skipped refs as "unread family".
- **The client's missing-texture image.** When a texture request returns no data, the client substitutes `Data\SystemTextures\missing.tga` (`SystemTextures.uop`, a 256x256 TGA decoded by its extension). The package stores that image as a normal slot and payload under the reserved id `TEX_AUX_EC_MISSING_TEXTURE_ID` = `u32::MAX` (`0xFFFFFFFF`).
  - No real texture can take that id: the client formats tile texture paths with a signed `%08d`, so reachable ids stop at `i32::MAX`.
  - The slot has family `SystemTextures` (6) and role `Unknown` (0).

**Virtual Files:**

- `metadata/slots.bin`: array of `TexAuxEcSlotRecord`, sorted by `texture_id` (the reserved entry sorts last). It has no version header.
- `textures/{texture_id:08x}`: raw pixels for one slot, RGBA8888 or a BC7 container according to the slot's `pixel_format`.
- Missing-source aliases: each distinct referenced id of a packed family whose image the EC archive does not contain gets a slot record with `TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE`, and no payload of its own. Its pixels are the reserved missing-texture payload, stored once.
  - The alias keeps the ref's id, family and role, and copies the missing image's `width`, `height` and `pixel_format`.
  - A source that exists but fails to decode gets no alias: the client shows Gamebryo's default texture for those, not `missing.tga`.
  - The reader rejects a slot table with aliases but no reserved missing-texture slot, or one not sorted by unique id.

**`TexAuxEcSlotRecord` (12 Bytes)**

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `texture_id` | Source texture id, or the reserved missing-texture id. |
| 0x04 | `u16` | `width` | Image width in pixels. |
| 0x06 | `u16` | `height` | Image height in pixels. |
| 0x08 | `u8` | `logical_family` | `EcMaterialLogicalFamily`: 1 = WorldArt; 6 = SystemTextures (reserved entry only). |
| 0x09 | `u8` | `stable_role` | `EcMaterialStableRole` of the first ref that named the id (from the block shader and the item position). |
| 0x0A | `u8` | `pixel_format` | 0 = RGBA8888, 1 = BC7. |
| 0x0B | `u8` | `flags` | `TEX_AUX_EC_SLOT_FLAG_*`. Bit 0 (`MISSING_SOURCE`): alias of the reserved missing-texture slot; read the payload `textures/ffffffff`. |

Tilemeta refs keep their true source ids; nothing is substituted at build time.

**Reading:**

- A normal lookup resolves a ref as the client does. `TexAuxEcPackage::slot_for_texture_id(id)` returns the packed slot or, for an absent source, its alias (`SlotInfo::is_missing_source`, `SlotInfo::payload_texture_id` = the reserved id, for caching). `load_rgba(id)` returns the alias's magenta pixels.
- `TexAuxEcPackage::missing_texture_slot()` returns the reserved slot itself, e.g. for art with no image in any package.
- `TexAuxEcPackage::load_bc7(id)` returns a BC7 slot's stored container (`Bc7RgbaUnormSrgb` blocks) without decoding, for a direct GPU upload; it refuses an RGBA slot. `TexAuxEcSlotRecord::is_bc7` tells the two apart.
- Packages built before the aliases existed have none (some carry an unread `metadata/absent_sources.bin`); absent ids then have no slot.

---

## 3. Animation Atlas Trimming

`mobile_anim_cc.uddp` and `mobile_anim_ec.uddp` always trim fully transparent frame borders before atlas packing. The frame payload keeps only the non-empty visual bounds, while the existing frame `center_x`/`center_y` fields are shifted by the removed left/top transparent border so the original animation pivot remains stable. BC7 output still uses the existing 4x4-oriented allocation path, so trimmed frame rectangles reduce atlas occupancy without breaking block alignment.

Mobile animation atlas pages store edge-extruded allocation gutters, including BC7 alignment padding, so linear previewers and block compression do not sample transparent-black padding around frame edges. This is an on-disk package invariant, not a runtime repair step.

---

## 4. `world_lights.uddp`

This package stores decoded light masks from Classic `light.mul`/`lightidx.mul` and optional EC light textures. `dynamapper` loads it when present and uses tilemeta light-source statics to place additive world light decals. Classic light masks are grayscale; colored EC light payloads keep their stored RGB at runtime unless the placed static has a nonzero hue id, in which case the runtime recolors the mask from `hues.uddp`. The `hues.uddp` package is the single runtime hue source and may be built from Classic `hues.mul` or EC `hues.uop`.

**Virtual Files:**

- `metadata/slots.bin`: Dense slot manifest indexed by light id.
- `lights/{light_id:08}.rgba8888`: Raw RGBA8888 light-mask payload for present slots.

### 4.1 `WorldLightSlotRecord` Struct (10 Bytes)

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `light_id` | Dense light id. |
| 0x04 | `u16` | `flags` | Bit 0 means the light payload is present. |
| 0x06 | `u16` | `width` | Light mask width in pixels. |
| 0x08 | `u16` | `height` | Light mask height in pixels. |

---

## 5. `paperdoll_ec.uddp`

The frames of the Enhanced Client's 3D paperdoll: the male (`build/paperdoll/000001/00.bin`) and female (`01.bin`) AMOU sets of `Paperdoll.uop`, decoded losslessly, with the hue tables of `Hues.uop` and what each dress-layer item takes off in Dynamapper's paperdolls, measured from the frames, `tileart.uop` and `string_dictionary.uop` (the names in the log). `udd-cli pack pack-ec-paperdoll --ecdir <EC>` writes it (lossless Zstd 19 by default; `--zstd=LEVEL` picks another level; `--report <file>` writes every finding of the dress-layer measurement, one line each). Reader: `udd_assets::PaperdollEcPackage`, which implements `uocf::enhanced::paperdoll::PaperdollFrames`, so the shared paperdoll lookup (`plan`) runs on it as on the client files, and `decode(frame, hue)` gives the pixels `FrameSets::decode` gives from `Paperdoll.uop` and `Hues.uop` (tested on every frame of the 2024 client under table, partial and shadow hues).

Only frames the paperdoll can find are stored: those whose action is their set's index (`UOAnimations::FindFrameEntry`), the first of a repeated number. EC 2024: 578 male and 577 female frames, 432 with a hue mask, 3,000 hue tables; 110 MiB raw, 8.2 MiB stored (the sources are 6.3 MB and 2.6 MB).

**Virtual Files:**

All three manifests carry version 3 (2026-11-07, which decides each garment beneath a dress-layer item in `metadata/dress_decisions.v3.bin`; version 2 stored a mask of slots in `metadata/dress_hides.bin`); a package of another version is refused with a message to repack it.

- `metadata/frames.bin`: magic `PDFR`, `u32` version, `u32` set count (2); per set the header box of its AMOU file (`i16` x0, y0, x1, y1) and the shadow statistics of its palette (`u8` minimum, mean and maximum inverse luminance, `u8` reserved 0), which `SHADOW_HUE` normalizes against; `u32` frame count; then one 12-byte record per frame, ascending by (set, number):

  | Offset | Type | Name | Description |
  |--------|------|------|-------------|
  | 0x00 | `u8` | `set` | 0 male, 1 female. |
  | 0x01 | `u8` | `hue_mode` | `FRAME_HUE_NONE` (0): no pixel takes a hue; `FRAME_HUE_ALL` (1): every coded pixel does; `FRAME_HUE_MASKED` (2): the `.r8` mask marks them. As in `mobile_anim_ec.uddp`. |
  | 0x02 | `u16` | `frame_number` | The number the paperdoll looks up (body, `Paperdoll` or `Appearance` value). |
  | 0x04 | `i16` | `x` | Where pixel (0, 0) goes from the paperdoll anchor: `x0` of the frame box. |
  | 0x06 | `i16` | `y` | `y0` of the frame box. |
  | 0x08 | `u16` | `width` | Frame width. |
  | 0x0A | `u16` | `height` | Frame height. |

- `frames/SS/NNNNN.rgba8888` (`DataType::Paperdoll`, 16): the straight-alpha RGBA of frame `NNNNN` of set `SS` as `AnimationFrame::decode_frame` gives it, edge pixels carrying their coverage in alpha (the lit lantern 0xA15 is 375x306 with 85,894 partly transparent texels).
- `frames/SS/NNNNN.r8` (`DataType::Paperdoll`): the hue mask of a `FRAME_HUE_MASKED` frame, one byte per pixel, `HUE_MASK_HUEABLE` where the palette colour takes the hue, else 0.
- `metadata/hue_tables.bin`: magic `PDHT`, `u32` version, `u32` count, then per table a `u16` bitmap number and its 256 RGBA entries (`EcHueTable`, the top row of `hue%04d.bmp`). The `hues.uddp` package can be built from Classic `hues.mul`, which has no such tables, so the paperdoll carries its own.
- `metadata/dress_decisions.v3.bin`: magic `PDDD`, `u32` version, `u32` slots (bit n set for layer n of `uocf::enhanced::paperdoll::SLOT_NAMES`, no bit past 25: the layers the decisions take garments off, torso, legs, skirt, arms, chest and abovechest), `u32` count, then per decision, ascending by (item id, body): `u32` item id, `u16` body (the number of the body frame the paperdoll draws: 400, 401, 605, 606, 666, 667), `u16` kept count, then per kept garment, ascending by id, `u32` item id and `u32` texels beyond the edge. Not client data: what the packer measured each dress-layer item to take off on each player body (`udd_conv::paperdoll_ec_dress`). The client draws the shirt, trousers, skirt, sleeves and chest pieces beneath a robe, and their edges show past it; for each garment of those slots the client draws beneath the item, the packer draws the body wearing it and the item, with and without the garment, and counts the texels that vanish where the item is not opaque, beyond 16 texels of the item. A garment is kept when 1,600 or more vanish there: it is in plain view. Dynamapper's paperdolls take off every garment of the slots that the client draws beneath a drawn item with a decision, except the garments it keeps, then take the draw order again; without a decision, nothing. `PaperdollOverrides.kdl` overrides a decision. EC 2024: 72 dress-layer items, 372 decisions (60 item and body pairs have no frame), 113 keeping none of the garments drawn beneath the item and 259 some; of the 61,120 garments measured, 9,770 are kept and 51,350 taken off. The slot rule of version 2 (the largest garment decided its slot) took off 458 garments in plain view and kept 2,287 whose edges alone show; deciding each garment draws all of them as measured.
