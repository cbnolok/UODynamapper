# Static volume and floor depth

`statics_collect::resolve_static_depth_class` controls the sprite's logical
depth, effective height priority, sorting and shading class. Visible-object
picking uses the same classifier and priority helper.

## Walkable volume is not a flat floor

The broad `static_tile_is_surface_like` predicate includes the tiledata Surface
flag (`0x200`), as well as authored surface-like hints. That is useful for
material routing, but cannot by itself establish flat-floor depth semantics.
The Bridge flag (`0x400`) marks walkable volume such as stairs and stone blocks;
the existing `effective_priority_height` halves their tiledata height.

Previously those pieces became `SurfaceLikeFloor`, which both discarded the
height priority and applied the floor class's `-4.0` logical world-Y offset.
For a stair at raw Z=15, that projected Y=-2.5 instead of its raised height.
Terrain and water meanwhile used their actual rasterized surface depth. The
result was missing stair ends and a support pillar clipped by water in map 0
at (2834,987).

Representative installed metadata:

| Graphic | Name | Flags | Height | Priority at Z=15 |
|---|---|---|---|---|
| 1872 | stone | `0x04002600` | 10 | 20 |
| 1874 | stone stairs | `0x80002600` | 5 | 17 |
| 1894/1895 | stone | `0x00002600` | 5 | 17 |

The classifier now leaves Bridge-bearing volume in `Regular`, retaining
`base Z + height/2` and avoiding the flat-floor offset. This is a flag-based
rule, not a graphic list or a bridge-coordinate exception. Background, roof and
foliage precedence remains earlier in the classifier. Background wooden planks
and actual flat floor/material quads retain their existing paths.

## Depth contracts retained

Upright sprites still use constant per-instance painter priority, including the
existing tie-breaker. Terrain, horizontal ground materials and authored EC
camera-plane surfaces retain rasterized geometric depth. No global depth
replacement, new bias, atlas, binding, instance field or draw pass was added.
The correction applies to EC and CC static sources through their shared metadata
classifier. It also restores the corresponding CPU picking priority.

## Validation

The regression uses the installed stone/stair flag and height combinations,
checking both volume classification and priority. Native Desktop-4 captures
show the complete stair run over the bank and the restored support pillar in
Classic, Enhanced and KR, with lava/crust, coast, acid, swamp, town and CC
controls and a separate CC bridge capture. The renderer suite passes 116 tests,
including its explicitly executed 72-case GPU regression. The original bridge reference is
`docs/images/ec_reference/20261026_181634.png`; this comparison validates the
occlusion correction, not complete lighting, framing or pixel parity.

## Perspective tie bias

The later perspective-only stair blocks over the wooden deck at `(2834,987)`
were caused by the equal-priority tie bias, not the sprite window geometry.
`assign_sprite_depth_tie_breakers` assigns an ordinal within each equal-key
group. The old shader subtracted `ordinal * 0.000001` directly from NDC depth.
An orthographic depth offset is linear in world distance; perspective depth is
approximately `near / distance`. The same NDC bias therefore grows quadratically
in world distance as the camera retreats. At distance 90 and near 0.1, ordinal
16 moves behind a point half a world unit farther away. It can change ordering
between different cells, rather than only resolving the original tie.

An isolated native control setting only that bias to zero removes the blocks.
`art/depth.wesl::static_priority_depth` retains the existing orthographic
calculation exactly. Perspective instead projects a 0.002-view-unit displacement
away from the camera per ordinal. This matches the old startup orthographic
epsilon's world distance (2000-unit depth range), without making the perspective
bias depend on camera distance. The displacement applies only to the synthetic
priority point in the vertex shader. Sprite vertices, alpha, raw XYZ, tile heights,
movement flags, flat surfaces and projected-shadow depth retain their own paths.
The CPU sort key and classification are unchanged. This is a correction to the
modern renderer's tie handling, not a recovered UOSA depth formula.

The explicit production-shader GPU test covers 65 cases across both projections,
six perspective distances from 1 to 5000, an additional orthographic point beyond
the clipping range, and ordinals 0, 1, 4, 16 and 32. It checks separation,
projection of an independently displaced view-space point and preservation of
neighbor order; a representative assertion demonstrates failure of the old bias.
The application suite passes 142 tests, with five adapter-dependent tests ignored.
Native Desktop-4 controls cover all three styles and appropriate object/land
sources: perspective bridge, road/roof, town and lava; orthographic bridge and
road/roof; and closer/farther Enhanced perspective bridge views. Complete images,
runtime settings, settled metadata, app logs and preview cleanup are recorded in
`/tmp/uodynamapper-ec-followup/depth-accepted-evidence.json`.
Fresh original/corrected Classic orthographic bridge and roof frames are identical.
A 114-pixel roof-edge difference against an older capture also occurs with the
unchanged original shader; it is not evidence of a regression from this change.
Both orthographic clamps are retained, including their out-of-range behavior.

The demonstrated deck overlap is repaired. Individual EC source scales/windows,
arbitrary-angle baked-art fidelity and general original-client depth parity remain
separate questions. The original bridge reference remains the orthographic image
above; there is no supplied original-client perspective reference.
