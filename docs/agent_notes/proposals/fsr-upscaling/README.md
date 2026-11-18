# FSR 1.0 EASU land sampling (not wired)

`fsr_easu.wesl` ports AMD FidelityFX FSR 1.0's Edge Adaptive Spatial Upsampling (MIT) as a sampler for the land
atlas: `sample_tile_fsr_easu(uv, tile)` upsamples one tile's texels with the 12-tap directional kernel.

No shader or system of Dynamapper ever imported it, so it was moved out of `dynamapper/assets/shaders/` on
2026-11-18 and kept for reuse. The CPU version of the same kernel, used by the converters, is
`lib/support/uot-image-postprocess/src/upscaling/fsr.rs`.

To wire it again:

1. Move it back to `dynamapper/assets/shaders/postprocess/upscaling/fsr_easu.wesl`, where its module path
   `package::shaders::postprocess::upscaling::fsr_easu` resolves.
2. It imports `TileUniform`, `tex_small`, `tex_big` and `land_page_atlas` from
   `package::shaders::world::land::land_bindings`; check that those bindings still have these names and layouts.
3. Call `sample_tile_fsr_easu` from the land sampling path in place of the plain atlas fetch, behind a setting, and
   measure it with `skills/renderer-reference-validation/scripts/frame_benchmark.py`.
