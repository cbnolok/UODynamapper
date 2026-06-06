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
| OmniScale | Pattern-based pixel-art upscaler. |
| Jinc2 | Hyllian Jinc2 windowed-jinc resampler variants. |
| CUT1 / CUT2 / CUT3 | Cheap Upscaling Triangulation 2x filters. |
| lqx | lq2x-style pixel-art upscaler. |
| hqx | hq2x/hq3x/hq4x (via C++ translation). |
| epx | EPX / Scale2x. |
| 2xSaI / Super2xSaI / SuperEagle | SAI-family pixel-art upscalers. |
| MMPX | MMPX-style pixel-art upscaler (via `mmpx`). |
| Kopf-Lischinski | Depixelizer algorithm. |
| NEDI | New Edge-Directed Interpolation. |
| AMD FSR | FidelityFX Super Resolution: EASU pass, or EASU + RCAS sharpening pass. |
| ScaleFX Smart Deblur | Edge-aware post-upscale crispening pass using ScaleFX-style tuned defaults. |
| Guestr Deblur | CPU port of `deblur/shaders/deblur.glsl`; edge-aware 3x3 neighborhood deblur using Libretro's exposed defaults. |
| Unsharp Mask Small | Small-radius post-upscale unsharp mask for controlled edge contrast. |
| High-pass Sharpen | Small-radius high-pass post-upscale sharpening pass. |

All algorithms are translated to Rust and use SIMD where available via the
`wide` crate.

Runtime pixel-art seam filters for fractional-scale rendering live in
`dynamapper/assets/shaders/world/pixel_art_filters.wgsl`. IQ is wired into the
linear sprite and terrain sample paths; BGolus AA-linear/AA-smoothstep, Klems,
and fat-pixel UV remaps are available there for later tuning.

## Post-upscale deblur

`GuestrDeblur` is inspired by guest(r)'s Libretro
`deblur/shaders/deblur.glsl` fragment pass. It samples the current pixel plus an
`OFFSET`-spaced 3x3 neighborhood, estimates local min/max contrast, builds an
edge-directed replacement color from inverse color-distance weights, then mixes
that result back by local contrast and `SMART`.

The exposed CPU defaults match the shader's `#pragma parameter` defaults:
`OFFSET=2.0`, `DEBLUR=4.5`, `SMART=0.5`. The CPU port keeps RGB math in
linear `0.0..1.0` byte-normalized space, uses bilinear reads for fractional
offsets, clamps source coordinates at image borders, and preserves the source
alpha channel instead of forcing alpha to opaque as the GLSL framebuffer pass
does.

Use `GuestrDeblur` when comparing against or reproducing the Libretro shader.
Use `ScaleFxSmartDeblur` as the looser tuned post-pass that was added for this
pipeline before the exact shader port; it is intentionally simpler and remains
available for visual comparison.

## Notes

- The `apply_filter_passes_owned` function is the main entry point, taking an
  image and a sequence of `UpscaleFilter` passes.
- The `palette` module provides the palette-safe sprite pipeline. It decodes
  source RGBA into indexed semantics, normalizes through a canonical palette,
  runs a scaler with explicit transparency rules, and restricts the final
  raster through `StrictSnap`, `RampAwareSnap`, `ExpandedPalette`, or `NoSnap`.
- Upscale preview is available in `uocf-inspector-gui` for visual comparison.
- Algorithm implementations are adapted from the sources listed in the main
  [README.md](../../README.md) (Algorithms section).

## Palette-safe sprite pipeline

Ultima Online art is treated as palette-constrained even when decoded as RGBA.
The safe pipeline keeps four concepts separate:

- source palette colors: stable `ColorId`s backed by canonical display colors;
- transparency topology: alpha and color-key state, never a color to average;
- working colors: temporary RGBA produced by an upscaler;
- final palette restriction: deterministic snapping and diagnostics.

Pipeline pseudocode:

```text
palette = source_palette or build_canonical_palette(decoded_rgba)
indexed = decode_rgba_to_indexed_pixels(decoded_rgba, palette, transparency_policy)
indexed = normalize(indexed, remap/collapse/dither/ramp options)
working_rgba = scaler.scale(indexed, palette_context)
if output_alpha == SourceNearest:
    restore transparency topology from nearest source pixels
if snap_mode != NoSnap:
    snap each opaque candidate to strict, ramp-aware, or expanded palette
    clean isolated illegal pixels near edges
diagnostics = stage color counts, off-palette candidates, snap histogram,
              alpha-edge pixels, dither cells, ramp/forbidden violations,
              per-algorithm timings
```

Palette restriction happens in three places. Before upscaling, decoded colors
are remapped to canonical palette entries so equivalent RGB values share stable
ids. During scaling, algorithms that consume `PaletteSemanticContext` should use
palette-aware equivalence, ramp, transition, and transparency checks for edge
classification instead of naive RGBA comparisons. After scaling, every opaque
working color is restricted by the selected snap policy unless `NoSnap` is used
for debugging.

Default transparency handling sanitizes hidden RGB in transparent pixels and
projects output alpha from nearest source topology. This prevents border halo
colors from transparent texels participating in interpolation. Blending across
transparency should only be enabled by an explicit policy because it changes the
sprite silhouette.

Dithering policy is explicit. `Off` treats the image normally. `DetectOnly`
emits diagnostics without modifying pixels. `CollapseToRamp` is for assets where
checkerboard shading should become a smoother ramp before scaling. `PreserveButConstrain`
keeps dither structure while forcing generated colors back into the relevant
palette/ramp. `ReinsertCheckerboard` is for mobile/humanoid sprites where an
upscaler has smoothed or scrambled intentional 2-color checkerboard shading; it
restores only detected source `A/B / B/A` regions, skips source pixels touching
transparency, and writes real palette colors before final cleanup. Dither
detection is intentionally local and deterministic.

Algorithm guidance:

- EPX / Scale2x / Eagle: best fit for strict palette output because they mostly
  copy source colors. Use `ImmediateSnap` or `StrictSnap`; compare by `ColorId`
  and transparency state, not RGBA.
- HQx: edge masks benefit from perceptual palette distance. Use
  `HybridSnap`: snap near hard edges immediately, defer smooth interiors, then
  final snap.
- xBR / xBRZ: strong edge classifiers but can produce blended colors. Use
  ramp-aware comparisons and final `RampAwareSnap`; strict mode is useful for
  silhouettes but may flatten ramps.
- NEDI: fundamentally interpolation-heavy. Keep it contained behind
  `DeferredSnap` or `NoSnap` comparison runs; strict palette output can look
  unstable unless ramp metadata is strong.
- Kopf-Lischinski depixelization: vector/region reconstruction conflicts with
  strict palette preservation when it optimizes smooth boundaries. Use it for
  comparison, or constrain generated fills to region source colors and snap
  boundaries at the end.

Recommended defaults:

| Asset class | Snap | Intermediate | Dither |
| ----------- | ---- | ------------ | ------ |
| Tiny humanoid sprites | `StrictSnap` or `RampAwareSnap` | `HybridSnap` | `PreserveButConstrain` |
| Terrain / ground tiles | `ExpandedPalette` with a small bound | `DeferredSnap` | `CollapseToRamp` when ramps are known |
| UI art / icons | `StrictSnap` | `ImmediateSnap` | `Off` or `DetectOnly` |
| Heavily dithered assets | `RampAwareSnap` | `HybridSnap` | `PreserveButConstrain` |

Validation should include unit tests for strict output palette membership,
transparent-border topology, deterministic expanded palette generation, and
diagnostic counters. Property tests should generate small indexed images and
assert deterministic output, no off-palette opaque pixels under strict/ramp
policies, no hidden RGB in transparent output, and no increase in forbidden
transitions after cleanup. Benchmarks should cover representative 36x36,
48x86, and terrain tile inputs and record total pipeline time plus per-scaler
timings from `PaletteDiagnostics`.
