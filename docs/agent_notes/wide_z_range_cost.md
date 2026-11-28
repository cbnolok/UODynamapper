# A z range wider than i8: memory, compute and depth cost

2026-11-28, on dev at a065ee48. Analysis only: nothing in the code changes. The numbers come from static analysis
and from two scripts in [`wide_z_range_cost/`](wide_z_range_cost/):

- `depth_precision.py` emulates art/depth.wesl's depth in f32;
- `footprint_chunks.py` gives the follow view's chunk counts.

No frame times were measured.

UO z is an i8 (-128..127) in every map format and in the protocol. The question is what a custom map with a wider
range would cost. Three widths are priced:

- **9-bit**: -256..255;
- **10-bit**: -512..511;
- **i16**: -32768..32767.

Bit stealing is allowed.

## Bottom line

| | 9-bit | 10-bit | i16 |
|---|---|---|---|
| GPU memory | 0 | 0 | land atlas +100 % (Rg32Uint): +64 MiB as allocated at start, +128 MiB with all of Britannia in view. Or +25 % with a side R8 texture: +16 / +32 MiB |
| CPU memory | about +0.1 MiB: the land z caches go from 1 to 2 B per cell. Statics stay at 6 B | same as 9-bit | the atlas CPU mirror grows like the GPU atlas. Statics go from 6 to 7 B (+2.8 MiB for map 0 if fully loaded) |
| Disk | 0 B raw. 0 B compressed for i8 content (bit-identical), a little for wide content | same as 9-bit | same as 9-bit |
| Land pixel shader | 0 instructions, 0 VGPRs: new constants only | +1 AND per tile-metadata read that keeps the payload, 0 VGPRs | 0 VGPRs, but each atlas tap fetches 8 B instead of 4 (42 height-tap call sites) |
| Art and mobile shaders, per instance | 0 (z travels as f32) | 0 | 0 |
| CPU compute | negligible | negligible | negligible |
| Streaming | 0: today's 64-tile pad covers the 60 tiles needed | a worst-case map needs a 106-tile pad at zoom below 10. A pad that follows the player's z keeps today's chunk count; a symmetric pad gives 2.4x the chunks at zoom 1 | cannot be padded (about 6,000 tiles); needs per-chunk z bounds |
| Depth | none: S = 264 kept, precision identical to today | none, same as 9-bit | today's mapping breaks: 31-36 % of adjacent levels invert. Needs a logical frame relative to the camera, plus either a fit to the content actually drawn or an integer key |
| Atlas spare bits left | 2 (R) | 1 (R) | 0 of 32: the texel must grow |
| Implementation | about 50-60 files of mostly mechanical i8 to i16 type changes, 6 shaders with new constants, 2 package encoders | the same, plus the wet bit moves into R and the light clip tag is repacked | the same, plus an atlas format change, a depth rework, z-bounded streaming, and the free camera, picking and cave lift bounded by map metadata |
| Risk | an i8 clamp missed somewhere silently clips z (list below) | the same, plus the streaming pad | high: depth, streaming, memory on a 4 GB card |

**Recommendation.**
- Take **9-bit** if 512 z of range is enough: it is free on every axis.
- Take **10-bit** if the maps need more. Its cost is one ALU op per metadata read, all but one of the atlas's spare
  bits, and a streaming pad that follows the player's z.
- In both cases keep `PRIORITY_Z_STEPS_PER_ROW` at 264 (see [Depth](#depth-key-precision)) and store z in the spare
  bits listed below.
- **i16 is not recommended.** The 10-bit range already spans 4,092 px on screen, four window heights at 1000 px and
  zoom 1. i16 buys range nobody can see, at the price of doubling the land atlas, reworking the depth and losing
  bounded streaming.

The formats are covered in [Formats for custom maps](#formats-for-custom-maps). Any of them is a format change that
needs the user's confirmation; it is proposed there, not decided.

## Inventory

`file:line` as of a065ee48. Paths without a crate prefix are under `dynamapper/src` (Rust) or
`dynamapper/assets/shaders/world` (WGSL). "Bits now" is the current storage.

### GPU

| Site | Bits now | z content |
|---|---|---|
| Land metadata atlas `Rg16u` (`render/assets/tile_atlas.rs:164`; pack `:238-270`; filled in `render/terrain/draw_mesh.rs:977-1010`). Texture array sizing in `render/assets/land/texture_array.rs:39-52` | G bits 0-7: z + 128 | absolute land z per cell (the cave floor z where a cave opens) |
| Its shader readers: `land/atlas.wesl:21-23` (`TILE_HEIGHT_BYTE_MASK`, `TILE_HEIGHT_BIAS`), `atlas_read_meta` `:75-87`, `atlas_read_z` `:147`, `atlas_read_terrain_flags` `:157` (flags at bit 12) | | |
| Atlas calls by shader: `land/main.wesl` 6 `atlas_read_meta` and 16 `atlas_read_height`/`_z`; `land/normals.wesl` 26 height reads; also cc_transitions, coverage, liquid_bank and transitions | | |
| `SpriteInstance`, `GroundTileInstance` (`render/statics/static_batches.rs:85`, `:135`) | f32 `world_y`, `base_world_y`, `priority_z_units`, `client_shadow_height` | scaled z; no cost |
| `MobileSpriteInstance` (`render/mobiles/draw.rs:61-74`): `MobileCeiling.z` f32 | f32 | no cost |
| `MobileGround` (`render/mobiles/ground.rs:66-74`) | `base_z` f32, plus 4-bit codes per cell that are relative to the mobile's z | no cost (see the note below) |
| Light scene, Rgba16Uint (`render/lighting/light_scene.rs:64`, `art/light_scene.wesl:27-30`) | a 16-bit channel holding z + 128, clamped to 255 | constants only |
| Light clip tag, a u32 MeshTag (`light_scene.rs:72-87`, `:112-125`; `art/light_clip.wesl:19-28`, `:81-84`) | cell 6 + 6, z 8, ceiling 8, cave bit 28 (29 bits used) | repack at 10-bit |
| Depth buffer: Bevy `Depth32Float`, reverse Z. Ortho near/far ±1000 (`render/view/camera.rs:29-30`); perspective is infinite, near 0.1 | | see [Depth](#depth-key-precision) |
| Draw-order constants `PRIORITY_Z_STEPS_PER_ROW` 264 and `PRIORITY_TIE_LEVELS_PER_Z` 9 (`art/depth.wesl:24`, `:28`; CPU copy at `render/statics/static_placement.rs:215-219`) | | |
| Cave lift `CAVE_LIFT_WORLD_HEIGHT` = 256 z (`art/depth.wesl:235`) | the whole i8 range | constant: 512 or 1024 z. Better: the map's range |
| `CAVE_MOUTH_UNCUT_Z` 128, "over any UO z" (`land/main.wesl:120`, used at `:1041`) | | constant |
| `SceneUniform.land_max_z` f32 (`common_bindings.wesl:9`; `LAND_NO_Z_LIMIT` in `mesh_material.rs:148`) | f32 | no cost |
| GPU picking (`render/picking/gpu_picking.rs`): Rgba8Unorm id plus Depth32Float | no z field | none |

**`MobileGround`.** The caller mentioned that another agent is widening its standing codes to the full i8 range.
That commit is not on dev as of a065ee48: the latest commits on `ground.rs` are a572a71d and 4f47f8df.

Today every code is relative to the mobile's own z: `drop + 1` in 4 bits, with drop at most 14.

If the widening stores the drop in 8 bits **relative to `base_z`**, it is independent of the absolute z width. If it
stores an absolute i8 per cell, a 9-bit or 10-bit z would need 16 bits per cell, +256 B per mobile. The relative
form is the one to keep.

### CPU (dynamapper)

The structs that grow if z becomes 16 bits wide. Padding absorbs the others.

| Site | Now | With 16-bit z | Count |
|---|---|---|---|
| `PackedStaticTile` (`lib/uocf/src/classic/maps/statics.rs:64`): xy 3 + 3 bits, **2 reserved**, z i8 | 6 B | 9-bit and 10-bit: **6 B**, with the high bits in the reserved bits. i16: 7 B | all statics of the loaded package chunks, never evicted (`world/statics.rs:17-26`). Map 0 has about 2.96M statics |
| `PackageLand.chunks: HashMap<u32, Option<Vec<i8>>>` (`world/z_view.rs:618-622`) | 1 KiB per chunk | 2 KiB | grows until the map changes. Instances: `mobiles/ceiling.rs:78`, `mobiles/ground.rs:182`, per lighting scan, per static chunk build |
| `ChunkCells.land_z: Vec<i8>` (`world/caves.rs:654-655`) | 1 KiB per chunk | 2 KiB | at most 81 chunks in `PlayerCaveFinder` |
| `UOVec4` (`world/coords.rs:39`), and `Player` with two of them | 6 B, 16 B | 8 B, 20 B | 1 |
| `ClientLightSpriteInstance` (`lighting/client_light_sprites.rs:142`), `ceiling: Option<i8>` | 56 B | 60 B | visible lights |

These already have room: `MapCell` (`lib/uocf/src/classic/maps/map.rs:44`, 4 B, z takes `_pad`), `StaticTile` (8 B, z takes
`_pad`), `StaticRenderTile`, `StaticPickTile`, `StaticCut`, `StaticLightKey`, `ClientLightSpriteKey`, `CoverStatic`,
`MapWalkCells` tuples and `PlacedMulti`.

These are already i16 or i32: `CaveBlock.below_z`, `PlayerCaveCell`, `ItemSurfaces`, `ZViewCut`, `Ceiling`,
`walk_z`, `MobileGroundInputs`, `MultiPart`.

**i8 range assumptions.** Each of these would silently clip z if missed.

- `z_view.rs`:
  - `:126` `ALL_LAND_HIDDEN_Z` (i8::MIN - 1);
  - `:447` `clamp_to_i8`.
- `walk_z.rs:22-24`: `NO_Z` -128 and `CEILING_Z` 128.
- `multis.rs:173` `expanded_multi_part_z`.
- `caves.rs:354` `floor_z`.
- `player_movement.rs:143`, `:416`.
- `maps.rs:346` and `:431`, the package decode: `(packed_meta as u8).wrapping_sub(128)`.
- Atlas decodes:
  - `static_surface.rs:75-78`;
  - `picking/terrain_cursor.rs:88`;
  - the ray band at `terrain_cursor.rs:200-202`.
- `view/footprint.rs:88`: free camera ground levels.
- `lighting/light_scene.rs:87`, `:121-122`.
- Change-detection hashes that keep only z's low byte (`z as u8`):
  - `lighting/static_lights.rs:237`;
  - `:326`;
  - `:341`.
- UI and control:
  - `ui/dialogs/world_scene.rs:57`, `:174` (`PLAYER_Z_RANGE`);
  - `ui/dialogs/teleport.rs:100`;
  - `input/dev_control.rs:35`, `:56` (serde `z: i8`).
- `export/placement.rs:9` `LOWEST_Z_STEPS`.
- The test ranges in `statics_collect.rs:3240-3253` and `statics/depth_tests.rs` (`TEST_PRIORITY_Z` -130..129).

### Packages (lib/udd-conv, lib/udd-assets)

| Package | Record | z now | Spare |
|---|---|---|---|
| `mapN.uddp` (`lib/udd-conv/src/maps/cc_map.rs:17-33`, pack `:68-73`): 32x32 `Rg16u` = 4,096 B per chunk, Zstd 19 | u16 land id, then u8 z + 128, then u8 flags | 8 bits | the flags byte is **always 0 on disk** (the runtime fills it), plus 2 bits of the land id (ids are below 0x4000) |
| `staticsN.uddp` (`cc_statics.rs:174-182`; `docs/dev_wiki/UDDP_FORMATS.md:93`): 8 B `StaticTile`, Zstd 7 | u16 graphic, u8 x, u8 y, i8 z, u8 pad, u16 hue | 8 bits | the pad byte, plus 3 bits each of x and y (0..31) |
| `tilemeta.uddp` (`lib/udd-assets/src/metadata/tilemeta.rs:328-354`): `height: i8` | item height, not a position | | out of scope |
| `UMAP` map metadata (`lib/udd-assets/src/metadata/map_metadata.rs:18-24`), 24 B | no z | | |

Map 0 figures:
- `mapN.uddp`: 28,672 chunks, 117.4 MB of texels, 15.94 MB stored;
- `staticsN.uddp`: 23.7 MB raw, 8.44 MB stored.

Both are from `docs/agent_notes/proposals/2026-11-01-mobile-anim-codec.md:233-240` and
`2026-11-01-data-track-open-items.md:454`.

### Client formats (lib/uocf) and tools

Every client cell and static format stores z as an i8:

- Classic `mapN.mul`/UOP cells: 3 B (`map.rs:167-172`).
- Classic statics: 7 B (`statics.rs:50-58`; writer `statics_write.rs:52-76`). x and y are 0..7 (enforced at
  `statics_write.rs:103-107`), which leaves 5 spare bits each.
- Dif and verdata (`dif_files.rs`).
- EC `facetN.uop` squares, delimiters and statics (`enhanced/maps/facet_sector.rs:12-14`).
- KR `StaticChunk.uop` (`kr/facet/static_chunk.rs:28-42`).
- EC waypoints.

The multis are the exception: Classic multis and EC `MultiCollection` already store part z as i16 (`multi.rs:15-22`,
`enhanced/structures/multis.rs:46-63`).

The tools hard-code the i8 range in:
- `tools/webmap-export-cli/src/geometry.rs:13-14` (`Z_MIN` and `Z_MAX`);
- `tools/uocf-cli/src/facet.rs:572` (`i8::try_from`);
- `tools/uddp-inspector-gui/src/logic/rendering.rs:77`, `:106`;
- the editor GUIs' `DragValue<i8>`.

**Protocol.** The repo has no packet code. The only sockets are the local JSON control socket (`Teleport` and
`SpawnMobile` with `z: i8`) and the executable patching in `lib/uocf/src/client`.

## Encodings and deltas per site

### Land atlas: the one site on the hot path

Today's G is `z+128 (8) | source (4) | flags (3) | wet (1)`. R holds the payload, which is below 0x4000, so 2 bits
are spare. The source mode takes the values 0..6 (`LAND_SOURCE_*`), so 3 bits suffice. That frees one bit in G.

**9-bit**
- G: `z+256 (9) | source (3, from bit 9) | flags (3, bit 12, unchanged) | wet (1, bit 15, unchanged)`.
- Every read stays the same single bit-field extract or mask (`v_bfe_u32` or `v_and_b32`), with other immediates.
- **0 instructions, 0 VGPRs, 0 bytes.**
- Optional: store z as a two's-complement field instead of biased. `extractBits(i32(g), 0, 9)` is then one
  `v_bfe_i32` in place of an `and` plus a `sub`. That saves one op per height tap (42 call sites).

**10-bit**
- G: `z+512 (10) | source (3, bit 10) | flags (3, bit 13)`; the wet bit moves to R bit 15.
- `atlas_read_meta` must mask the payload (`r & 0x7FFF`) before its lookups: one `v_and` per call whose payload stays
  live. The compiler drops it where the payload is dead.
- Height-only reads (`atlas_read_z`, and `atlas_read_height` after dead-code elimination) are unchanged.
- **At most ~9 extra ALU ops per pixel, 0 VGPRs.** The masked value replaces `r` in the same register.
- Only R bit 14 stays free.

**i16.** z takes the whole of G. Source, flags and wet (7 bits) do not fit in R's 2 spare bits. Two layouts work:

| Layout | Memory | Fetches | Notes |
|---|---|---|---|
| Rg32Uint: R = payload, source, flags and wet (21 bits); G = z | +100 %: 16 to 32 MiB per page | same count; z needs no unpack, payload +1 AND | `textureLoad` returns 4 dwords either way, so no VGPR change. Each of the 42+ height taps per pixel moves 8 B instead of 4 through the texture cache |
| Rg16Uint (R = payload, G = i16 z) plus an R8Uint (source, flags, wet) | +25 %: 16 to 20 MiB per page | height taps unchanged; +1 fetch per metadata read (~9 per pixel, and in the VS) | |

**The land pixel shader.** It sits at 256 VGPRs with spills, and z enters it two ways.

- **The draw-order depth** is per cell already. The VS writes `depth_cell = (x, y, average z)` as a flat varying
  (`land/main.wesl:415`), and the FS turns it into the depth (`:984`). Neither 9-bit nor 10-bit changes it.
- **The atlas taps for the neighbour blend and the normals** need each neighbour's z, so they cannot move to the VS.
  The only way to do them "once per tile" would be a precomputed per-cell height texture. That is the i16 split
  layout above, and 9-bit and 10-bit don't need it.

So the unpack can stay where it is. 9-bit adds nothing; 10-bit adds one AND per metadata read, with no new live value.
The 10-bit figure is static analysis. Confirm it with RADV stats on a scratch copy when implementing.

**Atlas memory.** One page is 2048² texels × 4 B = 16 MiB. Pages are allocated 4 at start, 64 MiB
(`TILE_ATLAS_INITIAL_LAYERS`), and grow to 8 for all of Britannia (128 MiB) and to at most 32 (512 MiB). The CPU
mirror (`tile_atlas.rs:504`) holds a copy of every layer in use. 9-bit and 10-bit add 0. For i16, see the table above.

### Other GPU sites

- **Light scene (Rgba16Uint).** Its z channel is already 16 bits. Only `LIGHT_SCENE_Z_BIAS`/`_TOP` change:
  - 9-bit: 256 / 511;
  - 10-bit: 512 / 1023;
  - i16: 32768 / 65535.

  0 bytes, 0 ops.

- **Light clip tag (u32).**
  - 9-bit: cells 6 + 6, z 9, ceiling 9, cave bit 30. That is 31 bits.
  - 10-bit: cells cut to 5 + 5. The reach becomes ±15; the farthest lights land about 5 cells away
    (`LIGHT_CLIP_CELL_REACH`). Then z 10, ceiling 10, cave bit 30: 31 bits.
  - i16: cells 4 + 4 (±7), z 16, ceiling stored as its rise over z in 7 bits (127 means none or out of reach), cave
    bit. That is 32 bits.
  - The shader's `extractBits` widths change, at no cost. i16 adds one add for the ceiling.
  - `LIGHT_CLIP_NO_CEILING` becomes the range's minimum.

- **Instances, MobileGround, picking, depth buffer.** z travels as f32 or relative values: 0.

- **Constants.** `CAVE_LIFT_WORLD_HEIGHT`, `CAVE_MOUTH_UNCUT_Z`, the free camera's ground levels and the cursor's ray
  band should come from the map's actual z range (see `UMAP` below), not from the format's. That way a 10-bit map
  with i8 content behaves exactly as today. The cave lift at 1024 z is 93 world units / |view y| ≈ 161 view units,
  inside ±1000.

### CPU

- **Statics.** 9-bit and 10-bit keep 6 B per static by putting the high bits in `xy_packed`'s 2 reserved bits. That
  costs about 2 ops per static per decode, which is negligible next to a chunk build. i16 takes 7 B: +1 B per static,
  +2.8 MiB for all of map 0.
- **Land z caches** (`PackageLand`, `ChunkCells`): +1 KiB per cached chunk.
  - At 1900x1000 that is about 50-170 chunks per instance: roughly +0.05 to 0.17 MiB each.
  - A pre-existing issue: `PackageLand` grows without bound until the map changes. A session that walked all of map
    0 would hold 28 MiB, which becomes 56 MiB.
- **Everything else** fits in padding or is already i16/i32. The arithmetic costs the same.
- **The cursor ray band** (`terrain_cursor.rs:200`) grows with the range: up to 2x (9-bit) or 4x (10-bit) more cells
  per cursor update in the worst case. That is microseconds.

### Streaming footprint

The follow view (`footprint.rs:133`) projects the window onto the ground plane and pads the result by 64 tiles at
chunk scale ≤ 2 (`chunk_edge_pad_tiles`, `:57`). That is 2 chunks; the pad is wider at larger scales.

A cell Δz above or below the player shifts by Δz/11 tiles along each axis. The tallest static art (591 px) reaches a
further 591/44 = 13.4 tiles.

The worst-case pad needed is range/11 + 13.4:
- i8: 37 tiles;
- 9-bit: 60 tiles;
- 10-bit: 106 tiles;
- i16: about 5,970 tiles.

Counts from `footprint_chunks.py` at 1900x1000:

| zoom (scale) | chunks today (pad 64) | 9-bit | 10-bit with a symmetric pad of 128 |
|---|---:|---|---:|
| 1 (1) | 50 | same | 122 |
| 4 (1) | 174 | same | 296 |
| 5 (2) | 67 | same | 104 |
| 9 (2) | 152 | same | 205 |
| 10 and up (4+) | 76 | same | same (pad 160 or more) |

Cells above the player rise into view from beyond the bottom edge. Cells below the player drop into view from beyond
the top edge. So the pad is only needed on each side for that side's z excess:
- the near side needs (max z - player z)/11 + 13.4;
- the far side needs (player z - min z)/11.

For 10-bit the two sides add up to about 106 tiles, against today's 64 + 64. A pad that follows the player's z,
bounded by the map's actual min and max z, therefore keeps the 10-bit chunk count at today's, apart from a minimum
for prefetch.

i16 cannot be padded. Its maps would need per-chunk z bounds to cull by (a 4 B table per chunk, 112 KiB for map 0).

## Depth key precision

**Setup.** Every depth writer uses the key `L = row + (priority z + tie/9) / S`, with S = 264. The logical point is
`(x, (priority z + tie/9) / S, y)` in world units. Its view z is computed relative to the camera (`art/depth.wesl:44`) and projected by the active
camera (`:63`).
- **Ortho:** `Depth32Float` with reverse Z and near/far ±1000. Logical points sit behind the camera plane, so their
  depth is at least 0.5, where one ulp is 2^-24, or 1.19e-4 view units.
- **One tie level** is 1/(9·264·√3) = 2.43e-4 view units, which is **2.04 ulps**.
- **Perspective:** `perspective_infinite_reverse`, near 0.1, so depth = 0.1/(-vz). Its precision is relative.
- **What relies on it:** the light scene tolerance of 0.45 levels (`light_scene.wesl:58`) assumes adjacent levels
  stay distinct.

**What 9-bit and 10-bit "need".** If z may never cross a row anywhere, S must exceed the priority range:
- 9-bit: priority range -258..257, so S ≈ 520;
- 10-bit: priority range -514..513, so S ≈ 1032.

That halves or quarters the size of a level. The emulation covers zoom 1-20 at three focuses (origin, Britain, far
corner) and the extreme player z. It reports the worst separation of adjacent tie levels in ulps (plain rounding and
FMA variants) and the share of pairs that collide or invert:

| Option | Ortho ±1000 (today) | Ortho fitted to the visible range (zoom 1 / 4 / 10 / 20) | Perspective (today) (zoom 1 / 4 / 10 / 20) |
|---|---|---|---|
| i8, S 264 (today) | 2.04 ulps per level, min 1-2, 0 collisions | 46 / 24 / 11 / 6 ulps | 12 / 3 ulps / **0.16 %** / **19 %** collide |
| 9-bit, S 264 | identical to today | 28 / 18 / 10 / 5 | 8 / 3 / 0.26 % / 19.6 % |
| 10-bit, S 264 | identical to today | 16 / 11 / 7 / 4 | 8 / 2 / 0.30 % / 20.8 % |
| 9-bit, S 520 | 1.03 ulps, **0.07-2.1 %** collide | 14 / 8 / 4 / 2 | 5 / 2 / 20 % / 55 % |
| 10-bit, S 520 | 1.03 ulps, 0.36-2.6 % collide | 8 / 6 / 4 / 2 | 5 / 1 / 22 % / 55 % |
| 10-bit, S 1032 | 0.52 ulps, **24-30 %** collide | 4 / 2 / 2 / 0 (1.4 % at zoom 20) | 2 ulps / 11 % / 57 % / 76 % |
| i16, S 264 | **broken**: 31-36 % of pairs invert, 28-36 % collide | 0.29 ulps, 64 % collide | broken |

S above the range is therefore not affordable at 10-bit with today's mapping. **It is also not needed.**

**Option A: keep S = 264 (recommended for 9-bit and 10-bit).** z crossing a row only matters for two objects that
overlap on screen.
- **Sprites.** A farther object wins wrongly only when it is more than S·d z above a nearer one d rows in front. It
  then stands 22d + 4·264d = 1,078d px higher on screen. The nearer art would have to be taller than 1,078 px; the
  tallest static art is 591 px, and mobiles are smaller.
- **Land.** One depth per cell. A cell's corner would have to rise about S - 13 + 5.5d ≈ 256 z above the cell's
  average inside the cell: a one-cell cliff that i8 can only just produce.
- **Precision.** Identical to today in ortho. The logical heights stay within ±1.95 world units (514/264). The camera's
  real height (up to 46.5 world units at 10-bit) moves the view z by at most 27 units, well inside ±1000.
- **Cost.** Zero changes to depth.wesl, the tests or the light tolerance.

**Option B: fit the near/far to the visible logical range** (ortho), or map linearly over it (perspective).
- **Gain.** 3-23x the precision in ortho. In perspective it removes today's collisions at zoom 10 and 20: the
  "focus-fit" run gives the same numbers as the ortho fit.
- **Cost.**
  - A per-frame uniform with the logical range.
  - Everything rasterised with real depth (the player cube) would have to write the same mapping.
  - The land's real vertices must stay inside the clip range: through a VS clip z set to the logical depth, or with
    unclipped depth.
- Not needed for 9-bit or 10-bit. It is the fix for the pre-existing perspective issue below.

**Option C: fewer tie levels.** Going from 9 to 5 levels (2 ranks instead of 4) gives 1.8x the precision. In return,
6,669 statics share the last rank instead of 246 today. The source is the tie groups in
`docs/dev_wiki/rendering/DEPTH_SCALE_AND_KR_CLIENT.md`: 4,571 groups of 3, 736 of 4, 140 of 5, 44 of 6, 6 of 7.
Those statics fall back to their draw order within their chunk batch. This is a small gain for a visible risk; not
recommended.

**Option D: a row weight smaller than the range (KR's 35.4).** At S = 35.4 one level is 15.2 ulps in ortho, 7.5x
today, and no z range ever needs a larger S. But z then crosses rows by design: a static more than 35.4·d z above a
nearer one draws over it. That is the KR client's traced rule, and it breaks:
- the rule the user accepted on 2026-11-16 for every style (ClassicUO's: the nearer row always wins). Classic and
  UOD-Painterly follow the Classic client, and Enhanced's rule is untraced;
- `depth_tests.rs`' range checks and the pair survey;
- the land's one depth per cell on slopes. Cells whose average sits more than 35 z above a nearer static win over
  it, as in KR, whose terrain bias sits on the same scale.

So it is a style decision (a per-style S uniform plus a revalidation of all four styles), not a fix for wide z.

**Option E: an integer key in the depth bits.** Positive floats sort as their bit patterns. The normal floats in
(0, 1] give 2^29.98 distinct depth values, ordered as their bit patterns, against 2^23 in today's ortho half range. A key in quarter z
needs `264 × 4 × 9` = 9,504 levels per row (quarter z covers the land's average and the -2.5 ceiling offset):
- 10-bit at zoom 20: 1,286 rows, 2^23.5 levels;
- i16: 24,746 rows, 2^27.8 levels.

Both fit with spare bits to space the keys by 8 or 4 ulps. That margin absorbs the perspective divide's rounding in
the sprite VS, which outputs a constant depth.

The costs:
- every depth writer writes the key;
- every depth reader must decode it: the light scene tolerance, GPU picking, anything that rebuilds a position from
  depth;
- the player cube must write the key too;
- an exact constant depth across a sprite card relies on the rasteriser's zero-slope plane equation. Validate that on
  RADV.

E is the only option that makes the full i16 range work. A's limits come before B's precision does.

**i16 with today's depth.** It fails twice:
- the camera sits at the player's real height (up to 2,979 world units), so view z leaves ±1000. That is the
  inversions;
- cells up to 4·65535/22 ≈ 11,900 rows away can show on screen, so even a logical frame relative to the camera spans
  about ±7,000 view units. Fitted to that worst case, one level is 0.29 ulps (64 % collide).

i16 needs a logical frame relative to the focus (subtract the player's real height: one uniform), plus either B fitted
to the drawn content's actual rows (per-chunk z bounds) or E.

**Pre-existing finding (not caused by wide z).** In the perspective camera, adjacent tie levels collide in 0.16 % of
pairs at zoom 10 and 19 % at zoom 20, already with i8. That is the emulation's worst case over the sampled rows, if the
perspective camera reaches those zooms. `depth_tests.rs` covers only orbit distances 138 and 552 (zoom ≈ 1 and 4).
Colliding pairs fall back to draw order within a chunk batch. Whether that shows on screen was not checked. Option B
(a linear mapping fitted to the visible range) is the fix.

## Formats for custom maps

**Where wider z could come from.**
- Every client format is i8 (see the inventory), so no Classic, EC or KR file can carry it.
- Map cells and statics never travel in the protocol: the client reads them from its own files. So a custom map's
  land and statics need only a source format and our packages.
- Bit stealing inside the Classic MUL is possible but rejected:
  - 2 spare bits of the land id give a 10-bit z at 3 B per cell;
  - x and y's 5 spare bits each give statics high bits at 7 B per record;
  - but every MUL reader that doesn't know it sees land ids ≥ 0x4000 or x ≥ 8, and the uocf statics writer rejects x
    ≥ 8 outright. A vanilla client would draw garbage.
- **Open question for the user:** the source.
  - (a) A wide variant of the Classic files, with 4 B cells (u16 id, i16 z) and 8 B statics (… i16 z …), read by uocf
    and udd-conv. A whole map 0 grows from 89.9 to 119.3 MB of cells and from 7 to 8 B per static.
  - (b) Editors that write `mapN.uddp` and `staticsN.uddp` directly.
  - Which map editor would produce them?

**The cheapest encoding in our packages.** No bit stealing is needed: both packages already have a spare byte.
UDDP stays v1 and the packages are regenerated. This is a proposal that needs the user's confirmation.

- **`mapN.uddp` cell:** G16 = (z + 128) mod 2^16; R16 = the land id.
  - For i8 content this is **bit-identical** to today's packages, because the flags byte on disk is always 0.
    Existing packages stay valid without a repack.
  - Decode: `(g as i16).wrapping_sub(128)`.
  - Raw size: 0 B more, for any width up to i16.
  - Compressed: 0 for i8 content. Wide content adds entropy only where z leaves the i8 range.
- **`staticsN.uddp` record:** z as i16 LE at @4..5, where the pad byte becomes the high byte.
  - Raw size: 0 B more.
  - Compressed: negative-z statics get 0xFF in byte 5. The estimate is well under 1 % of 8.44 MB; measure it on the
    repack.
  - To stay bit-identical, z = i8(byte 4) + 256·i8(byte 5) costs two ops per decode. It is not worth the oddity,
    since the packages are regenerated anyway.
- **`UMAP` metadata:** add `min_z` and `max_z` (i16 each), 24 to 28 B.
  - They let the streaming pad, the free camera's ground levels, the cursor's ray band, the cave lift and
    `CAVE_MOUTH_UNCUT_Z` follow the map's real range.
  - A 9-bit or 10-bit renderer then behaves exactly as today on i8 maps.
  - Optional for i16: a per-chunk z bounds table (4 B per chunk, 112 KiB raw for map 0).

**Protocol extension (server side, outside this repo).** From memory of the RunUO and ServUO packet code, not
verified here:
- the world-object packets carry z as a signed byte: items 0x1A and 0xF3, mobiles 0x20, 0x77 and 0x78, move
  rejection 0x21, effects 0x70 and 0xC0;
- the login confirm 0x1B already carries it as a short.

The cleanest extension is a capability negotiated at login, for example a 0xBF subcommand or a client feature bit.
After it, those packets carry z as an i16:
- +1 byte each;
- the fixed-length packets change length, so both sides switch together.

The server's own tile structures (land and static z as sbyte) need widening too. Separate wide packets, or a side
packet carrying only the z high byte, would work as well, but they double the paths for each object.

## Open questions for the user

1. Which width: 9-bit (free) or 10-bit (one op per metadata read, the atlas's spare bits, the z-following pad)?
2. The package changes above (`mapN.uddp` G16, `staticsN.uddp` i16 z, `UMAP` min/max z): confirm before
   implementing.
3. Where custom maps' wide z comes from: a wide Classic file variant, or editors writing UDDP directly. Which editor?
4. The protocol extension's shape, if a server will ever send z beyond i8.
5. Separate from wide z: fix the perspective camera's far-zoom tie collisions (Option B)?
