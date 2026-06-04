# udd-conv-gui

`udd-conv-gui` is a graphical frontend for building UODynamapper runtime
packages. It wraps the `udd-conv-cli` packing logic (`udd-pack` commands) in an
egui/eframe interface and lets users configure and run conversion jobs without
using the command line.

## Features

- Configure source client paths (Classic Client or Enhanced Client installation
  directories) via a file picker.
- Select the currently supported GUI pack jobs for tile metadata, art, land
  textures, mobile animations, maps, statics, and radar output.
- Use an optional upscale profile file for the existing image pack jobs. See
  [`../udd-conv-cli/UPSCALE_PROFILES.md`](../udd-conv-cli/UPSCALE_PROFILES.md).
- Run conversion jobs in the background with live progress reporting.
- Persist settings across sessions (eframe persistence).

## GUI / CLI Surface

`udd-conv-gui` is a curated frontend over the conversion libraries, not a full
replacement for every `udd-pack` and `udd-tool` command. Use the GUI for common
interactive conversion and inspection workflows. Use the CLI for batch scripts,
advanced mutation, and pack jobs not yet exposed in the GUI.

### Packing Support

| Package/job | GUI | CLI |
| ----------- | --- | --- |
| Tile metadata | Yes | Yes |
| Classic art | Yes | Yes |
| Classic texmaps / land textures | Yes | Yes |
| Classic mobile animations | Yes | Yes |
| Enhanced art | Yes | Yes |
| Enhanced land textures | Yes | Yes |
| Enhanced mobile animations | Yes | Yes |
| Maps | Yes | Yes |
| Statics | Yes | Yes |
| Radar DDS | Yes | Yes |
| World lights | CLI only | Yes |
| Hues | CLI only | Yes |
| Classic gumps | CLI only | Yes |
| Enhanced gumps | CLI only | Yes |
| Combined map+statics command | CLI only | Yes |

### Advanced Packing Controls

| Control | GUI | CLI |
| ------- | --- | --- |
| Basic output format and compression settings | Yes | Yes |
| Single upscale filter controls | Yes, for existing image packers | Yes |
| Upscale profile file | Yes, for existing image packers | Yes |
| Arbitrary inline ordered pass chains | CLI only | Yes |
| Per-command audit/report tools | CLI only | Yes |

When an upscale profile is selected in the GUI, it is loaded at conversion start
and used only for image targets that match the profile. Existing GUI upscale
filter controls remain the fallback for unmatched targets.

### Package Tools

| Tool action | GUI | CLI |
| ----------- | --- | --- |
| Package info | Yes | Yes |
| Extract package | Yes | Yes |
| Diff packages or metadata | Yes | Yes |
| Hash virtual path | Yes | Yes |
| Replace package entry | CLI only | Yes |
| Rebuild package | CLI only | Yes |
| Export CSV metadata | Yes | Yes |
| Import CSV metadata | CLI only | Yes |

## Binary

```
udd-conv-gui
```

## Notes

- Requires a valid Classic Client or Enhanced Client installation as the source.
- Produced `.uddp` packages should be placed in, or linked into, the Dynamapper
  `assets/` tree as described in
  [docs/dev_wiki/ASSET_PIPELINE.md](../../docs/dev_wiki/ASSET_PIPELINE.md).
- For scripted or automated builds, prefer `udd-pack` from
  [`udd-conv-cli`](../udd-conv-cli/README.md) directly.
