# Dynamapper module map

Where each `dynamapper/src` file lives today and which owner it moves to, following the target layout of the
[reorganization plan](2026-11-12-dynamapper-reorganization-plan.md#proposed-rust-layout). It is the working index
for the ownership moves: a contributor looking for a subsystem finds every file that implements it here, wherever
the file sits now.

Churn is the number of commits that touched the file in the 7 days before 2026-11-18, which tells the cold files
(movable at any time) from the hot ones (moved only when no parallel work is open on them). Line counts are as of
that date. Update a row when its file moves, and drop the "now" column when every row is in place.

## How the current tree misleads

- **Everything is under `core/`.** Application setup, world data, input, rendering, export and diagnostics are
  siblings there, so the path says nothing about the owner.
- **`core/render/scene/world/` is the render tree, not the world.** The logical world data (maps, statics,
  multis, the player's position, caves, the floor-view cut) is spread between it, `core/` and `controls/`.
- **Lighting has no owner.** Static lights, light sprites, light cover, the light scene, the night composite, the
  player light and the light scales sit in five folders (`world/art/`, `world/effects/`, `scene/`, `configs/`,
  plus the light enums in `world/land/mesh_material.rs`).
- **`texture_cache/` mixes GPU page caches with terrain logic.** Land coverage belongs with terrain; the land
  route file belongs with content.
- **UI lives under `render/`.** Dialogs and overlays are presentation, and some of them also own runtime state
  (the land shader panel applies uniforms every frame; preferences owns the frame limiter's settings).

## Map

Owners are the plan's: `app/`, `settings/`, `world/`, `content/`, `input/`, `render/{view, scene, assets,
terrain, statics, lighting, picking, postprocess}`, `ui/`, `export/`, `diagnostics/`. Paths are relative to
`dynamapper/src/`; `rsw/` abbreviates `core/render/scene/world/`.

### app

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `main.rs`, `prelude.rs` | 41 | 0 | Stay at the root. |
| `core.rs` | 464 | 0 | App construction and plugin wiring → `app/`. |
| `core/app_states.rs`, `core/system_sets.rs` | 52 | 1 | States and schedule sets. |
| `core/constants.rs` | 88 | 2 | Split by user: world light constants to `render/lighting`, paths to `app/`. |
| `console_logger.rs`, `core/bevy_log_filter.rs` | 453 | 1 | Logging. |
| `util_lib.rs`, `util_lib/tracked_plugin.rs` | 352 | 2 | Plugin tracking and the `fname!` macro. |
| `util_lib/image.rs`, `util_lib/math.rs`, `util_lib/wesl_source.rs` | 156 | 3 | Small shared helpers; stay in `util_lib/`. |

### settings

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `configs.rs`, `configs/settings.rs` | 1590 | 17 | Model, loading, saving. Runtime appliers leave for their systems. |
| `configs/shader_presets.rs` | 2028 | 26 | Preset model and persistence; `UniformState` (runtime render state) moves to `render/`. |
| `configs/runtime_specs.rs` | 209 | 1 | |

### world

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `world/coords.rs` | 121 | 1 | Moved 2026-11-18 from `util_lib/uo_coords.rs`, which re-exports it for its hot importers. |
| `world/{maps, statics, multis}.rs` | 908 | 12 | Moved 2026-11-18 from `core/`, which re-exports them for their hot importers. |
| `rsw.rs` (`core/render/scene/world.rs`) | 100 | 4 | The map catalog half; its plugin wiring stays with render. |
| `core/render/scene/player.rs` | 107 | 0 | Logical position → `world/player`; the rendered entity stays with `render/scene`. |
| `world/{player_movement, teleport}.rs` | 362 | 0 | Moved 2026-11-18 from `core/controls/`. |
| `rsw/caves.rs`, `rsw/z_view.rs` | 2211 | 24 | World analysis (cave membership, floor-view cut, cell queries) that render consumes. |

### content

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `core/uo_files_loader.rs` | 1290 | 13 | Split package discovery from the runtime gump provider. |
| `configs/land_routes.rs`, `core/texture_cache/land/route_file.rs` | 1122 | 6 | The land route table and its file. |
| `configs/land_texture_variation.rs` | 237 | 1 | |
| `core/render/dialogs/paperdoll_overrides.rs` (+ `measure.rs`) | 605 | 0 | Composition and override policy; its window goes to `ui/`. |

### input

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `core/controls.rs`, `controls/input_actions.rs`, `controls/window_controls.rs` | 158 | 0 | Actions, bindings, dispatch. |
| `core/controls/dev_control.rs` (+ `input.rs`, `settings.rs`) | 913 | 14 | Renderer control socket. |
| keyboard and mouse handling in `core/render/scene/camera.rs` | | | Zoom, free camera and Shift+arrow input leave the camera. |

### render

| Now | Lines | Churn | Owner |
|---|---:|---:|---|
| `core/render.rs`, `core/render/scene.rs` | 1023 | 2 | `render/scene`: chunk lifecycle; view selection (the follow, free and export footprints) to `render/view`. |
| `core/render/scene/camera.rs` (+ `perspective.rs`) | 880 | 5 | `render/view`. |
| `rsw/land/chunk_loader.rs` | 201 | 0 | `render/scene` (streaming loads). |
| `rsw/land.rs`, `land/{draw_mesh, mesh_material, setup_base_mesh, cave_mouth_veil}.rs` | 3013 | 50 | `render/terrain`. The light enums in `mesh_material.rs` go to `render/lighting`. |
| `core/texture_cache/land/coverage.rs` (+ `cache.rs`, `runtime.rs`, `gpu_tests.rs`) | 1057 | 17 | `render/terrain`. |
| `core/texture_cache/land/liquid_sampling_tests.rs` | 574 | 0 | `render/terrain`. |
| `rsw/land/tile_atlas.rs` | 958 | 9 | `render/assets`. |
| `core/texture_cache.rs`, `art.rs`, `residency.rs` | 1073 | 4 | `render/assets`. |
| `core/texture_cache/land.rs` (+ `cache.rs`, `texture_array.rs`, `aux_layers.rs`, `routing.rs`) | 4783 | 25 | `render/assets`; the shared EC item auxiliary setup leaves terrain. |
| `core/texture_cache/ec_item_liquid.rs`, `ec_item_surface.rs` (+ `hue_mask`, `water`) | 856 | 9 | `render/assets`. |
| `rsw/art.rs`, `art/{statics_collect, statics_draw, static_geometry, static_surface, transparent_order}.rs` | 8624 | 73 | `render/statics`; `statics_collect.rs` (5470 lines) splits into source policy, geometry, chunk cache, collection and batches. |
| `art/{client_grunge, depth_tests, shading_tests}.rs` | 609 | 9 | `render/statics`. |
| `art/{static_lights, client_light_sprites, light_cover, light_scene, light_scene_tests}.rs` | 4686 | 82 | `render/lighting`. |
| `rsw/effects/client_night.rs`, `core/render/scene/dynamic_light.rs`, `configs/light_scale.rs` | 1307 | 11 | `render/lighting`. |
| `art/gpu_picking.rs` (+ `tests.rs`), `land/picking.rs` (+ `tests.rs`) | 898 | 7 | `render/picking`. |
| `rsw/effects.rs`, `effects/fog_pass.rs` | 143 | 3 | `render/postprocess`. |

### ui

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `core/render/dialogs.rs`, `dialogs/{image_background, keybindings_help, teleport}.rs` | 507 | 0 | |
| `dialogs/gump.rs` (+ `robe_edges.rs`), `paperdoll_ec.rs`, `paperdoll_menu.rs` | 3260 | 7 | |
| `dialogs/preferences.rs` | 822 | 11 | The frame limiter settings leave for `diagnostics/`. |
| `dialogs/land_shader.rs` (+ `tips.rs`) | 2184 | 52 | The per-frame uniform push leaves for `render/`. |
| `core/render/overlays.rs`, `overlays/{player_position, sysmessages}.rs`, `ingame_sysmessage_logger.rs` | 426 | 1 | |
| `overlays/performance.rs` | 525 | 1 | |
| `overlays/cursor_behavior.rs` | 971 | 2 | Presentation here; dispatch to `input/`, teleport to `world/`, picking to `render/picking`. |

### export and diagnostics

| Now | Lines | Churn | Owner |
|---|---:|---:|---|
| `core/export_job.rs` | 881 | 4 | `export/`: job lifecycle, readiness, capture and spool, provenance. |
| `diagnostics.rs`, `diagnostics/{frame_rate, frame_limiter, gpu_profiling}.rs` | 1153 | 1 | Moved 2026-11-18. `core::diagnostics` re-exports it until `scene.rs`, `draw_mesh.rs`, `tile_atlas.rs` and `texture_cache/land/cache.rs` move. |

## Order

1. Cold files first, one owner at a time, each move a commit of its own with no behaviour change: `diagnostics/`,
   `input/` (without the camera's input), `world/` (coordinates, player, movement, teleport, the map catalog).
2. Hot files when parallel work on them is closed, in the plan's order: `world/` analysis (caves, z-view), the
   `statics_collect.rs` split, `render/lighting/`, then terrain, assets and UI.
3. A move keeps a re-export at the old path only while a hot file still imports it, and the re-export goes when
   that file moves.
