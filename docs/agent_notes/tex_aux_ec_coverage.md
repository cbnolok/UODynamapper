# tex_aux_ec coverage and the tilemeta PRIMARY_SELECTED flag

## What `tex_aux_ec` packs

`udd-cli pack pack-ec-tex-aux` walks every tilemeta item texture ref. It keeps the ones that
are not `PRIMARY_SELECTED` and whose family is in `TEX_AUX_EC_PACKED_FAMILIES` (WorldArt only),
and decodes them from `Texture.uop`. Slots are keyed by the texture number alone,
and the first ref seen wins.

The package also requires `metadata/water_primary_extents.bin`: sorted little-endian
`u32 [texture_id, width, height]` rows for block-0 `UOWaterShader` map-1 primaries.
These are full native source dimensions, recorded before optional upscaling, not
new image slots. The primary pixels remain in `tex_art_ec`; the runtime does not
decode/upload an otherwise unused fallback image. Rebuild the auxiliary package
with the current packer if the reader reports missing extent metadata.

TileArtLegacy/TileArtEnhanced refs are counted as "unread family" and skipped. They are the EC
legacy-art mode, which the project does not convert; Classic art comes from the CC files. Before
this rule they made up 38,771 of 39,620 slots (80 MB, against 6.3 MB now). They also shadowed
same-number WorldArt images: 1401 was packed as a 64x64 legacy image instead of the 256x256
WorldArt image.

The runtime consumers in dynapper are the EC item liquid (UOWaterShader sampler 3), the static
terrain surface (UOStaticTerrainShader samplers 1-3) and the sprite hue mask (UOSpriteShader
sampler 2). All of them read block-0 WorldArt refs from `Texture.uop` and check
`slot.logical_family == 1`.

## PRIMARY_SELECTED (lib/udd-conv/src/metadata/tilemeta.rs)

The aux packer skips `PRIMARY_SELECTED` refs (their image is in `tex_art_ec`), so a support ref
must never carry the flag. Since tilemeta format 2 the role does not depend on the flag: it comes
from the block shader and the item position.

- **Block 0 (the EC art set).** Only the item at the shader's primary position (1 for
  `UOWaterShader`, 0 otherwise) whose id is `ec_texture_id` is primary. Terrain
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

- File-name roles are gone (tilemeta format 2). UOSA.exe never reads a role from a file name, only
  (block shader, item position); see
  `scripts/dev-helpers/ec-research/ghidra-texture-roles/results/uosa_texture_roles.md` §5. The
  name rules were wrong at scale: the `"_n"` rule alone matched 85,504 refs, none a normal map
  (`01000045_noise`, `_NO_DRAW`, `_nautilus`, ...).
- Water primaries. `UOWaterShader` binds the surface at position 1 and the bump map at position 0.
  Format 1 made position 0 primary for 155 water owners (4651.., 5701, 7367..); format 2 takes
  position 1. 133 ids that format 1 packed as aux images are now only primary images (in
  `tex_art_ec`), and the bump map 1000020 joined the aux package. Pixels of the 650 common ids
  are unchanged.
  The primary's dimensions still matter: `UOTileArt::BuildImageWindowQuad` at
  `0x459b00` divides the authored pixel window by the full primary texture size.
  For example, item 7379 has a 64x28 packed art crop but a 64x64 primary in the
  canonical `Texture.uop`; that primary is not an auxiliary image. Neither the
  trimmed crop nor a differently numbered fourth-stage mask proves its full
  dimensions. The extent sidecar preserves this information without duplicating
  primary pixels or changing texture roles.
- Unavailable sources: tileart names WorldArt ids that `Texture.uop` lacks. Of the non-base ones
  the plain client has 19, among them the hue masks 2003211, 2003212, 20003979, 20004038 and 20004039.
  - When a texture request returns no data, UOSA.exe's tile material builder (FUN_0045cd80,
    0x0045ddf8 and 0x0045e42e) asks again for `Data\SystemTextures\missing.tga`, a 256x256
    opaque magenta pattern. A decode failure instead falls back to Gamebryo's default texture.
  - `tex_aux_ec` mirrors this. It stores `missing.tga` once under `TEX_AUX_EC_MISSING_TEXTURE_ID`
    (`u32::MAX`), and each absent id is an alias slot of it (`TEX_AUX_EC_SLOT_FLAG_MISSING_SOURCE`).
    A normal `slot_for_texture_id`/`load_rgba` returns the magenta image for those ids; the plain
    client has 19 (9 of them sprite hue masks, which now resolve; the hue crop audit went from
    331 available + 9 missing to 340 available).
- Two installed EC clients differ: `_Ultima Online Enhanced` (canonical, and the one Ghidra
  analyses) and `... fp`, a fully patched newer copy with a larger tileart, Texture and
  LegacyTexture. An aux package built from one client is not a regression baseline for the
  other. The two clients' LegacyTexture images also differ at the same
  ids, e.g. 4072.., 41718.. (different sizes).


## Runtime integration

The art page atlas qualifies each page by source; Classic fallback cannot alias
a KR page with the same number, including during growth and reupload. Artwork
availability is checked in the actual packages before selecting wetness, depth,
scale, anchor and CPU alpha. See [the artwork contract](ec_sprite_descriptor_geometry.md#kr-only-artwork-and-runtime-fallback).
Auxiliary image decoding, hue crops and full upload layers are keyed by
`payload_texture_id`; original requested IDs and family checks are retained.
The real-package regression checks all missing-source aliases share one layer
with the original reserved image's pixels. The canonical crop audit admits
11,673 sprites and resolves all 340 declared masks (313 distinct native crops).
Nine of these bindings intentionally sample the client's missing image.

A second data issue was found during native acceptance: installed Classic art
omitted book 39297 even though the selected original Classic UOP contains its
44x39 image and matching metadata. Rebuilding with the unchanged converter adds
4,883 slots, loses none, and brings all 39,386 resulting slots into exact RGBA
and native-anchor agreement with that original source. 499 old images and 466
old geometry records differ; this is an explicitly source-validated replacement,
not an image-preservation claim. The source comparison helpers in
`tools/udd-conv-cli/examples/` audit every slot independently of atlas packing.
