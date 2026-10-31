# Classic client.exe: Sound.def, the sound header, Config.txt

Programs `classic_7.0.103.0_client.exe` (SHA-256
`4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`, the canonical Classic client) and
`gold_client.exe` (`aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2`) for the Config.txt
check. **confirmed** = read in the code; **probable** = inferred from the code and the shipped data.

The Classic client plays sounds with DirectSound directly; Miles is not involved.

## Functions (7.0.103.0)

| Address | Name | Notes |
|---|---|---|
| 0x00447bc0 | `UOClassicDefs::LoadDefFiles` | sound, gump, anim1-4, art, texterr, body, corpse `.def` |
| 0x00447220 | `UOClassicDefs::DefTable_Load` | generic def file loader |
| 0x00446d90 | `UOClassicDefs::DefTable_ParseLine` | |
| 0x00448620 | `UOClassicDefs::SoundDefTable_Insert` | sound def table vtable +8 (vtable 0x00697114) |
| 0x00445dd0 | `UOClassicDefs::DefTable_Contains` | vtable +4 |
| 0x00446fa0 | `UOClassicDefs::DefTable_RemapRandom` | |
| 0x005fa840 | `UOClassicAudio::PlaySound` | |
| 0x005fa510 | `UOClassicAudio::Voice_Start` | |
| 0x005f9c70 | `UOClassicAudio::Voice_SetData` | |
| 0x005f9fb0 | `UOClassicAudio::Voice_CreateBuffer` | |
| 0x005aee50 | `UOClassicAudio::DigitalMusic_LoadConfig` | |
| 0x005ad3f0 | `UOClassicAudio::DigitalMusic_Play` | |

UO Gold: `UOClassicAudio::DigitalMusic_LoadConfig` 0x004f7940, `UOClassicAudio::DigitalMusic_Play`
0x004f8860.

## Q4. Sound.def and Music.def

### Music.def is not read

**confirmed** by string absence: no `music.def` string exists in Classic 7.0.103, UO Gold, UOKR.exe or
UOSA.exe. `LoadDefFiles` loads only the def files listed above. Classic music is selected by
`MUSIC\DIGITAL\CONFIG.TXT` (below).

### Loading

**confirmed** (`DefTable_Load`). The file is opened with `fopen "r"` (text mode) and read with `fgets` of
255 bytes, so a longer line is cut. **probable**: the rest of a cut line is parsed as a new line. A parsed
line is inserted only when its id is not already in the table: **the first line for an id wins**.

### Line grammar

**confirmed** (`DefTable_ParseLine`). A line that breaks any rule is dropped as a whole.

1. Leading whitespace is skipped. An empty line or a line starting with `#` is skipped.
2. The id is read with `sscanf "%hd"`.
3. The parser scans forward to `{`. The scan is not bounded by the line end, so a line without `{` reads
   stale buffer bytes. Never write one.
4. Each target is read in three steps:
   1. Non-digit characters are skipped. A `-` is skipped too, so `{-1}` reads as target **1**.
   2. `sscanf "%hd"` reads the number, and its digits are skipped.
   3. The parser skips to `,` or `}`. Whitespace may come first, but any other character (a digit
      included) drops the line.

   `{a, b, c}` therefore gives a group of several targets.
5. After `}` the parser skips to a digit or `-` (again unbounded), and `sscanf "%d"` reads the **third
   field**. The third field is **required**.
6. After the third field only whitespace or a `#` comment may follow.

### Use

**confirmed**:

- **The def applies only to missing sounds.** `SoundDefTable_Insert` adds an entry only when the sound
  slot is missing in the index: offset −1, or length < 0x29 (no data after the 40-byte header). A def
  line for a sound that exists is ignored.
- **Several targets.** `PlaySound` asks `DefTable_Contains`, then `DefTable_RemapRandom`. The target is
  picked with `rand() % count` on every play.
- **The third field** is parsed but not used by `PlaySound`. Its other readers were not searched.

### The shipped cases

| Line | Result | Why |
|---|---|---|
| `805 {331 0` (line 152) | **dropped** | After `331` the parser finds `0` before `,` or `}`. 805 has data, so the line would not apply anyway. **confirmed** (code and data). |
| the 351 `{-1}` lines (ids 778..1440) | parsed as target 1, never applied | Every one of these ids has data. **confirmed** (code and data). |
| `856 {550} 0` | applies | Of the 436 lines with positive targets, it is the only one whose id is missing. 856 plays 550. |

Ids from 0 to 1667 without data: 856, 1469, 1471, 1628–1633, 1658–1660 and 1667. `soundLegacyMUL.uop`
holds 1655 entries; its highest id is 1666.

### What the Sound.def parser should change

- **Accept and keep what the client accepts.** Keep the rejected lines as raw text with a warning
  ("ignored by the client: ..."), and never silently fix them. That covers `805 {331 0`.
- **Model the client's reading of each line:**
  - `-` before a number is ignored (`{-1}` targets 1). Report it as "plays 1 when 778 is missing", or
    keep `-1` as written but document that the client reads it as 1.
  - The third field is required.
  - For duplicate ids, the first line wins.
  - Lines are at most 254 bytes.
- **The writer** should emit `id {t1, t2, ...} third`.
- **Mark the lines that never apply.** A def entry matters only when the target id's own slot is empty.
- **Music.def needs no client-accurate parser**, because no client reads it. Keep a lenient round-trip
  only.

## The sound slot and its header (Q6)

**confirmed**:

- **Ids.** `PlaySound` rejects ids above 0x683 (1667). There are 8 voices.
- **Slot data.** It is fetched with the shared index reader (file index 5 = sound). The slot must be
  longer than 0x28 bytes.
- **The header is skipped.** `Voice_SetData` copies from data + 0x28 with length − 0x28. **Nothing reads
  bytes 0..40 of the header, so bytes 32..40 are unused**, and so are the name bytes 0..32.
  - The other callers of the index reader pass index 6, index 7, or the facet map file ids from
    `GetFacetMapFileId` (0x1b..0x43). None of them passes 5.
  - `SoundDefTable_Insert` looks only at the slot's offset and length.
- **Playback rate.** `Voice_CreateBuffer` creates every DirectSound secondary buffer as PCM mono 16-bit
  **22,048 Hz** (0x5620; 0xAC40 bytes/s, 0x10000-byte buffer). `Voice_Start` sets the frequency to 0
  (`DSBFREQUENCY_ORIGINAL`), or to 22,048 ± a random variation when random pitch is on.

  So the Classic client plays every sound as 16-bit mono at 22,048 Hz. Data at another rate plays at the
  wrong speed, and 8-bit or stereo data plays wrongly.

### What the Classic sound writer should change

- **Sample format.** Write 16-bit mono PCM at 22,048 Hz. Resample or refuse any other WAV.
- **Header.** Write a 40-byte header. Bytes 32..40 can be zero; nothing reads them.
- **Ids.** Accept ids up to 1667.
- **Empty slots.** A slot of length ≤ 40 counts as missing, and the Sound.def remap then applies.

## Q5. Config.txt and `lop`

**confirmed** in 7.0.103 and UO Gold.

### `DigitalMusic_LoadConfig`

1. It reads `MUSIC\DIGITAL\CONFIG.TXT` with `fgets` of 0x200 bytes.
2. Trailing characters below `!` are trimmed. Empty lines and lines starting with `#` are skipped.
3. `strtok(" ")` splits the line. The first token is the id (`atol`). Every further token is stored for that
   id, so **the last token wins**.

### `DigitalMusic_Play`

1. The value is split at the first `,` into a name and a flag.
2. The path is the client folder + `MUSIC\DIGITAL\` + the name.
3. `.mp3` is appended when the last 4 characters hold no `.`.
4. The extension must then be `mp3` (`_stricmp`), otherwise nothing plays.
5. The music **loops when the flag is exactly `loop` or `lop`**: a `std::string` compare, case-sensitive.
   The strings are `"lop"`/`"loop"`: 7.0.103 at 0x006b7c08, Gold at 0x005ce108/0x005ce10c.

### What the Config.txt parser should change

- **Accept `lop` as loop.** Keep it as written when round-tripping.
- **Case-sensitive flags.** `Loop` and `LOOP` do not loop.
- **One name per line.** Split on spaces, keep the last token, and split it at the first comma.
- **Names.** A name without an extension gets `.mp3`, and only `.mp3` plays.
