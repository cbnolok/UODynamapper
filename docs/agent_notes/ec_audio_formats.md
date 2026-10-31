# Client audio: formats, names and tables

What the shipped clients' audio holds, as `uocf-cli audio census` measured it (2026-10-31) on the reference
clients: the 2024 Enhanced Client, the current Classic Client, the old Kingdom Reborn client and UO Gold. Code:
`lib/uocf/src/audio/` (probe, decode, census), `lib/uocf/src/enhanced/audio.rs` (`Audio.uop`),
`lib/uocf/src/classic/sound.rs` (`sound.mul`, `soundLegacyMUL.uop`), `lib/uocf/src/classic/audio_def.rs`
(`Sound.def`, `Music.def`), `lib/uocf/src/classic/music.rs` (`Music/Digital/Config.txt`). Real-file tests:
`lib/uocf/tests/real_files/audio_client_files.rs`.

Rerun:

```sh
C=/mnt/dati/_proj_local/_uo_clients
uocf-cli audio census --client "$C/_Ultima Online Enhanced" --client "$C/_Ultima Online Classic" \
  --client "$C/UO - Old KR" --client "$C/Ultima Online Gold" \
  --dictionary tools/_shared_assets/Dictionary.dic --output census.md --tsv census.tsv
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
- The WAVs are the legacy sounds: for every one of the 1,655 slots of the Classic `soundLegacyMUL.uop`, the WAV's
  `data` chunk is byte-identical to the slot's PCM. The 22,048 Hz label is a header difference only.
- Entry names are lowercase, `/`-separated; see the tables below. With the current `Dictionary.dic`, 11 entries stay
  unnamed (below).

### Tables

- `data/audio/audio_sounds.csv`: header `#ID,FilePath`, 1,668 rows, e.g. `0,Sounds\aaaenvir\d_frst01.mp3`.
- `data/audio/audio_music.csv`: header `#ID,FilePath,Loop`, 107 rows, loop `0`/`1`.
- Entry name = `data/audio/` + lowercase(path) with `\` turned into `/`.
- Legacy WAV of sound id N = `data/audio/legacysounds/` + the path without its `sounds/` folder, `.wav` for
  `.mp3`. Sound ids are the Classic slot ids.
- Ids that have an entry: sounds 1,650 MP3, legacy WAV 1,656, music 107.
- Rows naming no entry: 19 `wave02_l`, 856 `flame04 .mp3` (a space in the path), 1469 `forcearro`,
  1471 `lightningarro`, 1628..1633 `pirates_*`, 1636..1640, 1658..1660 `drz1`..`drz3`. Which of the MP3 and the
  legacy WAV the client plays for a row is not known (Ghidra question 1).

### Unnamed entries (11)

- text, file lists: `2C34B9194A0A0560` (weapon sounds), `668DFEC4FE8F74A1` (ends with `z_end`),
  `AB055D6281ECFEB1` (trap sounds). The same three hashes exist in Kingdom Reborn.
- MP3: `438745BCB223823C`, `8131CA6737014352`, `814E39778A04107C`, `8E7736F30EEE5E3B`, `A5D6DDC3E7F68E44`,
  `B7D9D1C334E536A7`, `E99FA5FB5E4712E2`.
- WAV: `DBDCE27B2628F5B6`.

The labels of `tools/_shared_assets/manual_mappings_from_ec_super_viewer_todo/AudioCollection.xml` are display
labels, not hash preimages; the table paths and Classic slot names do not cover these entries.

## Kingdom Reborn `Audio.uop` (1,553 entries)

MP3 1,544 (all zlib; CBR 1,531, VBR 13; 13 with the last frame cut), text 9 (zlib). Its tables share the Enhanced
names but have another layout (`w,s,id,filename,rate,...` rows, `;` comments, music ids from 10000); the reader
detects and skips them (`AudioTables::KingdomReborn`), entries are reached by name. 3 unnamed (the file lists).

## Classic sounds

- Slot = 40-byte header (32-byte NUL-padded name, then 8 bytes) + 22,050 Hz mono 16-bit PCM.
- Header bytes 32..40: current client `30|40|50|00|76 00 00 00 98 fb 18 00`; UO Gold `cc cc cc cc cc cc cc cc`
  (684), `49 10 40 00 40 94 40 00` (589), `91 10 40 00 28 7a 41 00` (65). They look like uninitialized memory
  (stack/heap fill, pointers) of the packing tool; kept as read.
- Current client: `soundLegacyMUL.uop`, 1,655 slots, uncompressed, entry `build/soundlegacymul/NNNNNNNN.dat`
  (8 digits), 3,815 s. No `sound.mul`. UO Gold: `sound.mul`/`soundidx.mul`, 4,096 index entries, 1,338 slots.
- `SoundMap` prefers the MUL pair when both exist; `read_id` resolves a sound id through `Sound.def` first.

## `Sound.def` and `Music.def`

Grammar in `lib/uocf/src/classic/audio_def.rs`. One `id {targets} extra` per line; 685 ids in UO Gold, 787 in the
current client; every closed list has one target and every `extra` is 0; ids from 778 map to `{-1}`. Both
releases ship line 152 `805 {331 0` without `}`: read as an unclosed group (targets 331, 0; the first is
played), flagged `unclosed`. `Music.def` is the single line `0 {0} 0`.

## `Music/Digital/Config.txt` and the music files

Grammar in `lib/uocf/src/classic/music.rs`: `id name[,flag]`. Current client 103 ids (46 names with `.mp3`,
41 looping), UO Gold 52 (no extensions, 34 looping, line 10 `9 britainpos,lop` misspells the flag). Every file
named exists. Music folders: current Classic 91 MP3 + 1 Bink video; UO Gold 40 MP3 + 3 Bink videos. All MP3s CBR.

## Names added to `Dictionary.dic`

2,008 names (commit a99b0b73): 1,668 legacy WAVs, 278 sounds, 57 music (Enhanced and Kingdom Reborn), 5
`build/soundlegacymul/` slots (1662..1666). Everything else in the tables was already named.

## Questions for a Ghidra pass

1. For a sound id, does the Enhanced Client play the `sounds/` MP3 or the `legacysounds/` WAV, and on what
   setting? How does it handle table rows without an entry (`flame04 .mp3`, `pirates_*`)?
2. Which names do the three unnamed file lists have, and what reads them? Who plays the 7 unnamed MP3s and the
   unnamed WAV?
3. Which decoder plays the audio (Miles `mssmp3.asi`?), which WAV `fmt` tags, sizes and extra chunks it accepts,
   whether unpadded odd chunks matter, and whether 22,048 Hz WAVs are resampled or played as 22,050.
4. How the Classic client parses `Sound.def`/`Music.def`: the unclosed `805 {331 0`, several targets (random
   pick?), the third field, `-1`.
5. How `Config.txt` flags are read (is `lop` a loop?), and whether an extension is added.
6. Whether anything reads bytes 32..40 of a Classic sound header.
7. Kingdom Reborn table grammar (`w,s,id,filename,...`) and what `rate` and the other columns do.
