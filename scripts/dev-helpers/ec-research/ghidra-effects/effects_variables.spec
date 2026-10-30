# Variable spec for the functions tagged UO_EFFECTS by NameEffectCode.java; applied by
# ../ApplyVariableSpecs.java after ../ghidra-tileart/tileart_variables.spec (shared /UO types). Layouts are from
# results/uosa_effects.md; the definition structures in /UOEffects come from NameEffectCode.java.
# UOEffects::ParseEffectDefinition is covered by the tileart spec.
#
# Conventions checked at the instruction level:
# - IUOEffect methods take an `ok` byte pointer first: Init (ok, definition), Start/Stop/Release (ok),
#   Update (ok, dt), Get/SetPosition (NiPoint3*). Instantiate is (ok, out effect, object) and returns ok.
# - Each factory Create keeps the factory in ECX and takes (out, blob shared_ptr by value, position, ok); it
#   returns out (RET 0x14). CanHandle, NewInstance and Init do not read ECX.
# - A factory Init receives the new definition as a shared_ptr by value: its object (`definition`) and its
#   reference count block (`definition_refcount`) are two stack parameters.
# - Classes without a structure take their object in ECX as an explicit parameter (custom storage), so that
#   Ghidra does not create an empty class structure.
tag UO_EFFECTS

# ---- types ----

type /UO/MsvcList 0xc : MSVC std::list header
  0x4 void* head : sentinel node; nodes are {next, prev, value}
  0x8 uint size
end

type /UO/UOEffectContainer 0x20 : runtime effect holding the children of a multi-definition entry
  0x0 void* vftable
  0x10 /UO/MsvcList children : list of effect shared_ptrs
  0x1c bool visible : SetVisible
end

type /UO/UOSoundEffect 0x1c : runtime sound effect
  0x0 void* vftable
  0x4 uint sound_id
  0x8 /Gamebryo26/NiPoint3 position
  0x14 float value_08 : copied from the definition, never read
  0x18 void* sound_handle : set by Start, cleared by Stop
end

type /UO/UOEarthquakeEffect 0x28 : runtime earthquake effect
  0x0 void* vftable
  0x4 bool active
  0x8 float elapsed : seconds
  0x10 double amplitude
  0x18 double duration : seconds
  0x20 double frequency : Hz
end

class UOEffectLODDefinition /UOEffects/UOEffectLODDefinition
class UOEffectPropertyFlagDefinition /UOEffects/UOEffectPropertyFlagDefinition
class UOWaitCollectionDefinition /UOEffects/UOWaitCollectionDefinition
class UORandomEffectCollectionDefinition /UOEffects/UORandomEffectCollectionDefinition
class UORoundRobinEffectCollection /UOEffects/UORoundRobinEffectCollection
class UOParticleEffectDefinition /UOEffects/UOParticleEffectDefinition
class UOEarthquakeEffectDefinition /UOEffects/UOEarthquakeEffectDefinition
class UOUserPrefEffectFilterDefinition /UOEffects/UOUserPrefEffectFilterDefinition
class UOSoundEffectDefinition /UOEffects/UOSoundEffectDefinition
class UOPositionOffsetDefinition /UOEffects/UOPositionOffsetDefinition
class BinaryUOEffectDefinitionCollection /UOEffects/BinaryUOEffectDefinitionCollection
class UOEffectContainer /UO/UOEffectContainer
class UOSoundEffect /UO/UOSoundEffect
class UOEarthquakeEffect /UO/UOEarthquakeEffect

# ---- functions (in address order; InstantiateAndRegister before its caller InstantiateAndRegisterByValue) ----

function UOEffectContainerDefinition::Instantiate
  proto __thiscall bool* custom
  param definition void* @ECX
  param ok bool* @stack:0x4
  param out_effect /UO/SharedPtr* @stack:0x8
  param object void* @stack:0xc
end

function UOEffectLODDefinition::GetType
  proto __thiscall uint custom
end

function UOEffectPropertyFlagDefinition::GetType
  proto __thiscall uint custom
end

function UOWaitCollectionDefinition::GetType
  proto __thiscall uint custom
end

function UORandomEffectCollectionDefinition::GetType
  proto __thiscall uint custom
end

function UORandomEffectCollectionDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UORoundRobinEffectCollection::GetType
  proto __thiscall uint custom
end

function UOParticleEffectDefinition::GetType
  proto __thiscall uint custom
end

function UOEffectCollectionDefinition::Instantiate
  proto __thiscall bool* custom
  param definition void* @ECX
  param ok bool* @stack:0x4
  param out_effect /UO/SharedPtr* @stack:0x8
  param object void* @stack:0xc
end

function UOEffectLODDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOEffectLODDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOEffectPropertyFlagDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOEffectPropertyFlagDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOWaitCollectionDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOWaitCollectionDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOUserPrefEffectFilterDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UORandomEffectCollectionDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UORoundRobinEffectCollection::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UORoundRobinEffectCollection::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOParticleEffectDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOParticleEffectDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOEffectContainer::GetKind
  proto __thiscall uint custom
end

function UOParticleEffectNif::ScalarDeletingDestructor
  proto __thiscall void* custom
  param effect void* @ECX
  param flags byte @stack:0x4
end

function UOParticleEffectNif::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition /UOEffects/UOParticleEffectDefinition* @stack:0x8
end

function UOParticleEffectNif::Update
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param dt float @stack:0x8
end

function UOParticleEffectNif::Release
  proto __thiscall void custom
  param effect void* @ECX
  param ok bool* @stack:0x4
end

function UOParticleEffectChaos::ScalarDeletingDestructor
  proto __thiscall void* custom
  param effect void* @ECX
  param flags byte @stack:0x4
end

function UOParticleEffectChaos::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition /UOEffects/UOParticleEffectDefinition* @stack:0x8
end

function UOParticleEffectChaos::Start
  proto __thiscall void custom
  param effect void* @ECX
  param ok bool* @stack:0x4
end

function UOParticleEffectChaos::Stop
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
end

function UOParticleEffectChaos::IsFinished
  proto __thiscall bool custom
  param effect void* @ECX
end

function UOParticleEffectChaos::Update
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param dt float @stack:0x8
end

function UOParticleEffectChaos::Release
  proto __thiscall void custom
  param effect void* @ECX
  param ok bool* @stack:0x4
end

function UOParticleEffectChaos::GetPosition
  proto __thiscall void custom
  param effect void* @ECX
  param out_position /Gamebryo26/NiPoint3* @stack:0x4
end

function UOParticleEffectChaos::SetPosition
  proto __thiscall void custom
  param effect void* @ECX
  param position /Gamebryo26/NiPoint3* @stack:0x4
end

function UOParticleEffectChaos::SetLifetimeTicks
  proto __thiscall void custom
  param effect void* @ECX
  param ticks int @stack:0x4
end

function UOEffectContainer::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOEffectContainer::Init
  proto __thiscall void custom
  param ok bool* @stack:0x4
  param definition void* @stack:0x8
end

function UOEffectContainer::Start
  proto __thiscall void
  param ok bool*
end

function UOEffectContainer::Stop
  proto __thiscall void
  param ok bool*
end

function UOEffectContainer::IsFinished
  proto __thiscall bool
end

function UOEffectContainer::Update
  proto __thiscall void
  param ok bool*
  param dt float
end

function UOEffectContainer::Release
  proto __thiscall void
  param ok bool*
end

function UOEffectContainer::SetPosition
  proto __thiscall void
  param position /Gamebryo26/NiPoint3*
end

function UOEffectContainer::SetVisible
  proto __thiscall void
  param visible bool
end

function UOEarthquakeEffect::Init
  proto __thiscall void
  param ok bool*
  param definition /UOEffects/UOEarthquakeEffectDefinition*
end

function UOEarthquakeEffect::Start
  proto __thiscall void
  param ok bool*
end

function UOEarthquakeEffect::Stop
  proto __thiscall void
  param ok bool*
end

function UOEarthquakeEffect::IsFinished
  proto __thiscall bool
end

function UOEarthquakeEffect::Update
  proto __thiscall void
  param ok bool*
  param dt float
end

function UOHuedParticleEffectNif::ScalarDeletingDestructor
  proto __thiscall void* custom
  param effect void* @ECX
  param flags byte @stack:0x4
end

function UOHuedParticleEffectNif::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition /UOEffects/UOParticleEffectDefinition* @stack:0x8
end

function UOEffects::MakeProjectileEffect
  proto __cdecl void
  param record void*
end

function UOEffects::MakeTileEffect
  proto __cdecl void
  param record void*
  param - undefined4
end

function UOEffects::PlayEffectAtTile
  proto __cdecl void custom
  param effect_id uint @EAX
  param tile_position /Gamebryo26/NiPoint3* @EDI
  param out_effect /UO/SharedPtr* @ESI
  param ticks int @stack:0x4
  param ok bool* @stack:0x8
  global ref:+0x63 g_UOEffectCameraOffsetScale float
end

function UOEffects::PlayGraphicalEffect
  proto __cdecl bool*
  param ok bool*
  param record void*
end

function UOEffects::UpdateActiveEffects
  proto __cdecl void
  param dt float
  global ref:+0x6a g_UOActiveEffects /UO/MsvcList
end

function UOEffects::ReleaseActiveEffects
  proto __cdecl void
  global ref:+0x7b g_UOParticleNifCache
end

function UOEffects::InstantiateAndRegister
  proto __cdecl bool* custom
  param definition void* @ECX
  param object void* @EDX
  param ok bool* @ESI
  param out_effect /UO/SharedPtr* @stack:0x4
end

function UOEffects::InstantiateAndRegisterByValue
  proto __cdecl bool* custom
  param out_effect /UO/SharedPtr* @EAX
  param object void* @EDX
  param ok bool* @stack:0x4
  param definition /UO/SharedPtr @stack:0x8
end

function UOEffects::PlayGraphicalEffect_0x70
  proto __cdecl bool* custom
  param packet void* @ESI
  param ok bool* @EDI
end

function UOEffects::PlayGraphicalEffect_0xC0
  proto __cdecl bool* custom
  param packet void* @ESI
  param ok bool* @EDI
end

function UOEffects::PlayGraphicalEffect_0xC7
  proto __cdecl bool* custom
  param packet void* @ESI
  param ok bool* @EDI
end

function UOEffects::InstantiateEffectById
  proto __cdecl bool*
  param ok bool*
  param effect_id uint
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOObjectComponentEffects::HandleNotify
  proto __thiscall undefined4 custom
  param component void* @ECX
  param message uint @stack:0x4
end

function UOPositionOffset::ScalarDeletingDestructor
  proto __thiscall void* custom
  param effect void* @ECX
  param flags byte @stack:0x4
end

function UOPositionOffset::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition /UOEffects/UOPositionOffsetDefinition* @stack:0x8
end

function UOPositionOffset::GetPosition
  proto __thiscall void custom
  param effect void* @ECX
  param out_position /Gamebryo26/NiPoint3* @stack:0x4
end

function UOPositionOffset::SetPosition
  proto __thiscall void custom
  param effect void* @ECX
  param position /Gamebryo26/NiPoint3* @stack:0x4
end

function UOProjectileEffect::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition void* @stack:0x8
end

function UOProjectileEffect::SpawnExplosion
  proto __stdcall undefined4
  param projectile void*
  param ok bool*
end

function UOProjectileEffect::Update
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param dt float @stack:0x8
end

function UOTileEffect::Init
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param definition void* @stack:0x8
end

function UOTileEffect::IsFinished
  proto __thiscall bool custom
  param effect void* @ECX
end

function UOTileEffect::Update
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param dt float @stack:0x8
end

function UOEffects::IsParticleFiltered
  proto __cdecl bool
  param serial uint
  param serial_high uint
  param location_effect bool
end

function UOEarthquakeEffectDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOEarthquakeEffectDefinition::GetType
  proto __thiscall uint custom
end

function UOEarthquakeEffectDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOWaitEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOLODEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOPropertyFlagEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOUserPreferenceEffectFilterBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UORandomEffectCollectionBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOEarthquakeEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOUserPrefEffectFilterDefinition::GetType
  proto __thiscall uint custom
end

function UOUserPrefEffectFilterDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOWaitEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOWaitEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOWaitCollectionDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param children void* @stack:0xc
  param min_delay_ms uint @stack:0x10
  param max_delay_ms uint @stack:0x14
  param aligned bool @stack:0x18
end

function UOWaitEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOLODEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOLODEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOEffectLODDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param entry void* @stack:0xc
end

function UOLODEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOPropertyFlagEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOPropertyFlagEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOEffectPropertyFlagDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param mode bool @stack:0xc
  param mask qword @stack:0x10
  param property uint @stack:0x18
  param default_children void* @stack:0x1c
  param cases void* @stack:0x20
end

function UOPropertyFlagEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOUserPreferenceEffectFilterBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOUserPreferenceEffectFilterBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOUserPrefEffectFilterDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param mask uint @stack:0xc
  param children void* @stack:0x10
end

function UOUserPreferenceEffectFilterBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UORandomEffectCollectionBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UORandomEffectCollectionBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UORandomEffectCollectionDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param entry /UOEffects/UORandomEffectEntry* @stack:0xc
end

function UORandomEffectCollectionBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOEarthquakeEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOEarthquakeEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOEarthquakeEffectDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param amplitude double @stack:0xc
  param duration double @stack:0x14
  param frequency double @stack:0x1c
end

function UOEarthquakeEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOSoundEffect::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOSoundEffect::Init
  proto __thiscall void
  param ok bool*
  param definition /UOEffects/UOSoundEffectDefinition*
end

function UOSoundEffect::Start
  proto __thiscall void
  param ok bool*
  global ref:+0x0 g_UOSoundEnabled bool
end

function UOSoundEffect::Stop
  proto __thiscall void
  param ok bool*
end

function UOSoundEffect::IsFinished
  proto __thiscall bool
end

function UOSoundEffect::GetPosition
  proto __thiscall void
  param out_position /Gamebryo26/NiPoint3*
end

function UOSoundEffect::SetPosition
  proto __thiscall void
  param position /Gamebryo26/NiPoint3*
end

function BinaryUOEffectDefinitionCollection::GetCount
  proto __thiscall uint
end

function BinaryUOEffectDefinitionCollection::GetDefinition
  proto __thiscall void
  param out_definition /UO/SharedPtr*
  param index uint
end

function BinaryUOEffectDefinitionCollection::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOParticleEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UORoundRobinEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOSoundEffectBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOPositionOffsetBinaryBaseFactory::CanHandle
  proto __thiscall bool custom
  param key uint @stack:0x4
end

function UOSoundEffectDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOSoundEffectDefinition::GetType
  proto __thiscall uint custom
end

function UOSoundEffectDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOPositionOffsetDefinition::Instantiate
  proto __thiscall bool*
  param ok bool*
  param out_effect /UO/SharedPtr*
  param object void*
end

function UOPositionOffsetDefinition::GetType
  proto __thiscall uint custom
end

function UOPositionOffsetDefinition::ScalarDeletingDestructor
  proto __thiscall void*
  param flags byte
end

function UOParticleEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
  param name undefined4 @stack:0x8
end

function UORoundRobinEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UORoundRobinEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UORoundRobinEffectCollection* @stack:0x4
  param definition_refcount void* @stack:0x8
  param child void* @stack:0xc
  param child_refcount void* @stack:0x10
end

function UOSoundEffectBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOSoundEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOSoundEffectDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param sound_id uint @stack:0xc
  param value_08 float @stack:0x10
end

function UOPositionOffsetBinaryFactory::NewInstance
  proto __thiscall undefined4* custom
  param out_definition /UO/SharedPtr* @stack:0x4
end

function UOPositionOffsetBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOPositionOffsetDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param direction uint @stack:0xc
  param x int @stack:0x10
  param y int @stack:0x14
end

function UOSoundEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOParticleEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOParticleEffectBinaryFactory::Init
  proto __thiscall void custom
  param definition /UOEffects/UOParticleEffectDefinition* @stack:0x4
  param definition_refcount void* @stack:0x8
  param offset_x float @stack:0xc
  param offset_y float @stack:0x10
  param camera_offset float @stack:0x14
  param scale float @stack:0x18
  param attach_mode byte @stack:0x1c
  param duration float @stack:0x20
  param loop bool @stack:0x24
  param field_3c byte @stack:0x28
  param hued bool @stack:0x2c
end

function UORoundRobinEffectBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOPositionOffsetBinaryFactory::ScalarDeletingDestructor
  proto __thiscall void* custom
  param factory void* @ECX
  param flags byte @stack:0x4
end

function UOPacket::Read_0x70_GraphicalEffect
  proto __stdcall void custom
  param out_packet void* @EAX
  param packet /UO/SharedPtr @stack:0x4
end

function UOPacket::Read_0xC0_HuedEffect
  proto __stdcall void custom
  param out_packet void* @ESI
  param packet /UO/SharedPtr @stack:0x4
end

function UOPacket::Read_0xC7_ParticleEffect
  proto __stdcall void custom
  param out_packet void* @EAX
  param packet /UO/SharedPtr @stack:0x4
end

function UOObjectManager::Receive_0x70_GraphicalEffect
  proto __stdcall void
  param packet /UO/SharedPtr
end

function UOObjectManager::Receive_0xC0_HuedEffect
  proto __stdcall void
  param packet /UO/SharedPtr
end

function UOObjectManager::Receive_0xC7_ParticleEffect
  proto __stdcall void
  param packet /UO/SharedPtr
end

function UOEffects::ShowTargetIndicator
  proto __cdecl undefined4
  param serial uint
  param serial_high uint
  global ref:+0x0 g_UOTargetIndicatorEffect /UO/SharedPtr
end

function UOWaypoints::SetWaypointEffect
  proto __stdcall void custom
  param waypoint void* @EAX
  param show bool @stack:0x4
end

function UOSoundEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOParticleEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UORoundRobinEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOPositionOffsetBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOLODEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOWaitEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOPropertyFlagEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOUserPreferenceEffectFilterBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UORandomEffectCollectionBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function UOEarthquakeEffectBinaryBaseFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param factory void* @ECX
  param out_definition /UO/SharedPtr* @stack:0x4
  param blob /UO/AssetBlobSharedPtr @stack:0x8
  param position uint* @stack:0x10
  param ok bool* @stack:0x14
end

function BinaryUOEffectDefinitionCollection::AddDefinition
  proto __thiscall void
  param definition /UO/SharedPtr
end

function UOEffectDefinitionCollectionBinaryFactory::Create
  proto __thiscall /UO/SharedPtr* custom
  param out_asset /UO/SharedPtr* @stack:0x4
  param asset_id uint @stack:0x8
  param asset_type uint @stack:0xc
  param blob /UO/AssetBlobSharedPtr @stack:0x10
end

function BinaryUOEffectDefinitionCollection::GetMemorySize
  proto __thiscall uint
end

function UOEffects::ParseEffectDefinitionCollection
  proto __cdecl /UO/SharedPtr*
  param out_collection /UO/SharedPtr*
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

function UOEffects::ParseEffectList
  proto __stdcall void
  param factories void*
  param out_list /UOEffects/MsvcVector*
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

function UOEffects::ParseEffectMap
  proto __stdcall void
  param factories void*
  param out_map /UOEffects/MsvcMap*
  param blob /UO/AssetBlobSharedPtr
  param position uint*
  param ok bool*
end

