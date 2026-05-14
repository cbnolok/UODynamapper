# Visual Atmosphere Implementation Plan: KR vs. EC

This document outlines a comprehensive plan for implementing the distinct visual atmospheres of both Kingdom Reborn (KR) and the Enhanced Client (EC) within the Rust/Bevy-based `UODynamapper` engine.

By breaking the visual styles down into specific "atmospheric components," we can build a modular rendering pipeline that allows users to seamlessly toggle between the bright, nostalgic EC look and the dark, gloomy, painterly KR look.

---

## 1. Tonemapping and Exposure (The Post-Processing Pipeline)

The core driver of the overall mood in both clients is the screen-space post-processing pipeline, specifically the Reinhard Tonemapper. 

### The Difference
*   **KR Approach (The Gloom):** KR forcefully drives the Reinhard exposure curve down using static, aggressive constants (`Luminance = 2.0`, `MiddleGray = 0.18`). This crushes ambient light by ~90%, forcing the scene into deep shadows and leaving only bright spots to survive the clipping shoulder (`WhiteCutoff = 0.8`).
*   **EC Approach (The Brightness):** While EC uses the exact same shader binary (`F6`), the engine either disables the tonemap pass entirely or overrides the `Luminance` uniform at runtime to a much lower value (e.g., `0.2`), mapping the HDR colors 1:1 to the monitor without crushing them.

### Implementation Steps (UODynamapper)
1.  **Integrate Post-Processing:** Add a Bevy custom `PostProcessPass` using the translated `kr_bloom_pipeline.wgsl`.
2.  **Expose Uniforms:** Create a `PostProcessSettings` uniform buffer that controls the constants.
3.  **Toggle Settings:**
    *   *KR Preset:* Set `luminance = 2.0`, `middle_gray = 0.18`, `white_cutoff = 0.8`, `bloom_scale = 2.0`.
    *   *EC Preset:* Bypass tonemapping entirely, or set `luminance = 0.18` (neutral exposure) and lower `bloom_scale`.

---

## 2. The Grunge Pass (Ambient Weathering)

To make repeating tiles look organic and "painterly," the engine multiplies surfaces by a tiling noise texture (`noise.tga`).

### The Difference
*   **KR Approach:** Heavily relies on the grunge pass to break up its high-frequency, realistic textures. It acts as fake ambient occlusion, creating arbitrary dark and light patches that simulate dirt and weathering.
*   **EC Approach:** Uses grunge much more sparingly, relying on the inherently brighter, cleaner textures to carry the visual weight.

### Implementation Steps (UODynamapper)
1.  **Asset Loading:** Load `noise.tga` (or the equivalent DDS) as a global `Image` resource.
2.  **Shader Binding:** Bind the noise texture to `sampling.wgsl` and `mesh_material.wgsl`.
3.  **World-Space Multiplication:** In the fragment shader, sample the noise texture using world-space UVs (e.g., `world_pos.xz * 0.05`) so it spans seamlessly across multiple terrain tiles.
4.  **Application:** 
    *   `out_color.rgb *= mix(vec3(1.0), noise_sample.rgb, grunge_strength);`
    *   Expose `grunge_strength` as a uniform (1.0 for KR, 0.0-0.2 for EC).

---

## 3. Dynamic Lighting (Additive Decals)

Because full 3D normal-mapped point lights are expensive, both clients use 2D glowing sprites (Light Decals) rendered over the terrain to simulate local illumination from torches, spells, and windows.

### The Difference
*   **KR Approach:** Because the base Tonemapper crushes the ambient brightness, these additive light decals "pop" intensely. A soft yellow decal drawn over dark, grunged cobblestone creates a dramatic, painterly lighting illusion.
*   **EC Approach:** Because the base scene is already bright, the additive decals are less noticeable and look more like flat color overlays rather than true illumination.

### Implementation Steps (UODynamapper)
1.  **Light Extraction:** Use the `world_lights.uddp` package to spawn light entities at their correct world coordinates.
2.  **Billboard System:** Instead of standard Bevy `PointLight` components (which are true 3D lights), spawn 2D Quads (billboards) flat against the terrain.
3.  **Material Setup:** 
    *   Assign the extracted light PNGs (starburst, soft radial, directional beams) to the quads.
    *   Set the material `blend_mode` to **Additive** (`BlendState::ADDITIVE`).
    *   Multiply the texture by the light's designated color/hue.
    *   *Crucial:* Ensure these are drawn *before* the post-processing tonemap pass, so their bright additive values feed into the bloom threshold.

---

## 4. Normal Mapping (Terrain Depth)

We have confirmed that the assets contain tangent-space normal maps (the purplish-blue textures) for surfaces like water and terrain.

### The Difference
*   **KR Approach:** The normal maps react to a global directional light (the sun), creating self-shadowing in the crevices of the terrain and wave ripples on the water.
*   **EC Approach:** Normal maps were largely discarded or ignored to improve performance and flatten the look back to the classic 2D aesthetic.

### Implementation Steps (UODynamapper)
1.  **Atlas Expansion:** The current tile atlas (`tex_land_ec_page_atlas`) only holds the Albedo (color) map. It must be expanded to a struct or multiple arrays to hold the corresponding Normal map pages.
2.  **Shader Update (`sampling.wgsl`):** Create a `sample_tile_normal()` function that fetches from the normal atlas.
3.  **Directional Lighting:** In the main fragment shader, calculate basic N dot L lighting:
    ```wgsl
    let normal_sample = sample_tile_normal(uv);
    let world_normal = normalize(normal_sample.xyz * 2.0 - 1.0); // Convert from [0,1] to [-1,1]
    let sun_dir = normalize(vec3<f32>(0.5, 1.0, 0.3)); // Global sun angle
    let light_intensity = max(dot(world_normal, sun_dir), 0.2); // 0.2 is ambient baseline
    base_albedo *= light_intensity;
    ```
4.  **Toggle:** Expose an `enable_normal_mapping` flag in the terrain uniforms to disable this entirely for the EC preset.

---

## Summary of the "Knobs" required in UODynamapper

To allow the user to transition between EC and KR visually, the `LandEffectsUniform` and global `SceneSettings` must expose:

1.  **`post_process_profile`**: Enum `[Disabled, EC, KR]`. Drives the uniform constants sent to `kr_bloom_pipeline.wgsl`.
2.  **`grunge_strength`**: Float `[0.0 - 1.0]`. Controls the opacity of the `noise.tga` multiplication on terrain/statics.
3.  **`enable_normal_maps`**: Boolean. Toggles whether the directional lighting pass is calculated in the fragment shader.
4.  **`light_decal_intensity`**: Float. Scales the alpha multiplier of the additive 2D light quads.
