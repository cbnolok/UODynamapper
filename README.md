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
- `tex_art_ec.uddp`
- `tex_land_cc.uddp`
- `tex_land_ec.uddp`
- map and statics packages for the facets you want to inspect (i.e.: `map0.uddp`, `statics0.uddp`).

For conversion commands and package details, see [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md).

## Controls

Keybindings are configurable. See [docs/keybindings.md](docs/keybindings.md) for the full list of the default values.

## Provided Tools

Release builds should expose these binaries as separate executables.

- Standalone renderer
  - `dynamapper`: interactive map renderer.
- UDD package management
  - `udd-conv-gui`: graphical frontend for building UODynamapper runtime packages.
  - `udd-conv-cli`: CLI crate; generated executables are `udd-pack` for building `.uddp` packages and `udd-tool` for inspecting, extracting, diffing, editing, and rebuilding them.
  - `uddp-inspector-gui`: graphical inspector for `.uddp` package contents, atlas pages, and metadata slots.
- UOCF end-user command-line tools
  - `uop-tool`: hash, inspect, replace, crack candidate paths for, and rebuild `.uop` packages.
  - `cc-uop-mul-converter`: convert between Classic Client `.mul`/`.idx` files and modern `.uop` packages.
  - `sound-tool`: inspect or convert supported UO sound data.
  - `multimap-tool`: convert Classic Client `multimap.rle` files to and from BMP or PNG.
- UOCF discovery and development command-line tools
  - These are still built by the `uocf-cli` crate, but their entrypoints live under `tools/uocf-cli/src/bin/dev-tools/` because they support package research, evidence gathering, and asset-pipeline maintenance rather than common user workflows.
  - For end-users (CLI - command line tools):
    - `uop-dict-populator-cli`: build and expand UOP hash dictionaries without the GUI.
  - For end-users (GUI - UOCF graphical frontends):
    - `uocf-inspector-gui`: graphical inspector for supported Ultima Online source formats.
    - `uop-dict-populator-gui`: graphical tool for building and expanding UOP hash dictionaries.
  - For Dynamapper development:
    - `texture-scanner`: identify and isolate land candidates from UO texture pools.
    - `facet-evidence-tool`: gather facet evidence for EC/KR land and map analysis.
    - `kr-ec-terrain-diff-tool`: compare KR and EC land evidence.
- UOCF (UO Client Files) library
  - Shared package support:
    - `.uop` package reading, writing, compression handling, path hashing, hash dictionaries, and brute-force path discovery (hash "cracking").
    - `tools/_shared_assets/Dictionary.dic` is a MPE (Mythic Package Editor)-compatible UOP virtual path string/hash dictionary used to map 64-bit UOP path hashes back to virtual file names. The UOCF inspector can load `.dic` files for its UOP browser, `uop-dict-populator-cli` / `uop-dict-populator-gui` expand them from templates or brute-force searches, and `uop-tool merge-dic` can merge multiple `.dic` files.
  - Classic Client support:
    - map and statics files, art and land textures, tiledata, hues, lights, gumps, fonts, sounds, multis, radarmap colors, animation metadata, `body.def` / `bodyconv.def`, `verdata.mul`, map/statics DIFs, `multimap.rle`, and related `.mul` / `.idx` layouts.
  - Kingdom Reborn support:
    - Facet encoder/decoder.
    - Many Enhanced Client files share their format with the older KR ones.
  - Enhanced Client support:
    - EC world maps and statics are also stored as compressed facet sectors inside `facet*.uop`, but with a slightly different format than KR one.
    - `TerrainDefinition.uop` is parsed as the land/material ownership source: material ids, aliases, selected texture refs, shader names, repetition values, and preserved unknown fields. In this context, a material is a semantic land definition, not just an image: it groups land ids or aliases with the texture references, shader hints, repetition/stretch values, and other metadata the client uses to render that land family.
    - `tileart.uop` is parsed as the item/static ownership source: tile records, art windows, offsets, flags, shader/type hints, lighting fields, surface-like/liquid-like classification evidence, and linked texture refs.
    - Supporting EC data includes texture package access, string dictionaries, localized strings, hues, multis, land config, tile database data, animation frames, waypoints, and classic-to-EC tile mapping helpers.
    - As with KR, this is parser and conversion support. Dynamapper currently uses only the converted runtime packages and still has open work for full EC material routing, support textures, and static-art behavior.
  - Compatibility and custom-format work:
    - Michelangelo-style `.uop` handling, `.vd` codec support, and shared helpers used by inspectors and converters.
- Custom support libraries
  - `udd-container`: low-level UDDP/UDDF container infrastructure.
  - `udd-assets`: runtime readers and package access helpers for converted assets.
  - `udd-conv`: conversion and packaging logic shared by frontends and CLIs.
  - `udd-image-codecs`: GPU texture codec support for the conversion pipeline, including KTX2 writing, BC7 SIMD-assisted analytical decoding/encoding (inspired by basis-universal implementation) and the BC7 RDO pass used to improve outer compression (Rate Distortion Optimization: alters BC7 blocks data to make it highly repetitive and easier to further compress).
  - `image-postprocess`: image scaling and filtering support used before packaging.
    - Upscaling algorithms:
      - Nearest, Bilinear, CatmullRom, Lanczos3
      - xBRZ
      - lqx, hqx
      - epx
      - 2xSaI, Super2xSaI, and SuperEagle
      - MMPX-style
      - Kopf-Lischinski depixelizer
      - NEDI (New Edge-Directed Interpolation)
      - AMD FSR (single EASU pass or EASU+RCAS)

## Workspace

The repository is a Cargo workspace. The important top-level groups are:

- `dynamapper/`: the Bevy application and renderer.
- `lib/uocf/`: parsers for Ultima Online source formats.
- `lib/udd-container/`, `lib/udd-assets/`, `lib/udd-conv/`, `lib/udd-image-codecs/`, `lib/image-postprocess/`: package infrastructure, runtime readers, conversion logic, GPU texture codecs, and image postprocessing.
- `tools/`: CLI and GUI tools for conversion, inspection, package editing, and UOP-related workflows.

Detailed crate and tool responsibilities live in [docs/WORKSPACE_COMPONENTS.md](docs/WORKSPACE_COMPONENTS.md).

## Documentation

| Document | Purpose |
| -------- | ------- |
| [docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md) | Contributor-oriented project summary and current status |
| [docs/WORKSPACE_COMPONENTS.md](docs/WORKSPACE_COMPONENTS.md) | Workspace crates, tools, and responsibilities |
| [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md) | Conversion commands, package notes, and CSV editing rules |
| [docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) | Runtime code flow and system interactions |
| [docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md) | Data formats, rendering constants, and technical rules |
| [docs/UDDP_FORMATS.md](docs/UDDP_FORMATS.md) | `.uddp` and `.uddf` package formats |
| [docs/LAND_TEXTURES_AND_TRANSITIONS.md](docs/LAND_TEXTURES_AND_TRANSITIONS.md) | EC land/art classification and transition work |
| [docs/CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md) | Quick file reference and contributor workflows |

Recommended reading:

1. Start with this README to understand the project shape.
2. Read [docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md) for current implementation status.
3. Use [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md) when building packages.
4. Use [docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) and [docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md) when changing runtime behavior.

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
  - lqx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c)
  - hqx: [C++ implementation](https://github.com/brunexgeek/hqx)
  - epx: [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementation](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/epx.c)
  - 2xSaI, Super2xSaI, and SuperEagle (SAI variants): [PixelScalerWin repo](https://github.com/daelsepara/PixelScalerWin) and [Libretro implementations folder](https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/)
  - MMPX-style: [Rust crate](https://crates.io/crates/mmpx).
