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
```
main.rs → core.rs (Bevy app setup) → AppState machine
```

### Key Files at a Glance
| File | Purpose |
|------|---------|
| `dynamapper/src/main.rs` | Application entry point |
| `dynamapper/src/core.rs` | Bevy app configuration, plugin registration |
| `dynamapper/src/core/app_states.rs` | State machine (Startup → InGame) |
| `assets/shaders/worldmap/land_base.wgsl` | Terrain shader |

---

## 3. Core Features

### Rendering
- **Three Shader Modes**: Classic 2D, Enhanced Classic, KR-like
- **Paged Tile Atlas**: GPU-driven terrain metadata for massive maps
- **BC7 Compression**: 8x VRAM reduction (~160MB → ~20MB)
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

---

## 4. Documentation Structure

| Document | Purpose | Audience |
|----------|---------|----------|
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

```
StartupSetup → AssetsLoading → InGame
```

- **StartupSetup**: Initial state, startup systems run
- **AssetsLoading**: Loading UO game files
- **InGame**: Main interactive state (player movement, rendering, UI)

---

## 6. Plugin Architecture

Core plugins registered in `core.rs`:

| Plugin | Purpose |
|--------|---------|
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

```
Format: 4 bytes per tile
├─ R16: tile_id (0..65535)
└─ G16: packed [height_biased:low 8 | tex_size:high 8]
```

**Benefits**:
- Eliminates material churn (thousands of chunks share one material)
- Enables massive maps (10,000x10,000+ tiles)
- Seamless neighborhood sampling across chunk boundaries

### Shader Presets

| Mode | Value | Characteristics |
|------|-------|-----------------|
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
- [x] Three shader modes (Classic, Enhanced, KR-like)
- [x] LRU texture eviction (60s)
- [x] BC7 compression support
- [x] Multi-map discovery (map0-map5)
- [x] Performance overlay (FPS, CPU, RAM)
- [x] In-game logger with severity colors
- [x] Configurable keybindings
- [x] Teleport dialog (Ctrl+G)
- [x] Power saving mode

### Planned (docs/TODO.md)
- [ ] Split WGSL shader into multiple files
- [ ] Further optimize uocf texmap loading (SIMD)
- [ ] Move default shader preset to TOML
- [ ] Adapt 'far' projection to zoom level
- [ ] Hot-reload settings and presets
- [ ] Texture Array expansion (dynamic resize)

---

## 11. Key Constants

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

## 12. Common Issues Quick Reference

| Issue | Quick Fix |
|-------|-----------|
| `Binding is missing` | Verify `#[uniform(10X)]` = `@binding(10X)` |
| Colors washed out | Remove manual gamma correction |
| High GPU usage idle | Check for `get_mut()` in hot paths |
| Dialogs not showing | Use `EguiPrimaryContextPass` schedule |

For detailed troubleshooting, see `docs/CONTRIBUTORS_GUIDE.md` or `GEMINI.md`.

---

## 13. Getting Help

- **File locations**: `docs/CONTRIBUTORS_GUIDE.md`
- **Architecture details**: `docs/CODE_OVERVIEW.md`
- **AI agent workflow**: `GEMINI.md`
- **Planned features**: `docs/TODO.md`

---

**Last Updated**: sabato 14 marzo 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
