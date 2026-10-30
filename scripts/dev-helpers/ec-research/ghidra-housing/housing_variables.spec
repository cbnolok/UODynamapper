# Variable spec for the functions tagged UO_HOUSING by NameHousingCode.java; applied by
# ../ApplyVariableSpecs.java after ../ghidra-tileart/tileart_variables.spec (shared /UO types). Layouts are from
# results/uosa_housing.md; /UO/HouseCell and /UO/UOHouseCustomization come from the tileart spec. Many functions take their
# object in a register (LTCG), hence the custom prototypes.
tag UO_HOUSING

# ---- types ----

type /UO/HousingPieceFilter 0x14 : query filter of the UOCustomHousingPieces getters
  0x0 uint tile_type : 0 = any
  0x4 int category : -1 = any
  0x8 int group : -1 = any
  0xc uint expansion_mask : client expansion filter 0x6f3bff
  0x10 uint slot : 0 = any
end

type /UO/HousingSlotEntry 0x8 : {slot, id} entry of a piece list
  0x0 uint slot
  0x4 uint id : tile id (solid, window) or multi id
end

type /UO/HousingPiece 0x40 : housing.bin piece
  0x0 uint category
  0x4 uint group
  0x8 uint feature_mask
  0xc uint tooltip_cliloc
  0x10 /UOTileArt/MsvcVector solid_tiles : vector<HousingSlotEntry>
  0x20 /UOTileArt/MsvcVector multis : vector<HousingSlotEntry>
  0x30 /UOTileArt/MsvcVector window_tiles : vector<HousingSlotEntry>
end

type /UO/UOCustomHousingPieces 0x10 : housing.bin asset (type 0x14)
  0x0 void* vftable
  0x4 /UOTileArt/MsvcMap pieces : map<u32 tile type, vector<HousingPiece>>
end

type /UO/UOMultiPart 0x1c : MultiCollection part
  0x0 ushort tile
  0x2 short x
  0x4 short y
  0x6 short z
  0x8 ushort field_08 : always 0
  0xa byte not_drawn
  0xb byte flag2
  0xc /UOTileArt/MsvcVector strings : vector<std::string> part tags
end

type /UO/UOMultiCollection 0x4c : MultiCollection asset (type 0x13)
  0x0 void* vftable
  0x4 uint multi_id
  0x8 uint part_count
  0xc /UO/UOMultiPart* parts
  0x2c bool first_part : set until the first part grows the bounds
  0x30 int min_x
  0x34 int min_y
  0x38 int min_z
  0x3c int max_x
  0x40 int max_y
  0x44 int max_z
  0x48 uint largest_extent : maximum per-part metric
end

class HousingPieceFilter /UO/HousingPieceFilter

# ---- house customization ----

function UOHouseCustomization::Create
  proto __cdecl bool
end

function UOHouseCustomization::UOHouseCustomization
  proto __cdecl /UO/UOHouseCustomization* custom
  param house /UO/UOHouseCustomization* @EDI
end

function UOHouseCustomization::Initialize
  proto __cdecl bool custom
  param house /UO/UOHouseCustomization* @EDI
end

function UOHouseCustomization::BuildPieceTables
  proto __stdcall void
  param house /UO/UOHouseCustomization*
  local stack:-0x60 request void*
  local stack:-0x70 pieces /UO/UOCustomHousingPieces*
  local stack:-0x90 tile_type uint
  local stack:-0x94 filter_tile_type
  local stack:-0x98 category int
  local stack:-0x64 category_max int
  local stack:-0x8c filter /UO/HousingPieceFilter
  local call:TileTypeEntryAt tile_type_entry
  local call:CategoryEntryAt category_entry
  local stack:-0x24 server_feature_flags uint
  local stack:-0x38 solid_tiles /UOTileArt/MsvcVector
  local stack:-0x58 window_tiles /UOTileArt/MsvcVector
  local stack:-0x48 multis /UOTileArt/MsvcVector
  local stack:-0x74 tile_id uint
end

function UOHouseCustomization::LoadMultiAppearance
  proto __stdcall void
  param house /UO/UOHouseCustomization*
  param multi_id uint
  local stack:-0x24 request void*
  local stack:-0x2c collection /UO/UOMultiCollection*
  local stack:-0x30 appearance
  local stack:-0x34 appearance_tile uint*
  local stack:-0x3c part_tile
  local stack:-0x38 part_field_08
  local stack:-0x3d part_flag2
  local stack:-0x1c part_strings /UOTileArt/MsvcVector
end

function UOHouseCustomization::PublishCustomizedHousing
  proto __stdcall void
  param house /UO/UOHouseCustomization*
end

function UOHouseCustomization::SelectPieceById
  proto __stdcall bool* custom
  param house /UO/UOHouseCustomization* @ESI
  param tile_id uint @EDI
  param result bool* @stack:0x4
end

function UOHouseCustomization::SelectMulti
  proto __cdecl void custom
  param house /UO/UOHouseCustomization* @EAX
  param multi_id uint @ECX
end

function UOHouseCustomization::SelectTile
  proto __stdcall bool custom
  param tile_type uint @ECX
  param tile_id uint @EDX
  param house /UO/UOHouseCustomization* @stack:0x4
  param slot uint @stack:0x8
end

# map operator[] helpers: the map in one register or on the stack, the key by pointer
function UOHouseCustomization::SlotAt
  proto __cdecl uint* custom
  param slot_map void* @ECX
  param slot uint* @EDI
end

function UOHouseCustomization::GroupEntryAt
  proto __stdcall void* custom
  param group int* @EAX
  param group_map void* @stack:0x4
end

function UOHouseCustomization::CategoryEntryAt
  proto __stdcall void* custom
  param category int* @EDX
  param category_map void* @stack:0x4
end

function UOHouseCustomization::TileTypeEntryAt
  proto __stdcall void* custom
  param tile_type uint* @EDX
  param tile_type_map void* @stack:0x4
end

function UOHouseCustomization::TileLookupAt
  proto __cdecl uint* custom
  param tile_id uint* @EAX
  param tile_lookup void* @ECX
end

# ---- resource request keys ----

function UOResourceRequest::AddMultiCollectionKey
  proto __stdcall void custom
  param request void* @EAX
  param multi_id uint @stack:0x4
end

function UOResourceRequest::AddCustomHousingPiecesKey
  proto __cdecl void custom
  param request void* @EAX
end

# ---- MultiCollection ----

function UOMultiCollection::UOMultiCollection
  proto __stdcall /UO/UOMultiCollection*
  param collection /UO/UOMultiCollection*
  param multi_id uint
  param part_count uint
end

function UOMultiCollection::SetPart
  proto __stdcall void custom
  param index uint @EAX
  param collection /UO/UOMultiCollection* @stack:0x4
  param tile ushort @stack:0x8
  param x int @stack:0xc
  param y int @stack:0x10
  param z int @stack:0x14
  param not_drawn byte @stack:0x18
  param flag2 byte @stack:0x1c
  param strings /UOTileArt/MsvcVector* @stack:0x20
  param ok bool* @stack:0x24
end

function UOMultiCollection::GetPart
  proto __stdcall void custom
  param index uint @EAX
  param out_xyz int* @ECX
  param collection /UO/UOMultiCollection* @stack:0x4
  param out_tile ushort* @stack:0x8
  param out_field_08 ushort* @stack:0xc
  param out_not_drawn byte* @stack:0x10
  param out_flag2 byte* @stack:0x14
  param out_strings /UOTileArt/MsvcVector* @stack:0x18
  param ok bool* @stack:0x1c
end

function UOMultiCollectionBinaryFactory::Create
  proto __stdcall /UO/SharedPtr*
  param out_asset /UO/SharedPtr*
  param asset_id uint
  param asset_type uint
  param blob /UO/AssetBlobSharedPtr
end

function UOMultiCollection::Parse
  proto __stdcall /UO/UOMultiCollection*
  param blob /UO/AssetBlobSharedPtr
  param ok bool*
  local stack:-0x24 position uint
  local stack:-0x28 part_count uint
  local stack:-0x34 part_index uint
  local stack:-0x20 x short
  local stack:-0x1c y short
  local stack:-0x18 z short
  local stack:-0x12 not_drawn char
  local stack:-0x11 flag2 char
  local stack:-0x38 string_count int
  local stack:-0x30 strings_left int
  local stack:-0x3c string_index int
  local stack:-0x5c strings /UOTileArt/MsvcVector
end

# ---- housing.bin ----

function UOCustomHousingPieces::UOCustomHousingPieces
  proto __stdcall /UO/UOCustomHousingPieces*
  param pieces /UO/UOCustomHousingPieces*
end

function UOCustomHousingPiecesBinaryFactory::Create
  proto __stdcall /UO/SharedPtr*
  param out_asset /UO/SharedPtr*
  param asset_id uint
  param asset_type uint
  param blob /UO/AssetBlobSharedPtr
end

# The compiler keeps `ok` in EBX and reuses its argument slot for the read position.
function UOCustomHousingPieces::Parse
  proto __stdcall /UO/UOCustomHousingPieces*
  param blob /UO/AssetBlobSharedPtr
  param ok bool*
  local stack:-0x14 result /UO/UOCustomHousingPieces*
  local stack:-0x1c tile_types_left int
  local stack:-0x20 piece_count uint
  local stack:-0x24 category uint
  local stack:-0x28 group uint
  local stack:-0x2c feature_mask uint
  local stack:-0x30 tooltip_cliloc uint
  local stack:-0x88 piece /UO/HousingPiece
  local stack:-0x48 tile_type_pieces /UOTileArt/MsvcVector
end

function UOCustomHousingPieces::InitPiece
  proto __stdcall void
  param piece /UO/HousingPiece*
end

function UOCustomHousingPieces::ReadSlotList
  proto __stdcall void custom
  param position uint* @ECX
  param out_list /UOTileArt/MsvcVector* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param ok bool* @stack:0x10
end

function UOCustomHousingPieces::TileTypeAt
  proto __stdcall /UOTileArt/MsvcVector* custom
  param pieces_map /UOTileArt/MsvcMap* @ECX
  param tile_type uint* @stack:0x4
end

function UOCustomHousingPieces::AssignPieces
  proto __cdecl /UOTileArt/MsvcVector* custom
  param dest /UOTileArt/MsvcVector* @EAX
  param source /UOTileArt/MsvcVector* @ESI
end

function UOCustomHousingPieces::PushPiece
  proto __stdcall void custom
  param pieces_vector /UOTileArt/MsvcVector* @EAX
  param piece /UO/HousingPiece* @stack:0x4
end

# Getters: (pieces in EAX, result, filter, outputs); they return the result pointer.
function UOCustomHousingPieces::GetSolidTiles
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_list /UOTileArt/MsvcVector* @stack:0xc
end

function UOCustomHousingPieces::GetMultis
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_list /UOTileArt/MsvcVector* @stack:0xc
end

function UOCustomHousingPieces::GetWindowTiles
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_list /UOTileArt/MsvcVector* @stack:0xc
end

function UOCustomHousingPieces::GetTooltip
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_tooltip uint* @stack:0xc
end

function UOCustomHousingPieces::GetCategoryRange
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param out_min int* @EDI
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_max int* @stack:0xc
end

function UOCustomHousingPieces::GetFeatureMask
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_mask uint* @stack:0xc
end

function UOCustomHousingPieces::GetGroupRange
  proto __stdcall bool* custom
  param pieces /UO/UOCustomHousingPieces* @EAX
  param result bool* @stack:0x4
  param filter /UO/HousingPieceFilter* @stack:0x8
  param out_min int* @stack:0xc
  param out_max int* @stack:0x10
end

# ---- filter ----

function HousingPieceFilter::MatchesTileType
  proto __stdcall bool custom
  param filter /UO/HousingPieceFilter* @EAX
  param tile_type uint @stack:0x4
end

function HousingPieceFilter::MatchesSlot
  proto __stdcall bool custom
  param filter /UO/HousingPieceFilter* @EAX
  param slot uint @stack:0x4
end

function HousingPieceFilter::MatchesCategory
  proto __stdcall bool custom
  param filter /UO/HousingPieceFilter* @EAX
  param category int @stack:0x4
end

function HousingPieceFilter::MatchesGroup
  proto __stdcall bool custom
  param filter /UO/HousingPieceFilter* @EAX
  param group int @stack:0x4
end

function HousingPieceFilter::Describe
  proto __thiscall /UOTileArt/MsvcString*
  param out_text /UOTileArt/MsvcString*
end
