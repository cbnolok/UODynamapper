# ghidra-audio

Names the audio code of the Enhanced Client, Kingdom Reborn, the Classic clients and the Miles Sound System
files the EC ships. It records what a tool that replaces or adds sounds must produce for the clients to play
them:

- which file the EC plays for a sound id, and what a missing entry does;
- the UOP names of the audio entries, with names recovered for 5 of the unnamed EC MP3s;
- what Miles accepts as WAV and MP3, and what the client itself checks (nothing);
- the KR `audio_*.csv` grammar;
- the Classic `Sound.def` parser, the 40-byte sound header, the 22,048 Hz playback buffer, and the
  `MUSIC\DIGITAL\CONFIG.TXT` loop flag;
- how each client drives its audio: mixer rate, gain law, music, occlusion, delay and reverb, voice handling and the
  volume options. What that means for a playback path is in `docs/agent_notes/miles_playback_reference.md`.

Findings:

- [results/uosa_audio.md](results/uosa_audio.md): UOSA.exe, the play path, CSV reader, UOP names, the
  unnamed entries, how it drives Miles.
- [results/miles_audio.md](results/miles_audio.md): mss32.dll and miles/mssmp3.asi (Miles 7.2c), the WAV
  and MP3 acceptance rules, the replace whitelist, the mixer and WAV edge cases, `mssvoice.asi`.
- [results/uokr_audio.md](results/uokr_audio.md): UOKR.exe, the `audio_*.csv` tables, how it drives Miles.
- [results/classic_audio.md](results/classic_audio.md): Classic 7.0.103.0 and UO Gold: Sound.def,
  Music.def, the sound header, Config.txt, its DirectSound playback.

The file formats themselves are in `docs/agent_notes/ec_audio_formats.md`. Every result is marked
**confirmed** (read in the code) or **probable** (inferred from the code and the shipped data).

## Programs

All are in the Ghidra project `UOSA`. Each was imported read-only, never executed, auto-analysed, named with
`--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |
| `/UOKR.exe` | `UO - Old KR/UOKR.exe` | `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43` |
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |
| `/classic/gold_client.exe` | `Ultima Online Gold/client.exe` | `aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2` |
| `/libs/mss32.dll` | `_Ultima Online Enhanced/mss32.dll` (Miles 7.2c) | `b588b4791ff6803247962c661b6d3ff9a8625d0f60363a6fa53f61d401f0e74e` |
| `/libs/miles/mssmp3.asi` | `_Ultima Online Enhanced/miles/mssmp3.asi` | `0fb784a9cc9826a9ec2412b7d71453b917af6c74442647f38061512e2c3a05bb` |
| `/libs/miles/mssvoice.asi` | `_Ultima Online Enhanced/miles/mssvoice.asi` | `c0163aaf5c6ac0b252261dfbf718c32de02a34efc97761d6612bda3d529b772e` |
| `/libs-kr/mss32.dll` | `UO - Old KR/mss32.dll` (Miles 7.0x) | `272f65a0b4cb84aef765ee80547341de1b1dd9d16907add2d784cd5ad04d155a` |
| `/libs-kr/miles/mssmp3.asi` | `UO - Old KR/miles/mssmp3.asi` | `f6bbeccf925b1f2b3132143b5a289ea7041554e30d739560b9d5eadf1a427bdd` |
| `/miles-7.2a-sdk/mss32.dll`, `/miles-7.2a-sdk/mssmp3.asi` | The user's own copy of the Miles 7.2a SDK, with its PDBs | Not recorded |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything
extracted from them.

`/libs/mssmp3.asi` (directly under `/libs`) is an earlier failed import of the same file: it was loaded as
raw binary at base 0, without PE analysis, and closed without saving. Use `/libs/miles/mssmp3.asi`. Because
both programs are named `mssmp3.asi`, pass the full path `/libs/miles/mssmp3.asi` to GhidraMCP.

## NameAudioCode.java

GhidraScript, category `ECResearch`.

```
NameAudioCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="/libs/miles/mssmp3.asi", script_name="<abs path>/NameAudioCode.java",
                  args="<scratch>/mssmp3_audio.md --apply")
```

- It names fixed addresses. It holds one entry set per program above, selected by SHA-256, and refuses any
  other program.
- It checks every address before naming it. A function must exist there, and its stated checks must hold:
  - `calls:`: a reference to a function or data address;
  - `calledby:`: a reference from a stated caller;
  - `ref:`: a vtable slot (Classic def table);
  - `string:`: a referenced string with a given prefix (`audio_sounds.csv`, `miles`,
    `MUSIC\DIGITAL\CONFIG.TXT`, the Miles error texts, ...);
  - `import:`: a referenced import (`_AIL_set_named_sample_file@20`, `_AIL_set_file_callbacks@20`, ...);
  - `const:`: an operand (0x683 maximum Classic sound id, 0x5620 = 22,048 Hz, 0x28 header size, 0x61a7
    slot limit).

  A failed check is reported as a mismatch and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW: an existing file stops the run before any change. Spaces in the
  path are passed as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS into the namespaces `UOAudio`, `MythAudio`,
    `UOCsv`, `UOP`, `UOClassicAudio`, `UOClassicDefs`, `Miles` and `MilesMp3`;
  - adds the tag `UO_AUDIO`;
  - sets a plate comment `NameAudioCode: UO_AUDIO <name> [(probable)]: <evidence>` when the comment is empty,
    and refreshes its own older comments.
- Names with another source type are never changed; they are reported as "kept".
- A second `--apply` reports `changes: none`.

Results of the 2026-11-01 runs:

| Program | Functions named and tagged |
|---|---|
| UOSA.exe | 21 |
| UOKR.exe | 23 (5 added later the same day: the structure handles of `audio_effects.csv`) |
| classic_7.0.103.0_client.exe | 12 |
| gold_client.exe | 2 |
| mss32.dll | 5 |
| mssmp3.asi | 1 |

There were no mismatches, kept names or conflicts. The second `--apply` reported `changes: none` on all six
programs, and all six were saved.

The 2026-11-03 pass added the playback functions: 7 in UOSA.exe, 8 in UOKR.exe and 8 in classic_7.0.103.0_client.exe.
A second `--apply` reported `changes: none` on all three.

## TransferMilesNames.java

GhidraScript, category `ECResearch`. It carries the function names of the 7.2a SDK binaries, analysed with their PDBs
in `/miles-7.2a-sdk`, to the Miles binaries the clients ship, by comparing instructions.

```
TransferMilesNames.java OUTPUT_FILE [--apply] [--source <project path>]
```

- **Nothing from the SDK is in this repository.** The script reads the names at run time, and they stay in the Ghidra
  project. Only exported names, which are public API, appear in these notes. Keep the report and the match list outside
  the repository, since they hold the internal names.
- Each tier looks only at the functions still unmatched, and only takes a key that is unique on both sides:
  1. mnemonics, registers and constants, with addresses masked;
  2. mnemonics and registers, with every constant and address masked;
  3. mnemonics alone, for long enough bodies;
  4. the same call slot of a pair already matched, when the two bodies agree or are similar enough.
- Exported names shared by both sides are the seeds of tier 4 and check tiers 1 to 3. A pair that references strings on
  both sides with none in common is refused.
- `--apply` renames only default names, into the namespace `MilesSdk`, with the tag `MILES_SDK_NAME` and a comment
  giving the tier. A name that differs is reported as "kept". A function with no pair in any tier keeps its default
  name.
  `OUTPUT_FILE` is created with CREATE_NEW, as above.

Leaving out thunks, the SDK side has 1,249 functions, 1,066 named, in `mss32.dll`, and 33, 23 named, in `mssmp3.asi`.
Results of 2026-11-03, with a later `--apply` reporting `changes: none` on all four on 2026-11-04:

| Target | Matched | Tier 0, 1, 2, 3, 4 | Seeds, of which tiers 1 to 3 found again, and conflicts | Renamed | Kept | Source has no name |
|---|---|---|---|---|---|---|
| `/libs/mss32.dll` | 1,044 of 1,078 | 333, 589, 23, 2, 97 | 333, 273, 0 | 251 | 250 | 175 |
| `/libs/miles/mssmp3.asi` | 32 of 36 | 1, 29, 1, 0, 1 | 1, 1, 0 | 18 | 1 | 10 |
| `/libs-kr/mss32.dll` | 976 of 1,050 | 321, 495, 39, 1, 120 | 321, 201, 0 | 223 | 245 | 152 |
| `/libs-kr/miles/mssmp3.asi` | 29 of 36 | 1, 18, 7, 1, 2 | 1, 0, 0 | 19 | 0 | 7 |

Tier 0 is the exported names. One pair in the Kingdom Reborn `mss32.dll` was refused on its strings.

The `/libs/mss32.dll` names may not be on disk: on 2026-11-04 its save did not finish (see below), while the other
three were saved. Running the script with `--apply` again restores them, and nothing else depends on them.

## GhidraMCP: saves that never return

`save_program` and `save_all_programs` wait for analysis first, on one of the plugin's three request threads. On
`/libs/mss32.dll` that wait did not end: the thread spun at full CPU in a recursive
`AutoAnalysisManager.waitForAnalysis`, slower with each level, and one from 2026-11-03 was still spinning a day later.
Why analysis never reports done on that program was not found. A timeout on the caller's side does not stop the thread,
and each retry takes another. Once all three are taken, the server answers nothing. So do not retry a save that timed
out: look with `jcmd <pid> Thread.print`, and restart Ghidra if the threads are in `ProgramSaves.awaitAnalysis`.

## Probes

The probes used during the research are kept outside the repository, in `~/ghidra_scripts/`
(`AudioProbe*20261101.java`, `MilesProbe*2026110*.java`). They are read only, except `MilesSetPdb20261103.java`.
