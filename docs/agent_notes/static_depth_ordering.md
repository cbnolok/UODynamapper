# Static volume and floor depth

> **2026-11-16: superseded by ClassicUO's draw order** (pending the user's review of the sheets). The priority z is
> now ClassicUO's (z, - 1 Background, + 1 for a non-zero height, + 1 MultiMovable), the nearer row always wins, and
> land, flat quads, sprites, shadows, picking and the light scene share one logical distance in both cameras. The
> effective priority height, the depth-class offsets, the tie-break ordinal and its perspective bias described below
> are gone, and so are the per-pixel plane depth and the painter nudge below: the land, the flat quads and the ground
> art take one depth per cell. See [the rule and what it removed](../dev_wiki/rendering/DEPTH_SCALE_AND_KR_CLIENT.md#dynamapper-classicuos-draw-order-2026-11-16).
> The sections below are the history of the cases that rule must keep right; their fixtures stay in the harness.

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

## Upright surface items under the land

The rest of the Surface statics stay `SurfaceLikeFloor`: tables, counters,
display cases, beds and benches (Surface with a height, `0x4240` for the table
0x0B73), and the flat ones drawn upright (mats, blood, floor cracks, grass
patches off a level cell). Their upright sprite took the class's
`SURFACE_LIKE_DEPTH_CLASS_OFFSET`, -4 world y (44 z), on its priority point, which
put that point 44 z under its own cell: behind the land and the Background
floors around it. They vanished wherever land or a floor lay less than 44 z
below them, in every style, art source and z-view rule: the Minoc log hall
(map 1, 2467-2492,397-409, its 70 tables at z 15 on wooden land at z 15), the
Britain shops' tables and display cases, of which only the parts reaching above
the floor's diamond were left. Of the ~59k Surface statics of that class on
maps 0-5 (tiledata flags), ~56k stand less than 44 z above their land, ~41k of
them with a height (339 ids).

`art/main.wesl` `depth_class_logical_offset` now gives that class no offset: its
bare-z priority already sorts it behind what stands on it (z plus height, or 10
for a flat Regular item), and in front of the Background floor (-0.001) and the
land under it. The KR client shadows take the same point, so the tables' shadows
come back with them. The CPU key `static_depth_key` keeps -4: it orders the
draws (flat items first) and finds equal keys, and never reaches the depth.
Flat quads and ground statics write their plane depth and are unchanged.

### A tall wall over the table in front of it (2026-11-15, closed 2026-11-16)

In the Minoc hall the back log wall (0x92, z 15, height 20, flags `0x6050`) on
y 396 covers the back corner of each table (0x0B72/0x0B73, z 15, height 6) on
y 397 in a stair-step. The KR reference (`uokr/Schermata_20261114_072859`)
and ClassicUO draw the tables in front.

- **Cause: the priority height, not the depth scale.** Ours: wall 15 + 20 = 35,
  table bare z 15, one diagonal step nearer: 15 + 11 = 26 < 35, so the wall
  wins. Both clients add at most 1 for height (KR: `z + 128 + (height > 0)`;
  ClassicUO `PriorityZ`: z, +1 if Height != 0, -1 if Background): 16 against
  16, and the nearer table wins on any scale. `effective_priority_height`
  (the full tiledata height, 10 for a flat Regular item) is from be44e69c and
  is not traced from either client. A wall's 20 counts nearly two diagonal
  steps, so it beats anything at its own z one step in front with a priority
  height under 9.
- **Candidate: cap the height below one step**, `min(height,
  DEFAULT_PRIORITY_HEIGHT)`. A shader-only probe (`min(priority_z_units,
  base z + 10)` on the priority point) fixes the hall in Classic and KR-like;
  it also hides the lower floor's wall tops that poked through flat roofs and
  castle pavements in Britain (1496,1628). Sheets in
  `uodynamapper-scratch/surface-items/sheets/issue2-*`.
- **It is a depth-model change.** On map 1's MUL statics (8.29M pairs of
  statics one or two diagonal steps apart), the cap changes the winner of
  254k pairs: 153k move to the KR answer (a tall farther item now loses to the
  nearer one: trunks against grass, walls against tables and floors), 101k move
  away from it (a raised farther item now beats a tall nearer one: an upper
  storey's wall or eaves against the lower storey's wall one step in front,
  dz 20 > 11; the full height hid the 11 against 35.4 scale gap there). Pairs
  disagreeing with the KR rule, `py/priority_compare.py`:

  | Height term | Disagree | At the same z |
  |---|---|---|
  | full height (today) | 787k | 239k |
  | capped at 10 | 735k | 125k |
  | full height x 0.6 | 809k | 227k |
  | client, +1 if height | 785k | 132k |

  The rest at the same z is mostly foliage's +22 z offset, by design. KR
  client shadows sort at their caster's point, so a capped wall's shadow also
  reaches less far forward. Not implemented; superseded on 2026-11-16 by
  ClassicUO's rule (the client's + 1, rows dominating), which draws the tables
  in front; see
  [DEPTH_SCALE_AND_KR_CLIENT.md](../dev_wiki/rendering/DEPTH_SCALE_AND_KR_CLIENT.md#decision-2026-11-16-pending-the-users-review-of-the-sheets).

### Grass edges over the road in Classic (2026-11-15)

Since the floor class lost its -4 offset, the grass edges 0x1782..0x1785
(Surface, height 0) and 0x0CBB draw their fringe over the cobbles in Classic
(map 1, 2512,555). This is the original order. The edges share their cell with
the cobbles (0x0515..0x0518, Background|Surface): ClassicUO gives the cobbles
z - 1 and the grass z, so the grass draws after them on its cell, and the
cobbles of the nearer cells after the grass. Ours matches (Background -0.001,
the next cell one step nearer). Before, the -44 z offset hid the edges under
the cobbles of their own cell. Only Classic shows it: the grass continues the
land, so under EC or KR land (the other styles) it draws as an Enhanced
land-style quad, without the CC art's fringe
([water_terrain_static_resolution.md](water_terrain_static_resolution.md)).

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

## Hover geometry follow-up

The independent cursor reconstruction described in the earlier audit used a
stale CC anchor, misplaced EC ground and upright geometry for flat art. The
[hover geometry follow-up](static_hover_geometry.md) shares the renderer's
routing, anchor/bounds and flat-surface rules and tests projected mesh triangles.
Logical object XYZ remains separate. Alpha-accurate selection, general overlap
parity and expanded-multi/large-art coverage remain separate work.

## History-sensitive tree edges

The [effect audit](effect_visibility_validation.md) reproduces a small KR/EC-art
town difference at map 0 `(1647,1682)`, orthographic 1280x800, zoom 0.8. A direct
launch/teleport and a `(4050,500)` snow → `(1872,2416)` swamp → town sequence
differ at 234 tree-edge pixels above one byte (maximum 36), within screen
`x=1105..1238`, `y=527..784`. Normal maps, grunge, wash, fog, water animation,
projected shadows and static light collection are disabled for this control.
The unchanged old binary and normal-role repair produce identical frames when
their histories match. Restoring all old normal bindings in isolated data does
not change the direct repaired frame. These controls separate the issue from
the normal-layer fix.

The tree-edge cause is the stock transparent phase's equal-distance ties.
`build_art_batch_mesh` contains only repeated unit-square corner positions;
the art vertex shaders reconstruct world positions from instance buffers.
Every batch entity has an identity transform. Bevy 0.20's material queue uses
the transformed mesh AABB center for `TransparentSortingInfo3d::Sorted`, so
these batches share the same placeholder center `(0.5,0,0.5)`. Its stable
distance-only phase sort retains insertion order, which changes as chunks
leave/re-enter the scene. That is insufficient for reproducible alpha edges.

`art/transparent_order.rs` extracts a small order component on transparent
sprite/ground batches and runs after Bevy's transparent sort. Camera distance
and material depth bias remain primary. Equal-distance static ties use ground
before upright art, then the collector's stable map/chunk `(gy,gx,scale)` key.
Other transparent entries retain their relative order. Retained phases skip
the additional sort when already canonical. This changes no instance order
inside a chunk, shader depth, geometry, texture data, render pass, logical XYZ,
tile height or movement metadata. Opaque batches and projected-shadow materials
retain their existing paths.

Native controls cover direct town/bridge and snow→swamp→town→bridge histories
in all three styles, plus KR perspective and an enabled-shadow control. Town
pairs are now byte-identical, including the original KR 234-pixel defect.
Classic/Enhanced orthographic bridge pairs and the KR perspective bridge pair
are byte-identical. Shadow strength 0.22 still changes 30785 town pixels above
one byte (maximum 34), and 20795 bridge pixels (maximum 29). The normal build
and 92 focused static-art tests pass; two adapter-dependent tests remain ignored.

A separate 50-pixel KR orthographic bridge-edge history difference (maximum
22, bounds `x=117..448`, `y=45..377`) survived that transparent-phase repair.
The unchanged pre-fix binary reproduced it, and old/new bridge frames were
byte-identical for each matched history. The subsequent opaque-tie investigation
below establishes its independent cause. General pixel-alpha and original-client
depth parity are not claimed by the transparent-phase fix.

The follow-up acceptance set is 32 complete native frames from eleven owned
Desktop-4 previews, indexed by `transparent-order-accepted-evidence.json` in
`/tmp/uodynamapper-ec-followup`. The earlier effect-audit frames remain the
before controls; metadata submission alone is not proof of stable final art.

## Equal-priority sprites across chunk boundaries

The retained bridge case comes from an exact opaque depth tie. Raw planks
`(2820,991)` and `(2819,992)`, graphic 1993, occupy adjacent 32-cell chunks.
Both are Background, with priority Z 21 and equal anchored X+Y. The old
per-chunk tie assignment gave both ordinal zero. Opaque batch submission order
then selected different winners after different teleport histories; stable
transparent-phase ordering cannot repair the opaque winner underneath.

A temporary native source-cell/pass shader identifies different neighboring
owners at exactly the original 50 pixels. Raw snapshots contain the same 11267
sprite instances in the same order, with identical geometry, priorities, UVs,
ordinals, flags and colors. Only 2092 residency layer indices differ. Selecting
the layer directly from the instance instead of its interpolated varying leaves
the same defect, rejecting that alternative explanation. The diagnostic shader
and instance-dump hook are absent from production code.

`assign_sprite_depth_tie_breakers` now runs after assembling all visible chunk
batches. A per-base-key counter continues ties across noncontiguous groups in
the stable `(gy,gx,scale)` chunk stream. Cached chunk geometry and per-chunk
instance sorting remain intact; retained complete frames reuse the assembled
output. Reassembly starts counters at zero instead of accumulating old ordinals.
This preserves the existing shader bias, instance layout, batch ranges, ground
raster depth and lossless logical XYZ/height/flags. It is a cross-chunk tie
repair, not a replacement of the overall static depth policy.

The regression reproduces the separated plank groups and checks repeat
assignment. The focused static-art suite passes 93 tests, with two
adapter-dependent tests ignored. The normal build passes without diagnostic
hooks. Twenty-eight complete native frames from eight verified Desktop-4 runs
cover all three orthographic styles and KR perspective. Town and bridge history
pairs are byte-identical throughout. Towns, CC-art bridges and the perspective
bridge also match their previous accepted frames exactly. The corrected KR
orthographic bridge differs at 57 pixels from the old direct frame and seven
from the old history frame; the original 50-pixel history difference is gone.
Classic/Enhanced lava controls remain exact. The road/roof controls change at
113/110 roof-edge pixels above one byte after the shared tie repair; visual
review retains the roof silhouettes, ground transitions and bridge stair/deck
occlusion. This is not a claim of complete original-client depth parity.
Native acceptance and source/package evidence are retained in
`bridge-cross-chunk-accepted-evidence.json` under
`/tmp/uodynamapper-ec-followup`.


## Coplanar flat quads across chunk batches

Flat statics (Classic flat art, `flat_quad` 1; Enhanced item surfaces, 2; the
`art_ground` quads) write the depth of their horizontal plane solved at the
pixel centre (`horizontal_plane_frag_depth`), bit-identical for every quad of
one plane. The 44-pixel Classic diamonds overlap their neighbours by a pixel
along each shared edge, so those pixels tie exactly. Inside one chunk batch the
instances run in `static_depth_key` order (x + y ascending) and the later one
wins the `GreaterEqual` test: the cell nearer the viewer, as in the clients'
painter order (cells in map order, larger x + y later). Across a chunk boundary
the two cells sit in different batches, and the winner followed the batches'
submission order. The reported case is the one-pixel diagonal between the CC
cobblestones 1301-1304 (z 0) on the rows y = 575 and 576 at Minoc, which
changed with unrelated spawn-order changes.

`horizontal_plane_painter_depth` (art `depth.wesl`) now steps a fragment in the
near half of its cell, toward +x + z from the cell centre, one unit in the last
place farther (reverse Z: lower). On a shared edge the far cell's fragment is
in that half and the near cell's is not, so the near cell wins in either draw
order; inside a batch that is the winner it already had. Two quads of one cell
step alike at a pixel and keep their draw order. One unit in the last place
(about 6e-8 near depth 0.5) stays far below the art's 0.002 world-Y lift over
the terrain (about 6e-7). Perspective keeps the rasterised depth unchanged, and
the pick pass keeps its own depth.

## Visible-object query

Inspection/highlighting now use [the GPU material/depth query](static_hover_geometry.md#current-visibility-query).
The former CPU `static_depth_key` winner could disagree with projected priority
and per-fragment ground depth. In particular, the bridge's screen `(530,410)`
selects visible water 6043 at Z=-5 instead of an occluded plank, and `(530,490)`
selects stair 1874 at Z=10 instead of the overlapping Z=15 piece. The drawing
depth policy itself is unchanged. Exact coincident opaque ties remain a stated
query-ordering limit; this is not a global original-client depth replacement.
