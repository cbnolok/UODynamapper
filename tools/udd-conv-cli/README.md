# udd-conv-cli

`udd-conv-cli` is the command-line backend for building and managing UODynamapper
runtime packages. It provides two binaries: `udd-pack` and `udd-tool`.

## udd-pack

Converts Classic Client and Enhanced Client source assets into `.uddp` runtime
packages consumed by Dynamapper and the other tools at runtime.

### Packing commands

| Command | Output package | Source |
| ------- | -------------- | ------ |
| `pack-tex-art-cc` | `tex_art_cc.uddp` | Classic `art.mul` / `artidx.mul` |
| `pack-tex-land-cc` | `tex_land_cc.uddp` | Classic `texmaps.mul` |
| `pack-mobile-anim-cc` | `mobile_anim_cc.uddp` | Classic `anim*.mul` / `anim*.idx` |
| `pack-mobile-anim-ec` | `mobile_anim_ec.uddp` | EC `AnimationFrame*.uop` |
| `pack-tex-art-ec` | `tex_art_ec.uddp`, `tex_land_ec.uddp` | EC art and land in one pass |
| `pack-tilemeta` | `tilemeta.uddp` | CC tiledata + EC tileart |
| `pack-map` | `mapX.uddp` | Classic `mapX.mul` |
| `pack-statics` | `staticsX.uddp` | Classic `staticsX.mul` |
| `pack-map-statics` | `mapX.uddp` + `staticsX.uddp` | Combined map+statics pass |
| `pack-world-lights` | `world_lights.uddp` | CC and EC lighting textures |
| `pack-hues` | `hues.uddp` | Classic `hues.mul` or EC `hues.uop` |
| `pack-gumps-cc` | `gumps_cc.uddp` | Classic `gumpart.mul` / `gumpidx.mul` |
| `pack-gumps-ec` | `gumps_ec.uddp` | EC `interface.uop` gumpart |
| `pack-radar` | `facet0X.dds` | Classic map and statics radar image |

### Upscale profiles

Image packers that support upscale passes also accept `--upscale-profile
<file.toml|file.kdl>`. A profile can set ordered pass chains per image type and
override them for a specific type/family/id target.

Supported image types are `art_land`, `art_items`, `gumps_equip`,
`gumps_non_equip`, `cc_mobile_animation_frames`,
`ec_mobile_animation_frames`, `cc_land_textures`, and `ec_land_textures`.
Mobile animation overrides use `family = body_id` and `id = source_frame_index`.
Other overrides use the asset id as `id`.

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

- `audit-ec-material-texture-refs`: audit direct EC material texture references
  from `tileart.uop` and `TerrainDefinition.uop`.
- `inventory-ec-support-resources`: inventory `TerrainTexture.uop` and
  `EffectTexture.uop` support resources.
- `ec-material-baseline-report`: write a developer baseline report for current
  EC material heuristics and metadata axes.
- `audit-terrain-definition-texture-selection`: write a JSON audit of
  `TerrainDefinition` primary texture selection and layer reasoning.
- `terrain-decision-queue`: write a short JSON queue of terrain material
  decisions needed before runtime use.
- `compare-terrain-definition-kdl`: compare `TerrainDefinition.kdl` against
  `TerrainDefinition.uop` and report manual-only fields.
- `terrain-definition-review-skeleton`: write a KDL review skeleton for
  `TerrainDefinition` manual integration candidates.
- `validate-ec-terrain-overrides`: validate `EcTerrainOverrides.kdl` against
  source-derived `TerrainDefinition` evidence.
- `validate-terrain-routing-package`: validate a terrain routing KDL against a
  `tex_land_ec.uddp` package.
- `audit-surface-like-art-redirection`: write a JSON audit of surface-like art
  redirection through tilemeta and `tex_land_ec` provenance.

## udd-tool

Inspects, extracts, and edits `.uddp` and `.uddpi` package files.

### Commands

| Command | Description |
| ------- | ----------- |
| `info` | Show structural information about a package. |
| `extract` | Extract package contents into a folder. |
| `hash` | Compute the xxh64 virtual-path hash used by path-addressed packages. |
| `replace` | Replace one logical path-addressed file and write a rebuilt package. |
| `rebuild` | Rebuild a package image while preserving its logical files. |
| `export-csv` | Export editable CSV metadata from supported packages. |
| `import-csv` | Import edited CSV metadata into a rebuilt package. |
| `diff` | Compare two packages or two metadata CSV files. |

## Current Production Status

Both binaries are production-candidate tooling. They are exercised by the
conversion scripts under `scripts/uddconv/` and are the primary build path for
all Dynamapper runtime packages.

See [docs/dev_wiki/ASSET_PIPELINE.md](../../docs/dev_wiki/ASSET_PIPELINE.md)
for the full conversion workflow, package naming conventions, and CSV editing
rules.
