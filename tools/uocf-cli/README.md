# uocf-cli

`uocf-cli` contains general-purpose Ultima Online file tooling built on top of
the `uocf` parser library.

## Binaries

- `uop-tool`: hash UOP paths, brute-force hashes, extract packages, merge DIC
  dictionaries, replace package payloads, and rebuild packages.
- `cc-uop-mul-converter`: convert supported Classic Client legacy `.mul`/`.idx`
  files to and from modern `.uop` packages.
- `uop-dict-populator-cli`: populate DIC hash dictionaries from configured UOP
  templates.
- `texture-scanner`: scan unpacked DDS texture trees and write a CSV inventory.
- `sound-tool`: export Classic Client sound entries.
- `multimap-tool`: convert Classic Client `multimap.rle` to and from images.
- `facet-evidence-tool`: extract EC/KR facet terrain evidence into KDL.
- `kr-ec-terrain-diff-tool`: compare KR terrain routing against EC evidence.

## Current Production Status

The crate builds and has targeted tests for the highest-risk UOP and legacy
MUL/UOP paths, but it should still be treated as production-candidate tooling
rather than a fully exhaustive production surface.

Validated coverage currently includes:

- `uop-tool hash` command output.
- `uop-tool merge-dic` with synthetic DIC dictionaries.
- `uop-tool extract` against a generated UOP fixture.
- `uop-tool extract` failure on malformed UOP input.
- `uop-tool extract --dictionary` failure on a bad explicit dictionary.
- `uop-tool replace` and `uop-tool rebuild` against generated UOP fixtures.
- Safe extraction path handling for dictionary-resolved names.
- Unique temporary paths for package rewrite operations.
- Legacy MUL/UOP malformed chunk handling for truncated gump and multi payloads.
- A synthetic `sound.mul`/`soundidx.mul` to `soundLegacyMUL.uop` round trip.

## Important Behavior

- `uop-tool extract` rejects dictionary-resolved output names that are absolute,
  empty, contain parent-directory components, or otherwise escape the output
  directory.
- `uop-tool replace` and `uop-tool rebuild` write through a unique sibling temp
  file before renaming over the target package. They do not use the old fixed
  `*.uop.temp` path.
- `cc-uop-mul-converter` returns a nonzero exit status when attempted
  conversions fail. Missing optional map variants during extract are skipped.
- `texture-scanner` requires explicit input and output paths; it no longer uses
  hardcoded local development paths.

## Validation

Use targeted checks while developing:

```sh
cargo check -p uocf-cli
cargo test -p uocf-cli
cargo test -p uocf-cli --test uop_tool_cli
```

Do not run automatic formatters or linters as part of this repo's normal agent
workflow.

## Remaining Gaps

- `cc-uop-mul-converter` still needs broader fixture coverage for art, gump,
  multi, and map conversions.
- `uop-dict-populator-cli`, `facet-evidence-tool`, and
  `kr-ec-terrain-diff-tool` have only compile-level coverage.
- Real client package fixtures are still needed for end-to-end regression tests.
- Some tools intentionally continue after per-file warnings; those cases should
  be reviewed command-by-command before changing exit behavior.
