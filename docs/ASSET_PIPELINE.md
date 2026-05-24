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
- `pack-radar`: generate a radar map such as `facet0X.dds` from Classic Client map and statics data.
- `pack-map`: pack Classic Client map data into a block-based `.uddp` package.
- `pack-statics`: pack Classic Client statics data into a block-based `.uddp` package.

Typical Enhanced Client texture and metadata workflow:

```text
udd-pack pack-ec-textures --ecdir /path/to/ec --art-output tex_art_ec.uddp --land-output tex_land_ec.uddp
udd-pack pack-tilemeta --ccdir /path/to/cc --ecdir /path/to/ec --output tilemeta.uddp
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

Stores Enhanced Client static/art textures from the shared EC texture classification pass.

### `tex_land_ec.uddp`

Stores representative terrain images and required terrain provenance metadata. The package preserves how `TerrainDefinition` material entries, aliases, selected texture ids, and canonical packed slots relate to each other.

### `tilemeta.uddp`

Stores dense land/item metadata tables used by the runtime to merge Classic Client tiledata with Enhanced Client metadata and sidecars.

## Enhanced Client Source Inputs

Enhanced Client source packages are not a single asset stream. The current packers read and classify content from a mixed set of UOP inputs.

Ownership and linkage sources:

- `tileart.uop`: EC static/art metadata, including sampling windows, shader/type hints, ownership, entry-specific flags, and surface-like records.
- `TerrainDefinition.uop`: EC land/material semantics, selected textures, aliases, and runtime slot/provenance relationships.
- `string_dictionary.uop`: shared EC string resource used for path and shader-name resolution.

Shared or support texture pools:

- `Texture.uop`: shared texture pool used by both EC land and EC static-art classification.
- `LegacyTexture.uop`: classic-style shared texture pool.
- `TerrainTexture.uop`: land support-resource pool. Package origin alone does not prove land ownership.
- `EffectTexture.uop`: mixed effect-resource pool containing image and non-image resources.

Important rule:

- ownership matters more than raw id range or source package name. A texture id can legitimately appear in both `tex_art_ec` and `tex_land_ec` when different semantic sources require it.

## Current EC Classification Notes

- `Unused1` on EC tileart entries is currently treated as the strongest primary land hint when building the semantic translation table for EC land classification.
- `TerrainTranscode.kdl` is still the active loose override for mapping Classic Client land ids to EC material ids at runtime.
- `TerrainDefinition.kdl` is old override data from before this repo parsed `TerrainDefinition.uop` correctly. It may still be loaded for inspection or backward compatibility, but it is no longer the authoritative source for EC land semantics.
- The current target architecture for EC textures is a three-way split built from one classification pass: `ec_textures_land.uddp`, `ec_textures_art.uddp`, and `ec_textures_layers.uddp`.
- The long-term direction is to keep shared source textures intact and let runtime sampling windows and semantic lookup tables choose the correct sub-rect or layer at render time.
- `TerrainTranscode.json` is the semantic-family seed for Classic Client land normalization and is the right starting point for a hand-tuned translation table.

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
