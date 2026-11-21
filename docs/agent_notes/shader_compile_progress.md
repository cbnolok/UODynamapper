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
  warm-up callback). Wrapped now: the RIFE network and atlas steps (`rife_in_betweens/render.rs`, background pool) and
  the static picking MSAA resolve (`gpu_picking.rs`, render thread, only with MSAA). `UpscaleModel` (SPAN) and
  `MobileBlockEncoder` (BC1/BC7) were not created by the app yet, only by their tests (2026-11-21).

## Cold start: the app freezes, so the overlay cannot show it

With Mesa's cache empty (RADV, Radeon 880M, 2026-11-21) the app renders nothing for about 35 s after the scene
loads; the first frame comes with about 35 of the 45 to 48 startup pipelines ready, and the overlay shows the last ones
for a second or two. Two causes together:

- `app.rs` gives Bevy's async compute pool one thread, and Bevy compiles its queued pipelines on that pool, so they
  compile one after another.
- Bevy's upscaling prepare (`bevy_core_pipeline` `upscaling/mod.rs`) calls `block_on_render_pipeline` on its blit
  pipeline the first time a view's output format is seen. The blit is queued in the same frame as the world's
  pipelines, behind them on that one thread, so the render world (and with it the main world) waits for all of them.

Bevy pipelines queued after the blit is ready compile without a freeze and show in the overlay (the night pipelines on
the first switch to night, for example). The logged times of a startup batch include the wait behind the others.

## Testing with a cold cache

Point `MESA_SHADER_CACHE_DIR` at a fresh empty directory under the scratch folder for the launch (never delete
`~/.cache/mesa_shader_cache`); RADV keeps its pipeline cache there too. The control socket's captures cannot be taken
during the freeze; the first one completes after it.
