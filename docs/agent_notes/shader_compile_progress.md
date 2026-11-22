# Shader compiles and their overlay

`render/shader_compiles.rs` keeps one book of the pipelines compiling and `ui/overlays/shader_compiles.rs` draws it
bottom right ("Compiling shaders done/total", a bar, the three running longest by name, their seconds once over 1 s).
A batch shows once one of its compiles has run 0.3 s and stays 0.5 s after its last one ends; a compile ready after
more than 1 s is logged at info level (`Shader compile: <name> ready after <s> s.`).

Two sources feed it:

- Bevy's render-world `PipelineCache`, synced in `RenderSystems::Cleanup`: an entry of `waiting_pipelines()` is
  compiling, one that left it is done (failed when its state is `Err`). Its name is the descriptor label and the file of
  its fragment (or compute, mesh) shader, as `pbr_opaque_mesh_pipeline (ground.wesl)`.
- The app's own wgpu compiles outside the cache: hold `begin_shader_compile(name)` around the creation, or call
  `shader_compile_started(name)` and `shader_compile_ended(id)` from any thread (plain data, no Bevy types; meant for a
  warm-up callback). Fed now: the GPU compute warm-up (`mobiles/compute_warm_up.rs`), every pipeline of
  `lib/support/uot-gpu-compute` by its label, on up to three threads side by side: RIFE's at startup (`RIFE conv
  Stride1/Leaky tile 64x32`, `RIFE pixel step: merge`), SPAN's at startup (`upscale conv 3x3/Silu tile 64x48`,
  `upscale prepare`), and the block encoder's (`BC7 encoder (UltraFast)`) only while a loaded animation package
  is BC7, from the first frame one is (below); the RIFE network when the warm-up did not give it,
  and the atlas steps (`rife_in_betweens/render.rs`, background pool); the static picking MSAA resolve
  (`gpu_picking.rs`, render thread, only with MSAA). The SPAN pipelines are warmed but not drawn with yet
  (2026-11-21).

## The GPU compute pipelines, one by one

Measured on the Radeon 880M (RADV STRIX1) in a release probe outside the app (2026-11-21), each pipeline created
alone in a fresh process; cold is a new empty `MESA_SHADER_CACHE_DIR`, warm the same folder again; the median of 3
(range):

| pipeline | cold | warm |
|---|---:|---:|
| block_compression BC1 | 118 ms (117–121) | 1.5 ms |
| BC2 | 128 ms (124–130) | 1.5 ms |
| BC3 | 133 ms (131–144) | 1.7 ms |
| BC4 | 13 ms (13–14) | 0.7 ms |
| BC5 | 22 ms (21–24) | 0.7 ms |
| BC7 (one pipeline for every quality) | 20.64 s (20.54–20.75) | 4.3 ms |
| `GpuBlockCompressor::new` (all six, serial) | 21.20 s (20.95–21.28) | 24 ms |
| `MobileBlockEncoder` (the compressor and the BC1 punch-through) | 20.91 / 20.95 s (2 runs) | 25 ms |
| RIFE and SPAN (`uot_gpu_compute::warm_up`) | 1.29 s (1.29–1.32) | 84 ms |

The naga parse and validation of the modules is 4 ms (bc1_to_5.wgsl) and 10 ms (bc7.wgsl) a process, cold or warm.
BC1 to BC5 together are 0.4 s, 2 % of the compressor. These rows are block_compression's own pipelines, which the
app no longer builds: uot-gpu-compute runs its own copies of the crate's BC7 and BC1 kernels (below), and the crate is
a dev-dependency, the reference the copies are checked against. In the app the compressor took 27 to 50 s cold (37 s in an
earlier run), with Bevy's own pipelines compiling at the same time on its async compute thread and the machine more or
less busy.

**RADV compiles pipelines of two threads side by side.** BC7 on one thread and RIFE with SPAN on another: 20.8 s wall
(20.5–21.1) against 22.2 s (21.9–22.3) one after the other, each as long as alone (RIFE and SPAN 1.32–1.42 s, BC7
20.4–21.1 s); BC1–BC5 against BC7: 20.7 s against 21.1 s summed. So the block encoder, RIFE and SPAN each have a
thread of their own and none waits for another.

In the app (dev build, Britain, Desktop 4, one run each on a shared machine, 2026-11-21), the warm-up's threads as
logged, and the world's land/art pipelines ready (`main.wesl`, `ground.wesl`; the window draws from 2.5 s on):

| build | packages | start | warm-up threads | world pipelines ready |
|---|---|---|---|---|
| one thread for all, every package | RGBA8888 | cold | 52.0 s (BC compressor 49.9 s; machine busy) | 50.1–51.1 s |
| | BC7 | cold | 28.1 s (BC compressor 26.6 s) | 32.3–33.0 s |
| | either | warm | 0.1–0.3 s | |
| encoder only with BC7, its own thread | RGBA8888 | cold | networks 1.4 s, no BC compile | 31.3–32.0 s |
| | BC7 | cold | networks 1.4 s, encoder 29.9 s (with BC1–BC5) | 34.6–35.2 s |
| three threads, BC7 only | RGBA8888 | cold | RIFE 1.0 s, SPAN 0.5 s, no BC compile | 31.8–32.4 s |
| | BC7 | cold | RIFE 0.9 s, SPAN 0.5 s, encoder 26.5 s | 32.2–32.9 s |
| | either | warm | 0.0–0.1 s each | |

The world waits for Bevy's own pipelines, not for these: with the machine quiet the BC7 compile costs the world about
a second, and with RGBA8888 packages nothing of block_compression compiles at all.

**RIFE on the matrix units (2026-11-22)** compiles less: 14 convolution compiles instead of 20: 8 matrix kernels and 6
on the shader cores (the Head's 3, and 3 for grids under 8 pixels, whose split-K reductions the matrix kernels
share), the pixel and frame steps unchanged. `warm_up::time_the_warm_up`
(dev test build, a new `MESA_SHADER_CACHE_DIR` for cold and the same again for warm, two runs alternated with dev
5c1b85d7's build):

| | dev 5c1b85d7 | RIFE on the matrix units |
|---|---:|---:|
| RIFE pipelines, cold | 831 / 856 ms | 469 / 473 ms |
| SPAN pipelines, cold | 115–118 ms | 114 ms |
| whole warm-up (RIFE and SPAN), cold | 1010 / 1034 ms | 637 / 641 ms |
| whole warm-up, warm | 81 / 86 ms | 56 / 58 ms |

A matrix kernel compiles in 23 to 34 ms, a shader-core one in 29 to 47 ms. In the app the warm-up's line reads "The
RIFE pipelines are compiled for the Matrix, in X s".

## Cold start: the world waits for the window's pipelines

With Mesa's cache empty (RADV, Radeon 880M, 2026-11-21) the startup pipelines take about 35 s (up to 48 s with the
machine busy), most of it the first land/art `main.wesl` pipeline. `app.rs` gives Bevy's async compute pool one thread
and Bevy compiles its queued pipelines on that pool, in the order it spawned them, so a pipeline waits for every one
spawned before it. The app did not freeze at the start (frames ran at about 50 fps), but the window stayed blank: the
blit to the window, egui's pipeline, the tonemapping and the screenshot pipeline were queued in the same frames as the
world's and came out only at the end, with the overlay of the compiles. Bevy's upscaling prepare calls
`block_on_render_pipeline` on a view's blit when its key is new; in the first frame that returns at once (the blit's
shader is not loaded yet), but a blit for a new format requested while compiles are in flight waits for all of them:
switching to night 8 s into a cold start (the light buffer camera, whose image has its own format, turning on) froze
the app for 26 to 30 s. No other `block_on` runs on the frame path, in Bevy 0.20 (egui's pipelines are read with
`get_render_pipeline`) or in the app.

`render/world_draw_hold.rs` holds the world back once, at the start: every `Camera3d` (the world camera and the light
buffer camera) starts active on no render layer, so its pipelines are not even requested but its blit is, until the
render world reports the window's pipelines ready (every view drawing to a target has requested its blit and no
pipeline is waiting), at most `HOLD_LIMIT` (10 s); then each gets its own layers and activity back. The first frames
queue only Bevy's own and the window's pipelines, about 41, ready in 0.6 to 0.9 s even cold; the world's queue behind
nothing, the window and the overlay draw while they compile (one batch with the GPU compute warm-up, which keeps it
open) and the world fills in as they arrive. While held, `WaitingRenderPipelines` counts the hold as one pipeline, so
the export and the control channel's readiness wait for it. Measured from the launch in a headless KWin (below), with
the 1280 x 800 session at Britain (1488, 1686), each style's afternoon; cold one run each, warm the median of 5:

| start | style | first presented, before | after | world drawn, before | after | longest frame until then, before | after |
|---|---|---|---|---|---|---|---|
| cold | Classic | 36.7 s | 2.4 s | 37.2 s | 36.3 s | 331 ms | 302 ms |
| cold | Enhanced | 35.2 s | 2.9 s | 36.0 s | 36.1 s | 304 ms | 305 ms |
| cold | KR-like | 35.2 s | 2.7 s | 36.3 s | 36.1 s | 318 ms | 320 ms |
| cold | UOD-Painterly | 35.2 s | 2.9 s | 36.0 s | 36.2 s | 309 ms | 314 ms |
| warm | Classic | 3.0 s | 2.3 s | 3.2 s | 3.1 s | 215 ms | 209 ms |
| warm | Enhanced | 2.8 s | 2.6 s | 3.4 s | 3.4 s | 222 ms | 214 ms |
| warm | KR-like | 3.5 s | 2.6 s | 3.5 s | 3.3 s | 241 ms | 224 ms |
| warm | UOD-Painterly | 2.8 s | 2.6 s | 3.4 s | 3.1 s | 221 ms | 218 ms |

The night switch 8 s into a cold start: before, one 26.5 s frame; after, at most 301 ms.

**More async compute threads do not shorten it.** The one thread dates from c3c5bc6e (2026-03-22, "Memory savings"):
Bevy's default of a thread per CPU left about 20 idle threads, and 0bd65dce/0251834c tried 4 and went back; nothing
in the app spawns on that pool, only Bevy's pipeline compiles and its screenshot readback. Measured with the pool at
1, 2, 4 and 20 threads (an environment override in a measurement build, not kept), the same headless KWin as below,
Enhanced afternoon at Britain (1488, 1686), two runs each (2026-11-21):

| start | async compute threads | world drawn | pipelines done | longest frame until then |
|---|---:|---:|---:|---:|
| cold | 1 | 35.8 s | 35.8 s | 394 ms |
| cold | 2 | 35.1 s | 35.0 s | 267 ms |
| cold | 4 | 35.3 s | 35.1 s | 252 ms |
| cold | 20 | 35.2 s | 35.1 s | 235 ms |
| warm | 1 | 3.4 s | 3.5 s | 251 ms |
| warm | 2 | 3.4 s | 3.4 s | 242 ms |
| warm | 4 | 3.4 s | 3.5 s | 237 ms |
| warm | 20 | 3.4 s | 3.5 s | 221 ms |

(One recording of the 20-thread cold runs never settled; its pipelines were done at 35.1 s like the other.) The cold
start is one pipeline: the first land/art `main.wesl` pipeline alone takes about 32 s in RADV however many others
compile beside it, and the rest are ready with it. 0.7 s (2 %) is within the run-to-run spread, so the pool keeps its
one thread.

The screenshot is the one thing still behind the batch: Bevy maps its readback buffer in a task on the same async
compute pool, so a capture requested during the cold compiles completes after them (the frame is captured at once).

## The land pipeline: shade_land inlined ten times

The 32 s pipeline is the land's (`shaders/world/land/main.wesl`, `LandMaterialExtension`). It has no shader def of its
own: styles are uniforms, so one land pipeline serves all four styles, and only one compiles. RADV's statistics
(`RADV_DEBUG=shaderstats`, Mesa 26.2.7, ACO) of the cold Enhanced start at dev 3eee2b00: 120 shaders, 398,839
instructions, of which the land's pixel shader 286,211 (256 VGPRs, 44,032 bytes of scratch, 1.51 MB of code, 5,032
branches, 7,554 SGPRs spilled). The next largest are the art and ground pixel shaders at 12,753 and 8,841; every other
shader is under 6,000.

Bevy creates material shader modules unchecked (`ValidateShader::Disabled`, `ShaderRuntimeChecks::unchecked()`), so
wgpu's loop bounding is already off for this module: it is not the cost. The cost is inlining. NIR inlines every
function call, so each call site of a function is a copy of its body. `shade_land` (sampling, blur, sharpening, bump,
grunge, KR micro-contrast, the three shading modes, light decals, transition blends) had six call sites, four in
`compose_ec_terrain`, which itself has two calls (cache hit and miss): ten copies. Bisected on the composed WGSL in a
standalone wgpu 30 probe (a pipeline with the app's 20 shader defs, a fresh `MESA_SHADER_CACHE_DIR` per compile, the
app's cold time reproduced):

| land fragment shader | pixel shader instructions | compile |
|---|---:|---:|
| dev 3eee2b00 | 286,940 | 32.5 s |
| `shade_land` returning its input | 10,645 | 0.8 s |
| one `shade_land` call site | 65,385 | 3.8 s |
| one call site, the eight-neighbour EC transition blend stubbed | 35,478 | 1.7 s |

So `shade_land` is about 276,000 of the 287,000 instructions. The EC transition blend (eight unrolled neighbour
material samples) is about 30,000 of a copy that can run it, but only the fragment's own tile runs it: the composed
passes call `shade_land` with a literal `ordered_coverage` of true, which drops it. The compile time grows faster than
the size.

**Committed: the neighbour blend loops.** `blend_ec_terrain_transitions` loops over a constant table of the eight
neighbour offsets, in the old order (west, east, north, south, then the diagonals) and with the same accumulation, so
its sums are the same: one inlined material sample, not eight. The probe's compile goes from 32.5 to 25.9 s (260,612
instructions, 7,208 spilled SGPRs) and the app's cold start by about 6 to 10 s (below), with no frame cost.

**Committed after the neighbour loop: one `shade_land` call site for the passes.** The rest is the ten copies. `compose_land`
replaces `compose_ec_terrain` and the two `shade_land` calls of `land_color`: it works out the passes first (the lead,
taken whole; an optional second one, mixed by its alpha: the all-liquid chunk's own layer under the bank, or without
EC transitions the dry bank; then the coverage layers above the lead) and runs them through one `shade_land` call in a
loop, in the old order. `land_color` calls it twice, with a literal `cached` (the cache hit, or no EC coverage at all;
and the uncached reconstruction): two copies. It costs Enhanced a little frame time (below), taken for the cold start. The
variants, Enhanced cold start in the probe:

| land fragment shader | instructions | code | branches | spilled SGPRs / VGPRs | probe compile |
|---|---:|---:|---:|---:|---:|
| dev 3eee2b00 | 286,940 | 1.52 MB | 5,047 | 7,554 / 0 | 28.9 to 32.5 s |
| neighbour loop (committed) | 260,612 | | | 7,208 / 0 | 25.9 s |
| pass loop alone, neighbours unrolled | 104,661 | | | 192 / 586 | 11.3 s |
| both loops, two `shade_land` copies | 76,314 | 0.39 MB | 1,757 | 113 / 1 | 4.2 to 4.6 s |
| both, the lead pass outside the loop (four copies) | 138,690 | 0.72 MB | 2,902 | 2,207 / 0 | 13.9 s (machine busy) |
| both, one copy (`cached` a runtime value) | 48,864 | 0.25 MB | 1,243 | 84 / 0 | 2.3 to 2.4 s |

Cold starts in the app (pipelines done, one run each, the headless KWin below at Britain, 2026-11-22):

| style | dev 3eee2b00 | neighbour loop | both loops, two copies | one copy |
|---|---:|---:|---:|---:|
| Classic | 38.3 s | 28.0 s | 12.5 s | 7.4 s |
| Enhanced | 35.6 s | 28.8 s | 11.4 s | 7.8 s |
| KR-like | 35.5 s | 29.1 s | 11.4 s | 8.1 s |
| UOD-Painterly | 38.3 s | 28.9 s | 11.2 s | 9.0 s |

Warm frame times (`frame_benchmark.py`, 1900 x 1000, the same binary with each tree's shaders on a warmed Mesa cache,
median ms of 2 launches; Enhanced again with 3 launches and the neighbour loop alone):

| camera | style | view | dev | both loops | change | Enhanced again: dev | neighbour loop | both loops |
|---|---|---|---:|---:|---:|---:|---:|---:|
| ortho | Classic | city-britain | 7.56 | 7.59 | +0.4 % | | | |
| ortho | Classic | land-mountains | 5.28 | 5.30 | +0.4 % | | | |
| ortho | Enhanced | city-britain | 18.90 | 18.96 | +0.3 % | 19.17 | 19.13 (-0.2 %) | 19.18 (+0.1 %) |
| ortho | Enhanced | land-mountains | 14.64 | 14.86 | +1.5 % | 14.74 | 14.69 (-0.4 %) | 14.96 (+1.4 %) |
| ortho | KR-like | city-britain | 24.85 | 24.44 | -1.7 % | | | |
| ortho | KR-like | land-mountains | 15.51 | 15.22 | -1.9 % | | | |
| persp | Classic | city-britain | 7.68 | 7.62 | -0.7 % | | | |
| persp | Classic | land-mountains | 5.29 | 5.31 | +0.3 % | | | |
| persp | Enhanced | city-britain | 17.94 | 19.04 | +6.1 % | 19.10 | 18.84 (-1.4 %) | 19.50 (+2.1 %) |
| persp | Enhanced | land-mountains | 14.64 | 15.13 | +3.3 % | 15.07 | 14.73 (-2.2 %) | 15.10 (+0.2 %) |
| persp | KR-like | city-britain | 24.54 | 24.42 | -0.5 % | | | |
| persp | KR-like | land-mountains | 16.82 | 16.75 | -0.4 % | | | |

Per launch, Enhanced land-mountains with both loops was slower in every one (0.06 to 0.58 ms); the neighbour loop
alone never was, so the cost is the pass loop's. The one-copy variant cost 1.0 to 21.4 % in every style and is
rejected.

The regression harness (full tier, all styles, 493 captures) with both loops: 452 identical, 37 within tolerance
(at most 1 level), 4 changed, all acid-pools in KR-like and UOD-Painterly over Classic land (cc/cc, ec/cc): 0.010 to
0.017 % of the pixels, at most 18 levels, on single-pixel lines along the tile seams. A rerun of each tree reproduces
both images exactly, but the pass loop alone (neighbours unrolled) changes them differently from launch to launch, and
putting the lead pass back outside the loop changes them too (at most 11 and 18 levels). Those pixels lie on the 2 x 2
quads across a seam, whose lanes sit on two tiles; most likely (not traced to the instruction) a derivative there reads
a lane that took another path: Classic textures take their footprint with `fwidth(uv)`/`dpdx(uv)` of the per-tile UV
(`blurred_albedo`, `apply_sharpening`) and implicit-LOD samples, so the value depends on how the compiler laid the
shader out, and dev's is no more defined than the loop's. The neighbour loop alone leaves the acid-pools captures
identical; over the whole full tier it gives 476 identical and 17 within tolerance (at most 2 levels, EC land in
Enhanced, KR-like and UOD-Painterly), none changed.

The main.wesl comments at the pass loop and the neighbour loop say why they are loops; keep every `shade_land` and
material sample at its one call site. The full tier against dev 72c65d90 when it was committed: 423 identical, 66
within tolerance (at most 2 levels, mostly KR-like and UOD-Painterly), and the same 4 acid-pools captures changed
(0.010 to 0.017 %, at most 18 levels).

## What the cold start compiles now

With the pass loop committed (2026-11-22), a measurement build that logs every pipeline as it becomes ready
(`SLOW_COMPILE` at zero, not kept), the headless KWin below at Britain, each style's afternoon, one cold run each:

| style | pipelines done | land pipeline | art `main.wesl` pipelines | ground pipelines |
|---|---:|---:|---:|---:|
| Classic | 10.9 s | 7.4 s | 3: 0.47 s | none at Britain |
| Enhanced | 11.3 s | 7.5 s | 3: 0.40 s | 2: 0.25 s |
| KR-like | 11.7 s | 7.5 s | 3: 0.57 s | 2: 0.25 s |
| UOD-Painterly | 11.5 s | 7.5 s | 3: 0.40 s | 2: 0.27 s |

The times are the gaps between pipelines becoming ready on Bevy's one compile thread. Until about 2.3 s the app loads
and nothing compiles; Bevy's own and the window's pipelines (64 to 66 logged) are ready by 3.1 s, none over 0.15 s and
0.6 to 0.7 s in all; then the land, then the art and ground pipelines (0.02 to 0.37 s each). RIFE and SPAN compile on
their own threads beside them (ready by 3.3 s). The land takes 7.4 to 7.5 s here against the probe's 4.2 to 4.6 s
because the harness runs the app at idle priority on the second CPU cluster, beside the warm-up threads; its share of
the start is the same.

The next levers, measured:

- **Two async compute threads**: pipelines done at 10.5 to 10.9 s against 10.9 to 11.7 s (same runs, `UOD_ASYNC_THREADS`
  override in the measurement build, one run each), as the art and ground compile beside the land. The pool kept its
  one thread when the land was the whole start (above); 0.4 to 0.8 s is now 4 to 7 % and worth a repeated measurement.
- **The land's two `shade_land` copies** (cached and uncached) are the rest of the land's time. One copy with `cached`
  a runtime value costs frame time (above); one copy for free needs the uncached reconstruction to stop being a second
  shading path. A shader def per shading mode is not a lever: the probe's land pixel shader with `effects.shading_mode`
  a constant has 71,267 (mode 0), 72,749 (1) and 75,688 (2) instructions against 76,314, and compiles in 4.1 to 4.2 s
  against 4.4 s (one run each), while every style switch would compile a land pipeline of its own.
- **The BC7 block encoder** was the largest compile left anywhere, though off the world's path and only with a BC7
  animation package (below): block_compression's `compress_bc7` reads its `BC7Settings` from a storage buffer, so the
  one pipeline holds every mode, while the mobiles encode at UltraFast only (`MOBILE_BC7_QUALITY`, bc7_encoding.rs:
  modes 4/5 and 6). With the settings a WGSL `const` of UltraFast's values, block_compression's runtime checks, one
  core, a fresh Mesa cache per run (median of 3): 4.50 s (4.50 to 4.68) against 22.34 s (21.76 to 22.66). Done
  (2026-11-22) with the modes only as constants, at 6.6 s: see [Our block encoder](#our-block-encoder).

## Our block encoder

`uot-gpu-compute` runs its own copies of block_compression 0.11.0's kernels (`shaders/block_encode/bc7.wgsl` and
`bc1.wgsl`, MIT, CREDITS.md), one pipeline per `MobileBlockFormat` (`Bc7(quality)`, `Bc7Opaque(quality)`, `Bc1`),
each compiled the first time it is used. dynamapper compiles `Bc7(UltraFast)` only. A BC7 profile's modes (which of
the groups 0/2, 1/3/7, 4/5 and 6 it tries, and whether it skips mode 2) are WGSL constants appended to the source
(`Bc7Profile::wgsl_constants`), so the driver drops every mode the profile never runs. Its numeric settings
(refinement iterations, fast-skip thresholds, channels) stay in the crate's storage buffer: as constants too (the 4.5 s
above), 502 of 188,169 blocks of UltraFast with alpha and 32 of Slow opaque came out different from the crate's (the
compiler folds and reorders the float math), and with them in the buffer every profile is the crate's byte for byte
(`our_kernels_encode_as_block_compression`, block_encode/tests.rs, on real atlas page crops and frames).

Cold, a new empty `MESA_SHADER_CACHE_DIR` per run, the time of `warm_up::tests::time_the_warm_up`'s block encoder
(test binary, Radeon 880M, other agents' previews running; 2026-11-22, three runs):

| block encoder | cold | warm |
|---|---:|---:|
| block_compression's compressor (BC7 every quality) | 23.0 s (21.9–24.6) | 20 ms |
| with BC1–BC5 and the BC1 punch-through (the old `bc1` feature) | 26.2 s (22.8–28.7) | 25 ms |
| ours, `Bc7(UltraFast)` | 6.6 s (6.5–8.0) | 15 ms |
| ours, `Bc1` (after it) | 0.28 s (0.25–0.28) | 6 ms |

Each format compiled in turn in one process (`block_encode::tests::time_every_format_compile`, cold): profiles with
the same modes give the same source, which Mesa's cache then hands back in about 12 ms (alpha Fast is VeryFast's
modes, opaque Basic alpha Basic's and opaque Slow alpha Slow's):

| format | cold, alpha | cold, opaque |
|---|---:|---:|
| UltraFast (modes 4/5, 6; opaque 6) | 5.65 s (5.37–6.61) | 0.98 s (0.96–0.98) |
| VeryFast (1/3/7, 4/5, 6; opaque 1/3/7, 6) | 14.5 s (14.0–15.6) | 5.6 s (5.6–5.7) |
| Fast (VeryFast's modes) | 13 ms | 13 ms |
| Basic (every mode, mode 2 skipped) | 18.4 s (17.5–18.5) | 12 ms (alpha Basic's) |
| Slow (every mode) | 21.8 s (21.7–22.0) | 12 ms (alpha Slow's) |
| `Bc1` | 0.25 s (0.23–0.25) | |

Warm, every format takes 12 to 15 ms (BC1 5 ms). A converter that wants Slow pays about what the crate's one pipeline
cost; dynamapper pays UltraFast's 5.4 to 8 s, on its own thread, and never the others.

In the app (dev build, Britain, Desktop 4, the BC7 animation packages, one run each, other agents' previews queued;
2026-11-22), the warm-up's `The BC7 block encoder is compiled, in X s.`: cold 26.1 s with dev's crate compressor, 7.1 s
with ours; warm 0.0 s with both. The window's pipelines were ready after 0.9 to 1.0 s cold and 0.5 to 0.6 s warm with
both.

The BC7 kernel's solid blocks and its CPU-like opaque blocks (2026-11-22, atlas_page_compression.md, "Optimizing the
GPU BC7 encode") leave the compile where it was: the same test, before and after alternating, two cold runs each on a
loaded machine, UltraFast with alpha 7.5 and 6.2 s before, 6.5 and 6.4 s after; opaque UltraFast 1.3 to 1.4 s both;
BC1 0.23 to 0.27 s both; the slower profiles within their noise (VeryFast 15.5 to 17.9 s, Basic 17.0 to 25.7 s, Slow
23.8 to 44.3 s, both sides). Turning the kernels' bounds checks off (`ShaderRuntimeChecks`) took 0.5 s off UltraFast's
cold compile and made its kernel 1.6 times as fast with the same blocks. It was taken after an audit of every index
(2026-11-22, atlas_page_compression.md, "Bounds checks off"); measured again then, the cold compile is level within
the noise (UltraFast 7.9 and 5.5 s before, 4.8 and 5.3 s after, alternating on a loaded machine).
In the app (dev build, Minoc, the BC7 animation packages, an empty Mesa cache per launch, before and after alternating,
other agents' builds running), the warm-up's line read 11.9 and 14.4 s before, 12.4 and 13.6 s after: the whole
cache is cold there, so the encoder compiles beside every other pipeline. The renderer's quick tier is the same with
both (71 captures identical, 22 within tolerance; two Classic water captures caught the after binary's first, cold,
compile overlay and were identical when taken again).

## Atlas page formats and what compiles

Nothing Bevy compiles depends on a package's page format, so there is no RGBA8888 or BC7 pipeline to leave out. The
format is chosen where each texture is made: the Classic and EC land arrays (`land.rs`: `Rgba8UnormSrgb` or
`Bc7RgbaUnormSrgb`; the EC land atlas `Rgba8Unorm` or `Bc7RgbaUnorm`), the EC auxiliary layers (`aux_layers.rs`), the
art page atlases (`atlas_texture_format`, statics_draw.rs) and the mobiles' frame atlases (`AtlasFormat`,
frame_atlas.rs: the RGBA8 one bound as the first art atlas, the BC7 one as the Enhanced, `ART_LAYER_ENHANCED_ATLAS_BIT`
on the instance's layer). Every material binding declares a float filterable `texture_2d_array` (or `texture_2d`),
which a view of either format satisfies, and no shader def or pipeline key names a format (`ArtSpriteMaterialKey`
holds `mobile_frames` and `client_shadow`; the land material has no key), so one pipeline serves both formats.

What a format leaves dead is the other atlas's branch of the art shader: `art_page_sample` and
`art_page_client_filtered` (art/atlas_pages.wesl), and with `MOBILE_FRAMES` `mobile_frames_tap` and
`mobile_frames_client_filtered`, are inlined once per atlas. In the probe (art/main.wesl composed with the land's
defs, the page's `enhanced` forced false, median of 3 cold compiles, 2026-11-22):

| art pipeline | pixel shader instructions | compile |
|---|---:|---:|
| statics, both atlases | 12,742 | 0.20 s |
| statics, the first atlas only | 10,056 | 0.20 s |
| mobiles (`MOBILE_FRAMES`), both atlases | 18,602 | 0.37 s |
| mobiles, the RGBA8 atlas only | 13,039 | 0.29 s |

So a shader def for the formats loaded would save at most about 0.08 s per mobile pipeline, compiled after the land,
and would recompile the mobiles' pipelines (mobiles not drawn meanwhile) whenever a package of the other format
loads; the statics of the Enhanced styles draw from both atlases whatever the formats. Not done.

On the compute side the only format-dependent compile is the BC7 block encoder, started only while a loaded animation
package is BC7 (`sys_extract_block_encoder_wanted`, compute_warm_up.rs), on its own thread, or on the background pool
when a package needs it later; RGBA8 frames are copied into their atlas with nothing to compile. RIFE's and SPAN's
pipelines (and the RIFE frame steps, which read any float texture) serve both formats. The test packages are RGBA8,
so none of the runs above compiles the encoder.

## Testing with a cold cache

Point `MESA_SHADER_CACHE_DIR` at a fresh empty directory under the scratch folder for the launch (never delete
`~/.cache/mesa_shader_cache`); RADV keeps its pipeline cache there too. The control socket's captures cannot time the
start (see above), Xvfb cannot present with RADV (no DRI3) and ffmpeg cannot grab an XWayland window off the active
desktop (BadMatch). A private headless KWin works: `dbus-run-session -- kwin_wayland --virtual --xwayland --width 1280
--height 800 --socket <name> <holder script>` gives a `DISPLAY` with DRI3 where the app presents, and `ffmpeg -f
x11grab -draw_mouse 0 -window_id <id>` records it. Hold the preview lock around it. About one recording in eight froze
on one frame while the app ran on (before and after the hold alike): check the recording, not only its timings
(2026-11-21, scripts under the scratch folder's `cold-start/`).
