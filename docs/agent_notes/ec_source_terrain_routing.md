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

Color and support images currently share sRGB atlas views. Recover encoded RGB
with `texture_data_rgb` before interpreting normal, displacement, or RGB-mask
samples as data. Alpha is not sRGB-decoded. This corrects data-channel values;
precise interpolation of data maps would require a dedicated linear view.

Tileart-owned slots marked as land/material surfaces stay in the art atlas and
render as axis-aligned ground cells. Terrain-owned copies cannot replace their
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

The heuristic terrain-neighbor blend must keep solid substrates and liquid
surfaces separate. Mixing underwater sand (land 100, material 52) into water
material 5 creates a pale sawtooth boundary at static/terrain ownership changes.
Liquid coverage belongs to its material/alpha path, not a solid albedo average.

The complete original material graph now survives conversion in
`metadata/source_materials.json`. This is distinct from the route CSV and the
manual override graph. Original `UOWaterTerrainLayer` source indices 0/1/2 are
bump, fallback, and animated surface, rather than solid base/detail/mask.
The GPU lookup has five image roles and a sixth parameter record carrying the
source coordinate technique, bump UV velocity, and source-presence marker.
Both land and source-matched item water consume it. See
[ec_original_water_code.md](ec_original_water_code.md) for the executable evidence
and the remaining reflection/coverage proof boundary.
