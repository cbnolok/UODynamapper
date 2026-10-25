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
  --client /path/to/enhanced-client images --output /tmp/ec-source-images \
  1000013 1000017 2000051 6038 2100014
```

Omit material IDs to list all `UOWaterTerrainLayer` entries. The output preserves
source order, repetition, raw sampler fields, `Technique` and `Tweaks`. Exported
DDS files retain the original payload; PNGs use the same decoder as conversion.
Compare independently decoded DDSs when investigating a possible decoder defect.
Always supply the matching client bundle; standard and FP assets can differ.
