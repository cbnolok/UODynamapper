# Code Overview - UODynamapper

This document describes the high-level **code flow** and system interactions in UODynamapper. For authoritative technical specifications, data formats, and shared constants, see **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)**.

---

## 1. Core Application Flow

### 1.1 Startup Sequence
1. **Config Loading**: `main.rs` calls `settings::load_from_files()` to populate global configuration.
2. **Bevy Setup**: `core.rs` registers core plugins and applies window/render settings.
3. **Asset Discovery**: `UOFilesPlugin` scans for `.mul` and `.uop` files.
4. **State Transition**: The app starts in `StartupSetup`, executes scene initialization, and transitions to `InGame` via `app_states.rs`.

### 1.2 Schedule Hierarchy (`system_sets.rs`)
- **PreStartup**: Configuration and base mesh generation (`setup_base_mesh.rs`).
- **Startup**: Scene setup (camera, light, terrain orchestration).
- **Update**:
  - `MovementSysSet`: Input handling and camera updates.
  - `SceneRenderLandSysSet`: Detection of camera/zoom changes and terrain chunk lifecycle.
  - `SceneRenderArtSysSet`: Static art collection and draw orchestration.

### 1.3 Package And Conversion Entry Points
- `tools/udd-conv-cli`: build and inspection entry point for converted packages such as `tilemeta.uddp`, `tex_art_ec.uddp`, and `tex_land_ec.uddp`
- `udd-conv`: conversion logic for EC and classic packaging
- `udd-assets`: runtime package readers used by `dynamapper`
- `uocf`: low-level classic and EC source parsing

---

## 2. Terrain Pipeline Flow

### 2.1 Chunk Lifecycle
1. **Detection**: `ListenSyncRequests` monitors camera movement.
2. **Visibility**: `compute_visible_chunks` uses ray-projection to find the XZ footprint on the ground plane.
3. **Orchestration**: `SyncLandChunks` manages the spawn/despawn queue, throttled to prevent frame spikes.
4. **Data Fetch**: `create_land_chunk_material` fetches CC map blocks from `uocf` and resolves EC texture coordinates.
5. **Upload**: Chunk metadata is written to the global Paged Tile Atlas via `queue.write_texture`.

Current runtime note:
- runtime map chunking is intentionally kept at `32x32` for now even though package transport work can use larger units such as `64x64`.

### 2.2 Texture Management
- **Resolution**: `mesh_material.rs` handles the resolution chain (CC ID → EC Material → UDDP Slot Index).
- **Caching**: `LandTextureCachePlugin` manages the LRU residency of land textures in GPU arrays.
- **Shader Group 3**: All chunks share a single `LandMaterialExtension` containing global texture arrays and paging parameters.

### 2.3 EC Ownership And Packaging Flow
- `uocf/src/enhanced/tileart.rs` is the ownership seam for tileart-owned EC statics and texture refs.
- `uocf/src/enhanced/terrain_definition.rs` is the ownership seam for terrain-definition materials, layers, and aliases.
- `udd-conv/src/tilemeta.rs` writes tilemeta sidecars used later by runtime routing.
- `udd-assets/src/tilemeta.rs` exposes helpers such as main EC texture resolution from preserved item texture refs.
- Current package split:
  - `tex_art_ec.uddp` for tileart-owned visible bases
  - `tex_land_ec.uddp` for terrain-definition-owned visible bases
  - `tilemeta.uddp` for runtime metadata and sidecars

---

## 3. Static Art Pipeline Flow

### 3.1 Collection
1. **Statics Fetch**: `statics_collect.rs` queries `uocf` for static objects belonging to active map blocks.
2. **Multi Expansion**: optional `multi.mul`/`multi.idx` or `MultiCollection.uop` definitions expand multi placement statics into normal static-art parts before routing.
3. **Metadata Lookup**: collection can consult `tilemeta` and EC package state to distinguish ordinary art from surface-like EC statics.
4. **Filtering**: objects are filtered by visibility, routing, and depth-ordering requirements.
5. **Batching**: statics are grouped by material and atlas residency to minimize draw calls.

Performance notes for this path live in **[rendering/STATIC_ART_PERFORMANCE.md](rendering/STATIC_ART_PERFORMANCE.md)**. The short version is: steady frames must reuse cached chunk output, optional data paths must be gated, and per-frame systems must not parse UDDP metadata or decompress package entries.

### 3.2 Drawing
1. **Instance Data**: `statics_draw.rs` prepares per-instance data (position, UVs, color) for GPU submission.
2. **Alpha Handling**: Shaders use alpha-masking for clean overlap of isometric sprites.
3. **Depth Status**: current runtime still mixes normal sprite depth with staged logical-depth work; the full explicit UO-style depth-key port is still in progress.

### 3.3 Surface-Like EC Static Routing
- Reference implementation seam: `dynamapper/src/core/render/scene/world/art/statics_collect.rs` plus related inspection helpers.
- Routing rule: prioritize `tilemeta.is_surface_like()` plus a resolvable `ec_land` slot over direct `ec_art.present_slot(tile_id)`.
- Safe land resolution order for surface-like statics is:
  - CC-based `tex_land_ec.resolve_runtime_slot_id(cc_texture_id)`
  - direct populated `tex_land_ec.present_slot(main_ec_texture_id)`
  - provenance fallback by `selected_texture_id`
- If all land-style resolutions fail, the runtime should warn with sample tile ids instead of silently skipping the case.

---

## 4. Asset Streaming Pipeline

### 4.1 UDDP Streaming
- **I/O**: `UddpReader` uses `memmap2` for zero-copy access to converted packages.
- **Decompression**: Zstd/BC7 decoding is offloaded to Bevy task pools to avoid blocking the main thread.
- **Parallelism**: `rayon` is used during initial index building and bulk data processing.

Current transport note:
- terrain packaging work prefers `64x64` transport units, but the live renderer keeps its own `32x32` chunking decisions for now.

### 4.2 Texture Residency
- **Lazy Loading**: `TexMap2D` handles on-demand loading of individual texture entries.
- **Expansion**: GPU texture arrays grow dynamically as new unique IDs are encountered.
- **Eviction**: A background system monitors access timestamps and evicts stale textures from VRAM after 60 seconds of inactivity.

---

## 5. Performance Monitoring
- **Diagnostics**: `LogDiagnosticsPlugin` and custom overlays track frame times and VRAM usage.
- **Profiling**: `LandProfilingPlugin` provides granular metrics for chunk generation and atlas upload pressure.

Additional active concerns:
- upload batching must remain bounded; do not regress into per-object texture writes
- static-art depth behavior must be validated independently from visual billboard placement

---

## 6. Related Documentation
- **docs/TECHNICAL_REFERENCE.md**: Technical specs and data formats.
- **docs/PROJECT_OVERVIEW.md**: High-level summary and status.
- **docs/CONTRIBUTORS_GUIDE.md**: Quick file reference and workflows.
- **GEMINI.md**: AI agent instructions.
- **docs/TODO.md**: Planned features.

---

**Last Updated**: May 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
