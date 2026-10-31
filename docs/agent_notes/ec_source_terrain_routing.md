# EC terrain routing: original GameData table

## Primary source

`GameData.uop` contains `data/gamedata/legacyterrainmap.csv`, with columns
`legacyId,newId,newSubType`. This is the original client's Classic-land-to-EC-material
mapping, including its authored subtype. The filename is resolved by UOP hash;
the source payload, rather than a reconstruction dictionary, establishes the routes.

Do not confuse this CSV with `LegacyTerrain.uop`. The latter contains XML
`terrainType` records with `id`, `name`, `texMap`, and flags. Its numeric `texMap`
does not establish a WorldArt image path. For example, original land 203 is
forest, while WorldArt image 203 is wall art.

The standard and FP clients can have different route counts. Load the GameData
from the same source bundle used for the conversion; do not hardcode a count or
copy a different client's table into the packer.

## Runtime contract

- `uocf::enhanced::legacy_terrain_map` parses the source CSV and retains it verbatim.
- The EC land packer embeds it as `metadata/legacy_terrain_map.csv`.
- `TexLandEcPackage` builds the source routing base from this metadata, then
  applies embedded KDL and loose KDL as explicit policy overlays.
- `source_terrain_map()` exposes the original routes and subtypes even when
  an explicit overlay selects a different material.
- Missing EC material images still remain explicitly black. This source graph
  does not enable a Classic texmap fallback.

The coverage example accepts an optional final routing KDL argument and reports
source/override conflicts. Keep source coverage, override policy, and visual
fidelity as separate claims. A complete material-slot census does not certify
that every manual override matches original EC artwork.

## Related rendering contracts

Source layer indices and shader roles are different APIs. Reviewed liquid
materials use explicit base/normal roles; their raw reflection/ripple inputs
must not be reinterpreted as a solid material's detail/mask pair. The source-layer
reader remains available for inspection.

EC page and auxiliary images use UNORM views: original texture-stage flush
`0x74d9f0` explicitly disables `D3DSAMP_SRGBTEXTURE`, and pass `0x745640`
disables `D3DRS_SRGBWRITEENABLE`. Keep data channels encoded during filtering.
Liquid stages compose encoded RGB, apply their diffuse response and clamp, then
convert the completed color once for Bevy's linear postprocessing. Solid albedo
samples convert after native encoded filtering; normal/displacement/RGB masks
read native data directly. CC page images and ordinary art remain sRGB.
The shared liquid sampler uses one hardware read for interior/clamped linear
footprints, including bump data; wrapped border taps stay source-bounded.
Auxiliary item samples use the native dimensions of top-left-padded layers.

Liquid terrain also has a [height-aware bank policy](ec_liquid_bank_transitions.md):
its coverage detaches early on rising solid banks while retaining the underlying
solid pass stack. This is an intentional rendering policy, separate from the
recovered original mask generation and material routing.

Tileart-owned slots marked as land/material surfaces stay in the art atlas.
Flagged art-owned surfaces use their authored camera-plane quad in the default
isometric view; background liquid ground quads retain their horizontal depth
path. Terrain-owned copies cannot replace their
authored item windows. Classic shoreline sprite quads keep their separate
diagonal geometry and alpha-coverage contract.

Check the source reference marked `PRIMARY_SELECTED` when interpreting packed
tileart pixels or repetition. The packer uses `ArtData.ec_texture`, whereas the
runtime's role-aware `main_ec_texture_ref` may select a different image. A packed
support slot cannot become albedo just because the runtime chooser finds a base.
For example, shoreline item 6038 packs source 1000013 (alpha/support), while the
role-aware chooser returns visible source 2000051. Known support primaries use
the visible terrain-material route instead of painting support pixels as color.

An existing static/sprite slot also takes precedence over terrain redirection.
Walkable `Surface` flags on stairs, bridges, and furniture do not turn their art
graphic IDs into land IDs. For example, item 1993 owns wooden-plank sprite art;
interpreting 1993 through the land routing table selects unrelated terrain.

The fallback terrain-neighbor heuristic keeps solid substrates and liquid
surfaces separate. Its former averaging of underwater sand (land 100, material
52) into water material 5 created a pale sawtooth boundary at static/terrain
ownership changes. Source-backed composition instead uses per-layer coverage.

This separation describes the current heuristic's guard, not the original
client's complete transition model. Original terrain creates one 32×32 RGBA
ownership mask per material. `TerrainDefinitionEntry.coverage_exponent` is its
coverage exponent: positive values supply separate cardinal/diagonal neighbor bytes;
zero retains binary ownership. The original terrain comparator sorts normal
layers by material ID and water by `1000 * material_id`. UVs, chunk-edge tests,
filtering and pass-index evidence are documented in `ec_original_water_code.md`.
The reusable exporter/model reconstructs these bytes independently of rendering.
Runtime lookup role 6 now retains the source coverage parameters, and a per-chunk
Rg16Uint image retains the first/last payloads within the original 49-pass cap.
The fragment shader evaluates the full local mask footprint and composes shaded
material layers in source order, including liquids. It does not average wet and
solid albedos or introduce a compatibility mapping.

The complete original material graph now survives conversion in
`metadata/source_materials.json`. This is distinct from the route CSV and the
manual override graph. Original `UOWaterTerrainLayer` source indices 0/1/2 are
bump, fallback, and animated surface, rather than solid base/detail/mask.
The GPU lookup's first five roles carry images; role 5 carries the source
coordinate technique, bump UV velocity, and source-presence marker. Role 6
carries the separate terrain ownership/coverage parameters.
Both land and source-matched item water consume it. See
[ec_original_water_code.md](ec_original_water_code.md) for the executable evidence
and the remaining reflection/coverage proof boundary.

Item shader metadata is separate from the terrain graph. `ArtData.materials`
retains each original block's shader name, Technique, Tweaks and block index;
tilemeta packages require `metadata/item_materials.json` and expose it through
`item_material` / `item_materials`. Matching an item's source graph
to terrain water establishes shared base inputs, but does not establish its
fourth-image operation or UV domain. Read the item's own retained parameters.


Source-matched item water now preserves its item graphic through the ground
instance as well as the shared terrain payload. Its fourth image and own blend
parameters use an independent 16-bit item lookup, sharing the existing auxiliary
GPU atlas with terrain. This lookup works without the terrain sidecar. Local UV
set zero is preflipped by the original mesh before the affine transform; world
UV set one follows signed `(map X, -map Y)` coordinates. The liquid sampler
converts positive Bevy world Z before the affine transform for original bump,
world-indexed surface and item world-overlay sources. Technique 2 selects stream
zero, supplied separately as world UVs by terrain and local UVs by item art.
Reflection vectors and generated terrain coverage have separate domains.
Both original liquid mesh paths omit a vertex normal stream. Their fixed-function
reflection coordinates reduce to normalized view-position XY, generated at
vertices and interpolated before the source affine. The runtime uses that
contract for both owners; it does not reflect a fragment ray about a fabricated
horizontal normal. Terrain carries the coordinates in its otherwise unused UV
varying, keeping the current batched terrain/material architecture.
See `ec_original_water_code.md` for the source addresses,
neighbor-mask continuity check, and remaining terrain-specific composition work.

Items with local hardware-fallback masks (acid border items 11778–11817) cannot
require the same source-1 image as terrain. The source matcher also supports an
active-source pair: bump/surface IDs, their repetitions, Technique[0] and bump
scroll must match. This prevents unrelated ocean fill and keeps acid variants
with different scroll rates distinct. The existing complete three-image water
match remains available. Both routes require the original water shader and
WorldArt/Texture references, and still retain item-owned fourth-stage coverage.

Black wedges labeled "no draw" at the acid fixture are item 8612's diagnostic
art, not missing terrain textures. Tilemeta preserves the source NoDraw flag;
art emission and visible-object picking now skip it, while dot diagnostics and
full cell inspection retain the source data.
