# Renderer export mode (`DYNAMAPPER_EXPORT_JOB`) — 2026-11-01

This is the renderer side of the [web map export](../agent_notes/proposals/2026-11-01-web-map-export.md).
`webmap-export-cli run --renderer <dynamapper>` now produces a tile pyramid end to end. The Classic
2×2-frame pilot over Britain has no visible seams. Overlaps are not yet bit-identical everywhere; see
[Remaining limits](#remaining-limits).

## What the renderer does

- **Activation.** `DYNAMAPPER_EXPORT_JOB=<absolute job.toml>` (`core/export_job.rs`).
  - The job is read strictly, with `deny_unknown_fields`.
  - The renderer checks the version and that the projection is 22 px per half width and 4 px per Z.
  - It also checks the facet, the frame size (the padded target must fit 8192 px) and that the spool
    directory exists.
  - Frame ids must be plain file names.
  - A bad job exits with code 2 before any window opens.
- **No interactive state.**
  - The preset is applied to the loaded settings in memory only.
  - The settings sync, debounced save and exit flush do not run in export mode.
  - The window is created hidden. In every run here, no window of the process tree was ever mapped,
    and OS focus and pointer were unchanged.
- **Preset.**
  - Every static is drawn. Static and local lights are off, and so are hidden multi parts, the hover
    highlight, the player, overlays and anti-aliasing.
  - Zoom is 1.0 with the orthographic camera.
  - Fog and water animation are forced off.
  - Render-world `Time` is reset every frame, so every `globals.time` material is drawn at t = 0.
  - Daylight is the style preset's `afternoon` slot: `classic.afternoon` for `cc` and `kr.afternoon` for
    `ec`. This choice is open to review.
- **Placement.**
  - The camera looks at the whole tile whose top vertex is nearest the frame centre, from the
    interactive offset (5,5,5).
  - It renders a target of the frame size plus 44 px on every side, with that vertex on the target's
    centre pixel corner, and the frame is cut out around it.
  - Every frame therefore rasterises the same camera-relative geometry as the interactive view; only
    the cut moves.
  - The check tile and pixel come from `Camera::world_to_viewport`. The renderer fails the frame unless
    they land on a whole pixel and equal the tool's projection.
- **Chunks.**
  - `ExportChunkFootprint` replaces the window-derived chunk set in `sys_update_worldmap_chunks_to_render`.
  - It covers the frame's z = 0 footprint widened by `(z_top + 512) / 22 + 1` tiles on every side.
- **Readiness.**
  - Every footprint chunk must have its mesh.
  - Land uploads must be idle, with the submitted metadata revision equal to the atlas revision.
  - The art collection must show no atlas misses or pending pages.
  - No render pipeline may be waiting to compile, counted in the render world.
  - The player must be at the rendered position.
  - All of this must hold unchanged for 3 frames.
  - After that, the frame is captured repeatedly until two consecutive captures are byte-identical.
    Without this, the first frame after the switch to the offscreen target came out black while its
    pipelines compiled.
- **Output.**
  - Raw RGBA8 is written as `<id>.rgba`, then the sidecar `<id>.toml`. Each is written as `*.tmp` and
    renamed; the sidecar comes last.
  - The renderer waits while the spool holds `max_spool_frames` sidecars.
  - The sidecar lists `map{f}`, `statics{f}`, `tilemeta` and `hues`, plus the texture packages present:
    the `cc` set for `cc`, or the EC set plus the Classic fallbacks for `ec`. Each comes with its
    embedded `package_hash64`.
  - The process exits 0 after the last frame and 1 on any frame error.

## Shader change: sub-pixel snap

`camera_relative_world_to_clip` now rounds screen positions to 1/64 px. UO geometry lands on whole
pixels at zoom 1. Rounding could leave a vertex a hair below a pixel corner, and the rasteriser's own
snap then flipped pixel-centre ties along shared quad edges, depending on the camera.

In two overlapping Classic frames (512 px kept overlap), this cut the differing pixels from 1,883 to
82. Compared with the unsnapped shader, it changes 0.06% of a Classic frame, all on those tie pixels
along water-quad edges, and they look the same. In a KR frame it changes 1.1%, almost all by 1–2
levels. The GPU projection test gained a case that only passes with the snap.

## Validation

| Check | Result |
| --- | --- |
| `cargo test -p dynamapper` | 183 pass. With `--include-ignored` on land/art/export (GPU and installed packages): 122 pass |
| Manual job (Britain, frame `a`, frame `b` 3584 px to the right, `a` again), Classic | `a` and `a_again` are bit-identical. Kept overlap: 82 of 2,097,152 px differ, on one diagonal of land-water texels |
| Same job, `ec` (KR-like) | `a` and `a_again` are bit-identical. The kept overlap differs only on water: 10% of it, from the camera-focused EC reflections |
| `webmap-export-cli plan --region 1400,1600,1500,1700` + `run --renderer` | 4 frames, each passing the tool's rectangle, projection and package checks. 85 tiles per frame, levels 0–9, viewer written. About 3 s render per frame |
| Tiles | The level-9 mosaic and a full-resolution crop at the four-frame corner show no seam, player or overlay |

Evidence is in `/home/claudio/rust-target/uodynamapper-scratch/renderer/export/`:

- `manual*/`: jobs, spools, logs and difference crops.
- `pilot-cc/`: plan, tiles and `index.html`.
- `pilot-cc-mosaic-small.png` and `pilot-cc-seam-crop.png`.
- `snap-effect-*.png`.
- `scripts/run_hidden.py`, the hidden-window and desktop-state watcher.

## Remaining limits

- **Classic: about 0.004% of an overlap differs.** These are land-texel ties along tile diagonals.
  `uv_in_tile = fract(world_position)` is interpolated from absolute coordinates of about 1500, where
  f32 resolves about 1e-4. A tile-local varying would remove it. That was not done in this phase.
- **EC water reflections follow the camera focus**, by design (`ec_liquid_camera_reflection_uv`). An
  export needs a fixed reflection focus or reflections off. This is a decision for the user.
- **Interactive check not run.** The snap could not be checked in the interactive three-preset scene
  matrix. The session DBus socket that the Desktop 4 preview helper uses disappeared mid-session, so
  GUI previews were stopped. The snap was checked only through the offscreen export, in the Classic and
  KR presets.
- **Daylight slot.** The export daylight is the style's `afternoon` slot from the effective shader
  presets, so a user's tuned `land.toml` changes the export.
