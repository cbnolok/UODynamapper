# Tools

The workspace includes tools for working directly with Ultima Online client files and separate tools for building and inspecting UODynamapper packages. Client-file tools can be used without the renderer or its packages. Client data is not included.

## Client files

| Task | Tool | Starting point |
| --- | --- | --- |
| Browse and edit supported assets, metadata and client files | `uocf-editor-gui` | [Editor guide](uocf-editor-gui/README.md) |
| Convert facets, MUL/UOP assets and hues; import or export supported images, animations, sound and strings | `uocf-cli` | [CLI guide](uocf-cli/README.md) and `uocf-cli --help` |
| Inspect, extract, replace and rebuild UOP packages; work with path hashes and dictionaries | `uoppackage-cli` | `uoppackage-cli --help` |
| Build and expand UOP path dictionaries | `uop-dict-populator-gui` | [Dictionary tool guide](uop-dict-populator-gui/README.md) |
| Patch supported client executable settings | `uo-client-patcher-gui` | Open an executable in the application |

Availability of viewing, editing, and conversion varies by file and client version. See the editor and CLI guides for details. The [UOCF library guide](../lib/uocf/README.md) describes the underlying format support.

## Renderer packages and export

| Task | Tool | Starting point |
| --- | --- | --- |
| Build runtime packages from client assets | `udd-conv-gui`, `udd-cli pack` | [Converter GUI](udd-conv-gui/README.md), [CLI](udd-conv-cli/README.md), [asset pipeline](../docs/dev_wiki/ASSET_PIPELINE.md) |
| Inspect or edit converted packages | `uddp-inspector-gui`, `udd-cli tool` | [Inspector](uddp-inspector-gui/README.md), [CLI](udd-conv-cli/README.md) |
| Export a web map from renderer output | `webmap-export-cli` | `webmap-export-cli --help`; work in progress |

## Development tools

`uocf-devtools-cli` and `udd-devtools-cli` provide research and diagnostics. Their optional binaries require the `dev-tool-tests` feature; see the [UDD development tools guide](udd-devtools-cli/README.md). For the broader workspace, see [workspace components](../docs/dev_wiki/WORKSPACE_COMPONENTS.md).
