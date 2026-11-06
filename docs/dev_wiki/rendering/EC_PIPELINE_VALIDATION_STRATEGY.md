# EC pipeline validation

Validate changes at the parser, package, inspection and rendered-image levels
that they affect. Original UOP records, executable consumers and original shader
text establish client contracts; current engine behavior and third-party
reconstructions cannot establish those contracts by themselves.

## Parser and package checks

Use targeted existing tests for the changed reader or packer. Preserve shader
class, item position, source kind, sampler index, UV set, authored stretch,
image windows, native extents and logical gameplay metadata through conversion.
Roles come from shader and position, not image names. Tilemeta has no
`speculative_role` or `implementation_readiness` field.

Useful focused checks include:

```sh
cargo test -p uocf tileart_ref_preservation
cargo test -p udd-assets tilemeta
```

For other changes select the relevant test in `udd-conv` or `udd-container`;
check affected consumers when an interface changes. A package layout change
must update [UDDP formats](../UDDP_FORMATS.md), require a repack where needed,
and verify reader rejection of unsupported versions. `udd-cli tool validate`
checks actual package integrity; compilation alone cannot validate installed data.

## Inspect the actual source and package set

The inspection example prints source records, decoded image windows, texture
families/packages, stable roles, block/item positions, selection reasons,
stretch, source kind, sampler index and UV set:

```sh
cargo run -p udd-conv-cli --example inspect_ec_tile_surface_signals -- \
  --ecdir /path/to/enhanced-client \
  --tilemeta /path/to/tilemeta.uddp \
  --ec-art /path/to/tex_art_ec.uddp \
  --ec-land /path/to/tex_land_ec.uddp \
  4651 11070 1080
```

Its `renderer_decision` and runtime-slot output use a separate historical
surface-like resolver, not the renderer's current source selection. Do not
accept that output as proof of the live draw route. Likewise, its raw record
catalog indexes stored IDs, whereas the corrected art loader resolves named
assets; use the [geometry audit](../../agent_notes/ec_sprite_descriptor_geometry.md)
for owner identity and windows. Packed per-ref path lines remain placeholders;
source paths printed from the original client are a separate output.

Use one matching client/package manifest. Plain and FP Enhanced installations
can differ at the same numeric IDs. Record source identity, package versions
and hashes, effective cached settings, selected image source and native extents.
See [auxiliary coverage](../../agent_notes/tex_aux_ec_coverage.md) and
[terrain routing](../../agent_notes/ec_source_terrain_routing.md).

The old `ec_material_audit` CSVs and
`dynamapper/assets/regression_fixtures/ec_content_families.toml` are historical
census candidates based partly on path guesses. They are not current role,
readiness or routing specifications, nor proof that a material ID is a Classic
land ID. Reconfirm representative owners and coordinates from current sources.

## Rendered-image checks

Use [the renderer controls](../../../scripts/dev-helpers/renderer/README.md)
with isolated settings and settled streaming. Check affected sources and all
three styles for visual changes. Keep clocks, projection, source selection,
camera and effective presets explicit. Shader composition and GPU validation
are separate from a Rust build.

Existing bounded controls include road `(2518,561)`, bridge `(2834,987)`, town
`(1647,1682)`, coast `(1445,1790)`, lava `(5744,2928)`, swamp `(1872,2416)` and
hued chairs `(6404,2600)` on map 0. Their dated acceptance reports record the
package and capture limits. Use matched original-client references to assess
parity; old engine captures alone establish regression behavior.

Follow [the UV evidence map](EC_WORLD_SPACE_UV_EVIDENCE_MAP.md) for repetition
and material-stage contracts. Distinguish correct source selection, geometry,
coverage, filtering, lighting and animation when diagnosing a difference. Keep
historical repaired fixtures as controls and reproduce failures before reopening
them. Remaining work belongs in [the canonical TODO](../../TODO.md), rather
than a second phase checklist.
