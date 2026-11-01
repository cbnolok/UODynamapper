# ghidra-classic-art

Names the code of the Classic Client that reads the art records of `art.mul` and of the `artLegacyMUL.uop` entries
`build/artlegacymul/%08i.tga`, and records the rule the uocf art writer (`lib/uocf/src/classic/art_write.rs`)
follows: the art id says what a record holds, below 0x4000 a land tile, any other id a static (item id + 0x4000).
The record bytes carry no kind.

Findings: [results/classic_art.md](results/classic_art.md). Every result is marked **confirmed** (read in the code)
or **probable** (inferred from the code and the shipped data).

## Program

In the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binary or anything
extracted from it.

## NameClassicArtCode.java

GhidraScript, category `ECResearch`, built like `ghidra-classic-frames/NameClassicFrameCode.java`.

```
NameClassicArtCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe", script_name="<abs path>/NameClassicArtCode.java",
                  args="<scratch>/classic_art.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `calledby:`, `ref:`, `string:`, `const:` for an operand such as
  0x4000 and 0xffdc). A failed check is reported and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions into `UOClassicArt` and `UOClassicFiles`, adds the tag `UO_CLASSIC_ART` and sets a plate
  comment `NameClassicArtCode: UO_CLASSIC_ART <name> [(probable)]: <evidence>`. Names of another source are kept.

Results of the 2026-11-01 runs: 6 functions named and tagged (tag created, 6 plate comments). No mismatches,
kept names or conflicts. The second `--apply` reported `changes: none`, and the program was saved.

Probes: none; the GhidraMCP decompiler and cross-references from the MUL record lookup `0x0056e000` (named in
`ghidra-image-decoders` results as `FUN_0056e000`).
