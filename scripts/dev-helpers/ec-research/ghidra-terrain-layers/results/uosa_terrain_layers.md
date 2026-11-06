# UOSA.exe terrain layers: TerrainDefinition.uop items to texture stages

Sources:

- `UOSA.exe` SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, in Ghidra;
- `_Ultima Online Enhanced/TerrainDefinition.uop` + `string_dictionary.uop` (201 records), read with the repo
  reader `uocf::enhanced::terrain_definition` from a scratch crate ([`../terrain_layers_census.rs`](../terrain_layers_census.rs));
- `GameData.uop:data/gamedata/legacyterrainmap.csv` (2705 rows).

Names are the ones applied by [`../NameTerrainLayerCode.java`](../NameTerrainLayerCode.java) (see
[`uosa_terrain_layers_names.md`](uosa_terrain_layers_names.md)) and the earlier `NameTerrainCode.java` /
`NameTextureRoleCode.java`. Earlier work used: `ghidra-terrain/results/uosa_terrain.md` (record format),
`docs/agent_notes/ec_original_water_code.md` (coverage, sorting, water pass test).

"Verified" means read in the disassembly or the decompilation of this build. "Inference" is marked.

## 1. Loading: from a TerrainDefinition record to a layer object

- `UOTerrain::LoadTerrainLayer` 0x00461800 (verified):
  1. reads the shader name of the record's shader block (block vt +0x10) and calls
     `UOTerrain::CreateTerrainLayerForShader` 0x004616e0;
  2. `SetMaterialId(material)`; the water class stores `material × 1000` (`UOWaterTerrainLayer::SetMaterialId`
     0x004601e0), so water layers sort after the others (comparator 0x005996d0);
  3. builds the 32×32 coverage map (`UOTerrain::BuildTerrainCoverage` 0x00461060) from `coverage_exponent`
     (+0x38), sets its flags (`0x3100`, then `& 0xcf01 | 1`: texture coordinate set 1) and clears bit 0x100
     when `coverage_exponent == 0.0` (constant 0x00c98578), else sets it; `AddMap(coverage)` → **map 0**;
  4. for each texture item i in order: `UOTerrain::LoadTerrainLayerTexture` 0x004613b0 → `AddMap` → **map i + 1**;
  5. techniques → `AddTechnique` (+0x20), tweaks → `AddTweak` (+0x30).
- `CreateTerrainLayerForShader` 0x004616e0 (verified) compares the **shader name only** (std::string compare
  0x004229a0, equality helper 0x0045c2c0):

  | shader name in the record | class | ctor | vftable |
  |---|---|---|---|
  | `UOWaterTerrainLayer` (0x00ca08c4), `UOWaterShader` (0x00ca0538) | `UOWaterTerrainLayer` | 0x00460190 | 0x00ca0a5c |
  | `UOBumpMapTerrainLayer` (0x00ca08f0) | `UOBumpMapTerrainLayer` | 0x004600f0 | 0x00ca0a0c |
  | `UODefaultTerrainLayer` (0x00ca08d8), `UOTerrainShader` (0x00ca0858), anything else, allocation failure | `UODefaultTerrainLayer` | 0x00597170 | 0x00cb466c |

  Shipped data: 192 `UODefaultTerrainLayer`, 8 `UOWaterTerrainLayer`, 1 `UOBumpMapTerrainLayer` (material 8, snow).
- `LoadTerrainLayerTexture` 0x004613b0 (verified): `UOTerrain::MakeWorldArtTextureLocation` 0x004603f0 builds
  `"Data\\WorldArt\\" + path` (no test on the name); `source_kind` (+0x1c) 0 or 2 loads through the viewer
  texture manager (viewer +0xa8), anything else or a failed load gives the missing texture `DAT_00e33e94`;
  map flags `(f & 0xf1ff) | 0x3100`; texture transform translation 0, rotation 0, **scale 1 / stretch on both
  axes** (`1.0 / item +0x20`), centre (0.5, 0.5). `sampler_index` and `uv_set` are not read here (as found
  in `ghidra-terrain`: `uv_set` is 0 in every item, `sampler_index` has no terrain reader).
- `UOStaticTerrainShader` is not a terrain layer class: the factory never creates it. It is a tileart item
  shader (0x00595680 / 0x00595a00; see `ghidra-texture-roles`).

Layer objects (`UOBaseTerrainLayer`, vftable 0x00cb4624, 0x38 bytes) share these slots (verified, all three
class vftables):

| slot | function |
|---|---|
| +0x00 | destructor (default 0x005971b0, water 0x004601f0, bump 0x00460140) |
| +0x04 | `Draw(pass object, pass index)`: default 0x00597210, water 0x005988a0, bump 0x00597f00 |
| +0x08 / +0x0c | `GetMaterialId` 0x004600d0 / `SetMaterialId` 0x004600e0 (water 0x004601e0) |
| +0x10..+0x1c | `AddMap` 0x00596f30, `SetMap` 0x00596f80, `GetMap(i)` 0x00596fd0, `MapCount` 0x00597000 |
| +0x20..+0x2c | `AddTechnique` 0x00597020, `SetTechnique` 0x00597070, `GetTechnique(i)` 0x00597090, `TechniqueCount` 0x005970a0 |
| +0x30..+0x3c | `AddTweak` 0x005970c0, `SetTweak` 0x00597120, `GetTweak(i)` 0x00597140, `TweakCount` 0x00597150 |
| +0x40 / +0x44 | `SetupFourStages` 0x005974f0 / `SetupTwoStages` 0x00597b50 (default and bump; water and base have none) |

The layers do not use Gamebryo shader-map slots or HLSL samplers: each `Draw` binds maps to fixed-function
texture stages with `BindShaderMapToTextureStage` 0x0058dfc0 (EAX = map, stage pushed; the map's
coordinate set byte goes to TEXCOORDINDEX) and writes the D3D texture stage states directly. Stage objects come
from the pass object's stage array (+0x24, count +0x2a): `GetTerrainPassTextureStage` 0x00596930, inlined in the
default and bump draws. The caps tested are the pass object's +0x98 / +0x94 (stage counts) and +0x90
(D3DTEXOPCAPS: 0x200000 BUMPENVMAP, 0x400000 BUMPENVMAPLUMINANCE).

## 2. Per-class mapping (map index = item position + 1; map 0 = generated coverage)

### UODefaultTerrainLayer (`Draw` 0x00597210, verified)

`GetMap(0..3)`, stages 0..3. With 4 or more blend stages: `SetupFourStages(pass, map0, map1, map2, map3, stage0..3)`
(call at 0x005974b0); otherwise `SetupTwoStages(pass, map0, map1, stage0, stage1)` (0x005974de).

`SetupFourStages` 0x005974f0:

| stage | map | item | colour | alpha |
|---|---|---|---|---|
| 0 | 2 | **1** | SELECTARG1 TEXTURE, RESULTARG TEMP | |
| 1 | 3 | **2** | SELECTARG1 CURRENT | SELECTARG1 TEXTURE (the mask alpha) |
| 2 | 1 | **0** | BLENDCURRENTALPHA(TEXTURE, TEMP) = item0 · a + item1 · (1 − a), a = item 2 alpha | SELECTARG1 TEXTURE |
| 3 | 0 | coverage | MODULATE(CURRENT, DIFFUSE) | ALPHAARG1 DIFFUSE on pass 0, coverage TEXTURE otherwise (0x005979f0) |

`SetupTwoStages` 0x00597b50 (fewer than 4 stages): stage 0 = item 0, MODULATE(TEXTURE, DIFFUSE), alpha TEXTURE;
stage 1 = coverage, colour CURRENT, alpha DIFFUSE on pass 0 else coverage. **Item 0 is the only colour drawn.**

Item 3 is never fetched (`GetMap` stops at 3). 48 default records have a 4th item (`00001268_red_tiles.tga`,
`00001260_blue_tiles.tga`, brick textures, all stretch 1): unused by this client.

Roles: item 0 = base colour A (weight = mask alpha), item 1 = base colour B (weight = 1 − mask alpha),
item 2 = blend mask (only its alpha is used), item 3 = unused.

#### Blend factor (verified 2026-11-06)

The stage object's state block (`stage +0xc`) caches D3D texture stage states at `+0xbc + 4 · D3DTSS_*`
(COLOROP `+0xc0`, COLORARG1 `+0xc4`, COLORARG2 `+0xc8`, ALPHAOP `+0xcc`, ALPHAARG1 `+0xd0`, RESULTARG
`+0x12c`), with "set" flags at `+0x140 + index`. Read in the disassembly of `SetupFourStages` 0x005974f0
(arguments: pass, map0..3, stage0..3, `RET 0x24`):

- stage 1 binds map 3 (item 2) at 0x00597610; COLOROP SELECTARG1 / COLORARG1 CURRENT (0x00597624..0x005976a2),
  **ALPHAOP SELECTARG1 (2) at 0x005976ba, ALPHAARG1 = 2 = D3DTA_TEXTURE at 0x00597705**: no D3DTA_ALPHAREPLICATE
  (0x20) or D3DTA_COMPLEMENT (0x10) modifier, no luminance op, no texture factor or shader constant;
- stage 2 binds map 1 (item 0) at 0x00597751; **COLOROP 0x10 = BLENDCURRENTALPHA at 0x00597765**, COLORARG1
  TEXTURE (0x005977b9), COLORARG2 TEMP (0x00597804; stage 0 wrote item 1 there, RESULTARG 5 at 0x0059751d).

So `a` is the sampled alpha of item 2 and nothing else; its RGB never enters the blend. `Draw` 0x00597210 first
disables COLOROP/ALPHAOP on stages 0..7 and then picks `SetupFourStages` or `SetupTwoStages` from the pass's stage
counts only (+0x98, +0x94 > 3): no code looks at the mask's format. A mask without an alpha channel would sample
as alpha 1 (Direct3D's rule for such formats) and draw item 0 alone.

Data (`Texture.uop`, `build/worldart/%08d.dds`): every default and bump record uses `01000003_noise_alpha.tga`
(id 1000003) as item 2, except material 9, which uses id 1000004. Both are 256×256 DXT5 with one level. Their RGB
is placeholder art (a red smiley face; red "SHARP NOISE" lettering), unrelated to the alpha (correlation of
luma with alpha −0.24 and 0.00). Alpha of 1000003: 15..255, mean 142, 1.9 % of texels at 255. Alpha of
1000004: mostly 0 (43.4 %) or 255 (37.1 %).

### UOBumpMapTerrainLayer (`Draw` 0x00597f00, verified)

`GetMap(0..5)`; maps 0..3 go to the same `SetupFourStages` / `SetupTwoStages` (vftable slots +0x40/+0x44 are
the default ones), with the same argument order (call 0x0059825a / 0x00598892). So items 0..2 have the default
roles. Then, with 6 or more stages, the BUMPENVMAPLUMINANCE cap and both extra stages present:

| stage | map | item | states |
|---|---|---|---|
| 4 | 4 | **3** | BUMPENVMAPLUMINANCE, ARG1 TEXTURE, ARG2 CURRENT, matrix identity, LSCALE 1, LOFFSET 0, alpha SELECTARG1 CURRENT (bind 0x005982a9) |
| 5 | 5 | **4** | TEXCOORDINDEX CAMERASPACEREFLECTIONVECTOR (0x30000), ADD(TEXTURE, CURRENT), alpha CURRENT (bind 0x00598674) |

Roles: item 3 = luminance bump (du/dv) map perturbing item 4, item 4 = environment (reflection) texture added
on top. Data (material 8): item 3 `01000013_water_alpha.tga`, item 4 `01000014_cube0.tga`.

### UOWaterTerrainLayer (`Draw` 0x005988a0, verified)

`GetMap(0..3)`: map0 coverage, map1 item 0, map2 item 1, map3 item 2. The layer draws nothing unless items
0..2 and their textures exist (0x005988ea..0x00598922). It reads technique 0 (and technique 1, which it then
discards, 0x00598a5d) and tweaks 0 and 1 (tweaks 2 and 3 are read and popped unused, 0x00598aa4 / 0x00598ab1).
`t` = float at `*(0x00e3722c) + 0x58`: elapsed time in **seconds** (verified, see "Water scroll time" below).

Path with 4 or more stages (0x00598ac0..):

| stage | map | item | states |
|---|---|---|---|
| 0 | 1 | **0** | texture translation (t · tweak0, t · tweak1) via 0x005101c0; BUMPENVMAP (0x16), ARG1 TEXTURE, ARG2 CURRENT, matrix (1, 0, 0, 1) from 0x00c981c8; alpha SELECTARG1 TEXTURE |
| 1 | 3 | **2** | map flags `|= 0x3000`; TEXCOORDINDEX from technique 0 (jump table 0x005996b4): 1 → the map's own set, 2 → set 0, 3 → CAMERASPACENORMAL, 4 → SPHEREMAP, 0 or other → CAMERASPACEREFLECTIONVECTOR; colour SELECTARG1 TEXTURE, alpha CURRENT |
| 2 | 0 | coverage | colour CURRENT, alpha coverage TEXTURE; **COLOROP DISABLE on pass 0** (0x0059919c / 0x005991bd) |

Fallback (fewer than 4 stages, or a stage missing, unless BUMPENVMAP is supported with ≥ 4 stages, in which
case nothing is drawn; 0x005993c5): stage 0 = **item 1** with the same scroll, SELECTARG1 TEXTURE, alpha CURRENT;
on passes > 0 stage 1 = coverage alpha.

Roles: item 0 = du/dv bump map (scrolled by tweaks 0/1), item 1 = flat colour used only by the fallback,
item 2 = the colour seen on normal hardware, sampled through the bump perturbation with coordinates chosen
by technique 0, item 3 = unused.

| material | items (stretch) | technique 0 → item 2 coordinates |
|---|---|---|
| 5 water | `water_alpha`@16, `water`@4, `cube3`@1 | 0 → reflection vector |
| 10 lava | `lava_normal`@12, `Lava_A`@4, `Lava_A`@6, `Lava_B`@8 | 1 → own set |
| 49 / 168 / 169 acid | `lava_normal`@12, `Acid_A`@8, `Acid_A`@8 | 2 → set 0 |
| 53 stars | `Stars_C`@24, `Stars_A`@0.5, `Stars_A`@0.5, `Stars_B`@24 | 0 → reflection vector |
| 70 swamp water | `Swamp_Water_D`@16, `Swamp_Water_A`@4, `Swamp_Water_C`@2.5, `Swamp_Water_E`@10 | 0 → reflection vector |
| 195 void | `Stars_B`@24, `Black_Void_C`@1, `Black_Void_C`@1, `Stars_B`@24 | 0 → reflection vector |

The "role by position" rule holds for water even where the name says otherwise: `Stars_C`, `Swamp_Water_D` and
`Stars_B` are du/dv maps in position 0.

## 3. Verdicts on the name / repetition heuristics (`lib/uocf/src/enhanced/terrain_definition.rs`)

No terrain code reads a texture path except to build its file location: the factory compares only the shader
name (0x004616e0), `LoadTerrainLayer` 0x00461800 and `LoadTerrainLayerTexture` 0x004613b0 /
`MakeWorldArtTextureLocation` 0x004603f0 do not test the path, and the draws only use map positions (verified).

| heuristic | verdict | evidence |
|---|---|---|
| `is_support_layer_by_current_name_heuristic` (noise, normal, mask, `_alpha`) | **contradicted** (no name test in the client) | roles are by position (§2). It agrees with position 2 in all 192 default layers only because every one uses `01000003_noise_alpha*`; in water layers it marks the position-0 du/dv map only when it is called `lava_normal`/`water_alpha` (5 of 8) and misses `Stars_C`, `Swamp_Water_D`, `Stars_B`; in the bump layer it marks item 3 but not item 4 (`cube0`) |
| `has_support_like_name_clue` (alpha, mask, noise, normal, bump, ripple, flow, distort) | **contradicted** (no name test) | same; `cube*`, the only environment maps, carry none of these words |
| `has_preferred_primary_repetition` (stretch 4.0..=8.0) | **contradicted** (no evidence of any range rule) | stretch is only the UV divisor `1 / stretch` (0x004613b0). The range moves the "primary" to item 1 in 22 default records (e.g. 1 grass: item 0 stretch 10; 6 rock 9; 13 and 171..189 cave floor 12), where the client's base colour is still item 0 |
| `current_primary_selection_reason` | **no counterpart** | the client has no primary-layer selection step |

## 4. Primary layer and stretch

- There is no "primary" field or selection in the client. What the client itself draws as a single colour is:
  - `UODefaultTerrainLayer`, `UOBumpMapTerrainLayer`: **item 0** (the < 4-stage path, 0x00597b50; and the
    colour where the mask alpha is 1 on the full path);
  - `UOWaterTerrainLayer`: **item 1** (the fallback path, 0x005993da); on the full path the visible colour is
    item 2 seen through the item-0 du/dv perturbation.
- Stretch: the item's texture transform scale is `1 / stretch` on u and v (verified, 0x004613b0). Inference:
  with the terrain mesh's set-0 coordinates running one unit per cell, a texture repeats every `stretch` cells.
  Nothing selects on its value.

## 5. "Smooth"

- No such flag in the client (verified): the record fields are name, id, `field_34`, `coverage_exponent`,
  `field_3c`, aliases `{subtype, legacy_land_id, flags}` and the shader block; `field_34` and `field_3c` have no
  reader (`ghidra-terrain`), the alias flag bits have no smooth bit, and no terrain function references a
  "smooth" string. The only strings in the binary are `m_bSmooth` (0x00ccc868, one reference at 0x006d9603
  outside any defined function, among Gamebryo object vftables: an engine property name, inference) and the
  unreferenced font-hinting names `smooth`, `smooth-lcd`, `smooth-lcdv`.
- The client's softness control is `coverage_exponent`: 0 gives a hard coverage edge, point filtering and
  bit 0x100 clear; larger values give the 255 · 0.95^b / 255 · 0.6^b falloff and bilinear filtering
  (`BuildTerrainCoverage` 0x00461060, `LoadTerrainLayer` 0x00461831).
- Neither repo flag maps onto it: the old KDL "Smooth" set {1-4, 6-9, 13-16, 20, 21} all have exponent 1, but so
  do 17, 18, 19 without it; `policy "smooth"` in `EcTerrainOverrides.kdl` is on 5, 10, 49, 53, 70, 195, all
  `UOWaterTerrainLayer`, with exponents 0, 5, 1, 0, 0.15, 1.

## 6. A material with no definition; "terrain 201"

- `UOTerrain::GetTerrainDefinition` 0x004cc990 (verified) returns a null definition when the resource request
  state is 1 (0x004cca3d; inference: "failed"). `BuildTerrainMesh` 0x00461ae0 takes the cell material as a
  **byte** (0x00461b84), calls the lookup (0x00461b92) and reads `definition +0x8` (the alias vector) at
  0x00461b9b with no null test; with no subtype-0 alias it reads `[0 + 0xc]` at 0x00461bb6. The factory default
  (`TerrainDefinitionFactory_CreateDefault` 0x004cc5f0) is an empty definition without aliases, so it fails
  the same way. **The client draws nothing for such a material: it faults.** The shipped data never triggers it
  (all 2705 csv rows resolve).
- Terrain 201 is not a client material. The client's own `legacyterrainmap.csv` routes classic land **774 and 777
  to (64, 0)**, "Snow NE-SW" (with 381, 382, 1902; 10064 → (64, 1)). Material 64's items are
  `02000640_Snow_Cliff_NEtoSW_A.tga`@5, `02000641_Snow_Cliff_NEtoSW_B.tga`@5, `01000003_noise_alpha.tga`@16,
  the same textures and stretch as the repo's terrain-201 override (t0 2000640, t1 2000641, stretch 5). So 201
  is a repo-side duplicate of material 64.

## 7. Replacement rule for the repo

1. Class from `shader_name` exactly as `CreateTerrainLayerForShader`: `UOWaterTerrainLayer` / `UOWaterShader` →
   water; `UOBumpMapTerrainLayer` → bump; everything else → default.
2. Roles by item position, never by path:
   - default and bump: 0 base A, 1 base B, 2 blend mask (alpha only), colour = lerp(item1, item0, mask.a);
     default item 3 unused; bump 3 luminance du/dv, 4 reflection environment (added);
   - water: 0 du/dv (scroll = tweaks 0, 1 per time unit), 1 fallback colour, 2 main colour (coordinates from
     technique 0), 3 unused.
3. Single "primary" (for previews, atlases or radar colours): item 0 for default/bump, item 1 for water.
4. UV scale per item = `1 / stretch`; no range rule.
5. Edge softness from `coverage_exponent` (0 = hard edge, point filter); drop the "smooth" flag/policy as a
   client fact (keep it only as an explicit repo-side choice).
6. Route classic land through `legacyterrainmap.csv`; 774/777 → material 64 subtype 0. A material without a
   definition is invalid data (the client faults), so a converter should reject it rather than invent one.

### Water scroll time (verified 2026-10-31)

- `0x00e3722c` is the `UOApplication` singleton (created by 0x0047c560 → constructor 0x004833d0, which sets
  `UOApplication::vftable` and zeroes +0x58).
- The only stores to +0x58 from its vftable methods and their callees are in 0x00482530 (vftable slot 1, the
  main loop; probe `UoAppTimeWriterProbe20261031.java`). On entry it sets +0x58 = 0x0099a814() (a QPC tick
  count × the float at 0x00e32240: a start phase). Each frame it computes `local_28` = milliseconds since the
  last frame (0x0099a7c7 returns QPC ticks / (frequency / 10000), i.e. 0.1 ms units, divided by 10; the same
  value feeds `Sleep(1000 / fps - local_28)`), stores +0x5c = `local_28 / 1000.0` (frame seconds) and ends the
  frame with +0x58 += +0x5c.
- So `t` is seconds since start (plus a constant phase) and the scroll is `tweak` texture repeats per second:
  water 5 at -0.01 moves one repeat in 100 s.
- 0x005101c0 copies two floats to offsets 0 and 4 of its target and sets the byte at +0x1c when they change:
  the texture transform's translation setter with a dirty flag.

## Open items

- Technique 1 of water layers (values 1, 2, 3) is read and discarded in `Draw`; another reader (for example the
  `UOTerrainShader` pass count 0x00596c80) was not traced.
- The meaning of map flag bits 0x3000 / 0x3100 / 0x100 (filter and address modes, from the coverage behaviour)
  is inference.
- Which pass index each layer draws in (pass 0 opaque, then blended passes) is from
  `ec_original_water_code.md`, not re-traced here.
