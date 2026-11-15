# ghidra-classic-ui-files

Names the 2024 Classic client code that reads and writes `entitled.def`, the desktop (`desktop.nwb`,
`desktop.uos`), `credits.dat`, `helpindx.dat`, `Data/assetmap.xml` and the chat filter lists (`uobscene.lst`,
`ignore.lst`, `conf_ignore.lst`). It also records the grammar the uocf readers follow
(`lib/uocf/src/classic/ui/credits.rs`, `help_index.rs`, `asset_map.rs`, `filter_lists.rs`), and which of these files
are only written by the client.

Findings: [results/classic_ui_files.md](results/classic_ui_files.md). Every result is marked **confirmed**
(read in the code) or **probable** (inferred from the code and the shipped data).

## Programs

The program is in the Ghidra project `UOSA`. It was imported read-only, never executed, auto-analysed, then named
with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |

The other clients (Gold, ML, 7.0.20, `uotd.exe`, UOKR, UOSA) were searched for the file names only. The client
folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything extracted from
them.

## NameClassicUiFileCode.java

A GhidraScript, category `ECResearch`, built like `ghidra-classic-text-tables/NameClassicTextTableCode.java`.

```
NameClassicUiFileCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe",
                  script_name="<abs path>/NameClassicUiFileCode.java", args="<scratch>/names.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256. It refuses any other program.
- It checks every address first (`calls:`, `calledby:`, `ref:`, `string:`, `const:`). When a check fails, it
  reports the failure and names nothing.
- `OUTPUT_FILE` is created with CREATE_NEW.
- Without `--apply`, the script only reports.
- With `--apply`, it renames DEFAULT functions into `UOClassicUiFiles` (tag `UO_CLASSIC_UI_FILES`) and sets the
  plate comment `NameClassicUiFileCode: <tag> <name> [(probable)]: <evidence>`. It keeps names that come from
  another source.

The 2026-11-02 run named and tagged 23 functions, created one tag and set 23 plate comments
([results/classic_ui_file_names.md](results/classic_ui_file_names.md)). The first report run had one mismatch:
the root-node string is referenced by a callee of `AssetMap_Load`, not by `AssetMap_Load` itself. That check was
replaced with `calls:00419a90`. The second `--apply` reported `changes: none`, and the program was saved.
