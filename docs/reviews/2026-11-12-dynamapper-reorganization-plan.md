# Dynamapper reorganization plan

**Status:** proposed implementation plan; no code moves are implied by writing
this document. The target is a guide, not a requirement to create every file on
day one. Recheck the current tree and the [architecture audit](2026-11-08-dynamapper-architecture-audit.md)
before each phase; its findings are mostly static observations, not a blanket
claim that every conditional failure has been reproduced.

## Objective and boundaries

Give logical world data, content loading, rendering, UI, settings and export
clear owners. Reduce duplicated policies and obsolete code, including scattered
shader helpers, while retaining current functionality. Keep one `dynamapper`
binary crate unless an actual external consumer justifies a separate crate.
Avoid one-function files and new generic frameworks made solely for the move.

The intended dependency direction is world/content data into rendering, with
input, UI and export using explicit world and render-facing interfaces. Bevy
systems can coordinate these areas; the rule concerns ownership, not a demand
that every dependency form a perfectly layered graph. In particular, world
models must not own render mesh handles or GPU textures.

### Contracts to preserve

- Logical map coordinates, original signed Z, tile IDs, object placement and
  movement data remain separate from visual geometry and depth ordering.
- Classic uses manually placed terrain-like static transition art. Classic and
  EC/KR use the same map tiles, stored differently. EC/KR dynamically blend
  textures; Dynamapper's optional dynamic blending for Classic and EC/KR texture
  sources is unfinished. Preserve the placed artwork and logical tile identity.
  See [land textures and transitions](../dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md).
- Missing EC terrain has explicit missing-texture behavior; do not silently
  replace it with Classic terrain. Source formats and roles follow current
  package contracts and executable evidence, not old filename-based guesses.
- Desired chunk targets, actually resident rendered chunks, and GPU-ready data
  are different states. Static collection, lights and picking must continue to
  use the right state during handoffs. Terrain picking must preserve assigned
  LOD mesh topology, submitted metadata and original logical height; static
  picking must preserve production alpha/depth coverage and asynchronous
  readback. See [terrain cursor](../agent_notes/terrain_cursor_surface.md).
- Keep cache image/slot identity, generation, staging, render-world upload and
  readiness coherent across replacement, resize and eviction. Art and land may
  share lifecycle concepts while retaining their different allocation and
  residency policies. Export must use authoritative readiness.
- Keep active style groups Classic, Enhanced, KR-like and UOD-Painterly;
  KR-like and UOD-Painterly share a shading mode but not their preset group.
  Preserve active static-light collection and its separate terrain/art response
  and visible mask controls. See [static light visibility](../agent_notes/static_light_visibility.md).
- Preserve actual package/source format support, including DXT1/BC1 where used
  and BC7 where used. Do not design a common compression interface around BC7
  alone; potential mobile codecs are future work, not a phase requirement.
- Preserve strict hand-written config parsing, lenient app-written cache
  migration, existing serialized formats, configured controls, plugin ordering,
  feature gates and export isolation. Resolve failures explicitly; don't silently
  change fallback semantics as part of a move.
- Preserve the active style/time preset slot, scoped factory reset and the slot
  being left before switching. Keep the terrain picker's CPU liquid-bank mirror
  consistent with its shader counterpart.

## Proposed Rust layout

Names below describe the intended final ownership. Use fewer files if that keeps
a coherent unit together, and adjust the tree to current source as work proceeds.

```text
dynamapper/src/
├── main.rs
├── app/                 bootstrap, states, schedules, window, logging
├── settings/            model, loading, saving, shader presets
├── world/               coordinates, catalog, map/static stores, multis,
│                        player, movement, teleport, streaming selection/loading
├── content/             package discovery/resources, gump provider,
│                        paperdoll composition and override policy
├── input/               actions, bindings, dispatch, dev control and input
├── render/
│   ├── view/            camera and projection
│   ├── scene/           rendered chunk/entity lifecycle and residency
│   ├── assets/          art/land pages, shared item auxiliary resources,
│   │                    upload lifecycle and GPU readiness
│   ├── terrain/         mesh, draw, material, sampling/coverage, transitions
│   ├── statics/         source, geometry, collection, chunk cache, draw,
│   │                    material and ordering
│   ├── lighting/        shared static-light data/response and mask drawing
│   ├── picking/         separate terrain query and static GPU picker
│   └── postprocess/     actual render passes such as fog
├── ui/                  context, preferences, shader controls, dialogs,
│                        gump windows and overlays
├── export/              job, readiness, capture, spool, provenance
└── diagnostics/         frame rate/limiter, metrics, render timing
```

Concrete moves and splits:

| Current source | Intended responsibility |
| --- | --- |
| `core.rs`, `core/app_states.rs`, `core/system_sets.rs`, logging setup | `app/`; keep app construction and plugin wiring small. |
| `configs/settings.rs`, `configs/shader_presets.rs` | `settings/` owns the validated effective snapshot, persistence and preset state. Runtime appliers belong with their affected systems, not UI. |
| `core/maps.rs`, `core/statics.rs`, `core/multis.rs`, map catalog in `render/scene/world.rs` | `world/`; preserve logical data and package-store boundaries. |
| `render/scene/player.rs`, `controls/player_movement.rs`, `controls/teleport.rs` | Logical player and operations in `world/`; rendered position checkpoints stay with scene/streaming. |
| `core/uo_files_loader.rs` | Separate package discovery/publication from the runtime gump provider under `content/`; retain Classic fallbacks and shared-hash checks. |
| `render/scene.rs`, `land/{chunk_loader,setup_base_mesh,draw_mesh,picking}.rs` | View selection/loading versus actual rendered chunk lifecycle versus terrain rendering/picking. Do not replace resident meshes with desired targets. |
| `world/art/statics_collect.rs`, `statics_draw.rs` | Split source policy, geometry, chunk cache, collection, material/buffers, batch synchronization and ordering under `render/statics/`. Preserve all opaque, transparent, shadow and picking paths. |
| `world/art/static_lights.rs` | Shared collected light data and response under `render/lighting/`, with mask entity drawing separately owned. |
| `texture_cache/{art,land,ec_item_*}` | Page caches under `render/assets/`; extract shared EC auxiliary texture construction from terrain setup. Keep terrain coverage with terrain and maintain atlas/lookup/sentinel lifetimes. |
| `render/dialogs/`, `render/overlays/`, cursor interaction | Presentation under `ui/`; input dispatch under `input/`, validated teleport under `world/`, render picking under `render/picking/`. Separate paperdoll UI from reusable composition/persistence. |
| `core/export_job.rs` | `export/` owns job lifecycle, readiness, capture/spool and provenance through explicit interfaces. |
| `core/frame_{rate,limiter}.rs`, `core/diagnostics.rs` | `diagnostics/` or runtime owner; preferences edits settings rather than owning the limiter. |

## Proposed shader layout

Organize by rendering responsibility and actual execution stage, not one copy
per client style. This is a revised proposal incorporating consolidation of small
related WESL files:

```text
dynamapper/assets/shaders/
├── common/              shared bindings, coordinates, projection, texture data,
│                        noise and texture filtering
├── terrain/             main entry point, bindings, atlas, sampling, coverage,
│                        transitions, normals, lighting, shading, liquid bank
├── statics/             sprite and ground entry points/bindings, depth, hue,
│                        shading, surface blending, picking and WGSL resolve
├── materials/ec/        liquid and camera, item water/liquid, item surface,
│                        item quad geometry
├── effects/             material-stage surface effects, color, grunge, KR wash
└── passes/              fullscreen fog and other actual passes
```

Candidate consolidations, subject to tracing imports and keeping each result
cohesive:

1. Combine `ec_item_liquid.wesl` and `ec_item_water.wesl` as item-water logic;
   retain item-surface handling as a separate responsibility.
2. Combine `ec_liquid.wesl` and the small `ec_liquid_camera.wesl` helper while
   preserving shared terrain/static use.
3. Combine the short color grading, tonemapping and global-lighting helpers as
   `effects/color.wesl`; callers still explicitly determine operation order.
4. After verifying and removing unused experimental helpers, assess combining
   `pixel_art_filters.wesl` and `client_filter.wesl` as common texture filtering.

Move genuinely shared noise out of `world/land/`; retain active shared functions.
The current `postprocess/` color functions are called within material shaders,
whereas fog is a fullscreen pass. Keep terrain, sprite and ground entry points,
their distinct bindings and depth semantics, and the active picking variants.
The legacy fog and unused FSR candidate require audit-based verification before
removal; EC/KR reference translations are research with provenance, not active
runtime passes or automatically disposable files.

After path ownership is established, split oversized terrain/sprite entry points
by cohesive stages where useful. Do not shift work between vertex and fragment
stages merely because files move. Preserve the cached and uncached terrain
coverage arrangement: [the cache note](../agent_notes/ec_terrain_stack_cache.md)
documents active fallback semantics and measured compiler consequences.

Every shader move must update WESL asset-derived import paths, Rust shader entry
references, `include_str!` paths, virtual paths in composition/GPU harnesses,
packaging and affected documentation together. Keep imports before declarations,
Rust/WESL bindings and flag layouts aligned, and `STATIC_PICK`/MSAA variants
working. A file merge alone is not a performance claim.

## Phases and acceptance

Work one plugin or coherent ownership boundary at a time. A phase is complete
only when the code compiles, its affected callers and tests use the new owner,
behavioral contracts are checked, and the diff is reviewable. Commit validated
phases according to `AGENTS.md`; stage only relevant files. Keep tests near the
behavior they exercise and remove temporary migration aliases when their final
consumer has moved.

1. **Revalidate and characterize.** Inspect git status and current source; map
   each affected plugin, system ordering, features and tests. Preserve unrelated
   edits. Record current behavior and reproduce an audit finding before changing
   it where practical. Do not regard a historical audit as proof of present code.
2. **Protect state and export.** Address settings write/retry and independent
   debounce (ST1–ST3), preset undo/invalid keys (D4, SP1–SP2), FPS preference
   handling (D8–D9), and export timeout/provenance/isolation (E1–E3). Separate
   behavior fixes from mechanical moves where practical; test failures and
   recovery, slot switching, and resume mismatch. Establish the contract before
   changing serialization or external sidecars.
3. **World and settings ownership.** Move logical coordinates, player, map
   catalog and teleport behind world interfaces; consolidate settings loading
   into one startup snapshot and move runtime setting application out of UI.
   Preserve Bevy scheduling and existing controls (C1, P1, W1, D5, ST4).
4. **Content and shared GPU assets.** Split runtime gump access from startup
   loading; extract shared item auxiliary resources from terrain. Establish
   image/slot generation and upload-completion/readiness contracts (LC6,
   A7–A8, E3), preserving source fallback, resize and upload ordering.
5. **Terrain and scene streaming.** Move selection/loading, rendered residency,
   terrain meshes/materials/coverage and terrain query into their owners. Keep
   target/resident distinction, partial chunks, assigned LOD and logical-height
   contracts; address audit defects in focused changes (S1–S7, T1–T17, LC1–LC11,
   N11). Do not remove the fixed-array path until its reachability and missing
   source contract are resolved.
6. **Statics, lighting and picking.** Split collection, geometry, source routing,
   cache and draw systems; unify duplicated policy only where behavior matches.
   Separate shared light data from mask drawing and keep GPU static picking
   consistent with admitted rendered instances (A1–A16). Investigate ambiguous
   multi IDs (A9) and cover-rule precedence (D11) against their source contract
   before adopting a behavior change.
7. **UI, export and diagnostics.** Move dialogs/overlays out of render, route
   interaction through shared operations, split export lifecycle and put the
   limiter and telemetry with runtime owners. Preserve input capture, user
   settings and active picker data (C4–C7, O1–O8, D1–D13, R1–R4).
8. **Shader ownership and cleanup.** Move and consolidate shader families in
   small dependency-aware groups, then remove only confirmed unused code and
   experiments (H1–H3, N1–N2). Keep test-only research and active conditional
   paths. Update asset paths, packaging, documentation and the audit/roadmap.

## Validation and handoff

- For each Rust phase, run `cargo check -p dynamapper` and focused tests for the
  changed contract. Run `just bevy-lint` when touching Bevy plugin/system code;
  run `cargo check --workspace` if cross-crate interfaces change. Do not run
  `cargo fmt`, `rustfmt` or `cargo clippy` automatically.
- For shader moves, retain and update the existing entry-point dependency and
  sampling composition tests; note that they cover only their specific imports
  and variants, not the full pipeline. Run the relevant existing GPU tests when
  available and verify sprite/ground picking variants, including MSAA. Useful
  targeted commands include:

  ```sh
  cargo test -p dynamapper world_entry_points_discover_shader_dependencies
  cargo test -p dynamapper land_sampling_shader_composes
  cargo test -p dynamapper liquid_sampling_gpu_preserves_data_and_source_edges -- --ignored
  cargo test -p dynamapper pixel_query_uses_opaque_depth_and_resolves_discrete_msaa_ids -- --ignored
  cargo test -p dynamapper local_light_intensity_preserves_unlit_art_and_scales_collected_light -- --ignored
  ```

  The ignored tests require a suitable GPU/backend; report when unavailable.
- After visual changes, exercise all four styles as required by `AGENTS.md`.
  Report which runtime/GPU checks were actually run; do not infer success from
  a blank frame or a compilation-only check. The user did not require visual
  comparisons for the audit; do not add reference comparisons merely as an
  audit prerequisite.
- For each completed phase, document changes, test results and unresolved
  contracts in the relevant project documentation. Update this plan or the audit
  as findings become resolved; retain evidence and distinguish a conditional
  risk from an observed defect.
