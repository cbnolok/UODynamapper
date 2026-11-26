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
source, not the lighting preset. The Classic control blends two texmaps only
where their facing edges differ in colour, so the hand-drawn transition texmaps,
whose edges were painted to match, keep their art, and floors and paving stay
hard. The controls and their current limitations are recorded in
[terrain transition controls](../../agent_notes/terrain_transition_controls.md);
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
(`dynamapper/src/render/assets/land/routing.rs`). A land id the selected
EC-layout source lacks is drawn from the other one (Kingdom Reborn and Enhanced
stand in for each other), then from Classic: its texmap or, as the Classic
client does, its land art. An id no source has draws a magenta and black
missing-texture checker. Each load or source switch logs one summary of the
rerouted ids; a missing selected package also shows an in-window notice and is
marked in Preferences. Classic selected never borrows EC-layout terrain.
EC-layout cells from both packages blend through coverage (their material ids
stay distinct); a border between EC-layout and Classic or missing cells is a
hard edge, since coverage and transitions consider EC-layout cells only.
Black land (the cave mouths' openings 0x1AE and 0x1DB and the void 0x244,
`tile_atlas.rs` `is_black_land`) takes no part in coverage nor in the EC or
Classic transitions, in any source: its edge stays clean, no dirt or rock fading
into its black nor its black into them, as the Kingdom Reborn client draws a cave
floor's jagged edge against the opening. The cave exit frames (0x1D3-0x1DA,
0x7EC-0x7F1, 0x834-0x839, 0x8C1-0x8C6, `is_cave_exit_frame`), rock around a
black opening in their Classic texmaps, are drawn by the Enhanced Client and
Kingdom Reborn with all-black materials (44 and 45 "cave exit", 154 "rock hole").
From any source they never blend on the land of the player's cave's mouth only
(`cell_terrain_flags`, a cell's `TERRAIN_SHADER_FLAG_NEVER_BLENDS`, which the
coverage in `land/coverage.wesl` and `coverage/cache.rs` and the Classic
transitions honour per cell): there the dirt below a steep frame, as at the
Minoc mine's mouth, does not stretch up its black. Everywhere else they blend:
seen from outside, the opening's black fades softly into the rock around it.
In the player's cave the opening is see-through: on its mouth's land the cells
that never blend, the black openings and the frames (`land/main.wesl`
`cave_mouth_opening`), are left out of the land and drawn by the cave mouth veil
(`terrain/cave_mouth_veil.rs`), the land material blended over what lies behind it,
the cave's floor or the dimmed land outside, and under what stands in front of
it, the entrance arches. A darkness key over its texture's linear luminance
(`cave_mouth_veil_opacity`) draws the black at `CAVE_MOUTH_OPENING_OPACITY`
(0.65) and the rock opaque, ramping between `CAVE_MOUTH_KEY_BLACK_LUMINANCE`
(0.006) and `CAVE_MOUTH_KEY_ROCK_LUMINANCE` (0.02): the Classic frame texmaps
paint their opening at most 0.003 and their rock from 0.02, almost nothing
between, so a Classic frame's rock stays solid around a see-through opening;
the Enhanced and Kingdom Reborn frames are black throughout. The black reads as
a tunnel (`cave_mouth_tunnel_opacity`): `CAVE_MOUTH_OPENING_OPACITY` at the
z-view's land cut (`SceneUniform::land_max_z`), `CAVE_MOUTH_TUNNEL_INSIDE_OPACITY`
(0.35) toward the cave below it and `CAVE_MOUTH_TUNNEL_OUTSIDE_OPACITY` (0.9)
toward the outside above it, over `CAVE_MOUTH_TUNNEL_Z_SPAN` (12) z either side.
It fades in from nothing at the opening's border with the land around it over
`CAVE_MOUTH_FEATHER_TILES` (half a tile; `cave_mouth_feather`, from the eight
neighbours' terrain flags), so the opening has no straight cut. Under each veil
its backdrop draws the same quads black at the far plane before every other
blended draw, with the same feather: where nothing lies behind the opening it
shows black, not the clear colour. This holds in every style, Classic too. Seen
from outside, and in other caves, nothing changes.

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
