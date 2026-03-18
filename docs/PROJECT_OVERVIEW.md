# Project Overview - UODynamapper

High-level summary of the UODynamapper project for contributors and AI agents to quickly understand the goals, technology stack, and current status.

---

## 1. Project Summary

**Goal**: Create a 2D client/dynamic map renderer for Ultima Online with a highly configurable renderer that can emulate both classic and modern (Kingdom Reborn) visuals.

**Tech Stack**:
- **Language**: Rust (Edition 2024)
- **Engine**: Bevy v0.18.1
- **Shaders**: WGSL (WESL + naga-oil)
- **GPU API**: wgpu

**Workspace Members**:

- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `uocf/` - Ultima Online file parser (map.mul, art.mul, tiledata.mul)

---

## 2. Quick Start

### Entry Points

```text
main.rs → core.rs (Bevy app setup) → AppState machine
```

### Key Files at a Glance

| File | Purpose |
| ---- | ------- |
| `dynamapper/src/main.rs` | Application entry point |
| `dynamapper/src/core.rs` | Bevy app configuration, plugin registration |
| `dynamapper/src/core/app_states.rs` | State machine (Startup → InGame) |
| `assets/shaders/worldmap/land/main.wgsl` | Terrain shader |

---

## 3. Core Features

### Rendering

- **Three Shader Modes**: Classic 2D, Enhanced Classic, KR-like
- **Paged Tile Atlas**: GPU-driven terrain metadata for massive maps
- **BC7 Compression**: 8x VRAM reduction (~160MB → ~20MB)
- **Zoom-Driven Chunk Scaling**: 8x8 / 16x16 / 32x32 / 64x64 chunk coverage depending on zoom
- **Exact Viewport Visibility**: Uses Bevy camera ray projection rather than manual ortho estimation
- **Hot-Reload**: Shader changes apply automatically

### User Interface

- **Performance Overlay**: FPS, CPU%, RAM (top-right)
- **Player Position**: Coordinates display (top-left)
- **System Messages**: In-game log with severity colors (bottom-left)
- **Dialogs**:
  - F1: Keybindings help
  - F3: Terrain shader controls
  - Ctrl+G: Teleport to coordinates

### Configuration

- **Keybindings**: Fully runtime-configurable (`assets/keybindings.toml`)
- **Shader Presets**: Stored in `assets/shader_presets.toml`
- **Settings**: UO paths, window, debug options in `assets/settings.toml`

### Performance

- **Idle Eviction**: 60s timeout for inactive data
- **Lazy Loading**: Textures load on-demand
- **Power Saving**: Reactive low-power mode when unfocused
- **Spawn Throttling**: New chunk entities spawn center-out, capped per frame
- **Shared-Material Stability**: Terrain material updates only when relevant state changes
- **Dynamic Texture Array Expansion**: Small/big terrain arrays can grow on demand

---

## 4. Documentation Structure

| Document | Purpose | Audience |
| -------- | ------- | -------- |
| **docs/CONTRIBUTORS_GUIDE.md** | Quick file reference, common tasks | Humans + AI |
| **GEMINI.md** | AI agent workflow and best practices | AI agents |
| **docs/CODE_OVERVIEW.md** | Low-level architecture details | Developers |
| **docs/PROJECT_OVERVIEW.md** | This file - high-level summary | Everyone |
| **docs/TODO.md** | Planned features | Developers |

**Recommended Reading Order**:

1. **New contributors**: Start here → `CONTRIBUTORS_GUIDE.md` → `CODE_OVERVIEW.md`
2. **AI agents**: `GEMINI.md` → `CONTRIBUTORS_GUIDE.md`
3. **Quick lookup**: `CONTRIBUTORS_GUIDE.md`

---

## 5. Application States

```text
StartupSetup → AssetsLoading → InGame
```

- **StartupSetup**: Initial state, startup systems run
- **AssetsLoading**: Loading UO game files
- **InGame**: Main interactive state (player movement, rendering, UI)

---

## 6. Plugin Architecture

Core plugins registered in `core.rs`:

| Plugin | Purpose |
| ------ | ------- |
| `ControlsPlugin` | Player input (WASD, PageUp/Down for Z) |
| `RenderPlugin` | Scene, camera, world rendering |
| `UOFilesPlugin` | Load Ultima Online game files |
| `TextureCachePlugin` | Cache land/item textures |
| `SettingsPlugin` | Configuration management |
| `PerformanceOverlayPlugin` | FPS/CPU/RAM metrics |
| `SystemMessagesPlugin` | In-game log overlay |

---

## 7. Terrain Rendering Concepts

### Paged Tile Metadata Atlas

Instead of per-chunk uniforms, terrain metadata uses a **layered Rg16Uint texture array**:

```text
Format: 4 bytes per tile
├─ R16: tile_id (0..65535)
└─ G16: packed [height_biased:low 8 | tex_size:high 8]
```

**Benefits**:

- Eliminates material churn (thousands of chunks share one material)
- Enables massive maps (10,000x10,000+ tiles)
- Seamless neighborhood sampling across chunk boundaries

### Multi-Scale Chunk Rendering

Terrain rendering keeps the base logical chunk size at `8x8` tiles, but the renderer can merge base chunks into larger draw units as zoom increases:

| Zoom Range | Chunk Scale | Mesh Coverage | Typical Goal |
| ---------- | ----------- | ------------- | ------------ |
| `< 10` | 1 | 8x8 tiles | Near view detail |
| `10-25` | 2 | 16x16 tiles | Reduce entity count |
| `25-50` | 4 | 32x32 tiles | Far zoom-out |
| `>= 50` | 8 | 64x64 tiles | Extreme zoom-out |

This keeps the shader path unified while reducing draw-entity pressure dramatically at high zoom.

### Shader Presets

| Mode | Value | Characteristics |
| ---- | ----- | --------------- |
| Classic 2D | 0 | Faceted look, geometric normals, Gouraud lighting |
| Enhanced Classic | 1 | Smooth normals, per-fragment lighting, fill light |
| KR-like | 2 | Full suite: rim/spec highlights, fog, grading, tonemap |

---

## 8. Multi-Map Support

- Auto-discovers `map0.mul` through `map5.mul`
- Each map plane indexed by ID (0-5)
- Teleport dialog supports M (map plane) coordinate

---

## 9. Development Workflow

### Build Commands

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
3. Preset changes → edit `shader_presets.toml`, restart

### Testing Visual Changes

1. Press F3 to open terrain shader controls
2. Toggle between Classic/Enhanced/KR modes
3. Adjust lighting/fog/grading sliders in real-time
4. Verify all three presets after code changes

---

## 10. Current Status

### Implemented ✓

- [x] Land tile rendering with Paged Tile Atlas
- [x] Player movement (WASD + PageUp/Down for Z)
- [x] Camera follow + zoom
- [x] Exponential camera zoom
- [x] Three shader modes (Classic, Enhanced, KR-like)
- [x] LRU texture eviction (60s)
- [x] BC7 compression support
- [x] Multi-map discovery (map0-map5)
- [x] Performance overlay (FPS, CPU, RAM)
- [x] In-game logger with severity colors
- [x] Configurable keybindings
- [x] Teleport dialog (Ctrl+G)
- [x] Power saving mode
- [x] Shared-material change gating (no per-frame terrain material churn)
- [x] Exact camera-projected visible chunk computation
- [x] Chunk spawn throttling with center-out ordering
- [x] Zoom-based mesh LOD selection
- [x] Zoom-based chunk scaling (1/2/4/8)
- [x] Dynamic terrain texture-array expansion
- [x] Map-edge boundary validation for super-chunks and atlas uploads

### Planned (docs/TODO.md)

- [ ] Split WGSL shader into multiple files
- [ ] Further optimize block acquisition and in-memory map-block storage
- [ ] Reduce idle CPU further with more event-driven scheduling
- [ ] Adapt 'far' projection to zoom level
- [ ] Hot-reload settings and presets
- [ ] Reduce temporary allocations in chunk build/upload paths

---

## 11. Key Constants

```rust
// Chunk dimensions
TILE_NUM_PER_CHUNK_DIM = 8       // 8x8 tiles per chunk
TILE_NUM_PER_CHUNK_TOTAL = 64

// Chunk scaling by zoom
scale 1 -> 8x8 tiles
scale 2 -> 16x16 tiles
scale 4 -> 32x32 tiles
scale 8 -> 64x64 tiles

// Atlas paging
PAGE_TEXELS = 2048               // World page size
MAX_LAYERS = 8                   // Metadata atlas layers

// Land texture arrays
SMALL_INITIAL_LAYERS = 256
BIG_INITIAL_LAYERS = 128
```

---

## 12. Common Issues Quick Reference

| Issue | Quick Fix |
| ----- | --------- |
| `Binding is missing` | Verify `#[uniform(10X)]` = `@binding(10X)` |
| Colors washed out | Remove manual gamma correction |
| High GPU usage idle | Check for `get_mut()` or other asset-change triggers in hot paths |
| Missing edge chunks | Verify `compute_visible_chunks()` still uses camera ray projection and full super-chunk bounds checks |
| Crash while zoomed far out | Verify small/big texture array layer counts match the actual GPU array sizes |
| Dialogs not showing | Use `EguiPrimaryContextPass` schedule |

For detailed troubleshooting, see `docs/CONTRIBUTORS_GUIDE.md` or `GEMINI.md`.

---

## 13. Getting Help

- **File locations**: `docs/CONTRIBUTORS_GUIDE.md`
- **Architecture details**: `docs/CODE_OVERVIEW.md`
- **AI agent workflow**: `GEMINI.md`
- **Planned features**: `docs/TODO.md`

---

**Last Updated**: mercoledì 18 marzo 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
