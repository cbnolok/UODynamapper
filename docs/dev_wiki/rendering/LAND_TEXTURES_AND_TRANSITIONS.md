# Land Textures And Transition Tiles

This note captures the practical distinction between land textures and land art in Ultima Online, plus the consequences for UODynamapper's current renderer and future EC/CC asset interpretation.

## Ground sources and transitions

Classic map data supplies logical land IDs and heights. Classic terrain texmaps
supply repeatable ground textures, while art supplies images including manually
placed terrain-like static transition tiles. Classic and Enhanced use the same
map tiles, although their data is stored differently. Keep map land and placed
static items distinct: their ownership, geometry and ordering are not interchangeable.

The Classic client relies on authored transition artwork placed in the map.
Enhanced and KR clients can blend terrain textures dynamically, so that artwork
is not strictly required for transitions even though the shared map retains it.
The exact Classic client choice between land art and textured ground, including
its flat and sloped cases, needs source-backed verification; do not treat the
older flat-versus-sloped explanation as a complete client contract.

Dynamapper renders land and static ground art through separate paths. Its
optional Classic and Enhanced texture-transition controls follow the texture
source, not the lighting preset. The controls and their current limitations are
recorded in [terrain transition controls](../../agent_notes/terrain_transition_controls.md);
static ground patches and their engine-side boundary fade are described in
[static ground transitions](../../agent_notes/ec_static_ground_transitions.md).
Those engine improvements are not by themselves proof of original-client parity.
Dynamic blending remains unfinished and must not discard placed artwork or
alter logical tile IDs.

## EC ownership and routing

Logical ownership distinguishes map land from item art. A walkable floor, bridge,
roof or shoreline item remains tileart-owned art even when it draws as a material
surface. A shared WorldArt image, low numeric ID or flat appearance does not
establish land ownership.

- `tileart.uop` owns item records, shader blocks and per-owner image windows.
- `TerrainDefinition.uop` owns terrain materials, their layers and source parameters.
- `GameData.uop`'s `legacyterrainmap.csv` routes Classic land IDs to EC material
  and subtype. TerrainDefinition alias IDs do not replace this source table.
- `Texture.uop` supplies the current EC WorldArt images; Classic fallback art
  comes from `tex_art_cc.uddp`. The EC legacy-art pool is not packed into it.
- `string_dictionary.uop` resolves the client's strings; it is separate from
  the tools' bundled UOP hash dictionary.

`tex_land_ec.uddp` embeds the original routing CSV and an explicit
`TerrainTranscode.kdl` policy overlay. The runtime reads those embedded copies,
not loose KDL files. Preserve the original route and subtype even when an
explicit overlay selects another material.

Missing terrain follows an explicit, logged chain
(`dynamapper/src/core/texture_cache/land/routing.rs`). A land id the selected
EC-layout source lacks is drawn from the other one (Kingdom Reborn and Enhanced
stand in for each other), then from Classic: its texmap or, as the Classic
client does, its land art. An id no source has draws a magenta and black
missing-texture checker. Each load or source switch logs one summary of the
rerouted ids; a missing selected package also shows an in-window notice and is
marked in Preferences. Classic selected never borrows EC-layout terrain.
EC-layout cells from both packages blend through coverage (their material ids
stay distinct); a border between EC-layout and Classic or missing cells is a
hard edge, since coverage and transitions consider EC-layout cells only.

Texture roles follow the source shader/layer class and item position. Names,
stretch and visual resemblance do not select the base or establish a role.
`Rotated` and stretch affect item rendering; neither promotes an item to map
land. Source flags, logical signed Z, tile heights and movement data must remain
separate from visual geometry and painter priority.

Use the focused contracts for implementation details:

- [EC evidence map](EC_WORLD_SPACE_UV_EVIDENCE_MAP.md): executable findings and proof limits.
- [Source routing](../../agent_notes/ec_source_terrain_routing.md): GameData routing, overlays and active material paths.
- [Terrain layer table usage](../../agent_notes/terrain_layer_table_usage.md): the census of the table-driven paths, and their removal.
- [UDDP formats](../UDDP_FORMATS.md): current package schemas and role tables.
- [EC sprite geometry](../../agent_notes/ec_sprite_descriptor_geometry.md) and
  [art trimming](tex_art_ec_trimming.md): source selection, windows, scale and placement.

The former KR routing proposal and heuristic classification matrix are retired.
A separate KR routing/material model still requires original KR evidence; EC
pool containment or third-party reconstruction does not establish that model.
