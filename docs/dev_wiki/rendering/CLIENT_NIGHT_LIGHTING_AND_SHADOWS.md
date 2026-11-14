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
- **Scan:** a sprite reaches up to about 27 world units across and its offsets up to 11.5, so the collector scans the statics beyond the visible chunks by the largest reach of the loaded lights, computed once and enlarged by the largest item light scale, and rescans only when the visible chunks, the centring, the light cover rule or the z limit change. It leaves out the lights a roof or floor covers (Lights under a roof or a floor, below).
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

### Lights under a roof or a floor

**The clients** draw their light sprites with no depth test, but cull the sprite of a covered light first (traced 2026-11-13, correcting the earlier reading that they show every glow over its roof):

- **No depth test.** KR `UOObjectComponentLighting::BuildLightBatch` 0x00438ee0 builds the batch's `NiZBufferProperty` with flags 0xf (0x00438f72) and clears z test and z write (bit 0 and bit 1: `AND word [EAX+0x18], 0xfffc` at 0x00438f7e). The Enhanced client's lighting component does the same at 0x00449bfe (in 0x00449b80, after the property's constructor 0x0041a470, vtable 0x00c9e06c). `UOLightingPostProcessEffect::Render` (KR 0x0041fe50, Enhanced 0x0042a360) draws the night map, then begins the light target group with clear mode 4, `CLEAR_ZBUFFER` (KR `FUN_004122e0(param_1[0x9b], 4)`, the renderer's `BeginUsingRenderTargetGroup`; Enhanced at 0x0042a582), and draws the light list (+0x134) into it. The scene's depth is not bound: a sprite that is drawn shows over whatever covers its emitter.
- **Gathering.** Every object with a lighting component (component type 3) builds its batch: the static chunk build (KR `StaticChunkBuildRequest_Execute` 0x004580c0 → 0x00458160) calls `BuildLightBatch` for each such static unconditionally, and the dynamic items, multis and mobiles do through 0x0055c750, 0x0056a6d0 and 0x0056c3f0 (Enhanced: 0x00449b80 from 0x005af590, 0x005b6be0, 0x005bbe30, 0x005db6f0, `InitFromDefinition` 0x005bdac0). The batch goes into the one light render group (KR 0x00c931b4, the type-5 slot of the per-type groups at 0x00c931a0; Enhanced 0x00e32e3c), which the render loop draws whole.
- **The cull.** Right after attaching the batch, `BuildLightBatch` calls the component's second vtable's slot 0 (KR vtable 0x00b52244, slot 0x004391d0, with the current cut; Enhanced vtable 0x00c9fe18, slot 0x00449f60), which sets or clears the batch's `APP_CULLED` flag (`+0xac` bit 0). KR calls it again for every light when the cut changes (0x0043c7d0 → 0x0043c770, over the component list at 0x00c0ead8). It reads the objects of the cell (x + 1, y + 1) through the facet's cell query (KR 0x0058a300, Enhanced 0x005df710: the cell's statics, then its dynamic items and multi parts), the same cover cell as ClassicUO, with no flag test on them.
  - **Enhanced** (0x00449f60): the light is culled when its own item is not drawn (0x0044ee40: its z at or above the z limit at 0x00dba01c, or a Roof, while a limit is set, whose vtable +0xd8 test fails), or when a drawn object of the cover cell sits in a higher band: `(z - land_z + 140) / 20`, both from the land z of the light's cell (0x005dcd50), truncated toward zero. The limit is 0x80 (none) unless something stands over the player at least 15 z above it (0x0044f090, a ClassicUO-like max z).
  - **KR** (0x004391d0, traced 2026-11-14): the same test on its 16 height layers. Each object's layer (0x0043c8a0, its position through vtable +0x28 → 0x0043c950) is `clamp((z - land_z + 140) / 20, 0, 15)` (0x0043c5b0, `(ecx + 0x8c) / 0x14` clamped to 0 and 0xf), with `land_z` the land z of the object's own cell: the light's from its cell, a cover object's from the cover cell (x + 1, y + 1; the query 0x0058a300 takes EAX = x + 1, EDI = y + 1 at 0x00439294 and 0x004392a1). That land z is byte +0x10 of the cell's 0x54-byte record (0x00587e50), which `UOFacet::BuildChunkFromSector` 0x0058d890 fills with the cell's averaged corners, ClassicUO's AverageZ: `(top + bottom) / 2` when the top-bottom difference is not larger than the left-right one, else `(left + right) / 2` (0x00587ad0 computes the same live); +0xc is the cell's own z. While a cell is not loaded 0x0043c950 falls back to the player's z. The Enhanced client reads the raw z (0x005dcd50, +0xc of its 0x58-byte record) instead; its average, minimum and maximum (0x005dce80, 0x005dcf80, 0x005dd060) are not used there.
  - **KR's cut** (`DAT_00c0a534`, 16 at start by 0x0043be20): `BuildLightBatch` reads it (0x0043901d) and slot 0 culls a light whose layer is above it, and does not count a cover object whose layer is above it: a light is culled when `layer(light) > cut`, or a cover-cell object has `layer(light) < layer(object) <= cut`. The per-frame scene update 0x00459ae0, when the player's x, y or z changed, sets the cut to `0x0043c9f0(player)`: 0x0043c350 looks for a ceiling, a static of the player's cell more than 14 z above the player (below the lowest one found, from 0x7f) without flags 0x20004 and not a Roof unless a Surface, or a Roof more than 14 z above the player in the cell (x + 1, y + 1); with one the cut is the player's own layer (0x0043c950 at the player's position; the ceiling's z is not used), else 16. When the cut changes it stores it (+0x4c) and calls 0x0043c7d0, which sets the cut, culls the scene's layer groups above it (12 object types × 16 groups at 0x00c92ea0 to 0x00c931a0) and re-culls every light (0x0043c770: slot 0 over the component list at 0x00c0ead8, under the critical section 0x00c933c8). Nothing re-culls the lights while the cut stays the same; a new batch is culled when it is built. 0x00459ae0 also culls the group at 0x00c931a0, the land (type 0, built by `TerrainChunkBuildRequest_Execute` 0x004589e1, never filed by layer), while the player stands below the average land z of the player's cell (0x00587ad0), under the viewer's flag +0x50, which its constructor 0x00459360 sets to 1 and nothing found clears: in a cave all land goes and the cave's statics stand on black. The layer groups take the statics and dynamic items (0x00458160 → 0x0043c8a0, groups 0x0043d3a0, 0x0043d2c0, 0x0043d070) and the multis (0x0056a6d0, all but one sprite kind, filed in the unlayered group 0x00c931b0); the mobiles are unlayered (0x0056c3f0). There is no roof flag in the cut: a roof goes only with its layer (traced 2026-11-14).
- So in a ten-storey tower seen from outside, each floor's lights are culled by the floor above them in their cover cell, and only the top floor's (and those on outer walls facing the camera) glow; inside, the floors above the player are cut with their lights.
- **How it differs from ClassicUO's.** ClassicUO counts statics at least 5 z above the light that are not Transparent; the clients count any object, Transparent too, by 20-z bands above the land: a cover 20 z or more above always culls, one 5 to 19 z above culls only across a band boundary, and one 1 to 4 z above can cull across a boundary. On map 1, of 51177 lights ClassicUO hides 5579 and the clients 5193: 5166 by both, 413 by ClassicUO alone, 27 by the clients alone (map 0: 50568 lights, 5159 and 4798, 4766 by both).
- **How KR's differs from the Enhanced client's**, without a cut: on map 1 KR hides 5713 (5153 with the Enhanced client, 560 alone, 40 that the Enhanced client hides alone); on map 0 5324 (4761 both, 563 KR alone, 37 the Enhanced client alone). Of KR's 560, 547 come from the light's cell's averaged land z differing from its own z, mostly mushrooms on slopes (446 "mushrooms", 78 "mushroom", 14 crystals, 6 windows); the 40 are mostly windows at a shore or a slope (26), such as Britain 1407,1594 to 1407,1609 at z 50: their cell's averaged land z (7, by the water) lifts them into the layer of the pavers at 70 in front (layer 9, counted from that cell's 30), where the raw z (-15) puts the pavers a band above them. KR draws those windows' glow over the pavers of the roof above them; KR-like keeps them hidden under the pavers (`floor_covers_light`, below). The clamp joins everything at 160 z or more above the land into the top layer, where nothing covers.

**ClassicUO** (a Classic reference, `GameScene.cs` and `GameSceneDrawingSorting.cs` of the read-only copy, line numbers as of 2026-11-13): lights go into a 2D target, added with no depth test, but `GameScene.AddLight` (line 402) does not add a light when the cell (x + 1, y + 1), the diagonal neighbour in front of the emitter, holds a static or multi that is not tiledata Transparent, is `AllowedToDraw`, and has `z >= emitter z + 5` and `z < _maxZ`. Only drawn objects add lights: a static at or above `_maxZ`, or a roof while `_noDrawRoofs`, fades out and is skipped before `AddLight` (lines 833 to 900).

**Dynamapper** handles covered lights in all four styles by the rule each style's preset group picks (shader window: Lights under Roofs): `light_cover = "uod"` (the default in all four), `"classicuo"` or `"clients"`. The rules act on the light sprites of KR-like and Enhanced (and the light on the ground taken from them) and on the light.mul decals of Classic and UOD-Painterly; the land and static local lights read the lights that are left. `"classicuo"` and `"clients"` hide a whole light (`art/light_cover.rs`, `static_light_is_hidden`); `"uod"` hides none for being covered and clips each glow per pixel. The light sprite scan rescans when the rule changes; the light.mul scan runs every frame.

- `"uod"` is Dynamapper's own rule, made for what the clients get wrong: their glows show through the floors and roofs that cover them, or a whole room's light goes with the cell in front of it, and with it the light a doorway or a window lets out. A light goes only with its item: at or above the view's z limit (or a drawn cave's, below). Each glow is then clipped to the statics its light reaches (`art/light_scene.rs`):
  - **The light scene.** After the opaque pass, the art's opaque and transparent batches draw again into an `Rgba16Uint` image the size of the view (`render_light_scene`, `light_scene.wesl`), with the opaque pass's depth bound: the fragment whose depth is the one drawn at its pixel writes the cell, the z and the class of its static; on a wall or a solid static the z is the height of the point of its face under the pixel, where the view line through the card crosses the face's plane (the card's own height overstates it by up to a cell's depth), so a floor over a light cuts the wall it meets. The land, the mobiles and anything else that is not a static leave 0. The class comes from the tile flags (`static_light_class_flags`): a wall (Wall), a solid block (Impassable), or a height static (anything else, and every Surface, Roof, Foliage, Transparent or Translucent static). Each fragment tests its depth first and samples the atlas only when the depth matches, so most fragments of the redraw end before any texture work.
  - **The clip.** The glows draw with `LightClipMaterial` (`light_clip.wesl`), Bevy's unlit `StandardMaterial` path behind a texel test (`light_reaches`). The light's cell, z and ceiling ride in each glow's `MeshTag` (`light_clip_tag`); the ceiling is the lowest floor or roof (Surface or Roof, not Transparent or NoDraw) at least `LIGHT_COVER_MIN_RISE` (5) z above the light in its own cell or its cover cell, below the z limit (`static_light_ceiling_z`). A glow lights the land always; a height static below `LIGHT_COVER_MIN_RISE` above the light; a wall or block from `LIGHT_COVER_MIN_RISE` under the ceiling down (its face height reads the top of a wall's thickness, drawn in front of its plane, a few z low), on the light's own cell, and on the faces turned toward the light: a wall on a cell's west edge faces east and is lit by the lights on its column or east of it, one on the north edge faces south; a block's south and east faces are lit from beyond them. The clip reads the texels `LIGHT_CLIP_EDGE_REACH` (2) pixels to each side too, and lights a pixel only when the light reaches all five: a filtered static's edge fades its coverage, and the transparent pass blends the fade over what lies behind without writing depth, so the light scene holds the floor or the land behind a roof's tile edges, which let the glow through in a grid of seams in Enhanced before. So a glow stops at the floor above it and at the camera-facing side of the walls between it and the camera, but goes on over the ground and floors at or below the light on both sides of a wall: a room's light reaches the street through its door and around its walls, and under an eave the light stays while the roof above it takes none of its glow. With `show_only_player_floor` and the player upstairs, a light downstairs does not light the player's floor (at least 5 z above it), and its spill on the ground outside stays.
  - **When they draw.** The light.mul decals draw in the world view's transparent pass after the light scene, with no depth test: the clip stands in for it. The light sprites belong to the light buffer camera, which renders before the world view: their pipelines draw nothing there (`LightClipMode::Deferred`), and the world view draws them into the light buffer once the light scene is done (`render_deferred_light_sprites`), before the client night composite reads it. In the first frames after a start a light sprite's indirect draw can lie past the end of that frame's indirect parameters buffer, which wgpu refuses and Bevy quits on; the deferred sprites then skip the frame (`indirect_parameters_fit`). The light scene runs only while a clipped glow is drawn.
  - **Why this way.** The clip needs, per pixel, what the glow lands on. The scene's depth alone gives a world position, not the static it belongs to nor its cell, so a wall's face or a floor's height above the light cannot be told from it. Looking the statics up per glow fragment, on the CPU or from a storage buffer, would repeat the lookup under every overlapping glow; one redraw of the scene writes it once per pixel, and each glow reads one texel. A shadow map per light would cost a pass per light (982 lights at zoom 6 in Britain).
  - **Cost** (2026-11-14, map 1, Britain 1407,1596 at night, 1280 x 800, art and land EC, GPU pass timings): the light scene takes 0.8 to 1.0 ms at zoom 1 in Classic (318 decals), KR-like and Enhanced (630 sprites; drawing them deferred 0.10 to 0.16 ms), and up to 1.6 ms in a run with the machine under other load; at zoom 6, 2.0 ms in Classic and 1.9 to 2.4 ms in KR-like (deferred sprites 0.17 to 0.25 ms). Classic at zoom 6 draws 982 decals (124 with `"classicuo"`) at 30 to 35 frames per second (37 to 40 with `"classicuo"`); KR-like at zoom 1 runs at 45 frames per second against 47.6 with `"classicuo"`, at zoom 6 at 18.6 against 20.3, in the same session. The light.mul scan used to read and unpack each light's mask every frame to sample its colour, which at 982 lights held the frame rate at 14; it now keeps each light's sampled colour (`StaticLightMaterialCache::mask_color_by_light`).
  - Classic and UOD-Painterly look nearly the same with `"uod"` as with `"classicuo"` where a roof covers the room: the decals of the lights under it now draw, but the roof is in front of the room and takes none of their glow. The difference shows where a covered light's glow reaches past its roof: the ground in front of a door, a porch, the street beside an eave.
  - The static local lights (off by default, `enable_static_art_local_lights`) are not clipped: they read the lights that are left, all of them with `"uod"`, and their falloff weighs z (below).
- `"classicuo"` is ClassicUO's rule (`classicuo_light_is_hidden`): a light is hidden when its cover cell holds a static that is neither Transparent nor NoDraw, at least `LIGHT_COVER_MIN_RISE` (5) z above the light and below the view's z limit. Dynamapper no longer keeps the lights under an eave lit with it (an exception it had until 2026-11-14, which brought 1882 of ClassicUO's 5579 hidden lights on map 1 back, 1590 of them windows): `"uod"` keeps them, with their glow off the roof.
- `"clients"` is the rule of the client the style follows (`LightCoverRule::of_style`, picked by `UniformState::light_cover_rule` from the active preset's group; no setting of its own). Enhanced, Classic and UOD-Painterly take the Enhanced client's (`client_light_is_hidden`): a light is hidden when it is at or above the z limit, or any static of its cover cell below the limit sits in a higher band, `(z - land_z + 140) / 20` from the land z of the light's cell (the light's own z while that land block is not loaded). KR-like takes KR's (`kr_light_is_hidden`): the layers are clamped to 0 to `KR_LIGHT_TOP_LAYER` (15), the light's counted from its cell's averaged land z and each cover static's from the cover cell's (`average_land_z` of the four corners, as the KR cell record holds it; while a corner's land block is not loaded, the light's own z for the light's cell and the light's land z for the cover cell, where KR takes the player's z). KR-like does not follow KR where its layers let a light show through a floor: a floor or roof (Surface or Roof, neither Transparent nor NoDraw) of the cover cell at least `LIGHT_COVER_MIN_RISE` (5) z above the light and below the z limit hides it too (`floor_covers_light`). KR's rule lets such a floor through when it sits in the light's layer, or the light's layer is lifted to it by the land under the two cells (the Britain windows below); the KR client then draws the glow over the floor, one of its bugs that Dynamapper does not reproduce (the user's KR screenshots of 2026-11-14 show glows lying on roofs, such as two on a thatched roof, one from a candle on its outer wall under the eave; where the other's light hangs could not be told). On map 1 the guard hides 158 more lights than KR's rule alone, 5871 in all (109 windows, 20 wall torches, 8 lava); the Enhanced rule takes it too (since 2026-11-14), 135 more, 5328 in all. Neither keeps the eave-hung lights, as in the clients.
- KR's cut is reproduced in KR-like when its `player_floor_rule` is `"kr"` (the default; shader window: Player's Floor Rule), with `show_only_player_floor` on: the z view then hides the layers above the player's (`scene/world/z_view.rs`, `compute_kr_z_view_cut`, `KrLayerCut`), and every light cover takes, for a light, the z from which its cell's layers are hidden (`ZViewCut::max_z_on_cell`, from the cell's averaged land z) as its z limit, where the other styles and `"classicuo"` take ClassicUO's max z. A light in a hidden layer goes with its item. Under `"uod"` the clip's ceiling (`static_light_ceiling_z`) is the lowest floor or roof below that z, and the statics of the hidden layers are not drawn, so they are not in the light scene either: the glow of a light of the player's floor reaches up to the walls the view leaves, and one of a floor below stays under the player's floor. Under `"clients"` a cover object counts only below that z, so with the KR rule a light is hidden as KR culls it, `layer(light) > cut`, or a cover object with `layer(light) < layer(object) <= cut`, and `floor_covers_light` still hides a light under a floor or roof of the player's layer or below. The cover cell's objects are measured against the light cell's z rather than their own cell's, which differs from KR only where the two cells' averaged land differs across a layer boundary. In every cover the floor or roof the view removes above the player no longer hides or clips the lights of the player's floor, while the lights of the floors below stay under the player's floor.
- A room's lights under a roof or an upper floor go (Britain 1407,1595: window lights at z 30 under boards at 50 and pavers at 70; the castle keep's sconces at 63 under the floor at 72). The open-air lava (map 0, 4622,3560), a torch on an outer wall facing the camera (map 1, 1514,1470) and a torch under a roof over its own cell whose front cell is open (map 1, 1482,1480) keep theirs.
- The light's own cell never counts, so its own wall, the item it sits on and a roof over its own cell do not cover it; the floor it stands on starts at or below it.
- A light on a north or west wall (its front cell inside the building) goes when the building has a roof; one on a south or east wall stays unless a roof or eave runs over the cell in front of it. With `"uod"` neither goes; the roof takes none of their glow.
- Multi parts cover as statics do, wherever the renderer draws them (`MultiCoverParts`): each scan reads the parts of the multis placed within `MULTI_PART_MAX_REACH` of its cover cells (`PlacedMultisRes`), the drawn parts or all with `show_hidden_multi_parts`, at the renderer's z (`expanded_multi_part_z`); the light sprite scan rescans when `show_hidden_multi_parts` changes. The scans do not gather the lights of multi parts. Map statics are never multis, whatever their graphic: the maps hold thousands of ordinary items at 0x4000 and above (map 0: 3706, map 1: 9469, map 5: 168922), and they draw and cover as the items they are. 0x4000 marked a multi only in the legacy 0x1A WorldItem packet (ServUO's `WorldItem`, Server/Network/Packets.cs; ClassicUO's `PacketHandlers.UpdateItem`), which Dynamapper does not support; ServUO's `TileMatrix.ReadStaticBlock` and ModernUO read the statics as plain items and keep multis as separate objects, and ModernUO's TileData.cs takes item ids up to 0xFFFF (`core/multis.rs`). A multi is drawn, and covers, only once a source places it by its multi id, such as a server's 0xF3 objects of type 2 or a placement file; none does yet, so no multi is drawn and the multi cover has not run on real multis.
- The rule takes the view's z limit (the z-view limit's `max_z`, `scene/world/z_view.rs`; `NO_Z_VIEW_LIMIT` while it is off): a static the limit stops drawing no longer covers, so a room's lights come back when its roof is cut, and a light at or above the limit goes with its item. Checked with `show_only_player_floor` in all three rules (2026-11-14, Britain 1409,1596 and the keep 1333,1597 on map 1, the player on the ground and the upper floor): the floor or roof right above the player, which the limit cuts, no longer covers the lights of the player's floor; the lights of the floors below stay covered by the player's floor and those between; and a roof that `hide_roofs` hides below the limit still covers, as no rule reads `hide_roofs` (from the code; these views hold no such roof). The light sprite scan keys its rescan on the limit, and reruns once more land loads when it read the land z of a cell whose block was not loaded yet (the stand-in z otherwise stayed until the visible chunks changed, so the first view after a start could hide or show lights by it).
- With "Caves: Drawn Over the Map" (`world_rendering.caves`, `scene/world/caves.rs`) a light in a cave cell drawn over the map takes the limit a player standing on that cave floor gets, floor + 16 (`CavesRes::light_max_z`, the lower of it and the z-view's): the land and rock above the cave no longer cover its lights, and a light above the cave ceiling goes with its item. The light sprite scan keys its rescan on the drawn caves too.
- The static local lights' falloff does not ignore z: the land shader and `static_local_light_rgba` measure the distance with world Y weighted 1.8, so a roof 20 z above a light is about 3.3 world units from it, outside most lights' radius.

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
