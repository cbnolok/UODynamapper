# ghidra-classic-frames

Names the code of the Classic Client that reads and draws the mobile animation frames, from `anim*.mul` and
from the `AnimationFrame*.uop` entries `build/animationlegacyframe/%06d/%02d.bin`, and records the frame format
the uocf writer (`lib/uocf/src/classic/animation/anim_frames.rs`) follows:

- both sources hold the same frames: a 256-word RGB555 palette, then per frame an `i16` anchor x / y, `u16`
  width / height and runs (`u32` header: bits 0..11 length, 12..21 signed y, 22..31 signed x), ended by
  0x7FFF7FFF;
- the UOP entries wrap them in the AMOU container of the Enhanced Client, of which the client reads only the
  frame count, the table offset and each entry's key and data offset.

Findings: [results/classic_frames.md](results/classic_frames.md), and which UOP bodies can be requested:
[results/uop_reachability.md](results/uop_reachability.md). Every result is marked **confirmed** (read in
the code) or **probable** (inferred from the code and the shipped data).

## Program

In the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binary or anything
extracted from it.

## NameClassicFrameCode.java

GhidraScript, category `ECResearch`, built like `ghidra-paperdoll/NamePaperdollCode.java`.

```
NameClassicFrameCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe", script_name="<abs path>/NameClassicFrameCode.java",
                  args="<scratch>/classic_frames.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `calledby:`, `ref:`, `string:`, `const:` for an operand such as
  0x7FFF7FFF, 0xFFF, 0x3DEF and 0x50). A failed check is reported and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions into `UOClassicAnim`, `UOClassicFiles`, `UOClassicPackets`, `UOMobFrameSet` and
  `UOAnimationFrameSetFactory`,
  adds the tag `UO_CLASSIC_FRAMES` and sets a plate comment `NameClassicFrameCode: UO_CLASSIC_FRAMES <name>
  [(probable)]: <evidence>`. Names of another source are kept.

Results of the 2026-11-01 runs: 13 functions named and tagged (tag created, 13 plate comments). No mismatches,
kept names or conflicts. The second `--apply` reported `changes: none`, and the program was saved.

Results of the 2026-11-03 runs: 26 more functions named and tagged, and `DrawMobile` corrected to `DrawCorpse` (no
mismatches, kept names or conflicts). The second `--apply` reported `changes: none`, and the program was saved.

Probe: `~/ghidra_scripts/ClassicFramesConstProbe20261101.java` (read-only: finds the functions using an
operand), plus the GhidraMCP decompiler and cross-references.
