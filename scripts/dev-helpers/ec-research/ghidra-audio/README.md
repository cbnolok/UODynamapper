# ghidra-audio

Names the audio code of the Enhanced Client, Kingdom Reborn, the Classic clients and the Miles Sound System
files the EC ships. It records what a tool that replaces or adds sounds must produce for the clients to play
them:

- which file the EC plays for a sound id, and what a missing entry does;
- the UOP names of the audio entries, with names recovered for 5 of the unnamed EC MP3s;
- what Miles accepts as WAV and MP3, and what the client itself checks (nothing);
- the KR `audio_*.csv` grammar;
- the Classic `Sound.def` parser, the 40-byte sound header, the 22,048 Hz playback buffer, and the
  `MUSIC\DIGITAL\CONFIG.TXT` loop flag.

Findings:

- [results/uosa_audio.md](results/uosa_audio.md): UOSA.exe, the play path, CSV reader, UOP names, the
  unnamed entries.
- [results/miles_audio.md](results/miles_audio.md): mss32.dll and miles/mssmp3.asi (Miles 7.2c), the WAV
  and MP3 acceptance rules, the replace whitelist.
- [results/uokr_audio.md](results/uokr_audio.md): UOKR.exe, the `audio_*.csv` tables.
- [results/classic_audio.md](results/classic_audio.md): Classic 7.0.103.0 and UO Gold: Sound.def,
  Music.def, the sound header, Config.txt.

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
| UOKR.exe | 18 |
| classic_7.0.103.0_client.exe | 12 |
| gold_client.exe | 2 |
| mss32.dll | 5 |
| mssmp3.asi | 1 |

There were no mismatches, kept names or conflicts. The second `--apply` reported `changes: none` on all six
programs, and all six were saved.

The probes used during the research are kept outside the repository, in `~/ghidra_scripts/`
(`AudioProbe*20261101.java`). They are read only.
