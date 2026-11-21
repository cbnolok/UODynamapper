# Ultima Online tools

The UOClientFiles library and tools read, edit, and convert original **Classic**, **Enhanced**, and **Kingdom Reborn** client files. The UDD tools build and maintain the **UDDP packages** that Dynamapper streams. You can use the client-file tools independently of the renderer.

Download the platform's `-tools` artifact from a successful **Build Release** run. Unpack it and run the programs in `UODynamapper-tools/bin/`, with the bundled `shared/` folder in place. Use the help flag on any CLI to see its commands. [Build downloads](https://github.com/cbnolok/UODynamapper/actions/workflows/release.yml) · [Build from source](../docs/DEVELOPER_SETUP.md) · [Dynamapper showcase](../docs/SHOWCASE.md)

## Inspect and edit client assets

| Classic multi with Kingdom Reborn art | Classic UOP creature animation |
| --- | --- |
| [![Classic ship multi assembled with Kingdom Reborn world art](docs/images/tool-classic-multi-kr-art.webp)](uocf-editor-gui/README.md) | [![Dragon frame with its body and action tree in the UOP animation viewer](docs/images/tool-uop-animation.webp)](uocf-editor-gui/README.md) |

**UOCF Editor** can preview multis with a different client's art, inspect facets, and browse tile metadata, gumps, hues, fonts, audio, localized strings, and more. Animation tools include frame import and export, upscale previews, sequence-timed playback, and rider/mount checks. Availability of edits and conversions varies by asset and client version. [Graphical editor guide](uocf-editor-gui/README.md) · [Sprite upscaling examples](../docs/SHOWCASE.md#sprite-upscaling)

**`uocf-cli`** converts facets among Classic, Enhanced, and Kingdom Reborn, converts supported MUL/IDX and UOP assets, and round-trips Classic and Enhanced hue palettes. Use `sequence dump`, `sequence set`, and `sequence scan` to inspect timings, change supported fields, or check a client's sequence entries against its stored animation frames. [CLI guide](uocf-cli/README.md) · [UOCF library guide](../lib/uocf/README.md)

These workflows overlap with UOFiddler, MulPatcher, LegacyMULConverter-N, UOETE, UOEMC, huesmul2uop, and UOPTool. The editor guide gives the supported operations for each format, including views that are inspection-only.

## Build, inspect, and edit UDDP packages

| Asset Converter | UDDP Inspector |
| --- | --- |
| [![Asset Converter showing compression, upscale profiles, validation, and package jobs](docs/images/tool-converter-assets.webp)](udd-conv-gui/README.md) | [![UDDP Inspector showing Enhanced texture entries and a selected texture preview](docs/images/tool-uddp-texture-inspector.webp)](uddp-inspector-gui/README.md) |

- **Build runtime packages** with `udd-conv-gui` or `udd-cli pack`. Convert maps and statics, texture atlases, creature animations, user interface assets, and metadata. Choose compression and upscaling for supported assets. Enhanced packages can include Kingdom Reborn terrain and material sections. [Converter guide](udd-conv-gui/README.md) · [Asset pipeline](../docs/dev_wiki/ASSET_PIPELINE.md)
- **Inspect converted data** with `uddp-inspector-gui`. Browse individual textures and atlas pages, inspect metadata, and play packaged creature animations. [Inspector guide and screenshots](uddp-inspector-gui/README.md)
- **Edit or check a package** with `udd-cli tool`. Extract entries, replace a logical file, round-trip supported metadata as CSV, compare packages, and validate rebuilt files. [Package commands](udd-conv-cli/README.md#udd-cli-tool)

## Web map export

`webmap-export-cli` renders a region through Dynamapper and produces a WebP tile pyramid and a Leaflet viewer. Use `webmap-export-cli --help` for the available commands. [Export examples and validation limits](../docs/reviews/2026-11-01-webmap-export-renderer.md)

## Tool index

| Task | Tool | Starting point |
| --- | --- | --- |
| Inspect and edit supported client assets | `uocf-editor-gui` | [Editor guide](uocf-editor-gui/README.md) |
| Convert client formats and inspect animation sequences | `uocf-cli` | [CLI guide](uocf-cli/README.md) |
| Inspect, extract, replace, or rebuild a UOP package | `uoppackage-cli` | `uoppackage-cli --help` |
| Resolve hashed paths and expand UOP dictionaries | `uop-dict-populator-gui` | [Dictionary tool guide](uop-dict-populator-gui/README.md) |
| Patch supported client executable settings, such as facet sizes and login servers | `uo-client-patcher-gui` | [Client patcher guide](uo-client-patcher-gui/docs/README.md) |
| Build runtime packages | `udd-conv-gui`, `udd-cli pack` | [Converter GUI](udd-conv-gui/README.md) · [Packing commands](udd-conv-cli/README.md#udd-cli-pack) |
| Inspect or edit converted packages | `uddp-inspector-gui`, `udd-cli tool` | [Inspector](uddp-inspector-gui/README.md) · [Package commands](udd-conv-cli/README.md#udd-cli-tool) |
| Export rendered web maps | `webmap-export-cli` | [Web map export](#web-map-export) |

`uocf-devtools-cli` and `udd-devtools-cli` provide research and diagnostics. Their optional binaries require the `dev-tool-tests` feature. [UDD development tools](udd-devtools-cli/README.md) · [Workspace components](../docs/dev_wiki/WORKSPACE_COMPONENTS.md)
