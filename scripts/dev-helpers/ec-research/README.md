# Original-client inspection helpers

These tools inspect local inputs without executing or patching the original
client. Keep generated dumps, extracted bytes and Ghidra projects outside the
repository, for example under `/tmp`. Record conclusions and executable hashes
in `docs/agent_notes/`; do not commit client binaries or extracted proprietary data.

## PE strings, references and raw data

`pe_inspect.py` uses Python's standard library and handles PE32/PE32+ section
mappings. It reports the executable SHA-256, section layout, matching ASCII
strings and candidate absolute-pointer references in executable sections:

```sh
python3 scripts/dev-helpers/ec-research/pe_inspect.py /path/to/UOSA.exe \
  --string UOWaterShader --string UOWaterTerrainLayer > /tmp/ec-water-references.json
python3 scripts/dev-helpers/ec-research/pe_inspect.py /path/to/UOSA.exe \
  --extract-va 0xc981c8 --size 4 --output /tmp/ec-constant.bin
python3 -m unittest discover -s scripts/dev-helpers/ec-research -p 'test_*.py'
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
  -scriptPath /absolute/repo/scripts/dev-helpers/ec-research \
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

The login flow, packet table, login/game ciphers and the patch sites of the ECLO
loader are named and documented in [`ghidra-login-names`](ghidra-login-names/README.md).

The `tileart.uop` load path, its record format as the client reads it and the code
using tile definitions are named and documented in [`ghidra-tileart`](ghidra-tileart/README.md).
The effect definitions at the end of tileart records, which share their format with
`EffectDefinitionCollection.uop`, are decoded in [`ghidra-effects`](ghidra-effects/README.md). The
Chaos particle engine that plays `.ems` effects is documented in
[`ghidra-effects/results/uosa_chaos.md`](ghidra-effects/results/uosa_chaos.md).
`AnimationSequence.uop`, `AnimationDefinition.uop` and the 0x6E/0xE2 animation packets are traced in [`ghidra-animation`](ghidra-animation/README.md).
`build/multicollection/housing.bin` (house customization pieces) and the `MultiCollection.uop` multis are decoded in [`ghidra-housing`](ghidra-housing/README.md).
UOP data block headers (version-4 timestamps, version-5 signatures) and `data_block_hash` are decoded in [`ghidra-uop-header`](ghidra-uop-header/README.md).
How the client picks the decoder for each image it loads from a UOP (the extension of the requested path, the `assetmap.xml` suffixes of tile textures, the hue bitmap parser) and what each decoder checks are traced in [`ghidra-image-decoders`](ghidra-image-decoders/README.md).
How tileart texture items reach the shaders (item position to shader map index, per shader class), and the check that the client never infers a texture's role from its file name, are traced in [`ghidra-texture-roles`](ghidra-texture-roles/README.md).
Facet sectors (`facetN.uop`, the unread `data.map` entry, the `facetNx.uop` variants), `facets.uop`
FacetDefinition records and the `facetNN.dds` radar images are decoded in [`ghidra-facets`](ghidra-facets/README.md).
The same document covers the facet size table, with a cross-build recipe for patching map sizes, and the
UOKR.exe (Kingdom Reborn) sector format and format deltas (`results/uokr_facets.md`).
`TerrainDefinition.uop` (the field table, flag bits, floats and an encoder recipe) is decoded in
[`ghidra-terrain`](ghidra-terrain/README.md).
The Classic `client.exe` facet table is decoded in [`ghidra-classic-facets`](ghidra-classic-facets/README.md),
for UO Gold, ML, 5.0.9.1, 6.0.14.3, 7.0.20, 7.0.50 and 7.0.103. It covers the wrap rectangle, file ids,
the Felucca-rules flag, the `map0x` table copy selected by 0xA9 flag 0x8000, and the mapdif/stadif patches
and the client versions that still apply them.

### Variable names, prototypes and structures (ApplyVariableSpecs.java)

After the four `Name*Code.java` scripts have named and tagged their functions, `ApplyVariableSpecs.java`
gives those functions prototypes, parameter / local / global names and structure types. Each area has a
spec file next to its script:

| Area | Spec | Tag |
|---|---|---|
| tileart | `ghidra-tileart/tileart_variables.spec` | `UO_TILEART` |
| effects | `ghidra-effects/effects_variables.spec` | `UO_EFFECTS` |
| animation | `ghidra-animation/animation_variables.spec` | `UO_ANIMATION` |
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
python3 scripts/dev-helpers/ec-research/ghidra_query.py \
  --url http://127.0.0.1:8089 --program UOSA.exe \
  --output /tmp/ec-terrain-loader.json functions 0x461800 0x461060 0x5988a0
python3 scripts/dev-helpers/ec-research/ghidra_query.py \
  --url http://127.0.0.1:8089 --program UOSA.exe \
  --output /tmp/ec-layer-xrefs.json xrefs-to 0x461800
```

`GHIDRA_MCP_URL` can replace `--url`. Outputs must be new files. This helper
only calls GET `/get_functions` and `/get_xrefs_to`; it does not alter selection,
annotations or program data. Query just the needed `--fields` for large functions
and select specific JSON fields when reading evidence. Function decompilation
still needs instruction-level cross-checks for engine register arguments.

See [the water investigation](../../../docs/agent_notes/ec_original_water_code.md)
for the tested executable identity and the addresses established for it. Those
addresses are not portable to other executable builds.

## Original material graph and image extraction

The companion Rust example uses the repository's original UOP readers rather
than duplicating their formats in Python. Run from the repository root:

```sh
cargo run -p uocf-devtools-cli --example ec_research -- \
  --client /path/to/enhanced-client materials 5 10 49 70
cargo run -p uocf-devtools-cli --example ec_research -- \
  --client /path/to/enhanced-client items 6038 6057
cargo run -p uocf-devtools-cli --example ec_research -- \
  --client /path/to/enhanced-client images --output /tmp/ec-source-images \
  1000013 1000017 2000051 6038 2100014
```

Omit material IDs to list all `UOWaterTerrainLayer` entries. The output preserves
source order, repetition, raw sampler fields, `Technique` and `Tweaks`. Exported
DDS files retain the original payload; PNGs use the same decoder as conversion.
Compare independently decoded DDSs when investigating a possible decoder defect.
Always supply the matching client bundle; standard and FP assets can differ.
The `items` command prints original per-block shader parameters independently
of the retained `ArtData.materials` / `tilemeta` material metadata. Use source
values to cross-check item-specific composition; terrain Technique/Tweaks are
not substitutes for an item's parameters.

Use `items --summary` to count material families and sampler stages over the
whole original tileart package, or append item IDs to restrict the census. Counts
stay separate by source block; legacy alternatives are not additional EC layers.
This uses the production `ArtData.materials` parser (`has_texture=1`), rather than
treating every nonzero raw block marker as a material. A family count establishes
reachability, not complete runtime support for every sampler in that family.

## Preserve existing packed tables while adding source materials

Build a fresh source tilemeta package from the matching original bundle with
`udd-cli pack pack-tilemeta`. To retain the existing packed tables and add only
the new `metadata/item_materials.json` entry:

```sh
cargo run -p udd-conv-cli --example retain_ec_item_materials -- \
  --package /path/to/existing/tilemeta.uddp \
  --source /tmp/fresh-source-tilemeta.uddp --output /tmp/retained-tilemeta.uddp
```

The example validates all source material blocks against the existing sampler
refs, including physical package and repetition, and refuses a mismatch. It does
not guess a source bundle or overwrite its inputs/output. Compare logical payloads
with `udd-cli tool diff --kind package`: adding source metadata alone should report
one added entry, zero removed entries and zero changed entries. Test the new
package with isolated runtime settings before replacing an installed test asset.

## Packed item composition and shoreline coverage

Inspect original item material parameters against already packed auxiliary pixels:

```sh
cargo run -p udd-conv-cli --example inspect_ec_item_aux -- \
  --tilemeta /path/to/tilemeta.uddp --aux /path/to/tex_aux_ec.uddp \
  --output /tmp/ec-item-aux 6038 6057
```

Omit IDs to inventory every original `UOWaterShader`. The output directory must
be new. It exports each referenced packed image once and reports logical family,
physical source package, source sampler, repetition and original UV-set index.
To export a colliding numeric ID from another original archive, the original
image helper also accepts `images --archive LegacyTexture.uop` (default:
`Texture.uop`). Archive identity is part of the comparison.

The scene inspector reads the installed packed map/statics and tile metadata:

```sh
cargo run -p udd-conv-cli --example inspect_ec_scene_cells -- \
  --packages /path/to/packages --map 0 --x 1445 --y 1790 --radius 14 \
  > /tmp/ec-coast-cells.csv
```

It reports original land IDs/heights/water flags and wet item IDs/heights per
cell, plus every static graphic/height in the `items` column. This distinguishes
non-liquid diagnostic artwork such as NoDraw markers from missing water coverage.
Use the actual map dimensions from its final metadata record, not a guessed
facet height. Feed all observed wet item IDs to `inspect_ec_item_aux`, retaining
its stdout as `/tmp/ec-coast-refs.txt` and images in a new directory. Compare the
eight mask orientations against neighboring cells with:

```sh
python3 scripts/dev-helpers/ec-research/compare_mask_edges.py \
  --cells /tmp/ec-coast-cells.csv --refs /tmp/ec-coast-refs.txt \
  --images /tmp/ec-coast-aux
```

This Python helper requires Pillow and numpy. It compares source alpha-edge
continuity for same-height water coverage; inspect the census first when heights
or stacked layers differ. A low seam score is supporting coverage evidence, not
a substitute for actual renderer captures or original-client pixel validation.

## Original terrain ownership coverage

Export the 32×32 chunk containing a fixture, plus a one-cell ownership border:

```sh
cargo run -p udd-conv-cli --example inspect_ec_terrain_coverage -- \
  --packages /path/to/packages --map 0 --x 6512 --y 464 \
  --output /tmp/ec-acid-ownership.json
python3 scripts/dev-helpers/ec-research/terrain_coverage.py \
  --input /tmp/ec-acid-ownership.json --output /tmp/ec-acid-coverage
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s scripts/dev-helpers/ec-research -p 'test_*.py'
```

The exporter uses packed map cells, effective source/embedded material routing,
and preserved source definitions. Pass `--routing /path/to/TerrainTranscode.kdl`
when matching a preview that applies a loose routing overlay. It refuses existing
outputs, unresolved cells and facet-edge chunks without a full border. The JSON
retains raw land IDs and material definitions, so routing choices remain visible.
This does not establish original-client correctness of manual mapping overlays.

The Python model reconstructs `0x461060`'s raw coverage bytes from the original
`coverage_exponent` and its exact chunk-edge/neighborhood rules. Pillow exports one
32×32 PNG per material; `.rgba` contains the corresponding four-channel bytes.
`manifest.json` records layer sort keys, cardinal/diagonal bytes and histograms.
All outputs stay outside the repository. See the executable evidence and
sampling contract in `docs/agent_notes/ec_original_water_code.md`.

This is an offline byte model. It neither captures the original executable nor
reproduces the final framebuffer, ordered terrain passes, filtering or lighting.
Inspect its masks alongside actual renderer captures before implementing terrain
composition. A successful export is not a visual regression pass.
