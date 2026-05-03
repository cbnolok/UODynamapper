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

The classic client could not fully synthesize visually pleasing transitions between terrain materials from the base tiled textures alone.

As a result, `art.mul` contains land-art tiles for:

- flat terrain renderings
- terrain-to-terrain transitions
- overlays used to hide abrupt seams between neighboring terrain materials

These transition tiles were manually authored content. They were placed on top of the underlying textured ground to preserve the original game's intended appearance.

This is why a renderer that uses only `texmaps.mul` can reproduce the base terrain material, but still miss fidelity-critical seam-hiding transition artwork.

## 4. Implications For UODynamapper

Current practical state:

- UODynamapper currently renders terrain from `texmaps.mul` data only.
- This is sufficient for the main textured ground path.
- It is not sufficient for full classic-fidelity land transitions.

Immediate consequence:

- To match original content more closely, the renderer needs support for classic land art tiles from `art.mul`, at least for transition tiles.

Architectural consequence:

- "Terrain" is not a single asset source.
- The runtime needs to treat these as separate but cooperating inputs:
  - logical terrain ids and corner heights from the map
  - repeatable terrain textures from `texmaps.mul`
  - transition and flat land art from `art.mul`

## 5. How This Helps Interpret EC Files

The classic split is a useful warning against simplistic assumptions in the EC pipeline.

Important takeaways:

- A low numeric id range does not automatically mean "terrain-only" visuals.
- A terrain-related visual may live in a shared texture pool and still require metadata to classify it correctly.

Packages:

- `Texture.uop` / `build/worldart/*.dds` is a mixed pool, not a pure terrain-only pool.
- `tileart.uop` shader/type information helps distinguish sprite, water, and static-terrain usage.
- terrain packaging decisions are likely more correct when driven by terrain definitions or shader semantics than by simple id slicing.

That aligns with the current UODynamapper investigation: `ec_land.uddp` should probably be defined by terrain semantics, not just by scanning the first `0x4000` generic texture ids.

## 6. Classification Matrix For This Repo

The safest rule is to classify by logical owner first, then use shader and flag information to subtype the asset.

### 6.1 Primary Rule

- If the record belongs to the map land pipeline, classify it as `land`.
- If the record belongs to tileart or item tiledata, classify it as `art`.
- Do not let visual similarity override logical ownership.

This matters because the same motif can legitimately exist in both domains:

- wood terrain used as actual map ground
- wood floor art used as a flat object on docks, bridges, platforms, or below terrain

### 6.2 Repo-Specific Ownership Sources

Use these sources as the first discriminator:

| Source | Classify As | Why |
| ------ | ----------- | --- |
| `map*.mul` land ids + classic land tile table | `land` | Authoritative ground ownership |
| `TileMetaLandTile` / `texture_id` | `land` | Current unified terrain path stores land material textures here |
| `terrain.toml` / `TerrainDefinition` entries | `land` | Explicit terrain semantics for EC-style land rendering |
| classic item tiledata | `art` | Object/floor/bridge/static ownership |
| EC `tileart.uop` -> `ArtData` | `art` | Tileart is item/static-oriented, even when the visual looks like terrain |
| `TileMetaItemTile` / `ec_texture_id` / `cc_texture_id` | `art` | Current unified item path stores object art references here |

### 6.3 Shader Interpretation Rules

EC shader names are useful semantic hints, but they do not override ownership:

- `UOWaterShader`: water-like visual semantics
- `UOStaticTerrainShader`: terrain-like flat or ground-like object semantics
- `UOSpriteShader`: regular sprite/static semantics, unless stretch rules imply a flatter solid surface

In this repo, the current EC parser already maps those names to `TileType` in `uocf/src/enhanced/tileart.rs`.

Practical rule:

- if a record comes from `tileart.uop`, keep it in the `art` domain even when its shader is `UOStaticTerrainShader`
- use the shader only to subtype the art record as floor-like, liquid-like, or regular static-like

That avoids misclassifying bridge decks, suspended floors, under-terrain floors, and terrain-looking object overlays as map land.

### 6.4 Flag Interpretation Rules

Classic and EC tile flags should be used as secondary evidence for subtype, traversal behavior, and rendering policy.

Useful examples:

- `surface`: walkable flat support surface
- `bridge`: bridge/ramp-like support semantics
- `wet`: liquid-adjacent or water-like content
- `background`: often flat/decal-like or floor-like placement behavior
- `wall` / `roof`: structural art, not land

Practical rule:

- `surface` and `bridge` strongly suggest floor-like or deck-like object art when the owner is tileart/item data
- `wet` plus water-oriented shader suggests liquid-style treatment, but still not necessarily map land if owned by tileart
- `wall` and `roof` should rule out land classification immediately

Flags are therefore best treated as:

- render-behavior hints
- traversal hints
- subcategory hints inside the `art` domain

They are not the primary source of truth for deciding `land` versus `art`.

### 6.5 Decision Table

| Condition | Result | Notes |
| --------- | ------ | ----- |
| Referenced by map land id / `TileMetaLandTile` / `terrain.toml` | `land` | Authoritative terrain ownership |
| Referenced by classic item tiledata or EC `ArtData` | `art` | Even if flat, floor-like, or terrain-looking |
| EC tileart record with `UOStaticTerrainShader` | `art`, subtype `terrain-like floor art` | Do not promote to map land automatically |
| EC tileart record with `UOWaterShader` | `art`, subtype `liquid-like art` | Useful for special render handling, still item-owned |
| EC tileart record with `UOSpriteShader` and walkable/surface-like flags | `art`, subtype `floor or deck art` | Covers docks, platforms, suspended floors |
| `wall` or `roof` flags present | `art` | Strong negative evidence against land |
| Same EC `worldart` texture id used by both domains | keep both semantic references | Shared texture content is valid; do not force exclusivity |

### 6.6 How To Rule Out False Positives

Do not classify as `land` merely because:

- the texture lives under `build/worldart/...`
- the numeric id is low
- the art looks like a flat diamond ground tile
- the object is walkable

Those signals are insufficient on their own. They are compatible with:

- suspended wooden floors
- dock segments over water
- under-terrain support floors
- transition overlays
- ground-like decorative statics

The owner record must win.

### 6.7 Packaging Rule

For future `ec_land` and `ec_art` packaging, use this policy:

- `ec_land`: package terrain-owned references only
- `ec_art`: package tileart/item-owned references only
- if a texture id is referenced by both, duplication is acceptable, but semantic ownership must stay distinct

Long-term, a shared EC texture pool with separate semantic lookup tables would be cleaner than pretending all ids are exclusive to one domain.

Current branch direction:

- treat EC `Unused1` as the primary land hint when building the new semantic translation table
- if `Unused1` is absent, fall back to shader/type, ownership, and terrain-family hints
- keep `runtime_material_id_overrides.toml` narrow and runtime-only
- use `TerrainTranscode.json` as the starting point for the hand-tuned Classic land family table

## 7. Renderer Guidance

Recommended interpretation for future work:

- Keep `texmaps.mul` as the primary source for textured terrain.
- Add classic `art.mul` land-art support for at least transition tiles.
- Treat EC terrain extraction as a classification problem, not just a file-prefix or id-range problem.
- Preserve the distinction between:
  - terrain material textures
  - land art / transition overlays
  - static world art sprites

This distinction matters for both fidelity and package design.

Future packaging work is expected to split EC textures into land, art, and auxiliary layer outputs, so this doc should be read as the semantic-ownership reference for those outputs rather than a statement that the current package layout is final.
