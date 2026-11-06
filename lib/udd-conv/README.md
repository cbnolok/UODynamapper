# udd-conv

`udd-conv` is the shared conversion and packaging library used by `udd-conv-cli`
and `udd-conv-gui`. It contains all format-specific conversion logic: atlas
packing, texture preparation, source decoding, and `.uddp` package assembly.

## Source layout

| Folder | Responsibility |
| --- | --- |
| `src/animation/` | Classic and Enhanced mobile animation packages |
| `src/atlas/` | Block/page compression, packing searches and pixel bounds |
| `src/gumps/` | Classic and Enhanced gumps, paperdoll atlases and single gumps |
| `src/maps/` | Classic maps, statics, radar, source resolution and patches |
| `src/metadata/` | Terrain/tile metadata, world lights and hues |
| `src/textures/` | Classic/Enhanced art, land and auxiliary texture packages |
| `src/upscaling/` | Image preparation passes and upscale profiles |

The crate root retains effect packaging, shared source/configuration resolution,
progress reporting and common atlas primitives. Existing imports such as
`udd_conv::tex_land_ec` remain available through explicit re-exports; category
paths such as `udd_conv::textures::tex_land_ec` also work.

## Responsibilities

- **Atlas packing**: bin-packing tile and art textures into atlas pages using
  `guillotiere`, with configurable page sizes and padding.
- **Classic Client conversion**: `tex_art_cc`, `tex_land_cc`, CC gumps, CC map
  and statics, CC radar, CC lighting, CC hues.
- **Enhanced Client conversion**: `tex_art_ec` + `tex_land_ec` in a shared
  source pass, EC gumps, EC lighting.
- **Tilemeta**: packing CC tiledata and EC tileart into `tilemeta.uddp`.
- **Mobile animations**: atlas packing for CC and EC mobile animation frames
  (`mobile_anim_cc`, `mobile_anim_ec`).
- **EC material routing**: original client terrain mappings and explicit KDL
  overrides for terrain definition and land texture assignment. See
  [docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md](../../docs/dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md).
- **Hues**: CC `hues.mul` and EC `hues.uop` to `hues.uddp`.
- **Classic patches**: DIF patch application helpers.
- **Source path resolution**: `source_paths.rs` helpers for locating client
  files.

## Dependencies

- `uocf`: source format parsing.
- `udd-container`: `.uddp` package writing.
- `udd-assets`: package reading helpers used during conversion validation.
- `uot-texture-codecs`: BC7 encoding for GPU-ready textures.
- `uot-image-postprocess`: pre-encoding image filtering.
- `guillotiere`: rectangle bin-packing for atlas assembly.

## Notes

- Terminal conversion progress is represented as `tracing` spans and rendered
  by CLI frontends through `tracing-indicatif`; conversion logs continue to use
  the `log` facade.
- The `bc7-encode` feature of `uot-texture-codecs` is always enabled in this
  crate's dependency.
