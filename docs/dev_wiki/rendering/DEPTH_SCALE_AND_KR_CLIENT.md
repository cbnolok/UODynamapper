# Depth Scale: Dynamapper and the KR Client

How Dynamapper decides which static, shadow or land surface lies in front, how the Kingdom Reborn client decides it, where the two disagree, and why Dynamapper keeps its own scale (decision of 2026-11-12).

In short:

- Both renderers sort an upright sprite by its cell's diagonal x + y and by its priority z.
- They weigh the two differently. In Dynamapper one diagonal step is worth about **11** priority z units, which is what the camera geometry gives. In the KR client it is worth **35.4**, because the client adds a fixed bias of its own.
- KR client shadows sort at their caster's depth in both renderers, so a shadow follows the same scale as the sprites around it.

Related: [KR static shadows](CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md#kr-static-shadows), the client trace in [`uokr_atmosphere.md` P3.4](../../../scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md#p34-sprite-shadow-geometry), and [static depth ordering](../../agent_notes/static_depth_ordering.md) for the depth classes and the perspective tie bias.

## Dynamapper today

### World and camera

- The world axes are: x = UO x, z = UO y, and y is up.
- A UO z unit is `UO_Z_TO_WORLD_SCALE` = 4/44 world units (`util_lib/uo_coords.rs`, mirrored in `world/uo_coords.wesl` and used in the art shaders as `HEIGHT_SCALE`). That is four classic pixels in the 44-pixel grid.
- The orthographic camera sits at `PlayerCamera::BASE_OFFSET_FROM_PLAYER` = (5, 5, 5) from the player and looks at the player (`core/render/scene/camera.rs`). The view direction is therefore (-1, -1, -1) / √3.
- The perspective camera orbits along the same `Vec3::ONE` direction (`perspective_offset`).
- The lens stretches view y on screen to make the 44 × 44 diamond. That changes x and y in clip space, not the view depth.

### Upright sprites: a projected priority point

An upright billboard does not write its rasterised depth. Every fragment of the sprite writes one constant depth: the depth of a synthetic priority point (`art/main.wesl`, `logical_depth_from_projected_priority`).

```text
P = (tile_x, priority_z_units * HEIGHT_SCALE + depth_class_logical_offset(depth_class), tile_y)
depth = static_priority_depth(position_world_to_clip(P), clip_from_view, sort_bias_ordinal)      art/depth.wesl
```

- **`tile_x` and `tile_y`** are the sprite's anchored cell position. CC art adds (1, 1) and EC art adds (0.5, 1.5) (`statics_collect.rs`), so both add 2 to x + y.
- **`priority_z_units`** is `resolve_priority_z_units`: the tile z plus `effective_priority_height`.
  - A height of 0 counts as 10 unless the tile is Background or Surface.
  - Bridge halves the height.
  - `SurfaceLikeFloor` takes the bare z.

  This is the classic draw priority, as in ClassicUO.
- **`depth_class_logical_offset`** is in world-y units:
  - Background: -0.001;
  - Roof: +0.002;
  - Foliage: +2.0 (22 z);
  - SurfaceLikeFloor: -4.0 (-44 z).
- **`static_priority_depth`** subtracts the tie-break ordinal. In orthographic it subtracts 1e-6 NDC per ordinal. In perspective it moves the point 0.002 view units per ordinal (see [static depth ordering](../../agent_notes/static_depth_ordering.md#perspective-tie-bias)).
- **The ordinal** comes from `assign_sprite_depth_tie_breakers`. It numbers sprites whose CPU key `static_depth_key_with_tie_break` is equal.
  - That key, (x + y) + 0.01 (127 + priority z) + class offset, only finds equal keys and orders picking.
  - Its 0.01 per z is not the GPU's scale.
- **KR client shadows** take their caster's priority point (`client_shadow_logical_depth`), half a tie-break step behind (`CLIENT_SHADOW_DEPTH_ORDINAL_OFFSET`). A wall's shadow moves with the wall shift (-1, 0, -1) (`client_shadow_logical_offset`).
- **The projected-shadow pass** (`PASS_MODE_SHADOW`, not KR's) writes its rasterised depth.

### Why one diagonal step is worth about 11 z

Depth runs along the view direction. A point p is nearer to the camera by (p · (1, 1, 1)) / √3, which increases with x + y + world y:

- one step in x or in UO y (world z) adds 1;
- one UO z unit adds 4/44 of world y.

So a step along the diagonal is worth 1 / (4/44) = **11 z units**. Raising a sprite by 11 priority units brings its point as near as moving it one cell toward the viewer. Nobody chose this ratio; it follows from the 4-pixel z unit, the 44-pixel cell and the (1, 1, 1) camera. This is the depth that the camera and the screen geometry actually give.

### Land and flat quads: real surface depth

- **Terrain** is a real 3D mesh with vertex heights z · 4/44. Its shader writes no `frag_depth`, so it keeps its rasterised depth, which follows the same geometric scale.
- **Flat quads**, meaning CC surface-like ground and water quads and the EC item surfaces (`flat_quad` 1 and 2), write the depth of their horizontal plane, solved per pixel (`horizontal_plane_frag_depth`). Coplanar quads then write bit-identical depth.
- **`horizontal_plane_painter_depth`** pushes the near half of each cell one ULP back, so the cell nearer the viewer wins the pixel it shares with its neighbour, as in the clients' painter order.
- **Authored EC camera quads** (`flat_quad` 3) keep their rasterised depth.
- **Perspective** keeps the rasterised depth throughout.

Sprites, land and flat quads therefore all live on one geometric scale. That is why a sprite and the slope it stands on agree.

## The KR client

The client is UOKR.exe and its engine is Gamebryo. The trace is in P3.4, under "Depth order", "Stencil" and "Main scene pass".

- **Camera:** `Rz(-π/2) Rx(0.001 - π/4) Rz(-0.05)`, orthographic, near 8 and far 12 (0x0045ae50). Its view direction is V = (-0.0354, 0.9988, 0.0353), the vector at `[0x00c9dc70] + 0xc`.
- **Sprite base point:** (x + 0.5, 0, -(y + 1.5)) + z Zu U, from `UOSprite::PlaceSprite` 0x004422b0. Here U is camera up, V · U = 0, and Zu = 0.12856454. The base point alone gives a depth of -0.0354 x - 0.0353 y, with no z term.
- **Priority bias:** the sprite component `UOObjectComponentSprite` 0x00435120 adds a bias at 0x0043518a..0x00435216 and stores it in the descriptor's +0x60..+0x68 at 0x00435377..0x0043537f. `UOSprite::BuildSpriteBatch` 0x004454c0 adds it to every corner:

  ```text
  b = -0.001 A - 1e-5 B          -0.001 at 0x00b4f310, 1e-5 at 0x00b86de8;  bias = b V
  A = vfunc +0xdc, 0x00561c00 = z + 0x80 + (height (+0x3c) > 0) + (flags & 0x8000 and not +0xd0) - Background (+0x38 & 1)
  B = vfunc +0xe4, 0x00561c70 = UOObject +0x70, an ordinal (unidentified)
  ```

  A is the classic priority (z, plus 1 when the item has height, minus 1 for Background), the same quantity as our `priority_z_units` up to the height term. Net:

  ```text
  depth ≈ -0.0354 (x + y) - 0.001 A        (smaller = nearer)
  ```

  So one diagonal step is worth 0.0354 / 0.001 = **35.4 priority units**. The bias is a fixed choice in the client, independent of the camera's geometry. B separates equal A by 1/100 of a unit.
- **Terrain:** 0x00448de0, called from `UOTerrain::BuildTerrainMesh` 0x0044b7f0, biases each vertex by V · (-0.001 (z' + 127) + 2e-5). Here z' is the lower of the vertex's corner and its south corner. A terrain cell therefore sits about one priority unit behind a sprite standing at the same z in that cell, on the same 35.4 scale.
- **Shadows:**
  - The sprite component writes the same bias it gave the caster into the shadow descriptor that the placement 0x00440e50 returns (call at 0x004353cd, stores at 0x004353e4..0x004353ea). The refresh path 0x00435630 recomputes it from the caster (stores at 0x00435760..0x0043576a).
  - The placement adds ε = 1e-4 along V, 0.1 priority units.
  - A wall's shift, client world (-1, 0, +1), adds V · (-1, 0, 1) = +0.0707, two diagonal steps back.
  - So a shadow sorts at its caster's priority depth, just behind it, not at the ground under it.
  - It goes into the chunk's batch slot 3 (`0x0043cf40` with EAX = 3). It carries no z-buffer bits (+0x38 = 0x41), so the default z test applies.
- **Main scene pass 0x00459e00** draws the render groups in this order:
  1. 0x00c931a0, terrain;
  2. 0x00c931a4;
  3. 0x00c931ac;
  4. 0x00c931a8;
  5. 0x00c931b8, mobiles;
  6. optionally 0x00c931bc (weather) and 0x00c931c4 (foliage);
  7. 0x00c931cc.

  When the pass's +0x88 is set, a pre-pass draws 0x00c931a4, 0x00c931ac and 0x00c931c8 (the circle of transparency) first.

  The shadow effect `UOShadowPostProcessEffect::Render` 0x0043fe10 runs after the scene, before the lighting effect. It draws its list into a screen-sized buffer S against the scene's depth.
- **Stencil:** 0x00440a50 gives shadows test ALWAYS, reference 1, pass REPLACE (0x00c933f0; ZERO for Roof or Bridge casters, 0x00c93410). Mobile shadows (0x00425180) get the same. Ordinary sprites get GREATER 0xff with INCREMENT (0x00c933e8 / 0x00c93414).
  - The stencil serves the circle of transparency (`UOStencilEffect`, 0x00416db0 and 0x00417480), which writes 0xff where it cuts through, so the covered sprites fail their GREATER test.
  - An ALWAYS test never rejects a shadow fragment, so the stencil has no say in which surfaces get a shadow.
  - Dynamapper has no circle of transparency and nothing to reproduce.

## Where the two disagree

### Raised sprites

Take two upright sprites: a raised sprite A, dz priority units above a sprite B, with B d diagonal steps nearer the viewer. A wins the pixels they share when:

| | A in front when |
|---|---|
| Dynamapper | dz > 11 d |
| KR client | dz > 35.4 d |

The two agree when d = 0, when dz < 11 d, and when dz > 35.4 d. They disagree when 11 d < dz < 35.4 d: Dynamapper puts the raised sprite in front, the client the nearer one.

| Case | dz | d | Result |
|---|---|---|---|
| Items on a table, small stacks, a crate on a crate | 0..10 | 0..1 | Same answer in both. |
| An upper-floor wall against the ground floor's wall one cell nearer | ≈ 20 | 1 | Dynamapper: the upper wall in front. KR: the lower wall in front. |
| A roof (+30..+40 over the floor walls) | 30..40 | 1..3 | Dynamapper: the roof in front up to dz/11 ≈ 3 steps. KR: at most 1 step. |
| Towers and battlements (+40 and more) | 40+ | 1..3 | As for roofs, with a longer reach in Dynamapper. |

Which sprite wins only shows where their art overlaps on screen. A raised sprite is drawn higher up the screen, so many of these pairs never share a pixel.

### Shadow reach

A KR client shadow sorts at its caster's depth. It therefore darkens a receiver dz priority units below the caster that lies up to:

- **dz / 11** diagonal steps in front of the caster in Dynamapper;
- **dz / 35.4** steps in the client.

So a high caster reaches a little further forward onto low receivers in Dynamapper. At the caster's own level (dz = 0) the two match.

## The no-z experiment (2026-11-12)

The idea was that the client's shadows sort by x + y alone. An earlier reading of the trace had missed the +0x60 bias.

The test variant removed `priority_z_units * HEIGHT_SCALE` from `client_shadow_logical_depth` and changed nothing else. It was compared against the unchanged shader:

- KR-like, CC and EC art, KR land, afternoon, zoom 1 and 2;
- isolated runs on Desktop 4, all logging no errors;
- views:
  - Britain houses 1496,1628 and door 1434,1699;
  - Minoc 2505,555;
  - Trinsic 1905,2835;
  - Lord British's castle 1330,1620;
  - the bridges at 1385,1745 and 1510,1705;
  - tables on stacked floors at 1411,1604;
  - stacked stairs and blocks at 1334,1741.

Result:

- With z dropped, every raised caster's shadow sank behind the floor it stands on.
- Wall and battlement shadows vanished from the castle's wall-walks, the upper floors and terraces, and the raised wooden floor at 1334,1741.
- At ground level (Minoc) only slivers changed.

That is worse, and the trace shows it is not the client's rule. The shader was left unchanged.

The sheets show the current shader, the no-z variant, and the changed pixels in red. They are in `/home/claudio/rust-target/uodynamapper-scratch/shadow-depth-z/`, a local scratch folder that is not kept in the repository:

- `sheets/<view>-art_{cc,ec}-land_kr[-zoom2].png`;
- `crops/stairs-cc.png`.

## What moving to 35.4 would gain and cost

Gain: in KR-like, the shadows' reach and the stacking of raised sprites would match the KR client.

Costs:

- **Only KR's rule is traced.** Classic and UOD-Painterly follow the Classic client. Its painter order, taken from ClassicUO's `priorityZ` and not traced in the binary, is yet another weighting. The Enhanced client's depth rule is untraced. A per-style scale would need its own trace for each client.
- **Land would need the same bias per terrain vertex.** Otherwise a sprite's priority point and the slope under it would compare on different scales, and sprites would sink into or float over rising terrain. The flat-quad plane depth and its painter nudge would need the same treatment.
- **Depth would stop following the geometry.** Today the scale falls out of the camera. A fixed bias would have to be redefined for the perspective camera, where the view direction and the depth's non-linearity change with distance.
- **The tie-break ordinal, the depth-class offsets and the painter nudge** were tuned on the geometric scale. Their seams would need retuning.
- **Validation:** the change would need a full revalidation of all four styles (Classic, Enhanced, KR-like, UOD-Painterly) against all the art and land sources.

## Decision (2026-11-12)

The user kept the geometric scale: about 11 z per diagonal step, for every style. KR client shadows keep their caster's priority depth, which is the client's rule expressed on Dynamapper's scale.

Revisit this only if a KR reference screenshot shows a mismatch that this difference explains, for example:

- an upper-floor wall drawn over a nearer lower wall;
- a roof covering something the client shows in front of it;
- a shadow reaching a receiver the client leaves lit.

If so, run the cheap experiment first:

1. In KR-like only, scale the priority term in `logical_depth_from_projected_priority` and `client_shadow_logical_depth` by 11 / 35.4. This affects sprites and their shadows; leave land and flat quads untouched.
2. Capture multi-storey buildings, castles and roofs before and after against the reference.
3. Watch sprites on slopes: with land untouched, they are where the mismatch would first show.

## Open items

- **UOObject +0x70:** the B tie-break ordinal is unidentified. It could be a serial, a load order or a list index.
- **Stencil value 1:** whether any pass after the shadow effect reads the 1 that shadows write is not traced.
- **Render groups 0x00c931a4, 0x00c931ac and 0x00c931a8:** their contents in the main scene pass are not identified.
