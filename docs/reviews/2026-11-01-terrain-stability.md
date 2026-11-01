# Camera-relative world projection — 2026-11-01

The [web-map control pilot](2026-11-01-webmap-control-pilot.md) found that translated captures
of the same scene differ on terrain alone. This phase removes the main cause. Land, static sprites
and ground-art quads now project through one shared camera-relative function. The full scene is
now pixel-identical under translation at Britain in Classic and Enhanced. Small residual
differences remain at x≈7000 in all presets, and at Britain in KR-like; see below.

## Cause and change

The world shaders formed clip positions by applying Bevy's precombined world-to-clip matrix to
absolute map coordinates. At map coordinates in the thousands, that product rounds away low bits
while it cancels the camera translation. The result is a sub-pixel jitter that depends on where the
camera is. Interpolated values and texel selection then change between captures that show the same
tiles at different screen positions.

`shaders/world/projection.wesl` adds `camera_relative_world_to_clip(origin, offset)`. It first
subtracts the camera position from the tile/instance anchor, and only then adds the small local
mesh or sprite offset. Rotation and `clip_from_view` are applied afterwards. The geometry, the
camera projection (including jitter), the logical XYZ and the world-position varyings are
unchanged. The land vertex shader, the sprite/billboard/surface/camera-quad and shadow branches
of `art/main.wesl`, and `art/ground.wesl` use it. The logical priority depth still uses the
previous path. An attempted camera-relative priority depth changed no pixel in any capture below,
so it was not kept.

The renderer's shader-import test now checks that the three entry points discover the new module.
The GPU sampler test covers the function, including ±8192 translations and 1/32768 offsets.

## Validation

- `cargo test -p dynamapper` for `texture_cache::land` and `render::scene::world::art`: 112 pass.
  All ignored GPU and installed-package tests in those modules also pass (119 with `--include-ignored`).
- Scene checks: 64 captures, one before/after pair of each scene. Bridge, road, lava, swamp, town,
  coast and item-water blood (623,1497) were captured in Classic, Enhanced and KR-like (orthographic
  and KR perspective). KR also covers primary-only 7379 and dungeon blood. Every run used an
  exact-PID Desktop 4 placement and completed its cleanup, with no shader errors. OS focus and
  pointer were unchanged afterwards.
- Visual review: before/after crops differ only where a texel boundary falls on a pixel centre, mostly
  along land diagonals and in water texture noise. Those pixels change by up to the
  neighbouring-texel delta: 0.7–8.5% of pixels by more than 2 levels. Every after capture
  was inspected and shows no geometry, seam or material regression.
- Translated overlap pilot (`webmap-export-cli pilot`, step 8 tiles, 32 px guard, export-like
  preset, zoom 1). Each preset ran at Britain (1496,1640) and at (7000,392):

  | Preset | Britain: differing fraction (4 pairs) | x≈7000: differing fraction (4 pairs) |
  | --- | --- | --- |
  | Classic | 0, 0, 0, 0 | 0.0008%, 0.094%, 0.13%, 0.13% |
  | Enhanced | 0, 0, 0, 0 | 0.0005%, 0.094%, 0.13%, 0.13% |
  | KR-like | 0.69%, 0.011%, 0.26%, 0.016% (every difference is 1 level) | 0.039%, 0.0017%, 0.0093%, 0.0024% |

  In every preset, returning to Britain reproduces the first capture exactly.

## Remaining limits

The phase goal, exact translated overlap at both sites in all three presets, is **not met yet**:

- At x≈7000, the Classic/Enhanced differences (up to 98 levels) all sit on the internal tile seams of
  one ground-art carpet. These are pixel-centre ties between adjacent quads. Camera-relative priority
  depth did not change them. Their mechanism is not established.
- In KR-like, the differences are single-level changes on EC stone floor art. They are densest far
  from the camera. This is consistent with linear filtering amplifying rounding differences in the
  rasterised attributes, but that is not proven.
- EC water reflections deliberately follow the camera focus
  (`ec_liquid_camera_reflection_uv`), so they cannot be translation-invariant. An export needs a
  fixed reflection focus, which is a design decision.

Evidence: `/tmp/uodynamapper-ec-followup/p5final/` holds the pilot reports and captures, the return
captures, `scene-before-after-diff.json`, the `diffviz/` contact sheets and the difference maps. The
`first-run/` directory is the identical run made before the depth experiment. Scene captures and
driver records are under `/home/claudio/rust-target/uodynamapper-scratch/renderer/phase5-final/`.
Proprietary captures stay outside the repository.
