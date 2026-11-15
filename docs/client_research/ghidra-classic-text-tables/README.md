# ghidra-classic-text-tables

Names the 2024 Classic client code that reads the custom-house tables (`walls.txt`, `floors.txt`, `doors.txt`,
`misc.txt`, `stairs.txt`, `teleprts.txt`, `roof.txt`), `suppinfo.txt`, `prof.txt`, `keynames.txt`, `palette.mul`
and `animinfo.mul`, and records the grammar the uocf readers follow (`lib/uocf/src/classic/tables/house_tables.rs`,
`professions.rs`, `key_names.rs`, `legacy_tables.rs`) and which shipped files the client never opens.

Findings: [results/classic_data_files.md](results/classic_data_files.md). Every result is marked **confirmed**
(read in the code) or **probable** (inferred from the code and the shipped data).

## Programs

In the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |

`gold_client.exe` (UO Gold) was searched for the file names only, not named. The client folders are under
`/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything extracted from them.

## NameClassicTextTableCode.java

GhidraScript, category `ECResearch`, built like `ghidra-classic-defs/NameClassicDefCode.java`.

```
NameClassicTextTableCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe",
                  script_name="<abs path>/NameClassicTextTableCode.java", args="<scratch>/names.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:`, `calledby:`, `ref:`, `string:`, `const:`). A failed check is reported
  and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions into `UOClassicDataFiles` (tag `UO_CLASSIC_DATA_FILES`) and sets a plate comment
  `NameClassicTextTableCode: <tag> <name> [(probable)]: <evidence>`. Names of another source are kept.

Results of the 2026-11-01 run: 23 functions named and tagged (one tag created, 23 plate comments), no mismatches,
kept names or conflicts ([results/classic_data_file_names.md](results/classic_data_file_names.md)). The second
`--apply` reported `changes: none`, and the program was saved.

Probes: `FindRawAddressBytesTextTables.java` (kept in `~/ghidra_scripts`, not committed) found the `sjis2uni.mul`
and `version.dat` code by the raw bytes of their name strings' addresses, in code Ghidra had not made a function;
it was read with a dry-run disassembly, not named.
