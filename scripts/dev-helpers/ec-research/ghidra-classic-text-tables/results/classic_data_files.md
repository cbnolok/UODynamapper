# Classic house tables, prof.txt, keynames.txt and the small binaries

Read in `classic_7.0.103.0_client.exe` (the 2024 Classic client), 2026-11-01; the file names were also searched
in `gold_client.exe`. Names: [classic_data_file_names.md](classic_data_file_names.md).

Every file below is opened through `0x005653f0` (the client's file open: a name and an `fopen` mode). The text
files use mode `"r"` (text mode: CR LF reads as LF), the binaries `"rb"`.

## Which files the client opens

**Confirmed** by the name strings and their cross-references:

- read: `walls.txt`, `floors.txt`, `doors.txt`, `misc.txt`, `stairs.txt`, `teleprts.txt`, `roof.txt`,
  `suppinfo.txt`, `prof.txt`, `keynames.txt` (then `macros2d.txt`, copied from `macros.txt`), `palette.mul`,
  `animinfo.mul`, `language\sjis2uni.mul` (then `sjis2uni.mul`).
- **not read**: `Arttable.cfg`, `Tile256.txt`, `Tile1024.txt`, `gamegen.txt`. Neither the 2024 client nor UO Gold
  holds these names (searched as substrings without case). The `ArtTable`, `ArtTable_TileArt`, `ArtTable_Sound`
  and `ArtTable_Body` classes are used by `LoadDefFiles` (`0x00447c0f`): they are the `art.def`, `sound.def` and
  `body.def` remaps, not `Arttable.cfg`. `Arttable.cfg` has the `art.def` syntax (`3 { 3 } -1`).
- `version.dat`: the path is built and the file opened and closed at once, then another call follows; the content
  is not read there. **Probable** (the code at `0x005fbf22` is not a function in the analysis and was read as a
  disassembly only).

## The custom-house tables

`HouseTables_LoadAll` (`0x00576920`) loads each table through `HouseTable_Load` (`0x00575bf0`) with a kind:
walls 1, floors 2, doors 3, stairs 4, misc 5, teleporters 6, roof 7. **Confirmed.**

- `fgets(buffer, 255)` per line (254 bytes with the line feed). Lines 1 and 2 are skipped (the column types and
  names; `doors.txt` has a tab-only second line, so its names row comes third and is read as a row).
- Every later line gets a new row object of the kind (`*Row_Construct`, all fields 0) whose `Parse` (vtable slot
  0) is one `sscanf` of `%d`s; the row is kept when more than the minimum minus one is read, else deleted:

| File | `%d`s | Kept with | Parse |
|---|---|---|---|
| `walls.txt` | 18 | 17 | `WallsRow_Parse` `0x0056f580` |
| `floors.txt` | 18 | 17 | `FloorsRow_Parse` `0x0056f5f0` |
| `doors.txt` | 10 | 9 | `DoorsRow_Parse` `0x0056f6d0` |
| `misc.txt` | 12 | 11 | `MiscRow_Parse` `0x0056f770` |
| `stairs.txt` | 15 | 14 | `StairsRow_Parse` `0x0056f810` |
| `teleprts.txt` | 18 | 17 | `TeleportersRow_Parse` `0x0056f660` |
| `roof.txt` | 20 | 18 | `RoofRow_Parse` `0x0056f8c0` |

- The number columns are those of the shipped header rows (`Category`, ..., `FeatureMask`); the `Comment` column
  after them is not read. The shipped tables of Gold, ML, 7.0.20, 2024 and fp have every row complete and no line
  longer than 162 bytes.

## `suppinfo.txt`

`SuppInfo_Load` (`0x00443770`). **Confirmed.**

- `fgets(buffer, 255)`, lines 1 and 2 skipped, then `sscanf("%s %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d")`:
  a word into a 32-byte buffer (the shipped files write `\`, `./`, `v`, `o`, `*`, `.` and others), the tile and
  14 flags, each kept as `!= 0`. A line with fewer than all 16 conversions is dropped; UO Gold's and ML's section
  notes (`MISC TABLE STARTS HERE`, tab-indented names) are such lines.

## `prof.txt`

`Professions_Load` (`0x005252b0`), only when no profession is loaded yet. **Confirmed.**

- `fgets(buffer, 255)` until the end of the file or a failed read.
- `strstr(line, "Begin")`: a new profession (`0x13c` bytes); else `strstr(line, "End")`: the open profession is
  added. Both match anywhere in the line, case-sensitive.
- Any other line: `strtok(line, " \t\n\r")` gives the keyword, compared with `_stricmp`:

| Keyword | Value |
|---|---|
| `Name` | `strtok(NULL, " \t\n\r\"")`, copied into the 128 bytes at +0 |
| `TrueName` | `strtok(NULL, "\"")`: from the byte after the keyword's separator to the next quote, into the 128 bytes at +0x80. `TrueName\t\t"Samurai"` (every shipped file) reads as one tab |
| `Desc`, `NameId`, `DescId` | `atol` of the next word (`" \t\n\r\""`), at +0x100, +0x104, +0x108 |
| `TopLevel` | the byte at +0x111 is set when the next word is `true` |
| `Gump` | `atol` of the next word (`" \t\n\r"`), at +0x10c |
| `Type` | +0x110 = the next word is `Category` |
| `Children` | every further word (`" ,.\t\n\r\""`), each copied and appended |
| `Skill` | the next word through `SkillId_FromName` (`0x0051dbe0`: 58 names, `Alchemy` 0 to `Throwing` 57, any other -1), then `atol` of the word after it, both kept as 16 bits |
| `Stat` | `Str`, `Dex` or `Int`, then `atol` of the next word, at +0x112, +0x114, +0x116 |

  Other keywords are ignored. A keyword line before the first `Begin` writes through a null profession.
- **The 7.0.20, 2024 and fp files write `Spirit Speak` and `Evaluate Intelligence`** (UO Gold's and ML's
  professions have three skills, none of them with a space): the client reads skill `Spirit` (-1) with
  `atol("Speak")` = 0.

## `keynames.txt`

`KeyNamesAndMacros_Load` (`0x00589820`) reads 89 lines with `KeyNames_ReadLine` (`0x00588950`) into a table of
scan codes 0 to 88; a macro's key is looked up there and turned into a virtual key with
`MapVirtualKeyA(scan code, MAPVK_VSC_TO_VK)`. **Confirmed.**

- `KeyNames_ReadLine`: `fgetc` up to a line feed or a carriage return; a CR LF or LF CR pair is one line end, two
  CRs are two. At most 510 bytes are kept (a 512-byte buffer whose last two bytes are set to 0). At the end of
  the file the remaining lines read as empty.
- Without the file the 89 names are null. The shipped file (identical on every client) has 89 lines, `NULL`,
  `Esc`, `1`, ...

## `palette.mul`, `animinfo.mul`, `sjis2uni.mul`

- `Palette_Load` (`0x0056a100`): `fread` of 0x300 bytes, 256 RGB triples, used for the 8-bit desktop palette
  (`CreatePalette` when the desktop has 8 bits per pixel). **Confirmed.**
- `AnimInfo_Load` (`0x0056a970`): 2000 entries of two bytes, read a byte at a time. An entry with a zero byte is
  set to 4, 2 and the client reports the file corrupted; without the file every entry is 4, 2. The 7.0.20 and
  later files have 4000 bytes (2000 entries), UO Gold's and ML's 2000 bytes (1000 entries; the 2024 client would
  find the rest zero). **Confirmed.**
- `sjis2uni.mul` (code at `0x005eabf0`, read as a disassembly): `language\sjis2uni.mul`, else `sjis2uni.mul`;
  `fread` of 0x10000 16-bit entries into a 0x20000-byte table, the Shift-JIS to Unicode map of the Japanese
  auto-translation ("Cannot open file 'sjis2uni.mul'" otherwise). **Confirmed** from the disassembly.
