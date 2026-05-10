# UODynamapper Roadmap and Technical TODO

This document is the canonical implementation roadmap for graphics and asset streaming.

Primary targets:
- 400+ FPS in military orthographic gameplay mode.
- 144+ FPS minimum in advanced perspective features.
- Low RAM/VRAM footprint with strict batching and cache discipline.
- Baseline hardware: GTX 1060 class GPU.

Hard constraints:
- No Mesh Shaders or vendor-only rendering features in core runtime path.
- Partial texture uploads must be batched at least once per frame, never one upload per object.
- Metadata used for gameplay logic must remain lossless end-to-end.

## Architecture Principles

- GPU-driven rendering: CPU submits compact state, shader code performs reconstruction/compositing.
- Data indirection: immutable geometry + mutable metadata textures/buffers.
- Bounded memory: LRU with hysteresis for streamed content.
- Streaming-first I/O: mmap + async tasks + upload scheduler.
- Format-aware compression: codec chosen by payload type and payload size.

## Package and File Format Plan

- Container package: UDDP (UODynamapper Data Package), based on current UOP-compatible logic.
- Single-object wrapper: UDDF (UODynamapper Data File).
- Keep index and dictionary sections near header for fast startup lookup.
- Keep visual payload and logic payload physically separable in package layout.

### Compression Header Bit Layout (u16)

Planned packed semantics:
- Bits 0..3: base compression type (0..15)
- Bits 4..6: custom compression type (0..7)
- Bits 7..15: content type id (0..511)

Notes:
- Compression and content selections are mutually exclusive enums, not bitmask combinations.
- Content type drives decoding pipeline and optional preprocessing reversal.

## Compression Policy Matrix

- Land visual textures (runtime sampled): BC7 payload, usually stored raw in package (no second compression).
- Metadata textures for logic: lossless integer formats in VRAM; package compression via Zstd/LZ4 depending on chunk size.
- Small binary metadata (~2 KB): Zstd with per-content dictionaries.
- Medium chunk payloads (around 16 KB): default to Zstd level 1..3; evaluate LZ4 where decompression latency dominates.
- Tiny art tiles (8x8, 12x30, 16x16): never compress one-by-one; cluster into larger groups first.

## Milestone Roadmap

## M0. Baseline and Instrumentation

- [x] Add frame-time breakdown telemetry: CPU update, extraction, upload, render.
- [x] Add upload statistics: queued writes, bytes/frame, batch count.
- [x] Add cache diagnostics: LRU hit rate, eviction reason, hysteresis counters.
- [ ] Define golden benchmark scenes and reproducible camera paths.

Acceptance:
- [x] Metrics visible in debug overlay and persisted in logs for A/B runs.

## M1. Static Art Pipeline (Orthographic Military)

- [ ] Implement art clustering in UDDP (group N art records per package entry, target payload 32..96 KB uncompressed).
- [ ] Implement atlas packing with guillotiere for static art.
- [ ] Support two runtime storage modes for art atlas pages:
  - [ ] RGBA8 (lossless default for tiny/alpha-sensitive art)
  - [ ] BC7 (optional for larger pages)
- [ ] Build metadata table per art id: atlas layer, UV rectangle, pivot/offset, behavior flags.
- [ ] Implement shader path with alpha mask and depth write for clean overlap.

Acceptance:
- [ ] One shared material path for static art in military mode.
- [ ] No visible sorting artifacts on masked sprites in baseline test map.

### M1.x UO-Style Static Depth, Transparency, and Building Occlusion Follow-Up

Status:
- Current runtime state after the first fixed-depth port:
  - foliage ordering improved materially
  - terrain-versus-land-static fighting improved partially
  - remaining wall-vs-wall and similar same-band fighting is still effectively unchanged in live testing
- Conclusion:
  - the current projected logical-depth path is not sufficient
  - the next required step is an explicit UO-style depth key derived from tile coordinates and priority Z, not more ad hoc bias tuning

#### Technical Specification

- Replace clip-space-derived logical ordering for statics with an explicit UO ordering metric.
- Keep visual billboard placement and logical depth computation independent.
- Preserve the current split between:
  - visual position (`world_y`, billboard offsets, ground quad placement)
  - logical ordering inputs (`tile_x`, `tile_y`, `priority_z_units`, class depth offset)

Target logical depth model:

- `priority_z_units` is the effective UO Z used for ordering and occlusion, not necessarily the raw tile base Z.
- `depth_key = (tile_x + tile_y) + (127 + priority_z_units) * 0.01`
- Convert `depth_key` to stable GPU `frag_depth` in a deterministic way.
- Do not feed visual Y bias into the logical depth formula.

Priority Z rules to preserve:

- Default static priority Z: `tile_z + effective_height`
- Effective height rules:
  - use metadata `height` when nonzero
  - if `height == 0` and the tile is not `Background` and not `Surface`, use fallback height `10`
  - if `Bridge`, halve the effective height
- Surface-like floor/land-backed static tiles should keep using the base tile Z unless a verified client rule requires otherwise.

Flag-based logical depth offset rules currently targeted from client analysis:

- `Background` -> `-0.001`
- `Roof` -> `+0.002`
- `Foliage` -> client reference uses a much larger positive offset; runtime tuning in this renderer may differ
- default -> `0.0`

Depth-class precedence must remain:

- `Background`
- `Roof`
- `Foliage`
- `SurfaceLikeFloor`
- `Regular`

Transparency / second-pass behavior:

- Opaque art/static pass:
  - alpha-mask style discard
  - writes logical `frag_depth`
- Transparent art/static pass:
  - draws alpha below the cutoff only
  - should not reuse opaque fixed-depth blindly if that causes artifacts
  - uses client-style color modulation for translucent pixels

Opacity smoothing target behavior to port:

- introduce timer-based alpha lerp rather than sudden opacity changes
- target cadence: 20 ms
- target step: 25 alpha units
- full fade duration target: about 220 ms
- applies to foliage fades, roofs, circle/transparency effects, and upper-floor occlusion fades

Building / roof / upper-floor occlusion target behavior:

- add `_maxZ` / `_maxGroundZ` style state derived from a local vertical scan at the player tile
- detect overhead `Surface` / `Roof` above `player_z + 14`
- when found, treat that overhead surface as the current building ceiling/floor cutoff
- roofs above that level can be hidden
- upper-floor statics above that level should fade or be excluded according to the later alpha/occlusion policy

UO coordinate translation notes to preserve:

- screen-space vertical movement: 1 UO Z unit corresponds to 4 screen pixels in the classic isometric projection
- current render-side world placement still uses the engine's own world units for visuals
- the important porting requirement is that logical ordering follows UO depth semantics even if the visual world-space scale differs

#### Implementation Plan

- [ ] Freeze the explicit UO depth-key formula in code comments, docs, and shader/Rust contracts before further tuning.
- [ ] Add `tile_x`, `tile_y`, and `priority_z_units` as first-class logical-depth inputs for static instances.
- [ ] Stop using projected clip depth as the final ordering source for statics; use the explicit UO depth key instead.
- [ ] Reapply the class offset table on top of `priority_z_units`, not on visual Y.
- [ ] Re-tune foliage only after the explicit depth key is live.
- [ ] Implement timer-based alpha lerp for fade-in/fade-out behavior.
- [ ] Add player-local `_maxZ` building scan.
- [ ] Hide/fade roofs and upper floors above `_maxZ`.
- [ ] Validate all of the above in dense town/building scenes with equal-Z walls, foliage near walls, and terrain/land-static overlap.

#### Validation Requirements

- Verify equal-Z walls in towns/buildings specifically; this is the current known failure that remained unchanged after the priority-height experiment.
- Verify terrain versus land-backed static tiles after explicit depth-key migration.
- Verify foliage against walls and dense tree clusters after the explicit depth-key migration, not before.
- Verify roof hiding and upper-floor fade behavior while walking into and out of buildings.
- Record representative before/after captures for regression comparison.

## M2. Streaming Core (UDDP/UDDF + mmap + upload scheduler)

- [x] Finalize UDDP on-disk layout with sections:
  - [x] header
  - [x] dictionary index section
  - [x] dictionary blobs section
  - [x] entry table
  - [x] data blocks
- [x] Keep UDDF as single-object envelope for standalone resources and tooling.
- [x] Implement async read/decompress tasks via Bevy task pools.
- [x] Implement upload scheduler that merges writes and emits bounded GPU copy batches each frame.
- [ ] Implement per-content dictionary handling for small payload classes.
- [ ] Implement fallback decode paths per content type.
- [ ] Adopt `64x64` as the preferred UDDP transport/decompression unit for terrain payloads.
- [ ] Keep transport granularity separate from render granularity; do not force render entities to match UDDP chunk size.
- [ ] Treat predictive prefetch rings as optional until telemetry proves they improve terrain streaming under real motion.

Acceptance:
- [ ] No main-thread stalls during normal camera motion.
- [ ] Upload bursts remain bounded under stress camera sweeps.

## M3. Land Clipmap and Continuous LOD

- [ ] Keep the current threshold-based terrain scale/mesh path explicitly documented as transitional.
- [ ] Replace discrete zoom mesh swapping with GPU clipmap rings.
- [ ] Implement snapped world-space sampling in vertex stage.
- [ ] Use a continuous LOD path so zoom no longer depends on hard render-threshold swaps.
- [ ] Add live-edit compatible terrain invalidation/update rules for the chosen clipmap path.

Acceptance:
- [ ] No zoom-threshold redraw hitches.
- [ ] Stable visual continuity at all zoom levels.

## M4. Camera Modes and Material Switches

- [ ] Keep camera projection concerns in camera setup, not content data.
- [ ] Implement shader specialization switches for:
  - [ ] military orthographic mode
  - [ ] free orthographic mode with billboarding where enabled
  - [ ] perspective mode with optional enhanced effects
- [ ] Keep minimal register pressure in military mode variant.
- [ ] Treat shader specialization as benchmark-first if the unified shader path already holds the military-mode target.

Acceptance:
- [ ] One-time switch hitch acceptable; no recurring hitch after pipeline warmup.
- [ ] 400+ FPS maintained in military mode on target scene.

## M5. Terrain Blending and Water Modes

- [ ] M5.1 Terrain blend modes:
  - [ ] Keep original transition-tile path for classic fidelity.
  - [ ] Add automatic blend mode for custom maps lacking transition tiles.
  - [ ] Keep source logical tile IDs immutable in both modes.
  - [ ] Treat stochastic/noise seam refinement as optional after the base auto-blend path works.
- [ ] M5.2 Water modes:
  - [ ] Keep classic frame-based water as the baseline mode.
  - [ ] Add enhanced procedural water with depth-aware tinting only if benchmark cost is acceptable.
- [ ] M5.3 Pass ordering correctness:
  - [ ] Render opaque terrain with water fragments excluded.
  - [ ] Render submerged art/mobile content before transparent water.
  - [ ] Render transparent water as the final water pass.

Acceptance:
- [ ] Mode switches produce deterministic visuals without state corruption.

## M6. Mobiles and Paperdoll Foundation

- [ ] Define animation asset normalization path (classic and KR/EC sources).
- [ ] Store a hue mask channel for tintable pixels.
- [ ] Keep hue as runtime parameter and apply in fragment shader (no per-hue atlas generation).
- [ ] Use second UV set for hue/runtime payload where practical.
- [ ] Implement mobile streaming cache with LRU + hysteresis.

Acceptance:
- [ ] Multiple hue variants share same base atlas allocation.
- [ ] No dynamic atlas duplication per hue.

## M7. Mobile Advanced Composition

- [ ] Evaluate two composition paths and pick per platform profile:
  - [ ] multi-quad layered composition
  - [ ] single-quad fragment compositing
- [ ] Preserve deterministic layering (mount/body/equipment/hair etc.).
- [ ] Validate depth behavior against world statics in all camera modes.

Acceptance:
- [ ] Correct visual order in combat-density test scenes.

## M8. Export and Tooling

- [ ] Implement 1:1 tiled world export pipeline.
- [ ] Add WebP output profile controls (quality/lossless/speed).
- [ ] Add CLI pack validation for UDDP consistency and dictionary coverage.

Acceptance:
- [ ] End-to-end reproducible export from same content build.

## Risks and Mitigations

- Risk: over-fragmented atlas allocation under long sessions.
  - Mitigation: periodic defrag/copy plan + migration map updates.
- Risk: too many small write_texture calls.
  - Mitigation: per-frame upload queue coalescing and hard caps.
- Risk: over-specialized compression policy complexity.
  - Mitigation: keep policy table centralized by content id.

## Deferred but Tracked

- [ ] Improve `uddp_inspector` atlas preview path with selective CPU decode for BC7 sub-rects instead of decoding full atlas pages first. Keep this explicitly lower priority than the RGBA path, because most current UDDP texture payloads are plain uncompressed RGBA rather than BC7.
- [ ] Full lighting model parity with original client time-of-day rules.
- [ ] Networking protocol integration.
- [ ] Editor UX layer for live map sculpting and tile painting.
