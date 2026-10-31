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
static namespace. With the inspected packed atlas and named asset ownership:

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
packed static owners. Neither count proves that an existing atlas was rebuilt:
the inspected installed package still lacks all 22 misnamed-owner slots. Rebuild
EC art and tilemeta together before claiming those repaired source owners render.

Focused catalog tests cover two named assets sharing a stored ID, a creature ID
that would overwrite a real static after truncation, and malformed named data.
The full original-source inventory checks each resolved owner's stored ID.

The raw source explicitly distinguishes `world_art_scale` and
`legacy_art_scale`. The source parser preserves them, but `TileArtEntry::process`
currently drops them from `ArtData`; neither packed sprite slots nor runtime
geometry carry them. Conversion chooses world pixels when an EC base and world
package are available, otherwise legacy pixels. Runtime applies one 64-pixel EC
normalization to either source. This is a source/geometry metadata gap, not a
reason to substitute different artwork.

## Scale and window evidence

Read-only Ghidra checks at `/UOSA.exe:0x4592e0` and `0x459b00` confirm the
[ordinary sprite findings](ec_original_water_code.md#ordinary-sprite-windows-and-legacy-artwork).
The image-window builder uses `0.021427423 * descriptor_scale` on both
camera-plane axes. Default legacy scale 1.5 corresponds to the calibrated
legacy pixel unit. Thus a selected source pixel has relative size
`descriptor_scale / 1.5` in that unit. This is not the camera-quad material's
world repetition/stretch scale.

For a default world-art sprite, the original ratio is 2/3; current `44/64`
normalization is 3.125 percent larger. A default legacy-source sprite retains
full legacy pixel size; applying `44/64` instead shrinks it by 31.25 percent.
Non-default world-art scales require per-owner metadata, so changing 64 to 66
globally would leave most of the source distinction unresolved.

The original mode-1 world window adds one to both end coordinates. The
converter's `ArtTexture`/clip rectangle currently treats raw ends as exclusive.
Packed alpha trims, image extent clamps and aliases must be checked before
changing this independently of scale. Examples 1993 and 1874 have raw world
windows 67x73 and 68x93 and packed logical extents matching those raw differences;
the ordinary source-window path must stay separate from the bit-34 camera quad.

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
