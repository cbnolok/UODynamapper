# Bevy 0.19: the render graph is gone — render passes are systems

Discovered 2026-09-06 while porting the workspace from Bevy 0.18.1 to 0.19.1.

## The change

`bevy::render::render_graph` **no longer exists**. There is no `Node`,
`ViewNode`, `RenderLabel`, `RenderGraphExt`, `add_render_graph_node`, no
`Node3d`/`Core3d` graph labels, and no `bevy::core_pipeline::core_3d::graph`
module.

Instead:

- `bevy::render::renderer::RenderGraph` is a **`ScheduleLabel`**, run once per
  frame by `render_system`. Its top-level sets are
  `RenderGraphSystems::{Begin, Render, Submit, Finish}`.
- `Core3d` (`bevy::core_pipeline::Core3d`) is a **schedule** run once per camera
  by `camera_driver`. Its sets are
  `Core3dSystems::{Prepass, MainPass, EarlyPostProcess, PostProcess}`
  (`Core2d`/`Core2dSystems` mirror this).
- What used to be a graph node is now a plain system added to `Core3d`, ordered
  with normal `.in_set()` / `.before()` / `.after()`. It gets its command
  encoder from `RenderContext`, which is now a **`SystemParam`**
  (`render_context.command_encoder()`), and its view via `ViewQuery<...>`.
- Node ordering that used to be expressed as graph edges between labels is now
  expressed against the *systems* themselves — e.g. `after(tonemapping)`, where
  `bevy::core_pipeline::tonemapping::tonemapping` is a public system fn.
  `upscaling` (the final blit) runs `after(Core3dSystems::PostProcess)`.

## Where this repo touches it

- `core/render/scene/world/effects/fog_pass.rs` — implements
  `FullscreenMaterial`. The trait's `node_edges()` / `sub_graph()` /
  `node_label()` methods were replaced by `schedule()` and
  `schedule_configs(ScheduleConfigs<BoxedSystem>)`. The fog pass wants to run
  *after* tonemapping (the default impl runs *before* it), so it overrides
  `schedule_configs` with `.in_set(Core3dSystems::PostProcess).after(tonemapping)`.
- `core/render/scene/world/land/profiling.rs` (feature `gpu-profiling`, so
  `cargo check` misses it by default — check it explicitly) — the two
  wgpu-profiler graph nodes became `sys_profiling_begin` /
  `sys_profiling_end`, added to `Core3d` before/after `Core3dSystems::MainPass`.
  Both still run once per camera, so the existing `ActiveProfilingQuery`
  "only if none active" guard is still what keeps a single query per frame.
- `core.rs` — the old `WireframePanicFixPlugin` (which inserted an `EmptyNode`
  at `Node3d::PostProcessing` because 0.18's `WireframePlugin` panicked on the
  missing graph edge) is **obsolete and removed**: 0.19's wireframe renders as a
  system in `Core3d` and touches no graph.

## Other 0.19 API renames hit during the port

- `Assets::get_mut` returns `Option<AssetMut<'_, A>>`, a Deref/DerefMut guard —
  bindings need `let Some(mut x)`, and passing it where `&mut A` is expected
  needs `&mut x`. Upside: `AssetEvent::Modified` is now only emitted if the
  guard is actually deref-mutated, so the "don't call get_mut unless something
  changed" comments scattered through `texture_cache/land.rs` and
  `world/land.rs` are now belt-and-braces rather than load-bearing.
- `bevy::render::storage::ShaderStorageBuffer` → `ShaderBuffer`.
- `RenderCreation::Automatic` takes `Box<WgpuSettings>`.
- `DirectionalLight::shadows_enabled` → `shadow_maps_enabled`.
- `WireframeConfig` gained `default_line_width` and `default_topology`.
- `TextFont::font` is now `FontSource` (not `Handle<Font>`) and
  `TextFont::font_size` is now `FontSize` (not `f32`); both have `From` impls,
  so `.into()` is enough.

## `cargo check` does not cover `#[cfg(test)]` code

Two port errors hid in `texture_cache/land.rs`'s test module and only surfaced
under `cargo test`: `bevy::shader::PipelineCacheError` → `ShaderCacheError`, and
`ShaderCache::new` gained a leading `device` parameter (still generic over
`<ShaderModule, RenderDevice>`, so the test's `ShaderCache::<(), ()>` trick for
compiling shaders without a GPU still works — pass `()` as the device). When
porting, run `cargo test --workspace` and
`cargo check -p dynamapper --features gpu-profiling`, not just `cargo check`.

## Third-party plugins must be bumped in lockstep

`bevy_egui` (and formerly `bevy_framepace`, since replaced by the local
`dynamapper/src/core/frame_limiter.rs`) link Bevy directly, so a stale version silently
pulls a **second** copy of Bevy into the graph. The symptom is a flood of
nonsense errors — `FramepaceSettings is not a Resource`,
`EguiPrimaryContextPass: ScheduleLabel is not satisfied`, `expected Entity,
found a different Entity` — none of which are real API changes. Check with
`cargo tree -p dynamapper -i bevy`: it must print exactly one `bevy vX`.

For 0.19 the working pair is `bevy_egui 0.42` (which brings egui 0.33 → 0.36)
and `bevy_framepace 0.22` (later dropped). egui 0.36 renamed the context input-capture queries:
`ctx.wants_pointer_input()` / `wants_keyboard_input()` →
`egui_wants_pointer_input()` / `egui_wants_keyboard_input()`, and
`EguiContextSettings::scale_factor` is gone — UI scale is egui's own zoom factor
(`ctx.set_zoom_factor()` / `ctx.zoom_factor()`), which bevy_egui only mirrors
one-way into the read-only `EguiZoomFactor` component.
