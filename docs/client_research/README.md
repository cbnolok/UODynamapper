# Original-client research

What was learned by reverse engineering the original clients: the Enhanced Client
(`UOSA.exe`), Kingdom Reborn (`UOKR.exe`) and the Classic `client.exe` builds, mostly
in Ghidra. Each `ghidra-<topic>/` folder holds one study:

- `README.md`: what was traced and how to reproduce it;
- `results/`: the findings, with the hashes of the executables they hold for;
- the Ghidra scripts (and their specs and inputs) that put the recovered names,
  prototypes and types on a fresh import of those executables. The annotated Ghidra
  project itself is not shared, so these scripts are the way to rebuild it;
- reference decoders and census scripts that check the findings against the shipped
  data.

[`tools/`](tools/) holds the helpers shared by every study (PE inspection, bounded
decompilation, `ApplyVariableSpecs.java`, the read-only Ghidra query), described
below. [`water/`](water/README.md) holds the EC water and terrain-material
investigation: the original material, item and scene inspectors and the coverage
and mask-edge checks. [`kr-facet-statics/`](kr-facet-statics/README.md) compares
Kingdom Reborn's facet statics and land with the Classic and Enhanced ones: the
cave arches only KR's facets hold (the Minoc mine's) and how far KR's world differs.

These tools inspect local inputs without executing or patching the original
client. Keep generated dumps, extracted bytes and Ghidra projects outside the
repository, for example under `/tmp`. Record conclusions and executable hashes
in the study's `results/` or in `docs/agent_notes/`; do not commit client binaries
or extracted proprietary data.

Addresses hold for one build only, so wherever a study or script quotes a hard
address, name the client version next to the executable: `UOSA.exe 4.0.102.3`,
`UOKR.exe 2.58.0.6`. Take the version from the executable's PE version resource
(`exiftool`, or Ghidra's Program Information option `PE Property[FileVersion]`),
not from folder files such as `buildinfo.txt`: the `UO - Old KR` folder's one says
2.53.0.2, but its `UOKR.exe` is 2.58.0.6. When an executable has no version
resource (the UO Gold `client.exe`), say so and give its PE timestamp. The scripts'
report headers print the version of the program they ran on.

## PE strings, references and raw data

`pe_inspect.py` uses Python's standard library and handles PE32/PE32+ section
mappings. It reports the executable SHA-256, section layout, matching ASCII
strings and candidate absolute-pointer references in executable sections:

```sh
python3 docs/client_research/tools/pe_inspect.py /path/to/UOSA.exe \
  --string UOWaterShader --string UOWaterTerrainLayer > /tmp/ec-water-references.json
python3 docs/client_research/tools/pe_inspect.py /path/to/UOSA.exe \
  --extract-va 0xc981c8 --size 4 --output /tmp/ec-constant.bin
python3 -B -m unittest discover -s docs/client_research/tools -p 'test_*.py'
```

Candidate references are addresses of matching bytes, not established instruction
or function boundaries. This deliberately simple search does not find relative,
indirect or encoded references. Virtual zero-fill has no file-backed bytes.
Disassemble from an established function entry, never from a guessed byte offset:

```sh
objdump -d -Mintel --start-address=0x59bbd0 --stop-address=0x59d004 \
  /path/to/UOSA.exe > /tmp/ec-water.asm
```

## Bounded Ghidra decompilation

`DecompileFunctions.java` accepts an output path followed by known function entry
VAs, interpreted as hexadecimal. It creates missing functions only at those
entries and decompiles each with a 30-second timeout. Use an absolute script path:

```sh
/path/to/ghidra/support/analyzeHeadless /tmp/ec-ghidra water \
  -import /path/to/UOSA.exe -noanalysis \
  -scriptPath /absolute/repo/docs/client_research/tools \
  -postScript DecompileFunctions.java /tmp/ec-functions.c 692760 59bbd0
```

Use a fresh project directory for import; use `-process UOSA.exe` for an existing
project. Outputs are created exclusively to avoid overwriting earlier evidence.
Ghidra writes its normal user cache/configuration as well as the project. This
headless command opens no GUI and needs no desktop placement.

`-noanalysis` limits work but also leaves callees and their calling conventions
unknown. Decompiled register arguments, stack values and pruned jump-table
branches can therefore be wrong. Check important results against the assembly;
run normal Ghidra analysis or define callee signatures when needed. A decompiler
listing is a navigation aid, not independent proof of shader semantics.

For Direct3D 9 / D3DX9 signatures and COM interface types, apply the archive in
[`ghidra-d3d9-types`](ghidra-d3d9-types/README.md).

To recover Gamebryo class names, inheritance, vtables, stream loaders and object
sizes from NiRTTI records, run the script in
[`ghidra-gamebryo-classes`](ghidra-gamebryo-classes/README.md). Then fill the class
structures with Gamebryo 2.6 member layouts where the sizes match, using
[`ghidra-gamebryo-layouts`](ghidra-gamebryo-layouts/README.md), and name functions
from Gamebryo 2.6 by vtable slot and BSim similarity with
[`ghidra-gamebryo-names`](ghidra-gamebryo-names/README.md).

To name UO's Lua API bindings, the Lua 5.1 library and core functions and LuaPlus
helpers, run the script in [`ghidra-lua-names`](ghidra-lua-names/README.md).

The statically linked zlib, FreeType, TinyXML and Crypto++ functions are named from
their sources, and the UOP read/decompress chain is traced, in
[`ghidra-source-names`](ghidra-source-names/README.md).

The login flow, packet table, login/game ciphers and the patch sites of the ECLO loader are named with
[`ghidra-login-names`](ghidra-login-names/README.md). The recovered names and executable evidence live with the
[client patcher documentation](../../tools/uo-client-patcher-gui/docs/README.md).

The `tileart.uop` load path, its record format as the client reads it and the code
using tile definitions are named and documented in [`ghidra-tileart`](ghidra-tileart/README.md).
What the Classic Client (7.0.102.3) reads from its own copy of `tileart.uop` (the same version 4 records as the
Enhanced Client's: only animation appearance, body type, the alternate animation, the paperdoll appearance and
sitting; never art ids, offsets, flags or heights, which come from `tiledata.mul`) is traced in
[`ghidra-cc-tileart`](ghidra-cc-tileart/README.md).
The effect definitions at the end of tileart records, which share their format with
`EffectDefinitionCollection.uop`, are decoded in [`ghidra-effects`](ghidra-effects/README.md). The
Chaos particle engine that plays `.ems` effects is documented in
[`ghidra-effects/results/uosa_chaos.md`](ghidra-effects/results/uosa_chaos.md).
`AnimationSequence.uop`, `AnimationDefinition.uop` and the 0x6E/0xE2 animation packets are traced in [`ghidra-animation`](ghidra-animation/README.md).
The `AnimationFrame*.uop` (AMOU) code of UOSA.exe and UOKR.exe (frame entry search, palette and hue key, RLE decoders, atlas) and the rules a frame encoder must follow are in [`ghidra-animation-frames`](ghidra-animation-frames/README.md).
How UOSA.exe loads a mobile animation at run time (synchronous main-thread UOP reads and inflates, the asset and frame composite caches and their limits, per-frame D3D9 texture creation) and what makes it stutter are traced in [`ghidra-ec-anim-loading`](ghidra-ec-anim-loading/README.md).
`build/multicollection/housing.bin` (house customization pieces) and the `MultiCollection.uop` multis are decoded in [`ghidra-housing`](ghidra-housing/README.md).
UOP data block headers (version-4 timestamps, version-5 signatures) and `data_block_hash` are decoded in [`ghidra-uop-header`](ghidra-uop-header/README.md).
How the client picks the decoder for each image it loads from a UOP (the extension of the requested path, the `assetmap.xml` suffixes of tile textures, the hue bitmap parser) and what each decoder checks are traced in [`ghidra-image-decoders`](ghidra-image-decoders/README.md).
How tileart texture items reach the shaders (item position to shader map index, per shader class), and the check that the client never infers a texture's role from its file name, are traced in [`ghidra-texture-roles`](ghidra-texture-roles/README.md).
How the client sizes and places classic (TileArtLegacy) art next to EC world art (descriptor modes, pixel unit, Z step, legacy window anchors) and how TileArtLegacy images compare with Classic art are traced in [`ghidra-legacy-art-scale`](ghidra-legacy-art-scale/README.md).
Why EC's tileart cuts 215 creature statuettes' world art (an enhanced window the EC never reads, copied from the legacy window, where KR's own record fits the figure), which window each client reads, and the proposed fix are in [`world-art-windows`](world-art-windows/README.md).
How TerrainDefinition.uop layers bind to the terrain texture stages (per layer class: item position to stage and role), what the stretch value does, whether any role, primary layer or "smooth" flag comes from data or file names, and what a missing definition does are traced in [`ghidra-terrain-layers`](ghidra-terrain-layers/README.md).
Facet sectors (`facetN.uop`, the unread `data.map` entry, the `facetNx.uop` variants), `facets.uop`
FacetDefinition records and the `facetNN.dds` radar images are decoded in [`ghidra-facets`](ghidra-facets/README.md).
The same document covers the facet size table, with a cross-build recipe for patching map sizes, and the
UOKR.exe (Kingdom Reborn) sector format and format deltas (`results/uokr_facets.md`).
`TerrainDefinition.uop` (the field table, flag bits, floats and an encoder recipe) is decoded in
[`ghidra-terrain`](ghidra-terrain/README.md).
Audio is traced in [`ghidra-audio`](ghidra-audio/README.md): how the clients drive Miles and DirectSound, with the SDK's names carried to the shipped Miles binaries, the EC sound and music tables and play path (which file plays for a sound id, what a missing entry does), the audio UOP names, what Miles 7.2c (`mss32.dll`, `mssmp3.asi`) accepts as WAV and MP3, the KR `audio_*.csv` grammar, the Classic `Sound.def` parser, sound header and playback rate, and the `CONFIG.TXT` loop flag.
`Paperdoll.uop` is traced in [`ghidra-paperdoll`](ghidra-paperdoll/README.md): its two entries are the male and female paperdoll image sets in the AMOU format, loaded as `PaperdollFrameSet` assets 0x40 and 0x41 and searched by gender and body or item id.
How Kingdom Reborn darkens and tints the scene (the light-level and night-map composite, gamma, sprite shadows, the baked half-Lambert terrain shade, and the absence of sun, ambient, fog, tonemap, bloom and grading), with an item-by-item comparison against Dynamapper's KR-like preset, is traced in [`ghidra-kr-atmosphere`](ghidra-kr-atmosphere/README.md).
WGSL translations of the EC and KR post-process and sprite shaders, from the `Shaders.uop` dump (bloom pipeline, sprite base and hue, the EC death effect), with a reference note each, are in [`client-shader-translations`](client-shader-translations/ec/ec_shaders_reference.md) (`ec/` and `kr/`); none is compiled into Dynamapper.
The Classic `client.exe` facet table is decoded in [`ghidra-classic-facets`](ghidra-classic-facets/README.md),
for UO Gold, ML, 5.0.9.1, 6.0.14.3, 7.0.20, 7.0.50 and 7.0.103. It covers the wrap rectangle, file ids,
the Felucca-rules flag, the `map0x` table copy selected by 0xA9 flag 0x8000, and the mapdif/stadif patches
and the client versions that still apply them.
The Classic custom-house tables, `suppinfo.txt`, `prof.txt`, `keynames.txt`, `palette.mul`, `animinfo.mul` and
`sjis2uni.mul` readers, and the files the client never opens (`Arttable.cfg`, `Tile256/1024.txt`, `gamegen.txt`),
are traced in [`ghidra-classic-text-tables`](ghidra-classic-text-tables/README.md).

### Variable names, prototypes and structures (ApplyVariableSpecs.java)

After the four `Name*Code.java` scripts have named and tagged their functions, `ApplyVariableSpecs.java`
gives those functions prototypes, parameter / local / global names and structure types. Each area has a
spec file next to its script:

| Area | Spec | Tag |
|---|---|---|
| tileart | `ghidra-tileart/tileart_variables.spec` | `UO_TILEART` |
| effects | `ghidra-effects/effects_variables.spec` | `UO_EFFECTS` |
| animation | `ghidra-animation/animation_variables.spec` | `UO_ANIMATION` |
| animation frames | `ghidra-animation-frames/animation_frame_variables.spec` | `UO_ANIMATION_FRAME` |
| housing | `ghidra-housing/housing_variables.spec` | `UO_HOUSING` |

Run the tileart spec first: it creates the shared `/UO` types (`SharedPtr`, `AssetBlob`,
`AssetBlobSharedPtr`, `AssetKey`, `UOObject`, ...) that the other three use. The effects and animation specs
also use the `/UOEffects` MSVC container types created by `NameEffectCode.java`.

```
ApplyVariableSpecs.java SPEC_FILE OUTPUT_FILE [--apply]
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/ApplyVariableSpecs.java",
                  args="<abs path>/ghidra-tileart/tileart_variables.spec <scratch>/tileart_vars.md --apply",
                  timeout_seconds=900)
```

- `OUTPUT_FILE` is created with CREATE_NEW; an existing file stops the run before any change.
- Without `--apply` it only reports (`would set: ...`). A second `--apply` reports `Changes: none`.
- Functions are found by full name and must carry the spec's tag. Variables are found by role, never by
  address: parameters by ordinal (or the storage given in the spec), locals by stack offset, by the register
  defined at a function-relative offset or by the result of the n-th call to a named function, globals by
  the single data reference of the instruction at a function-relative offset.
- Names and types are written with source USER_DEFINED. A global whose current name was not set by
  analysis (a default or ANALYSIS label) is kept and listed under "Kept names".
- The report counts functions, prototypes, parameters, locals, globals and types, and lists anomalies
  (unknown types or namespaces, a RET purge that disagrees with the spec's stack parameters, a register
  parameter that is not read before it is written, a missing local or data reference).

Spec syntax (`#` starts a comment):

```
tag UO_EXAMPLE
type /UO/Name SIZE : description          # create or extend a structure; fields below, then `end`
  OFFSET TYPE NAME [: comment]            # TYPE: a builtin (uint, ushort, short, byte, char, bool, float,
end                                       # qword, void), a unique type name or a /Path; `T*`, `T[N]`
class NAMESPACE /UO/Name                  # turn NAMESPACE into a class whose `this` is /UO/Name*
function Namespace::Name
  proto CONVENTION RETURN_TYPE [custom]   # custom = every parameter has an explicit @storage
  param NAME|- TYPE [@REG|@stack:0xN]     # `-` keeps the default name; ordinal order, `this` excluded
  local stack:-0xN|reg:REG@+0xOFF|call:Callee[#N] NAME [TYPE]
  global ref:+0xOFF NAME [TYPE]
end
```

Instruction-level rules the specs follow (MSVC x86 with link-time code generation):

- The stack purge of every RET must match the parameters of a callee-pops convention (`__stdcall`,
  `__thiscall`); `__cdecl` must not purge.
- A register parameter must be read before it is written. A `PUSH reg` counts as a read only when the
  matching `POP` does not follow (a save / restore pair is not an input), and callee-saved registers (EBX,
  ESI, EDI, EBP) survive calls. The `MOV EAX,handler; CALL __EH_prolog` prolog is stepped over; the helper
  names `__EH_prolog` and sets its `EH_prolog` call fixup so `[EBP+8]` is the first stack argument.
- "other register inputs [...]" in a function row is a note, not an anomaly: it lists registers the
  decompiler sees as inputs that the prototype does not cover. Each was checked by hand: they are slot
  reservations (`PUSH ECX` in place of `SUB ESP,4`), registers passed through to a callee, or values the
  callers never set.
- Classes without a recovered structure take their object as an explicit `ECX` (or other register)
  parameter with custom storage, so that Ghidra does not create an empty class structure.
- A boost/MSVC `shared_ptr` passed by value is two stack slots: the object and its `_refcount` block
  (`/UO/SharedPtr` when it stays together).
- A parameter whose role is not established keeps its default name (`-`).

For an already analyzed program in the configured ghidra-mcp GUI project, prefer
the plugin's MCP tools when the client exposes them. Start with
`get_metadata(program="UOSA.exe")`; batch known entries with
`get_functions(program="UOSA.exe", functions="0x461800,0x4613b0", fields="entry_point,body_end,decompiled_code")`.
Use `disassemble_function` for instruction-level cross-checks. The plugin loads
some categories lazily: `search_tools` identifies the needed tool and
`load_tool_group` enables its category. Always pass the explicit program rather
than changing the shared GUI selection. Consult the installed tool schemas;
names and supported fields can change between plugin versions.

If MCP tools are unavailable to the client, the read-only HTTP helper reaches
the same analyzed program without opening another project. Do not use this
fallback to bypass an explicit tool permission or read-only restriction:

```sh
python3 docs/client_research/tools/ghidra_query.py \
  --url http://127.0.0.1:8089 --program UOSA.exe \
  --output /tmp/ec-terrain-loader.json functions 0x461800 0x461060 0x5988a0
python3 docs/client_research/tools/ghidra_query.py \
  --url http://127.0.0.1:8089 --program UOSA.exe \
  --output /tmp/ec-layer-xrefs.json xrefs-to 0x461800
```

`GHIDRA_MCP_URL` can replace `--url`. Outputs must be new files. This helper
only calls GET `/get_functions` and `/get_xrefs_to`; it does not alter selection,
annotations or program data. Query just the needed `--fields` for large functions
and select specific JSON fields when reading evidence. Function decompilation
still needs instruction-level cross-checks for engine register arguments.

See [the water investigation](../agent_notes/ec_original_water_code.md)
for the tested executable identity and the addresses established for it. Those
addresses are not portable to other executable builds.
