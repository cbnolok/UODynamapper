# KR Post-Processing & Sprite Shaders — WGSL Translation Reference

Translated from the Mythic Package Editor KR shader dump (`Shaders.uop_B0_F*.txt` and `data/shaders/`).
All files live in `dynamapper/assets/shaders/postprocess_kr/`.

---

## Files Created

| File | Source shader(s) | Applied on |
|------|-----------------|-----------|
| [kr_bloom_pipeline.wgsl](file:///mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/postprocess_kr/kr_bloom_pipeline.wgsl) | F0 (all techniques) | Full screen — after main render |
| [kr_sprite_hue.wgsl](file:///mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/postprocess_kr/kr_sprite_hue.wgsl) | uosprite.psh / uospriteui.psh | Per-sprite fragment (hued entities & UI) |
| [kr_sprite_base.wgsl](file:///mnt/dati/__cloud/mega/_proj/_Rust/UODynamapper/dynamapper/assets/shaders/postprocess_kr/kr_sprite_base.wgsl) | F1, uosprite.vsh, uospriteui.vsh | Per-sprite vertex (all sprites + UI) |

---

## KR vs EC Shader Differences

While Kingdom Reborn (KR) and Enhanced Client (EC) share a similar rendering pipeline, there are notable differences in how the shaders process data:

### 1. Bloom Pipeline (Post-Processing)
- **Identical Logic:** The KR bloom pipeline (`F0`) is byte-for-byte identical to the EC bloom pipeline (`F6`). Both use the same 9 techniques for downscaling, Gaussian blurring, tonemapping, and bright-pass extraction.

### 2. Sprite Hue Calculation
- **EC (Enhanced Client):** Uses a Rec.709 dot product to calculate luminance for hue palette lookups: `luma = dot(rgb, vec3(0.2125, 0.7154, 0.0721))`.
- **KR (Kingdom Reborn):** Uses a simple average of the RGB channels: `luma = (r + g + b) / 3.0`.

### 3. Sprite Ethereal Effect
- **EC:** Includes an `IS_ETHEREAL` flag that adds a subtle blue-shift bias (`vec3(0.0, 0.01, 0.025)`) to entities, representing ghosts or hidden players.
- **KR:** This ethereal blue-shift is absent in the KR shaders.

### 4. Hue Masking
- **KR:** Explicitly supports a `HAS_HUEMASK_TEX` compile-time permutation. If enabled, the shader samples a mask texture and only applies the hue palette to pixels where the mask's alpha is greater than `0.5`.

### 5. UI Vertex Coordinates
- **EC:** The UI vertex shader (`F10`) calculates NDC (Normalized Device Coordinates) directly from pixel coordinates using a `screenParams` uniform containing scale and bias factors. It skips the `viewProjMatrix` entirely.
- **KR:** The UI vertex shader (`uospriteui.vsh`) still uses the standard `worldTransform` and `viewProjMatrix` multiplication, just like regular world sprites.

### 6. Hue Coordinate Unpacking
- **EC:** The hue texture coordinates (`hue_coord`) are pre-calculated on the CPU and passed directly into the vertex shader attributes.
- **KR:** The UI vertex shader receives a single scalar uniform `hueIndex` and unpacks the 2D texture coordinates inside the shader math:
  ```wgsl
  out.tex_coord1.x = floor(hue_index / 1024.0) / 1024.0 * 255.0;
  out.tex_coord1.y = (hue_index % 1024.0) / 1024.0;
  ```

---

## Shader Breakdown

### `kr_bloom_pipeline.wgsl`
Implements the exact same 9-technique post-processing pipeline as the EC version.

### `kr_sprite_hue.wgsl`
Consolidates both `uosprite.psh` and `uospriteui.psh`. It features:
- Base sprite texture sampling.
- Conditional hue masking.
- RGB-average based luma calculation for the hue palette lookup.
- Conditional grunge texture multiplication.

### `kr_sprite_base.wgsl`
Contains the vertex shaders for KR sprites:
- `world_sprite_vs` (from F1): Standard world billboard.
- `world_sprite_effect_vs` (from `uosprite.vsh`): Applies 3x3 `spriteTransform` and `grungeTransform` matrices for animated UVs.
- `screen_sprite_ui_vs` (from `uospriteui.vsh`): UI billboard with dynamic scalar-to-2D hue coordinate unpacking.
