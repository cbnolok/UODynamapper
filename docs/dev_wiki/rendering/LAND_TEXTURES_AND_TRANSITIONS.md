# Land Textures And Transition Tiles

This note captures the practical distinction between land textures and land art in Ultima Online, plus the consequences for UODynamapper's current renderer and future EC/CC asset interpretation.

## 1. Classic Client Ground Rendering Model

Classic UO splits ground visuals across two different asset families:

- `map*.mul` stores the logical terrain grid. Each tile contributes:
  - a 16-bit land tile id
  - an 8-bit altitude for the tile's north-west corner
- `texmaps.mul` stores repeatable terrain textures used when the ground is rendered as a textured quad
- `art.mul` stores pre-rendered land artwork, including:
  - flat "raw" land tiles
  - land transition tiles
  - other land-art overlays the original client could not synthesize at runtime

The map height model is corner-driven rather than tile-center-driven. For a tile at `(x, y)` the four rendered corner heights come from:

- north-west: `(x, y)`
- north-east: `(x + 1, y)`
- south-west: `(x, y + 1)`
- south-east: `(x + 1, y + 1)`

On wrapped map edges, the client samples the adjacent corners from the opposite side of the map.

## 2. Flat Tiles Versus Textured Quads

Classic ground rendering chooses between two fundamentally different visual paths:

- If all four corner heights are equal, the client can draw a pre-rendered isometric land art tile from `art.mul`.
- If the four corner heights differ, the client draws a textured ground quad using data from `texmaps.mul`.

Useful historical details:

- The pre-rendered "raw tile" footprint is `44x44` pixels in screen space.
- The textured terrain quad is also `44` pixels wide in isometric projection.
- Corner screen positions shift by roughly `4` pixels for each `+/-1` altitude delta.
- The textured path uses `64x64` or `128x128` terrain textures from `texmaps.mul`.

This explains why a land tile id is not enough to determine the final visual. The client chooses between art and textured geometry based on local height relationships.

## 3. Why Transition Tiles Exist In `art.mul`

The classic client could not fully synthesize visually pleasing transitions between land materials from the base tiled textures alone.

As a result, `art.mul` contains land-art tiles for:

- flat terrain renderings
- terrain-to-terrain transitions
- overlays used to hide abrupt seams between neighboring land materials

These transition tiles were manually authored content. They were placed on top of the underlying textured ground to preserve the original game's intended appearance.

This is why a renderer that uses only `texmaps.mul` can reproduce the base land material, but still miss fidelity-critical seam-hiding transition artwork.

## 4. Implications For UODynamapper

Current practical state (`dynamapper/src/core/render/scene/world/land/draw_mesh.rs`, `dynamapper/src/core/texture_cache/land.rs`, `dynamapper/assets/shaders/world/land/`):

- Classic land mode (`land_texture_source = cc`) draws every land cell as a textured quad from `tex_land_cc.uddp` (`texmaps.mul`): the tile's tiledata texture id, or the tile id when that is 0. A cell without a packaged texmap is drawn as missing. Without the package, texmaps stream from the client's `texmaps.mul` (the same images).
- Classic land art from `art.mul` (`tex_art_cc.uddp`) is drawn in one case only, through `classic_land_art_atlas`: a **flat wet** cell (IsWet tiledata flag, all four corners at the same height) whose texture id is 0 or its own tile id (`classic_land_art_payload_id`, `land.rs`; `use_classic_flat_wet_land_art`, `main.wesl`). Shoreline statics and the water under them then share the same source pixels. Sloped wet cells keep the texmap. The array holds those slots as RGBA8, or as their BC7 blocks when `tex_art_cc` is BC7.
- **Dry transition land art is not drawn.** Dry cells always use their texmap. The optional Classic smoothing (`cc_terrain_transitions`, off by default; `cc_transitions.wesl`) blends neighbouring dry texmaps procedurally across cell edges. It is not the authored transition art.
- Enhanced land mode (`land_texture_source = ec`) draws terrain from `tex_land_ec.uddp` (TerrainDefinition materials and their layers) with base/detail/mask roles in its land pages. Auxiliary item inputs come from `tex_aux_ec.uddp`; the separate terrain-layer-table path is not evidence of live two-base composition (see the usage audit below). An unresolved EC route stays explicitly missing; it never falls back to texmaps or art.
- Wetness: in Classic mode a land cell is wet from the IsWet tiledata flag. In Enhanced mode it is wet when its TerrainDefinition material is the water layer class (`UOWaterTerrainLayer`, `TERRAIN_SHADER_FLAG_REVIEWED_LIQUID`); the tiledata flag is not used there. On the shipped clients every IsWet land tile is also a water-layer tile (67 of 146).

Consequences:

- Classic mode reproduces the base land materials and the flat wet art, but not the authored dry transition tiles, so Classic seams between dry materials differ from the original client.
- Drawing those transitions would need the map-side choice the original client makes (art for flat cells, textured quads for sloped ones) applied to dry cells too, with the art from `tex_art_cc`.

Architectural consequence:

- "Land" is not a single asset source.
- The runtime treats these as separate but cooperating inputs:
  - logical land ids and corner heights from the map
  - repeatable land textures from `texmaps.mul` (Classic) or TerrainDefinition materials (Enhanced)
  - flat land art from `art.mul`, today only for flat wet cells

## 5. EC ownership and routing

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
explicit overlay selects another material. Missing EC terrain remains black;
there is no implicit Classic texmap fallback.

Texture roles follow the source shader/layer class and item position. Names,
stretch and visual resemblance do not select the base or establish a role.
`Rotated` and stretch affect item rendering; neither promotes an item to map
land. Source flags, logical signed Z, tile heights and movement data must remain
separate from visual geometry and painter priority.

Use the focused contracts for implementation details:

- [EC evidence map](EC_WORLD_SPACE_UV_EVIDENCE_MAP.md): executable findings and proof limits.
- [Source routing](../../agent_notes/ec_source_terrain_routing.md): GameData routing, overlays and active material paths.
- [Terrain layer table usage](../../agent_notes/terrain_layer_table_usage.md): why table presence does not establish shader reachability.
- [UDDP formats](../UDDP_FORMATS.md): current package schemas and role tables.
- [EC sprite geometry](../../agent_notes/ec_sprite_descriptor_geometry.md) and
  [art trimming](tex_art_ec_trimming.md): source selection, windows, scale and placement.

The former KR routing proposal and heuristic classification matrix are retired.
A separate KR routing/material model still requires original KR evidence; EC
pool containment or third-party reconstruction does not establish that model.
