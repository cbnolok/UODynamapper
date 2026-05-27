# AI Agent Instructions for UODynamapper

This document provides essential information for AI agents to effectively assist with the development of the UODynamapper project.

---

## 1. Project Summary

**Goal**: Create a 2D client/dynamic map renderer for Ultima Online with a highly configurable renderer that can emulate both classic and modern (Kingdom Reborn) visuals.

**Tech Stack**: Rust, Bevy Engine v0.18.1, WGSL (with naga-oil) shaders, wgpu.

**Workspace Members**:
- `dynamapper/` - Main application (Bevy app, rendering, UI, controls)
- `lib/uocf/` - Ultima Online Client Files 
- `lib/udd-conv/` - UODynamapper-specific converted asset packaging and runtime readers
- `lib/udd-conv-ktx2/` - KTX2 texture handling
- `tools/udd-conv-cli/` - CLI for building and inspecting UODynamapper-specific converted packages
- `tools/uocf-cli/` - Generic UO tooling CLI crate with multiple binaries:
  - `uoptool`: Utility for hashing, brute-force cracking, and rebuilding `.uop` files.
  - `cc-uop-mul-converter`: Converter for switching between legacy `.mul`/`.idx` and modern `.uop` formats.
  - `texture_scanner`: Utility for identifying and isolating land/terrain candidates from UO texture pools.
  - `uop-dict-populator-cli`: GUI-less tool for populating UOP hash dictionaries via templates or brute-force.
- `tools/uddp-inspector-gui/` - GUI for inspecting .uddp package contents
- `tools/udd-conv-gui/` - GUI frontend for asset conversion
- `tools/uop-inspector-gui/` - GUI for inspecting .uop files
- `tools/uop-dict-populator-gui/` - GUI for populating UOP hash dictionaries

---

## 2. Documentation Structure

| Document | Purpose |
|----------|---------|
| **GEMINI.md** (this file) | AI agent workflow, rules, and task-specific guidance |
| **docs/TECHNICAL_REFERENCE.md** | Authoritative tech specs, data formats, and constants |
| **docs/CONTRIBUTORS_GUIDE.md** | Quick reference for file locations and common tasks |
| **docs/PROJECT_OVERVIEW.md** | High-level project summary, goals, and status |
| **docs/CODE_OVERVIEW.md** | Low-level design choices and architecture details |

**When starting a task**:
1. Check **docs/TECHNICAL_REFERENCE.md** for technical specs.
2. Check `docs/CONTRIBUTORS_GUIDE.md` for file locations.
3. Read relevant sections in this file for workflow guidance.

**Note**: Asset paths in this document and others are relative to the workspace root. Shaders, settings, and defaults are under `dynamapper/assets/`.

---

## 3. Core Architectural Concepts (Summary)

For full technical specifications, data formats, and constants, see **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)**.

### 3.1 Rendering Presets
The renderer supports **Classic 2D (0)**, **Enhanced Classic (1)**, and **KR-like (2)** modes. Each affects normal generation and lighting complexity.

### 3.2 Paged Tile Metadata Atlas
Map metadata is stored in a layered `Rg16Uint` texture array (R16=ID, G16=Height/Size). This allows massive maps without material churn.

### 3.3 Modular Shader Architecture
Terrain shaders are in `dynamapper/assets/shaders/worldmap/land/` as WGSL modules using **naga_oil** `#import`. 
*Note: WGSL variable names cannot end with a digit (naga_oil constraint).*

### 3.4 Uniform Binding Protocol
Rust `#[uniform(10X)]` must match WGSL `@binding(10X)`. See `mesh_material.rs` and `bindings.wgsl` for current bindings.

### 3.5 EC Land Missing-Texture Policy

In EC/enhanced land mode, missing EC terrain textures must remain explicit. Do not hide unresolved EC land ids by silently falling back to Classic Client texmaps. Black/missing output is a useful failure signal while routing or provenance is being repaired.

Correct fixes should trace the affected land ids through `TerrainTranscode.kdl`, embedded `tex_land_ec.uddp` transcode metadata, `TerrainDefinition.uop` provenance, and EC terrain overrides. Add a CC fallback only as an explicitly named compatibility or diagnostic mode, never as the default regression fix.

---

## 4. Code Editing Rules

### General Rules

- **Comments**: Do **NOT** remove comments. Code must be thoroughly commented for people not well-versed in graphics programming.
- **No Magic Numbers**: Use `const` values in both Rust and WGSL code.
- **Preserve Structure**: Maintain existing code organization and naming conventions.

### Configuration Rules (TOML Settings)

- **No Default Values in Rust**: Never use `.unwrap_or()`, `.unwrap_or_default()`, or similar fallback patterns when loading TOML settings. Do NOT hardcode
 default values in Rust code.
- **Explicit Configuration**: All settings must be explicitly defined in TOML files. Hidden defaults in Rust code are difficult to discover and lead to configuration drift.
- **Fail Fast**: If a required setting is missing from the TOML file, the application should fail with a clear error message indicating which setting is missing.
- **Modular Keybindings**: All primary keybindings (F1-F3, Altitude, etc.) must be defined in `assets/settings/keybindings.toml` and accessed via `Settings.keybindings`. Do NOT hardcode KeyCodes for these actions.
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

1. **Identify Target**: Shader files are in `dynamapper/assets/shaders/worldmap/land/`
2. **Locate Logic**: Find relevant section in the appropriate `.wgsl` module
3. **Use Hot-Reload**: Test changes via F3 UI without recompiling Rust
4. **Verify Presets**: Check all three rendering modes (Classic, Enhanced, KR-like)

### Task: Add a New Uniform Parameter

**Follow this exact order**:

1. **Step 1: Rust Struct** - Add field to appropriate struct in `dynamapper/src/core/render/scene/world/land/mesh_material.rs`
2. **Step 2: Populate Uniform** - In `dynamapper/src/core/render/scene/world/land/draw_mesh.rs`, inside `create_land_chunk_material`, set the value
3. **Step 3: Shader Struct** - Add corresponding field in `dynamapper/assets/shaders/worldmap/land/bindings.wgsl`
4. **Step 4: Use in Shader** - Use the new parameter in the appropriate shader module
5. **Step 5: Verify Bindings** - Ensure binding indices match between Rust `#[uniform(10X)]` and WGSL `@binding(10X)`

### Task: Debug Common Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `Binding is missing from pipeline layout` | `#[uniform(10X)]` ≠ `@binding(10X)` | Verify indices match in Rust and WGSL files |
| Colors washed out / grayish | Double gamma correction | Remove manual `pow(color, 1.0/2.2)` - Bevy handles gamma |
| Shader compile error | WGSL syntax/alignment | Read wgpu error (points to exact line) |
| High GPU usage (70%+) idle | `get_mut()` in hot path | Use `get()` or `write_texture` to atlas |
| Dialog/overlay not showing | Wrong Bevy schedule | Use `EguiPrimaryContextPass`, not `Update` |

---

## 6. Performance & Memory Management (Summary)

Detailed specifications can be found in **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)**.

### 6.1 CRITICAL: Avoid `get_mut()` in Hot Paths
Calling `get_mut()` on Materials triggers expensive re-extraction and re-binding every frame. Use `get()` for read-checks.

### 6.2 Idle Eviction
MapBlocks and texture pixel data are evicted after **60 seconds** of inactivity.

### 6.3 BC7 Compression
Reduces VRAM usage ~8x. Align row bytes to 16-byte blocks.

---

## 7. Common Pitfalls & Debugging

### Color Space Issues
Avoid double gamma correction; Bevy handles gamma automatically.

### Texture Upload Alignment
- **BC7**: `(width + 3) / 4 * 16`
- **Rg16u**: `width * 4`

---

## 8. Testing & Verification

1. `cargo build` + `cargo clippy` + `cargo fmt`.
2. Test all three shader modes (**Classic**, **Enhanced**, **KR-like**).
3. Use **F3 UI** for real-time uniform testing.

---

## 9. When Searching Code

- Use **`glob`** for file patterns.
- Use **`grep_search`** for keywords.
- Use **`task`** agent for multi-round exploration.

---

## 10. Related Documentation

- **docs/TECHNICAL_REFERENCE.md**: Authoritative tech specs and constants.
- **docs/CONTRIBUTORS_GUIDE.md**: Quick file reference.
- **docs/PROJECT_OVERVIEW.md**: High-level status and goals.
- **docs/CODE_OVERVIEW.md**: Detailed architecture details.
- **docs/TODO.md**: Roadmap.
