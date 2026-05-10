# Project Overview - UODynamapper

High-level summary of the UODynamapper project for contributors and AI agents to quickly understand the goals, technology stack, and current status.

---

## 1. Project Summary

**Goal**: Create a 2D client/dynamic map renderer for Ultima Online with a highly configurable renderer that can emulate both classic and modern (Kingdom Reborn) visuals.

**Tech Stack**:
- **Language**: Rust (Edition 2024)
- **Engine**: Bevy v0.18.1
- **Shaders**: WGSL (with naga-oil)
- **GPU API**: wgpu

**Workspace Members**:
- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `uocf/` - Ultima Online file parser (map.mul, art.mul, tiledata.mul)
- `uddconv/` - UODynamapper-specific converted asset packaging and runtime readers
- `uddconv_ktx2/` - KTX2 texture handling
- `tools/uddconv_cli/` - CLI for building and inspecting UODynamapper-specific converted packages
- `tools/uocf_cli/` - Generic UO tooling CLI crate for UOP/package operations and format conversion utilities

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
- **Zoom-Driven Chunk Scaling**: Dynamic chunk coverage (8x8 up to 256x256)
- **Exact Viewport Visibility**: Uses Bevy camera ray projection
- **Hot-Reload**: Shader changes apply automatically

### User Interface
- **Performance Overlay**: FPS, CPU%, RAM (top-right)
- **Player Position**: Coordinates display (top-left)
- **System Messages**: In-game log with severity colors (bottom-left)
- **Dialogs**: F1 (Help), F2 (Options), F3 (Shader Controls) — all configurable — plus Ctrl+G (Teleport)

### Configuration
- **Modular TOML**: Settings split into `core.toml`, `uo_files.toml`, `graphics.toml`, etc. under `assets/settings/`.
- **Shader Presets**: Stored in `assets/defaults/shader_presets.toml`.

---

## 4. Documentation Structure

| Document | Purpose |
| -------- | ------- |
| **docs/TECHNICAL_REFERENCE.md** | Authoritative tech specs, data formats, and constants |
| **docs/CONTRIBUTORS_GUIDE.md** | Quick file reference, common tasks |
| **GEMINI.md** | AI agent workflow and best practices |
| **docs/CODE_OVERVIEW.md** | Low-level architecture details |
| **docs/TODO.md** | Roadmap and planned features |

---

## 5. Application States
```text
StartupSetup → InGame
```
- **StartupSetup**: Default state. Loads UO files, sets up scene.
- **InGame**: Main interactive state.
- **Stop**: Shutdown state.

---

## 6. Plugin Architecture
Core plugins are registered in `core.rs`. For details on sub-plugins and hierarchy, see **[docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md)**.

---

## 7. Core Architecture Concepts (Summary)

For full technical specifications, data formats, and constants, see **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)**.

### 7.1 Paged Tile Metadata Atlas
Terrain metadata is stored in a layered `Rg16Uint` texture array. This eliminates per-chunk uniform updates and allows massive maps with minimal draw calls.

### 7.2 Multi-Scale Chunk Rendering
The renderer merges base 8x8 blocks into larger "super-chunks" (up to 256x256) as zoom increases to reduce draw-entity pressure.

### 7.3 Rendering Presets
Supports **Classic 2D**, **Enhanced Classic**, and **KR-like** modes with unified shader paths.

---

## 8. Performance Targets
- **60s Idle Eviction**: Automatic memory management for stale data.
- **BC7 Compression**: 8x VRAM reduction.
- **Zero-Copy Streaming**: Memory-mapped UDDP files.

---

## 9. Current Status

### Implemented ✓
- [x] Land tile rendering with Paged Tile Atlas
- [x] Three shader modes (Classic, Enhanced, KR-like)
- [x] Multi-map discovery and support (map0-map5)
- [x] BC7 compression & LRU eviction
- [x] Zoom-based chunk scaling and LOD selection
- [x] Exact camera-projected visibility computation
- [x] Async asset streaming via Bevy task pools

### Planned (docs/TODO.md)
- [ ] Reduce idle CPU further with event-driven scheduling
- [ ] Adapt 'far' projection to zoom level
- [ ] Hot-reload settings and presets
- [ ] Reduce temporary allocations in chunk build/upload paths

---

**Last Updated**: May 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
