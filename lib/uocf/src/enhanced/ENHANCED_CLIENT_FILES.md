# Enhanced Client Asset Reference

This document describes the relationship between the Ultima Online Enhanced Client (EC) data files and the Rust modules in `uocf/src/enhanced/`.

## 1. Raw Data Decoding (UOP Parsers)

These files are responsible for bit-for-bit decoding of raw `.uop` blocks into internal structs.

| Rust Module | Target UOP File | Content Description |
|:---|:---|:---|
| `string_dictionary.rs` | `string_dictionary.uop` | A dictionary of virtual paths (e.g., `build/tileart/00000001.dat`) indexed by an internal ID. Crucial for mapping IDs to binary blocks. |
| `tileart.rs` | `tileart.uop` | Detailed metadata for "Art Tiles" (statics). Contains flags, heights, radar colors, and pointers to textures. |
| `textures.rs` | `Texture.uop`, `LegacyTexture.uop` | The actual image payloads. Stores metadata headers followed by raw DDS (BC1/BC3) or TGA blobs. |
| `animationframe.rs` | `animationframe.uop` | Frame-by-frame metadata for animations, including coordinates and RLE encoding info. |
| `mobile_animation.rs` | `mobileanimation*.uop` | Metadata and sequences for character and creature animations. |
| `multis.rs` | `multi.uop` | Multi-part structures (houses, ships) and their components. |
| `facet.rs` | `facet*.uop` | Map data. Stores terrain IDs and height information in a paged block format. |
| `waypoints.rs` | `waypoints.uop` | Raw waypoint section decoder for `build/sectors/waypoint.bin`; field semantics are still intentionally unresolved. |

## 2. Abstractions & Custom Data

These modules do not mirror a single file format but instead provide convenience layers or aggregate data from multiple sources.

| Rust Module | Purpose |
|:---|:---|
| `tile_database.rs` | **High-level Facade**. The primary interface for the engine. It orchestrates `TerrainDefinition`, `ArtDefinition`, and `ClassicTileMapper` to provide a single point of lookup. |
| `terrain_definition.rs` | **Terrain Material Decoder**. Parses `TerrainDefinition.uop`, which carries EC land-material semantics, selected textures, aliases, and runtime/provenance relationships. |
| `classic_tile_mapper.rs` | **ID Translation**. Maps multiple classic 2D client tile IDs (which are sparse and duplicated) to their unified Enhanced Client **"Base ID"** equivalents. |
| `facet_encoder.rs` | **Processing Utility**. Contains logic for re-encoding or optimizing facet data for runtime consumption. |

## 3. Data Origin vs. Convenience

### Raw Binary Mappings (`Internal Raw File-Mapping Structs`)
Structs in this category (e.g., `TileArtEntry`, `TextureItem`, `TaeProp`) are designed to match the binary layout of the UOP files. They often contain:
- `unk` fields (unknown data preserved for alignment).
- Raw offsets and indices.
- Raw integer values instead of enums.

### Convenience Abstractions (`Public API`)
Structs in this category (e.g., `ArtData`, `TextureFile`, `TerrainProperties`) are designed for use by the rest of the application. They provide:
- Resolved strings and IDs.
- Clean enums (e.g., `TileType::Liquid` instead of `type_val = 2`).
- Simplified coordinate systems.
- Thread-safe, reference-counted access to raw bytes (`Arc<[u8]>`).

## 4. Key Relationships

- **ID Resolution**: Most UOP files store an integer "String Offset". This value (usually `offset - 1`) is used as an index into the `UoStringDictionary` to retrieve a virtual path string. This string is then hashed to find the actual binary data in another UOP package.
- **Many-to-One Mapping**: While classic IDs and EC IDs often overlap, the EC uses a **Base ID** system. Multiple legacy classic IDs (which might represent the same grass tile in different variations) are mapped to a single "Base ID" in EC that defines the shared textures and physics properties.
- **Visual Styles**: `ArtData` (from `tileart.rs`) often contains two texture definitions: one for the native "Enhanced" visual style and one for a "Classic" legacy style, allowing real-time toggling in the renderer.

## 5. Verified EC Texture Classification Model

The most important practical discovery for this repo is that EC texture ownership and EC render mode are separate questions.

Ownership:

- `tileart.uop` owns item/static semantics.
- `TerrainDefinition.uop` owns terrain/material semantics.
- `Texture.uop` and `LegacyTexture.uop` are shared decode pools, not ownership boundaries.

Consequences:

- a low numeric texture id does not prove that an entry is land-only
- a `build/worldart/...` path does not prove that an entry belongs to terrain
- the same raw texture id may legitimately be referenced by both art and land outputs when the semantic owners differ

## 6. `tileart.rs` Parser Rules

`tileart.rs` converts raw `TileArtEntry` records into `ArtData`.
The current `TileType` mapping is intentionally render-oriented:

- `UOWaterShader` -> `TileType::Liquid`
- `UOStaticTerrainShader` -> `TileType::Solid`
- `UOSpriteShader` with `TaeFlag::Unused1` -> `TileType::Solid`
- `UOSpriteShader` with primary texture `texture_stretch != 1.0` -> `TileType::Solid`
- otherwise -> `TileType::Static`

The `Unused1` rule is important. Real EC tileart entries such as flat wooden boards, sandstone floor tiles, and palm-frond roof tiles use `UOSpriteShader` but still need surface-like flat rendering rather than billboard/static rendering. In other words, `Unused1` is a verified flat-render hint inside `tileart.uop`, not proof of terrain ownership.

## 7. Practical Interpretation For Other Crates

For downstream crates such as `uddconv` and `dynamapper`:

- use `TerrainDefinition.uop` to decide land ownership
- use `tileart.uop` to decide static/item ownership
- use `ArtData::tile_type` plus flags to decide whether an art-owned entry should render as regular art, liquid-like art, or surface-like art

This avoids an earlier class of mistakes where floor-like EC statics were treated either as ordinary billboards or as terrain materials simply because they looked flat or shared numeric ids with classic assets.
