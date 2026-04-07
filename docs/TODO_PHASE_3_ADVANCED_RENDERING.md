# Phase 3: Advanced Rendering Modes and Unified Runtime Switches

Phase 3 introduces camera and visual-mode expansion while preserving the fast path.

Hard targets:
- Military orthographic remains the primary high-FPS mode (400+ FPS class benchmark).
- Perspective/free modes never drop below 144 FPS in the defined stress profile.
- Runtime mode switches are deterministic and do not require world reload.

## 1. Mode Taxonomy and Ownership

Camera modes:
- Military orthographic.
- Free orthographic.
- Perspective.

Ownership separation (must be preserved):
- Projection math belongs to camera state.
- Billboard/structural/water behavior belongs to material/shader variants.

This prevents camera code from accumulating content-specific branches.

## 2. Shader Variant Strategy

## 2.1 Why Variants (Not One Giant Branchy Shader)

Global uniform branching is valid, but heavy optional code (for example POM loops) can increase register pressure in fast modes.

Policy:
- Use shader defs / pipeline specialization for major mode groups.
- Keep military variant lean and free of expensive perspective-only logic.
- Accept one-time pipeline warmup stutter at first switch.

## 2.2 Required Variant Set

Minimum variant matrix:
- `terrain_military_fast`
- `terrain_free_or_perspective`
- `art_billboard_enabled`
- `art_structural_advanced` (optional heavy path)
- `water_classic`
- `water_enhanced`

Variants are selected by runtime mode and material flags; do not duplicate scene entities per mode.

## 3. GPU Clipmap Terrain

## 3.1 Geometry

- Use nested rings around camera center.
- Ring scales in powers of two.
- Topology is static; only sampling domain shifts with camera.

## 3.2 Vertex Pulling

Vertex stage responsibilities:
- Compute world sample coordinate from camera anchor + local ring coordinate.
- Snap sample coordinate to ring resolution to avoid shimmer.
- Read height/material metadata from lossless metadata texture.
- Emit world position and tile/material IDs to fragment stage.

## 3.3 Metadata Requirements

- Metadata texture must remain lossless (integer format).
- Sampling for logical fields is nearest/unfiltered.
- Mip chain is used for LOD selection, with strict consistency after edits.

## 3.4 Zoom Behavior

- Remove threshold-triggered mesh swap/rebuild.
- Continuous zoom transitions through clipmap level blend logic.
- Avoid hard pop at LOD boundaries.

## 4. Static Art Behavior in Free/Perspective Modes

## 4.1 Category Policy

Categories:
- Billboard category: foliage, effects, entities intended to face camera.
- Structural category: walls/roofs/features that must keep world orientation.

## 4.2 Billboard Math Policy

- Use cylindrical billboard by default for world-grounded sprites.
- Keep origin/footpoint stable on ground plane.
- Avoid per-entity CPU transform updates in hot path.

## 4.3 Structural Advanced Path

Optional heavy path:
- Per-fragment volumetric illusion (for example POM-like behavior).
- Enabled only for flagged structural materials in perspective/free mode.

Do not enable globally; cost amplification is unacceptable.

## 5. Terrain Transition Modes

Runtime switch (no reload):
- Mode A: classic transition tiles only.
- Mode B: automatic stochastic blend for maps without transition assets.

Automatic blend requirements:
- Neighbor-aware blend factor.
- Noise modulation to avoid straight synthetic seams.
- No mutation of source logical tile IDs.

## 6. Water Modes and Pass Order

## 6.1 Water Modes

- Classic: original animated tile sequence path.
- Enhanced: procedural distortion/depth tint path.

## 6.2 Mandatory Pass Ordering

For submerged-asset visibility correctness:
1. Opaque terrain pass with water fragments discarded.
2. Static/mobile pass.
3. Transparent water pass.

This order is required to see submerged art through water while preserving depth semantics.

## 6.3 Enhanced Water Inputs

Enhanced path may use:
- Time uniform.
- Normal/noise map(s).
- Scene depth input for depth-based tint/opacity.

Depth-based tint must be bounded to avoid over-darkening in shallow areas.

## 7. Runtime Switching and State Management

Switchable at runtime:
- camera mode
- terrain blend mode
- water mode
- structural advanced path toggle

Switch behavior requirements:
- No world reload.
- No full atlas rebuild.
- Bounded warmup hitch only on first pipeline compile.

## 8. Performance Governance

Mandatory telemetry:
- frame time breakdown by pass
- variant usage counts
- expensive-path pixel coverage estimate
- upload queue pressure (from Phase 2)

Acceptance criteria:
- Military mode: no regression vs Phase 1 benchmark.
- Perspective stress scene: >= 144 FPS baseline profile.
- No runaway spikes during frequent mode toggles after warm caches.

## 9. Deterministic Export Compatibility

Phase 3 must preserve offline deterministic rendering:
- fixed camera stepping
- fixed quality profile per export mode
- fixed seed for stochastic blend/noise paths

This is required for reproducible map renders and regression comparison.

## 10. Deliverables

- Camera+material variant system integrated.
- Clipmap terrain path replacing threshold LOD mesh swaps.
- Dual-mode terrain blending integrated.
- Dual-mode water integrated with required pass order.
- Comparative benchmark report (military/free/perspective).
