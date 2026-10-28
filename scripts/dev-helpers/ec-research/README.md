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
`unk2` exponent and its exact chunk-edge/neighborhood rules. Pillow exports one
32×32 PNG per material; `.rgba` contains the corresponding four-channel bytes.
`manifest.json` records layer sort keys, cardinal/diagonal bytes and histograms.
All outputs stay outside the repository. See the executable evidence and
sampling contract in `docs/agent_notes/ec_original_water_code.md`.

This is an offline byte model. It neither captures the original executable nor
reproduces the final framebuffer, ordered terrain passes, filtering or lighting.
Inspect its masks alongside actual renderer captures before implementing terrain
composition. A successful export is not a visual regression pass.
