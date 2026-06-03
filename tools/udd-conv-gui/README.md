# udd-conv-gui

`udd-conv-gui` is a graphical frontend for building UODynamapper runtime
packages. It wraps the `udd-conv-cli` packing logic (`udd-pack` commands) in an
egui/eframe interface and lets users configure and run conversion jobs without
using the command line.

## Features

- Configure source client paths (Classic Client or Enhanced Client installation
  directories) via a file picker.
- Select which packages to build (land textures, art textures, maps, statics,
  tilemeta, hues, gumps, animations, etc.).
- Run conversion jobs in the background with live progress reporting.
- Persist settings across sessions (eframe persistence).

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
