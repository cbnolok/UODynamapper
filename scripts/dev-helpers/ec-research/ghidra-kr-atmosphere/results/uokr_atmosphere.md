# Kingdom Reborn atmosphere: how UOKR.exe darkens and tints the scene

Program: `UOKR.exe` 2.53.0.2 (2008/05/27, CL 108899), SHA-256
`1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`. Cross-checked against `UOSA.exe`, SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`. Addresses are UOKR unless marked UOSA.

Every finding has a label:

- **confirmed**: read in the code (decompiled and checked against the assembly or the data);
- **probable**: inferred from the code and the data, with one step not read;
- **unknown**: not resolved.

Function names are the ones `NameAtmosphereCode.java` applied (see [the UOKR names](uokr_atmosphere_names.md) and
[the UOSA names](uosa_atmosphere_names.md)).

The research had three passes:

- The first pass covered the lighting post-process, the shadows and the terrain shade.
- The second pass covered sprites, filtering, terrain data, other darkening, normals, the reference images and
  side-by-side captures. It also corrected the first pass in five places; each one is marked **Corrected in the
  second pass**.
- The third pass ([Third pass](#third-pass)) compared the Enhanced Client with KR, traced the data sources for the
  packages, the light sprites and the exact sprite shadow geometry. Its corrections are marked **Corrected in the
  third pass**.

## Summary

The client's gloom does not come from a world light model, fog, tonemapping or colour grading. **UOKR.exe has
none of these.** At full daylight the lighting composite is exactly the identity. The daytime look therefore comes
from what is drawn into the scene and how it is sampled:

1. **The KR art itself.** The pre-rendered statics and the terrain textures are dark, olive and low in chroma.
   For example, the KR grass base `Grass_A` has a mean of (57, 79, 45).
2. **A world grunge on statics and items.** Every sprite whose tileart entry has shader map 1 is multiplied by
   the noise texture `01000045_noise` (mean 0.84, range 0.55 to 1.0). The noise is world-anchored, with one repeat
   every 16 tiles. This covers 75 % of the KR art and 99.5 % of the legacy art. It never brightens. **Corrected
   in the third pass:** light sprites are not grunged.
3. **Blotchy terrain.** Each material blends two base textures through a large coverage-noise mask (one repeat
   every 24 tiles for grass). The two bases differ in brightness and hue.
4. **A baked half-Lambert terrain shade** from one fixed light, per vertex with Gouraud interpolation. Flat
   ground gets 0.854; slopes run from 0.15 to 1.0.
5. **Plain bilinear filtering without mip levels**, at about 1.2 texels per pixel at 1024 px. The low-resolution
   grunge and mask textures are magnified 2.5 to 3.7 times. Together these give the soft, smeared, "painterly"
   texture.
6. **Sprite drop shadows.** Every static, item and foliage sprite casts a black copy of itself, leaning right by
   11.25 degrees from its ground point. The scene under it is multiplied by 0.75. Mobiles cast a 50 % black copy
   of their frame. See [P3.4](#p34-sprite-shadow-geometry).
7. **At night** the light level fades the scene towards a dark-blue, world-anchored **night map**. Light sprites
   (not grunged) brighten it again only where the night has darkened the scene.

The Enhanced Client runs the same composite and the same terrain shade. Its programmable sprite shader never
applies the grunge (confirmed), it draws Classic art by default, and only its mobiles cast shadows (see
[P3.1](#p31-the-enhanced-client-against-kr)).

## What makes the KR look (revised)

These shares are estimates for daytime (L = 0), judged from the formulas, the captures and the reference images.
They were not measured in the client.

| Ingredient | Effect | Share | Status |
|---|---|---|---|
| KR art palette (statics, terrain textures) | dark olive mid tones; lighting baked into the pre-rendered art | about 35 % | confirmed data |
| World grunge on statics and items | ×0.55-1.0 blotches, 16-tile period, the same pattern across neighbouring objects: "dirty", uneven | about 15 % | confirmed |
| Terrain two-base blend through coverage noise | large light and dark patches on the ground | about 15 % | confirmed |
| Terrain half-Lambert, per vertex, Gouraud | flat 0.854; triangle-faceted shading on hills; dark cliffs facing away | about 10 % (more in hills) | confirmed |
| Bilinear, no mips; planar terrain UV | soft, smeared texture; streaks on steep cliffs | about 10 % | confirmed |
| Sprite drop shadows: statics, items and foliage ×0.75 (overlaps do not stack), mobiles ×0.5 | dark sprite-shaped pools leaning right behind trees and walls | about 5 % | confirmed |
| Light level, night map, light sprites | none by day; dominant at night | 0 % by day | confirmed |
| Screenshot compression and downscaling | extra muddiness in the reference images only; not a client term | about 10 % of the impression the references give | probable |

## Frame order of operations

1. The 3D scene is drawn into the scene render target of `UOLightingPostProcessEffect` (+0x220, render-target
   group +0x240):
   - **terrain**: the layer blend times the baked vertex grey; EMBM environment layers on the bump layers (snow).
     See [§4](#4-terrain-shade-baked-half-lambert-one-fixed-light) and [P2.3](#p23-terrain-kr-ec-and-dynamapper).
   - **statics and items**: `UOSpriteShaderHLSL` (`UOSprite.vsh` / `UOSprite.psh` from `Shaders.uop`). It
     computes texture × hue lookup × grunge map. Vertex RGB is unused; vertex alpha scales alpha. **Corrected in
     the second pass:** the first pass assumed the fixed-function `MODULATE(texture, diffuse)`. See
     [P2.1](#p21-sprites).
   - **mobiles**: fixed-function, unlit white, no grunge (*confirmed* in the third pass). With `showShadows` on,
     each mobile also draws a 50 % black copy of its frame into the scene (see
     [P3.4](#p34-sprite-shadow-geometry)).
   - **particle effects**: rain, snow and spell effects.

   No lights, materials or fog take part. *Confirmed* for the terrain stages, the sprite shader and the absence of
   lights and fog.
2. `UOShadowPostProcessEffect::Render` (0x0043fe10):
   - it clears the screen-sized shadow buffer S to white;
   - it draws the static, item and foliage shadow sprites into S with `UOSpriteShaderShadow`
     (MODULATE(texture, diffuse), 0x00546e30), depth-tested against the scene;
   - it composites `scene * clamp(S + 1 - s)` (0x0043f9f0, `UOShadowShader` 0x00545e40).

   This runs only while `showShadows` is on (0x00c4a014). *Confirmed* for the formula. **Third pass:** the
   shadow effect is appended before the lighting effect (*confirmed*); that the effects run in that order is
   *probable*.
3. `UOLightingPostProcessEffect::Render` (0x0041fe50):
   - it draws the night map (or the night-sight map) into the light buffer B (+0x24c);
   - it draws the light sprites of the list +0x134 over B;
   - it draws the composite quad into the output (+0x100).

   *Confirmed*, except the blend of the light sprites into B. **Corrected in the second pass:**
   - 0x00856600 is a generic Gamebryo draw dispatch (it calls 0x008448a0, then the object's vtable +0xc), not a
     blend state;
   - the light sprite batch uses the shared alpha property 0x00c933ec (SRCALPHA / INVSRCALPHA) with z test and
     z write off, so the blend is *probably* a lerp, not additive.
4. The UI is drawn on top and is not affected (*probable*).
5. **Death effect**: only while the player is dead, `UODeathPostProcessEffect` converts the frame to grey
   (`UODeathShader` 0x00413d20: DOTPRODUCT3, then TFACTOR - CURRENT). *Confirmed* for the stages; the
   TFACTOR value is *unknown*.

## Ingredients

### 1. Light level, night map and gamma: the lighting composite (largest contribution at night)

**Formula** (*confirmed*). `UOLightingPostProcessEffect::BuildCompositeQuad` 0x0041f5c0 and
`UOLightingShader::SetupTextureStages` 0x005441c0:

```
stage 0:  BLENDDIFFUSEALPHA(DIFFUSE, B)      -> c * level + B * (1 - level)
stage 1:  MODULATE4X(CURRENT, scene)         -> 4 * (c * level + B * (1 - level)) * scene
stages 2, 3: disabled                        (shader map 1, the day map, is bound but never sampled)
output = clamp(4 * (c * level + B * (1 - level)) * scene, 0, 1)        per channel, LDR
diffuse = (c, c, c, level)
c = 0.25 + 0.2 * (gamma - 0.5)   floats 0x00b86fac = 0.25, 0x00b86f94 = 0.2, 0x00b4e048 = 0.5
```

**Light level** (*confirmed*):

- `UOAtmosphere::OnGlobalLightLevel` 0x0056e8a0 sets `level = 1 - L * 0.032258064` (that is,
  `1 - L / 31`) for a server level `L < 31`. `L = 0` is full daylight.
- Before the first packet the level is 0.75: `UOAtmosphere::InitPostProcessEffects` 0x0041ecd0, global
  0x00c4a018. The effect's constructor (0x0041f190) uses 0.5.
- House customisation forces full light (0x005fcc30, 0x006008b0).
- The light level is server data. Typical emulator cycles (for example ServUO's `LightCycle`) are about 0 by
  day, about 9 to 10 at night and 26 in dungeons. Check the server you target; the client does not decide this.

**Night sight** (*confirmed*): `UOAtmosphere::OnNightSight` 0x0056e940 → `SetNightSight` (+0x218). A weight of 0
draws the night map into B; any other value draws the night-sight map.

**Light buffers** (*confirmed* data; `TerrainTexture.uop`, 256x256; statistics of the decoded previews, entries 21 and 22 from `kr_composite_table.py`):

| Entry | File the exe names | Mean RGB (0..1) | p10 / p90 | Look |
|---|---|---|---|---|
| 20 | `Data\Textures\00000020_daymap.tga` | (0.261, 0.271, 0.307) | | soft grey-blue clouds; **unused** (stages 2 and 3 disabled) |
| 21 | `00000021_nightmap` | (0.012, 0.016, 0.152) | B 0.082 / 0.224, max 0.259 | dark navy with soft cloud blotches |
| 22 | `00000022_nightsightmap` | (0.260, 0.328, 0.345) | B 0.310 / 0.376 | even grey-cyan |

**Night-map placement.** `UONightMapShader::SetupTextureStages` 0x00544600 applies a texture transform:

- the scale is (f / 13, 0.75 f / 13);
- the transform is rotated -45 degrees;
- it is translated by the camera position.

**Corrected in the second pass:** f is no longer an open point. It is `UOWorldViewer` +0x6c, the half-width of the
orthographic frustum (*confirmed*):

- the viewer object at 0x00c9dc70 is built by `UOWorldViewer::UOWorldViewer` 0x00459360, which sets +0x6c = 13
  (0x00b4f2e4);
- `UOWorldViewer::SetDefaultFrustum` 0x0045b510 sets the frustum (-13, 13, 9.75, -9.75).

At the default zoom the scale is therefore (1, 0.75): one repeat of the night map spans the 26 x 19.5 world-unit
view. The constants 6 and 26 at 0x00b4f2f0 / 0x00b4f2ec are *probably* the zoom limits of +0x6c. The clouds are
anchored to the world (*probable*: the translation was read, not tested).

**Gamma** (*confirmed*):

- It is the user option `SystemData.Settings.Resolution.gamma` (UI `settingswindow.lua` 538/754, slider 0..1,
  TID 3000166).
- It is read and clamped to [0, 1] by 0x004769f0 into 0x00c93b20, then applied by
  `UOSettings::ApplyResolutionSettings` 0x0047e790 → `UOAtmosphere::SetGamma` 0x0041efe0 →
  `UOLightingPostProcessEffect::SetGamma` 0x0041f440 (+0x21c).
- The defaults routine 0x0047b660 resets it to **0.5** (0x0047b7be).
- At full light the gamma slider is a linear brightness multiplier of 0.6 (gamma 0) to 1.4 (gamma 1). At night
  only the `c * level` term scales.

**Scene multiplier per server level** (gamma 0.5, `output = clamp(multiplier * scene)`):

| L | level | night map, mean | night map, p10 | night map, p90 | night sight, mean |
|---|---|---|---|---|---|
| 0 (day) | 1.000 | (1.000, 1.000, 1.000) | (1.000, 1.000, 1.000) | (1.000, 1.000, 1.000) | (1.000, 1.000, 1.000) |
| start | 0.750 | (0.762, 0.766, 0.902) | (0.750, 0.750, 0.832) | (0.789, 0.801, 0.974) | (1.010, 1.078, 1.095) |
| 5 | 0.839 | (0.846, 0.849, 0.937) | (0.839, 0.839, 0.892) | (0.864, 0.872, 0.983) | (1.007, 1.050, 1.061) |
| 10 | 0.677 | (0.693, 0.699, 0.874) | (0.677, 0.677, 0.784) | (0.728, 0.743, 0.966) | (1.013, 1.100, 1.123) |
| 15 | 0.516 | (0.539, 0.548, 0.810) | (0.516, 0.516, 0.676) | (0.592, 0.615, 0.949) | (1.020, 1.150, 1.184) |
| 20 | 0.355 | (0.386, 0.397, 0.747) | (0.355, 0.355, 0.567) | (0.456, 0.486, 0.932) | (1.026, 1.200, 1.245) |
| 26 (dungeon) | 0.161 | (0.201, 0.216, 0.671) | (0.161, 0.161, 0.438) | (0.293, 0.332, 0.911) | (1.034, 1.260, 1.319) |
| 30 | 0.032 | (0.078, 0.096, 0.621) | (0.032, 0.032, 0.351) | (0.184, 0.230, 0.898) | (1.039, 1.300, 1.368) |

**Contribution.** None by day. At night it is the dominant effect:

- red and green fall linearly with the level;
- blue is held at about 0.6 to 0.9 by the night map, so the night is a strong blue cast. The blue/red ratio is
  1.26 at L = 10 and 3.3 at L = 26;
- the night map's clouds modulate blue by about ±0.25 at dungeon levels.

Night sight brings the scene back to about daylight with a cyan tint (slightly above 1 in green and blue).

### 2. Light sprites (local lights)

- **What is confirmed:**
  - the light sprites are drawn into B, the same buffer as the night map, before the composite;
  - in the composite, B is weighted by `4 * (1 - level)`. A light therefore has **no effect at full daylight**
    and grows as the night deepens. At L = 26 a light texel of value v multiplies the scene by about
    `0.16 + 3.36 v`, so the scene reaches full brightness (the multiplier clamps at 1) at v ≈ 0.25.
  - Second pass: `UOObjectComponentLighting::PlaceLightSprite` 0x00438a80 places each light sprite with the
    common sprite placement 0x004422b0. `UOObjectComponentLighting::BuildLightBatch` 0x00438ee0 builds the batch
    with the common sprite batch builder 0x004454c0, with an NiZBufferProperty that turns z test and z write off.
    The batch goes to render group 0x00c931b4.
  - Second pass, data: the tileart light sources (source block 3, 599 entries: 462 `*_light_generic`, 39
    `*_light_window`, then crystals, mushrooms, candles, sconces and others) all carry the grunge map as
    shader map 1, under `UOSpriteShader`. **Corrected in the third pass:** the light material exposes only map 0,
    so that grunge is never bound (see [P3.3](#p33-light-sprites)).
- **The textures** (*confirmed* data):
  - the light textures are `TerrainTexture.uop` entries 1-17 (DXT5; entry 16 is `circle_of_trans`, used by the
    circle of transparency, not by lights: see [P2.4](#p24-other-darkening-and-tint));
  - tileart block 3, map 0 names them (`Data\Textures\000000NN_<name>.tga`): `light_generic` on 462 items,
    `light_window` on 39, with light scale 2.0 on windows (see `ghidra-tileart`).
- **Probable:** the light sprites lerp over the night map (the batch's SRCALPHA / INVSRCALPHA alpha property
  0x00c933ec) rather than add.
- **Third pass** (*confirmed*): no grunge and no hue; the colour is the texture's own RGBA, and the size is the
  texture times the tileart light scale. See [P3.3](#p33-light-sprites).

### 3. Projected sprite shadows

**Formula** (*confirmed*): `out = scene * clamp(S + 1 - s)`. S = 1 outside shadows and is lowered by the shadow
sprites. The intensity s comes from three places:

- constructor 0x0043f740: 0.4;
- `InitPostProcessEffects` sets **0.25** (0x00c4a024);
- `UOAtmosphere::SetShadowIntensity` 0x0041f0c0 changes it (debug keys only, ±0.05).

**Contribution.** Full shadow is ×0.75. It is the same at every time of day and is multiplied again by the light
composite at night. The shadow shape is the sprite's own alpha, through `UOSpriteShaderShadow`: no blur pass and
no length or softness parameters. The `showShadows` setting toggles it. **Third pass:** its default is on (KR
turns it off with less than 32 MB of video memory). The exact geometry, the casters and the separate 50 % mobile
shadow are in [P3.4](#p34-sprite-shadow-geometry).

### 4. Terrain shade: baked half-Lambert, one fixed light

**Formula** (*confirmed*). `UOTerrain::BuildTerrainMesh` 0x0044b7f0 writes four vertices per cell:

```
vertex colour = (g, g, g, 1),   g = (N . D + 1) * 0.5
D = (0, -0.7071, -0.7071)       constant at 0x00c0f3b8 (read only, never written)
N = UOTerrain::ComputeVertexNormal 0x004488a0
```

`ComputeVertexNormal` averages three normalised triangle normals from the corner heights; the height axis is y
negated; one UO z unit is 0.12856 world units (0x00b86f38). The terrain stages modulate the layer blend by this
diffuse (UODefaultTerrainLayer, see `ghidra-terrain-layers` and [P2.3](#p23-terrain-kr-ec-and-dynamapper)).

**Light direction** (*probable*: the axis mapping was read from the normal and position code, not tested in a
capture). D points 45 degrees above the horizon, towards decreasing cell row, which is UO north, the upper right of
the screen.

**Values:**

| slope | facing the light | facing east or west | facing away |
|---|---|---|---|
| flat | 0.854 | 0.854 | 0.854 |
| 22.5 | 0.962 | 0.827 | 0.691 |
| 45 | 1.000 | 0.750 | 0.500 |
| 67.5 | 0.962 | 0.635 | 0.309 |
| 90 | 0.854 | 0.500 | 0.146 |

**Contribution:**

- all terrain is at most 1.0 and flat ground loses 15 %;
- slopes facing south (towards the viewer) darken strongly; cliffs facing away fall to 0.15 to 0.3;
- there is no ambient floor other than the half-Lambert's own 0.5 at 90 degrees from the light;
- the shade is evaluated per vertex and interpolated across two triangles per cell, so hills look faceted.

### 5. Sprites (statics, items, mobiles)

**Corrected in the second pass.** The first pass said "texture × diffuse, grunge unknown". The details are in
[P2.1](#p21-sprites). In short:

- **Unlit** (*confirmed* for KR): no NiDirectionalLight, NiAmbientLight, NiFogProperty or NiMaterialProperty
  is linked. The RTTI scan found only a NiPointLight type descriptor, with no vtable.
- Statics and items use `UOSpriteShaderHLSL`: texture, hue lookup, then `rgb *= grunge` when the tileart entry
  has shader map 1. Vertex RGB is not used (*confirmed*). Light sprites use the same shader with one map only:
  no hue and no grunge (third pass).
- Mobiles use the fixed-function path without grunge, with a white diffuse (*confirmed* in the third pass).
- `LightingShader` (vtable 0x00b692bc) exists for the Mythic library's own 3D lights; no UO world path was
  found that uses it.

### 6. Terrain normal and specular use

- UOBumpMapTerrainLayer (vtable 0x00b52bac, 0x00549e60) is EMBM: an environment texture perturbed by a bump map
  and added. **Corrected in the second pass:** it is the snow material (material 8: `Snow_A`/`Snow_B`, noise
  mask, `01000013_water_alpha` repetition 12 as the bump, `01000014_cube0` repetition 2 as the environment), the
  same in KR and EC. Water, lava, acid, stars and swamp use UOWaterTerrainLayer instead.
- There is no normal-mapped diffuse lighting and no specular. *Confirmed* by the absence of lights.

### 7. Bloom, tonemapping, colour grading, vignette, fog: not used

- **Fog** (*confirmed*): there is no NiFogProperty class. The post-process composites set FOGENABLE 0. The
  linked `figleaf` sky library, which has a fog term, is never called (see [P2.4](#p24-other-darkening-and-tint)).
- **Bloom / HDRBloom / Color grading** (*probable*: not used):
  - the Mythic factory `PostProcessEffectsManager::CreateEffectByType` 0x00700b20 (3 = Bloom, 4 = HDRBloom,
    5 = Color) has one caller, a helper at 0x006f1300 (jump table 0x006f133c);
  - that helper has no code reference and no pointer in the image (a byte search for `00 13 6f 00` finds none);
  - even if reached, the `Shaders.uop` BrightPass constants (LumaScale 0.1, Luminance 2.0, BloomScale 2.0,
    fMiddleGray 0.18, fWhiteCutoff 0.8) give zero bloom for LDR input.
  - The claim in `dynamapper/assets/shaders/visual_atmosphere_implementation_plan.md` that KR "crushes ambient
    light by ~90%" through this tonemap is therefore not supported.
- **Vignette, desaturation, colour grading** (*confirmed* absent from the UO post-process classes): the UO
  effects are Lighting, Shadow and Death only (RTTI scan of `*PostProcessEffect`). Desaturation happens only in
  the death effect.

### Shipped defaults and their sources

| Value | Default | Source |
|---|---|---|
| light level | 0.75 until the first server packet, then `1 - L / 31` | exe 0x0041ecd0, 0x0056e8a0; server |
| night sight | 0 | exe; server (0x0056e940) |
| gamma | 0.5 (slider 0..1) | exe defaults 0x0047b660; user settings `SystemData.Settings.Resolution.gamma` |
| shadow intensity | 0.25 | exe 0x0041ecd0 (debug keys change it) |
| shadows on | on (off with less than 32 MB of video memory); user setting `showShadows` | exe video defaults 0x0047bac0 (third pass) |
| night map, night-sight map, day map | `TerrainTexture.uop` 21, 22, 20 | UOP data |
| terrain light D | (0, -0.7071, -0.7071) | exe constant 0x00c0f3b8 |
| light textures | `TerrainTexture.uop` 1-17, named by tileart block 3 map 0, scaled by the tileart light scale | UOP data |
| grunge map | `01000045_noise.tga`, repetition 16, UV set 1, shader map 1 | tileart data; `UOSprite.psh` |
| sprite filter | bilinear, no mip levels (nearest when the tileart flag +0x38 bit 2 is set; probably nearest for the hue map) | exe 0x004454c0; DDS data |
| view width | 26 world units (frustum half-width 13), about 18.4 tiles across | exe 0x00459360, 0x0045b510 |

There is no registry or ini value for any of these. The `UserInterface` Lua/XML only exposes the gamma slider
and the shadow checkbox.

## KR and the Enhanced Client

| Piece | UOKR.exe | UOSA.exe | Status |
|---|---|---|---|
| lighting composite | 0x0041f5c0 / 0x005441c0 | `BuildCompositeQuad` 0x00429950 (`c = 0.25 + 0.2 (gamma - 0.5)`, alpha = level +0x21c, gamma +0x224), `UOLightingShader::SetupTextureStages` 0x0058f7f0 (same stages: BLENDDIFFUSEALPHA, MODULATE4X, stages 2-3 disabled) | identical, *confirmed* |
| day map loaded, unused | yes | yes (0x0042a360 loads `00000020_daymap.tga`) | identical, *confirmed* |
| terrain half-Lambert | 0x0044b7f0, D at 0x00c0f3b8 | `BuildTerrainMesh` 0x00461ae0, D at 0x00dc18b8 = (0, -0.7071, -0.7071), `(N . D + 1) * 0.5`, normals 0x00460c70 | identical, *confirmed* |
| object layout | light +0x214, gamma +0x21c | light +0x21c, gamma +0x224 | EC adds 8 bytes before |
| sprite shader | `UOSpriteShaderHLSL::BindSamplers` 0x005477d0 binds shader map 1 to GrungeSampler and sets HAS_GRUNGE_TEX | `UOSpriteShaderHLSL::BindSamplers` 0x005940e0 never reads shader map 1 and sets only HAS_HUE_TEX and HAS_HUEMASK_TEX | **KR grunges statics, EC does not**, *confirmed* |
| sprite shader choice | library table at 0x00c18f60: UOSpriteShader listed first with ps_2_0 / vs_1_1, then without a requirement | `UOShaderLibrary::RegisterShaders` 0x00c0d610: UOSpriteShaderHLSL (ps_2_0 / vs_1_1), then UOSpriteShaderFFP | same rule; the HLSL path on any card with ps_2_0, *probable* |
| terrain materials | 100 material names | 107 names (adds `coollava`, `eodon clouds`, `rock hole blend`, two moss pavers, `void`, `void_destruction`) | the same records except grass and forest; 32 of the textures KR references hold other images in EC (see [P3.2](#p32-data-sources)), *confirmed* data |
| art | KR art (tileart block 0) | Classic art unless `UseNewArt` | see the memory note on EC art |
| sprite shadows (third pass) | statics, items, foliage and mobiles | mobiles only; the item placement 0x00457bf0 is unreferenced | **differs**, see [P3.1](#p31-the-enhanced-client-against-kr) |

**Probable:** the EC's brighter, cleaner look does not come from a different post-process. It comes from its art
(Classic by default) and from the missing grunge. Server light levels apply to both.

## Second pass

### P2.1 Sprites

**KR statics and items** (*confirmed*):

- `UOSpriteShaderHLSL` (vtable 0x00b61264) is set up by `Initialize` 0x005476c0. It loads
  `LoadPixelShader` 0x00547010 (`UOSprite.psh`) and `LoadVertexShader` 0x00546fa0 (`UOSprite.vsh`), both from
  `Shaders.uop`, and declares five sampler stages.
- `UOSprite.vsh` passes three UV sets:
  - the sprite UV through `spriteTransform`;
  - the world grunge UV through `grungeTransform`;
  - the hue coordinate.

  It also passes the vertex colour.
- `UOSprite.psh` has three permutations: `HAS_HUE_TEX`, `HAS_HUEMASK_TEX` and `HAS_GRUNGE_TEX`. It computes:

  ```
  colour = tex(SpriteSampler, uv0)
  if hued: colour.rgb = hue lookup (optionally only where the hue mask alpha > 0.5)
  if HAS_GRUNGE_TEX: colour.rgb *= tex(GrungeSampler, uv1).rgb
  colour.a *= vertex alpha          // "no need to use diffuse RGB since the pixel is already hued"
  ```
- `UOSpriteShaderHLSL::BindSamplers` 0x005477d0 (vtable slot 7) maps the textures and sets the flags:

  | Source | Sampler | Flag |
  |---|---|---|
  | shader map 0 | SpriteSampler (+0x11c) | |
  | base map (the hue texture) | HueSampler (+0x13c) | HAS_HUE_TEX (+0x19c) |
  | shader map 2 | HueMaskSampler (+0x15c) | HAS_HUEMASK_TEX (+0x1bc) |
  | shader map 1 | GrungeSampler (+0x17c) | HAS_GRUNGE_TEX (+0x1dc) |
- `UOSprite::BuildSpriteBatch` 0x004454c0 builds each batch. The grunge map is the tileart entry's shader map 1.
  Its UV is the ground-plane projection of each quad corner plus the object's x / z, divided by the repetition
  16, so the grunge is anchored to the world: neighbouring objects share one continuous pattern. The quad itself
  comes from `UOSprite::BuildSpriteQuad` 0x00442430; placement is `UOSprite::PlaceSprite` 0x004422b0 (*probable*
  name).
- **Which art has grunge** (`kr_tileart_grunge_census.py` over the KR tileart dump):

  | Tileart source block | Entries | With `01000045_noise` (repetition 16, UV set 1) |
  |---|---|---|
  | 0, KR art | 12402 | 9256 (75 %) |
  | 1, legacy art | 16384 | 16298 |
  | 3, light sprites | 599 | 599 (never bound: see [P3.3](#p33-light-sprites)) |

  In block 0, 289 more entries reference the white 4x4 `20000000_empty_grunge`. The 2857 without a grunge map
  are mostly tree bark and branches, `blank`, banners, cave entrances and some roofs.
- **The grunge texture** (`Texture.uop` `build/worldart/01000045.dds`): 256x256 DXT5, one level. Its mean is
  0.840 and its range 0.55 to 1.0, close to grey (chroma about 1/255). It only darkens. With a 16-tile repeat it
  holds 16 texels per tile edge. At the client's scale (about 39 screen pixels per world unit, one tile edge, at
  1024 px) it is magnified about 2.5 times into soft blotches.
- **Hue path** (*confirmed* in the shader): a hued pixel's RGB is replaced by the hue texture entry that the
  average of its RGB channels selects (shader constants `c_fHueTextureHeight` 1024 and `c_fNumHuePixels` 255);
  the hue alpha multiplies the alpha. The hue row comes from the third UV set, which
  `UOSprite::SetSpriteHueColour` 0x00441ce0 writes (*probable* name). No per-vertex colour reaches the RGB.
- **Diffuse and vertex darkening:** there are none. Vertex RGB is ignored and vertex alpha only fades.
- **Alpha** (*confirmed*): `UOSprite::GetSpriteAlphaProperty` 0x00440910 returns one of two shared properties:
  - 0x00c933ec: blend SRCALPHA / INVSRCALPHA with alpha test GREATER;
  - 0x00c9340c: alpha test only.

  In the batch builder, one path rewrites the flags with `& 0xf3fe | 0x1200` (alpha test only).
  `GetVertexColourProperty` 0x004408a0 supplies the shared vertex-colour property.

**KR mobiles** (*probable*): the mobile shape builder 0x004278b0 (referenced from two vtables, 0x00b51d04 and
0x00b627cc) uses fixed-function texturing with a white colour (1, 1, 1, 1 at 0x00bf85b8) and no shader map 1. So
mobiles are unlit and have no grunge. **Confirmed in the third pass**, in both clients (see
[P3.1](#p31-the-enhanced-client-against-kr)).

**Shaders.uop vs fixed function:**

- Both clients list `UOSpriteShader` twice in the shader library: first with the requirement ps_2_0 / vs_1_1
  (0xffff0200 / 0xfffe0101), then the fixed-function version with no requirement.
- UOSA names the two entries: `UOShaderLibrary::RegisterShaders` 0x00c0d610 registers UOSpriteShaderHLSL, then
  UOSpriteShaderFFP. The KR table at 0x00c18f60 has the same layout.
- Any card with ps_2_0 therefore gets the HLSL path (*probable*: the selection loop was not read).

**EC** (*confirmed* for the HLSL path):

- `UOSpriteShaderHLSL::BindSamplers` 0x005940e0 binds shader map 0, the base map and shader map 2 only. It
  never reads shader map 1 and never sets a grunge flag, so EC statics are not grunged even where the data names
  the noise.
- The EC fixed-function fallback multiplies shader map 1 (*probable*; it runs only without ps_2_0).
- The hue path and the unused vertex RGB are the same as KR's.

### P2.2 Filtering and scale

- **Filter** (*confirmed*, `UOSprite::BuildSpriteBatch` 0x004454c0):
  - The sprite map gets filter BILERP (flags 0x100) unless bit 2 of the tileart field +0x38 is set; then it gets
    NEAREST.
  - The grunge map is always BILERP.
  - A third map, *probably* the hue texture, is set to NEAREST.
  - No map is created with TRILERP or a mip filter.
  - Clamp versus wrap follows the repetition (1.0 gives clamp unless bit 3 of +0x38 is set).
- **Mip levels** (*confirmed* data): the sprite, grunge and terrain DDS files have one level each (`dds_info.py`
  over the extracted textures). Gamebryo's default of not generating mips at load is *probable*.
- **Terrain:** `UOTerrain::LoadTerrainLayerTexture` 0x0044b180 loads the material sources (*probable*: bilinear,
  single level, as above).
- **LOD bias and anisotropy:** none set (*probable*: no sampler-state call for either was found; device
  defaults are bias 0 and anisotropy 1).
- **Scale** (*confirmed*):
  - the camera is orthographic, half-width 13 (`UOWorldViewer` +0x6c), so the view is 26 world units (about 18.4
    tiles) across at any resolution;
  - KR art is authored at 46.67 px per world unit (1 / 0.021427), about 66 px per tile diagonal, so about 1213 art
    pixels span the view.

  | Window width | Art texels per pixel | Effect |
  |---|---|---|
  | 1024 | 1.19 | slight minification without mips: fine detail averages out or aliases, never sharp |
  | 1280 | 0.95 | almost 1:1 |
  | larger | below 1 | magnified |

  The scale is non-integer, so bilinear filtering always mixes neighbouring texels.
- **Low-resolution maps are magnified:**

  | Texture | Texels per tile | Magnification at 1024 px |
  |---|---|---|
  | grunge | 16 | 2.5 times |
  | grass mask | 10.7 | 3.7 times |
  | `Grass_A` | 51 | about 1.3 times minified |

**Contribution:** the soft, smeared look of the reference images is bilinear filtering of minified art,
magnified noise and grunge, and the planar terrain UV on slopes (see P2.3). There is no sharpening or blur pass.

### P2.3 Terrain: KR, EC and Dynamapper

**Client stages** (*confirmed*):

- `UODefaultTerrainLayer::SetupFourStages` 0x00549c30 blends two base textures by the coverage mask's alpha,
  then multiplies by the vertex diffuse (the half-Lambert grey). `SetupTwoStages` 0x00549d90 uses one base. KR
  and EC are the same (UOSA `SetupFourStages` 0x005974f0).
- The mesh has four vertices per cell, one quad per cell. Each cell is split along the (x+1, y)-(x, y+1)
  diagonal and Gouraud-shaded.
- UV0 is the world (x, -y): a planar projection, so steep cliffs stretch their textures into streaks.
- `UOStaticTerrainShader::SetupTextureStages` 0x00548d10 drives the static terrain pieces (the lava and swamp
  tileart).

**KR vs EC TerrainDefinition** (*confirmed* data; a scratch comparison of the two material dumps):

- KR has 191 material entries (100 distinct names) and EC 201 (107 names). KR's 191 entries are EC's first 191,
  with the same names at the same indices. EC adds `coollava`, `eodon clouds`, `rock hole blend`, two moss
  pavers, `void` and `void_destruction`.
- Of the 191 shared entries only two differ in their sources: grass (entry 1) uses different texture files and
  repetitions, and forest (entry 4) different repetitions. **Corrected in the third pass:** the records match
  otherwise, but 32 of the texture ids they reference hold different images in the two clients (see
  [P3.2](#p32-data-sources)).

  | Material | KR | EC |
  |---|---|---|
  | grass | `Grass_A` rep 10, `Grass_B` rep 5, `noise_alpha` rep 24 | `Grass_C` rep 10, `Grass_B` rep 4, `noise_alpha` rep 32 |
  | forest | `Forest_A` / `Forest_B` rep 5 | rep 4 |

- KR `Grass_A` (512x512) has a mean of (57, 79, 45) and chroma 35: dark green. `Grass_B` has a mean of
  (102, 99, 29): yellow. The mask decides which one shows, in blotches about 24 tiles across in KR and 32 in EC.
- The second terrain field is 3 in KR and 1 in EC for grass, jungle, sand and rock. Overall it is higher in KR for
  13 materials and lower for 6. Its meaning is *unknown* (probably the blend priority or a coverage exponent).
- **Corrected in the second pass:** the directional cliff materials ("Sand Cliff E-W / NW-SE / N-S / NE-SW",
  "Dirt Cliff ...") exist in both clients. They are not a KR difference.

**Dynamapper** (read from `dynamapper/assets/shaders/world/land/`):

| Piece | Dynamapper | Client | Match |
|---|---|---|---|
| material blend (`sampling.wesl` `sample_ec_material_albedo`, `ec_material_mask_value`) | the mask alpha weights item 0, as in `SetupFourStages` | the same | **matches** |
| coverage (`coverage.wesl`) | evaluates the original per-material 32x32 ownership masks, up to 17 layers | the client's material passes | matches in intent |
| transitions (`transitions.wesl`) | exact boundary weights for EC tiles | the client's material passes | matches in intent |
| `cc_transitions.wesl` | smoothstep blending between cell centres, Classic textures only | none | not involved for EC data |
| sampling | world-planar UV (`ec_world_uv`; the repeat is the slot's `texture_stretch`, or width / 44 without one) | world-planar UV, repeat from the material | same projection |
| filter | `pixel_art_uv_iq_grad` remap before `textureSampleGrad`, even with `enable_linear_filtering` 1 | plain bilinear | **differs**: the remap sharpens texel edges |
| shading (`normals.wesl`) | per-pixel normals: mode 0 central differences, mode 1 bicubic, optional bent normals; then the land lighting model | per-vertex normal (three triangle normals) and a per-vertex half-Lambert | **differs** |
| data | the installed packages, *probably* the EC TerrainDefinition (`Grass_C`, 32-tile mask) | KR `Grass_A`, 24-tile mask | data difference, not a code one |

### P2.4 Other darkening and tint

| Candidate | Finding | Status |
|---|---|---|
| Weather | `UORainEffect::LoadParticles` 0x00418420 loads `00070006_Rain.nif` and `00070002_HvyRainPuddles.nif` into render group 7; the puddles get a stencil property with flags 0x4180. `UOSnowEffect::AttachToRenderGroup` 0x00418cd0 attaches `00070000_Snow.nif`. Particles only. | confirmed |
| Light level from weather | `UOAtmosphere::SetLightLevel` 0x0041eef0 has four callers: `OnGlobalLightLevel` 0x0056e8fb, house customisation 0x005fcc30 and 0x006008b0 (forces 1.0), and the debug key handler 0x004619a0. That handler uses cases 0xc / 0xd for ±DAT_00b86e88 and 0xe for night sight +0.2. No weather code darkens the scene. | confirmed |
| Sky, clouds, fog | The `figleaf` sky library is linked: `worldLighting`, `cloudLighting`, `halo`, `bellCurve`, `moon`, `fog`, `worldAmbientBase`, `worldAmbientSky`, `cloudlayer1/2`, near 0x00b7d1a0. `figleaf::ParseSkyDefinition` 0x0073025b has one caller, 0x00730fde, which has no code or data reference, and the strings have no references. So there is no sky, no cloud shadow and no fog. | probable (dead code) |
| Roof and interior | `UOStencilEffect` is the **circle of transparency**: `Update` 0x00417de0 places it at the camera focus; `BuildFirstCircleShape` 0x00416db0 and `BuildSecondCircleShape` 0x00417480 use `Data\Textures\00000016_circle_of_trans.tga` with alpha 0x12ec and stencil 0x4180. It does not darken. The only roof code found is `HousingSetRoofZOffset`. | confirmed (circle); probable (no interior darkening) |
| Region and personal light | Only the global light level is applied; no personal-light handler was found. | probable |
| Per-object light | Light sprites (§2), at night only. | confirmed |
| Other UOAtmosphere members | Shadow intensity and the debug steps above; nothing else touches colour. | confirmed |
| Light-sprite blend (0x00856600) | 0x00856600 is a generic Gamebryo draw dispatch, not a blend. The blend is the batch alpha property: probably a SRCALPHA / INVSRCALPHA lerp over the night map, with z off. | confirmed (dispatch); probable (lerp) |

### P2.5 Normals

**The client** (*confirmed*):

- terrain: per-vertex normals for the baked half-Lambert (§4), nothing per pixel;
- snow: EMBM bump (UOBumpMapTerrainLayer). The bump perturbs an environment lookup; it does not light the diffuse;
- water-like layers: UOWaterTerrainLayer (cube-map environment, scrolling);
- sprites: no normal input in `UOSprite.psh`; mobiles are unlit.

**Dynamapper term by term:**

| Dynamapper term | File | Client counterpart |
|---|---|---|
| `art_fake_normal` (side slope, bevel), highlight, rim, specular on art | `world/art/shading.wesl` | none |
| geometric land normal (mode 0) | `world/land/normals.wesl` | partial: the client computes a normal from the corner heights, but per vertex, then Gouraud |
| bicubic land normal (mode 1), bent normals | `world/land/normals.wesl` | none |
| land normal map (`enable_normal_maps`, strength 0.45, `sample_ec_material_normal_world`) | `world/land/sampling.wesl` | none for diffuse lighting; only the snow EMBM bump and the water layers use extra maps |
| wrapped Lambert with wrap 0.5 (mode 2) | `world/land/shading.wesl` | the same curve as the client's half-Lambert, but per pixel and from `BAKED_GLOBAL_LIGHT` (-1, 2.5, -1) instead of D |

### P2.6 Reference images (`docs/images/kr_reference/`)

Statistics from `kr_reference_stats.py`; "dark B/R" is the blue/red ratio of the darker half of the pixels.

| Image | Size | Mean RGB | Luma p10 / p50 / p90 | B/R, dark B/R | Chroma | Likely level | Ingredients |
|---|---|---|---|---|---|---|---|
| `254602…16x9` | 576x323 | (87, 79, 70) | 43 / 73 / 130 | 0.80, 0.74 | 23 | L 0 | house interior; blue-purple wall art (the same walls are in the 2006 IGN shot), grunge on walls and floor, fireplace effect, tree shadows |
| `2712200822561889526` | 1000x750 | (80, 80, 69) | 36 / 74 / 129 | 0.86, 0.89 | 21 | L 0 | keep roof; grunged stone; dark half-Lambert cliff with streaky planar UV |
| `350px-Lakeshirekr` | 350x349 | (63, 68, 38) | 44 / 62 / 83 | 0.60, 0.58 | 33 | L 0 to 5 | KR grass blend (dark `Grass_A` with yellow `Grass_B` blotches); a campfire halo needs L > 0 unless it is the fire effect's own sprite |
| `6490` | 1152x864 | (87, 82, 77) | 17 / 65 / 160 | 0.88, 0.67 | 22 | L 0 | closed-beta snow (EMBM layer), dark cliffs facing away (p10 17) |
| `UO_KR_screen.webp` | 800x600 | (93, 86, 71) | 33 / 83 / 145 | 0.76, 0.79 | 30 | L 0 | town, dirt blend, tree shadows (art plus ×0.75), lightning effect |
| `gallery_74` | 993x768 | (122, 105, 86) | 52 / 105 / 168 | 0.71, 0.72 | 40 | L 0 | the brightest: sandstone walls and stairs with visible grunge |
| `images.jpg` | 259x194 | (99, 83, 64) | 44 / 79 / 142 | 0.64, 0.67 | 36 | L 0 to 5 | small; torch pools |
| `sanctuary-warzone` | 500x368 | (85, 88, 74) | 41 / 73 / 152 | 0.87, 0.85 | 23 | L 0 (low L possible) | dark stone ground, many monsters |
| 2006 IGN interior | 1024x768 | (85, 82, 76) | 36 / 74 / 142 | 0.89, 0.90 | 29 | L 0 (low L possible) | purple and blue wall and floor art, soft sprite shadows |

**Corrected in the second pass.** The first pass read several of these as dusk or night scenes with the night
composite. None of them shows the night map's blue cast:

- at L 10 the multiplier raises B/R by 1.26 and cuts luma by about 30 %;
- the dark halves here have B/R 0.58 to 0.90, which is the art's own warm balance.

Most are therefore daylight, with an identity composite. A low level (L 10 or less) cannot be excluded for the
three coolest images. The blue and purple areas are wall and floor art, not light.

All of them are JPEG or WebP and most are downscaled, which adds blur and blocking that the client does not
produce.

### P2.7 Captures

**Rig** (`capture-rig/`, following `renderer-reference-validation` and `kde-x11-preview`):

- **Isolation:**
  - each run has its own working directory with copies of `assets/` and `cache/`;
  - the packages come from `/home/claudio/test/uddp/`;
  - the KR-like preset `kr.afternoon` is overridden only in the run's cache copy of `land.toml`;
  - the user's saved settings were never touched, and no code was changed.
- **Run conditions:**
  - window 1024x768 at zoom 0.79, which shows the client's 18.4 tiles across;
  - every run on Desktop 4 under `flock .../preview.lock` and `lowprio.sh`, driven through
    `DYNAMAPPER_CONTROL_SOCKET`;
  - the first capture of each run was discarded.
- **Runs** (each reported `placement_verified: true` and `cleanup_complete: true`):

  | Variant | PID | Settings |
  |---|---|---|
  | a | 5429 | the preset as shipped |
  | b | 6020 | every term the client lacks switched off (see `CLIENT_TERMS_OFF` in `rig.py`) |
  | c | 6601 | b plus projected shadows at strength 0.25 and softness 0.05, the nearest stand-in for the client's hard ×0.75 shadow |

  Variant b's control status confirmed the overrides (`global_lighting` 1.0, `normal_mode` 0, grunge, grime,
  wash, fog and the art terms off).
- **Views:**
  - open slopes (5650, 2950);
  - town, Britain (1434, 1699);
  - forest, Yew (620, 1020);
  - cliff and mountain, Covetous hills (2460, 880).
- **Sheet:** `/home/claudio/rust-target/uodynamapper-scratch/kr-atmosphere/caps/kr_atmosphere_sheet.png`, built
  by `sheet.py` (rows are views, columns a / b / c). It is not committed because it shows game art.

**Statistics** (debug overlay band excluded):

| View | Variant | Mean RGB | Luma p10 / p50 / p90 | B/R | Chroma |
|---|---|---|---|---|---|
| slopes | a | (98, 86, 32) | 42 / 74 / 131 | 0.33 | 66 |
| slopes | b | (102, 91, 67) | 44 / 80 / 146 | 0.66 | 36 |
| town | a | (60, 62, 37) | 37 / 57 / 83 | 0.62 | 31 |
| town | b | (64, 63, 49) | 37 / 60 / 89 | 0.77 | 19 |
| forest | a | (78, 78, 9) | 42 / 69 / 99 | 0.12 | 79 |
| forest | b | (83, 81, 39) | 44 / 74 / 111 | 0.47 | 49 |
| cliff | a | (89, 87, 73) | 59 / 87 / 108 | 0.82 | 19 |
| cliff | b | (93, 88, 79) | 61 / 90 / 113 | 0.85 | 14 |
| references | | | 17-52 / 62-105 / 83-168 | 0.60-0.89 | 21-40 |

Variant c matches b to within one level everywhere (forest p50 73 against 74).

**What the captures show:**

- **The shipped preset is far too yellow.**
  - Its grading, temperature and warm light colour crush blue: forest B/R 0.12 and slopes 0.33, against 0.58 or
    more in every reference.
  - They also raise chroma to 66-79, against 21-40 in the references.
  - With the client-missing terms off (b), blue comes back (0.47-0.85) and chroma falls to 14-49. Brightness
    barely changes (p50 +3 to +6).
  - The yellow cast is a Dynamapper term, not part of the KR look.
- **b is closer in colour but too clean.**
  - Statics have no grunge blotches.
  - The grass is a bright, even yellow-green rather than KR's dark olive with blotches. The packages carry the EC
    grass (*probable*).
  - Texel edges look crisper than the client's plain bilinear.
- **The projected-shadow stand-in (c) barely registers.** The client's sprite-shaped ×0.75 shadow is much more
  visible in the references (tree pools in `UO_KR_screen`).

**Client terms that settings cannot reproduce:**

1. The client grunge on statics: texture `01000045`, multiply 0.55-1.0, 16-tile world period, on 75 % of KR art
   and none on land. Dynamapper's grunge is procedural (`visual_grunge_image(128)` plus fbm in
   `postprocess/grunge.wesl`). It has a period of about 40 tiles and a range of 0.48-1.16 at the KR gain, and it
   also runs on land.
2. Plain bilinear without mips. The land and art samplers apply pixel-art UV remaps (`pixel_art_uv_iq_grad`,
   `pixel_art_atlas_uv_iq`) even with linear filtering on.
3. The light direction D (0, -0.7071, -0.7071). `BAKED_GLOBAL_LIGHT` (-1, 2.5, -1) is a constant.
4. Per-vertex Gouraud shading over the client's triangle split. Every normal mode is per pixel.
5. The hard, sprite-shaped ×0.75 shadow.
6. The KR terrain and art data (`Grass_A`, the 24-tile mask, KR art), if the packages carry EC data.
7. The night composite and light-sprite weighting (not in play for the afternoon views).

## Third pass

The third pass compared the Enhanced Client with KR, traced the data sources for the packages, the light-sprite
path and the sprite shadow geometry. It used Ghidra and the client data only, with no captures.

- The GUI's MCP bridge stopped answering during the pass. The rest of the code was read with headless Ghidra on
  a read-only copy of the project (`KrShadowHeadlessDecompile20261107.java`) and with `objdump`.
- The data comparisons are `kr_ec_data_compare.py` over scratch extractions of both clients.

### P3.1 The Enhanced Client against KR

| Piece | UOKR.exe | UOSA.exe | Status |
|---|---|---|---|
| shadow post-process | ctor 0x0043f740 (0.4), init 0x0041ecd0 sets 0.25, Render 0x0043fe10, `UOShadowShader` 0x00545e40 | ctor 0x00456060 (0.4 at 0x00c9b2bc), init 0x00429070 sets 0.25 (0x00e13e90), `SetIntensity` 0x004561d0 (clamp 0..1), Render 0x00456880, composite 0x00456350, `UOShadowShader` 0x005921b0 | identical, *confirmed* |
| shadow composite | stage 0 `ADD(TEXTURE S, DIFFUSE 1 - s)`, alpha `SELECTARG1 TEXTURE`; stage 1 `MODULATE(CURRENT, scene)`: `scene * clamp(S + 1 - s)`, no blur | the same | identical, *confirmed* |
| `showShadows` default | **on**: the video defaults 0x0047bac0 set 0x00c93b30 = 1, off with less than 32 MB of video memory; init copies it to 0x00c4a014 | **on**: `UOSettings::SetVideoDefaults` 0x00493c20 sets 0x00e33588 = 1, off with less than 128 MB or a pixel shader major version (0x00e31e20) below 2; init copies it to 0x00e13e84 | both on, *confirmed* |
| static, item and foliage shadows | placed by 0x00440e50 from the sprite and foliage components (see [P3.4](#p34-sprite-shadow-geometry)) | the same placement exists at 0x00457bf0 with the same constants, but nothing references it (no call, jump or pointer). The EC sprite and foliage components (vtables 0x00c9fc6c and 0x00c9fce4; updates 0x00445060, 0x004460e0, 0x00446c10) never read 0x00e13e84 | **KR only**: *confirmed* that the EC placement is unreferenced; *probable* that nothing else fills S, so the EC shadow buffer stays white |
| mobile shadows | builder 0x004278b0 (gate 0x00427db4), update 0x00425220 | builder 0x00435140 (gate 0x00435a19), update 0x00432110, the same constants | identical, *confirmed* |
| mobiles | `MobAnimTriShape`, vertex colour white (0x00bf85b8), `UOSpriteShaderMob` with the fixed-function stages 0x005465c0 (`MODULATE(texture, diffuse)`), no shader map 1, render group 0x00c931b8 | the same; white at 0x00dde130, stages `UOSpriteShaderFFP::SetupTextureStages` 0x00592a00, render group 0x00e32e40 | identical, *confirmed* (pass 2 had *probable*) |
| sprite shader (statics, items) | `UOSprite.psh`: hue index = average of RGB, hue mask threshold 0.5, grunge | `uosprite.psh`: no grunge; hue index = red channel; the hue mask **lerps** by its alpha; a vertex-colour tint: when the vertex RGB is not white, `rgb = colour.rgb * lerp(0.1, 0.9, texture red)` | differs, *confirmed* (shader sources). The tint is off with the usual white vertex colour; its caller was not read |
| EC-only post-process | none | `UOFadePostProcessEffect` (ctor 0x00424a20, Render 0x004250d0 with `UOFadeShader`, factory 0x00425800): the event-driven screen fade (effect type 4) | not a steady tint, *probable* |
| EC-only shaders | none | `uoeffects.psh/.vsh` (effects) and `uodeath.psh` (monochrome by luminance 0.2125, 0.7154, 0.0721); two unnamed entries with vertex-colour and texture permutations (`Shaders.uop` hashes 0x0530e223fe06a3a1 and 0x3574e0b3a13aef7f) | KR's death effect is the fixed-function `UODeathShader`; *confirmed* data |
| `postprocess.psh/.vsh` | library effects, unreached | byte-identical, unreached | identical, *confirmed* |
| lighting composite, terrain shade, night maps | | | identical (see [KR and the Enhanced Client](#kr-and-the-enhanced-client)), *confirmed* |
| light sprites | `PlaceLightSprite` 0x00438a80 | not read in UOSA; the composite that consumes them is identical | |

**What differs between a faithful KR preset and a faithful EC preset:**

1. **Art:** KR art (tileart block 0) against Classic art by default.
2. **Grunge:** KR multiplies statics and items by its noise map; EC never does.
3. **Static, item and foliage shadows:** KR only. Both draw the 50 % mobile shadow.
4. **Terrain data:** KR's grass and forest records and 32 texture ids with different images (see
   [P3.2](#p32-data-sources)).
5. **Hue maths:** for hued items only (average RGB and a hard mask in KR; red channel and a soft mask in EC).
6. **Light data:** light texture 17 and the offsets of 18 light items differ; EC has 1040 light items against KR's 599.
7. **EC only:** the vertex-colour tint mode, the fade effect and the shader-based death effect. These are event
   effects, not part of the steady look.

Everything else that shapes the steady look is the same: the lighting composite, the night and night-sight maps,
the terrain half-Lambert and its light direction, the shadow composite, and the mobile draw path and shadow.

### P3.2 Data sources

`KR` is `UO - Old KR/`, `EC` is `_Ultima Online Enhanced/`. All comparisons are byte comparisons of the UOP entries
(SHA-256), plus a luma correlation for the images that differ.

**(a) The grunge noise** (*confirmed*). EC's `Texture.uop` ships `build/worldart/01000045.dds`, but it is **not
KR's image**:

- EC: mean 0.898; KR: mean 0.840;
- luma correlation 0.213, so it is a different pattern, not a re-encoding;
- `tex_aux_ec.uddp` therefore holds EC's version. Pack KR's.

**(b) The per-item grunge reference** (*confirmed* data). Items whose block carries `01000045` as shader map 1:

| Tileart block | KR | EC (same ids) | Both | EC only | KR only | EC, all items |
|---|---|---|---|---|---|---|
| 0, KR art | 9257 | 9265 | 9222 | 43 | 35 | 11211 |
| 1, legacy art | 16298 | 16298 | 16298 | 0 | 0 | 36254 |
| 2, TileArtEnhanced | none (EC only) | 16384 | | | | 36589 |
| 3, light | 599 | 601 | 597 | 4 (2188, 3511, 11520, 11696) | 2 (7575, 7576) | 1040 |

- For block 0 the two clients agree on 16306 of the 16384 item ids (99.5 %); 16240 ids have identical block-0
  references.
- **Decoder:** `lib/uocf/src/enhanced/metadata/tileart.rs` keeps every reference:
  `TileArtEntry.texture_vector[block].texture_items[i]` (`TaeTexture` / `TaeTextureImage`, about lines 413-438).
  The item's texture name comes through `name_string_off`; `texture_stretch` holds the repetition (16) and
  `uv_set` the UV set (1).
- **Tilemeta:** `lib/udd-assets/src/metadata/tilemeta.rs` stores each reference as a `TileMetaItemTextureRef`
  (lines 397-417) in `metadata/item_texture_refs.v4.bin`, with `texture_id` 1000045, `block_index`,
  `item_index` 1, `texture_stretch` 16, `uv_set` and `stable_role` `Noise` (5, from `SPRITE_ROLES`).
- So the flag is already in the packages and needs **no format change**: an item is grunged when its art block has
  a ref with `item_index` 1 and `texture_id` 1000045. Block 3 refs are tagged `Noise` too (`LIGHT_ROLES`), but the
  client never binds them (see [P3.3](#p33-light-sprites)), so a renderer must ignore them.
- Packages built from EC tileart are 99.5 % right for KR. For an exact KR set, build them from KR's tileart.

**(c) The light buffers and light textures** (*confirmed*). `TerrainTexture.uop` `build/terraintexture/%08d.dds`:

| Entries | KR against EC |
|---|---|
| 1-15 (light textures) | byte-identical |
| 16 (`circle_of_trans`) | differs |
| 17 (`GoldGlow_Treasure`, a light texture) | differs |
| 20, 21, 22 (day, night, night-sight map) | byte-identical |
| 23-35, 37-40 | EC only |

So the night, night-sight and day maps can come from either client.

**(d) The KR terrain materials** (*confirmed* data).

- The records are in `TerrainDefinition.uop`, with names from `string_dictionary.uop`:
  - grass (entry 1): `Grass_A` 2000010 repetition 10, `Grass_B` 2000011 repetition 5, mask `noise_alpha` 1000003
    repetition 24;
  - forest (entry 4): `Forest_A` / `Forest_B` 2000040 / 2000041, repetition 5.
- The textures are in `Texture.uop` as `build/worldart/<id>.dds`.
- **EC does not ship KR's `Grass_A`.** EC's 2000010 is `Grass_C` (luma correlation 0.008), and no EC entry holds
  KR's `Grass_A` bytes.
- **Corrected in the third pass:** the two records are not the only terrain difference. Of the texture ids that
  KR's TerrainDefinition references, 32 hold a different image in EC under the same id, mostly with the same file
  name. The full comparison (`terrain-textures`) gives:
  - identical: 166;
  - EC only: 8;
  - the same picture, re-encoded or re-graded: 13 (one of them, 2000103, is referenced by EC only);
  - a different picture: 20.

  | KR ids that differ | Where KR's bytes exist in EC |
  |---|---|
  | `Forest_A/B` 2000040/41, `Rock_A/B` 2000060/61, `Sand_A/B` 2000070/71, `Cave_Floor_A/B` 2000130/31, `Stone_Tan_Light_A/B` 2000280/81, `Stone_Tan_Dark_A/B` 2000290/91, `Dirt_Cliff_EW_A/B` 2000580/81, `Dirt_Rocks_A/B` 2001540/41, `Road` 2001900 (17) | byte-identical at id + 1000000 (3000040 and so on) |
  | `noise_alpha` 1000003 (correlation 1.000), `UO_dirt_02_256` 1000006, `cube3` 1000017, `Grass_A/B` 2000010/11, `Dirt_A/B` 2000020/21, `Jungle_A/B` 2000030/31, `Cobblestone_A/B` 2000090/91, `Dirt_Cliff_NWtoSE_A/B` 2000590/91, `Dirt_Cliff_NS_A/B` 2000600/01 (15) | KR only |

- Some differences are large: KR's `Dirt_A` has luma 70.5 against EC's 55.2, and the KR dirt cliffs are about 20
  levels brighter (55.8 against 35.8) with the same picture.

**What to pack from where:**

| Package item | Source | Status |
|---|---|---|
| grunge texture | KR `Texture.uop` `build/worldart/01000045.dds` (EC's differs) | confirmed |
| per-item grunge flag | already in tilemeta (`item_texture_refs.v4.bin`, ref `item_index` 1, `texture_id` 1000045); EC tileart agrees on 99.5 % of the items, KR tileart is exact | confirmed |
| night, night-sight, day maps | `TerrainTexture.uop` 21, 22, 20, either client | confirmed |
| light textures | `TerrainTexture.uop` 1-15 from either client; 17 from KR | confirmed |
| light scale and offsets | tileart (`light_scale`, `light_offset_x/y`; KR and EC agree on 585 of 603 light items); no package stores them today | confirmed data |
| KR terrain | KR `TerrainDefinition.uop`, plus KR `Texture.uop` for the 32 differing ids: 17 are also in EC at id + 1000000, 15 exist only in KR | confirmed data |

### P3.3 Light sprites

**Placement** (*confirmed*, `UOObjectComponentLighting::PlaceLightSprite` 0x00438a80):

- The texture comes from `LightingTextureFileLocation` 0x00438680: an object override (vtable +0x38) or the
  definition (vtable +0x60). That is tileart block 3, map 0, named `Data\Textures\000000NN_<name>.tga`; these names
  are the `TerrainTexture.uop` entries NN (*probable*: the name match was checked, the lookup was not traced).
- The material is a `UOShaderFilenameLight` (ctor 0x004387d0, vtable 0x00b522d4) with the shader name
  `UOSpriteShader` (0x00438930). Its map count (0x004388e0) is **1** and its map (0x004388f0) is the texture with
  repetition 1.0.
- `UOSprite::BuildSpriteBatch` 0x004454c0 binds maps 1 and up only to the material's count. **So the light sprites
  get no grunge**, although all 599 KR light items list the noise as map 1 (*confirmed*). This corrects the first
  pass ("probable: grunged").
- **Colour:** the descriptor defaults (0x00441890) are white (1, 1, 1, 1) with hue 0, and 0x00442890 writes no
  colour. The light colour is the texture's own RGBA (*confirmed*).
- **Size:** descriptor +0x3c is the definition's +0x6c, the tileart `light_scale`; +8 / +0xc are the texture's
  width and height. The quad is the texture's pixels times the scale, built by `UOSprite::BuildSpriteQuad`
  0x00442430 like a static.
- **Anchor:** descriptor +0x10 = 22 - width / 2 and +0x14 = 0, in pixels: centred on the 44-pixel tile (*probable*
  reading of the anchor fields).
- **Mode:** descriptor +0x38 = 3.
- **Position:** `UOSprite::PlaceSprite` 0x004422b0 at the object's position, plus the camera transform (0x0083aab0)
  of the two tileart light offsets, added to +0x60..+0x68.
  - Which offset is up and which is right, and whether they are scaled, was not resolved.
  - KR values are mostly 0 to -5. For example 2576 has (x -5, y -4) in KR and (0, -4.5) in EC.

**Batch** (*confirmed*): `BuildLightBatch` 0x00438ee0 turns z test and z write off and sends the batch to render
group 0x00c931b4, which `UOLightingPostProcessEffect::Render` draws into B. The blend is the shared alpha property
0x00c933ec (SRCALPHA / INVSRCALPHA), so the lights *probably* lerp over the night map.

**Data** (*confirmed*):

| Id (`TerrainTexture.uop`) | Name | KR items | Alpha-weighted mean RGB | Size |
|---|---|---|---|---|
| 1 | `light_generic` | 462 | (192, 162, 93); alpha at most 115 | 256x256 |
| 2 | `light_post` | 3 | (192, 162, 93); alpha at most 255 | 256x256 |
| 3, 4 | `light_window` | 19, 20 | (192, 162, 93); alpha at most 147 | 256x256 |
| 6 | `MushroomA` | 16 | (255, 31, 31) | 256x256 |
| 7 | `CandleA` | 14 | (209, 206, 153) | 256x256 |
| 8, 9 | `SconceA`, `SconceB` | 13, 8 | (157, 147, 57) | 256x256 |
| 11, 12, 13 | `crystal` | 10, 9, 4 | (81, 160, 113) | 128x128 |
| 14 | `blob` | 1 | (74, 203, 101) | 128x128 |
| 15 | `rune` | 5 | (106, 225, 238) | 128x128 |
| 17 | `GoldGlow_Treasure` | 15 | white, alpha mean 2.8, at most 56 | |

- KR light scales: 2.0 on 454 items, 1.0 on 72, 1.5 on 39, 1.6 on 24, 0.75 on 5, 5.0 on 4, 1.25 on 1. On the
  same ids, EC has six fewer at 1.5 and eight more at 2.0.
- The light fields follow `body_type` in the raw tileart record, at offset 0x23 (KR, version 3) or 0x25 (EC,
  version 4), as `<fIff` (scale, unused field +0x70, offset x, offset y).
- KR and EC agree on fields and texture for 585 of the 603 items that have a light in either client.
- In this repository only the decoder (`tileart.rs`, `light_scale`, `light_offset_x`, `light_offset_y`) and the
  tileart editor GUI read these fields; no package stores them. `world_lights.uddp` holds the Classic `light.mul`
  shapes (light id, flags, width, height), not the TerrainTexture lights.

### P3.4 Sprite shadow geometry

**Who casts** (*confirmed*, KR):

- The shadow placement 0x00440e50 (`obj`, `mode`; not named in Ghidra) is called from three places, each gated on
  `showShadows` (0x00c4a014):

  | Caller | Component | Vtable slot | Gate | Call |
  |---|---|---|---|---|
  | 0x00435120 | `UOObjectComponentSprite` (vtable 0x00b5203c): statics and items (multi parts *probable*) | +0x20 | 0x004353b2 | 0x004353cd |
  | 0x00435630 | shared by both components | +0x50 | 0x00435693 | 0x00435749 |
  | 0x00436320 | `UOObjectComponentFoliage` (vtable 0x00b520b4) | +0x20 | 0x0043661e | 0x00436638 |

- `mode` is the component's +0x24, 1 by default (ctors 0x00434de0 and 0x00436160). Mode 0 casts nothing. What
  writes other values is *unknown*.
- No shadow when the tileart flags (`obj` +0x34) have any of 0x10004200:
  - Surface 0x200;
  - Roof 0x10000000;
  - bit 14, 0x4000. Our decoder calls it `ArticleA`, but in KR's 32-bit flags it is EC's `Rotated` (bit 34): 1593
    of the 1604 KR bit-14 items are EC `Rotated` items, and UOSA's (unreferenced) placement tests bit 34 where KR
    tests bit 14. So rotated flat art casts no shadow.
- EC's version adds `NoShadow` (bit 36); KR has no such flag.
- There is no size limit.
- Mobiles cast a separate quad (below).

**The shadow sprite** (*confirmed*):

- It is a 0xfc-byte sprite descriptor (defaults 0x00441890), copied from the caster's own descriptor by 0x00441b80:
  the same texture, UVs, pixel rectangle, anchor and scale.
- +0xf8 = 1 marks it as a shadow. `UOSprite::BuildSpriteBatch` reads that at 0x00445658 and binds
  `UOSpriteShaderShadow` (0x00b51740) at 0x00445f23.
- Colour (+0x20..+0x2c) is (0, 0, 0, 1); +0x38 = 0x41 (its z-buffer bits 0x30 are clear, so the default z
  property applies).

**Geometry** (*confirmed* from the decompile and the assembly). The quad lies in the camera plane, like every
sprite. Camera space is Gamebryo's: X is the view direction (depth), Y up, Z right; that +Z is screen right is
*probable*.

```
desc       the caster's descriptor: pixel rectangle (x0, y0, x1, y1) at +0x00..+0x0c, anchor offsets (ax, ay) at
           +0x10 / +0x14, scale s at +0x3c, position P at +0x54..+0x5c
K  = 0.021427422   0x00b87024, world units per art pixel
Zu = 0.12856454    0x00b86f38 (= 6 K), world units per UO z unit
θ  = 0.19634375    0x00b87170 (= π / 16, 11.25 degrees)
ε  = 1e-4          0x00b42f48
Cam                the camera rotation matrix, [[0x00c9dc70] + 0x68] + 0x2c

h  = obj.z (+0x4c) - G          G = 0x00587770(map 0x00c9dc7c, position, z): the land z (capped at z) or the top of a
                                surface or bridge object in the cell whose top is at most z + 4, whichever is higher
W  = ceil((|x1 - x0| + 1) * s) * K            ceil is 0x004405f0
H  = ceil((|y1 - y0| + 1) * s) * K
r0 = ax * s * K
u0 = -ay * s * K + h * Zu
u1 = u0 + H
P' = P - Cam * (0, h * Zu, 0)                 the pivot: the caster's anchor lowered to the ground

Rot(r, u) = (r cos θ + u sin θ,  u cos θ - r sin θ)       0x0083a770 builds R, 0x0083ab10 applies R^T
C(r, u)   = P' + Cam * (ε, u, r)                          ε puts the quad just behind the caster

TL = C(Rot(r0, u1))
TR = TL + Cam * (0, 0, W)                     rotated top-left, then W along the unrotated right axis
if h == 0 and mode != 4:                      0x00839e50 resets R to the identity after the top corners
    BL = C(r0, u0),          BR = C(r0 + W, u0)
else:
    BL = C(Rot(r0, u0)),     BR = C(Rot(r0 + W, u0))
```

- The top edge stays horizontal and moves right by `u1 sin θ + r0 (cos θ - 1)` and down by
  `u1 (1 - cos θ) + r0 sin θ`. The quad is a parallelogram leaning **right**, with the caster's full height and
  width: the length is the sprite's height and the scale is the sprite's own.
- The direction is **fixed**. It does not depend on the terrain light D, the time of day or the light level.
- **Wall branch:** for modes 1-4 with the Wall flag (+0x34 & 0x10), or mode 3:
  - u0 and u1 are both lowered by 1.05 (0x00b8716c) before the corners;
  - after the camera transform, every corner gets world x -1.0 and world z +1.0 (0x00b4e8bc);
  - with the terrain's axes (x = UO x, z = UO y), that is one tile west and one tile south, one tile width to the
    screen left (*probable*).
- Modes outside 1-4 take the default branch; modes 1, 2 and 4 the same without the wall shift. Mode 4 keeps the
  bottom rotated at h = 0.

**Drawing into S** (*confirmed* stages, *probable* blend):

- `UOSpriteShaderShadow::SetupTextureStages` 0x00546e30: colour `MODULATE(TEXTURE, DIFFUSE)` = black, alpha
  `MODULATE(TEXTURE, DIFFUSE)` = the sprite's alpha; the blend comes from the batch's alpha property; fog off.
- With a black source and an INVSRCALPHA destination, every shadow does `S = S * (1 - a)`.
- The composite `scene * clamp(S + 1 - s)` with s = 0.25 floors at 0.75: **overlapping shadows do not darken
  further.**

**The shadow buffer** (*confirmed* unless noted):

- 0x0043f940 creates the effect's targets at the back buffer's size (+0xf0 / +0xf4, from 0x006f02e0), retrying at
  1/2, 1/3 and so on if that fails. S (+0x264) is therefore **screen-sized**.
- 0x006f0560 attaches the back buffer's depth-stencil (+0xfc) to every post-process target group, so the shadow
  quads are depth-tested against the scene drawn before them (*probable* that the shadow batch tests z). The
  caster hides its own shadow (the quad is ε behind it), and the shadow lands only on what lies behind the
  caster's plane.
- `Render` 0x0043fe10 clears S to (1, 1, 1, 1), draws the list +0x134 (count +0x13a) through 0x00856600 into
  +0x264, then composites with `UOShadowShader`. That the item shadows reach list +0x134 is *probable*.
- **Order:** `InitPostProcessEffects` 0x0041ecd0 appends the shadow effect (0x00c4a02c, ctor at 0x0041edd3) before
  the lighting effect (0x00c4a010, ctor at 0x0041ee8e). The append order is *confirmed*; that the effects run in
  array order is *probable*. So the shadow composite comes **before** the lighting composite.

**Mobile shadow** (*confirmed*, KR and EC alike):

- Built by 0x004278b0 when `showShadows` is on (0x00427db4):
  - a `MobAnimTriShapeShadow` (vtable 0x00b51e6c) with 4 vertices, colour (0, 0, 0, 0.5) (0x00b4e048) and
    indices 0, 2, 1, 1, 2, 3;
  - `NiAlphaProperty` flags 0x12e1: blend on, source ONE, destination INVSRCALPHA, alpha test GREATER 10;
  - `NiZBufferProperty` test on, write off; `NiVertexColorProperty`; `UOSpriteShaderShadow`.
- It is attached to the mobile render group 0x00c931b8, which the main scene pass 0x00459e00 draws. **The mobile
  shadow goes into the scene, not into S:** `scene * (1 - 0.5 a)` with a the frame's alpha, independent of the
  shadow intensity.
- Update 0x00425220 (vtable +0x88), per frame:

  ```
  (left, top, right, bottom) = UOAnimations::GetLayerBounds, pixels;   sK = mobile scale (+0x98) * K
  r_l = 0.71 + left * sK     r_r = 0.71 + right * sK               0.71 = 0x00b871bc
  u_t = 0.71 - top * sK      u_b = 0.71 - bottom * sK
  R   = rotation by -θ (0x00b871c0) applied as R v: the same lean right as Rot above
  TL  = Cam * Rot(r_l, u_t)      TR = Cam * Rot(r_r, u_t)          only the top corners are rotated
  BL  = Cam * (0, u_b, r_l)      BR = Cam * (0, u_b, r_r)
  every corner += position (+0x8c..+0x94) + ε * V - Cam * (0, (z - ground z (+0xac)) * Zu * Zu, 0)
  V   = the vector at [0x00c9dc70] + 0xc (probably the view direction)
  UV  = (0, 0) to ((right - left) / texture width, (bottom - top) / texture height) of the frame atlas
  ```

  - The height factor is squared (`Zu * Zu`) in the assembly, probably a client slip.
  - The ground z comes from 0x00427180.
  - The top edge tilts, unlike the item shadow, whose top edge stays horizontal.
- EC: builder 0x00435140 (gate 0x00435a19), vtable 0x00c9fa54, 0.5 at 0x00c95604, the same flags, update
  0x00432110 with 0.71 (0x00d053c8), -θ (0x00d053cc) and the same squared factor.

**EC item shadows:** none (see [P3.1](#p31-the-enhanced-client-against-kr)). The unreferenced placement
0x00457bf0 uses the same constants (K 0x00d051d8, 6K 0x00d050a0, θ 0x00d05368, 1.05 0x00d05364, ε 0x00c981d0,
1.0 0x00c981c8, ground 0x005dcad0), a 0x120-byte descriptor with the shadow flag at +0x100 and +0x40 = 0x41.

### P3.5 Decoder notes

- **Bit 14 of KR tileart flags.** `TileArtEntry::flags1` reads a KR (version 3) record's 32-bit flags in the EC
  bit layout. KR's bit 14 comes out as `ArticleA`, but it is EC's `Rotated` (bit 34), as shown in P3.4.
  - `get_tile_type` tests `Rotated`, so KR data loses it.
  - EC bits 32 and up (`NoHouse`, `NoDraw`, `Rotated`, `AlphaBlend`, `NoShadow` and the rest) cannot appear in a
    KR record.
  - *Probable*: the data and the code agree.
- **The KR test record.** `kingdom_reborn_record()` in `tileart.rs` writes
  `(TaeFlag::Rotated | TaeFlag::AlphaBlend).bits() as u32`. Both flags are above bit 31, so the value truncates
  to 0 and the test never exercises KR flags.

## Comparison with Dynamapper's KR-like preset (revised)

Dynamapper sources:

- `dynamapper/assets/defaults/shader_presets.toml` and `dynamapper/assets/settings/shaders/land.toml`
  (`[kr.*]`);
- `assets/shaders/world/land/{shading,lighting,main,sampling,normals}.wesl`;
- `assets/shaders/world/art/{main,shading}.wesl`;
- `assets/shaders/postprocess/{tonemapping,color_grading,global_lighting,kr_color_wash,grunge}.wesl`;
- `world/effects/fog_pass.wesl`;
- `src/core/constants.rs` (`BAKED_GLOBAL_LIGHT`).

Values are given as morning / afternoon / night / cave where they differ. Rows marked **(new)** or **(revised)**
come from the second or third pass.

| Item | KR client | Dynamapper KR-like | Gap |
|---|---|---|---|
| Sun | none at run time; terrain only, baked `0.5 (1 + N.D)` per vertex, D 45 degrees above north; flat 0.854 | `BAKED_GLOBAL_LIGHT` (-1, 2.5, -1): 64 degrees elevation, from -x -z; wrapped Lambert (wrap 0.35 / 0.35 / 0.20 / 0.15), sharpened (2.0, mix 0.55); diffuse 1.28 / 1.35 / 0.62 / 0.54 × `light_color`; on land and art | different direction, elevation and model |
| Ambient, fill, rim, specular | none | ambient 0.24 / 0.25 / 0.12 / 0.07; hemisphere fill 0.32 / 0.34 / 0.22 / 0.18; rim 0.13-0.16; specular 0.02-0.04 | Dynamapper only |
| Energy cap | none (one clamp at the end) | headroom cap 0.85 (`grade_params.y` 0.15): flat ground lands at about 0.85, close to the client's 0.854 by coincidence | similar on flat ground only |
| Time of day | server level → `4 (c level + B (1 - level))`: identity by day, (0.69, 0.70, 0.87) at L 10, (0.20, 0.22, 0.67) at L 26 | `global_lighting` 1.04 / 1.08 / 0.76 / 0.56 (neutral), `light_color` night (0.8, 0.88, 1.05) | client night far bluer and, at L 10, brighter |
| Night tint pattern | world-anchored night-map clouds (navy, blue ±0.25), one repeat per 26 x 19.5 world units at default zoom, night only | `kr_color_wash` noise (warm/cool, scale 64-80, strength 0.55-0.74) at all times; `fog_night_color` | different source and timing |
| Gamma / exposure | slider 0..1 → ×0.6..1.4, default ×1 | `exposure` 1.03 / 1.06 / 1.06 / 0.92 | similar role |
| Tonemap | none (LDR) | KR curve: gain 0.9, white 0.8. At exposure 1.06: 0.25 → 0.247, 0.5 → 0.454, 0.75 → 0.602, 1.0 → 0.704 | Dynamapper compresses highlights by up to 30 % |
| Colour grading **(revised)** | none; captures of the references give B/R 0.60-0.89, chroma 21-40 | saturation 1.12-1.14 (night/cave 1.02), vibrance 0.38-0.45, contrast 1.10-1.12, split tone 0.65-0.80, strength 0.9-1.1, plus land and art temperature 0.75 and a warm `light_color`; measured afternoon B/R 0.12-0.82, chroma 19-79 | Dynamapper only; it causes a strong yellow cast |
| Gloom / desaturation | none (death effect only) | gloom 0.16 / 0.14 / 0.24 / 0.30 with desaturation 0.12 / 0.10 / 0.22 / 0.28, faded out 0.005-0.01 world units above y = 0 | Dynamapper only |
| Fog / vignette | none (the `figleaf` sky and fog code is never called) | screen-space radial fog, colour about (0.02-0.04, 0.03-0.05, 0.03-0.07), cap 0.28-0.36 | Dynamapper only |
| Sprite shadows **(revised)** | a black copy of each static, item and foliage sprite, leaning right 11.25 degrees from its ground point, depth-tested; hard, ×0.75 with no stacking, every time of day; mobiles ×0.5 (see [P3.4](#p34-sprite-shadow-geometry)) | projected strength 0.22 / 0.24 / 0.18 / 0.16, length 0.28-0.44, softness 0.62-0.74; a 0.25 / 0.05 stand-in barely shows in captures | client constant and stronger |
| Extra shadows | none | contact 0.30-0.42, relief/crease 0.58-0.70, cliff bank, shadow mottle 0.48-0.60 | Dynamapper only |
| Local lights **(revised)** | light sprites in the light buffer, probably lerped, not grunged, coloured by their texture, sized by the tileart light scale, weight `4 (1 - level)`, none by day | additive decals `albedo × colour × 0.32 × 1.35` at all times; static visibility 0.55 | client lights only at night, multiplicative |
| Statics grunge **(revised)** | KR's `01000045_noise` (EC's differs) × on 75 % of KR art and 99.5 % of legacy art, not on light sprites; world-anchored, 16-tile period, ×0.55-1.0 (mean 0.84); never on terrain or mobiles; never in the EC HLSL path | procedural grunge 0.58-0.65 × gain 1.35 on art and land: about 40-tile period, ×0.48-1.16 | same idea on statics; wrong texture, scale and range; land grunge is Dynamapper only |
| Transition grime, colour wash **(revised)** | none; the ground's unevenness is the two-base blend through the coverage noise | transition grime 0.58-0.65, colour wash 0.55-0.74, on top of the same two-base blend | Dynamapper only |
| Terrain data **(revised)** | KR TerrainDefinition: `Grass_A` (dark green), 24-tile grass mask, forest repetition 5; KR textures for 32 ids | the packages' data, probably EC: `Grass_C`, 32-tile mask | grass and forest records differ, and 32 referenced textures hold other images (see [P3.2](#p32-data-sources)) |
| Texture filtering **(new)** | plain bilinear, no mip levels, about 1.2 texels per pixel at 1024 px; low-resolution grunge and masks magnified 2.5-3.7 times | pixel-art UV remap before the sampler (land and art), `enable_linear_filtering` 1; blur 0.27 in the preset | Dynamapper crisper at texel edges |
| Terrain shading evaluation **(new)** | per-vertex half-Lambert, Gouraud over the (x+1, y)-(x, y+1) split | per-pixel normals (bicubic and bent in the preset) | Dynamapper smoother; no facets |
| Sprite lighting **(revised)** | none: texture × hue × grunge; vertex RGB unused | fake normals, highlight 0.24-0.42, depth tint 0.20-0.34, temperature 0.65-0.75, mottle | Dynamapper only |
| Normals, specular | per-vertex terrain normal; EMBM on snow and the water layers only | land normal map 0.45, normal maps off, specular 0.02-0.04 | minor |
| Bloom | none | none | same |

**Worked example: flat ground, afternoon.**

| Albedo | Client | Dynamapper |
|---|---|---|
| 0.5 | 0.427 | about 0.44 (yellow tint) |
| 0.9 | 0.769 | about 0.63 |

Mid tones are close. Dynamapper's highlights are about 18 % darker. It also adds a yellow cast, fog, grunge and
wash on top.

## Proposed changes, ranked by visual impact (revised)

These are research proposals; no rendering code was changed. Each one is a design decision for the user. The
first proposal needs only settings. The rest need code or data.

1. **Daytime KR: switch off every term the client lacks.** Settings only; variant b of the captures.
   - Switch off the tonemap, grading, gloom, fog, the land and art temperature, the warm `light_color`, ambient,
     fill, rim, specular, the art fake normals, highlight and depth tint, contact and relief shadows, mottle,
     transition grime and colour wash. Set `global_lighting` and `exposure` to 1.
   - Measured effect: blue comes back (forest B/R 0.12 → 0.47, slopes 0.33 → 0.66; references 0.58-0.89).
     Chroma falls from 66-79 to 36-49 (references 21-40). Brightness barely changes.
2. **Statics: the client's grunge instead of the procedural one** (code):
   - multiply the sprite RGB by the client's grunge texture (`01000045_noise`, KR's version: EC's file is a
     different image, see [P3.2](#p32-data-sources));
   - sample it at the world position divided by 16 tiles, at full strength;
   - apply it only to art whose tileart entry has shader map 1 (75 % of KR art, nearly all legacy art); the tilemeta
     refs already carry this;
   - never apply it to land, mobiles or light sprites, and never brighten;
   - for an EC-faithful mode, skip it altogether.

   This is the main missing "dirty, uneven" ingredient.
3. **Terrain data for KR: the KR TerrainDefinition and textures** (data): `Grass_A`, the 24-tile grass mask, the
   KR forest repetitions, and KR's images for the 32 texture ids that differ (15 of them exist only in KR's
   `Texture.uop`; see [P3.2](#p32-data-sources)). The blend code already matches the client. This gives the dark
   olive, blotchy ground of the references.
4. **Filtering: plain bilinear for KR** (code):
   - no pixel-art remap and no extra blur;
   - single-level sampling, or a positive LOD bias to keep the client's softness;
   - a view of 18.4 tiles (zoom 0.79 at 1024 px) to match the client's texel ratio.
5. **Terrain shade: the client's half-Lambert** (code):
   - grey `0.5 (1 + N.D)` with D 45 degrees above north (UO -y), computed per vertex from the corner heights and
     Gouraud-interpolated over the client's diagonal;
   - flat 0.854; 45-degree slopes 0.5 to 1.0; vertical 0.146 to 0.854.

   This changes both the slope contrast and the side that is lit.
6. **Night and cave: the client's composite** (code). Apply `scene × clamp(4 (c level + B (1 - level)))` as a
   post-pass:
   - `c = 0.25 + 0.2 (gamma - 0.5)`;
   - B is the night map (`TerrainTexture.uop` 21), world-anchored, rotated -45 degrees, scale (f / 13,
     0.75 f / 13), with f the view half-width.

   Suggested levels: morning and afternoon 1.0 (L 0); night 0.677 (L 10); cave 0.161 (L 26). This is the
   biggest difference at night and none by day.
7. **Sprite shadows: hard, sprite-shaped, constant ×0.75** at every time of day (code). The current projected
   shadows at the stand-in values barely show. Use the geometry of [P3.4](#p34-sprite-shadow-geometry):
   - a black copy of each static, item and foliage sprite (not Surface, Roof or rotated flat art), leaning right
     by π/16 about its ground point;
   - drawn into a screen-sized buffer with the scene's depth, then `scene * clamp(S + 0.75)`, before the night
     composite;
   - mobiles: a 50 % black copy of the frame drawn straight into the scene.

   An EC-faithful mode keeps only the mobile shadows.
8. **Local lights: inside the composite** (code):
   - weight them by `4 (1 - level)`;
   - lerp them over the night map, not grunged and not hued;
   - give no light at full daylight;
   - use the `TerrainTexture.uop` light textures that tileart block 3 names, scaled by the tileart light scale and
     moved by its light offsets (see [P3.3](#p33-light-sprites)).
9. **Brightness: expose the client's gamma** (×0.6..1.4 at full light, default ×1) as the one control, in place
   of `exposure` and `global_lighting`.

## Open points

- The light-sprite blend: the batch alpha property points to a lerp; the D3D state was not read in a running
  client.
- The meaning of the tileart field +0x38 bits 2 (nearest filter) and 3 (wrap), and of the second terrain field
  (3 in KR, 1 in EC).
- Whether the installed packages carry EC or KR terrain data and art (this decides proposal 3).
- The death effect's TFACTOR.
- The zoom limits of the view half-width (probably 6 and 26).
- Whether the post-process effects run in the order they were appended (shadow first). Both are multiplies, so
  the order only matters through the clamp.
- Third pass:
  - the shadow modes other than 0 and 1 (component +0x24) and what sets them;
  - whether the shadow batch tests z (the depth buffer is shared);
  - the units and the axis order of the tileart light offsets;
  - the light sprite's quad anchor in `BuildSpriteQuad`;
  - UOSA's light sprite placement (not read).
