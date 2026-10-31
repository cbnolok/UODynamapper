# Texture roles in UOSA.exe

Program: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.

Question: does the Enhanced Client infer a tileart texture's role from its file name or path? Or does it
bind each texture item to a shader input by data (block, shader name, item position, flags)? The repo rule
under test is `stable_role_from_tileart_texture_path` (`lib/udd-conv/src/tilemeta.rs`). It checks the
lowercased file stem in this order: "normal" or "_n" -> NormalLike, "alpha" -> AlphaMask, "mask" ->
GenericMask, "noise" -> Noise, "detail" -> Detail, "light" or "glow" -> Overlay.

Answer: no. The client never tests a texture file name to pick a role. The role of an item is fixed by
(shader name of its block, item position), and the C++ code of each shader class reads its inputs by fixed
shader map index. "Verified" below means read in the disassembly or decompilation of this build. Items taken
from earlier notes are marked as such.

## 1. Binding chain

| Step | Code | What decides | Conf. |
|---|---|---|---|
| Block | `UOTileArt::BuildSpriteDescriptor` 0x004592e0, `UOObjectDefinition::GetPrimaryTextureLocation` 0x004c9050 | block 0 (WorldArt), 1 (TileArtLegacy) or 2 (TileArtEnhanced) by the record flags and the legacy setting; block 3 feeds the lighting component, created only when the light shader name is set (`ghidra-tileart` 4.1) | verified earlier |
| Shader class | `BuildShaderDefinitionMaterial` 0x0045cd80 | `NiShaderFactory` (0x00e31ffc) vt+8 `RetrieveShader(GetShaderName(def), GetField04(def), 1)` at 0x0045da8d-0x0045dace; shadow pass `("UOSpriteShaderShadow", 0, 1)` at 0x0045da71; fallback `"UOSpriteShader"` at 0x0045dc49. `field_04` is the shader implementation index (0 in all records) | verified |
| Item -> shader map | same | item 0: `GetTexture(0)` (push 0 at 0x0045dd05) -> `NiTexturingProperty::SetShaderMap(0, map)` (0x0045e09e push 0, call 0x0045e0a5). Items 1..n: loop counter set to 1 at 0x0045e145, `GetTexture(i)` at 0x0045e166, `SetShaderMap(i, map)` at 0x0045e5ac (push EDI = i), loop test 0x0045e5e8 | verified |
| Map fields | same | `uv_set` -> low byte of the map flags (0x0045de94, 0x0045e1ec); `stretch` != 1 -> wrap and UV scale 1/stretch (0x0045e200); `source_kind` 0/2 load, other values skip (0x0045e23f); missing file -> `missing.tga` (0x0045ddf8, 0x0045e42e). The path is passed only to `SpriteTextureFileLocation::ConstructFromPath` and the texture loader | verified |
| Base map | same | `Build\Hues\Hues.dds` (0x0045e5fc) -> `ConstructFromPath` -> loader -> `NiTexturingProperty::SetBaseTexture` 0x004169a0 (call 0x0045e666). The base map is the hue atlas, not an item | verified |
| Alpha property | same | `std::string::compare` (`FUN_004229a0`) of the shader name with `"UOWaterShader"` at 0x0045e79f | verified |
| Primary texture | `GetPrimaryTextureLocation` 0x004c9050, `FUN_0045a080` | `GetTexture(shaderName == "UOWaterShader" ? 1 : 0)` (compares at 0x004c908c, 0x0045a575) | verified |
| `sampler_index` | - | never read (`ghidra-tileart` 8.10); the position is the index | verified earlier |

`UOShaderDefinition` (vtable 0x00ca5cbc): +4 `GetTextureCount`, +8 `GetTexture(i)`, +0xc `GetField04`
(implementation index), +0x10 `GetShaderName`, +0x2c `GetTechniqueValues`, +0x30 tweaks. Technique and
Tweaks are attached to the material as named extra data (`"Technique"` 0x0045db3b, `"Tweaks"` 0x0045dbe6).

Corrections to `ghidra-tileart/results/uosa_tileart.md`: item 0 goes to shader map 0, not to the
`NiTexturingProperty` base map (the base map is the hue atlas). `field_04` is the implementation index
passed to `RetrieveShader`, not an unknown.

## 2. What each shader class reads

All four read the shader map array `[property+0x30]` (data `[arr+4]`, count ushort `[arr+0xa]`) with constant
indices behind count checks. None of them touches a texture name.

| Shader (vtable) | Code | Shader map 0 | Shader map 1 | Shader map 2 | Shader map 3 | Base map |
|---|---|---|---|---|---|---|
| `UOSpriteShaderHLSL` (0x00cb425c) | `UOSpriteShaderHLSL::BindSamplers` 0x005940e0 (slot +0x1c): reads at 0x00594101, 0x00594117, 0x00594123 | `SpriteSampler` (+0x120): colour | not read | `HueMaskSampler` (+0x160), `HAS_HUEMASK_TEX` (+0x1a0); `uosprite.psh` uses its `.a` to blend toward the hue | not read | `HueSampler` (+0x140), `HAS_HUE_TEX` (+0x180): hue atlas |
| `UOSpriteShaderFFP` (0x00cb3fbc), also `UOSpriteShaderMob` (0x00cb4064) | `UOSpriteShaderFFP::SetupTextureStages` 0x00592a00 (slot +0xa0): reads at 0x00592a1e, 0x00592a36 | stage 0 (0x00592af2): colour and alpha MODULATE(texture, diffuse) | stage 1 (0x00592d0f), if present: MODULATE(texture, current); else stage 1 disabled | not read | not read | not read |
| `UOWaterShader` (0x00cb46bc) | `UOWaterShader::SetupTextureStages` 0x0059bbd0 (slot +0xa0): reads at 0x0059bbf2, 0x0059bc0a, 0x0059bc23, 0x0059bc3c | bump source, `D3DTOP_BUMPENVMAP` (bound 0x0059be80, op 0x0059bead) | surface on low-capability hardware (0x0059c6b7) | surface with reflection-vector texgen (0x0059c289, 0x30000 at 0x0059c3b9) | optional fourth stage, composition by `Technique[1]` (0x0059c43c) | - |
| `UOStaticTerrainShader` (0x00cb446c) | `UOStaticTerrainShader::SetupTextureStages` 0x00595a00 (slot +0xa0): reads at 0x00595a25, 0x00595a41, 0x00595a5a, 0x00595a73 | RGB A | RGB B | blend alpha: RGB = mix(B, A, map2.a) | coverage alpha | - |

The sprite HLSL parameter names (`SpriteSampler`, `HueSampler`, `HueMaskSampler`, `HAS_HUE_TEX`,
`HAS_HUEMASK_TEX`) are bound by name to the compiled `UOSprite.psh` in `UOSpriteShaderHLSL::LoadPixelShader`
0x00593b30. That binds a C++ handle to a shader parameter. Which texture feeds the handle is fixed in C++ by
shader map index. The water and static terrain operations come from `docs/agent_notes/ec_original_water_code.md`
(stage table and source order 1, 2, 0, 3 for static terrain). This round verified only the fixed-index map reads
listed above. The map-to-stage helper is `BindShaderMapToTextureStage` 0x0058dfc0 (EAX = map: texture,
texture coordinate index, filter, clamp, transform).

Which sprite implementation runs (HLSL or FFP) was not traced. It is probably picked by the implementation
index and the device capabilities.

## 3. String sweep

Probes `~/ghidra_scripts/ProbeTexRoles01.java` to `ProbeTexRoles04.java`.

- Standalone literals: none of `_n`, `_nm`, `_nrm`, `_norm`, `_s`, `_spec`, `spec`, `noise`, `_normal`,
  `_mask`, `_alpha`, `_noise`, `_detail`, `_light`, `_glow`, `_n.tga`, `_n.dds`, `_s.tga`, `_s.dds` exists
  anywhere in the image. `_a` appears twice, unreferenced.
- `normal`, `mask`, `glow` (0x00cfeec0, 0x00cfe1cc, 0x00d021a0) are used by Lua `luaB_costatus`, by
  `UOAssetMap::ParseBitmaskMap` 0x00a74549 (asset-map XML attribute `mask`) and by the figure/decal XML
  parsers `FUN_00afd24d` (`figpart`, `decal`, `diffuse`, `specular`, `glow`, `normal`) and `FUN_00b1a3e2`.
  `FUN_00b1a3e2` builds decal file names `"%sdecals/%d.%s.%s.%s.%s"` from the role words `tint`,
  `specular`, `normal`, `mask`, `glow`, `diffuse`. That is the reverse direction (a role gives a name), it
  belongs to the Mythic figure/character-art code (`characterart`, `sector.dat`), and its caller chain
  (`FUN_00b1a513` <- `FUN_00b1a574`) has no further callers found. It is not reached from tileart.
- `alpha` (0x00ce6310) and `detail` (0x00ce6368): UI XML parsers and the effect texture block parser
  `FUN_008c3430` (`Texture`, `filename`, `scrollU`, `base`, `dark`, `detail`; XML element names).
- `light` (0x00d01e6c): sky dome / sector XML parsers (`FUN_00b09827`, `FUN_00b02f12`, `FUN_00aefeb3`;
  `sky`, `dome`, `lightsource`, `fixtures`, `figpart`, `sector.dat`).
- Capitalised `Normal`, `Detail`, `Glow`, `Bump` belong to Gamebryo (`FUN_006727b0` texture-map viewer
  names, vertex semantic tables `FUN_00ab1590`/`FUN_00ab17f0`). `NormalMap`, `DetailMap`, `GlowMap`,
  `TintMask`, `SpecularMask` (`FUN_00a5e220`, `FUN_00a5f5c0`) and `normalSampler`, `detailSampler`,
  `glowSampler` (`FUN_00a62fd0`) are Mythic material slot and shader parameter names (`mythicshaders`). The
  tileart data names only the UO shaders, so `RetrieveShader` never reaches the Mythic library from tileart.
- C runtime helpers (`strstr`, `strncmp`, `_stricmp`, `_strnicmp`, `strrchr`, `strcmp`, `_mbsicmp`,
  `_splitpath`, `_strlwr`, ascii variants): of all call sites with nearby literals, the only texture ones are
  Gamebryo extension tests (`NiDX9SourceTextureData::LoadTextureFile` `.bmp`/`.tga`/`.dds`, the readers'
  `CanReadFile`). No call is in the tileart, material or shader code.
- Path finder `FUN_0058d9d0` (substring search on a location path): its only callers are
  `SpriteTextureFileLocation::ConstructFromPath` 0x0058d130 / `CopyConstruct` 0x0058d1f0 (`Textures\` -> type
  8, `TileArtLegacy\` -> 0x16, `TileArtEnhanced\` -> 0x1a, else 1) and `IsDirectLoad` 0x0058d270. These
  choose the package and the asset id for loading, not a role.
- Per-function scans of the material builder, the shader binders and the texture request chain (27
  functions): the only literals are shader names, `Technique`, `Tweaks` and the hue atlas path. There are no
  string-helper calls and no comparisons with ASCII immediates.

## 4. Verdicts on the repo name rules

Census: all 65536 tile ids through `uocf-devtools-cli --example ec_research items` (the repo tileart
parser), with the repo rule applied to each path. Counts are texture item references.

| Rule | Client code | Verdict | Data |
|---|---|---|---|
| "normal" or "_n" -> NormalLike | no use of these strings on paths; `normal` only in Lua and the decal parser | no evidence; wrong in effect | "_n" matches 85504 refs. 85140 of them are `01000045_noise.tga` (sprite position 1 in blocks 0-3, 1 static terrain position 1 ref, 1 block 1 position 2 ref). Static terrain blend alphas `*_noise_alpha.tga` (182) also match. So do colour images such as `*_NO_DRAW`, `*_bed_of_nails` and `*_Brick_Red_12rows_NS_*` (174), and hue masks such as `*_Nightshade_mask` and `*_no-dachi_mask` (8). "normal" matches 167 refs, all `*_lava_normal.tga` at water position 0. That slot is the bump source by position: it also holds `*_ripple` (61) and `*_water_alpha` (94). |
| "alpha" -> AlphaMask | `alpha` only in UI XML and the effect parser | no evidence; contradicted | Matches `*_water_alpha.tga`: 94 at water position 0 (bump source) and 5 at sprite position 0 (colour). The real blend alphas `*_noise_alpha.tga` never reach this rule because "_n" matches first. |
| "mask" -> GenericMask | `mask` only in the asset-map and decal parsers | no evidence; a good proxy only | Most matches sit where a mask is read by position: sprite position 2 (`HueMaskSampler`: 352 in block 0, 16154 in block 1), static terrain positions 2/3 (87, 81), water positions 1/3 (36, 59). Misses: 8 sprite colour images (`*_mask_tribal`, `*_feathered_mask`), 249 block 1 position 1 `_mask` refs (map 1, which the HLSL sprite never reads), 1 block 1 position 3 ref (never read), and position 2 masks the rule skips (`*_flax.tga_mask.tga` stem `*_flax`, `*_Grape_Bunches_mast`, the "_n" names). |
| "noise" -> Noise | no `noise` literal | no evidence; dead | Every noise path is `*_noise*`, which "_n" catches first, so the rule never fires. The noise texture is sprite shader map 1: unused by `UOSpriteShaderHLSL`, multiplied in at stage 1 by `UOSpriteShaderFFP`. |
| "detail" -> Detail | `detail` only in the effect parser and Mythic names | no evidence; dead | No tileart path contains "detail". |
| "light" or "glow" -> Overlay | `light` only in sky/sector XML parsers | no evidence; contradicted | Block 3 is the light texture by block index. Of its 1042 position 0 refs only 667 match (`*_light_generic`, `*_light_window`, `*_GoldGlow_Treasure`); 375 do not (`*_Red_Lantern`, `*_CandleA`, `*_firefield`). Wrong matches: 153 sprite colour images (`*_Light_Wooden_Wall`, `*_Lighthouse`) and 8 static terrain RGB refs (`*_Stone_Light_A`). |

## 5. The mapping that replaces the rules

The role is a function of (block, shader name, item position):

| Block | Shader | Pos. 0 | Pos. 1 | Pos. 2 | Pos. 3 |
|---|---|---|---|---|---|
| 0, 1, 2 | `UOSpriteShader` | colour (HLSL `SpriteSampler`; FFP stage 0) | FFP stage 1 multiply; HLSL unused | hue mask, alpha channel (HLSL `HueMaskSampler`) | unused |
| 0 | `UOWaterShader` | bump source (BUMPENVMAP) | low-capability surface; primary texture (`GetPrimaryTextureLocation`) | surface with reflection texgen | fourth stage, composed by `Technique[1]` |
| 0 | `UOStaticTerrainShader` | RGB A | RGB B | blend alpha | coverage alpha |
| 3 | light shader | light sprite texture | sprite map 1 (noise) | - | - |

Census by position (item counts): sprite block 0 has 14608 / 11532 / 364 refs at positions 0 / 1 / 2;
block 1 has 36611 / 36525 / 16155 / 1 at positions 0 / 1 / 2 / 3; block 2 has 36611 / 36611 at positions 0 / 1.
`UOStaticTerrainShader` has 341 / 341 / 341 / 311 and `UOWaterShader` 322 / 322 / 322 / 190. Block 3 has
1042 / 1042.

Block 3 is listed with `UOSpriteShader` by the repo parser. Its shader is drawn by the lighting component,
whose draw path was not traced this round.

## 6. Where the repo's other texture-ref fields come from

- `is_auxiliary` (`lib/uocf/src/enhanced/tileart.rs` `is_auxiliary_texture_path`): inferred from the file
  stem ("noise", "normal", a `_`/`-` segment n/nm/nrm/norm). The client has no such flag; the closest
  client fact is "position > 0".
- `texture_type` / `ec_logical_family` (`EcLogicalFamily::infer_from_path`): the folder name (`data\worldart\`,
  `data\tileartlegacy\`, ...). The client also tests the folder (`ConstructFromPath`), but only to choose the
  package and asset type for loading.
- `sampler_index`: stored by the client, never read (section 1).

## 7. Open

- Whether `UOSpriteShaderHLSL` or `UOSpriteShaderFFP` runs, and the selection rule (implementation index or
  capability).
- The lighting component's draw path for block 3, including whether it reads shader map 1.
- `FUN_0045a080` (primary texture use in sprite descriptors and lighting) is not named. Its role beyond the
  water special case was not traced.
