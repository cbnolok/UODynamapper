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
  `lib/support/uot-gpu-compute` by its label: the networks on a thread of their own at startup (`RIFE conv
  Stride1/Leaky tile 64x32`, `RIFE pixel step: merge`, `upscale conv 3x3/Silu tile 64x48`, `upscale prepare`), and the
  block encoder on another (`BC7 compressor (every quality; and BC1-BC5)`, `BC1 punch-through`) only while a loaded
  animation package is BC7, from the first frame one is (below); the RIFE network when the warm-up did not give it,
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
BC1 to BC5 together are 0.4 s, 2 % of the compressor. In the app the compressor took 27 to 50 s cold (37 s in an
earlier run), with Bevy's own pipelines compiling at the same time on its async compute thread and the machine more or
less busy.

**RADV compiles pipelines of two threads side by side.** BC7 on one thread and RIFE with SPAN on another: 20.8 s wall
(20.5–21.1) against 22.2 s (21.9–22.3) one after the other, each as long as alone (RIFE and SPAN 1.32–1.42 s, BC7
20.4–21.1 s); BC1–BC5 against BC7: 20.7 s against 21.1 s summed. So the block encoder has a thread of its own and
does not wait for the networks.

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

The screenshot is the one thing still behind the batch: Bevy maps its readback buffer in a task on the same async
compute pool, so a capture requested during the cold compiles completes after them (the frame is captured at once).

## Testing with a cold cache

Point `MESA_SHADER_CACHE_DIR` at a fresh empty directory under the scratch folder for the launch (never delete
`~/.cache/mesa_shader_cache`); RADV keeps its pipeline cache there too. The control socket's captures cannot time the
start (see above), Xvfb cannot present with RADV (no DRI3) and ffmpeg cannot grab an XWayland window off the active
desktop (BadMatch). A private headless KWin works: `dbus-run-session -- kwin_wayland --virtual --xwayland --width 1280
--height 800 --socket <name> <holder script>` gives a `DISPLAY` with DRI3 where the app presents, and `ffmpeg -f
x11grab -draw_mouse 0 -window_id <id>` records it. Hold the preview lock around it. About one recording in eight froze
on one frame while the app ran on (before and after the hold alike): check the recording, not only its timings
(2026-11-21, scripts under the scratch folder's `cold-start/`).
