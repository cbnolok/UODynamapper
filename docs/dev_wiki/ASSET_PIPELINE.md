# Asset Pipeline

This document collects the package, conversion, and inspection details that are too specific for the top-level README.

The runtime renderer consumes converted `.uddp` packages. Source Ultima Online client files are read by conversion tools, classified, packed, and then accessed by `dynamapper` through runtime package readers.

## Main Tools

### `udd-pack`

`udd-pack` builds runtime packages from source assets. It is produced by the `udd-conv-cli` crate.

Common commands:

- `pack-art`: pack Classic Client `art.mul` / `artidx.mul` into `tex_art_cc.uddp`.
- `pack-ec-textures`: pack Enhanced Client static-art and terrain textures into `tex_art_ec.uddp` and `tex_land_ec.uddp`.
- `pack-tilemeta`: pack Classic Client tiledata and Enhanced Client tileart metadata into `tilemeta.uddp`.
- `pack-ec-tex-aux`: pack the EC support images that runtime consumers read (WorldArt hue masks, terrain-surface masks, liquid inputs) from a `tilemeta.uddp`'s texture refs into `tex_aux_ec.uddp`. The package also holds the client's missing-texture fallback; see below.
- `pack-radar`: generate a radar map such as `facet0X.dds` from Classic Client map and statics data.
- `pack-map`: pack Classic Client map data into a block-based `.uddp` package.
- `pack-statics`: pack Classic Client statics data into a block-based `.uddp` package.
- `pack-map-statics`: pack Classic Client map and statics packages together, sharing map dimensions discovered by the map pass.

Relative output paths and default output file names resolve against the current working
directory; absolute paths are used as given. Nothing is written into the client folders
passed as `--ccdir` / `--ecdir` unless an output path points there.

Typical Enhanced Client texture and metadata workflow:

```text
udd-pack pack-ec-textures --ecdir /path/to/ec --art-output tex_art_ec.uddp --land-output tex_land_ec.uddp
udd-pack pack-tilemeta --ccdir /path/to/cc --ecdir /path/to/ec --output tilemeta.uddp
```

Then build the auxiliary images from that tilemeta, with the same EC directory (it also reads `SystemTextures.uop`):

```text
udd-pack pack-ec-tex-aux --ecdir /path/to/ec --tilemeta tilemeta.uddp --zstd --output tex_aux_ec.uddp
```

When building all EC texture outputs plus `tilemeta.uddp`, prefer the grouped form so
`tileart.uop`, `TerrainDefinition.uop`, `string_dictionary.uop` and `Texture.uop` are loaded
once for the shared EC classification pass:

```text
udd-pack pack-ec-textures --ccdir /path/to/cc --ecdir /path/to/ec --art-output tex_art_ec.uddp --land-output tex_land_ec.uddp --tilemeta-output tilemeta.uddp
```

For Classic facet packages that need both map and statics data, prefer:

```text
udd-pack pack-map-statics --ccdir /path/to/cc --map-id 0 --map-output map0.uddp --statics-output statics0.uddp
```

### `udd-tool`

`udd-tool` inspects and edits already-built packages. It is produced by the `udd-conv-cli` crate.

Common commands:

- `info`: print package structure, logical file counts, and metadata summaries.
- `extract`: unpack atlas packages to PNG pages plus CSV metadata.
- `diff`: compare two packages or metadata CSV files.
- `replace`: replace one logical file in a path-addressed package.
- `rebuild`: rebuild a package image while preserving logical files.
- `hash-path`: compute the `xxh64` hash used by path-addressed packages.
- `export-csv`: export editable package metadata.
- `import-csv`: import controlled metadata edits after invariant checks.
- `validate`: decode every entry and run each package's own reader; exits non-zero on any failure.

Typical inspection commands:

```text
udd-tool info tilemeta.uddp
udd-tool extract tex_art_ec.uddp --output tex_art_ec.extract
```

### `uop-tool`

`uop-tool` works with modern `.uop` package files. It is produced by the `uocf-cli` crate.

Common uses:

- compute UOP path hashes with `hash`
- brute-force candidate virtual paths with `crack`
- replace a payload in-place by hash with `replace`
- recompress and rebuild an entire package with `rebuild`

## Current Runtime Packages

### `tex_art_cc.uddp`

Stores Classic Client land/static atlas pages plus a sparse slot table keyed by classic `art_id`.

### `tex_art_ec.uddp`

Stores Enhanced Client static/art textures from the shared EC texture classification pass: the block-0 `WorldArt` image of each item (`Texture.uop`), which is the KR art.

- **KR art only.** The official EC draws an item's KR art only when its tileart record sets `UseNewArt` (795 items; `TileMetaPackage::uses_new_art`), otherwise classic-style art. The renderer deliberately ignores that flag: it is off for almost every item, even where the client packs KR art, so the client disabled that art on purpose. Enhanced mode draws the KR art wherever a `tex_art_ec` slot exists, then the Classic art, then magenta. The project does not use the EC's legacy-art set (`LegacyTexture.uop`): Classic art comes from `tex_art_cc.uddp`. On the plain clients 13,311 statics have a KR slot (all of them also have Classic art) and 23,216 have Classic art only.
- **Renderer fallback in Enhanced mode:** KR slot, then the `tex_art_cc` art at classic scale, then the client's magenta `missing.tga` (the reserved slot of `tex_aux_ec`).

### `tex_land_ec.uddp`

Stores representative terrain images and required terrain provenance metadata. The package preserves how `TerrainDefinition` material entries, aliases, selected texture ids, and canonical packed slots relate to each other.

EC land materials are role-based, not single-texture records. The runtime lookup table currently uses these material roles:

| Role | Meaning | Default / bump item | Water item |
|------|---------|---------------------|------------|
| `0` | base colour | 0 (colour A, the primary) | 1 (fallback colour, the primary) |
| `1` | detail | 1 (colour B) | - |
| `2` | mask/alpha | 2 (blend alpha) | - |
| `3` | normal | bump only: 3 (luminance bump) | 0 (distortion du/dv) |
| `4` | animated liquid | - | 2 (main colour) |

The source item behind each role depends on the material's layer class, which comes from its shader name (`TerrainLayerClass`; `UOWaterTerrainLayer`/`UOWaterShader` water, `UOBumpMapTerrainLayer` bump, anything else default). Positions are the client's (`scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md`); names and stretch never pick a layer, and stretch is only a UV divisor. Default and bump colour is `item0 * a + item1 * (1 - a)` with `a` from item 2; item 3 of a default material is never read, and item 4 of the bump material (environment map) is not drawn yet. Edge softness is the material's `coverage_exponent` (0: hard edge, point filtering; larger: soft, bilinear), not a KDL policy.

The default build is lossless (RGBA8888 pages, Zstd 19). The lossy build, `--land-bc7`, is BC7 at Zstd 19 (9.0 MB against 17.2 MB) and reaches the GPU as BC7 blocks; Classic land has no lossy build. Format details are in [UDDP_FORMATS.md](UDDP_FORMATS.md) section 2.4.

Normal maps are stored in the same UDDP page-atlas infrastructure as visible EC land textures, either as RGBA8888 pages or BC7/BC7-RDO GPU payloads depending on the package build. Do not assume a separate normal-map atlas exists at runtime. Original-client DDS files are source evidence only: for example, `01000013.dds` from a KR unpack is `256x256` DXT1, which shows the source normal map is RGB tangent-space data rather than BC5/ATI2 two-channel data. The renderer samples the repacked UDDP atlas and decodes its RGB channels as the normal.

### `gumps_cc.uddp` and `gumps_ec.uddp`

Paperdoll equipment gumps go to lossless atlas pages; every other gump is a single payload, RGBA8888 by default or BC1 with `--bc1` (about 21 MB against 46 MB Classic and 84 MB Enhanced). BC1 rounds soft alpha to 0 or 255, which loses the soft shadows of a few Enhanced gumps. Layout: [UDDP_FORMATS.md](UDDP_FORMATS.md) section 2.3.

### `tex_aux_ec.uddp`

Stores the non-base support images that tileart refs name and that a runtime consumer reads: sprite hue masks, static terrain-surface masks and EC item liquid inputs. All of them are WorldArt images from `Texture.uop`, keyed by texture id.

- **No legacy-art images.** TileArtLegacy and TileArtEnhanced refs are not packed. They are the EC's legacy-art mode, which the project does not convert; Classic art comes from the Classic Client files (`tex_art_cc.uddp`).
- **The client's fallback image.** When a texture request returns no data, the client draws `Data\SystemTextures\missing.tga` instead. The package carries that image once under a reserved id, and each referenced id that `Texture.uop` lacks is an alias slot of it.
- **Lookup.** `TexAuxEcPackage::slot_for_texture_id` and `load_rgba` return the magenta image for an absent id, as the client does. Layout details are in [UDDP_FORMATS.md](UDDP_FORMATS.md) section 2.5.

### `tilemeta.uddp`

Stores dense land/item metadata tables used by the runtime to merge Classic Client tiledata with Enhanced Client metadata and sidecars.

Each EC texture ref carries the shader input it feeds (`stable_role`), taken from the block's shader name and the item position, as the client binds them; file names are not read. Packages from before this rule (tilemeta format 1) are rejected; repack them.

## Enhanced Client Source Inputs

Enhanced Client source packages are not a single asset stream. The current packers read and classify content from a mixed set of UOP inputs.

Ownership and linkage sources:

- `tileart.uop`: EC static/art metadata, including sampling windows, shader/type hints, ownership, entry-specific flags, and surface-like records.
- `TerrainDefinition.uop`: EC land/material semantics, selected textures, aliases, and runtime slot/provenance relationships.
- `string_dictionary.uop`: shared EC string resource used for path and shader-name resolution.

Shared or support texture pools:

- `Texture.uop`: shared texture pool used by both EC land and EC static-art classification.
- `LegacyTexture.uop`: classic-style shared texture pool of the EC's legacy-art mode. No package reads it (`tex_art_ec`, `tex_land_ec` and `tex_aux_ec` take `Texture.uop` only); Classic art and land come from the Classic Client files.
- `SystemTextures.uop`: client system images; `missing.tga` is packed into `tex_aux_ec` as the missing-texture fallback.
- `TerrainTexture.uop`: land support-resource pool. Package origin alone does not prove land ownership.
- `EffectTexture.uop`: mixed effect-resource pool containing image and non-image resources.

Important rule:

- ownership matters more than raw id range or source package name. A texture id can legitimately appear in both `tex_art_ec` and `tex_land_ec` when different semantic sources require it.

## Current EC Classification Notes

- The `Rotated` flag (formerly `Unused1`) on EC tileart entries is currently treated as the strongest primary land hint when building the semantic translation table for EC land classification.
- `TerrainTranscode.kdl` maps Classic Client land ids to EC material ids, over the client's own `legacyterrainmap.csv` routes. It lives in `tex_land_ec.uddp`: the packer always embeds it as `metadata/transcode.bin` (the copy shipped with the tools, or the one given with `--land-transcode-kdl`), and the runtime reads only that copy. Dynamapper reads no loose copy, so an edit takes effect after a repack, and a package without the table fails to load with a message to repack it.
- EC terrain materials come from `TerrainDefinition.uop`. Reviewed layer-texture substitutions it does not carry live in `EcTerrainOverrides.kdl` (currently none: the file has no entries). Like the routing table, it lives in `tex_land_ec.uddp`: the packer always embeds it as `metadata/terrain_overrides.json` (the copy shipped with the tools, or the one given with `--land-terrain-overrides-kdl`), Dynamapper reads no loose copy, and a package without it fails to load with a message to repack it. Liquid motion and edge softness come from the UOP (tweaks, `coverage_exponent`), so the file has no liquid or policy nodes. The hand-written `TerrainDefinition.kdl` from before the UOP was parsed, and its audit commands, were removed.
- The current target architecture for EC textures is a three-way split built from one classification pass: `ec_textures_land.uddp`, `ec_textures_art.uddp`, and `ec_textures_layers.uddp`.
- The long-term direction is to keep shared source textures intact and let runtime sampling windows and semantic lookup tables choose the correct sub-rect or layer at render time.
- `TerrainTranscode.kdl` began as a KDL transcription of `TerrainTranscode.json` from a third-party client (stefanomerotta's UOClient `FileConverter/Content/`). That JSON is not in this repo; a reference copy sits in the git-ignored `old/compare_impl/`.

## CSV Editing Notes

CSV export/import is intended for inspection and controlled metadata edits, not as the runtime storage format.

Runtime packages remain binary-first:

- `.bin` metadata inside `.uddp` packages is authoritative.
- CSV is a tooling surface layered on top of binary package metadata.

CSV import is intentionally strict. It rejects edits that would break package invariants, including:

- changing the set of present slots
- changing slot kinds
- introducing out-of-bounds rectangles
- producing inconsistent `tex_land_ec` canonical terrain mappings

Use [LAND_TEXTURES_AND_TRANSITIONS.md](LAND_TEXTURES_AND_TRANSITIONS.md) together with [UDDP_FORMATS.md](UDDP_FORMATS.md) for the current EC land/art classification direction. The first document explains the semantic ownership split; the second documents package schemas.
