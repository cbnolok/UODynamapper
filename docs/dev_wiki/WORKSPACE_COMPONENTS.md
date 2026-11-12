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

Bundled shaders and settings live under `dynamapper/assets/`; the runtime package directory is configured separately.

## Libraries

### `lib/uocf/`

Parser library for Ultima Online source formats used by Classic, Enhanced Client, and Kingdom Reborn data.

Current scope includes:

- map, statics, art, tiledata, and UOP-related parsing
- Enhanced Client package metadata such as `tileart.uop` and `TerrainDefinition.uop`
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

### `lib/support/uot-logging/`

Shared terminal logging backend for tools. It installs a `log` crate backend that formats records with `paris`, so libraries continue to emit hookable `log::*` records while command-line tools get a consistent colored style.

### `lib/support/uot-kdl/`

KDL parsing for the workspace, with knuffel's grammar compiled once instead of in every crate that parses KDL. Used by the
converters, `uocf`, the client tools and Dynamapper.

### `lib/support/uot-texture-codecs/`

Texture block codecs: BC1 and BC7 encoding and decoding, VRAM layout helpers, premultiplied colour conversion, and KTX2
handling behind the `ktx2` feature.

### `lib/support/uot-gui/`

Helpers shared by the egui tools: app startup, file dialogs and the common zoom and scroll handling.

### `lib/udd-conv/`

Shared conversion and packaging logic for UODynamapper runtime assets.

Responsibilities:

- Classic and Enhanced Client asset conversion
- land/art classification
- tile metadata packaging
- `.uddp` package generation logic used by CLI and GUI frontends

### `lib/support/uot-image-postprocess/`

Image processing support code used by texture packaging work, including BC7-related paths.

### `lib/ec-effects/gamebryo-nif/`

Reader and writer for Gamebryo NIF files (version 20.0.0.4), with block types generated from a schema written against the Gamebryo 2.6 SDK. It reads the Enhanced Client effect NIFs and writes them back byte-identically.

### `lib/ec-effects/particle-sim/`

CPU simulation and preview rendering of the Enhanced Client particle effects: the Chaos `.ems` engine, the NIF effect runtime, hued effects, a software rasterizer, the `effects_ec.uddp` description codec and texture mip rules. Known differences from the client are listed in its README.

### `lib/ec-effects/effect-gltf/`

glTF 2.0 binary (`.glb`) export of the Enhanced Client effects: Gamebryo NIF scenes and whole particle effects.

## Tools

Executable names sometimes differ from Cargo crate names. Command details are available through `--help`.

- `dynamapper`: interactive map renderer.
- `udd-conv-gui`: graphical frontend for building runtime packages.
- `udd-cli` (from `udd-conv-cli`): `pack` builds packages and `tool` inspects and edits them. See [Asset pipeline](ASSET_PIPELINE.md).
- `uddp-inspector-gui`: inspector for converted packages and their contents.
- `uocf-cli`: client-file operations including animation, image, sound, hue, and facet conversion. See [its README](../../tools/uocf-cli/README.md).
- `uoppackage-cli`: content-agnostic UOP package operations.
- `uocf-editor-gui`: inspector and editor for client formats.
- `uop-dict-populator-gui`: UOP path dictionary builder.
- `uo-client-patcher-gui`: supported client executable patches.
- `webmap-export-cli`: static web map export using the renderer.
- `uocf-devtools-cli` and `udd-devtools-cli`: research and diagnostics; their optional binaries use the `dev-tool-tests` feature. See the [UDD development tools README](../../tools/udd-devtools-cli/README.md).

## Related Documentation

- [Engineering guide](../../ENGINEERING.md): workspace conventions and build commands.
- [Land textures and transitions](rendering/LAND_TEXTURES_AND_TRANSITIONS.md): terrain rendering guidance.
- [Client night lighting, static shadows and sun](rendering/CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md): the KR and Enhanced clients' effects KR-like and Enhanced reproduce.
- [UDDP formats](UDDP_FORMATS.md): package schemas.
- [Asset pipeline](ASSET_PIPELINE.md): conversion commands and package workflow notes.
