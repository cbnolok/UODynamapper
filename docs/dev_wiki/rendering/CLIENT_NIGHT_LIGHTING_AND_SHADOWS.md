# Client Night Lighting, Static Shadows and Sun

KR-like and Enhanced reproduce three parts of the KR and Enhanced clients' look. Both clients run the same code for all three, except the static shadows, which only KR draws:

- the night lighting composite, which darkens and tints the finished frame;
- the static shadows;
- the fixed terrain light.

Classic and UOD-Painterly use none of them: they keep `global_lighting`, their own sun and their own sprite shadows.

The client evidence is the KR atmosphere research, [`uokr_atmosphere.md`](../../../scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md). Its sections are cited below as §1, §4, P3.3 and P3.4, and the addresses are UOKR.exe unless marked otherwise. The research scripts are in [`ghidra-kr-atmosphere/`](../../../scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/README.md), and the reference screenshots are in [`docs/images/kr_reference/`](../../images/kr_reference/).

Two other places carry user-facing descriptions of the same settings, and they must stay in line with this page:

- the comments in `dynamapper/assets/defaults/shader_presets.toml` (the legend and the KR-like and Enhanced headers);
- the dialog tooltips in `dynamapper/src/core/render/dialogs/land_shader.rs`.

## Night lighting composite

Code:

- `dynamapper/src/core/render/scene/world/effects/client_night.rs`: its module comment gives the formulas with their client addresses, the constants and a Rust mirror of the maths with unit tests.
- `dynamapper/assets/shaders/world/effects/client_night_composite.wesl`.

The settings are `[<group>.<period>.client_lighting]` (`ClientNightLighting`). The dialog shows them under Whole scene as "Light Level", "Night Overlay Strength" and "Night Overlay Colour".

### Formula

The composite is one LDR pass over the finished frame (§1, `UOLightingPostProcessEffect::BuildCompositeQuad` 0x0041f5c0). Per channel, on the 8-bit (sRGB-encoded) values:

```text
out   = clamp(4 * (c * level + B * (1 - level)) * scene, 0, 1)
c     = 0.25 + 0.2 * (gamma - 0.5) = 0.25 at the client's default gamma 0.5   (CLIENT_AMBIENT_TERM)
level = 1 - L / 31                                                           (CLIENT_LIGHT_LEVEL_DIVISOR)
```

L is the server light level, capped at 30 (`MAX_CLIENT_LIGHT_LEVEL`).

- At L = 0 the formula is the identity. Neither the pass nor the light camera runs then, so a day frame is unchanged.
- At L > 0 the composite runs after tonemapping and fog and before FXAA and SMAA, on the PlayerCamera only.

### The light buffer B

B starts as the night map: `TerrainTexture.uop` entry 21, 256 x 256. Entry 22 is the night-sight map and entry 20 is a day map the clients load but never sample.

The night map is sampled the way `UONightMapShader::SetupTextureStages` (0x00544600) does it: bilinear and wrapping, one repeat every 26 world units on the camera plane, with square pixels, turned 45 degrees and anchored to the camera focus P:

```text
u = (s_x + P_r) / 26,  v = (s_y + P_d) / 26
```

- s is the pixel's offset from the screen centre, in world units (s_y down).
- P_r = (x - z) / sqrt(2) and P_d = (x + z) / sqrt(2) are the focus along screen right and screen down, in our axes (x = UO x, z = UO y).
- The camera translation stands in for P, which only shifts the pattern by a constant (`night_map_placement`).

### Strength and colour

These two controls are Dynamapper's own: they change the night map N, not the lights.

```text
T  = N * overlay_color / NIGHT_MAP_MEAN
N' = clamp(luma(T) * (1 - overlay_strength) + T * overlay_strength, 0, 1)      (Rec. 709 luma)
```

- `NIGHT_MAP_MEAN` = (0.012604, 0.017285, 0.153818) is the measured mean of entry 21.
- With the default colour (the mean) and strength 1, N' = N, which is the client exactly.
- Strength 0 replaces the map by its luminance. Because the composite is affine in B, the night then dims by the same amount as in the client, without the blue tint.
- Strengths above 1 tint more strongly, up to `MAX_NIGHT_OVERLAY_STRENGTH` = 2.
- Another colour rescales the map's channels and keeps its cloud pattern.

### Light levels

`light_level` is a per-period setting in KR-like and Enhanced. It replaces `global_lighting` in those two groups; `exposure` stays. Each group dims in only one way: `validate_preset_lighting` in `configs/shader_presets.rs` refuses the other way in the hand-written files, and a cached `land.toml` is normalised with a warning.

| Period | Default L | Why |
|---|---|---|
| morning, afternoon | 0 | the client's full day: the composite is the identity |
| night | 10 | ServUO's night level; the research gives (0.69, 0.70, 0.87) as the multiplier at L 10 |
| cave | 26 | ServUO's dungeon level; the multiplier at L 26 is (0.20, 0.22, 0.67) |

The unit tests check those two research values within 0.0015. KR-like's and Enhanced's former daytime `global_lighting` (1.04 to 1.14) is gone.

### Packaging

The atmosphere maps go into `tex_aux_ec.uddp` without a format change. See [UDDP_FORMATS.md](../UDDP_FORMATS.md), the `tex_aux_ec` section.

- **Where they live:** entries 20, 21 and 22 sit under the reserved keys `0x80000000 | entry` (`tex_aux_ec_terrain_texture_key`), as lossless RGBA8888 at native size.
- **Source file:** the KR and Enhanced clients carry byte-identical copies, and the packer reads the Enhanced client's `TerrainTexture.uop`.
- **Older packages:** a package built before 2026-11-07 has no maps. Dynamapper then logs one warning and uses a flat texel of `NIGHT_MAP_MEAN`.

### Interim lights

The client draws its light sprites into B, lerped one after the other over the night map (P3.3, `UOObjectComponentLighting::PlaceLightSprite` 0x00438a80). Each sprite is:

- a `TerrainTexture.uop` image named by the item's tileart, in its own RGBA;
- scaled by the tileart `light_scale`;
- placed by the tileart light offsets.

No package stores the scale or the offsets yet. That needs a later format proposal.

Until then, the static light quads of `art/static_lights.rs` stand in for the client's sprites in KR-like and Enhanced:

- **Render target:** an offscreen HDR camera (`LightBufferCamera`, a child of the PlayerCamera, order -1, transparent clear) draws them on `LIGHT_BUFFER_LAYER` into an Rgba16Float image the size of the frame.
- **Blending:** they are drawn with straight alpha blending, which leaves the lights lerped over black with their coverage in alpha. The composite then forms `B = L.rgb + N' * (1 - L.a)`.
- **When they show:** B is weighted by 1 - level, so the lights appear only through the composite and vanish by day, as in the client. The additive light decals are off in these two styles.
- **Other passes:** render-world passes for the world view skip this camera (`gpu_picking` filters `Without<LightBufferCamera>`), and main-world camera queries filter `With<PlayerCamera>`.

How the interim lights differ from the client:

- Their shapes are the light.mul masks, not the client's light textures, and their size and position are ours, not `light_scale` and the light offsets.
- An unhued light takes the colour of light_generic (`CLIENT_GENERIC_LIGHT_COLOR` = (192, 162, 93)), the sprite of 462 of the client's 599 light sources. A hued light keeps its hue at light_generic's brightness, 192.
- Coverage is the mask's brightness times light_generic's peak alpha, 115/255 (`CLIENT_LIGHT_SPRITE_PEAK_ALPHA`).
- Overlapping lights blend in Bevy's transparent sort order, not the client's draw order.

## KR static shadows

Code:

- `dynamapper/assets/shaders/world/art/client_shadow.wesl`: the quad formulas and constants with their addresses;
- the `PASS_MODE_CLIENT_SHADOW` branches in `art/main.wesl`;
- `client_shadow_flags` and `client_shadow_ground_z` in `core/render/scene/world/art/statics_collect.rs`;
- the material specialisation in `art/statics_draw.rs`;
- the composite term in `client_night.rs`.

The setting is `client_shadows`, shown in the dialog as "Client Shadows" under Standard art; it is the client's Show Shadows option. It is required in the `kr` group and refused in every other group, so only KR-like has these shadows.

- In KR-like they replace the sprite shadow shape, contact and projected shadows. Those settings stay at 0 there and are greyed out in the dialog.
- UOD-Painterly keeps its own shadows.
- Enhanced has no static shadows, because the Enhanced client's shadow placement is unreferenced (P3.1).

### Who casts

Following P3.4 (the placement at 0x00440e50, gated on `showShadows`), casters are the upright billboards, meaning statics, items and foliage, whose tileart flags have none of these:

- Surface 0x200;
- Roof 0x10000000;
- Rotated 0x400000000 (bit 34);
- NoShadow 0x1000000000 (bit 36).

Notes on the flags and the casters:

- **Bit 14:** KR tests its 32-bit bit 14, which is EC's Rotated. Our tilemeta stores the EC layout (`lib/udd-assets/src/metadata/tilemeta.rs`, decoded by `lib/uocf/src/enhanced/metadata/tileart.rs`), where bit 14 decodes as `ArticleA`, so Dynamapper tests Rotated itself and ignores bit 14. NoShadow comes from EC; KR has no such flag.
- **Walls:** tileart Wall (0x10) takes the client's wall branch.
- **Missing flags:** none; every flag the client needs is in the package.
- **Mobiles:** the client gives mobiles a separate 0.5 black quad. Dynamapper draws no mobiles (the player is a debug cuboid), so there is no mobile shadow.

### Geometry and axis mapping

The client shadow is a black copy of the caster's sprite, a hair behind it on the camera plane, rotated by θ about a pivot on the ground below the caster. It works in camera-plane units, with r to the right and u up, relative to the pivot P' = P - h Zu (camera up):

```text
r0 = the sprite's left edge, W its width; u0 = its bottom edge + h Zu, u1 = u0 + its height
Rot(r, u) = (r cos θ + u sin θ, u cos θ - r sin θ)
TL = Rot(r0, u1), TR = TL + (W, 0)
h == 0: BL = (r0, u0), BR = (r0 + W, u0)       otherwise BL = Rot(r0, u0), BR = Rot(r0 + W, u0)
wall:   u0 and u1 drop by 1.05, and every corner moves by world (-1, 0, +1): one tile west and one tile south
```

Constants:

| Constant | Value | Source |
|---|---|---|
| θ | π/16 (11.25 degrees) | 0x00b87170 |
| Zu | 0.12856454 world units per UO z (4 classic pixels on screen) | 0x00b86f38 |
| ε (how far behind the caster) | 1e-4 | 0x00b42f48 |
| wall drop | 1.05 | 0x00b8716c |

The height is h = z - G. G is the land z under the caster, or the top of a Surface or Bridge item in the same cell within 4 z units below the caster (`CLIENT_SHADOW_GROUND_REACH_Z`). The land z comes from `MapPlanesRes`. Two cases are assumptions:

- a Bridge top is taken as z + height;
- when the map block is not loaded, G = z.

**Axis mapping.** The client's camera space is Gamebryo's: X is the view direction, Y up and Z right. Ours is Bevy's: x right (`BILLBOARD_RIGHT_XZ`, world (1, 0, -1) / sqrt 2), y up, and -z the view direction. So:

```text
client (X, Y, Z) = our (-view z, view y, view x),   r = view x,  u = view y
```

The client's units are ours along the screen: one tile is one world unit, and a 44-pixel diamond is sqrt 2 units wide.

Our orthographic projection stretches view y on screen, so that the classic 44 x 44 diamond matches. The maths therefore runs in square screen units, `u_screen = u_view / pixel_aspect` with `pixel_aspect = (clip_from_view[0][0] * viewport.z) / (clip_from_view[1][1] * viewport.w)`, and converts back afterwards. The sprite's card rectangle (`local_min`, `local_max`) stands in for the client's K x pixel rectangle.

### Shadow buffer S in the frame's alpha

The client draws its shadows into a separate buffer S: black, with the sprite's alpha, multiplied together. Dynamapper keeps S in the alpha channel of the main colour target (Rgba8UnormSrgb, cleared to 1). Tonemapping and the fog pass both keep alpha.

The shared shadow material switches to `PASS_MODE_CLIENT_SHADOW` while `client_shadows` is on. Its specialisation (`ArtSpriteMaterialKey`) sets:

- the write mask to alpha only;
- the alpha blend to `{Zero, OneMinusSrcAlpha, Add}`, so each shadow texel does `S = S * (1 - a)`;
- no depth writes and no depth bias.

It keeps the scene depth test, which is GreaterEqual under reverse-Z.

The composite reads S first, before the night maths, because the client runs its shadow effect before its lighting effect:

```text
scene' = scene * clamp(S + 1 - s, 0, 1),   s = 0.25   (CLIENT_SHADOW_INTENSITY, 0x00c4a024; the ctor's 0.4 at 0x0043f740 is overwritten)
```

The pass runs while L > 0 or the shadows are on, and writes alpha 1. A shadow darkens by at most 0.25, and overlapping shadows floor at 0.75 instead of stacking.

Keeping S in alpha has three limits, all different from a separate buffer:

- **Masked sprites:** a masked sprite drawn over a shadow writes alpha of at least 0.5, which `clamp(S + 0.75)` reads as no shadow. So it lifts the shadow off itself, which is also what the client's depth test gives for a sprite in front.
- **Translucent sprites:** a translucent sprite blends S partly back towards 1.
- **Partly transparent casters:** only S below 0.25 darkens. A caster texel with alpha under 0.75 leaves no mark. This follows from the client's formula, not from storing S in alpha.

### The z test

The client tests each shadow quad against the scene depth at ε behind the caster's plane. Dynamapper's sprites write a logical priority depth, not their rasterised depth, so a raster depth would not compare correctly against them. The shadow therefore takes its caster's own priority depth (`client_shadow_priority_depth`): the caster's cell, its priority z times the height scale plus its depth-class offset, pushed half a tie-break step behind the caster.

This is our reading of the client's ε: the shadow lands only on what is drawn behind the caster's plane, and the caster hides its own shadow.

The wall shift (world -1, 0, +1) runs purely along the screen's left axis, so it does not change depth.

Because of the 11.25 degree lean and the orthographic view, most of a shadow lies behind its caster. The visible parts are slivers to the right of trees and darker bands beside walls, as in the reference screenshots (for example `6490.jpg`).

## Sun direction

Both clients shade the terrain per vertex with one fixed light (§4, `UOTerrain::BuildTerrainMesh` 0x0044b7f0):

```text
g = (N . D + 1) / 2,   D = (0, -0.7071, -0.7071)   (KR 0x00c0f3b8, Enhanced 0x00dc18b8)
```

KR-like and Enhanced reproduce this with `terrain_shading_model` 1 (`client_vertex_terrain_shade` in `world/land/normals.wesl`) under the sun at azimuth 180 and elevation 45, from the south. The sun is `[<group>.<period>.sun]`, `SunAngles`, and `CLIENT_SUN` in `configs/shader_presets.rs`.

The direction comes from `UOTerrain::ComputeVertexNormal` (0x004488a0), which builds N from cross products of triangle edges. Ghidra's output shows which axis each term belongs to:

- **Edge vectors:** the decompile multiplies height differences by position differences. N[2] pairs height differences with the column differences, so the column is x. N[0] pairs them with the differences of `-row` and `1 - row`, so z = -row.
- **Height axis:** the heights are `-z * 0.12856454`, that is negated, with 0x00b86f14 = -0.0.
- **Flat ground:** N = (0, -1, 0), which points up in that frame. The world mesh uses the same x = column and z = -row with y = +height, a mirror of this frame in y only.
- **Light direction:** D therefore points towards the light. Its y component of -0.7071 points up, and its z component of -0.7071 points to increasing rows, which is UO south.

So the light comes from the south, 45 degrees above the horizon, and slopes facing the viewer are the brightest.

This corrects §4 of the research, which read D as pointing towards UO north. Commit a7767e53 also reached azimuth 180, but by treating D as pointing from the light to the ground. Captures of KR-like at azimuth 0 and 180 differ only subtly on the test slopes, and the KR reference screenshots do not settle the question on their own. The decompile does.
