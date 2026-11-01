# Web-map control-channel pilot — 2026-11-01

The requested live-app pilot establishes correct placement, but does **not** pass
the pixel-identical overlap requirement for stitched export. The error persists
on terrain alone. Stronger settling and returning to the same camera location
produce identical pixels, separating this issue from loading, clock and history
effects in the tested scene.

## Fixture and results

The pinned `2314b493` renderer used installed packages, map 0, CC art/land,
`classic.afternoon`, orthographic zoom 1.0, a 1280x800 window and scale factor 1.
Water animation, fog, blur, AA, projected/static/local lights, player and overlays
were disabled. No masks were needed. Three previews verified exact-PID Desktop 4
placement and cleanup, with unchanged OS focus and pointer.

`webmap-export-cli pilot` captured centers `(1496,1640)`, `(1504,1632)`,
`(1504,1648)` and `(1512,1640)`. Every best shift is exactly the expected shift:

| Pair | Shift px | Different pixels | Mean absolute RGB difference |
| --- | --- | ---: | ---: |
| 0 → 1 | 352, 0 | 6.416% | 1.045 |
| 0 → 2 | 0, 352 | 8.414% | 1.376 |
| 0 → 3 | 352, 352 | 0.973% | 0.151 |
| 1 → 2 | -352, 352 | 0.053% | 0.0034 |

The tool's current WebP defaults produced 419–444 kB per frame, 3.27–3.47 bits
per pixel and 164–190 ms encode times. These are pilot measurements, not a facet
size/performance guarantee.

A second preview used the stricter app helper's readiness check plus another
second before each capture, repeated all four locations and returned to location
0. Every decoded RGB raster exactly equals its corresponding first-pilot raster;
the returned location also exactly equals its first capture.

A third completed preview disabled static art in copied settings. Terrain-only
overlap differences remain: 15.628%, 16.540%, 1.648% and 0.134% for the same pairs,
with mean absolute differences 2.456, 2.614, 0.253 and 0.022. Terrain was settled
with matching logical/rendered positions and no pending metadata. The tool's
general pilot cannot wait on this diagnostic mode because disabled art has a
null readiness block; these captures therefore used terrain-specific status
checks after the teleport wait. The interrupted generic attempt is excluded.

## Implication and next checks

Terrain sampling/rasterization is a demonstrated component of the seam problem.
Point filtering and interpolation precision are hypotheses, not an established
root cause. `land/main.wesl` currently reconstructs per-tile UVs with
`fract(in.world_position.xz)` and chooses tile metadata with `floor`; those are
relevant places to investigate camera-position sensitivity. The pilot does not
prove that static-art sampling is independently seam-free.

Before export acceptance, isolate those precision/filter decisions and establish
stable source texel selection on an exact pixel grid. Then repeat these translated
overlap checks and the same-frame/other-frame/same-frame history check. The
offscreen export path must also implement the approved fixed frame placement,
complete footprint streaming, frozen state, readiness and atomic spool contract.
No `DYNAMAPPER_EXPORT_JOB` implementation is claimed by this pilot.

Evidence is under `/tmp/uodynamapper-ec-followup/webmap-control-pilot/`:
`accepted/pilot-report.toml`, `settled-0..4.png`, `terrain-only-comparison.json`,
`ground-0..3.png` and the corresponding normalized control/desktop records.
Reference images and proprietary captures stay outside the repository. The later
camera-quad water UV correction changes only KR item material descriptors; this
CC pilot does not exercise that path.
