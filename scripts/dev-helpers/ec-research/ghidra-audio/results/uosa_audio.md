# UOSA.exe: how the Enhanced Client plays sounds

Program `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
**confirmed** = read in the code; **probable** = inferred from the code and the shipped data.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x004b5600 | `UOAudio::InitAudio` | registers Lua `PlaySound`, starts the sound system, loads both tables |
| 0x004b5180 | `UOAudio::LoadSoundTable` | `data/audio/audio_sounds.csv` |
| 0x004b53a0 | `UOAudio::LoadMusicTable` | `data/audio/audio_music.csv` |
| 0x004b61c0 | `UOAudio::SoundTable_GetOrInsert` | `std::map` `operator[]` |
| 0x004b6290 | `UOAudio::MusicTable_GetOrInsert` | `std::map` `operator[]` |
| 0x004b5770 | `UOAudio::PlaySoundById` | id in AX, mode argument |
| 0x004b5a30 | `UOAudio::PlayMusicById` | |
| 0x004b4f20 | `UOAudio::HandlePlaySoundPacket` | server sound packet |
| 0x008840e1 | `MythAudio::SoundSystem_Startup` | Miles start-up, file callbacks |
| 0x00885b67 | `MythAudio::MilesFileOpenCallback` | UOP lookup |
| 0x00884ab1 | `MythAudio::SoundSystem_LoadSample` | |
| 0x00884aeb | `MythAudio::SoundSystem_RequestSampleFile` | asynchronous load job |
| 0x00883c98 | `MythAudio::SampleLoadRequest_ctor` | probable |
| 0x00884ff9 | `MythAudio::SoundSystem_PlaySample` | |
| 0x00898194 | `MythAudio::SampleSound_Start` | `AIL_set_named_sample_file` |
| 0x00898392 | `MythAudio::SampleSound_StartAtPosition` | probable |
| 0x009893e3 | `UOCsv::ReadLine` | |
| 0x00989551 | `UOCsv::NextLine` | |
| 0x009a28c7 | `UOCsv::ReadStringField` | |
| 0x009a2949 | `UOCsv::ReadIntField` | |
| 0x009899ef | `UOP::NormalizePath` | used by every UOP name lookup |

Already named before this folder: `UOLua::PlaySound` (0x004b5fc0), `UOP::FileExists` (0x00989fdb),
`AudioConfig_Init` (0x0088db6a) and its helpers.

## Q1. Which file plays for a sound id

- **The EC plays only the file named by `audio_sounds.csv`.** **confirmed**. `PlaySoundById` looks the id
  up with `SoundTable_GetOrInsert` and builds `sprintf("%s/%s", "data/audio", FilePath)` in a 255-byte
  buffer. Every shipped `FilePath` is under `sounds/` and ends in `.mp3`.
- **`legacysounds/` is never used.** **confirmed**. UOSA.exe has no `legacysound` string, in ASCII or
  UTF-16. No setting, option or packet selects the WAVs. The `data/audio/legacysounds/*.wav` entries are
  dead data for this client.
- **The id is the server's id.** **confirmed**. `HandlePlaySoundPacket` passes the u16 id at packet offset
  +6 to `PlaySoundById` unchanged. 0xFFFF is skipped. The mode byte is passed as is. There is no ±1
  offset: the server id equals the CSV `ID` (KR differs, see `uokr_audio.md`). `g_UOSoundEnabled` gates
  everything.
- **Modes.** **confirmed**:
  - 0: 2D. Id 0 is ignored.
  - 1: positional. Gain is `1 - d * (1/24) * 0.9`, clamped to 0..1.
  - 2: stop all.
- **A row without a UOP entry is silent.** **confirmed**:
  1. `SoundSystem_LoadSample` queues an asynchronous load through the Miles file callbacks.
     `MilesFileOpenCallback` looks the name up only in the mounted UOPs: `UOP::FileExists`, mode 2,
     archive only, then `GetFileDecompressedSize`. There is no loose-file fallback and no fallback to
     another file.
  2. The load fails and the slot is marked failed (state 3). `SoundSystem_PlaySample` logs it and
     returns 0.
  3. In mode 0 the failed handle stays cached, so the id is never retried in that session. In mode 1 the
     handle is reset, so it is retried on every play.
- **An id with no CSV row is silent too.** **confirmed**. `operator[]` inserts an empty record. Its path
  becomes `data/audio/`, which never exists.
- **Music.** **confirmed**. `PlayMusicById` works the same way with `audio_music.csv`.
- **MythAudio's own tables are dead code.** **confirmed**. The configuration reader that loads
  `audio_filelist.csv` and the other MythAudio tables (`AudioConfig_Init`) has no caller in UOSA.exe.

### CSV reader

**confirmed**:

- **Lines.** `ReadLine` stops at CR or LF and consumes CR, LF or CRLF. A line holds at most 0x7fe
  characters. `NextLine` strips one trailing CR/LF.
- **Fields.** `ReadStringField` splits on commas. A double quote toggles quoting and is dropped. There are
  no escapes and no whitespace trimming. `ReadIntField` is `atol` of the string field, so a non-number
  reads as 0.
- **Names.** `FilePath` goes through `UOP::NormalizePath`: `\` becomes `/`, then `tolower`. There is no
  trimming and no `//` collapse. The UOP name hash is taken over `data/audio/` + that string, so a path with
  a space, a typo or a doubled slash names a different (absent) entry.

## Q2. UOP names of the audio entries

- **Name pattern.** **confirmed**: `data/audio/<FilePath of audio_sounds.csv or audio_music.csv>`,
  lowercase, forward slashes. Hash: lookup3 `hashlittle2`, as for every UOP. No other audio name is built
  by the client.
- **Recovered names of 5 unnamed MP3s** (**confirmed** by hash):

  | Hash | Name |
  |---|---|
  | `E99FA5FB5E4712E2` | `data/audio/sounds/highseas/pirates_cannon_shot_01.mp3` |
  | `438745BCB223823C` | `data/audio/sounds/highseas/pirates_electric_crackle_01.mp3` |
  | `A5D6DDC3E7F68E44` | `data/audio/sounds/highseas/pirates_cannon_fuse_01.mp3` |
  | `8131CA6737014352` | `data/audio/sounds/highseas/pirates_cavern_roar_01.mp3` |
  | `B7D9D1C334E536A7` | `data/audio/sounds/highseas/pirates_water_roar_01.mp3` |

  None of these 5 names appears in `audio_sounds.csv`, so the client never plays them. The HighSeas rows
  name other files:
  - IDs 1636–1640 name `sounds/highseas/<name>` without the `pirates_` prefix;
  - IDs 1628–1633 name `pirates_*` files that do not exist.

  All 11 HighSeas IDs are therefore silent in the EC. **confirmed** by hash and CSV. The 5 MP3s are the
  same recordings as the matching legacy WAVs: each is 0.078 s shorter, and all carry 2010 ID3 tags.
  **probable**.
- **Other CSV IDs that name no entry** (**confirmed** by hash):

  | ID | CSV path | What exists |
  |---|---|---|
  | 19 | `wave02_l.mp3` | only the legacy WAV |
  | 856 | `flame04 .mp3` (with a space) | `flame04.mp3` |
  | 1469 | `forcearro.mp3` | `forcearrow.mp3` |
  | 1471 | `lightningarro.mp3` | `lightningarrow.mp3` |
  | 1658–1660 | `drz1`, `drz2`, `drz3` | nothing |
- **Still unnamed** (**probable**: no reader in the client, and no name found):
  - **The 3 text lists.** `2C34B9194A0A0560` lists 11 `Sounds\weapons\` files, `668DFEC4FE8F74A1` 16
    `Sounds\z_end\` files and `AB055D6281ECFEB1` 26 `Sounds\traps\` files. They are CRLF folder listings,
    and KR's `Audio.uop` holds the same hashes. No client string or reader matches them. They are
    build-tool leftovers.
  - **`814E39778A04107C`.** MP3, 44.1 kHz mono, 0.601 s, with the same 2010 tags as the HighSeas set.
  - **`8E7736F30EEE5E3B` and `DBDCE27B2628F5B6`.** An MP3 (32 kHz mono, 6.876 s, Lavf58.29, 2019-09-30) and
    a WAV (22,050 Hz mono 16-bit, 6.774 s, Adobe Audition, bext 2019-09-30, with LIST/bext/iXML/_PMX
    chunks). They look like one sound as an MP3 plus its legacy WAV. No CSV row and no client string names
    them.
  - Dictionary attacks found none of their names: about 0.8 M names built from the CSV, folder and
    file-name parts, and 4.7 M from merged dictionaries.

## What the replace/add path should do

1. **EC: write only `data/audio/` + the CSV `FilePath`.** The EC never reads `legacysounds/`, so replacing
   a legacy WAV changes nothing in the EC. Report that when a user targets one.
2. **Normalize every name the way the client does:** lowercase, `/`. Never trim or fix a CSV path on the
   client's behalf. Hash exactly what the client hashes.
3. **Validate the CSV against the archive.** Every row should name an existing entry. Report the shipped
   typos above as warnings (IDs 19, 856, 1469, 1471, 1628–1640, 1658–1660), not as errors, because they
   ship that way.
4. **Adding a sound needs two changes:** a new UOP entry, and a new `audio_sounds.csv` row (`ID,FilePath`)
   whose `FilePath` hashes to that entry. The `ID` is the server sound id, with no offset. A `FilePath`
   with a space or upper case still works if the entry was hashed from its normalized form.
5. **Fixing a shipped silent row** is a CSV edit (for example `flame04 .mp3` → `flame04.mp3`). The HighSeas
   rows can be fixed by renaming either side.
