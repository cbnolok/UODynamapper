# UODynamapper Package (UDDP) Custom Formats

This document describes the structure of custom binary `.uddp` formats used by UODynamapper for fast parsing/ram/vram upload and fast rendering. All `.uddp` files are packaged using the core `UddpBuilder` and compressed (typically with `ZstdNoDict`), containing multiple virtual files (metadata tables and texture payloads).

The current implementation still uses a unified `flags` field in `tilemeta.uddp`, but the active design direction is to separate CC and EC metadata fields more explicitly when the schema is bumped. Treat the tables below as the current wire format, not the final semantic model.

For EC-specific content, remember that the UOP source set is mixed and semantic:

- `Texture.uop` and `LegacyTexture.uop` are shared texture pools.
- `tileart.uop` carries static-art ownership, per-entry clip windows, shader/type hints, and EC item/static metadata.
- `TerrainDefinition.uop` carries land/material ownership, aliases, selected textures, and runtime slot relationships.
- `string_dictionary.uop` is a string resource package and are not visual art sources.

---

## 1. `tilemeta.uddp`

This package merges Classic Client (CC) physical tile properties and Enhanced Client (EC) rendering definitions into a zero-copy, tightly packed binary.

The current wire format stores CC and EC texture-window fields side by side, but CC and EC behavior flags are still collapsed into one unified bitmask. Future work may split those fields so classic behavior and EC behavior can be preserved independently.
The long-term schema direction is to keep CC and EC ownership/behavior separate in the wire format instead of relying on runtime inference.

**Virtual Files:**

- `metadata/land.bin`: Dense array of `TileMetaLandTile` structs.
- `metadata/items.bin`: Dense array of `TileMetaItemTile` structs.

### 1.1 `TileMetaLandTile` Struct (48 Bytes, 8-Byte Aligned)

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

### 1.2 `TileMetaItemTile` Struct (80 Bytes, 8-Byte Aligned)

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
| 0x16 | `u16` | `_pad2` | Padding. |
| 0x18 | `[u8; 4]` | `radar_color` | RGBA minimap colors. |
| 0x1C | `[u8; 20]`| `name` | Null-terminated classic ASCII name. |
| 0x30 | `u32` | `ec_texture_id` | EC Texture slot ID. |
| 0x34 | `i16` | `ec_start_x` | X sampling-window start for EC. Shifted when paired with cropped `tex_art_ec`. |
| 0x36 | `i16` | `ec_start_y` | Y sampling-window start for EC. Shifted when paired with cropped `tex_art_ec`. |
| 0x38 | `i16` | `ec_offset_x` | X draw offset for EC. |
| 0x3A | `i16` | `ec_offset_y` | Y draw offset for EC. |
| 0x3C | `u32` | `cc_texture_id` | CC Fallback texture slot ID. |
| 0x40 | `i16` | `cc_start_x` | X bounding box start for CC. |
| 0x42 | `i16` | `cc_start_y` | Y bounding box start for CC. |
| 0x44 | `i16` | `cc_offset_x` | X draw offset for CC. |
| 0x46 | `i16` | `cc_offset_y` | Y draw offset for CC. |

When `tilemeta.uddp` is generated for cropped EC statics, only `ec_start_x` and `ec_start_y` are adjusted to match the cropped art payload. `ec_offset_x` and `ec_offset_y` remain the original EC draw offsets.

Future work is expected to keep the original source texture intact for EC art and let runtime sampling windows handle subrect selection, so cropped packing here should be understood as the current behavior, not the final target.
When the schema is eventually widened, the CC texture coordinates, EC texture coordinates, and the CC/EC flags should be split into explicit fields rather than compressed into one mixed record layout.

---

## 2. `tex_art_cc.uddp`, `tex_art_ec.uddp`, `tex_land_ec.uddp`, and related EC texture packages

These are GPU texture atlases packed into fixed-size pages to avoid texture array limits.
`tex_art_cc` packages Classic Art sprites, `tex_art_ec` packages Enhanced Client Art sprites from the shared EC texture pass, and `tex_land_ec` packages Enhanced Client Terrain Textures from the same pass.

The next architecture under discussion is a three-way split for EC textures: land, art, and auxiliary layers/masks/noise. That split does not exist yet in the current wire format, but docs should assume it as the target shape when discussing future changes.
Do not use lossy BC7 compression for art tiles.

Current EC packing semantics:

- `tex_art_ec.uddp` is keyed by tileart/static ownership and uses per-entry sampling windows.
- `tex_land_ec.uddp` is keyed by TerrainDefinition semantics and may pack sparse slot ids plus alias/provenance metadata.
- `ec_textures_layers.uddp` is the planned destination for extra shared layers, masks, and noise-like resources that are referenced semantically but do not belong to the primary land/art splits.
- If a source texture is claimed by both art and land semantics, duplication across outputs is valid and should be decided by ownership, not by avoiding repeated ids.

**Virtual Files:**

- `metadata/pages.bin`: Binary array of `PageRecord` structs representing page dimensions and occupancy.
- `metadata/slots.bin`: Binary array of `SlotRecord` structs indexed by `art_id` containing the UV mapping.
- `pages/{page_id}.rgba8888`: Raw RGBA8888 pixel payloads (compressed by Zstd via UDDP).

For the historical cropped EC art layout, cropping was performed per art entry, not per decoded source texture. The correct order was:

1. apply the tileart `start_x/start_y/end_x/end_y` clip rect for that art entry
2. alpha-trim inside that clipped rectangle

This rule matters because different art ids can reference the same source texture while sampling different sub-rectangles of it.
It also explains why `tileart.uop` must be treated as the source of entry-local EC art semantics rather than as a blunt texture-id list.

### 2.1 Metadata Structures

**Page Record (16 Bytes)**

| Offset | Type | Name | Description |
|--------|------|------|-------------|
| 0x00 | `u32` | `page_index` | The index of the atlas page. |
| 0x04 | `u32` | `tile_count` | Number of tiles packed into this page. |
| 0x08 | `u32` | `used_width` | Maximum X extent used in the page. |
| 0x0C | `u32` | `used_height`| Maximum Y extent used in the page. |

**Slot Record (20 Bytes)**

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

### 2.2 Texture Compression Strategies (BC1 vs BC7 and Supercompression)

The `tex_land_ec.uddp` pipeline commonly utilizes **BC7** block compression. The original EC `Texture.uop` generally contains textures encoded in **DXT1 (BC1)** (4 bits per pixel), with some relying on **DXT5 (BC3)** (8 bits per pixel) for alpha transparency.

Storing the converted textures as BC7 within the UDDP package creates a file that is typically larger than the raw source UOP file. This quality/space trade-off is made for three critical reasons:

1. **Alpha Transparency**: DXT1 only supports 1-bit alpha. Standardizing on BC1 would destroy the smooth translucency of textures that originally used DXT5 (like water or foliage). BC7 supports high-quality, full 8-bit alpha.
2. **Upscaling Fidelity**: When using FSR or xBRZ upscaling options, the smooth details generated by the upscaler are far better preserved by BC7 than by re-compressing them back into the highly artifacted DXT1 format.
3. **Texture Array Uniformity**: Modern GPU renderers use `Texture2DArray` to pack multiple pages into a single draw-call pipeline. All layers in a texture array must share the exact same format. BC7 provides the best "highest common denominator" for mixing opaque and translucent terrain tiles.

To mitigate the inherent size increase of the 8bpp BC7 format, UODynamapper uses a **Supercompression** pass (applying `ZstdNoDict` compression on top of the BC7 payload). Even though BC7 is a fixed-rate GPU format, Zstd is highly effective at compressing the repeating, identical BC7 16-byte blocks that are generated for transparent or flat-color "empty space" regions within the atlas pages.
