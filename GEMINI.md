# AI Agent Instructions for UODynamapper

This document provides essential information for an AI agent to effectively assist with the development of the UODynamapper project.

## 1. Project Overview

* **Goal**: Create a 2D client/dynamic map renderer for Ultima Online with a highly configurable renderer that can emulate both classic and modern (Kingdom Reborn) visuals.
* **Core Technologies**: Rust, Bevy Engine v0.16+, WGSL (WESL + naga-oil) shaders.

## 2. Key Files & Directories

* **Entry Point**: `dynamapper/src/main.rs` -> `dynamapper/src/core.rs` (main Bevy app setup).
* **Terrain Shaders**: in the folder `assets/shaders/worldmap/`.
* **Shader Uniform Structs (Rust)**: `dynamapper/src/core/render/scene/world/land/mesh_material.rs` (MUST be kept in sync with the shader).
* **Material/Uniform Creation**: `dynamapper/src/core/render/scene/world/land/draw_chunk_mesh.rs` (Where uniform data is populated from game data and passed to the material).
* **Application State**: `dynamapper/src/core/app_states.rs` (Controls the application flow, e.g., loading vs. in-game).
* **More in-depth code architecture and workflow** in CODE_OVERVIEW.md.

## 3. Core Architectural Concepts

### 3.1. Rendering Presets & Philosophy

The terrain shader is designed around three main presets:

* **0: Classic 2D**: Faceted look, geometric normals, simple Gouraud (per-vertex) lighting.
* **1: Enhanced Classic**: Smooth normals, per-fragment lighting, but with simpler effects like fill light.
* **2: KR-like**: The full suite of effects: smooth/bent normals, per-fragment lighting, rim/specular highlights, procedural fog, color grading, and tonemapping.

When making changes, always consider how they will affect each preset.

### 3.2 Terrain rendering

#### The Paged Tile Metadata Atlas

Map data is stored in UO mul/uop files as a collection of blocks in row-major order. Each block has 8x8 tiles.

* **8x8 Core**: The logical size of a game chunk.
* **Paged Atlas**: Instead of per-chunk uniforms, terrain metadata is stored in a layered **Rg16Uint** GPU texture array.
    * **R16**: `tile_id` (0..65535).
    * **G16**: Packed `[height:low 8 | tex_size:high 8]`. `height` is biased by +128.
* **LRU Paging**: World pages (e.g. 2048x2048 tiles) are dynamically mapped to physical layers in the GPU atlas.
* **Incremental Updates**: Chunks are uploaded to the atlas using `queue.write_texture` on the fly.
* **Neighborhood Sampling**: The shader samples across chunk boundaries by resolving global world coordinates into atlas page/layer coordinates, enabling high-quality bicubic normal calculations without per-chunk padding.

### 3.3. Uniform-Driven Shaders

Visual features are controlled by uniforms grouped by concern:

* `LandUniform`: Per-chunk instanced data (coordinates, atlas layer).
* `SceneUniform`, `LightingUniforms`, `EffectsUniform`: Global/Shared uniforms moved to a shared bind group to reduce material churn.
* `AtlasParams`: Metadata about the Paged Tile Atlas (page size, layer mappings).

**BC7 Compression**: When `lossy_texture_compression` is enabled in `settings.toml`, terrain textures are compressed to **BC7** on the CPU using `intel_tex_2` before being uploaded to VRAM, reducing texture array memory usage by ~8x (~160MB to ~20MB).

**CRITICAL**: The layout of these structs in Rust (`mesh_material.rs`) must **exactly** match the shader structs in `land_base.wgsl`, including `std140` alignment and padding.

## 4. General Code Editing Rules

* **Comments**: Do NOT remove comments if not expressly and precisely told to. The code has to be thoroughly commented with the aim of it being easily readable and understandable
    also from people not well versed in graphics programming.
* NO **Magic Numbers**: Use 'const' values whenever possible, both in Rust and in Wgsl code.

## 5. Agent Workflow: How to Approach Common Tasks

General rules:

* When done with a task, ask for confirmation (does the agent solution do or work as the user meant? is it was the user expected?).
    If affirmative, ask if you should update CODE_OVERVIEW.md: that file has to always be kept in sync.

### Task: Modify a Visual Effect in the Shader

1. **Identify the Target File**: The primary file is `assets/shaders/worldmap/land_base.wgsl`.
2. **Locate the Logic**: Find the relevant section (e.g., `Lighting composition`, `KR-style multiplicative clouds/fog`).
3. **Use Hot-Reload Toggles**: For quick iteration, uniforms are modifiable from a ui `dynamapper/src/core/render/terrain_shader_ui.rs` at the top of the shader to test changes without recompiling Rust code.

### Task: Add a New Uniform Parameter

This requires modifying both Rust and WGSL code in a specific order.

1. **Step 1: Add to Rust Struct**: Add the new field to the appropriate uniform struct in `dynamapper/src/core/render/scene/world/land/mesh_material.rs`. **Pay close attention to `std140` alignment and add padding if necessary.**
2. **Step 2: Populate the Uniform**: In `dynamapper/src/core/render/scene/world/land/draw_chunk_mesh.rs`, inside the `create_land_chunk_material` function, set the value for your new uniform field.
3. **Step 3: Add to Shader Struct**: Add the corresponding field to the uniform struct in `assets/shaders/worldmap/land_base.wgsl`.
4. **Step 4: Use the Uniform**: Use the new parameter in the shader's logic.

## 6. Common Pitfalls & Debugging

* **WGPU Panic: `Binding is missing from the pipeline layout`**: This is a **binding index mismatch**. The `#[uniform(10X)]` attribute in `mesh_material.rs` does not match the `@binding(10X)` in the shader. This often happens when a uniform is added or removed. Carefully check that the binding indices are sequential and identical in both files.

* **Colors are Washed Out / Whitish / Grayish**: This is almost always a **color space issue**.
  * **Do NOT add manual gamma correction**. Bevy's rendering pipeline expects linear color output from the fragment shader and performs gamma correction itself. Adding `pow(color, 1.0/2.2)` will apply it twice and wash out the image.
  * Check the tonemapping and fog calculations. An incorrect blend or exposure setting can desaturate or overly brighten the scene.

* **Shader Fails to Compile**: Read the `wgpu` error message carefully. It will usually point to the exact line in the WGSL shader that has a syntax error. Remember that WGSL is more strict than GLSL in many ways (e.g., no implicit type conversions).

## 7. Performance & Memory Management

### 7.1. Idle Eviction (60s)
To keep the RAM footprint low, the following data is evicted if not accessed for 60 seconds. The check is performed by a dedicated system every 5 seconds:
* **MapBlocks**: Cached blocks in `MapPlane`.
* **Land Textures**: Pixel data in `TexMap2D` (lazy-loaded on demand).

### 7.2. BC7 Compression & VRAM
* **VRAM Savings**: Reduces texture array usage from ~160MB to ~20MB.
* **Format**: Uses `intel_tex_2` with `alpha_basic_settings`.
* **Alignment**: GPU uploads must respect the 4x4 pixel block size (16 bytes per block) for $bytes\_per\_row$ calculations.

### 7.3. Build Optimizations
* **Linker**: Release builds use the `mold` linker (via GitHub Actions) for 3-5x faster link times.
* **Binary Size**: Production builds (`profile.release`) use `opt-level = "s"` + LTO + stripping to minimize executable weight.

### 7.4. Performance Pitfall: get_mut()
> [!IMPORTANT]
> **NEVER use `get_mut()` on Materials or Assets inside hot loops (like `Update` systems) unless you have confirmed the data *actually* changed.**
> 
> * **The Pitfall**: Calling `get_mut()` triggers Bevy's change detection. For large assets like terrain materials (which bind global texture arrays), this forces the renderer to perform an expensive **re-extraction** (copying data to the Render World) and **re-binding** (updating GPU bind groups) of the entire material every single frame.
> * **The Result**: High GPU usage (70%+) even when idle, micro-stutters, and sabotaging of UI-driven uniform updates (as defaults overwrite UI changes every frame).
> * **The Workaround (direct GPU write)**: To update terrain metadata without material overhead, we use the **Paged Tile Metadata Atlas** (Section 3.2). Instead of modifying a material uniform, we use `render_queue.write_texture` to upload only the changed texels directly to a GPU texture. The shader then samples from this texture. This bypasses Bevy's material mutation tracking entirely.
> * **Solution**: Use `get()` for read-only checks. Only use `get_mut()` if a comparison (using a `Local` or `is_changed()`) proves that a uniform update is strictly necessary.

