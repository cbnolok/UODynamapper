# AI Agent Instructions for UODynamapper

This document provides essential information for AI agents to effectively assist with the development of the UODynamapper project.

---

## 1. Project Summary

**Goal**: Create a 2D client/dynamic map renderer for Ultima Online with a highly configurable renderer that can emulate both classic and modern (Kingdom Reborn) visuals.

**Tech Stack**: Rust, Bevy Engine v0.18.1, WGSL (with naga-oil) shaders, wgpu.

**Workspace Members**:
- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `uocf/` - Ultima Online file parser (map.mul, art.mul, tiledata.mul)

---

## 2. Documentation Structure

| Document | Purpose |
|----------|---------|
| **GEMINI.md** (this file) | AI agent workflow, rules, and task-specific guidance |
| **docs/CONTRIBUTORS_GUIDE.md** | Quick reference for file locations and common tasks |
| **docs/PROJECT_OVERVIEW.md** | High-level project summary, goals, and status |
| **docs/CODE_OVERVIEW.md** | Low-level design choices and architecture details |

**When starting a task**:
1. Check `docs/CONTRIBUTORS_GUIDE.md` for file locations
2. Read relevant sections in this file for workflow guidance
3. Consult `docs/CODE_OVERVIEW.md` for detailed architecture if needed

---

## 3. Core Architectural Concepts

### 3.1 Rendering Presets

The terrain shader supports three modes controlled by uniforms:

| Mode | Value | Normals | Shading | Features |
|------|-------|---------|---------|----------|
| **Classic 2D** | 0 | Geometric | Gouraud (vertex) | Lambert only |
| **Enhanced Classic** | 1 | Bicubic | Per-fragment | Diffuse + subtle fill |
| **KR-like** | 2 | Bicubic + Bent | Per-fragment | Full stack: rim/spec/fill + fog + grading + tonemap |

When making changes, always consider how they affect each preset.

### 3.2 Paged Tile Metadata Atlas

Map data is stored in UO mul/uop files as blocks (8x8 tiles each). Instead of per-chunk uniforms:

- **Atlas Format**: Layered `Rg16Uint` GPU texture array
  - **R16**: `tile_id` (0..65535)
  - **G16**: Packed `[height_biased:low 8 | tex_size:high 8]` where `height_biased = height + 128`
- **LRU Paging**: World pages (2048x2048 tiles) mapped to physical GPU layers
- **Incremental Updates**: Uploads via `queue.write_texture` on the fly
- **Neighborhood Sampling**: Shader samples across chunk boundaries seamlessly

**Key Files**:
- `dynamapper/src/core/render/scene/world/land/tile_atlas.rs` - LRU cache management
- `dynamapper/src/core/render/scene/world/land/draw_mesh.rs` - Mesh creation, atlas uploads
- `dynamapper/src/core/render/scene/world/land/mesh_material.rs` - Rust uniform structs
- `assets/shaders/worldmap/land/` - Modular WGSL shader directory (see Shader Modules below)

### 3.3 Modular Shader Architecture

The terrain shader is split into modular WGSL files composed at compile time via **naga_oil** `#import` directives:

| File | Purpose |
|------|---------|
| `main.wgsl` | Entry point: vertex/fragment functions, lighting composition, fog |
| `bindings.wgsl` | Struct definitions and `@group(3)` bind group declarations |
| `atlas.wgsl` | Tile metadata atlas lookups (page → layer → UV) |
| `sampling.wgsl` | Texture sampling (small/big atlas, filtering) |
| `normals.wgsl` | Normal generation (geometric, bicubic, bent) |
| `shading.wgsl` | Shading models (Gouraud, per-fragment, color grading) |
| `lighting.wgsl` | Light evaluation (Lambert, rim, specular, fill, tonemap) |
| `noise.wgsl` | Noise utilities |

**naga_oil Naming Constraint**: Variable and uniform names in WGSL **cannot end with a digit** (naga_oil limitation). Use a trailing underscore for padding fields (e.g., `_pad0_` instead of `_pad0`).

### 3.4 Uniform-Driven Shaders

Visual features are controlled by uniforms in shared bind groups, split into three uniform buffers for clear separation of concerns:

**Binding Indices** (MUST match between Rust and WGSL):
```rust
#[uniform(104)] pub atlas_params: AtlasParams           // Tile atlas paging params
#[uniform(105)] pub scene_uniform: SceneUniform          // Camera, light direction, zoom
#[uniform(106)] pub effects_uniform: LandEffectsUniform  // Texture/rendering toggles
#[uniform(107)] pub global_lighting_uniform: GlobalLightingUniforms  // Shared lighting (fog, tonemap, grading, gloom)
#[uniform(108)] pub land_lighting_uniform: LandLightingUniforms     // Land-specific lighting (bent, rim, fill, specular)
```

**WGSL variable names** (in `bindings.wgsl`):
```wgsl
@binding(106) var<uniform> effects:      LandEffectsUniform;
@binding(107) var<uniform> global_light:  GlobalLightingUniforms;
@binding(108) var<uniform> land_light:   LandLightingUniforms;
```

**Design Rationale**: `GlobalLightingUniforms` contains lighting parameters that apply to all geometry (fog, tonemap, color grading, ambient) and will be shared with future art/item shaders. `LandLightingUniforms` contains parameters specific to land terrain that require 3D normals (bent normals, rim light, specular, fill sky/ground).

**BC7 Compression**: When enabled in settings, terrain textures are compressed to BC7 on CPU using `intel_tex_2`, reducing VRAM usage ~8x (~160MB → ~20MB).

**CRITICAL**: Rust struct layout in `mesh_material.rs` must **exactly** match shader structs in `bindings.wgsl`, including `std140` alignment and padding.

---

## 4. Code Editing Rules

### General Rules

- **Comments**: Do **NOT** remove comments. Code must be thoroughly commented for people not well-versed in graphics programming.
- **No Magic Numbers**: Use `const` values in both Rust and WGSL code.
- **Preserve Structure**: Maintain existing code organization and naming conventions.

### Configuration Rules (TOML Settings)

- **No Default Values in Rust**: Never use `.unwrap_or()`, `.unwrap_or_default()`, or similar fallback patterns when loading TOML settings.
- **Explicit Configuration**: All settings must be explicitly defined in TOML files. Hidden defaults in Rust code are difficult to discover and lead to configuration drift.
- **Fail Fast**: If a required setting is missing from the TOML file, the application should fail with a clear error message indicating which setting is missing.
- **Example Pattern**:
  ```rust
  // WRONG - hidden default
  let value = settings.get("my_setting").unwrap_or(42);

  // CORRECT - explicit, fails if missing
  let value = settings.get("my_setting")
      .ok_or_else(|| ConfigError::Missing("my_setting".to_string()))?;
  ```

### Bevy-Specific Rules

- **Egui Rendering**: In Bevy 0.18 with `bevy_egui` 0.39, all egui rendering systems MUST run in `EguiPrimaryContextPass` schedule, NOT in `Update` or `PostUpdate`.
- **Change Detection**: Avoid `get_mut()` in hot paths (see Performance section below).

### 4.3 Plugin Rules

- **Plugin Tracking**: ALL internal plugins MUST implement the `TrackedPlugin` trait.
    - Add `pub registered_by: &'static str` to the plugin struct.
    - Call `log_plugin_build(self)` at the start of `build()`.
- **External Plugins**: Manually record third-party plugins in the registry (e.g. in `core.rs`) so they appear in the startup graph.

---

## 5. Agent Workflow: How to Approach Tasks

### General Workflow

1. **Understand the Request**: Clarify ambiguous requirements before starting
2. **Locate Relevant Files**: Use `docs/CONTRIBUTORS_GUIDE.md` for quick reference
3. **Read Existing Code**: Understand current implementation before modifying
4. **Make Changes**: Follow established patterns and conventions
5. **Verify**: Run `cargo build` and `cargo clippy`
6. **Test**: If visual changes, test all three shader presets (Classic, Enhanced, KR-like)
7. **Confirm**: Ask user if solution works as expected
8. **Update Docs**: Ask if `docs/CONTRIBUTORS_GUIDE.md` or `docs/CODE_OVERVIEW.md` need updates

### Task: Modify a Visual Effect in the Shader

1. **Identify Target**: Shader files are in `assets/shaders/worldmap/land/` (see section 3.3 for module list)
   - Lighting/shading logic → `shading.wgsl` and `lighting.wgsl`
   - Texture sampling → `sampling.wgsl`
   - Normal generation → `normals.wgsl`
   - Fragment entry point / fog → `main.wgsl`
   - Struct definitions / bindings → `bindings.wgsl`
2. **Locate Logic**: Find relevant section in the appropriate module
3. **Use Hot-Reload**: Test changes via F3 UI without recompiling Rust
4. **Verify Presets**: Check all three rendering modes

### Task: Add a New Uniform Parameter

**Follow this exact order**:

1. **Step 1: Rust Struct** - Add field to appropriate struct in `mesh_material.rs`
   - Choose correct struct: `LandEffectsUniform` (texture/rendering), `GlobalLightingUniforms` (shared lighting), or `LandLightingUniforms` (land-specific lighting)
   - Ensure `#[repr(C, align(16))]` and `ShaderType` derive
   - Add padding fields as needed for `std140` alignment
   - Padding field names must NOT end with a digit (naga_oil constraint)

2. **Step 2: Populate Uniform** - In `draw_mesh.rs`, inside `create_land_chunk_material`, set the value

3. **Step 3: Shader Struct** - Add corresponding field in `bindings.wgsl`

4. **Step 4: Use in Shader** - Use the new parameter in the appropriate shader module

5. **Step 5: Verify Bindings** - Ensure binding indices match between Rust and WGSL

### Task: Debug Common Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `Binding is missing from pipeline layout` | `#[uniform(10X)]` ≠ `@binding(10X)` | Verify binding indices are identical in `mesh_material.rs` and `bindings.wgsl` |
| Colors washed out / grayish | Double gamma correction | Remove manual `pow(color, 1.0/2.2)` - Bevy handles gamma |
| Shader compile error | WGSL syntax/alignment | Read wgpu error (points to exact line) |
| High GPU usage (70%+) idle | `get_mut()` in hot path | Use `get()` or `write_texture` to atlas |
| Dialog/overlay not showing | Wrong Bevy schedule | Use `EguiPrimaryContextPass`, not `Update` |

---

## 6. Performance & Memory Management

### 6.1 CRITICAL: Avoid `get_mut()` in Hot Paths

**The Problem**:
Calling `get_mut()` on Materials or Assets inside Update systems triggers Bevy's change detection. For large assets like terrain materials (binding global texture arrays), this forces expensive **re-extraction** (copying to Render World) and **re-binding** (updating GPU bind groups) every frame.

**Result**: 70%+ GPU usage even when idle, micro-stutters, UI changes overwritten.

**Solutions**:
- Use `get()` for read-only checks
- Only use `get_mut()` if comparison (using `Local` or `is_changed()`) proves data changed
- For terrain metadata: use **direct GPU write** via `write_texture` to Tile Atlas

### 6.2 Idle Eviction (60s)

- **What**: MapBlocks in `MapPlane`, texture pixel data in `TexMap2D`
- **When**: Not accessed for 60 seconds
- **Check**: Every 5 seconds via `sys_evict_map_blocks`

### 6.3 BC7 Compression & VRAM

- **Savings**: ~160MB → ~20MB (~8x reduction)
- **Library**: `intel_tex_2` with `alpha_basic_settings`
- **Alignment**: `bytes_per_row = (width + 3) / 4 * 16` (4x4 blocks)

### 6.4 Build Optimizations

- **Linker**: `mold` (3-5x faster linking, CI only)
- **Release**: `opt-level = "s"` + LTO + strip
- **Debug**: `opt-level = 1`, incremental enabled

---

## 7. Common Pitfalls & Debugging

### Color Space Issues

**Symptom**: Colors appear washed out, whitish, or grayish.

**Cause**: Double gamma correction. Bevy's rendering pipeline expects linear color output and performs gamma correction itself.

**Fix**: Do NOT add manual `pow(color, 1.0/2.2)`. Output linear color from fragment shader.

### Texture Upload Alignment

**BC7**: `bytes_per_row = (width + 3) / 4 * 16` (16 bytes per 4x4 block)

**Rg16u**: `bytes_per_row = width * 4` (4 bytes per texel)

### WGSL Strictness

WGSL is more strict than GLSL:
- No implicit type conversions
- Explicit padding for `std140` alignment
- Error messages point to exact line

---

## 8. Testing & Verification

### Before Committing Changes

1. **Build**: `cargo build` - ensure no compile errors
2. **Lint**: `cargo clippy` - fix any warnings
3. **Format**: `cargo fmt` - ensure consistent formatting
4. **Test Presets**: Verify all three shader modes work correctly
5. **Check Performance**: Ensure no new GPU overhead introduced

### Visual Testing Workflow

1. Use F3 dialog to toggle between Classic/Enhanced/KR modes
2. Adjust lighting/fog/grading sliders in real-time
3. Verify changes look correct in all three modes
4. Check performance overlay (top-right) for GPU impact

---

## 9. When Searching Code

### Recommended Tools

- **`glob`**: Find files by pattern (e.g., `**/*.wgsl`)
- **`grep_search`**: Search for keywords in file contents
- **`task` agent**: Complex multi-file searches requiring multiple rounds

### Search Strategy

1. Start with specific file patterns if you know the location
2. Use keyword search for concepts (e.g., "tile_atlas", "uniform")
3. For open-ended exploration, use the task agent

---

## 10. Related Documentation

- **docs/CONTRIBUTORS_GUIDE.md**: Quick file reference and common workflows
- **docs/PROJECT_OVERVIEW.md**: High-level project summary and status
- **docs/CODE_OVERVIEW.md**: Detailed architecture and design choices
- **docs/TODO.md**: Planned features and improvements
