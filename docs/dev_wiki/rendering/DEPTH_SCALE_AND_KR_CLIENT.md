# Depth Scale: Dynamapper and the KR Client

How Dynamapper decides which static, shadow or land surface lies in front, how the Kingdom Reborn client decides it, where the two disagree, and the rule Dynamapper follows: ClassicUO's (decision of 2026-11-16, accepted by the user on 2026-11-16; it replaces the geometric scale kept on 2026-11-12).

In short:

- Every renderer here sorts by the cell's diagonal row x + y and by a priority z.
- Dynamapper follows ClassicUO: the nearer row always wins, whatever the heights; within a row the priority z decides. One rule for every style and for both cameras.
- The KR client weighs a diagonal step as **35.4** priority z units, so a static more than 35.4 z above a nearer one draws over it there. Dynamapper's earlier geometric depth weighed it as about **11**.
- KR client shadows sort at their caster's depth in both renderers, so a shadow follows the same rule as the sprites around it.

Related: [KR static shadows](CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md#kr-static-shadows), the client trace in [`uokr_atmosphere.md` P3.4](../../client_research/ghidra-kr-atmosphere/results/uokr_atmosphere.md#p34-sprite-shadow-geometry), and [static depth ordering](../../agent_notes/static_depth_ordering.md) for the history of the depth classes and tie-breaks.

## Dynamapper: ClassicUO's draw order (2026-11-16)

### World and camera

- The world axes are: x = UO x, z = UO y, and y is up.
- A UO z unit is `UO_Z_TO_WORLD_SCALE` = 4/44 world units (`util_lib/uo_coords.rs`, mirrored in `world/uo_coords.wesl` and used in the art shaders as `HEIGHT_SCALE`). That is four classic pixels in the 44-pixel grid.
- The orthographic camera sits at `PlayerCamera::BASE_OFFSET_FROM_PLAYER` = (5, 5, 5) from the player and looks at the player (`core/render/view/camera.rs`). The view direction is therefore (-1, -1, -1) / √3.
- The perspective camera orbits along the same `Vec3::ONE` direction (`perspective_offset`).
- The lens stretches view y on screen to make the 44 × 44 diamond. That changes x and y in clip space, not the view depth.

### The rule

ClassicUO (`Map.cs AddGameObject`, `GameScene` draw) draws the cells by diagonal row and, within a cell, by priority z:

- **Statics:** priority z = tile z, - 1 for Background, + 1 when the tiledata height is not 0, + 1 for MultiMovable. Surface adds nothing (ClassicUO has that line commented out). `statics_collect.rs resolve_priority_z_units`.
- **Land:** its average z - 2 (z - 1 for the tile, - 1 again when it is added to the cell). The average z is `Land.AverageZ`: the mean of the corner pair along the diagonal that rises less; on level ground the cell's z.
- **Equal priority z in one cell:** the later static draws over the earlier one.

Dynamapper turns that into one logical distance (`statics_collect.rs logical_draw_distance`, `art/depth.wesl`):

```text
L = row + (priority z + tie level / PRIORITY_TIE_LEVELS_PER_Z) / PRIORITY_Z_STEPS_PER_ROW
    PRIORITY_Z_STEPS_PER_ROW = 264   (> the priority z range, -130..129: z never crosses a row)
    PRIORITY_TIE_LEVELS_PER_Z = 9
```

- **The row** is the diagonal x + y of the point: an upright sprite's anchor (CC art anchors at (1, 1), EC art at (0.5, 1.5), so both add 2). The land, the flat quads, the ground art and the projected shadows take their cell's anchor, the cell's corner + (1, 1) (`CELL_DEPTH_ANCHOR`), for every fragment of the cell. ClassicUO draws a cell's land and statics as whole objects in the cell's row; one depth per cell does the same, so a cell's floor covers its own land and the walls below it whatever their real heights.
- **The authored EC camera quads** (`flat_quad` 3, Rotated in the tileart, such as the EC cobblestones) stand as cards at the EC art's anchor, so they take that anchor (`sprite_depth_anchor`).
- **The tie level** is 0 for the land, the flat quads (`flat_quad` 1 and 2) and the ground art. Upright sprites take 1..8: 1 + 2 × min(rank, 3) + (x mod 2), with rank the sprite's place among the statics of its cell and priority z in emission order (`assign_sprite_tie_levels`). The later static is nearer, as in ClassicUO; the x parity separates equal statics of neighbouring cells in one row, whichever chunk batch draws first. A KR client shadow takes its caster's level - 1.
- **Every fragment that writes depth** takes the depth of its logical point: the cell at a logical height of the priority term in world units, `(x, (priority z + level / 9) / 264, y)`. Its view z, `dot(view z axis, P - camera)`, is computed camera-relative, so the levels survive at any map position. The active camera's projection maps it to the depth (`view_z_depth`), orthographic or perspective. The view direction runs along (1, 1, 1), so the view z is L / √3 plus a constant and the depth is monotonic in L in both cameras.
  - Upright sprites and KR client shadows: one constant depth over the card (vertex shader, `sprite_priority_view_z`).
  - Wall caps in the perspective camera: a wall's card is drawn for the isometric view, where a cap pixel's line of sight meets the wall's top inside its own cell. Off the screen's centre a perspective ray meets the top up to a pixel or two past the cell's far edge, in the farther row, where the floor laid on the top lost to the nearer wall: a 1-pixel line of every covered wall top showed along the floor tiles' edges. Those cap fragments take the farther row, at the top's z - 2 and the highest tie level (`perspective_cap_row_shift`, `perspective_cap_priority_z`): under the floor laid on the top, Background floors included, over the wall's neighbours and what else stands below the top. Only the walls with a floor laid on their top in the farther row move their caps: a Surface or Bridge in the cell west or north of the wall whose top is the wall's (`wall_cap_meets_floor_flag` in `statics_collect.rs`, the instance bit `STATIC_FLAG_CAP_MEETS_FLOOR`; the blocks west and north of a block lend their floors to its edge walls). A wall under a roof, or under another wall stacked on it, keeps its cap in its own row. Moving those too let the next wall up, standing on the top in the farther row, and the lower piece's art in the wall's own row cover the moved cap: dashes along the eaves and the palisades' tops and notches on the crenellations. A tree trunk or a boulder has no flat top and never moves its cap, and an orthographic camera never moves one. The fixtures `britain-castle-wall-walk-perspective-wall-tops` (the floors cover the caps) and `britain-house-gable-palisade-perspective` (stacked gable walls keep theirs) guard it.
  - Land: its cell's depth at the average z - 2, in the fragment shader (`land/main.wesl`, `cell_priority_view_z`, `frag_depth`). Each vertex passes its cell and the cell's average z as a flat varying (`depth_cell`), which every fragment of that cell reads; a fragment of another cell (the coarser LOD meshes, past a bank) reads the four corners from the tile atlas. Writing the depth in the fragment shader turns off the early depth test for the land.
  - Flat quads and ground art: one constant depth at their cell's anchor, which the instance carries as `tile_x`, `tile_y` (`cell_depth_anchor`).
  - Projected shadows: the depth of the cell each fragment falls on, at the caster's base z - 0.5, level 0: in front of the land and the Background floors there, behind the statics standing at that base.
  - Pick pass and light scene: the same depth. The light scene accepts a drawn depth within 0.45 tie levels of view z (`LIGHT_SCENE_DEPTH_TOLERANCE_LEVELS`).
  - Caves drawn over the map: lifted toward the camera in view z (`cave_lifted_view_z`), so the lift holds in both cameras.
- **The CPU sort** (`static_depth_key`, f64) and the GPU take the same function. `depth_tests.rs` checks that every pair the CPU orders the GPU orders the same way, strictly, down to adjacent tie levels, at Britain's map position, in the orthographic camera and in the perspective camera at orbit distances 138 (1080p, zoom 1) and 552.
- **KR uses one parameter more.** To weigh a row as the KR client does, set `PRIORITY_Z_STEPS_PER_ROW` to 35.4 (shader and CPU together). There is no preset or settings key.

### What the rule removed, and why

| Removed | What it did | Why it went |
|---|---|---|
| `effective_priority_height` | Full tiledata height (10 for a flat Regular item, half for a Bridge) on the priority. | Not traced from either client. A wall's 20 counted nearly two rows, so it covered the table one row in front (Minoc hall, 2026-11-15). Replaced by ClassicUO's + 1. |
| The geometric scale (11 z per row) | Sprites' priority points and the land's rasterised depth on the camera's own geometry. | A raised static more than 11 z per row above a nearer one drew over it. Rows now dominate. |
| Foliage +2.0 world y (22 z) | Trees two rows nearer. | No client has it. Under row dominance it can only cross rows, which the rule forbids. |
| Background -0.001 and Roof +0.002 world y | Tie nudges between floors, roofs and walls at one priority. | Background is ClassicUO's - 1 priority z; a roof has no special priority in ClassicUO. |
| SurfaceLikeFloor bare z, and its -4.0 CPU key | Tables and counters at their bare z; the CPU key 44 z down. | ClassicUO gives them z + 1 for their height, like any static. The CPU key now equals the GPU depth. |
| The tie-break ordinal (1e-6 NDC orthographic, 0.002 view units perspective) | Separated equal keys; the earlier static drew nearer. | Replaced by the tie levels above, a fixed fraction of a priority z in both cameras; the later static is nearer, as in ClassicUO. |
| The KR shadow's half tie-break step | Shadow just behind its caster. | Half a level is under one unit in the last place of the orthographic depth; a whole level is the least that survives. |
| Perspective's rasterised land and flat-quad depth | Real geometry in perspective. | One rule in both cameras. |
| The horizontal plane solved per pixel for flat quads and ground art (`horizontal_plane_frag_depth`) | The real point of a flat surface under the pixel. | A flat surface spanned its cell's rows, from x + y to x + y + 2, while the sprites stand at x + y + 2: a lower wall of the same cell drew over most of the floor laid on its top (the keep's and the castle's upper floors). It takes its cell's depth now. |
| The painter nudge (`horizontal_plane_painter_depth`) | One unit in the last place in the near half of a flat quad's cell, so the nearer of two coplanar quads of neighbouring cells won the pixels they share. | Neighbouring cells that share an edge are a row apart now. Cells of one row meet only at a corner. |
| The land's per-vertex depth | The land's real point under the pixel. | As for the flat quads: the land of a cell spanned its rows, so a sprite of the cell behind covered the near half of the nearer land, and the land of a cell covered the far part of the flat art laid on it. |

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
| Dynamapper (ClassicUO, 2026-11-16) | never: d > 0 decides |
| Dynamapper before 2026-11-16 (geometric) | dz > 11 d |
| KR client | dz > 35.4 d |
| ClassicUO exactly | dz > 100 d (its 0.01 per z) |

Dynamapper and the KR client now disagree only when dz > 35.4 d: the client draws a static that much higher over the nearer one, Dynamapper the nearer one. ClassicUO's own key, (x + y) + 0.01 (127 + priority z), lets z cross rows only above 100 z per step; Dynamapper does not.

| Case | dz | d | Result |
|---|---|---|---|
| Items on a table, small stacks, a crate on a crate | 0..10 | 0..1 | Same answer everywhere. |
| An upper-floor wall against the ground floor's wall one cell nearer | ≈ 20 | 1 | Dynamapper and KR: the lower wall in front (before 2026-11-16: the upper wall). |
| A roof (+30..+40 over the floor walls) | 30..40 | 1..3 | Dynamapper: the nearer row. KR: the roof in front at 1 step when dz > 35.4. |
| Towers and battlements (+40 and more) | 40+ | 1..3 | KR: the raised sprite in front up to dz / 35.4 steps. Dynamapper: the nearer row. |

Which sprite wins only shows where their art overlaps on screen. A raised sprite is drawn higher up the screen, so many of these pairs never share a pixel.

The land, the flat quads and the ground art follow the same rule: their cell's row and priority z. In the pair survey they count as statics of their cell.

Pair survey, map 1's MUL statics: every pair of statics one or two diagonal steps apart (8,292,065 pairs; `uodynamapper-scratch/cuo-draw-order/py/survey.py`, not in the repository).

| Rule | Disagrees with KR | of them at the same z | Disagrees with ClassicUO | of them at the same z |
|---|---|---|---|---|
| Before (full height, foliage +22 z, 11 z per step) | 786,936 | 238,699 | 915,578 | 238,699 |
| ClassicUO, rows dominate (2026-11-16) | 129,431 | 0 | 609 | 0 |

- 916,183 pairs change winner (238,699 of them at the same z).
- The 609 left against ClassicUO are its 0.01 per z crossing rows (dz > 100 per step).
- The largest groups left against KR are raised statics more than 35.4 z per step above a nearer one: Background against Background one step apart with dz 21..40 (22,368 pairs, where KR's 35.4 also lets z win), a roof against Background with dz > 40 (13,792), Background against a wall with dz 21..40 (12,071).
- Earlier variants of the height term, against KR: capped at 10, 735k (125k at the same z); the client's + 1, 785k (132k).
- Ties left to the tie levels: groups of statics with one cell and one priority z, of sizes 1: 2,765,200; 2: 71,017; 3: 4,571; 4: 736; 5: 140; 6: 44; 7: 6. 246 statics (0.009 %) lie beyond the 4 ranks and share the last rank's level; they keep their draw order within their chunk batch.

### Shadow reach

A KR client shadow sorts at its caster's depth, one tie level behind. It darkens a receiver dz priority units below the caster that lies:

- in Dynamapper, in the caster's own row or behind it, never a row in front (before 2026-11-16: up to dz / 11 rows in front);
- in the client, up to dz / 35.4 steps in front of the caster.

So a high caster's shadow reaches a little less far forward onto low receivers in Dynamapper than in the client. At the caster's own row the two match.

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

Since 2026-11-16 the land, the flat quads, the sprites and their shadows share one logical distance, so the scale is one parameter, `PRIORITY_Z_STEPS_PER_ROW` (the shader's and the CPU's). The costs left:

- **Only KR's rule is traced.** Classic and UOD-Painterly follow the Classic client (ClassicUO's rule). The Enhanced client's depth rule is untraced. A per-style scale would need a per-style parameter (a uniform) and its own trace for each client.
- **z would cross rows.** At 35.4 the tie levels and the land's z - 2 still order as in KR, but the range check in `depth_tests.rs` and the pair survey would need redoing.
- **Validation:** a full revalidation of all four styles (Classic, Enhanced, KR-like, UOD-Painterly) against all the art and land sources.

## Decision (2026-11-12, superseded)

The user kept the geometric scale: about 11 z per diagonal step, for every style. KR client shadows kept their caster's priority depth, which is the client's rule expressed on Dynamapper's scale. The section [Where the two disagree](#where-the-two-disagree) gives the old rule's numbers next to the new ones.

## Decision (2026-11-16, accepted by the user on 2026-11-16)

One draw-order rule for every style (Classic, Enhanced, KR-like, UOD-Painterly), identical in the isometric and perspective cameras: ClassicUO's. The nearer row always wins over any height; within a row the priority z decides (tile z, + 1 for a non-zero tiledata height, - 1 for Background, + 1 for MultiMovable; land z - 2); `effective_priority_height` is removed. Delivered on a branch with before/after sheets of every fixture in all four styles and both cameras. The user reviewed the sheets and accepted the rule on 2026-11-16, after two fixes: the wall caps in the perspective camera, and the land's GPU cost (see Benchmark).

### Before/after sheets

Each sheet shows one fixture, before and after for every style and source. The sheets are in a local scratch folder, which is not kept in the repository, under `/home/claudio/rust-target/uodynamapper-scratch/visual-baselines/compares/`:

- `before-cuo-order-ortho--after-cuo-order-ortho/sheets/<fixture>.jpg`: the isometric camera, the full tier;
- `before-cuo-order-persp--after-cuo-order-persp/sheets/<fixture>.jpg`: the same fixtures in the perspective camera;
- `after-cuo-order-persp--after-v3-persp/sheets/<fixture>.jpg`: the two fixes against the reviewed rule, in the perspective camera. 55 captures of 12 fixtures change, every one a wall-top line over the floor laid on it that is gone (the castle's and the keep's wall-walks, the roof ridge's and Trinsic's walls, the upper floors, the stacked stairs' landing), plus 1 to 3 px specks in the Yew crypt and the keep at z 50. The isometric camera has no changed capture (`after-cuo-order-ortho--after-v3-ortho`: 344 identical, 19 within the tolerance).

The before binary is a23b6e0d. It includes dev 7ba3b49d and the draw-order fixtures. The after binary is fc8c31db, the rule the user reviewed; v3 adds the two fixes. (v2, which moved every wall's cap and kept the land's early depth test, was not merged: its caps made dashes under the eaves and on palisades.)

### Benchmark

The benchmark used a 1900 x 1000 window at zoom 0.5, with no frame limiter and no vsync, in the afternoon. It had two views:

- land-mountains: map 1, (2542, 488), statics off;
- city-britain: map 1, (1496, 1628).

Each cell is the median of three launches; each launch's figure is the median of 8 s of the app's frame time. dev is a23b6e0d (the geometric scale), v1 fc8c31db (ClassicUO's order, the land's average z read from the atlas per fragment), v3 this branch (the average z as a flat varying). dev and v1 ran together, their launches rotating; v1 and v3 ran together later, on v3's binary with v1's shaders for v1 (its Rust differs only by the wall-cap flag). The two runs of v1 agree within 1%, so the v3 vs dev column compares across them. The same measure is rerun with [`frame_benchmark.py`](../../../skills/renderer-reference-validation/SKILL.md#frame-time-benchmark) (2026-11-18), its views and timings these.

| Camera | Style | View | Frame ms dev (range) | v1 (range) | v1 rerun (range) | v3 (range) | v3 vs v1 rerun | v3 vs dev | GPU busy dev / v1 / v3 |
|---|---|---|---|---|---|---|---|---|---|
| orthographic | Classic cc/cc | city-britain | 8.29 (8.12-8.38) | 8.26 (8.16-8.33) | 8.23 (8.23-8.33) | 8.28 (8.18-8.29) | +0.6% | -0.1% | 67% / 73% / 71% |
| orthographic | Classic cc/cc | land-mountains | 5.65 (5.63-5.73) | 5.84 (5.76-5.86) | 5.75 (5.73-5.89) | 5.77 (5.75-5.78) | +0.4% | +2.1% | 62% / 70% / 67% |
| orthographic | KR-like ec/kr | city-britain | 22.24 (22.16-22.24) | 23.14 (23.10-23.25) | 23.14 (23.10-23.14) | 22.70 (22.65-22.77) | -1.9% | +2.1% | 99% / 99% / 99% |
| orthographic | KR-like ec/kr | land-mountains | 14.47 (14.40-14.51) | 15.56 (15.51-15.58) | 15.43 (15.43-15.54) | 14.98 (14.96-14.99) | -3.0% | +3.5% | 98% / 98% / 98% |
| perspective | Classic cc/cc | city-britain | 8.15 (8.13-8.24) | 8.31 (8.27-8.49) | 8.31 (8.31-8.42) | 8.33 (8.28-8.35) | +0.2% | +2.2% | 68% / 73% / 72% |
| perspective | Classic cc/cc | land-mountains | 5.69 (5.67-5.77) | 5.86 (5.72-5.88) | 5.80 (5.75-5.84) | 5.84 (5.80-5.87) | +0.7% | +2.6% | 61% / 69% / 66% |
| perspective | KR-like ec/kr | city-britain | 21.94 (21.92-22.11) | 22.92 (22.91-22.97) | 22.83 (22.74-23.05) | 22.44 (22.39-22.46) | -1.7% | +2.3% | 99% / 99% / 99% |
| perspective | KR-like ec/kr | land-mountains | 15.71 (15.57-15.72) | 16.83 (16.70-16.86) | 16.72 (16.67-16.78) | 16.32 (16.17-16.34) | -2.4% | +3.9% | 98% / 98% / 98% |

**Classic** is not GPU-bound. Its frame time stays within about 3% of dev and moves by under 1% between v1 and v3; its GPU busy rises by 4 to 6 points over dev.

**KR-like** is GPU-bound. v1 cost about 1 ms; v3 recovers about half of it:

- 3.5 to 4% over dev on land-heavy views (v1: 7 to 8%);
- 2% in a city (v1: 4%).

An earlier build also declared the land's depth no nearer than a conservative per-vertex bound, so the early depth test could run (`@early_depth_test(less_equal)`). The flat varying alone measured the same (KR-like orthographic, three launches each: land 15.04 ms, city 22.81 ms against that build's 15.11 and 22.79), and so did that build without the cave-lift reads of its bound. It was dropped. The rest of the cost is the land's depth export itself.

## Open items

Left by the 2026-11-16 rule, for the user's review:

- **The player cube** keeps its rasterised depth, so at a high z it can draw over statics several rows in front of it.
- **A free or rotated perspective camera** is row-dominant only near the (1, 1, 1) direction: the logical points are projected along the camera's own view axis.
- **Statics beyond four ranks** (246 on map 1) share the last tie level and keep their order only within their chunk batch.
- **KR client shadows** no longer reach rows in front of their caster (see Shadow reach).
- **Mixed CC and EC anchors** in one row differ by x parity, not by art source; an EC sprite and a CC sprite of one cell and priority z keep their emission order.
- **The land's depth is written in the fragment shader** (`frag_depth`), which turns off its early depth test. With the flat average z it still costs the GPU-bound KR-like style 3.5 to 4% of its frame time on land and 2% in a city over dev (v1: 7 to 8% and 4%; see Benchmark); declaring a conservative depth to keep the early depth test gained nothing measurable. Writing no depth at all would need each quad of the full-detail mesh to own its vertices, so that a constant per-vertex depth is the cell's, with the fragment depth kept for the coarser meshes and the banks: not attempted.
- **A wall cap moved back a row in perspective** still loses to a static rising over the wall's top in that row, where a floor lies on the top too (a merlon beside a wall-walk's floor tile). Only the walls with such a floor move their caps (see Wall caps), and the full perspective tier shows no notch or dash from it. A wall without a floor on its top keeps the hairline of its cap where the perspective ray meets a farther floor at another z, or a floor of a multi in the neighbouring block (the flag reads the map statics of the blocks west and north, and a multi's own parts): none shows in the fixtures. A single depth for the moved cap cannot do it alone: the floor at the top's z and a wall or roof standing on the top differ by one priority z, the latter higher.
- **EC cave rock faces** composite differently: they are ordered by row now, no longer by height (minoc-mine-floor-flat-art and the Minoc mine fixtures, all EC styles). Neither order has been checked against an EC client capture.
- **ClassicUO's non-stretched land.** ClassicUO keeps the bare z for a land with no texmap or a wet land without a texture; Dynamapper stretches every land and takes the average z. They differ only on sloped cells of such land.
- **UOObject +0x70:** the B tie-break ordinal is unidentified. It could be a serial, a load order or a list index.
- **Stencil value 1:** whether any pass after the shadow effect reads the 1 that shadows write is not traced.
- **Render groups 0x00c931a4, 0x00c931ac and 0x00c931a8:** their contents in the main scene pass are not identified.
