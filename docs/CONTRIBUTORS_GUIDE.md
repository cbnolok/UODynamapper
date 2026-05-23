# Contributors Guide - UODynamapper

Quick reference document for contributors and AI agents to quickly locate relevant files and understand key concepts without traversing the entire codebase.

---

## 1. Quick File Reference

### By Task Type
| I want to... | Look in... |
| ------------ | ---------- |
| **Modify terrain visuals** | `dynamapper/assets/shaders/worldmap/land/main.wgsl` |
| **Work on generic UO CLI tooling** | `tools/uocf_cli/src/bin/` + `tools/uocf_cli/src/lib.rs` |
| **Work on converted package CLI tooling** | `tools/uddconv_cli/src/` |
| **Add a new uniform** | `dynamapper/src/core/render/scene/world/land/mesh_material.rs` (Rust) + `dynamapper/assets/shaders/worldmap/land/bindings.wgsl` (shader) |
| **Change UI overlays** | `dynamapper/src/core/render/overlays/` |
| **Modify dialogs (F3, F1, F2, etc.)** | `dynamapper/src/core/render/dialogs/` |
| **Modify app settings** | `dynamapper/assets/settings/` (modular TOML files) |

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
| `dynamapper/assets/shaders/worldmap/land/` | WGSL modular shaders |
| `dynamapper/src/core/render/scene/world/land/mesh_material.rs` | Rust uniform structs |
| `dynamapper/src/core/render/scene/world/land/draw_mesh.rs` | Chunk data collection and atlas uploads |
| `dynamapper/src/core/render/scene/world/land/tile_atlas.rs` | Metadata atlas paging and LRU management |

---

## 2. Core Concepts (Summary)

For full technical specifications, data formats, and constants, see **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)**.

### 2.1 Paged Tile Metadata Atlas
Terrain metadata (ID, height, size) is stored in a layered `Rg16Uint` texture array. This avoids material churn and supports massive maps.

### 2.2 Zoom-Driven Chunk Scaling
Terrain entities merge 8x8 blocks into larger "super-chunks" (up to 256x256) as the camera zooms out to maintain high performance.

### 2.3 Shader Presets
Supports **Classic 2D**, **Enhanced Classic**, and **KR-like** modes. Toggled via **F3** menu.

---

## 3. Common Workflows

### Add a New Uniform Parameter
1. Add field to correct Rust struct in `mesh_material.rs`.
2. Populate uniform in `draw_mesh.rs` (`create_land_chunk_material`).
3. Add field to WGSL struct in `bindings.wgsl`.
4. Use uniform in appropriate shader module.
5. **Verify**: Binding indices must match.

### Modify a Visual Effect
1. Open `assets/shaders/worldmap/land/main.wgsl`.
2. Use **F3 UI** to test changes in real-time.
3. Verify all three shader presets.

### Debug Common Issues
| Symptom | Fix |
| ------- | --- |
| `Binding is missing` | Verify `#[uniform(10X)]` = `@binding(10X)` |
| Colors washed out | Remove manual gamma correction (Bevy handles it) |
| High GPU usage idle | Check for `get_mut()` in hot paths |
| Dialog/overlay not showing | Use `EguiPrimaryContextPass` schedule |

---

## 4. Configuration Files
Settings are split into modular TOML files under `assets/settings/` (`core.toml`, `graphics.toml`, `runtime_assets.toml`, etc.). Shader presets are in `assets/defaults/shader_presets.toml`.

---

## 5. Build & Development
- `cargo run`: Run debug build.
- `cargo build --release`: Build optimized binary.
- `cargo clippy`: Run linter.
- **Hot-Reload**: Shaders reload automatically on save.

---

## 6. Performance Guidelines

### CRITICAL: Avoid `get_mut()` in Hot Paths
Calling `get_mut()` on Materials triggers re-extraction/re-binding every frame. Use `get()` for read-only checks.

### Configuration Loading Policy
**No Hidden Defaults**: All settings must be explicitly defined in TOML files. Avoid `.unwrap_or()` fallback patterns in Rust code.

---

## 7. Related Documentation
- **docs/TECHNICAL_REFERENCE.md**: Authoritative tech specs and constants.
- **GEMINI.md**: AI agent workflow and best practices.
- **docs/PROJECT_OVERVIEW.md**: High-level summary and status.
- **docs/CODE_OVERVIEW.md**: Detailed architecture details.
