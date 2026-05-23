# UODynamapper

UODynamapper is a Rust and Bevy renderer for Ultima Online maps.

The main application is `dynamapper`: an interactive map viewer that reads converted Ultima Online data, streams terrain and static-art assets, and renders the world with Classic, Enhanced Client, and Kingdom Reborn inspired visual modes. The broader workspace contains conversion tools, package readers, inspectors, and format libraries that support that renderer.

The project is still early. Terrain rendering, multi-map support, BC7-compressed texture packages, modular shaders, and the first EC material-routing paths are in place. Static art, depth ordering, EC support resources, and packaging validation are still active work.

## What Dynamapper Does

- Renders Ultima Online terrain from converted Classic Client map data.
- Supports Classic 2D, Enhanced Classic, and KR-like terrain shader modes.
- Streams converted `.uddp` packages instead of reading every source client file at runtime.
- Handles multiple maps and camera-controlled exploration.
- Uses paged terrain metadata, texture residency, and compressed GPU textures to keep large maps practical.
- Preserves Enhanced Client material provenance so the renderer can distinguish terrain-owned, art-owned, and surface-like assets.

The goal is not to replace the original clients. It is a rendering and tooling project for studying, converting, and visualizing Ultima Online world data with a modern renderer while keeping source-data relationships explicit.

## Current Scope

Implemented:

- Land tile rendering with a paged GPU metadata atlas.
- Multi-map discovery for the standard map set.
- Free camera movement, zoom, rotation, and teleport controls.
- Modular TOML configuration under `dynamapper/assets/settings/`.
- Shader hot reload for terrain work.
- Conversion tooling for Classic and Enhanced Client source assets.
- `.uddp` package inspection, extraction, diffing, and controlled metadata editing.

In progress:

- Production static-art rendering and explicit UO-style depth behavior.
- Full EC terrain/art/support-resource routing.
- More complete packaging validation and runtime diagnostics.
- Future renderer work such as clipmap terrain, mobiles, paperdoll, and export tooling.

See [docs/TODO.md](docs/TODO.md) for the working roadmap.

## Requirements

- Rust toolchain compatible with the workspace edition.
- A valid Ultima Online installation for the client data you want to convert or render.
- Converted `.uddp` packages for runtime use. These are built from local client files; source game assets are not included in this repository.

The main app expects shared assets, shaders, fonts, settings, and runtime data under `dynamapper/assets/`.

## Basic Setup

Configure your Ultima Online paths in the modular settings files under `dynamapper/assets/settings/`. The active file for client locations is `uo_files.toml`.

Runtime packages are produced with the conversion tools and then placed in, or linked into, the Dynamapper asset tree. A typical working set includes:

- `tilemeta.uddp`
- `tex_art_cc.uddp`
- `tex_art_ec.uddp`
- `tex_land_ec.uddp`
- map and statics packages for the facets you want to inspect

For conversion commands and package details, see [docs/ASSET_PIPELINE.md](docs/ASSET_PIPELINE.md).

## Running

From the workspace root:

```text
cargo run -p dynamapper
```

The exact result depends on the configured client paths, the converted packages available under `dynamapper/assets/`, and the current state of the renderer.

## Controls

| Key | Action |
| --- | ------ |
| `W` / `A` / `S` / `D` | Move player/camera anchor |
| `PageUp` / `PageDown` | Increase/decrease altitude |
| `Scroll Wheel` | Zoom in/out |
| `F1` | Keybindings help |
| `F2` | Options menu |
| `F3` | Terrain shader controls |
| `Ctrl+G` | Teleport dialog |
| `F11` / `Alt+Enter` | Toggle fullscreen |
| `Esc` | Close dialogs |

Keybindings are configurable. See [docs/keybindings.md](docs/keybindings.md) for the full list.

## Workspace

The repository is a Cargo workspace. The important top-level groups are:

- `dynamapper/`: the Bevy application and renderer.
- `lib/uocf/`: parsers for Ultima Online source formats.
- `lib/udd-container/`, `lib/udd-assets/`, `lib/udd-conv/`, `lib/udd-conv-ktx2/`: package infrastructure, runtime readers, and conversion logic.
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
