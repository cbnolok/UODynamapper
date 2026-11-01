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
| | KR `audio_sounds.csv`, `audio_effects.csv` (0 / 2) | typed (UOKR.exe's grammar), lookup by server id | replace (parse-checked); rows not edited | writer, RT KR (`audio_client_files`, byte for byte) | none (hex) | `enhanced/kr_audio.rs` |
| | other KR CSV and text (3 / 7) | raw | replace | no | none (hex) | |
| EffectDefinitionCollection | collections (269 / 258) | typed, both dialects, KDL | editor | writer, RT EC (`effects_client_files::effect_definition_collection_round_trip`) and KR (`kr_client_files::kr_effect_collection_entries_reencode_identically`), package RT | Effects (edit, save) | `effect_definition*` a93fb9cd, 0042dbbd |
| EffectTexture | Chaos scripts `.ems` + unnamed (61 / 59) | typed | editor | writer, RT EC (`effects_client_files::chaos_scripts_round_trip`) | Effects (export, import) | `chaos_script/` 0042dbbd |
| | NIF (466 / 451) | typed (`gamebryo-nif`, inspector side) | editor (inspector NIF editor, import) | writer, RT EC (`gamebryo-nif` `real_nifs_round_trip_byte_exact`, 466 NIFs) and KR (`kr_nifs_round_trip_byte_exact`, 451 NIFs, 36,041 blocks) | Effects NIF (edit, export .nif/glTF) | `lib/ec-effects/gamebryo-nif` ac3439ae; gap 5 |
| | TGA (238 / 237) | typed (image crate) | replace (also through the effects import) | no TGA writer of the shipped conventions | Effects, UOP explorer (image) | gap 12 |
| | DDS (62 / 0) | typed | replace | writer (see Texture) | Effects, UOP explorer | `dds_writer.rs` |
| | PSD (2 / 2) | raw | replace | no | none (hex) | gap 18 |
| | package | | editor (entries replaced, added, removed) | edit RT (`effects_client_files::effect_texture_package_rewrite`) | Effects (save) | `effect_texture.rs` 0042dbbd, 622749ed |
| GameData | `legacyterrainmap.csv` (1 / 0) | typed | replace (parse-checked) | no writer | none (hex) | `legacy_terrain_map.rs` 096c9ab5; gap 8 |
| | other CSV and text (76 / 63) | raw | replace (kind check) | no | none (hex) | gaps 2, 10 |
| | OLE compound file, Latin-1 CSV (2 / 1) | raw | replace | no | none (hex) | low value |
| Hues | hue bitmaps (3,000 / 3,000) | typed (`EcHueTable::from_bitmap`) | editor (from `hues.mul`: whole package) | writer; shipped bitmaps reproduced in part, reported only (`hues_mul_uop_client::linear_ramp_reproduces_ea_hue_data_for_unedited_hues`); own packages RT (`classic_hues_mul_round_trips_through_own_uop_in_both_modes`) | Hues (view) | `hues.rs`, `hues_mul_uop.rs` 0113f3a6, f7852681; gap 9 |
| | atlas, swatch DDS (2 / 2) | typed | editor (rebuilt from `hues.mul`) | writer; headers asserted, atlas rows of unedited hues asserted | Hues | same |
| | `huenames.csv` (1 / 1) | typed (lenient) | editor (from `hues.mul`) | writer | Hues | same |
| Interface | DDS (1,744 / 809) | typed | editor (`uop replace-texture`: image in, format and mips kept) | writer: headers RT, uncompressed RT, BC lossy within bound (`dds_client_textures`) | UOP explorer (image) | `dds_writer.rs` abd89877, 271b21aa |
| | TGA gumps (4,445 / 23) | typed (image crate; strict validation) | replace | no TGA writer of the shipped conventions | UOP explorer (image) | gap 12 |
| | Lua and plain text (129 / 239) | raw (text) | replace (kind check) | no | none (hex) | gaps 2, 3 |
| | XML (126 / 247) | raw (text) | replace (kind check) | no | UOP explorer (text) | gap 4 |
| | fonts TrueType/OpenType (32 / 29) | raw (table directory checked) | replace (parse-checked) | no | none (hex) | 06e3fded |
| | 18-byte 0x0 TGA placeholders, UTF-16 text, OLE, AMOU, JPEG (39 / 3) | raw (AMOU typed) | replace | no | none (hex) | low value |
| LegacyTerrain (EC) | terrain documents XML (2,723) | typed | editor | writer, RT (`legacy_terrain_client_file::every_document_writes_back_byte_for_byte`), edit RT | Legacy terrain (edit, save) | `legacy_terrain.rs` b2dcd31b |
| LegacyTexture | DDS (49,268 / 16,877) | typed | editor (`uop replace-texture`) | writer (see Interface DDS) | UOP explorer (image), Art viewer | `dds_writer.rs`, `textures.rs` |
| LocalizedStrings | string tables (8 / 3) | typed | replace (parse-checked) | no writer | Clilocs (view) | `localized_strings.rs` e65e9602; gap 7 |
| MainMisc | XML (2 / 2), UTF-16 text (1 / 1), binary (1 / 0) | raw | replace | no | UOP explorer (XML text) | gap 4 |
| MultiCollection | multis (862 / 241) | typed, both dialects | editor (multi editor, KDL) | writer, RT EC (`multi_collection_client_entries::every_multi_reencodes_identically`) and KR (`kr_client_files::kr_multis_reencode_identically`), edit RT | Multis, Multi editor (save) | `multis.rs` 38705ade, 7fd85a38 |
| | `housing.bin` (1 / 1) | typed, KDL | editor | writer, RT EC (`housing_bin_reserializes_identically_directly_and_through_kdl`) and KR (`kr_client_files::kr_housing_bin_reserializes_identically_directly_and_through_kdl`), both directly and through KDL | Housing (edit, save) | `housing*` 02fb5dad, beeecc11 |
| Paperdoll | AMOU (2 / 2) | typed (AMOU reader) | replace (parse-checked) | writer exists (AMOU container), not tested on these | none (hex) | gap 15 |
| Shaders | `.psh`/`.vsh` and unnamed text (12 / 6) | raw (text) | replace (kind check) | no | none (hex) | gap 2 |
| SystemTextures | TGA (5 / 5), PNG (2 / 2) | typed (image crate) | replace (parse-checked) | no | UOP explorer (image) | |
| TerrainDefinition | definitions (201 / 191) | typed, both dialects | editor | writer, RT EC and KR (`terrain_definition_client_files::*_write_back_byte_for_byte`), edit RT | Terrain definition (edit, save) | `terrain_definition.rs` af85d41b, 59e77b3e |
| TerrainTexture, Texture | DDS (37+9,752 / 20+9,348) | typed | editor (`uop replace-texture`) | writer (see Interface DDS) | UOP explorer (image) | `dds_writer.rs`, `textures.rs` |
| facetN, facetNx | sectors (7168 per Britannia facet / KR the same per facet) | typed, both dialects | editor (sector encoder, build from Classic) | writer, RT EC (`facet_client_files::facet_sectors_reencode_identically_and_delimiters_follow_the_rule`) and KR (`kr_facet_client_files::kr_facet_sectors_reencode_identically_and_follow_the_rules`) | none in the inspector (dynamapper renders them) | `facet_*` bf596b93, 378e103e, 7fd85a38 |
| | data map (1 per package) | typed | editor (written by the facet builder) | writer, RT EC (`facet_client_files::facet_data_maps_reencode_identically`, 10 packages) and KR (`kr_facet_client_files::kr_facet_data_maps_reencode_identically`) | none | `facet_sector.rs` |
| facets (EC) | facet definitions (7) | typed | editor | writer, RT (`facet_client_files::facet_definitions_reencode_identically`) | none | `facet_definition.rs` bf596b93 |
| gumpartLegacyMUL (EC) | Classic gumps (4,342 parse; 34 do not) | typed (`decode_uop_gump`) | replace (parse-checked); MUL <-> UOP repack (`uocf-cli mul-convert`) | no gump pixel encoder; repack RT (synthetic) | Gumps (view) | `classic/gump.rs` 7b89a450, a09b2255; gap 11 |
| string_dictionary | dictionary (1 / 1) | typed | editor (library only) | writer, RT (synthetic) | String dictionary (view) | `string_dictionary.rs` fe344236; gap 7 |
| tileart | records (37,756 / 17,086) | typed, both dialects | editor | writer, RT EC (`tileart_client_records::tileart_reencodes_every_client_record`) and KR (`kr_client_files::kr_tileart_records_reencode_identically`), edit RT | Tile metadata, tileart editor (save) | `tileart.rs` 38705ade, 938dc6e7 |
| waypoint | waypoints (1 / 1) | typed | editor | writer, RT EC (`waypoints_client_file::waypoints_reencode_identically`) and KR (`kr_client_files::kr_waypoints_reencode_identically`) | UOP explorer (waypoint editor) | `waypoints.rs` c368a4f0, 38705ade |
| StaticChunk (KR) | static chunks (145) | typed | editor (`apply_to_package`) | writer, RT (`kr_facet_client_files::kr_static_chunks_reencode_identically`) | none | `kr/facet/static_chunk.rs` 7fd85a38 |
| TileArtLegacy (KR) | TGA (16,383), placeholder (1) | typed (image crate) | replace (parse-checked) | no TGA writer | UOP explorer (image) | gap 12 |

Summary: in the 40 EC packages, every entry can be replaced or added with content checks. 36 packages have a typed
reader for all their entries but stray files; raw text and binary remain in GameData, Interface, MainMisc and
Shaders. 25 packages (AnimationDefinition, AnimationFrame1-6, AnimationSequence, EffectDefinitionCollection,
LegacyTerrain, MultiCollection, TerrainDefinition, the 10 facet packages, facets, tileart, waypoint) have a typed
editor whose writer is round-trip tested on every shipped entry; Audio, EffectTexture and Hues have editors for
their typed kinds (MP3 is stored as given, not encoded). What remains raw is text without a parser (Lua, XML outside LegacyTerrain, CSV outside the audio
tables and the legacy terrain map, shaders), the fonts, a few stray files (OLE, UTF-16 text, PSD, one JPEG) and
the unknown `MainMisc` binary. What has a reader but no writer: `LocalizedStrings`, `legacyterrainmap.csv`, the
TGA images and the Classic gumps (pixels).

## Replacement checks

`UopEntryEditor` (`lib/uocf/src/enhanced/entry_edit.rs`) checks every replaced or added file:

- **Kind** (all entries): the file's content kind ([`classify_content`], families: TGA with trailing bytes = TGA,
  BMP with a wrong size field = BMP) must be the kind the name's extension declares, text for `csv lua txt ems
  psh vsh`, or the current entry's kind. Refused under every policy.
- **Parse** (where a strict reader exists): the readers, in the order tried, are Chaos script, sound table, music
  table, legacy terrain map, legacy terrain document, hue bitmap, tileart record, animation definition, animation
  sequence, AMOU animation frames, effect definition collection, terrain definition, facet definition, facet
  sector, facet data map, housing pieces, multi, waypoints, string dictionary, localized strings, Classic gump,
  KR static chunk, then the generic DDS header, font table directory, PNG/BMP decode and TGA decode. A replacement
  must parse with the reader (and dialect) the current entry parses with; an addition with one of its package's
  readers for its kind.
- **Audio**: MP3 and WAV entries with a known name also pass the `Audio.uop` format checks under the caller's
  policy.
- **Kind only** (no parser in uocf): XML outside LegacyTerrain, Lua, CSV other than the audio tables (the KR
  `audio_sounds.csv` and `audio_effects.csv` are parse-checked since the audio batch) and the legacy terrain map,
  shaders, plain text, UTF-16 text, NIF, PSD, JPEG, OLE files,
  and plain binary (the `MainMisc` binary, the TGA placeholders, the 34 gumps that do not decode).

## Gaps, ranked by usefulness

| # | Gap | Why it matters | Effort |
|---|---|---|---|
| 1 | ~~KR `audio_sounds.csv` strict parser, byte-identical writer, lookup by server id~~ done (`enhanced/kr_audio.rs`); left: adding KR rows | KR sounds cannot be added with a table row | 0.25 day |
| 2 | Text view and editor for CSV, Lua, plain and UTF-16 text in the inspector's UOP explorer (today a hex dump of the first KB) | Interface Lua and GameData CSV are what EC modders edit most | 0.5 day |
| 3 | Lua syntax check for replaced Interface Lua (128 EC) | a broken UI script fails only in the client | 0.5-1 day, needs a Lua parser crate (design decision: a new dependency) |
| 4 | XML well-formedness check for Interface and MainMisc XML (EC 128, KR 249) | same as 3, for the UI layouts | 0.5 day, needs an XML crate (new dependency) |
| 5 | NIF parse check in the entry editor | a replaced NIF is only kind-checked | 0.5 day: uocf depends on `gamebryo-nif`, or the check moves to the CLI and inspector layer |
| 6 | ~~KR NIF round trip (451)~~ | **closed**: all 451 KR NIFs read, write back byte for byte and have no bad links | done |
| 7 | `LocalizedStrings` writer and editor; `string_dictionary` round trip on the shipped files | localized text cannot be edited; the dictionary writer is untested on real data | 1 day; 0.1 day |
| 8 | `legacyterrainmap.csv` writer | land routing can only be replaced whole | 0.5 day |
| 9 | Hues: shipped hue bitmaps reproduced byte for byte (wrong size field, hand-picked swatches) | edited `Hues.uop` differs from EA's in unedited hues | 1 day |
| 10 | Strict CSV reader for the other GameData tables (65 EC) | GameData CSVs are only kind-checked | 0.5 day to measure the shipped grammar, then per table |
| 11 | Classic gump encoder for `gumpartLegacyMUL.uop` | gumps can be replaced only as encoded payloads | 1 day |
| 12 | TGA writer of the shipped conventions (RLE type 10, footer, padding) for Interface gumps and TileArtLegacy | images go in as whatever TGA the user provides | 1-2 days |
| 13 | ~~Facet data map round-trip test~~ | **closed**: every EC and KR `data.map` re-encodes byte for byte and matches the client size table | done |
| 14 | ~~KR `waypoint.uop` and `housing.bin` round-trip tests~~ | **closed**: both re-encode byte for byte on KR (housing also through KDL) | done |
| 15 | `Paperdoll.uop` AMOU entries: what they are, container RT | two 6 MB animations of unknown role | 0.5 day research |
| 16 | ~~Unchanged-save byte-identical test for every package~~ | **closed**: all 40 EC and 34 KR packages save back byte for byte | done |
| 17 | JPEG decode in parse checks (the image crate's `jpeg` feature) | one entry | 0.1 day |
| 18 | PSD, OLE, the `MainMisc` binary | stray or unknown files | low value |

[`classify_content`]: ../../lib/uocf/src/enhanced/image_census.rs
