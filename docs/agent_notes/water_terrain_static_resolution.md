# Water terrain/static rendering resolution

Status: resolved and visually accepted on 2026-07-21.

Production commits:

- `8475ddc fix(water): eliminate terrain-static render seams`
- `f3d581a fix(shader-ui): preserve render style across presets`

## Resolved behavior

Classic Client water can be rendered by two geometry owners at the same coast:
wet terrain cells and authored flat static-art quads. They now use one coherent
flat-water material response, so the ownership boundary is not visible under
Classic, Enhanced, or KR rendering. Fog remains functional in KR without
reintroducing the terrain/art color step or the bright patterned bands.

The accepted visual gate was map 0 near `[1445, 1790, 0]`, with water animation
and grunge checked independently and KR fog enabled.

## Final implementation

### Classic terrain source parity

Flat wet Classic terrain samples the corresponding `art.mul` land diamond from
a compact derived atlas instead of the square `texmaps.mul` texture when all of
these conditions hold:

- the land source is the Classic page-atlas path (`texture_size == 4`);
- tile metadata marks the land tile wet;
- the tiledata texture mapping is identity or zero/fallback;
- the Classic land-art slot exists;
- all four terrain-cell corners have the same height.

Sloped wet terrain retains the normal terrain source. Enhanced Client land and
art paths are unchanged; EC textures do not use the Classic one-diamond-per-cell
mapping.

Relevant code:

- `dynamapper/src/core/texture_cache/land.rs`
- `dynamapper/src/core/render/scene/world/land/mesh_material.rs`
- `dynamapper/assets/shaders/world/land/land_bindings.wgsl`
- `dynamapper/assets/shaders/world/land/sampling.wgsl`

### Shared flat-water lighting

Both terrain-owned and art-owned Classic water use the same level-surface
wrapped-Lambert response:

`albedo * (ambient + diffuse * wrapped_lambert(up, light))`

This prevents seabed/heightfield normals from producing different brightness
or low-light bands on what is visually one flat liquid surface. The art uniform
receives the land diffuse strength through the existing aligned padding slot so
both paths consume the same live preset value.

Relevant code:

- `dynamapper/assets/shaders/world/effects/surface_effects.wgsl`
- `dynamapper/assets/shaders/world/art/main.wgsl`
- `dynamapper/assets/shaders/world/land/main.wgsl`
- `dynamapper/assets/shaders/world/common_bindings.wgsl`
- `dynamapper/src/core/render/scene/world/art/statics_draw.rs`

### KR material parity

Flat Classic water under KR uses the same visual grunge on both owners, skips
terrain-only transition grime and Enhanced/KR land-light decals, and does not
feed per-sprite vertical shadow or local-light ownership into KR color wash.
Ordinary terrain, sloped wet terrain, non-water statics, and EC textures retain
their established paths.

### Fog ownership

Fog is applied exactly once by
`dynamapper/assets/shaders/world/effects/fog_pass.wgsl` after the frame is
composed. The stale per-material land/art fog functions were removed. Do not
reintroduce material-local fog: the fullscreen pass exists specifically to give
terrain and statics uniform coverage, and double application caused the final
KR-only mismatch.

## Confirmed investigation results

- Water animation was not the terrain/static tonal-gap cause.
- After the compact atlas path was active, raw sampled albedo matched across
  terrain and static owners.
- Grunge was not the remaining KR defect.
- The final KR discriminator was fog: disabling fog hid the mismatch because
  land/art material fog was being applied before the fullscreen fog pass.
- Flat quads must keep authored sprite alpha and variant selection. Replacing
  sparse shoreline/wave artwork with a generic fill loses intentional detail.
- A rejected world-cell UV experiment clamped portions of rotated flat quads
  outside their owning cell, turning transparent corners opaque and creating
  repeated chevrons. Coverage/discard must remain based on authored art UVs.

All temporary ownership colors and shader checkpoint branches were removed
before the production commit.

## Validation

- User visual acceptance: Classic, Enhanced, and KR, including KR fog enabled.
- `cargo check -p dynamapper`: passed.
- `cargo test -p dynamapper`: 94/94 passed after final cleanup.
- Diagnostic-free desktop/Vulkan run reached `InGame` without shader, module,
  binding, or pipeline errors.

## Separate follow-ups resolved on 2026-10-31

Legacy Translucent static art now carries the original Classic dispatcher's
source weight into the blended pass: 3/4 normally, 1/4 with Foliage. Original
executable blitters establish those weights; the GPU does not claim their
5-bit quantization or exact gamma-space composition. New EC world art and
authored surface materials retain their separate paths.

The isolated coast mark is original graphic 6061 at `(1447,1790,-5)`. App-only
ownership probes and an exact original-UOP/packed-RGBA comparison establish
that it is authored water detail. It is not evidence for changing map data or
selecting a different animation frame.

See [the translucency source proof and native controls](classic_static_translucency.md).
The current matching-pair Classic, Enhanced and KR coast/bridge controls are
byte-identical across the repair. Keep the accepted source, lighting, grunge
and fog ownership decisions unless new evidence contradicts them.

## Minoc river streaks are authored, not seams (2026-11-14)

Map 0 west of the Minoc mine, around `(2542,490,-5)`, looks like a water seam
but is not one. The river is static water (0x1797–0x179C at z -5) over seabed
land 0x64 at z -15, with a raised strip of 0x64 and a few land-water cells
(0xA8/0xAA/0xAB) at z -5 under the same statics.

- The pale curved streak in every style is wave art: wet foam statics
  0x179F/0x17A0/0x17AC/0x17AF along y 489 (x 2539–2543) and 0x17A3/0x17A4 down
  x 2542 (y 490–499), stacked on the plain water statics. Classic `art.mul`
  draws the same foam.
- The darker diagonal band in KR-like only is the KR static shadow of the
  bridge's north railing (wooden fence 0x085D/0x0860 at z 30, row y 498). Those
  cells have no plank under the fence, so G is the seabed (-15) and h = 45 puts
  the shadow well onto the water; `client_shadows = false` removes it. The KR
  client shows static shadows on water (tree and rider shadows in the KR
  reference capture of 2026-11-14 07:28:26).
- Matched sources show no land-water/static-water boundary: the exposed
  land-water cells at (2540,468–469) blend in Classic, Enhanced, KR-like and
  UOD-Painterly. The pale glints near the screen centre in the EC-water styles
  move with the camera (camera-space reflection); the same KR reference shows
  them around the rider.
- Mixed sources do show it. With art CC and land EC those two cells are a darker
  EC-terrain diamond among Classic static water. With art EC and land CC the EC
  item water is near-black: the art ground material binds the active land
  material's page atlas (`land_page_binding_handles` in `statics_draw.rs`), so
  the EC liquid payload indexes the Classic atlas. No client mixes them.

### Art EC with land CC draws the water statics the Classic way (2026-11-14)

The user's choice, to keep the Classic land style: while the land page atlas
holds Classic land (`LandRoutes::atlas() == Cc`, mirrored in
`StaticArtSourceState::land_style`), Enhanced art draws its water pieces from
their Classic art, as Classic art mode does (`static_continues_land` in
`statics_collect.rs`, generalised below). The pieces are the statics whose EC visual kind is
`TexLandEc` with a liquid payload, the only ones whose shader reads the land
page atlas: 261 items in the current packages, water and foam 0x1796–0x17B2 and
0x346E–0x34D2, acid 0x2E02–0x2E39 and the lava the same UOWaterShader draws
(0x12EE–0x131C, 0x2B3E–0x2B65, 0x3296–0x3299, 0x343C–0x354B). They become
`CcRegular`, so the EC sprite atlas (which already holds Classic pages for EC
art fallbacks) draws them through the Classic flat-quad path with Classic
wetness, depth class and picking geometry; tex_art_cc.uddp loads whatever the
art source. The EC-art blood items (`EcSurface`, sprite path) and the other EC
art are unchanged, as are matched sources and land EC/KR.

The opposite mix (art CC, land EC/KR) keeps its land-water cells as they are
(its water statics follow the land since the next section): they keep
the EC terrain liquid. Matching it the same way would route the wet land ids to
Classic while the art is Classic (`resolve_land_routes`, re-resolved on an art
source change, with a land atlas rebuild) and draw those Classic-routed flat
wet cells from the compact Classic land-art atlas, as land CC does, rather than
from square texmaps.

### Statics that continue the land take the land's style (2026-11-14)

The user generalised the rule above: every static that continues the land
draws in the land's style, whatever the art source, without switching an art
tile that is also used as an ordinary floor. `StaticLandStyle` (Classic,
Enhanced for land EC and KR, Unknown before the land cache exists) replaces
`classic_land`; `static_continues_land` decides from the data alone:

- the item's Enhanced kind is a land liquid (`TexLandEc` with a liquid
  payload: 261 items), or an item surface or land-textured item
  (`EcSurface`, dry `TexLandEc`) one of whose world-art base textures a
  terrain material of tex_land_ec.uddp draws (selected or primary texture of a
  terrain provenance record): grass, dirt, snow, swamp, the lava fields;
- and it is not NoShoot (tiledata 0x2000). Every built-floor family that
  shares a terrain texture (cobblestones, stone pavers and their dark kind,
  marble and white marble, grey slate, sandstone paver, bricks, flagstones,
  blue and red tiles, cave floor, stone roof) carries NoShoot; no nature
  family (liquids, lava, swamp, grass, snow, dirt, whirlpool, forest) does.
  Measured over all maps, those floors lie at their cell's land z on land of
  the same texture for 0-4% of their placements (cobblestones 151 of 44,310);
  the rest are raised, roofed or on other land, as house, castle and dungeon
  floors, so they keep their art source.

837 items continue the land in the current packages: the 261 liquids and 576
terrain-textured surfaces. Under Classic land Enhanced art draws them from
their Classic art when it exists (else the Enhanced kind). Under Enhanced or KR
land Classic art draws their Enhanced kind when tex_art_ec has their image: the
art atlases are then built for Enhanced art (`art_atlas_source` in
`statics_draw.rs`: the EC sprite atlas holds the Classic pages too, and the
ground atlas holds the land pages the liquids read), and the item-surface
boundary coverage runs as in Enhanced art mode. Matched sources resolve
exactly as before (`canonical_statics_that_continue_the_land_take_its_style`).
