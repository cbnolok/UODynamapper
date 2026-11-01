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
  classic_animation_frame_encoding gump_client_encoding classic_art_encoding hues_mul_uop_client::classic audio_writers::classic \
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
  (or every shipped entry) decodes and re-encodes byte for byte; *RT but padding* = byte for byte except padding
  that the shipped files fill with old memory (the encoder writes zeros); *RT (synthetic)* = only unit tests do; *raw copy* =
  the bytes are moved between containers unparsed; *no* = no writer.
- **Package RT**: an unchanged save of every Classic UOP is byte-identical (`SaveLayout::Preserve`):
  `uop_package_unchanged_save::every_classic_client_package_saves_unchanged_byte_for_byte` (`UO_CLASSIC_CLIENT_ROOTS`),
  22 packages of the canonical client and the 8 of 7.0.20.
- **Stale UOP bytes (Classic fp)**: `map0`, `map1` and `map1xLegacyMUL.uop` of the fp client hold 1,076,787,
  960,540 and 960,540 bytes that no entry or table uses (old memory). Any uocf UOP save writes zeros there, an
  unchanged one too, so those three come back equal to an unchanged save, not to the original file; every entry
  and table is the same. Left as it is (the user, 2026-11-01): the client never reads those bytes.
- **MUL/UOP repack**: `uocf-cli mul-convert extract|pack` (`tools/uocf-cli/src/legacy_mul`, 198974e4, 5f0b2381)
  moves `art`, `gumpart`, `sound` and the multis between their MUL/IDX and `*LegacyMUL.uop`/`MultiCollection`
  forms as raw copies; the maps go through the typed map writer (below). On the real `artLegacyMUL`, `gumpartLegacyMUL` and `soundLegacyMUL`, an
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
| `artLegacyMUL.uop` | 39,386 | typed (land raw 44x44, static RLE) | replace (static or land check); from an image (`replace_image`, land or static by the entry's id) | writer, RT but padding (`classic_art_encoding::every_shipped_art_record_encodes_back`) | `uop replace-image`, `classic-image`; Art view export/import, UOP explorer image replace; `mul-convert` | `classic/art.rs` 262c2743, 8fe7831b; `art_write.rs` |
| `gumpartLegacyMUL.uop` | 4,456 (4,422 gumps, 34 0x0 placeholders) | typed | replace from an image (`replace_image`) | writer, RT (`gump_client_encoding::every_shipped_gump_encodes_back`) | `uop replace-image`, `classic-image`; Gumps view export/import, UOP explorer image replace | `classic/gump.rs`, `gump_write.rs` 7f5842ce |
| `map0-5(x)LegacyMUL.uop` | 8-113 blocks each, 11 packages | typed (`MapPlane::init_uop`) | replace (whole-block check) | writer (`classic/map_write.rs`, through `SaveLayout::Preserve` with the shipped package as template), RT on 7.0.20 and 2024 (fp: every entry, but the stale unused bytes any save zeroes) (`classic_map_writers::classic_maps_and_statics_write_back_mul_and_uop`) | `facet convert`, `facet patch-classic`, `mul-convert` | `classic/map.rs`, `facet_sources.rs`, `map_edition.rs` acc42a13 |
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
| `art.mul` + `artidx.mul` | typed | from an image (`image_files`: the record appended to a copy, its index record pointed there, the extra field kept) | records: RT but padding (`every_shipped_art_record_encodes_back`); file: raw copy from the UOP; import: `classic_image_import` | `classic-image`; Art view export/import; `mul-convert` | `classic/art.rs`, `art_write.rs` |
| `fonts.mul`, `unifont*.mul` (13) | typed (RGBA glyphs) | glyphs (`AsciiFontsFile`, `UnicodeFontData::set_glyph`) | writer, RT on every era plus an edit read back (`classic_difs_fonts_and_music::classic_fonts_write_back_byte_for_byte`): `fonts.mul` keeps its 143 bytes after the faces; `unifont*.mul` lays the glyphs out in code point order as shipped, keeping the unread bytes after the width-0 glyphs of `unifont3` on and U+FFE0's offset to the file end | Fonts view (glyphs, Unicode pages, sample text) | `classic/fonts.rs` c3915278 |
| `gumpart.mul` + `gumpidx.mul` (older clients) | typed | from an image (`image_files`: the rows appended to a copy, the index extra field the new size) | rows: RT (`every_shipped_gump_encodes_back`); file: raw copy from the UOP; import: `classic_image_import` | `classic-image`; Gumps view export/import; `mul-convert` | `classic/gump.rs`, `gump_write.rs` |
| `hues.mul` | typed | API only (`HuesMul::hue_mut`) | writer, RT (`hues_mul_uop_client::classic_hues_mul_round_trips_through_own_uop_in_both_modes`: the file comes back byte for byte through a written `Hues.uop`) | `hues mul-to-uop`/`uop-to-mul`; Hues view (view) | `classic/hues.rs` 074c28e9, f7852681 |
| `light.mul` + `lightidx.mul` | typed (RGBA; `LightMulFile`: intensities, dimensions, the bytes after the pixels) | API (`set_light`) | writer (`LightMulFile::to_bytes`), RT on every era and an edit read back by `LightMap` (`classic_indexed_mul_writers::classic_multi_light_and_texmap_files_write_back_byte_for_byte_and_edit`) | Tex/Art view | `classic/light.rs` 3a5600a5, `classic/light_write.rs` |
| `mapN.mul` / `mapNx.mul` (older clients) | typed | API (`ClassicMapBlocks::set_cell`) | writer, RT on 7.0.20 (Gold and ML: rewritten in the modern layout, the block headers being the index) | `facet convert`, `facet patch-classic`; Maps view (radar colours, tile and statics on hover) | `classic/map.rs` |
| `multi.mul` + `multi.idx` | typed (old and HS record sizes; `MultiMulFile` keeps the HS field) | API (`set_parts`) | writer (`MultiMulFile::to_bytes`) in the part size read, RT on every era and an edit read back by `MultiMap` (`classic_indexed_mul_writers::classic_multi_light_and_texmap_files_write_back_byte_for_byte_and_edit`) | Multis view (Classic MUL source); `mul-convert` | `classic/multi.rs` ca9d0690 |
| `radarcol.mul` | typed (`Vec<Rgb555>`) | API | writer (`encode_radarcol`), RT on every era (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | Tex/Art view; `facet rebuild-radar` | `classic/radarcol.rs` 074c28e9 |
| `sound.mul` + `soundidx.mul` (older clients) | typed | editor | writer, RT (`classic_sounds_and_defs_write_back_and_edit`) | `sound`; Sounds view | `classic/sound_edit.rs` |
| `staidxN.mul` + `staticsN.mul` (`x` editions too) | typed | API (`ClassicStaticsBlocks::add_static`, after the cell's statics in (y, x) order) | writer in the modern layout, RT on 7.0.20, 2024 and fp (Gold and ML: same records, modern layout) | `facet convert`, `facet patch-classic`; Maps view | `classic/statics.rs` 79344fad |
| `texmaps.mul` + `texidx.mul` | typed (`TexmapMulFile`: RGB555 colors) | API (`set_texture`, `TexmapRecord::from_rgba`) | writer (`TexmapMulFile::to_bytes`), RT on every era and an edit read back by `TexMap` (`classic_indexed_mul_writers::classic_multi_light_and_texmap_files_write_back_byte_for_byte_and_edit`) | Tex/Art view, legacy terrain editor | `classic/land_texture.rs` |
| `tiledata.mul` (6 layouts: 32/64-bit flags, 16,384/32,768/65,536 statics) | typed, every byte kept (the unknown item bytes and the block headers too) | API (`land_tiles_mut`, `item_tiles_mut`) | writer (`TileData::to_bytes`) in the layout read, RT on every era and an edit changing only its bytes (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | CC tiledata view (view) | `classic/tiledata.rs` b5076c1d, 21c3ceb8 |
| `mapdifN`/`mapdiflN`, `stadifN`/`stadifiN`/`stadiflN` (Gold, ML; the canonical client still ships `stadif0-2`) | typed, applied on request; baked into the base map (`bake_map_difs`, `bake_static_difs`: every id, a later one winning), equal to the dif-applying readers on Gold and ML (`classic_map_writers::baked_difs_match_the_dif_applying_readers`) | records (`MapDifFile`, `StaticDifFile`: a block id's last listing set, its listings removed) | writer (`classic/dif_files.rs`: the ids as listed, duplicates and order too; the unused `stadifi` extras kept), RT on every client that ships them (Gold and ML maps and statics 0-2, 2024 and fp statics 0-2) plus an edit read back (`classic_difs_fonts_and_music::classic_map_and_static_difs_write_back_byte_for_byte`); also baked | `facet from-classic --include-map-difs/--include-static-difs`, `facet convert --difs` | `classic/map_statics_diff.rs` a0d6cfd9, `facet_sources.rs` acc42a13, `dif_files.rs`; `classic_facet_difs`, `classic_dif_usage` |
| `verdata.mul` (Gold, ML) | typed, patches applied on request | patches (`VerdataFile::set_patch`, `remove_patch`) | writer (`VerdataFile::to_bytes`: count, records, payloads in record order), **unverified against a shipped file**: no client at hand ships one, so it is tested on synthetic files only (`verdata::tests::the_writer_lays_out_the_documented_layout_and_reads_back`, read back by `Verdata`). The `.vd` anim patch writer is another format | inspector "include verdata" | `classic/verdata.rs` a0d6cfd9 |
| `facet00-05.mul` | **no** | | | | |
| `speech.mul` | typed, strict (UTF-8, whole records) | records | writer, RT on every era (`classic_speech_and_skills::classic_speech_skills_and_skill_groups_write_back_byte_for_byte`) | Def files view | `classic/speech.rs` |
| `skills.mul` + `Skills.idx`, `skillgrp.mul` | typed, strict (action flag, NUL-terminated Latin-1 names; groups in range); the Unicode `skillgrp.mul` of ClassicUO too | skills and groups | writer, RT on every era plus a renamed skill and group (same test) | Def files view | `classic/skills.rs`, `indexed_mul.rs` |
| `palette.mul`, `animinfo.mul`, `sjis2uni.mul` | typed, strict as the 2024 client reads them (`ghidra-classic-text-tables`: 256 RGB colours; two-byte entries, 2000 read, a zero byte an error as the client calls it corrupted; 65,536 UTF-16 units) | colours, entries, code units | writer, RT on every era (`classic_text_tables::classic_palette_animinfo_and_sjis_write_back_byte_for_byte`) | no | `classic/small_binaries.rs` |

## Text, def and config files

| File | Read | Modify | Write | CLI / inspector | Modules, commits |
|---|---|---|---|---|---|
| `Cliloc.*` (8 languages) | typed | editor | writer, RT (`string_tables_client_files::classic_cliloc_files_write_back_through_the_table_format`) | `strings`; Clilocs view (edit, save) | `classic/cliloc.rs` 39255886, `enhanced/localized_strings.rs` |
| `Sound.def`, `Music.def` | typed, strict | editor (table) | writer, RT (`classic_sounds_and_defs_write_back_and_edit`) | `sound`, `audio` | `classic/audio_def.rs` 4d568937 |
| `Music/Digital/Config.txt` + MP3s | typed, strict | entries (`MusicConfig::set`, `remove`) | writer (`MusicConfig::to_text`: every line as read, comments, spacing and CRLF too), RT on every era plus an edit changing only its line (`classic_difs_fonts_and_music::classic_music_config_writes_back_byte_for_byte`) | `audio` | `classic/music.rs` 16bddb09 |
| `Body.def`, `Bodyconv.def` | typed; strict as the client reads them (`def_files.rs`) | records (add, edit, remove) | writer, RT on every era plus an edit read back by `BodyDef`/`BodyConvDef` (`classic_def_files::classic_def_files_read_strictly_and_write_back_byte_for_byte`) | via the mobile packer (udd-conv); Def files view | `classic/body_def.rs`, `bodyconv_def.rs` a0d6cfd9, `def_files.rs` |
| `Anim1-4.def`, `Corpse.def`, `Equipconv.def`, `art.def`, `gump.def`, `TexTerr.def` | typed, strict as the client reads them, line and column errors | records | writer, RT on every era (same test) | Def files view (which line the client uses) | `classic/def_files.rs` |
| `Login.cfg`, `Uo.cfg` | typed, keeps comments, unknown lines and endings | editor (login servers) | writer; folder census `client_login_servers::every_client_folder_tells_where_its_login_server_comes_from`, edits synthetic | `login`; patcher | `client_config/login_servers.rs` 41021f71 |
| `mobtypes.txt` | typed (udd-conv only) | no | no | mobile packer | `udd-conv/src/mobile_anim_cc.rs` |
| `Multimap.rle` | typed | editor (bitmap or PNG import) | writer, RT on every era (`classic_mul_writers::classic_tile_radar_animdata_and_multimap_files_write_back_byte_for_byte`) | `multimap`; Multimap view (edit, save, export) | `classic/multimap_rle.rs` 9d3d7dd7 |
| `stitchin.def`, `Intrface.def`, `entitled.def` | **no** | | | | |
| `Intloc*`, `Chat.*`, `Gesture.*`, `Gt_*`, `Intro.*`, `Tilehelp.*`, `Tooltips.*`, `Professn.*`, `options.*`, `OPTNUOTD.*`, `SKILL16/30.*`, `Skilname.*`; `Langcode.iff` | typed, strict (`LocalizedTextFile`: IFF `INFO`, `TEXT` as Latin-1 or UTF-16LE by the `INFO` width, `TIME`, `POSN`/`COLR`, `Tilehelp`'s `TKEY` tile groups; `LanguageCodesFile`: 135 codes) | strings and rows | writer, RT on every era: all 652 tables and `Langcode.iff` of Gold, ML, 7.0.20, 2024 and fp, plus an edited string read back (`classic_ui_files::classic_localized_text_and_language_codes_write_back_byte_for_byte`) | no | `classic/iff.rs`, `localized_text.rs` |
| `doors/floors/misc/roof/stairs/walls/teleprts.txt`, `suppinfo.txt` | typed, strict as the 2024 client reads them (`ghidra-classic-text-tables`: two header lines, then `%d` rows kept with the client's minimum count, the comment after them kept; `suppinfo`: a word under 32 bytes, the tile and 14 flags); a partial row, a number running into text and a number past 32 bits are errors, with line and column | rows | writer (`DefDocument`: every line as read), RT on every era plus an edit changing only its line (`classic_text_tables::classic_house_tables_and_suppinfo_write_back_byte_for_byte`) | no | `classic/house_tables.rs` |
| `Prof.txt` | typed, strict as the 2024 client reads it (`Begin`/`End` blocks, `strtok` keywords, `TrueName` up to the next quote, `atol` values); the shipped `Spirit Speak` and `Evaluate Intelligence` are kept as the client reads them, skill -1 with the value 0. An unknown keyword or stat, a word after a value and a name past 127 bytes are errors | lines (`ProfessionLine`), professions (`ProfessionsFile::professions`) | writer, RT on every era plus an edit changing only its line (`classic_text_tables::classic_professions_write_back_byte_for_byte`) | no | `classic/professions.rs` |
| `Keynames.txt` | typed, strict (89 names, scan codes 0-88; a line ends at LF, CR LF or LF CR; a lone CR, a NUL, a name past 510 bytes and a 90th line are errors) | names (`KeyNamesFile::set_name`) | writer, RT on every era plus an edit (`classic_text_tables::classic_key_names_write_back_byte_for_byte`) | no | `classic/key_names.rs` |
| `Tile256/1024.txt`, `Arttable.cfg` | typed; **not read by the client** (no 2024 or UO Gold client holds the names), so what ships is accepted and only what the model cannot hold is an error (`Tile*`: a tile and a number per line; `Arttable.cfg`: the `art.def` grammar) | records | writer, RT on every era plus an edit (`classic_text_tables::classic_unread_tables_write_back_byte_for_byte`) | no | `classic/legacy_tables.rs` |
| `gamegen.txt` | **no**: not read by the client (no client holds the name); 11 bytes, `metrics.txt` | | | | |
| `metrics.txt`, `Macros.txt`, `Default.mac`, `*.lst` | **no** (`metrics.txt` is written by the client, a hardware report; `macros.txt` is read with `keynames.txt`, its grammar not traced) | | | | |
| `Anim256/1024.bin`, `Antx256/1024.bin`, `Blueprints.tbp`, `Credits.dat`, `Helpindx.dat`, `desktop.nwb`, `Version.*`, `*.myp`, `UO.bin`, `Data/assetmap.xml` | **no** | | | | |

The executables are read for facet sizes and login sites (`client_exe`, `uocf-cli exe`), not listed here.

## Summary

- Every Classic UOP package saves back byte for byte unchanged, and every entry passes its replacement check, a
  parse check for all but the 34 gump placeholders and the 2 `MainMisc` text and binary entries. Typed
  writers with real-file round trips cover the animations (UOP and MUL), the gumps, the art (but padding), the sounds, `hues.mul`,
  `MultiCollection`, `tileart`, the string dictionary, the clilocs and `Sound.def`/`Music.def`.
- `tiledata.mul` (all six layouts), `radarcol.mul`, `animdata.mul`, `Multimap.rle`, `multi.mul`, `light.mul` and
  `texmaps.mul` write back byte for byte on every era (UO Gold to the 2024 client). Maps (MUL and UOP) and
  statics write back byte for byte on the modern clients and in the modern layout from Gold and ML; the difs are
  baked into them; `facet convert` moves a facet between Classic, EC and KR files and between client sizes.
  Verdata has a reader only. Art and gumps are exported as PNG and imported from an image into copies of either
  container (`classic/image_files.rs`, `uocf-cli classic-image`, the Art and Gumps views).
- The remap def files (`Body`, `Bodyconv`, `Equipconv`, `Corpse`, `art`, `gump`, `TexTerr`, `Anim1-4`) read strictly
  as the client reads them and write back byte for byte; so do `speech.mul`, `skills.mul` and `skillgrp.mul`.
- About 50 text, def and binary types have no reader; most are localized UI text or files the client no longer reads.

## Classic art encoding

`classic/art_write.rs`, measured on every record of the Classic (`artLegacyMUL.uop`), Classic fp, 7.0.20,
Mondain's Legacy and UO Gold (`art.mul`) clients (`classic_art_encoding::every_shipped_art_record_encodes_back`):

- The art id decides: below 0x4000 a land tile, else a static (item id + 0x4000). Confirmed in the 7.0.103
  client: `UOClassicArt::GetArtRecord` 0x005d7c00 (`scripts/dev-helpers/ec-research/ghidra-classic-art/`).
- Land: 2,048 bytes, the 1,012 diamond colors then 24 bytes, zero in the Classic clients, old memory in 3,725
  Mondain's Legacy and UO Gold records. Color 0 is opaque black.
- Static: `u32` byte length of the rows (not read by the client), `u16` width, height, a `u16` offset per row,
  the rows: `(x offset, run)` pairs and their colors, `(0, 0)` at the end, each row padded to 4 bytes with one
  `u16`. A row equal to an earlier one points at it. No run holds color 0 or touches the next run; no color
  sets bit 15.
- The padding words: of 973,367 in the Classic client, 12% are zero, 71% equal the word at the same offset of
  the previous record (by id) and 12% another word of the record: an uncleared buffer reused from record to
  record (probable). So about 1 static in 10 is byte for byte; all are byte for byte outside the padding.

| Client | Land records (byte for byte) | Static records (byte for byte; not decodable) |
|---|---|---|
| Classic | 2,859 (100%) | 36,527 (3,517) |
| Classic fp | 4,244 (100%) | 39,189 (3,694) |
| 7.0.20 | 2,855 (100%) | 31,648 (3,118) |
| Mondain's Legacy | 2,288 (292) | 19,460 (2,164; 1) |
| UO Gold | 1,784 (55) | 17,824 (1,635; 1) |

Every record but the two that do not decode (one each in Mondain's Legacy and UO Gold) gives back its colors
and is byte for byte outside the padding. From an image: the gump rules (alpha 0 or 255, 5-bit rounding, opaque
black stored as 0x0001 in a static); a land tile is a 44x44 image with an opaque diamond and transparent corners.

## Gaps, ranked by usefulness

| # | Gap | Why it matters | Effort |
|---|---|---|---|
| 1 | ~~Map and statics writers (`mapN.mul`, `mapNLegacyMUL.uop`, `staidxN`/`staticsN`, the `x` editions)~~ | **closed** (option B of the [proposal](proposals/2026-11-01-classic-map-writers.md)): `classic/map_write.rs`, `statics_write.rs`, `world.rs` (resize, fill, crop), `target_client.rs`, `enhanced/facet_to_classic.rs`; `facet convert` and `facet patch-classic`. The dynamapper editing path (option C) is still open | done |
| 2 | ~~Art encoder (`artLegacyMUL.uop`, `art.mul`): land 44x44 and static RLE, encoded as shipped~~ | **closed**: `classic/art_write.rs` (land or static by the art id, as the client's `UOClassicArt::GetArtRecord`, `ghidra-classic-art`); every land and static of the five clients encodes back to the same colors and bytes, but for padding words of old memory (see below); an image replaces an `artLegacyMUL` entry through `replace_image` | done |
| 3 | ~~`tiledata.mul` writer, all 6 layouts~~ | **closed**: the reader keeps the unknown item bytes and the block headers; `TileData::to_bytes` writes the layout read; Gold, ML, 7.0.20, 2024 and fp come back byte for byte | done |
| 4 | ~~Parse checks for `artLegacyMUL`, `soundLegacyMUL` and `map*LegacyMUL` entries~~ | **closed**: static or land diamond, whole map blocks, samples after the sound header; all 41,594 shipped entries pass (the land/static choice by entry id is left to the art encoder, #2) | done |
| 5 | ~~`radarcol.mul` and `animdata.mul` writers~~ | **closed**: `encode_radarcol`, `AnimData::to_bytes`; byte for byte on every era | done |
| 6 | ~~Real-file round trips of `mul-convert` and `Multimap.rle`~~ | **closed**: art, gump, map (now `classic_map_writers`) and sound payloads come back on the 2024 client (two pack bugs fixed, above); `Multimap.rle` byte for byte on every era. The `MultiCollection` repack stays synthetic | done |
| 7 | ~~`multi.mul`, `light.mul`, `texmaps.mul` writers~~ | **closed**: `classic/indexed_mul.rs` lays the payloads out from 0 in the order read (id order in every shipped file but `skills.mul`, #9), and keeps each absent record as read (they differ by file and era: `(-1,-1,-1)`, `(-1,0,0)`, `(0,0,0)`, and Mondain's Legacy's mixed `texidx.mul` with its 4-byte tail); `multi_write.rs`, `light_write.rs`, `texmap_write.rs` type the payloads. Gold, ML, 7.0.20, 2024 and fp come back byte for byte; a grown entry changes only itself and reads back through the renderer's readers | done |
| 8 | ~~`Anim1/2.def`, `Corpse.def`, `Equipconv.def`, `art.def`, `gump.def`, `TexTerr.def` readers; `Body.def`/`Bodyconv.def` writers~~ | **closed**: `classic/def_files.rs` reads them as the 2024 client does (`ghidra-classic-defs`: `ID {T1, T2} VALUE`, the first line of an id wins, `{-1}` is target 1; `bodyconv.def` and `Equipconv.def` cut at `#`, the last line wins, the Equipconv hue is hexadecimal), strictly, with line and column errors where the client would skip text, drop or misread a line; every line is kept, so all 50 shipped files of Gold, ML, 7.0.20, 2024 and fp write back byte for byte, and an edited record changes only its line | done |
| 9 | ~~`speech.mul`, `skills.mul`/`Skills.idx`, `skillgrp.mul` readers~~ | **closed**: readers and writers (`classic/speech.rs`, `classic/skills.rs`), byte for byte on Gold, ML, 7.0.20, 2024 and fp. From 7.0.20 on `skills.mul` holds 57, 56, 55 last, so `IndexedMul` keeps the data order read; UO Gold's `skillgrp.mul` name fields hold old text after the NUL, kept until a group is renamed | done |
| 10 | ~~Inspector views for maps, statics, fonts, def files; gump and art import in their own views~~ | **closed**: read-only Maps (a facet in `radarcol.mul` colours, the highest static over the land; the land tile and statics under the pointer or a click; MUL or UOP map, difs on request; Felucca in under a second), Fonts and Def files (the remap defs with the line the client uses, `bodyconv`, `Equipconv`, `speech`, `skills`, `skillgrp`) views in uocf-inspector-gui, shown when the CC path is set. The Art view (Legacy MUL and CC UOP sources) and the Gumps view (CC) export an id as PNG and import an image into a copy of the MUL pair or the UOP: the encoders' rules and errors, the conversion notes and the image as stored are shown before saving (`ui/classic_image_import.rs` on `classic/image_files.rs`; exported records import back to the same pixels on 2024, 7.0.20 and Gold, `classic_image_import`) | done |
| 11 | ~~Difs, verdata, fonts, `Config.txt` writers~~ | **closed**: `classic/dif_files.rs` (the lists as shipped, duplicates and order too), `AsciiFontsFile` and `UnicodeFontData` in `fonts.rs`, `MusicConfig::to_text`; byte for byte on every client that ships each file (difs: Gold, ML, 2024, fp; fonts and `Config.txt`: all five), with edits read back. `VerdataFile` follows the documented layout and is **unverified against a shipped file** (none at hand); synthetic tests only | done |
| 12 | ~~Localized UI text~~, ~~`palette.mul`, `animinfo.mul`, `sjis2uni.mul`~~, ~~the house tables, `suppinfo`, `Prof`, `Keynames`, `Tile256/1024`, `Arttable.cfg`~~; still open: `facetNN.mul`, `*.bin`, `Blueprints.tbp`, `Credits.dat`, `Helpindx.dat`, `desktop.nwb`, `Macros.txt`, `*.lst`, `stitchin`/`Intrface`/`entitled.def` | **partly closed**: `localized_text.rs` and `iff.rs` (652 tables and `Langcode.iff`), `small_binaries.rs`, `house_tables.rs`, `professions.rs`, `key_names.rs`, `legacy_tables.rs`, each strict as the 2024 client reads it (`ghidra-classic-text-tables`) or, for the files the client never opens, as shipped; byte for byte on Gold, ML, 7.0.20, 2024 and fp. `gamegen.txt` is not read by any client. The rest is not read by our tools (`facetNN.mul` belongs to the facet work) | low value |
