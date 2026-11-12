# Light sprites as the Enhanced and KR clients draw them: proposal

Status: **implemented** (2026-11-12), with the user's answers to §8 and the differences below. The current
formats are in [UDDP_FORMATS.md](../dev_wiki/UDDP_FORMATS.md) §2.3 and §2.5, and the renderer in
[CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md](../dev_wiki/rendering/CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md), "Light sprites".
The rest of this note is the proposal as written.

What was built differs from the proposal in these points:

- **Required, not optional** (Q1): `metadata/item_lights.bin` is a required tilemeta entry. It carries the magic
  `TMIL` and `TILEMETA_FORMAT_VERSION` (1), not a version of its own. A tilemeta without it is refused with the
  message to repack.
- **Light textures required:** `pack-ec-tex-aux` fails when a light texture named by the tilemeta is missing from
  `TerrainTexture.uop`. `udd-cli tool validate` checks them against the `tilemeta.uddp` beside the package.
  `tex_aux_ec.uddp`'s `slots.bin` gained the `AXSL` header in the same repack.
- **No interim fallback:** §5.3 is dropped. KR-like and Enhanced always draw the client sprites, and the light.mul
  quads remain only in Classic and UOD-Painterly. A light whose texture is absent from `tex_aux_ec.uddp` (a package
  packed against another tilemeta) draws nothing, with one warning per texture.
- **Counts:** the shipped EC tileart gives 1,042 records (not 1,040) and 29 textures (25 of 256x256, 4 of 128x128).
- **Q2:** yes. No hue on the sprites, and the flag-only items get no light, in KR-like and Enhanced.
- **Q3:** EC's entry 17 only. The placement comes from the map and the tileart alone, so it is the same under CC
  and EC art.
- **Q4:** `pack-lights` reads only the Classic folder now. The old Enhanced lookup (`build/terraintexture/%08u.dds`)
  never matched an entry of the shipped `TerrainTexture.uop`. The installed `world_lights.uddp` therefore held only
  the 55 light.mul lights, and the repack is byte-identical.
- **U1:** the offsets are applied literally, in world units. No client night screenshot confirms the (-5, -4)
  shift yet.
- **Still open:** U2, U4, U5 as in §8. The land shader decals and the static local lights of KR-like and Enhanced
  still read the light.mul instances (LightSource flag, quality as light id); only the additive decal quads are
  replaced.

KR-like and Enhanced draw the night composite's light buffer B with interim quads: light.mul masks with our own
size and position (see [CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md](../dev_wiki/rendering/CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md),
"Interim lights"). This note pins down what the two clients draw instead, measures the data, and proposes:

- the smallest package change that carries it;
- the renderer change that replaces the interim quads.

Evidence:

- the KR atmosphere research P3.3 ([uokr_atmosphere.md](../../scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/results/uokr_atmosphere.md));
- this pass's Ghidra traces of both binaries, with `LightSpriteProbe20261112.java`;
- the census `light_sprite_census.py`.

Both scripts are in [`ghidra-kr-atmosphere/`](../../scripts/dev-helpers/ec-research/ghidra-kr-atmosphere/README.md).
Results are marked **confirmed**, **probable** or **unknown**.

## 1. The decision needed

| Item | Proposal |
|---|---|
| Per-item light inputs | New optional entry `metadata/item_lights.bin` in `tilemeta.uddp`, with its own magic and version. `TILEMETA_FORMAT_VERSION` stays 1 (reset on 2026-11-12), so no repack is forced. |
| Light textures | `TerrainTexture.uop` entries named by the light items, stored in `tex_aux_ec.uddp` under the existing reserved keys `0x80000000 \| entry`, as the night maps already are. No new entry, and no reader change. |
| `world_lights.uddp` | Unchanged. It stays the source of the Classic and UOD-Painterly decals, and of the KR-like and Enhanced fallback for old packages. |
| Old packages | A tilemeta without `item_lights.bin`, or a `tex_aux_ec` without the light textures, keeps today's interim quads. |

§5 gives the exact layout, and §8 lists the open questions.

## 2. What the clients do

Both clients run the same code. KR addresses are in `UOKR.exe` and EC addresses in `UOSA.exe`. EC's definition
vtable sits 4 bytes later than KR's, and its sprite descriptor 8 bytes later; the definition field offsets are
identical.

| Step | KR | EC | Status |
|---|---|---|---|
| Component created | 0x00569cb0, at 0x0056a09c: when definition vt+0x38 (light shader name) is not null | `CreateComponents` 0x005befd0, at 0x005bf3f6: definition vt+0x3c (`GetLightShaderName` 0x00842940) not null | confirmed |
| Place the sprite | `PlaceLightSprite` 0x00438a80 (`UOObjectComponentLighting`, vtable slot +4) | `UOObjectComponentLighting::Initialize` 0x00449740 (vtable 0x00c9fdfc, slot +4) | confirmed |
| Texture location | `LightingTextureFileLocation` 0x00438680: the definition's block-3 name, else object vt+0x60 | 0x00449330: the same | confirmed |
| Quad | `BuildSpriteQuad` 0x00442430 | `BuildImageWindowQuad` 0x00459b00, through `BuildSpriteGeometry` 0x0045a080 | confirmed |
| Base point | `PlaceSprite` 0x004422b0 with the object's map position | `SetSpriteMapPosition` 0x004598d0 with (0, 0, 0); the object's node carries the position | KR confirmed; EC probable (§8, U3) |
| Pixel scale K | 0.021427423 at 0x00b87024 | the same value at 0x00d051d8 | confirmed |
| Light fields | definition +0x6c scale, +0x70 unused, +0x74 offset x, +0x78 offset y. Getters 0x004b56f0..0x004b5720 (vt+0x3c..+0x48); parser stores at 0x007be0ce..0x007be0eb | the same offsets. Getters `GetLightScale` 0x004c94e0 .. `GetLightOffsetY` 0x004c9510 (vt+0x40..+0x4c) | confirmed |

### 2.1 Which objects get a light

An object gets a light sprite when its tileart **shader block 3 names a texture**. The LightSource tile flag
(0x00800000) plays no part (confirmed in both clients).

The two overlap only partly. In EC, 943 items have both, 97 have a light texture and no flag, and 166 have the
flag and no texture (§3). Today's interim gates on the flag and on `quality`, the light.mul id.

The texture is block 3, map 0: `Data\Textures\000000NN_<name>.tga`, which is `TerrainTexture.uop`
`build/terraintexture/%08d.dds` entry NN.

- The name match is confirmed: every name corresponds to an entry, and 1..15 are byte-identical in both clients.
- The lookup itself was not traced (probable).
- Correction to P3.3: that note read the order as "object override vt+0x38, else definition vt+0x60". It is the
  definition's block-3 name first, else object vt+0x60. In both clients the fallback never runs for a light,
  because the component exists only when the name does.

### 2.2 Material and colour

These are unchanged from P3.3 (confirmed):

- `UOSpriteShader` with one map, the light texture, so there is no grunge;
- descriptor colour white and hue 0, so the light is the texture's own RGBA;
- no hue is applied, even when the placed item has one.

### 2.3 Geometry

The sprite descriptor (KR offsets; EC adds 8 from +0x3c on):

```text
window  x0 = y0 = 0,  x1 = W,  y1 = H                 (W, H: texture size, texture vt+0x48 / vt+0x4c)
anchor  ox = 22 - W/2  (integer, SAR),  oy = 0
scale   s  = tileart light_scale                       (descriptor +0x3c; EC +0x44)
```

`BuildSpriteQuad` / `BuildImageWindowQuad` work in the camera frame, X = view, Y = up, Z = right; K = sqrt(2)/66:

```text
left   = ox * s * K              right = left   + floor(W * s) * K
bottom = -oy * s * K = 0         top   = bottom + floor(H * s) * K
u = x / W,  v = 1 - y / H
```

The corners are rotated by the camera rotation M (KR `[[0x00c9dc70]+0x68]+0x2c`; EC `[g_UOWorldViewer+0x7c]+0x2c`).

The base point is the static's: `(x + 0.5, 0, -y - 1.5)`, moved by z * Zu along the viewer's up column
(viewer +0x24 in EC), with Zu = 0.12856454 per UO z. Our EC statics use the same point.

Then both clients add the light offsets, rotated by the same M, to the sprite position. The disassembly is KR
0x00438d43..0x00438dbc (rotation `0x0083aab0`); EC 0x00449972..0x00449a31:

```text
position += M * (0, offset_y, offset_x)
```

- **offset_x** is camera right and **offset_y** is camera up, both in **world units**.
- They are not scaled by K or by the light scale.
- This settles the open point of P3.3 (confirmed in both binaries).

What this means:

- The light's horizontal centre is 22 * s * K right of the base, plus offset_x.
  - At s = 1.5 that is sqrt(2)/2, which lands on the tile's near vertex (x + 1, y + 1). That is the same point as
    our Classic static anchor (`CC_STATIC_TILE_TRANSLATION_*`).
  - At the common s = 2 the centre sits 0.24 further right.
- The sprite's bottom edge sits at the base, and the sprite rises floor(H * s) * K before offset_y.
- Example: light_generic (W = H = 256) at s = 2 is 10.97 world units wide. With the common offset_y of -4.5 its
  centre is 0.99 units above the base along camera up.

### 2.4 Batch and composite

These are unchanged from P3.3 and the night-lighting page (confirmed):

- `BuildLightBatch` 0x00438ee0 turns the z test and z write off and sends the sprites to render group
  0x00c931b4.
- The blend is SRCALPHA / INVSRCALPHA over the night map, which gives B.
- The composite is `out = clamp(4 * (c * level + B * (1 - level)) * scene)`.

The lights therefore show only through the composite, scaled by 1 - level, and they are never drawn into the
scene directly.

## 3. The data

Census over both clients' `tileart.uop`, using the full `ec_research items` dumps:

| | KR | EC |
|---|---|---|
| Items with a light texture | 599 | 1040 |
| … that also have the LightSource flag | 512 | 943 |
| LightSource flag without a light texture | 79 | 166 |
| Light textures used (`TerrainTexture` entries) | 1–4, 6–9, 11–15, 17 | the same, plus 10 (firefield), 23, 25–29 (coloured lanterns, DragonLamp), 32–35 (stained glass), 37–40 (coloured glows) |
| `light_scale` | 2.0 ×454, 1.0 ×72, 1.5 ×39, 1.6 ×24, 0.75 ×5, 5.0 ×4, 1.25 ×1 | 2.0 ×864, 1.0 ×93, 1.5 ×39, 1.6 ×33, 0.75 ×5, 5.0 ×4, 1.25 ×2 |
| `offset_x` (world units) | -5.0 .. 2.0; -5.0 on 384 items | -5.0 .. 2.0; -5.0 ×378, 0.0 ×210, -0.1 ×179 |
| `offset_y` (world units) | -11.5 .. 0.0; -4.0 on 408 items | -11.5 .. 0.0; -4.0 ×407, -4.5 ×207, -5.1 ×135 |
| field +0x70 | 0 everywhere | 0 everywhere |

Agreement between the clients:

- 597 items have a light in both clients. 585 of them agree on texture, scale and offsets.
- 443 light items are EC only. Four of those exist in KR without a light: 2188, 3511, 11520 and 11696.
- The 12 that differ fall into two groups:
  - EC moved some offsets. Items 2576, 2577, 10922, 11518 and 15 had offset_x -5 or 1.75 in KR and 0 in EC.
  - Items 15764–15770 changed texture and scale: KR has Sconce 8 or 9 at 1.5, EC light_generic at 2.0.

Textures (EC `TerrainTexture.uop`):

| Entries | Size | Notes |
|---|---|---|
| 1–15 | 256² (11–15: 128²) | byte-identical in KR and EC |
| 17 (GoldGlow) | 256² | differs between KR and EC, both very faint (alpha at most 56 or 61) |
| 23–40 | 256² | EC only. Lights use 23, 25–29, 32–35 and 37–40; 24, 30 and 31 exist unused, and 36 is absent |

- All light textures are DXT5 without mips.
- The alpha centroids sit at 0.48–0.52 of the width, except:
  - the windows: 3 peaks at 0.64 and 4 at 0.35;
  - CandleA 7 sits at 0.59 down;
  - light_post 2 peaks high, at 0.23.

The colours are the P3.3 table, plus the EC lanterns: purple (162, 74, 241), blue (0, 81, 173), red (195, 69, 68)
and green (90, 211, 112).

Common combinations:

| Texture, scale, (offset_x, offset_y) | EC items | What they are |
|---|---|---|
| 1, 2.0, (-5, -4) | 376 | blank light-source items, lava, fireplaces, hearths, stained glass, soulstones |
| 1, 2.0, (0, -4.5) | 112 | lanterns, among them 2594 |
| 27 / 28, 2.0, (-0.1, -5.1) | 73 / 51 | EC coloured lanterns |
| 3, 1.0, (-1.35, -3); 4, 1.0, (1.75, -3) | 16; 19 | window lights, south and east |

## 4. What the packages carry today

- **`tilemeta.uddp` (format 1):**
  - the block-3 texture refs: `TileMetaItemTextureRef` with `texture_id` = NN, `stable_role` Light (10) and
    `physical_package` TerrainTexture (3);
  - the LightSource flag in `TileMetaItemTile`;
  - not the scale or the offsets. `ArtData` drops them in `TileArtEntry::process`.
  - `TileMetaItemTile` is 64 bytes with only `_pad1` spare, so the fields cannot go there.
- **`tex_aux_ec.uddp`:**
  - the night maps 20–22 at `0x80000000 | entry`;
  - not the light textures. Their refs are in the Textures family, which the packer counts as "unread family".
- **`world_lights.uddp` v1:**
  - Classic light.mul masks keyed by light.mul id (0..1023);
  - EC `TerrainTexture` entries as a fallback in the **same** id space. Ids with no light.mul mask, such as 20–22,
    can pick up a night map as a "light". That is a packer issue to tidy separately (§8, Q4).

## 5. The format change

### 5.1 `tilemeta.uddp`: `metadata/item_lights.bin` (new, optional)

The entry is little-endian and stored like the other tilemeta entries (DataType Metadata, the package compression):

| Offset | Type | Name | Value |
|---|---|---|---|
| 0x00 | `[u8; 4]` | magic | `b"TMIL"` (`TILEMETA_ITEM_LIGHTS_MAGIC`) |
| 0x04 | `u32` | version | 1 (`TILEMETA_ITEM_LIGHTS_VERSION`) |
| 0x08 | `u32` | count | number of records |
| 0x0C | `TileMetaItemLight[count]` | records | sorted by `tile_id`, unique |

`TileMetaItemLight` (20 bytes, `Pod`, 4-byte aligned):

| Offset | Type | Name | Description |
|---|---|---|---|
| 0x00 | `u32` | `tile_id` | item id, the same namespace as `TileMetaItemTile::tile_id` |
| 0x04 | `u32` | `light_texture_entry` | `TerrainTexture.uop` entry NN of block 3, map 0 (`extract_texture_id_from_path`); its `tex_aux_ec` key is `0x80000000 \| NN` |
| 0x08 | `f32` | `scale` | tileart `light_scale` |
| 0x0C | `f32` | `offset_x` | tileart `light_offset_x`: world units along camera right |
| 0x10 | `f32` | `offset_y` | tileart `light_offset_y`: world units along camera up |

- A record exists exactly when the item's block 3 has a map-0 texture (§2.1): 1040 records, 20.8 KB raw.
- Field +0x70 is not stored, because no client reads it and it is 0 everywhere.
- The entry has no texture size: the `tex_aux_ec` slot gives it.

**Versioning.** The entry carries its own version, as `world_lights` `slots.bin` does.

- `TILEMETA_FORMAT_VERSION` stays 1 (all formats were reset to 1 on 2026-11-12), so every tilemeta in use keeps loading.
- A reader that meets another `item_lights` version or a bad magic refuses the package with the usual "repack
  with the current udd-cli".

**Packer** (`lib/udd-conv/src/metadata/tilemeta.rs`):

1. Add `light_scale`, `light_offset_x` and `light_offset_y` to `uocf::enhanced::tileart::ArtData`, copied in
   `TileArtEntry::process`. This is an API addition, not a format change.
2. In `build_tilemeta_tables_with_ec_sources`, emit one record per item whose block 3 (`LIGHT_TEXTURE_BLOCK_INDEX`)
   has a `TextureItem` at position 0 of the Textures family:
   - `light_texture_entry` = that item's `id`;
   - the three fields from `ArtData`.
3. Write the entry in `write_tilemeta_uddp_with_progress`.
4. A Classic-only build (no EC tileart) writes no entry.
5. The source is the Enhanced Client's tileart, like the rest of tilemeta. §3 lists the 12 KR differences.

**Reader** (`lib/udd-assets/src/metadata/tilemeta.rs`):

- an `Option<Vec<TileMetaItemLight>>` loaded in `from_uddp_package`;
- `item_light(tile_id) -> Option<&TileMetaItemLight>`, a binary search;
- `has_item_lights() -> bool`.

### 5.2 `tex_aux_ec.uddp`: the light textures under the reserved keys

The light textures go into `tex_aux_ec.uddp` under the existing reserved keys. There is no new entry and no reader
change.

- **Keys:** each distinct `light_texture_entry` of the tilemeta's `item_lights` gets a slot at
  `tex_aux_ec_terrain_texture_key(entry)`. That is 29 entries from the shipped EC.
- **Slots:** the same rules as the night maps:
  - family Textures (4);
  - lossless RGBA8888 at native size, whatever the package's other format and upscale passes;
  - `stable_role` Light (10). The night maps keep Unknown (0).
- **Source:** the packer reads the Enhanced Client's `TerrainTexture.uop`, as `load_atmosphere_maps` does. Entry 17
  is therefore EC's GoldGlow.
- **Size:** about 7 MB of raw RGBA, mostly transparent, so far smaller under zstd 19.
- **Missing entries:** an entry missing from the archive gets no slot. The packer warns and counts it.
- **Packer** (`lib/udd-conv/src/textures/tex_aux_ec.rs`): the packer already loads the tilemeta, and it reads the
  entry list from `tilemeta.item_lights()`. With no `item_lights` entry, it packs no light textures.
- **Reader:** the existing `TexAuxEcPackage::load_terrain_texture_rgba(entry)` serves them. A package built before
  this change returns `None`.

The textures could instead go into `world_lights.uddp` as a v2 namespace. That was not chosen:

- the night maps already live in `tex_aux_ec` under this exact scheme, with the lossless and native-size rules the
  light buffer needs;
- the night composite already loads `tex_aux_ec`;
- `world_lights` would need a second manifest and a version bump.

### 5.3 Backward compatibility

| tilemeta has `item_lights` | tex_aux_ec has the light texture | KR-like and Enhanced draw |
|---|---|---|
| no | – | today's interim quads (light.mul, flag-gated), as now |
| yes | no (any light texture missing at load) | today's interim quads; one warning: "repack tex_aux_ec for the client light sprites" |
| yes | yes | the client light sprites (§6) |

Classic and UOD-Painterly are unaffected in every case.

## 6. The renderer change

Lights stay a shader style chosen per `StaticLightRenderStyle`/`StaticLightTarget`. The art source (CC, EC) and the
texture sources stay independent of it.

**KR-like and Enhanced** (`StaticLightTarget::ClientLightBuffer`): when §5.3 says "client light sprites", these
replace the interim quads in `art/static_lights.rs`.

- **Which statics:** a static draws a sprite when `tilemeta.item_light(graphic)` exists (§2.1). This replaces the
  0x00800000 + `quality` + world_lights slot test on this path.
  - Flag-only items (166 in EC) get no light, as in the clients.
- **Texture:** the `tex_aux_ec` light entry, uploaded once per entry as `Rgba8Unorm` (not sRGB, so B holds the
  client's encoded values as the interim path does), linear filtering, clamp addressing, no mips.
  - No hue, no grunge, no `static_light_style_profile`, no `CLIENT_GENERIC_LIGHT_COLOR` or
    `CLIENT_LIGHT_SPRITE_PEAK_ALPHA`: the texture carries its own colour and alpha.
  - These constants stay for the interim fallback only.
- **Quad:** use the EC static billboard of `statics_collect.rs`. The EC statics are built by the same client
  function, and the same mapping is already validated there.
  - Origin O = `(x + EC_STATIC_TILE_TRANSLATION_X, scale_uo_z_to_bevy_units(z), y + EC_STATIC_TILE_TRANSLATION_Z)`,
    the client base point (x + 0.5, y + 1.5).
  - Billboard axes: right = `STATIC_BILLBOARD_RIGHT_XZ`, up = world Y.
  - Bounds: `resolve_static_billboard_bounds(s / LEGACY_PIXEL_UNIT, 22 - W/2, 0, floor(W * s) / s, floor(H * s) / s)`.
    Then add `offset_x` to both x bounds and `offset_y * STATIC_WORLD_Y_PER_XZ_PIXEL` (1/sqrt(2)) to both y bounds.
  - With that pixel ratio the x scale is K * s per texture pixel, as in the client. A camera-up world unit becomes
    1/sqrt(2) of world Y, the ratio `CC_WORLD_Y_PER_PIXEL / CC_WORLD_XZ_PER_PIXEL` the static sprites use.
  - The v orientation is the EC static's (`uv.y = 1 - uv.y` for billboards). See U2.
  - New named constants: `CLIENT_LIGHT_SPRITE_ANCHOR_PIXELS` = 22, and the light-block facts.
  - No `STATIC_LIGHT_Y_BIAS` is needed, because the light camera has no depth test.
- **Target:** `LIGHT_BUFFER_LAYER` through the existing `LightBufferCamera` with `AlphaMode::Blend`, as the
  interim. `B = L.rgb + N' * (1 - L.a)` stays.
- **Culling:** a sprite reaches up to 27 world units across (MushroomA at s = 5), and its offsets reach 11.5 units.
  - The collector must scan beyond the visible chunks by the largest reach in the loaded `item_lights`:
    max(|offset| + half the extent), computed once at load.
  - Otherwise lights pop in at the screen edges.
- **Order:** overlapping sprites lerp in Bevy's transparent order, not the client's batch order (U4).

**Classic and UOD-Painterly** (`StaticLightTarget::SceneDecal`): no change. They keep the additive light.mul
decals with their style profiles, hue and flag gating, because they do not use the client composite.

**CC and EC art:** the sprite anchors to the object's map position and to the tileart light fields, never to the
art image or its offsets. It is therefore the same under CC art, EC art, and the EC client's legacy-art mode, which
creates the same lighting component. The record comes from tilemeta, which every art source loads.

**Docs to update with the implementation:**

- `UDDP_FORMATS.md` §2 (the new entry) and §2.5 (the light keys);
- the "Packaging" and "Interim lights" sections of `CLIENT_NIGHT_LIGHTING_AND_SHADOWS.md`;
- P3.3 in `uokr_atmosphere.md`: offsets resolved; texture order corrected; gate is block 3.

**Validation:**

- unit tests for the record round trip, the bounds of a known item (2594: x -4.543 .. 6.428 around O,
  y 0.697 above O at the centre), and the fallback table;
- a night capture at the lantern fixture (map 0, 2507, 560, 12) in all four render styles;
- `just bevy-lint` is not installed here.

## 7. Example: lantern 2594

Inputs: texture 1 (256²), s = 2, offsets (0, -4.5). In our axes, x = UO x and z = UO y:

- **Horizontal:** local x from (22 - 128) * 2 * K = -4.543 to -4.543 + 512 K = 6.428. The centre is +0.943 along
  (1/sqrt 2, 0, -1/sqrt 2), which puts it at about (x + 1.167, y + 0.833).
- **Vertical:** local y from -4.5/sqrt 2 = -3.182 to -3.182 + 10.971/sqrt 2 = 4.576. The centre is 0.697 world Y
  above the base.
- **Footprint:** the visible glow (alpha > 0 over 202 of the 256 pixels) is about 8.7 world units across.

## 8. Uncertain points and how to check them

| # | Point | Status | How to check |
|---|---|---|---|
| U1 | The large default offsets (-5, -4) on 376 EC items put the glow centre about 4 world units left of the item. The binaries apply the offsets literally (confirmed). EC zeroed offset_x on several lanterns (2576, 2577, 10922, 11518), which suggests the authors saw the shift. | confirmed reading, surprising effect | A night screenshot of a fireplace or lava (EC or KR) next to a Dynamapper capture. `docs/images/kr_reference/` holds day promo shots and no lit night scene, so the user would need to supply one; client binaries are never run here. |
| U2 | Texture v orientation: the quad's v = 1 - y/H at the top edge. This matters only for the asymmetric textures: 2 (post), 3 and 4 (windows), 7 (candle). | probable | Compare a window light at night; or check how `Textures` loads `TerrainTexture` DDS rows against the EC static path, which uses the same quad code. |
| U3 | EC calls `SetSpriteMapPosition(0, 0, 0)` and relies on the component's node carrying the object position. KR passes the position directly. | probable | Trace the node attach after `Initialize` in UOSA (`CreateComponents` 0x005bf4xx). |
| U4 | Draw order of overlapping lights inside the batch. | unknown | `BuildLightBatch` 0x00438ee0 sort key; only visible where two lights overlap. |
| U5 | Sampler state of the light map (filter, mips). The DDS files have no mips. | probable: linear, clamp | The `UOShaderFilenameLight` sampler setup. |
| U6 | Descriptor mode +0x38 = 3 (EC +0x40). | unknown, no visible effect expected | Not needed for the proposal. |

Open questions for the user:

- **Q1:** confirm the format change (§5.1 and §5.2), or choose the alternative of a tilemeta format 5 with the entry
  mandatory, which forces a repack.
- **Q2:** with the client sprites, KR-like and Enhanced drop the hue on lights and drop the 166 flag-only items, as
  the clients do. Confirm that this faithfulness is wanted. Statics on the shipped maps are almost all unhued.
- **Q3:** pack EC's entry 17 for every style, or carry KR's too? The two are near-invisible (alpha at most 56 or 61)
  and used by 15 items.
- **Q4:** should the `world_lights` packer stop filling light.mul ids from `TerrainTexture` (§4)? That is a
  separate packer change. It is no longer needed once the client sprites have their own home.
