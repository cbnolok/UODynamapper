# Release fixture measurements

Further renderer optimization is deferred at the user's request until the
desktop workstation profiling session. The recorded hypotheses and minimal
tests are in [the performance handoff](../reviews/2026-10-29-renderer-performance-handoff.md).
Visual fidelity is the current priority; measurements below remain historical evidence.

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

## Compact cached view and shared geometry normal

The follow-up keeps one composition policy for cache hits and misses. Hits read
the existing packed texture through a compact view; misses construct the original
arrays. A literal selector at each composition call lets the compiler discard
the hit's unused fallback argument. The geometry normal is shared across material
layers, with authored normal maps still applied per material. Details are in
[ec_terrain_stack_cache.md](ec_terrain_stack_cache.md).

This needed a filtering correction first: derivatives inside material-dependent
branches produced image changes despite bit-identical ownership. The continuous
footprint in [ec_terrain_filtering.md](ec_terrain_filtering.md) makes those filters
independent of stack execution. It intentionally corrects affected seam pixels;
the image-equivalence control uses that same footprint on both readers.

The final native matrix has 36 comparisons across the six fixtures, all three
styles, KR perspective, the Enhanced CC control and forced uncached KR. Enhanced,
Classic and CC comparisons are byte-identical; KR comparisons stay within one
byte per channel. Fog animation timelines are matched. All 133 renderer tests
pass, including the Vulkan record/alpha-bit regression and stale-guard checks.

Final stable-release measurements use the same adapter, 1280×800, zoom 0.8,
EC art/map, paused water and uncapped Desktop-4 configuration. Both variants use
one rebuilt binary and identical CPU preparation. The baseline shaders are the
previous expanded-cache renderer; the final shaders include the footprint and
shared-normal changes. Each fixture settles twenty seconds and samples twelve.
KR orthographic runs use baseline/final/final/baseline order; values below average
the two run medians. Perspective and Classic have one baseline/final pair.

| Style / camera / fixture | Baseline sampled FPS | Final sampled FPS | Baseline opaque GPU ms | Final opaque GPU ms |
|---|---:|---:|---:|---:|
| KR orthographic bridge | 78.63 | 120.68 | 11.70 | 7.31 |
| KR orthographic lava | 53.69 | 94.98 | 17.80 | 9.82 |
| KR perspective bridge | 71.54 | 112.26 | 12.84 | 7.90 |
| KR perspective lava | 52.16 | 92.31 | 18.38 | 10.12 |
| Classic orthographic bridge | 212.38 | 280.04 | 3.78 | 2.64 |
| Classic orthographic lava | 163.09 | 233.97 | 5.47 | 3.61 |

These retain the smoothed FPS and smoothed/rounded GPU-duration semantics above.
Every accepted timing window has sixteen published cache chunks, equal metadata
revisions, no pending metadata writes, and constant rounded fragment counts
matching its control. That last check matters: an earlier cold shader capture
was entirely clear color even after metadata submission. Exclude compilation and
partial-draw windows rather than treating missing terrain as a speedup.

The KR orthographic repeats establish roughly 53%/77% higher sampled FPS and
37%/45% lower opaque GPU time at bridge/lava. The Classic pair improves too.
The 400 FPS orthographic and 144 FPS perspective targets remain open on this
workstation. Continuous movement, larger views and the GTX 1060 baseline are
not established by these settled-fixture measurements.

Raw final data uses `release-kr-passes-lazy-warm-final-*`,
`gpu-passes-lazy-warm-final-*` and `lazy-warm-final-performance.json` under
`/tmp/uodynamapper-ec-followup`. Accepted images and cleanup evidence are indexed
by `lazy-final-accepted-matrix.json` and `lazy-final-accepted-logs.json`.
All eight final timing previews verified Desktop 4 and cleanup, with no shader
validation errors. Reusable compiler-statistic extraction is in
`scripts/dev-helpers/renderer/shader_stats.py`.

Intermediate single-shading-call, owner-array-pointer, unused-normal-guard and
mode/profile-specialization candidates were not retained. The single-call version
increased spills and GPU time; mode specialization was neutral in KR and slower
in Classic. Changing the owner-array parameter to a pointer compiled to identical
reported code, and the unused-normal guard added no convincing benefit. The
combined cached/fallback-view prototype also retained eager scratch clears and
regressed the Classic bridge. These temporary artifacts do not describe the
accepted implementation.

## Classic fragment normals

The customized Classic slot used above has `normal_mode=0` and bent normals
disabled. It therefore does not measure the cost of the factory Classic slot,
which enables bicubic and bent normals for its vertex/Gouraud lighting. The
fragment shader was also reconstructing those normals even though Classic's
lighting, light decals, micro-contrast and color-wash shadow input do not use
them. Only Classic combined with active KR-profile shadow weathering needs a
fragment normal. The guard in `land_surface_normal` preserves that exception
and all vertex normal/lighting work. It changes no presets or texture policy.

The factory comparison copies the checked-in presets into isolated preview
settings; saved user preferences are untouched. Both shader variants use the
same stable-release executable, EC art/map, paused water, 1280×800, zoom 0.8,
uncapped orthographic camera and the same AMD adapter. Each fixture settles
20 seconds and samples 12. Baseline/final/final/baseline order gives these
averages of the two run medians:

| Factory Classic fixture | Baseline sampled FPS | Final sampled FPS | Baseline opaque GPU ms | Final opaque GPU ms |
|---|---:|---:|---:|---:|
| Bridge | 218.72 | 334.50 | 3.69 | 2.09 |
| Lava | 185.20 | 268.22 | 4.73 | 3.07 |

Opaque GPU time falls approximately 43%/35% at bridge/lava, with the same stable
rounded fragment counts (2.57M/1.18M), sixteen cache chunks, matching metadata
revisions and no pending writes. All four timing previews verified Desktop 4
placement and cleanup, without shader errors. This is a factory-Classic result;
it must not be presented as a KR gain or compared directly with the earlier
customized Classic row. The original 400/144 FPS targets remain open, including
raw frame-time minima, movement and GTX 1060-class validation.

Raw data uses `release-kr-passes-factory-normals-{initial,repeat}-*`,
`gpu-passes-factory-normals-{initial,repeat}-*` and
`factory-normals-accepted-performance.json` under the temporary evidence folder.

The native matrix has 31 accepted comparisons: six each for factory Classic,
customized Classic, Enhanced EC, KR EC and Enhanced CC, plus Classic lighting
combined with KR-profile shadow weathering at lava. All except KR EC are
byte-identical; KR EC stays within one byte per channel. An intentional negative
control that skips normals for every Classic combination changes 102,707 pixels
by more than one byte (maximum ten), demonstrating that the exception fixture
actually detects the lost shadow response. The negative control is not shipped.
All nine capture previews and four timing previews verified placement/cleanup,
with no shader errors. The complete index is `normal-guard-accepted-evidence.json`.
No Rust or binding changes require a rebuilt executable; shaders load at runtime.

Do not require an arbitrary distinct-color count to accept a capture: the
valid CC bridge/swamp controls have only 4,819/2,969 colors. Reject uniform clear
frames and compare matched controls instead. A corrected temporary harness
rechecked all six existing CC captures as byte-identical without rerendering.

Sharing the weathering texture/noise once across material layers was also
measured in a separate release pair. Six matched KR captures stayed within one
byte, but opaque GPU time improved only about 1.5% at bridge and 3.4% at lava.
That larger shader refactor was removed. Its `shared-weathering-*` artifacts
are diagnostic evidence, not the accepted renderer.

## Rejected liquid-only fragment normals

A conservative shader-only prototype delayed geometry-normal reconstruction
until after opaque-layer and bank selection. It skipped the normal only for a
single remaining, resolved flat-liquid pass with zero bank weight. Missing
slots (`texture_size=3`), mixed stacks and banks retained regular shading. The
actual `TileUniform` matters: raw wet ownership or liquid class alone cannot
prove that the normal is unused. Noncomposed EC liquids used the same check.

The policy passed 39 native comparisons: the six fixtures in Classic, Enhanced,
KR, KR perspective, Enhanced CC and forced uncached KR; two synthetic missing
liquid-slot scenes; and Classic lighting with KR shadow grunge at lava. Classic,
Enhanced and CC comparisons were exact; KR and the missing-slot diagnostic
stayed within one byte. The Classic/KR-grunge exception was exact as well.

The same release/settings methodology gives these opaque GPU durations:

| KR orthographic fixture | Baseline ms | Prototype ms | Comparison |
|---|---:|---:|---|
| Bridge | 7.32 | 6.98 | Two run medians per variant, ABBA |
| Lava | 9.81 | 9.42 | Two run medians per variant, ABBA |
| Swamp | 4.89 | 3.62 | One reverse-order pair |
| Town | 9.33 | 8.92 | One reverse-order pair |

Swamp's sampled FPS rose from 178.93 to 223.88. Perspective bridge/lava also
improved, but factory Classic regressed in a repeat with background compilation
finished: bridge 2.05→2.19 ms, lava 3.08→3.14 ms. An earlier Classic pair had
concurrent Rust compilation and a large FPS/GPU-duration mismatch; that pair
is excluded from the performance conclusion.

Moving Classic's uniform no-normal check ahead of the varying liquid decision
did not eliminate the Classic GPU cost in a separate pair (2.135→2.195 ms at
bridge, 3.06→3.16 ms at lava). The cause of that residual cost was not established;
shader register allocation and control flow are hypotheses. Neither prototype
was retained. The shipped `land_surface_normal` and fragment composition remain
as before this investigation. This is not a claimed production speedup.

Evidence uses `liquid-normals-*` under the temporary fixture folder. The first
prototype's accepted image matrices are `liquid-normals-first-matrix.json`,
`liquid-normals-standard-matrix.json`, `liquid-normals-missing-matrix.json` and
`liquid-normals-exception-matrix.json`. Timing sets are `initial`, `wide`,
`perspective`, `factory-repeat` and `uniform-factory`; `factory` is the excluded
busy comparison. Preserve these distinctions when reconsidering this approach.

A separate [cache-only compiler-footprint diagnostic](ec_terrain_shader_footprint.md)
removed the cold reconstruction call while preserving settled fixture output.
It reduced static code/scratch substantially but changed opaque GPU time by less
than one percent. That result does not justify a split cache-hit/miss render path;
active height/material work needs further attribution.

## Exact transition-grime neighbor cache

The active-work probes in [ec_terrain_shader_footprint.md](ec_terrain_shader_footprint.md)
identified transition grime as a useful target. The retained implementation
caches only its eight neighbor decisions for each shaded layer and owning cell,
using the completed stack's former sort-key word. It preserves raw aliases,
wet/reviewed-liquid compatibility and original floating-point weight arithmetic.
GPU allocation remains 9,461,760 bytes; liquid snapshots add about 18 KiB of
bounded CPU storage. Uncached terrain and an extra bank material keep the
original lookup path. See [ec_terrain_stack_cache.md](ec_terrain_stack_cache.md)
for representation and publication requirements.

Waiting for resident flags exposed a startup binding dependency. The first
cache-image resize must invalidate the shared land material once; otherwise it
can keep the sentinel GPU view despite sixteen CPU-published chunks. A diagnostic
that marks actual cache misses magenta showed 308,234 affected bridge pixels
on the first visit and none after revisiting. After the one-time rebind, the
same first/lava/revisit probe has zero marked pixels in all three captures.
Ordinary same-size bakes reuse the GPU texture/view and do not mutate materials.
The initial `grime-initial` and `grime-order` timing sets expose this defect and
are excluded from the accepted comparison.

The accepted stable-release executable is rebuilt from the current source;
its SHA256 is `44f02d3398d16a88b2d84404b506776346739acf1245d50c3c57ad841f287256`.
Both shader variants use this executable and identical new CPU preparation.
The baseline retains the old neighbor reads; the final shader consumes cached
decisions. Settings remain KR morning, paused EC art/map, 1280×800, zoom 0.8,
uncapped Desktop 4, twenty-second settling and twelve-second sampling. All
windows have sixteen published chunks, matching metadata revisions, no pending
writes and stable, matching rounded fragment counts. Orthographic values average
two run medians per variant in baseline/final/final/baseline order; perspective
has one pair. Durations and FPS retain the smoothed diagnostic semantics above.

| KR camera / fixture | Baseline sampled FPS | Final sampled FPS | Baseline opaque ms | Final opaque ms |
|---|---:|---:|---:|---:|
| Orthographic bridge | 121.33 | 139.83 | 7.295 | 6.215 |
| Orthographic lava | 95.76 | 113.84 | 9.788 | 8.085 |
| Orthographic town | 98.14 | 115.81 | 8.895 | 7.328 |
| Perspective bridge | 112.52 | 131.15 | 7.880 | 6.650 |
| Perspective lava | 92.67 | 111.07 | 10.095 | 8.315 |

The orthographic repeats establish roughly 15%/19%/18% higher sampled FPS
and 15%/17%/18% lower opaque GPU time at bridge/lava/town. The perspective pair
shows a similar benefit with less repetition. A factory-Classic control has
opaque times 2.12→2.13 ms at bridge and 3.13→3.06 ms at lava, with no material
GPU regression. Its sampled FPS is 226→228 and 233→271 respectively; those
deltas do not track GPU time closely enough to claim a Classic speedup. The
new executable also includes newer unrelated source changes, so its absolute
Classic FPS is not directly comparable with the older 334-FPS measurement.

The 400/144 targets, raw frame-time minima, movement and GTX 1060-class validation
remain open. The accepted raw timing sets are `release-kr-passes-grime-final-*`
and `gpu-passes-grime-final-*`; `grime-final-performance-index.json` gathers the
twenty fixture/run summaries. All eight timing previews verified placement and
cleanup without shader errors. The standalone startup probe is indexed by
`grime-startup-{before,final}-result.json` and the corresponding preview metadata.

All 136 renderer tests pass, including the Vulkan record/order, alpha-bit and
grime-weight comparison. The native matrix has 48 comparisons across the six
fixtures: all three styles, KR perspective, Enhanced CC, forced uncached KR,
cached-versus-uncached final KR and one-upload-per-frame final KR. Classic,
Enhanced and CC are byte-identical; KR stays within one channel byte. All
thirteen capture previews verified Desktop 4 and cleanup, with no shader errors.
The three `grime-native-{kr,other,controls}-matrix.json` files and
`grime-native-{cache,upload}-control.json` retain the comparisons and metadata.

The next architectural audit is documented in
[terrain_shader_work_placement.md](terrain_shader_work_placement.md). The vertex
normal path does not yet replace Enhanced/KR fragment reconstruction. Reusing
the current one-vertex-per-tile normal is a measured lighting diagnostic, not
an accepted fidelity-preserving optimization. Larger gains need a deliberate
normal/height representation and an attribution of hidden terrain pixel work.
