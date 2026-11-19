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
`dynamapper/src/`; `rsw/` abbreviates `render/scene/world/`. `core/render/` moved to the top-level `render/` 2026-11-19; every path below, the "from" paths of earlier moves included, is written without its former `core/` prefix.

### app

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `main.rs`, `prelude.rs` | 41 | 0 | Stay at the root. |
| `app.rs` | 464 | 0 | App construction and plugin wiring: `core.rs` renamed 2026-11-19, once export and content had left `core/`. |
| `app/app_states.rs`, `app/system_sets.rs` | 52 | 1 | States and schedule sets. |
| `app/paths.rs` | 70 | 2 | The asset and cache directories. Split 2026-11-19 from `core/constants.rs`: the baked sun constants went beside `SunAngles` in `render/terrain/mesh_material.rs`, their only user. |
| `console_logger.rs`, `app/bevy_log_filter.rs` | 453 | 1 | Logging. |
| `util_lib.rs`, `util_lib/tracked_plugin.rs` | 352 | 2 | Plugin tracking and the `fname!` macro. |
| `util_lib/image.rs`, `util_lib/math.rs`, `util_lib/wesl_source.rs` | 156 | 3 | Small shared helpers; stay in `util_lib/`. |

### settings

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `settings.rs` | 1590 | 17 | Model, loading, saving: moved 2026-11-19 from `configs/settings.rs`, with `ExternalDataPlugin` from `configs.rs`. Runtime appliers leave for their systems. |
| `settings/shader_presets.rs` | 1901 | 26 | Preset model and persistence, and the conversions between a preset and the live state. `UniformState` and `UniformSnapshot` (the live uniforms) moved 2026-11-19 to `render/uniform_state.rs`. |
| `settings/runtime_specs.rs` | 209 | 1 | |

### world

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `world/coords.rs` | 121 | 1 | Moved 2026-11-18 from `util_lib/uo_coords.rs`; its importers took the new path 2026-11-19. |
| `world/{maps, statics, multis}.rs` | 908 | 12 | Moved 2026-11-18 from `core/`; their importers took the new path 2026-11-19. |
| `world/maps.rs` (`WorldGeoData`) | | | The map catalog: moved 2026-11-19 from `render/scene/world.rs`, with the startup system that fills it. `WorldPlugin` (wiring, 70 lines) stays in `render/scene/world.rs` and still registers it. |
| `world/player.rs`, `render/scene/player.rs` | 107 | 0 | The `Player` component moved 2026-11-19 to `world/player.rs`. The entity's spawn and marker cube stay in `render/scene/player.rs`. Its `prev_rendered_pos` is the scene's checkpoint and would move to a component of the scene's own if the two are ever split, which would change the queries in `scene`, `export`, `dev_control` and `teleport`. |
| `world/{player_movement, teleport}.rs` | 362 | 0 | Moved 2026-11-18 from `core/controls/`. |
| `world/caves.rs`, `world/z_view.rs` | 2211 | 24 | Moved 2026-11-19 from `rsw/`, whole: their systems write the analyses' private state. World analysis (cave membership, floor-view cut, cell queries) that render consumes. They still read the render side: the chunks they send to be baked (`LCMesh`, `PendingTextureBake`), the map block size in `land`, the player and scene state, and `PlayerFloorRule` and `LAND_NO_Z_LIMIT` in `mesh_material`. The KR z bands (`client_light_band`, `kr_light_layer`, their constants and `NO_Z_VIEW_LIMIT`) moved here from `light_cover` 2026-11-19. |

### content

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `content.rs` | 1290 | 13 | Moved 2026-11-19 from `core/uo_files_loader.rs`, whole. Split package discovery from the runtime gump provider. |
| `content/land_routes.rs`, `render/assets/land/route_file.rs` | 1122 | 6 | The land route table (moved 2026-11-19 from `configs/`) and its file. |
| `content/land_texture_variation.rs` | 237 | 1 | Moved 2026-11-19 from `configs/`. |
| `ui/dialogs/paperdoll_overrides.rs` (+ `measure.rs`) | 605 | 0 | Composition and override policy; its window goes to `ui/`. |

### input

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `input.rs`, `input/{input_actions, window_controls}.rs` | 158 | 0 | Moved 2026-11-19 from `core/controls`. Actions, bindings, dispatch. |
| `input/dev_control.rs` (+ `input.rs`, `settings.rs`) | 913 | 14 | Moved 2026-11-19. Renderer control socket. |
| `input/camera_input.rs` | 170 | 0 | The camera's controls (wheel and +/- zoom, the free camera's keys): moved 2026-11-19 from `render/view/camera.rs`. `CameraPlugin` still registers them, in the same order and under the same conditions. |

### render

| Now | Lines | Churn | Owner |
|---|---:|---:|---|
| `render.rs`, `render/scene.rs` | 824 | 2 | `render/scene`: chunk lifecycle. View selection (the follow, free and export footprints) left for `view/footprint.rs` 2026-11-19. |
| `render/view.rs`, `view/{camera, footprint}.rs`, `camera/perspective.rs` | 937 | 5 | `render/view`: the camera moved 2026-11-19 from `render/scene/`, and the footprints from `scene.rs`. `ScenePlugin` still adds `CameraPlugin`. |
| `render/terrain/chunk_loader.rs` | 201 | 0 | Stays in `render/terrain`: only `terrain/draw_mesh` uses it, and it owns the loader thread. It goes to `render/scene` if streaming loads ever move there. |
| `render/terrain.rs`, `terrain/{draw_mesh, mesh_material, setup_base_mesh, cave_mouth_veil}.rs` | 3013 | 50 | `render/terrain`: moved 2026-11-19 from `rsw/land`. Its light enums moved 2026-11-19: `LightCover` to `light_cover.rs`, `ClientLocalLightSource` to `static_lights.rs`. |
| `render/terrain/coverage.rs` (+ `cache.rs`, `runtime.rs`, `gpu_tests.rs`) | 1057 | 17 | `render/terrain`: moved 2026-11-19 from `assets/land/`. `assets::land` still builds its bases and registers its stack cache system, in the same order; `assets::land` already read `terrain` (the shared material, `LCMesh`), so the move adds no new dependency direction. |
| `render/assets/land/liquid_sampling_tests.rs` | 574 | 0 | `render/assets`: it samples the EC aux atlas and tests `assets::land`'s aux padding, so it stays with `land.rs`. |
| `render/assets/tile_atlas.rs` | 958 | 9 | `render/assets`: moved 2026-11-19 from `rsw/land`, with the atlas upload budget and telemetry it reads (`LandUploadBudget`, `LandUploadTelemetry`, from `terrain.rs`). The terrain plugin still registers its systems. |
| `render/assets.rs`, `art.rs` | 1073 | 4 | `render/assets`: moved 2026-11-19 from `core/texture_cache`. The unused full-preload `residency.rs` was deleted 2026-11-19. Its pages are read and decoded on the async compute pool since 2026-11-19, from the `Arc` packages; a page waits as pending until it is uploaded. |
| `render/assets/land.rs` (+ `cache.rs`, `texture_array.rs`, `aux_layers.rs`, `routing.rs`) | 4783 | 25 | `render/assets`; the shared EC item auxiliary setup leaves terrain. |
| `render/assets/ec_item_liquid.rs`, `ec_item_surface.rs` (+ `hue_mask`, `water`) | 856 | 9 | `render/assets`. |
| `render/statics.rs`, `statics/{statics_collect, static_source, static_placement, static_chunk_cache, static_batches, statics_draw, static_surface, transparent_order}.rs` | 8624 | 73 | `render/statics`: moved 2026-11-19 from `rsw/art`; `WorldPlugin` still adds its plugin. Its shaders stay under `shaders/world/art/`. `statics_collect.rs` (5470 lines) split 2026-11-19: source policy (`static_source`), anchors, bounds and draw order (`static_placement`), the chunk cache (`static_chunk_cache`), the instances and batches (`static_batches`); collection, its stats and local-light sampling stay, with the tests of all five, which share their fixtures. |
| `statics/{client_grunge, depth_tests, shading_tests}.rs` | 609 | 9 | `render/statics`: moved 2026-11-19 from `rsw/art`. |
| `render/lighting.rs`, `lighting/{static_lights, client_light_sprites, light_cover, light_scene, light_scene_tests, client_night, dynamic_light, light_scale}.rs` | 5998 | 93 | Moved 2026-11-19 from `art/`, `rsw/effects/`, `scene/` and `configs/`: one owner for lighting. It reads the statics' placement, source and land z, the picking query and the transparent draw order (`pub(crate)` in `statics/`); it moves under `render/` with the rest of `render`. |
| `render/picking.rs`, `picking/{gpu_picking, static_geometry, terrain_cursor}.rs` (+ their `tests.rs`) | 1258 | 7 | `render/picking`: moved 2026-11-19 from `art/` (`gpu_picking`, and `static_geometry`, the statics' pick and highlight geometry) and `terrain/` (`picking.rs`, now `terrain_cursor.rs`). The art and terrain plugins still install them, in the same order. They read the statics' source, placement and batches and the land mesh (`pub(crate)` in `statics/` and `terrain/`). |
| `render/postprocess.rs`, `postprocess/fog_pass.rs` | 143 | 3 | `render/postprocess`: moved 2026-11-19 from `rsw/effects`. Its shader stays at `shaders/world/effects/fog_pass.wesl`, beside the surface effects the art and land import. |

### ui

| Now | Lines | Churn | Notes |
|---|---:|---:|---|
| `ui/dialogs.rs`, `dialogs/{image_background, keybindings_help, teleport}.rs` | 507 | 0 | Moved 2026-11-19 with `overlays/` from `render/`; `RenderPlugin` still adds both plugins, in the same order. |
| `dialogs/gump.rs` (+ `robe_edges.rs`), `paperdoll_ec.rs`, `paperdoll_menu.rs` | 3260 | 7 | |
| `dialogs/preferences.rs` | 822 | 11 | The settings-to-limiter applier (`sys_apply_performance_settings`) and the limiter helpers (from `app.rs`) moved 2026-11-19 to `diagnostics/frame_limiter.rs`; the dialog still registers the applier and applies its own edits at once. |
| `dialogs/land_shader.rs` (+ `tips.rs`) | 2184 | 52 | The per-frame uniform push moved 2026-11-19 to `render/terrain.rs` (`sys_push_land_uniforms_if_dirty`); the panel still registers it, in the same place. |
| `ui/overlays.rs`, `overlays/{player_position, sysmessages}.rs`, `ingame_sysmessage_logger.rs` | 426 | 1 | |
| `overlays/performance.rs` | 525 | 1 | |
| `overlays/cursor_behavior.rs` | 971 | 2 | Presentation here; dispatch to `input/`, teleport to `world/`, picking to `render/picking`. |

### export and diagnostics

| Now | Lines | Churn | Owner |
|---|---:|---:|---|
| `export.rs` | 881 | 4 | `export/`: moved 2026-11-19 from `core/export_job.rs`, whole. Split into job lifecycle, readiness, capture and spool, provenance. |
| `diagnostics.rs`, `diagnostics/{frame_rate, frame_limiter, gpu_profiling}.rs` | 1153 | 1 | Moved 2026-11-18; its importers took the new path 2026-11-19. |

## Order

1. Cold files first, one owner at a time, each move a commit of its own with no behaviour change: `diagnostics/`,
   `input/` (without the camera's input), `world/` (coordinates, player, movement, teleport, the map catalog).
2. Hot files when parallel work on them is closed, in the plan's order: `world/` analysis (caves, z-view), the
   `statics_collect.rs` split, `render/lighting/`, then terrain, assets and UI.
3. A move keeps a re-export at the old path only while a hot file still imports it, and the re-export goes when
   that file moves.
