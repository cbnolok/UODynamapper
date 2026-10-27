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
