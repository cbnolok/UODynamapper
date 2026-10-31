# ghidra-facets

Names the code in `UOSA.exe` (Enhanced Client) that reads facet data:

- the sectors in `facetN.uop` (asset type 9);
- the `facetNx.uop` variant swap;
- the FacetDefinition records in `facets.uop` (asset type 23);
- the loose `facetNN.dds` radar images;
- the radar code that uses them;
- the readers of the hard-coded facet size table `g_FacetDimensions`, the Felucca/Trammel x = 5120 split
  and the 0xBF/8 facet change.

It also names the matching code in `UOKR.exe` (Kingdom Reborn):

- the KR sector reader;
- the KR copy of the table and its readers;
- the radar DDS mip offsets;
- the chunk build requests;
- the package mount list.

Findings: [results/uosa_facets.md](results/uosa_facets.md). It covers:

- the unread 8-byte entry `build/sectors/facet_%02d/data.map` and its hashes;
- the sector format and the client's constraints on it;
- the facet dimension table and sector numbering;
- the DDS format, how the client uses it and an offline regeneration recipe with match rates;
- the full FacetDefinition layout;
- map sizes: every reader of the table, the other hard-coded dimension sites, the limits, and a cross-build
  patch recipe with SHA-256s, signatures, refusal checks, PE checksum and Authenticode;
- the KR differences;
- an assessment of ManawydanMapConverter and of the repo's encoder and decoder.

The KR findings are in [results/uokr_facets.md](results/uokr_facets.md):

- the sector format, read from the parser;
- TerrainDefinition, effects, AnimationSequence, tileart v3 and MultiCollection deltas;
- StaticChunk.uop and `data.map`.

The script reports after `--apply` are [results/uosa_facets_names.md](results/uosa_facets_names.md) and
[results/uokr_facets_names.md](results/uokr_facets_names.md).

## NameFacetCode.java

GhidraScript, category `ECResearch`.

```
NameFacetCode.java OUTPUT_FILE [--apply]
```

- It names fixed addresses. It holds one entry set per tested build, selected by SHA-256:
  - `UOSA.exe` `5a5e9dc5...ebad50`, the same build as `ghidra-uop-header`;
  - `UOKR.exe` `1b802e5c...348a43`.

  It refuses any other program.
- Before naming, it checks each address. A function must exist there and its stated evidence must hold:
  vtable slot of `UOFacetDefinition`, `UOSectorClientInstance` or the two binary factories, callers,
  callees, asset-type constants, referenced strings, and a single reference for the vtable getters. The
  table label `g_FacetDimensions` (UOSA 0x00da72a8, UOKR 0x00bfcc88) is checked by its first record
  (`"Felucca"`, 7168, 4096) and by the code that reads its width and height fields. A failed check is reported as a mismatch and nothing
  is named.
- `OUTPUT_FILE` is created with CREATE_NEW; an existing file stops the run before any change. Spaces in the
  path are passed as `%20`.
- Without `--apply` the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS into the `UOFacet` namespace;
  - adds the tag `UO_FACET`;
  - sets a plate comment `NameFacetCode: UO_FACET: <evidence>` when the comment is empty, and refreshes its
    own comment from an earlier run when the evidence changed.
- It keeps and reports names with another source type, and reports a name already used in the namespace
  as a conflict. A second `--apply` reports `changes: none`.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameFacetCode.java",
                  args="<scratch>/facets_names.md --apply")
```

### Notes

- Every reader of `g_FacetDimensions` is now named: `SetPlayerChunkPosition`, `QueueNeighbourChunks`,
  `LoadWaypoints`, `SectorIndexFromXY` and `QueueSectorChunks`, plus `FacetDds_GetMipOffset` in KR.
- The offline checks behind the findings are scratch tools: Python, stdlib plus numpy, run with `-I`.
  They cover the UOP hash and entry reader, the header-name search, the sector and FacetDefinition
  parsers, the sector statistics and the radar model. They are not part of the repo.
  `results/uosa_facets.md` gives the recipes they confirmed.
