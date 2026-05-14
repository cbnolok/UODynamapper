# Technical Reference - UODynamapper

This document is the authoritative source for technical specifications, data formats, and shared constants used across the UODynamapper project.

---

## 1. Terrain Rendering Specifications

### 1.1 Shader Presets

The renderer supports three distinct visual modes, controlled by the `shading_mode` uniform:

| Mode | Value | Normals | Shading | Features |
|------|-------|---------|---------|----------|
| **Classic 2D** | 0 | Geometric | Gouraud (vertex) | Faceted look, Lambert only |
| **Enhanced Classic** | 1 | Bicubic | Per-fragment | Smooth normals, diffuse + fill |
| **KR-like** | 2 | Bicubic + Bent | Per-fragment | Full stack: rim/spec/fill, fog, grading, tonemap |

### 1.2 Uniform Bindings (Group 3)

Rust `#[uniform(10X)]` must **exactly match** WGSL `@binding(10X)`.

| Binding | Struct | Scope | Purpose |
|---------|--------|-------|---------|
| 104 | `AtlasParams` | Land | Tile atlas paging parameters (paging, layers, UV math) |
| 105 | `SceneUniform` | Global | Camera position, light direction, zoom, time |
| 106 | `LandEffectsUniform` | Land | Texture/rendering toggles (filtering, blur, reconstruction) |
| 107 | `GlobalLightingUniforms` | Global | Shared lighting: fog, tonemap, color grading, ambient |
| 108 | `LandLightingUniforms` | Land | Land-specific: bent normals, rim, specular, sky/ground fill |

> [!IMPORTANT]
> `GlobalLightingUniforms` is designed for sharing across all geometry (Land, Statics, Mobiles).
> `LandLightingUniforms` is specific to terrain requiring 3D surface normals.

---

## 2. Data Formats

### 2.1 Paged Tile Metadata Atlas

Stored in a layered `Rg16Uint` GPU texture array (4 bytes per tile).

| Channel | Bits | Content | Notes |
|---------|------|---------|-------|
| **Red** | 16 | `tile_id` | Graphic ID (0..65535) |
| **Green (Low 8)** | 8 | `height_biased` | Signed `i8` height + 128 offset |
| **Green (High 8)**| 8 | `tex_size` | 0 = 64x64 (small), 1 = 128x128 (big) |

**Logical Paging Strategy**:
- **Page Size**: 2048x2048 tiles.
- **Mapping**: Logical world pages are mapped to physical layers through an LRU table.
- **Updates**: Per-chunk metadata is uploaded as sub-rect updates via `queue.write_texture`.
- **Headroom**: Supports up to 32k × 32k tile maps with current `WORLD_PAGES_X` (16) config.

### 2.2 ID Spaces

To navigate the asset resolution chain, it is critical to distinguish between three distinct ID spaces:

1.  **Classic Client ID (CC ID)**: The 16-bit ID stored in legacy `map.mul` files (e.g., `168` for water).
2.  **EC Material ID**: A canonical category ID used by the EC Client to group related textures (e.g., Material `5` is "Water").
3.  **UDDP Slot Index**: A direct index into the pre-computed `slots` table in a `.uddp` package. This is what the engine uses at runtime to find atlas coordinates.

### 2.3 EC Texture Resolution Chain

The resolution process is split into two phases to minimize runtime CPU overhead:

#### Phase 1: Build Time (`uddconv_cli`)
1. **Blueprint**: Reads `TerrainDefinition.uop` to map Materials to texture hashes and aliases.
2. **Packing**: Extracts images from `Textures.uop`, deduplicates, and packs them into UDDP Pages.
3. **Slots**: Records physical atlas coordinates `[Page Index, X, Y, Width, Height]` for each image.
4. **Provenance**: Bakes a mapping of `EC Material ID ➔ Slot ID`.

#### Phase 2: Runtime (`dynamapper`)
1. **Translation**: CC Tile ID is optionally translated to EC Material ID via `TerrainTranscode.kdl`.
2. **Provenance**: Map `EC Material ID ➔ Slot ID` using the baked provenance table.
3. **Slot**: Resolve `Slot ID ➔ Atlas Coordinates`.
4. **GPU**: Upload coordinates to the `tex_land_ec_lookup` texture.

**Fallback**: If a CC Tile ID is unmapped in KDL, the engine directly checks the provenance array for legacy aliases named after the CC ID.

---

## 3. World Concepts & Performance

### 3.1 Mesh Selection (Scale + LOD)

Mesh selection uses a two-level dispatch:

1. **Chunk Scale** (Primary): As zoom increases, 8x8 blocks merge into "super-chunks" (up to 256x256). Each wide mesh maintains 81 vertices but increases the inter-vertex step. This is the primary mechanism for reducing draw-call pressure.
2. **LOD (Scale=1 only)**: Three detail levels (High: 81, Medium: 25, Low: 9 vertices). Swapped via cheap `Mesh3d` handle reassignment.

**Rationale**: LOD variation saves negligible GPU work in real-time but is critical for **offline export rendering** (capturing full-map 1:1 images), where total vertex counts for Britannia can drop from 35M to 4M.

### 3.2 Capacity Analysis (Britannia Baseline)

*Britannia: 7168 × 4096 tiles*

| Resource | Requirement | Limit | Headroom |
| -------- | ----------- | ----- | -------- |
| **Atlas pages** | ceil(7168/2048) × ceil(4096/2048) = **8** | 32 layers | ×4 |
| **Small texture layers** | ≤1 869 unique IDs | 2 048 | ×1.1 |
| **Big texture layers** | ≤1 869 unique IDs | 2 048 | ×1.1 |
| **Atlas VRAM** | 8 × 2048² × 4 B = **128 MiB** | — | — |
| **Texture VRAM (BC7)** | 4 + 16 = **~20 MiB** | — | — |

### 3.3 Texture Compression (BC7)

- **Savings**: ~8x VRAM reduction.
- **Alignment**: `bytes_per_row = (width + 3) / 4 * 16`.
- **Block Size**: 4x4 pixels, 16 bytes per block.

---

## 4. Shared Management & Safety

### 4.1 Boundary Safety
- **Super-chunks**: Only accepted if the *entire* scaled chunk fits within map dimensions.
- **Map Blocks**: Missing blocks are skipped safely during atlas enqueue to prevent out-of-bounds panics.

### 4.2 Change Detection (The `get_mut()` Rule)

**CRITICAL**: Avoid `materials.get_mut()` in hot paths.
- **Problem**: Triggers Bevy's change detection → forces expensive re-extraction and re-binding of global textures.
- **Result**: High GPU overhead (70%+) even when idle.
- **Solutions**:
  - Use `get()` for read-only checks.
  - Use `write_texture` for direct atlas updates.
  - Use WGSL built-in `globals.time` instead of manual time uniforms.

### 4.3 Resource Lifetimes
- **Idle Eviction**: 60 seconds of inactivity.
- **Eviction Check**: Every 5 seconds via `sys_evict_map_blocks`.
- **Targets**: `MapBlock` (RAM), `TexMap2D` pixel data (RAM/VRAM).

---

## 5. Developer Guidelines & Patterns

### 5.1 Configuration Policy (No Hidden Defaults)

**Rule**: All settings must be explicitly defined in TOML files. Never use `.unwrap_or()` or `.unwrap_or_default()` in Rust code for configuration.

**Rationale**:
- Hidden defaults lead to "configuration drift" and make debugging harder.
- Users should see all available options by inspecting the TOML files.

**Correct Pattern**:
```rust
let value = settings.get("my_setting")
    .ok_or_else(|| ConfigError::Missing("my_setting".to_string()))?;
```

### 5.2 Logging & Diagnostics API

**Internal Log Categories (`LogAbout`)**:
Used to filter and format messages in the console and in-game logger.
- `RenderWorldLand`, `RenderWorldArt`, `Performance`, `UoFiles`, `Startup`, `Settings`, `Player`.

**Usage**:
- `console_logger::one(is_sys, severity, category, message)`
- `console_logger::system(message)` (shorthand for startup info)

**Interception**:
An `InterceptLogLayer` captures `tracing` events from Bevy and `uocf`, routing them through the internal formatter to ensure a unified visual style.

### 5.3 Plugin Tracking (`TrackedPlugin`)

All internal plugins must implement `TrackedPlugin` to maintain a visual registry tree at startup.
1. Add `pub registered_by: &'static str` to the struct.
2. Call `log_plugin_build(self)` at the start of `build()`.
3. Use `impl_tracked_plugin!(MyPlugin);` macro.

---

## 6. UI & Interaction Specification

### 6.1 Dialog Mappings

| Dialog | Trigger | Purpose | Schedule |
|--------|---------|---------|----------|
| **Terrain Shader** | `F3` (Configurable) | Real-time uniform testing | `EguiPrimaryContextPass` |
| **Keybindings** | `F1` (Configurable) | Help reference | `EguiPrimaryContextPass` |
| **Options Menu** | `F2` (Configurable) | Application settings | `EguiPrimaryContextPass` |
| **Teleport** | `Ctrl+G` | Coordinate navigation | `EguiPrimaryContextPass` |

### 6.2 Overlay System
- **Performance**: Top-right (FPS, CPU, RSS RAM).
- **Position**: Top-left (Player coordinates).
- **Log**: Bottom-left (Max 5 messages, 8s timeout).

---

## 7. Advanced Rendering Details

### 7.1 Neighborhood Sampling (Bicubic)

The terrain shader performs a 4x4 neighborhood read for bicubic normal reconstruction.
- **Implementation**: `textureLoad` reads from `tile_meta_atlas` using absolute world-tile coordinates.
- **Boundary Handling**: Because the atlas is global/paged, sampling naturally crosses chunk boundaries without requiring per-chunk padding ("ghost tiles").

### 7.2 Multi-Map Orchestration
- **Discovery**: `UOFilesPlugin` auto-discovers `map0.mul` through `map5.mul`.
- **Tagging**: Every chunk entity is tagged with a `parent_map_id` component.
- **Validation**: Movement and teleportation logic validates coordinates against the target plane's dimensions stored in `WorldGeoData`.

---

## 8. Build & Optimization Specs

### 8.1 Release Profile
- **LTO**: Full.
- **Optimization**: `opt-level = "s"` (Size).
- **Symbols**: Stripped.

### 8.2 Power Management
- **ReactiveLowPower**: Triggered when the window loses focus.
- **Impact**: Reduces CPU/GPU polling and frame rate to conserve resources.
