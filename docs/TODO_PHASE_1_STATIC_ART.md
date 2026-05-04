# Phase 1: Static Art Pipeline (Production Technical Spec)

Phase 1 establishes the first production-grade static-art path with strict performance constraints and zero ambiguity in data ownership.

Hard targets:
- GTX 1060 baseline compatibility.
- 400+ FPS in military orthographic benchmark scene.
- No CPU Y-sorting for the default static path.
- Bounded upload cost through batched partial writes.

## 1. Scope

In scope:
- Static art extraction from classic UO sources.
- Runtime atlas residency, metadata indirection, and draw path.
- Deterministic depth correctness using alpha mask + depth buffer.

Out of scope:
- Full mobiles/paperdoll compositing (Phase 4).
- Perspective/POM heavy path (Phase 3).

## 2. Build-Time Asset Pipeline

## 2.1 Inputs

- `artidx.mul`/`art.mul` for static art frames.
- `tiledata.mul` flags for behavior (`Wall`, `Window`, `Roof`, etc.).
- Optional KR/EC assets for future extension (not required for Phase 1 runtime).

If EC assets are present, they should be understood as a mixed source bundle rather than a single texture list:
- `Texture.uop` and `LegacyTexture.uop` are the shared texture pools.
- `tileart.uop` supplies EC static-art ownership, shader/type hints, and per-entry sampling windows.
- `TerrainDefinition.uop` supplies EC land ownership, aliases, and terrain-material semantics.
- `string_dictionary.uop` and `string_Wdictionary.uop` are supporting EC resources, not render inputs.

## 2.2 Decode Rules

- Decode source art losslessly to CPU intermediate.
- Preserve hard alpha edges; no filtering, no color-space smoothing.
- Emit canonical per-art record:
  - `art_id: u32`
  - `width: u16`
  - `height: u16`
  - `pivot_x: i16`
  - `pivot_y: i16`
  - `behavior_flags: u32`

## 2.3 Clustered Packaging (Mandatory)

Do not store tiny art tiles as independent compressed records.

Required build strategy:
- Group art records into clusters targeting 32-96 KB uncompressed payload.
- Keep cluster-local offset table (`art_id -> byte_offset, byte_len`).
- Compress cluster payload with Zstd (level 1-3 default).

Rationale:
- Tiny single-record compression wastes headers and destroys ratio.
- Clustered layout improves filesystem locality and reduces decoder churn.

## 2.4 Atlas Packing

- Use `guillotiere` for rectangle allocation.
- Keep at least 1px gutter (2px recommended) to avoid bleed under non-integer zoom.
- For BC7 atlas pages, enforce 4x4 alignment on allocation and write regions.
- Keep immutable atlas page dimensions per pool (example: 2048x2048).

For the EC path, packing should eventually be driven by semantic ownership first, not by a flat texture-id sweep. The current raw texture pools contain art-like and terrain-like content together, so any EC packer that ignores `tileart.uop` and `TerrainDefinition.uop` will misclassify content.

## 3. Runtime Data Model

## 3.1 Core Runtime Structures

Example layout (guideline):

```rust
#[repr(C)]
pub struct StaticInstance {
    pub world_x: f32,
    pub world_y: f32,
    pub world_z: f32,
    pub art_id: u32,
}

#[repr(C)]
pub struct ArtMeta {
    pub atlas_layer: u16,
    pub flags: u16,
    pub u0: u16,
    pub v0: u16,
    pub w: u16,
    pub h: u16,
    pub pivot_x: i16,
    pub pivot_y: i16,
}
```

`StaticInstance` stays minimal. `ArtMeta` is fetched via indirection.

## 3.2 Metadata Indirection Storage

Allowed options:
- Integer texture (`Rgba16Uint`, `Rgba32Uint`) with nearest sampling.
- Packed read-only buffer/SSBO equivalent.

Rules:
- Metadata reads must be exact (no filtering).
- Art ID to metadata lookup must be O(1).
- Metadata updates (when atlas slot changes) are partial writes, never full-buffer rebuilds.

## 4. Rendering Path

## 4.1 Material and Pass Policy

Default static path is alpha-masked opaque:
- `depth_write_enabled = true`
- `depth_compare = Less` (or `LessEqual` if required by existing terrain path)
- `alpha_mode = Mask(threshold)` semantics in shader/material

Reason:
- Transparent pixels do not write depth.
- Visible opaque pixels participate in proper depth occlusion.
- Eliminates classic quad-over-quad transparency artifacts.

## 4.2 Vertex Stage Responsibilities

- Build quad from unit corners.
- Apply per-art pivot (`pivot_x`, `pivot_y`) in world units.
- Place in world with military ortho camera transform.
- Forward atlas addressing info to fragment stage.

## 4.3 Fragment Stage Responsibilities

- Sample atlas (or atlas array) using metadata-derived UV.
- Apply alpha discard (`if alpha < threshold { discard; }`).
- Return unlit/base color for Phase 1.
- Optional hue hook allowed but disabled by default in this phase.

## 4.4 Draw Submission Strategy

- Batch by material + atlas page/layer set.
- Keep draw-call growth proportional to page count, not object count.
- Avoid per-entity material handles for static world content.

## 5. Upload Scheduler and Batching

Hard requirements:
- No per-object `queue.write_texture` calls from gameplay systems.
- Partial writes are enqueued and flushed in frame batches.

Scheduler responsibilities:
- Group writes by destination texture/layer.
- Coalesce adjacent regions when possible.
- Enforce per-frame byte and op budget.
- Carry over excess work to next frame by priority.

Priority recommendation:
1. Visible this frame.
2. Near-prefetch ring.
3. Far-prefetch ring.

## 6. Cache and Residency

- VRAM atlas slots managed by LRU + hysteresis timer.
- Eviction is logical first (free slot bookkeeping), overwrite on reuse.
- No explicit "clear evicted pixels" operation.

Metrics to track:
- Atlas occupancy ratio.
- Fragmentation ratio.
- Evictions per second.
- Upload bytes/frame.

## 7. Validation Plan

## 7.1 Functional

- Correct art ID to UV mapping for golden sample set.
- Stable pivot alignment across neighboring statics and terrain.
- Correct alpha mask behavior at object boundaries.

## 7.2 Performance

Benchmark scene requirements:
- Dense static city + foliage sample.
- Camera sweep + zoom stress.
- GTX 1060 baseline.

Pass criteria:
- 400+ FPS in military mode baseline profile.
- No sustained upload-induced frame spikes from static streaming.
- Draw calls remain bounded by batching model.

## 7.3 Memory

- Atlas stays under configured cap.
- No unbounded growth of resident static pages.
- LRU hysteresis prevents rapid thrashing at frustum boundaries.

## 8. Deliverables

- Clustered static packer integrated in build pipeline.
- Runtime static loader with metadata indirection.
- Batched upload scheduler integrated.
- Profiling capture package with reproducible benchmark settings.
