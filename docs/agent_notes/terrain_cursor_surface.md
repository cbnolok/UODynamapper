# Terrain cursor surface query

`world/land/picking.rs` supplies `TerrainCursor` to the position label, cell
inspector and click-teleport helper. The old path intersected Y=0, rounded X/Y
and either fetched the wrong cell's Z or teleported at zero height. This is a
terrain-only query; visible static ownership remains the separate GPU query.

## Geometry and identity

The query uses only land chunk entities with an assigned mesh and current view
visibility on the player's facet. `LandMeshHandles::grid` identifies the actual
assigned handle, rather than predicting it from the current zoom. This matters
because LOD swaps are throttled and wide meshes have different sample spacing.
The dimension table is tested against all eight meshes from `setup_land_mesh`.

Triangles use the mesh's NW-SE diagonal. Positions read the CPU metadata atlas,
the same calibrated signed-Z scale as WESL, and the bank vertex adjustments from
`land/liquid_bank.wesl`. The CPU counterpart includes missing/Classic exclusions,
bounded solid-height easing and the view-facing quarter-cell bevel. Six cases
in the existing Vulkan liquid-sampling test compare CPU positions directly with
the production bank shader. Changes to that shader must keep this parity check.

The ray uses the physical pixel center, matching static picking, and respects
inset viewports and scale factors. Front-face triangle intersection chooses the
nearest surface, including terrain that hides more distant terrain. Ray/chunk
height-band boxes restrict the search; vertices are reused within a query.
Double-precision local ray differences avoid facet-sized origin cancellation.
The hit also obeys camera clipping. There is no new rendering pass or readback.

`TerrainCursorHit` keeps two distinct values:

- `world_position`: the continuous displayed surface point, including bank
  presentation adjustments and the assigned mesh's interpolation.
- `logical`: owning map cell X/Y, its **original signed map Z**, and facet.
  The source graphic is retained alongside it. Ownership uses `floor`, matching
  the terrain fragment shader, rather than rounding to the nearest vertex.

Click teleporting uses `logical` through the existing validated
`controls::teleport::teleport_player` operation. It never derives logical Z from
visual relief or the interpolated point. Outside-map, back-facing and unloaded
terrain do not fabricate an edge-clamped or zero-height destination.

## Streaming and invalidation

Do not query CPU metadata that has not reached the renderer. The query requires
no pending atlas writes or upload operations, and equality between content and
submitted metadata revisions. Missing vertex metadata rejects that triangle.
A pointer, view/projection/viewport, map, content, EC package, upload-readiness or
admitted mesh-grid change invalidates the cached hit. Static frames reuse it.

`status.terrain_cursor` exposes the logical input pointer and optional hit with
`logical`, `graphic` and `world_position`. A null hit can mean no rendered terrain
or that streaming has not settled; `status.terrain` distinguishes readiness.
Static inspection still works when there is no terrain hit underneath it.
The query reads Bevy's computed camera/global transform in `Update`; during
camera motion the diagnostic has the normal computed-view update latency.
It is not a synchronous collision feed.

This is an editing/inspection query, not a walkable-surface or collision system.
It queries terrain underneath objects; it does not turn a bridge sprite into
terrain or a walkable plane. Texture-only normal mapping does not move the mesh.
MSAA edge samples can straddle two surfaces; terrain uses the pixel center,
while the static GPU picker resolves sample coverage. Broader free-camera,
facet and world editing cases need their own acceptance fixtures.
