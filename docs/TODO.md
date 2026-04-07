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

- [ ] Add frame-time breakdown telemetry: CPU update, extraction, upload, render.
- [ ] Add upload statistics: queued writes, bytes/frame, batch count.
- [ ] Add cache diagnostics: LRU hit rate, eviction reason, hysteresis counters.
- [ ] Define golden benchmark scenes and reproducible camera paths.

Acceptance:
- [ ] Metrics visible in debug overlay and persisted in logs for A/B runs.

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

## M2. Streaming Core (UDDP/UDDF + mmap + upload scheduler)

- [ ] Finalize UDDP on-disk layout with sections:
  - [ ] header
  - [ ] dictionary index section
  - [ ] dictionary blobs section
  - [ ] entry table
  - [ ] data blocks
- [ ] Keep UDDF as single-object envelope for standalone resources and tooling.
- [ ] Implement async read/decompress tasks via Bevy task pools.
- [ ] Implement upload scheduler that merges writes and emits bounded GPU copy batches each frame.
- [ ] Implement per-content dictionary handling for small payload classes.
- [ ] Implement fallback decode paths per content type.

Acceptance:
- [ ] No main-thread stalls during normal camera motion.
- [ ] Upload bursts remain bounded under stress camera sweeps.

## M3. Land Clipmap and Continuous LOD

- [ ] Replace discrete zoom mesh swapping with GPU clipmap rings.
- [ ] Implement snapped world-space sampling in vertex stage.
- [ ] Use metadata mip chain for continuous LOD.
- [ ] Add live editing compatibility (single-tile edits + mip maintenance).

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

Acceptance:
- [ ] One-time switch hitch acceptable; no recurring hitch after pipeline warmup.
- [ ] 400+ FPS maintained in military mode on target scene.

## M5. Terrain Blending and Water Modes

- [ ] Keep original transition-tile path for classic fidelity.
- [ ] Add optional automatic blending path for custom maps lacking transition tiles.
- [ ] Add dual water mode:
  - [ ] classic frame-based water
  - [ ] enhanced procedural water with depth-aware tinting
- [ ] Implement correct render pass ordering for transparent water over submerged art.

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

- [ ] Full lighting model parity with original client time-of-day rules.
- [ ] Networking protocol integration.
- [ ] Editor UX layer for live map sculpting and tile painting.
