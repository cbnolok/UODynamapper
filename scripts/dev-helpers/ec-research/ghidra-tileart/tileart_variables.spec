# Variable spec for the functions tagged UO_TILEART by NameTileartCode.java; applied by
# ../ApplyVariableSpecs.java (see README). Run this spec before the effects, animation and housing specs:
# it defines the shared /UO types they use. Offsets are from results/uosa_tileart.md (sections 1, 2, 8.4.1)
# and the instruction-level checks listed in the README. Parameters whose role is not established keep the
# default name ("-"). The UOObjectDefinitionBase methods take the definition in ECX as an explicit `def`
# parameter (custom storage): the derived /UOTileArt/UOObjectDefinition is the object they work on, and no
# separate base structure exists.
tag UO_TILEART

# ---- shared types ----

type /UO/SharedPtr 0x8 : boost::shared_ptr passed by value: object pointer and reference count block
  0x0 void* px
  0x4 void* pn
end

type /UO/AssetBlob 0xc : asset data read by UOAssets::StreamRead (only the fields it reads; the object may be larger)
  0x4 byte* data
  0x8 uint size
end

type /UO/AssetBlobSharedPtr 0x8 : shared_ptr<AssetBlob> passed by value to the asset factories and parsers
  0x0 /UO/AssetBlob* px
  0x4 void* pn
end

type /UO/AssetKey 0x8 : asset key pushed into a UOResourceRequest and passed to UOAssets::GetAsset
  0x0 uint id
  0x4 uint type : asset type in the top byte (0x06000000 tileart, 0x13000000 multi, 0x14000000 housing.bin)
end

type /UO/ObjectDefinitionSharedPtr 0x8 : shared_ptr<UOObjectDefinition> returned by UOTileArt::GetObjectDefinition
  0x0 /UOTileArt/UOObjectDefinition* px
  0x4 void* pn
end

type /UO/UOObject 0x58 : world object; only the fields the tileart code touches (the object is larger)
  0x8 uint tile_id : UOObject::GetObjectDefinition loads the definition of this tile id
  0x2c uint direction : UOObject::SetDirection
  0x38 qword flags1 : copied from the definition flags1
  0x44 /UO/ObjectDefinitionSharedPtr definition : cached by UOObject::GetObjectDefinition
  0x50 uint body_type : UOObject::SetBodyType
  0x54 bool alternate_animation : UOObject::SetAlternateAnimation
end

type /UO/HouseCell 0x90 : cell of the house customization grid; cell (level, x, y) is cells[level + (x * 31 + y) * 4]
  0x4 uint floor_tile_type : 1 stairs, 6 teleporter, 8 none
  0x14 undefined4 floor_piece : start of the floor piece record
  0x38 undefined4 slot_piece : start of the second piece slot record
  0x5c undefined4 object_piece : start of the object piece record
  0x6c bool supported_from_below : object supported from below
  0x6d bool floor_support
  0x6e bool slot_support
  0x6f bool floor_supported
  0x70 bool object_supported
  0x71 bool slot_supported
end

type /UO/UOHouseCustomization 0x877b8 : house customization object (g_UOHouseCustomization)
  0x50 /UO/HouseCell[3844] cells : 4 levels x 31 x 31
  0x876f4 int level_count : Initialize sets 4
  0x876f8 uint selected_id : tile or multi id
  0x876fc uint selected_tile_type
  0x87700 uint selected_slot
  0x87704 bool selected_is_multi
  0x87710 int origin_x
  0x87714 int origin_y
  0x87728 int width
  0x8772c int height
  0x87734 undefined4 tile_type_map : map tile type -> category -> group (0x1c bytes)
  0x87750 undefined4 tile_lookup : map<u32 tile id, {tile type, slot}> (0x1c bytes)
  0x8776c undefined4 multi_appearances : map multi id -> 10 part offsets and tiles (0x1c bytes)
end

class UOObjectDefinition /UOTileArt/UOObjectDefinition
class UOShaderDefinition /UOTileArt/UOShaderDefinition
class UOObject /UO/UOObject

# ---- asset stream and packages ----

function UOAssets::StreamRead
  proto __stdcall bool custom
  param stream /UO/AssetBlob* @ECX
  param position uint* @ESI
  param size uint @EDI
  param dest void* @stack:0x4
  param ok bool* @stack:0x8
end

function UOAssets::Initialize
  proto __cdecl void custom
  param - undefined4 @ECX
end

function UOAssets::MountPackages
  proto __stdcall undefined4
  param - undefined4
end

function UOStringDictionary::Load
  proto __stdcall void
  param manager void*
end

# (manager, out, id, type, 0); asset type in the top byte
function UOAssets::GetAsset
  proto __stdcall undefined4
  param manager void*
  param out_asset /UO/SharedPtr*
  param asset_id uint
  param asset_type uint
  param - uint
end

function UOAssets::TryResolvers
  proto __stdcall bool custom
  param resolvers void* @EDI
  param asset_id uint @stack:0x4
  param asset_type uint @stack:0x8
  param - undefined4 @stack:0xc
end

function UOAssets::ResolveAsset
  proto __stdcall void
  param asset_id uint
  param asset_type uint
  param - undefined4
  param - undefined4
  global ref:+0x8 g_UOAssetResolvers
end

function UOAssets::ReadPackagedFile
  proto __stdcall void custom
  param path /UOTileArt/MsvcString* @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
end

function UOAssets::ReadFileFromPath
  proto __stdcall void custom
  param - undefined4 @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
  param - undefined4 @stack:0xc
end

function UOAssets::CreateAssetFromBlob
  proto __stdcall undefined4
  param - undefined4
  param - undefined4
  param - undefined4
  param - undefined4
  param - undefined4
  param blob /UO/AssetBlobSharedPtr
end

function UOP::OpenFilePtr
  proto __cdecl void*
  param out_file void*
  param path char*
  param mode uint
end

function UOP::MountArchive
  proto __cdecl bool
  param package_name undefined4
end

# Jenkins lookup3 hashlittle2: pc and pb are the in/out seeds and results
function UOP::HashLittle2
  proto __cdecl void
  param key void*
  param length uint
  param pc uint*
  param pb uint*
end

function UOP::HashString
  proto __cdecl void custom
  param pb uint* @ECX
  param pc uint* @ESI
  param key char* @stack:0x4
end

function UOP::HashFileName
  proto __cdecl ulonglong
  param path char*
end

function UOP::LookupHash
  proto __stdcall bool custom
  param table void* @ECX
  param hash_lo uint @stack:0x4
  param hash_hi uint @stack:0x8
  param out_index uint* @stack:0xc
end

function UOP::FindEntryIndex
  proto __stdcall int custom
  param hash uint* @EAX
  param archive void* @stack:0x4
end

# before the size getters: they hand ECX on to it
function UOP::MypArchive_GetEntry
  proto __stdcall bool custom
  param index uint @EAX
  param archive void* @ECX
  param out_entry void* @stack:0x4
end

function UOP::MypArchive_GetEntryDecompressedSize
  proto __stdcall uint custom
  param archive void* @ECX
  param index uint @stack:0x4
end

function UOP::MypArchive_GetEntryCompressedSize
  proto __stdcall uint custom
  param archive void* @ECX
  param index uint @stack:0x4
end

function UOP::MypArchive_ReadEntry
  proto __stdcall bool custom
  param archive void* @ECX
  param index uint @stack:0x4
  param dest void* @stack:0x8
  param - undefined4 @stack:0xc
end

function UOP::MythMemoryInputFile_ctor
  proto __stdcall void* custom
  param size uint @ECX
  param file void* @stack:0x4
  param buffer void* @stack:0x8
  param - undefined4 @stack:0xc
end

function UOP::MythMypInputFile_ctor
  proto __stdcall void* custom
  param archive void* @ECX
  param file void* @stack:0x4
  param index uint @stack:0x8
  param - undefined4 @stack:0xc
end

# ---- definition loading and parsing ----

function UOTileArt::GetObjectDefinition
  proto __stdcall /UO/ObjectDefinitionSharedPtr* custom
  param out_definition /UO/ObjectDefinitionSharedPtr* @EDI
  param tile_id uint @stack:0x4
end

function UOTileArt::RequestObjectDefinition
  proto __cdecl /UO/ObjectDefinitionSharedPtr*
  param out_definition /UO/ObjectDefinitionSharedPtr*
  param tile_id uint
end

function UOObjectDefinitionBinaryFactory::NewInstance
  proto __stdcall /UO/ObjectDefinitionSharedPtr*
  param out_definition /UO/ObjectDefinitionSharedPtr*
end

function UOObjectDefinitionBinaryBaseFactory::NewInstance
  proto __stdcall undefined4
  param out_definition /UO/ObjectDefinitionSharedPtr*
end

function UOObjectDefinitionBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end

function UOObjectDefinitionBase::UOObjectDefinitionBase
  proto __stdcall /UOTileArt/UOObjectDefinition*
  param def /UOTileArt/UOObjectDefinition*
  global ref:+0x37 g_UOObjectDefinitionDefaultField60 uint
  global ref:+0x76 g_UOObjectDefinitionDefaultLightField70 uint
  global ref:+0x85 g_UOObjectDefinitionDefaultLightOffsetX float
  global ref:+0x9d g_UOObjectDefinitionDefaultLightOffsetY float
end

# EBP-based frame (PUSH EBP; SUB ESP before __EH_prolog): the first two arguments are not traced
function UOTileArt::ParseObjectDefinition
  proto __stdcall undefined4
  param - undefined4
  param - undefined4
  param blob /UO/AssetBlobSharedPtr
  param ok bool*
end

function UOTileArt::ParseStackAliases
  proto __stdcall void
  param definition /UO/ObjectDefinitionSharedPtr
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

function UOTileArt::ParseEffects
  proto __stdcall void
  param definition /UO/ObjectDefinitionSharedPtr
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

function UOTileArt::ParseProperties1
  proto __stdcall void custom
  param position uint* @ECX
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param ok bool* @stack:0x14
end

function UOTileArt::ParseProperties2
  proto __stdcall void custom
  param position uint* @ECX
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param ok bool* @stack:0x14
end

function UOTileArt::ParseAppearanceFilters
  proto __stdcall void custom
  param position uint* @ECX
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param ok bool* @stack:0x14
end

function UOTileArt::ParseSitting
  proto __stdcall void custom
  param position uint* @ECX
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param ok bool* @stack:0x14
end

function UOTileArt::ParseRadarColor
  proto __stdcall void custom
  param position uint* @ECX
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param ok bool* @stack:0x14
end

function UOTileArt::ParseShaderBlock
  proto __stdcall /UOTileArt/UOShaderDefinitionSharedPtr*
  param - undefined4
  param out_shader /UOTileArt/UOShaderDefinitionSharedPtr*
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

function UOTileArt::ParseAppearanceFilterByOwnerBodyType
  proto __cdecl void* custom
  param position uint* @ECX
  param blob /UO/AssetBlobSharedPtr @stack:0x4
  param ok bool* @stack:0xc
end

function UOTileArt::ParseAppearanceFilterByOwnerGender
  proto __cdecl void* custom
  param position uint* @ECX
  param blob /UO/AssetBlobSharedPtr @stack:0x4
  param ok bool* @stack:0xc
end

function UOTileArt::ParseAnimationAppearanceFilterByOwnerBodyType
  proto __cdecl void* custom
  param position uint* @ECX
  param blob /UO/AssetBlobSharedPtr @stack:0x4
  param ok bool* @stack:0xc
end

# UOTileArt::ParseEffects pushes ok, position, out, factories after copying the blob
function UOEffects::ParseEffectDefinition
  proto __stdcall /UO/SharedPtr*
  param factories void*
  param out_definition /UO/SharedPtr*
  param position uint*
  param ok bool*
  param blob /UO/AssetBlobSharedPtr
end

function UOTileArtTextureItem::Init
  proto __stdcall void custom
  param path char* @ECX
  param item /UOTileArt/UOTileArtTextureItem* @ESI
  param source_kind uint @stack:0x4
  param stretch float @stack:0x8
  param sampler_index uint @stack:0xc
  param uv_set uint @stack:0x10
end

# ---- appearance filters ----

function AppearanceFilterByOwnerBodyType::AppearanceFilterByOwnerBodyType
  proto __stdcall void*
  param filter void*
end

function AnimationAppearanceFilterByOwnerBodyType::AnimationAppearanceFilterByOwnerBodyType
  proto __stdcall void*
  param filter void*
end

function AppearanceFilterByOwnerGender::AppearanceFilterByOwnerGender
  proto __stdcall void* custom
  param gender int @ECX
  param filter void* @stack:0x4
  param appearance uint @stack:0x8
end

function AppearanceFilterByOwnerBodyType::AddBodyMapping
  proto __stdcall void custom
  param body uint @EAX
  param filter void* @stack:0x4
  param appearance uint @stack:0x8
end

# vtable +0xc; the constant result does not read ECX
function AppearanceFilterByOwnerBodyType::GetFilterType
  proto __thiscall uint custom
end

function AnimationAppearanceFilterByOwnerBodyType::GetFilterType
  proto __thiscall uint custom
end

function AppearanceFilterByOwnerGender::GetFilterType
  proto __thiscall uint custom
end

function AppearanceFilterByOwnerBodyType::Apply
  proto __thiscall bool custom
  param filter void* @ECX
  param owner /UOTileArt/UOObjectDefinition* @stack:0x4
  param - undefined4 @stack:0x8
  param out_appearance uint* @stack:0xc
end

function AppearanceFilterByOwnerGender::Apply
  proto __thiscall bool custom
  param filter void* @ECX
  param owner /UOTileArt/UOObjectDefinition* @stack:0x4
  param - undefined4 @stack:0x8
  param out_appearance uint* @stack:0xc
end

function AnimationAppearanceFilterByOwnerBodyType::Apply
  proto __thiscall bool custom
  param filter void* @ECX
  param owner /UOTileArt/UOObjectDefinition* @stack:0x4
  param - undefined4 @stack:0x8
  param out_appearance uint* @stack:0xc
end

# ---- object definition (vtable methods; `def` is ECX) ----

function UOObjectDefinition::GetPrimaryTextureLocation
  proto __thiscall undefined4 custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param - undefined4 @EBX
end

function UOObjectDefinition::GetSpriteDescriptor
  proto __thiscall void*
  param - undefined4
end

function UOObjectDefinition::GetMemorySize
  proto __thiscall uint
end

function UOObjectDefinitionBase::GetName
  proto __thiscall char* custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetTileId
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetEnhancedImageWindow
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_window /UOTileArt/UOTileArtImageWindow* @stack:0x4
end

function UOObjectDefinitionBase::GetLegacyImageWindow
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_window /UOTileArt/UOTileArtImageWindow* @stack:0x4
end

function UOObjectDefinitionBase::HasWorldArt
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::UsesEnhancedTileArt
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetWorldArtScale
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLegacyArtScale
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetField60
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasLegacyArtData
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLightScale
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLightField70
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLightOffsetX
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLightOffsetY
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasLightData
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasFlags1
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param mask_lo uint @stack:0x4
  param mask_hi uint @stack:0x8
end

function UOObjectDefinitionBase::GetFlags1
  proto __thiscall ulonglong custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasFlags2
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param mask_lo uint @stack:0x4
  param mask_hi uint @stack:0x8
end

function UOObjectDefinitionBase::GetFlags2
  proto __thiscall ulonglong custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasFlags1OrProperties1
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasFlags2OrProperties2
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetOldId
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetRotation
  proto __thiscall float custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetFacing
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetWorldArtShader
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_shader /UOTileArt/UOShaderDefinitionSharedPtr* @stack:0x4
end

function UOObjectDefinitionBase::GetLegacyShader
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_shader /UOTileArt/UOShaderDefinitionSharedPtr* @stack:0x4
end

function UOObjectDefinitionBase::GetEnhancedShader
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_shader /UOTileArt/UOShaderDefinitionSharedPtr* @stack:0x4
end

function UOObjectDefinitionBase::GetLightShader
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_shader /UOTileArt/UOShaderDefinitionSharedPtr* @stack:0x4
end

function UOObjectDefinitionBase::GetBodyType
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetAlternateAnimation
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit1
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit2
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit3
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit4
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit5
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsWestA
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsWestB
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsEastA
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsEastB
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsSouthA
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsSouthB
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsNorthA
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::ConnectsNorthB
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit14
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasHouseJoinBit15
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetHouseJoinBits
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::SetHouseJoinBits
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param bits uint @stack:0x4
end

function UOObjectDefinitionBase::HasSitting
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSittingDirections
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSeatOffsetX
  proto __thiscall int custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSeatOffsetY
  proto __thiscall int custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSittingField10
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSittingField14
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetSittingWindowShift
  proto __thiscall int custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::SetSitting
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param sitting /UOTileArt/UOTileArtSittingSharedPtr @stack:0x4
end

function UOObjectDefinition::GetEffectCount
  proto __thiscall uint
end

function UOObjectDefinition::GetEffect
  proto __thiscall void
  param out_effect /UO/SharedPtr*
  param index uint
end

function UOObjectDefinition::SetWorldArtShader
  proto __thiscall void
  param shader /UOTileArt/UOShaderDefinitionSharedPtr
end

function UOObjectDefinition::SetLegacyShader
  proto __thiscall void
  param shader /UOTileArt/UOShaderDefinitionSharedPtr
end

function UOObjectDefinition::SetEnhancedShader
  proto __thiscall void
  param shader /UOTileArt/UOShaderDefinitionSharedPtr
end

function UOObjectDefinition::SetLightShader
  proto __thiscall void
  param shader /UOTileArt/UOShaderDefinitionSharedPtr
end

function UOObjectDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOObjectDefinitionBase::GetWorldArtShaderName
  proto __thiscall char* custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLegacyShaderName
  proto __thiscall char* custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetLightShaderName
  proto __thiscall char* custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::SetProperty1
  proto __stdcall void custom
  param def /UOTileArt/UOObjectDefinition* @EAX
  param key uint @stack:0x4
  param value uint @stack:0x8
end

function UOObjectDefinitionBase::HasProperty1
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param key uint @stack:0x4
end

function UOObjectDefinitionBase::GetProperty1
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param key uint @stack:0x4
end

function UOObjectDefinitionBase::SetProperty2
  proto __stdcall void custom
  param def /UOTileArt/UOObjectDefinition* @EAX
  param key uint @stack:0x4
  param value uint @stack:0x8
end

function UOObjectDefinitionBase::HasProperty2
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param key uint @stack:0x4
end

function UOObjectDefinitionBase::GetProperty2
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param key uint @stack:0x4
end

function UOObjectDefinitionBase::AddStackAlias
  proto __stdcall void custom
  param def /UOTileArt/UOObjectDefinition* @ESI
  param amount uint @stack:0x4
  param tile_id uint @stack:0x8
end

function UOObjectDefinitionBase::AddAppearanceFilter
  proto __stdcall void custom
  param def /UOTileArt/UOObjectDefinition* @EDI
  param filter void* @stack:0x4
end

function UOObjectDefinitionBase::GetAppearance
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
end

function UOObjectDefinitionBase::GetAnimationAppearance
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
end

function UOObjectDefinitionBase::GetGraphicForAmount
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param amount uint @stack:0x4
end

function UOObjectDefinitionBase::HasRace
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetRace
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::HasGender
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetGender
  proto __thiscall uint custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

function UOObjectDefinitionBase::GetRadarColor
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param out_rgba uint* @stack:0x4
end

function UOObjectDefinitionBase::SetRadarColor
  proto __thiscall void custom
  param def /UOTileArt/UOObjectDefinition* @ECX
  param rgba uint @stack:0x4
end

function UOObjectDefinitionBase::HasRadarColor
  proto __thiscall bool custom
  param def /UOTileArt/UOObjectDefinition* @ECX
end

# ---- shader definition ----

function UOShaderDefinition::GetTextureCount
  proto __thiscall uint
end

function UOShaderDefinition::GetTexture
  proto __thiscall void
  param out_texture /UO/SharedPtr*
  param index uint
end

function UOShaderDefinition::GetField04
  proto __thiscall uint
end

function UOShaderDefinition::GetShaderName
  proto __thiscall /UOTileArt/MsvcString*
  param out_name /UOTileArt/MsvcString*
end

function UOShaderDefinition::AddTexture
  proto __thiscall void
  param texture /UO/SharedPtr
end

function UOShaderDefinition::SetField04
  proto __thiscall void
  param value uint
end

function UOShaderDefinition::SetShaderName
  proto __thiscall void
  param name /UOTileArt/MsvcString*
end

# returns 4 without reading ECX
function UOShaderDefinition::GetTechniqueCount
  proto __thiscall uint custom
end

function UOShaderDefinition::GetTweakCount
  proto __thiscall uint custom
end

function UOShaderDefinition::GetTechniqueValues
  proto __thiscall int*
end

function UOShaderDefinition::GetTweakValues
  proto __thiscall float*
end

function UOShaderDefinition::SetTechniqueValue
  proto __thiscall void
  param index uint
  param value int
end

function UOShaderDefinition::SetTweakValue
  proto __thiscall void
  param index uint
  param value float
end

function UOShaderDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

# ---- objects, components and sprites ----

# the table maps direction 0 -> 1, 2 -> 2, 4 -> 4, 6 -> 8
function UOObject::NearestAllowedDirection
  proto __cdecl uint custom
  param direction uint @EAX
  param allowed_mask uint @ECX
  global ref:+0x3 g_UODirectionSittingMask uint[8]
end

function UOObject::SetBodyType
  proto __thiscall void
  param body_type uint
end

function UOObject::GetBodyType
  proto __thiscall uint
end

function UOObject::SetAlternateAnimation
  proto __thiscall void
  param value bool
end

function UOObject::GetAlternateAnimation
  proto __thiscall bool
end

function UOObject::SetDirection
  proto __thiscall void
  param direction int
end

function UOObject::GetFlags1
  proto __thiscall ulonglong
end

function UOObject::GetObjectDefinition
  proto __stdcall /UO/ObjectDefinitionSharedPtr* custom
  param object /UO/UOObject* @EAX
  param out_definition /UO/ObjectDefinitionSharedPtr* @stack:0x4
end

function UOObject::InitFromDefinition
  proto __stdcall undefined4
  param - undefined4
  param object /UO/UOObject*
end

function UOObject::CreateComponents
  proto __stdcall void
  param - undefined4
  param - undefined4
  param object /UO/UOObject*
  param - undefined4
  param definition /UO/ObjectDefinitionSharedPtr
end

function UOTileArt::GetGraphicForAmount
  proto __stdcall ushort
  param tile_id ushort
  param amount uint
end

function UOObjectComponentSprite::Initialize
  proto __thiscall undefined4 custom
  param component void* @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
  param definition /UO/ObjectDefinitionSharedPtr @stack:0xc
end

function UOObjectComponentLighting::Initialize
  proto __thiscall undefined4 custom
  param component void* @ECX
  param - undefined4 @stack:0x4
  param - undefined4 @stack:0x8
  param definition /UO/ObjectDefinitionSharedPtr @stack:0xc
end

function UOTileArt::GetCachedSpriteDescriptor
  proto __cdecl void*
  param use_legacy bool
  param definition /UO/ObjectDefinitionSharedPtr
  global ref:+0x7 g_UOTileArtSpriteDescriptorLock
  global ref:+0x21 g_UOTileArtSpriteDescriptorMap
end

function UOTileArt::BuildSpriteDescriptor
  proto __stdcall void* custom
  param descriptor void* @EAX
  param use_legacy bool @stack:0x4
  param definition /UO/ObjectDefinitionSharedPtr @stack:0x8
end

function SpriteTextureFileLocation::ConstructFromPath
  proto __cdecl void custom
  param path char* @ECX
  param location void* @ESI
end

function UOPaperdoll::BuildImage
  proto __cdecl bool custom
  param - undefined4 @ECX
end

# ---- house customization support (section 8.4.1); cell (level, x, y) = cells[level + (x * 31 + y) * 4] ----

function UOHouseCustomization::SetPieceSupportHue
  proto __cdecl void
  param supported bool
  param - undefined4
  param - undefined4
  param piece void*
  param - undefined4
end

function UOHouseCustomization::UpdateCellSupportHues
  proto __cdecl void custom
  param cell /UO/HouseCell* @ESI
end

function UOHouseCustomization::IsClearOfStairs
  proto __stdcall bool custom
  param x int @ECX
  param level int @EDX
  param house /UO/UOHouseCustomization* @ESI
  param y int @stack:0x4
end

function UOHouseCustomization::IsNotAboveStairs
  proto __stdcall bool custom
  param x int @EAX
  param level int @EDX
  param y int @EDI
  param house /UO/UOHouseCustomization* @stack:0x4
end

# position is the world {x, y}; tile_type 1 stairs, 4, 6 teleporter take the stairs checks
function UOHouseCustomization::IsPlacementAllowed
  proto __stdcall bool custom
  param tile_id uint @EAX
  param house /UO/UOHouseCustomization* @ECX
  param position int* @stack:0x4
  param tile_type uint @stack:0x8
end

function UOHouseCustomization::HasUnsupportedPieces
  proto __cdecl bool custom
  param house /UO/UOHouseCustomization* @EAX
end

function UOHouseCustomization::ShowSupport
  proto __stdcall void
  param house /UO/UOHouseCustomization*
end

function UOHouseCustomization::UpdateSupport
  proto __stdcall void
  param house /UO/UOHouseCustomization*
  param - undefined4
end

function UOHouseCustomization::TilesConnect
  proto __stdcall bool custom
  param tile_id uint @EAX
  param cell /UO/HouseCell* @ECX
  param dx int @stack:0x4
  param dy int @stack:0x8
end

function UOHouseCustomization::IsConnectedToSupportedNeighbour
  proto __stdcall bool custom
  param x int @ESI
  param house /UO/UOHouseCustomization* @stack:0x4
  param y int @stack:0x8
  param level int @stack:0xc
end

function UOHouseCustomization::ResetLevelSupport
  proto __cdecl void custom
  param level int @EAX
  param house /UO/UOHouseCustomization* @EDX
  param - undefined4 @EDI
end

function UOHouseCustomization::MarkSupportAbove
  proto __stdcall undefined4 custom
  param level int @EAX
  param house void* @EDI
  param x int @stack:0x4
  param y int @stack:0x8
  param piece_below_definition /UOTileArt/UOObjectDefinition* @stack:0xc
  param piece_below_refcount void* @stack:0x10
end

function UOHouseCustomization::PropagateSupportFromBelow
  proto __stdcall undefined4
  param house /UO/UOHouseCustomization*
  param - undefined4
  param level int
end

function UOHouseCustomization::ComputeObjectSupport
  proto __stdcall undefined4
  param house /UO/UOHouseCustomization*
  param - undefined4
  param level int
end
