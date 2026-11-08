# UODynamapper and UOClientFiles Tools

UODynamapper is an interactive Ultima Online map renderer written in Rust with the Bevy engine. It uses locally converted client data to explore maps in Classic, Enhanced Client, Kingdom Reborn-inspired, and UODynamapper custom styles. UOClientFiles (UOCF) provides a library and tools for inspecting, editing, and converting Ultima Online client files independently of the renderer.

## Features

- Explore facets with camera movement, zoom, rotation, and navigation controls.
- Render land and static art with selectable texture sources (CC, EC, KR), visual presets, and adjustable shader settings.
- Convert Classic, Enhanced and KR Client assets into packages for streaming by the renderer.
- Inspect and work with source client files and converted packages using the included tools.
- Export rendered maps for a static web viewer (in progress).

Some rendering and tooling work is still in progress; see [the roadmap](docs/TODO.md).

## Ultima Online client file tools

UOCF and its graphical and command-line tools work directly with Classic Client, Enhanced Client, and Kingdom Reborn files. You can use them independently of the map renderer and its converted packages. Their workflows overlap with tools such as UOFiddler, MulPatcher, LegacyMULConverter-N, UOETE (UO Enhanced Tileart Editor), UOEMC (UO Enhanced Map Converter), huesmul2uop, and UOPTool; available operations vary by file and client version.

- **Browse and edit client assets:** inspect art, gumps, animations, tile metadata, terrain definitions, multis, hues, fonts, audio, localized strings, maps, and more. Edit supported records and import or export images, animation frames, and audio.
- **Convert client formats:** convert supported Classic MUL/IDX files to and from UOP, convert hues between `hues.uop` and `hues.mul`, and convert map/facets among Classic, Enhanced, and KR formats. Facets can also be inspected and individual Classic map cells edited.
- **Work with UOP packages:** browse and extract entries, resolve or search for hashed paths, replace content, rebuild packages, and manage path dictionaries.
- **Configure client executables:** patch supported settings including facet sizes and login configuration using the client patcher.

Start with [the graphical editor](tools/uocf-editor-gui/README.md) or [the client-file CLI](tools/uocf-cli/README.md); use `uocf-cli --help` for its available commands. Some views are inspection-only, and the graphical map view does not currently provide a tile painter.

## Requirements and setup

You need a local Ultima Online Classic, Enhanced or Kingdom Reborn Client installation containing the data you want to convert. Game assets are not included in this repository. Build runtime packages with `udd-conv-gui` or `udd-cli pack`; see the [asset pipeline guide](docs/dev_wiki/ASSET_PIPELINE.md) for examples and package details. A Rust toolchain is needed only if building the project from source.

The bundled shaders, fonts, and settings are in [dynamapper/assets/](dynamapper/assets/). Configure the package directory in [runtime_assets.toml](dynamapper/assets/settings/runtime_assets.toml): the default `uddp/` path is relative to the working directory. Settings saved by the app override the bundled settings and are stored in a `cache/` directory alongside the resolved assets directory.

The application looks for an `assets/` directory in the working directory, the Cargo crate directory, or next to the executable. The package directory can be elsewhere; it need not be inside the assets tree. The conversion guide and [package format reference](docs/dev_wiki/UDDP_FORMATS.md) describe the available packages.

Keybindings are configurable; see [the default keybindings](docs/keybindings.md).

## Tools

- `dynamapper` — interactive map renderer.
- `udd-conv-gui` and `udd-cli` — build runtime packages; `udd-cli tool` also inspects and edits them.
- `uddp-inspector-gui` — browse and preview converted packages.
- `uocf-cli` and `uocf-editor-gui` — front-ends for the UOCF library, for inspecting, editing, and converting supported UO client files.
- `uo-client-patcher-gui` — initial focus on Enhanced Client: patch supported client executable settings (login servers, login encryption and more).
- `uoppackage-cli` and `uop-dict-populator-gui` — work with UOP packages and virtual path/hash dictionaries.
- `webmap-export-cli` — build a web map from renderer output.

Development tools are also included. See [workspace components](docs/dev_wiki/WORKSPACE_COMPONENTS.md) and each tool's `--help` for details. The packages use a project-specific format designed for compact storage and streaming; its rationale and layouts are described in [UDDP formats](docs/dev_wiki/UDDP_FORMATS.md).

## Documentation

- [Tools guide](tools/README.md) — find the right tool for client files, packages, and export.
- [UOCF library guide](lib/uocf/README.md) — client-file format support and library documentation.
- [Asset pipeline](docs/dev_wiki/ASSET_PIPELINE.md) — conversion and package workflows.
- [UDDP formats](docs/dev_wiki/UDDP_FORMATS.md) — package architecture and layouts.
- [Workspace components](docs/dev_wiki/WORKSPACE_COMPONENTS.md) — libraries and tools.
- [Engineering guide](ENGINEERING.md) — contributor guidance.

## License and credits

Released under the [Apache license](LICENSE). See [credits and references](CREDITS.md) for UO file-format and algorithm sources.
