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
| 0x0072773a | `MythAudio::FindStructureHandle` | name → structure handle, through six name tables |
| 0x0072664d | `MythAudio::NameTable_FindId` | the id stored with a name, 0 when absent |
| 0x0072667f | `MythAudio::NameTable_AddName` | a new name's id; 0 for a name already there |
| 0x00826211 | `MythAudio::IdAllocator_Next` | probable |
| 0x008262bc | `MythAudio::IdAllocator_FindFree` | bitmap id allocator |
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
read as an id, otherwise it is looked up by name. `;` rows are comments.

The id is a **structure handle** (**confirmed**: both branches store into the same map value). A name goes
through `FindStructureHandle`, which looks it up in six name tables in turn and returns `id | kind`: the sound
structures (+0x80, `| 0x10000`), then +0xac (`| 0x20000`), +0xd8 (`| 0x40000`), +0x104 (`| 0x80000`), +0x15c
(`| 0x1000000`) and +0x188 (`| 0x400000`), presumably groups, queues, loops and the rest. A value starting with a
digit is `atol`'d and stored as is, so `65537` (0x10001) is the sound structure of id 1.

Sound structure ids: `ParseSoundsCsv` gives an entry an id with `NameTable_AddName` on the +0x80 table only
after every field passed and when the name is not `s_default`; a name already there gets 0 (the "Duplicate
sound sample name" message, no id); at most 4000. The table's ids come from a bitmap allocator
(`IdAllocator_FindFree`: the first free id from the hint, which then moves past it). Nothing is freed while the
tables load, so the structures are numbered in the order they are accepted. **probable**: that the first id is 1
(a record is at `id * 0x98 - 0x98` in the array, so id 1 is the first record), and that `audio_sounds.csv` is
the only file adding to the +0x80 table (the only callers are the CSV parsers, each on its own table). The file states, and the data
agrees, that `ID#` = server sound id + 1: "Mythic code assumes ID 0 to be invalid". The EC has no such
offset.

### `audio_music.csv`

**probable**. Fields: `,ID#,name,mp3,loop,...,priority,fade-general,fade-combat,,Legacy ID`, with ids from
10000. The file-name string is reached only through a table pointer (no direct cross-reference), so the
reader was not traced.

## Playback: how the client drives Miles

Read in Ghidra on 2026-11-03; Miles here is 7.0x. **confirmed** = read in the code. It is a static reading, not a
measurement.

**Driver** (`AudioDevice_OpenDriver` 0x007241f3, from `AudioDevice_Startup` 0x007225dd). **confirmed**: 44,100 Hz
(0xbfd228), 16 bits (0xbfd22c), spec 0x10 (0xbfd234), constants in `.data`. On failure it halves the rate down to
11,025, then goes to 8 bits. No DS3D, EAX or other provider opens.

**Volume.** **confirmed**:

- A 2D sound (`Sound_StartSample` 0x0072428d): `AIL_set_sample_volume_pan(h, vol, pan)` (0x00724458), so **`v ^ (5/3)`
  and the pan law apply**, with the same exponent and centre factor in the shipped 7.0x (0x2112afb0). `vol = category *
  own` (`FUN_007244c3`). Updates and fades are `FUN_0072368c`, `FUN_00723884`, `FUN_00723779`, `FUN_0072391d`.
- A 3D sound (`Sound_Update3D` 0x00724823): `AIL_calculate_3D_channel_levels` (0x0072492a) with both cone angles at
  360, outer volume 1, the sound's own minimum and maximum, and the listener's position and rolloff. The result is
  multiplied by `vol` (linear) and set with `AIL_set_sample_channel_levels` (0x00724974). It runs again each tick when
  the listener moved (0x00722925). `AIL_set_sample_3D_position` is not imported.
- The distance units are not determined: the packet position goes through `FUN_0045b6a0`.

**Occlusion**, in the same function. **confirmed**. With `d` the source to listener distance
(`FUN_0071bd90`):

```text
t = 0 below 2000, 1 at 18000 or more, else (d - 2000) / 16000
occlusion = 0.5 * log(400 * t) / log(20), at least 0     when t > 0
```

So 0 up to `d` = 2040, about 0.88 at `d` = 10000, and 1.0 from 18000. It is set only when it changed (0x00724a6d). In
7.0x the cut-off is `(1 - occlusion) + 0.01` (0x2111d10a) and the filter is off above 0.999 (0x2112bfd0), so it starts
just past 2040.

**Distance delay.** **confirmed** for the first call. `Sound_StartSample` adds the `Delay Filter` as the sample's
stage 2 (0x00724409) when the sound's maximum distance reaches 8000, with `Max Delay Time` 600. `Sound_SetDistanceDelay`
(0x00724b19) then sets `Mix` to 0 below 8000, and between 8000 and 20000 from 0.05 to 0.4, with `Delay TimeL` and `R`
from 300 to 600 ms. **probable**: the value it gets on the tick (0x00722932) is the distance, not proven.

**Reverb.** `AIL_set_sample_reverb_levels` is reached (`FUN_00727f65` 0x00728065, `FUN_0072d1d1` 0x0072d924), but the
function that sets the master reverb (`FUN_00724517`) has no caller, so the wet path has no reverb to send to unless
7.0x has a non-zero default, which was not checked.

**Music.** **confirmed**, for any sound of the stream type:

- `HandlePlayMusicPacket` (0x00496de0): `id & 0xfff == 0xfff` stops. Otherwise it stops the startup event and the
  current one, then triggers the new one (`FUN_0072a715`).
- `Sound_StartStream` (0x00723d1d): the same id and loop only changes the volume. Otherwise the old stream pauses and
  closes (`FUN_007240f8`), and `AIL_open_stream` opens the new one. Levels are linear (`AIL_set_sample_volume_levels`,
  `+0x28 * vol`), with `AIL_set_stream_loop_count(0)` when looping. One stream at a time.
- **No crossfade was traced: a hard cut.** Stopping is immediate too (`FUN_00727bc5`, id 0x8000).
- **Not determined:** the `audio_music.csv` reader and its `fade-general` and `fade-combat` fields, and that the
  shipped music rows are of the stream type. `FUN_0072d1d1` is not music but the per-tick ambience manager, which
  fades through `FUN_00727cd6`.

**Voices.** **confirmed**: handles 1 to 30 are allocated (0x00722658), and the start loop uses index 1 to 29, so **29
sample voices**. None free returns -5, a drop. There are 31 sound slots, 4,000 sample slots and a table of 99 streams.
`FUN_0072a90f` applies the polygroups (25 groups): mode 0 refuses at the group's maximum, and mode 1 starts the new
sound and stops the group's oldest over the maximum (`FUN_00723604`), the client's own FIFO stealing.

**Options** (`ApplyVolumeOptions` 0x00495720). **confirmed**:

- Master (0xc93ad4) reaches `AIL_set_digital_master_volume_level`, linear.
- **The effects (0xc93ad8) and music (0xc93adc) volumes reach no audio call.** They are only loaded, reset, saved
  (`%0.2f`) and exposed to Lua. The category setters (`FUN_00723a5a`, `FUN_00723ac6`, `FUN_00723b32`, `FUN_00724131`)
  have no reference, so the categories stay 1.0 from the constructor `FUN_007223d9`.
- The enable flags do act: 0xc93ae0 master (sends 0), 0xc93ae1 effects, 0xc93ae2 music (stops the music events).
- `FUN_00492850` saves and mutes on 0x20021 and restores on 0x20022.

## What the KR parsers should change

1. **Parse with the client's rules, not a fixed column schema.** That means: the first-field markers, `!`
   sections, `;` comments, and a header row skipped as an invalid line.
2. **Parse ranges as the client does:** strip whitespace; `%`, `-`, `*` → space; empty = default;
   `low-high` or a single value.
3. **Keep unknown trailing columns** byte for byte when writing.
4. **Effects ids are server id + 1.** Show both values in the inspector.
