# Dynamapper audit revalidation (2026-11-18)

**Status:** Phase 1 ("Revalidate and characterize") of the
[reorganization plan](2026-11-12-dynamapper-reorganization-plan.md), done at
`dev` commit `adaf218f`. It rechecks every finding of the
[architecture audit](2026-11-08-dynamapper-architecture-audit.md) (checkpoint
`38c4bb34`) and audits the modules added since. Static review only: no code was
changed and no cargo command was run. Findings marked *verified* were re-read
directly at this commit while consolidating; the others carry the static
evidence noted, with inferred effects marked as such. Paths are relative to
`dynamapper/src/` unless they start with `assets/`.

## What changed since the audit

246 commits landed between `38c4bb34` and `adaf218f`; `dynamapper/src` and its
shaders gained about 20.7k lines and lost 2.4k across 99 files. No ownership
move from the plan has been made: the tree under `core/` is the one the audit
described. About 10.3k of the new lines are in 26 files the audit never saw
(caves, z-view, client night, client light sprites, light scene/cover, land
routes and their shaders); they are covered by NEW1–NEW20 below.

Files byte-identical to the checkpoint, so the audit's line numbers still hold:
`core/render/scene.rs`, `scene/player.rs`, `scene/dynamic_light.rs`,
`core/controls.rs`, `controls/player_movement.rs`, `controls/teleport.rs`,
`land/profiling.rs`, `core.rs`, `main.rs`, `build.rs`, the overlays except the
cursor overlay, `dialogs.rs`, `keybindings_help.rs`, `dialogs/teleport.rs`,
`frame_*.rs`, `console_logger.rs`, `bevy_log_filter.rs` and `util_lib/` except
the new `wesl_source.rs`.

Largest files now: `art/statics_collect.rs` 5465 lines (4244 at the audit),
`texture_cache/land.rs` 2299, `art/statics_draw.rs` 2090, `configs/shader_presets.rs`
2033, `dialogs/land_shader.rs` 1938, `dialogs/gump.rs` 1919, `art/static_lights.rs`
1737, `configs/settings.rs` 1576, `world/caves.rs` 1468.

## Status of the audit findings

**Fixed:** T9 (land view vector now from Bevy's view uniform, `b3443566`), T11
(pinning uses native blocks plus ring, with a test), A9 (map statics are never
multis, `82148e58`; `PlacedMultisRes` is still never filled and has no revision).

**Settled or reshaped:** T7 — a missing EC/KR id or package now follows the
explicit, notified chain in `texture_cache/land/routing.rs`, matching the plan's
contract. A13 — the ground shader now writes priority depth; only the stale
`GroundTileInstance` in `art_bindings.wesl` remains. A4 — picking's camera now
comes from the render-graph view; the water-animation term still changes the
signature every frame. C5 — dev-control settings commands moved to a typed
`SystemParam`; status assembly grew inline.

**Partly fixed:** T8 (a changed-cells log exists; `read_cell` still returns
stale or zero cells), T14 (`DEFAULT_KR_TONEMAP_GAIN` removed), F2 (height-bias
control removed; legacy shader remains), F5 (noise controls gated), O7, U2, U3,
A11 (light bytes no longer read per light; the scan itself got worse, NEW1).

**Present, unchanged in substance:** C1–C4, C6, C7, P1–P3, V1–V6, S1–S7, L1–L2,
W1–W3, N11, T1–T6, T10, T12, T13, T15–T17, GP1–GP4, F1, F3, F4, LC1–LC10, AP1,
A1–A3, A5–A8, A10, A12, A14–A16, O1–O6, O8, D1–D13, R1–R4, U1, N1–N10, N12,
N13, H1–H3, ST1–ST5, SP1–SP3, E1–E3.

**Present and wider than reported:** ST4 (the startup load now also performs a
gumps migration write and reads `land.toml` three times), SP1 (a key with a
valid group and bad period, e.g. `kr.dusk`, mixes Classic slot values with KR
style-level fields), SP2 (a second instance in `saved_preset_key`), D4 (the
snapshot now carries style-level fields; dev-control slot switches bypass it),
D8 (amplified by NEW21), D10 (more fields, and the dialog reverts dev-control
writes while open), E2 (route file/profile, LandTextureVariation and style-level
fields are unrecorded; the 1 s route watch runs during export), E3 (a second
cache write in export mode), C1 (two new readers, `z_view.rs` and `caves.rs`),
V2 (a sixth owner of the view size in `client_night.rs`), A2 (source selection
resolved three ways), A3 (resident chunk keys derived three times, NEW8), LC11
(also triggered by route revisions), AP1 (KR pages join the atlas spec unseen by
the format warning), R4 (light-scene timings printed separately and also summed).

Notable current details:

- **V7 is now confirmed statically** and has a wider cause, NEW21.
- **V1 has a performance face:** in free-camera mode `sys_free_camera_movement`
  sends a recompute every frame a key is held, and each recompute reconciles the
  same player-based footprint (`camera.rs:613-637`, `scene.rs:477-485`).
- **S7 verified:** `scene.rs:417` consumes one message per frame; one
  orthographic zoom tick emits two (`camera.rs:532`, `:412-421`), giving two
  identical full recomputes on consecutive frames. There are 13 producer sites.
- **F1 verified:** `FogPostProcessUniform` is always attached
  (`camera.rs:190`); the shader returns early per pixel (`fog_pass.wesl:129`)
  but the fullscreen sample-and-write pass still runs. `client_night.rs:617`
  already shows the component-removal pattern that skips a pass.
- **T1:** the camera is shifted by `>>3` and compared with 32-tile chunk
  coordinates (`draw_mesh.rs:449-462`, `:1223-1235`).

## New findings

Same conventions as the audit: classification, confidence, evidence, follow-up.
NEW1–NEW20 cover the modules added since the checkpoint; NEW21–NEW36 are new
observations in modules the audit covered.

### Per-frame work without a change gate (performance)

**NEW1 — Static-light collection rescans every frame.** *Verified.*
`art/static_lights.rs:365-561` clears its output through `ResMut` every frame,
so the output is always changed. With the `light.mul` source it re-collects and
sorts chunk keys, reads `MultiCoverParts`, builds a fresh `PackageLand` (re-reads
map package chunks), walks every resident block and allocates per block and per
light (`light_cover.rs` `cell_statics`). Downstream, `static_lights.rs:279`
hashes and `world/land.rs:299-450` collects, sorts and hashes all lights every
frame. A corrupt statics chunk is re-read every frame (A10). Follow-up: gate on a
scan key (the one `client_light_sprites.rs` `last_scan` uses, completed with
package, light-scale and multi revisions), keep one `PackageLand` and write the
output only on change.

**NEW2 — z-view reads the map package before its input gate.** *Verified.*
`world/z_view.rs:473` calls `PackageLand::default().averaged_land_z(..)` before
the `last_inputs` early return at `:484`: up to four package-chunk decodes per
frame while "show only player floor" is on. The comment at `:472` says "once
per change". A missing statics store at `:492` keeps the previous cut (medium
confidence). Follow-up: compute after the gate, keyed by the player cell.

**NEW3 — The land material is rewritten while walking, three times over.**
The light uniform selection follows the player, so `world/land.rs` mutates the
land material as the player moves; `land/cave_mouth_veil.rs:73-106` then clones
it into two veil materials and mutates both, even with no cave. Each change
re-prepares three materials. Follow-up: move the light list into a
lighting-owned buffer or extracted resource; gate the veil on a mouth existing.

**NEW21 — `Settings` is flagged changed on every FixedUpdate tick.** *Verified.*
`configs/settings.rs:872-883` passes `&mut settings` from a `ResMut` into the
sync helper, which sets the change flag whether or not a value differs. Every
`is_changed`/`resource_changed::<Settings>` gate is therefore open: camera AA
re-inserts `Msaa` and inserts/removes FXAA/SMAA on both cameras
(`camera.rs:99`, `:301-329`), preferences re-syncs its draft
(`preferences.rs:148`), the cursor overlay rebuilds its inspection
(`cursor_behavior.rs:390`), and `world/land.rs:204` and `route_file.rs:181`
re-run. It also makes `sys_apply_performance_settings` restore the saved
limiter almost every frame (D8). `dev_control/settings.rs:117` already avoids
this pattern. The save debounce additionally clones all sections every tick
(`settings.rs:1084`). Follow-up: compare before writing; land together with D8,
because it changes when D8–D10 code runs.

**NEW22 — Open dialogs flag resources changed every frame.** While open, the
shader panel passes `&mut UniformState` to its controls each frame
(`land_shader.rs:~405-425`), defeating `fog_pass.rs:106` and
`StaticPickingResources` change checks; when dirty, `push_uniforms_if_dirty`
(`land_shader.rs:1061`) mutates every land material. Preferences writes
`WireframeConfig.global` (`preferences.rs:187`) and takes `RouteFileRes`
mutably (`:489`) every frame. Low when closed.

**NEW23 — Statics upload compares whole instance vectors every frame.**
`statics_draw.rs:1632`, `:1694` compare full instance `Vec`s and keep a second
CPU copy, although the collector takes its outputs mutably only on its slow
path; `is_changed()` would do. A16 (five `get_mut` material calls per frame)
sits beside it.

**NEW24 — Chunk streaming repeats selection and allocations.** On an
incremental recompute with unchanged scale `visible_chunks` is computed twice
(`scene.rs:487`, `:559`), sorted vectors are re-sorted (`:601-602`) and cloned
several times; `queue_entity_for_despawn` is a linear `Vec::contains` per chunk
(`:317-321`), and each handoff frame scans the chunk query two or three times.

**NEW25 — Idle land draw clears scratch every frame.** About 456 KB of scratch
is zeroed before the no-targets check (`draw_mesh.rs:416-419` vs `:445`); it is
sized for 65536 ids while routes have 32768 rows. `wet_bits` (65536 route
lookups) is rebuilt on every frame with work (`:861-881`).

**NEW26 — The static-chunk fast path allocates every frame.** Chunk keys are
rebuilt and sorted (`statics_collect.rs:2281`); `requested_pages` is extended,
sorted and deduplicated (`:1490-1515`); with local lights on every light is
hashed (`:1641`). Local-light sampling is statics × 4 corners × all lights with
no spatial index (`:1706-1745`), and the light signature in the cache config
(`:2270`) wipes the whole static chunk cache on any light-set change (opt-in,
default off).

### Rebuild and upload volume (performance)

**NEW4 — A cave change re-bakes every land chunk after a synchronous flood.**
`world/caves.rs:912-980` floods 7×7 package chunks on the main thread on each
cell or z change and inserts `PendingTextureBake` on every `LCMesh`
(`:972-978`, `:1060-1063`); the EC coverage loop then runs sequentially with
SipHash lookups per cell (`draw_mesh.rs:893-930`). Follow-up: analysis off
thread, re-bake only chunks whose mouth or inside set changed.

**NEW5 — Night lighting adds a second `Camera3d`.** `effects/client_night.rs:386`
spawns a camera with its own HDR target, activated when night or glow is on
(`:547-567`): a second Core3d view with its own visibility pass, an upscale blit
and a composite fullscreen pass, with a bind group created per frame
(`:781-782`). Classic pays this to add `light.mul` quads. Follow-up: draw the
light-buffer phase from a node on the main view.

**NEW6 — The light scene is a full-resolution second geometry pass.**
`art/light_scene.rs` `render_light_scene` redraws every transparent static batch
into an `Rgba16Uint` target when a glow is active, allocating a `HashSet`,
a `Vec` and a bind group per frame, with pipeline machinery copied from
`gpu_picking.rs:205-238`. Follow-up: a second colour attachment of the art pass,
or at least cached bind groups and one shared derived-pipeline helper.

**NEW27 — Batch meshes are rebuilt far more than needed.** Batch meshes store
absolute cumulative `start` offsets (`statics_draw.rs:324-370`), so a chunk
entering or leaving rebuilds every later batch in all five passes; sprite
opaque, shadow and transparent build three identical meshes per batch
(`:1257`, `:1330`, `:1393`); the despawn check is O(n²) (`:1285-1288`). This is
the cost side of A15.

**NEW28 — Static-chunk rebuild re-reads neighbours and land.** Each block
re-reads both neighbour blocks and allocates two surface-top vectors
(`statics_collect.rs:2482-2499`); each chunk gets its own `PackageLand`
(`:2181`), re-decoding map package chunks per static chunk.

**NEW29 — Coverage stack cache bakes mutate the whole image.** A bake calls
`images.get_mut` on the 16-slot image (`coverage/cache/runtime.rs:224-237`),
about 9.5 MB, likely re-uploaded whole on every baking frame (inferred from
Bevy's image-asset semantics; not measured). `EcTerrainBases::flush` rewrites
its per-map image the same way (`coverage.rs:196-201`).

**NEW30 (includes NEW19) — Land fragment repeats work and disables early-Z.** `main.wesl`
reads the same cell's metadata twice (`:1078`, `:1111`) and always writes
`out.depth` (`:1092-1094`), which disables early depth testing for land; the
`7541d7ab` frame benchmark targets this cost. `bump.wesl:118-148` can reach
about 36 samples per pixel at close zoom with texture variation (static count,
GPU cost unmeasured).

**NEW31 — Uploads clone their bytes into the render world.** Terrain atlas
extraction uses `extend_from_slice` (`tile_atlas.rs:630`, contradicting the
comment at `:326`); art extraction clones staged page bytes
(`texture_cache/art.rs:787`, `:796`).

**NEW14 — Light-sprite sync respawns everything and loads textures twice.**
`client_light_sprites.rs:844-919` despawns and respawns every sprite on any
change; glow RGB and material each load the same texture.

**NEW15 — Light-scene images are kept and touched when idle.**
`sys_sync_light_scene_image` iterates every glow material each frame without an
`InGame` gate; neither the light-scene nor the light-buffer image shrinks when
inactive.

**NEW16 — Cave discovery spawns OS threads.** `find_map_caves`
(`caves.rs:984-1064`) spawns a thread that spawns `available_parallelism`
scoped threads, beside Bevy's pools, with an O(n²) `missing` lookup.

**NEW32 — Performance overlay polls when hidden.** O3's poll reads every
`/proc/<pid>/fdinfo/*` file four times a second (`performance.rs:89-97`);
`System::new_all()` enumerates every process, disk and network at init
(`:52-53`). N12's compute pool of 1–3 threads (`core.rs:82-88`) remains
unmeasured.

### Correctness and contracts

**NEW7 — Caves and z-view caches go stale after a routes change.** Caves read
raw package statics (`caves.rs:650-680`), bypassing item routes and multis; the
per-map `found` cache is never invalidated (`:849-862`); `ZViewInputs`
(`z_view.rs:380-390`) has no routes revision.

**NEW9 — Three readers of a cell's objects disagree.** `ZViewRegion::read`
(`z_view.rs:315-365`), `MultiCoverParts::read` plus `cell_statics`
(`light_cover.rs`) and `ChunkCells::read` (`caves.rs:650-680`) differ in multi
expansion, item routing and error handling.

**NEW11 — Block read errors become "no statics" in three more places.**
`z_view.rs:335`, `light_cover.rs` `cell_statics` (`unwrap_or(&[])`) and the
light-sprite scan; this is the A10 class and changes cuts, cover and caves.

**NEW12 — The `is_wet_flags` bit layout has no single definition.** Bits are
allocated in `client_grunge.rs` (10), `statics_collect.rs:92-107` (11, 12, 15,
24, 25) and `light_scene.rs:50-58` (13, 14, 16–23), mirrored in
`client_grunge.wesl`, `depth.wesl`, `light_scene.wesl` and `shading.wesl`; only
`depth_tests.rs:208` cross-checks one bit. Follow-up: one Rust table, an
overlap test and `wesl_source` tests for each mirror.

**NEW13 — `CavesRes` changes behind change detection.** `caves.rs:984-1064`
uses `bypass_change_detection` and then edits `active` and `revision`;
`is_changed` consumers miss what `revision` consumers see.

**NEW33 — Pin keys and cache keys differ.** Pins are set by land id
(`draw_mesh.rs:764-768`), the fixed-array cache is keyed by texmap id
(`cache.rs:833-850`); EC-layout atlases routed to Classic texmaps protect the
wrong layers.

**NEW34 — Three sources of "current map".** `Player.current_pos.m`,
`SceneStateData.map_id` and `session_state.world.last_p.m` are each read by
different systems; z-view and caves combine the streaming map id with the
logical position without ordering against `SyncLandChunks`. Latent while one
facet loads.

**NEW35 — Settings have a third writer.** The dev control channel writes
`Settings` and `UniformState` directly (`dev_control/settings.rs:79-110`);
the open preferences dialog (D10) and the shader-panel undo snapshot (D4)
ignore those writes. The gumps migration write inside the settings loader
(`settings.rs:603-605`) ignores errors and runs in export mode (E3).

**NEW36 — Smaller contract leftovers.** `StaticSurfaceLikeResolutionCache` is
never invalidated though KR packages can change (`statics_collect.rs:1880-1890`);
the light-sprite `last_scan` omits package and light-scale changes; picking's
highlight geometry re-derives emission choices with literal `flat_quad` values
(`static_geometry.rs:153`, `:190`, `:198`); routine ground counts log at Warn
(`statics_draw.rs:1501`, `:1705`); the art chunk cache wipes on atlas
`mapping_revision`, which would loop if atlas capacity were ever capped
(`statics_collect.rs:1355`).

### Ownership and duplication

**NEW8 — Resident chunk keys are derived three times** (`static_lights.rs:410-421`,
`client_light_sprites.rs:654-659`, `statics_collect.rs:2281`). Follow-up: one
revisioned resource published by chunk residency.

**NEW10 — Module cycles put world policy inside render modules.** z_view and
light_cover import each other; caves imports light_cover and draw_mesh
internals; the serde settings enums `CaveView`/`CaveSurroundings` live in
`caves.rs`; `dev_control/settings.rs` imports render enums and the UI's
`switch_render_preset`. `statics_collect` also forms cycles with
`static_lights` and `static_surface`.

**NEW17 — One light-target policy is split across three modules.**
`static_lights.rs`, `client_light_sprites.rs` and `effects/client_night.rs`
each decide style-to-light behaviour; `effects/` imports from `art/`.

**NEW18 — Spec-loader boilerplate is duplicated.** `route_file.rs:54-62`
repeats `runtime_specs.rs` reporting; id-range decoding and overlap checks are
copied in `land_routes.rs`, `light_scale.rs` and `land_texture_variation.rs`.
Routes hot-reload; light scales and texture variation load once.

**NEW20 — WESL helpers are duplicated.** `srgb.wesl` and `texture_data.wesl`
hold the same transfer functions with different clamps; value noise is
identical in `land/noise.wesl`, `effects/fog_pass.wesl` and
`postprocess/grunge.wesl`; luminance exists in `land/lighting.wesl`,
`art/shading.wesl`, `postprocess/color_grading.wesl` and inline; two grunge
owners (`art/client_grunge.wesl`, `postprocess/grunge.wesl`).

## Decomposition of the largest statics files

`art/statics_collect.rs` groups, with approximate line ranges: constants and
flag bits 30-107; geometry 110-247 and 3150-3166; visual-kind core 249-303;
depth and ordering 305-523; EC surface resolution 524-777; source and
visual-kind policy 778-1183 (including the test-only resolver of A5 and
`KrStaticPackages`); GPU instance ABI and outputs 1206-1335; chunk cache
1337-1532 and 2299-2408; diagnostics; local-light sampling 1641-1836;
surface-resolution cache 1874-1958; source sync system 1959-2087; frame config
2088-2298; chunk rebuild 2409-2571; per-tile admission 2572-2828; instance
emission 2829-3339; assembly 3340-3465; the collection system 3466-3620; tests
3621-5465 (1845 lines). Geometry, visual kind, depth/ordering, EC surface,
payload, chunk cache, local lights, surface cache and assembly are pure and
testable without Bevy.

`art/statics_draw.rs` holds material extensions and resources, a duplicate of
the source selection (A2), atlas spec and format policy (AP1), atlas images,
resize/rebind, startup setup, five near-identical batch sync systems (A15,
NEW27), instance upload (A16, NEW23) and material uniforms (A1, A6).
`art/static_lights.rs` holds style/target policy, collection (NEW1), entity
sync, material build and a pure colour-response block (about 470 lines);
`client_light_sprites.rs` duplicates its per-light cover check (`:772-805` vs
`static_lights.rs:497-531`), entity sync and `LightClipMaterial` caching.

## Revised priorities

The plan's contracts and target layout still hold. Given the current state, and
with performance and contributor readability as explicit goals, the evidence
supports this order:

1. **Change gates and per-frame waste** — behaviour-preserving, small, measurable:
   NEW21 with D8, NEW1 with NEW3's per-frame light sort, NEW2, F1, S7 with V1's
   per-frame recompute and NEW24, A16 with NEW23, NEW25 and T1. Each lands alone
   with a focused test of its gate and before/after frame times.
2. **Verified dead-code removal** in one commit (unused `util_lib` helpers and
   regex helpers, commented-out blocks, unused shader functions and imports,
   `_q_mat_handles`, `SourceContainerKind::Uop`, the non-Windows DXGI stub).
   Legacy fog, the FSR shader, unused fonts and research directories need an
   explicit decision first.
3. **Ownership moves**, starting with files that rarely change: the `world/`
   catalog, `Player` and teleport (their files are unchanged since the audit),
   then the mechanical split of `statics_collect.rs`, then one `render/lighting`
   owner for the static-light, light-sprite, light-scene, night and cover code
   with world analysis (caves, z-view, cell queries) under `world/`.
4. **State and export contracts** (plan Phase 2), after the maintainer decides
   ST1 retry/reporting, ST2 cadence, D4 undo meaning, D10 dialog model and
   E2/E3 export inputs.
5. **Larger render-path redesigns** with their own measurements: NEW5, NEW6,
   NEW27, NEW29, NEW4 and the land light buffer from NEW3.

The existing `frame_benchmark.py` keeps the camera still, so it does not
exercise streaming, collection, pinning or uploads; the per-frame items above
need a moving scenario (scripted walk and teleport over the control socket)
before their gains can be stated.
