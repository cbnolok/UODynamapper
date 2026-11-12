# udd-conv-cli

`udd-conv-cli` is the command-line backend for building and managing UODynamapper
runtime packages. It builds one binary, `udd-cli`, with two command groups: `udd-cli pack` and `udd-cli tool`
(for example `udd-cli pack pack-art`, `udd-cli tool validate`).

## udd-cli pack

Converts Classic Client and Enhanced Client source assets into `.uddp` runtime
packages consumed by Dynamapper and the other tools at runtime.

### Packing commands

| Command | Output package | Source |
| ------- | -------------- | ------ |
| `pack-art` | `tex_art_cc.uddp` | Classic `art.mul` / `artidx.mul` |
| `pack-texmaps` | `tex_land_cc.uddp` | Classic `texmaps.mul` |
| `pack-mobile-anims` | `mobile_anim_cc.uddp` | Classic `anim*.mul` / `anim*.idx` |
| `pack-ec-mobile-anims` | `mobile_anim_ec.uddp` | EC `AnimationFrame*.uop` |
| `pack-ec-textures` | `tex_art_ec.uddp`, `tex_land_ec.uddp` | EC art and land in one pass |
| `pack-kr-land` | `tex_land_kr.uddp` | Kingdom Reborn `TerrainDefinition.uop` and `Texture.uop` with its facet land routes, with what KR lacks from the EC (`--krdir`, `--ecdir`); `tex_land_ec.uddp` layout, plus KR's grunge texture; `--report` lists each item's client |
| `pack-ec-tex-aux` | `tex_aux_ec.uddp` | EC auxiliary textures named by `tilemeta.uddp`, the missing-texture fallback and the night maps |
| `pack-ec-effects` | `effects_ec.uddp` | EC particle effects: definitions, Chaos and NIF systems, textures |
| `pack-ec-paperdoll` | `paperdoll_ec.uddp` | EC 3D paperdoll frames (`Paperdoll.uop`), lossless, and what each dress-layer item takes off (see below); hued with `hues.uddp` |
| `pack-tilemeta` | `tilemeta.uddp` | CC tiledata + EC tileart |
| `pack-map` | `mapX.uddp` | Classic `mapX.mul` |
| `pack-statics` | `staticsX.uddp` | Classic `staticsX.mul` |
| `pack-map-statics` | `mapX.uddp` + `staticsX.uddp` | Combined map+statics pass |
| `pack-lights` | `world_lights.uddp` | CC light.mul masks (`--ccdir`) |
| `pack-hues` | `hues.uddp` | Classic `hues.mul` (`--hue-ramp linear`, the default, or `flat`) or EC `hues.uop` |
| `pack-gumps` | `gumps_cc.uddp` | Classic `gumpart.mul` / `gumpidx.mul`, or `gumpartLegacyMUL.uop` |
| `pack-ec-gumps` | `gumps_ec.uddp` | EC `interface.uop` gumpart; `--share-with <gumps_cc.uddp>` leaves out the single gumps that package draws the same, so the package then needs it |
| `pack-radar` | `facet0X.dds` | Classic map and statics radar image |

Without a format flag, the art, land and auxiliary texture packers write lossless RGBA8888
pages with Zstd 19 per page; `--zstd=LEVEL` picks another level.

### Enhanced paperdoll: what a robe takes off

`pack-ec-paperdoll` also reads `tileart.uop` and `string_dictionary.uop` and measures,
for every dress-layer item (robes, dresses, tabards, epaulets...) on each player body
(human, elf and gargoyle, male and female), which of the garments the client draws
beneath it in the slots torso, legs, skirt, arms, chest and abovechest Dynamapper's
paperdolls take off, because only their edges would show past the item. The decisions
are stored in the package (`docs/dev_wiki/UDDP_FORMATS.md` section 5): the garments
each item keeps on each body. The log gets one line per item and body, naming the
garments kept (the first 8, the most in view first), then a summary:

```
paperdoll_ec: 0x1F03 robe, human male: takes off 81 of the 81 garments drawn beneath it; keeps none.
paperdoll_ec: 0xA412 tabard, human male: takes off 39 of the 147 garments drawn beneath it; keeps 0x279A Hakama (22,889 texels beyond the edge), ... and 100 more.
paperdoll_ec: 72 dress-layer items measured on 6 bodies: 113 decisions keep none of the garments drawn beneath the item, 259 some, 0 all, 0 measure none; ...
```

`--report <file>` writes every finding to a text file, one `info:` or `warning:` line
each, as `pack-ec-effects` does: the largest garment of each slot on each body, each
decision, each slot not measured and why, and one line per garment measured with its
texels near the edge and beyond it, whether it is kept, and where a slot rule (the
largest garment deciding its slot) would decide otherwise. A `warning:` line
is a body that could not be measured.
`PaperdollOverrides.kdl` in Dynamapper's `runtime_specs/paperdoll` (the user's copy in
`cache/runtime_specs/paperdoll`) overrides a decision.

Output paths (`--output`, `--art-output`, `--land-output`, `--map-output`, ...) and the
default file names above resolve against the current working directory, like any
command-line tool; absolute paths are used as given. Packers never write into
`--ccdir` / `--ecdir` unless an output path explicitly points there. `pack-radar --outdir`
puts the output file name in that folder instead.

### Upscale profiles

Image packers that support upscale passes also accept `--upscale-profile
<file.toml|file.kdl>`. A profile can set ordered pass chains per image type and
override them for a specific type/family/id target.
See [Palette-Safe Upscaling](../../docs/dev_wiki/rendering/PALETTE_SAFE_UPSCALING.md)
for the purpose, pass ordering, palette snap modes, and recommended defaults.

Supported image types are `art_land`, `art_items`, `gumps_equip`,
`gumps_non_equip`, `cc_mobile_animation_frames`,
`ec_mobile_animation_frames`, `cc_land_textures`, and `ec_land_textures`.
Mobile animation overrides use `family = body_id` and `id = source_frame_index`.
Other overrides use the asset id as `id`.
See [UPSCALE_PROFILES.md](UPSCALE_PROFILES.md) for the full profile format,
target resolution rules, tunable parameters, and practical examples.

```toml
[art_items]
passes = [
  { filter = "bilinear2x" },
  { filter = "vibrance30", factor = 0.35 },
  { filter = "unity-contrast-enhance35", intensity = 0.4, threshold = 0.08, blur_spread = 2.5 },
]

[[overrides]]
image_type = "art_items"
id = 4000
passes = [{ filter = "nearest2x" }]
```

```kdl
cc_mobile_animation_frames {
    pass "xbr2x"
    pass "local-laplacian-clarity25" radius=3 amount=0.25
}

override type="cc_mobile_animation_frames" family=42 id=3 {
    pass "nearest2x"
    pass "adaptive-log-contrast80" radius=3 gamma=0.8
}
```

### EC material audit commands

Development commands for auditing and validating EC material routing before
packaging. These produce JSON or KDL reports rather than `.uddp` packages:

- `audit-ec-material-refs`: audit direct EC material texture references
  from `tileart.uop` and `TerrainDefinition.uop`.
- `inventory-ec-support-textures`: inventory `TerrainTexture.uop` and
  `EffectTexture.uop` support resources.
- `report-ec-material-baseline`: write a developer baseline report for current
  EC material heuristics and metadata axes.
- `audit-ec-terrain-primary-selection`: write a JSON audit of
  `TerrainDefinition` primary texture selection and layer reasoning.
- `report-ec-terrain-practical-review`: write a short JSON queue of terrain material
  decisions needed before runtime use.
- `audit-ec-terrain-overrides`: validate `EcTerrainOverrides.kdl` against
  source-derived `TerrainDefinition` evidence.
- `audit-land-routing-kdl`: validate a terrain routing KDL against a
  `tex_land_ec.uddp` package.
- `audit-ec-surface-redirection`: write a JSON audit of surface-like art
  redirection through tilemeta and `tex_land_ec` provenance.

## udd-cli tool

Inspects, extracts, and edits `.uddp` and `.uddpi` package files.

### Commands

| Command | Description |
| ------- | ----------- |
| `info` | Show structural information about a package. |
| `extract` | Extract package contents into a folder. |
| `hash-path` | Compute the xxh64 virtual-path hash used by path-addressed packages. |
| `replace` | Replace one logical path-addressed file and write a rebuilt package. |
| `rebuild` | Rebuild a package image while preserving its logical files. |
| `export-csv` | Export editable CSV metadata from supported packages. |
| `export-kdl` | Write what a `paperdoll_ec.uddp` decides beneath each dress-layer item as `PaperdollOverrides.kdl` covers (`<stem>.PaperdollOverrides.kdl` beside it, or `--output`), one cover per item and set of bodies keeping the same garments, with the items' names as comments from the `tilemeta.uddp` beside it (or `--tilemeta`). Used as Dynamapper's `PaperdollOverrides.kdl`, it decides each garment of the decisions' slots as the package does; its slot rules also take off the garments the packer could not measure, which the stored decisions leave on. |
| `import-csv` | Import edited CSV metadata into a rebuilt package. |
| `diff` | Compare two packages or two metadata CSV files. |
| `validate` | Check that packages (or directories of them) open with this build's readers: container, package hash, every entry decoded with its codec and dictionary, and the package's own reader. A `gumps_ec` packed with `--share-with` is also checked, gump by gump, against the `gumps_cc.uddp` in its folder, or noted as depending on one when there is none. Exits non-zero when any fails. |

## Current Production Status

Both command groups are production-candidate tooling. They are exercised by the
conversion scripts under `scripts/uddconv/` and are the primary build path for
all Dynamapper runtime packages.

See [docs/dev_wiki/ASSET_PIPELINE.md](../../docs/dev_wiki/ASSET_PIPELINE.md)
for the full conversion workflow, package naming conventions, and CSV editing
rules.
