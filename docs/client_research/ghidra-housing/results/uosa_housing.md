# UOSA.exe: housing.bin and MultiCollection.uop

Client: `UOSA.exe`, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`.
Names below are the ones applied by [`NameHousingCode.java`](../NameHousingCode.java) (tag `UO_HOUSING`)
unless marked otherwise. Addresses are for this build. "Instr." means the conclusion was checked in the
disassembly, not only in the decompiler output.

## 1. Load path and timing

| Step | Function | Notes |
|---|---|---|
| Factory registration | `FUN_004afd80` | registers the asset factories by asset type: 0x13 `UOMultiCollectionBinaryFactory` (`build/multicollection/%06d.bin`), 0x14 `UOCustomHousingPiecesBinaryFactory` (`build/multicollection/housing.bin`) |
| Asset key | `UOResourceRequest::AddCustomHousingPiecesKey` 0x004cb770, `AddMultiCollectionKey` 0x004cb700 | key `{id, type << 24}`: housing.bin is `{0, 0x14000000}`, multi N is `{N, 0x13000000}` (instr.) |
| Request | `FUN_00a6ced4` -> `UOAssets::GetAsset` | synchronous: cache lookup, resolvers, blob read, `CreateAssetFromBlob` -> factory `Create`. The asset manager keeps the asset in its cache. |
| Factory | `UOCustomHousingPiecesBinaryFactory::Create` 0x0083eefe | `UOCustomHousingPieces::Parse` 0x0083eff3; on failure the asset resolvers run |
| Factory | `UOMultiCollectionBinaryFactory::Create` 0x00841c3c | `UOMultiCollection::Parse` 0x00841d31 |

`housing.bin` is read only by `UOHouseCustomization::BuildPieceTables` 0x00652090, called from
`UOHouseCustomization::Initialize` 0x00651980. Initialize runs:

- when the play state starts (`FUN_004aa110`), right after `FUN_004aa720` = `UOHouseCustomization::Create`
  allocated the 0x877b8-byte object (`operator new` + constructor `0x006512c0`) and stored it in
  `g_UOHouseCustomization` 0x00e37244;
- from the world event handler `FUN_00470970` (event type 2, code 0x1f: world reset / reconnect), after
  `FUN_006517e0` cleared the house maps +0x87734 / +0x87750 / +0x8776c and the Lua tables
  (`WindowData.CustomizedHousing.*`, `HousingSystem.*`).

Initialize: clear the selection, `BuildPieceTables`, mode = 4, `FUN_00652950` (registers the housing Lua
functions), `PublishCustomizedHousing` 0x00652a20. Nothing reads `housing.bin` at the moment a house is
customized: the tables built at start-up are used. Multis referenced by housing pieces are requested in the
same pass (`LoadMultiAppearance`). Confidence: high.

## 2. housing.bin layout

All fields are little-endian u32. Parser `UOCustomHousingPieces::Parse` 0x0083eff3 (every size checked in the
`UOAssets::StreamRead` 0x00a6fe55 calls, instr.):

```
u32 tile_type_count
tile_type_count x {
    u32 tile_type
    u32 piece_count
    piece_count x {                         // 0x40-byte piece, InitPiece 0x0083f1f0
        u32 category                        // piece +0x00
        u32 group                           // piece +0x04
        u32 feature_mask                    // piece +0x08
        u32 tooltip_cliloc                  // piece +0x0c
        u32 n, n x {u32 slot, u32 tile_id}  // Solid  -> piece +0x10 vector (ReadSlotList 0x0083f279)
        u32 n, n x {u32 slot, u32 multi_id} // Multi  -> piece +0x20 vector
        u32 n, n x {u32 slot, u32 tile_id}  // Window -> piece +0x30 vector
    }
}
```

- The object `UOCustomHousingPieces` is 0x10 bytes (constructor 0x0083ee1c); +4 is a
  `std::map<u32 tile_type, std::vector<piece>>`. Pieces are pushed (`PushPiece` 0x0083f511) into a local
  vector, then assigned to `map[tile_type]` (`TileTypeAt` 0x0083f36f, `AssignPieces` 0x0083f42b); a repeated
  tile type replaces the earlier one.
- After the last tile type the position must equal the blob size; otherwise `FUN_0040c6c0` reports an error
  and the asset fails. No counts are bounded.
- The real file (41,060 bytes) is consumed exactly: 7 tile types, 424 pieces, 3,567 tile references, 74
  multi references.

## 3. What the fields mean

### 3.1 Tile type

| Value | Meaning | Pieces | Categories | Lists used |
|---:|---|---:|---|---|
| 1 | stairs | 19 | 0..18 | Solid (slots 1..9), Multi (slots 1..4) |
| 2 | roof | 36 | 0..21 | Solid (slots 1..16) |
| 3 | door | 37 | 0..36 | Solid (slots 1..8) |
| 4 | floor | 58 | 0..57 | Solid (slots 1..16) |
| 5 | wall | 180 | 0..46 | Solid (slots 1..8), Window (slots 1..6) |
| 6 | teleporter | 1 | 0 | Solid (12 of slots 1..16) |
| 7 | misc | 93 | 0..37 | Solid (slots 1..8) |

The values are the Lua `CustomizeHouseWindow` constants (`TILE_TYPE_STAIRS = 1` ... misc = 7) and
the cell "floor tile type" of the house grid (`+0x54`, 1 stairs, 6 teleporter, 8 none; see
[`uosa_tileart.md`](../../ghidra-tileart/results/uosa_tileart.md) section 8.4.1). BuildPieceTables visits
tile types 1..7 (0 and 8 are skipped). The tile type of the selected piece decides which cell slot the
piece goes in (`FUN_00655f80`: roof, floor-like 1/4/6/8, other). Confidence: high.

### 3.2 Category and group

`category` is the row of the house customization window inside a tile type (the "style"); `group` is a
sub-row of the category (the wall and misc categories have up to 7 / 9 groups; all other tile types use only
group 0). Lua publishes them 1-based: `WindowData.CustomizedHousing.Type[tile type].Category[category + 1]
.Group[group + 1]`. BuildPieceTables walks categories and groups from the min to the max found
(`GetCategoryRange` 0x00846629, `GetGroupRange` 0x00846736), so a category number without pieces still gets
an (empty) entry; there are no such gaps in the current data, and no (tile type, category, group) key occurs
twice. Confidence: high.

### 3.3 Slot lists

Each list entry is `{slot, id}`. The client stores them in per-group maps keyed by slot (last write wins):

| List | Group entry | Lua | Meaning |
|---|---|---|---|
| Solid (+0x10) | group +0x00 map<slot, tile> | `Group[g].Solid[slot].id` | the tile ids of the piece variants (wall orientations, roof pieces, floor tiles, door states, stair parts...) |
| Window (+0x30) | group +0x1c map<slot, tile> | `Group[g].Window[slot].id` | wall variants with windows; walls only |
| Multi (+0x20) | group +0x38 map<slot, multi id> | `Group[g].Multi[slot].{id, appearance}` | multi ids 7600..7675 of the stair blocks; stairs only |

Solid and Window tile ids also go in the house lookup +0x87750 `map<tile id, {tile type, slot}>`
(`TileLookupAt` 0x0065b120; the last piece wins for the 287 tile ids that appear in more than one slot). It
is used by `SelectPieceById` (Lua `SetHousingTileSelected`) and by `FUN_006535f0` -> `FUN_00651d80` /
`FUN_00651f00` (the server house design handling) to recover tile type and slot from a tile id. The slot is
the "tile subtype" passed to `SelectTile` (+0x87700). Confidence: high for the storage, medium for the
per-type slot meanings (from the classic text files, section 6).

`PublishCustomizedHousing` sets `previewTile` of a category to the first nonzero Solid id it finds in the category.

### 3.4 Feature mask and entitlement

`feature_mask` is compared with two masks:

- the client expansion filter 0x6f3bff (`HousingPieceFilter` +0xc): queries ignore pieces whose mask has no
  bit in it; every piece passes in the current data;
- the 0xB9 server feature flags `g_ServerFeatureFlags` 0x00da987c (named by another script; written by the
  0xB9 handler): `GetFeatureMask` 0x008466be returns the mask of the **last piece of the category** (any
  group, no expansion filter) and the category entry gets `entitlement = (flags & mask) != 0` (+0x20) and an
  `entitlementTID` (+0x24) chosen from the mask (instr.):

| Mask | Meaning (Describe text / cliloc) | entitlementTID | Categories |
|---|---|---|---:|
| 0x1 | "no purchased expansions" (always granted) | 1062930 | 76 |
| 0x40 | AOW | 1062930 | 48 |
| 0x80 | EP1 | 1062930 | 19 |
| 0x200 | 9th | 1062930 | 12 |
| 0x10000 | SA | 1062930 | 48 |
| 0x40000 | Gothic Theme Pack | 1150774 | 3 |
| 0x80000 | Rustic Theme Pack | 1150773 | 3 |
| 0x400000 | Time of Legends | 1156114 | 13 |

1062930 is "You must upgrade to the latest Expansion Pack in order to place these tiles ...";
1150774 / 1150773 / 1156114 name the Gothic / Rustic theme packs and Time of Legends. Five categories have
groups with different masks (walls 2..5 and misc 11: one AOW group among no-expansion groups); only the last
group's mask counts. The expansion names in the table come from `HousingPieceFilter::Describe` 0x00846164
(0x40 " AOW", 0x80 " EP1", 0x100 " 8th", 0x200 " 9th", 0x10000 " SA"); the theme names from the cliloc texts.
Confidence: high for the logic, medium for the expansion naming.

### 3.5 Tooltip

`tooltip_cliloc` is the category name (e.g. 1021826 "wooden stairs", 1063375 "Tile Roof", 1156116
"Shadowguard Walls", 1061019 "Stairs", 1061020 "Teleporters"). `GetTooltip` 0x0084658f returns the nonzero
tooltip of the category's pieces (any group); every category has exactly one, except misc 29 where groups 0
and 1 both carry 1112094 "Two-Tone Stone Archways": GetTooltip then returns false, keeps the first value, and
`HousingPieceFilter::Describe` builds a diagnostic string that is thrown away. The result is the same.
Published as `Category[c].tooltip`. All 153 distinct tooltip clilocs resolve. Confidence: high.

### 3.6 Multi appearances

For every Multi list entry BuildPieceTables calls `LoadMultiAppearance` 0x006526e0 (instr.): it requests the
multi `{id, 0x13000000}` and fills the house +0x8776c entry (`FUN_0065b240`):

- +0x00..+0x77: 10 x `{i32 x, i32 y, i32 z}` part offsets,
- +0x78..+0x9f: 10 x u32 part tiles,

taken from the first 10 parts whose "not drawn" byte (+0xa) is 0 (`UOMultiCollection::GetPart` 0x00835c6d);
unused tiles are zero. The Lua `Multi[slot].appearance` is the first tile. `FUN_006549e0` uses the 10 offsets
to test placement (`IsPlacementAllowed`) when a multi is selected; `FUN_006537a0` also skips not-drawn parts
and tile 0x31f4 (dirt). No referenced multi has more than 10 drawn parts. Confidence: high.

### 3.7 Selection

- `UOLua::SetHousingTileSelected` -> `SelectPieceById` 0x006547d0: id found in the tile lookup with a nonzero
  tile type -> `SelectTile` 0x006548d0 (tile type +0x876fc, id +0x876f8, slot +0x87700, multi flag +0x87704
  = 0); otherwise id found in the multi map -> `SelectMulti` 0x00654890 (id, tile type 1, multi flag 1).
- `UOLua::SetHousingMultiSelected` -> `SelectMulti`.
- The eyedropper `FUN_00659f70` (caller `FUN_0057d380`) finds the category / group of a picked tile.

## 4. MultiCollection multi entries

`build/multicollection/%06d.bin`, parser `UOMultiCollection::Parse` 0x00841d31 (instr.):

```
u32 multi_id                     // stored at object +4; equals the file id for all 862 entries
u32 part_count
part_count x {
    u16 tile_id
    i16 x, i16 y, i16 z          // z read with MOVSX word: full signed 16 bits
    u8  not_drawn                // part +0x0a
    u8  flag2                    // part +0x0b
    u32 n
    n x u32 string_index         // string dictionary index ([g_UOAssetManager+0x154], 0x1c-byte std::string
                                 // per index); the string is copied and pushed only when non-empty
}
```

- Object `UOMultiCollection` 0x4c bytes (constructor 0x00835902 (id, count)): +4 id, +8 count, +0xc parts.
  `SetPart` 0x00835ad8 writes a 0x1c-byte part `{+0 u16 tile, +2/+4/+6 i16 x/y/z, +8 u16 = 0, +0xa u8,
  +0xb u8, +0xc vector<std::string>}` and grows the bounds: min/max x, y, z at +0x30..+0x44, +0x48 the
  maximum of `FUN_00835d26` (a per-part metric), +0x2c a flag set from the first part.
- The position must equal the blob size at the end; otherwise `FUN_0040c6c0` and the asset fails.
- Data: all 862 entries consumed exactly; z in -8..123; strings per part 0 / 1 / 2; all 11,895 part tiles
  exist in tileart.

| Field | Meaning | Data | Evidence | Conf. |
|---|---|---|---|---|
| `not_drawn` | 1 = part not shown: skipped by `LoadMultiAppearance`, `FUN_006537a0` and the multi renderer `FUN_005ae0d0` (except multis whose object +0x30 is 24..99, probably the new ships). Classic `multi.mul` flags = 0 for these parts (e.g. the "nodraw" tile 1). | 4,026 ones | instr. | high |
| `flag2` | 1 = the renderer passes hue 0 instead of the multi hue (`FUN_005ae0d0`). Equals the extra dword of classic `multi.mul` records. | 1,359 ones | decompiler | low-medium |
| strings | part tags such as `house_register_fixture` (730), `house_door` (586), `hueshadow` (491), `shiphold_linked` (410), `shipaccess` (256), `house_teleporter` (144), `house_multi` (104) | 3,914 refs | data | medium |

Flag pairs (not_drawn, flag2): (0,0) 174,330; (1,0) 3,119; (1,1) 907; (0,1) 452.

## 5. housing.bin and the multis

`housing.bin` lives in `MultiCollection.uop` but is a different asset type (0x14) with its own object.
The only link is the stairs Multi list: 74 references to multis 7600..7675 (the stair blocks of the
customization tool, 4 slots N/E/S/W), all present in the collection. The multi entries do not refer to
`housing.bin`. The classic client's `MultiCollection.uop` contains the same payloads.

## 6. Comparison with the classic customization files

The classic client reads the same data from `stairs.txt`, `roof.txt`, `doors.txt`, `floors.txt`,
`walls.txt`, `teleprts.txt` and `misc.txt`. Row by row (scratch comparison, not in the repo):

- Feature mask: classic 0 corresponds to EC 1 (175 rows); the other values are equal (64, 128, 512, 65536,
  262144, 524288, 4194304), apart from five single-row differences.
- Stairs: EC Solid slots 1..8 = classic North..Rounded2, slot 9 = Block (the classic column order is
  rotated); the Multi slots 1..4 are the N/E/S/W stair multis.
- Walls: the six classic window columns map to the Window slots, sometimes reordered (e.g. category 43).
- Floors and teleporters: 16 slots with zero gaps; a few EC floors (56, 57) and walls (46) renumber slots.
- Misc: classic categories >= 16 are shifted by one (classic 16, a teleporter-like row, is missing in EC). EC
  has extra styles (22/1-3, 25/1-2, 29/1, 37/1-2); classic has 12 rows absent from EC (walls 43/4-5,
  misc 16/1, 23/1-3, 26/2, 27/1, 30/1, 38/0-2).

## 7. Open questions

- `flag2` (part +0xb): only the hue use in `FUN_005ae0d0` was found; other readers of part +0xb may exist.
- The multi object +0x30 range 24..99 that draws `not_drawn` parts anyway: which multis are these?
- Part +8 (u16) is always written 0 by SetPart; part +0x2c / +0x48 of the collection were not traced further.
- The tooltip clilocs were resolved with the classic `Cliloc.enu`; the EC `LocalizedStrings.uop` was not
  read. The texts match the window labels.

## 8. Mismatches with `lib/uocf/src/enhanced/multistructures/multis.rs`

| Item | Repo | Client | Kind |
|---|---|---|---|
| part z | `(z as i8) as i16` | full `i16` (MOVSX word) | wrong (no effect on current data: z is -8..123) |
| part flags | one `u16 flags` | two `u8`: +0xa not drawn, +0xb flag2 | naming / semantics |
| `cliloc_offsets: Vec<i32>` | cliloc offsets | `u32` string dictionary indices; empty strings dropped | wrong name and sign |
| trailing bytes | ignored | error unless the entry ends exactly | missing check |
| `bounds()` | x / y | x / y / z (+0x30..+0x44) | missing z |
| housing.bin | hash and byte length only | parsed (section 2) | missing |

The header id is read and the file id used instead; both are equal in the data, so that is harmless.

## 9. Proposed Rust changes

1. `multis.rs`: read z as `i16`; split `flags` into `not_drawn: bool` (+0xa) and `flag2: u8` (+0xb, document
   as "ignore multi hue, low confidence"); rename `cliloc_offsets` to `string_ids: Vec<u32>` (string
   dictionary indices); fail on trailing bytes; add z to the bounds; skip `not_drawn` parts where the parts
   are drawn.
2. New `housing.bin` parser next to it:

```rust
pub struct HousingPieces { pub tile_types: Vec<HousingTileType> }
pub struct HousingTileType { pub tile_type: u32, pub pieces: Vec<HousingPiece> }
pub struct HousingPiece {
    pub category: u32,
    pub group: u32,
    pub feature_mask: u32,
    pub tooltip_cliloc: u32,
    pub solid: Vec<HousingSlot>,  // file order: solid, multi, window
    pub multis: Vec<HousingSlot>,
    pub windows: Vec<HousingSlot>,
}
pub struct HousingSlot { pub slot: u32, pub id: u32 }
```

   with named constants for the tile types (1..7) and the entitlement mask bits, and an exact-length check.

## 10. Ghidra names

41 functions renamed and tagged `UO_HOUSING` (plate comments with the evidence), one label
(`g_UOHouseCustomization` 0x00e37244); `g_ServerFeatureFlags` 0x00da987c kept and commented.

| Address | Name |
|---|---|
| 0x004aa720 | `UOHouseCustomization::Create` |
| 0x004cb700 | `UOResourceRequest::AddMultiCollectionKey` |
| 0x004cb770 | `UOResourceRequest::AddCustomHousingPiecesKey` |
| 0x006512c0 | `UOHouseCustomization::UOHouseCustomization` |
| 0x00651980 | `UOHouseCustomization::Initialize` |
| 0x00652090 | `UOHouseCustomization::BuildPieceTables` |
| 0x006526e0 | `UOHouseCustomization::LoadMultiAppearance` |
| 0x00652a20 | `UOHouseCustomization::PublishCustomizedHousing` |
| 0x006547d0 | `UOHouseCustomization::SelectPieceById` |
| 0x00654890 | `UOHouseCustomization::SelectMulti` |
| 0x006548d0 | `UOHouseCustomization::SelectTile` |
| 0x0065abf0 | `UOHouseCustomization::SlotAt` |
| 0x0065acc0 | `UOHouseCustomization::GroupEntryAt` |
| 0x0065ae00 | `UOHouseCustomization::CategoryEntryAt` |
| 0x0065af90 | `UOHouseCustomization::TileTypeEntryAt` |
| 0x0065b120 | `UOHouseCustomization::TileLookupAt` |
| 0x00835902 | `UOMultiCollection::UOMultiCollection` |
| 0x00835ad8 | `UOMultiCollection::SetPart` |
| 0x00835c6d | `UOMultiCollection::GetPart` |
| 0x0083ee1c | `UOCustomHousingPieces::UOCustomHousingPieces` |
| 0x0083eefe | `UOCustomHousingPiecesBinaryFactory::Create` |
| 0x0083eff3 | `UOCustomHousingPieces::Parse` |
| 0x0083f1f0 | `UOCustomHousingPieces::InitPiece` |
| 0x0083f279 | `UOCustomHousingPieces::ReadSlotList` |
| 0x0083f36f | `UOCustomHousingPieces::TileTypeAt` |
| 0x0083f42b | `UOCustomHousingPieces::AssignPieces` |
| 0x0083f511 | `UOCustomHousingPieces::PushPiece` |
| 0x00841c3c | `UOMultiCollectionBinaryFactory::Create` |
| 0x00841d31 | `UOMultiCollection::Parse` |
| 0x008460ef | `HousingPieceFilter::MatchesTileType` |
| 0x00846117 | `HousingPieceFilter::MatchesSlot` |
| 0x00846130 | `HousingPieceFilter::MatchesCategory` |
| 0x0084614a | `HousingPieceFilter::MatchesGroup` |
| 0x00846164 | `HousingPieceFilter::Describe` |
| 0x0084632e | `UOCustomHousingPieces::GetSolidTiles` |
| 0x008463f9 | `UOCustomHousingPieces::GetMultis` |
| 0x008464c4 | `UOCustomHousingPieces::GetWindowTiles` |
| 0x0084658f | `UOCustomHousingPieces::GetTooltip` |
| 0x00846629 | `UOCustomHousingPieces::GetCategoryRange` |
| 0x008466be | `UOCustomHousingPieces::GetFeatureMask` |
| 0x00846736 | `UOCustomHousingPieces::GetGroupRange` |
