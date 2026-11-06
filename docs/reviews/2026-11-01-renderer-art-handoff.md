# Renderer artwork and package handoff — 2026-11-01

This acceptance report records the KR-only artwork contract and installation
of the matching asset set on 2026-11-01.
Performance work remains deferred to the profiling workstation.

## Behavior and cause

The canonical EC package now contains KR WorldArt only. The renderer previously
assumed every EC-mode sprite lived in that package and used an atlas keyed by
numeric page number. Implementing fallback only at the source selector would
therefore select wrong pages when Classic and KR page numbers coincide, and
would leave picking, anchors and wetness inconsistent.

The per-object chain is now present KR art, then Classic owner/texture alias,
then original `missing.tga`. Classic fallback retains Classic dimensions,
pixel units, anchors, flat-art eligibility, hue and legacy opacity. Source
selection precedes render wetness and depth classification. Geometry, CPU
alpha and inspector routing use the same selection. Classic slot IDs remain
u32 after adding 0x4000. Logical graphics, XYZ, signed Z, heights and flags are
unchanged. `UseNewArt` is informational, as requested.

The existing batched atlas uses source-qualified page keys and preserves them
when growing/reuploading. Native matching BC7 pages stay compressed; mixed
formats use RGBA. Missing-source auxiliary aliases share decoding, hue crops
and upload layers by payload ID, retaining requested IDs and family checks.
No extra rendering pass or GPU instance-layout change was added.

A separate installed-data gap appeared at `(6404,2600)`: book 39297 at
`(6403,2599,5)` had neither a KR slot nor a packed Classic slot, although the
selected original Classic UOP contains its 44x39 pixels and matching tiledata.
The old Classic atlas had 34,503 present slots. A full lossless rebuild with the
unchanged converter has 39,386: 4,883 added, none lost. Compared with the old
atlas, 499 images and 466 geometry records differ. Every resulting slot was
independently checked against the original Classic UOP: decoded RGBA, native
extent and static anchor match, with zero mismatches. This is a source-validated
replacement; it is not a claim that every old Classic pixel is preserved.

## Assets accepted on 2026-11-01

At this acceptance checkpoint, six files were installed together under `/home/claudio/test/uddp`:

| File | Accepted source |
| --- | --- |
| `tex_art_ec.uddp`, `tilemeta.uddp`, `tex_aux_ec.uddp` | `kr-art-only-20261031` candidate, plain Enhanced client |
| `tex_land_ec.uddp` | `client-same-items-20261031` candidate |
| `terrain_meta.bin` | `terrain-layers-by-class-20261031` candidate |
| `tex_art_cc.uddp` | Complete native Classic UOP rebuild, 2048px pages, RGBA/Zstd |

The readers at this checkpoint required tilemeta 3 and terrain provenance 4.
These are recorded acceptance versions, not current format requirements. The
terrain sidecar was absent from the destination before installation; the five
existing packages have checksum-verified backups. The installation report
records all source/destination/backup hashes. The earlier fp/plain auxiliary
coverage diagnosis and conservative missing-mask policy are superseded; see
[the source/role investigation](../agent_notes/tex_aux_ec_coverage.md).

The terrain integration retains the asset agent's class/position contracts,
correct blend direction, source-selected lava/swamp bases, shader-based EC
render wetness and original water tweak clock. The executable evidence is in
[the terrain-layer research](../../scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md)
and [the terrain documentation](../dev_wiki/rendering/LAND_TEXTURES_AND_TRANSITIONS.md).
Raw IsWet remains gameplay metadata. Still captures do not establish motion
or reflection parity.

## Validation

The focused checks pass: 100 art tests, five atlas tests, four draw tests,
three material/hue tests, six controller tests and two Python readiness tests.
Real-package checks exercise every drawable metadata owner and shared
missing-image uploads. Two original bridge sprites and both Classic crane
pieces match original source RGBA/windows. The canonical hue audit admits
11,673 sprites, resolves all 340 masks and records 313 native crops; nine
bindings intentionally use the client's missing image.

Native previews use exact-PID X11 placement on Desktop 4 and isolated presets.
The supplied official EC references use legacy artwork; Enhanced controls use
Classic artwork with EC land. KR WorldArt cannot claim pixel parity from those
legacy screenshots.

| Controls | Evidence prefix under `/tmp/uodynamapper-ec-followup/phase1-handoff` |
| --- | --- |
| Three styles: bridge, road, town, lava, swamp, coast | `matrix-*` |
| Restored roof/log graphs, dry trough, hued chairs, missing-source flowers | `surfaces-*` |
| KR perspective: bridge, town, crane | `perspective-*` |
| Actual Classic fallback beam 18910; alpha hole passes to pier 942; mixed KR bridge | `fallback-*` |
| Deliberately omitted Classic/KR packages | `no-classic-*`, `no-kr-*` |
| Complete Classic rebuild: three styles, bridge/town/crane/chairs | `complete-*` |
| Fresh installed-set verification | `installed-*` |

The complete atlas removes the false missing-image rectangles beside the chairs.
Fresh installed-set bridge/chairs frames exactly match their accepted candidate
frames. App-only probes select book 39297 at `(6403,2599,5)` through Classic slot
55681 and the adjacent chair 2901 with hue 1157 through its KR slot; OS focus and
pointer are unchanged. All 41 matrix frames have settled art queues and no
shader/pipeline validation errors.
KR bridge/town frames are exactly unchanged by that Classic rebuild. Classic
and Enhanced controls have small differences and must not be described as
universally byte-identical. The full source comparison explains which old
slots changed; earlier historical water comparisons also contain sparse
triangle-edge differences. No coordinate-specific suppression was introduced.

Terrain metadata readiness alone initially captured an empty bridge before
sprite streaming settled. `status.art` and `control.py wait-ready` now require
matching positions, submitted terrain metadata and stable art with no misses
or queued pages. This is a bounded CPU queue check, not a GPU fence; actual
frames and shader logs remain part of acceptance. The bridge appears fully
once those queues settle. See [the control workflow](../../scripts/dev-helpers/renderer/README.md).

## Scope and remaining work

This completes artwork-source integration, not every renderer fidelity issue.
The subsequent [visible static picking acceptance](2026-11-01-visible-static-picking.md)
adds composed coverage, terrain occlusion and visible-fragment priority. Some
individual anchors, rare overlaps, arbitrary camera directions, animation,
liquid/reflection/filtering parity, effects and optional lighting remain in
later phases. The carpet placement used here is roof-occluded and is not
positive visible-carpet proof. Original-source graph checks and visible roof/log
controls must not be conflated with it.

The old speculative ripple branch remains unreachable under the current water
classification. It has no established original-client counterpart; its removal
or replacement is separate from this artwork handoff. Walking/collision must
continue to use the preserved logical world data.

Temporary evidence includes `candidate-packages.json`, the previous-set manifest,
`classic-source-complete-comparison.json`, `hue-crops-final.json`,
`package-installation.json`, source checks, effective preset/status records and
native logs. Proprietary pixels remain outside the repository. Reusable source
comparison helpers live in `tools/udd-conv-cli/examples/`.


To reproduce the complete Classic rebuild and verify its relationship to a
previous package without exporting thousands of images:

```sh
cargo run -p udd-conv-cli --bin udd-cli -- pack pack-art \
  --ccdir /path/to/classic --output /new/tex_art_cc.uddp \
  --atlas-width 2048 --atlas-height 2048 --raw --zstd=8
cargo run -p udd-conv-cli --example compare_classic_art_packages -- \
  --before /old/tex_art_cc.uddp --after /new/tex_art_cc.uddp \
  --classic-client /path/to/classic --output /new/comparison.json
```

Without `--classic-client`, the comparison requires every old present image and
geometry record to be preserved. With it, changed content is accepted only when
every resulting native slot matches the original Classic UOP; lost slots still
fail. `compare_classic_sprite_source --allow-missing-packed` exports an available
original sprite and reports an absent packed slot instead of aborting the audit.
