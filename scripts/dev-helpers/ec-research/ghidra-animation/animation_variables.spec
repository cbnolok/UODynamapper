# Variable spec for the functions tagged UO_ANIMATION by NameAnimationCode.java; applied by
# ../ApplyVariableSpecs.java after ../ghidra-tileart/tileart_variables.spec (shared /UO types) and
# ../ghidra-effects/effects_variables.spec. Layouts are from results/uosa_animation.md; the MSVC container types
# (MsvcMap, MsvcVector, MsvcString) are the /UOEffects ones created by NameEffectCode.java.
#
# Conventions checked at the instruction level:
# - The UOAnimationSequence lookups are mostly __stdcall (sequence, action, ...); a few keep the sequence in
#   EAX (NextBaseFrame, IsStandAction, GetDirectionFrames, ResolveAction, FrameToAction, IsFrameSkipped), EDI
#   (GetActionFps) or ECX (MapCharacterAnimationAction, AddAliasAction, AddFrameEffects, AddSkippedFrame,
#   GetFrameEffects, GetOffset*). ActionAt takes the action map itself in EDI.
# - Functions that report failure through 0x0040c6b0 / 0x0040c6c0 take an `ok` byte pointer (PlayAction,
#   AddFrameSet, SetNext, SetStartFrameSets, the parsers) and usually return it.
# - The animation components have no structure: their object is an explicit ECX (or stack) parameter
#   `component`, so that Ghidra does not create an empty class structure. Their sequence shared_ptr is at
#   +0x7c/+0x80 (read by OnCharacterAnimation, SelectAnimationGroup, PlayTransitionAction, GetLayerAction).
# - The asset factory Create methods do not read ECX: (out asset, asset id, asset type, blob shared_ptr).
# - GetActionFps returns its float in XMM0, which no Ghidra x86-32 prototype models: it stays `void`.
tag UO_ANIMATION

# ---- types ----

type /UO/AnimationFrameFile 0x28 : header of an AnimationFrame*.uop entry (the client reads 0x18..0x27 only)
  0x0 uint signature : 'AMOU'
  0x4 uint version : 1 in every file
  0x8 uint total_size : the entry size
  0xc uint animation_id : body of the entry name, never read
  0x18 uint colours_count
  0x1c uint colours_offset : from the header start; 4-byte entries [b0, b1, b2, keep colour flag]
  0x20 uint frames_count
  0x24 uint frames_offset : from the header start; sorted by (action, number)
end

type /UO/AnimationFrameEntry 0x10 : frame entry of an AnimationFrame*.uop file
  0x0 ushort action
  0x2 ushort number
  0x4 short x0 : box relative to the anchor
  0x6 short y0
  0x8 short x1
  0xa short y1
  0xc uint offset : RLE data at the entry address + offset
end

type /UO/UOMobFrameSet 0x1c : asset of type 0x10 (one AnimationFrame*.uop entry)
  0x0 void* vftable
  0x4 uint signature
  0x8 uint version
  0xc uint total_size
  0x10 /UO/AnimationFrameFile* data : the blob data
  0x14 /UO/AssetBlobSharedPtr blob
end

type /UO/AnimationActionRecord 0xc8 : value of the UOAnimationSequence action map
  0x0 uint action
  0x4 /UOEffects/MsvcString name : printed by the duplicate-action log
  0x20 uint frame_count : frames per direction; 0 in alias records
  0x24 int alias : -1 none; ResolveAction follows one level
  0x28 uint base_frame : NextBaseFrame; not in the file
  0x2c float fps : frame interval = 1 / fps
  0x30 /UOEffects/MsvcMap effect_frames : map<u32 frame, effect collection>
  0x3c /UOEffects/MsvcMap skipped_frames : map<u32 frame, bool>
  0x48 int[8] offset_x : per direction rider offset (i8 in the file, -128 -> 0)
  0x68 int[8] offset_y
  0x88 int[8] extra_offset_x : added to the i8 offset (i16 in the file, -0x8000 -> header value)
  0xa8 int[8] extra_offset_y
end

type /UO/UOAnimationSequence 0xc4 : asset of type 0x0c (AnimationSequence.uop entry)
  0x0 void* vftable
  0x4 /UOEffects/MsvcMap groups : map<i8 key, FrameSetLookup*>
  0x10 uint stored_id : the sequence's own body identity (wear check)
  0x14 /UOEffects/MsvcString name : constructed empty
  0xb4 uint max_action : highest action id added (AddAction, AddAliasAction)
  0xb8 /UOEffects/MsvcMap actions : map<u32 action, AnimationActionRecord>
end

type /UO/FrameSetLookup 0x30 : animation group of a sequence
  0x0 void* vftable
  0x4 /UOEffects/MsvcMap transitions : map<i8 key, u32 action>
  0x10 /UOEffects/MsvcMap moves : map<u8 move, {action, alternate}>
  0x1c /UOEffects/MsvcMap typed : map<{i8 type, u8 sub}, vector<u32>>
  0x28 void* parent : FrameSetLookup searched when a lookup misses
  0x2c uint default_action
end

type /UO/UOAnimationDefinition 0x48 : asset of type 0x0b (AnimationDefinition.uop entry), also built for mobiles
  0x0 void* vftable
  0x4 uint stored_id
  0x8 uint start_set : key of the first frame set
  0xc uint second_set
  0x14 float value : >= 0; 0.0 in all data
  0x18 /UOEffects/MsvcMap frame_sets : map<u32 key, frame set shared_ptr>
end

type /UO/UOAnimationFrameSet 0x44 : frame set of a UOAnimationDefinition
  0x0 void* vftable
  0x4 uint key
  0xc /UOEffects/MsvcString name
  0x28 uint frame_count
  0x2c void* frames : frame_count entries of 0x14 bytes
  0x30 /UO/SharedPtr timing : SetFrameInterval (GetSharedTiming)
  0x38 /UO/SharedPtr next : SetNext; the set played after this one
end

type /UO/CharacterAnimationPacket 0x10 : packet 0x6E as read by UOPacket::Read_0x6E_CharacterAnimation
  0x0 uint serial
  0x4 ushort action : the only field used
  0x6 ushort frame_count
  0x8 ushort repeat_count
  0xa bool forward
  0xb bool repeat
  0xc byte delay
end

type /UO/NewCharacterAnimationPacket 0xc : packet 0xE2 as read by UOPacket::Read_0xE2_NewCharacterAnimation
  0x0 uint serial
  0x4 ushort type
  0x6 ushort sub_type
  0x8 byte variant
end

class UOMobFrameSet /UO/UOMobFrameSet
class UOAnimationSequence /UO/UOAnimationSequence
class FrameSetLookup /UO/FrameSetLookup
class UOAnimationDefinition /UO/UOAnimationDefinition
class UOAnimationFrameSet /UO/UOAnimationFrameSet

# ---- hues and frame decoding ----

function UOHues::LoadHueBitmap
  proto __cdecl bool custom
  param out_table uint* @EDI
  param path char* @stack:0x4
  param ok bool* @stack:0x8
end

function UOHues::LoadHueTables
  proto __cdecl bool*
  param ok bool*
  param - char
end

function UOHues::GetHueTable
  proto __cdecl uint* custom
  param hue uint @ECX
end

function UOAnimations::GetLayerBounds
  proto __cdecl void custom
  param layers /UOEffects/MsvcVector* @EAX
  param out_min_x int* @stack:0x4
  param out_min_y int* @stack:0x8
  param out_max_x int* @stack:0xc
  param out_max_y int* @stack:0x10
end

function UOAnimations::BuildFramePalette
  proto __cdecl /UO/SharedPtr* custom
  param frame_file /UO/AnimationFrameFile* @EAX
  param hue uint @ECX
  param out_palette /UO/SharedPtr* @stack:0x4
end

function UOAnimations::DecodeFrameRle
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EAX
  param palette uint* @stack:0x4
  param canvas uint* @stack:0x8
  param pitch int @stack:0xc
end

function UOAnimations::DecodeFrameRleMirrored
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EAX
  param palette uint* @stack:0x4
  param canvas uint* @stack:0x8
  param pitch int @stack:0xc
end

function UOAnimations::DecodeFrameMask
  proto __cdecl void custom
  param entry /UO/AnimationFrameEntry* @EDX
  param mask byte* @stack:0x4
  param pitch int @stack:0x8
  param mark byte @stack:0xc
end

# ---- tile animation component ----

function UOObjectComponentAnimationTile::LoadAnimationDefinition
  proto __stdcall /UO/SharedPtr*
  param out_definition /UO/SharedPtr*
  param item_id uint
end

function UOObjectComponentAnimationTile::Initialize
  proto __thiscall undefined4 custom
  param component void* @ECX
  param object void* @stack:0x4
  param object_refcount void* @stack:0x8
  param - undefined4 @stack:0xc
  param - int* @stack:0x10
end

function UOObjectComponentAnimationTile::Update
  proto __thiscall void custom
  param component void* @ECX
  param dt float @stack:0x4
end

function UOObjectComponentAnimationTile::PlayAction
  proto __thiscall bool* custom
  param component void* @ECX
  param ok bool* @stack:0x4
  param frame_set_key uint @stack:0x8
end

# ---- frame sets, definitions and mobile layers ----

function UOAnimationFrameSet::UOAnimationFrameSet
  proto __stdcall void custom
  param frame_set /UO/UOAnimationFrameSet* @ESI
  param key uint @EAX
  param - byte @CL
  param frame_count uint @EDI
  param name char* @stack:0x4
end

function UOAnimationDefinition::UOAnimationDefinition
  proto __cdecl void custom
  param definition /UO/UOAnimationDefinition* @ESI
  param stored_id uint @EAX
end

function UOAnimations::GetAnimationFrameSet
  proto __cdecl void custom
  param body uint @EAX
  param out_frame_set /UO/SharedPtr* @ESI
  param action uint @stack:0x4
end

function UOAnimations::BuildFrameAtlas
  proto __stdcall void*
  param - void*
  param out_atlas void*
  param layers /UOEffects/MsvcVector*
end

function UOAnimations::BuildMobileAnimation
  proto __cdecl void
  param out_definition /UO/SharedPtr*
  param out_sequence /UO/SharedPtr*
  param object void*
  param object_refcount void*
end

function UOObjectComponentAnimationMob::Initialize
  proto __thiscall undefined4 custom
  param component void* @ECX
  param object void* @stack:0x4
  param object_refcount void* @stack:0x8
  param - int* @stack:0xc
  param - int* @stack:0x10
end

function UOObjectComponentAnimationMob::PlayAction
  proto __thiscall bool* custom
  param component void* @ECX
  param ok bool* @stack:0x4
  param action uint @stack:0x8
end

function UOObjectComponentAnimationMob::Update
  proto __thiscall void custom
  param component void* @ECX
  param dt float @stack:0x4
end

function UOObjectComponentAnimationMob::UpdateFrame
  proto __stdcall void
  param component void*
end

function UOAnimations::SelectAnimationGroup
  proto __cdecl int custom
  param component void* @ECX
end

function UOObjectComponentAnimationMob::OnCharacterAnimation
  proto __stdcall void custom
  param component void* @EDI
  param packet /UO/CharacterAnimationPacket* @stack:0x4
end

function UOObjectComponentAnimationMob::OnNewCharacterAnimation
  proto __stdcall void custom
  param packet /UO/NewCharacterAnimationPacket* @EAX
  param component void* @stack:0x4
end

function UOObjectComponentAnimationMob::BuildLayers
  proto __stdcall undefined4
  param component void*
  param - void*
  param - void*
  param sequence /UO/UOAnimationSequence*
  param sequence_refcount void*
end

function UOAnimations::GetLayerAction
  proto __stdcall uint custom
  param component void* @EAX
  param definition /UO/UOAnimationDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
end

function UOAnimations::LoadLayerFrameSet
  proto __stdcall /UO/SharedPtr*
  param component void*
  param out_frame_set /UO/SharedPtr*
  param id uint
  param any_action bool
  param definition /UO/UOAnimationDefinition*
  param definition_refcount void*
end

function UOObjectComponentAnimationMob::PlayTransitionAction
  proto __stdcall bool
  param component void*
  param key char
end

function UOProjectileEffect::LoadAnimationDefinition
  proto __cdecl bool*
  param ok bool*
  param item_id uint
  param out_component void*
end

function UOTileEffect::LoadAnimationDefinition
  proto __cdecl bool*
  param ok bool*
  param item_id uint
  param out_component void*
end

function UOSettings::ParseOptimizationSettings
  proto __cdecl void
  param node void*
end

# ---- AnimationDefinition assets ----

function UOAnimationDefinitionBinaryFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end

function UOAnimationDefinition::Parse
  proto __stdcall /UO/UOAnimationDefinition*
  param factory void*
  param ok bool*
  param blob /UO/AssetBlobSharedPtr
end

function UOAnimationDefinition::ParseFrameSets
  proto __stdcall void
  param factory void*
  param definition /UO/UOAnimationDefinition*
  param position uint*
  param start_set uint
  param second_set uint
  param value float
  param ok bool*
  param blob /UO/AssetBlobSharedPtr
end

function UOAnimationDefinitionBinaryFactory::ReadFrame
  proto __thiscall void custom
  param frame_set /UO/UOAnimationFrameSet* @stack:0x4
  param frame_index uint @stack:0x8
  param blob /UO/AssetBlobSharedPtr @stack:0xc
  param position uint* @stack:0x14
  param ok bool* @stack:0x18
end

function UOAnimations::RequestAnimationDefinition
  proto __stdcall void custom
  param request void* @EAX
  param id uint @stack:0x4
end

function UOAnimations::RequestAnimationSequence
  proto __stdcall void custom
  param request void* @EAX
  param id uint @stack:0x4
end

function UOAnimations::HasSeatedGroup
  proto __cdecl bool custom
  param object void* @ECX
end

# ---- packets ----

function UOPacket::Read_0x6E_CharacterAnimation
  proto __stdcall void custom
  param out_packet /UO/CharacterAnimationPacket* @EAX
  param packet /UO/SharedPtr @stack:0x4
end

function UOPacket::Read_0xE2_NewCharacterAnimation
  proto __stdcall void custom
  param out_packet /UO/NewCharacterAnimationPacket* @ESI
  param packet /UO/SharedPtr @stack:0x4
end

function UOObjectManager::Receive_0x6E_CharacterAnimation
  proto __stdcall void custom
  param manager void* @ECX
  param packet /UO/SharedPtr @stack:0x4
end

function UOObjectManager::Receive_0xE2_NewCharacterAnimation
  proto __stdcall void custom
  param manager void* @ECX
  param packet /UO/SharedPtr @stack:0x4
end

# ---- frame set and definition methods ----

function UOAnimationFrameSet::GetSharedTiming
  proto __cdecl /UO/SharedPtr* custom
  param - undefined4 @ECX
  param out_timing /UO/SharedPtr* @stack:0x4
  param interval float @stack:0x8
end

function UOAnimationFrameSet::SetNext
  proto __stdcall void custom
  param ok bool* @ESI
  param frame_set /UO/UOAnimationFrameSet* @stack:0x4
  param next /UO/UOAnimationFrameSet* @stack:0x8
  param next_refcount void* @stack:0xc
end

function UOAnimationFrameSet::SetFrameInterval
  proto __stdcall void custom
  param frame_set /UO/UOAnimationFrameSet* @ESI
  param interval float @stack:0x4
end

function UOAnimationDefinition::GetFrameSet
  proto __thiscall /UO/SharedPtr*
  param out_frame_set /UO/SharedPtr*
  param key uint
end

function UOAnimationDefinition::AddFrameSet
  proto __thiscall bool*
  param ok bool*
  param frame_set /UO/UOAnimationFrameSet*
  param frame_set_refcount void*
end

function UOAnimationDefinition::SetStartFrameSets
  proto __stdcall bool* custom
  param definition /UO/UOAnimationDefinition* @ESI
  param ok bool* @stack:0x4
  param start_set uint @stack:0x8
  param second_set uint @stack:0xc
  param value float @stack:0x10
end

function UOAnimations::GetAnimationDefinition
  proto __cdecl /UO/SharedPtr*
  param out_definition /UO/SharedPtr*
  param id uint
end

# ---- FrameSetLookup ----

function FrameSetLookup::FrameSetLookup
  proto __stdcall /UO/FrameSetLookup*
  param lookup /UO/FrameSetLookup*
  param parent /UO/FrameSetLookup*
  param default_action uint
end

function FrameSetLookup::GetTransitionAction
  proto __stdcall bool
  param lookup /UO/FrameSetLookup*
  param key char
  param out_action uint*
end

function FrameSetLookup::GetTypedAction
  proto __stdcall uint custom
  param lookup /UO/FrameSetLookup* @EAX
  param type char @stack:0x4
  param sub_type byte @stack:0x8
  param variant byte @stack:0xc
  global ref:+0xb g_UOFrameSetRestriction uint
end

function FrameSetLookup::GetMoveAction
  proto __stdcall uint
  param lookup /UO/FrameSetLookup*
  param move byte
  param alternate bool
end

function FrameSetLookup::IsStandAction
  proto __stdcall bool
  param lookup /UO/FrameSetLookup*
  param action uint
end

# ---- UOAnimationSequence ----

function UOAnimationSequence::UOAnimationSequence
  proto __stdcall /UO/UOAnimationSequence*
  param sequence /UO/UOAnimationSequence*
  param stored_id uint
  param - int*
  param - int*
  param - int*
  param - int*
end

function UOAnimationSequence::HasAction
  proto __stdcall bool
  param sequence /UO/UOAnimationSequence*
  param action uint
end

function UOAnimationSequence::FindAction
  proto __stdcall /UO/AnimationActionRecord*
  param sequence /UO/UOAnimationSequence*
  param action uint
end

function UOAnimationSequence::AddAliasAction
  proto __thiscall void
  param action uint
  param alias int
  param - undefined4
end

function UOAnimationSequence::AddAction
  proto __stdcall void
  param sequence /UO/UOAnimationSequence*
  param action uint
  param frame_count uint
  param offset_x int*
  param offset_y int*
  param extra_offset_x int*
  param extra_offset_y int*
  param - undefined4
  param fps float
end

function UOAnimationSequence::NextBaseFrame
  proto __cdecl uint custom
  param sequence /UO/UOAnimationSequence* @EAX
end

function UOAnimationSequence::IsStandAction
  proto __stdcall bool custom
  param sequence /UO/UOAnimationSequence* @EAX
  param action uint @stack:0x4
end

function UOAnimationSequence::GetDirectionFrames
  proto __stdcall uint custom
  param out_mirrored bool* @EAX
  param sequence /UO/UOAnimationSequence* @stack:0x4
  param action uint @stack:0x8
  param direction uint @stack:0xc
  param out_count uint* @stack:0x10
  global ref:+0x5d g_UOIdleAnimation bool
end

function UOAnimationSequence::GetActionFps
  proto __stdcall void custom
  param sequence /UO/UOAnimationSequence* @EDI
  param action uint @stack:0x4
end

function UOAnimationSequence::ResolveAction
  proto __stdcall uint custom
  param sequence /UO/UOAnimationSequence* @EAX
  param action uint @stack:0x4
  global ref:+0x2c g_UOFlyingAnimations bool
end

function UOAnimationSequence::IsFrameSkipped
  proto __stdcall bool custom
  param sequence /UO/UOAnimationSequence* @EAX
  param action uint @stack:0x4
  param frame uint @stack:0x8
end

function UOAnimationSequence::FrameToAction
  proto __stdcall bool custom
  param sequence /UO/UOAnimationSequence* @EAX
  param frame uint @ECX
  param out_action uint* @stack:0x4
  param out_frame uint* @stack:0x8
end

function UOAnimationSequence::MapCharacterAnimationAction
  proto __cdecl uint custom
  param action uint @EAX
  param sequence /UO/UOAnimationSequence* @ECX
end

function UOAnimationSequence::HasGroup
  proto __stdcall bool
  param sequence /UO/UOAnimationSequence*
  param key char
end

function UOAnimationSequence::GetGroup
  proto __stdcall /UO/FrameSetLookup*
  param sequence /UO/UOAnimationSequence*
  param key char
  global ref:+0x23 g_UOEmptyFrameSetLookup /UO/FrameSetLookup
end

function UOAnimationSequence::GetOrCreateGroup
  proto __stdcall /UO/FrameSetLookup*
  param sequence /UO/UOAnimationSequence*
  param key char
  param default_action uint
  param parent /UO/FrameSetLookup*
end

function UOAnimationSequence::AddFrameEffects
  proto __thiscall void
  param action uint
  param frame uint
  param effects /UO/SharedPtr
end

function UOAnimationSequence::AddSkippedFrame
  proto __thiscall void
  param action uint
  param frame uint
end

function UOAnimationSequence::GetFrameEffects
  proto __thiscall /UO/SharedPtr*
  param out_effects /UO/SharedPtr*
  param action uint
  param frame uint
end

function UOAnimationSequence::GetOffsetX
  proto __thiscall int
  param action uint
  param direction uint
end

function UOAnimationSequence::GetOffsetY
  proto __thiscall int
  param action uint
  param direction uint
end

function UOAnimationSequence::GetExtraOffsetX
  proto __thiscall int
  param action uint
  param direction uint
end

function UOAnimationSequence::GetExtraOffsetY
  proto __thiscall int
  param action uint
  param direction uint
end

function UOAnimations::GetAnimationSequence
  proto __cdecl /UO/SharedPtr*
  param out_sequence /UO/SharedPtr*
  param id uint
end

function UOAnimationSequence::GetFrameCount
  proto __stdcall uint
  param sequence /UO/UOAnimationSequence*
  param action uint
end

function UOAnimationSequence::ActionAt
  proto __stdcall /UO/AnimationActionRecord* custom
  param actions /UOEffects/MsvcMap* @EDI
  param action uint* @stack:0x4
end

function UOAnimationSequenceBinaryFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end

function UOAnimationSequence::Parse
  proto __stdcall /UO/UOAnimationSequence*
  param blob /UO/AssetBlobSharedPtr
  param ok bool*
end

# ---- AnimationFrame assets ----

function UOMobFrameSet::UOMobFrameSet
  proto __stdcall /UO/UOMobFrameSet*
  param frame_set /UO/UOMobFrameSet*
  param blob /UO/AssetBlobSharedPtr
  param ok bool*
end

function UOAnimationFrameSetFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end
