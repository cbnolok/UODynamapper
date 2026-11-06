# ghidra-gump-art

Names the Enhanced Client code that draws a single gump of `Interface.uop`
(`data/interface/default/textures/gumpart/%08d.tga`) outside the paperdoll, and records how it treats
transparency:

- one texture request, `UOGumpArt::RequestGumpArtTexture`, serves the Lua `RequestGumpArt` (legacy containers and
  books) and the generic server gumps (`gumppic`, `resizepic`, `picinpic`, `gumppictiled`, buttons);
- the texture is the TGA as stored: 24 bits becomes a texture without alpha, 32 bits keeps its alpha byte; no colour
  key and no alpha fix-up;
- the UI quad blends with `SrcAlpha, InvSrcAlpha`, no alpha test, z off, texture modulated by the window tint and
  alpha; `checkertrans` halves the alpha of the items inside its rectangle.

Findings: [results/uosa_gump_art.md](results/uosa_gump_art.md), with the answer for gumps 1059, 5595 and 1621 and what
a renderer must do to match. Every result is marked **confirmed** (decompiler and assembly), **decompiler** or
**inferred**. The paperdoll's own colour key is in [`ghidra-paperdoll`](../ghidra-paperdoll/README.md) (items 15–18).

## Program

In the Ghidra project `UOSA`, imported read-only, never executed, auto-analysed, named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |

The client folder is under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binary, `Interface.uop` or
anything extracted from them.

## NameGumpArtCode.java

GhidraScript, category `ECResearch`, built like `ghidra-paperdoll/NamePaperdollCode.java`.

```
NameGumpArtCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="/UOSA.exe", script_name="<abs path>/NameGumpArtCode.java",
                  args="<scratch>/uosa_gump_art_names.md --apply")
```

- It names fixed addresses of the tested file, selected by SHA-256; any other program is refused.
- Every address is checked first (`calls:` for a call, a vtable store or a constant's address, `calledby:`,
  `string:`, `const:`). A failed check is reported and nothing is named.
- Namespaces may be nested (`MythUi::ScreenTexture`); missing levels are created.
- `OUTPUT_FILE` is created with CREATE_NEW. Without `--apply` the script only reports. With `--apply` it renames
  DEFAULT functions (source ANALYSIS), adds the tag `UO_GUMP_ART` and sets a plate comment
  `NameGumpArtCode: UO_GUMP_ART <name> [(probable)]: <evidence>`. Names of another source are kept.

Results of the 2026-11-06 runs on UOSA.exe: 36 functions named and tagged, no mismatches, kept names or conflicts.
The second `--apply` reported `changes: none`, and the program was saved.

Read-only probes used for the research, kept in `~/ghidra_scripts/` (GhidraMCP deletes inline scripts):
`GumpArtProbeRefs20261106.java` (references to addresses or `sym:` names) and `GumpArtProbeNames20261106.java`
(current name, source, tags and plate comment), plus the GhidraMCP decompiler and disassembler.

The texel counts for the example gumps came from a throwaway script over an extracted `Interface.uop` in scratch;
neither the script's output nor the data is committed.
