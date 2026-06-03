# uocf

`uocf` (Ultima Online Client Files) is the core parsing and format support
library for the workspace. It provides readers, writers, and data structures for
Classic Client, Kingdom Reborn, and Enhanced Client asset files, as well as the
`.uop` package infrastructure used across all tools.

## Module overview

### `uop_container`

Low-level `.uop` (Mythic Package) support:

- Package reading and writing (`package.rs`, `block.rs`, `file.rs`).
- Path hashing (`hash.rs`) and brute-force hash cracking (`hash_bruteforce.rs`).
- Hash dictionary loading and management (`hash_dictionary.rs`).
- Compression codec dispatch (`codec.rs`, `compression.rs`): zlib/deflate,
  zlib-bwt, and raw.
- Hash template expansion for dictionary population (`template.rs`).

### `classic`

Classic Client (`.mul` / `.idx`) parsers:

- `map.rs`, `statics.rs`, `map_statics_diff.rs`: world map tiles and static objects, DIF patches.
- `art.rs`, `land_texture.rs`: art and land texture bitmaps.
- `tiledata.rs`: land and item tile metadata tables.
- `anim.rs`, `animdata.rs`, `animationframe_cc.rs`: mobile animation frames and sequence tables.
- `gump.rs`, `fonts.rs`: UI gumps and bitmap fonts.
- `hues.rs`: hue palette tables.
- `sound.rs`: audio entries.
- `multi.rs`, `multimap_rle.rs`: multi-tile structures and radar map RLE.
- `light.rs`, `radarcol.rs`, `verdata.rs`: lights, radar colors, and verdata patches.
- `cliloc.rs`: localized string tables.
- `body_def.rs`, `bodyconv_def.rs`, `generic_def.rs`: body/animation definition files.
- `vd_codec.rs`: `.vd` (animation patch stream) codec.
- `michelangelo_uop_codec.rs`: Michelangelo-style animation UOP codec.

### `enhanced`

Enhanced Client and Kingdom Reborn parsers:

- `facet_decoder.rs`, `facet_encoder.rs`: KR/EC facet sector decode and encode.
- `terrain_definition.rs`: `TerrainDefinition.uop` material entries, texture
  references, shader hints, and manual override fields.
- `tileart.rs`: `tileart.uop` item/static records, art windows, flags, and
  surface-like/liquid-like classification evidence.
- `textures.rs`: EC texture package access.
- `animationframe.rs`: EC `AnimationFrame*.uop` mobile animation frames.
- `hues.rs`: EC `hues.uop`.
- `multis.rs`: EC multi-tile structures.
- `localized_strings.rs`, `string_dictionary.rs`, `cliloc.rs`: EC string data.
- `terrain.rs`, `terrain_config.rs`: land configuration helpers.
- `tile_database.rs`, `waypoints.rs`, `classic_tile_mapper.rs`: supporting EC data.

### `kr`

Kingdom Reborn facet format support (format shared with EC, with minor
differences). Parsed via the `enhanced` facet decoder/encoder.

### `animation_sequence.rs`

Shared animation sequence metadata helper.

## Features

- `profiling`: enables profiling hooks.
- `tile_mappings_builder`: enables the CC-to-EC tile mapping builder path.

## Notes

- This crate emits log events via the `log` facade; the application provides
  the backend (`udd-logging` in tools and the Bevy logger in `dynamapper`).
- All public error types use `color-eyre`.
- `memmap2` is used for large file reads.
