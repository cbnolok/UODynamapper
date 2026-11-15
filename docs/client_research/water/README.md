# EC water and terrain-material inspection

Inspectors and checks from the investigation of the Enhanced Client's water and
terrain materials. The executable evidence, the addresses and the sampling contract
are in [the water investigation](../../agent_notes/ec_original_water_code.md). The
coverage model is now implemented in the renderer
(`dynamapper/src/core/texture_cache/land/coverage.rs`); `terrain_coverage.py` stays
as the independent offline model of the client's bytes.

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
python3 docs/client_research/water/compare_mask_edges.py \
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
python3 docs/client_research/water/terrain_coverage.py \
  --input /tmp/ec-acid-ownership.json --output /tmp/ec-acid-coverage
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s docs/client_research/water -p 'test_*.py'
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
