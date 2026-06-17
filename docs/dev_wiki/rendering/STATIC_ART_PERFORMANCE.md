# Static Art Runtime Performance Notes

This note records the performance pitfalls found while profiling static art rendering, what fixed them, and the rules to follow when adding new systems in the same area.

The main affected runtime path is:

```text
dynamapper/src/core/render/scene/world/art/statics_collect.rs
```

Related draw and asset paths:

```text
dynamapper/src/core/render/scene/world/art/statics_draw.rs
dynamapper/src/core/render/scene/world/art/static_lights.rs
dynamapper/src/core/texture_cache/art.rs
lib/udd-assets/src/hues.rs
```

## Pitfalls Found

### Rebuilding Visible Static Chunks On Steady Frames

`sys_collect_visible_statics` originally spent too much CPU time on frames where the player had not moved and no visible chunks had changed.

The expensive pattern was:

- walk visible land chunk entities every frame
- rebuild or retry static chunk data too often
- clear output vectors
- copy cached chunk instance vectors back into the output buffers
- cause downstream sync systems to see changed data

The fix was to make steady frames return from cache:

- cache per static chunk output in `StaticChunkRenderCache`
- track the visible chunk key list
- return early when the visible set and cache config are unchanged
- retry incomplete chunks only when atlas state has changed
- keep output buffers stable on skipped retry frames

New systems should not clear and repopulate large resources on an unchanged frame. First prove that the visible set, configuration, and relevant residency state changed.

### Treating Static Lights As Static-Art Geometry Invalidation

Static light decal rendering and CPU-baked static-art tinting are separate concerns.

The expensive pattern was invalidating static-art chunk cache whenever static light instances changed. That forced geometry/art data to rebuild even when only lighting changed.

The fix was:

- keep static light decals controlled by `world_rendering.enable_static_lights`
- make CPU-baked local lighting on static art opt-in via `world_rendering.enable_static_art_local_lights`
- when the opt-in is disabled, do not scan `RenderStaticLightInstances` per static tile
- include the static-light signature in chunk cache config only when CPU-baked static-art lighting is enabled

New static-art features should separate stable geometry/art data from dynamic per-frame visual effects.

### Resolving EC Surface-Like Land Slots Per Tile Per Frame

Surface-like EC statics can route through `tex_land_ec` instead of ordinary art. The original resolution path did repeated package/provenance work inside visible static loops.

The fix was `StaticSurfaceLikeResolutionCache`, keyed by art source and tile graphic, storing both hits and misses.

New resolution code should cache negative results too. A missing EC mapping can be as hot as a successful mapping when many tiles share the same graphic.

### Repeated Static Draw Entity Sync Work

The draw side used Bevy mesh entities for static sprite, shadow, transparent, ground, and ground-transparent batches. Even when batch data was unchanged, sync systems built maps and considered command updates.

The fix was:

- add `StaticBatchEntitySyncState`
- hash desired batch identity and mesh range
- return before map building and command work when the signature is unchanged
- mark local render-only marker/key components with `#[component(clone_behavior = Ignore)]`

Do not assume `Mesh3d` entities can avoid `Transform`; Bevy requires transform data for mesh extraction. To avoid that completely, static art would need a custom render-world path instead of normal Bevy mesh entities.

### Lazy UDDP Or Texture Decoding From Runtime Systems

Package parsing and texture payload decompression must not happen under static collection or per-frame draw systems.

Examples found during profiling:

- `HuesPackage::from_uddp_package` parses `metadata/hues.csv` and validates `textures/hues.rgba8888`
- `HuesPackage::read_texture_bytes()` used to read/decompress the texture entry again
- runtime hue users in static art, static lights, and gump composition called that method after startup

The fix was to store the validated hue texture bytes in `HuesPackage` at load time and expose `HuesPackage::texture_bytes()` for runtime borrowers.

New UDDP readers should follow this rule:

- startup/load path: parse metadata and decode small shared payloads once
- runtime path: borrow cached data or use an explicit page/cache API
- per-frame systems: never call package constructors or general decompression helpers

Large texture collections are the exception: they should use explicit residency/page caches, not ad hoc reads inside gameplay systems.

### Optional Features Still Need Runtime Gates

Unused data paths can dominate CPU even when the UI does not expose them yet.

Examples:

- gump package loading is opt-in through runtime asset settings
- CPU-baked static-art local lights are opt-in
- cursor overlay EC slot resolution is cached and only runs for relevant hovered objects
- Bevy wireframe rendering is only installed when `app.debug.map_render_wireframe` is true at startup; `WireframeConfig.global = false` does not remove the plugin's extract, specialize, queue, or render graph work

New optional systems should have both:

- an asset loading gate so packages are not loaded by default
- a runtime usage gate so systems do no work when the feature is inactive

## Implementation Rules

Use these rules when editing static-art runtime systems:

1. Cache at chunk granularity before optimizing individual tiles.
2. Keep output resources stable on unchanged frames.
3. Cache negative lookups for package routing and metadata resolution.
4. Separate stable render data from dynamic effects.
5. Make expensive optional features opt-in.
6. Do not parse UDDP metadata or decompress package entries from per-frame systems.
7. Avoid repeated Bevy entity command work; use signatures before building maps or spawning/despawning.
8. Treat `ResMut` changes carefully, because they can cascade into downstream work even when values are equivalent.

## Profiling Counters

`StaticArtCollectDebugState` tracks path counters for `sys_collect_visible_statics`:

- `complete_fast_path`
- `incomplete_wait_fast_path`
- `slow_rebuild`
- copied sprite instances
- copied ground instances

When profiling static collection, first check whether steady frames are using a fast path. A stationary camera should not steadily report `slow_rebuild` with large copied instance counts.

Expected steady-frame behavior:

- visible chunks unchanged
- package/config revisions unchanged
- no large output vector rebuild
- no UDDP package constructors under the collector
- no hue texture decompression under the collector

## Last Updated

June 2026
