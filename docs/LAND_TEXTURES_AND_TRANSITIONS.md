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

Current practical state:

- UODynamapper currently renders terrain from `texmaps.mul` data only.
- This is sufficient for the main textured ground path.
- It is not sufficient for full classic-fidelity land transitions.

Immediate consequence:

- To match original content more closely, the renderer needs support for classic land art tiles from `art.mul`, at least for transition tiles.

Architectural consequence:

- "Land" is not a single asset source.
- The runtime needs to treat these as separate but cooperating inputs:
  - logical land ids and corner heights from the map
  - repeatable land textures from `texmaps.mul`
  - transition and flat land art from `art.mul`

## 5. How This Helps Interpret EC Files

The classic split is a useful warning against simplistic assumptions in the EC pipeline.

Important takeaways:

- A low numeric id range does not automatically mean "land-only" visuals.
- A land-related visual may live in a shared texture pool and still require metadata to classify it correctly.
- The numeric vpath/id overlap between EC and CC is useful for locating payloads, but it is not sufficient to decide whether a tile belongs to land, static art, or a flat floor-like static render path.

Packages:

- `Texture.uop` / `build/worldart/*.dds` is a mixed pool, not a pure land-only pool.
- `LegacyTexture.uop` is part of the same source bundle and supplies the EC-side classic-style image references.
- `tileart.uop` is the authoritative EC static-art source for ownership, clipping windows, and item-side metadata.
- `TerrainDefinition.uop` is the authoritative EC land source for material semantics, selected textures, alias chains, and canonical slot relationships.
- `string_dictionary.uop` is a required support package for resolving virtual paths, but itself is not a visual source.
- EC art and EC land are both semantic outputs from the same shared texture pools, so the same raw texture id can legitimately appear in both packages when ownership differs.

That aligns with the current UODynamapper investigation: `tex_land_ec.uddp` should be defined by land semantics, not by scanning a numeric id range or assuming every flat-looking `worldart` texture is land-owned.

### 5.0.1 Missing EC Terrain Must Stay Explicit

When the renderer is in EC/enhanced land mode, an unresolved EC land route is a data or resolver failure, not a rendering style choice.

Do not silently fall back from a missing EC land texture to the Classic Client texmap path. Black or otherwise explicit missing output is preferred during investigation because it preserves the failure signal and keeps bad EC routing visible. A fix should identify the missing land ids, trace them through `TerrainTranscode.kdl`, embedded `tex_land_ec.uddp` transcode metadata, `TerrainDefinition.uop` provenance, and override data, then repair the incorrect data or resolver behavior.

Classic fallback is acceptable only as an explicitly named user-facing compatibility mode or diagnostic mode. It must not be the default behavior for EC land rendering regressions.

## 5.1 KR And EC Land Relationship

Kingdom Reborn and Enhanced Client land evidence should be kept separate until a specific runtime mode intentionally merges them.

Verified package relationship from `kr-ec-terrain-diff-tool` using:

- KR: `/mnt/dati/_proj_local/_uo_clients/UO - Old KR/`
- EC: `/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced fp/`
- KR routing: `KrFacetTranscode.generated.kdl`
- EC routing/material evidence: `EcFacetTranscode.generated.kdl`

Observed texture-pool containment:

| Package | KR files | EC files | KR files absent from EC | Meaning |
| ------- | -------- | -------- | ----------------------- | ------- |
| `Texture.uop` | 9348 | 9798 | 0 | EC appears to be a superset of KR for this pool. |
| `TerrainTexture.uop` | 20 | 38 | 0 | EC appears to be a superset of KR support land textures. |
| `LegacyTexture.uop` | 16877 | 52430 | 2 | EC is nearly a superset; two KR classic-style hashes need separate review if referenced. |

Observed routing relationship:

- The Manawydan KR dictionary agrees with the current `TerrainTranscode.kdl` semantic family routing.
- EC facet comparison against the matching FP Classic client is mostly direct/id-preserving: `cc -> ec facet id`.
- EC `TerrainDefinition.uop` then maps that facet id through aliases to EC material definitions.
- KR and EC often choose different material ids for the same Classic land id. In the first generated report, 288 rows were comparable through EC TerrainDefinition aliases; 105 matched the KR material id and 183 differed.

This means:

- KR material routing is not just "EC routing with older packages".
- EC texture packages are probably sufficient as a physical image pool for many KR land experiments.
- A separate `tex_land_kr.uddp` should not be added until a report proves that KR uses visible texture refs or layer roles that cannot be represented by EC packages plus KR routing metadata.

Recommended near-term KR model:

```text
Classic land id
  -> KR routing KDL material/family id
  -> EC texture pool lookup where compatible
  -> KR-specific material/layer metadata only where proven
```

Recommended evidence files:

- `KrFacetTranscode.generated.kdl`: generated KR dictionary/facet evidence; use compact `t cc=... kr=...` rows.
- `EcFacetTranscode.generated.kdl`: generated EC facet and TerrainDefinition evidence; use compact `t cc=... ec=...` rows plus material/layer evidence.
- `KrTerrainRouting.generated.kdl`: generated review evidence grouped as `t KR_MATERIAL_ID CC_LAND_ID...`; it is not an active runtime routing mode while it remains equivalent to `TerrainTranscode.kdl` for overlapping ids.
- `KrEcTerrainDiff.generated.csv`: generated comparison report; do not hand-edit as runtime policy.

If future evidence proves a meaningful runtime KR split, use this KDL shape:

```kdl
// Generated or reviewed KR routing. Grouped rows keep the runtime table compact.
route client="kr" source="manawydan-tile-dictionary"

t 5 168 169 170 171

material kr=5 {
    // Optional only after KR TerrainDefinition or shader evidence is decoded.
    // Keep physical texture refs out of this file until the role is proven.
    alias cc=168
    alias cc=169
}
```

Design rules for this routing file:

- Use grouped `t TARGET_ID SOURCE_ID...` rows for runtime routing tables.
- Keep evidence files free to use `t cc=... kr=...` / `t cc=... ec=...` rows when preserving observed counts.
- Use `code` or `source` fields to distinguish generated dictionary evidence from reviewed manual corrections.
- Do not copy EC TerrainDefinition layers into KR routing unless the KR source package proves the same relationship.
- Let manual/reviewed routing override generated routing in file order, matching the existing `TerrainTranscode.kdl` conflict behavior.
- Keep the routing KDL separate from `EcTerrainOverrides.kdl`; the latter is about EC material behavior, not KR family routing.
- Use `udd-pack pack-ec-textures --land-transcode-kdl <routing.kdl>` when intentionally embedding a non-default routing table into `tex_land_ec.uddp`.

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
| `TileMetaLandTile` / `texture_id` | `land` | Current unified land path stores land material textures here |
| `terrain.toml` / `TerrainDefinition` entries | `land` | Explicit land semantics for EC-style land rendering |
| classic item tiledata | `art` | Object/floor/bridge/static ownership |
| EC `tileart.uop` -> `ArtData` | `art` | Tileart is item/static-oriented, even when the visual looks like terrain |
| `TileMetaItemTile` / `ec_texture_id` / `cc_texture_id` | `art` | Current unified item path stores object art references here |

Additional EC source guidance:

- `Texture.uop` and `LegacyTexture.uop` are shared texture pools and should be treated as the common decode backend.
- `tileart.uop` contributes entry-local EC art semantics: shader type, tile type, ownership, sampling windows, and item-specific behavior hints.
- `TerrainDefinition.uop` contributes EC land semantics: land source texture ids, aliases, selected textures, runtime slots, and provenance.
- `string_dictionary.uop` is an auxiliary package resource, not art classification input.

### 6.3 Shader And Flag Interpretation Rules

EC shader names are useful semantic hints, but they do not override ownership:

- `UOWaterShader`: water-like visual semantics
- `UOStaticTerrainShader`: land-like flat or ground-like object semantics
- `UOSpriteShader`: sprite/static semantics, but some entries still need land-style flat rendering

The verified rule in this repo is:

- if a record comes from `tileart.uop`, keep it in the `art` domain even when its shader is `UOStaticTerrainShader`
- use shader plus flags to subtype the art record as floor-like, liquid-like, roof-like, or regular static-like

The important reverse-engineered refinement is that `UOSpriteShader` is not equivalent to "billboard sprite".
Some EC tileart entries use `UOSpriteShader` while still representing flat floor or roof visuals that the client renders as a surface-like tile.

Current parser rule:

- `UOWaterShader` -> `TileType::Liquid`
- `UOStaticTerrainShader` -> `TileType::Solid`
- `UOSpriteShader` with `Unused1` -> `TileType::Solid`
- `UOSpriteShader` with primary texture `stretch != 1.0` -> `TileType::Solid`
- otherwise -> `TileType::Static`

This rule was derived from verified EC examples such as wood boards (`1211`), sandstone floor (`2077`), and palm-frond roof (`1510`): all are flat 44x44 classic-style tiles with zero offsets, all carry `Unused1`, and all must be treated as surface-like in the EC path even though they do not come through `UOStaticTerrainShader`.

That avoids misclassifying bridge decks, suspended floors, under-land floors, roofs, and land-looking object overlays as map land while still keeping them out of the billboard/static path.

### 6.4 Flag Interpretation Rules

Classic and EC tile flags should be used as secondary evidence for subtype, traversal behavior, and rendering policy.

Useful examples:

- `surface`: walkable flat support surface
- `bridge`: bridge/ramp-like support semantics
- `wet`: liquid-adjacent or water-like content
- `background`: often flat/decal-like or floor-like placement behavior
- `wall` / `roof`: structural art, not land
- `unused1`: verified EC-side hint that some `tileart.uop` entries should be rendered as flat surface-like tiles even when their shader is `UOSpriteShader`

Practical rule:

- `surface` and `bridge` strongly suggest floor-like or deck-like object art when the owner is tileart/item data
- `wet` plus water-oriented shader suggests liquid-style treatment, but still not necessarily map land if owned by tileart
- `wall` and `roof` should rule out land classification immediately
- `unused1` should be treated as a secondary EC render-mode hint, not as proof of land ownership

Flags are therefore best treated as:

- render-behavior hints
- traversal hints
- subcategory hints inside the `art` domain

They are not the primary source of truth for deciding `land` versus `art`.

In other words: `Unused1` says "render this tileart entry like a flat surface", not "this texture belongs to the land material system".

### 6.5 Decision Table

| Condition | Result | Notes |
| --------- | ------ | ----- |
| Referenced by map land id / `TileMetaLandTile` / `terrain.toml` | `land` | Authoritative land ownership |
| Referenced by classic item tiledata or EC `ArtData` | `art` | Even if flat, floor-like, or land-looking |
| EC tileart record with `UOStaticTerrainShader` | `art`, subtype `land-like floor art` | Do not promote to map land automatically |
| EC tileart record with `UOWaterShader` | `art`, subtype `liquid-like art` | Useful for special render handling, still item-owned |
| EC tileart record with `UOSpriteShader` and `Unused1` | `art`, subtype `surface-like floor/roof art` | Covers flat boards, roofs, sandstone floor tiles, and similar EC land-like statics |
| EC tileart record with `UOSpriteShader` and walkable/surface-like flags | `art`, subtype `floor or deck art` | Covers docks, platforms, suspended floors |
| `wall` or `roof` flags present | `art` | Strong negative evidence against land |
| Same EC `worldart` texture id used by both domains | keep both semantic references | Shared texture content is valid; do not force exclusivity |

### 6.6 How To Rule Out False Positives

Do not classify as `land` merely because:

- the texture lives under `build/worldart/...`
- the numeric id is low
- the numeric vpath matches a classic art id
- the art looks like a flat diamond ground tile
- the tile carries `Unused1`
- the object is walkable

Those signals are insufficient on their own. They are compatible with:

- suspended wooden floors
- dock segments over water
- under-land support floors
- transition overlays
- ground-like decorative statics

The owner record must win.

### 6.7 Packaging Rule

For future `tex_land_ec` and `tex_art_ec` packaging, use this policy:

- `tex_land_ec`: package land-owned references only
- `tex_art_ec`: package tileart/item-owned references only
- if a texture id is referenced by both, duplication is acceptable, but semantic ownership must stay distinct

Long-term, a shared EC texture pool with separate semantic lookup tables would be cleaner than pretending all ids are exclusive to one domain.

Current branch direction:

- keep land ownership anchored in `TerrainDefinition.uop`
- keep item/static ownership anchored in `tileart.uop`
- use EC `Unused1` plus shader/type to decide whether a tileart entry is surface-like and should avoid the billboard/static path
- use `TerrainTranscode.kdl` as the starting point for the hand-tuned Classic land family table

That direction is specifically aimed at reconciling the shared EC texture pools with distinct semantic outputs instead of pretending the pools are disjoint.

## 7. Renderer Guidance

Recommended interpretation for future work:

- Keep `texmaps.mul` as the primary source for textured land.
- Add classic `art.mul` land-art support for at least transition tiles.
- Treat EC land extraction as a classification problem, not just a file-prefix or id-range problem.
- Preserve the distinction between:
  - land material textures
  - land art / transition overlays
  - static world art sprites

This distinction matters for both fidelity and package design.

Future packaging work is expected to split EC textures into land, art, and auxiliary layer outputs, so this doc should be read as the semantic-ownership reference for those outputs rather than a statement that the current package layout is final.
