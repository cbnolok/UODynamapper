# Converted-asset diagnostics

Development tools recovered from the scratch workspace. They accept explicit
inputs; keep generated packages, captures, CSVs and benchmark output outside the
repository. These binaries are not included in release archives.

```sh
cargo run -p udd-devtools-cli --features dev-tool-tests --bin udd-devtools-cli -- --help
```

| Command | Purpose |
| --- | --- |
| `package-compare REFERENCE CANDIDATE --repetitions 3` | Compare decoded records, keys and types; report texture storage and best in-memory read times |
| `gumps same-pixels REFERENCE CANDIDATE` | Compare whole canvases, dimensions, scale and shared-gump tables |
| `gumps score REFERENCE CANDIDATE OUTPUT.csv [--linear]` | SSIMULACRA2 scores for single gumps over grey, plus alpha changes |
| `gumps sizes PACKAGE` | Per-format pixel storage and atlas storage |
| `paperdoll-compare CLASSIC_DUMP ENHANCED_DUMP CLASSIC_PACKAGE ENHANCED_PACKAGE` | Compare baked paperdolls with the independent pre-bake composition rule |

Equality commands fail when differences are found. `package-compare` compares
decompressed payload bytes, not decoded image pixels. Use `gumps same-pixels`
when encodings differ. Read timing excludes disk I/O and reports the best of the
requested repetitions, not a cold-cache load benchmark.

The paperdoll comparison requires archived **unscaled, pre-bake** dumps containing
`index.tsv` and `paperdoll.bin`, produced by the old package reader. These are
external fixtures; the current reader does not reopen version 3 packages. The
tool checks resolved membership, shared hashes, non-paperdoll pixels and each Enhanced gump
composed over its body. It reports known glow and pure-black differences
separately and fails on other composition differences. It is a migration oracle,
not a validator of arbitrary gump packages.

## GPU experiments

```sh
cargo run --release -p udd-devtools-cli --features dev-tool-tests,gpu-bench --bin udd-gpu-cost -- --help
# run|compile|upload DUMP_DIR cc|ec
# --assets PATH real
```

`run`, `compile` and `upload` read `cc.gpu` or `ec.gpu` animation-codec dumps. The
writer is the `qd` experiment in
[`mobile_anim_codec/shrink/tiny/lossy.rs`](../../lib/udd-conv/tests/integration/mobile_anim_codec/shrink/tiny/lossy.rs),
using `MOBILE_ANIM_CODEC_DUMP_DIR`. Preserve original dumps with old results; this
research format has no compatibility guarantee.

Synthetic layouts measure sampling, hue lookup, branching and upload costs on
the selected Vulkan adapter. `real` compiles the current art WESL and labelled
text-rewrite experiments; it does not draw that shader. Timings do not predict
performance on other hardware. The [animation codec research](../../docs/agent_notes/proposals/2026-11-01-mobile-anim-codec.md)
records the original methodology and limits.

## Web-map codec experiments

```sh
cargo run --release -p udd-devtools-cli --features dev-tool-tests,web-codecs --bin udd-webmap-codecs -- \
  --output /tmp/webmap-codecs --skip-avif /path/to/crop.png
```

Inputs are 512x512 PNG crops with distinct file stems. The tool compares WebP and optionally AVIF for size, encode/decode time, SSIMULACRA2 and colour
error. `--water-crop NAME` enables the zero-red diagnostic for a named input;
unavailable colour subsets are reported as `n/a`. AVIF requires
`avifenc` and `avifdec`, or pass `--skip-avif`. These experiments do not add codecs
to production package formats.

Sources came from `lossless-zstd/pagecheck`, `gumps-bc7/score`,
`paperdoll-bake/scripts/examples/check_paperdoll_bake.rs`, `gpu-cost` and
`webmap-codecs` in the external scratch tree. Copied checkouts, encoder forks,
GUI injection scripts, scheduling wrappers and generated data were not imported.
Overlap and seam diagnostics live with the
[renderer helpers](../../scripts/dev-helpers/renderer/README.md).
