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

`land/coverage.wesl::ec_terrain_stack` currently reconstructs a 4×4 ownership
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

The perspective capture also exposes roof overlap striping. Ground blending
works in that view, but this is not a clean perspective-depth acceptance result;
the roof/art overlap needs its own diagnosis. The orthographic bridge/stair
ordering correction is documented separately in `static_depth_ordering.md`.
