# UODynamapper

UODynamapper is a renderer for Ultima Online maps written in Rust and using the Bevy engine.

The main application is `dynamapper`: an interactive map viewer that reads converted Ultima Online data, streams land and static-art assets, and renders the world with Classic, Enhanced Client, and Kingdom Reborn inspired visual modes. The broader workspace contains conversion tools, package readers, inspectors, and format libraries that support that renderer.


## What Dynamapper Does

- Renders Ultima Online land from converted Classic Client map data (also KR and EC facets are supported).
- Supports Classic 2D, Enhanced Classic, and Kingdom Reborn-like land shader modes.
- Streams custom `.uddp` packages instead of using UO client file at runtime.
- Handles multiple maps, camera-controlled exploration, click-based teleportation and go-to dialog.

## Why a new package format?

`uddp` files are containers conceptually similar to `uop`, but with a slimmer per-file metadata structure and better compression algorithms. This helps noticeably reduce asset size on disk, even if the stored data kept the same file format.
Moreover, `uddp` packs textures in GPU-ready formats (texture atlases when useful, otherwise single textures per virtual file) that make asset streaming fast and efficient. Textures can be uncompressed raw RGBA8888 files or BC7-compressed. BC7 is a high quality lossy GPU block-compression format that can be sampled directly by the GPU without a decompression prepass. It preserves far more quality than KR/EC DXT1/BC1 while using 1/4 of the VRAM required by uncompressed RGBA8888 assets.

The workspace includes an in-tree BC7 encoder and a BC7 RDO (rate-distortion optimization) pass. RDO deliberately allows tiny, bounded visual changes when they make neighboring BC7 blocks more compressible by the outer package compressor. In practice, this still keeps runtime textures GPU-ready while improving disk package size and streaming locality (but the same VRAM usage) compared with storing naive BC7 output.

## Current Scope

Noteworthy Dynamapper features implemented:

- Free camera movement, zoom, rotation, and teleport controls.
- Advanced shading tunable settings and presets for Classic Client, Enhanced Client or Kingdom Reborn visual style.
- Choose between usage of Classic or Enhanced/KR land textures, switch between Classic and KR art tile sets (still stored in the EC data files) seamlessly.
- Experimental land-only perspective and isometric free camera modes (without statics, which are rendered via billboarding).
- Modular TOML configuration under `dynamapper/assets/settings/`.
- Shader hot reload for land work.
- Conversion tooling for Classic and Enhanced Client source assets (see "Provided Tools" README section).
- Viewers for Classic and Enhanced Client source assets (see "Provided Tools" README section).
- `.uddp` package inspection, extraction, diffing, and controlled metadata editing (see "Provided Tools" README section).

In progress: see [docs/TODO.md](docs/TODO.md).

## Requirements

- A valid Ultima Online Classic Client (both mul and uop formats are supported) or Enhanced Client installation for the client data you want to convert or render.
- Converted `.uddp` packages for runtime use. These are built from local client files via the provided `udd-conv-gui` or `udd-conv-cli`; source game assets are not included in this repository.
- Rust toolchain compatible with the workspace edition only if you are building from source.

Most users are expected to use distributed binaries rather than compile the workspace themselves. Release packages are expected to include `dynamapper` and the individual tool binaries. Source builds are mainly for contributors, toolchain work, and renderer development.

## Basic Setup

The main app expects shared assets, shaders, fonts, settings under `assets/` (already provided).

Configure the runtime UDDP package path in `assets/settings/runtime_assets.toml`.

Runtime packages are produced with the conversion tools and then placed in, or linked into, the Dynamapper asset tree. A typical working set includes:

- `tilemeta.uddp`
- `tex_art_cc.uddp`
- `tex_art_ec.uddp` (optional)
- `tex_land_cc.uddp`
- `tex_land_ec.uddp` (optional)
- map and statics packages for the facets you want to inspect (i.e.: `map0.uddp`, `statics0.uddp`).

For conversion commands and package details, see [docs/dev_wiki/ASSET_PIPELINE.md](docs/dev_wiki/ASSET_PIPELINE.md).

## Controls

Keybindings are configurable. See [docs/keybindings.md](docs/keybindings.md) for the full list of the default values.

## Provided Tools

Release builds expose these binaries as separate executables.

### dynamapper

Interactive map renderer. The main application described throughout this README.

### udd-conv-gui

[`udd-conv-gui`](tools/udd-conv-gui/README.md) — graphical frontend for building UODynamapper runtime `.uddp` packages from
Classic Client or Enhanced Client source installations. Wraps the `udd-pack`
conversion commands in an egui interface with path pickers, job selection, and
live progress reporting.

### udd-conv-cli

[`udd-conv-cli`](tools/udd-conv-cli/README.md) — CLI crate that provides two binaries:

- `udd-pack`: converts Classic Client and Enhanced Client source assets into
  `.uddp` runtime packages (land textures, art textures, maps, statics,
  tilemeta, hues, gumps, animations, and more). Also includes EC material audit
  and validation commands used during development.
- `udd-tool`: inspects, extracts, hashes, replaces, rebuilds, diffs, and
  exports/imports CSV metadata for `.uddp` and `.uddpi` package files.

### uddp-inspector-gui

[`uddp-inspector-gui`](tools/uddp-inspector-gui/README.md) — graphical inspector for `.uddp` package contents. Browse raw
entries and their compression metadata, inspect atlas pages, preview decoded
textures, and play back mobile animation sequences from `mobile_anim_cc.uddp`
or `mobile_anim_ec.uddp`.

### uocf-cli

[`uocf-cli`](tools/uocf-cli/README.md) — CLI crate for Ultima Online file tooling built on top of the `uocf`
library. End-user binaries:

- `uop-tool`: hash UOP virtual paths, brute-force unknown hashes, extract,
  replace, and rebuild `.uop` packages, and merge `.dic` hash dictionaries.
- `cc-uop-mul-converter`: convert supported Classic Client `.mul`/`.idx` files
  to and from modern `.uop` packages.
- `sound-tool`: inspect or export Classic Client sound entries.
- `multimap-tool`: convert Classic Client `multimap.rle` to and from BMP or PNG.

Discovery and development binaries (under `src/bin/dev-tools/`; support asset
research and pipeline maintenance rather than common user workflows):

- `uop-dict-populator-cli`: populate `.dic` hash dictionaries from TOML
  templates without the GUI.
- `texture-scanner`: scan unpacked DDS texture trees and write a CSV inventory.
- `facet-evidence-tool`: extract EC/KR facet land evidence into KDL.
- `kr-ec-terrain-diff-tool`: compare KR land routing against EC evidence.

### uocf-asset-cli

[`uocf-asset-cli`](tools/uocf-asset-cli/README.md) — CLI crate for asset-level tooling built on top of `uocf`. Binary:

- `export-anim-patch`: export animation payloads from Classic Client
  `anim*.mul`/`anim*.idx`, CC `AnimationFrame*.uop`, or EC
  `AnimationFrame*.uop` as single-entry `.vd` files or Michelangelo/UOAnimTool
  `.uop` patch streams.

### uocf-inspector-gui

[`uocf-inspector-gui`](tools/uocf-inspector-gui/README.md) — graphical inspector for Ultima Online source formats. Reads
directly from client installation files without requiring prior conversion.
Views: UOP package explorer (with optional `.dic` dictionary resolution), CC
art tiles, CC/EC tiledata and tileart metadata, CC and EC mobile animations,
anim sequence tables, CC/EC gumps, multis, hues, clilocs, EC TerrainDefinition,
EC string dictionary, and CC sounds. Most views support switching between
Classic Client and Enhanced Client sources.

### uop-dict-populator-gui

[`uop-dict-populator-gui`](tools/uop-dict-populator-gui/README.md) — graphical tool for building and expanding UOP hash
dictionaries (`.dic` files). Load an existing dictionary, point the tool at a
directory of `.uop` packages, define candidate path templates in TOML, and run
a background hash-expansion search. Saves results as a `.dic` file compatible
with MPE (Mythic Package Editor) and `uocf-inspector-gui`.

The shared starting-point dictionary lives at
`tools/_shared_assets/Dictionary.dic`. `uop-tool merge-dic` can merge multiple
`.dic` files.

### uocf library

[`uocf`](lib/uocf/README.md) — the core Ultima Online client format parser library used by all tools and
the conversion pipeline. Covers:

- **`.uop` container**: reading, writing, compression dispatch (zlib/deflate,
  zlib-bwt, raw), path hashing, brute-force hash cracking, and hash dictionary
  management.
- **Classic Client** (`.mul` / `.idx` and their `.uop` equivalents): map and
  statics, art and land textures, tiledata, animations, gumps, fonts, hues,
  sounds, multis, radar map RLE, lights, radar colors, verdata patches,
  clilocs, body definition files, and DIF patch handling. Several CC file types
  have been re-released or supplemented in `.uop` format (e.g.
  `artLegacyMUL.uop`, `gumpartLegacyMUL.uop`, `soundLegacyMUL.uop`,
  `AnimationFrame*.uop`); these wrap data with the same logical layout as their
  `.mul` counterparts and are parsed by the same `classic` module.
- **Kingdom Reborn / Enhanced Client**: facet encoder/decoder, EC
  `TerrainDefinition.uop`, `tileart.uop`, texture packages, animation frames,
  hues, multis, localized strings, string dictionary, land config, tile
  database, waypoints, and CC-to-EC tile mapping helpers.
- **Compatibility formats**: `.vd` animation patch stream codec,
  Michelangelo-style animation `.uop` codec.

### Custom support libraries

- [`udd-container`](lib/udd-container/README.md): low-level `.uddp` / `.uddf` container I/O, compression
  (zstd, JPEG XL), and xxHash-64 path addressing.
- [`udd-assets`](lib/udd-assets/README.md): typed runtime readers for converted `.uddp` packages; used by
  `dynamapper` at runtime and by tools for inspection and validation.
- [`udd-conv`](lib/udd-conv/README.md): conversion and atlas-packing logic shared by `udd-conv-cli` and
  `udd-conv-gui`.
- [`udd-image-codecs`](lib/udd-image-codecs/README.md): GPU texture codec support — BC7 SIMD-assisted analytical
  encoder/decoder (inspired by basis-universal), BC7 RDO pass for improved
  outer compression, and KTX2 container writing for supercompressed BC7 + zstd textures.
- [`image-postprocess`](lib/image-postprocess/README.md): image scaling and filtering used before packaging.
  Supported upscalers: Nearest, Bilinear, CatmullRom, Lanczos3, xBRZ, lqx,
  hqx, epx, 2xSaI, Super2xSaI, SuperEagle, MMPX, Super-xBR, ScaleFX,
  OmniScale, Jinc2, CUT1/CUT2/CUT3, Kopf-Lischinski depixelizer, NEDI, and AMD FSR
  (EASU or EASU+RCAS).

## Workspace

The repository is a Cargo workspace. The important top-level groups are:

- `dynamapper/`: the Bevy application and renderer.
- `lib/uocf/`: parsers for Ultima Online source formats.
- `lib/udd-container/`, `lib/udd-assets/`, `lib/udd-conv/`, `lib/udd-image-codecs/`, `lib/image-postprocess/`: package infrastructure, runtime readers, conversion logic, GPU texture codecs, and image postprocessing.
- `tools/`: CLI and GUI tools for conversion, inspection, package editing, and UOP-related workflows.

Detailed crate and tool responsibilities live in [docs/dev_wiki/WORKSPACE_COMPONENTS.md](docs/dev_wiki/WORKSPACE_COMPONENTS.md).

## Documentation

| Document | Purpose |
| -------- | ------- |
| [docs/dev_wiki/PROJECT_OVERVIEW.md](docs/dev_wiki/PROJECT_OVERVIEW.md) | Contributor-oriented project summary and current status |
| [docs/dev_wiki/WORKSPACE_COMPONENTS.md](docs/dev_wiki/WORKSPACE_COMPONENTS.md) | Workspace crates, tools, and responsibilities |
| [docs/dev_wiki/ASSET_PIPELINE.md](docs/dev_wiki/ASSET_PIPELINE.md) | Conversion commands, package notes, and CSV editing rules |
| [docs/dev_wiki/CODE_OVERVIEW.md](docs/dev_wiki/CODE_OVERVIEW.md) | Runtime code flow and system interactions |
| [docs/dev_wiki/TECHNICAL_REFERENCE.md](docs/dev_wiki/TECHNICAL_REFERENCE.md) | Data formats, rendering constants, and technical rules |
| [docs/dev_wiki/UDDP_FORMATS.md](docs/dev_wiki/UDDP_FORMATS.md) | `.uddp` and `.uddf` package formats |
| [docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md](docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md) | EC land/art classification and transition work |
| [docs/dev_wiki/CONTRIBUTORS_GUIDE.md](docs/dev_wiki/CONTRIBUTORS_GUIDE.md) | Quick file reference and contributor workflows |


Recommended reading:

1. Start with this README to understand the project shape.
2. Read [docs/dev_wiki/PROJECT_OVERVIEW.md](docs/dev_wiki/PROJECT_OVERVIEW.md) for current implementation status.
3. Use [docs/dev_wiki/ASSET_PIPELINE.md](docs/dev_wiki/ASSET_PIPELINE.md) when building packages.
4. Use [docs/dev_wiki/CODE_OVERVIEW.md](docs/dev_wiki/CODE_OVERVIEW.md) and [docs/dev_wiki/TECHNICAL_REFERENCE.md](docs/dev_wiki/TECHNICAL_REFERENCE.md) when changing runtime behavior.

## License, credits and references

Released under the [Apache license](LICENSE.md)

### UO Files

- Good ol' [Heptazane wiki](https://uo.stratics.com/heptazane/fileformats.shtml#3.8)
- Lots of C++ tools/libs i wrote in the years: uocf and uopp (mainly wrote to be used by Leviathan), UO ETE (Enhanced Tileart Editor), UO EMC (Enhanced Map Converter), huesmul2uop.
- ClassicUO: "zlib_bwt" codec.  
- KR facet format from ManawydanMapConverter.
- Lots of EC file formats from [Kons' wiki](https://code.google.com/archive/p/kprojects/wikis).
- Stefanomerotta's proof of concept [UO Client](https://github.com/stefanomerotta/UOClient/) for guidance on animated land textures shaders.
- UO-EC-Super-Viewer: xml source data for AnimationsCollection, AudioCollection, MultiCollection.
- UOFiddler: audio, fonts.

### Algorithms

Used as sources for Rust translation, which has been incrementally optimized further.

- BC7 codec: [basis-universal implementation](https://github.com/BinomialLLC/basis_universal/tree/master/transcoder)
- RDO implementation: [richgel999/bc7enc_rdo](https://github.com/richgel999/bc7enc_rdo) and [richgel999/bc7enc_rdo_devel](https://github.com/richgel999/bc7enc_rdo_devel/blob/master/bc7enc.cpp#L3711)
- Upscalers:
  - NEDI: [Python implementation](https://github.com/Kirstihly/Edge-Directed_Interpolation)
  - Kopf-Lischinski Depixelizer: [Python implementation](https://github.com/vvanirudh/Pixel-Art/tree/master)
  - AMD FidelityFX 1.1 (ESAU - - and RCAS - - passes): [Official C implementation](https://github.com/GPUOpen-Effects/FidelityFX-FSR).
  - xBRZ: [Rust crate](https://docs.rs/xbrz-rs/latest/xbrz/)
  - Super-xBR: [hansonw/super-xbr](https://github.com/hansonw/super-xbr)
  - ScaleFX: [Themaister/slang-shaders ScaleFX](https://github.com/Themaister/slang-shaders/tree/master/scalefx)
  - OmniScale: [Themaister/slang-shaders OmniScale](https://github.com/Themaister/slang-shaders/tree/master/omniscale)
  - Jinc2: local Hyllian GLSL passes from `/home/claudio/Scaricati/jinc2*`
  - Cheap Upscaling Triangulation: [Swordfish90/cheap-upscaling-triangulation](https://github.com/Swordfish90/cheap-upscaling-triangulation)
  - lqx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c)
  - hqx: [C++ implementation](https://github.com/brunexgeek/hqx)
  - epx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/epx.c)
  - 2xSaI, Super2xSaI, and SuperEagle (SAI variants): [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementations folder](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/)
  - MMPX-style: [Rust crate](https://crates.io/crates/mmpx).
