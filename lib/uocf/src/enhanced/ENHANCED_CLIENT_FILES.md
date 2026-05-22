# Enhanced Client Asset Reference

This document explains how UODynamapper reads Enhanced Client (EC) map/static
data, packages it into UDDP files, and applies manual KDL overrides. It is meant
for first-time contributors and coding agents: keep source evidence separate
from convenience data, and do not treat a texture id or file path as ownership
proof by itself.

## 1. Evidence Order

Use this priority order when deciding how EC terrain or statics should behave:

1. EC UOP source files are primary evidence.
2. Packed UDDP metadata is derived evidence used by runtime.
3. Active KDL files are narrow manual integration/override data.
4. Legacy KDL files are historical review material only.
5. Current Rust/WGSL behavior is implementation, not source truth.

## 2. Raw Data Decoding

These modules decode raw EC files into internal structs. Parsers should preserve
unknown fields when they affect alignment or future auditability, but higher
layers should avoid assigning meaning to unknown fields without evidence.

| Rust Module | Target UOP File | Content Description |
|:---|:---|:---|
| `string_dictionary.rs` | `string_dictionary.uop` | A dictionary of virtual paths (e.g., `build/tileart/00000001.dat`) indexed by an internal ID. Crucial for mapping IDs to binary blocks. |
| `tileart.rs` | `tileart.uop` | Detailed metadata for "Art Tiles" (statics). Contains flags, heights, radar colors, and pointers to textures. |
| `textures.rs` | `Texture.uop`, `LegacyTexture.uop` | The actual image payloads. Stores metadata headers followed by raw DDS (BC1/BC3) or TGA blobs. |
| `terrain_definition.rs` | `TerrainDefinition.uop` | EC land material definitions: aliases, shader names, layered texture refs, repetition/stretch, and selected primary texture evidence. |
| `animationframe.rs` | `animationframe.uop` | Frame-by-frame metadata for animations, including coordinates and RLE encoding info. |
| `mobile_animation.rs` | `mobileanimation*.uop` | Metadata and sequences for character and creature animations. |
| `multis.rs` | `multi.uop` | Multi-part structures (houses, ships) and their components. |
| `facet.rs` | `facet*.uop` | EC map data. Stores terrain IDs and height information in a paged block format. |
| `waypoints.rs` | `waypoints.uop` | Raw waypoint section decoder for `build/sectors/waypoint.bin`; field semantics are still intentionally unresolved. |

## 3. Abstractions And Aggregates

These modules do not mirror a single file format but instead provide convenience layers or aggregate data from multiple sources.

| Rust Module | Purpose |
|:---|:---|
| `tile_database.rs` | **High-level Facade**. The primary interface for the engine. It orchestrates `TerrainDefinition`, `ArtDefinition`, and `ClassicTileMapper` to provide a single point of lookup. |
| `terrain_definition.rs` | **Terrain Material Decoder**. Parses `TerrainDefinition.uop`, which carries EC land-material semantics, selected textures, aliases, and runtime/provenance relationships. |
| `classic_tile_mapper.rs` | **ID Translation**. Maps multiple classic 2D client tile IDs (which are sparse and duplicated) to their unified Enhanced Client **"Base ID"** equivalents. |
| `facet_encoder.rs` | **Processing Utility**. Contains logic for re-encoding or optimizing facet data for runtime consumption. |

## 4. Ownership Model

EC texture ownership and render role are separate questions.

Owners:

- `TerrainDefinition.uop` owns terrain/material semantics.
- `tileart.uop` owns item/static semantics, including surface-like flat statics.
- `facet*.uop` owns map cell terrain ids and heights.
- `Texture.uop` and `LegacyTexture.uop` are shared image pools, not ownership boundaries.

Consequences:

- A low numeric texture id does not prove land ownership.
- A `build/worldart/...` path does not prove terrain ownership.
- The same raw texture id may appear in both art and land outputs when different semantic owners need it.
- `TerrainTexture.uop` / `EffectTexture.uop` style package names should not be collapsed into terrain/particle policy without direct owner evidence.

## 5. Map, Terrain, And Static Workflow

Terrain:

- Read terrain ids and heights from map/facet data.
- Use `TerrainDefinition.uop` aliases and material entries to map those ids to EC material ids.
- Preserve all referenced material layers: visible bases, detail textures, masks, normals, and other support textures.
- Pick a conservative primary visible texture for baseline rendering, but keep the full layer graph for blending and future shaders.

Statics:

- Read item/static metadata from `tileart.uop`.
- Use tileart shader names, flags, clip windows, and texture refs to decide how a static should render.
- Surface-like statics remain tileart-owned even when they look like terrain. They should not be moved into terrain ownership just because they are flat.

Problematic tiles:

- Prefer source-derived mappings first.
- If the source-derived primary texture is wrong, record why and fix the narrow case in `EcTerrainOverrides.kdl`.
- If a classic id needs routing to an EC material id, use `TerrainTranscode.kdl` until that mapping can be derived or replaced by a better source.
- Do not use opaque fields such as `unk6` as policy unless an audit proves they are reliable.

## 6. UDDP Texture Atlases And Metadata

`tex_art_ec.uddp`:

- Contains tileart-owned EC static/art images.
- Keyed by art id / tileart semantics.
- Uses atlas pages plus sparse slot metadata for page, rectangle, and dimensions.

`tex_land_ec.uddp`:

- Contains `TerrainDefinition.uop`-owned terrain material textures.
- Packs all required material layers when possible, not only the current primary texture.
- Stores `metadata/terrain_provenance.bin`, which links:
  - material id
  - alias/classic terrain id
  - selected texture id
  - canonical packed slot id
  - selected layer index
  - authored texture repetition/stretch
  - primary texture decision and reason flags
- The renderer uses this provenance to avoid selecting masks/normals as visible bases and to blend base/detail/mask layers with the correct stretch.

`tilemeta.uddp`:

- Stores dense land/item metadata used by runtime routing.
- Merges classic tiledata with EC tileart-derived sidecars where needed.

`tex_art_cc.uddp`:

- Contains Classic Client art/land atlas data for classic rendering paths.

## 7. KDL Files

Active KDL files live in `dynamapper/assets/cc_ec_convtables/`.

`EcTerrainOverrides.kdl`:

- Narrow manual integration table for terrain facts not currently derivable from UOP data.
- Use it for reviewed policy/liquid facts, explicit layer texture substitutions, seasonal/material reroutes, and ignore/transparent decisions.
- It may be embedded into `tex_land_ec.uddp` as packaged default/provenance, and may also be loaded loose at runtime to override the embedded copy.
- Loose overrides are useful for fast experiments: light/dark mountains, snowy map variants, or one-off problematic tiles. If a loose override references a texture that was not packed, the runtime should warn and fall back.

`TerrainTranscode.kdl`:

- Active loose fallback table mapping CC land ids to EC material ids.
- It is legacy-shaped but still useful for ids not fully covered by direct `TerrainDefinition.uop` aliases.
- Do not add visual policy here; keep it to id routing.

Legacy KDL files:

- Stored under `dynamapper/assets/cc_ec_convtables/legacy/`.
- Kept for review history and migration reference.
- Do not load them in runtime paths.

When to modify KDL:

- Modify active KDL only after source-derived data has been inspected and the missing/wrong fact is clear.
- Prefer reason-code values over free-form prose in machine-readable fields.
- Keep comments short and evidence-oriented.

## 8. `tileart.rs` Parser Rules

`tileart.rs` converts raw `TileArtEntry` records into `ArtData`.
The current `TileType` mapping is intentionally render-oriented:

- `UOWaterShader` -> `TileType::Liquid`
- `UOStaticTerrainShader` -> `TileType::Solid`
- `UOSpriteShader` with `TaeFlag::Unused1` -> `TileType::Solid`
- `UOSpriteShader` with primary texture `texture_stretch != 1.0` -> `TileType::Solid`
- otherwise -> `TileType::Static`

The `Unused1` rule is important. Real EC tileart entries such as flat wooden boards, sandstone floor tiles, and palm-frond roof tiles use `UOSpriteShader` but still need surface-like flat rendering rather than billboard/static rendering. In other words, `Unused1` is a verified flat-render hint inside `tileart.uop`, not proof of terrain ownership.

## 9. Practical Rules For Other Crates

For downstream crates such as `uddconv` and `dynamapper`:

- use `TerrainDefinition.uop` to decide land ownership
- use `tileart.uop` to decide static/item ownership
- use `ArtData::tile_type` plus flags to decide whether an art-owned entry should render as regular art, liquid-like art, or surface-like art
- keep exploration/audit output out of normal pack paths unless the data becomes a stable package contract
- keep loose KDL overrides separate from embedded package defaults so contributors can iterate without repacking

This avoids an earlier class of mistakes where floor-like EC statics were treated either as ordinary billboards or as terrain materials simply because they looked flat or shared numeric ids with classic assets.
