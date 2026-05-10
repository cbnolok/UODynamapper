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

---

## 2. Terrain Pipeline Flow

### 2.1 Chunk Lifecycle
1. **Detection**: `ListenSyncRequests` monitors camera movement.
2. **Visibility**: `compute_visible_chunks` uses ray-projection to find the XZ footprint on the ground plane.
3. **Orchestration**: `SyncLandChunks` manages the spawn/despawn queue, throttled to prevent frame spikes.
4. **Data Fetch**: `create_land_chunk_material` fetches CC map blocks from `uocf` and resolves EC texture coordinates.
5. **Upload**: Chunk metadata is written to the global Paged Tile Atlas via `queue.write_texture`.

### 2.2 Texture Management
- **Resolution**: `mesh_material.rs` handles the resolution chain (CC ID → EC Material → UDDP Slot Index).
- **Caching**: `LandTextureCachePlugin` manages the LRU residency of land textures in GPU arrays.
- **Shader Group 3**: All chunks share a single `LandMaterialExtension` containing global texture arrays and paging parameters.

---

## 3. Static Art Pipeline Flow (Missing previously)

### 3.1 Collection
1. **Statics Fetch**: `statics_collect.rs` queries `uocf` for static objects belonging to active map blocks.
2. **Filtering**: Objects are filtered by visibility and depth-ordering requirements.
3. **Batching**: Statics are grouped by material and atlas residency to minimize draw calls.

### 3.2 Drawing
1. **Instance Data**: `statics_draw.rs` prepares per-instance data (position, UVs, color) for GPU submission.
2. **Alpha Handling**: Shaders use alpha-masking for clean overlap of isometric sprites.
3. **Z-Sorting**: Handled via a combination of Painter's Algorithm (logical order) and standard Z-buffering.

---

## 4. Asset Streaming Pipeline

### 4.1 UDDP Streaming
- **I/O**: `UddpReader` uses `memmap2` for zero-copy access to converted packages.
- **Decompression**: Zstd/BC7 decoding is offloaded to Bevy task pools to avoid blocking the main thread.
- **Parallelism**: `rayon` is used during initial index building and bulk data processing.

### 4.2 Texture Residency
- **Lazy Loading**: `TexMap2D` handles on-demand loading of individual texture entries.
- **Expansion**: GPU texture arrays grow dynamically as new unique IDs are encountered.
- **Eviction**: A background system monitors access timestamps and evicts stale textures from VRAM after 60 seconds of inactivity.

---

## 5. Performance Monitoring
- **Diagnostics**: `LogDiagnosticsPlugin` and custom overlays track frame times and VRAM usage.
- **Profiling**: `LandProfilingPlugin` provides granular metrics for chunk generation and atlas upload pressure.

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
