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
  `upscale prepare`), and the block encoder's (`BC7 compressor (every quality)`) only while a loaded animation package
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
BC1 to BC5 together are 0.4 s, 2 % of the compressor; they and the BC1 punch-through are behind uot-gpu-compute's
`bc1` feature (block_compression's `bc15`), which no app turns on (hakari leaves block_compression out of the
workspace-hack, so dynamapper builds without `bc15`). In the app the compressor took 27 to 50 s cold (37 s in an
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

Next: with the pass loop the land pipeline is 8.9 s of the cold start (the app log's "ready after") and the other 119
shaders about 4 s (112,000 instructions in all; the art and ground pixel shaders the largest at 12,753 and 8,841);
without it the land pipeline stays about 27 s of it.

## Testing with a cold cache

Point `MESA_SHADER_CACHE_DIR` at a fresh empty directory under the scratch folder for the launch (never delete
`~/.cache/mesa_shader_cache`); RADV keeps its pipeline cache there too. The control socket's captures cannot time the
start (see above), Xvfb cannot present with RADV (no DRI3) and ffmpeg cannot grab an XWayland window off the active
desktop (BadMatch). A private headless KWin works: `dbus-run-session -- kwin_wayland --virtual --xwayland --width 1280
--height 800 --socket <name> <holder script>` gives a `DISPLAY` with DRI3 where the app presents, and `ffmpeg -f
x11grab -draw_mouse 0 -window_id <id>` records it. Hold the preview lock around it. About one recording in eight froze
on one frame while the app ran on (before and after the hold alike): check the recording, not only its timings
(2026-11-21, scripts under the scratch folder's `cold-start/`).
