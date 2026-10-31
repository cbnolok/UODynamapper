# tex_aux_ec coverage and the tilemeta PRIMARY_SELECTED flag

## What `tex_aux_ec` packs

`udd-cli pack pack-ec-tex-aux` walks every tilemeta item texture ref. It keeps the ones whose
`stable_role` is not `Base`/`SecondaryBase` and whose family is in `TEX_AUX_EC_PACKED_FAMILIES`
(WorldArt only), and decodes them from `Texture.uop`. Slots are keyed by the texture number alone,
and the first ref seen wins.

TileArtLegacy/TileArtEnhanced refs are counted as "unread family" and skipped. They are the EC
legacy-art mode, which the project does not convert; Classic art comes from the CC files. Before
this rule they made up 38,771 of 39,620 slots (80 MB, against 6.3 MB now). They also shadowed
same-number WorldArt images: 1401 was packed as a 64x64 legacy image instead of the 256x256
WorldArt image.

The runtime consumers in dynapper are the EC item liquid (UOWaterShader sampler 3), the static
terrain surface (UOStaticTerrainShader samplers 1-3) and the sprite hue mask (UOSpriteShader
sampler 2). All of them read block-0 WorldArt refs from `Texture.uop` and check
`slot.logical_family == 1`.

## PRIMARY_SELECTED (lib/udd-conv/src/tilemeta.rs)

`stable_role_for_tileart_texture_ref` gives `Base` to any ref that has `PRIMARY_SELECTED` and no
path role, and the aux packer skips `Base`. A support ref must therefore never carry the flag.

- **Block 0 (the EC art set).** Only the item `ec_texture_id` was selected from is primary: the
  first non-auxiliary matching item, or the first matching item if all are auxiliary. Terrain
  statics reuse the base image number on another sampler, e.g. stone roofs 1364.. use it on
  sampler 2. Flagging by number alone turned those refs into `Base`, so they were left out of the
  aux package and dynapper dropped the whole UOStaticTerrainShader graph for 51 owners (roofs,
  carpets 2749.., wood floors 9253.., sunken log 12878).
- **Blocks 1 and 2 (TileArtLegacy / TileArtEnhanced art sets).** An item with the same number is
  that set's counterpart of the primary image and stays primary.
  `main_ec_texture_id_role_aware` / `_with_reason` (`role_base*` branches) rely on this. Without
  it, 8 owners fall to the 20000000 grunge image (4039, 6373..). The aux packer skips these
  families anyway.

## Gotchas

- `stable_role_from_tileart_texture_path` no longer has a `"_n"` rule. It matched 85,504 refs, none
  a normal map: `01000045_noise` (85,140), `*_noise_alpha`, and names such as `_NO_DRAW`,
  `_nautilus`, `_Nightshade`, `_bed_of_nails`. With it, 165 primary images had `NormalLike`
  instead of `Base` and were packed as aux images (68 slots), and 7 of them were listed as absent
  sources. The other name rules are still heuristics: UOSA.exe never reads a role from a file name,
  only (block shader, item position); see
  `scripts/dev-helpers/ec-research/ghidra-texture-roles/results/uosa_texture_roles.md`.
- Unavailable sources: tileart names WorldArt ids that `Texture.uop` lacks. Of the non-base ones
  the plain client has 21, among them the hue masks 2003211, 2003212, 20003979, 20004038 and 20004039.
  - When a texture request returns no data, UOSA.exe's tile material builder (FUN_0045cd80,
    0x0045ddf8 and 0x0045e42e) asks again for `Data\SystemTextures\missing.tga`, a 256x256
    opaque magenta pattern. A decode failure instead falls back to Gamebryo's default texture.
  - `tex_aux_ec` mirrors this. It stores `missing.tga` under `TEX_AUX_EC_MISSING_TEXTURE_ID`
    (`u32::MAX`) and lists the absent ids in `metadata/absent_sources.bin`.
    `TexAuxEcPackage::get_or_missing(id, family)` returns the fallback only for listed ids.
- Two installed EC clients differ: `_Ultima Online Enhanced` (canonical, and the one Ghidra
  analyses) and `... fp`, a fully patched newer copy with a larger tileart, Texture and
  LegacyTexture. An aux package built from one client is not a regression baseline for the
  other. The two clients' LegacyTexture images also differ at the same
  ids, e.g. 4072.., 41718.. (different sizes).
