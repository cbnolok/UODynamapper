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

## Separate follow-ups

These were observed during the investigation but are not part of the resolved
terrain/static seam:

- partially transparent/see-through Classic water statics may lose their
  intended transparency;
- a stray authored wave sprite may require separate map-data or animation-frame
  routing analysis if it still reproduces.

Treat either as a new bounded task. Do not reopen the accepted source,
lighting, grunge, or fog ownership decisions without new evidence.
