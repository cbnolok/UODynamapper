# Effect visibility: sources, gates and receiver controls

The reported inactive controls need different receivers. A checked flag alone
does not establish that the current material supplies an input, or that lighting
will preserve a visible difference. Use the effective cached preset: bundled
`settings/shaders/land.toml` also overrides factory defaults. In particular, the
bundled KR morning preset disables projected shadows even though factory KR
morning enables them. This audit changes no preset values or shader strengths.

## Normal layers are material inputs, not arbitrary fourth images

The inspected packed source inventory has 201 terrain definitions: 192
`UODefaultTerrainLayer`, eight `UOWaterTerrainLayer`, and one
`UOBumpMapTerrainLayer` (snow, material 8). Grass, dirt, forest, cobblestone and
rock commonly provide two albedos and a blend mask; they supply no authored
normal map. The liquid material graph has its own bump sampling and composition;
`enable_normal_maps` does not bypass the original liquid path. Classic Gouraud
shading and CC texmaps do not consume the optional terrain normal-map lighting.

`TexLandEcPackage::resolve_material_render_layers()` previously treated every
source index 3 as a normal map. This admitted blue floor artwork (material 89)
and sixteen swamp artwork variants (129 through 144). The resolver now admits
the authored bump family's index 3, liquid graph inputs through their existing
branch, and explicit normal-role overrides. Ordinary fourth images remain
available through raw `resolve_material_layer_slot()` and source metadata;
only their unsupported normal interpretation is removed. Base/detail/mask,
liquid inputs, packed pixels and logical world data are unchanged.

This distinction is supported by the original loader and draw path, rather
than image filenames alone. In the analyzed UOSA.exe, loader `0x461800` appends
generated terrain coverage at `0x461901`–`0x461909`, then the authored sources
in the loop beginning at `0x461920`. The default layer driver `0x597210` reads
property-vector entries 0 through 3 and forwards them to the high-capability
body `0x5974f0` (call at `0x5974b0`). They are coverage plus the first three
authored inputs. A fourth authored image is not a default-terrain normal.
Factory `0x4616e0` independently selects `UOBumpMapTerrainLayer` and its
constructor `0x4600f0`. See [the original terrain/water notes](ec_original_water_code.md)
for the executable identity, coverage contract and separate liquid semantics.
Names in the shared Ghidra project can change; these are build-specific addresses.

The reusable diagnostic is:

```sh
cargo run -p udd-conv-cli --example inspect_ec_land_layers -- \
  --package /path/to/tex_land_ec.uddp \
  --overrides dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl \
  --output /tmp/ec-land-layers.json
```

It preserves authored definitions beside effective roles and present slots.
Omit `--overrides` to inspect embedded policy only. It refuses an existing output.
An available normal slot is still not proof of sampled visible fragments.

## Other effect gates

| Control | Actual scope and prerequisites |
| --- | --- |
| Grunge | Terrain, upright art and ground art; positive `grunge_strength`. Its non-KR response is deliberately much weaker than KR. Current support texture is generated, not established as the original client's support image. |
| KR Transition Grime | Terrain material boundaries, KR post-process profile, Grunge enabled and positive `kr_transition_grime_strength`. Flat CC water and original EC liquid composition are excluded. Setting ordinary grunge strength to zero retains the separately controlled boundary grime. It is not a whole-scene dirt overlay. |
| KR Color Wash | Terrain and art, KR post-process profile, positive `kr_color_wash_strength`; bounded, low-frequency warm/cool and value variation. It is intentionally subtle. |
| Art Projected Shadows | Eligible sprite casters, positive strength, textured art rendering. Ground/flat forms and background/floor depth classes are excluded or suppressed. Empty liquid views are not valid positive controls. These projected sprite silhouettes are distinct from baked art shadows and collected light masks. |

Static mask visibility and local illumination have their own completed checks
in [static light visibility](static_light_visibility.md). Non-KR optional fake
normals have separate [wall validation](art_fake_normal_validation.md).

## Native controls and limits

Native controls use 1280x800, zoom 0.8, orthographic morning presets, map 0,
paused water animation, fog off, and no collected static lights. Sources are
CC art/CC land for Classic, CC legacy art/EC land for Enhanced, and EC art/EC
land for KR. The town reference uses legacy object artwork, so Enhanced is
the relevant object-source comparison; KR object changes are not reference
parity evidence.

Four fixtures isolate the ordinary controls: static cobblestones `(2518,561)`,
town `(1647,1682)`, lava `(5744,2928)` and swamp `(1872,2416)`. Grunge changes
actual pixels in all three styles. Projected shadow strength 0.22 changes town
objects in all three styles, while the lava and swamp shadow controls are exact
matches. KR grime at strength 0.65 darkens material boundaries independently of
ordinary grunge strength; KR wash at 0.55 produces a restrained visible change.
Normal-map controls at strength 0.45 have no change above one byte in these
four views. Their materials do not establish a valid dry normal-map receiver.

Snow at `(4050,500)` supplies the retained bump layer. Under the existing morning
presets its normal-map lighting effect is small: Enhanced changes 518 pixels
above one byte (maximum 12), KR changes 19 (maximum 3). The fragment shading
caps combined ambient/fill/diffuse energy when `grade_params.w` enables
headroom limiting. This can hide most of a normal's diffuse response on already
bright level terrain. An isolated Enhanced receiver pair with diffuse strength
0.5, fill strength 0 and ambient strength 0.10 confirms the active normal path:
728064 pixels change above one byte (maximum 14). Normal strength remains 0.45;
no normal input or preset is replaced to obtain that control. The original
morning lighting and its weak snow response remain unchanged.

Temporary evidence uses `effects-audit-*`, `effects-snow-before-*` and
`effects-role-after-*` under `/tmp/uodynamapper-ec-followup`. The source-role
inventory identifies exactly seventeen removed false normal bindings, with
base/detail/mask/animated roles preserved. Native before/after snow frames are
exact matches in both Enhanced and KR, with normals off and on. Enhanced town
frames are exact; swamp frames differ by at most one byte. All
previews use verified exact-PID Desktop-4 placement and isolated settings.

The complete acceptance set is 76 native frames from 27 owned previews,
15 focused `udd-assets` terrain-package tests, the normal Dynamapper build,
and the final layer-inspector run. The compact report is
`effects-accepted-evidence.json`; the helper entry points are
`scripts/audit_effect_visibility.py` and `scripts/collect_effect_visibility.py`
inside that temporary workspace. Source and package hashes, actual effect
uniforms, settled metadata, placement/cleanup and native differences are retained.

One cross-fixture comparison exposed a separate history-sensitive edge result:
234 KR town tree-edge pixels differ above one byte (maximum 36) between a
direct-to-town launch and the snow-to-swamp-to-town sequence. With matched
history, the old and repaired binaries are byte-identical. The new binary with
explicit isolated overrides reproducing all old normal bindings also matches
the direct repaired frame exactly; a package inventory verifies those bindings.
Thus this is not a normal-role regression. The subsequent
[transparent-batch follow-up](static_depth_ordering.md#history-sensitive-tree-edges)
identifies and repairs insertion-dependent equal-distance sorting. Its matched
town histories are now exact; the separate small bridge-edge case remains open.

These checks establish control reachability and source ownership, not original
client pixel parity, normal orientation/filtering parity, every caster's depth,
or final strength tuning. Do not increase every effect globally to make one
unsuitable receiver visibly respond. Liquid clock/interpolation/filtering,
original support-image provenance and receiver-aware shadows remain separate.
