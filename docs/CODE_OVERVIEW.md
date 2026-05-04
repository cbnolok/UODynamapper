# Code Overview - UODynamapper

This document provides detailed information about the codebase architecture, design choices, and implementation details. Read this after understanding the high-level concepts in `docs/PROJECT_OVERVIEW.md` and using `docs/CONTRIBUTORS_GUIDE.md` for file locations.

---

## 1. Application Architecture

### 1.1 Core Setup (`dynamapper/src/core.rs`)

The `core.rs` file builds and configures the Bevy `App`:

**Configuration Loading**:

- Loads settings from modular TOML files under `assets/settings/` via `settings::load_from_files()`
- Extracts window size, wireframe flag, power-saving flag, and framerate limit before building the Bevy App

**Bevy Plugin Configuration**:

```rust
DefaultPlugins
    .set(LogPlugin { /* custom fmt layer + InterceptLogLayer for uocf/bevy logs */ })
    .set(WindowPlugin { /* title, size, resizable, min 440x440 */ })
    .set(TaskPoolPlugin { /* default thread pools */ })
    .set(RenderPlugin { /* POLYGON_MODE_LINE for wireframe */ })
    .set(ImagePlugin::default_linear())
    .set(AssetPlugin { /* custom assets/ folder path */ })
```

**Plugin Registration**:

- **Third-Party (Manually Tracked)**: `DefaultPlugins` (group), `WireframePlugin`, `FramepacePlugin`, `EguiPlugin`
- **Internal (TrackedPlugin)**:
  - `ExternalDataPlugin` → sub-plugins: `SettingsPlugin`, `ShaderPresetsPlugin`
  - `ControlsPlugin`
  - `RenderPlugin` → sub-plugins: `ScenePlugin`, `OverlaysPlugin`, `DialogsPlugin`, `LandProfilingPlugin`
  - `TextureCachePlugin` → sub-plugin: `LandTextureCachePlugin`
  - `UOFilesPlugin`
  - `WireframePanicFixPlugin`
  - Diagnostics: `FrameTimeDiagnosticsPlugin`, `RenderDiagnosticsPlugin`, `LogDiagnosticsPlugin` (manual)

**Plugin Tree** (nested registration & tracking):

```text
core.rs
├── [External] DefaultPlugins
├── [External] WireframePlugin
├── [External] FramepacePlugin
├── [External] EguiPlugin
├── WireframePanicFixPlugin
├── ExternalDataPlugin
│   ├── SettingsPlugin
│   └── ShaderPresetsPlugin
├── ControlsPlugin
├── RenderPlugin
│   ├── LandProfilingPlugin
│   ├── ScenePlugin
│   │   ├── WorldPlugin
│   │   ├── PlayerDynamicLightPlugin
│   │   ├── CameraPlugin
│   │   └── PlayerPlugin
│   ├── OverlaysPlugin
│   └── DialogsPlugin
├── TextureCachePlugin
│   └── LandTextureCachePlugin
└── UOFilesPlugin
```

### 1.2 State Machine (`dynamapper/src/core/app_states.rs`)

```rust
enum AppState {
    StartupSetup,  // Initial state (default), startup systems run
    InGame,        // Main interactive state
    Stop,          // Shutdown
}
```

**Transitions**:

- `advance_state_after_init_core()` (runs in `PreStartup`): logs state change, stays in `StartupSetup`
- `advance_state_after_scene_setup_stage_2()` (runs after `SetupSceneStage2`): `StartupSetup` → `InGame`

Note: There is no `AssetsLoading` intermediate state — asset loading is handled within the `Startup` schedule system sets.

### 1.3 System Execution Order (`dynamapper/src/core/system_sets.rs`)

**Startup Schedule**:

```text
StartupSysSet::First
    ↓
StartupSysSet::LoadStartupUOFiles
    ↓
StartupSysSet::SetupSceneStage1
    ↓
StartupSysSet::SetupSceneStage2
    ↓
StartupSysSet::Done
```

**Update Schedule**:

```text
MovementSysSet::MovementActions       // Process input
    ↓
MovementSysSet::UpdateCamera          // Follow player
    ↓
SceneRenderLandSysSet::ListenSyncRequests  // Detect camera/zoom/map changes
    ↓
SceneRenderLandSysSet::SyncLandChunks      // Spawn/despawn chunks
    ↓
SceneRenderLandSysSet::RenderLandChunks    // Upload data, update materials
```

`MovementSysSet::UpdateCamera` is configured to run before `SceneRenderLandSysSet::ListenSyncRequests`, ensuring camera position is settled before visible chunk computation.

---

## 2. Terrain Rendering Pipeline

### 2.1 Chunk Management

The world is divided into base 8x8 tile chunks, but rendering now uses a **multi-scale chunk system** driven by zoom.

**Base Constants**:

```rust
TILE_NUM_PER_CHUNK_DIM = 8
TILE_NUM_PER_CHUNK_TOTAL = 64
```

**Chunk Scaling by Zoom**:

- `zoom < 10` -> scale 1 -> standard 8x8 chunks
- `10 <= zoom < 25` -> scale 2 -> 16x16 tile super-chunks
- `25 <= zoom < 50` -> scale 4 -> 32x32 tile super-chunks
- `zoom >= 50` -> scale 8 -> 64x64 tile super-chunks

This dramatically reduces entity count at high zoom-out levels while keeping the shader logic unchanged, because terrain lookup is derived from world-space tile coordinates rather than mesh-local tile IDs.

**Visible Set Computation**:

- Uses `Camera::viewport_to_world()` rather than manual orthographic math
- Samples 8 screen points (4 corners + 4 mid-edges)
- Intersects rays with the `Y=0` ground plane
- Builds an exact XZ footprint and converts it into chunk coordinates
- Adds a 2-chunk safety pad to keep displaced mountain peaks from popping at the edges

**Spawn/Despawn Strategy**:

- Recompute on meaningful camera, zoom, map, or resize changes
- Spawn queue is sorted center-out using Chebyshev distance
- Chunk creation is throttled to `512` spawns per frame to avoid burst stalls
- Scale changes force a full despawn/respawn because the grid granularity changes

**Boundary Safety**:

- Visible super-chunks are only accepted if the **entire** scaled chunk fits inside the map
- This prevents out-of-bounds map block requests near map edges
- Missing map blocks are skipped safely during atlas enqueue instead of panicking

### 2.2 Paged Tile Metadata Atlas

**Design Rationale**:
Solves two primary problems:

1. **Material Churn**: Thousands of chunks share single material/bind group, eliminating CPU/GPU stalls from uniform buffer updates
2. **Scalability**: Paged system (2048x2048 layers) allows massive maps (10,000x10,000+) within GPU texture dimension limits

**Data Format** (4 bytes per tile, `Rg16Uint`):

```text
R16: tile_id (0..65535)
G16: packed metadata
   - Low 8 bits: height_biased (signed i8 height + 128 offset)
   - High 8 bits: tex_size flag (0=small 64x64, 1=big 128x128)
```

**CPU-Side Flow**:

1. Chunk spawned → 8x8 tile region mapped to logical page
2. `TileAtlas` LRU cache assigns physical layer
3. Subregion updates enqueued via `queue.write_texture`

**GPU-Side Flow**:

1. Shader resolves world coordinates to `(layer, uv)` using `AtlasParams`
2. `textureLoad` for deterministic integer lookups of IDs/heights
3. Neighborhood sampling for bicubic normals/slopes across chunk boundaries

### 2.2.1 EC Texture Resolution Chain (CC ID ➔ EC Atlas Coordinates)

**The Problem**:
We are rendering legacy maps (`map.mul`), which only contain **CC Tile IDs** (e.g., Tile `168` is Water). The Enhanced Client (EC) textures use completely different IDs, and grouping concepts like "Materials". We need a bridge between them.

To make this efficient, the resolution process is split into two phases: **Build Time** (packing the assets) and **Runtime** (drawing the screen).

#### Phase 1: Build Time (`uddconv_cli build-ec-land`)
This phase runs once to create `ec_land.uddp`. It completely eliminates the need for the game engine to understand `.uop` files or `.tga` images.

1. **Reads `TerrainDefinition.uop`**: This file is the EC client's blueprint. It defines conceptual "Materials" (e.g., Material `5` is Water) and says exactly which texture hash to use for it. It also declares "Aliases" (e.g., "These 50 dirt materials all use the exact same dirt texture").
2. **Packs Atlases**: The builder extracts the actual images from `Textures.uop`, deduplicates them using the alias data, and packs them tightly into giant texture arrays (the UDDP "Pages").
3. **Bakes Slots**: It records the exact physical coordinates `[Page Index, X, Y, Width, Height]` of where each image landed in the atlas. This record is called a **Slot**.
4. **Bakes Provenance**: It writes a lightweight table (the `terrain_provenance` array) that maps `EC Material ID ➔ Slot ID`.

#### Phase 2: Runtime (`dynamapper`)
At runtime, the engine has no knowledge of `Textures.uop` or `TerrainDefinition.uop`. It only uses the fast, pre-baked arrays inside `ec_land.uddp`.

When the engine needs to render CC Tile `168`:
1. **KDL Translation**: It looks up `168` in `TerrainTranscode.kdl`. The KDL says: *"CC Tile 168 translates to EC Material 5"*. (The KDL contains no file names, only ID-to-ID translations).
2. **Provenance Lookup**: It asks the UDDP's provenance array: *"Which UDDP Slot Index belongs to Material 5?"*. The provenance array answers: *"Index 16426"*.
3. **Slot Lookup**: It looks up `slots[16426]` to get the physical atlas coordinates (e.g., Page 1, X:1543, Y:1).
4. **GPU Upload**: Those coordinates are uploaded to the `ec_land_lookup` GPU texture, and the shader draws the pixel perfectly.

#### The Fallback (When KDL is Missing)
If a CC Tile ID has no entry in `TerrainTranscode.kdl`, the engine skips Step 1. It directly asks the provenance array: *"Do you have any legacy alias named after this CC ID?"* 
Because the EC client imported many legacy CC textures using their original IDs, the provenance array often successfully returns a Slot ID, allowing unmapped legacy tiles to render flawlessly. If it fails, the tile safely renders blank.

#### 2.2.2 Understanding the ID Spaces
To navigate this resolution chain, it is critical to distinguish between the three ID spaces:

1.  **Classic Client ID (CC ID)**: The 16-bit ID stored in legacy `map.mul` files (e.g., `168` for water).
2.  **EC Material ID**: A canonical category ID used by the EC Client to group related textures (e.g., Material `5` is "Water").
3.  **UDDP Slot Index (Runtime)**: A direct index into the pre-computed `slots` table in the `.uddp` package. This is what the engine uses at runtime to find atlas coordinates.

**Note on the Build Phase**: During the conversion stage (`uddconv_cli`), the builder uses **EC ArtIDs** (the internal slot IDs in `Texture.uop`) to resolve provenance. However, once the UDDP is baked, these are translated into the **Slot Indices** used by the engine, completely abstracting away the EC Client's internal ID system.

---

### 2.3 Uniform Management

**Shared Bind Group** (group 3):

```wgsl
@binding(100) var tex_small_sampler: sampler;
@binding(101) var tex_small: texture_2d_array<f32>;
@binding(102) var tex_big:   texture_2d_array<f32>;
@binding(103) var tile_meta_atlas: texture_2d_array<u32>;
@binding(104) var<uniform> ATLAS:        AtlasParams;
@binding(105) var<uniform> scene:        SceneUniform;
@binding(106) var<uniform> effects:      LandEffectsUniform;
@binding(107) var<uniform> global_light:  GlobalLightingUniforms;
@binding(108) var<uniform> land_light:   LandLightingUniforms;
```

**Rust Structs** (`mesh_material.rs`):

```rust
#[uniform(104)] pub atlas_params: AtlasParams
#[uniform(105)] pub scene_uniform: SceneUniform
#[uniform(106)] pub effects_uniform: LandEffectsUniform
#[uniform(107)] pub global_lighting_uniform: GlobalLightingUniforms
#[uniform(108)] pub land_lighting_uniform: LandLightingUniforms
```

**Three-Uniform Design**:

| Binding | Struct | Scope | Contents |
|---------|--------|-------|----------|
| 106 | `LandEffectsUniform` | Land rendering | shading_mode, normal_mode, texture filtering, blur, reconstruction |
| 107 | `GlobalLightingUniforms` | All geometry (shared) | fog, tonemap, color grading, gloom, ambient, exposure, gamma |
| 108 | `LandLightingUniforms` | Land-specific | bent normals, diffuse/specular/rim/fill intensities, fill sky/ground colors |

`GlobalLightingUniforms` is designed to be shared with future art/item shaders. `LandLightingUniforms` contains parameters that require 3D surface normals (only available for land terrain).

**CRITICAL**: Rust structs must use `#[repr(C, align(16))]` and derive `ShaderType`. Layout must exactly match WGSL with proper `std140` padding. Padding field names must NOT end with a digit (naga_oil constraint).

### 2.3.1 Modular Shader Architecture

The terrain shader is split into 8 WGSL modules composed via **naga_oil** `#import` directives:

| File | Purpose |
|------|---------|
| `main.wgsl` | Vertex/fragment entry points, lighting composition, fog |
| `bindings.wgsl` | All struct definitions and `@group(3)` bind declarations |
| `atlas.wgsl` | Tile metadata atlas lookups (page → layer → UV) |
| `sampling.wgsl` | Texture sampling (small/big atlas, filtering modes) |
| `normals.wgsl` | Normal generation (geometric, bicubic, bent) |
| `shading.wgsl` | Shading models (Gouraud, per-fragment, color grading) |
| `lighting.wgsl` | Light evaluation (Lambert, rim, specular, fill, tonemap) |
| `noise.wgsl` | Noise utilities |

### 2.4 Base Mesh Setup (`setup_base_mesh.rs`)

Generates shared vertex grids for terrain chunks across both LOD and chunk scale.

**Scale-1 Mesh LODs**:

- `high`: `build_chunk_mesh(8, 1)` -> 81 vertices
- `medium`: `build_chunk_mesh(8, 2)` -> 25 vertices
- `low`: `build_chunk_mesh(8, 4)` -> 9 vertices

**Wide Meshes for Reduced Entity Count**:

- `wide16`: `build_chunk_mesh(16, 2)` -> covers 16x16 tiles with 81 vertices
- `wide32`: `build_chunk_mesh(32, 4)` -> covers 32x32 tiles with 81 vertices
- `wide64`: `build_chunk_mesh(64, 8)` -> covers 64x64 tiles with 81 vertices

**Important Detail**:

- Scale `> 1` uses fixed wide meshes rather than per-zoom mesh swaps
- LOD swapping only applies to scale `1`
- Manual AABBs are scale-aware so frustum culling remains correct despite vertex-displaced terrain heights

**Design Rationale — Two-Level Mesh Selection (Scale + LOD)**:

Mesh selection is a two-level dispatch:

1. **Chunk Scale** (primary): `scale_from_zoom` → `mesh_for_scale`. At higher zoom,
   8×8 blocks merge into super-chunks (16×16 → 256×256). Each wide mesh keeps 81
   vertices but increases the inter-vertex step. This is the *primary* mechanism for
   reducing draw-call and entity pressure.

2. **LOD within Scale=1**: `lod_from_zoom` → `mesh_for_lod`. Three detail levels
   (81 / 25 / 9 verts). `sys_update_existing_chunk_mesh_lod` live-swaps the `Mesh3d`
   handle — no geometry rebuild, just a cheap handle reassignment.

For real-time interactive rendering, LOD variation saves negligible GPU work
(at most ~7 200 vertex shader invocations across ~100 chunks). Modern GPUs
process millions of vertices per millisecond.

However, LOD becomes meaningful for **offline / export rendering**: when
rasterising the full 7 000 × 4 000 tile map at 1:1 zoom to an image file,
the renderer must produce geometry for the entire world:
- 1:1 at scale=1 → ~437 500 chunks × 81 verts = 35 M vertices.
- Dropping to Medium or Low reduces this to ~11 M or ~4 M, which can matter
  for the offline capture pipeline.

The pre-built mesh assets cost trivial VRAM (~8 meshes, ≤81 verts each ≈ a few KB),
so the complexity cost is purely code-side. Wide meshes (scale ≥ 2) don't need LOD
because at those zoom levels screen-space density is inherently low.

### 2.5 Tile Atlas Management (`tile_atlas.rs`)

The paged tile metadata atlas is a small layered `Rg16Uint` array used only for terrain metadata, not color textures.

**Current Paging Setup**:

```rust
PAGE_TEXELS      = 2048
INITIAL_LAYERS   = 4    // grows on demand via requested_expansion
MAX_LAYERS       = 32   // hard ceiling
WORLD_PAGES_X    = 16   // supports up to 32k × 32k tile maps
```

**Behavior**:

- Logical world pages are mapped to physical layers through an LRU table
- Per-chunk metadata is uploaded as sub-rect updates via `queue.write_texture`
- The atlas stays small to reduce startup VRAM while still supporting large maps through paging
- When all layers are occupied, the LRU layer is evicted *and* expansion is requested (doubling layers up to `MAX_LAYERS`)

Unlike the metadata atlas, the land color texture arrays now support **dynamic expansion** and maintain separate small/big layer counts.

**Full-Map Capacity Analysis (Britannia: 7 168 × 4 096 tiles)**:

| Resource | Requirement | Limit | Headroom |
| -------- | ----------- | ----- | -------- |
| Atlas pages | ceil(7168/2048) × ceil(4096/2048) = **4 × 2 = 8** | 32 layers | ×4 |
| Small texture layers | ≤1 869 unique tile IDs | 2 048 | ×1.1 |
| Big texture layers | ≤1 869 unique tile IDs | 2 048 | ×1.1 |
| Atlas page VRAM | 8 × 2048² × 4 B = **128 MiB** | — | — |
| Texture VRAM (BC7) | 4 + 16 = **~20 MiB** | — | — |
| Texture VRAM (RGBA8) | 32 + 128 = **~160 MiB** | — | — |

The atlas and texture arrays comfortably fit the entire Britannia map. For custom
maps exceeding 32k tiles per axis, `WORLD_PAGES_X` would need to increase and the
`page_to_layer` array (currently 256 slots packed in 64 `UVec4`s) would need
expansion.

**CPU Enqueue Budget for Full-Map Rendering**:

At scale=32 the full Britannia map is ~448 super-chunks. Each iterates
`(32+2)² = 1 156` sub-blocks × 64 cells per block = **~33.5 M** cell lookups.
The per-frame budget (`MAX_ATLAS_BLOCKS_PER_FRAME = 4 096`) processes ~3–4 chunks
per frame, requiring **~112–150 frames (~2 s at 60 fps)** to populate the
entire atlas. This progressive fill is intentional — it prevents multi-millisecond
stalls — but for offline export a "flush all" bypass may be desirable.

### 2.6 Shared Material and Shader Update Strategy

All terrain chunks share one `ExtendedMaterial<StandardMaterial, LandMaterialExtension>`.

Recent optimizations removed a major feedback loop:

- Fog animation now uses Bevy's built-in `globals.time` in WGSL
- The CPU no longer mutates terrain material time uniforms every frame
- `materials.get_mut()` is only called when atlas params, lighting, or zoom meaningfully change

This avoids triggering Bevy asset change detection for every chunk each frame, which previously caused expensive material re-extraction and high idle GPU/CPU cost.

### 2.7 Zoom and LOD Behavior

Camera zoom now uses **exponential stepping** rather than linear stepping.

Benefits:

- Smoother control over a very wide zoom range
- Better usability at both near and far zoom levels
- Cleaner thresholds for chunk scale transitions

The terrain shader also simplifies itself progressively at higher zoom:

- Reduced texture filtering detail
- Normal-generation simplifications
- Expensive visual features disabled earlier when they are no longer visible

---

## 3. UO File Parsing (uocf crate)

### 3.1 Map Parser (`uocf/src/geo/map.rs`)

**Block Format** (196 bytes per block):

```text
Header: 4 bytes (u32, unused)
Cells: 64 × 3 bytes = 192 bytes
  - Tile ID: 2 bytes (u16)
  - Height Z: 1 byte (i8)
```

**Coordinate System**: Column-major order (top-to-bottom, left-to-right)

**Sequential I/O Optimization**:
Groups non-contiguous block requests into sequential ranges to minimize filesystem seeks.

**Current Runtime Use**:

- Visible chunks request a set of base map blocks
- Neighbor and border blocks are loaded too so terrain stitching remains correct
- Super-chunks at scale 2/4/8 still fetch base 8x8 blocks internally

The current implementation is already reasonably efficient on the I/O side, but remaining CPU work is mostly in temporary allocation, caching strategy, and per-frame orchestration.

### 3.2 Texture 2D (`uocf/src/geo/land_texture_2d.rs`)

**Lazy Loading**:

- Loads land textures from `texmaps.mul`/`texidx.mul` on-demand
- Cached in `Arc<Vec<u8>>`
- 60s idle eviction

**BC7 Compression**:

```rust
// Alignment calculation for BC7 uploads
bytes_per_row = (width + 3) / 4 * 16
```

---

### 3.3 Land Texture Arrays (`dynamapper/src/core/texture_cache/land/`)

Terrain color textures are uploaded into two separate GPU array textures:

- **Small**: 64x64 tiles, initial `256` layers
- **Big**: 128x128 tiles, initial `128` layers

Both arrays can now grow dynamically up to `2048` layers.

Important implementation notes:

- Layer `0` is reserved as a permanent fallback black tile
- Allocation prefers evicting non-visible textures first
- When no safe eviction is possible, expansion is requested and rendering temporarily falls back instead of overrunning the GPU array
- The cache now tracks small and big initial layer counts separately, matching the real GPU texture sizes

**Shared Residency Abstraction**:

- Collection-agnostic residency planning lives in `dynamapper/src/core/texture_cache/residency.rs`
- The shared layer models three reusable concerns: grouping texture IDs, resolving layer budgets, and iterating IDs/layer assignments in a deterministic order
- `LandTextureCachePlugin` only supplies terrain-specific grouping (`Small` / `Big`) and texmap byte loading
- This keeps future texture families free to reuse the same residency/planning code without copying the land-specific cache logic verbatim

**Terrain Residency Flow**:

1. `build_texture_residency_plan()` scans `TexMap2D` and groups IDs by `LandTextureSize`
2. `resolve_layer_allocations()` chooses either the configured LRU sizes or the exact preload sizes for each group
3. In preload mode, `prime_full_file_residency()` first warms the underlying `TexMap2D` source cache, then assigns deterministic GPU layers starting at layer `1`
4. In LRU mode, the existing demand-driven residency path remains active, including eviction and dynamic array growth

For the shared concepts behind this flow, see `docs/TEXTURE_RESIDENCY.md`.

## 4. Logging System

### 4.1 console_logger Module (`dynamapper/src/console_logger.rs`)

The project uses a custom `console_logger` module (NOT Bevy's `debug!`/`info!` macros) for all application logging. This provides structured, filterable output with timestamps.

**Severity Levels** (`LogSev`):

```rust
enum LogSev {
    Debug,
    DebugVerbose,
    Diagnostics,
    Error,
    Info,
    Warn,
}
```

**Context Categories** (`LogAbout`):

```rust
enum LogAbout {
    AppState,
    Camera,
    General,
    Input,
    InternalAssets,
    Performance,
    Bevy,
    Player,
    Plugins,
    Renderer,
    RenderWorldArt,
    RenderWorldLand,
    Settings,
    Startup,
    SystemsGeneral,
    UoFiles,
}
```

**API**:

```rust
console_logger::one(Some(false), LogSev::Info, LogAbout::RenderWorldLand, "message");
console_logger::system("startup message");  // shorthand for system-level info
```

**Bevy Log Interception**: An `InterceptLogLayer` (tracing subscriber) captures log events from the `uocf` crate and Bevy itself, routing them through `console_logger::one()` for consistent formatting. The default Bevy fmt layer is replaced with a sink to prevent double-logging.

### 4.2 In-Game Logger (`ingame_sysmessage_logger.rs`)

**API**:

```rust
normal(msg)    // ⓘ White
warning(msg)   // ⚠ Yellow
error(msg)     // ✖ Red
custom(...)    // Custom severity/color
```

**Overlay** (`sysmessages.rs`):

- Bottom-left corner
- Max 5 messages visible
- 8s timeout + 1s fade-out
- Egui-based rendering

### 4.3 Custom Timestamps

Bevy logging configured with `HH:MM:SS` local timestamps, stripping ISO-8601 milliseconds and timezone data.

---

## 5. User Interface

### 5.1 Overlays (Modular)

Located in `dynamapper/src/core/render/overlays/`:

| File | Purpose | Position |
|------|---------|----------|
| `performance.rs` | FPS, CPU%, RAM (RSS) | Top-right |
| `player_position.rs` | Player coordinates | Top-left |
| `sysmessages.rs` | Log messages | Bottom-left |

### 5.2 Dialogs (Egui-based)

Located in `dynamapper/src/core/render/dialogs/`:

| File | Key | Purpose |
|------|-----|---------|
| `options.rs` | - | General settings |
| `terrain_shader.rs` | F3 | Shader uniform controls |
| `keybindings_help.rs` | F1 | Key reference |
| `teleport.rs` | Ctrl+G | Teleport to coordinates |

**CRITICAL**: All egui rendering systems MUST run in `EguiPrimaryContextPass` schedule (Bevy 0.18 + `bevy_egui` 0.39).

---

## 6. Configuration System

### 6.1 Config Files

| File | Purpose |
|------|---------|
| `config.toml` | Cargo build settings, linker config |
| `assets/settings/core.toml` | Window settings, debug options, power saving |
| `assets/settings/uo_files.toml` | UO installation paths and file configuration |
| `assets/settings/graphics.toml` | Rendering settings (BC7, wireframe, etc.) |
| `assets/settings/maps.toml` | Map-specific configuration |
| `assets/settings/preferences.toml` | User preferences |
| `assets/settings/keybindings.toml` | Keyboard shortcuts (runtime-configurable) |
| `assets/defaults/shader_presets.toml` | Shader uniform presets (Classic/Enhanced/KR × time of day) |

### 6.2 Settings Structure

Settings are split across modular TOML files under `assets/settings/`:

```text
assets/settings/
├── core.toml          # Window size, debug flags (wireframe), power saving
├── uo_files.toml      # UO installation paths and file configuration
├── graphics.toml      # Rendering settings (BC7 compression, texture options)
├── maps.toml          # Map-specific configuration
├── preferences.toml   # User preferences
└── keybindings.toml   # Keyboard shortcuts (runtime-configurable)
```

Shader presets are stored separately in `assets/defaults/shader_presets.toml` (12 presets: 3 modes × 4 times of day).

### 6.3 Configuration Loading Policy

**No Hidden Defaults**: All configuration values must be explicitly defined in TOML files. Never use `.unwrap_or()`, `.unwrap_or_default()`, or similar fallback patterns in Rust code.

**Rationale**:
- Hidden defaults in Rust code are difficult to discover and lead to configuration drift
- Users cannot see what options are available by inspecting the TOML files
- Makes debugging harder when behavior differs between environments

**Correct Pattern**:
```rust
// FAIL FAST - required setting
let installation_dir = settings
    .get::<String>("uo_paths.installation_dir")
    .ok_or_else(|| ConfigError::Missing("uo_paths.installation_dir".to_string()))?;

// FAIL FAST - optional setting with explicit None
let wireframe = settings
    .get::<bool>("rendering.wireframe")
    .ok_or_else(|| ConfigError::Missing("rendering.wireframe".to_string()))?;
```

**Incorrect Pattern**:
```rust
// WRONG - hidden default, easy to miss
let wireframe = settings.get("rendering.wireframe").unwrap_or(false);
```

**Implementation**:
- Use a custom deserializer that errors on missing fields
- Provide example TOML files with all fields populated
- Document all available settings in the corresponding TOML file with comments

---

## 7. Multi-Map Support

### 7.1 Map Discovery

`UOFilesPlugin` auto-discovers `map0.mul` through `map5.mul`:

```rust
// Indexing maps 0-5
for map_id in 0..=5 {
    if map_file_exists(map_id) {
        register_map_plane(map_id);
    }
}
```

### 7.2 World Geo Data

`WorldGeoData` resource stores metadata per map plane:
- Dimensions
- Chunk grid size
- Active chunks

**Entity Tagging**:
Chunk entities include `parent_map_id` component for multi-map tracking.

---

## 8. Performance Optimizations

### 8.1 Idle Eviction

**System**: `sys_evict_map_blocks`
**Interval**: Every 5 seconds
**Threshold**: 60s since last access
**Targets**:
- `MapBlock`s in `MapPlane`
- Texture pixel data in `TexMap2D`

### 8.2 BC7 Compression

**Preferred Library**: `dds`
**Optional Accelerator**: `intel_tex_2`
**Settings**: `alpha_basic_settings`
**VRAM Savings**: ~160MB → ~20MB (~8x)
**Alignment**:
```rust
// BC7: 4x4 pixel blocks, 16 bytes per block
bytes_per_row = (width + 3) / 4 * 16
```

### 8.3 Power Saving

When window unfocused:
- Switches to `ReactiveLowPower` mode (configurable)
- Reduces CPU/GPU usage significantly

### 8.4 Build Optimizations

**Release Profile** (`Cargo.toml`):
```toml
[profile.release]
opt-level = "s"     # Optimize for size
lto = true          # Link-time optimization
strip = true        # Strip debug symbols
```

**Linker**: `mold` (CI only, 3-5x faster linking)

---

## 9. Critical Implementation Details

### 9.1 Avoid `get_mut()` in Hot Paths

**Problem**:
```rust
// WRONG - triggers change detection every frame
let mat = materials.get_mut(chunk_material);
mat.uniform.value = new_value;
```

For global materials binding large texture arrays, this causes:
1. Change detection triggers
2. Re-extraction (copy to Render World)
3. Re-binding (update GPU bind groups)
4. **Result**: 70%+ GPU overhead even idle

**Solution**:
```rust
// CORRECT - read-only check
let mat = materials.get(chunk_material);
if mat.uniform.value != new_value {
    // Only then use get_mut() with explicit change tracking
}

// OR - direct GPU write for terrain metadata
queue.write_texture(
    ImageDataLayout { /* ... */ },
    data,
);
```

### 9.2 Texture Upload Alignment

**BC7** (compressed):
```rust
bytes_per_row = (width + 3) / 4 * 16  // 4x4 blocks, 16 bytes each
```

**Rg16Uint** (atlas):
```rust
bytes_per_row = width * 4  // 4 bytes per texel
```

### 9.3 Neighborhood Sampling

Shader reads neighboring texels for bicubic normals:
```wgsl
// Sample 4x4 neighborhood for bicubic interpolation
for dy in -1..3 {
    for dx in -1..3 {
        let neighbor_pos = world_pos + ivec2(dx, dy);
        let sample = textureLoad(tile_meta_atlas, neighbor_pos, layer);
        // ... accumulate for normal calculation
    }
}
```

**Key Insight**: Global/paged atlas allows seamless sampling across chunk boundaries without per-chunk padding.

---

## 10. Related Documentation

- **docs/PROJECT_OVERVIEW.md**: High-level project summary and status
- **docs/CONTRIBUTORS_GUIDE.md**: Quick file reference and workflows
- **GEMINI.md**: AI agent instructions and best practices
- **docs/TODO.md**: Planned features and improvements

---

**Last Updated**: giovedì 2 aprile 2026
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
