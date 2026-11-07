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

## Summary

The client's gloom does not come from a world light model, fog, tonemapping or colour grading. **UOKR.exe has
none of these.** Five simple things make the look:

1. **The light level from the server** drives one screen-space multiply. At full daylight it is exactly 1
   (no change). As the level falls, it fades towards a dark-blue, cloudy, world-anchored **night map**, scaled
   by 4.
2. **Projected sprite shadows** multiply the scene by 0.75.
3. **The terrain** carries a baked grey **half-Lambert** vertex shade from one fixed light. Flat ground gets
   0.854.
4. **Light sprites** (torches, windows, the light decals) brighten only where the night has darkened the scene.
5. **The KR art itself** is darker and less saturated than the Classic art.

The Enhanced Client runs the same composite and the same terrain shade.

## Frame order of operations

1. The 3D scene is drawn into the scene render target of `UOLightingPostProcessEffect` (+0x220, render-target
   group +0x240). This covers:
   - **terrain**: the layer blend times the baked vertex grey; EMBM environment layers on bump layers;
   - **statics and mobiles**: sprites, texture times diffuse with the hue;
   - **particle effects**.

   No lights, materials or fog take part. *Confirmed* for the terrain stages and the absence of lights and fog;
   *probable* for the sprite diffuse.
2. `UOShadowPostProcessEffect::Render` (0x0043fe10):
   - it clears the shadow buffer S to white;
   - it draws the shadow sprites into S with `UOSpriteShaderShadow` (MODULATE(texture, diffuse), 0x00546e30);
   - it composites `scene * clamp(S + 1 - s)` (0x0043f9f0, `UOShadowShader` 0x00545e40).

   This runs only while `showShadows` is on (0x00c4a014). *Confirmed* for the formula. The order of this step
   against step 3 is *probable* (the shadow effect is appended first).
3. `UOLightingPostProcessEffect::Render` (0x0041fe50):
   - it draws the night map (or the night-sight map) into the light buffer B (+0x24c);
   - it draws the light sprites of the list +0x134 into B (through 0x00856600);
   - it draws the composite quad into the output (+0x100).

   *Confirmed*, except the blend of the light sprites into B (*unknown*, probably additive).
4. The UI is drawn on top and is not affected (*probable*).
5. **Death effect**: only while the player is dead, `UODeathPostProcessEffect` converts the frame to grey
   (`UODeathShader` 0x00413d20: DOTPRODUCT3, then TFACTOR - CURRENT). *Confirmed* for the stages; the
   TFACTOR value is *unknown*.

## Ingredients

### 1. Light level, night map and gamma: the lighting composite (largest contribution)

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

**Night-map placement** (*probable*). `UONightMapShader::SetupTextureStages` 0x00544600 applies a texture
transform:

- the scale is (f / 13, 0.75 f / 13), where f = `*(DAT_00c9dc70 + 0x6c)`, probably the visible width;
- the transform is rotated -45 degrees;
- it is translated by the camera position.

So the clouds are anchored to the world and do not move with the screen. One repeat spans about 26 world units.

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
- **The textures** (*confirmed* data):
  - the light decals are `TerrainTexture.uop` entries 1-17 (DXT5; entry 16 is `circle_of_trans`);
  - the tileart light textures are tileart block 3 `*_light_window.tga`, with light scale 2.0 on windows
    (see `ghidra-tileart`).
- **Unknown:** the blend of the light sprites into B (probably additive) and their colour scale.

### 3. Projected sprite shadows

**Formula** (*confirmed*): `out = scene * clamp(S + 1 - s)`. S = 1 outside shadows and is lowered by the shadow
sprites. The intensity s comes from three places:

- constructor 0x0043f740: 0.4;
- `InitPostProcessEffects` sets **0.25** (0x00c4a024);
- `UOAtmosphere::SetShadowIntensity` 0x0041f0c0 changes it (debug keys only, ±0.05).

**Contribution.** Full shadow is ×0.75. It is the same at every time of day and is multiplied again by the light
composite at night. The shadow shape is the sprite's own alpha, through `UOSpriteShaderShadow`: no blur pass and
no length or softness parameters. The `showShadows` setting toggles it; its default is *unknown*.

### 4. Terrain shade: baked half-Lambert, one fixed light

**Formula** (*confirmed*). `UOTerrain::BuildTerrainMesh` 0x0044b7f0 writes four vertices per cell:

```
vertex colour = (g, g, g, 1),   g = (N . D + 1) * 0.5
D = (0, -0.7071, -0.7071)       constant at 0x00c0f3b8 (read only, never written)
N = UOTerrain::ComputeVertexNormal 0x004488a0
```

`ComputeVertexNormal` averages three normalised triangle normals from the corner heights; the height axis is y
negated; one UO z unit is 0.12856 world units (0x00b86f38). The terrain stages modulate the layer blend by this
diffuse (UODefaultTerrainLayer, see `ghidra-terrain-layers`).

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
- there is no ambient floor other than the half-Lambert's own 0.5 at 90 degrees from the light.

### 5. Sprites (statics, mobiles)

- **Unlit** (*confirmed* for KR): no NiDirectionalLight, NiAmbientLight, NiFogProperty or NiMaterialProperty
  is linked. The RTTI scan found only a NiPointLight type descriptor, with no vtable.
- The sprite fixed-function stage is MODULATE(texture, diffuse), with the hue lookup.
- **Probable:** the diffuse is white. There is no evidence of baked sprite shading.
- **Unknown in KR:** the programmable `uosprite.psh` in `Shaders.uop` has an optional `HAS_GRUNGE_TEX` path
  (`rgb *= grunge`, grunge noise `Texture.uop` `build/worldart/01000045.dds`, 256x256, mean 0.84, min 0.55).
  Whether KR selects it is not resolved.
- `LightingShader` (vtable 0x00b692bc) exists for the Mythic library's own 3D lights; no UO world path was
  found that uses it.

### 6. Terrain normal and specular use

- UOBumpMapTerrainLayer (vtable 0x00b52bac, 0x00549e60) is EMBM: an environment texture perturbed by a bump map
  and added. It is used by water-like layers. *Confirmed* for the stage use; which terrains use it is in
  `ghidra-terrain-layers`.
- There is no normal-mapped diffuse lighting and no specular. *Confirmed* by the absence of lights.

### 7. Bloom, tonemapping, colour grading, vignette, fog: not used

- **Fog** (*confirmed*): there is no NiFogProperty class. The post-process composites set FOGENABLE 0.
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
| shadows on | user setting `showShadows` | settings file; default *unknown* |
| night map, night-sight map, day map | `TerrainTexture.uop` 21, 22, 20 | UOP data |
| terrain light D | (0, -0.7071, -0.7071) | exe constant 0x00c0f3b8 |
| light decals | `TerrainTexture.uop` 1-17; tileart light textures | UOP data |

There is no registry or ini value for any of these. The `UserInterface` Lua/XML only exposes the gamma slider
and the shadow checkbox.

## KR and the Enhanced Client

| Piece | UOKR.exe | UOSA.exe | Status |
|---|---|---|---|
| lighting composite | 0x0041f5c0 / 0x005441c0 | `BuildCompositeQuad` 0x00429950 (`c = 0.25 + 0.2 (gamma - 0.5)`, alpha = level +0x21c, gamma +0x224), `UOLightingShader::SetupTextureStages` 0x0058f7f0 (same stages: BLENDDIFFUSEALPHA, MODULATE4X, stages 2-3 disabled) | identical, *confirmed* |
| day map loaded, unused | yes | yes (0x0042a360 loads `00000020_daymap.tga`) | identical, *confirmed* |
| terrain half-Lambert | 0x0044b7f0, D at 0x00c0f3b8 | `BuildTerrainMesh` 0x00461ae0, D at 0x00dc18b8 = (0, -0.7071, -0.7071), `(N . D + 1) * 0.5`, normals 0x00460c70 | identical, *confirmed* |
| object layout | light +0x214, gamma +0x21c | light +0x21c, gamma +0x224 | EC adds 8 bytes before |

**Probable:** the EC's brighter look does not come from a different post-process. It comes from its art: the
official EC draws Classic art unless `UseNewArt` is set, while KR draws the darker KR art. Server light levels
apply to both. EC shadows, EC sprite diffuse and EC settings defaults were not re-checked here.

## Comparison with Dynamapper's KR-like preset

Dynamapper sources:

- `dynamapper/assets/defaults/shader_presets.toml` (`[kr.*]`);
- `assets/shaders/world/land/{shading,lighting,main}.wesl`;
- `assets/shaders/postprocess/{tonemapping,color_grading,global_lighting,kr_color_wash}.wesl`;
- `world/effects/fog_pass.wesl`;
- `src/core/constants.rs` (`BAKED_GLOBAL_LIGHT`).

Values are given as morning / afternoon / night / cave where they differ.

| Item | KR client | Dynamapper KR-like | Gap |
|---|---|---|---|
| Sun | none at run time; terrain only, baked `0.5 (1 + N.D)`, D 45 degrees above north; flat 0.854 | `BAKED_GLOBAL_LIGHT` (-1, 2.5, -1): 64 degrees elevation, from -x -z; wrapped Lambert (wrap 0.35 / 0.35 / 0.20 / 0.15), sharpened (2.0, mix 0.55); diffuse 1.28 / 1.35 / 0.62 / 0.54 × `light_color`; on land and art | different direction, elevation and model |
| Ambient, fill, rim, specular | none | ambient 0.24 / 0.25 / 0.12 / 0.07; hemisphere fill 0.32 / 0.34 / 0.22 / 0.18; rim 0.13-0.16; specular 0.02-0.04 | Dynamapper only |
| Energy cap | none (one clamp at the end) | headroom cap 0.85 (`grade_params.y` 0.15): flat ground lands at about 0.85, close to the client's 0.854 by coincidence | similar on flat ground only |
| Time of day | server level → `4 (c level + B (1 - level))`: identity by day, (0.69, 0.70, 0.87) at L 10, (0.20, 0.22, 0.67) at L 26 | `global_lighting` 1.04 / 1.08 / 0.76 / 0.56 (neutral), `light_color` night (0.8, 0.88, 1.05) | client night far bluer and, at L 10, brighter |
| Night tint pattern | world-anchored night-map clouds (navy, blue ±0.25), night only | `kr_color_wash` noise (warm/cool, scale 64-80, strength 0.55-0.74) at all times; `fog_night_color` | different source and timing |
| Gamma / exposure | slider 0..1 → ×0.6..1.4, default ×1 | `exposure` 1.03 / 1.06 / 1.06 / 0.92 | similar role |
| Tonemap | none (LDR) | KR curve: gain 0.9, white 0.8. At exposure 1.06: 0.25 → 0.247, 0.5 → 0.454, 0.75 → 0.602, 1.0 → 0.704 | Dynamapper compresses highlights by up to 30 % |
| Colour grading | none | saturation 1.12-1.14 (night/cave 1.02), vibrance 0.38-0.45, contrast 1.10-1.12, split tone 0.65-0.80, strength 0.9-1.1 | Dynamapper only |
| Gloom / desaturation | none (death effect only) | gloom 0.16 / 0.14 / 0.24 / 0.30 with desaturation 0.12 / 0.10 / 0.22 / 0.28, faded out 0.005-0.01 world units above y = 0 | Dynamapper only |
| Fog / vignette | none | screen-space radial fog, colour about (0.02-0.04, 0.03-0.05, 0.03-0.07), cap 0.28-0.36 | Dynamapper only |
| Sprite shadows | sprite-shaped, ×0.75, every time of day | projected strength 0.22 / 0.24 / 0.18 / 0.16, length 0.28-0.44, softness 0.62-0.74 | close by day; client constant |
| Extra shadows | none | contact 0.30-0.42, relief/crease 0.58-0.70, cliff bank, shadow mottle 0.48-0.60 | Dynamapper only |
| Local lights | light sprites in the light buffer, weight `4 (1 - level)`, none by day | additive decals `albedo × colour × 0.32 × 1.35` at all times; static visibility 0.55 | client lights only at night, multiplicative |
| Grunge, grime, wash | none in the KR fixed-function paths (`uosprite.psh` grunge in KR *unknown*) | grunge 0.58-0.65, transition grime 0.58-0.65, colour wash 0.55-0.74 | Dynamapper only |
| Sprite lighting | none (texture × diffuse × hue) | fake normals, highlight 0.24-0.42, depth tint 0.20-0.34, temperature 0.65-0.75 | Dynamapper only |
| Normals, specular | EMBM water-like layers only | land normal map 0.45, normal maps off, specular 0.02-0.04 | minor |
| Bloom | none | none | same |

**Worked example: flat ground, afternoon.**

| Albedo | Client | Dynamapper |
|---|---|---|
| 0.5 | 0.427 | about 0.44 (cool tint) |
| 0.9 | 0.769 | about 0.63 |

Mid tones are close. Dynamapper's highlights are about 18 % darker, and it adds fog, grunge and wash on top.

## Proposed changes, ranked by visual impact

These are research proposals; no rendering code was changed. Each one is a design decision for the user.

1. **Night and cave: replace the neutral dimming with the client's composite.**
   - Apply `scene × clamp(4 (c level + B (1 - level)))` as a post-pass, with:
     - `c = 0.25 + 0.2 (gamma - 0.5)`;
     - B = the night map (`TerrainTexture.uop` 21), world-anchored, rotated -45 degrees, scale (f / 13, 0.75 f / 13).
   - Suggested levels:
     - morning and afternoon 1.0 (L 0);
     - night 0.677 (L 10): about (0.69, 0.70, 0.87);
     - cave 0.161 (L 26): about (0.20, 0.22, 0.67).
   - This is the biggest single difference: the client's night is a blue cast with cloud modulation, and
     Dynamapper's is neutral grey.
2. **Daytime KR: make the tonemap the identity and drop the grading.** The client has neither. Dynamapper's KR
   curve maps white to 0.70 and 0.75 to 0.60; the client leaves them at 1.0 and 0.75. The saturation (1.12-1.14),
   contrast (1.10-1.12) and split toning have no counterpart.
3. **Terrain: use the client's shade.** Grey `0.5 (1 + N.D)` with D 45 degrees above north (UO -y). Flat 0.854;
   45-degree slopes 0.5 to 1.0; vertical 0.146 to 0.854. Use no ambient, fill, rim, specular, wrap, sharpening,
   temperature, relief, crease or mottle terms. This changes both the slope contrast and the side that is lit:
   Dynamapper lights from -x -z at 64 degrees.
4. **Fog and vignette: off for KR.** The client has no fog. Dynamapper's radial fog darkens the screen edges
   towards near black, by up to 28-36 %.
5. **Sprites: unlit.** Texture × hue only. Drop fake normals, highlight, depth tint, temperature and contact
   shadow for KR art.
6. **Local lights: weight them by `4 (1 - level)` inside the composite.** They should multiply, not add, and give
   no light at full daylight. Use the light decal textures (`TerrainTexture.uop` 1-17) and the tileart light
   textures.
7. **Sprite shadows: a constant ×0.75** (strength 0.25) at every time of day, shaped by the sprite. Drop the
   extra contact, crease and bank shadows.
8. **Grunge, transition grime, colour wash, gloom, shadow mottle: off for KR fidelity.** Keep them as optional
   styling only. Most of KR's daytime gloom comes from its art, which Dynamapper already draws.
9. **Brightness: expose the client's gamma** (×0.6..1.4 at full light, default ×1) as the one control, in place
   of `exposure` and `global_lighting`.

## Reference images (`docs/images/kr_reference/`, visual guidance only)

| Image | Size | Content | Notes |
|---|---|---|---|
| `254602…16x9` | 576x323 | house interior at dusk: blue-purple walls, warm fireplace glow, dark shade under trees | muted; mean (87, 79, 70), chroma 23. Blue walls and a warm pool fit the night composite plus a light sprite |
| `2712200822561889526` | 1000x750 | daylight keep roof: neutral grey stone, dark cliffs | mean (80, 80, 69); no blue cast, as an identity composite would give |
| `350px-Lakeshirekr` | 350x349 | dusk grass with a campfire halo: desaturated olive green | mean (63, 68, 38), luminance p50 62 |
| `6490` | 1152x864 | closed-beta snow area in full daylight: bright snow, dark cliffs | luminance p10 17, p90 160; strong slope contrast, as the half-Lambert gives |
| `UO_KR_screen.webp` | 800x600 | daylight town: warm dirt, lightning, tree shade | |
| `gallery_74` | 993x768 | the brightest shot: warm daylight | mean (122, 105, 86) |
| `images.jpg` | 259x194 | small dusk scene with torch light pools | |
| `sanctuary-warzone` | 500x368 | dark cool blue-grey ground, many monsters | consistent with a low light level |
| 2006 IGN interior | 1024x768 | warm left half, cool blue floor on the right; soft sprite shadows visible | light sprites over a blue night composite |

Across the set:

- median luminance is about 62-105 of 255, and chroma about 21-40;
- the daylight shots have no blue cast;
- the dusk and night shots are blue to purple, with warm light pools.

This matches the composite: identity by day, the night map plus light sprites at night.

## Open points

- The blend and colour scale of the light sprites in the light buffer (0x00856600).
- Whether KR uses the programmable `uosprite.psh` (grunge) or only the fixed-function sprite shader.
- The sprite diffuse colour (probably white) and the default of `showShadows`.
- The exact night-map repeat: the meaning of `*(DAT_00c9dc70 + 0x6c)`.
- The order of the shadow and lighting effects (probably shadow first; both are multiplies, so the order only
  matters through the clamp).
