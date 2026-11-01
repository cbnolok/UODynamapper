# Terrain cursor and click teleport — 2026-11-01

The terrain cursor now intersects the admitted, displaced terrain triangles.
Previously it intersected Y=0 and rounded the coordinates; on slopes the
inspector read another cell, and click teleporting placed the player at Z=0.
Cell ownership now uses the terrain shader's floor rule. Teleporting uses the
selected cell's original signed Z through the shared validated teleport helper.

The query follows each assigned mesh's actual detail/spacing and the EC bank
vertex adjustments. It handles the calibrated height scale, nearest terrain
occlusion, front-face admission, camera clipping, viewport offsets and physical
pixel centers. It caches unchanged inputs and adds no GPU pass. Logical map
coordinates stay separate from displayed surface positions. See the durable
[terrain query contract](../agent_notes/terrain_cursor_surface.md).

## Validation

Eight focused tests cover the mesh diagonal, oblique/front/back-facing rays,
nearest hills, missing/unrendered geometry, coarse meshes, signed-Z extremes,
large facet coordinates, bank easing, all eight generated mesh dimensions,
both projections, viewport offsets and HiDPI. The existing Vulkan sampler test
also compares six CPU bank positions with the production shader. Normal builds
are clean. The shared teleport regression and all five static GPU-query
owner/lifecycle regressions also pass.

Native validation evidence is under
`/tmp/uodynamapper-ec-followup/phase3-terrain-cursor`. Previews use exact-PID
Desktop 4 placement, isolated effective settings and the accepted installed
packages. All six style/projection previews pass: 72 terrain hits and 18 native frames
across lava, road and bridge fixtures. The orthographic hits also pass an
independent 44-pixel isometric screen-projection check at the recorded zoom.
A final smoke clicks the lava slope at screen `(800,230)` and reaches the
original cell `(5747,2925,40)`. Its displayed surface is about 3.49 world units;
that interpolated/bank-adjusted value is not substituted for raw Z. Outside-map
clicks leave `(0,0,0,0)` unchanged, and zoom 4.28 exercises a coarser assigned
mesh. All preview placement/cleanup reports and OS focus/pointer checks pass.

The matched old binary selects `(5744,2922)` at that lava pixel. The new query
selects the rock under the pointer, `(5747,2925)` with original Z=40. On flat
roads, floor-based ownership also corrects the former nearest-vertex rounding.
Three matched world frames differ by at most one channel byte outside the
recorded conservative UI masks; the source shaders are unchanged. The
`accepted-evidence.json` manifest retains hashes, probes and test logs.

## Scope

This queries terrain beneath static objects. Static inspection/highlighting
retains the material/depth GPU picker; a missing terrain hit no longer suppresses
that independent object query. No map, object, source texture, shader appearance
or package format is changed. Bank relief never becomes authoritative map Z.

This does not add walking, collision, bridge/floor surfaces, mobile rendering or
persistent gameplay selection. Rare coincident opaque static ties, individual
art geometry, animated static frames and water/effect material fidelity remain
separate work. Animated-MSAA water artifacts reproduced in the old binary remain
open. The asset agent's new variant-route census and water-shader findings need
separate source/native acceptance. Performance work remains deferred.
