# UODynamapper

A dynamic map renderer for Ultima Online, built with Rust and Bevy. Plans are to recreate both classic and modern (Kingdom Reborn/Enhanced Client) visuals with a single codebase.
It's still in an early stage.  
Status: Land/terrain rendering supported.

![Current state of version 0.1](./docs/screenshot-0.1.webp)

---

## Workspace Components

This project is organized as a Cargo workspace with several specialized components:

### Applications & Tools

- **[dynamapper](dynamapper/)**: The main application.
- **[uddconv_cli](uddconv_cli/)**: CLI tooling for packing, inspecting, editing, diffing, and extracting UODynamapper `.uddp` packages.
- **[uocf_cli](uocf_cli/)**: General-purpose UO tooling, providing:
  - `uoptool`: Utility for hashing, brute-force cracking, replacing files inside, and rebuilding modern `.uop` package files.
  - `cc_uop_mul_converter`: Converter for switching between legacy `.mul`/`.idx` and modern `.uop` formats.

### Libraries

- **[uocf](uocf/)**: A parser library for core Ultima Online file formats (Map, Art, Tiledata, UOP) for Classic, Enhanced and Kingdom Reborn clients. Support for custom formats is being added (Michelangelo's `.uop`, `.vd` files).
- **[uddconv](uddconv/)**: Shared library for UODynamapper-specific asset conversion and runtime data loading.

---

## Features

- Classic isometric (actually orthogonal military projection) view.
- Perspective view.
- Free camera movement and rotation (it will be limited if art sprites rendering is enabled).
- Free "player" movement and teleport via click or teleport menu (supports different maps).
- Different shading styles.

### CLI Features

#### `uddconv_cli`

- `uddpack`: build runtime packages from source assets.
  - `pack-art`: pack classic `art.mul` / `artidx.mul` into `cc_art.uddp`.
  - `pack-ec-art`: pack Enhanced Client statics into `ec_art.uddp`.
  - `pack-ec-art-cropped`: pack Enhanced Client statics into `ec_art_cropped.uddp` after clipping each tileart sampling window and trimming transparent borders inside that window.
  - `pack-ec-art-cropped --uddp-dir /path/to/packages`: find the raw `tilemeta.uddp` in that directory and write a derived `*_ec_art_cropped.uddp` tilemeta package with updated EC sampling offsets.
  - `pack-ec-land`: pack Enhanced Client terrain textures into `ec_land.uddp` and print TerrainDefinition raw-texture coverage plus ignored source ids.
  - `pack-tilemeta`: build `tilemeta.uddp` from classic `tiledata.mul` plus Enhanced `tileart.uop` metadata.
  - `pack-tilemeta --ec-art-cropped`: shift EC sampling start coordinates so tile metadata stays aligned with `pack-ec-art-cropped` output.
  - `pack-unified-tiledata`: compatibility alias for `pack-tilemeta`.
- `uddtool`: inspect and edit already-built packages.
  - `info`: print package structure, logical file counts, recognized package summaries, and `ec_land` terrain provenance row counts.
  - `extract`: unpack known atlas packages to PNG pages plus CSV metadata. `tilemeta.uddp` is recognized directly.
  - `rebuild`: rebuild a package image while preserving its logical files.
  - `replace`: replace one logical path-addressed file by virtual path and write a rebuilt package.
  - `hash-path`: compute the `xxh64` hash used by path-addressed UDDP packages.
  - `export-csv`: export editable CSV metadata.
    - `--kind terrain-provenance` for `ec_land` TerrainDefinition provenance rows.
    - `--kind slots` for `cc_art`, `ec_art`, and `ec_land` slot metadata.
  - `import-csv`: validate edited CSV metadata and write it back into a rebuilt package.
  - `diff`: compare two packages or two metadata CSV files.
    - `--kind package` compares logical payload fingerprints.
    - `--kind slots` compares atlas slot metadata.
    - `--kind terrain-provenance` compares `ec_land` TerrainDefinition provenance.

### Package Notes

- EC source packages are not a single asset stream. The current packers read and classify content from a mixed set of UOP inputs:
  - `Texture.uop` and `LegacyTexture.uop`: shared texture pools used by both EC land and EC static-art classification
  - `tileart.uop`: EC static/art metadata, including sampling windows, shader/type hints, ownership, and entry-specific flags
  - `TerrainDefinition.uop`: EC land/material semantics, selected textures, aliases, and runtime slot/provenance relationships
  - `string_dictionary.uop` and `string_Wdictionary.uop`: shared EC string resources used by the client package set, not visual art sources
- The important distinction is ownership, not raw id range. A texture id can legitimately appear in both `ec_art` and `ec_land` if the semantic sources require it.
- `Unused1` on EC tileart entries is currently treated as the strongest primary land hint when building the semantic translation table for EC land classification.
- `runtime_material_id_overrides.toml` is intentionally narrow and only normalizes known runtime collisions; it is not the full Classic-to-EC translation table.

- `cc_art.uddp` stores classic land/static atlas pages plus a sparse slot table keyed by classic `art_id`.
- `ec_art.uddp` stores Enhanced Client statics only. Terrain-owned source textures are excluded using `TerrainDefinition.uop` so terrain data does not leak into static art packages.
- `ec_art_cropped.uddp` uses the same static-art contract as `ec_art.uddp`, but first clips each art entry to the tileart `start_x/start_y/end_x/end_y` sampling window and only then trims transparent borders inside that window. This changes the packed source rect, so any matching item metadata must be built with `pack-tilemeta --ec-art-cropped`.
- `pack-ec-art-cropped --uddp-dir` derives a second tilemeta package from the raw package in that directory by rewriting only `metadata/items.bin`. The raw tilemeta package is left untouched, so repeated cropped-art test conversions do not stack offset edits.
- `ec_land.uddp` stores one representative terrain image per land slot plus required `metadata/terrain_provenance.bin`, which preserves how TerrainDefinition material entries, aliases, selected texture ids, and canonical packed slots relate to each other.
- `tilemeta.uddp` stores dense land/item metadata tables used by the runtime to merge classic tiledata with Enhanced Client metadata.
- When `tilemeta.uddp` is built with `--ec-art-cropped`, the EC sampling start coordinates are shifted to match the cropped `ec_art` payloads while preserving the original EC draw offsets.
- Different tileart entries can share the same decoded source texture while sampling different sub-rectangles of it. Because of that, cropped EC art must never globally trim or rewrite a shared non-alpha source texture without first applying the per-entry tileart clip rect.
- Current target architecture for EC textures is a three-way split built from one classification pass: `ec_textures_land.uddp`, `ec_textures_art.uddp`, and `ec_textures_layers.uddp`.
- The long-term direction is to keep the shared source textures intact and let runtime sampling windows and semantic lookup tables choose the right sub-rect or layer at render time.
- `runtime_material_id_overrides.toml` is a narrow runtime collision table for `ec_land`, not the full Classic-to-EC terrain translation table.
- `TerrainTranscode.json` is the semantic-family seed for Classic land normalization and is the right starting point for a hand-tuned translation table.

### Recommended Workflows

- Standard EC static packaging:
  - `uddpack pack-ec-art --ecdir /path/to/ec --output ec_art.uddp`
  - `uddpack pack-tilemeta --ccdir /path/to/cc --ecdir /path/to/ec --output tilemeta.uddp`
- Cropped EC static packaging:
  - `uddpack pack-ec-art-cropped --ecdir /path/to/ec --output ec_art_cropped.uddp --uddp-dir /path/to/packages`
  - This writes cropped art plus a derived cropped tilemeta package such as `tilemeta_ec_art_cropped.uddp` while keeping the raw `tilemeta.uddp` unchanged.
- Inspecting package metadata:
  - `uddtool info tilemeta.uddp`
  - `uddtool extract ec_art_cropped.uddp --output ec_art_cropped.extract`

### CSV Editing Notes

- CSV export/import is intended for inspection and controlled metadata edits, not as the runtime storage format.
- Runtime packages remain binary-first: `.bin` metadata inside `.uddp` is authoritative, and CSV is a tooling surface layered on top.
- CSV import is intentionally strict. It rejects edits that would break package invariants such as changing the set of present slots, changing slot kinds, introducing out-of-bounds rectangles, or producing inconsistent `ec_land` canonical terrain mappings.
- If you are looking for the current land/art classification direction, use `docs/LAND_TEXTURES_AND_TRANSITIONS.md` and `docs/UDDP_FORMATS.md` together: the former explains the semantic ownership split, the latter documents the package schema.

#### `uoptool`

- Compute UOP path hashes with `hash`.
- Brute-force candidate virtual paths with `crack`.
- Replace a payload in-place by hash with `replace`.
- Recompress and rebuild an entire package with `rebuild`.

---

## Configuration

Edit `assets/settings.toml` to set your Ultima Online installation directory:

```toml
[uo_paths]
installation_dir = "/path/to/your/uo"
```

Check other toml configuration files in the 'assets' folder.

---

## Keybindings

| Key | Action |
| --- | ------ |
| `W` / `A` / `S` / `D` | Move player (NW / SW / SE / NE) |
| `PageUp` / `PageDown` | Increase / Decrease altitude |
| `Scroll Wheel` | Zoom in / out |
| `F1` | Keybindings help overlay |
| `F2` | Options menu |
| `F3` | Terrain shader controls |
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
| **[docs/PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)** | High-level project summary, quick start, and current status |
| **[docs/CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md)** | Quick file reference, common workflows, and troubleshooting |
| **[docs/CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md)** | Low-level architecture details and design choices |
| **[docs/ec_art_trimming.md](docs/ec_art_trimming.md)** | Cropped EC art rules, tilemeta update flow, and why repeated conversions stay safe |
| **[docs/keybindings.md](docs/keybindings.md)** | Complete list of keyboard shortcuts |
| **[docs/TODO.md](docs/TODO.md)** | Planned features and future improvements |
| **[GEMINI.md](GEMINI.md)** | AI agent instructions and best practices |

**Recommended Reading Order**:

1. **New users**: Start with [PROJECT_OVERVIEW.md](docs/PROJECT_OVERVIEW.md)
2. **Contributors**: Read [CONTRIBUTORS_GUIDE.md](docs/CONTRIBUTORS_GUIDE.md) for quick reference
3. **Deep dive**: Consult [CODE_OVERVIEW.md](docs/CODE_OVERVIEW.md) for architecture details
