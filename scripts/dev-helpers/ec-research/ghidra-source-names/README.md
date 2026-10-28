# Source library function names for Ghidra

`UOSA.exe` (Enhanced Client, Visual Studio 2005 build) links zlib, FreeType 2, TinyXML and
Crypto++ statically and without symbols. `NameSourceFunctions.java` names their functions
from evidence taken from the library sources: function pointer tables, MSVC vtables,
string literals and the call graph. It works for any C/C++ library once
`extract_source_names.py` has turned the library's sources into a map.

| Path | Content |
|---|---|
| `NameSourceFunctions.java` | Ghidra script: report, and with `--apply` annotations |
| `extract_source_names.py` | Generates a map from a library source tree |
| `maps/zlib-1.2.2.tsv` | zlib 1.2.2 map |
| `maps/freetype-2.3.5.tsv` | FreeType 2.3.5 map (with its bundled zlib 1.1.4, `src/gzip`, as `ft_zlib::`) |
| `maps/tinyxml-2.5.3.tsv` | TinyXML 2.5.3 map |
| `maps/cryptopp-5.4.tsv` | Crypto++ 5.4 map |
| `NameUopTrace.java` | Ghidra script: names the UOP read/decompress path of the tested `UOSA.exe` (address-based) |
| `results/uosa_uop_trace.md` | Its report-only run on the tested `UOSA.exe` |
| `results/uosa_source_names.md` | Tested `UOSA.exe`: versions, counts, the UOP decompression path, all names and addresses |

## What it recovers

Each name carries a tag `<LIB>_<METHOD>` (`ZLIB_STRING`, `FREETYPE_TABLE`,
`CRYPTOPP_VTABLE`, ...). At one address the strongest method wins, in the order
TABLE, VTABLE, STRING, CALLGRAPH; equal names from two methods are counted as agreements,
different names as conflicts.

- **TABLE**: statically initialized structs mixing strings and function pointers
  (FreeType `FT_Module_Class`/`FT_Driver_ClassRec` driver classes, service tables,
  `FT_Renderer_Class`, ...). The extractor computes each field's byte offset and size from
  the struct definitions (32-bit MSVC: 4-byte pointers, natural alignment, nested structs
  and arrays flattened) and classifies each initializer as string, function, pointer to
  another table, null or integer. A table is anchored by a data pointer to its unique
  string (`"truetype"`, `"bdf"`, ...) at the string's field offset; every other checked
  field (null, integer, pointer to code, pointer to a string) must agree. A table must
  pass at least 2 checks and only one variant may fit. Pointers to other tables anchor
  them in turn (`cff_driver_class` -> `cff_services` -> `cff_service_ps_info`), and tables
  without a string are also anchored by data pointers to a function already named.
- **VTABLE**: for every C++ class the extractor computes the MSVC primary vtable: base
  classes first, new virtuals appended in declaration order (overloads grouped at the
  first one, in reverse order, as MSVC does), an override replaces its base slot
  (signatures compared after typedef and template parameter substitution; a parameter
  spelled through a typedef matches when it is the only base virtual of that name, arity
  and constness), slot of the virtual destructor is the scalar (or vector) deleting
  destructor. Virtual inheritance is skipped. The script reads Ghidra's RTTI labels
  (`Class::vftable`), takes the primary one (complete object locator offset 0), and accepts
  a class only when the slot count equals the computed count and `_purecall` appears
  exactly at the pure slots. A slot target that also appears in vtables of classes outside
  the library (identical code folding) is not named after the library method.
- **STRING**: literals used by exactly one source function (by qualified name), at least 6
  characters and 4 letters outside format specifiers, string macros expanded. A literal
  is accepted when it is in the program, no data pointer and no code outside a function
  refers to it, and exactly one function refers to it. A function whose only evidence is
  a single plain word (`"Script"`, `"Anchor"`) is not named: such words also occur in UO
  code. Texts unique in two libraries are dropped.
- **CALLGRAPH** (C libraries): a named function whose source has exactly one callee not
  yet placed gets it from its program callees, when all its placed source callees are
  among its program callees and exactly one program callee is still `DEFAULT`-named.
  Callers are followed the same way, only from `static` functions (exported functions
  are also called by host code). Rounds repeat until nothing changes. Names from an
  earlier `--apply` (identified by the tag) seed the next run.

A function already named by another source (Gamebryo `GB26_*`, `UOLua::*`, FID, user) is
never renamed and is not used as a call graph anchor; the report lists it under
"Existing names kept".

### The map

`extract_source_names.py` is a tokenizer with a small preprocessor, not a compiler.
`#if` conditions become atoms; atoms fixed by the library profile (MSVC/Win32 on,
debug off, the library's configuration) are resolved, the others produce one variant
per combination for tables and vtables (at most 5 free atoms per record), and the script
keeps the variant matching the program. Literals and calls take both branches. Include
guards, K&R definitions, debug-only macros (`FT_TRACE*`, `CRYPTOPP_ASSERT`) and
structural macros (`FT_DEFINE_SERVICE`, `NAMESPACE_BEGIN`) are handled by the profiles
`zlib`, `freetype`, `tinyxml`, `cryptopp`; another library uses the generic profile and
may need its own.

Records (tab separated): `FUNC file name flags` (`i` inline or header, `s` static),
`CALL file caller callee`, `STRING file function text`, `TABLE name variant size type
conditions` + `FIELD name variant offset size kind value`, `VTABLE class variant count
conditions` + `SLOT class variant index owner::method flags signature` (`p` pure, `d`
deleting destructor).

## Fetching the sources

The sources are not part of the repository. Download each archive into its own empty
directory and only read it (no build scripts):

| Library | Archive |
|---|---|
| zlib 1.2.2 | `https://zlib.net/fossils/zlib-1.2.2.tar.gz` |
| FreeType 2.3.5 | `https://download.savannah.gnu.org/releases/freetype/freetype-old/freetype-2.3.5.tar.gz` |
| TinyXML 2.5.3 | `https://downloads.sourceforge.net/project/tinyxml/tinyxml/2.5.3/tinyxml_2_5_3.zip` |
| Crypto++ 5.4 | `https://codeload.github.com/weidai11/cryptopp/tar.gz/refs/tags/CRYPTOPP_5_4` (cryptopp.com was unreachable) |

Then, from a directory outside the sources:

```sh
python3 -I extract_source_names.py --lib zlib --version 1.2.2 --out maps/zlib-1.2.2.tsv /src/zlib-1.2.2
python3 -I extract_source_names.py --lib freetype --version 2.3.5 --out maps/freetype-2.3.5.tsv /src/freetype-2.3.5
python3 -I extract_source_names.py --lib tinyxml --version 2.5.3 --out maps/tinyxml-2.5.3.tsv /src/tinyxml
python3 -I extract_source_names.py --lib cryptopp --version 5.4 --out maps/cryptopp-5.4.tsv /src/cryptopp-CRYPTOPP_5_4
```

## Versions found in UOSA.exe

- **zlib 1.2.2**: `" inflate 1.2.2 Copyright 1995-2004 Mark Adler "` at `0x00c956a0` and
  `"1.2.2"` beside the `z_errmsg` table. No deflate copyright and no deflate-only
  literal: only inflate is linked (Crypto++ brings its own `Deflator`/`Inflator`).
- **FreeType 2.3.5**: `FT_Init_FreeType` (`0x00ab551a`) stores version 2, 3, 5. Literals
  confirm it: BDF `"Font header/glyphs corrupted or missing fields"` and `"tt-glyf"` are
  absent before 2.3.5; `"/..namedfork/rsrc"` (2.3.6, 2.3.7) and `"DFKaiShu"`, `"FSType"`,
  `"ForceBold"` (2.3.9) are absent from the program. The matched table variants show the
  build configuration: `FT_CONFIG_OPTION_OLD_INTERNALS`, `TT_CONFIG_OPTION_EMBEDDED_BITMAPS`,
  `TT_USE_BYTECODE_INTERPRETER`, `TT_CONFIG_OPTION_GX_VAR_SUPPORT`, PostScript names and
  the Adobe glyph list on. The `src/gzip` module brings zlib 1.1.4 (`"1.1.4"`, 1.1.4
  `inftrees` messages).
- **Crypto++ 5.4**: every Crypto++ literal of the program exists in 5.4; 5.3.0 lacks the
  IV messages (`": this object requires an IV"`), 5.5 and later lack
  `"RandomPool: nonblocking transfer is not implemented by this object"` and
  `"CipherModeBase: GetNextIV() must be called on an encryption object"`.
- **TinyXML 2.5.x, built with `TIXML_USE_STL`**: `TiXmlPrinter`/`TiXmlVisitor` (2.5+),
  `"Memory allocation failed."` absent in 2.6, node vtables of 18 slots (19 for
  `TiXmlDeclaration`), the `TIXML_USE_STL` layout. 2.5.2 and 2.5.3 differ only in
  `sscanf` vs `sscanf_s` (both are linked by the CRT here), so 2.5.3 is assumed.

## Running

In the GUI, add this folder to the Script Manager's directories and run
`NameSourceFunctions.java` (category `ECResearch`) with arguments
`OUTPUT_FILE MAP.tsv [MAP.tsv...] [--apply] [--range 0xSTART-0xEND]`. Arguments are split on
whitespace, so write spaces in paths as `%20`. `--range` limits STRING and CALLGRAPH
targets.

ghidra-mcp copies the script into `~/ghidra_scripts`, so give the maps as absolute paths:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-source-names/NameSourceFunctions.java",
  args="/tmp/uosa-source-names.md /absolute/repo/.../maps/zlib-1.2.2.tsv /absolute/repo/.../maps/freetype-2.3.5.tsv /absolute/repo/.../maps/tinyxml-2.5.3.tsv /absolute/repo/.../maps/cryptopp-5.4.tsv")
```

Headless, on an existing analyzed project:

```sh
/path/to/ghidra/support/analyzeHeadless /path/to/project-dir PROJECT \
  -process UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-source-names \
  -postScript NameSourceFunctions.java /tmp/uosa-source-names.md maps/zlib-1.2.2.tsv ... --apply
```

The output file is created exclusively before anything else, so an existing file stops
the run unchanged. Without `--apply` the program is not changed. Run all maps together:
texts unique in two libraries are then recognized, and conflicts across libraries show.

## What `--apply` changes

- Functions are created at table and vtable entries that point at code with no function
  (before string matching, so literals in them count).
- Names (source `ANALYSIS`) only over `DEFAULT` (`FUN_...`) names. C++ methods go into
  their class namespace, reusing the RTTI class namespace when Ghidra has one; a name
  taken in its namespace gets an `_<address>` suffix (listed as an anomaly).
- One tag per function (`<LIB>_<METHOD>` of the winning method).
- A plate comment `NameSourceFunctions: <TAG> <library version>: <evidence>` on functions
  without a plate comment.

Nothing is deleted or retyped. A second `--apply` changes nothing.

## Limitations

- Only what the binary still shows by structure is named: inlined and folded functions
  have no address of their own, functions without strings, tables or vtables are reached
  only through the call graph, and the call graph needs a unique open neighbour. zlib gets
  just `inflate` this way (its callees are 6 open functions at once).
- Variant enumeration is bounded (5 free atoms); larger tables are skipped and listed by
  the extractor on stderr.
- Vtables of classes with virtual bases are not computed; a class whose computed count
  differs from the program (`CryptoPP::ByteQueue::Walker`: Ghidra's vtable ends after 7
  of 44 slots; `CryptoPP::DH2`: slot 5 is implemented although the source declares no
  override) is rejected as a whole.
- Folded (ICF) slot targets carry the name of the first library class only.
- Template arguments in method owners are shown as written in the base clause
  (`CryptoPP::AutoSignaling<Filter>`).
- Ghidra's Function ID with databases built from the same library versions compiled by
  VS2005 (`/O2`, `/GL`) would match many more small functions by body hash, and BSim would
  match by similarity. Both need the libraries compiled with the original compiler and
  options (MSVC 8 was not available; clang or zig code generation would differ), so they
  were not tried. This tool needs only the sources and complements them.

## Tested result

Ghidra 12.1.4, `UOSA.exe` SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`, all four maps:
50 of 105 FreeType tables found, 49 vtables matched (11 TinyXML, 38 Crypto++), 2 rejected.
`--apply` named 683 functions (84 created, 15 namespaces): zlib 1 (STRING), FreeType 289
(196 TABLE, 10 STRING, 83 CALLGRAPH), TinyXML 84 (VTABLE), Crypto++ 309 (294 VTABLE,
15 STRING); 7 method agreements, no conflicts, no existing names in the way. The second
`--apply` changed nothing. Details, the UOP decompression path and every name with its
address are in `results/uosa_source_names.md`.

## UOP trace names (`NameUopTrace.java`)

`NameUopTrace.java` names the code that reads and decompresses UOP entries, traced by hand
from `UOVirtualFileOpen` (the chain is in `results/uosa_source_names.md`): 15 functions in
namespace `UOP` with tag `UOP_TRACE` (`OpenFilePtr`, `MythMypInputFile_ctor`,
`MypArchive_ReadEntry`, `MypArchive_GetEntry`, `Decompress`, the async
`FileLoadJobRequest_Execute`/`DecompressJobRequest_Execute`, ...) and the zlib 1.2.2
functions they reach, `uncompress`, `inflateInit_`, `inflateReset`, `inflateEnd`,
`zcalloc`, with tag `ZLIB_UOP_TRACE`.

UO code has no source to derive evidence from, so the names are tied to addresses: the
script refuses to run when the program's SHA-256 is not that of the tested `UOSA.exe`.
Before naming, each address passes stated checks: the `MypArchive`/`FileLoadJobRequest`
vftable slot holding it, the calls it makes or receives (`Decompress` -> `uncompress` ->
`inflate`), the `"Decompress"` allocation tag string, the `MythMypInputFile`/
`MythMemoryInputFile` vftable it stores, the constants `0x22` (entry record size) and
`0x1ba8` (`inflate_state` size). A failed check is reported as a mismatch and the address
is left alone. Arguments `OUTPUT_FILE [--apply]`; the output is created exclusively,
without `--apply` nothing changes, `--apply` renames only `DEFAULT` names, adds the tag
and a plate comment only where there is none, and reports kept and conflicting names.

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-source-names/NameUopTrace.java",
  args="/tmp/uosa-uop-trace.md")
```

Tested: all 20 addresses pass their checks; the report-only run after the names were
applied shows every entry as already applied (`results/uosa_uop_trace.md`).
