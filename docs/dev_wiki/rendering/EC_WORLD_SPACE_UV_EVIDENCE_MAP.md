# EC texture coordinates and evidence

The EC renderer and UDDP conversion rework follows the original client's decoded
records and executable consumers. Third-party FX files are reconstruction references;
they do not define the package format, texture roles or client behavior.

## Original-client contracts

- `UOSA.exe`'s terrain texture loader at `0x004613b0` sets each item's U/V
  transform scale to `1 / stretch` (transform method 2, centre 0.5); `UOKR.exe`'s
  at `0x0044b180` does the same. Both mesh builders (`UOSA 0x00461ae0`,
  `UOKR 0x0044b7f0`) write set 0 as `(x, -0.0 - y)` in whole map cells and
  set 1 as the chunk-local cell times 1/32, so every item repeats every
  `stretch` cells. Neither client offsets, rotates or swaps UVs per cell or
  chunk; on-screen periods measured on 2026-11-12 match the item stretches
  (EC grass 4, KR grass 5, sand 6, EC forest 4 with an exact 16-cell repeat).
  Neither image dimensions nor a preferred stretch range selects the client's
  base. The client's set 0 v runs against map y while this renderer's runs
  with it, yet the screen orientation matches: an EC client screenshot of a
  grass plain (2026-11-13, 19.62 px per cell axis, EC zoom 0.89) folded into
  one period correlates with item 1 (stretch 4) at r = +0.87 with v along +y
  and at most +0.16 in the seven other flips and quarter turns, and with
  item 0 (stretch 10) at +0.52, again only along +y. Terrain is not mirrored
  here. The same screenshot repeats every 4 cells, with item 0's 10 cells and
  the mask's 32 under it, and adds no further detail or macro layer; it
  repeats less visibly than this renderer mainly because its filtering is
  softer and its contrast lower (period-4 correlation about 0.3 against 0.4
  in Enhanced and 0.45 in KR-like).
- Terrain roles follow layer class and item position. Default and bump layers
  use item 0 for colour A; water's fallback is item 1, while its full path uses
  item 0 to perturb item 2. Names such as `normal`, `alpha` or `cube` do not
  establish roles.
- Classic land IDs route through the original `legacyterrainmap.csv` to a
  material and subtype. TerrainDefinition alias IDs are not a substitute.
- Water scrolling follows the client's techniques and tweaks. There is no
  recovered FollowCenter flag; the former renderer flag was removed.
- Item image-window UVs and camera-quad UVs are separate contracts. A trimmed
  packed image does not establish its original primary texture dimensions.

The [terrain-layer executable research](../../../scripts/dev-helpers/ec-research/ghidra-terrain-layers/results/uosa_terrain_layers.md)
records stage positions, addresses, confidence and the rejected name/repetition
heuristics. The [water investigation](../../agent_notes/ec_original_water_code.md)
records transforms, clocks, reflection and item-window evidence and its limits.

## Current package and renderer behavior

`tex_land_ec.uddp` carries source routing, material provenance and role-specific
land pages. See [the terrain guide](LAND_TEXTURES_AND_TRANSITIONS.md),
[UDDP formats](../UDDP_FORMATS.md) and
[the source-routing contract](../../agent_notes/ec_source_terrain_routing.md).

`dynamapper/assets/shaders/world/land/sampling.wesl` uses a positive packed
`texture_stretch` when available. Its fallback is image width divided by 44.
That fallback is engine behavior, not evidence that the client derives stretch
from pixel width. Preserve authored repetition separately from native image
extent and optional upscaling. Since 2026-11-06 every solid Enhanced land tile
carries its material's item 0 stretch, so the fallback serves only the 10
direct-slot tiles without a material (103, 109-112, 128, 16380-16383), which no
shipped facet places.

The renderer reads terrain roles from the role-specific land pages only. The
[table usage audit](../../agent_notes/terrain_layer_table_usage.md) found no
tiles reaching the table-driven secondary-base/ripple shader paths in its
canonical-package census, and those paths were removed on 2026-11-06.

For item surfaces, [source geometry](../../agent_notes/ec_sprite_descriptor_geometry.md)
and [direct item-water acceptance](../../reviews/2026-11-01-item-water-materials.md)
describe native windows, primary extents and constructor-specific UVs. Keep
logical XYZ separate from rendering geometry; the calibrated height conversion
is documented in [the height notes](../../agent_notes/static_height_calibration.md).

## Validation boundary

Original records prove stored data; executable consumers and original shader
text establish how the client uses it. Current Rust/WESL code establishes this
engine's behavior. Matched original-client captures are still required for
visual parity, including filtering, reflection, animation and rare geometry.
A decoded format or complete texture census alone does not certify that parity.
