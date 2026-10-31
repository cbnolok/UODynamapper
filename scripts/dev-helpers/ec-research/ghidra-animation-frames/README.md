# ghidra-animation-frames

Names the `AnimationFrame*.uop` (AMOU) code in `UOSA.exe` (Enhanced Client) and `UOKR.exe` (Kingdom Reborn),
and records what an encoder must produce for the clients to accept a frame file: the header fields the client
reads, the frame entry search, the palette, the RLE decoder rules and the atlas limits.

Findings:

- [results/uosa_animation_frames.md](results/uosa_animation_frames.md): UOSA.exe; adds the frame entry
  search to the functions already named by [`ghidra-animation`](../ghidra-animation/README.md).
- [results/uokr_animation_frames.md](results/uokr_animation_frames.md): UOKR.exe; the same functions, their
  differences from UOSA.exe (palette hue key, constructor convention, empty atlas fill).

The byte-level format and the encoder rules inferred from the shipped files are in
`lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md` (AnimationFrame section).

## NameAnimationFrameCode.java

GhidraScript, category `ECResearch`. Works on both programs; uses no fixed addresses.

```
NameAnimationFrameCode.java OUTPUT_FILE [--apply]
```

- `OUTPUT_FILE` is created with CREATE_NEW. If the file already exists, the run stops before making any change.
- Without `--apply`, the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, in the RTTI class namespaces or in
    `UOAnimations` / `UOHues`;
  - adds the tag `UO_ANIMATION_FRAME` (also to the UOSA.exe functions that already carry `UO_ANIMATION`);
  - sets a plate comment `NameAnimationFrameCode: UO_ANIMATION_FRAME <name> [(probable)]: <evidence>` when
    the comment is empty.
- Names with another source type are never changed; they are listed under "Kept names".
- A second `--apply` reports `Changes: none`.

```
run_ghidra_script(program="UOKR.exe", script_name="<abs path>/NameAnimationFrameCode.java",
                  args="<scratch>/uokr_frames.md --apply", timeout_seconds=1200)
```

### How functions are found

| Evidence | Finds |
|---|---|
| RTTI vtable of `UOAnimationFrameSetFactory` (+4) | `UOAnimationFrameSetFactory::Create` |
| Create callee that writes the `UOMobFrameSet` vftable | `UOMobFrameSet::UOMobFrameSet` |
| Functions without calls that use the packed mask 0xff00ff and the scalar 0x80; their common caller; the one with more than one caller is the normal decoder | `DecodeFrameRle`, `DecodeFrameRleMirrored`, `BuildFrameAtlas` |
| Callee without calls shared by the atlas and every atlas caller | `GetLayerBounds` (probable) |
| Callee of another DecodeFrameRle caller that uses 0x80 but not 0xff00ff and has no calls | `DecodeFrameMask` (probable) |
| Call-free function under 256 bytes that reads +0x20 and +0x24 and halves with `SAR 1`, whose caller calls a function that calls a small user of 0x4001 | `FindFrameEntry`, `BuildFramePalette`, `UOHues::GetHueTable` |

## animation_frame_variables.spec

Applied by `../ApplyVariableSpecs.java` after `NameAnimationFrameCode.java`
(see [the shared description](../README.md#variable-names-prototypes-and-structures-applyvariablespecsjava)):

```
run_ghidra_script(program="UOKR.exe", script_name="<abs path>/ApplyVariableSpecs.java",
                  args="<abs path>/ghidra-animation-frames/animation_frame_variables.spec <scratch>/frames_vars.md --apply")
```

- Repeats, unchanged, the shared types it needs (`/UO/SharedPtr`, `AssetBlob`, `AssetBlobSharedPtr`,
  `/UOEffects/MsvcVector`, `/UO/AnimationFrameFile`, `AnimationFrameEntry`), so it also works on UOKR.exe,
  where the tileart, effects and animation scripts were never run. On UOSA.exe they report "unchanged".
- Sets 9 prototypes. On UOSA.exe only `FindFrameEntry` is new (the others come from
  `animation_variables.spec` and are identical); on UOKR.exe all 9 are new. The UOMobFrameSet constructor is
  left out because the two clients pass its object differently (UOSA.exe on the stack, UOKR.exe in ESI).
- `FindFrameEntry` takes the frame file in EAX and pops its two stack arguments (`RET 8`): `__stdcall` with
  custom storage.

Results of the 2026-10-31 runs: UOSA.exe 1 function renamed (`FindFrameEntry`), 10 tagged, 1 prototype set;
UOKR.exe 10 functions renamed and tagged, 6 types created, 9 prototypes set; no anomalies; the second
`--apply` of both scripts reported `Changes: none` on both programs, and both programs were saved.
