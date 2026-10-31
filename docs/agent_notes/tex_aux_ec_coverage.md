# tex_aux_ec coverage and the tilemeta PRIMARY_SELECTED flag

## What `tex_aux_ec` packs

`udd-cli pack pack-ec-tex-aux` walks every tilemeta item texture ref and keeps the ones whose
`stable_role` is not `Base`/`SecondaryBase` and whose physical package is known. It decodes
WorldArt from `Texture.uop` and TileArtLegacy from `LegacyTexture.uop`. Slots are keyed by the
texture number alone, and the first ref seen wins.

The runtime consumers in dynapper are the EC item liquid (UOWaterShader sampler 3), the static
terrain surface (UOStaticTerrainShader samplers 1-3) and the sprite hue mask (UOSpriteShader
sampler 2). All of them read block-0 WorldArt refs from `Texture.uop` and check
`slot.logical_family == 1`. Nothing reads TileArtLegacy (family 2) aux slots. They are most of
the package, but no current consumer uses them.

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
  that set's counterpart of the primary image and stays primary. Two things rely on this:
  - `main_ec_texture_id_role_aware` / `_with_reason` (`role_base*` branches);
  - the aux `Base` exclusion. Without it, about 10k legacy base images move into `tex_aux_ec`.

## Gotchas

- `stable_role_from_tileart_texture_path` uses `stem.contains("_n")` for `NormalLike`. In the
  current tileart no stem matches a real `_n`/`_nrm` normal-map segment. The hits are words such
  as `_nautilus`, `_nightshade`, `_no-dachi` and `01000045_noise`, so noise is classed as
  `NormalLike` rather than `Noise`.
- Unavailable masks: tileart names some hue masks that `Texture.uop` lacks, e.g. 2003211,
  2003212, 20003979, 20004038 and 20004039. When a texture request returns no data, UOSA.exe's
  tile material builder (FUN_0045cd80, 0x0045ddf8 and 0x0045e42e) asks again for
  `Data\SystemTextures\missing.tga`, a 256x256 opaque magenta pattern. A decode failure instead
  falls back to Gamebryo's default texture.
- Two installed EC clients differ: `_Ultima Online Enhanced` and the newer `... fp`, which has a
  larger tileart, Texture and LegacyTexture. An aux package built from one client is not a
  regression baseline for the other. The two clients' LegacyTexture images also differ at the same
  ids, e.g. 4072.., 41718.. (different sizes).
