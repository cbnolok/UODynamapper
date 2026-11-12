# KR-like tuned toward the KR reference screenshots

The aim is to make KR-like feel more like the KR screenshots in `docs/images/kr_reference/`, even where the
screenshots were post-processed. This note measures the screenshots and KR-like by region type, sets two preset
values from the measurements, and proposes one new shader term.

- **Build:** dev `3ad8597c` (debug build). No rendering code changed. The tuning is preset values only.
- **Settings:** isolated copies of the bundled settings. Views at the KR client's frustum (zoom 0.79 at a 1024 x 768
  window, 18.4 tiles across). EC art, with land from each source (KR, EC, CC).
- **Script:** [`scripts/dev-helpers/renderer/krlike_reference_tuning.py`](../../scripts/dev-helpers/renderer/krlike_reference_tuning.py).
  It reuses the launch machinery of `shader_settings_audit.py`: KDE Desktop 4, the control socket and the preview
  lock. It also measures and builds the sheets, and makes the colour mock-ups.
- **Sheets:** in `/home/claudio/rust-target/uodynamapper-scratch/krlike-tuning/sheets/`.
  - `ref_vs_krlike.png`: each reference beside its matched view, before the tuning, after it, and with the proposed
    chroma knee. KR land.
  - `ref_vs_krlike_ec_land.png`: the same with EC land.
  - `periods_kr.png`, `periods_ec.png`, `periods_cc.png`: the four periods, before and after, at Buccaneer's Den and
    Trinsic, one sheet per land source.

## The references

None of the shots can be placed on the map, so each one is matched by content to a view. Region boxes are drawn by
hand on the shots and on the captures (`REFERENCE_BOXES` and `VIEW_BOXES` in the script).

| Reference | What it shows | Time | Matched view |
|---|---|---|---|
| `6490.jpg` | Closed beta. A custom house on snow beside dark cliffs, a static-water pool, cobbles, wood and flowers | day | snow (4050, 500) |
| `2712200822561889526.jpg` | Keep roof with cannons above steep grass cliffs, with the sea at the bottom left | day | coast (1445, 1790) |
| `gallery_74_1428_img.jpg` | Warm sandstone walls and stairs with grass. A heavily processed marketing shot | day | Britain (1434, 1699) |
| `UO_KR_screen.webp` | 2006 promo. Town dirt, a timber house, a stone castle with banners, blue-grey trees | day | Britain |
| `ultima-online-kingdom-reborn-2006…jpg` (IGN) | An interior with blue-purple wall art, grass and dirt | day | Britain |
| `350px-Lakeshirekr.jpg` | Dark grass with a campfire | day | Yew (620, 1020) |
| `254602-…16x9.jpg.jpg` | A house interior with a wood floor and grass | day | Yew |
| `sanctuary-warzone.jpg` | Dark grey ground with a sandstone pyramid | day | Covetous (2460, 880) |
| `images.jpg` | Thatch and fences. Too small to measure | day | — |

Two more views cover water, wood and roofs: Trinsic (1867, 2780) and Buccaneer's Den (2700, 2150).

Every shot is daylight: the night composite is the identity at light level 0, so it plays no part in them. The KR
atmosphere research (`uokr_atmosphere.md` §7, P2.6 and P2.7) confirms that the client has no bloom, HDR, grading,
vignette or fog. The only client term Dynamapper still lacks is the grunge on statics, which another change
covers. So every difference measured below is texture data or post-processing, not a missing effect.

## Measurements before the tuning (afternoon, KR land, EC art)

Values are sRGB luma (Rec. 709, 0..255) and mean HSV saturation per pixel. The medians pool every box of a kind.

| Kind | References | KR-like before | Remark |
|---|---|---|---|
| water | 41 (sea in 2712; hue 206, sat 0.72). The 6490 pool box (86) also holds pillars | 22 (hue 207, sat 0.76) | same hue, 1.9 times darker |
| grass | 75, sat 0.33 (n 6) | 76, sat 0.51 | brightness matches, too saturated |
| foliage | 58, sat 0.33 (n 5) | 74, sat 0.64 | too saturated, and brighter |
| stone, wall, flag | 103, sat 0.34 (n 9) | 106, sat 0.28 | close |
| wood | 67, sat 0.38 | 56, sat 0.39 | close |
| ground (sand, dirt) | 85, sat 0.33 | 101, sat 0.28 | close |
| rock | 38 (6490 cliffs) | 39 (Covetous) | matches |
| snow | 145, sat 0.12, hue 236 (closed beta) | 224, sat 0.05 | see open questions |

The shots' fine detail (mean absolute difference from a 2-pixel blur: grass 6 to 13, stone 11 to 19) matches
KR-like's (grass 7 to 14, stone 10 to 15). Region spread is somewhat larger in the shots (stone sd about 36
against about 25), but the shots' boxes hold mixed content and JPEG blocks. No existing setting adds contrast
around mid-grey without also changing brightness: the grading's contrast curve `x + t·2(x − x²)` brightens every
value below 1. So contrast and the tone curve stay at the client's identity.

### Water: a texture difference between KR and EC

The water layer's colour (item 2 of material 5) is `01000017_cube3`. It has the same name in both clients, but
not the same texture:

| `build/worldart/01000017.dds` | mean RGB | luma |
|---|---|---|
| KR `Texture.uop` | 9.5, 26.5, 40.9 | 24 |
| EC `Texture.uop` | 17.1, 47.7, 72.7 | 43 |

KR-like water on EC land (38, sat 0.75, hue 206) already matches the 2712 sea (41, sat 0.72, hue 206). Water on KR
land is darker because the KR texture is darker. The 2712 shot most likely shows a KR build that carried the
brighter texture; the KR client extracted here draws its water at about 24.

The client also draws its water layer unlit: its stages never modulate by the vertex light (`uosa_terrain_layers.md`,
`UOWaterTerrainLayer`). Dynamapper multiplies every water path by `ambient_strength + diffuse_strength ×` the
wrapped flat Lambert. With the sun at 45 degrees that Lambert is 0.854, so before the tuning water was drawn 15%
darker than the texture. Every water path reads this factor: KR/EC liquid land, classic flat wet land, flat-water
statics and EC item water.

`ambient_strength` is the only lever on it. Under per-vertex land it lights nothing but water. But it acts on every
land source alike, and the land source is a graphics setting apart from the preset. These were the trials
(afternoon water luma, before → after):

| ambient_strength | KR land | EC land (the default source) |
|---|---|---|
| 0 (before) | 22 | 38 |
| 0.146 (factor 1, the client's unlit water) | 26 | 45 |
| 0.6 | 37 | — |
| 0.8 | 41 | 74 |

At 0.8, KR land matches the sea but EC land, the default, is drawn 1.8 times too bright. The tuning therefore
takes 0.146, the client's own unlit water. Bringing KR land the rest of the way needs the texture, not the preset:
see the proposals.

### Saturation: grading at 0.8

Grass and foliage are 0.5 to 0.65 saturated in KR-like and about 0.33 in the shots. Stone, wood and ground already
match. The grading's saturation mixes toward Rec. 709 luma in linear light, before the tonemap and before the
night composite. A numpy mock-up of that step on the captures matches the shader's own output to within 1 level
(mean 0.23), so mock-ups stand in for launches (`mockup`, then `stats` and `compare`). These are RMS errors of
saturation against the references over grass, foliage, stone, wood and ground. They are mock-ups of the ungraded
afternoon captures: KR land over coast, Britain, Yew, Trinsic and Buccaneer's Den; EC and CC land over coast,
Trinsic and Buccaneer's Den.

| Saturation | KR land | EC land | CC land |
|---|---|---|---|
| 1.0 (before) | 0.160 | 0.141 | 0.254 |
| 0.9 | 0.111 | 0.101 | 0.178 |
| 0.85 | 0.095 | 0.089 | 0.147 |
| 0.8 | 0.086 | 0.082 | 0.120 |
| 0.75 | 0.084 | 0.084 | 0.097 |
| chroma knee 0.6 (proposal) | 0.048 | 0.040 | 0.066 |

The choice is 0.8. Below it, the gain on greens is paid for by stone, wood and water, which are already at or
below the shots. Vibrance cannot single out the vivid greens: its mask `smoothstep(0, 0.7, 1 − chroma)` is about
1 for every colour these scenes hold, so it acts as more saturation.

## The tuning (all four KR-like periods)

Both files changed identically: `dynamapper/assets/defaults/shader_presets.toml` and the shipped
`dynamapper/assets/settings/shaders/land.toml`.

| Setting | Before | After | Why |
|---|---|---|---|
| `lighting.ambient_strength` | 0.0 | 0.146 | Water factor 0.146 + 0.854 = 1: the client's unlit water. +17% on every water |
| `lighting.enable_grading` | 0 | 1 | for the saturation below |
| `lighting.grade_extra` | per period, e.g. [0.45, 1.14, 1.12, 0.80] (unused while grading was off) | [0.0, 0.8, 1.0, 0.0] | Saturation 0.8 alone: no vibrance, contrast 1 (identity), split tone 0 |

Unchanged:
- the night composite (light levels 0 / 0 / 10 / 26, overlay strength and colour);
- the client shadows and the sun;
- the identity tone curve and the exposures (1.03 / 1.06 / 1.06 / 0.92);
- the per-vertex land and `grade_params`;
- the grunge settings, which belong to the other change.

The grading runs in the world shaders before the night composite pass, so it does not touch the composite's
formula or its overlay colour. The preset header now lists grading and ambient with their reasons. The tooltips
for Sun Strength and Terrain Ambient wrongly said that Enhanced-texture water ignores them; they now say these
settings light every water under per-vertex land. The Color Grading tooltip names KR-like.

### Results (KR land, afternoon; luma / saturation)

| View | Region | Before | After |
|---|---|---|---|
| coast | water | 22 / 0.76 | 26 / 0.64 |
| coast | grass | 81 / 0.59 | 81 / 0.43 |
| coast | foliage | 83 / 0.62 | 84 / 0.45 |
| coast | sand | 101 / 0.38 | 101 / 0.29 |
| Britain | grass | 73 / 0.51 | 73 / 0.38 |
| Britain | foliage | 65 / 0.49 | 66 / 0.37 |
| Britain | stone | 61 / 0.32 | 61 / 0.25 |
| Britain | roof | 72 / 0.32 | 73 / 0.24 |
| Yew | grass | 76 / 0.66 | 76 / 0.47 |
| Yew | foliage | 83 / 0.65 | 84 / 0.47 |
| Trinsic | stone | 118 / 0.38 | 118 / 0.31 |
| Trinsic | water | 29 / 0.61 | 32 / 0.52 |
| Trinsic | wood | 58 / 0.36 | 58 / 0.30 |
| Buccaneer's Den | water | 22 / 0.74 | 26 / 0.62 |
| Buccaneer's Den | grass | 70 / 0.37 | 70 / 0.28 |
| Buccaneer's Den | foliage | 63 / 0.72 | 64 / 0.53 |
| Covetous | rock | 39 / 0.23 | 39 / 0.19 |
| snow | snow | 224 / 0.05 | 224 / 0.04 |

Water luma per period, at coast, Trinsic and Buccaneer's Den (before → after):

| Land | Morning | Afternoon | Night | Cave |
|---|---|---|---|---|
| KR | 21→25, 28→32, 22→26 | 22→26, 29→32, 22→26 | 15→18, 20→22, 16→19 | 5→6, 6→6, 5→6 |
| EC | 38→45, 38→43, 37→44 | 38→46, 38→44, 38→45 | 27→33, 27→31, 27→32 | 9→11, 8→9, 9→11 |
| CC | 36→42, 19→20, 12→15 | 36→42, 19→21, 13→15 | 26→30, 13→15, 9→11 | 9→10, 4→4, 3→4 |

Saturation by period at Buccaneer's Den (KR land):

| Region | Morning | Afternoon | Night | Cave |
|---|---|---|---|---|
| grass | 0.37→0.29 | 0.37→0.28 | 0.26→0.17 | 0.45→0.49 |
| foliage | 0.72→0.53 | 0.72→0.53 | 0.72→0.54 | 0.78→0.66 |

EC and CC land move the same way: CC grass goes from 0.59 to 0.43, EC grass from 0.39 to 0.30. Region luma is
unchanged except water.

In caves, saturation measures mostly the composite's blue overlay, which is applied after the grading, so it barely
moves. Classic, Enhanced and UOD-Painterly are pixel identical before and after at Buccaneer's Den and Trinsic,
with KR, EC and CC land (18 captures, every one differs by 0).

## Proposals (not implemented)

1. **Brighter KR water texture.** KR land's water is 26 against the shots' 41, because KR's `01000017_cube3` has
   luma 24 and EC's has 43. If the KR land package took EC's cube3 for material 5, water on KR land would draw
   the same as on EC land: 44 to 46, measured. That is a data change to `tex_land_kr.uddp` (its builder's
   source choice), so it waits for confirmation. The other route, a separate water gain per land source, would be
   a new setting in the user-written settings file, a schema change.
2. **Chroma knee: a new grading term.** It would compress only vivid colours. In linear light, relative chroma
   c = (max − min) / max above a knee K maps to K + (c − K)·m, by the same mix toward luma as saturation. Measured
   by the validated mock-up (`mockup … --knee 0.6 --slope 0`, in place of saturation 0.8):
   - KR land: grass 0.37, foliage 0.40, stone 0.27, wood 0.36, ground 0.28;
   - EC land: grass 0.36, foliage 0.39, stone 0.30, wood 0.36, ground 0.33;
   - the RMS error against the references is about half that of saturation 0.8 (KR 0.048, EC 0.040, CC 0.066).

   Water goes to 0.53 to 0.55, against 0.62 with saturation 0.8. It would need two new `grade_extra`-style values
   (knee, slope), which is a schema change, so it is proposed only. The `ref_vs_krlike*.png` sheets show it in
   their fourth column.

## Open questions

- **Snow.** It is 224 in KR-like and 145, slightly blue (hue 236, sat 0.12), in the closed-beta 6490 shot. The KR
  snow textures (`02000080` / `02000081`) are 238..245 and 213..232, and the client adds an EMBM environment map
  (`01000014_cube0`, almost black) on top. So the final client should draw snow near white, and the beta shot
  probably shows other textures or a darker gamma. Nothing was changed for snow.
- **Foliage brightness.** EC art foliage is about 16 levels brighter than the shots' (74 against 58), while grass
  matches. No setting dims art alone. The grunge on statics (×0.55..1.0) that the other change adds may close part
  of this; measure again after it lands.
- **Water on KR land** stays darker than the shots until proposal 1 is decided.
- **Per-pixel land.** If a user switches KR-like to per-pixel land, the new ambient 0.146 is also added to the land's
  light factor.
