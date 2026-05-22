# Enhanced Client Asset Reference

This document explains the EC map/static material pipeline as implemented in
this repo. It is written for contributors and agents who need to change the
pipeline without reintroducing the earlier texture-selection bugs.

The main rule is: keep source facts, derived metadata, runtime policy, and
manual overrides separate. A numeric texture id, file path, or UOP package name
is evidence, not by itself a rendering decision.

## 1. Evidence Order

Use this order when deciding how EC terrain or statics should behave:

1. Original EC UOP files are primary evidence.
2. Packed UDDP metadata is derived runtime evidence.
3. Active KDL files are reviewed integration or override data.
4. Legacy KDL files are review history only.
5. Current Rust/WGSL behavior is implementation, not source truth.

Exploration and audit outputs belong in tooling paths, not in normal pack code,
unless the data becomes a stable package contract.

## 2. EC Source Files

`string_dictionary.uop`:

- Stores strings used by many EC records.
- Most source records store 1-based string offsets. The parser resolves offset
  `n` through dictionary index `n - 1`.
- Typical values are virtual paths such as `Data\WorldArt\02000130_...tga`,
  shader names such as `UOStaticTerrainShader`, or material names.

`facet*.uop`:

- Stores EC map chunks under paths like `build/sectors/facet_0M/XXXXXXXX.bin`.
- Each decompressed `.bin` is a 64x64 tile chunk in column-major order.
- Header: facet id byte and file id word.
- Each cell stores land z, land graphic id, delimiter records, and static
  records. Static records store graphic id, z, and hue.
- The decoder converts this to classic-style 8x8 map blocks plus statics. The
  land graphic id from the facet/map is the terrain query id used by runtime.

`TerrainDefinition.uop`:

- Owns EC terrain/material semantics.
- Each entry currently decodes:
  - `name_id`
  - `id` material id
  - `unk`, `unk2`, `unk3` floats, preserved but not interpreted
  - alias records: `count_index`, `alias`, `tile_flags`
  - optional texture block
- Alias records are the bridge from map/facet terrain ids to EC material ids.
  Non-zero aliases are runtime terrain slot ids. If an entry has no concrete
  aliases, the material id itself is used as a fallback runtime slot id.
- The optional texture block is the same general shape as `TextureItem`:
  - `unk1`, preserved but not currently used as policy
  - `shader_name_id` / resolved `shader_name`
  - layered image refs, each with `name_string_off`, resolved path,
    extracted `texture_id`, logical family, `unk4`, `texture_repetition`,
    `unk6`, and `unk7`
  - trailing `unk8` integer vector and `unk9` float vector
- `texture_repetition` is the authored stretch/repetition factor. Runtime uses
  it for EC terrain layer sampling; losing it shrinks large authored textures
  into a tiny tile.
- Current interpretation of terrain texture unknowns:
  - `unk1` appears to be texture-block mode/class metadata, but it is not
    needed for the current material routing.
  - `unk4` is preserved per image ref; no rendering meaning is assigned yet.
  - `unk6` has ordering correlation inside otherwise equivalent layer choices,
    so it is used only as a deterministic final tie-breaker and audit signal.
  - `unk7`, `unk8`, and `unk9` are preserved for future audits.

`tileart.uop`:

- Owns EC item/static semantics, including flat surface-like statics.
- Raw records include tile id, two boolean fields, several unknown numeric
  fields, old id, `type_val`, lighting fields, two 64-bit flag fields, facing,
  EC and CC image windows/offsets, property vectors, stack aliases, appearance
  metadata, optional sitting metadata, radar color, and four texture blocks.
- Texture block 0 is used as the primary EC visual block. Texture block 1 is
  used as the preferred classic/legacy visual block. Blocks 2 and 3 are
  preserved as referenced texture data when present.
- Each texture item stores string offset, `texture_stretch`, `unk4`, `unk6`,
  and `unk7`. These are copied into `tilemeta.uddp` texture-ref metadata.
- Current tile type classification is render-oriented:
  - `UOWaterShader` -> liquid
  - `UOStaticTerrainShader` -> solid/surface-like
  - `UOSpriteShader` + `Unused1` flag -> solid/surface-like
  - `UOSpriteShader` + first texture stretch not equal to `1.0` ->
    solid/surface-like
  - otherwise regular static art
- Important: current code does not use texture-block `unk1` as the switch for
  land-style rendering. Prior diagnostics showed this signal is fairly
  consistent for surface-like/static terrain-style tileart, but the current
  pipeline already resolves those cases through shader/flag/stretch
  classification plus texture provenance. Keep `unk1` documented and preserved;
  do not add it to routing unless the current evidence path stops covering a
  real case.
- Current interpretation of tileart texture unknowns:
  - texture-block `unk1` likely encodes a block mode/class. It is useful audit
    evidence, not active policy.
  - item `unk4` is preserved per texture ref; no rendering meaning is assigned.
  - item `unk6` and `unk7` are preserved in `tilemeta.uddp`. They may encode
    sampler/channel/blend parameters, but current routing does not depend on
    them.

`Texture.uop` and `LegacyTexture.uop`:

- Shared image pools, not ownership boundaries.
- `Texture.uop` usually resolves `build/worldart/{:08}.dds`.
- `LegacyTexture.uop` usually resolves `build/tileartlegacy/{:08}.dds`.
- Payloads are raw DDS or TGA image data. Tooling decodes them for packing, but
  the original payload is the source evidence.

## 3. Ownership Model

Ownership and render role are separate axes.

- `TerrainDefinition.uop` owns terrain/material semantics.
- `tileart.uop` owns item/static semantics.
- `facet*.uop` owns placed map terrain ids, heights, statics, and static hues.
- `Texture.uop`, `LegacyTexture.uop`, `TerrainTexture.uop`, and
  `EffectTexture.uop` are physical package pools, not semantic owners.

Consequences:

- A low texture id does not prove terrain ownership.
- A `Data\WorldArt\...` path does not prove terrain ownership.
- A `Data\Textures\...` path can be an art-owned support reference.
- The same decoded texture id can appear in both art and land packages when
  different semantic owners need it.

## 4. Terrain Texture Selection

Terrain packing starts from `TerrainDefinition.uop`, not from classic tile ids
or raw texture package membership.

The current primary visible texture selector ranks layers in this order:

1. Layers with a resolved texture id.
2. Non-support names before support names.
3. Preferred repetition before non-preferred repetition.
4. Lower `unk6`.
5. Lower `name_string_off`.

Current support-name heuristic:

- Support for primary exclusion: path contains `noise`, `normal`, `mask`, or
  `_alpha`.
- Wider support-like diagnostic clue: path contains `alpha`, `mask`, `noise`,
  `normal`, `bump`, `ripple`, `flow`, or `distort`.

Preferred repetition is `4.0..=8.0`. This matches the observed EC terrain
albedo/detail layers better than choosing the first layer blindly. It prevents
the old failures where an alpha mask, normal map, displacement-like texture, or
red diagnostic-looking support texture became the visible terrain base.

`unk6` is only a deterministic final tie-breaker after the support and
repetition rules. It is not treated as semantic policy. The packer marks
`TERRAIN_PRIMARY_FLAG_OPAQUE_UNK6_TIEBREAKER` when multiple layers have the
same current rank, so those cases stay visible for audit. Do not promote `unk6`
unless an audit proves it works in at least 3/4 of reviewed cases.

Reason codes written to metadata:

- `non_support_preferred_repetition`
- `non_support_repetition_fallback`
- `support_preferred_repetition_fallback`
- `support_repetition_fallback`

Diagnostic flags written to metadata include:

- selected current support layer
- selected support-like layer
- selected preferred repetition
- fallback reason
- multiple preferred non-support layers
- support-like clue outside current heuristic
- opaque `unk6` tie-breaker

## 5. Layer Roles

For EC terrain runtime blending, layer index currently matters:

- Layer 0 is treated as base/albedo.
- Layer 1 is treated as detail/grunge/secondary albedo.
- Layer 2 is treated as mask/alpha blend control.

This is a runtime interpretation of the layer graph preserved from
`TerrainDefinition.uop`. The packer does not flatten layers into one image. It
packs all source texture ids it can decode, then writes provenance so runtime
can sample base/detail/mask with the authored stretch.

Normals, noise, masks, ripple/flow/distortion clues, and other support textures
are preserved as metadata and packed texture refs when available. They are not
all active shader inputs yet. Do not discard them just because the current
shader only uses base/detail/mask.

For EC tileart texture refs, stable roles are inferred from paths and selection
state:

- `normal` or `_n` -> normal-like
- `alpha` -> alpha mask
- `mask` -> generic mask
- `noise` -> noise
- `detail` -> detail
- `light` or `glow` -> overlay
- primary selected ref -> base
- auxiliary ref or `Data\Textures\...` -> image support
- otherwise unknown support

Tileart auxiliary detection currently treats names containing `noise`,
`normal`, or token segments `n`, `nm`, `nrm`, `norm` as auxiliary.

## 6. Surface-Like Statics

Surface-like statics are still tileart-owned. They render on the ground, but
that does not make them terrain materials.

Build-time flow:

- `tileart.rs` classifies EC tileart entries into regular static, solid
  surface-like, or liquid-like.
- `tilemeta.uddp` stores `TileMetaItemVisualKind::SurfaceLike` for non-static
  tileart classifications.
- It also stores all tileart texture refs, including logical family, physical
  package, stable role, speculative role, `texture_stretch`, `unk4`, `unk6`,
  `unk7`, block index, item index, and primary/auxiliary flags.

Runtime flow for EC surface-like statics:

- Prefer a land-atlas path over the regular EC art-atlas path.
- Ask `tilemeta` for the main EC texture id.
- For surface-like tiles, exact `texture_id == tile_id` matches are disabled
  during main texture selection. This avoids treating a CC art id as if it were
  an EC WorldArt texture id.
- If the selected texture id is directly present in `tex_land_ec.uddp`, use it.
- Otherwise, search terrain provenance for records with that selected texture
  id. If there is exactly one canonical slot, use it. If there is no canonical
  slot and exactly one alias slot, use that. Otherwise fall back through
  `resolve_runtime_slot_id(meta.cc_texture_id)`.
- If nothing resolves, skip the surface-like static and log samples.

This is why flat tileart entries such as marble floor corners can correctly use
the land-style texture from `Data\WorldArt\...` without hardcoding their ids.

### Static Water Selects Normal Maps Instead of Base Texture

Known current failure: some surface-like static water tiles route to an EC
normal/displacement-looking texture instead of the real visible water
base/albedo texture. In these cases the chosen texture refs are support normal
maps, not the water base texture that should be rendered. This is separate from
regular map water and from regular static art. The affected path is the
surface-like static land-atlas path:

- `statics_collect.rs::resolve_static_visual_kind`
- `resolve_surface_like_tex_land_ec_slot_id`
- `StaticVisualKind::TexLandEcArt`
- `ground_atlas.resolve_tex_land_ec`
- `assets/shaders/world/art/ground.wgsl`

Do not fix this by special-casing water ids in the shader. The wrong texture is
already selected before the shader samples it. The shader only receives the
resolved atlas layer and UVs.

What to verify first:

- Inspect the tileart entry for the affected static id in `tileart.uop`.
- List all texture refs in its texture blocks, including `shader_name`,
  `texture_stretch`, path, extracted `texture_id`, stable role, block index,
  item index, and primary/auxiliary flags.
- Confirm whether the visible water texture and the normal/displacement texture
  are both present in tileart metadata. If only the support texture is present,
  the missing albedo may have to be resolved through TerrainDefinition
  provenance or a reviewed override.
- Check whether the current `tilemeta.main_ec_texture_id` points at a ref whose
  stable role is `normal`, `mask`, `noise`, `alpha`, `ripple`, `flow`, or
  `distort`. If yes, the bug is in tileart main-texture selection.
- Check whether `resolve_surface_like_tex_land_ec_slot_id` reroutes a correct
  tileart texture id to a terrain provenance slot whose selected primary layer
  is support-like. If yes, the bug is in terrain primary selection or in the
  surface-like redirection resolver.
- Preserve `is_wet_flags`: the flag is for water animation and should not
  decide the visible albedo by itself.

Likely correct policy:

- For liquid surface-like statics, prefer tileart refs classified as visible
  albedo/base over support refs, exactly as terrain primary selection does.
- Treat names containing `normal`, `bump`, `mask`, `_alpha`, `noise`, `ripple`,
  `flow`, or `distort` as support for visible-primary selection unless a manual
  review proves otherwise.
- Keep `texture_stretch` from the selected visible ref or resolved terrain
  provenance slot; otherwise water scale and motion will look wrong even after
  the texture id is fixed.
- If the source data does not expose a visible albedo automatically, add a KDL
  override entry for that tile id rather than weakening the global heuristic.

Useful diagnostics to add before changing policy:

- Extend `audit-ec-surface-redirection` or add a targeted report for liquid
  surface-like statics that writes one row per candidate texture ref and marks
  the selected runtime slot.
- Include: `tile_id`, tile type, `is_wet`, shader name, ref path, role,
  support-like reason, texture id, physical package, block/item index,
  stretch/repetition, selected/unselected reason, resolved runtime slot, and
  whether the final slot came from direct land presence, canonical terrain
  provenance, alias terrain provenance, or fallback slot resolution.
- Test against the current failing water tiles and at least one known-good
  surface-like non-water tile, so the fix does not regress marble/floor routing.

## 7. Classic Id Ranges

Classic art ids below `0x4000` are land diamonds in classic art data. Classic
static art ids are stored at `item_id + 0x4000`.

Current handling:

- `tex_art_cc.uddp` decodes art ids `< 0x4000` as 44x44 land diamonds and ids
  `>= 0x4000` as statics.
- Runtime CC static rendering adds `0x4000` to the item graphic id before
  looking up the static art atlas.
- `tilemeta` radar color lookup also uses `id + 0x4000` for item/static radar
  entries.
- EC standard static art often keeps the same id as the CC item slot. Surface
  land-like art is the exception: it often points through tileart metadata to a
  separate EC WorldArt texture id.

Do not assume that a CC item id directly equals an EC terrain material id.
For regular static art it can often match the EC tileart id. For surface-like
art, use the tileart texture refs and terrain provenance resolver.

## 8. End-To-End Workflow

Terrain package build:

1. Load `TerrainDefinition.uop` and neighboring `string_dictionary.uop`.
2. Resolve material aliases, shader names, layer paths, texture ids, layer
   repetition, and preserved unknown fields.
3. Load `Texture.uop` and `LegacyTexture.uop`.
4. Collect every terrain source texture id, plus texture ids referenced by
   `EcTerrainOverrides.kdl`.
5. Decode those texture ids from the EC texture pools.
6. Pack decoded textures into atlas pages.
7. Build sparse slot records and alias records.
8. Write terrain provenance for every material alias and selected/available
   layer.
9. Embed KDL-derived override metadata and transcode metadata when present.

Tile metadata build:

1. Load classic `tiledata.mul` and optional classic radar colors.
2. Load EC `tileart.uop` definitions.
3. Build dense land metadata from classic land records.
4. Build dense item metadata from classic item records plus EC tileart records.
5. Store EC/CC texture ids, crop starts, offsets, visual kind, radar color,
   flags, and full EC texture-ref side tables.
6. Compute tileart texture roles from path names, selected primary refs,
   auxiliary clues, and package/family classification.

Runtime:

1. Load UDDP map/statics, `tilemeta.uddp`, `tex_art_cc.uddp`,
   `tex_art_ec.uddp`, and `tex_land_ec.uddp`.
2. Load loose `TerrainTranscode.kdl` and apply it over embedded transcode data.
3. Load loose `EcTerrainOverrides.kdl` and apply it over embedded terrain
   override defaults.
4. Build land shader lookup entries from terrain provenance and override refs.
5. For statics, choose regular art, CC art, or EC land-atlas rendering from
   source mode and `tilemeta` visual kind.

## 9. UDDP Packages

`tilemeta.uddp`:

- `metadata/land.bin`: dense `TileMetaLandTile` table from classic land
  metadata, mapped into EC-compatible flags.
- `metadata/items.bin`: dense `TileMetaItemTile` table from classic item
  metadata plus EC tileart facts.
- `metadata/item_texture_refs_index.bin`: per-item span table into refs.
- `metadata/item_texture_refs.bin`: all EC tileart texture refs.
- Item records store EC and CC texture ids, crop starts, offsets, radar color,
  flags, height, visual kind, and names.
- Texture refs store texture id, logical family, physical package, stable role,
  speculative role, block/item index, primary/auxiliary flags, stretch, `unk4`,
  `unk6`, and `unk7`.

`tex_art_ec.uddp`:

- Contains tileart-owned EC static/art images.
- It must not drop a tileart-owned surface entry just because its visual role is
  land-like. Runtime may choose the land path for that tile, but tileart remains
  the semantic owner.

`tex_land_ec.uddp`:

- Contains terrain material images derived from `TerrainDefinition.uop` plus
  reviewed override texture refs.
- Package layout:
  - `pages/*`: atlas page payloads
  - `metadata/pages.bin`: page dimensions, count, pixel format, used extents
  - `metadata/slots.bin`: sparse slot table keyed by runtime land/art slot id
  - `metadata/terrain_provenance.bin`: material-to-texture provenance
  - optional `metadata/terrain_overrides.json`: embedded KDL-derived defaults
  - optional `metadata/transcode.bin`: embedded transcode table
- `terrain_provenance.bin` v3 stores:
  - material id
  - material name id
  - alias count index
  - alias slot id
  - alias tile flags
  - selected texture id
  - canonical packed slot id
  - selected layer index
  - selected texture repetition/stretch
  - primary texture id
  - primary layer index
  - primary selection reason
  - primary selection flags
- Runtime uses provenance to resolve direct terrain, transcoded terrain, and
  surface-like static texture refs, and to fill base/detail/mask lookup entries
  for the land shader.

`tex_art_cc.uddp`:

- Contains Classic Client land and static art.
- Land diamonds and static art live in the classic `0x4000` split described
  above.

## 10. KDL Files

Active KDL files live in `dynamapper/assets/cc_ec_convtables/`.

`EcTerrainOverrides.kdl`:

- Reviewed manual integration table for facts not currently derivable from UOP
  data.
- Supported actions:
  - `policy`: shader/runtime policy such as smoothing or follow-center
  - `liquid`: reviewed liquid speed/wave parameters
  - `layer`: ordered texture layer substitution with optional stretch
  - `texture`: extra texture id reference with role
  - `ignore`: intentionally hidden/transparent terrain
- The packer embeds it into `tex_land_ec.uddp` as
  `metadata/terrain_overrides.json` and also packs referenced override textures.
- Runtime also loads the loose file and applies it over the embedded defaults.
  Loose overrides are fast for local experiments such as dark/light mountains
  or snow variants. A loose override can only use texture pixels already packed
  into `tex_land_ec.uddp`; missing refs are warned and fall back.
- Prefer reason-code values over prose in machine-readable fields.

`TerrainTranscode.kdl`:

- Loose fallback table mapping CC land ids to EC material ids.
- Keep it to id routing. Do not add visual policy here.
- It remains active for ids not fully resolved through direct
  `TerrainDefinition.uop` aliases.

Legacy KDL files:

- Live under `dynamapper/assets/cc_ec_convtables/legacy/`.
- They are review history and migration reference.
- Runtime paths must not load them.

Modify active KDL only when the source-derived pipeline is wrong or incomplete
and the reason has been inspected. If a rule can be derived from UOP data, fix
the parser or metadata builder instead of adding a broad manual table.

## 11. Conflict Handling

Diagnostics are not a goal by themselves. Add or run them only for targeted
questions: a suspicious tile, a candidate weak signal, an override review, or a
regression in the rendered map. If routing is visually correct and metadata
contains the decision reasons, prefer keeping the runtime path simple.

When a case is unclear:

1. Preserve every source reference first.
2. Record the current automatic decision and reason flags.
3. Prefer narrow source-derived criteria over id hardcoding.
4. Use `EcTerrainOverrides.kdl` for reviewed exceptions.
5. Warn and fall back when loose overrides point to unpacked texture ids.
6. Do not change runtime rendering policy until the metadata phase supports the
   decision.

Known weak signals:

- `unk6` is only a tie-breaker and audit flag today.
- `unk1` has prior diagnostic support for surface-like tileart mode, but it is
  preserved rather than used because shader/flag/stretch plus provenance
  currently solves the routing.
- Physical package names are not semantic ownership.
- `EffectTexture.uop` evidence is metadata/support evidence unless linked by a
  real owner; do not infer particle-only or terrain-only behavior from the file
  name alone.
