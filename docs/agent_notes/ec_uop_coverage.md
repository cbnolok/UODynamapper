# Enhanced Client and Kingdom Reborn UOP coverage

What uocf can decode, modify and encode in every UOP of the canonical Enhanced Client
(`_Ultima Online Enhanced`, 40 packages) and the Kingdom Reborn client (`UO - Old KR`, 35 packages), per entry
kind. Audited 2026-11-01 from the code and the tests, not from memory: every "round trip" names the test that
asserts it. Entry counts come from `entry_editor_client_packages::every_shipped_entry_passes_its_own_check`
(`lib/uocf/tests/real_files/entry_editor_client_packages.rs`), which classifies every shipped entry by content and
runs its replacement checks on it; all 292,272 entries of both clients pass.

Rerun the audit's evidence:

```sh
C=/mnt/dati/_proj_local/_uo_clients
TMPDIR=/home/claudio/test/uop/<date> UO_EC_CLIENT_DIR="$C/_Ultima Online Enhanced" UO_KR_CLIENT_DIR="$C/UO - Old KR" \
  cargo test --release -p uocf --test real_files entry_editor_client_packages:: -- --ignored --nocapture
```

## Terms

- **Decode**: *typed* = a reader that gives the entry's fields (or pixels, samples); *raw* = bytes only.
- **Modify**: *editor* = a typed edit (fields, KDL, images) that writes the entry back; *replace* = whole-entry
  replace or add only. Every entry of every package has *replace*: `uocf::enhanced::entry_edit::UopEntryEditor`
  (commit f93e35c9), `uocf-cli uop replace-entry` / `uop add-entry`, the inspector's "Replace file..." in the UOP
  explorer. The new file must be of the entry's kind (by content), and must parse where the entry has a strict
  reader ([Replacement checks](#replacement-checks)).
- **Encode**: *writer* = bytes are produced from the typed form. *RT* = a real-file test asserts that every shipped
  entry decodes and re-encodes byte for byte; *RT (synthetic)* = only unit tests do; *lossy* = re-encodes within a
  stated error; *no* = no writer.
- **Package RT**: an unchanged save of the whole package is asserted byte-identical (`SaveLayout::Preserve`).
  Asserted for every package of both clients (40 EC, 34 KR `*.uop` files) by
  `uop_package_unchanged_save::every_enhanced_client_package_saves_unchanged_byte_for_byte` and
  `every_kingdom_reborn_package_saves_unchanged_byte_for_byte`; AnimationDefinition, AnimationFrame1-6, Audio
  (EC and KR), EffectDefinitionCollection and GameData also through their typed editors. *Edit RT*: a rewrite with edits is asserted to change only
  the edited entries.

## Coverage table

| Package (EC / KR) | Entry kind (EC / KR count) | Decode | Modify | Encode | Inspector | Modules, commits |
|---|---|---|---|---|---|---|
| AnimationDefinition | item definitions (1,102 / 712) | typed, both dialects | editor (KDL export/import, package rewrite) | writer, RT EC+KR (`animation_definition_editing::every_entry_round_trips_binary_and_kdl`), package RT | Animation definitions (edit, save) | `animation_definition*` 6c0dc4b0, 6336777e |
| AnimationFrame1-6 | AMOU (31,877 / 19,617) | typed (frames, palette, hue mask) | editor (PNG folder import, frame encoder) | writer, RT EC+KR: container, indexed and RGBA paths (`animation_frame_encoding::every_frame_round_trips`), package RT | Animations (export, import) | `animationframe*` 3a5600a5, 29d69f93, 75562dc4 |
| AnimationSequence | sequences (370 / 252) | typed, both dialects | editor (frame counts via the frame package writer) | writer, RT EC (`animation_client_entries::animation_sequence_reencodes_every_client_entry`) and KR (`kr_client_files::kr_animation_sequences_reencode_identically`) | Animations | `animation_sequence.rs` 6c0dc4b0, 7fd85a38 |
| Audio | MP3 (1,764 / 1,544) | typed (probe, decode to PCM) | editor (replace/add by id or name, tables) | no MP3 encoder; the stored file is kept as given; package RT EC+KR (`audio_writers`) | Sounds (play, export, replace, add) | `audio/`, `enhanced/audio*.rs` 16bddb09, 78aaca7d, 28acc831 |
| | WAV (1,666 / 0) | typed | editor (any PCM converted with `--convert`) | writer (PCM 16 WAV); package RT | Sounds | same |
| | sound/music CSV (2 / 0) | typed (EC grammar) | editor (rows added with entries) | writer, RT EC (`audio_writers`, the tables rebuild exactly) | Sounds | `enhanced/audio.rs` |
| | KR `audio_sounds.csv`, `audio_effects.csv` (0 / 2) | typed (UOKR.exe's grammar), lookup by server id | replace (parse-checked); `audio add` adds a structure and its effects row for a new server id | writer, RT KR (`audio_client_files`, byte for byte) | UOP explorer (text editor) | `enhanced/kr_audio.rs` |
| | other KR CSV and text (3 / 7) | raw | replace | no | UOP explorer (text editor) | |
| EffectDefinitionCollection | collections (269 / 258) | typed, both dialects, KDL | editor | writer, RT EC (`effects_client_files::effect_definition_collection_round_trip`) and KR (`kr_client_files::kr_effect_collection_entries_reencode_identically`), package RT | Effects (edit, save) | `effect_definition*` a93fb9cd, 0042dbbd |
| EffectTexture | Chaos scripts `.ems` + unnamed (61 / 59) | typed | editor | writer, RT EC (`effects_client_files::chaos_scripts_round_trip`) | Effects (export, import) | `chaos_script/` 0042dbbd |
| | NIF (466 / 451) | typed (`gamebryo-nif`, inspector side) | editor (inspector NIF editor, import) | writer, RT EC (`gamebryo-nif` `real_nifs_round_trip_byte_exact`, 466 NIFs) and KR (`kr_nifs_round_trip_byte_exact`, 451 NIFs, 36,041 blocks) | Effects NIF (edit, export .nif/glTF) | `lib/ec-effects/gamebryo-nif` ac3439ae; replace parse-checked through the caller hook |
| | TGA (238 / 237) | typed (image crate) | replace (also through the effects import); an image converted to the entry's convention | writer (`tga_writer.rs`), RT EC, EC fp, KR, every entry (`tga_client_encoding`) | Effects, UOP explorer (image; "Replace file..." converts a PNG) | |
| | DDS (62 / 0) | typed | replace | writer (see Texture) | Effects, UOP explorer | `dds_writer.rs` |
| | PSD (2 / 2) | raw | replace | no | none (hex) | gap 18 |
| | package | | editor (entries replaced, added, removed) | edit RT (`effects_client_files::effect_texture_package_rewrite`) | Effects (save) | `effect_texture.rs` 0042dbbd, 622749ed |
| GameData | `legacyterrainmap.csv` (1 / 0) | typed (row order kept) | replace (shipped format checked); library edits (`set_route`, `remove_route`) | writer, RT (`terrain_definition_client_files::legacy_terrain_map_csv_writes_back_byte_for_byte`); `check_canonical` names the first line off the shipped format and is the entry editor's check: a replacement must be in the shipped format | UOP explorer (text editor) | `legacy_terrain_map.rs` 096c9ab5 |
| | other CSV tables (77 / 63, with the Windows-1252 `itemprop.csv`) | generic table (the client's CSV grammar: comments, blank lines, quoted commas, column names); typed `legacycontainers`, `weaponability`, `mobart`, `buffdata` | replace (parse-checked, shape kept: column names, field counts, column kinds, unique ids); library edits (`set_field`, `push_record`) | writer, RT EC+KR (`game_data_client_tables::every_game_data_table_reads_and_writes_back`, byte for byte) | UOP explorer (text editor) | `game_data.rs` |
| | OLE compound file (1 / 1) | raw | replace (kind check) | no | none (hex) | low value |
| Hues | hue bitmaps (3,000 / 3,000) | typed (`EcHueTable::from_bitmap`) | editor (from `hues.mul`: whole package; `--base` keeps an existing package's unedited hues) | writer; by rule 2,635 EC bitmaps byte-identical (`hues_mul_uop_client::linear_ramp_reproduces_ea_hue_data_for_unedited_hues`); on EA's package every unedited hue byte-identical, all 3,000 EC and EC fp with their Classic file, 2,972 KR with Mondain's Legacy (`hues_mul_written_on_an_ea_package_keeps_its_unedited_hues_byte_for_byte`); own packages and EA-based packages convert back to the identical `hues.mul` | Hues (view) | `hues.rs`, `hues_mul_uop.rs`, `hues_mul_uop_base.rs` 0113f3a6, f7852681 |
| | atlas, swatch DDS (2 / 2) | typed | editor (rebuilt from `hues.mul`) | writer; byte-identical on EA's package when every hue is unedited (EC, EC fp) | Hues | same |
| | `huenames.csv` (1 / 1) | typed (lenient) | editor (from `hues.mul`) | writer; byte-identical on EA's package for EC and EC fp (base lines kept for unedited names) | Hues | same |
| Interface | DDS (1,744 / 809) | typed | editor (`uop replace-texture`: image in, format and mips kept) | writer: headers RT, uncompressed RT, BC lossy within bound (`dds_client_textures`) | UOP explorer (image) | `dds_writer.rs` abd89877, 271b21aa |
| | TGA gumps (4,445 / 23) | typed (image crate; strict validation) | replace; an image converted to the entry's convention | writer (`tga_writer.rs`), RT EC, EC fp, KR, every entry | UOP explorer (image; "Replace file..." converts a PNG) | |
| | Lua and plain text (129 / 239) | raw (text); Lua syntax-checked (130 / 239 pass, `syntax_client_files`) | replace (Lua parse-checked) | no | UOP explorer (text editor, live syntax error) | |
| | XML (126 / 247) | raw (text); read as TinyXML reads it (126 / 247 pass) | replace (parse-checked) | no | UOP explorer (text editor, live syntax error) | |
| | fonts TrueType/OpenType (32 / 29) | raw (table directory checked) | replace (parse-checked) | no | none (hex) | 06e3fded |
| | 18-byte 0x0 TGA placeholders, UTF-16 text, OLE, AMOU, JPEG (39 / 3) | raw (AMOU typed, JPEG decoded) | replace (JPEG parse-checked) | no | none (hex) | low value |
| LegacyTerrain (EC) | terrain documents XML (2,723) | typed | editor | writer, RT (`legacy_terrain_client_file::every_document_writes_back_byte_for_byte`), edit RT | Legacy terrain (edit, save) | `legacy_terrain.rs` b2dcd31b |
| LegacyTexture | DDS (49,268 / 16,877) | typed | editor (`uop replace-texture`) | writer (see Interface DDS) | UOP explorer (image), Art viewer | `dds_writer.rs`, `textures.rs` |
| LocalizedStrings | string tables (8 / 3; `data/localizedstrings/%03d.cliloc`) | typed (strict UTF-8) | editor (`uocf-cli strings`, KDL) | writer, RT EC and KR, directly and through KDL; the Classic `Cliloc.*` files (same format, 8) too (`string_tables_client_files`), edit RT | Clilocs (edit, save, KDL) | `localized_strings.rs` |
| MainMisc | XML (2 / 2), UTF-16 text (1 / 1), binary (1 / 0) | raw; XML read as TinyXML reads it (2 / 2 pass) | replace (XML parse-checked) | no | UOP explorer (text editor) (XML, UTF-16) | |
| MultiCollection | multis (862 / 241) | typed, both dialects | editor (multi editor, KDL) | writer, RT EC (`multi_collection_client_entries::every_multi_reencodes_identically`) and KR (`kr_client_files::kr_multis_reencode_identically`), edit RT | Multis, Multi editor (save) | `multis.rs` 38705ade, 7fd85a38 |
| | `housing.bin` (1 / 1) | typed, KDL | editor | writer, RT EC (`housing_bin_reserializes_identically_directly_and_through_kdl`) and KR (`kr_client_files::kr_housing_bin_reserializes_identically_directly_and_through_kdl`), both directly and through KDL | Housing (edit, save) | `housing*` 02fb5dad, beeecc11 |
| Paperdoll | AMOU (2 / 2): the male and female paperdoll image sets (`PaperdollFrameSet` assets 0x40, 0x41; action = gender, number = body or worn item id) | typed (AMOU reader) | replace (parse-checked) | writer, RT EC and KR, container and frames re-encoded (`paperdoll_frame_sets::paperdoll_frame_sets_round_trip`) | none (hex) | `ghidra-paperdoll` |
| Shaders | `.psh`/`.vsh` and unnamed text (12 / 6) | raw (text) | replace (kind check) | no | UOP explorer (text editor) | |
| SystemTextures | TGA (5 / 5), PNG (2 / 2) | typed (image crate) | replace (parse-checked); TGA from an image | TGA writer, RT EC, EC fp, KR | UOP explorer (image) | |
| TerrainDefinition | definitions (201 / 191) | typed, both dialects | editor | writer, RT EC and KR (`terrain_definition_client_files::*_write_back_byte_for_byte`), edit RT | Terrain definition (edit, save) | `terrain_definition.rs` af85d41b, 59e77b3e |
| TerrainTexture, Texture | DDS (37+9,752 / 20+9,348) | typed | editor (`uop replace-texture`) | writer (see Interface DDS) | UOP explorer (image) | `dds_writer.rs`, `textures.rs` |
| facetN, facetNx | sectors (7168 per Britannia facet / KR the same per facet) | typed, both dialects | editor (sector encoder, build from Classic) | writer, RT EC (`facet_client_files::facet_sectors_reencode_identically_and_delimiters_follow_the_rule`) and KR (`kr_facet_client_files::kr_facet_sectors_reencode_identically_and_follow_the_rules`) | none in the inspector (dynamapper renders them) | `facet_*` bf596b93, 378e103e, 7fd85a38 |
| | data map (1 per package) | typed | editor (written by the facet builder) | writer, RT EC (`facet_client_files::facet_data_maps_reencode_identically`, 10 packages) and KR (`kr_facet_client_files::kr_facet_data_maps_reencode_identically`) | none | `facet_sector.rs` |
| facets (EC) | facet definitions (7) | typed | editor | writer, RT (`facet_client_files::facet_definitions_reencode_identically`) | none | `facet_definition.rs` bf596b93 |
| gumpartLegacyMUL (EC) | Classic gumps (4,342 parse; 34 are 0x0 placeholders) | typed (`decode_uop_gump`, `GumpColors`) | replace (parse-checked); from an image (`UopEntryEditor::replace_image`, `uocf-cli uop replace-image`); MUL <-> UOP repack (`uocf-cli mul-convert`) | writer of the shipped RLE, RT byte for byte on every gump that decodes: EC, EC fp 4,342, Classic 4,422, Classic fp 5,522 UOP entries, 7.0.20 3,164, Mondain's Legacy 2,600, Gold 2,416 `gumpart.mul` slots (`gump_client_encoding::every_shipped_gump_encodes_back`) | Gumps (view) | `classic/gump.rs` 7b89a450, a09b2255; `classic/gump_write.rs`, `enhanced/entry_image.rs` |
| string_dictionary | dictionary (1 / 1) | typed | editor (library only) | writer, RT EC and KR (`string_tables_client_files::*string_dictionar*`) | String dictionary (view) | `string_dictionary.rs` fe344236 |
| tileart | records (37,756 / 17,086) | typed, both dialects | editor | writer, RT EC (`tileart_client_records::tileart_reencodes_every_client_record`) and KR (`kr_client_files::kr_tileart_records_reencode_identically`), edit RT | Tile metadata, tileart editor (save) | `tileart.rs` 38705ade, 938dc6e7 |
| waypoint | waypoints (1 / 1) | typed | editor | writer, RT EC (`waypoints_client_file::waypoints_reencode_identically`) and KR (`kr_client_files::kr_waypoints_reencode_identically`) | UOP explorer (waypoint editor) | `waypoints.rs` c368a4f0, 38705ade |
| StaticChunk (KR) | static chunks (145) | typed | editor (`apply_to_package`) | writer, RT (`kr_facet_client_files::kr_static_chunks_reencode_identically`) | none | `kr/facet/static_chunk.rs` 7fd85a38 |
| TileArtLegacy (KR) | TGA (16,383), placeholder (1) | typed (image crate) | replace (parse-checked); an image converted to the entry's convention | TGA writer, RT KR, every entry | UOP explorer (image; "Replace file..." converts a PNG) | |

Summary: in the 40 EC packages, every entry can be replaced or added with content checks. 36 packages have a typed
reader for all their entries but stray files; raw text and binary remain in Interface, MainMisc and Shaders,
and the OLE file in GameData. 25 packages (AnimationDefinition, AnimationFrame1-6, AnimationSequence, EffectDefinitionCollection,
LegacyTerrain, MultiCollection, TerrainDefinition, the 10 facet packages, facets, tileart, waypoint) have a typed
editor whose writer is round-trip tested on every shipped entry; Audio, EffectTexture and Hues have editors for
their typed kinds (MP3 is stored as given, not encoded). What remains raw is text without a parser (Lua and XML have syntax checks only; CSV outside the audio
tables and GameData, shaders), the fonts, a few stray files (OLE, UTF-16 text, PSD, one JPEG) and
the unknown `MainMisc` binary. What has a reader but no writer: the
TGA images.

## Replacement checks

`UopEntryEditor` (`lib/uocf/src/enhanced/entry_edit.rs`) checks every replaced or added file:

- **Kind** (all entries): the file's content kind ([`classify_content`], families: TGA with trailing bytes = TGA,
  BMP with a wrong size field = BMP) must be the kind the name's extension declares, text for `csv lua txt ems
  psh vsh`, or the current entry's kind. Refused under every policy.
- **Parse** (where a strict reader exists): the readers, in the order tried, are Chaos script, sound table, music
  table, legacy terrain map, GameData table, legacy terrain document, hue bitmap, tileart record, animation definition, animation
  sequence, AMOU animation frames, effect definition collection, terrain definition, facet definition, facet
  sector, facet data map, housing pieces, multi, waypoints, string dictionary, localized strings, Classic gump,
  Classic art, map blocks and sound slots (`artLegacyMUL`, `map*LegacyMUL`, `soundLegacyMUL`), KR static chunk, the Interface Lua script and the Interface and MainMisc XML document syntax checks
  (`syntax_check.rs`), then the generic DDS header, font table directory, PNG/BMP/JPEG decode and TGA decode. A replacement
  must parse with the reader (and dialect) the current entry parses with, and keep what that reader asks of the
  entry it replaces (`ParseCheck::keeps`; a GameData table keeps its column names, its records' field counts, the
  kind of each column and unique ids: `game_data::keeps_shape`); an addition with one of its package's readers for
  its kind. A reader for one name (the audio tables, the legacy terrain map) or extension (`.lua`) also applies
  to a target by hash: by the name's hash, or (an extension) when the entry it replaces parses with it.
- **Syntax checks**: Lua as the client's Lua (5.1 with `continue` and the bitwise operators `& | ^^ << >>`: the
  EC lexer's token table and opcodes; the shipped scripts use `continue` and `&`), read by `full_moon` as Lua 5.3
  after a pass that keeps byte offsets (`continue` and `^^` replaced by same-length 5.3 syntax, a backtick by
  `$`: `full_moon` 3.0 panics on a backtick otherwise), with `goto`, labels, `//` and `~` refused by token;
  UTF-16 scripts too (2 EC). XML as TinyXML reads it, then `roxmltree`: attributes without whitespace between
  them get a space (EC `customizehousewindow.xml`, `managedtextures.xml`, `fontscht.xml`), a `&` starting no
  predefined entity or character reference and a `<` in an attribute value are characters (KR `icons.xml`),
  a comment ends at the first `-->` (KR `bulkorderfilter.xml` nests one), a later XML declaration is a node
  (KR `choicetext.xml`), and the elements open at the end are closed (EC and KR `startmenu.xml` lack
  `</Interface>`; whether the client reads a document TinyXML reports this error for is not traced). Errors name
  the line and column; the inspector's text editor shows the first one as the text is edited.
- **Caller checks**: readers above uocf, passed with `UopEntryEditor::with_check` and tried before the built-in
  ones. The CLI (`uop replace-entry`, `add-entry`) and the inspector (text editor, "Replace file...") add the NIF
  check: `gamebryo_nif::read_checked` reads the model and finds no wrong link; the inspector's effects import uses
  the same function.
- **Audio**: MP3 and WAV entries with a known name also pass the `Audio.uop` format checks under the caller's
  policy.
- **Kind only** (no parser in uocf): XML outside LegacyTerrain, Interface and MainMisc, Lua outside Interface, CSV outside the audio tables and
  `GameData.uop` (the KR `audio_sounds.csv` and `audio_effects.csv` are parse-checked since the audio batch),
  shaders, plain text, UTF-16 text, PSD, OLE files,
  and plain binary (the `MainMisc` binary, the TGA placeholders, the 34 0x0 gump placeholders).

## Text entries

Every shipped text entry ends its lines in CRLF. EC: 3,130 UTF-8, 6 UTF-8 with a BOM (Interface), 3 UTF-16 LE with a
BOM (2 Interface, 1 MainMisc), and 1 Windows-1252 table (`GameData.uop` `data/gamedata/itemprop.csv`, whose
`Prized` ends in an `A0` no-break space: `classify_content` calls it binary, the entry editor's kind check counts
it as text). KR: 626 UTF-8 and 1 UTF-16 LE. Some end without a final line break (EC Interface 130 of 257,
GameData 8 of 78, KR Interface 99 of 486). The inspector's text editor shows `\n` for the ending and stores the file's.

## GameData tables

Measured on the 78 EC and 63 KR text entries of `GameData.uop` (the reader: `enhanced/game_data.rs`):

- CRLF throughout (8 EC tables without the final one); UTF-8 without a BOM, but `itemprop.csv` (Windows-1252).
- Comma-separated as Excel saves: `"` toggles quoting and is dropped, quoted fields hold commas (`mobart.csv`
  descriptions, `skilldata.csv` comments), no escapes, every line has an even number of quotes; fields are not
  trimmed (some keep trailing spaces). Longest line about 460 characters; the client's line buffer holds 2,046.
- `#` rows are comments: the craft and house tables open with a `#int,int,...,string` type row, the house tables
  (`classic_housedata`, `foundationstyle`, `housesign*`, `*_cust_housedata`) and `skilldata.csv` carry 11 to 13
  of them. Blank lines separate records in the clothing tables. The column names are the row before the first
  record (`abilities.csv` has a `,,,,,Buff,...` row above them; it and `starting_wearables_human_female.csv`
  repeat a name). Every record starts with an integer id; ids are unique but in some house tables.
- Field counts match the header but in `elfskinhues.csv` (3 or 2), `playerstats.csv` (EC 7, 6 or 5; KR 6 or 5)
  and `mobart.csv` (3, or 5 with empty or blank extras). Column values are integers, `TRUE`/`FALSE` or text.
- UOSA.exe opens eight of them itself (`abilities`, `bugreport`, `equipconv`, `legacycontainers`,
  `legacyterrainmap`, `mobart`, `skilldata`, `weaponability`, with "ID exceeds limit range" messages); the
  Interface Lua reads others (`itemprop`, `tipoftheday`). That the C++ tables go through the audio tables' CSV
  reader (Ghidra-confirmed there) is probable, not traced.
- Names found by hash and added to `tools/_shared_assets/Dictionary.dic`: `femalegargoylepants`,
  `femalegargoyleshirts`, `malegargoylepants`, `malegargoyleshirts`, `tipoftheday` (the dictionary had a wrong
  `da` for it), `itemprop`. Still unnamed: 7 house customization tables (`06049F5F9520EEC6`, `180B76710FCCF153`,
  `2103671F4F662B94`, `341EC3C28A7BCB26`, `629EA7108E6459EF`, `92C3AA51CEABDC49`, `E010A4D243ABA65F`) and the OLE
  file `B31F680844B96EB9`.

## Gaps, ranked by usefulness

| # | Gap | Why it matters | Effort |
|---|---|---|---|
| 1 | ~~KR `audio_sounds.csv` strict parser, byte-identical writer, lookup by server id, adding KR rows~~ | **closed**: `enhanced/kr_audio.rs`; `audio add --sound <id> --table-path` adds the structure and its effects row (`KrAudioTables::with_sound`), checked against the client's limits | done |
| 2 | ~~Text view and editor for CSV, Lua, plain and UTF-16 text in the inspector's UOP explorer~~ | **closed**: `enhanced/entry_text.rs` keeps the encoding, BOM and line endings (every shipped text entry encodes back byte for byte: `entry_editor_client_packages::every_text_entry_encodes_back_to_its_bytes`); saved through `UopEntryEditor` to a copy | done |
| 3 | ~~Lua syntax check for replaced Interface Lua~~ | **closed**: `enhanced/syntax_check.rs` (`full_moon` 3.0.0, the client's dialect); all 130 EC and 239 KR Interface scripts pass, by name and by hash | done |
| 4 | ~~XML well-formedness check for Interface and MainMisc XML~~ | **closed**: `roxmltree` 0.21.2 after a TinyXML pass (5 loosenings, above); all 128 EC and 249 KR documents pass (and the 2,723 LegacyTerrain documents) | done |
| 5 | ~~NIF parse check in the entry editor~~ | **closed**: a caller hook (`UopEntryEditor::with_check`) that the CLI and the inspector fill with `gamebryo_nif::read_checked`; uocf does not depend on `gamebryo-nif` | done |
| 6 | ~~KR NIF round trip (451)~~ | **closed**: all 451 KR NIFs read, write back byte for byte and have no bad links | done |
| 7 | ~~`LocalizedStrings` writer and editor; `string_dictionary` round trip on the shipped files~~ | **closed**: every EC, KR and Classic table and both dictionaries write back byte for byte; `uocf-cli strings` and the Clilocs view edit them | done |
| 8 | ~~`legacyterrainmap.csv` writer~~ | **closed**: `to_csv_bytes` gives the shipped table back byte for byte; `check_canonical` for the entry editor's CSV check | done |
| 9 | ~~Hues: shipped hue bitmaps reproduced byte for byte (wrong size field, hand-picked swatches)~~ | **closed**: written on EA's package (`build_hues_uop_entries_on_base`, `uocf-cli hues mul-to-uop --base`), every unedited hue keeps its bitmap, atlas row, swatch and name line; the 2024 EC and EC fp packages come back whole from their Classic `hues.mul`. Without a base, 365 EC hues stay unreproducible: `hues.mul` holds only their 5-bit reduction | done |
| 10 | ~~Strict CSV reader for the other GameData tables~~ | **closed**: `enhanced/game_data.rs` reads every EC and KR table as the client's CSV reader does and writes it back byte for byte; replacements keep the table's shape; typed readers for `legacycontainers`, `weaponability`, `mobart`, `buffdata` | done |
| 11 | ~~Classic gump encoder for `gumpartLegacyMUL.uop`~~ | **closed**: `classic/gump_write.rs` encodes as every shipped gump (UOP and `gumpart.mul`, the same rows) and gives each back byte for byte; an image replaces a gump through `UopEntryEditor::replace_image`. The 34 that do not decode (ids 29, 33, 34, 37, 47, 49, 98, 99, 114-119, 122-141) are 8 zero bytes, the slots every `gumpart.mul` client leaves empty | done |
| 12 | ~~TGA writer of the shipped conventions~~ | **closed**: `enhanced/tga_writer.rs` reads an entry's convention (type 2, 3 or 10, depth, descriptor, the RLE packing of the tool that made it, the footer, 3ds max extension area or EC fp padding after the pixels) and writes an image the same way; all 32,624 EC, EC fp and KR TGAs come back byte for byte, from their pixels and from the decoded RGBA. Packings: single-pixel raw packets (the 2,750 EC Interface gumps without a footer, 16,026 of KR `TileArtLegacy`), the 3ds max lookahead (5 variants seen: what follows a row's last pixel, how long runs and lone pixels are packed), runs of 3 or more (one `SystemTextures` image) | done |
| 13 | ~~Facet data map round-trip test~~ | **closed**: every EC and KR `data.map` re-encodes byte for byte and matches the client size table | done |
| 14 | ~~KR `waypoint.uop` and `housing.bin` round-trip tests~~ | **closed**: both re-encode byte for byte on KR (housing also through KDL) | done |
| 15 | ~~`Paperdoll.uop` AMOU entries: what they are, container RT~~ | **closed**: the paperdoll images, one AMOU set per gender, read by the EC and CC paperdoll textures (`scripts/dev-helpers/ec-research/ghidra-paperdoll/`); both sets write back byte for byte on EC and KR, frames re-encoded from their indices | done |
| 16 | ~~Unchanged-save byte-identical test for every package~~ | **closed**: all 40 EC and 34 KR packages save back byte for byte | done |
| 17 | ~~JPEG decode in parse checks (the image crate's `jpeg` feature)~~ | **closed**: the pure-Rust `zune-jpeg` decoder; the EC and KR Interface JPEG decodes | done |
| 18 | PSD, OLE, the `MainMisc` binary | stray or unknown files | low value |

[`classify_content`]: ../../lib/uocf/src/enhanced/image_census.rs
