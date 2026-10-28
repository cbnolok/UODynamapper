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
