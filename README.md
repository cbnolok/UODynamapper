# UODynamapper

UODynamapper is a renderer for Ultima Online maps written in Rust and using the Bevy engine.

The main application is `dynamapper`: an interactive map viewer that reads converted Ultima Online data, streams land and static-art assets, and renders the world with Classic, Enhanced Client, and Kingdom Reborn inspired visual modes. The broader workspace contains conversion tools, package readers, inspectors, and format libraries that support that renderer.

The project is still early. Land rendering, multi-map support, BC7-compressed texture packages, modular shaders, and the first EC material-routing paths are in place. Static art, depth ordering, EC support resources, and packaging validation are still active work.

## What Dynamapper Does

- Renders Ultima Online land from converted Classic Client map data.
- Supports Classic 2D, Enhanced Classic, and KR-like land shader modes.
- Streams custom `.uddp` packages instead of reading every source client file at runtime.
- Handles multiple maps and camera-controlled exploration.

## Why a new package format?

`uddp` files are containers conceptually similar to `uop`, but with a slimmer per-file metadata structure and better compression algorithms. This helps noticeably reduce disk size.
Moreover, `uddp` packs textures in GPU-ready formats (texture atlases when useful, otherwise single textures per file) that make asset streaming fast and efficient. Textures can be uncompressed raw RGBA8888 files or BC7-compressed. BC7 is a GPU block-compression format that can be sampled directly by the GPU without a decompression prepass. It preserves far more quality than KR/EC DXT1/BC1 while using 1/4 of the VRAM required by uncompressed RGBA8888 assets.

The workspace includes an in-tree BC7 encoder and a BC7 RDO (rate-distortion optimization) pass. RDO deliberately allows tiny, bounded visual changes when they make neighboring BC7 blocks more compressible by the outer package compressor. In practice, this keeps runtime textures GPU-ready while improving package size and streaming locality compared with storing naive BC7 output.

## Current Scope

Implemented:

- Land tile rendering with a paged GPU metadata atlas.
- Multi-map discovery for the standard map set.
- Free camera movement, zoom, rotation, and teleport controls.
- Experimental land-only perspective and isometric free camera modes (without statics, which are rendered via billboarding).
- Modular TOML configuration under `dynamapper/assets/settings/`.
- Shader hot reload for land work.
- Conversion tooling for Classic and Enhanced Client source assets.
- `.uddp` package inspection, extraction, diffing, and controlled metadata editing.

In progress:

- Production static-art rendering and explicit UO-style depth behavior.
- Full KR land/art/support-resource routing.
- Full EC land/art/support-resource routing.
- More complete packaging validation and runtime diagnostics.
- Future renderer work such as clipmap land, mobiles, paperdoll, and export tooling.

See [docs/TODO.md](docs/TODO.md) for the working roadmap.

## Requirements

- A valid Ultima Online Classic Client (both mul and uop formats) or Enhanced Client installation for the client data you want to convert or render.
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
- `tex_land_ec.uddp`
- map and statics packages for the facets you want to inspect (i.e.: `map0.uddp`, `statics0.uddp`).

For conversion commands and package details, see [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md).

## Controls

| Key | Action |
| --- | ------ |
| `W` / `A` / `S` / `D` | Move player/camera anchor |
| `PageUp` / `PageDown` | Increase/decrease altitude |
| `Scroll Wheel` | Zoom in/out |
| `F1` | Keybindings help |
| `F2` | Options menu |
| `F3` | Land shader controls |
| `Ctrl+G` | Teleport dialog |
| `F11` / `Alt+Enter` | Toggle fullscreen |
| `Esc` | Close dialogs |

Keybindings are configurable. See [docs/keybindings.md](docs/keybindings.md) for the full list.

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
  - `texture-scanner`: identify and isolate land candidates from UO texture pools.
  - `facet-evidence-tool`: gather facet evidence for EC/KR land and map analysis.
  - `kr-ec-terrain-diff-tool`: compare KR and EC land evidence.
  - `uop-dict-populator-cli`: build and expand UOP hash dictionaries without the GUI.
- UOCF graphical frontends
  - `uocf-inspector-gui`: graphical inspector for supported Ultima Online source formats.
  - `uop-dict-populator-gui`: graphical tool for building and expanding UOP hash dictionaries.
- UOCF (UO Client Files) library
  - Shared package support:
    - `.uop` package reading, writing, compression handling, path hashing, hash dictionaries, and brute-force path discovery.
    - `tools/_shared_assets/Dictionary.dic` is a MPE (Mythic Package Editor)-compatible UOP virtual path string/hash dictionary used to map 64-bit UOP path hashes back to probable virtual file names. The UOCF inspector can load `.dic` files for its UOP browser, `uop-dict-populator-cli` / `uop-dict-populator-gui` expand them from templates or brute-force searches, and `uop-tool merge-dic` can merge multiple `.dic` files.
  - Classic Client support:
    - map and statics files, art and land textures, tiledata, hues, lights, gumps, fonts, sounds, multis, radar colors, animation metadata, `body.def` / `bodyconv.def`, `verdata.mul`, map/statics DIFs, `multimap.rle`, and related `.mul` / `.idx` layouts.
  - Kingdom Reborn support:
    - KR world maps and statics are stored as compressed facet sectors inside `facet*.uop` packages.
    - UOCF decodes and can encode those facet sectors, loads the KR tile/static dictionaries, and translates KR land/static ids toward Classic-style 8x8 map blocks and statics where a mapping is known. The tile dictionary maps KR land tile ids to Classic land tile ids; the static dictionary is a known-static whitelist used when decoding or encoding KR facet statics.
    - This is format support for conversion, inspection, and evidence gathering. It does not mean the Dynamapper runtime already renders every KR-only material or visual rule as the original KR client did.
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
  - `udd-image-codecs`: GPU texture codec support for the conversion pipeline, including KTX2 writing, BC7 decoding/encoding, SIMD-assisted analytical BC7 paths, and the BC7 RDO pass used to improve outer compression.
  - `image-postprocess`: image scaling and filtering support used before packaging, including xBRZ, hqx, and MMPX-style postprocess steps. Compression-specific code now lives in `udd-image-codecs`.

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
| [docs/keybindings.md](docs/keybindings.md) | Complete keyboard shortcut reference |

Recommended reading:

1. Start with this README to understand the project shape.
2. Read [docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md) for current implementation status.
3. Use [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md) when building packages.
4. Use [docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) and [docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md) when changing runtime behavior.
