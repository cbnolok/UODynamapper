# UODynamapper

UODynamapper is an interactive Ultima Online map renderer written in Rust with the Bevy engine. It uses locally converted client data to explore maps in Classic, Enhanced Client, Kingdom Reborn-inspired, and UOD-Painterly styles. The workspace also provides tools for working with client files and converted packages.

## Features

- Explore facets with camera movement, zoom, rotation, and navigation controls.
- Render land and static art with selectable texture sources, visual presets, and adjustable shader settings.
- Convert Classic and Enhanced Client assets into packages for streaming by the renderer.
- Inspect and work with source client files and converted packages using the included tools.
- Export rendered maps for a static web viewer.

Some rendering and tooling work is still in progress; see [the roadmap](docs/TODO.md).

## Requirements and setup

You need a local Ultima Online Classic or Enhanced Client installation containing the data you want to convert. Game assets are not included in this repository. Build runtime packages with `udd-conv-gui` or `udd-cli pack`; see the [asset pipeline guide](docs/dev_wiki/ASSET_PIPELINE.md) for examples and package details. A Rust toolchain is needed when building from source.

The bundled shaders, fonts, and settings are in [dynamapper/assets/](dynamapper/assets/). Configure the package directory in [runtime_assets.toml](dynamapper/assets/settings/runtime_assets.toml): the default `uddp/` path is relative to the working directory. Settings saved by the app override the bundled settings and are stored in a `cache/` directory alongside the resolved assets directory.

The application looks for an `assets/` directory in the working directory, the Cargo crate directory, or next to the executable. The package directory can be elsewhere; it need not be inside the assets tree. The conversion guide and [package format reference](docs/dev_wiki/UDDP_FORMATS.md) describe the available packages.

Keybindings are configurable; see [the default keybindings](docs/keybindings.md).

## Tools

- `dynamapper` — interactive map renderer.
- `udd-conv-gui` and `udd-cli` — build runtime packages; `udd-cli tool` also inspects and edits them.
- `uddp-inspector-gui` — browse and preview converted packages.
- `uocf-cli` and `uocf-editor-gui` — work with Ultima Online client formats.
- `uoppackage-cli` and `uop-dict-populator-gui` — work with UOP packages and path dictionaries.
- `uo-client-patcher-gui` — patch supported client executable settings.
- `webmap-export-cli` — build a web map from renderer output.

Development tools are also included. See [workspace components](docs/dev_wiki/WORKSPACE_COMPONENTS.md) and each tool's `--help` for details. The packages use a project-specific format designed for compact storage and streaming; its rationale and layouts are described in [UDDP formats](docs/dev_wiki/UDDP_FORMATS.md).

## Documentation

- [Asset pipeline](docs/dev_wiki/ASSET_PIPELINE.md) — conversion and package workflows.
- [UDDP formats](docs/dev_wiki/UDDP_FORMATS.md) — package architecture and layouts.
- [Workspace components](docs/dev_wiki/WORKSPACE_COMPONENTS.md) — libraries and tools.
- [Engineering guide](ENGINEERING.md) and [renderer architecture](docs/agents/dynamapper_architecture.md) — contributor guidance.
- [Land textures and transitions](docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md) — terrain rendering notes.

## License and credits

Released under the [Apache license](LICENSE). See [credits and references](CREDITS.md) for UO file-format and algorithm sources.
