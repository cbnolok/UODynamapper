# ghidra-paperdoll

Names the code of the Enhanced Client and Kingdom Reborn that reads `Paperdoll.uop`, and records what its two
AMOU entries are:

- `build/paperdoll/000001/00.bin` and `01.bin` are the paperdoll images, one frame set per gender, in the
  `AnimationFrame` AMOU format;
- the client loads them as assets 0x40 and 0x41 of type 0x12 (`PaperdollFrameSet`), and the EC and CC
  paperdoll textures look a frame up by action = gender and frame number = the body or the worn item's id.

Findings: [results/uosa_paperdoll.md](results/uosa_paperdoll.md) (UOSA.exe and UOKR.exe). Every result is marked
**confirmed** (read in the code) or **probable** (inferred from the code and the shipped data).

## Programs

Both are in the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply`
and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |
| `/UOKR.exe` | `UO - Old KR/UOKR.exe` | `1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything
extracted from them.

## NamePaperdollCode.java

GhidraScript, category `ECResearch`, built like `ghidra-audio/NameAudioCode.java`.

```
NamePaperdollCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NamePaperdollCode.java",
                  args="<scratch>/uosa_paperdoll.md --apply")
```

- It names fixed addresses, one entry set per program above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `ref:` for a vtable slot, `const:` for an operand such as
  0x12000000, 0x40, 0x41, 0x190 = 400 and 0x989680 = 10,000,000). A failed check is reported and nothing is
  named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions (source ANALYSIS) into `PaperdollTextures`, `ECPaperdollTexture` and `CCPaperdollTexture`,
  adds the tag `UO_PAPERDOLL` and sets a plate comment `NamePaperdollCode: UO_PAPERDOLL <name> [(probable)]:
  <evidence>`. Names of another source are kept.

Results of the 2026-11-01 runs:

| Program | Functions named and tagged |
|---|---|
| UOSA.exe | 8 (9 since 2026-11-06: `UOPaperdoll::DrawGumpLayer`, the partial hue of the gump paperdoll) |
| UOKR.exe | 1 |

No mismatches, kept names or conflicts. The second `--apply` reported `changes: none` on both programs, and both
were saved.

No new probe was written: the research reused the read-only probes of `ghidra-audio`
(`~/ghidra_scripts/AudioProbe*20261101.java`) and the GhidraMCP decompiler and cross-references.
