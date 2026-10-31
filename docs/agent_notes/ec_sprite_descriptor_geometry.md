# EC sprite source geometry

## Original source and packed inventory

`inspect_ec_sprite_geometry` reads original `tileart.uop` records and compares
their source descriptors with present `tex_art_ec.uddp` slots. It refuses to
overwrite evidence. Run it with a matching client/package pair:

```sh
cargo run -p udd-conv-cli --example inspect_ec_sprite_geometry -- \
  --client /path/to/enhanced-client \
  --package /path/to/tex_art_ec.uddp \
  --output /tmp/ec-sprite-geometry.json
```

The installed EC source has 37756 records, including 36611 assets in the 16-bit
static namespace. The earlier, pre-rebuild atlas had this named owner coverage:

| Packed family/source | Owners |
| --- | ---: |
| Ordinary sprite / world art | 11549 |
| Ordinary sprite / legacy art | 20158 |
| Authored camera quad / world art | 1606 |
| Other material surface / world art | 41 |

Of the ordinary world-art sprites, 11112 use scale 1.0 and 437 use other authored
scales. All 20158 selected legacy-source sprites in this atlas use legacy scale
1.5. The full original record set also contains other legacy scales; those
counts must not be confused with the packed ordinary-sprite population.

### Asset names are authoritative

Original records contain nine repeated stored IDs; 22 named static assets store
another ID. `build/tileart/%08d.bin`, hashed through `tileart_entry_hash`, identifies
the requested asset. `ArtDefinition::from_package` now loads that 16-bit namespace
by entry hash and sets `ArtData.id` to the requested asset. `source_tile_id`
retains the original 32-bit field. Malformed named records fail with asset context
instead of silently becoming missing art.

The old raw-ID catalog omitted 21 named static IDs, including 43957..43963 and
43965..43974. It also admitted 29 IDs with no corresponding named static entry
through truncation of large creature placeholder IDs. Large creature records
could also overwrite real static owners after truncation. The 1145
out-of-namespace records belong outside this static catalog.
The corrected lookup does not admit those records as static assets. This is an
asset-namespace correction; creature records remain available through the raw
32-bit parser.

The earlier raw-record inventory counted 20179 packed legacy occurrences because
it counted repeated stored IDs. Named ownership reduces that to 20158 distinct
packed static owners. At that checkpoint the atlas lacked all 22 misnamed-owner
slots. The matching art/tilemeta rebuild below now packs 21 of them; asset 39777
still has no selected image base and is not filled with guessed artwork.

Focused catalog tests cover two named assets sharing a stored ID, a creature ID
that would overwrite a real static after truncation, and malformed named data.
The full original-source inventory checks each resolved owner's stored ID.

The raw source explicitly distinguishes `world_art_scale` and
`legacy_art_scale`. Before the repair, high-level processing dropped those
scales and runtime applied one 64-pixel normalization to both world and legacy
pixels. `ArtDefinition::sprite_descriptors` now preserves the source ID, both
scales and both original six-word windows under the named owner.

## Scale and window evidence

Read-only Ghidra checks at `/UOSA.exe:0x4592e0` and `0x459b00` confirm the
[ordinary sprite findings](ec_original_water_code.md#ordinary-sprite-windows-and-legacy-artwork).
The image-window builder uses `0.021427423 * descriptor_scale` on both
camera-plane axes. Default legacy scale 1.5 corresponds to the calibrated
legacy pixel unit. Thus a selected source pixel has relative size
`descriptor_scale / 1.5` in that unit. This is not the camera-quad material's
world repetition/stretch scale.

For a default world-art sprite, the original ratio is 2/3; the former `44/64`
normalization is 3.125 percent larger. A default legacy-source sprite retains
full legacy pixel size; applying `44/64` instead shrinks it by 31.25 percent.
Non-default world-art scales require per-owner metadata, so changing 64 to 66
globally would leave most of the source distinction unresolved.

The original mode-1 world window adds one to both end coordinates. Source
images for planks 1993 and stairs 1874 confirm visible alpha in the formerly
omitted final row/column: 7/2 and 4/4 pixels respectively. The converter now
includes both end pixels for positive ordinary world-art windows. Their packed
extents become 68x74 and 69x94, and match the original source crops byte for byte.
Legacy windows keep their existing exclusive ends. Degenerate windows,
material surfaces and the separately flagged bit-34 camera quad retain their
own handling; their policies are not established by this ordinary-sprite proof.

## Required package geometry and runtime bounds

EC art metadata version 7 requires `metadata/sprite_geometry.json`. Each present
slot has its named owner's original descriptor, actual selected source family
and texture ID. The converter shares the source decision with image decoding:
if the world package is unavailable and legacy pixels are packed instead, the
record carries legacy scale. Aliased image rectangles retain separate owner
records and can therefore have different scales. Inferring this from the
logical EC texture ID alone is unsafe.

The reader rejects missing geometry or a nonpositive/nonfinite selected scale
with a rebuild diagnostic. It does not invent a scale for an older package.
Collection passes `descriptor_scale / 1.5` relative to the calibrated legacy
pixel unit into ordinary billboard bounds. Picking reads the same per-owner
value. CC art uses ratio 1; camera/material quads keep their existing world-cell
or authored camera geometry. The GPU instance layout, shader bias and logical
XYZ/raw-Z/height/movement metadata are unchanged by this bounds correction.

The rebuilt atlas has 34616 present owners: 11664 ordinary world-art sprites,
21305 ordinary legacy-source sprites, 1606 camera quads and 41 other surfaces.
436 of the ordinary world-art owners have non-default scale. The old atlas's
counts above describe a different coverage set, not a scale fallback rule.

Rebuild matching art and tilemeta without regenerating the unrelated terrain
atlas, writing to a new directory:

```sh
cargo run -p udd-conv-cli --example rebuild_ec_sprite_assets -- \
  --ec-client /path/to/enhanced-client --cc-client /path/to/classic-client \
  --output /new/output/directory
```

The helper emits lossless RGBA/Zstd pages and validates matching metadata. Use
both output files together. The inventory supports repeated `--verify-pixels ID`
with matching `--tilemeta` for exact original-source comparisons of native-size
ordinary art, including mandatory alpha trims, source origins and placement
offsets.
`find_hued_statics --include-unhued --ec-source legacy` (or `scaled-world`)
locates actual placements from packed source metadata.

## Native acceptance and scope

The 0.8-scale banana tree at map 0 `(1046,2253)` exercises a non-default world
scale. Legacy crane assemblies at `(500,768)` and `(4569,2308)` visibly join
after applying their native pixel unit; an identical-image control restoring
the old 64-pixel treatment separates their parts. The first legacy fixture at
`(285,977)` is roof-occluded and is not positive evidence for that scale.

Forty-two native frames from thirteen verified Desktop-4 runs cover three styles,
perspective, matched teleport histories, independent scale/window controls and
projected shadows. Eight Classic/Enhanced CC-art frames exactly match earlier
town, bridge, road and lava baselines. Corrected KR town/bridge histories also
match exactly. An installed-package smoke test also matches the isolated final
town/bridge output exactly. App-only picking selects raw crane piece `(500,769)`, graphic
18911, Z=0 and plant `(1046,2253)`, graphic 3240, Z=0 with matching quad bounds;
OS focus and pointer are unchanged. Two additional picking captures are retained.
123 focused tests pass; two adapter-dependent tests remain ignored. Four sampled
packed sprites have exact source RGBA/window matches. Acceptance evidence is
`sprite-geometry-accepted-evidence.json` under `/tmp/uodynamapper-ec-followup`.

This establishes source-specific scales and positive ordinary window edges.
Every individual anchor, original extent-rounding/sampler choice, degenerate
window, arbitrary-angle baked-art projection and visible-alpha/depth picking
still needs its own evidence. The original supplied scenes use legacy artwork
and cannot establish pixel parity for newly selected world art.

`UseNewArt` is tested in **flags2**, not the logical flags1 table. Original
non-legacy descriptor selection chooses mode 1 for 795 records, mode 2 for zero,
and mode 0 for 36961. The viewer's explicit EC-art source currently admits world
art more broadly. Do not silently change that source policy while correcting
its geometry, or claim that every admitted world-art sprite follows the
original client's default selection. The user-supplied EC reference screenshots
use legacy artwork, so they cannot establish pixel parity for new world art.

Preserve logical XYZ, signed raw Z, heights and movement flags. Source pixel
scales/windows, visual bounds and painter priority remain rendering metadata.

The source/slot inventory and original-function snapshots are retained under
`/tmp/uodynamapper-ec-followup/`. This inventory establishes authored metadata
and packed coverage; it does not establish actual runtime routing, trims or
visible pixel parity by itself.

## Mandatory-trim handoff

The subsequent converter change makes alpha trimming unconditional after the
owner's source window. Tilemeta `ec_start_x/y` are the resulting source-space
left/top, not billboard offsets. Slot X adds the left trim; slot Y subtracts
the bottom trim. The runtime must consume a matching art/tilemeta pair; it
must not reapply either correction. See [the format contract](../dev_wiki/rendering/tex_art_ec_trimming.md).

Independent source checks cover bridge pieces 1993/1874, scaled plant 3240,
legacy owner 2739 and shifted hue-mask owners 3239/5118/11532. All seven packed
images match original RGBA exactly, with independently derived trim bounds and
placement corrections. All six world-art source origins match tilemeta. The
banana tree's original positive window is 274x220; its packed 273x218 image
removes one right column and two bottom rows, with draw offset `[-94,-14]`.
The earlier window/geometry acceptance above used an untrimmed atlas.

`inspect_ec_sprite_hue_crops` audits the runtime's actual source gates, decoded
auxiliary images and crop bounds. A count of changed source starts is not a
count of recovered masks: many entries lack a supported UOSprite source-2
binding, and missing source images remain distinct from invalid crop bounds.

Fourteen matched native placement frames cover the scaled plant, both legacy
cranes, town, road and perspective bridge/town. Eight additional frames disable
optional UV-normalized art shading to separate it from placement. The unlit
crane differs at only six edge pixels; source-space alignment remains intact.
Trimming changes normalized shading coordinates and boundary filtering, so
the full shaded frames are not byte-identical. Do not describe those changes
as an anchor correction or claim original-client sampler parity. The matching
pair and preservation-safe auxiliary extension are installed with verified
backups; see [the hue-mask integration](ec_hue_palette_conversion.md#sprite-mask-crops-and-auxiliary-coverage).
