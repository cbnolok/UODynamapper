# Visual Atmosphere Implementation Plan: KR vs. EC

This document outlines a comprehensive plan for implementing the distinct visual atmospheres of both Kingdom Reborn (KR) and the Enhanced Client (EC) within the Rust/Bevy-based `UODynamapper` engine.

By breaking the visual styles down into specific "atmospheric components," we can build a modular rendering pipeline that allows users to seamlessly toggle between the bright, nostalgic EC look and the dark, gloomy, painterly KR look.

---

## Current Implementation Status

### Implemented
1.  **Shared visual profile uniform**
    *   `LandEffectsUniform.post_process_profile` exists.
    *   Values:
        *   `0 = Neutral`
        *   `1 = Enhanced Client (EC)`
        *   `2 = Kingdom Reborn (KR)`
    *   The same profile is pushed to land, art sprites, and ground art.

2.  **Shared tonemapping helpers**
    *   `dynamapper/assets/shaders/postprocess/tonemapping.wgsl` contains shared tonemap logic.
    *   Land and art shaders call the same EC/KR-aware tone curve.
    *   EC currently uses a neutral/lower-luminance profile.
    *   KR currently uses the KR-like constants: `luminance = 2.0`, `middle_gray = 0.18`, `white_cutoff = 0.8`.

3.  **Shared global lighting helper**
    *   `dynamapper/assets/shaders/postprocess/global_lighting.wgsl` contains scene-wide lighting scale logic.
    *   Land and art shaders use this shared helper.

4.  **Shared color grading helper**
    *   `dynamapper/assets/shaders/postprocess/color_grading.wgsl` contains the shared vibrant grading logic.
    *   Land and art shaders use the same grading path before tonemapping.

5.  **Texture-backed grunge/weathering fallback**
    *   `dynamapper/assets/shaders/postprocess/grunge.wgsl` contains procedural world-space grunge.
    *   Runtime land/art materials bind a generated repeatable grunge texture and blend it with the procedural fallback.
    *   The effect applies to land, art sprites, and ground art.
    *   `enable_grunge` and `grunge_strength` are exposed in `LandEffectsUniform`.
    *   Presets use:
        *   Classic: grunge off.
        *   EC: subtle grunge.
        *   KR: stronger grunge.

6.  **UI controls**
    *   Terrain shader UI exposes:
        *   Visual Profile: `Neutral / EC / KR`
        *   Grunge / Weathering
        *   Grunge Strength
    *   `light_decal_intensity` scales static-light decal and local-light response.
    *   `enable_normal_maps` toggles EC land role-3 normal-map sampling.

7.  **Preset defaults**
    *   `dynamapper/assets/defaults/shader_presets.toml` has profile/grunge defaults for Classic, EC, and KR slots.

8.  **Material-family surface response**
    *   KR land shading separates water, lava-like liquids, swamp-like liquids, and ice/snow-like bright surfaces using existing wet/reviewed-liquid flags plus conservative albedo heuristics.
    *   Remaining work: replace heuristics with metadata-backed material-family classification when the terrain evidence is strong enough.

9.  **Static light color response**
    *   Static light collection derives a local-response color from known ClassicUO light shader ids and falls back to warm light otherwise.
    *   Land and art local-light response consume that color instead of a single fixed warm tint.
    *   Remaining work: package-hue colors and representative sampled-mask colors for light sources without a ClassicUO shader mapping.

10. **KR cliff/high-bank compression**
    *   KR land shading adds a neighbor-height shadow term for steep local height transitions, controlled by `kr_land_relief_shadow_strength`.
    *   Remaining work: validate against cliff-heavy KR screenshots and tune per terrain family if metadata becomes available.

11. **KR art side falloff**
    *   Art fake-normal shading adds a subtle one-sided depth tint for foliage, roofs, and tall non-ground statics.
    *   Remaining work: validate against KR tree, wall, and large-static screenshots and tune depth-class profiles if needed.

### Still Missing
1.  **Real fullscreen EC/KR post-process pipeline**
    *   Current tonemapping is still applied in material shaders, not as a single fullscreen post-process after the whole scene is composed.
    *   Bloom, bright-pass extraction, downscale, blur, and final additive bloom composite are not implemented in the Bevy render graph yet.
    *   Reference shaders remain in:
        *   `dynamapper/assets/shaders/postprocess_ec_doc/`
        *   `dynamapper/assets/shaders/postprocess_kr_doc/`

2.  **Official texture-backed grunge provenance**
    *   Current grunge uses a generated repeatable texture blended with procedural variation.
    *   KR/EC-style grunge texture loading from `noise.tga` or equivalent DDS is not implemented because the official source texture has not been identified in the packaged asset path.
    *   A future implementation should replace the generated fallback image with the verified support texture while preserving the same shader bindings and procedural fallback.

3.  **Dynamic additive light decals**
    *   Static light-source tiles now collect visible light masks and local-light influence.
    *   `light_decal_intensity` scales the visible decal material and land/art light response.
    *   Remaining work: support mobile/spell/temporary lights, stronger color provenance, and ordering guarantees before fullscreen tonemap/bloom.

4.  **Texture normal maps**
    *   `enable_normal_maps` toggles EC land role-3 texture normals.
    *   The runtime samples the existing `tex_land_ec` page atlas; source DDS files are decoded/repacked into UDDP RGBA8888 or BC7/BC7-RDO payloads before shader sampling.
    *   Remaining work: validate channel orientation, add strength tuning if needed, and decide whether any art/static classes should consume texture normals.

5.  **Post-process settings resource**
    *   There is not yet a dedicated `PostProcessSettings` resource/uniform for fullscreen profile constants.
    *   The current profile is carried through `LandEffectsUniform` because the active implementation runs in land/art material shaders.

---

## 1. Tonemapping and Exposure (The Post-Processing Pipeline)

The core driver of the overall mood in both clients is the screen-space post-processing pipeline, specifically the Reinhard Tonemapper. 

### The Difference
*   **KR Approach (The Gloom):** KR forcefully drives the Reinhard exposure curve down using static, aggressive constants (`Luminance = 2.0`, `MiddleGray = 0.18`). This crushes ambient light by ~90%, forcing the scene into deep shadows and leaving only bright spots to survive the clipping shoulder (`WhiteCutoff = 0.8`).
*   **EC Approach (The Brightness):** While EC uses the exact same shader binary (`F6`), the engine either disables the tonemap pass entirely or overrides the `Luminance` uniform at runtime to a much lower value (e.g., `0.2`), mapping the HDR colors 1:1 to the monitor without crushing them.

### Implementation Steps (UODynamapper)
1.  **DONE (material path):** Add shared EC/KR-aware tonemapping helpers in `postprocess/tonemapping.wgsl`.
2.  **DONE (material path):** Expose `post_process_profile` through the shared effects uniform and presets.
3.  **TODO (fullscreen path):** Add a Bevy custom `PostProcessPass` using the translated `kr_bloom_pipeline.wgsl`.
4.  **TODO (fullscreen path):** Create a `PostProcessSettings` uniform buffer that controls the constants independently of material uniforms.
5.  **TODO (fullscreen path):** Toggle settings:
    *   *KR Preset:* Set `luminance = 2.0`, `middle_gray = 0.18`, `white_cutoff = 0.8`, `bloom_scale = 2.0`.
    *   *EC Preset:* Bypass tonemapping entirely, or set `luminance = 0.18` (neutral exposure) and lower `bloom_scale`.

---

## 2. The Grunge Pass (Ambient Weathering)

To make repeating tiles look organic and "painterly," the engine multiplies surfaces by a tiling noise texture (`noise.tga`).

### The Difference
*   **KR Approach:** Heavily relies on the grunge pass to break up its high-frequency, realistic textures. It acts as fake ambient occlusion, creating arbitrary dark and light patches that simulate dirt and weathering.
*   **EC Approach:** Uses grunge much more sparingly, relying on the inherently brighter, cleaner textures to carry the visual weight.

### Implementation Steps (UODynamapper)
1.  **DONE (procedural fallback):** Add procedural world-space grunge in `postprocess/grunge.wgsl`.
2.  **DONE:** Apply grunge to land, art sprites, and ground art.
3.  **DONE:** Expose `enable_grunge` and `grunge_strength`.
4.  **DONE (generated fallback):** Create a repeatable grunge `Image` at runtime and bind it to land, art sprite, and ground-art shader materials.
5.  **DONE (generated fallback):** Sample the texture with world-space UVs and blend it with procedural variation to avoid uniform single-scale noise.
6.  **TODO (official KR/EC parity):** Identify and load `noise.tga` or the equivalent DDS support texture, replacing the generated image without changing the shader binding model.
7.  **Target application:** 
    *   `out_color.rgb *= mix(vec3(1.0), noise_sample.rgb, grunge_strength);`
    *   Expose `grunge_strength` as a uniform (1.0 for KR, 0.0-0.2 for EC).

---

## 3. Dynamic Lighting (Additive Decals)

Because full 3D normal-mapped point lights are expensive, both clients use 2D glowing sprites (Light Decals) rendered over the terrain to simulate local illumination from torches, spells, and windows.

### The Difference
*   **KR Approach:** Because the base Tonemapper crushes the ambient brightness, these additive light decals "pop" intensely. A soft yellow decal drawn over dark, grunged cobblestone creates a dramatic, painterly lighting illusion.
*   **EC Approach:** Because the base scene is already bright, the additive decals are less noticeable and look more like flat color overlays rather than true illumination.

### Implementation Steps (UODynamapper)
1.  **TODO:** Use the `world_lights.uddp` package to spawn light entities at their correct world coordinates.
2.  **TODO:** Instead of standard Bevy `PointLight` components (which are true 3D lights), spawn 2D quads/billboards flat against or slightly above the terrain.
3.  **TODO:** Add a dedicated light decal material and draw path.
4.  **TODO:** Feed `light_decal_intensity` into that material once the draw path exists.
5.  **TODO:** Material setup:
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
1.  **Current:** EC land normal maps are carried as material role `3` in the existing `tex_land_ec` page atlas and lookup table. No separate normal-map binding is required for the active path.
2.  **Current:** `sampling.wgsl` exposes EC role-based normal sampling and `main.wgsl` blends the decoded texture normal into the geometric/bicubic terrain normal when `enable_normal_maps` is enabled. Liquid materials animate the sampled normal using the same role-3 scroll convention used by liquid UV perturbation.
3.  **Format:** Known KR normal-map inputs can be ordinary DXT1 RGB DDS files, not BC5/ATI2. Decode stored RGB from `[0,1]` to `[-1,1]`; use blue as up, red as tangent X, and green as tangent Z unless visual validation proves a channel flip is needed.
4.  **Validity:** Role-3 samples with an implausibly low blue/up channel are rejected so unresolved or misrouted non-normal textures do not corrupt terrain lighting.
5.  **Current scope:** The first implementation is EC land only and excludes Classic shading mode. Art/static normal maps are not active yet.
6.  **TODO:** Validate channel orientation on water, stone, snow, and grass material families using side-by-side screenshots with `enable_normal_maps` toggled.
7.  **Current:** `land_normal_map_strength` controls the base EC land texture-normal blend. Liquid normal response derives from the same strength with a bounded boost.
8.  **TODO:** Validate useful strength ranges per material family and decide whether channel-flip diagnostics are needed.
9.  **Target directional lighting:** In the main fragment shader, calculate basic N dot L lighting:
    ```wgsl
    let normal_sample = sample_tile_normal(uv);
    let world_normal = normalize(normal_sample.xyz * 2.0 - 1.0); // Convert from [0,1] to [-1,1]
    let sun_dir = normalize(vec3<f32>(0.5, 1.0, 0.3)); // Global sun angle
    let light_intensity = max(dot(world_normal, sun_dir), 0.2); // 0.2 is ambient baseline
    base_albedo *= light_intensity;
    ```
7.  **DONE (reserved only):** Expose an `enable_normal_maps` field in the shared effects uniform.
8.  **TODO:** Make the reserved toggle active once the normal atlas exists.

---

## Summary of the "Knobs" required in UODynamapper

To allow the user to transition between EC and KR visually, the `LandEffectsUniform` and global `SceneSettings` must expose:

1.  **`post_process_profile`**: Enum `[Neutral, EC, KR]`.
    *   Current: drives material-shader tonemap constants.
    *   Future: should drive fullscreen `PostProcessSettings` once bloom/tonemap is moved to a real post-process pass.
2.  **`enable_grunge`**: Boolean. Enables shared grunge/weathering.
3.  **`grunge_strength`**: Float `[0.0 - 1.0]`.
    *   Current: controls procedural grunge on terrain/statics.
    *   Future: should control texture-backed `noise.tga`/DDS grunge.
4.  **`enable_normal_maps`**: Boolean.
    *   Current: toggles EC land role-3 texture normal-map sampling from the existing UDDP land atlas.
    *   Future: may gain per-material strength or art/static support after visual validation.
5.  **`light_decal_intensity`**: Float.
    *   Current: scales static-light visible decal alpha and local land/art light response.
    *   Future: should also scale temporary dynamic additive 2D light quads.
