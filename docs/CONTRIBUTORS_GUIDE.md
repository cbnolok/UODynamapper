# Contributors Guide - UODynamapper

Quick reference document for contributors and AI agents to quickly locate relevant files and understand key concepts without traversing the entire codebase.

---

## 1. Quick File Reference

### By Task Type

| I want to... | Look in... |
| ------------ | ---------- |
| **Modify terrain visuals** | `assets/shaders/worldmap/land/main.wgsl` |
| **Add a new uniform** | `dynamapper/src/core/render/scene/world/land/mesh_material.rs` (Rust) + `assets/shaders/worldmap/land/bindings.wgsl` (shader) |
| **Change UI overlays** | `dynamapper/src/core/render/overlays/` |
| **Modify dialogs (F3, F1, etc.)** | `dynamapper/src/core/render/dialogs/` |
| **Adjust keybindings** | `assets/settings/keybindings.toml` |
| **Change shader presets** | `assets/defaults/shader_presets.toml` |
| **Modify app settings** | `assets/settings/` (modular TOML files: core, graphics, uo_files, etc.) |
| **Understand map file parsing** | `uocf/src/geo/map.rs` |
| **Understand texture loading** | `uocf/src/geo/land_texture_2d.rs` |
| **Debug rendering issues** | `dynamapper/src/core/render/scene/world/land/` |
| **Change application flow** | `dynamapper/src/core/app_states.rs` |
| **Modify system execution order** | `dynamapper/src/core/system_sets.rs` |

### Core Architecture Files

| File | Purpose |
| ---- | ------- |
| `dynamapper/src/main.rs` | Application entry point |
| `dynamapper/src/core.rs` | Bevy app setup, plugin registration |
| `dynamapper/src/core/app_states.rs` | State machine (Startup → InGame) |
| `dynamapper/src/core/system_sets.rs` | System execution order |

### Terrain Rendering Pipeline

| File | Purpose |
| ---- | ------- |
| `assets/shaders/worldmap/land/main.wgsl` | WGSL terrain shader entry point |
| `assets/shaders/worldmap/land/bindings.wgsl` | Struct definitions and bind group declarations |
| `assets/shaders/worldmap/land/shading.wgsl` | Shading models (Gouraud, per-fragment, grading) |
| `assets/shaders/worldmap/land/lighting.wgsl` | Light evaluation (Lambert, rim, specular, fill, tonemap) |
| `assets/shaders/worldmap/land/sampling.wgsl` | Texture sampling (small/big atlas, filtering) |
| `assets/shaders/worldmap/land/normals.wgsl` | Normal generation (geometric, bicubic, bent) |
| `assets/shaders/worldmap/land/atlas.wgsl` | Tile metadata atlas lookups |
| `assets/shaders/worldmap/land/noise.wgsl` | Noise utilities |
| `dynamapper/src/core/render/scene/world/land/mesh_material.rs` | Rust uniform structs (must match WGSL) |
| `dynamapper/src/core/render/scene/world/land/draw_mesh.rs` | Chunk data collection, atlas uploads, scale-aware mesh assignment |
| `dynamapper/src/core/render/scene/world/land/tile_atlas.rs` | Metadata atlas paging and LRU layer management |
| `dynamapper/src/core/render/scene/world/land/setup_base_mesh.rs` | Shared LOD and wide-mesh generation |
| `dynamapper/src/core/render/terrain_shader_ui.rs` | F3 shader uniform controls |
| `dynamapper/src/core/render/scene.rs` | Visible chunk set computation, throttled spawn/despawn orchestration |
| `dynamapper/src/core/texture_cache/land.rs` | Terrain texture cache setup, eviction, dynamic array resize |
| `dynamapper/src/core/texture_cache/land/cache.rs` | Small/big texture-array residency and GPU uploads |

### UO File Parsing (uocf crate)

| File | Purpose |
| ---- | ------- |
| `uocf/src/geo/map.rs` | `map.mul` parser (MapBlock, MapCell) |
| `uocf/src/geo/land_texture_2d.rs` | `TexMap2D` lazy texture loading |
| `uocf/src/tiledata.rs` | `tiledata.mul` parser |
| `uocf/src/generic_def.rs` | Generic definitions |
| `uocf/src/generic_index.rs` | Index file parsing |

---

## 2. Key Concepts at a Glance

### Paged Tile Metadata Atlas

Terrain metadata is stored in a **layered Rg16Uint texture array** instead of per-chunk uniforms:

```text
Format: 4 bytes per tile
├─ R16: tile_id (0..65535)
└─ G16: packed [height_biased:low 8 | tex_size:high 8]
   - height_biased = height + 128 (signed i8 → u8)
   - tex_size: 0=64x64 atlas, 1=128x128 atlas
```

**Why**: Eliminates material churn, enables massive maps, allows seamless neighborhood sampling across chunk boundaries.

### Shader Presets

| Mode | Value | Features |
| ---- | ----- | -------- |
| Classic 2D | 0 | Faceted normals, Gouraud (vertex) lighting |
| Enhanced Classic | 1 | Smooth normals, per-fragment lighting, fill light |
| KR-like | 2 | Full suite: rim/spec highlights, fog, color grading, tonemapping |

### Uniform Binding Indices

**CRITICAL**: Rust `#[uniform(10X)]` must match WGSL `@binding(10X)`:

```text
@binding(104) → AtlasParams
@binding(105) → SceneUniform
@binding(106) → LandEffectsUniform      (texture/rendering toggles)
@binding(107) → GlobalLightingUniforms  (shared: fog, tonemap, grading, ambient)
@binding(108) → LandLightingUniforms    (land-specific: bent, rim, fill, specular)
```

`GlobalLightingUniforms` is designed to be shared with future art/item shaders. `LandLightingUniforms` contains parameters requiring 3D surface normals (land terrain only).

### Idle Eviction

- **What**: MapBlocks + texture pixel data
- **When**: Not accessed for 60 seconds
- **Check**: Every 5 seconds via `sys_evict_map_blocks`

### Zoom-Driven Chunk Scaling

Visible terrain is rendered at different chunk granularities depending on zoom:

| Zoom Range | Scale | Coverage |
| ---------- | ----- | -------- |
| `< 10` | 1 | 8x8 tiles |
| `10-25` | 2 | 16x16 tiles |
| `25-50` | 4 | 32x32 tiles |
| `>= 50` | 8 | 64x64 tiles |

The renderer still loads base 8x8 map blocks internally, but combines them into fewer entities at high zoom.

### BC7 Compression

- **Savings**: ~160MB → ~20MB VRAM (~8x reduction)
- **Library**: `intel_tex_2` with `alpha_basic_settings`
- **Alignment**: `bytes_per_row = (width + 3) / 4 * 16` (4x4 blocks)

---

## 3. Common Workflows

### Add a New Uniform Parameter

1. Add field to correct Rust struct in `mesh_material.rs` (respect `std140` alignment)
   - `LandEffectsUniform` for texture/rendering toggles
   - `GlobalLightingUniforms` for shared lighting (fog, tonemap, grading)
   - `LandLightingUniforms` for land-specific lighting (bent, rim, fill, specular)
   - Padding field names must NOT end with a digit (naga_oil constraint)
2. Populate uniform in `draw_mesh.rs` (`create_land_chunk_material`)
3. Add field to WGSL struct in `bindings.wgsl`
4. Use uniform in appropriate shader module
5. **Verify**: Binding indices match between Rust and WGSL

### Modify a Visual Effect

1. Open `assets/shaders/worldmap/land/main.wgsl`
2. Find relevant section (e.g., "Lighting composition", "KR-style fog")
3. Use F3 UI to test changes without recompiling Rust
4. Verify all three shader presets (Classic, Enhanced, KR-like)

### Debug Common Issues

| Symptom | Likely Cause | Fix |
| ------- | ------------ | --- |
| `Binding is missing from pipeline layout` | Binding index mismatch | Verify `#[uniform(10X)]` = `@binding(10X)` in `mesh_material.rs` and `bindings.wgsl` |
| Colors washed out / grayish | Double gamma correction | Remove manual `pow(color, 1.0/2.2)` |
| Shader compile error | WGSL syntax/alignment | Read wgpu error (points to exact line) |
| High GPU usage (70%+) idle | `get_mut()` or unnecessary asset mutation in hot path | Use `get()` or change-only `get_mut()` |
| Missing far edge chunks | Visible-set math drift or bad super-chunk bounds | Inspect `scene.rs::compute_visible_chunks()` |
| WGPU Z-layer overrun | Texture cache layer mismatch or out-of-bounds chunk requests | Check texture-array initial sizes and map-edge bounds guards |
| Dialog/overlay not showing | Wrong Bevy schedule | Use `EguiPrimaryContextPass`, not `Update` |

---

## 4. Configuration Files

| File | Purpose |
| ---- | ------- |
| `config.toml` | Cargo build settings, linker config |
| `assets/settings/core.toml` | Window size, debug flags, power saving |
| `assets/settings/uo_files.toml` | UO installation paths and file configuration |
| `assets/settings/graphics.toml` | Rendering settings (BC7, texture options) |
| `assets/settings/maps.toml` | Map-specific configuration |
| `assets/settings/preferences.toml` | User preferences |
| `assets/settings/keybindings.toml` | Keyboard shortcuts (fully runtime-configurable) |
| `assets/defaults/shader_presets.toml` | Shader uniform presets (3 modes × 4 times of day) |

---

## 5. Build & Development

### Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (optimized)
cargo run                # Run debug
cargo clippy             # Lint
cargo fmt                # Format
```

### Hot-Reload Workflow

1. Shader changes → automatic hot-reload (Bevy file watcher)
2. Uniform tweaks → F3 UI for runtime testing
3. Preset changes → edit `assets/defaults/shader_presets.toml`, restart

### Build Optimizations

- **Linker**: `mold` (3-5x faster linking, CI only)
- **Release**: `opt-level = "s"` + LTO + strip
- **Debug**: `opt-level = 1`, incremental enabled

---

## 6. Performance Guidelines

### CRITICAL: Avoid `get_mut()` in Hot Paths

**Problem**: Calling `get_mut()` on Materials triggers Bevy's change detection → expensive re-extraction + re-binding every frame → high idle GPU/CPU usage.

**Solution**: 

- Use `get()` for read-only checks
- For terrain metadata: use `write_texture` directly to Tile Atlas
- Only use `get_mut()` if comparison proves data actually changed
- Prefer built-in shader globals like `globals.time` over CPU-side time uniforms when possible

### Memory Management

- **Lazy Loading**: Textures load on-demand, not upfront
- **Idle Eviction**: 60s timeout, checked every 5s
- **BC7 Compression**: Enable in `assets/settings/graphics.toml` for 8x VRAM savings
- **Power Saving**: `ReactiveLowPower` mode when window unfocused

### Configuration Loading Policy

**No Hidden Defaults**: Never use `.unwrap_or()` or `.unwrap_or_default()` when loading TOML settings. All values must be explicitly defined in TOML files.

**Why**: Hidden defaults in Rust code are hard to discover and lead to configuration drift.

**Pattern**:

```rust
// CORRECT - fails with clear error if missing
let value = settings
    .get("my_setting")
    .ok_or_else(|| ConfigError::Missing("my_setting".to_string()))?;

```rust
// WRONG - hidden default
let value = settings.get("my_setting").unwrap_or(42);
```

---

## 7. Plugin Registry & Tracking

The project includes a custom `TrackedPlugin` mechanism (see `util_lib/tracked_plugin.rs`) to maintain a clear visual tree of all registered plugins at startup.

### Rules for New Plugins

1. **Internal Plugins**: MUST implement `TrackedPlugin`.
   - Add a `pub registered_by: &'static str` field to the plugin struct.
   - Use the `impl_tracked_plugin!(MyPlugin);` macro.
   - Call `log_plugin_build(self);` at the very beginning of the `build()` method.
2. **External Plugins**: If you add a third-party plugin (e.g., from crates.io), manually record it in the registry in `core.rs` (or the parent plugin's `build` method) using:
   ```rust
   let mut registry = crate::util_lib::tracked_plugin::plugin_registry().lock().unwrap();
   registry.record("ExternalPluginName", "RegisteredByPluginName");
   ```

This ensures the "Plugin registry tree" logged at startup remains accurate.

## 8. Logging System

### Categories

- `RenderWorldLand` - Terrain rendering
- `Performance` - Heavy operations (atlas uploads, eviction, BC7)
- Severity: `Info` | `Warning` | `Error`

### In-Game Logger

- **Location**: Bottom-left overlay
- **Symbols**: ⓘ (info), ⚠ (warning), ✖ (error)
- **Colors**: White, Yellow, Red
- **Timeout**: 8s + 1s fade-out

---

## 9. Multi-Map Support

- Auto-discovers `map0.mul` through `map5.mul`
- Each map plane indexed by ID (0-5)
- Chunk entities tagged with `parent_map_id`
- Teleport dialog supports M (map plane) coordinate

---

## 10. Key Constants

```rust
TILE_NUM_PER_CHUNK_DIM = 8       // 8x8 tiles per chunk
TILE_NUM_PER_CHUNK_TOTAL = 64

scale 1 -> 8x8 tiles
scale 2 -> 16x16 tiles
scale 4 -> 32x32 tiles
scale 8 -> 64x64 tiles

PAGE_TEXELS = 2048               // World page size
MAX_LAYERS = 8                   // Metadata atlas layers
SMALL_INITIAL_LAYERS = 256
BIG_INITIAL_LAYERS = 128
```

---

## 11. Related Documentation

- **GEMINI.md**: AI agent instructions, workflow, and best practices
- **docs/PROJECT_OVERVIEW.md**: High-level project summary, goals, status
- **docs/CODE_OVERVIEW.md**: Low-level design choices, architecture details

---

**Last Updated**: giovedì 2 aprile 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
