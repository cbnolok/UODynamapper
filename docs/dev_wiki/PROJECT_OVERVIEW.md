# Project Overview - UODynamapper

High-level summary of the UODynamapper project for contributors and AI agents to quickly understand the goals, technology stack, and current status.

---

## 1. Project Summary

**Goal**: Create a dynamic Ultima Online renderer and tooling stack that can reproduce classic content, ingest Enhanced Client ownership and texture data, and evolve toward richer modern rendering without losing data provenance.

**Tech Stack**:
- **Language**: Rust (Edition 2024)
- **Engine**: Bevy v0.18.1
- **Shaders**: WGSL (with naga-oil)
- **GPU API**: wgpu

**Workspace Members**:
- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `uocf/` - Ultima Online file parser for classic and EC package formats
- `udd-assets/` - Runtime readers and package access helpers for converted assets
- `udd-container/` - Container and package infrastructure
- `udd-conv/` - UODynamapper-specific conversion and packaging logic
- `udd-conv-ktx2/` - KTX2 texture handling
- `tools/udd-conv-cli/` - CLI for building and inspecting converted UODynamapper packages
- `tools/uocf-cli/` - Generic UO tooling CLI crate for UOP/package operations and format conversion utilities

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
| `assets/shaders/world/land/main.wgsl` | Land shader |

---

## 3. Core Features

### Naming
- Use `land` for project-owned files, modules, UI labels, and documentation.
- Preserve exact upstream resource names such as `TerrainDefinition.uop`, `TerrainTexture.uop`, and client-provided virtual paths when referring to those files directly.
- The supported client families are Classic, Kingdom Reborn, and Enhanced.

### Rendering
- **Three Shader Modes**: Classic 2D, Enhanced Classic, KR-like
- **Paged Tile Atlas**: GPU-driven land metadata for massive maps
- **BC7 Compression**: 8x VRAM reduction (~160MB → ~20MB)
- **Zoom-Driven Chunk Scaling**: Dynamic chunk coverage (8x8 up to 256x256)
- **Exact Viewport Visibility**: Uses Bevy camera ray projection
- **Hot-Reload**: Shader changes apply automatically
- **EC Material Routing**: surface-like statics can route through land-style EC slots when tile metadata and provenance support it

### Current Runtime Decisions
- **Map Chunking**: `32x32` for now
- **Statics Chunking**: `32x32` for now
- **Land Transport Unit**: prefer `64x64` UDDP decompression units while keeping render granularity independent
- **Static Depth Port**: staged migration; current runtime is still mixed between interpolated sprite depth and early logical-depth work

### User Interface
- **Performance Overlay**: FPS, CPU%, RAM (top-right)
- **Player Position**: Coordinates display (top-left)
- **System Messages**: In-game log with severity colors (bottom-left)
- **Dialogs**: F1 (Help), F2 (Options), F3 (Shader Controls) — all configurable — plus Ctrl+G (Teleport)

### Configuration
- **Modular TOML**: Settings split into `core.toml`, `runtime_assets.toml`, `graphics.toml`, etc. under `assets/settings/`.
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

### 7.3 Enhanced Client Material Ownership
The EC pipeline does not rely on one flat texture list. Current work uses:
- `tileart.uop` for art and static ownership
- `TerrainDefinition.uop` for land ownership and aliases
- `Texture.uop` and `LegacyTexture.uop` as shared mixed pools
- `TerrainTexture.uop` and `EffectTexture.uop` as support-resource pools that must be preserved even when they are not directly renderable yet

### 7.4 Rendering Presets
Supports **Classic 2D**, **Enhanced Classic**, and **KR-like** modes with unified shader paths.

---

**Last Updated**: May 2026  
**Bevy Version**: 0.18.1  
**Rust Edition**: 2024
