# Release fixture measurements

Use `scripts/dev-helpers/renderer/benchmark.py` against an isolated, already
running preview. The socket's FPS/frame-time fields are Bevy **smoothed**
diagnostics. Median/min/max sampled FPS describe that signal, not raw frame
percentiles or GPU timestamp measurements. Keep its JSON replies as evidence.

## Measured configuration

The 2026-10-28 run used the stable recipe
`just build-local-workspace-release --bin dynamapper --jobs 2`, the configured
Cargo target directory, KR morning, EC art/map packages, map 0, 1280×800 and
zoom 0.8. Compilation completed before measurement. VSync, frame limiting and
unfocused throttling were disabled in temporary settings. The exact-PID KDE
helper kept the app on Desktop 4 without activation and verified cleanup.
Both camera runs used one process for all four same-map teleports.

Each location settled for eight seconds, then received status requests every
0.25 seconds for twenty seconds (74–77 replies). The GPU is PCI ID `1002:150e`,
AMD Strix Radeon 880M/890M. These are local, off-desktop measurements on a
shared workstation, not portable benchmark guarantees or original-client
comparisons.

| Fixture | Position | Orthographic median sampled FPS | Perspective median sampled FPS |
|---|---|---:|---:|
| Static/map transition | 2518,561 | 46.0 | 46.8 |
| Bridge | 2834,987 | 46.1 | 42.1 |
| Lava | 5744,2928 | 32.7 | 34.1 |
| Town | 1647,1682 | 36.7 | 37.0 |

The 400 FPS orthographic / 144 FPS perspective goals are **not met** by this
configuration. A simultaneous GPU-busy read was 99% with the owned preview
running and approximately 10% after cleanup; its CPU usage was around half a
core. That supports a GPU bottleneck, but a shared-device utilization reading
does not attribute time to individual passes.

## Diagnostic isolation

The existing `shading_mode=3` early exit replaces terrain fragment shading with
flat color; it retains terrain geometry and all statics. Using the same release
binary/settings, the bridge and lava measured 61.0 and 60.2 median sampled FPS.
This removes ownership-stack construction, texture sampling and terrain
lighting together; it does not identify one of those as the sole cause.

Disabling statics measured 51.8 FPS at the bridge and 35.1 FPS at lava. The
small lava change compared with the flat-terrain result further points toward
terrain shading there. This is a complementary temporary diagnostic, not a
production optimization. These captures must not be used as fidelity results.
All production settings were restored after each run.

The baseline `land/coverage.wesl::ec_terrain_stack` reconstructed a 4×4 ownership
footprint and sorted local material stack per fragment, including atlas and
material lookups and dynamic arrays. `liquid_bank.wesl` can search another
4×4 footprint at liquid/solid boundaries. These are concrete candidates for
precomputation/cache work; retain the recovered chunk-edge weights, ordering,
aliases and bank behavior when changing them. A measured pass breakdown and
controlled before/after comparison are still needed before claiming a gain.

## Evidence and visual limits

Temporary JSON reports are
`/tmp/uodynamapper-ec-followup/release-kr-{orthographic,perspective}.json`;
diagnostic suffixes identify separate runs. Logs use
`/tmp/uodynamapper-release-profile-*.log`. Final normal captures use
`release-kr-{orthographic,perspective}-transition.png`, with completed PNGs and
zero pending land uploads in their overlays. No shader/pipeline errors appeared.

Those initial perspective captures exposed roof overlap striping, subsequently
corrected as documented in `isometric_perspective_projection.md`. The orthographic
bridge/stair
ordering correction is documented separately in `static_depth_ordering.md`.

## Rejected ownership shortcut

A CPU-proven five-by-five uniform footprint was tested using a spare EC metadata
bit, comparing all seven material lookup records and retaining the full stack's
canonical/first-alias payload. The direct shader variant measured approximately
2% bridge and 5% lava gains in an ABBA release comparison. However, matched
captures retained one-pixel differences around static edges, while repeated full
baselines stayed within two byte values. Moving single-layer shading to a shared
call site reduced but did not eliminate them. Derivative-sensitive filtering is
a hypothesis, not an established cause. This candidate was removed: the small
measured gain did not justify an unresolved image change and extra CPU policy.
Temporary `release-kr-uniform-direct-*` and `shared-call-*` reports/captures describe
that rejected experiment, not the accepted renderer or performance guarantees.

A second candidate split raw metadata reads from full texture-slot resolution for
height-only normals/occlusion queries. All six matched fixture captures differed
by at most one byte. Release ABBA medians were nevertheless effectively unchanged:

| Query | Bridge sampled medians | Lava sampled medians |
|---|---|---|
| Full metadata | 53.32, 53.35 | 39.03, 39.07 |
| Direct height | 53.89, 53.08 | 39.09, 39.21 |

This does not establish a material speedup; shader dead-code elimination may
already prune unused slot fields, but that was not verified in GPU assembly.
The refactor was removed as well. Reports use `release-kr-height-*`; measurements
follow the same paused KR/EC configuration, eight-second settling and twelve-second
sampling as the ownership experiment. The next performance work needs a measured
pass breakdown or a larger ownership-cache change, rather than assuming helper
call counts equal GPU memory traffic. The original FPS targets remain open.

## Hardware pass breakdown

The completed sprite-hue renderer was rebuilt with the same stable release
recipe before profiling. Existing Bevy hardware diagnostics were enabled in
isolated startup settings, with one-second console dumps. `benchmark.py` now
records wall-clock measurement windows; `pass_timings.py` correlates those with
the owned preview log, excluding both boundary seconds. The same paused KR/EC,
1280×800, zoom-0.8 orthographic fixtures settle for eight seconds and sample for
twelve seconds, with caps disabled and Desktop-4 ownership/cleanup verified.

| Diagnostic variant | Bridge opaque GPU ms | Lava opaque GPU ms |
|---|---:|---:|
| Production renderer | 18.16 | 25.54 |
| Statics disabled | 17.45 | 25.47 |
| Flat terrain fragment | 1.20 | 0.52 |
| Ownership/composition bypassed | 3.32 | 2.59 |
| Terrain albedo before lighting | 8.42 | 9.98 |

These are medians of **smoothed hardware opaque-pass durations**, rounded by the
console, not raw frame percentiles. The reported sum of timed GPU spans is not
frame wall time. The flat diagnostic reaches roughly 229/233 sampled FPS while
its timed GPU work is shorter than a whole frame, exposing a different limit.

The bypass retains terrain geometry, statics and `shade_land`, but selects the
current raw tile instead of reconstructing ownership, layered composition and
liquid-bank selection. It is visibly incorrect and was restored after each run.
The albedo diagnostic retains reconstruction and texture work but skips the
later terrain lighting/grunge/material response. Neither is a proposed fidelity
change. Their costs cannot be added/subtracted as independent stage durations:
shader compilation, register pressure and the number of shaded layers interact.

The combined results identify terrain ownership/composition as a substantial
optimization target, with statics and upscaling unable to explain the normal
cost. A cache must preserve canonical/first aliases, original 49-pass selection,
edge membership, point/bilinear alpha and bank behavior, while leaving the actual
shading call sites intact. Temporary reports use `release-kr-passes-final-*` and
`gpu-passes-final-*`; all production shaders/settings were restored.

## Exact terrain stack cache

The bounded cache described in [ec_terrain_stack_cache.md](ec_terrain_stack_cache.md)
retains the original reconstruction as `ec_terrain_stack_uncached`. It moves
ownership, sorting and mask normalization to chunk preparation, preserving
the existing terrain shading calls and liquid-bank policy.

Native comparisons use six fixtures: transition (2518,561), bridge (2834,987),
lava (5744,2928), coast (1445,1790), swamp (1872,2416), town (1647,1682).
Settings and screenshots are isolated, water is paused, and overlays are disabled
for whole-image comparisons. Metadata revisions agree and no uploads are pending
at capture; EC fixtures publish all sixteen cache chunks, CC publishes none.

| Matched cached/uncached captures | Largest channel-byte difference |
|---|---:|
| Enhanced EC, orthographic, six fixtures | 0 |
| Classic EC, orthographic, six fixtures | 0 |
| KR EC, orthographic, six fixtures | 1 |
| KR EC, perspective, six fixtures | 1 |
| Enhanced CC, orthographic, six fixtures | 0 |
| KR EC, one metadata upload/frame versus normal budget, six fixtures | 1 |

The renderer suite passes 133 tests including the explicit Vulkan regressions.
The cache test requires every packed layer and alpha bit to match the original
GPU reconstruction at 9,216 positions, with separate cache-hit and stale-guard
proofs. Native captures remain necessary: an earlier byte-mask representation
passed a tolerance-based compute comparison but retained repeatable coastal
pixel differences. It was replaced with normalized float bits in the existing
RGBA record, without increasing the allocation or changing shading calls.

Temporary accepted evidence uses `stack-float-kr-native-*`,
`stack-original-kr-repeat-*`, `stack-final-*`, and `stack-final-matrix.json` under
`/tmp/uodynamapper-ec-followup`. Earlier `stack-cache-kr-native-*`,
`stack-symbolic-*` and diagnostic-color captures are investigation artifacts,
not accepted visual baselines. Every final preview verified Desktop 4 placement
and cleanup, with no shader/pipeline errors.

The final stable-release comparison uses the same KR morning, paused EC art/map,
1280×800, zoom-0.8 configuration and hardware above. Compilation finishes before
measurement; caps and unfocused throttling are disabled. One binary runs both
variants, changing only the cache-lookup guard in isolated shader assets. Both
retain identical CPU preparation, with sixteen published chunks and equal
CPU/submitted metadata revisions throughout every sample window.

Orthographic runs use original/cached/cached/original order; table values average
the two run medians. Perspective has one original/cached pair. Each fixture
settles eight seconds and samples for twelve seconds. GPU values retain the
existing smoothed, rounded opaque-pass diagnostic semantics.

| Camera / fixture | Original sampled FPS | Cached sampled FPS | Original opaque GPU ms | Cached opaque GPU ms |
|---|---:|---:|---:|---:|
| Orthographic bridge | 51.99 | 78.63 | 18.11 | 11.68 |
| Orthographic lava | 38.14 | 53.54 | 25.42 | 17.79 |
| Perspective bridge | 47.86 | 71.80 | 19.52 | 12.86 |
| Perspective lava | 36.82 | 51.82 | 26.19 | 18.42 |

The orthographic repeats establish approximately 51%/40% higher sampled FPS and
36%/30% lower opaque GPU time at bridge/lava respectively. The perspective pair
shows a similar gain, with less repetition. This validates the cache's benefit
in these settled fixtures, not arbitrary viewports or continuous movement.
The 400/144 FPS targets remain open; cache misses retain the full reconstruction
and the remaining material/bank/lighting cost still needs separate profiling.

Raw status reports use `release-kr-passes-stack-final-*`; correlated hardware
reports use `gpu-passes-stack-final-*`. `stack-final-performance.json` retains
all run summaries. All six timing previews verified Desktop 4 and cleanup;
production shaders and isolated settings were restored afterward.
