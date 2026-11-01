# Classic client file coverage

What uocf can read, modify and write in every file type of the canonical Classic client
(`_Ultima Online Classic`, 2024: 22 UOP packages, 98 MUL/IDX files, 13 `.def` files, text, config and localized
files), and in the older formats read from UO Gold, Mondain's Legacy and Classic 7.0.20 (the old `tiledata.mul`
layouts, the map and statics difs, `verdata.mul`). The Classic counterpart of [ec_uop_coverage.md](ec_uop_coverage.md).
Audited 2026-11-01 from the code and the tests, not from memory: every "round trip" names the test that asserts it,
and every listed real-file test passed on the clients below on that date.

Rerun the audit's evidence:

```sh
C=/mnt/dati/_proj_local/_uo_clients
TMPDIR=/home/claudio/test/uop/<date> UO_CC_CLIENT_DIR="$C/_Ultima Online Classic" \
  UO_CLASSIC_CLIENT_ROOTS="$C/_Ultima Online Classic:$C/Ultima Online Classic_7_0_20_0:$C/Ultima Online Mondain's Legacy:$C/Ultima Online Gold" \
  cargo test --release -p uocf --test real_files -- --ignored \
  uop_package_unchanged_save::every_classic entry_editor_client_packages::every_shipped classic_client_eras \
  classic_animation_frame_encoding gump_client_encoding hues_mul_uop_client::classic audio_writers::classic \
  audio_client_files::classic animation_client_entries::animation_sequence multi_collection_client_entries \
  tileart_client_records::tileart tileart_client_records::rewritten string_tables_client_files::classic \
  string_tables_client_files::string_dictionaries
# the dif tests take other variables:
UO_PRE7_CLIENT_DIR="$C/Ultima Online Mondain's Legacy" UO_CLIENT_ROOTS="$C" \
  cargo test --release -p uocf --test real_files -- --ignored classic_dif_usage classic_facet_difs
```

## Terms

- **Read**: *typed* = a reader that gives the fields (or pixels, samples); *raw* = bytes only; *no* = nothing in
  the repository reads the file.
- **Modify**: *editor* = a typed edit (fields, images, KDL) that writes the file back; *replace* = whole-entry
  replace or add. Every entry of every Classic UOP has *replace* through `uocf::enhanced::entry_edit::UopEntryEditor`
  (`uocf-cli uop replace-entry` / `add-entry`, the inspector's "Replace file..."): the census
  `entry_editor_client_packages::every_shipped_entry_passes_its_own_check` (`UO_CC_CLIENT_DIR`) passes on all
  94,654 Classic entries. `artLegacyMUL` entries must decode as a static or hold a land diamond
  (`classic::art::check_art_payload`), `map*LegacyMUL` entries be whole 196-byte blocks, `soundLegacyMUL` entries
  have samples after their header.
- **Write**: *writer* = bytes are produced from the typed form. *RT* = a real-file test asserts that the shipped file
  (or every shipped entry) decodes and re-encodes byte for byte; *RT (synthetic)* = only unit tests do; *raw copy* =
  the bytes are moved between containers unparsed; *no* = no writer.
- **Package RT**: an unchanged save of every Classic UOP is byte-identical (`SaveLayout::Preserve`):
  `uop_package_unchanged_save::every_classic_client_package_saves_unchanged_byte_for_byte` (`UO_CLASSIC_CLIENT_ROOTS`),
  22 packages of the canonical client and the 8 of 7.0.20.
- **MUL/UOP repack**: `uocf-cli mul-convert extract|pack` (`tools/uocf-cli/src/legacy_mul`, 198974e4, 5f0b2381)
  moves `art`, `gumpart`, `mapN`, `sound` and the multis between their MUL/IDX and `*LegacyMUL.uop`/`MultiCollection`
  forms as raw copies. On the real `artLegacyMUL`, `gumpartLegacyMUL`, `map0LegacyMUL` and `soundLegacyMUL`, an
  extract then pack gives every payload back under its name, and the packed UOP extracts to the same records
  (`legacy_mul::tests::real_classic_packages_extract_and_pack_back_to_their_payloads`, `UO_CC_CLIENT_DIR`; the
  MUL record order follows the package's entry order, so art and gump MULs differ in layout only). The test found
  and fixed two pack bugs: gump 9834 was named with seven digits (both shipped clients use eight, the name the
  client looks up), and a map whose size is not whole 4,096-block entries (every shipped one: a trailing block)
  failed to pack. The multis' repack has synthetic tests only.

## UOP packages

| Package | Entries | Read | Modify | Write | CLI / inspector | Modules, commits |
|---|---|---|---|---|---|---|
| `AnimationFrame1-4.uop` | 476 / 4,628 / 4,080 / 426 `animationlegacyframe` | typed (`ClassicUopEntry`) | editor: PNG folder + KDL import, frames replaced per action | writer, RT (`classic_animation_frame_encoding::every_classic_uop_frame_round_trips`); edit RT (`folder_imports_and_writers_change_only_the_edited_entries`) | `classicanim`; Animations view (export, import, replace) | `classic/anim_frames/uop.rs` 920d2cf9, 615cebf5 |
| `AnimationSequence.uop` | 370 | typed (shared with EC) | entries rewritten by the frame import | writer, RT (`animation_client_entries::animation_sequence_reencodes_every_client_entry`) | via `classicanim` / `animframe` | `animation_sequence.rs` |
| `MainMisc.uop` | 4 (2 TinyXML, 1 UTF-16 text, 1 binary) | XML parse check, text decode | replace; text editor | text: RT through `entry_text` | UOP explorer text view | `enhanced/entry_text.rs` |
| `MultiCollection.uop` | 862 multis + `housing.bin` | typed | editor (multis, housing KDL) | writer, RT (`multi_collection_client_entries::every_multi_reencodes_identically`, `housing_bin_reserializes_identically_directly_and_through_kdl`); edit RT (`rewritten_multi_collection_changes_only_*`) | Multis, Housing views | `enhanced/multi*`, `enhanced/housing*` |
| `artLegacyMUL.uop` | 39,386 | typed (land raw 44x44, static RLE) | replace (static or land check) | **no** | Tex/Art view (view); `mul-convert` | `classic/art.rs` 262c2743, 8fe7831b |
| `gumpartLegacyMUL.uop` | 4,456 (4,422 gumps, 34 0x0 placeholders) | typed | replace from an image (`replace_image`) | writer, RT (`gump_client_encoding::every_shipped_gump_encodes_back`) | `uop replace-image`; Gumps view (view), UOP explorer image replace | `classic/gump.rs`, `gump_write.rs` 7f5842ce |
| `map0-5(x)LegacyMUL.uop` | 8-113 blocks each, 11 packages | typed (`MapPlane::init_uop`) | replace (whole-block check) | **no** (conversion to an EC facet only: `facet from-classic`) | `facet` CLI | `classic/map.rs`, `facet_sources.rs`, `map_edition.rs` acc42a13 |
| `soundLegacyMUL.uop` | 1,655 | typed (`SoundMap`) | editor (replace, add from WAV) | writer, RT (`audio_writers::classic_sounds_and_defs_write_back_and_edit`) | `sound`; Sounds view (replace, save, export) | `classic/sound.rs`, `sound_edit.rs` 78aaca7d |
| `string_dictionary.uop` | 1 | typed | editor | writer, RT (`string_tables_client_files::string_dictionaries_write_back_byte_for_byte`) | `strings`; String dictionary view | `enhanced/string_dictionary.rs` |
| `tileart.uop` | 37,756 | typed | editor | writer, RT (`tileart_client_records::tileart_reencodes_every_client_record`); edit RT | Tile metadata view (edit, save) | `enhanced/tileart.rs` |

## MUL/IDX files

Reader checks over every era: `classic_client_eras::classic_client_files_parse_fully` (`UO_CLASSIC_CLIENT_ROOTS`:
tiledata, maps, statics, multis, hues, radarcol, art, gumps, texmaps, anims, animdata, lights, difs, verdata).

| File | Read | Modify | Write | CLI / inspector | Modules, commits |
|---|---|---|---|---|---|
| `anim.mul`-`anim5.mul` + `.idx` (Gold/ML: `anim-anim4`; fp: `anim6`) | typed | editor: frames appended or replaced per action | writer, RT (`classic_animation_frame_encoding::every_classic_mul_frame_round_trips`); edit RT | `classicanim`, `export-anim-patch`; Animations view | `classic/anim_frames/mul.rs` 920d2cf9, `anim.rs` ca9d0690 |
| `animdata.mul` | typed (every byte kept, trailing bytes too) | API (fields) | writer (`AnimData::to_bytes`), RT on every era (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | AnimData view (view) | `classic/animdata.rs` ad8451ba |
| `art.mul` + `artidx.mul` | typed | no | **no** (raw copy from the UOP) | Tex/Art view; `mul-convert` | `classic/art.rs` |
| `fonts.mul`, `unifont*.mul` (13) | typed (RGBA glyphs) | no | **no** | none | `classic/fonts.rs` c3915278 |
| `gumpart.mul` + `gumpidx.mul` (older clients) | typed | no (a gump encodes with `GumpColors::encode_rows`, but no MUL writer saves it) | rows: RT (`every_shipped_gump_encodes_back`); file: raw copy from the UOP | Gumps view; `mul-convert` | `classic/gump.rs`, `gump_write.rs` |
| `hues.mul` | typed | API only (`HuesMul::hue_mut`) | writer, RT (`hues_mul_uop_client::classic_hues_mul_round_trips_through_own_uop_in_both_modes`: the file comes back byte for byte through a written `Hues.uop`) | `hues mul-to-uop`/`uop-to-mul`; Hues view (view) | `classic/hues.rs` 074c28e9, f7852681 |
| `light.mul` + `lightidx.mul` | typed (RGBA) | no | **no** | Tex/Art view | `classic/light.rs` 3a5600a5 |
| `mapN.mul` / `mapNx.mul` (older clients) | typed | no | **no** | `facet` CLI | `classic/map.rs` |
| `multi.mul` + `multi.idx` | typed (old and HS record sizes) | no | **no** (raw unpack from `MultiCollection.uop`) | Multis view (Classic MUL source); `mul-convert` | `classic/multi.rs` ca9d0690 |
| `radarcol.mul` | typed (`Vec<Rgb555>`) | API | writer (`encode_radarcol`), RT on every era (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | Tex/Art view; `facet rebuild-radar` | `classic/radarcol.rs` 074c28e9 |
| `sound.mul` + `soundidx.mul` (older clients) | typed | editor | writer, RT (`classic_sounds_and_defs_write_back_and_edit`) | `sound`; Sounds view | `classic/sound_edit.rs` |
| `staidxN.mul` + `staticsN.mul` (`x` editions too) | typed | no | **no** | `facet` CLI | `classic/statics.rs` 79344fad |
| `texmaps.mul` + `texidx.mul` | typed | no | **no** | Tex/Art view, legacy terrain editor | `classic/land_texture.rs` |
| `tiledata.mul` (6 layouts: 32/64-bit flags, 16,384/32,768/65,536 statics) | typed, every byte kept (the unknown item bytes and the block headers too) | API (`land_tiles_mut`, `item_tiles_mut`) | writer (`TileData::to_bytes`) in the layout read, RT on every era and an edit changing only its bytes (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | CC tiledata view (view) | `classic/tiledata.rs` b5076c1d, 21c3ceb8 |
| `mapdifN`/`mapdiflN`, `stadifN`/`stadifiN`/`stadiflN` (Gold, ML; the canonical client still ships `stadif0-2`) | typed, applied on request | no | **no** | `facet from-classic --include-map-difs/--include-static-difs` | `classic/map_statics_diff.rs` a0d6cfd9, `facet_sources.rs` acc42a13; `classic_facet_difs`, `classic_dif_usage` |
| `verdata.mul` (Gold, ML) | typed, patches applied on request | no | **no** (the `.vd` anim patch writer is another format) | inspector "include verdata" | `classic/verdata.rs` a0d6cfd9 |
| `facet00-05.mul` | **no** | | | | |
| `speech.mul`, `skills.mul` + `Skills.idx`, `skillgrp.mul`, `animinfo.mul`, `palette.mul`, `sjis2uni.mul` | **no** | | | | |

## Text, def and config files

| File | Read | Modify | Write | CLI / inspector | Modules, commits |
|---|---|---|---|---|---|
| `Cliloc.*` (8 languages) | typed | editor | writer, RT (`string_tables_client_files::classic_cliloc_files_write_back_through_the_table_format`) | `strings`; Clilocs view (edit, save) | `classic/cliloc.rs` 39255886, `enhanced/localized_strings.rs` |
| `Sound.def`, `Music.def` | typed, strict | editor (table) | writer, RT (`classic_sounds_and_defs_write_back_and_edit`) | `sound`, `audio` | `classic/audio_def.rs` 4d568937 |
| `Music/Digital/Config.txt` + MP3s | typed, strict | no | **no** | `audio` | `classic/music.rs` 16bddb09 |
| `Body.def`, `Bodyconv.def` | typed | no | **no** | via the mobile packer (udd-conv) | `classic/body_def.rs`, `bodyconv_def.rs` a0d6cfd9 |
| `Login.cfg`, `Uo.cfg` | typed, keeps comments, unknown lines and endings | editor (login servers) | writer; folder census `client_login_servers::every_client_folder_tells_where_its_login_server_comes_from`, edits synthetic | `login`; patcher | `client_config/login_servers.rs` 41021f71 |
| `mobtypes.txt` | typed (udd-conv only) | no | no | mobile packer | `udd-conv/src/mobile_anim_cc.rs` |
| `Multimap.rle` | typed | editor (bitmap or PNG import) | writer, RT on every era (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | `multimap`; Multimap view (edit, save, export) | `classic/multimap_rle.rs` 9d3d7dd7 |
| `Anim1.def`, `Anim2.def`, `Corpse.def`, `Equipconv.def`, `art.def`, `gump.def`, `TexTerr.def`, `stitchin.def`, `Intrface.def`, `entitled.def` | **no** | | | | |
| `Intloc*`, `Chat.*`, `Gesture.*`, `Gt_*`, `Intro.*`, `Tilehelp.*`, `Tooltips.*`, `Professn.*`, `options.*`, `OPTNUOTD.*`, `SKILL16/30.*`, `Skilname.*` | **no** | | | | |
| `doors/floors/misc/roof/stairs/walls/teleprts.txt`, `Prof.txt`, `suppinfo.txt`, `gamegen.txt`, `metrics.txt`, `Tile256/1024.txt`, `Arttable.cfg`, `Macros.txt`, `Keynames.txt`, `Default.mac`, `*.lst` | **no** (`stairs.txt` columns are only cited by the housing labels) | | | | |
| `Anim256/1024.bin`, `Antx256/1024.bin`, `Blueprints.tbp`, `Credits.dat`, `Helpindx.dat`, `Langcode.iff`, `desktop.nwb`, `Version.*`, `*.myp`, `UO.bin`, `Data/assetmap.xml` | **no** | | | | |

The executables are read for facet sizes and login sites (`client_exe`, `uocf-cli exe`), not listed here.

## Summary

- Every Classic UOP package saves back byte for byte unchanged, and every entry passes its replacement check, a
  parse check for all but the 34 gump placeholders and the 2 `MainMisc` text and binary entries. Typed
  writers with real-file round trips cover the animations (UOP and MUL), the gumps, the sounds, `hues.mul`,
  `MultiCollection`, `tileart`, the string dictionary, the clilocs and `Sound.def`/`Music.def`.
- `tiledata.mul` (all six layouts), `radarcol.mul`, `animdata.mul` and `Multimap.rle` write back byte for byte on
  every era (UO Gold to the 2024 client). The world and art data have readers only: art, maps, statics, lights,
  texmaps, `multi.mul`, the difs and verdata. A Classic map, its statics and its art cannot be edited and
  written back as Classic files; `facet from-classic` converts them to an Enhanced Client facet instead.
- About 60 text, def and binary types have no reader; most are localized UI text or files the client no longer reads.

## Gaps, ranked by usefulness

| # | Gap | Why it matters | Effort |
|---|---|---|---|
| 1 | Map and statics writers (`mapN.mul`, `mapNLegacyMUL.uop`, `staidxN`/`staticsN`, the `x` editions) | the Classic world cannot be edited and saved as Classic files; dynamapper and server tools need it | large: block layout of both containers, statics re-indexing, byte-identical unchanged blocks |
| 2 | Art encoder (`artLegacyMUL.uop`, `art.mul`): land 44x44 and static RLE, encoded as shipped | no item or land art replacement for the Classic client (a raw replacement is only structure-checked) | medium: the static run convention must be inferred, as for gumps (7f5842ce) |
| 3 | ~~`tiledata.mul` writer, all 6 layouts~~ | **closed**: the reader keeps the unknown item bytes and the block headers; `TileData::to_bytes` writes the layout read; Gold, ML, 7.0.20, 2024 and fp come back byte for byte | done |
| 4 | ~~Parse checks for `artLegacyMUL`, `soundLegacyMUL` and `map*LegacyMUL` entries~~ | **closed**: static or land diamond, whole map blocks, samples after the sound header; all 41,594 shipped entries pass (the land/static choice by entry id is left to the art encoder, #2) | done |
| 5 | ~~`radarcol.mul` and `animdata.mul` writers~~ | **closed**: `encode_radarcol`, `AnimData::to_bytes`; byte for byte on every era | done |
| 6 | ~~Real-file round trips of `mul-convert` and `Multimap.rle`~~ | **closed**: art, gump, map and sound payloads come back on the 2024 client (two pack bugs fixed, above); `Multimap.rle` byte for byte on every era. The `MultiCollection` repack stays synthetic | done |
| 7 | `multi.mul`, `light.mul`, `texmaps.mul` writers | the modern client reads `MultiCollection.uop`; lights and texmaps are rarely edited | small each (index + payloads) |
| 8 | `Anim1/2.def`, `Corpse.def`, `Equipconv.def`, `art.def`, `gump.def`, `TexTerr.def` readers; `Body.def`/`Bodyconv.def` writers | mobile and item remapping as the client applies it | small-medium each |
| 9 | `speech.mul`, `skills.mul`/`Skills.idx`, `skillgrp.mul` readers | server and tooling reference data | small |
| 10 | Inspector views for maps, statics, fonts, def files; gump and art import in their own views | today through the UOP explorer or not at all | medium |
| 11 | Difs, verdata, fonts, `Config.txt` writers | pre-7.0 or rarely edited; `Config.txt` is in the audio agent's modules | low value |
| 12 | Localized UI text, `facetNN.mul`, `palette.mul`, `animinfo.mul`, `*.bin`, the remaining text and config files | not read by our tools; several are not read by the current client | low value |
