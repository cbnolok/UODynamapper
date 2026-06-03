# image-postprocess

`image-postprocess` provides image scaling and filtering algorithms used before
textures are encoded and packed into `.uddp` packages. It is consumed by
`udd-conv` during the conversion pipeline.

## Upscaling algorithms

| Algorithm | Notes |
| --------- | ----- |
| Nearest | Integer nearest-neighbor. |
| Bilinear | Bilinear interpolation. |
| CatmullRom | Catmull-Rom bicubic. |
| Lanczos3 | Lanczos with radius 3. |
| xBRZ | Pixel-art upscaler (via `xbrz-rs`). |
| Super-xBR | Hyllian Super-xBR 2x pixel-art upscaler. |
| ScaleFX | ScaleFX pixel-art upscaler; native 3x output with 2x/4x wrappers. |
| CUT1 / CUT2 / CUT3 | Cheap Upscaling Triangulation 2x filters. |
| lqx | lq2x-style pixel-art upscaler. |
| hqx | hq2x/hq3x/hq4x (via C++ translation). |
| epx | EPX / Scale2x. |
| 2xSaI / Super2xSaI / SuperEagle | SAI-family pixel-art upscalers. |
| MMPX | MMPX-style pixel-art upscaler (via `mmpx`). |
| Kopf-Lischinski | Depixelizer algorithm. |
| NEDI | New Edge-Directed Interpolation. |
| AMD FSR | FidelityFX Super Resolution: EASU pass, or EASU + RCAS sharpening pass. |

All algorithms are translated to Rust and use SIMD where available via the
`wide` crate.

## Notes

- The `apply_filter_passes_owned` function is the main entry point, taking an
  image and a sequence of `UpscaleFilter` passes.
- Upscale preview is available in `uocf-inspector-gui` for visual comparison.
- Algorithm implementations are adapted from the sources listed in the main
  [README.md](../../README.md) (Algorithms section).
