# Lua function names for Ghidra

`UOSA.exe` embeds LuaPlus (build 1100, on Lua 5.1) and exposes a few hundred C functions to
the UI scripts. `NameLuaFunctions.java` names the UO Lua API functions from their
registration calls, the Lua 5.1 library functions from their `luaL_Reg` tables, and other
Lua/LuaPlus functions from string literals unique to one source function, and can
annotate the program with them.

| Path | Content |
|---|---|
| `NameLuaFunctions.java` | Ghidra script: report, and with `--apply` annotations |
| `extract_lua_sources.py` | Generates `lua51_sources.tsv` from the LuaPlus sources |
| `lua51_sources.tsv` | Source map: functions, `luaL_Reg` tables, `luaL_register` calls, `linit.c` libraries, unique literals |
| `results/uosa_lua_names.md` | Report for the tested `UOSA.exe` (all names and addresses) |
| `CrossReferenceUi.java` | Ghidra script: UI scripts and XML against `UOSA.exe`, both directions (see below) |
| `extract_ui_xref.py` | Generates the UI index (API call sites, handler registrations, Lua definitions, `SystemData`/`WindowData` paths, XML schema) from the unpacked `UserInterface/Default` folder; the index is generated, not shipped |
| `results/uosa_ui_xref.md` | Cross-reference summary for the tested `UOSA.exe` (report-only run after `--apply`, without `--details`) |

## What it recovers

Each name carries the tag of the method that produced it.

- **`LUA_API`, UO Lua API**: the registration helper is found generically. A byte scan
  collects every `CALL` preceded within a short window by a `PUSH` of an identifier string
  and a `PUSH` of a code address; the callee with most such calls is the helper
  (`UOLua::RegisterFunction(obj, func, name)`, inferred). The name and function argument
  positions are voted over all sites, then each site's pushes are decoded backwards (from
  the listing, or with the pseudo-disassembler where Ghidra did not disassemble). Pushes
  of `[reg+k]` with `reg` loaded from a constant are read as `{func, name}` tables walked
  by a loop. Functions are named `UOLua::<Name>`; a dotted name such as
  `Settings.LoadChatSettings` becomes `UOLua::Settings::LoadChatSettings`. One function
  under several names takes the first registration (by address) and lists the others in
  its plate comment.
- **Other registration helpers**: callees with the same push pattern are listed. Those
  inside the Lua code range where at least half the sites pass a name and a code pointer
  are LuaPlus global registrations (`LOG`, `_ALERT`, `LuaDumpGlobals`, ...), named in
  namespace `LuaPlus`. Their dispatcher sites (the real function stored to a stack slot
  before the call) are resolved.
- **`LUA_REG`, `luaL_Reg` tables**: `{ name, func }` arrays in initialized data, at least two
  entries of identifier plus code pointer, `{0, 0}` terminated. The `luaL_register` call
  using each table (`PUSH table; PUSH libname; CALL`) gives the library name and
  identifies `luaL_register` itself. Each source table is matched to the program table
  with the best Jaccard score over entry names (at least 0.5). Entries take the C function
  name from the source; an extra entry takes its own name when a function of that name
  exists in the same source file, else `<libname>::<entry>`. Tables not best-matched (the
  LuaPlus `wstring` library, a second base table) and tables without a source
  (`string.pack`/`unpack`) use `<libname>::<entry>`. The function registering a table is
  named from the source `luaL_register` call (`luaopen_os`, `base_open`, `createmeta`, ...).
- **Unrolled `luaL_openlibs`**: `PUSH luaopen_x; ...; PUSH "x"` pairs clustered by distance,
  with library names from `linit.c` plus the `luaL_register` library names; `luaopen_<x>`
  for a library missing from `linit.c` (`wstring`) follows the naming convention.
- **`LUA_STRING`, unique literals**: string literals used by exactly one source function
  (by name, across the Lua 5.1 and LuaPlus sources), at least 6 characters and 4 letters
  outside format specifiers. A literal is accepted when it is in the program, no data
  pointer and no code outside a function refers to it, and exactly one function refers to
  it, inside the target range (the `LUA_REG` functions widened by `0x20000`, or `--range`).
  A function matching several source names, or a non-overloaded source name matching
  several functions, is a conflict and stays unnamed.
- **UI validation** (`--ui`): the registered names are compared with the global names
  called and defined in the UI Lua scripts.

`LUA_REG` wins over `LUA_API` and `LUA_STRING`; `LUA_API` wins over `LUA_STRING`. Different
names for one function are reported as conflicts, equal names as agreements.

## Regenerating the source map

```sh
python3 -I extract_lua_sources.py /path/to/luaplus
```

The argument is a LuaPlus checkout (the folder with `Src/LuaPlus`); the output defaults
to `lua51_sources.tsv` next to the script. It parses `Src/LuaPlus/lua51-luaplus/src`
and `Src/LuaPlus/lua51/src` (without the standalone `lua.c`, `luac.c`, `print.c`) and the
LuaPlus C++ files in `Src/LuaPlus` (qualified names prefixed with `LuaPlus::`). The parser
is a tokenizer, not a preprocessor: both branches of `#if` blocks are kept, string macros
and `LUA_QL` are expanded and `luaplus_assert(e)` is stringified as MSVC does. The tested
map was generated from the 2021 LuaPlus snapshot (MIT), newer than the build in UOSA;
the program's tables show where they differ (`towstring`, `gmatch_aux`, `wstring`).

## Running

In the GUI, put this folder in the Script Manager's script directories and run
`NameLuaFunctions.java` (category `ECResearch`) with arguments
`OUTPUT_FILE [--apply] [--sources TSV] [--ui DIR] [--range 0xSTART-0xEND]`. The source map
defaults to `lua51_sources.tsv` next to the script. Arguments are split on whitespace, so
write spaces in paths as `%20`.

ghidra-mcp copies the script into `~/ghidra_scripts`, so pass `--sources` explicitly:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-lua-names/NameLuaFunctions.java",
  args="/tmp/uosa-lua-names.md --sources /absolute/repo/scripts/dev-helpers/ec-research/ghidra-lua-names/lua51_sources.tsv --ui /path/to/Ultima%20Online%20Enhanced/UserInterface/Default")
```

Headless, on an existing analyzed project:

```sh
/path/to/ghidra/support/analyzeHeadless /path/to/project-dir PROJECT \
  -process UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research/ghidra-lua-names \
  -postScript NameLuaFunctions.java /tmp/uosa-lua-names.md
```

The output is created exclusively, before anything else, so an existing file stops the
run unchanged. Add `--apply` to annotate; headless runs save the program unless
`-readOnly` is given. Without `--apply` the program is not changed.

## What `--apply` changes

- Functions are created at registered entry points that are not functions yet, before
  string matching, so literals referenced from them count as referenced by a function.
- Function names (source `ANALYSIS`) only for functions with a `DEFAULT` (`FUN_...`) name;
  names from other sources are kept and listed. Namespaces (`UOLua`, `UOLua::Settings`,
  `LuaPlus`, `wstring`, ...) are created or reused. A name already used in the namespace
  gets an `_<address>` suffix.
- Tags `LUA_API`, `LUA_REG`, `LUA_STRING` (the winning method only).
- A plate comment `NameLuaFunctions: <TAG> <name>: <evidence>` on functions with no plate
  comment.

Nothing is deleted or retyped. A second `--apply` changes nothing.

## Limitations

- `UOLua::RegisterFunction` and the LuaPlus helper names are inferred; the target table of
  a UO API registration (global or a table) is not recovered, except for dotted names.
- Three LuaPlus helper sites pass the name and function in registers (wrappers) and stay
  unresolved; registrations done in other ways (Lua scripts, `lua_setfield` sequences)
  are not covered.
- `LUA_STRING` depends on the function boundaries Ghidra has: a report without `--apply`
  on a program missing the registered functions accepts fewer literals than the
  `--apply` run. A unique literal can belong to an inlined callee, so these names are the
  weakest evidence; the plate comment lists the strings.
- The `wstring` library and `string.pack`/`unpack` have no source in the snapshot and are
  named after their library names.

## Tested result

Ghidra 12.1.4, `UOSA.exe` SHA-256
`5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`: registration helper
`0x0098fb25`, 560 UO API registrations (559 direct calls and one `{func, name}` table
loop) of 536 distinct names on 536 functions (81 created), no name on two functions and no
function under two names; 8 LuaPlus globals through `0x00a7c020`; 14 `luaL_Reg` tables, 13
matched to a source table (9 with score 1.00); `luaL_register` `0x00a9cae0` and the
unrolled `luaL_openlibs` at `0x00a862e7` (9 libraries). `--apply` named 766 functions: 545
`LUA_API` (with the helper and the LuaPlus globals), 162 `LUA_REG`, 59 `LUA_STRING` (124 of
374 literals accepted, 22 more agreeing with `LUA_REG`), no conflicts and no non-default
names in the way. Of the registered names, 343 appear in the 114 UI scripts of
`UserInterface/Default`; the UI calls 7 global names that are neither registered nor
defined there (`WindowGetShowing`, `WindowGetDimensions`, `LabelGetText*` are registered
as `_WindowGetShowing`, ...; `ERROR`, `GetFormatStringFromTable`). Texture-related examples:
`UOLua::DynamicImageSetTexture` `0x008f4dc3`, `UOLua::RequestTileArt` `0x0051b650`,
`UOLua::UOGetTextureSize` `0x00528370`.

# UI cross-reference

`CrossReferenceUi.java` runs after `NameLuaFunctions.java --apply` and matches the
Enhanced Client UI against the program in both directions.

## Regenerating the UI index

```sh
python3 -I extract_ui_xref.py "/path/to/Ultima Online Enhanced/UserInterface/Default" /tmp/ui_xref.tsv
```

The index is derived from the UI files (about 1.4 MB for the Default UI) and is not part of
the repository: write it outside the repo and pass it with `--xref`. The scripts are tokenized (not
parsed): `L"..."` is a wide string literal, calls through the Lua standard library and
through names the UI defines itself are left out, and for each remaining call it records
the argument count (`+` when the last argument is a call or `...`), how many values the
caller assigns (`*` when the results are forwarded: `return F()` or a last argument) and
the enclosing function. `WindowRegisterEventHandler`/`WindowRegisterCoreEventHandler`
calls give handler registrations; `SystemData.*`/`WindowData.*` chains (up to 4
components, indexes as `[]`) are counted as reads and writes. The XML files (`.xml`, `.mod`)
give elements, parent/child pairs, attributes, `EventHandler` event/function values and
named windows. Only identifiers, counts and line numbers are stored.

## Running

Arguments: `OUTPUT_FILE [--apply] [--details] [--xref TSV]`. Always pass `--xref` (the default,
`ui_xref.tsv` next to the script, is not shipped, and ghidra-mcp runs a copy of the script
from `~/ghidra_scripts`). `results/uosa_ui_xref.md` is exactly the output of:

```text
run_ghidra_script(program="UOSA.exe",
  script_name="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-lua-names/CrossReferenceUi.java",
  args="/absolute/repo/scripts/dev-helpers/ec-research/ghidra-lua-names/results/uosa_ui_xref.md --xref /tmp/ui_xref.tsv")
```

run after `--apply` (the labels show the applied names). `--details` adds the per-call-site
index (file, line, enclosing function, args -> results for every UI call), the per-string
lists (paths used by the UI, engine-only `SystemData`/`WindowData` strings, UI globals and
windows) and every element dispatch site; without it those sections give only their size.
The output is created exclusively before anything else. Without `--apply` nothing changes.

## What it reports

1. **Lua -> C**: for every `LUA_API` function (Lua names taken from the NameLuaFunctions
   plate comment, including "also registered as"), a summary line (calls, files, argument
   and result counts, one example), and with `--details` every call site with file, line,
   enclosing function, argument and result counts. Names the UI calls
   that are not registered are listed with the strings found (some are registered as
   `_Name` and wrapped by Lua code outside `UserInterface/Default`).
2. **C -> Lua**: exact NUL-terminated strings equal to UI names (`SystemData`/`WindowData`
   paths, event names, `EventHandler` functions, Lua globals of 5+ characters, window
   names), their code references (raw imm32 scan of executable blocks) and the call that
   takes each pushed string (`FUN_00994480` resolves dotted Lua paths, `FUN_008d67ac`
   creates a window from a template, TinyXML `FirstChildElement(name)` `FUN_00b666e2` reads
   settings files with the same names). Engine `SystemData`/`WindowData` strings the UI never uses are listed.
   UI tables are matched to C functions by field names: a function referencing at least 3
   and 25% of the fields the UI reads from a table (for example `WindowData.PlayerStatus`
   -> `0x00577be0`).
3. **XML schema -> parsers**: tag compares `PUSH "Tag"; ...; CALL compare; ...; Jcc` are
   decoded; the element handler is the first call in either branch whose function (or its
   direct callees) references at least 2 of the element's attribute/child names, one of
   them directly. Functions reading a tag and 2+ of its names without a dispatch (for
   example the `ManagedTextures.xml` reader) are listed as direct parsers. Attributes
   whose name is not a string in the program are listed (never read by name).
4. **Texture chain**: a fixed table of the UI texture path (XML `<Texture>`, `<DynamicImage
   texture=>`, `RequestTexture`, `RequestTileArt`, `RequestGumpArt`, down to
   `NiSourceTexture::Create`), each step re-verified by calls, strings, vftable references
   or vftable slots.

## What `--apply` changes

- A second plate comment line `UI: <n> calls in <m> files, args <a>, results <r>; e.g.
  <file>:<line>` on `LUA_API` functions called by the UI, only when the plate comment is
  exactly the NameLuaFunctions line (or that line plus an earlier `UI:` line, which is
  updated or removed). Other comments are left alone and listed.
- Tag `UI_HOOK` on functions referencing at least 2 UI names, one of them a path, event,
  window or matched field table.
- Tag `UI_XML` on element handlers, direct parsers and functions dispatching 2+ elements.
- Name `UOXml::Parse<Element>` (source `ANALYSIS`) for a handler serving one element that
  has one handler, or a unique direct parser, only over `DEFAULT` names.
- Functions tagged by the Gamebryo and library scripts (`GB26_`, `ZLIB_`, `CRYPTOPP_`,
  `TINYXML_`, `FREETYPE_`) are never tagged or renamed.

A second `--apply` changes nothing.

## Limitations

- String matching is exact: names built at run time, wide strings and substrings
  (`_WindowGetShowing` for `WindowGetShowing`) are not found; strings reached through
  data tables rather than imm32 operands have no code reference.
- The UI index covers only `UserInterface/Default`; the `InterfaceCore` Lua inside the
  packages is not unpacked.
- Elements the Default UI never uses (`HyperLinkWindow`, `FixedFontLabel`, `PageWindow`,
  `MapDisplay`, `CooldownDisplay`, `RenderedImage`, `NifDisplay`, `ActionButtonGroup`,
  `Font`, `MapIcon`, ...) have no attribute list, so their handlers stay unnamed even
  where the dispatch is visible.
- Elements sharing a handler (`<TexCoords>`, `<AbsPoint>` and the other offset/size
  elements at `0x0095672b`; the color elements at `0x00955d2d`) are reported, not named.

## Tested result (UI)

Same program, 114 Lua and 105 XML files: 12592 calls to 392 non-UI names; 344 of the 545
`LUA_API` functions are called by the UI (11784 call sites); 48 called names are not
registered. 211 UI `SystemData`/`WindowData` paths are strings in the program, 116 engine
paths are never used by the UI, 29 event names, 107 UI globals/windows, 39 UI tables
matched by field names, 80 `UI_HOOK` functions. 82 XML elements and 365 element/attribute
pairs; 252 tag compares through `FUN_0099e8b9` in 40 functions, 40 handlers, 38
`UOXml::Parse<Element>` names. `--apply`: 344 `UI:` comment lines, 80 `UI_HOOK` and 50
`UI_XML` tags, 38 renames; the second `--apply` changed nothing. `typeid` on `<Texture>`
(365 uses) is not a string in the program: the client ignores it. All 27 texture chain
steps verified.
