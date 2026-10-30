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

# ---- Chaos particle engine (results/uosa_chaos.md) ----

type /UO/ChaosRange 0x8 : value range; sampled as min + (max - min) * r when min < max
  0x0 float min
  0x4 float max
end

type /UO/ChaosIntRange 0x8 : integer range (BURST); parsed with atol & 0xffff
  0x0 uint min
  0x4 uint max
end

type /UO/ChaosVectorRange 0x18 : XYZ range, each component sampled separately
  0x0 /Gamebryo26/NiPoint3 min
  0xc /Gamebryo26/NiPoint3 max
end

type /UO/ChaosToken 0x20 : script token
  0x0 /UOEffects/MsvcString text : uppercased by ChaosScript::ClassifyToken
  0x1c uint type : token type (1 number, 2 '=', ... 0x2a other)
end

type /UO/ChaosCelSetup 0x18 : CelSetup(frames, u step, v step, columns, rows, fps, loop)
  0x0 ushort frames
  0x4 float u_step
  0x8 float v_step
  0xc byte columns
  0xd byte rows
  0x10 float fps
  0x14 bool loop : last argument > 0
end

type /UO/ChaosParticleEvent 0x24 : event of an EventSequence (colour events 0x3c, vector events 0x34)
  0x0 void* vftable : +4 Apply(particle), +8 SupportsFade, +0xc Parse
  0x4 /UO/ChaosParticleEvent* next_fade : first later fade event of the same type
  0x8 /UO/ChaosRange time_range : as written (INITIAL 0, FINAL = lifetime max, AT value)
  0x10 float time : sampled once when the sequence is compiled
  0x14 bool is_fade : FADE SO
  0x18 uint type_id
  0x1c /UO/ChaosRange value : colour events: RGBA min at +0x1c, max at +0x2c; vector events: min/max XYZ
end

type /UO/ChaosParticle 0xa8 : pooled particle
  0x0 void* vftable
  0x4 /UO/ChaosParticleEvent** next_event : iterator into the sequence's sorted events
  0x8 float size : 1.0 by default
  0xc float size_step : per second
  0x10 float lifetime : seconds, sampled at creation
  0x14 float age
  0x18 float event_timer : advanced by dt; EVENTTIMER sets it
  0x1c float birth_time
  0x20 float last_update_time
  0x24 float last_dt
  0x28 float[4] color : RGBA, clamped to [0,1] only when rendered
  0x38 float[4] color_step
  0x48 /Gamebryo26/NiPoint3 velocity
  0x54 /Gamebryo26/NiPoint3 velocity_step
  0x60 float thrust
  0x64 float thrust_step
  0x68 /Gamebryo26/NiPoint3 position : world, or emitter-relative when Local
  0x74 /Gamebryo26/NiPoint3 previous_position
  0x80 /Gamebryo26/NiPoint3 rotation : radians; billboards use y only
  0x8c /Gamebryo26/NiPoint3 rotation_step
  0x98 float length : AXIAL quad length, 1.0 by default
  0x9c float length_step
  0xa4 uint pool_id
end

type /UO/ChaosEventSequence 0x11c : compiled EventSequence block
  0x0 void* vftable
  0x4 uint refcount
  0x8 uint flags : bit1 CENTERED, 2 SORT, 3 ROTATION used, 4 STAYATEMITTER, 5 THRUST used, 6/7 multipliers set, 8 LOCAL, 9 CELANIMATE, 10 cel loop, 11 EMISSIONBOOST, 12 COLLISIONCHECK, 13 STOP, 14 REFLECT
  0xc /UOEffects/MsvcString name
  0x28 /UOEffects/MsvcString resource : TEXTURE, MESHNAME or EMITTERNAME
  0x44 /UOEffects/MsvcVector events : /UO/ChaosParticleEvent*, sorted by time
  0x54 /UO/ChaosCelSetup cel
  0x6c uint dest_blend : ALPHA_* index
  0x70 uint src_blend : ALPHA_* index
  0x74 uint particle_type : POINT 1, MESH 2, BILLBOARD 3, EMITTER 4, AXIAL 5, LIGHT 6
  0x78 /UO/ChaosRange lifetime
  0x80 /UO/ChaosRange emit_rate : particles per second; the emitter uses max only
  0x88 /UO/ChaosVectorRange gravity : sampled every update, multiplied by (0,0,1)
  0xa0 /UO/ChaosVectorRange inherited : scales the emitter velocity per component
  0xb8 /UO/ChaosVectorRange chaos_velocity
  0xd0 /UO/ChaosVectorRange chaos_location
  0xe8 ushort num_particles : live particle cap, 100 by default
  0xec int emitted : particles created
  0xf4 int loops : -1 by default, never read at run time
  0xf8 float scale_multiplier : applied to SCALE events at parse time
  0xfc float velocity_multiplier : applied to VELOCITY events at parse time
  0x104 /UO/ChaosIntRange burst : particles per emission step, 1..1 by default
  0x10c /UO/ChaosRange emission_boost : extra emit rate per unit of emitter movement
  0x114 /UO/ChaosRange collision_absorption
end

type /UO/ChaosEmitter 0x7c : compiled .ems particle system (one per EmitterGroup clone)
  0x0 void* vftable
  0x4 void* renderer : ParticleSet
  0x8 void* owner : EmitterEffect node, null when stopped
  0x14 /UO/ChaosEventSequence** sequences_begin
  0x18 /UO/ChaosEventSequence** sequences_end
  0x24 void* particle_deques : std::deque<Particle*> per sequence, stride 0x14
  0x34 float* last_emit_times : per sequence
  0x44 float start_time
  0x48 float last_update_time
  0x4c /Gamebryo26/NiPoint3 previous_position : owner world translate of the previous update
  0x58 /Gamebryo26/NiPoint3 node_velocity
  0x64 float emission_remaining : seconds of emission left
  0x68 bool infinite : duration <= 0
  0x69 bool first_burst
  0x6a bool one_shot
  0x6c /UO/ChaosRange min_emit_time : MINEMITTIME
  0x74 bool updated
  0x75 bool skip_first_update
  0x76 bool location_only : LOCATIONONLY, never read at run time
  0x77 bool started
  0x78 void* manager
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

function UOHuedParticleEffectNif::UOHuedParticleEffectNif
  proto __cdecl void custom
  param effect void* @ESI
  param object void* @EDI
end

function UOHuedParticleEffectNif::Update
  proto __thiscall undefined4 custom
  param effect void* @ECX
  param ok bool* @stack:0x4
  param dt float @stack:0x8
end

function UOHuedParticleEffectNif::RenderToTexture
  proto __stdcall void
  param effect void*
end

function IUOEffectDefinition::PlayOnObject
  proto __thiscall bool* custom
  param definition void* @ECX
  param ok bool* @stack:0x4
  param object void* @stack:0x8
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

# ---- Chaos particle engine ----
# Emitter methods are virtual __thiscall. The sequence, particle and parser helpers pass objects in registers
# (EAX, ECX, EDX, ESI, EDI) or on the stack without ECX; their prototypes are custom.
# CreateParticle also takes the particle pool in EDX (MOV ESI,EDX after the cap check, read by the allocator at
# [ESI+0xbc]); the read sits behind a conditional branch, which the read-before-write check does not follow, so the
# register is left out here.

function Emitter::Start
  proto __thiscall void custom
  param emitter /UO/ChaosEmitter* @ECX
  param start_time float @stack:0x4
  param duration float @stack:0x8  # <= 0 means infinite
  param not_looping bool @stack:0xc
end

function Emitter::ClearStarted
  proto __thiscall void custom
  param emitter /UO/ChaosEmitter* @ECX
end

function Emitter::Stop
  proto __thiscall void custom
  param emitter /UO/ChaosEmitter* @ECX
  param pool void* @stack:0x4
  param kill_stay_at_emitter bool @stack:0x8
  param kill_all bool @stack:0xc
end

function Emitter::Update
  proto __thiscall void custom
  param emitter /UO/ChaosEmitter* @ECX
  param time float @stack:0x4  # manager time in seconds (millisecond steps)
  param dt float @stack:0x8
end

function Emitter::IsActive
  proto __thiscall bool custom
  param emitter /UO/ChaosEmitter* @ECX
end

function Emitter::CompileScript
  proto __thiscall undefined4 custom
  param emitter /UO/ChaosEmitter* @ECX
  param - void* @stack:0x4
  param - void* @stack:0x8
  param status /UOEffects/MsvcString* @stack:0xc
end

function Emitter::CountParticles
  proto __cdecl int custom
  param emitter /UO/ChaosEmitter* @EDI
end

function ParticleEventSequence::ParticleEventSequence
  proto __stdcall void* custom
  param sequence /UO/ChaosEventSequence* @stack:0x4
end

function ParticleEventSequence::ResolveEventTimes
  proto __cdecl void custom
  param sequence /UO/ChaosEventSequence* @EDI
end

function ParticleEventSequence::CreateParticle
  proto __stdcall bool custom
  param sequence /UO/ChaosEventSequence* @stack:0x4
  param particles void* @stack:0x8
  param node_position /Gamebryo26/NiPoint3* @stack:0xc
  param node_velocity /Gamebryo26/NiPoint3* @stack:0x10
  param node_rotation float* @stack:0x14
  param time float @stack:0x18
  param node_scale float @stack:0x1c
end

function ParticleEventSequence::Update
  proto __stdcall void custom
  param sequence /UO/ChaosEventSequence* @stack:0x4
  param pool void* @stack:0x8
  param particles void* @stack:0xc
  param node_position /Gamebryo26/NiPoint3* @stack:0x10
  param time float @stack:0x14
end

function ParticleEventSequence::RunEvents
  proto __stdcall void custom
  param sequence /UO/ChaosEventSequence* @stack:0x4
  param particle /UO/ChaosParticle* @EDI
end

function Particle::Update
  proto __stdcall bool custom
  param time float @stack:0x4
  param gravity /Gamebryo26/NiPoint3* @stack:0x8
  param sequence_flags uint* @stack:0xc
  param node_position /Gamebryo26/NiPoint3* @ECX  # read only by StayAtEmitter
  param particle /UO/ChaosParticle* @EDI
end

function ChaosScript::SampleIntRange
  proto __cdecl int custom
  param min int @EAX
  param max int @ECX
end

function ChaosScript::SampleFloatRange
  proto __cdecl float custom
  param range /UO/ChaosRange* @EAX
end

function ChaosScript::ClassifyToken
  proto __cdecl void custom
  param token /UO/ChaosToken* @ESI
end

function ChaosScript::ParseRange
  proto __cdecl bool custom
  param tokens /UO/ChaosToken** @ESI
  param token_end /UO/ChaosToken** @EDI
  param out_range /UO/ChaosRange* @stack:0x4
end

function ChaosScript::ParseIntRange
  proto __cdecl bool custom
  param tokens /UO/ChaosToken** @ESI
  param token_end /UO/ChaosToken** @EDI
  param out_range /UO/ChaosIntRange* @stack:0x4
end

function ChaosScript::ParseCelSetup
  proto __cdecl bool custom
  param tokens /UO/ChaosToken** @ECX
  param token_end /UO/ChaosToken** @EAX
  param out_cel /UO/ChaosCelSetup* @stack:0x4
end

function MersenneTwister::NextU32
  proto __cdecl uint
end

function ParticleSystemManager::Update
  proto __stdcall void custom
  param manager void* @stack:0x4
  param dt_ms uint @stack:0x8
end

function EmitterEffect::Start
  proto __thiscall void custom
  param effect void* @ECX
  param duration float @stack:0x4
  param not_looping bool @stack:0x8
end

function EmitterEffect::Stop
  proto __thiscall void custom
  param effect void* @ECX
end

function EmitterEffect::GetVelocity
  proto __thiscall /Gamebryo26/NiPoint3* custom
  param effect void* @ECX
  param dt float @stack:0x4
end

function ParticleTriShape::BuildVertices
  proto __thiscall void custom
  param shape void* @ECX
end

function ParticleTriShape::SetCelTexCoords
  proto __stdcall void custom
  param shape void* @stack:0x4
  param index ushort @stack:0x8
  param frame uint @ECX
  param cel /UO/ChaosCelSetup* @EDI
end

function ParticleEventScale::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventColor::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventRedColor::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventGreenColor::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventBlueColor::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventAlpha::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventVelocity::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventVelocityX::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventVelocityY::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventVelocityZ::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventThrust::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventEventTimer::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventRotation::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end

function ParticleEventLength::Apply
  proto __thiscall void custom
  param event /UO/ChaosParticleEvent* @ECX
  param particle /UO/ChaosParticle* @stack:0x4
end
