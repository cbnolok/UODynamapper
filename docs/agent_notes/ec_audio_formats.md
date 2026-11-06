# Client audio: formats, names and tables

What the shipped clients' audio holds, as `uocf-cli audio census` measured it (2026-10-31) on the reference
clients: the 2024 Enhanced Client, the current Classic Client, the old Kingdom Reborn client and UO Gold. Code:
`lib/uocf/src/audio/` (probe, decode, census), `lib/uocf/src/enhanced/sound/audio.rs` (`Audio.uop`),
`lib/uocf/src/classic/audio/sound.rs` (`sound.mul`, `soundLegacyMUL.uop`), `lib/uocf/src/classic/audio/audio_def.rs`
(`Sound.def`, `Music.def`), `lib/uocf/src/classic/audio/music.rs` (`Music/Digital/Config.txt`),
`lib/uocf/src/enhanced/sound/kr_audio.rs` (Kingdom Reborn's `audio_sounds.csv` and `audio_effects.csv`). Writers:
`lib/uocf/src/enhanced/sound/audio_edit.rs`, `lib/uocf/src/classic/audio/sound_edit.rs`, `lib/uocf/src/audio/validate.rs`,
`lib/uocf/src/audio/convert.rs` ([Writing](#writing)). Real-file tests:
`lib/uocf/tests/real_files/audio_client_files.rs`, `lib/uocf/tests/real_files/audio_writers.rs`. How the clients
read and play all this was traced in Ghidra (commit 7906ae18): `scripts/dev-helpers/ec-research/ghidra-audio/results/`
`uosa_audio.md` (Enhanced Client), `miles_audio.md` (its decoder, Miles 7.2c), `uokr_audio.md` (Kingdom Reborn),
`classic_audio.md` (Classic Client and UO Gold). How Miles sounds, for a playback path: `miles_playback_reference.md`.

Rerun:

```sh
C=/mnt/dati/_proj_local/_uo_clients
uocf-cli audio census --client "$C/_Ultima Online Enhanced" --client "$C/_Ultima Online Classic" \
  --client "$C/UO - Old KR" --client "$C/Ultima Online Gold" \
  --dictionary lib/uocf/assets/Dictionary.dic --output census.md --tsv census.tsv
```

Every MP3, WAV and Classic slot of the four clients decodes (symphonia for MP3, own code for PCM) to exactly the
sample count the probe derives from the headers, except one Kingdom Reborn MP3 whose last frame is cut (within one
frame). No decode failures.

## Enhanced Client `Audio.uop` (3,435 entries, 334 MB)

| content | entries | compression | notes |
|---|---|---|---|
| MP3 | 1,764 | none | all CBR, Layer III; 16/22.05/32/44.1 kHz |
| WAV | 1,666 | zlib 1,477, none 189 | all 16-bit PCM |
| text | 5 | zlib | the two tables, three file lists |

- MP3: ID3v2.3 178, v2.4 2, v2.2 1; ID3v1 278; Xing `Info` header 1,438 (none 326); LAME 3.97 1,381, LAME 3.93
  40, LAME 3.98r 15, Lavc58.54 2. 110 start with zero bytes before the first frame. No resyncs, no cut frames, no
  trailing bytes. 11,150 s in all.
- WAV: 22,048 Hz mono 1,237 (sic: not 22,050), 22,050 Hz mono 424, 44,100 Hz stereo 3, 22,050 Hz stereo 1,
  44,100 Hz mono 1. `fmt ` size 18 in 1,448, 16 in 218. Chunk layouts besides `fmt data`: `LIST DISP bext cart`,
  leading `bext`, `fact`, `CDif`, `_PMX`, `iXML`, `cue`, `LGWV`, `JUNK`. 110 files have odd-sized chunks without
  the RIFF pad byte (the reader accepts both). RIFF sizes always match. 3,174 s in all.
- The 110 unpadded files do not trip Miles' chunk walk (which steps `size + (size & 1) + 8` and so needs the pad
  byte): all 110 lay out `fmt  data LIST DISP bext cart`, and the odd, unpadded chunk is `bext`, after `fmt ` and
  `data`. The walk finds both before it reaches `bext`, so it never mis-steps (checked by simulating the walk on
  every WAV, 2026-11-01). It would not matter anyway: the Enhanced Client never plays these WAVs.
- The WAVs are the legacy sounds: for every one of the 1,655 slots of the Classic `soundLegacyMUL.uop`, the WAV's
  `data` chunk is byte-identical to the slot's PCM. 22,048 Hz is the rate the Classic client plays them at.
- **The Enhanced Client plays only the MP3s.** It plays a sound id from `data/audio/` + the `audio_sounds.csv`
  `FilePath`, and music the same way from `audio_music.csv`; UOSA.exe has no `legacysound` string, and no setting
  selects the WAVs (`uosa_audio.md` Q1). The `legacysounds/` WAVs are dead data for it.
- Entry names are lowercase, `/`-separated; see the tables below. With the current `Dictionary.dic`, 6 entries stay
  unnamed (below).

### Tables

- `data/audio/audio_sounds.csv`: header `#ID,FilePath`, 1,668 rows, e.g. `0,Sounds\aaaenvir\d_frst01.mp3`. The
  id is the server's sound id (packet 0x54's u16, 0xFFFF skipped), with no offset.
- `data/audio/audio_music.csv`: header `#ID,FilePath,Loop`, 107 rows, loop `0`/`1`.
- Entry name = `data/audio/` + the path with `\` turned into `/` and lowercased (`UOP::NormalizePath`), nothing
  trimmed and `//` not collapsed, built by `sprintf("%s/%s", "data/audio", FilePath)` in 255 bytes (so at most 254).
  The client's CSV reader splits at commas, drops a `"` and toggles quoting, trims nothing, and holds 0x7fe
  characters a line.
- Legacy WAV of sound id N = `data/audio/legacysounds/` + the path without its `sounds/` folder, `.wav` for
  `.mp3`. Sound ids are the Classic slot ids.
- Ids that have an entry: sounds 1,650 MP3, legacy WAV 1,656, music 107.
- Rows naming no entry (`AudioPackage::silent_rows`; warnings in `audio census` and `audio list`): 19 `wave02_l`
  (only its legacy WAV exists), 856 `flame04 .mp3` (a space in the path), 1469 `forcearro`, 1471 `lightningarro`,
  1628..1633 `pirates_*`, 1636..1640 (the `pirates_` prefix missing), 1658..1660 `drz1`..`drz3`. The client plays
  nothing for them: a row without an entry is silent (in mode 0 the failed load is cached for the session), and so
  is an id without a row. Fixing one is a table edit or an entry renamed to the row's path.

### Unnamed entries (6)

- text, file lists: `2C34B9194A0A0560` (weapon sounds), `668DFEC4FE8F74A1` (ends with `z_end`),
  `AB055D6281ECFEB1` (trap sounds). The same three hashes exist in Kingdom Reborn. MythAudio's table reader
  (`AudioConfig_Init`) has no caller in UOSA.exe, so nothing reads them.
- MP3: `814E39778A04107C`, `8E7736F30EEE5E3B`. Dictionary attacks (0.8 M names from the tables' parts, 4.7 M from
  merged dictionaries) found nothing; no row names them, so the client never plays them.
- WAV: `DBDCE27B2628F5B6`.
- Named from the Ghidra pass (in `Dictionary.dic` now): `E99FA5FB5E4712E2`
  `data/audio/sounds/highseas/pirates_cannon_shot_01.mp3`, `438745BCB223823C` `.../pirates_electric_crackle_01.mp3`,
  `A5D6DDC3E7F68E44` `.../pirates_cannon_fuse_01.mp3`, `8131CA6737014352` `.../pirates_cavern_roar_01.mp3`,
  `B7D9D1C334E536A7` `.../pirates_water_roar_01.mp3`. No row names them either (the rows say `pirates_*` without
  `_01`).

The retired EC Super Viewer `AudioCollection.xml` contained display labels, not
hash preimages. Of its 1,685 rows, 1,682 hashes already have names in the bundled
dictionary. The three remaining labels are retained here as unverified research
hints, not dictionary entries:

| Hash | Super Viewer display label |
|---|---|
| `8E7736F30EEE5E3B` | `sounds/highseas/guardian01.mp3` |
| `814E39778A04107C` | `sounds/treasure/m_mob_mine.mp3` |
| `DBDCE27B2628F5B6` | `legacy/(1664) guardian01.wav` |

The table paths and Classic slot names do not resolve these hashes. The original
XML remains in git history; no runtime or conversion code consumed it.

## Kingdom Reborn `Audio.uop` (1,553 entries)

MP3 1,544 (all zlib; CBR 1,531, VBR 13; 13 with the last frame cut), text 9 (zlib). 3 unnamed (the file lists).

Kingdom Reborn plays with its own Miles build, not the Enhanced Client's: its `mss32.dll` is version 7.0x (SHA-256
`272f65a0...`), the Enhanced Client's 7.2c (`b588b479...`), and the `miles/mssmp3.asi` files differ too
(`f6bbeccf...` against `0fb784a9...`). `miles_audio.md`'s "KR ships the same `miles` folder" is wrong: what it
found holds for the Enhanced Client only. The validator follows 7.2c. Its frame seek and its handling of a cut last
frame are the same in 7.0x (`miles_playback_reference.md`), and all 1,544 MP3s pass it without
`--allow-unverified-format`, the 13 cut ones with a note. The rest of the tag skip and frame walk was not compared
with 7.0x beyond that; the WAV walk was not read there, and Kingdom Reborn ships no WAV.

### Tables (`lib/uocf/src/enhanced/sound/kr_audio.rs`)

Read the way UOKR.exe reads them (`uokr_audio.md`), with the same CSV reader as the Enhanced Client, and written
back byte for byte (every field as read, quotes included, each line's own line end).

- `data/audio/audio_sounds.csv` (`MythAudio::ParseSoundsCsv`), 1,587 lines, CRLF. By the first field: `!` starts a
  section (`POLYGROUPS`, `ROLLOFF FACTOR`, `DEFAULT PARAMETER`, or another: `MASTER VOLUME`, `RATE PRESETS`,
  `SOUND DEFAULT PARAMETER VALUES`); `;` is a comment (17); `#` is a polygroup row, a rolloff row or a sound entry
  in those three sections and skipped in the others (so the `#,,s_default,...` row is not read); empty or a space
  is a sound entry (1,561); anything else is skipped (the `w,s,id,filename,...` header). A sound entry is 14 fields
  after the marker: type id, name (at most 0x50), file name (at most 0x50), rate, volume, pan, offset (ranges:
  white space removed, `%`, `-`, `*` turned into spaces, empty = the default, one value or `low high`, so `-5`
  reads as 5 and is refused here), min and max range (`*` or empty = default), repeat (ignored), mono, polygroup,
  priority, streamed; then trailing status, usage and notes. Shipped: rate `1.0-1.1` (1,489) or `1.0-1.0` (72),
  every other range `*`, mono `1` (1,488) or `*`. A second structure of a name is skipped by the client; 11 shipped
  (`citynightedit`, `walking`, `turfin`, the six `*.mp3` names, ...). At most 4,000 names, 30 structures per file.
- `data/audio/audio_effects.csv` (`UOAudio::LoadEffectsTable`): `;,ID#,description,sound,notes`, then 1,567
  `,ID#,name[,,notes]` rows. `ID#` = the server's sound id + 1 ("Mythic code assumes ID 0 to be invalid"); the
  Enhanced Client has no such offset. 6 ids are defined twice (2061..2066; which row the client keeps was not
  traced, the reader takes the first) and 11 ids name no sound structure (`l_turfin`, music names without
  `.mp3`).
- A server sound id S plays: the effects row `ID#` S + 1, its structure, that structure's file, `data/audio/` +
  the normalized file name (`KrAudioTables::sound_for_server_id`, `AudioPackage::entry_for`). 1,541 of the 1,567
  rows reach an entry.
- `audio_music.csv` (`,ID#,name,mp3,loop,...`, ids from 10000) and the group, queue and loop tables are kept
  unread.

## Classic sounds

- Slot = 40-byte header (32-byte NUL-padded name, then 8 bytes) + mono 16-bit PCM, which the client plays at
  22,048 Hz (its DirectSound buffers are created at 0x5620 Hz; `classic_audio.md`). The PCM was made at 22,050 Hz.
- Header bytes 32..40: current client `30|40|50|00|76 00 00 00 98 fb 18 00`; UO Gold `cc cc cc cc cc cc cc cc`
  (684), `49 10 40 00 40 94 40 00` (589), `91 10 40 00 28 7a 41 00` (65). They look like uninitialized memory
  (stack/heap fill, pointers) of the packing tool; kept as read. No client reads bytes 0..40 (`Voice_SetData`
  copies from byte 40 on).
- A slot of 40 bytes or fewer counts as missing. `PlaySound` plays no id above 1667 (0x683).
- Current client: `soundLegacyMUL.uop`, 1,655 slots, uncompressed, entry `build/soundlegacymul/NNNNNNNN.dat`
  (8 digits), 3,815 s. No `sound.mul`. UO Gold: `sound.mul`/`soundidx.mul`, 4,096 index entries, 1,338 slots.
- `SoundMap` prefers the MUL pair when both exist; `read_id` takes the id's own slot, and `Sound.def` only when the
  slot is missing.

## `Sound.def` and `Music.def`

Grammar and the client's reading in `lib/uocf/src/classic/audio/audio_def.rs` (`classic_audio.md` Q4, the same in UO
Gold and 7.0.103). One `id {targets} extra` per line; 685 ids in UO Gold, 787 in the current client; every group
has one target and every `extra` is 0.

- The client reads lines with `fgets` of 255 bytes, skips blank and `#` lines and leading white space, reads ids
  and targets with `%hd`, and skips a `-` (so `{-1}` reads as target 1). After a target only white space and then
  `,` or `}` may follow, else the line is dropped; the third field is required; the first line of an id wins.
- A line applies only when its id has no slot (or one of 40 bytes or fewer). With several targets the client picks
  one at random (`rand() % count`) on every play.
- Both releases ship line 152 `805 {331 0`: the client drops it (`0` follows `331` without a comma), flagged
  `client_ignored`. Ids 778..1440 (UO Gold 778..1338, 351 / 249 ids) write `{-1}`; the parser keeps `-1` and
  notes that the client reads it as 1. Only one line ever applies: `856 {550} 0`, whose slot is missing; every other
  id has a slot of its own (`SoundMap::translation_notes`, in the census).
- `Music.def` is the single line `0 {0} 0`, and no client reads it: neither UO Gold nor 7.0.103 has the string.

## `Music/Digital/Config.txt` and the music files

Grammar and the client's reading in `lib/uocf/src/classic/audio/music.rs` (`classic_audio.md` Q5): `fgets` of 0x200
bytes, trailing characters below `!` trimmed, blank and `#` lines skipped, `strtok(" ")` (spaces only; the last
token wins), the value split at the first `,` into name and flag, `.mp3` appended when the last 4 characters hold
no `.`, any extension but `mp3` plays nothing, and it loops when the flag is exactly `loop` or `lop`
(case-sensitive). Current client 103 ids (46 names with `.mp3`, 41 looping), UO Gold 52 (no extensions, 35
looping: line 10 `9 britainpos,lop` loops too). Every file named exists. Music folders: current Classic 91 MP3 + 1
Bink video; UO Gold 40 MP3 + 3 Bink videos. All MP3s CBR.

## MIDI music

No client folder we hold ships a `.mid`, `.midi`, `.rmi` or `.xmi` file, and neither does the OSI patch archive
listing, so the old music is known only from client strings: UO Gold's `client.exe` names `moonglow.mid`, `cove.mid`
and others with the path format `music\\%s%s`, and uses `mciSendCommandA` and `midiOutSetVolume`; the current
`client.exe` still has 49 such names. `lib/uocf/src/audio/midi.rs` reports what a standard MIDI file declares, tested
on synthetic data only. The workspace has no MIDI synthesizer, so the inspector's Audio tab shows that report and
exports the file without playing it.

## Names added to `Dictionary.dic`

2,008 names (commit a99b0b73): 1,668 legacy WAVs, 278 sounds, 57 music (Enhanced and Kingdom Reborn), 5
`build/soundlegacymul/` slots (1662..1666). Everything else in the tables was already named. Then the 5 HighSeas
MP3s the Ghidra pass recovered (above).

## Writing

Round trips (2026-11-01, `audio_writers` real-file tests): an unchanged copy is byte-identical to the shipped file
for the Enhanced `Audio.uop` (3,435 entries), the Kingdom Reborn `Audio.uop` (1,553), the current Classic
`soundLegacyMUL.uop`, and `sound.mul` + `soundidx.mul` of UO Gold, Mondain's Legacy and 7.0.20. `Sound.def` and
`Music.def` of all five Classic clients write back byte for byte (the dropped `805 {331 0` and the current
client's trailing empty line included). The two Enhanced tables write back byte for byte from their rows, and
Kingdom Reborn's `audio_sounds.csv` and `audio_effects.csv` from their fields. After a replace or add, every other
entry or slot keeps its bytes.

### `Audio.uop` entries (`AudioPackageEditor`, `uocf-cli audio replace|add`, inspector "Replace...")

- Checked against what the Enhanced Client's decoder plays (`audio::validate::enhanced_format_report`). The client
  checks nothing itself: `SampleSound_Start` hands the whole entry and its name to Miles 7.2c's
  `AIL_set_named_sample_file`. Every shipped MP3 and WAV passes (3,430 entries, `FormatPolicy::VerifiedOnly`).
  Refused under every policy, with the evidence in `miles_audio.md` and, for the tag skip and the walks,
  `miles_playback_reference.md` ("What the tag skip and the walks accept"):

  | Rule | Evidence |
  |---|---|
  | The content matches the name's extension (`kind_mismatch`) | `SetNamedSampleFile_Impl` picks the decoder by extension; a leading valid ID3v2 (`ID3`, version bytes < 0xFF, syncsafe size) forces MP3 |
  | WAV: `RIFF` with `WAVE` at +8 | `WavInfo_Impl` ("Not a WAV file.") |
  | WAV: Miles' walk (`validate::miles_wav_walk`) takes the `fmt ` and `data` chunks ours finds: from +12, step `size + (size & 1) + 8` in 32 bits, each step landing below the RIFF size (an odd chunk before them needs its pad byte; `data` must start below the RIFF size) | `WavInfo_Impl`; ids compared case-insensitively (`StrnicmpN`); `fmt ` searched first, `data` again from the first chunk; it does not know the file's length, so a walk that leaves the file or a size that steps back is refused |
  | WAV, IMA ADPCM: the search for `fact` ends inside the file; block align not 0 | it runs over the whole RIFF size; without `fact` Miles divides by the block align |
  | WAV: `fmt ` of 16 bytes or more | Miles reads the first 16 (16 and 18 both ship) |
  | WAV: PCM of 8 or 16 bits, IMA ADPCM (tag 0x11, 4 bits), or EXTENSIBLE with `cbSize` ≥ 22, `KSDATAFORMAT_SUBTYPE_PCM` and block align = channels × 2 | `SetSampleFile_Impl`, `InitSampleFromWavInfo` ("Unsupported wave file format.", "Not a WAVEFORMATPCMEX file.", "Not KSDATAFORMAT_SUBTYPE_PCM.") |
  | WAV: mono or stereo, rate not 0 | more channels take the multichannel path, which was not read; any rate plays (`AIL_set_sample_playback_rate`, the mixer resamples) |
  | WAV: a `data` chunk with samples, inside the file | "Invalid or missing data block."; Miles clamps the size neither to the file nor to the RIFF size |
  | MP3: Layer III | `MilesMp3::SeekFrameHeader` rejects layers I, II and reserved, bitrate index 15 and rate index 3 |
  | MP3: the first header Miles' byte scan locks onto is the stream's first frame (`validate::miles_mp3_seek` simulates it) | `SeekFrameHeader` reads a byte while it is at most 0x2000 bytes past the window start, so a header must start within 0x1FFD bytes; the window starts where Miles' skip of a leading ID3v2 tag ends (`Id3v2Tag::miles_leading_skip`: flags not read, footer not skipped, 128 bytes short when the sum carries and the next size byte is odd) and restarts after each inline `TAG` (128 bytes) or valid `ID3` tag (with its footer) it skips; it takes the first sync (`0xFF`, three set bits) that is Layer III with bitrate index ≠ 15 and rate index ≠ 3, and checks no next frame, so a false sync in junk before the stream would be decoded (the 110 shipped files with 720 or 1,044 leading zeros pass) |
  | MP3: Miles' walk over the whole file (`validate::miles_mp3_frames`) decodes the frames of the stream and no others | a sync in junk, or in what is left of a tag, is decoded as a frame; more than 8 KiB without a frame ends the stream, and a header of another rate makes it pass over the next frame, so the frames after are lost |
  | MP3: every frame of the first frame's version, layer and rate | each frame is compared with the last header seen: one that differs is lost and replaces the values, and the playback rate stays the first's; the bitrate and the channel mode are not compared, so VBR plays |

  Accepted: VBR, any bitrate, ID3v1 and ID3v2 tags (inline ones are skipped while seeking), the Xing/Info frame
  (decoded as a silent frame), CRC-protected frames (the CRC is skipped). Notes, always accepted: an MP3 stream
  unlike the shipped ones (MPEG-1 32 kHz mono, 44.1 kHz mono/stereo/joint; MPEG-2 16 kHz mono, 22.05 kHz
  mono/stereo/joint), frames that change the channel mode (no shipped stream does), a break in the frame chain that
  Miles scans over, a cut last frame (Kingdom Reborn's 13 files), trailing bytes that are not a tag, an APE tag, a PCM
  WAV whose byte rate or block align disagree with its channels and bits (Miles reads neither), a legacy WAV unlike
  22,048 Hz mono 16-bit, a RIFF size past the end of the file (it only bounds the walk), bytes after the end it gives,
  a `data` chunk running past it (Miles plays them), bytes after the last chunk, and every `legacysounds/` name (the
  client never plays it).
- Refused unless `--allow-unverified-format` (`FormatPolicy::AllowUnverified`) turns them into warnings, for trying
  a file out in a test client:
  - MPEG-2.5. Miles plays it at the right rate, with the band tables of MPEG-2 (`miles_audio.md`); no shipped file is
    MPEG-2.5. This is known to be wrong rather than unknown, and no MPEG rate is left over: the header table holds
    only the nine rates the decoder takes.
  - A PCM WAV whose data is not whole sample frames: the mixer reads up to `frame - 1` bytes past the data [7.2c, a
    static reading], so one source frame of whatever follows, a possible click at the very end
    (`miles_playback_reference.md`).
- Refused here although Miles would take them, deliberately: each is a file no tool should write, and none is
  shipped. They stay refused under every policy.
  - WAV chunk ids in another case (`FMT `, `DATA`): Miles compares them ignoring case, our walk does not.
  - A `fmt ` tag of 0x55 (MP3 in a WAV): `mssmp3.asi` declares that wave tag, so Miles would take it. MP3 data belongs
    in an `.mp3` entry.
  - `WAVE_FORMAT_EXTENSIBLE` of another sample size than 16 bits (Miles checks only block align = channels × 2).
  - MP3: a free-format bitrate (index 0) and the reserved version bits `01`: `SeekFrameHeader` takes both, but no
    frame length follows from them.
  - `Sound.def` and `Music.def`: whitespace before `{` and before a trailing `#` is required here; the client also
    takes `5{1} 0` and `5 {1} 0#x`. Lines of more than 253 characters are refused, though the client reads a
    line of 254 whole too (`fgets` of 255 bytes leaves only its line feed, read next as an empty line).
  - `Config.txt`: the 510-byte line limit counts the CR of a CRLF line, which the client's text-mode read drops
    before its `fgets` of 0x200 bytes.
- `--convert`: a WAV entry takes any WAV `audio::decode` reads (8/16/24/32-bit PCM, 32/64-bit float, extensible)
  or an MP3, rewritten as a canonical 16-bit PCM WAV of the same rate and channels. No resampling (a pure-Rust
  option would be `rubato`), no MP3 encoder: encode with `lame --cbr -b 128` or `ffmpeg -c:a libmp3lame -b:a 128k`
  (LAME bindings are C).
- A replaced entry keeps its compression flag. An added one is stored like the shipped ones of its kind: MP3
  uncompressed, WAV and tables zlib. Edited entries get a fresh timestamp data block header; the others keep their
  bytes and place. The copy goes to an absolute path other than the source: an output that is the source, or a
  symbolic or hard link to it, is refused by file identity (`utils::output_file`, the `same-file` crate: device
  and inode, or the Windows volume and file index). It is written to a temporary file beside the output and
  renamed over it, so a failed save leaves the output as it was. The same holds for every UOP writer built on
  `uop_entries::save_package_as` and for the Classic sound files.
- Names: by sound id (the row's MP3), legacy sound id (the row's `legacysounds/` WAV), music id, or entry name
  (`data/audio/...`, lowercase with `/`: names are hashed as written, as the client normalizes them).
- A playable sound is an MP3 entry and a table row naming it. A new id gets its row: `Sounds\folder\x.mp3` for a
  sound (which names both its MP3 and its legacy WAV), `Music/x.mp3` for music (`--loop`), checked like the shipped
  paths (`.mp3`, printable ASCII, no comma or `"`, an entry name of at most 254 bytes; sound ids up to 0xFFFE). The
  table is rewritten in id order with CRLF lines. An add stores the entry and its row together or neither: the
  table is written first and put back when the entry cannot be added. A legacy sound id gets no new row unless
  the package already holds the MP3 the row names (the client plays only that MP3): add the MP3 by sound id with
  the path first, then the WAV, which takes that row. An MP3 added by name must be one a row already names (a
  shipped silent row); anything else is refused, since the client would never play it. Replacing or adding a
  `legacysounds/` WAV stays allowed, with the note that the Enhanced Client does not play it.
- Kingdom Reborn: a sound id is the server's id, followed through `audio_effects.csv` (`ID#` = id + 1) to its
  structure's file (a structure starting with a digit is a structure handle, `0x10000 | n` for the n-th sound
  structure UOKR accepted; see `uokr_audio.md`). A server id without an effects row, added with a table path
  (`Sounds\folder\file.mp3`), gets a sound structure at the end of `audio_sounds.csv`, named as the shipped ones
  (`folder\file`) and with the parameters of 1,488 of the 1,561 shipped structures (`1.0-1.1` rate, mono, the
  rest `*`), and the row `,<id + 1>,folder\file` at the end of `audio_effects.csv`; both tables keep every shipped
  byte and get one line, ended like their first (CRLF), and are read again strictly, so the client's limits hold
  (names and file names up to 0x50 characters, 4000 names, 30 structures per file, the 14 fields, 0x7fe-character
  lines). Refused: a structure name taken, `s_default`, or starting with a digit (the effects table would read it as
  a handle). Real-file test: `audio_client_files::kingdom_reborn_audio_add_plays_a_new_server_id` (server id 2072
  plays the new MP3; every shipped server id plays what it played). Entries are also replaced or added by name.

### Classic sounds (`ClassicSoundEditor`, `uocf-cli sound import`, inspector "Replace slot...")

- `sound.mul` layout in all three MUL clients: the slots back to back in index order from offset 0, the data file
  ending with the last; index entry `i` is `(offset, length, i + 1)`, empty `(-1, -1, i + 1)`. The writer keeps
  each entry's third field and the empty entries as read and lays the slots out the same way; a slot past the end
  extends the index with empty entries.
- `soundLegacyMUL.uop`: a replaced entry keeps its flag; an added one is uncompressed, as all shipped ones.
- Header: a replaced slot keeps its 40 bytes (a new `--name` overwrites the start of the name field and a NUL; the
  bytes after it, leftovers in the shipped files, stay). A new slot gets the name and zeros, which is safe: no
  client reads bytes 0..40.
- Slot ids go up to 1667, the highest the client plays.
- Input: 16-bit PCM mono WAV at 22,048 Hz, the rate the client plays, copied as is; 22,050 Hz (the nominal rate,
  and the label of 424 Enhanced legacy WAVs) is still taken, with a note that it plays 0.01% slower. Other rates
  are refused: they would play at the wrong speed. `--convert` decodes any WAV or MP3 at those rates and downmixes
  to mono (`ffmpeg -ar 22048 -ac 1 -c:a pcm_s16le`).
- Verdata is not applied or written. The `sound.mul` pair is edited when present, as `SoundMap` reads it.

### `Sound.def` and `Music.def` (`DefTranslationTable::to_text`, `set`, `remove`)

Every line read is written back as it was (blank and `#` lines, spacing, dropped lines), with the line end of the
first line and the final line end or none, so any file the parser takes round-trips byte for byte. A line set
after parsing is written `id {a, b} extra`; ids and targets go up to 32,767 (`%hd`).

## Questions for a Ghidra pass

Answered by the pass of commit 7906ae18 (above, and in `scripts/dev-helpers/ec-research/ghidra-audio/results/`):
the Enhanced Client plays the table's MP3 and never the legacy WAV; rows without an entry are silent; the decoder
is Miles 7.2c with the rules above; how `Sound.def`, `Music.def` and `Config.txt` are read; bytes 32..40 are
unread; Kingdom Reborn's table grammar and its id offset. Still open:

1. The names of the three file lists, the two unnamed MP3s and the unnamed WAV (nothing reads them).
2. Answered in `miles_playback_reference.md`: both 7.2c and 7.0x play a cut last frame, partly from stale buffer
   bytes, so a short glitch is possible. Nothing has listened to it yet.
3. Which of two `audio_effects.csv` rows of an id Kingdom Reborn keeps, and the reader of its `audio_music.csv`.
