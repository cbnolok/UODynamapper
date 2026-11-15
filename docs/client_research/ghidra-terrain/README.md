# ghidra-terrain

Names the code in `UOSA.exe` (Enhanced Client) that reads `TerrainDefinition.uop` (asset type 3) and uses
the definitions:

- the binary factory and the record parser;
- the lookup by material id;
- the terrain mesh, coverage and layer loaders;
- the per-cell land flag and material accessors.

It also names the TerrainDefinition parser in `UOKR.exe` (Kingdom Reborn).

Findings: [results/uosa_terrain.md](results/uosa_terrain.md). It covers:

- the field table of every record byte, with types, meaning, client use and confidence;
- the three floats;
- each flag bit, read or not read by the client;
- the offline validation over all 201 entries;
- a model, encoder and invariants recipe for the repo;
- the KR variant (12-byte aliases).

The script reports after `--apply` are [results/uosa_terrain_names.md](results/uosa_terrain_names.md) and
[results/uokr_terrain_names.md](results/uokr_terrain_names.md).

## NameTerrainCode.java

GhidraScript, category `ECResearch`.

```
NameTerrainCode.java OUTPUT_FILE [--apply]
```

- It names fixed addresses. It holds one entry set per tested build, selected by SHA-256:
  - `UOSA.exe` `5a5e9dc5...ebad50`, the same build as `ghidra-uop-header`;
  - `UOKR.exe` `1b802e5c...348a43`, from `UO - Old KR`.

  It refuses any other program.
- Before naming, it checks each address. A function must exist there and its stated evidence must hold:
  - the vtable slot of `UOTerrainDefinitionBinaryFactory` or `UOTerrainDefinitionBase`;
  - callers and callees;
  - the asset-type constant `0x03000000`;
  - the flag constants `0x40` and `0x100`.

  A failed check is reported as a mismatch and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW; an existing file stops the run before any change. Spaces in the
  path are passed as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS into the `UOTerrain` namespace;
  - adds the tag `UO_TERRAIN`;
  - sets a plate comment `NameTerrainCode: UO_TERRAIN: <evidence>` when the comment is empty, and refreshes
    its own comment from an earlier run when the evidence changed.
- It keeps and reports names with another source type, and reports a name already used in the namespace as
  a conflict. A second `--apply` reports `changes: none`.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameTerrainCode.java",
                  args="<scratch>/terrain_names.md --apply")
```

### Notes

- `UOTileArt::ParseShaderBlock` (0x008456a3) parses the trailing shader block. The tileart work named it
  (`ghidra-tileart`), so this script only checks the call to it.
- `UOFacet::BuildChunkFromSector` copies the alias flags into the cell. It belongs to `ghidra-facets`.
- The offline checks behind the findings are scratch tools (Python stdlib, run with `-I`).
