# ghidra-ec-anim-loading

Traces how UOSA.exe (Enhanced Client) loads a mobile animation, from "a body/action is needed" to "the frame is a
D3D9 texture": the thread, the UOP read and zlib inflate, the asset cache and its eviction, the per-frame
composite cache, `MobAnimTexture` and the Gamebryo DX9 texture creation. It also answers why the client stutters
when new mobiles or actions come into view.

Findings: [results/uosa_anim_loading.md](results/uosa_anim_loading.md). Every result is marked **V** (read in
the code) or **I** (inferred). Builds on [`ghidra-animation`](../ghidra-animation/README.md),
[`ghidra-animation-frames`](../ghidra-animation-frames/README.md) and the UOP trace in
[`ghidra-source-names`](../ghidra-source-names/README.md).

## Program

`UOSA.exe` in the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply`
and saved. SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`. Do not commit the binary or
anything extracted from it.

## NameEcAnimLoadingCode.java

GhidraScript, category `ECResearch`, built like `ghidra-classic-text-tables/NameClassicTextTableCode.java`.

```
NameEcAnimLoadingCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameEcAnimLoadingCode.java",
                  args="<scratch>/names.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `calledby:`, `ref:`, `import:`, `const:`). A failed check is reported
  and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions into `UOAssets`, `UOAnimations`, `UOApplication`, `UOSettings`, `UOClock`, `MobAnimTexture`,
  `MobAnimTriShape`, `MobAnimTriShapeShadow`, `NiDX9Renderer` and `NiDX9SourceTextureData` (tag
  `UO_EC_ANIM_LOADING`) and sets a plate comment `NameEcAnimLoadingCode: <tag> <name> [(probable)]: <evidence>`.
  Names of another source are kept.

Results of the 2026-11-01 run: 29 functions named and tagged (one tag created, 29 plate comments), no mismatches,
kept names or conflicts ([results/uosa_anim_loading_names.md](results/uosa_anim_loading_names.md)). The second
`--apply` reported `changes: none`, and the program was saved.

Probes (kept in `~/ghidra_scripts`, not committed): `UnknownsVtableDump.java` (vtable slots with decompiles) and
`EcAnimLoadingFindDisp.java` (instructions in a range using a given displacement, used to find every writer of
the asset cache limits). The UOP entry statistics and inflate timings came from two short Python scripts over the
shipped `AnimationFrame*.uop`, kept in the scratch folder.
