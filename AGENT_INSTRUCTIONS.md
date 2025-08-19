# AI Agent Instructions for UODynamapper

This document provides essential information for an AI agent to effectively assist with the development of the UODynamapper project.

## 1. Project Overview

*   **Goal**: Create a 2D client for Ultima Online with a highly stylized and configurable terrain renderer that can emulate both classic and modern (Kingdom Reborn) visuals.
*   **Core Technologies**: Rust, Bevy Engine, WGSL shaders.
*   **Key Feature**: A sophisticated, uniform-driven terrain shader (`land_base.wgsl`) that supports multiple rendering presets.

## 2. Key Files & Directories

*   **Entry Point**: `dynamapper/src/main.rs` -> `dynamapper/src/core.rs` (main Bevy app setup).
*   **Terrain Shader**: `assets/shaders/worldmap/land_base.wgsl` (This is the primary file for visual changes).
*   **Shader Uniform Structs (Rust)**: `dynamapper/src/core/render/scene/world/land/mesh_material.rs` (MUST be kept in sync with the shader).
*   **Material/Uniform Creation**: `dynamapper/src/core/render/scene/world/land/draw_chunk_mesh.rs` (Where uniform data is populated from game data and passed to the material).
*   **Application State**: `dynamapper/src/core/app_states.rs` (Controls the application flow, e.g., loading vs. in-game).

## 3. Core Architectural Concepts

### 3.1. Rendering Presets & Philosophy

The terrain shader is designed around three main presets, controlled by a `const PRESET` in the shader:
*   **0: Classic 2D**: Faceted look, geometric normals, simple Gouraud (per-vertex) lighting.
*   **1: Enhanced Classic**: Smooth normals, per-fragment lighting, but with simpler effects like fill light.
*   **2: KR-like**: The full suite of effects: smooth/bent normals, per-fragment lighting, rim/specular highlights, procedural fog, color grading, and tonemapping.

When making changes, always consider how they will affect each preset.

### 3.2. The 8x8 -> 9x9 -> 13x13 Grid System

This is fundamental to the terrain renderer:
*   **8x8 Core**: The logical size of a game chunk.
*   **9x9 Vertices**: The mesh for a chunk is a 9x9 grid of vertices to ensure all 8x8 tiles have distinct corners.
*   **13x13 Tile Data**: The shader is passed a 13x13 grid of tile data in a uniform. This extra 2-tile border is crucial for high-quality **bicubic normal** calculations, preventing artifacts at chunk edges.

### 3.3. Uniform-Driven Shaders

Almost all visual features are controlled by uniforms, which are grouped by concern:
*   `LandUniform`: Per-chunk terrain data (heights, textures).
*   `SceneUniform`: Per-frame data (camera, light direction, time, fog).
*   `LightingUniforms`: Global lighting and color settings (fill, rim, grading, exposure).
*   `TunablesUniform`: Terrain-specific toggles and strength parameters.

**CRITICAL**: The layout of these structs in Rust (`mesh_material.rs`) must **exactly** match the shader structs in `land_base.wgsl`, including `std140` alignment and padding.

## 4. Agent Workflow: How to Approach Common Tasks

### Task: Modify a Visual Effect in the Shader

1.  **Identify the Target File**: The primary file is `assets/shaders/worldmap/land_base.wgsl`.
2.  **Locate the Logic**: Find the relevant section (e.g., `Lighting composition`, `KR-style multiplicative clouds/fog`).
3.  **Use Hot-Reload Toggles**: For quick iteration, use the `HOT_OVERRIDE_USE_UNIFORMS` and other `HOT_*` constants at the top of the shader to test changes without recompiling Rust code.
4.  **Consider Presets**: Ensure your changes do not break other rendering presets. The logic in the fragment shader uses the `shading_mode` and other variables to enable/disable features.

### Task: Add a New Uniform Parameter

This requires modifying both Rust and WGSL code in a specific order.

1.  **Step 1: Add to Rust Struct**: Add the new field to the appropriate uniform struct in `dynamapper/src/core/render/scene/world/land/mesh_material.rs`. **Pay close attention to `std140` alignment and add padding if necessary.**
2.  **Step 2: Populate the Uniform**: In `dynamapper/src/core/render/scene/world/land/draw_chunk_mesh.rs`, inside the `create_land_chunk_material` function, set the value for your new uniform field.
3.  **Step 3: Add to Shader Struct**: Add the corresponding field to the uniform struct in `assets/shaders/worldmap/land_base.wgsl`.
4.  **Step 4: Use the Uniform**: Use the new parameter in the shader's logic.

## 5. Common Pitfalls & Debugging

*   **WGPU Panic: `Binding is missing from the pipeline layout`**: This is a **binding index mismatch**. The `#[uniform(10X)]` attribute in `mesh_material.rs` does not match the `@binding(10X)` in the shader. This often happens when a uniform is added or removed. Carefully check that the binding indices are sequential and identical in both files.

*   **Colors are Washed Out / Whitish / Grayish**: This is almost always a **color space issue**.
    *   **Do NOT add manual gamma correction**. Bevy's rendering pipeline expects linear color output from the fragment shader and performs gamma correction itself. Adding `pow(color, 1.0/2.2)` will apply it twice and wash out the image.
    *   Check the tonemapping and fog calculations. An incorrect blend or exposure setting can desaturate or overly brighten the scene.

*   **Shader Fails to Compile**: Read the `wgpu` error message carefully. It will usually point to the exact line in the WGSL shader that has a syntax error. Remember that WGSL is more strict than GLSL in many ways (e.g., no implicit type conversions).
