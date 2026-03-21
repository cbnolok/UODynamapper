# Code Overview - UODynamapper

This document provides detailed information about the codebase architecture, design choices, and implementation details. Read this after understanding the high-level concepts in `docs/PROJECT_OVERVIEW.md` and using `docs/CONTRIBUTORS_GUIDE.md` for file locations.

---

## 1. Application Architecture

### 1.1 Core Setup (`dynamapper/src/core.rs`)

The `core.rs` file builds and configures the Bevy `App`:

**Configuration Loading**:

- Loads `config.toml` via the `settings` module
- Controls window size, debug options (e.g., wireframe rendering)

**Bevy Plugin Configuration**:

```rust
DefaultPlugins.set(WindowPlugin { /* title, size, resizable */ })
DefaultPlugins.set(LogPlugin { /* custom log format */ })
DefaultPlugins.set(AssetPlugin { /* assets/ root */ })
DefaultPlugins.set(WgpuPlugin { /* required features */ })
```

**Plugin Registration**:

- **Third-Party**: `WireframePlugin` (debug meshes), `FramepacePlugin` (framerate limiting)
- **Custom**:
  - `ControlsPlugin` - Player input handling
  - `RenderPlugin` - Scene, camera, world rendering
  - `SettingsPlugin` - Configuration management
  - `TextureCachePlugin` - Land/item texture caching
  - `UOFilesPlugin` - UO game file loading
  - `PerformanceOverlayPlugin` - FPS/CPU/RAM metrics
  - `SystemMessagesPlugin` - In-game log overlay

### 1.2 State Machine (`dynamapper/src/core/app_states.rs`)

```rust
enum AppState {
    StartupSetup,      // Initial state, startup systems run
    AssetsLoading,     // Loading game assets
    InGame,            // Main interactive state
}
```

**Transitions**:

- `advance_state_after_init_core()`: `StartupSetup` → `AssetsLoading`
- `advance_state_after_scene_setup_stage_2()`: `AssetsLoading` → `InGame`

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
MovementSysSet::MovementActions  // Process input
    ↓
MovementSysSet::UpdateCamera     // Follow player
```

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

### 2.3 Uniform Management

**Shared Bind Group** (group 3):

```wgsl
@binding(100) var tex_small_sampler: sampler;
@binding(101) var tex_small: texture_2d_array<f32>;
@binding(102) var tex_big:   texture_2d_array<f32>;
@binding(103) var tile_meta_atlas: texture_2d_array<u32>;
@binding(104) var<uniform> ATLAS: AtlasParams;
@binding(105) var<uniform> scene:   SceneUniform;
@binding(106) var<uniform> effects: EffectsUniform;
@binding(107) var<uniform> lighting: LightingUniforms;
```

**Rust Structs** (`mesh_material.rs`):

```rust
#[uniform(104)] pub atlas_params: AtlasParams
#[uniform(105)] pub scene_uniform: SceneUniform
#[uniform(106)] pub effects_uniform: LandEffectsUniform
#[uniform(107)] pub lighting_uniform: LandLightingUniforms
```

**CRITICAL**: Rust structs must use `#[repr(C, align(16))]` and derive `ShaderType`. Layout must exactly match WGSL with proper `std140` padding.

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

### 2.5 Tile Atlas Management (`tile_atlas.rs`)

The paged tile metadata atlas is a small layered `Rg16Uint` array used only for terrain metadata, not color textures.

**Current Paging Setup**:

```rust
PAGE_TEXELS = 2048
MAX_LAYERS = 8
WORLD_PAGES_X = 16
```

**Behavior**:

- Logical world pages are mapped to physical layers through an LRU table
- Per-chunk metadata is uploaded as sub-rect updates via `queue.write_texture`
- The atlas stays small to reduce startup VRAM while still supporting large maps through paging

Unlike the metadata atlas, the land color texture arrays now support **dynamic expansion** and maintain separate small/big layer counts.

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

## 4. Logging System

### 4.1 Categories

```rust
enum LogAbout {
    RenderWorldLand,    // Terrain rendering
    Performance,        // Heavy operations
    // ... other categories
}

enum LogSev {
    Info,
    Warning,
    Error,
}
```

### 4.2 In-Game Logger (`ingame_logger.rs`)

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
| `assets/settings.toml` | UO paths, window settings, debug options, power saving |
| `assets/shader_presets.toml` | Shader uniform presets (Classic/Enhanced/KR) |
| `assets/keybindings.toml` | Keyboard shortcuts (runtime-configurable) |

### 6.2 Settings Structure

```toml
# Example settings.toml
[uo_paths]
installation_dir = "/path/to/uo"

[window]
width = 1920
height = 1080
fullscreen = false

[rendering]
lossy_texture_compression = true  # BC7
wireframe = false

[power]
reactive_low_power = true  # Reduce usage when unfocused
```

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

**Library**: `intel_tex_2`
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

**Last Updated**: mercoledì 18 marzo 2026
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
