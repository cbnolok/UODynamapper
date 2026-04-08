# Phase 3: Advanced Rendering Modes and Unified Runtime Switches

Phase 3 introduces camera and visual-mode expansion while preserving the fast path.

Hard targets:
- Military orthographic remains the primary high-FPS mode (400+ FPS class benchmark).
- Perspective/free modes never drop below 144 FPS in the defined stress profile.
- Runtime mode switches are deterministic and do not require world reload.

Status vocabulary used in this document:
- Current implementation: already present in the renderer today.
- Accepted target architecture: chosen replacement or expansion path.
- Optional / benchmark-first: only worth doing if profiling shows a real gain.
- Deferred / unclear: postponed until prerequisites or product needs are clearer.

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

Status: Optional / benchmark-first.

## 2.1 Why Variants (Not One Giant Branchy Shader)

Global uniform branching is valid, but heavy optional code (for example POM loops) can increase register pressure in fast modes.

Policy:
- Use shader defs / pipeline specialization for major mode groups.
- Keep military variant lean and free of expensive perspective-only logic.
- Accept one-time pipeline warmup stutter at first switch.

Current renderer is still allowed to ship on a unified shader path if military-mode performance stays within target. Specialization is a performance lever, not a prerequisite for Phase 3 terrain work.

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

Status: Accepted target architecture.

Decision-history note:
- The current threshold-based mesh/scale path is implemented today and should be treated as transitional, not final.
- GPU clipmap terrain remains the chosen replacement because it addresses zoom-threshold hitching, live-edit compatibility, arbitrary-resolution rendering, and deterministic export goals established in the earlier design work.

Comparison:

| Current threshold path | Observed weakness | Accepted replacement | Why chosen |
|---|---|---|---|
| Discrete `scale_from_zoom()` and mesh swaps at zoom thresholds | Pop/hitch risk, chunk respawn pressure, zoom bands baked into render topology | GPU clipmap rings with continuous sampling/blend logic | Smoother zoom, better edit behavior, cleaner long-term terrain architecture |

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

- Remove threshold-triggered mesh swap/rebuild as the end-state terrain path.
- Continuous zoom transitions through clipmap level blend logic.
- Avoid hard pop at LOD boundaries.

Current implementation note:
- Threshold-triggered terrain scale changes and mesh swaps exist today.
- Keep them operational until the clipmap path is ready, but do not promote them to the long-term design.

## 4. Static Art Behavior in Free/Perspective Modes

Status: Deferred until the static-art rendering pipeline is in place.

## 4.1 Category Policy

Categories:
- Billboard category: foliage, effects, entities intended to face camera.
- Structural category: walls/roofs/features that must keep world orientation.

## 4.2 Billboard Math Policy

- Use cylindrical billboard by default for world-grounded sprites.
- Keep origin/footpoint stable on ground plane.
- Avoid per-entity CPU transform updates in hot path.

## 4.3 Structural Advanced Path

Status: Optional / benchmark-first.

Optional heavy path:
- Per-fragment volumetric illusion (for example POM-like behavior).
- Enabled only for flagged structural materials in perspective/free mode.

Do not enable globally; cost amplification is unacceptable.

## 5. Terrain Transition Modes

Status: Mode A is the current fidelity path; Mode B is accepted target architecture; stochastic refinements are optional / benchmark-first.

Runtime switch (no reload):
- Mode A: classic transition tiles only.
- Mode B: automatic stochastic blend for maps without transition assets.

Automatic blend requirements:
- Neighbor-aware blend factor.
- Optional noise modulation to avoid straight synthetic seams.
- No mutation of source logical tile IDs.

Decision-history note:
- Classic transition tiles remain the baseline for original-content fidelity.
- Automatic blending is a first-class roadmap item for custom maps that do not ship with transition assets.
- Noise/stochastic seam refinement should not block the base auto-blend mode.

## 6. Water Modes and Pass Order

Status: Classic water remains baseline; explicit water pass ordering is accepted target architecture; enhanced-water extras are optional / benchmark-first.

## 6.1 Water Modes

- Classic: original animated tile sequence path.
- Enhanced: procedural distortion/depth tint path.

## 6.2 Mandatory Pass Ordering

For submerged-asset visibility correctness:
1. Opaque terrain pass with water fragments discarded.
2. Static/mobile pass.
3. Transparent water pass.

This order is required to see submerged art through water while preserving depth semantics. It has priority over cosmetic enhanced-water work.

## 6.3 Enhanced Water Inputs

Optional enhanced path may use:
- Time uniform.
- Normal/noise map(s).
- Scene depth input for depth-based tint/opacity.

Depth-based tint must be bounded to avoid over-darkening in shallow areas.

## 7. Runtime Switching and State Management

Status: Accepted target architecture, with partial support already present.

Switchable at runtime:
- camera mode
- terrain blend mode
- water mode
- structural advanced path toggle

Switch behavior requirements:
- No world reload.
- No full atlas rebuild.
- Bounded warmup hitch only on first pipeline compile.

Implementation note:
- Camera-only switching exists today.
- Terrain blend mode, water mode, and structural-path toggles should follow the same uniform/material update model rather than triggering world rebuilds.

## 8. Performance Governance

Status: Required baseline metrics are mandatory; deeper specialization telemetry is optional.

Mandatory telemetry:
- frame time breakdown by pass
- variant usage counts
- expensive-path pixel coverage estimate
- upload queue pressure (from Phase 2)

Acceptance criteria:
- Military mode: no regression vs Phase 1 benchmark.
- Perspective stress scene: >= 144 FPS baseline profile.
- No runaway spikes during frequent mode toggles after warm caches.

Priority split:
- Must have: military-mode regression check, runtime-switch hitch validation, upload-queue pressure visibility.
- Nice to have: variant usage counts and expensive-path pixel coverage once specialization-heavy paths actually exist.

## 9. Deterministic Export Compatibility

Status: Accepted target architecture for future export work.

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
