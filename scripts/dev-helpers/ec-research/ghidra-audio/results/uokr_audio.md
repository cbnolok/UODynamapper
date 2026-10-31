# UOKR.exe: the Kingdom Reborn audio tables

Program `UOKR.exe`, SHA-256 `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`.
**confirmed** = read in the code; **probable** = inferred from the code and the shipped data.

KR uses the full MythAudio system that UOSA.exe carries as dead code: sound structures with parameter
ranges, groups, queues, loops and polygroups, loaded from `data/audio/audio_*.csv`. The EC replaced it with
the two-column `audio_sounds.csv`.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x004951c0 | `UOAudio::InitAudio` | `AudioSystem_Init("data/audio")`, then `LoadEffectsTable` |
| 0x004937e0 | `UOAudio::LoadEffectsTable` | `audio_effects.csv`: server id → sound structure |
| 0x00726bf4 | `MythAudio::AudioSystem_Init` | |
| 0x0072821f | `MythAudio::AudioSystem_LoadCsvFiles` | `<dir>/audio_` + sounds, groups, queues, loops `.csv` |
| 0x0072836a | `MythAudio::ParseSoundsCsv` | |
| 0x00728e1b | `MythAudio::ParseGroupsCsv` | |
| 0x007291fd | `MythAudio::ParseQueuesCsv` | |
| 0x00729606 | `MythAudio::ParseLoopsCsv` | |
| 0x007264dc | `MythAudio::ParseSectionName` | |
| 0x0072ee63 | `MythAudio::ParseFloatRange` | |
| 0x0072efbf | `MythAudio::ParsePanRange` | probable |
| 0x0072ede9 | `MythAudio::ParseIntRange` | |
| 0x0072ed98 | `MythAudio::ReplaceCharWithSpace` | |
| 0x0072ec60 | `MythAudio::TrimWhitespace` | |
| 0x0072ecf1 | `MythAudio::RemoveWhitespace` | |
| 0x00729f4e | `MythAudio::ResolveNameId` | name → id table, `s_silence` reserved |
| 0x00824bf0 | `UOCsv::ReadStringField` | same reader as UOSA.exe |
| 0x00662fa4 | `UOCsv::ReadField` | |

## Q4b. `audio_sounds.csv` grammar

**confirmed** (`ParseSoundsCsv`). Fields are read with the UOSA.exe CSV reader: comma separated, double
quotes toggle, no trimming at that level.

### First field

| First field | Meaning |
|---|---|
| starts with `!` | A section directive. The next field is upper-cased and selects the mode: `POLYGROUPS` = 0, `ROLLOFF FACTOR` = 1, `DEFAULT PARAMETER` = 2, other = 3. The shipped file also has `MASTER VOLUME`, `RATE PRESETS` and `SOUND DEFAULT PARAMETER VALUES`, which fall into mode 3. |
| starts with `;` | Comment row. |
| starts with `#` | In mode 0, a polygroup row. In mode 1, a rolloff row. In mode 2, a sound entry. |
| empty, a space, or `,` | Sound entry. |
| any other character | Logged "invalid character '%c' found at beginning of line definition" and skipped. This covers the header row `w,s,id,...`. |

### Sound entry fields, in order

| # | Field | Parsing |
|---|---|---|
| 1 | type id | int. 0 = sample, 1 = stream. |
| 2 | name | Up to 0x50 characters. `s_default` is the default-parameter row. |
| 3 | filename | Up to 0x50 characters. The path is the CSV directory + the filename, joined with `_splitpath`/`sprintf`. |
| 4 | rate | Float range. |
| 5 | volume | Float range. |
| 6 | pan | Range (`ParsePanRange`). |
| 7 | offset | Int range. |
| 8 | min range | `*` or empty → −1.0 (default), else `atof`. |
| 9 | max range | `*` or empty → −1.0 (default), else `atof`. |
| 10 | repeat | `atol`. Read, then ignored. |
| 11 | mono flag | int. |
| 12 | polygroup | `*` or empty → 0, else a name lookup (`ResolveNameId`). |
| 13 | priority | int. |
| 14 | streamed flag | int. |

Trailing fields (status, usage, notes) are ignored.

### Range fields

`ParseFloatRange` and `ParseIntRange` work in four steps:

1. All whitespace is removed.
2. `%`, `-` and `*` become spaces.
3. The result is trimmed. If it is empty, the value is −1, which means the `s_default` value.
4. Otherwise `"%f %f"` (or `"%d %d"`) reads `low-high`. If that fails, one `atof`/`atol` sets both ends.

So `90-110` is a range, `100%` is 100, and `*` is the default. A negative number cannot be written: `-5`
reads as 5.

### Validation

- Each invalid field increments a counter ("min sample rate", ...). An entry with any invalid field is
  skipped ("Entry ... will be skipped due to %d invalid parameters").
- At most 4000 sound names.
- A duplicate name is skipped ("Could not generate ID ... Duplicate").
- One filename may be shared by up to 30 structures and is loaded once.

### The other tables

**confirmed** for the field order, from the parsers' error strings:

| File | Fields |
|---|---|
| `audio_groups.csv` | name, member list, repeat interval |
| `audio_queues.csv` | name, member list, interval, repeat |
| `audio_loops.csv` | name, start event, body event, end event, duration, interrupt flag, repeat |

The member lists are split with `strtok`. A group or queue with no members, or a loop with no event, is
skipped.

### `audio_effects.csv` (`LoadEffectsTable`)

**confirmed**. Rows: marker, `ID#` (`atol`), sound structure name. If the name starts with a digit it is
read as an id, otherwise it is looked up by name. `;` rows are comments. The file states, and the data
agrees, that `ID#` = server sound id + 1: "Mythic code assumes ID 0 to be invalid". The EC has no such
offset.

### `audio_music.csv`

**probable**. Fields: `,ID#,name,mp3,loop,...,priority,fade-general,fade-combat,,Legacy ID`, with ids from
10000. The file-name string is reached only through a table pointer (no direct cross-reference), so the
reader was not traced.

## What the KR parsers should change

1. **Parse with the client's rules, not a fixed column schema.** That means: the first-field markers, `!`
   sections, `;` comments, and a header row skipped as an invalid line.
2. **Parse ranges as the client does:** strip whitespace; `%`, `-`, `*` → space; empty = default;
   `low-high` or a single value.
3. **Keep unknown trailing columns** byte for byte when writing.
4. **Effects ids are server id + 1.** Show both values in the inspector.
