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

The settings are `[<group>.<period>.client_lighting]` (`ClientNightLighting`). The dialog shows them under Whole scene as "Light Level", "Night Overlay Strength" and "Night Overlay Colour". Next to them, "Local Lights" and its intensity slider are one setting per style, `[<group>.local_lights]` (`ClientLocalLights`, see Terrain and statics below).

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
- **Light sprites:** `tilemeta.uddp` carries each item's light (`metadata/item_lights.bin`: the light texture's entry, the tileart light scale and offsets), and `tex_aux_ec.uddp` carries every light texture those name, under the same reserved keys and the same lossless rules (UDDP_FORMATS.md §2.3 and §2.5). Both are required.

### Light sprites

The client draws its light sprites into B, lerped one after the other over the night map (P3.3; the full trace is in `docs/agent_notes/light_sprite_proposal.md`). KR-like and Enhanced draw the same sprites (`art/client_light_sprites.rs`):

- **Which statics:** a static draws a sprite when its tileart shader block 3 names a light texture (`TileMetaPackage::item_light`). The LightSource flag plays no part: the items with the flag and no texture get no light, as in the clients.
- **Texture:** the `TerrainTexture.uop` entry from `tex_aux_ec.uddp`, in its own RGBA, uploaded as `Rgba8Unorm` (not sRGB, so B holds the client's 8-bit values) with linear filtering, clamped addressing and no mips. No hue, even on a hued item.
- **Quad:** the EC static billboard (right = screen right, up = world Y) at the static's base point, the client's (x + 0.5, y + 1.5) at its z. The sprite window is the texture at K * light_scale world units per pixel (K = sqrt(2) / 66), its left edge 22 - W/2 pixels right of the base and its bottom edge at the base; then the light offsets move it, in world units along the camera's right (offset_x) and up (offset_y, which is 1/sqrt(2) of world Y). The placement follows the map position and the tileart alone, so it is the same under CC and EC art.
- **Centring (Dynamapper's):** the style's `[<group>.local_lights] centre_on_items`, on in the bundled presets ("Centre Lights on Items" in the shader window), moves a sprite across the ground until it shows over its item's cell centre, at its height above the item, when the clients' placement shows it more than 4.5 tiles from that centre (`client_light_sprite_centred`, `CLIENT_LIGHT_SPRITE_KEPT_PLACEMENT_REACH`). The rule follows the tilemeta: the offsets the tileart gives an item of its own keep the glow within 4.04 tiles of its cell (655 of the 1042 lights: lantern 0.36, window beams 1.8 to 2, wall torches 0.64, fireplace fires 1.1 to 1.9, sconce 2555 3.28, the farthest a window at 3.96), while the tileart's default offsets, (-5, -4) on 378 items and (0, 0) at scale 2 on 7, and two candelabras' (-4.7, -2) throw it 4.78 to 5.59 tiles away (387 lights: lava, fields, fire columns, campfires, hearths and fireplaces beside their fire, wall sconces 2557-2559 and 2563-2564, windows such as 186, 141 and 143), and no light falls in between. Neither the tiledata flags nor the facing separate the two: the (-5, -4) group holds Wall and Window items of both facings next to floor fires. The light on the ground taken from a centred sprite lands on the cell centre. Off, the sprites keep the clients' placement.
- **Item light scale (Dynamapper's):** `assets/runtime_specs/lights/LightScale.kdl` (`configs::light_scale`, strict; a copy under `cache/runtime_specs/lights/` takes its place, read at startup) multiplies chosen items' sprite size about the glow's centre, and with it the light on the ground taken from it; with `source = "light_mul"` it multiplies the light.mul light's size in these two styles. The bundled file enlarges the lava 1.5 times. Classic and UOD-Painterly draw every light as the client does.
- **Scan:** a sprite reaches up to about 27 world units across and its offsets up to 11.5, so the collector scans the statics beyond the visible chunks by the largest reach of the loaded lights, computed once and enlarged by the largest item light scale, and rescans only when the visible chunks or the centring change.
- **Render target:** an offscreen HDR camera (`LightBufferCamera`, a child of the PlayerCamera, order -1, transparent clear) draws them on `LIGHT_BUFFER_LAYER` into an Rgba16Float image the size of the frame.
- **Blending:** they are drawn with straight alpha blending, which leaves the lights lerped over black with their coverage in alpha. The composite then forms `B = L.rgb + N' * (1 - L.a)`.
- **When they show:** B is weighted by 1 - level, so the lights appear only through the composite and vanish by day, as in the client. The additive light.mul decals of `art/static_lights.rs` are off in these two styles; Classic and UOD-Painterly keep them.
- **Terrain and statics:** the clients light them only through the composite: their sprites are unlit, and their terrain has no light other than the half-Lambert sun. The optional land light decals and static local lights (Light/Static Interaction, 0 in the KR-like and Enhanced presets; the static local lights also need `enable_static_art_local_lights`) are Dynamapper's own. Each style's `[<group>.local_lights]` `source`, one setting for every time of day, picks what they read in these two styles (the Local Lights choice in the shader window). `"light_sprites"`, the default, reads the light sprites: each sprite gives a light on the ground under its centre (the centre moved down the camera's view ray to the static's base height), sized like the light.mul masks (legacy pixels / 44) and coloured by its texture's alpha-weighted mean colour (`client_light_sprite_static_light`). `"light_mul"` reads light.mul as Classic does: the statics with the LightSource flag (flag-only items included), `quality` as the light.mul id, the light's size and its colour (the hue, the ClassicUO light shader or the mask's colour). Either way the lights are added at any time of day, scaled by Light/Static Interaction times the source's own intensity (`light_sprites_intensity` or `light_mul_intensity`, 0 to 4, 1 the lights as they come; the shaders read it as `effects.local_light_intensity`, 1 in Classic and UOD-Painterly), before the composite darkens the scene, and the light buffer keeps its sprites. At one intensity light.mul lights as brightly at a light's centre (its colours are at full strength in their strongest channel, the sprites' the textures' alpha-weighted mean) but fewer statics and smaller pools, none at some lanterns the sprites light, so it lights less overall; about 1.3 evens them out where both have a light.
- **How the static local lights fall on the statics:** each static takes them at its quad's four corners (`static_local_light_corners` in `art/statics_collect.rs`, packed RGBA8) and the art shaders interpolate them, so a light falls off smoothly across a floor or a roof of many tiles; a flat static (a flat quad, or a CC card showing a ground diamond, `STATIC_FLAG_FLAT_ART`, which takes its corners on the ground they show) catches them evenly, an upright one more towards its top. Taking each static's strongest sample and ramping it up every tile drew dark and light rectangles on lit floors and flat roofs.
- **Other passes:** render-world passes for the world view skip this camera (`gpu_picking` filters `Without<LightBufferCamera>`), and main-world camera queries filter `With<PlayerCamera>`.
- **Without the packages' entries:** a `tex_aux_ec.uddp` packed against another tilemeta may lack a texture; Dynamapper warns once per texture and draws those lights without a sprite. Without `tex_aux_ec.uddp` it warns once and draws none.

Where they may still differ from the client:

- Overlapping lights blend in Bevy's transparent sort order, not the client's batch order (U4).
- The texture's v orientation is the EC statics' (row 0 at the top), which matters only for the asymmetric textures 2, 3, 4 and 7 (U2).
- The large default offsets (-5, -4) of 378 EC items put their glow about 4 world units left of the item. Both binaries apply them literally (re-traced on 2026-11-12, for the lava too), so Dynamapper does with `centre_on_items` off. Enhanced client night screenshots (2026-11-13: a sandstone building with lit window slits and a column fireplace, and day lava) show every glow at its emitter, as Dynamapper places the tileart's own offsets, but no displaced (-5, -4) glow one can tie to an item, so they neither confirm nor refute the shift (U1); the building was not identified on the map.

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
wall:   u0 and u1 drop by 1.05, and every corner moves by the client's world (X -1, Z +1): one tile west and one tile north
```

The client's world Z is -UO y (`UOSprite::PlaceSprite` 0x004422b0 sets the base point's Z to -(y + 1.5)), so the wall shift is UO (x - 1, y - 1), our world (-1, 0, -1). On screen that is one tile straight up, 1.412 units, of which the 1.05 drop takes back all but 0.36. Along the view it puts the quad two diagonal steps further back (see the z test below). Until 2026-11-12 Dynamapper read Z as +UO y and moved wall shadows one tile to the screen left and 1.05 down, which laid each wall's shadow over its own lower neighbours: a sawtooth of horizontal shadow edges down every wall face, and the upper floors' wall shadows on the lower floors' walls and on the ground in front of doorways.

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

The client's view direction is V = (-0.0354, 0.9988, 0.0353) in its world (the camera rotation `Rz(-π/2) Rx(0.001 - π/4) Rz(-0.05)`, 0x0045ae50, whose right and up columns are the R and U of the grunge research, P3.6). A sprite's base point is (x + 0.5, 0, -(y + 1.5)) + z Zu U, and V . U = 0, so the base point alone gives -0.0354 (x + y). The sprite component (0x00435120) then adds a bias along V to every corner (descriptor +0x60..+0x68, read by `BuildSpriteBatch` 0x004454c0):

```text
depth = -0.0354 (x + y) - 0.001 A - 1e-5 B            (0x00b4f310 = -0.001, 0x00b86de8 = 1e-5)
A     = z + 128 + (height > 0) + (flag 0x8000 and not +0xd0) - Background     UOObject vfunc +0xdc, 0x00561c00
B     = UOObject +0x70                                                        vfunc +0xe4, 0x00561c70; an ordinal, not identified
```

A is the classic draw priority, the one `resolve_priority_z_units` follows, so the client's depth does carry the z: one diagonal step weighs 35.4 priority units, one z unit 1/35 of a step. The component writes the same bias into its shadow's descriptor (0x004353e4..0x004353ea, after the placement call at 0x004353cd; the refresh path 0x00435630 does the same at 0x00435760..0x0043576a), and the placement adds ε along V. **The shadow sorts at its caster's priority depth, ε behind:** the bias is computed from the caster, not from the ground under it. Terrain takes a matching bias per vertex (0x00448de0, from `BuildTerrainMesh` 0x0044b7f0): -0.001 (z' + 127) + 2e-5, with z' the lower of the cell's own corner and its south corner, about one priority unit behind a sprite standing at the same z.

Dynamapper keeps that rule (`client_shadow_logical_depth`): the caster's cell and its priority z times the height scale, half a tie-break behind. What differs is the scale. Our logical depth weighs a diagonal step as 11 z units (`HEIGHT_SCALE` 4/44 against the camera's (1, 1, 1) direction) where the client weighs it as 35.4. A receiver dz priority units below the caster takes the shadow when it lies less than dz / 11 diagonal steps in front of the caster, against dz / 35.4 in the client, so a high caster reaches a little further forward onto low receivers than in the client. The same scale orders the sprites themselves, so the shadows stay consistent with what hides what; matching the client's ratio would mean changing the whole scene's depth, not the shadows. [Depth scale: Dynamapper and the KR client](DEPTH_SCALE_AND_KR_CLIENT.md) derives both scales, lists where they disagree and records the decision to keep ours. Dropping the caster's z from the shadow is not the client's rule. Tried on 2026-11-12, it sank every raised caster's shadow behind the floor it stands on. Walls and battlements lost their shadows on castle wall-walks, on upper floors and on terraces (Britain 1330,1620, 1334,1741 and 1496,1628), while at ground level only slivers changed.

**Stencil.** The shadow's +0x38 = 0x41 (bit 6) selects, in 0x00440a50, a cached `NiStencilProperty` with test ALWAYS, reference 1, pass REPLACE (0x00c933f0; ZERO for Roof or Bridge casters, 0x00c93410); mobiles' shadows take the same (0x00425180). Ordinary sprites test GREATER against reference 0xff and INCREMENT (0x00c933e8; ZERO for Roof or Bridge, 0x00c93414), and the circle of transparency (`UOStencilEffect`, 0x00416db0 and 0x00417480) writes 0xff where it cuts through, so the sprites it covers fail. That is the stencil's whole role: an ALWAYS test never rejects a shadow fragment, so it plays no part in which receivers take a shadow. Dynamapper has no circle of transparency and no stencil state to reproduce.

The wall shift (client world X -1, Z +1) changes the depth by V . (-1, 0, 1) = +0.0707, two diagonal steps further back, so the shadow's priority point moves with it: a wall's shadow sorts as if cast from the cell one tile west and one tile north (`client_shadow_logical_offset`), and its neighbours along the wall, one step back, stay in front of it.

### Filtering

The shadow descriptor's +0x38 is 0x41 (set at the end of the placement 0x00440e50). `UOSprite::BuildSpriteBatch` 0x004454c0 gives the sprite map BILERP unless bit 2 of that field is set, so the client samples every shadow bilinear, whatever the caster's own filter. Dynamapper's shadow pass therefore takes the client filter (`client_filtered_atlas_rect`) in every preset; until 2026-11-12 it sampled the atlas nearest, which stair-stepped the leaning edges.

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
