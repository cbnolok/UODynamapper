# Cached terrain shader footprint

The ownership cache retains uncached reconstruction for missing or unsettled
neighborhoods. A controlled RADV diagnostic removed only the fragment's call to
that reconstruction, marking a covered-material cache miss magenta. It kept
geometry, texture filtering, material selection, bank policy, lighting and
postprocessing in their existing paths. The production shader was restored
afterward; the diagnostic is not a valid streaming renderer.

All six settled KR orthographic fixtures matched the full shader within one
color byte, so the miss marker did not conceal an untextured region. Metadata
revisions agreed, no writes were pending, and sixteen chunks were published.
The terrain pipelines have identical reported vertex hashes; their fragment
statistics differ as follows on the measured AMD RADV adapter:

| Compiler field | Full renderer | Cache-only diagnostic |
|---|---:|---:|
| Code size | 780,456 | 462,356 |
| Instructions | 150,797 | 89,009 |
| Reported scratch size | 44,032 | 0 |
| Reported spilled SGPRs | 816 | 351 |
| VGPRs | 168 | 168 |
| Subgroups per SIMD | 6 | 6 |

These are driver-reported compile statistics, not executed instructions per
fragment or measured occupancy. Do not convert the scratch field into a
per-fragment allocation without checking the driver's reporting convention.
Compilation statistics were captured with the Mesa disk cache disabled;
that override was removed for the timing comparison.

The same stable-release executable, paused KR/EC configuration, 1280×800,
zoom 0.8 and uncapped Desktop-4 preview was used for a normal-cache timing pair.
Each fixture settled twenty seconds and sampled twelve. The reported opaque
GPU durations were 7.32→7.26 ms at bridge and 9.81→9.765 ms at lava: less than
one percent in both scenes. Rounded fragment counts matched their controls
(2.57M/1.18M), with complete metadata submission. This does not establish a
worthwhile production speedup despite the much smaller static footprint.

Splitting cache hits and misses into separate render submissions would add
geometry and guard work, plus streaming and pipeline-readiness obligations.
These measurements do not justify that change. They also do not establish its
cost on other GPUs, larger views or moving cameras. Retain the existing fallback.

## Further profiling

The next measurements should attribute actively executed terrain work. Source
candidates include the fragment's bicubic height normal (sixteen height queries),
bent normal (five), contact and cliff shadows (four each), and transition grime
(eight metadata queries). Shared coordinates do not prove that the compiler
shares the reads, and source helper-call counts do not prove actual GPU traffic.
The earlier direct-height-query refactor in `renderer_release_performance.md`
had no established speedup. Use isolated diagnostics and hardware pass timings
before introducing a new precomputed neighborhood representation.

Evidence under `/tmp/uodynamapper-ec-followup/` uses `cache-footprint-*`.
`cache-footprint-native.json` records the six comparisons and compiler fields;
the complete raw stage lists are `cache-footprint-{full,cached}-shader-stats.json`.
`cache-footprint-performance.json` and `gpu-passes-cache-footprint-*` record the
release pair. All four diagnostic/timing previews verified Desktop 4 and cleanup,
with no shader errors. This experiment used the shipped shader, independently
of the rejected liquid-normal prototypes.

## Active neighborhood attribution

A subsequent diagnostic used one frozen release executable and changed only
isolated shader function bodies. Replacing the fragment normal with the existing
interpolated vertex normal leaves vertex work intact. Disabling relief returns
zero from contact/cliff shadow helpers; disabling grime returns its input color.
These alter lighting and are attribution probes, not accepted visual changes.

| KR orthographic fixture | Production opaque ms | Vertex-normal probe | Relief-off probe | Grime-off probe |
|---|---:|---:|---:|---:|
| Bridge | 7.305 | 5.56 | 6.57 | 4.99 |
| Lava | 9.81 | 8.12 | 8.65 | 6.26 |
| Town | 8.90 | 7.18 | 7.84 | 5.77 |

All use paused KR morning, EC art/map, 1280×800, zoom 0.8, twenty-second settling
and twelve-second sampling. Opaque durations retain the smoothed, rounded
diagnostic semantics. Rounded fragment counts stay constant (2.57M/1.18M/2.96M),
with sixteen published chunks, matching metadata revisions and no pending writes.
The twelve owned previews verified Desktop 4 and cleanup without shader errors.
Raw data is `height-queries-profile.json` and `gpu-passes-height-queries-*` in
the temporary evidence directory; its recorded executable hash identifies the
pre-rebuild attribution baseline.

The grime probe identifies a substantial active cost while preserving none of
that effect. The exact neighbor-mask cache in `ec_terrain_stack_cache.md` instead
preserves its eight decisions and original weight arithmetic. Production gains
must be measured separately: diagnostic differences are not additive costs and
do not predict the retained effect's shader allocation or full frame rate.
