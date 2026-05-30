# Workspace Components

UODynamapper is organized as a Cargo workspace. The main product is `dynamapper`; the other crates and tools exist to parse Ultima Online source data, build runtime packages, inspect those packages, and support renderer development.

## Main Application

### `dynamapper/`

The Bevy-based renderer and interactive map viewer.

Responsibilities:

- runtime app setup, configuration loading, UI, and controls
- terrain and static-art rendering
- shader assets and visual presets
- runtime package loading from `.uddp` files
- renderer-facing settings under `dynamapper/assets/settings/`

Runtime source assets and converted packages are expected under `dynamapper/assets/` or linked from there.

## Libraries

### `lib/uocf/`

Parser library for Ultima Online source formats used by Classic, Enhanced Client, and Kingdom Reborn data.

Current scope includes:

- map, statics, art, tiledata, and UOP-related parsing
- Enhanced Client package metadata such as `tileart.uop` and `TerrainDefinition.uop`
- custom project formats such as `.uddp` and `.uddf`
- support for additional community/custom formats as they are needed

### `lib/udd-container/`

Container and package infrastructure for UDDP/UDDF-style files.

Responsibilities:

- package section layout
- indexes and dictionaries
- low-level read/write structure shared by conversion and runtime code

### `lib/udd-assets/`

Runtime package readers and asset access helpers.

Responsibilities:

- reading converted `.uddp` packages
- exposing slot tables, terrain provenance, tile metadata, and texture/package lookup helpers
- keeping runtime-facing access separate from conversion-time parsing

### `lib/udd-logging/`

Shared terminal logging backend for tools. It installs a `log` crate backend that formats records with `paris`, so libraries continue to emit hookable `log::*` records while command-line tools get a consistent colored style.

### `lib/udd-conv/`

Shared conversion and packaging logic for UODynamapper runtime assets.

Responsibilities:

- Classic and Enhanced Client asset conversion
- land/art classification
- tile metadata packaging
- `.uddp` package generation logic used by CLI and GUI frontends

### `lib/udd-conv-ktx2/`

KTX2 texture handling used by the conversion pipeline.

### `lib/image-postprocess/`

Image processing support code used by texture packaging work, including BC7-related paths.

## Distributed Binaries

Release packages are expected to ship `dynamapper` and each tool as separate binaries. Some executable names differ from the Cargo crate names.

### Standalone Renderer

- `dynamapper`: interactive Bevy map renderer and viewer.

### UDD Package Management

- `udd-conv-gui`: graphical frontend for building UODynamapper runtime packages.
- `udd-conv-cli`: CLI crate that builds:
  - `udd-pack`: package builder for `.uddp` runtime assets.
  - `udd-tool`: package inspector/editor for existing `.uddp` files.
- `uddp-inspector-gui`: graphical inspector for `.uddp` package contents, atlas pages, and metadata slots.

### UOCF Command-Line Tools

All of these are built by the `uocf-cli` crate:

- `uop-tool`: hash, inspect, replace, crack candidate paths for, and rebuild `.uop` packages.
- `cc-uop-mul-converter`: convert between Classic Client `.mul`/`.idx` files and modern `.uop` packages.
- `uop-dict-populator-cli`: populate UOP hash dictionaries from templates or brute force.
- `texture-scanner`: identify and isolate land candidates from UO texture pools.
- `sound-tool`: inspect or convert supported UO sound data.
- `multimap-tool`: convert Classic Client `multimap.rle` to and from BMP or PNG.
- `facet-evidence-tool`: gather facet evidence for EC/KR land and map analysis.
- `kr-ec-terrain-diff-tool`: compare KR and EC land evidence.

### UOCF Graphical Frontends

- `uocf-inspector-gui`: inspector for UOCF-supported source client formats.
- `uop-dict-populator-gui`: GUI for building and expanding UOP hash dictionaries.

## CLI Tools

### `tools/udd-conv-cli/`

Command-line tooling for UODynamapper package workflows.

Primary binaries:

- `udd-pack`: builds runtime packages from source client assets.
- `udd-tool`: inspects, extracts, diffs, edits, and rebuilds existing `.uddp` packages.

See [ASSET_PIPELINE.md](ASSET_PIPELINE.md) for commands and package notes.

### `tools/uocf-cli/`

General-purpose Ultima Online format tooling built around `uocf`.

Included tools:

- `uop-tool`: hash, inspect, replace, crack candidate paths for, and rebuild modern `.uop` packages.
- `cc-uop-mul-converter`: convert between Classic Client `.mul`/`.idx` files and modern Classic Client `.uop` packages.
- `texture-scanner`: identify and isolate land candidates from UO texture pools.
- `uop-dict-populator-cli`: populate UOP hash dictionaries from templates or brute force.
- `sound-tool`: inspect or convert supported UO sound data.
- `multimap-tool`: convert Classic Client `multimap.rle` to and from BMP or PNG.
- `facet-evidence-tool`: gather facet evidence for EC/KR land and map analysis.
- `kr-ec-terrain-diff-tool`: compare KR and EC land evidence.

## GUI Tools

### `tools/udd-conv-gui/`

GUI frontend for the UODynamapper conversion pipeline.

### `tools/uddp-inspector-gui/`

Inspector for converted `.uddp` packages, atlas layers, logical files, and metadata slots.

### `tools/uocf-inspector-gui/`

Inspector for UOCF-supported source client formats, including `.uop` packages, `tiledata.mul`, and Enhanced Client `tileart.uop` metadata.

### `tools/uop-dict-populator-gui/`

GUI for building and expanding UOP hash dictionaries.

## Related Documentation

- [PROJECT_OVERVIEW.md](PROJECT_OVERVIEW.md): high-level project status.
- [CODE_OVERVIEW.md](CODE_OVERVIEW.md): runtime flow and system interactions.
- [TECHNICAL_REFERENCE.md](TECHNICAL_REFERENCE.md): technical rules, formats, and constants.
- [ASSET_PIPELINE.md](ASSET_PIPELINE.md): conversion commands and package workflow notes.
