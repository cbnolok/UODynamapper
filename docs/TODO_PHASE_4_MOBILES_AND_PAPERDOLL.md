# Phase 4: Mobiles, Animations, Hues, and Paperdoll Composition

Phase 4 introduces dynamic entity rendering with strict controls on VRAM growth and frame-time stability.

Hard targets:
- No per-hue atlas duplication.
- Stable frame pacing in crowded scenes.
- Compatibility with both classic indexed assets and KR/EC true-color sources.

## 1. Source Normalization and Runtime Canonical Format

Source families:
- Classic indexed/palette-based animation frames.
- KR/EC true-color frames.

Runtime canonical representation:
- Base atlas: `RGBA8`.
- Hue mask atlas: `R8`.

Rationale:
- Single shader path for both source families.
- Runtime hue changes via shader, no texture rebuild.

## 1.1 Mask Generation Rules

Classic source:
- Generate mask offline from hue-eligible index ranges.

KR/EC source:
- Use native tint mask where available.
- If absent, derive with explicit offline rules, not runtime heuristics.

## 2. Animation Packaging and Compression

## 2.1 Clustered Records (Mandatory)

Do not compress tiny frames independently.

Required:
- Group frames into cluster payloads sized for decoder efficiency.
- Keep frame index table (`frame_id -> offset, size, w, h, pivot, dt`).

## 2.2 Compression Policy

Default:
- Zstd for clustered frame payloads.

BC7 policy:
- BC7 is optional and content-scoped.
- Avoid BC7 for tiny alpha-heavy frame sets by default.
- Allow BC7 for large pre-baked atlases only after artifact and throughput validation.

## 2.3 Why Not Per-Frame BC7 by Default

- Tiny and irregular frame sizes amplify block overhead.
- Alpha-edge quality is more sensitive on pixel-art silhouettes.
- Zstd on clustered RGBA+mask often gives better disk and streaming behavior for this class.

## 3. Runtime Hue Pipeline

## 3.1 Inputs

- Base sampled color from RGBA atlas.
- Hue mask from R8 atlas.
- Hue ID from runtime entity data.
- Global hue lookup texture/table.

## 3.2 Fragment Logic

Per-pixel:
1. Sample base RGBA.
2. Early discard on alpha threshold.
3. Sample mask.
4. If mask allows tint and hue id != 0, compute tinted color via hue lookup.
5. Output final color with base alpha.

## 3.3 Attribute Transport

Constraint:
- Avoid custom vertex attributes where possible.

Allowed method:
- Reuse existing secondary UV channel to carry compact per-instance hue/runtime payload.

This must remain documented and deterministic to avoid future pipeline confusion.

## 4. Animation Time and Frame Selection

CPU responsibilities:
- State transitions only (idle/walk/run/attack/etc.).
- Start time and direction updates.

GPU/shader responsibilities (or compact runtime evaluator):
- Compute frame index from `global_time`, `start_time`, `fps`, `frame_count`.

Frame metadata table must include:
- UV region.
- pivot offsets.
- duration/frame timing.
- atlas layer/page binding.

## 5. Composition Model for Paperdoll Layers

Layer families:
- mount
- body
- torso/legs equipment
- weapon/shield
- hair/beard
- optional overlays/effects

Two implementation paths:

Path A: multi-quad layering
- Easier debugging and tooling.
- Higher draw and state overhead.

Path B: single-quad compositing in shader
- Lower draw overhead.
- Heavier fragment cost and stricter metadata correctness requirements.

Decision gate:
- Choose after benchmark on GTX 1060 baseline scene.
- Keep both paths possible behind profile flags until final lock.

## 6. Depth, Ordering, and Stability

Requirements:
- Mobiles depth-test correctly with terrain/statics.
- Rider/mount and equipment overlays remain stable under movement.
- Deterministic tie-break policy when entities share near-identical depth.

Recommended tie-break inputs:
- tile coordinate
- z value
- stable entity id

## 7. Streaming and Residency for Mobiles

Reuse Phase 2 discipline:
- LRU + hysteresis for decoded frame clusters and VRAM residency.
- Prioritize currently visible animation states.
- Batch frame uploads in scheduler; no uncontrolled direct writes.

Burst control requirement:
- Mass state transitions (for example combat spikes) must respect upload budget caps.

## 8. Profiling and Acceptance

## 8.1 Functional

- Hue changes visible immediately without atlas rebuild.
- Layer ordering remains correct across representative equipment sets.
- No pivot jitter between consecutive frames.

## 8.2 Performance

Stress scenario:
- High-density crowd with mixed animation states and hue variation.

Pass criteria:
- Stable frame pacing (no sustained spikes caused by animation streaming).
- Upload queue remains bounded under sustained combat transitions.
- Composition path meets baseline thresholds on GTX 1060 profile.

## 9. Deliverables

- Animation build pipeline with clustered packaging.
- Runtime loader for clustered decode and residency management.
- Unified RGBA+mask hue shader path integrated.
- Benchmark report comparing composition path A vs B and final selected default.
