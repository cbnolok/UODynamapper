# Development helpers

These are reusable diagnostics and research tools. Keep generated dumps,
captures, logs and Ghidra projects outside the repository; the committed result
notes record evidence and its limits, not current implementation requirements.

Rust package comparisons, gump scoring and optional GPU/codec experiments are
workspace members under [tools/udd-devtools-cli](../../tools/udd-devtools-cli/README.md).

- [EC research](ec-research/README.md): PE inspection, Ghidra scripts, recovered
  client formats and consumers, terrain-coverage checks and mask comparisons.
  The scripts and result notes make the executable-derived renderer/UDDP
  contracts reproducible; preserve them together.
- [Season profiles](seasons/README.md): the generator of the bundled route file's season profiles from the
  Classic client's season tables, and where those tables are in the client.
- [Renderer controls](renderer/README.md): app-only input, readiness and capture
  controls, benchmarks, GPU pass timings and optional shader statistics. These
  support repeatable validation without taking over the working desktop.
- `check_dep_alignment.py`: dependency-version diagnostics, invoked by
  `just check-dep-alignment` and `just check-dep-alignment-ref`.
- `classify_effecttexture_dump.py`: standalone signature-based classification
  of an already extracted EffectTexture dump. This remains useful without
  building Rust tools; for package-wide client audits, use the image census in
  `uocf` instead. File signatures do not establish material roles.

The Python unit checks need no client installation or GUI:

```sh
python3 -B -m unittest discover -s scripts/dev-helpers/ec-research -p 'test_*.py'
python3 -B -m unittest discover -s scripts/dev-helpers/renderer -p 'test_*.py'
```

`compare_mask_edges.py` and the renderer frame/seam helpers require Pillow and NumPy;
the frame-comparison unit tests use these dependencies too. Ghidra helpers
require the locally installed Ghidra tooling; follow each research README.
