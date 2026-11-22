# udd-conv-gui

`udd-conv-gui` is a graphical frontend for building UODynamapper runtime
packages. It wraps the `udd-conv-cli` packing logic (`udd-cli pack` commands) in an
egui/eframe interface and lets users configure and run conversion jobs without
using the command line.

## Quick Convert

On the first screen, choose a Classic or Enhanced Client folder and an output folder. **Convert Classic** and **Convert Enhanced** become available when their source folders are set. **Convert both** builds the full set in one run.

The batch uses BC7 and BC1 where supported and Zstd by default. The packer can choose faster-decoding LZ4 HC for atlas pages that stay within its size threshold. Native formats remain in use for packages such as the Classic texmaps and paperdoll frames. Quick Convert disables upscaling, upscale profiles, and optional filters. The other tabs keep their manual job settings.

A Classic run builds art, land, animations, gumps, tile metadata, world lights, hues, and the available facet and statics packages. For radar images, use the World tab. An Enhanced run builds Enhanced art, land, animations, gumps, effects, paperdolls, and hues. With a Classic folder configured, it also builds merged tile metadata and auxiliary textures. The Enhanced packages can use the bundled Kingdom Reborn delta or the optional Kingdom Reborn folder chosen in Sources.

The log names the current job and reports failures. Stop requests cancellation, and packages that were already completed stay in the output folder. The converter asks before replacing existing outputs.

## Features

- Configure source client paths (Classic Client or Enhanced Client installation
  directories) via a file picker.
- Select the currently supported GUI pack jobs for tile metadata, hues, world
  lights, art, land textures, mobile animations, Enhanced effects, the Enhanced
  paperdoll, gumps, maps,
  statics, and radar output.
- Use an optional upscale profile file for the existing image pack jobs. See
  [`../udd-conv-cli/UPSCALE_PROFILES.md`](../udd-conv-cli/UPSCALE_PROFILES.md).
- Run conversion jobs in the background with live progress reporting and a
  Stop button. A stopped job leaves no package file behind.
- Validate packages as `udd-cli tool validate` does, from the Tools tab or after
  each packing job (Validate packages after packing, off by default).
- Persist settings across sessions (eframe persistence).

## GUI / CLI Surface

`udd-conv-gui` is a curated frontend over the conversion libraries, not a full
replacement for every `udd-cli pack` and `udd-cli tool` command. Use the GUI for common
interactive conversion and inspection workflows. Use the CLI for batch scripts,
advanced mutation, and pack jobs not yet exposed in the GUI.

Every GUI job calls the same `udd-conv` functions as its CLI command. The GUI
reads the sources from the Sources tab, writes into its output folder under the
CLI's default file names, and reads `tilemeta.uddp` from its input folder.
Yes means the GUI offers every option of the command, apart from the image job
gaps below. Partial lists the other flags the GUI does not offer.

Two gaps apply to the image jobs, and the rows below leave them out:

- The GUI sets one upscale filter per slot, where the CLI chains repeated
  `--upscale-pass` style flags. An upscale profile can still chain passes.
- The GUI has no uncompressed RGBA8888 output (`--raw` without `--zstd`) for
  the atlas and texture jobs. Its lossless choice is RGBA8888 with Zstd.

### `udd-cli pack`

| Command | GUI | Not in the GUI |
| ------- | --- | -------------- |
| `pack-tilemeta` | Partial: Assets, Tile Metadata | `--use-ec-radarcol` |
| `pack-art` | Partial: Assets, Classic Art | `--mul` (the GUI reads `artLegacyMUL.uop` first), `--atlas-width`, `--atlas-height`, `--gutter` |
| `pack-texmaps` | Partial: Assets, Classic Texmaps | `--atlas-width`, `--atlas-height`, `--gutter` |
| `pack-mobile-anims` | Partial: Assets, Classic Mobile Animations | `--atlas-width`, `--atlas-height`, `--gutter` |
| `pack-ec-textures` | Partial: Assets, Enhanced Art and Enhanced Land, as two jobs (`--krdir` is the Sources tab's Kingdom Reborn folder, else the shipped delta; `--kr-land-report` is the Enhanced Land card's Findings Report cell; the Kingdom Reborn land is packed as an overlay over the land the job just wrote) | `--kr-delta`, `--tilemeta-output`, `--tilemeta-use-ec-radarcol`, `--land-transcode-kdl` and `--land-terrain-overrides-kdl` (the GUI reads them from the routing folder, else the bundled copies), the art and land atlas sizes and gutters |
| `pack-ec-mobile-anims` | Partial: Assets, Enhanced Mobile Animations | `--tables` and `--ec-mobile-animations-kdl` (routing folder or bundled copy only), `--atlas-width`, `--atlas-height`, `--gutter` |
| `pack-ec-tex-aux` | Partial: Assets, Enhanced Auxiliary Textures | `--tilemeta` (always the input folder's `tilemeta.uddp`), `--kr-delta` |
| `make-kr-delta` | CLI only | |
| `pack-gumps` | Yes: Assets, Classic Gumps | |
| `pack-ec-gumps` | Yes: Assets, Enhanced Gumps (`--share-with` is the output folder's `gumps_cc.uddp`) | |
| `pack-map` | Partial: World, Map | `--zstd`, `--stored`; `--uop` only in the settings file (`map_preferences`) |
| `pack-statics` | Partial: World, Statics | `--zstd`, `--stored` |
| `pack-map-statics` | Partial: World, ALL, which also builds the radar | `--map-output`, `--statics-output`, `--zstd`, `--stored`; `--uop` only in the settings file |
| `pack-radar` | Partial: World, RadarMap | `--zstd-level` and `--uop` only in the settings file (`radar_zstd`, `map_preferences`); `--output` |
| `pack-lights` | Yes: Assets, World Lights (`--use-verdata` is the overview's verdata.mul) | |
| `pack-hues` | Yes: Assets, Hues (the hue ramp is greyed out when the source is an Enhanced `Hues.uop`, which is copied as it is) | |
| `pack-ec-effects` | Yes: Assets, Enhanced Effects | |
| `pack-ec-paperdoll` | Yes: Assets, Enhanced Paperdoll (`--report` is the Findings Report cell; the log panel shows each dress-layer decision) | |
| `audit-ec-material-refs` | CLI only | |
| `inventory-ec-support-textures` | CLI only | |
| `report-ec-material-baseline` | CLI only | |
| `audit-ec-terrain-primary-selection` | CLI only | |
| `report-ec-terrain-practical-review` | CLI only | |
| `audit-ec-terrain-overrides` | CLI only | |
| `audit-land-routing-kdl` | CLI only | |
| `audit-ec-surface-redirection` | CLI only | |
| `write-ec-effect-meta-manifest` | CLI only | |
| `write-ec-abnormality-report` | CLI only | |
| `write-tileart-owner-audit` | CLI only | |
| `write-terrain-definition-owner-audit` | CLI only | |
| `write-terrain-definition-review` | CLI only | |
| `write-tileart-liquid-surface-report` | CLI only | |

When an upscale profile is selected in the GUI, it is loaded at conversion start
and used only for image targets that match the profile. Existing GUI upscale
filter controls remain the fallback for unmatched targets.

### `udd-cli tool`

| Command | GUI | Not in the GUI |
| ------- | --- | -------------- |
| `info` | Yes: Tools | |
| `extract` | Partial: Tools | `--output` (the GUI extracts next to the package) |
| `hash-path` | Yes: Tools | |
| `replace` | CLI only | |
| `rebuild` | CLI only | |
| `export-csv` | Partial: Tools | `--kind` (always `auto`), `--output` |
| `export-kdl` | Partial: Tools, Export Paperdoll Overrides | `--tilemeta`, `--output` (the GUI names the items from the `tilemeta.uddp` beside the package and writes next to it) |
| `apply-tilemeta-overrides` | CLI only (packing tilemeta with its Kingdom Reborn section applies the bundled file) | |
| `import-csv` | CLI only | |
| `validate` | Yes: Tools, Validate Packages; also Validate packages after packing (Assets and World) | |
| `diff` | Partial: Tools | `--kind` (always `auto`) |

## Binary

```
udd-conv-gui
```

## Notes

- Requires a valid Classic Client or Enhanced Client installation as the source.
- Produced `.uddp` packages should be placed in, or linked into, the Dynamapper
  `assets/` tree as described in
  [docs/dev_wiki/ASSET_PIPELINE.md](../../docs/dev_wiki/ASSET_PIPELINE.md).
- Stop ends a job from its progress callbacks, before its package file is
  created. A stop asked once the package is being assembled lets the job finish,
  so no partial package is left.
- For scripted or automated builds, prefer `udd-cli pack` from
  [`udd-conv-cli`](../udd-conv-cli/README.md) directly.
