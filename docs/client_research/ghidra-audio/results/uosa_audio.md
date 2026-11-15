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

## Playback: how the client drives Miles

Read in Ghidra on 2026-11-03. **confirmed** = read in the code. It is a static reading, not a measurement: nothing
was run. "Dead" means that no static reference reaches the code from a live root; a caller through a table that
Ghidra did not see would change that.

**Live paths.** `PlaySoundById` and `PlayMusicById`, both through `SoundSystem_PlaySample` (0x00884ff9).

**Driver** (`SoundSystem_OpenDriver` 0x008859f1, call at 0x00885a21). **confirmed**: 48,000 Hz (0x00da7038), 16 bits
(0x00da703c) and spec 0x10 (0x00da7044), constants in `.data`, not config. So the output layout follows the Windows
speaker setting, and is stereo by default. If the open fails it tries 44,100, 22,050 and 11,025, then the same at 8
bits. `FUN_008843fe` opens and closes spec 0xE0 (EAX 4) once as a probe and restores the spec.

**Volume** (`SampleSound_ApplyVolume` 0x00898993, the +0x38 slot of the vtable at 0x00ce4964). **confirmed**:

1. `vol = category * own volume`, the own volume clamped to 0..1 and to the cap at +0x10.
2. `AIL_set_sample_volume_pan(h, 0.81, pan)` (0x00898a1e), with 0.81 a constant (0x00d05070).
3. `AIL_sample_volume_levels` (0x00898a34), both sides multiplied by `vol`, then `AIL_set_sample_volume_levels`
   (0x00898a98).

So Miles' `v ^ (5/3)` applies only to the constant, and the volume itself is **linear**. At centre pan the factor is
`0.81 ^ (5/3) * 0.8123`, about 0.572 or -4.9 dB, times `vol`.

**Distance** (`PlaySoundById` 0x004b5770). **confirmed**: mode 1 is `1 - d * (1/24) * 0.9`, clamped to 0..1, with `d`
the Euclidean distance to the player object. It goes in as volume and cap. Pan is always 0.5, so **there is no
positional pan**. Mode 0 is gain 1.0. Pitch is `native_rate * multiplier` through `AIL_set_sample_playback_rate`.

**Music** (`PlayMusicById` 0x004b5a30). **confirmed**:

- A sample, not a stream: `AIL_set_named_sample_file`, then `AIL_set_sample_loop_count(h, 0)` when the CSV loop byte
  (+0xff) is set. So the **sample loop gap** applies (at least 39 ms for a tagged 44.1 kHz file, see
  `miles_playback_reference.md`).
- **No crossfade.** The same id is ignored. A different id goes to `SoundSystem_RequestSampleFile` (0x00884aeb), and
  `SoundSystem_EndSampleHandles` (0x00884bce) ends the id's playing handles at once. The 3,000 ms passed to
  `PlaySample` (0x00c9b8a4) is used only as the fade time of a later stop, and there is no fade-in.
- Id 0xffff stops with that 3 s fade (`SampleSound_Stop` 0x00898ae8), linear in time through the levels.
- The new file loads asynchronously, so the silence between tracks is the load time.
- The ducking value at +0x144 is only ever zeroed.

**Filters, reverb, occlusion, 3D.** None on the live paths, as far as static references go:

- `AIL_set_sample_3D_*` run in `SampleSound_StartAtPosition` only when +0x75 is set, by `FUN_00898e63`, from
  `SoundSystem_PlaySample3D` (0x008851db). Its only caller is `FUN_00892dd7`, whose closure ends in unreferenced
  roots (0x0087f10c, 0x0088efb6, 0x0088f761, 0x0088f7c8, 0x0088f7ff, 0x0088f832, 0x0088f86c, 0x0088f89f).
- Two references at 0x00893adb and 0x00893b30 come from code outside any function, so they could not be attributed.
  That is the weak point of this conclusion.
- `FUN_008853fc`, the only taker of `StreamSound`, is in the same closure, so `StreamSound` has no live path.
- The filter adders (`FUN_00897f41`, from `FUN_00889a2f` and `FUN_00898c05`) work on a list filled from zones set only
  in the dead root 0x0087f10c. `FUN_0087f60a` is called from the sound start and tick, but it finds no zones.
- `FUN_0087ed30` and `FUN_0087ee3d` have no reference.
- The MP3 limit of 7 (+0x94, 0x008841d7) applies only when an entry's +0x120 is set, by `AudioConfig_Init`'s
  closure, which is dead too.

**Voices** (`SoundSystem_Startup` 0x008840e1). **confirmed**: up to 64 handles, the last released, so 63. There are
96 `SampleSound` and 96 `StreamSound` objects, and 25,000 sample slots. `SoundSystem_FindFreeHandle` (0x008859be)
returns -1 when none is free and the start returns -5: **the sound is dropped, with no stealing and no priority**. A
category at volume 0 refuses the start (`FUN_00885e47`, -7).

**Options** (`ApplyVolumeOptions` 0x004b5c00). **confirmed**, all on a 0..1 scale, clamped, stored as `%0.2f`:

| Option | Global | Flag | Reaches Miles as |
|---|---|---|---|
| Master | 0x00e33540 | 0x00e3354c | `AIL_set_digital_master_volume_level(drv, 1.0 * master)`, linear |
| Music | 0x00e33548 | 0x00e3354e | Category 0 (`FUN_004b5de0`), a linear factor in `vol` |
| Effects | 0x00e33544 | 0x00e3354d | Categories 1 to 4 (`FUN_00885631`), a linear factor in `vol` |

A flag that is off sends 0. `FUN_004b50a0` saves and mutes the master on event 0x28 and restores it on 0x29, probably
window focus.

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
