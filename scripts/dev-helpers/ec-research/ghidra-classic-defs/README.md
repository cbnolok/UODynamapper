# ghidra-classic-defs

Names the Classic Client code that reads the def files `art.def`, `body.def`, `bodyconv.def` and `Equipconv.def`,
and the Enhanced Client code that loads an Interface XML document, and records the grammar the uocf def readers
(`lib/uocf/src/classic/def_files.rs`) follow and why the EC drops `startmenu.xml`
(`lib/uocf/src/enhanced/syntax_check.rs`). `gump.def`, `anim1-4.def`, `texterr.def` and `corpse.def` go through
`UOClassicDefs::DefTable_Load` and `DefTable_ParseLine`, named by `ghidra-audio`.

Findings: [results/classic_defs.md](results/classic_defs.md). Every result is marked **confirmed** (read in the
code) or **probable** (inferred from the code and the shipped data).

## Programs

In the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything
extracted from them. The UO Gold client's `bodyconv.def` reader (`gold_client.exe` `0x004f4650`) was read, not named.

## NameClassicDefCode.java

GhidraScript, category `ECResearch`, built like `ghidra-classic-art/NameClassicArtCode.java`.

```
NameClassicDefCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe" or "UOSA.exe",
                  script_name="<abs path>/NameClassicDefCode.java", args="<scratch>/names.md --apply")
```

- It names fixed addresses of the programs above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `calledby:`, `string:`, `const:`). A failed check is reported and
  nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions into `UOClassicDefs` (tag `UO_CLASSIC_DEFS`) or `UOXml` (tag `UO_INTERFACE_XML`) and sets a
  plate comment `NameClassicDefCode: <tag> <name> [(probable)]: <evidence>`. Names of another source are kept.

Results of the 2026-11-01 runs: 6 functions of the Classic client and 2 of UOSA named and tagged (two tags
created, 8 plate comments). No mismatches, kept names or conflicts. The second `--apply` on each reported
`changes: none`, and both programs were saved.

Probes: none; the GhidraMCP decompiler and cross-references from the def file names and the log message.
