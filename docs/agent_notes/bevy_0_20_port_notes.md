# Bevy 0.20 port notes: naga-oil is gone, shaders are WESL

Discovered 2026-10-25 while porting dynamapper from Bevy 0.19.1 to 0.20.0
(bevy_egui 0.43, wgpu 30). Read `bevy_0_19_render_graph_is_a_schedule.md`
too; the render-graph-as-schedule model it describes is unchanged.

## Shaders: `cargo check` passes, the app would not render

Bevy 0.20 dropped naga-oil for WESL (`wesl` crate 0.6). Nothing in Rust
fails to compile; every custom shader would just fail at runtime. What
mattered:

- **The file extension picks the pipeline.** `ShaderLoader` hands `.wgsl`
  to wgpu raw (no imports, no defs). Anything that imports or is imported
  must be `.wesl`, and the `ShaderRef` paths in Rust must say `.wesl` too.
  The `postprocess_*_doc/` reference shaders stay `.wgsl`: they import
  nothing and nothing loads them.
- **Import syntax.** `#import "shaders/a/b.wgsl"::{x, y}` becomes
  `import package::shaders::a::b::{x, y};`. `package::` is the asset root
  (`PathOrigin::Absolute`). Use it, not `super::`: `ShaderLoader` only loads
  absolute and package imports as file dependencies and skips relative ones.
  Imports must end with `;` and come before any declaration.
- **Bevy module paths now follow the crate's file layout**:
  `bevy_pbr::{forward_io, mesh_functions, mesh_view_bindings,
  view_transformations}` all live under `bevy_pbr::render::`.
  `bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput` did
  not move. Material bind group is still `@group(3)`
  (`MATERIAL_BIND_GROUP_INDEX`).
- Things that kept working: importing an item a module itself imported
  (`art_bindings` passes on `SceneUniform` from `common_bindings`), calling
  through a module import (`mesh_functions::get_world_from_local(...)`), and
  identifiers ending in a digit (the old naga-oil rule no longer applies).
- **Dead-code stripping.** Bevy compiles with `strip: true`, so a library
  module with no entry point compiles to an empty string, and its imports
  are never resolved. That is why `texture_cache/land.rs`'s
  `land_sampling_shader_composes` test builds a small `@fragment` root that
  calls the sampling functions, and runs the output through
  `wgpu::naga` for parsing and validation. The 0.20 `ShaderCache` API is
  `ShaderCache::new(device, load_module)` plus `set_shader(id, Shader)` and
  `get(pipeline, id, defs)`. The `composer` field is gone.
- **Validating every shader without a GPU.** Write a throwaway binary
  against `wesl = "=0.6.0"` (feature `naga-ext`) and `naga = "30"`
  (`wgsl-in`). Give it a `wesl::Resolver` that maps `Absolute` paths to
  `dynamapper/assets/<path>.wesl`, maps `Package(p)` to
  `~/.cargo/registry/src/*/<p>-0.20.0/src/<path>.wesl` (keep only the last
  `/` segment of `p`, the same way bevy's `ShaderResolver::canonical_path`
  does), and serves `constants` from a string. Compile with
  `{ imports: true, condcomp: true, visibility: false, ..Default }` and
  then validate the result with naga. Mesh materials need the flags
  `MESH_PIPELINE`, `VERTEX_OUTPUT_INSTANCE_INDEX`, `VERTEX_POSITIONS`,
  `VERTEX_NORMALS`, `VERTEX_UVS`, `VERTEX_UVS_A`, `VERTEX_UVS_B` (and
  `VERTEX_COLORS` for art), plus the constants `MATERIAL_BIND_GROUP`,
  `MAX_DIRECTIONAL_LIGHTS`, `MAX_CASCADES_PER_LIGHT` and
  `AVAILABLE_STORAGE_BUFFER_BINDINGS`.

## Rust API breaks hit

- `#[derive(ExtractResource)]` / `#[derive(ExtractComponent)]` need
  `#[extract_app(bevy::render::RenderApp)]` (extraction now targets any
  sub-app). `bevy::render::extract_resource::ExtractResourcePlugin<R>` is
  still a one-parameter alias fixed to `RenderApp`.
- Everything else in the migration guide (exclusive systems, query-many
  `Result`s, `set_if_different`, `BindGroupBuilder`, ...) did not touch
  this crate.

## Semantic changes checked

- Render/Core3d set ordering is now weak (`chain_weak`). Our render-world
  systems are safe: the atlas/texture upload systems in
  `RenderSystems::Queue` conflict with `prepare_assets` through
  `RenderAssets<GpuImage>`, `render_system` is exclusive, and the profiling
  and fog systems use explicit `.before`/`.after`, which are still strict.
- `RenderDebugOverlayPlugin` is in `DefaultPlugins` only with the
  `render_dev_tools` feature, which dynamapper does not enable. Even when
  present, its F1/F2 keybindings (the same keys as our help and settings
  dialogs) are off by default.
- `Tonemapping::None` is now a full passthrough, but the camera uses the
  default `TonyMcMapface`, so nothing changes for us.
