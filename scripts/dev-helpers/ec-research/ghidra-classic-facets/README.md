# ghidra-classic-facets

Names the Classic client (`client.exe`) code around the facet table:

- the 48-byte facet records: size, wrap rectangle, file ids and the Felucca-rules flag;
- the second table copy, selected by bit 15 (0x8000) of the 0xA9 flags. It is the `map0x`/`map1x`/... variant;
- the 0xBF/0x18 map patch counts, the `mapdifl`/`stadifl` loaders and the dif lookups;
- the code that applies the wrap rectangle: movement, distance, tile build and radar.

Findings: [results/classic_facets.md](results/classic_facets.md). The EC/KR `facetNx.uop` findings are in
[../ghidra-facets/results/uosa_facets.md](../ghidra-facets/results/uosa_facets.md#facetnx-variants-and-the-classic-x-maps)
and [../ghidra-facets/results/uokr_facets.md](../ghidra-facets/results/uokr_facets.md#no-facetnx-variants-in-kr).

## Programs

All are imported into the Ghidra project `UOSA` under `/classic`. Each was imported read-only from a copy,
never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | PE timestamp / version | SHA-256 |
|---|---|---|---|
| `classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` | 7.0.103.0 | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |
| `classic_7.0.20.0_client.exe` | `Ultima Online Classic_7_0_20_0/client.exe` | 7.0.20.0 | `27eef2c0a4679f204b923f29dd1506bb2c7e649410054aefad680264bb504c9e` |
| `classic_6.0.14.3_client.exe` | `_packs/Ultima Online 6.0.14.3 Patch95.tar.zst` | 6.0.14.3 | `9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d` |
| `gold_client.exe` | `Ultima Online Gold/client.exe` | 2004-11-11, no version resource | `aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2` |

Other builds were not imported. Their facet tables were decoded from the file bytes only (see the results):

| Build | Source | SHA-256 |
|---|---|---|
| Mondain's Legacy folder | `Ultima Online Mondain's Legacy/client.exe` (2005-07-26) | `17ab35e10bf38077fa101938a4a0b85d1b4caa505245fc6bc83d3b131a0c2918` |
| 5.0.9.1 | `_packs/uoml_setup_fully_patched_5.0.9.1.zip` | `6250963022ec03855dedf89176c54dc7ad603e230b415edaa4b08d1509f568e9` |
| 7.0.50.0 | `_packs/Ultima Online Classic_7_0_50_00.zip` | `8b3af41390d34930ac1dc914b865edb7961c5448e0592b43a912790814987940` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything
extracted from them.

## NameClassicFacetCode.java

GhidraScript, category `ECResearch`.

```
NameClassicFacetCode.java OUTPUT_FILE [--apply]
```

- It names fixed addresses. It holds one entry set per tested build (the four imported programs above),
  selected by SHA-256, and refuses any other program.
- It checks every address before naming it:
  - a function must exist there, and its stated checks must hold. The checks are references to the
    table fields or globals, callers, referenced strings (`mapdifl%d.mul`, `"You have no characters on
    %s!"`, ...), constants (9999, 0x244, 0x8001), and `refs:0` for the dif lookups that have no caller;
  - a data label (the facet tables, `g_UseFacetVariants`, `g_SiegeRules`, the wrap globals, the Gold dif
    maps) must be referenced by a stated function. A facet table's first record must also name "Felucca".

  A failed check is reported as a mismatch and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW: an existing file stops the run before any change. Spaces in the
  path are passed as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`/`DAT_...`) symbols with source ANALYSIS into the `UOClassicFacet` namespace;
  - adds the tag `UO_CLASSIC_FACET`;
  - sets a plate comment `NameClassicFacetCode: UO_CLASSIC_FACET: <evidence>` when the comment is empty, and
    refreshes its own comment from an earlier run.
- It keeps and reports names from another source. A second `--apply` reports `changes: none`.

Run it through ghidra-mcp, once per program:

```
run_ghidra_script(program="classic_7.0.103.0_client.exe",
                  script_name="<abs repo path>/scripts/dev-helpers/ec-research/ghidra-classic-facets/NameClassicFacetCode.java",
                  args="<scratch>/classic_7.0.103.0_names.md --apply")
save_program(program="classic_7.0.103.0_client.exe")
```

The `--apply` reports are in `results/`: [7.0.103.0](results/classic_7.0.103.0_names.md),
[7.0.20.0](results/classic_7.0.20.0_names.md), [6.0.14.3](results/classic_6.0.14.3_names.md),
[Gold](results/gold_names.md).

## Reproducing the research

- Import: `import_file` of a scratch copy of each `client.exe` into `/classic`, then auto-analysis.
- Facet tables: in every build, find the `.rdata` pointer to the string `"Felucca"` followed by 48-byte
  records. The file name table is located from the string `staidx0.mul`. Its stride is the distance
  from the preceding `map0...` name: 56 bytes up to 7.0.20, 60 bytes from 7.0.50. A file id is the index
  of an entry in that table.
- Field readers: an imm32 scan of `.text` for the table address range finds every accessor. In 7.0.x
  the accessors read both copies.
- Ghidra created no function at 0x0056cfe0 in 7.0.103 (it has no caller). It was created with
  `create_function` before naming.
