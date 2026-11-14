# uocf

`uocf` (Ultima Online Client Files) is the core parsing and format support
library for the workspace. It provides readers, writers, and data structures for
Classic Client, Kingdom Reborn, and Enhanced Client asset files, as well as the
`.uop` package infrastructure used across all tools.

## Start here

UOCF works with client files directly; it does not require UODynamapper runtime packages. It supports UOP containers and many Classic, Enhanced, and Kingdom Reborn assets, including maps and statics, art, animation, tile metadata, hues, audio, and strings. Reading, editing, and writing support varies by format.

- Looking for an application? Start with the [tools guide](../../tools/README.md), [graphical editor](../../tools/uocf-editor-gui/README.md), or [client-file CLI](../../tools/uocf-cli/README.md).
- Building with the library? Use the module overview below and generate the API reference with `cargo doc -p uocf --no-deps` from the workspace root.
- Looking for format details or coverage? See the [Enhanced and KR format notes](src/enhanced/ENHANCED_CLIENT_FILES.md), [login configuration](../../tools/uo-client-patcher-gui/docs/LOGIN_SERVER.md), [encryption and KR handshake](../../tools/uo-client-patcher-gui/docs/ENCRYPTION.md), and the dated [Classic](../../docs/agent_notes/cc_file_coverage.md) and [Enhanced/KR](../../docs/agent_notes/ec_uop_coverage.md) coverage audits. The audits record what was checked at the time; consult current code and tests for later changes.

## Bundled data

`assets/Dictionary.dic` is the shared UOP hash-to-path dictionary, maintained by
this crate alongside its readers. Tools find it through
`uocf::utils::bundled_files::HASH_DICTIONARY`; use that descriptor instead of
hardcoding another crate's source path. Release packaging copies it to
`shared/Dictionary.dic` beside the tools' `bin/` folder. Development builds fall
back to this crate's `assets/` folder. This hash dictionary is distinct from the
original client's `string_dictionary.uop` string table.

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

| Source folder | Responsibility |
| --- | --- |
| `classic/animation/` | Mobile animation frames, tile animation and body definitions |
| `classic/audio/` | Sound entries, music and audio definition tables |
| `classic/formats/` | Shared binary layouts, indices, animation codecs and verdata patches |
| `classic/graphics/` | Art, land textures, lights, palettes and image editing |
| `classic/maps/` | Maps, statics, DIF patches, facet sources and radar images |
| `classic/multis/` | Multi-tile structures and their writers |
| `classic/tables/` | Text, localization, definition files and client tables |
| `classic/ui/` | Gumps, fonts, credits and help files |

`tiledata.rs` and the shared target-client description remain at the `classic/` root.

### `enhanced`

Enhanced Client and Kingdom Reborn parsers:

| Source folder | Responsibility |
| --- | --- |
| `enhanced/animation/` | Animation definitions, frames and mobile body mappings |
| `enhanced/effects/` | Chaos scripts, effect definitions and effect resources |
| `enhanced/graphics/` | Textures, DDS/TGA writers and hue tables/conversion |
| `enhanced/land/` | Terrain definitions, original source routing and land configuration |
| `enhanced/maps/` | Facet sectors, package editing, conversion, radar and waypoints |
| `enhanced/metadata/` | Tileart records, tile properties and game-data tables |
| `enhanced/packages/` | Entry editing, image census and replacement validation |
| `enhanced/sound/` | EC audio and KR audio tables |
| `enhanced/strings/` | Localized strings and the client string dictionary |
| `enhanced/structures/` | Multis and customizable housing |

Client dialect detection and the internal shared entry-writing helper remain at the
`enhanced/` root. Nested implementation files stay beside their owning module.

Category modules own the source files. Existing public imports such as
`uocf::classic::map` and `uocf::enhanced::terrain_definition` remain available through
explicit re-exports; the category paths also work.

### `kr`

Kingdom Reborn facet format support (format shared with EC, with minor
differences). Parsed via the `enhanced` facet decoder/encoder.

### `client`

Classic login configuration and client executable inspection/patching share this module.
`login_servers.rs` reads and edits Classic `uo.cfg`/`login.cfg` and reports built-in EC/KR
addresses. `login.rs` locates executable login sites; the remaining modules handle PE
inspection, versions, facet dimensions and patches. Dormant `LoginServers*.xml` files
are ignored. See [login configuration](../../tools/uo-client-patcher-gui/docs/LOGIN_SERVER.md).

### `facet_convert.rs`

One facet between Classic, Enhanced Client and Kingdom Reborn files, with the checks
against a target client: what `uocf-cli facet convert` and the inspector's `Convert and
save a facet` both call.

### `animation_sequence.rs`

Shared animation sequence metadata helper.

## Features

- `profiling`: enables profiling hooks.
- `tile_mappings_builder`: enables the CC-to-EC tile mapping builder path.

## Notes

- This crate emits log events via the `log` facade; the application provides
  the backend (`uot-logging` in tools and the Bevy logger in `dynamapper`).
- Many operations return `color-eyre` reports; some expose specific error types.
- `memmap2` is used for large file reads.
