# Visible static picking — 2026-11-01

Static inspection and highlighting now follow the material fragments and depth
used by the renderer. The old CPU picker tested ordinary sprite alpha but
accepted entire authored material quads, then ranked them with an approximate
logical key. Transparent mask regions and terrain-hidden pieces could therefore
win over what was actually drawn.

## Implementation

An on-demand one-pixel query reuses the current art/ground shaders and the
existing transparent batch meshes, instance buffers and material bindings. A
shader variant returns a temporary instance index immediately after the same
material composition, coverage fades, opacity and pass-admission checks used
for drawing. There is no separate CPU material implementation.

The query uses two local passes: opaque fragments write the local pixel depth;
transparent fragments retain the actual transparent phase order without writing
it. Both compare against the completed main opaque depth, sample by sample.
The main camera depth texture enables texture sampling; the query never writes
main color or depth. Shadows and other untagged transparent effects are excluded.
MSAA resolves discrete object IDs before color averaging, choosing the most
covered nonzero ID with deterministic first-sample ties. Normal rendering keeps
its original four-bind-group limit: art does not use the stock mesh bind group,
so the query reuses that slot for its coordinates and depth texture.

CPU snapshots map the sorted sprite/ground instance indices back to the
collector's logical tiles. Cached but unrendered chunks are excluded. Signed raw
Z, graphic, hue and expanded-part positions survive asynchronous replies;
indices are transient rendering IDs, not persistent game identities. View,
pointer, source/content or alpha-policy changes invalidate old replies. Animated
materials may use a recent reply from the same view while the next frame is in
flight. GPU readback is asynchronous and callback polling never waits for GPU
completion. Static views reuse their completed query. No query rendering runs
without an inspection/highlight request.

A native test exposed a prerequisite change-detection bug: the source selector
cleared an already-empty warning every frame. That marked the resource changed
and discarded every asynchronous reply. It now clears an existing warning only.
Query cache keys use actual alpha controls/source choice and terrain content
revision; unrelated lighting or cache bookkeeping does not invalidate a hit.

The former CPU alpha cache and projected-quad priority picker were removed.
CPU quad reconstruction remains for the selected object's highlight and source
label, sharing the collector's anchors, source scales and geometry gates.

## Validation

The focused art suite passes 100 checks, and five query/lifecycle checks cover
stale pointers, animation replies, source replacement, valid empty hits and GPU
readback failures. A Vulkan test runs 12 production-helper cases for opaque-depth
rejection, exact ID bytes and single/four-sample resolution. Three Python control
checks pass. Normal builds have no renderer warnings.

Seven verified Desktop-4 previews cover all three styles in both projections,
plus KR with animated water and four-sample MSAA: 112 ready picks and 28 native
frames. The Classic fallback crane beam remains graphic 18910, and its empty
window passes to pier 942 at raw Z=-5. A final accepted-build smoke passes eight exact expected picks across five
fixtures, including the original logical book/chair source controls: book 39297
at (6403,2599,5) selects Classic slot 55681, while chair 2901 retains hue 1157
and its KR artwork. Two additional probes with the inspector hidden confirm
that highlighting alone requests and receives the same visibility query; native
frames show the crane beam and the pier through its empty window.

A matched 120-point KR comparison changes ten former hits:

| Scene/pixel | Before | Visible result |
| --- | --- | --- |
| Road `(2518,561)`, `(650,330)`, `(690,370)`, `(730,410)` | Grass overlay 6018 | No static in the fully transparent region |
| Lava `(5744,2928)`, five recorded points | Rock pieces 12939/12945/12947/12948 at Z=30 | No static where terrain hides them |
| Bridge `(2834,987)`, `(530,410)` | Plank 1993 at Z=20 | Visible water 6043 at Z=-5 |
| Same bridge, `(530,490)` | Stair 1874 at Z=15 | Visible stair 1874 at Z=10 |

The complete point list is `pick-differences.json` in the temporary evidence
folder. Outside the inspector/message rectangles, all four matched world frames
differ by at most one channel byte after shader recompilation. No geometry,
texture source, alpha policy or intentional color change was introduced.

Evidence is under `/tmp/uodynamapper-ec-followup/phase2-picking`: before/after
frames and replies, matrix summaries, pinned binaries/shaders, focused test logs,
and `accepted-evidence.json` with final targeted smoke results. Each preview verifies exact-PID Desktop 4
placement/cleanup and unchanged OS focus/pointer. Package files are the accepted
phase-1 set. Controlled before/after runs retain the same loose terrain routes;
the asset agent's later variant-route changes are a separate visual acceptance
item.

## Limits and next work

This selects visible statics for inspection/highlighting. It does not implement
walking, collision, persistent gameplay selection or a terrain-surface query.
The subsequent [terrain cursor acceptance](2026-11-01-terrain-cursor.md) replaces
the separate Y=0 cursor/click-teleport query with admitted terrain triangles
and preserves the selected cell's original signed Z.
Logical world data must remain authoritative for gameplay.

Queries use the render pixel center, before fullscreen fog, blur, FXAA/SMAA and
UI. Fractional coverage above the drawing discard threshold remains selectable;
there is no invented low-opacity click-through threshold. Replies have normal
asynchronous readback latency. Exactly coincident opaque surfaces with identical
numeric depth remain a rare ordering limit because the opaque query uses the
existing carrier batches rather than replaying the opaque bin submission order.
Arbitrary cameras, other facets and large multis need broader coverage.

The animated MSAA bridge capture also reveals thin gold lines and translucent
triangular artifacts over water. A new bounded run of the pre-change binary with
its original shaders reproduces them (`baseline-msaa-animation-bridge.png`);
this phase does not repair that existing visual defect.

Original-client depth parity, individual sprite/window geometry, static
animation, effects and reflection/filtering fidelity remain later work in
[the roadmap](../TODO.md). This GPU implementation
is a modern visibility query, not a recovered UOSA rendering algorithm.
