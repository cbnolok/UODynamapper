# UODynamapper - Session Overview

Quick reference document for AI agents to understand the project architecture and key concepts at the start of a coding session.

---

## 1. Project Summary

**Goal**: 2D client/dynamic map renderer for Ultima Online with configurable renderer emulating classic and modern (Kingdom Reborn) visuals.

**Tech Stack**: Rust, Bevy Engine v0.18.1, WGSL (WESL + naga-oil) shaders, wgpu.

**Workspace Members**:
- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `uocf/` - Ultima Online file parser (map.mul, art.mul, tiledata.mul)

---

## 2. Entry Points & Core Architecture

### Application Flow
```
main.rs → core.rs (Bevy app setup) → AppState machine
```

### Key Files
| File | Purpose |
|------|---------|
| `dynamapper/src/main.rs` | Application entry point |
| `dynamapper/src/core.rs` | Bevy app configuration, plugin registration |
| `dynamapper/src/core/app_states.rs` | State machine (StartupSetup → AssetsLoading → InGame) |
| `dynamapper/src/core/system_sets.rs` | System execution order (Startup, Update schedules) |

### Plugin Architecture
Core plugins registered in `core.rs`:
- **ControlsPlugin** - Player input handling
- **RenderPlugin** - Scene, camera, world rendering
- **UOFilesPlugin** - Load Ultima Online game files
- **TextureCachePlugin** - Cache land/item textures
- **SettingsPlugin** - Configuration management
- **PerformanceOverlayPlugin** - FPS/CPU/RAM metrics
- **SystemMessagesPlugin** - In-game log overlay

---

## 3. Terrain Rendering Pipeline

### 3.1 Paged Tile Metadata Atlas (CRITICAL)
Instead of per-chunk uniforms, terrain metadata uses a **layered Rg16Uint texture array**:

**Data Format** (4 bytes per tile):
- **R16**: `tile_id` (0..65535)
- **G16**: Packed `[height_biased:low 8 | tex_size:high 8]`
  - `height_biased = height + 128` (signed i8 → u8)
  - `tex_size`: 0=small 64x64 atlas, 1=big 128x128 atlas

**LRU Paging**:
- World pages (2048x2048 tiles) → physical GPU layers
- Dynamic mapping via `TileAtlas` resource
- 60s idle eviction (checked every 5s)

**Key Files**:
- `dynamapper/src/core/render/scene/world/land/tile_atlas.rs` - LRU cache, upload queue
- `dynamapper/src/core/render/scene/world/land/draw_mesh.rs` - Chunk mesh creation, atlas uploads
- `dynamapper/src/core/render/scene/world/land/mesh_material.rs` - Rust uniform structs
- `assets/shaders/worldmap/land_base.wgsl` - WGSL shader

### 3.2 Shader Uniforms (std140 aligned)
**CRITICAL**: Rust structs in `mesh_material.rs` must **exactly** match WGSL structs in `land_base.wgsl`.

**Uniform Bindings** (binding indices must match!):
```rust
#[uniform(104)] pub atlas_params: AtlasParams
#[uniform(105)] pub scene_uniform: SceneUniform
#[uniform(106)] pub effects_uniform: LandEffectsUniform
#[uniform(107)] pub lighting_uniform: LandLightingUniforms
```

**Shader Binding Groups**:
```wgsl
@group(3) @binding(100) var tex_small_sampler: sampler;
@group(3) @binding(101) var tex_small: texture_2d_array<f32>;
@group(3) @binding(102) var tex_big:   texture_2d_array<f32>;
@group(3) @binding(103) var tile_meta_atlas: texture_2d_array<u32>;
@group(3) @binding(104) var<uniform> ATLAS: AtlasParams;
@group(3) @binding(105) var<uniform> scene:   SceneUniform;
@group(3) @binding(106) var<uniform> effects: EffectsUniform;
@group(3) @binding(107) var<uniform> lighting: LightingUniforms;
```

### 3.3 Rendering Presets
Three shader modes controlled by uniforms:

| Mode | Value | Normals | Shading | Features |
|------|-------|---------|---------|----------|
| **Classic 2D** | 0 | Geometric | Gouraud (vertex) | Lambert only |
| **Enhanced Classic** | 1 | Bicubic | Per-fragment | Diffuse + subtle fill |
| **KR-like** | 2 | Bicubic + Bent | Per-fragment | Full stack: rim/spec/fill + fog + grading + tonemap |

---

## 4. uocf Crate (UO File Parser)

### Structure
```
uocf/src/
├── geo/
│   ├── map.rs           # map.mul parser (MapBlock, MapCell)
│   └── land_texture_2d.rs # TexMap2D (lazy texture loading)
├── tiledata.rs          # tiledata.mul (tile properties)
├── generic_def.rs       # Generic definitions
├── generic_index.rs     # Index file parsing
└── lib.rs
```

### Map Data Format
`map.mul` blocks (196 bytes each):
- Header: 4 bytes (u32, unused)
- 64 cells × 3 bytes = 192 bytes
  - Tile ID: 2 bytes (u16)
  - Height Z: 1 byte (i8)

**Coordinate System**: Column-major order (top-to-bottom, left-to-right)

### Lazy Texture Loading
- `TexMap2D` loads art.mul textures on-demand
- Cached in `Arc<Vec<u8>>` with 60s idle eviction
- BC7 compression available (via `intel_tex_2`) for VRAM reduction (~8x savings)

---

## 5. Performance Optimizations

### 5.1 Avoid `get_mut()` in Hot Paths
**CRITICAL**: Never call `get_mut()` on Materials/Assets inside Update systems unless data **actually changed**.

**Why**: Triggers Bevy's change detection → expensive re-extraction + re-binding of entire material every frame → 70%+ GPU usage even idle.

**Solution**: Use `get()` for read-only checks. For terrain metadata updates, use **direct GPU write** via `write_texture` to the Tile Atlas.

### 5.2 Idle Eviction (60s)
- MapBlocks in `MapPlane`
- Land texture pixel data in `TexMap2D`
- Checked every 5 seconds by `sys_evict_map_blocks`

### 5.3 BC7 Compression
- Reduces texture array VRAM: ~160MB → ~20MB
- Uses `intel_tex_2` with `alpha_basic_settings`
- Alignment: `bytes_per_row = (width + 3) / 4 * 16` (4x4 blocks)

### 5.4 Build Optimizations
- **Linker**: `mold` (3-5x faster linking)
- **Release profile**: `opt-level = "s"` + LTO + strip
- **Debug profile**: `opt-level = 1`, incremental enabled

---

## 6. Uniform Update Workflow

### Adding a New Uniform Parameter
**Order matters** (follow exactly):

1. **Step 1**: Add field to Rust struct in `mesh_material.rs`
   - Ensure `#[repr(C, align(16))]` and `ShaderType` derive
   - Add padding fields as needed for std140 alignment

2. **Step 2**: Populate uniform in `draw_mesh.rs` (`create_land_chunk_material`)

3. **Step 3**: Add corresponding field to WGSL struct in `land_base.wgsl`

4. **Step 4**: Use uniform in shader logic

### Updating Uniform Values at Runtime
Two patterns:

**A. Shared Material Updates** (for global data like time, atlas params):
```rust
// In sys_update_shared_land_material
if atlas_changed || time_changed {
    if let Some(mat) = materials.get_mut(&shared_mat.0) {
        mat.extension.scene_uniform.time_seconds = current_time;
        // ... update other fields
    }
}
```

**B. UI-Driven Updates** (for shader presets):
- Monitor `UniformState` resource `dirty` flag
- Push to GPU materials only when dirty
- See `TerrainUiPlugin::push_uniforms_if_dirty`

---

## 7. Common Pitfalls & Debugging

### Binding Index Mismatch
**Error**: `wgpu Panic: Binding is missing from the pipeline layout`

**Cause**: `#[uniform(10X)]` in Rust ≠ `@binding(10X)` in WGSL

**Fix**: Verify binding indices are sequential and identical in both files.

### Washed Out / Grayish Colors
**Cause**: Color space issue (double gamma correction)

**Fix**: Do NOT add manual `pow(color, 1.0/2.2)`. Bevy expects linear output and handles gamma.

### Shader Compile Errors
- WGSL is strict (no implicit type conversions)
- Error messages point to exact line in WGSL
- Check std140 alignment in uniform structs

### Texture Upload Alignment
- BC7: `bytes_per_row = (width + 3) / 4 * 16`
- Rg16u: `bytes_per_row = width * 4` (4 bytes per texel)

---

## 8. Configuration & Assets

### Config Files
| File | Purpose |
|------|---------|
| `config.toml` | Cargo build settings |
| `assets/settings.toml` | UO file paths, window settings, debug options |
| `assets/shader_presets.toml` | Shader uniform presets (Classic/Enhanced/KR) |
| `assets/keybindings.toml` | Keyboard shortcuts |

### Asset Structure
```
assets/
├── shaders/worldmap/land_base.wgsl
├── settings.toml
├── shader_presets.toml
├── keybindings.toml
└── (UO files: mapX.mul, art.mul, tiledata.mul - external)
```

---

## 9. UI Overlays & Dialogs

### Overlays (Modular)
- `performance.rs` - FPS, CPU%, RAM (RSS) in top-right
- `player_position.rs` - Player coordinates in top-left
- `system_messages.rs` - Log messages in bottom-left (5 max, 1s fade-out)

### Dialogs (Egui-based)
- `options.rs` - General settings
- `terrain_shader.rs` - Shader uniform controls (F3)
- `keybindings_help.rs` - Key reference (F1)
- `teleport.rs` - Teleport to coordinates (Ctrl+G)

---

## 10. Logging System

### Categories
- `LogAbout::RenderWorldLand` - Terrain rendering
- `LogAbout::Performance` - Heavy operations (atlas uploads, eviction, BC7)
- `LogSev::Info` | `Warning` | `Error`

### In-Game Logger
- Backend: `ingame_logger.rs`
- API: `normal()`, `warning()`, `error()`, `custom()`
- Symbols: ⓘ (info), ⚠ (warning), ✖ (error)
- Colors: White, Yellow, Red
- Overlay: Egui bottom-left, 8s timeout + 1s fade

---

## 11. Multi-Map Support

- Auto-discovers `map0.mul` through `map5.mul`
- Each map plane indexed by ID (0-5)
- `WorldGeoData` resource stores metadata per map
- Chunk entities tagged with `parent_map_id`

---

## 12. Key Constants

```rust
// Chunk dimensions
TILE_NUM_PER_CHUNK_DIM = 8       // 8x8 tiles per chunk
TILE_NUM_PER_CHUNK_TOTAL = 64

// Grid sizes (for shader data)
DATA_GRID_BORDER = 2
DATA_GRID_SIDE = 13              // 2 + 8 + 2 + 1 (bicubic + bent normals)
MESH_GRID_SIDE = 9               // 8 + 1 (vertex grid)

// Atlas paging
PAGE_TEXELS = 2048               // World page size
MAX_LAYERS = 64                  // GPU texture array layers
```

---

## 13. Development Workflow

### Hot-Reload Workflow
1. Shader changes → automatic hot-reload (Bevy file watcher)
2. Uniform tweaks → use Terrain Shader UI (F3) for runtime testing
3. Preset changes → edit `shader_presets.toml`, restart

### Testing Visual Changes
- Use F3 dialog to toggle modes (Classic/Enhanced/KR)
- Adjust lighting/fog/grading sliders in real-time
- Verify all three presets after changes

### Build Commands
```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized)
cargo run                # Run debug
cargo clippy             # Lint
cargo fmt                # Format
```

---

## 14. Current Status & TODOs

### Implemented ✓
- Land tile rendering with Paged Tile Atlas
- Player movement (WASD + PageUp/Down)
- Camera follow + zoom
- Three shader modes (Classic, Enhanced, KR-like)
- LRU texture eviction (60s)
- BC7 compression support
- Multi-map discovery
- Performance overlay
- In-game logger
- Configurable keybindings
- Teleport dialog

### Planned (docs/TODO.md)
- [ ] Split WGSL shader into multiple files
- [ ] Further optimize uocf texmap loading (SIMD)
- [ ] Move default shader preset to TOML
- [ ] Adapt 'far' projection to zoom level
- [ ] Hot-reload settings and presets
- [ ] Texture Array expansion (dynamic resize)
- [ ] Update CODE_OVERVIEW.md

---

## 15. Quick Reference: File Locations

### Rendering
- Shader: `assets/shaders/worldmap/land_base.wgsl`
- Material: `dynamapper/src/core/render/scene/world/land/mesh_material.rs`
- Draw chunks: `dynamapper/src/core/render/scene/world/land/draw_mesh.rs`
- Tile Atlas: `dynamapper/src/core/render/scene/world/land/tile_atlas.rs`
- Base mesh: `dynamapper/src/core/render/scene/world/land/setup_base_mesh.rs`

### UO Files
- Map parser: `uocf/src/geo/map.rs`
- Texture 2D: `uocf/src/geo/land_texture_2d.rs`

### UI
- Overlays: `dynamapper/src/core/render/overlays/`
- Dialogs: `dynamapper/src/core/render/dialogs/`

### Configuration
- Presets: `assets/shader_presets.toml`
- Settings: `assets/settings.toml`
- Keybindings: `assets/keybindings.toml`

---

## 16. Agent Workflow Reminders

### Before Making Changes
1. Read GEMINI.md for detailed architectural guidance
2. Check CODE_OVERVIEW.md for high-level flow
3. Verify binding indices match between Rust and WGSL
4. Test all three shader presets after visual changes

### After Completing Tasks
1. Ask user for confirmation (does it work as expected?)
2. If yes, ask if SESSION_OVERVIEW.md and CODE_OVERVIEW.md should be updated
3. Run `cargo build` to verify no compile errors
4. Run `cargo clippy` for linting

### When Searching Code
- Use `glob` for file patterns
- Use `grep_search` for keyword searches
- Use `task` agent for complex multi-file searches

### Common Issues & Fixes

**Dialogs/Overlays not showing**: In `bevy_egui` 0.39 with Bevy 0.18, all egui rendering systems MUST run in the `EguiPrimaryContextPass` schedule, NOT in `Update` or `PostUpdate`.
```rust
// WRONG (won't render):
app.add_systems(PostUpdate, sys_render_dialog);

// CORRECT (will render):
app.add_systems(EguiPrimaryContextPass, sys_render_dialog);
```

---

**Last Updated**: venerdì 13 marzo 2026
**Bevy Version**: 0.18.1
**Rust Edition**: 2024
