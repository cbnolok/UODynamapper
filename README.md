# UODynamapper

A dynamic map renderer for Ultima Online, built with Rust and Bevy. Plans are to recreate both classic and modern (Kingdom Reborn/Enhanced Client) visuals with a single codebase.
It's still in an early stage.  
Status: Land/terrain rendering supported. Multi-map support, BC7 compression, and modular shaders implemented.

![Current state of version 0.1](./docs/screenshot-0.1.webp)

---

## Workspace Components

This project is organized as a Cargo workspace with several specialized components:

### Main App & Tools

### Dynamapper

- **[dynamapper](dynamapper/)**: The main application (Bevy-based renderer).
  - **Requirements**: A valid Ultima Online installation (Classic and/or Enhanced Client).
  - **Assets**: Shared assets (shaders, fonts, settings) are located in `dynamapper/assets/`. Runtime data (`.uddp` packages) should be placed or linked there.
  - **Configuration**: Managed via modular TOML files in `dynamapper/assets/settings/`.

### Tools

- **[udd-conv-cli](tools/udd-conv-cli/)**: CLI tooling for packing, inspecting, editing, diffing, and extracting UODynamapper `.uddp` packages.
- **[uocf-cli](tools/uocf-cli/)**: General-purpose UO tooling, providing:
  - `uoptool`: Utility for hashing, brute-force cracking, replacing files inside, and rebuilding modern `.uop` package files.
  - `cc_uop_mul_converter`: Converter for switching between legacy `.mul`/`.idx` and modern `.uop` formats for Classic Client.
  - `texture_scanner`: Identification and isolation of land/terrain candidates from UO texture pools.
  - `uop-dict-populator-cli`: GUI-less tool for populating UOP hash dictionaries via templates or brute-force.
- **[uddp-inspector-gui](tools/uddp-inspector-gui/)**: GUI for inspecting `.uddp` package contents, atlas layers, and metadata slots.
- **[uop-inspector-gui](tools/uop-inspector-gui/)**: GUI for inspecting `.uop` package structures, files, and hash redirects.
- **[udd-conv-gui](tools/udd-conv-gui/)**: GUI frontend for the asset conversion pipeline.
- **[uop-dict-populator-gui](tools/uop-dict-populator-gui/)**: GUI for building and expanding UOP hash dictionaries.


### Libraries

- **[uocf](lib/uocf/)**: A parser library for core Ultima Online file formats (Map, Art, Tiledata, UOP) for Classic, Enhanced and Kingdom Reborn clients. Support for custom formats is being added (Michelangelo's `.uop`, `.vd` files).
It adds support for the custom formats `.uddp` and `.uddf`.
- **[udd-conv](lib/udd-conv/)**: Shared library for UODynamapper-specific asset conversion and runtime data loading.

---

## Features

- Classic isometric (actually orthogonal military projection) view.
- Perspective view.
- Free camera movement and rotation (it will be limited if art sprites rendering is enabled).
- Free "player" movement and teleport via click or teleport menu (supports different maps).
- Different shading styles.

### CLI Features

#### `uddconv_cli`

`uddpack`: build runtime packages from source assets.
- `pack-art`: pack classic `art.mul` / `artidx.mul` into `tex_art_cc.uddp`.
- `pack-ec-textures`: pack Enhanced Client statics and terrain textures into `tex_art_ec.uddp` and `tex_land_ec.uddp`.
- `pack-tilemeta`: pack CC tiledata and EC tileart into `tilemeta.uddp`.
- `pack-radar`: generate a radar map (`facet0X.dds`) from Classic map and statics.
- `pack-map` / `pack-statics`: pack Classic `.mul` files into `.uddp` block-based packages.

`uddtool`: inspect and edit already-built packages.
- `info`: print package structure, logical file counts, and metadata summaries.
- `extract`: unpack atlas packages to PNG pages plus CSV metadata.
- `diff`: compare two packages or metadata CSV files (kind: `package`, `slots`, `terrain-provenance`).
- `replace`: replace one logical file in a path-addressed package.
- `rebuild`: rebuild a package image while preserving its logical files.
- `hash-path`: compute the `xxh64` hash used by path-addressed packages.
- `export-csv` / `import-csv`: manage editable CSV metadata (kind: `terrain-provenance`).

### Package Notes

- EC source packages are not a single asset stream. The current packers read and classify content from a mixed set of UOP inputs:
  - `Texture.uop` and `LegacyTexture.uop`: shared texture pools used by both EC land and EC static-art classification
  - `tileart.uop`: EC static/art metadata, including sampling windows, shader/type hints, ownership, and entry-specific flags
  - `TerrainDefinition.uop`: EC land/material semantics, selected textures, aliases, and runtime slot/provenance relationships
  - `string_dictionary.uop`: shared EC string resource used by the client package set, not visual art sources
- The important distinction is ownership, not raw id range. A texture id can legitimately appear in both `tex_art_ec` and `tex_land_ec` if the semantic sources require it.
- `Unused1` on EC tileart entries is currently treated as the strongest primary land hint when building the semantic translation table for EC land classification.
- `TerrainTranscode.kdl` is still the active loose override for mapping classic land ids to EC material ids at runtime.
- `TerrainDefinition.kdl` is legacy override data from before this repo could parse `TerrainDefinition.uop` correctly. It may still be loaded for inspection/backward compatibility, but it is no longer the authoritative source for EC land semantics.

- `tex_art_cc.uddp` stores classic land/static atlas pages plus a sparse slot table keyed by classic `art_id`.
- `tex_art_ec.uddp` stores Enhanced Client statics from the shared EC texture classification pass.
- `tex_land_ec.uddp` stores one representative terrain image per land slot plus required `metadata/terrain_provenance.bin`, preserving how TerrainDefinition material entries, aliases, selected texture ids, and canonical packed slots relate to each other.
- `tilemeta.uddp` stores dense land/item metadata tables used by the runtime to merge classic tiledata with Enhanced Client metadata.
- Current target architecture for EC textures is a three-way split built from one classification pass: `ec_textures_land.uddp`, `ec_textures_art.uddp`, and `ec_textures_layers.uddp`.
- The long-term direction is to keep the shared source textures intact and let runtime sampling windows and semantic lookup tables choose the right sub-rect or layer at render time.
- `TerrainTranscode.json` is the semantic-family seed for Classic land normalization and is the right starting point for a hand-tuned translation table.

### Recommended Workflows

- Shared EC texture packaging:
  - `uddpack pack-ec-textures --ecdir /path/to/ec --art-output tex_art_ec.uddp --land-output tex_land_ec.uddp`
  - `uddpack pack-tilemeta --ccdir /path/to/cc --ecdir /path/to/ec --output tilemeta.uddp`
- Inspecting package metadata:
  - `uddtool info tilemeta.uddp`
  - `uddtool extract tex_art_ec.uddp --output tex_art_ec.extract`

### CSV Editing Notes

- CSV export/import is intended for inspection and controlled metadata edits, not as the runtime storage format.
- Runtime packages remain binary-first: `.bin` metadata inside `.uddp` is authoritative, and CSV is a tooling surface layered on top.
- CSV import is intentionally strict. It rejects edits that would break package invariants such as changing the set of present slots, changing slot kinds, introducing out-of-bounds rectangles, or producing inconsistent `tex_land_ec` canonical terrain mappings.
- If you are looking for the current land/art classification direction, use `docs/LAND_TEXTURES_AND_TRANSITIONS.md` and `docs/UDDP_FORMATS.md` together: the former explains the semantic ownership split, the latter documents the package schema.

#### `uoptool`

- Compute UOP path hashes with `hash`.
- Brute-force candidate virtual paths with `crack`.
- Replace a payload in-place by hash with `replace`.
- Recompress and rebuild an entire package with `rebuild`.

---

## Dynamapper Configuration

Edit `assets/settings.toml` to set your Ultima Online installation directory:

```toml
[uo_paths]
installation_dir = "/path/to/your/uo"
```

Check other toml configuration files in the 'assets' folder.

---

## Dynamapper Keybindings

| Key | Action |
| --- | ------ |
| `W` / `A` / `S` / `D` | Move player (NW / SW / SE / NE) |
| `PageUp` / `PageDown` | Increase / Decrease altitude (Configurable) |
| `Scroll Wheel` | Zoom in / out |
| `F1` | Keybindings help overlay (Configurable) |
| `F2` | Options menu (Configurable) |
| `F3` | Terrain shader controls (Configurable) |
| `Ctrl+G` | Teleport dialog |
| `F11` / `Alt+Enter` | Toggle fullscreen |
| `Esc` | Close dialogs |

> [!NOTE]
> All keybindings are configurable, edit `assets/keybindings.toml` to customize.

See [docs/keybindings.md](docs/keybindings.md) for the complete list with descriptions.

---

## Documentation

| Document | Description |
| -------- | ----------- |
| **[docs/TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md)** | Authoritative technical specs, data formats, and shared constants |
| **[docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)** | High-level project summary, quick start, and current status |
| **[docs/CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md)** | Quick file reference, common workflows, and troubleshooting |
| **[docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md)** | Code flow and high-level system interactions |
| **[docs/tex_art_ec_trimming.md](docs/tex_art_ec_trimming.md)** | Cropped EC art rules and tilemeta update flow |
| **[docs/keybindings.md](docs/keybindings.md)** | Complete list of keyboard shortcuts |
| **[docs/TODO.md](docs/TODO.md)** | Planned features and future improvements |
| **[GEMINI.md](GEMINI.md)** | AI agent instructions and best practices |

**Recommended Reading Order**:

1. **New users**: Start with [PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)
2. **Contributors**: Read [CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md) for quick reference
3. **Deep dive**: Consult [CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) for code flow, then [TECHNICAL_REFERENCE.md](docs/TECHNICAL_REFERENCE.md) for specs.
