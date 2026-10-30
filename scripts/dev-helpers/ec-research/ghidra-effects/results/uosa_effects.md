# UOSA.exe effect definitions

The Enhanced Client (`UOSA.exe`, Ghidra project `UOSA`) stores effect definitions in two places:

- at the end of every `tileart.uop` record: a u8 count, then that many definitions;
- in each entry of `EffectDefinitionCollection.uop`: a u32 count, then that many definitions.

One recursive reader decodes both: `UOEffects::ParseEffectDefinition`, through a table of type-keyed
factories. This document gives the format the client parses, what the fields mean, the verification
against the shipped data, and a parser specification for the repo. The sections from "Load path" on
describe how the client uses `EffectDefinitionCollection.uop` at run time: how an entry is loaded, which
packets select it, how the composite types behave, and where particle names are resolved.

Conventions:

- All values are little-endian and unaligned. The stream is read field by field through
  `UOAssets::StreamRead` 0x00a6fe55 (ECX = stream, ESI = position pointer, EDI = byte count, stack =
  destination and parse context).
- Confidence levels: **confirmed** means read in the assembly and checked against the data. **probable**
  means inferred from how the field is used. **unknown** means no reader was found.

## Dispatch

| Item | Address | What it does |
|---|---|---|
| `UOEffects::ParseEffectDefinition` | 0x008415ae | (factories, out definition, stream, position, context). Reads a u32 type key, then calls each factory's CanHandle (vtable +0) in vector order. The first factory that accepts the key runs Create (vtable +8). If none accepts it, the parse context's ok byte is cleared and a null definition is returned without reading anything more. |
| `g_UOEffectDefinitionFactories` | 0x00e3737c | pointer to a std::vector of factory pointers. It is pushed before every ParseEffectDefinition call. |
| registration | `FUN_004af590` (a startup state) | creates the ten factories as shared_ptrs and appends them in this order: Particle, Sound, RoundRobin, PositionOffset, Wait, LOD, PropertyFlag, UserPreference, Random, Earthquake |
| `UOEffects::ParseEffectList` | 0x0084167f | u32 count, then that many ParseEffectDefinition calls, appended to a vector of shared_ptrs |
| `UOEffects::ParseEffectMap` | 0x00841718 | u32 count, then that many {u32 key, ParseEffectList} pairs, inserted into a `std::map<u32, vector>` |
| `UOEffects::ParseEffectDefinitionCollection` | 0x0084126a | u32 count, then that many ParseEffectDefinition calls, into a `BinaryUOEffectDefinitionCollection` (0x14 bytes) |
| `UOTileArt::ParseEffects` | 0x00844f9f | tileart record: u8 count, then that many ParseEffectDefinition calls |

The class `UOGenericEffectFactory` exists, but it is not registered.

Each Create (a thiscall with stack arguments (out, stream, position, context); `RET 0x14`) follows the same
steps:

1. Read the fields.
2. Call `NewInstance` (vtable +0x14) to allocate the definition.
3. Call `Init` (vtable +0x18) with the decoded values.

None of the Create functions check a version.

## Factory / key table

The CanHandle key and the definition's GetType (vtable +0xc) return value are equal for all ten factories.

| Key | Factory (RTTI) | Factory vftable | CanHandle | Create | Definition (RTTI) | Def. vftable | Def. size |
|---:|---|---|---|---|---|---|---:|
| 0 | `UOParticleEffectBinaryFactory` | 0x00ca59f0 | 0x004c3a80 | 0x0083b657 | `UOParticleEffectDefinition` | 0x00c9d3a8 | 0x44 |
| 1 | `UOSoundEffectBinaryFactory` | 0x00ca5a30 | 0x004c3aa0 | 0x0083b590 | `UOSoundEffectDefinition` | 0x00ca59c8 | 0x0c |
| 2 | `UORoundRobinEffectBinaryFactory` | 0x00ca5a10 | 0x004c3a90 | 0x0083b7f0 | `UORoundRobinEffectCollection` | 0x00c9d394 | 0x18 |
| 7 | `UOPositionOffsetBinaryFactory` | 0x00ca5a50 | 0x004c3ab0 | 0x0083b8f9 | `UOPositionOffsetDefinition` | 0x00ca59dc | 0x44 |
| 10 | `UOWaitEffectBinaryFactory` | 0x00ca52d0 | 0x004ae3f0 | 0x0083ba55 | `UOWaitCollectionDefinition` | 0x00c9d36c | 0x24 |
| 11 | `UOLODEffectBinaryFactory` | 0x00ca5290 | 0x004ae4d0 | 0x0083b9a6 | `UOEffectLODDefinition` | 0x00c9d344 | 0x10 |
| 12 | `UOPropertyFlagEffectBinaryFactory` | 0x00ca52b0 | 0x004ae5a0 | 0x0083bb65 | `UOEffectPropertyFlagDefinition` | 0x00c9d358 | 0x30 |
| 15 | `UOEarthquakeEffectBinaryFactory` | 0x00ca5174 | 0x004ae810 | 0x0083bee9 | `UOEarthquakeEffectDefinition` | 0x00ca5160 | 0x20 |
| 16 | `UORandomEffectCollectionBinaryFactory` | 0x00ca5324 | 0x004ae740 | 0x0083bd90 | `UORandomEffectCollectionDefinition` | 0x00c9d380 | 0x14 |
| 17 | `UOUserPreferenceEffectFilterBinaryFactory` | 0x00ca5304 | 0x004ae670 | 0x0083bcc1 | `UOUserPrefEffectFilterDefinition` | 0x00ca52f0 | 0x18 |

Keys 3 to 6, 8, 9, 13 and 14 have no factory. A definition with one of those keys cannot be parsed.

## Formats per type

Every definition is `u32 type_key` followed by the body below. "List" means an effect list: a u32 count,
then that many definitions. "Map" means a u32 count, then that many {u32 key, list} pairs. The "Stored at"
column gives the definition object offset.

### 0 Particle (body 28 bytes, Create 0x0083b657)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u32 | name | +0x04 (std::string) | string dictionary index. It is resolved through `[0x00e37220]+0x154`, an array of 0x1c-byte std::string. The name is a `.nif` or `.ems` file. | confirmed |
| 2 | f32 | offset_x | +0x20 | tiles east: added to the world x of the effect offset (Nif Init 0x0041aa6a..0x0041aaf9, Chaos Init 0x0041b119..0x0041b1ba), as `UOEffects::PlayEffectAtTile` 0x00421130 converts a tile x | confirmed |
| 3 | f32 | offset_y | +0x24 | tiles south: subtracted from the world z of the effect offset, as `PlayEffectAtTile` converts a tile y (world z = -tile y). On the client's camera one tile east is (0.706, 0.707) and one tile south (-0.708, 0.706) in (screen right, screen up) units, so neither is a pure screen axis | confirmed |
| 4 | f32 | camera_offset | +0x28 | k = -0.0 (0x00d04fb0) - camera_offset * 0.12856 (0x00d050a0) times the camera down vector `g_UOWorldViewer` 0x00e37224 +0x24..+0x2c (= -up, written by `UOWorldViewer::CreateCamera` 0x00471580 at 0x004718ac..0x004718de as -(R·(0,1,0)), with +0x0c = R·(1,0,0) the direction). Nif Init 0x0041aa6a..0x0041aaf9 and Chaos Init 0x0041b119..0x0041b1ba store (offset_x + k·v.x, k·v.y, -offset_y + k·v.z) as the effect offset that `SetPosition` (Chaos 0x0041b500) adds to the position. Net: a positive value moves the effect camera_offset * 0.12856 units up on screen. | confirmed |
| 5 | f32 | scale | +0x2c | scale of the scene node (`FUN_00419ca0`, node +0x5c) | confirmed |
| 6 | u8 | attach_mode (was field_30) | +0x30 | stored as u32. Only reader: `UOObject::CreateComponents` 0x005befd0, which does not attach a particle with value 2 to the world object (`0x005bf69d CMP dword [EAX+0x30],2; JNZ`); such an effect plays only on the moving-effect (projectile) paths. 0 and 1 are not told apart. Data: 2 on thrown models, 0 on arrows, 1 elsewhere. See "Particle attach mode". | confirmed (value 2) |
| 7 | f32 | duration | +0x34 | seconds. If the effect doesn't loop, it stops once elapsed >= duration. If it loops and duration != 0, elapsed = fmod(elapsed, duration). | confirmed |
| 8 | u8 | loop | +0x38 | normalized with SETNZ, so any non-zero value means true | confirmed |
| 9 | u8 | field_3c | +0x3c | the Nif Init fails when the value is 1 (0x0041a8fb `CMP [EBX+0x3c],1; JNZ`), and the Chaos Init requires 0 (0x0041aff8, error with the effect name otherwise). No other reader. Always 0 in the data, so a valid definition holds 0. Meaning of non-zero values unknown. | confirmed (checks) |
| 10 | u8 | hued | +0x40 | non-zero together with a non-zero object hue gives `UOHuedParticleEffectNif` | confirmed |

Particle bytes aren't validated when they are parsed.

`UOParticleEffectDefinition::Instantiate` 0x0040e490 chooses the runtime class:

- a name containing `.ems` gives `UOParticleEffectChaos`;
- otherwise, object hue != 0 and hued give `UOHuedParticleEffectNif`;
- otherwise, `UOParticleEffectNif`.

The positional math is in `UOParticleEffectNif::Init` 0x0041a8e0: (offset_x, offset_y, camera_offset) is a
tile-space offset (dx, dy, dz) turned into world space the way `PlayEffectAtTile` turns a tile position
(world x = tile x, world z = -tile y; camera_offset along the camera's up). The effect node's own rotation
does not apply to it, so a viewer drawing in the effect's local frame converts the two tile axes with the
world camera rotation (`client_world_camera_rotation` in `lib/ec-effects/particle-sim/src/effect.rs`).

#### Particle attach mode (definition +0x30)

`UOObject::CreateComponents` 0x005befd0 walks the tileart effects of the object's definition: `GetEffect`
(definition vtable +0x90, 0x005bf66e), then the effect definition's `GetType` (vtable +0xc, 0x005bf68c).
For type 0 (particle) it tests `0x005bf69d CMP dword ptr [EAX+0x30],0x2` / `JNZ`: with value 2 the effect
is skipped (jump to 0x005bf7e3) and is not added to the object's effects component; other values are
attached. The moving-effect code that shows an item in flight (`FUN_0044bb40` at 0x0044c5e2/0x0044c606,
`FUN_00466940` at 0x00466d0a) instantiates the same definitions without testing +0x30. A whole-program scan
of `+0x30]` operands after `GetEffect`/`GetType` found no other reader, and `Instantiate` only zeroes the
instance's own +0x30.

Data: value 2 on thrown models (boomerang, glaives, `rock02` boulders, `eaglestrike.ems`: tiles 2303, 2305,
2314, 4962-4973, 16487, 16491, 16492, 16506), 0 on arrow projectiles (3906, 7166, 20346), 1 on the other
550; always 1 in EffectDefinitionCollection.uop. So 2 means "projectile only": the trail is shown while the
item flies but not when it lies on the ground. The meaning of 0 versus 1 is not visible in the client.

### 1 Sound (body 8 bytes, Create 0x0083b590)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u32 | sound_id | +0x04 | `UOSoundEffect::Start` plays this id at the effect position, with distance attenuation, when sound is enabled | confirmed |
| 2 | f32 | value_08 | +0x08 | copied to `UOSoundEffect`+0x14 (`UOSoundEffect::Init` 0x004b88d0, `FSTP [ECX+0x14]` at 0x004b88e8; constructor default 1.0). No `UOSoundEffect` method (vftable 0x00ca5594) reads +0x14; `Start` 0x004b8900 -> `FUN_004b5770` computes its own distance volume. 1.0 in the data. Probably an intended gain. | confirmed (not read) |

### 2 RoundRobin (Create 0x0083b7f0)

| # | Type | Name | Meaning | Confidence |
|---:|---|---|---|---|
| 1 | u32 | count | number of children | confirmed |
| 2 | count x definition | children | each child is appended through Init (vtable +0x18). Instantiate takes the child at index +0x14 and advances that index modulo the count (see "Runtime semantics"). | confirmed |

The Create calls ParseEffectDefinition directly, but the bytes are the same as an effect list.

### 7 PositionOffset (body 16 bytes, Create 0x0083b8f9)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1..8 | 8 x {i8 x, i8 y} | offsets[direction] | x at +0x04+4d, y at +0x24+4d (sign-extended to i32) | per-direction (0..7) position offset copied into `UOPositionOffset` | confirmed |

### 10 Wait (Create 0x0083ba55)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | list | children | +0x04 | effects played when the throttle allows | confirmed |
| 2 | u32 | min_delay_ms | +0x14 | default 2000 | confirmed |
| 3 | u32 | max_delay_ms | +0x18 | default 4000 | confirmed |
| 4 | u8 | aligned | +0x1c | stored as `value == 1`. Any value other than 0 or 1 clears the context's ok byte. When set, the next fire time is aligned to a multiple of min_delay_ms. | confirmed |

`Instantiate` 0x0040d9b0 uses QueryPerformanceCounter milliseconds. It plays the children only when
now >= next (+0x20), then schedules the next time; it is a throttle, not a repeating timer (see "Runtime
semantics").

### 11 LOD (Create 0x0083b9a6)

| # | Type | Name | Meaning | Confidence |
|---:|---|---|---|---|
| 1 | map | levels | LOD level -> children. Instantiate runs the entry for `g_UOParticleLODLevel`, falling back to lower keys (see "Runtime semantics"). | confirmed |

### 12 PropertyFlag (Create 0x0083bb65)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u8 | mode | +0x04 | stored as `value == 1`. Any value other than 0 or 1 clears the context's ok byte. | confirmed |
| 2 | u64 | mask | +0x08 | mode 0: the key is `(object flags (vt+0xb0) & mask) != 0` | confirmed |
| 3 | u32 | property | +0x10 | mode 1: which object value is the key (see below) | confirmed |
| 4 | list | default_children | +0x14 | mode 1: used when no case matches | confirmed |
| 5 | map | cases | +0x24 | key -> children | confirmed |

The fields are read in this order: mode, mask, property, default list, map. The 0/1 check on mode runs after
the map has been read.

In mode 1, `property` selects the key:

- 2: object vt+0x80 & 0xffff;
- 3: vt+0xb4;
- 7: the object hue (vt+0x58);
- 8: vt+0x68;
- any other value: `UOTileArt::GetObjectDefinition(vt+0x60)->GetProperty1(property)`, which is a tileart
  props1 key.

In mode 0, no match produces nothing.

### 15 Earthquake (body 24 bytes, Create 0x0083bee9)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | f64 | amplitude | +0x08 | camera target x += amplitude * sin(elapsed * frequency * 2pi) | confirmed |
| 2 | f64 | duration | +0x10 | seconds; the effect is finished when elapsed > duration | confirmed |
| 3 | f64 | frequency | +0x18 | Hz | confirmed |

### 16 Random (Create 0x0083bd90)

| # | Type | Name | Meaning | Confidence |
|---:|---|---|---|---|
| 1 | u32 | count | number of entries | confirmed |
| 2 | count x {f32 weight, list} | entries | weights are percentages; Instantiate picks one entry or nothing (see "Runtime semantics") | confirmed |

### 17 UserPreference (Create 0x0083bcc1)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u32 | mask | +0x04 | the children are skipped when `(mask & user effect setting `g_UOUserEffectFilter` 0x00e33584) == mask`, or when mask == 1 and object vt+0xa0 < 0. Bit 0 = footsteps disabled. | confirmed |
| 2 | list | children | +0x08 | | confirmed |

## Verification against the shipped data

`verify_effects.py` was run on the Enhanced Client files in
`/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced/` with this command:

```
python3 -I verify_effects.py --tileart "$C/tileart.uop" \
    --collection "$C/EffectDefinitionCollection.uop" --strings "$C/string_dictionary.uop"
```

It exited with code 0.

### tileart.uop

All 37756 records were consumed to the exact last byte, with the effects as the last field.

Records by effect count:

| Effects per record | Records |
|---:|---:|
| 0 | 37427 |
| 1 | 313 |
| 2 | 16 |

Definitions by type:

| Key | Type | All levels | Top level |
|---:|---|---:|---:|
| 0 | Particle | 572 | 321 |
| 11 | LOD | 1 | 1 |
| 12 | PropertyFlag | 23 | 23 |

The maximum nesting depth is 1.

Value distributions:

- **PropertyFlag:**
  - always mode 1, property 7 (hue) and mask 0;
  - case keys are hue values: 0 x23, 33 x21, 52 x21, 152 x21, 355 x18, 32 x6, 153 x5, 237 x1.
- **LOD:** keys 1, 2 and 3.
- **Particle:**
  - loop: 1 x532, 0 x40;
  - hued: 1 x6;
  - attach_mode (field_30): 1 x550, 2 x19, 0 x3;
  - field_3c: always 0;
  - duration: 0 x275, 1 x147, 3 x65, ...;
  - scale: 0.25 x269, 1.0 x135, ...;
  - offset_x: mostly 0;
  - offset_y: mostly 0;
  - camera_offset: 0 x253, -10 x85, -2 x39, 10 x32, ...;
  - 202 distinct names.

### EffectDefinitionCollection.uop

All 269 entries were consumed exactly. Each entry is `u32 count` followed by the definitions. There are 280
top-level definitions in all.

| Key | Type | All levels | Top level |
|---:|---|---:|---:|
| 0 | Particle | 279 | 278 |
| 1 | Sound | 2 | 0 (nested in the Wait) |
| 10 | Wait | 1 | 1 |
| 15 | Earthquake | 1 | 1 |

Value distributions:

- **Particle:**
  - loop: 0 x247;
  - offset_x and offset_y: -8 in 23 definitions;
  - attach_mode (field_30): always 1;
  - hued: always 0;
  - 258 distinct names.
- **Wait:** (500, 2000, aligned 0), containing two Sounds (id 43, 1.0).
- **Earthquake:** (0.1, 1.0, 10.0).

### Relation between the two files

- **Same format.** Both files use the same definition format and the same reader.
- **Inline copies.** Tileart effects are inline copies, not references into the collection. Only 8 particle
  names appear in both files: `00009970_AirElemental.nif`, `00014026_Explosion3.nif`,
  `00014052_SmallFireball.nif`, `00014138_sparkle.nif`, `DarkBall.nif`, `Nether_Bolt.nif`, `fizzle.ems`
  and `flamestrike_anim.ems`.
- **Loading.** `EffectDefinitionCollection.uop` is mounted by `UOAssets::MountPackages` (string at
  0x00cfdaf0). Each entry is one asset of type 0x0d000000, keyed by effect id (`FUN_004cb8c0` pushes
  {id, 0x0d000000}).
- **Entry names.** The entry names are `build/effectdefinitioncollection/%08d.bin`. 267 of the 269 hashes
  match ids in 0..0x3fffff, including 0, 1, 666, 1001-..., up to 70006. The other 2 entries have unknown
  names.
- **Asset factory.** The factory is `UOEffectDefinitionCollectionBinaryFactory` (vftable 0x00cdef1c),
  whose Create is 0x00841137. It calls ParseEffectDefinitionCollection, and the result is a
  `BinaryUOEffectDefinitionCollection`:
  - vtable +0x0: GetMemorySize;
  - +0x4: destructor;
  - +0xc: GetCount;
  - +0x10: GetDefinition.
- **Lookup by effect id.** `UOEffects::InstantiateEffectById` 0x004220c0 looks up an effect id; see
  "Load path" below.
- **Server effect ids.** The collection id is a 0xC7 packet effect field minus 1 (the 0x70/0xC0 copiers leave
  both effect fields at 0). Which field and which special values apply depend on the packet type; see
  "Effect fields per path" under "Keying".
- **Other readers.** Two other readers of the same format exist:
  - `FUN_0083c0ba`, a larger asset that embeds effect collections per entry;
  - `FUN_004b93d0`, which reads u32 id, u8 count and that many definitions.

  Neither was decoded further.

## Load path

| Item | Address | What it does |
|---|---|---|
| `UOEffects::InstantiateEffectById` | 0x004220c0 | cdecl (out ok, u32 id, out effect shared_ptr, object). The only user of the string `Missing Effect Definition %d\n` (0x00c9e6e8). |
| asset request | `FUN_004cb8c0` | builds the request {id, 0x0d000000} for `g_UOAssetManager` (0x00e37220) |
| `UOEffects::InstantiateAndRegister` | 0x00421b70 | (ECX definition, ESI ok, EDI out, EDX object): definition Instantiate (vtable +8); on success pushes the effect into `g_UOActiveEffects` |
| `UOEffects::InstantiateAndRegisterByValue` | 0x00421b20 | the same, taking the definition shared_ptr by value |
| `g_UOActiveEffects` | 0x00e32d68 | `std::list` of every running effect (head pointer 0x00e32d6c) |
| `UOEffects::UpdateActiveEffects` | 0x004218d0 | per frame: Update (+0x14) on each effect; if IsFinished (+0x10), Release (+0x18) and erase. It also updates the weather subsystem. |
| `UOEffects::ReleaseActiveEffects` | 0x00421a20 | releases every effect and clears the list |

`InstantiateEffectById` does the following (**confirmed**):

1. **Load.** The entry is requested synchronously from the asset manager on first use, then served from the
   manager's cache. There is no preload. The entry name is `build/effectdefinitioncollection/%08d.bin`
   (asset type 0x0d000000).
2. **Missing id.** It logs `Missing Effect Definition %d` and repeats the request with id 0 (a
   `Placeholder01.ems` particle). If id 0 is missing too, ok is cleared and nothing plays.
3. **One definition.** It is instantiated directly with the object argument and registered.
4. **Several definitions.** A `UOEffectContainerDefinition` is instantiated and registered, and each
   top-level definition is instantiated as a child of that one `UOEffectContainer`. **All the top-level
   definitions of an entry play at the same time**; there is no sequencing.

`UOEffectContainer` (vftable 0x00c9e15c) forwards Start, Stop, SetVisible, SetPosition and the +0x24 vector
to every child. It is finished when it has no children left (Update releases and erases finished children).
GetKind returns 6. Its +0x30 slot (lifetime ticks) is a shared no-op, so a packet duration does not reach the
children of a multi-definition entry.

## Keying: packets and fixed ids

The collection id is the effect number. These are all the code paths that select an entry (the 5 call
sites of `InstantiateEffectById`; **confirmed**):

| Caller | Address | Effect id |
|---|---|---|
| `UOEffects::PlayEffectAtTile` | 0x00421130 | from `UOEffects::PlayGraphicalEffect` (packets 0x70/0xC0/0xC7): `effect - 1` for types 2/3 (9501 and 9502 intercepted first), or 666 for type 1 lightning |
| `UOProjectileEffect::SpawnExplosion` | 0x0044ca90 | `(explode effect - 1) & 0xffff` of a moving effect with the explode flag, with no 9501/9502 intercept; explode effect 0 (always for 0x70/0xC0) selects the classic tile effect 0x36CB for 10 ticks instead |
| `UOEffects::ShowTargetIndicator` | 0x005cfb70 | 10001 (`Splat_Golden.nif` + `Splat_Splash.nif`), attached to the new current target. It is called by the set-current-target function `FUN_005cfa70` when the target type is 2 and the option byte `DAT_00e33657` is 0. The effect is kept in a global and the previous one is stopped. |
| `UOWaypoints::SetWaypointEffect` (probable) | 0x005d29e0, 2 sites | `FUN_005d49d0(waypoint type)`: a per-type id table (`[this+0x34]`, index from a map; an unknown type uses index 0). It is attached to the waypoint's mobile/item, or to a placeholder object created at its location. It is reached from the Lua `UOCreateUserWaypoint` paths. Probably ids 20000-20011. |

### Packet handlers

| Packet | Handler (`UOObjectManager` callback, registered in `FUN_005bc540`) | Reader | Field copier |
|---|---|---|---|
| 0x70 | `UOObjectManager::Receive_0x70_GraphicalEffect` 0x005c0140 | `UOPacket::Read_0x70_GraphicalEffect` 0x005bb1a0 | `UOEffects::PlayGraphicalEffect_0x70` 0x00421e60 |
| 0xC0 | `UOObjectManager::Receive_0xC0_HuedEffect` 0x005c01c0 | `UOPacket::Read_0xC0_HuedEffect` 0x005bb4e0 | `UOEffects::PlayGraphicalEffect_0xC0` 0x00421f10 |
| 0xC7 | `UOObjectManager::Receive_0xC7_ParticleEffect` 0x005c0240 | `UOPacket::Read_0xC7_ParticleEffect` 0x005bb770 | `UOEffects::PlayGraphicalEffect_0xC7` 0x00421fd0 |

Each copier fills a 0x38-byte record from the parsed packet and calls `UOEffects::PlayGraphicalEffect`
0x00421220. The 0x70 copier leaves hue, render mode and effect at 0. The 0xC0 copier adds hue and render
mode. The 0xC7 copier also copies the effect, explode effect, explode sound, the serial, the layer and two
trailing bytes. Only the three copiers call the dispatcher. No use of an effect id was found for 0x54
(sound), 0xBF or the Lua API (Lua reaches the collection only through the waypoint path).

`PlayGraphicalEffect` switches on the packet type (**confirmed**):

| Type | Action |
|---|---|
| 0 moving | `UOEffects::MakeProjectileEffect` 0x00420a90: a `UOProjectileEffectDefinition` (item id, hue + 1, speed, serials, explode flags, explode effect and sound, render mode; effect 9502 sets `suppress_tileart_effects`, any other effect value is ignored, and the fixed-direction byte is not read). It has its own particle filter test. On arrival, see "Effect fields per path". |
| 1 lightning | particle filter on the source serial, then `PlayEffectAtTile(666, ticks 0)` (`lightning.ems`) |
| 2 fixed location, 3 fixed on mobile | Gets the source object (type 3 without an object plays nothing), then tests `UOEffects::IsParticleFiltered(serial, location = type == 2)`. Effect 9501 (0x251D) plays nothing. Effect 0 or 9502 (0x251E) plays the classic item effect `UOEffects::MakeTileEffect` 0x00420e10, if the item id is non-zero. Any other effect plays `PlayEffectAtTile((effect - 1) & 0xffff)`, with the packet duration as lifetime ticks for type 2 and 0 for type 3; type 3 then attaches the effect to the object's effects component (`UOObjectComponentEffects::AddEffect` 0x00443e90). |
| 4 | screen fade: a `UOFadeEffectDefinition` from the item id |

Consequences:

- **Packet effect value.** `effect - 1` is the collection id (verified by name: packet effect 5002 = id 5001
  `00005001_clumsy`, and the like). Ids reachable from packets are 0..65534.
- **Effect 0.** Packet effect 0 is not id -1: it means "no EC effect, use the item id".
- **9501 and 9502.** These values are special only in the `effect` field of types 2/3 (and 9502 in the
  `effect` field of type 0). The explode-effect path has no intercept. See the table below.
- **Hue and render mode.** `PlayEffectAtTile` passes object 0, so collection effects ignore the packet hue
  and render mode. A `hued` particle needs an object hue, so it is never hued on this path. PropertyFlag
  has no object to read either. The classic paths do carry them: the projectile and tile-effect objects get
  hue = packet hue + 1 (0 stays 0) and the render mode, and the `hued` particles of their item's tileart
  effects are hued with that value (see "Hued NIF effects").

### Effect fields per path

**confirmed** (instruction level). Copier record offsets: +0 type, +4 source serial, +8 target serial, +0xc
item id, +0xe/+0x10/+0x12 source x/y/z (u16, u16, i8), +0x14/+0x16/+0x18 target x/y/z, +0x19 speed, +0x1a
duration, +0x1d fixed direction, +0x1e explode; 0xC0 adds +0x20 hue and +0x24 render mode; 0xC7 adds
+0x28 effect, +0x2a explode effect, +0x2c explode sound, +0x30 serial and +0x34 layer.

| Type | Field read | Value | Result |
|---|---|---|---|
| 0 moving | effect (+0x28) | 9502 (0x251E) | projectile `suppress_tileart_effects` = 1 (its item shows without its tileart particles) |
| 0 moving | effect | anything else | ignored; the projectile shows the item id with its tileart effects |
| 0, on arrival with explode set | explode effect (+0x2a) | 0 | `UOTileEffectDefinition` item 0x36CB, projectile hue and render mode, 10 ticks, attached to the target |
| 0, on arrival with explode set | explode effect | N != 0 | collection `(N - 1) & 0xffff` at the target, no intercept: 9501 → 9500 (`ColorSwirl`, missing file), 9502 → 9501 (`Null.nif`, missing file), 9503 → 9502 (`fizzle.ems`) |
| 0, on arrival | explode sound (+0x2c) | any | played when exploding; a return flag sends a projectile back |
| 1 lightning | none | | collection 666 |
| 2 / 3 | effect (+0x28) | 9501 (0x251D, CMP at 0x00421391) | nothing |
| 2 / 3 | effect | 0 or 9502 (0x251E, CMP at 0x004213a7) | `MakeTileEffect` 0x00420e10 if item id != 0; 9502 sets `suppress_tileart_effects` (CMP at 0x00420ea2, tested by `UOTileEffect` at 0x00466c04) |
| 2 / 3 | effect | N, other | collection `(N - 1) & 0xffff`; ticks = duration (type 2) or 0 (type 3, then attached to the source object) |
| 4 fade | item id | | `UOFadeEffectDefinition` |

Reachability of the special entries: collection 9500 and 9501 only through the explode effect (9501, 9502;
both files are missing, so nothing plays after the error log); collection 9502 (`fizzle.ems`) as effect or
explode effect 9503. 0x70 and 0xC0 packets never reach the collection except through type 1 (666).

New types in `/UOEffects`: `UOProjectileEffectDefinition` (0x40) and `UOTileEffectDefinition` (0x24), with
the fields above (hue, render mode, `suppress_tileart_effects`, `on_mobile`, `duration_ticks`).
- **Position.** `PlayEffectAtTile` sets the effect at the packet location. Type 3 then follows the mobile,
  because the effects component moves its effects when the object moves.

`UOEffects::IsParticleFiltered` 0x0049a800 (serial, serial high, location effect) uses the user setting
`g_UOParticleFilter` 0x00e33580. The setting comes from the settings file (`Particles` element, `filter`
attribute, default 3) and from Lua `SystemData.Settings.Resolution.particleFilter`.

| Value | UI (cliloc) | Shown |
|---:|---|---|
| 0 | None (1112331) | nothing |
| 1 | Player/Target | effects on the player and on the current target, and location effects |
| 2 | Party/Target | as 1, plus party members |
| 3 | All (default) | everything |
| 4 | Hide AoE (1158020) | everything except location effects |

## Runtime semantics

All of the following are **confirmed** in the assembly unless marked.

- **Lifetime ticks.** IUOEffect vtable +0x30 (`SetLifetimeTicks`) stores an update countdown.
  `UOParticleEffectNif` keeps it at +0x40 and `UOParticleEffectChaos` at +0x3c. Init sets it to -1 (no
  limit). Update decrements it while it is positive, and IsFinished is true at 0. The packet duration
  byte is therefore a count of update ticks (frames), not seconds. It applies only to single-definition
  entries, because the container ignores it. The particle `duration` field (seconds) is independent of it.
- **Chaos stop.** `UOParticleEffectChaos::Stop` clears active (+0x24) and sets stopping (+0x25); the loop
  flag is +0x26. After Stop, Update advances a fade timer (+0x2c) and keeps updating the node until 5.0 s
  (constant 0x00c98590). IsFinished (probable in detail) is true when the countdown is 0 or the fade
  timer reached 5 s, and false while fading. While active, it is true once no particle system of the node is
  alive.
- **RoundRobin** (`UORoundRobinEffectCollection::Instantiate` 0x0040e330): instantiates child[index]
  directly (no container), then sets `index = (index + 1) % count`. The index (+0x14) belongs to the
  definition, so it is shared by every use of that definition: all objects of one tile id, and every play of
  one collection entry.
- **Wait** (`UOWaitCollectionDefinition::Instantiate` 0x0040d9b0): a throttle evaluated only when the
  definition is instantiated. It never fires again by itself.
  - The time base is QueryPerformanceCounter in ms. `next` (+0x20, per definition) starts at 0, so the
    first use fires.
  - When `now >= next`, it sets `next = now + min`. With `aligned`, it is instead `now + (min - now % min)`,
    the next multiple of min. It then adds an MT19937 uniform integer in [0, max - min] and instantiates all
    children in a new container.
  - Otherwise it produces nothing, with ok still set.
  - The only collection Wait is id 70003 (thunder flash plus two sounds 43; 500/2000 ms, not aligned).
- **Random** (`UORandomEffectCollectionDefinition::Instantiate` 0x0040de80): `r = u32 * 2^-32 * 100`
  (MT19937 `FUN_0099aa85`), so r is in [0, 100). It walks the entries in order: if `r < weight`, it plays
  that entry's children; otherwise `r -= weight`. **Weights are percentages.** If the weights sum to less
  than 100, the remainder plays nothing; any excess beyond 100 is unreachable.
- **LOD** (`UOEffectLODDefinition::Instantiate` 0x0040cfd0): the key is `g_UOParticleLODLevel`
  0x00e3357c. That comes from the settings file (`Particles` element, `lod` attribute, decimal, accepted
  when < 4, default 3), and from Lua `SystemData.Settings.Resolution.particleLOD`, where the UI combo Low /
  Medium / High (cliloc 1079210-1079212) maps to 1..3. A missing key falls back to the next lower key, down
  to 0. If none matches, it plays nothing.
- **UserPreference** (`UOUserPrefEffectFilterDefinition::Instantiate`): the setting is
  `g_UOUserEffectFilter` 0x00e33584.
  - It comes from the settings file (`Effects` element, `filter` attribute, hex, reset to 0 before reading;
    default 0).
  - Bit 0 is set when footsteps are disabled. Both the sound settings `Footsteps` element (`FUN_0048d330`)
    and Lua `SystemData.Settings.Sound.footsteps.enabled` (`FUN_00499860`) set it. No other bit is
    written.
  - The children are skipped when `(mask & setting) == mask`, or when the mask is 1 and the object's
    vtable +0xa0 is negative.
- **PositionOffset:** the runtime `UOPositionOffset` (0x50 bytes) holds the 8 offsets, reports finished at
  once and is never read (no dynamic_cast of its type descriptor, ProbeEffColl05). The *definition* is not
  inert, though: `UOObjectComponentAnimationMob::UpdateFrame` 0x00436d40 reads type 7 definitions from the
  mount's sequence frame effects to place the rider (test at 0x0043725c; see `uosa_animation.md`, "Body to
  sequence and frames"). No collection, tileart or sequence data uses it (all 56 frame effects are
  UserPreference + Sound).
- **PropertyFlag and Earthquake:** as documented under "Formats per type". PropertyFlag needs an object, so
  in collection entries played from packets (object 0) only the default children of mode 1 can play.
- **Double registration** (probable quirk, not checked at run time). The composite Instantiates (LOD, Wait,
  UserPreference, Random, `UOEffectCollectionDefinition`) build their container through
  `InstantiateAndRegisterByValue`, so it is registered in `g_UOActiveEffects`. The caller's
  `InstantiateAndRegister` then registers the returned container again. A composite at top level is
  therefore updated twice per frame, which halves tick-based lifetimes.

## Particle resource resolution

Both particle kinds build the path `"Data/Effects/" + name` (string 0x00c9e6d8, appended with
`FUN_00423ec0`). The file is opened through Gamebryo's virtual file system (`NiFile::ms_pfnFileCreateFunc`),
which the client points at its UOP packages (**confirmed**).

- **`.nif` → `UOParticleEffectNif`** (`FUN_00421be0`): caches the loaded scene per name in the map
  `DAT_00e32d74`, under the critical section 0x00e32d4c. It loads with `NiStream` (`FUN_00681960`,
  `FUN_00683470`), and each instance clones the cached scene (`FUN_00420200`). A failure logs `Unable to
  load particle nif %s`.
- **`.ems` → `UOParticleEffectChaos`** (`FUN_00421d70`): the Chaos `ParticleSystemManager` (`DAT_00e32da8`).
  1. `FUN_008a7920` uppercases the path and checks the manager's cache.
  2. It requires the `EMS` extension (an `EMI` check also exists).
  3. It parses the text script (`FUN_0089ec30`, a `particlesystem "<NAME>" <version> { eventSequence ...
     }` file).
  4. `_splitpath` gives the file name, and `FUN_008a7ca0` instantiates the cached system by its uppercase
     name.

  An `.ems` script names further `.nif` meshes and `.tga` textures, also resolved under `Data/Effects/`.
- **Package.** The files are entries of **`EffectTexture.uop`** named `data/effects/<lowercase file name>`.
  Checked with the UOP name hash for the collection's 258 distinct names: 254 resolve (233 `.nif`, 21 `.ems`).
  The `.tga` and `.nif` names referenced from `.nif` and `.ems` files also resolve there.
- **Missing names.** Four names have no entry: `00009011_TrailEarth1s.nif`, `00009500_ColorSwirl.nif`,
  `00009501_Null.nif` (the "nothing" effect, apparently on purpose) and
  `00009533_Life_Tap_Send_Impact.nif`. The client then logs the error and plays nothing.

## Tileart effects vs collection effects

| | Tileart effects | Collection effects |
|---|---|---|
| Source | the record of the item's graphic | an entry of `EffectDefinitionCollection.uop`, keyed by the packet effect - 1 or a fixed id |
| Instantiated by | `UOObject::CreateComponents` 0x005befd0, once per world object creation; also `UOTileEffect::Init` 0x00466940 (classic 0x70/0xC0/0xC7 item effects and the projectile explosion 0x36CB) and `UOProjectileEffect::Init` 0x0044bb40 (the moving item) | `UOEffects::InstantiateEffectById` per packet or event |
| Object | the world object: hue, `hued` and PropertyFlag work. For classic packet effects this is the projectile or tile-effect object, whose hue is packet hue + 1 | none for packet effects (no hue, no PropertyFlag); the target or waypoint object for the indicator and waypoints; the animated object for animation frame effects |
| attach_mode | `CreateComponents` skips particles with attach_mode 2 (`PARTICLE_ATTACH_PROJECTILE_ONLY`). `UOTileEffect::Init` and `UOProjectileEffect::Init` instantiate every particle. | not tested (all collection particles are 1) |
| Ownership | registered in `g_UOActiveEffects` and added to the object's `UOObjectComponentEffects` (component id 4, vftable 0x00c9fb44; `UOObjectComponentEffects::AddEffect` 0x00443e90). The component sets visibility and position, follows the object (`HandleNotify` message 1) and stops everything when the component is cleared (+0x8); it never updates the effects. | registered in `g_UOActiveEffects`; type 3, the indicator and waypoints are also added to the object's component |
| Lifetime | the object's: started once; ends when the effect finishes or the component is cleared | the effect's own end, or the packet duration in ticks (type 2, single definition) |

So a classic packet effect (effect field 0, or 0x70) still gets Enhanced Client particles, through the
tileart effects of its item id. RoundRobin and Wait state in tileart effects advances per object creation
and is shared by all objects of the same graphic.

## Hued NIF effects

### Selection

`UOParticleEffectDefinition::Instantiate` 0x0040e490 (**confirmed**):

- a name ending in `.ems` → `UOParticleEffectChaos` (Init 0x0041aff0). The object is not passed, so **Chaos
  effects are never hued**, whatever the `hued` flag says;
- object != 0, object hue (`UOObject::GetHue`, vtable +0x58 0x005b3930, field +0x44) != 0 and `hued` set →
  `UOHuedParticleEffectNif` (0x84 bytes, constructor 0x0041d9c0, ESI = effect, EDI = object);
- otherwise → `UOParticleEffectNif`.

Hue sources (the object passed down to Instantiate):

| Path | Object | Hue H |
|---|---|---|
| tileart effects (`UOObject::CreateComponents`) | the world object | its stored hue (`SetHue` 0x005b3940), not masked |
| classic packet effects (`UOProjectileEffect::Init`, `UOTileEffect::Init`) | the projectile / tile-effect object | packet hue + 1, 0 stays 0 (0xC0/0xC7; 0x70 has hue 0) |
| animation frame effects (`IUOEffectDefinition::PlayOnObject`) | the animated object | its stored hue |
| collection effects from packets (`PlayEffectAtTile`) | none | never hued |
| target indicator, waypoints | target / waypoint object | its hue (no `hued` particle in those entries) |

No 0x8000 / 0x4000 bit is tested or masked on this path. H = 0 is never hued.

### Construction and Init

- **Constructor** 0x0041d9c0: +0x14 = H (raw), +0x18 = object position (vtable +0x28), +0x30 = camera
  direction × (object vtable +0x11c × -0.001 - object vtable +0x124 × 1e-5 (0x00da7b24)), a depth bias.
  **confirmed** (the meaning of the two object values is unknown).
- **Init** 0x0041dc30 (**confirmed**):
  1. clones the NIF scene (+0x44) as for the plain effect;
  2. `UOHuedParticleEffectNif::CreateOffscreenCamera` 0x00472160: a new `NiCamera`, viewport (0, 1, 1, 0),
     orthographic frustum left/right ∓4·w/h, top 4.0 (0x00c9b2c8), bottom -4.0 (0x00d05258), near 8
     (0x00c9c3a4), far 12 (0x00c9c3a8), where w/h is the screen size (0x00e3352c when the byte 0x00e33538 is
     set, else 0x00e33520);
  3. `UOHuedParticleEffectNif::PlaceOffscreenCamera` 0x004722a0: rotation copied from the world camera
     (`g_UOWorldViewer` +0x7c, NiAVObject +0x2c), translation = `g_UOWorldViewer` +0x18 (the -10 × view
     direction camera offset) + effect position;
  4. `UOHuedParticleEffectNif::BuildHueQuad` 0x0041df40: a 4-vertex camera-facing quad, half size
     4.8541555 (0x00d05360) along camera right and up (vertices top-left, top-right, bottom-left,
     bottom-right; triangles (0, 2, 1), (1, 2, 3)), centred on the effect position plus 2 × the depth bias
     (+0x30), vertex colour white. Three texture coordinate sets are written in one array: set 0 = (0, 0),
     (1, 0), (0, 1), (1, 1), the whole render target with no half-texel offset (0x0041e420..0x0041e499,
     **confirmed**); set 1 = (0, 0) for every vertex (0x0041e4aa..0x0041e511); set 2 = the hue coordinate
     (below) for every vertex (0x0041e593..0x0041e5b4). Material `UOEffectsShader`, a 512 × 512 render target as shader map 0, `Build\Hues\Hues.dds`
     (loaded through the texture manager, `g_UOWorldViewer` +0xa8, vtable +4) as base map, and an
     `NiAlphaProperty` with flags 0x12ED: blend SRCALPHA / INVSRCALPHA, alpha test GREATER, reference 0.

  Map flags (Gamebryo `NiTexturingProperty::Map`: bits 12-13 clamp, bits 8-11 filter), **confirmed**: the
  render-target `ShaderMap` (vftable 0x00c9db18) is created with 0x3100 (0x0041e845), added
  (`FUN_00416830`), then rewritten to `& 0xc1ff | 0x100` = CLAMP_S_CLAMP_T + BILERP (0x0041e874) before it is
  set as shader map 0 (`FUN_00670bd0`); the `Hues.dds` base map (vftable 0x00cbf528) keeps 0x3100 =
  WRAP_S_WRAP_T + BILERP (0x0041e922).

### Per frame

`UOHuedParticleEffectNif::Update` 0x0041edc0 (**confirmed**):

- `elapsed (+0x48) += dt × 0.5` (0x00c95604): **hued NIF effects run at half speed**, and a non-looping
  effect (`duration` +0x5c compared with elapsed) lasts twice as long in real time. Looping uses fmod. The
  tick countdown (+0x80) is unaffected.
- Skips rendering while hidden (vtable +0xc).
- Updates the scene (`FUN_00663db0`), then `UOHuedParticleEffectNif::RenderToTexture` 0x0041ebc0: renderer
  `DAT_00e31ff8`; saves the background colour (`NiDX9Renderer::GetBackgroundColor`, vtable +0x60,
  0x007614b0), sets it to `NiColorA(0, 0, 0, 0)` (`SetBackgroundColor`, +0x58, 0x00761580, which packs the
  alpha into the D3DCOLOR at renderer +0x3fc), begins the render target with clear flags 7 (colour, depth,
  stencil; `FUN_00416030` → renderer +0x168), sets the camera data from the offscreen camera, draws with
  `NiDrawVisibleArray`, ends the target (`FUN_004160d0`) and restores the saved colour. **The target is
  cleared to transparent black (confirmed).** No light is set by this path.
- Updates the quad node (+0x10), which draws in the world.

### Shader and formula

`UOEffectsShaderHLSL` (constructor 0x0058ec40, vtable 0x00cb3bac; FFP variant 0x0058e350, vtable 0x00cb3b04).
`UOEffectsShaderHLSL::UpdatePipeline` 0x0058f110 (vtable +0x1c): shader map 0 → sampler +0x120, base map →
sampler +0x140 and the "has hue texture" constant +0x160 = 1; each sampler takes clamp = flags >> 12 & 3 and
filter = flags >> 8 & 0xf; it then sets D3DRS_SRGBWRITEENABLE (0xc2) to 0.

Hue coordinate from `BuildHueQuad` (**confirmed**), with H the raw object hue:

```text
column = H / 1024          (integer)
row    = H % 1024
uv2    = (0, 0)                                   if H == 0
uv2.u  = column * (1 / 1024) (0x00d05158) * 255.0 (0x00d04ef0)
uv2.v  = row / 1024
```

H is the signed 32-bit hue as stored: `/ 1024` and `% 1024` are signed (CDQ / AND 0x3ff / SAR 10 and
AND 0x800003ff with the negative fix-up, 0x0041e53f..0x0041e56c). Nothing masks or clamps H, so a hue past
the atlas's loaded rows samples whatever the atlas holds at its coordinate (u wraps past 1 with the WRAP
base map): transparent where the atlas is empty.

Pixel shader `uoeffects.psh` (Shaders.uop; **confirmed** from the source):

```text
c    = tex2D(render target, uv)                    // the effect as rendered offscreen
lum  = dot(c.rgb, (0.2125, 0.7154, 0.0721))
if (uv2.x > 0 || uv2.y > 0)                        // HAS_HUE_TEX branch
    h   = tex2D(HueSampler, uv2 + (lum * 255 / 1024, 0))
    out = (h.rgb, c.a * h.a * vertex.a)
```

So the hued effect replaces each pixel's colour with the hue gradient entry selected by its luminance, keeps
the rendered alpha, and is drawn with normal alpha blending (not additive), whatever the NIF's own blend
mode was.

Quirks (derived from the confirmed numbers and map flags, not observed in a frame capture):

- The atlas column stride is 255 texels, not the 256-texel strips of `hues.dds` (`atlas_coord_for_hue` in
  `lib/uocf/src/enhanced/hues.rs`). The shipped atlas is 1024 texels wide (see "Hue atlas" below), so
  hues >= 1024 sample `column` texels to the left of their strip; at low luminance they reach the
  previous strip.
- With bilinear filtering, u and v fall on texel edges: each sample is a 50/50 blend of rows `row - 1` and
  `row` (row 0 wraps to 1023) and of two adjacent gradient entries.
- The offscreen frustum is 8 × 8·w/h world units but the quad is 9.708 × 9.708 (2 × 4.8541555) and its
  UV set 0 spans the whole target (**confirmed**), so a hued effect is drawn about 1.21× taller than the
  plain one and squeezed horizontally by the screen aspect (1.21 / (w/h) as wide), and anything outside
  ±4 units vertically or ±4·w/h horizontally is clipped. The 512 × 512 target therefore has non-square
  texels (w/h wider than tall); `lib/ec-effects/particle-sim` renders it with square texels and the same mapping,
  which differs only in horizontal sampling density.

### Hue atlas (`Hues.dds`)

The texture (all **confirmed** on the shipped data and the binary unless marked):

- Package `Hues.uop` (EC), version 4, 3003 entries, 12-byte timestamp headers. Entry
  `build/hues/hues.dds`: DDS, 1024 × 1024, flags 0x81007 (no `MIPMAPCOUNT`), pitch 4194304, mip count 0,
  pixel format flags 0x41 (RGB + alpha pixels), 32 bpp, masks R 0xff0000 / G 0xff00 / B 0xff / A
  0xff000000 (A8R8G8B8, uncompressed), caps 0x1000. The other entries are `hue%04d.bmp` (1..3000),
  `huenames.csv` (3000 lines) and the 64 × 64 A8R8G8B8 swatch texture with hash 0xFA5C6A1BC0D8B01B.
- Layout: four 256-texel strips. Strip 0 holds rows 1..1023 = hues 1..1023 (row 0 is transparent black),
  strip 1 rows 0..1023 = hues 1024..2047, strip 2 rows 0..952 = hues 2048..3000, strip 3 is empty
  (transparent black). Filled texels have alpha 255. So hue H is at strip `H / 1024`, row `H % 1024`; each
  row is a 32-colour gradient, colour i at x = 8·i within the strip, linearly interpolated between
  neighbours (clamped at the ends). The bitmap `hue%04d.bmp`'s top row equals the atlas row (maximum
  difference 0 for hues 33, 500, 1500, 2600, 3000). Against Classic `hues.mul` the gradients match colour
  5-to-8-bit conversion `c << 3` (mean error 0.52, maximum 20 per channel over the atlas).
- Load: the string `Build\Hues\Hues.dds` (0x00c9dfa0) is loaded through the texture manager
  (`g_UOWorldViewer` +0xa8, vtable +4) from `BuildHueQuad` (0x0041e897), `FUN_0045cd80` (0x0045e5fc) and
  `FUN_00595010` (0x005951d3, the sprite shader data setup that reads the `CustomShaderData`
  `NiFloatsExtraData`). The texture goes through the normal Gamebryo DDS path; its size is never read back
  into a shader constant (**high**: the `HueSampler` users `FUN_0058ebf0` (UOEffects setup), `FUN_00593b30`
  (UOSprite) and `FUN_00594960` set no size constant).
- The constants are hard-coded, in C++ and in the shaders. `BuildHueQuad` multiplies by 1/1024
  (0x00d05158) and 255.0 (0x00d04ef0). The effects pixel shader (Shaders.uop entry 0xd6ebea59f37b1f2b)
  says so itself:

  ```text
  // TODO: have c++ inform the shader of these constants so we can change the size of the hue
  // texture if needed.
  static const float c_fHueTextureHeight = 1024.0f;
  static const float c_fNumHuePixels = 255.0f;
  static const float c_fHuePixelUVOffset = (1.0f / c_fHueTextureHeight) * c_fNumHuePixels;
  ```

  The two sprite shaders (0x00e0ca9c950954a8 with `256.0f / 1024.0f`, 0xa2e9963e6d9a7e50 with
  `255.0f / 1024.0f`) carry the comment "this is hardcoded for a 1024x1024 hue texture with four
  255-pixel columns".

Conclusion (**confirmed**): the atlas is 1024 × 1024, and `HUES_ATLAS_WIDTH = 1024` in
`lib/uocf/src/enhanced/hues.rs` and the constants of `lib/ec-effects/particle-sim/src/hued.rs`
(`HUE_ATLAS_ROWS` 1024, `HUE_ATLAS_UV_SCALE` 1/1024, `HUE_ATLAS_COLUMN_STRIDE` 255,
`HUE_ATLAS_STRIP_WIDTH` 256) match the client. No code change is needed.

Cross-check with the user's older converter `huesmul2uop` (`main.cpp` `createHuesDDS`, `uohues.*`):
the same 1024 × 1024 uncompressed ARGB32 image and the same strip and row mapping (strip 0 from row 1,
strips 1 and 2 from row 0), so it agrees with the shipped file and with `hues.rs`. Its pixels differ from
EA's: it expands 5-bit colour as `c * 255 / 31` (EA: `c << 3`), blurs each 8-texel block (`customBlur`,
radius 6) instead of a linear ramp, and `ARGB16::getR` masks `0xF800` instead of `0x7C00` before `>> 10`,
which takes in the alpha bit and drops red's low bit. Only the red mask is a bug; the rest is a fidelity
difference.

The ramp is exact (**confirmed**): texel `8·i + w` (w = 0..7) is `(c_i·(8 - w) + c_(i+1)·w) / 8` per channel
with `c = c5 << 3` and colour 31 held to the end. Both ends are multiples of 8, so the division never rounds.
Bitmap row 1 (the swatch) is the larger of colour 0 and colour 31 as 15-bit values, and the 64 × 64 texture
0xFA5C6A1BC0D8B01B is the swatch grid: hue h at x = h / 64, y = h % 64 (column-major, texel 0 empty). Against
the Classic `hues.mul` of the same release, 2635 of 3000 hues (2024 clients) have matching colours, and for
every one of them the atlas row, the swatch texel and the whole 2102-byte bitmap are reproduced byte for byte.
The other 365 were repainted in the EC: their bitmaps have other headers (2104 bytes with 96 dpi, or a 124-byte
V5 header) and colours off the `c << 3` grid, although `v >> 3` at x = 8·i still gives back the Classic colour
for all 3000. `huenames.csv` is hand-edited (lines without a comma, names one byte longer than the Classic
20-byte field allows). The KR `Hues.uop` (2016) has the same entries, order, atlas layout and swatch grid.

Repo note: `lib/uocf/src/enhanced/hues_mul_uop.rs` converts `hues.mul` to and from `Hues.uop` with these
rules, and `lib/udd-conv/src/hues.rs` builds the Classic atlas with them (`HueRampMode::Linear` by default;
`Flat` keeps the old 8-texel blocks). The EC path copies the bitmaps' row 0, which is exact.

## Effect camera and lighting

- **Effect orientation** (**confirmed**): `g_UOWorldViewer` 0x00e37224 (`UOWorldViewer`, constructor
  0x0046f870, `Init` 0x0046fa70) keeps the camera direction (+0xc), the camera offset -10 × direction
  (+0x18), -up (+0x24) and the effect rotation (+0x30) = C · P with P = [[0,1,0],[0,0,1],[1,0,0]]. Every
  NIF and Chaos effect node takes this rotation, so in the NIF's local frame **X = camera right, Y = view
  direction (into the screen), Z = camera up**: effects are camera-aligned billboards, not placed in world
  axes. A viewer reproduces the client by rendering the NIF orthographically along local +Y with local +Z
  up; Chaos gravity along local Z points along camera up.
- **World camera** (**confirmed**): `UOWorldViewer::CreateCamera` 0x00471580 and `SetCameraFrustum`
  0x004712f0: orthographic, left/right ∓f / 0.75 (0x00db9754), top/bottom ±f, near 8, far 12, f = 16.0
  (+0x80) × the viewport height fraction (0x00e33590). The rotation is built with `NiMatrix3` products
  (0x00668f10) as C = Z(-π/2) · X(0.001 - π/2 × 0.5) · Z(-0.05), with `MakeZRotation(t)` =
  [[c, s, 0], [-s, c, 0], [0, 0, 1]], `MakeXRotation(t)` = [[1, 0, 0], [0, c, s], [0, -s, c]], π/2 at
  0x00dc1fd0, 0.001 at 0x00c9857c, 0.5 at 0x00c95604 and -0.05 at 0x00d051c0. C ≈
  [[-0.0354, -0.7069, 0.7064], [0.9988, -0.0500, 0], [0.0353, 0.7055, 0.7078]]; its columns are the view
  direction, up and right.
- **Lighting of plain NIF effects** (**confirmed: no light**):
  - `UOParticleEffectNif::Init` 0x0041a8e0 attaches the scene to `g_UOEffectRenderGroups` 0x00e32e28
    entry 7, a parentless `NiNode` with an empty effect list. The only callers of
    `NiDynamicEffect::AttachAffectedNode` 0x006a1900 are `LinkObject`, `ProcessClone`,
    `NiNode::AttachEffect` 0x006768c0 (reached only through `CopyEffectListClones`) and
    `MythUi::NifDisplay`; none adds a light to the group or the effect root. `MythicAmbientLight`
    (factory 0x00a5db60) is a streamable `NiAmbientLight` subclass used only through NIF streaming.
  - So no world sun, ambient or day/night level reaches effects: the effect lights are the lights inside the
    NIF itself. Of the 466 shipped NIFs only `00005540_shadowstrike.nif` has lights: two white
    `NiDirectionalLight`s (dimmer 1, ambient 0) on its root.
  - The renderer keeps `NiStandardMaterial` as the default material on shader model 2+ hardware
    (`FUN_00754fb0` 0x00755612..0x00755631 compares the VS/PS versions; otherwise it installs the legacy
    material at 0x007556a2); `NiDX9Renderer::UseLegacyPipelineAsDefaultMaterial` 0x0075f800 has no caller.
    With no lights, NiStandardMaterial (and the fixed-function pipeline) gives
    `emissive + ambient × 0 + diffuse × 0` = the material emissive, with the alpha from the
    material or the vertex colours by the `NiVertexColorProperty` mode (full formula with the texture
    maps in "NiStandardMaterial texture combine").
  - What that means for the data (survey of the 466 NIFs, 671 meshes and 1086 particle systems): files
    lit with `E_A_D` show only their emissive colour; the vertex colour modes (`SRC_EMISSIVE`) show the
    vertex colours; meshes with `E_A_D` / `SRC_IGNORE` and emissive 0 (33 + 4 meshes, about 10 particle
    systems) draw black. `lib/ec-effects/particle-sim` defaults to this (`CLIENT_EFFECT_LIGHTING`).
- Hued effects render offscreen with no light set by the client (same result).

## Effect textures and mipmaps

All **confirmed** on the binary unless marked.

- `NiTexture_ms_bMipmapByDefault` 0x00e31ea3 (`NiTexture::ms_bMipmapByDefault`) lies in `.data`'s
  zero-filled part (false) and is never written (two reads: `NiDX9SourceTextureData::Create` 0x00770fe0 and
  `FUN_00774a80`). No `SetMipmapByDefault` call exists.
- The texture prefs come from each effect NIF's `NiSourceTexture` (`LoadBinary` 0x0068ac00 streams the
  pixel layout, mip flag, alpha format and the load-direct hint). The `NiSourceTexture` blocks of the shipped effect NIFs: 1043
  external `MIP YES`, 1 external `MIP YES` bump map, 6 external `MIP NO`, 5 internal `MIP YES`; none uses
  `MIP_DEFAULT` and none sets the load-direct hint. So the default flag never decides.
- `NiDX9SourceTextureData::Create` 0x00770b90: with the load-direct hint and a `.bmp`/`.tga`/`.dds` name it
  loads through D3DX (`LoadTextureFile` 0x00771af0); otherwise (every effect texture) it calls
  `NiDevImageConverter::ReadImageFile` 0x006942d0 (converter vtable 0x00cc0c60 +0xc), which opens the file
  through `NiFile` (the UOP-backed file) and asks the first reader whose `CanReadFile` accepts the
  extension. Mipmapped = prefs `YES`, or `MIP_DEFAULT` and the default flag (0x00770fcf..0x00770fee).
- `NiDevImageConverter::NiDevImageConverter` 0x00693cf0 registers BMP, DDS, SGI, TGA and
  `NiNIFImageReader` (vtable 0x00ccc1d8). `NiNIFImageReader::CanReadFile` 0x006ccde0 is
  `stricmp(ext, ".nif")`; `ReadFile` 0x006cce00 loads the file as an `NiStream` and returns its object only
  when the stream holds exactly one object (+0x188 == 1) and that object is an `NiPixelData` (RTTI chain
  against `NiPixelData::ms_RTTI` 0x00ddf5cc); otherwise 0. `ReadHeader` 0x006cceb0 reads the whole file and
  reports levels > 1 as mipmapped. So a texture named `*.nif` holding one `NiPixelData` loads.
- `NiDevImageConverter::ConvertPixelData` 0x00694700 (vtable +0x14, tail at `FUN_00694796`) converts the
  pixel data to the renderer format found for the prefs, keeping every stored level; it generates levels
  (`GenerateMipmapLevels` 0x00695590, box filter) only when mipmapping is on, the source has one level,
  both sides are powers of two and the target format is RGB, RGBA or a bump format (0x00694832..0x00694889).
  `CreateSurf` then creates the D3D texture with that many levels (1 if the sides are not powers of two and
  the device lacks non-power-of-two support).
- The three `.nif` texture files (`03000017_star.nif`, `03000018_circle_g.nif`, `03002014_flare.nif`) are
  Gamebryo 20.0.0.4 NIFs with one `NiPixelData`: RGBA, 128 × 128, 8 stored levels. They are referenced with
  `MIP YES`; their stored levels are used as they are.

## Texture filtering and anisotropy

Effect NIF maps (survey of the 466 shipped effect NIFs, all version 20.0.0.4: 991
`NiTexturingProperty` blocks, 1509 maps; **confirmed**):

| Map | NEAREST (0) | BILERP (1) | TRILERP (2) |
|---|---|---|---|
| base | 2 | 4 | 910 |
| bump | | | 32 |
| dark | | | 99 |
| decal | | | 42 |
| detail | | 1 | 213 |
| gloss | | | 19 |
| glow | | 1 | 186 |

No map uses `FILTER_ANISOTROPIC` (6) or any mode above 2, and no shader map is present. The NEAREST base maps
are in `615883b5d439d9c2`. Version 20.0.0.4 maps have no per-map max-anisotropy field.

Renderer (`UOSA.exe`, Gamebryo 2.3 `NiDX9Renderer`):

- The filter table `ms_akD3DFilterMapping` at 0x00e16648 (12-byte entries mag, min, mip) is filled by the
  device caps setup `FUN_00756990` (called from `FUN_00754fb0`) with **six** entries only: NEAREST (POINT,
  POINT, NONE), BILERP (LINEAR, LINEAR, NONE), TRILERP (LINEAR, LINEAR, LINEAR), NEAREST_MIPNEAREST (POINT,
  POINT, POINT), NEAREST_MIPLERP (POINT, POINT, LINEAR), BILERP_MIPNEAREST (LINEAR, LINEAR, POINT). The
  next words, 0x00e16690..0x00e166ac, are the clamp table `ms_akD3DAddressMapping` (CLAMP/CLAMP,
  CLAMP/WRAP, WRAP/CLAMP, WRAP/WRAP). There is no FILTER_ANISOTROPIC entry: Gamebryo 2.6 adds one
  (`NiDX9Renderer.cpp` 2056-2070), this build does not. **Confirmed.** A mode 6 map would index the clamp
  table (**probable**, irrelevant since no map uses it).
- The stage setters `FUN_0074d530` (three sampler states from the filter table) and `FUN_0074d4e0` (two from
  the clamp table) read these tables; `FUN_0074d5d0` drops the mip filter to NONE for a texture without
  mipmaps. **Confirmed.**
- `FUN_00756990` reads `D3DCAPS9.TextureCaps` (renderer +0xe4: POW2, NONPOW2CONDITIONAL, PROJECTED,
  MIPCUBEMAP), `MaxTextureBlendStages` and `MaxSimultaneousTextures`, but neither `TextureFilterCaps`'
  anisotropic bits nor `MaxAnisotropy` (caps +0x6c). No TEST/AND with `D3DPTFILTERCAPS_MINFANISOTROPIC`
  (0x400) or `MAGFANISOTROPIC` (0x4000000) on a caps field exists in the renderer (0x0074xxxx-0x0077xxxx).
  **High.**
- `D3DSAMP_MAXANISOTROPY`: the initial sampler state table at 0x00da8690 (read by
  `NiDX9RenderState::InitTextureStageState` 0x00748e70) gives it 1 (pair at 0x00da86d8). Its slot in
  `ms_auiSamplesStateMappings` (0x00da867c) has no reference, and no direct `SetSamplerState(…, 0xa, …)`
  call exists (virtual-call scan for `+0x114` with the constant 0xa). So it stays 1. **High.**
- The effect shaders in `Shaders.uop` set no sampler state (no `anisotrop` text). **Confirmed.**

Conclusion: UOSA never filters effect textures anisotropically (max anisotropy 1). A faithful preview uses
each map's own mode as plain point/linear min, mag and mip filters, with anisotropy off (wgpu
`anisotropy_clamp` 1). Mapping a mode-6 map to trilinear, as
`lib/ec-effects/particle-sim/src/nif/desc.rs` `TextureFilter::from_raw` does, is fine (no shipped map has
it). No code change is needed.

## Draw order

All **confirmed** on the binary; the SDK references are Gamebryo 2.6 (`CoreLibs/NiMain`), which matches the
2.3 code at these points.

- **World pass.** `DrawWorldRenderGroups` 0x00470430 (probable name) draws the render groups of
  `g_UOEffectRenderGroups` 0x00e32e28 in the order 0, 1, 2, 3, 11, 6, then **7** (all NIF effects and the
  Chaos `ParticleSystemRoot`; skipped while the viewer +0x54 is set), then 9. Each group gets its own cull
  and its own `NiDrawVisibleArray` 0x0067f330 call, so **each group is sorted on its own**; groups share
  only the depth buffer. Group 1 and group 10 have an optional first pass through `UOShaderOverrideSorter`
  (`RegisterObjectArray` 0x00455ed0: every object at once in array order, no sort); effects never take it.
- **Sorter.** `UOGraphics::CreateRendererAndSorter` 0x0058a950 (probable name) creates the renderer and sets
  a new `NiAlphaAccumulator` (constructor 0x00698f70, vtable 0x00cc0cf8) as its sorter, with the defaults
  `ObserveNoSortHint = true`, `SortByClosestPoint = false` (`NiAlphaAccumulator.cpp:26-27`).
- **Per object** (`NiAlphaAccumulator::RegisterObjectArray`, `NiAlphaAccumulator.cpp:33-66`): an object is
  queued for sorting when its `NiAlphaProperty` has blending on, its no-sorter flag (alpha flags bit 13) is
  clear, and the object's no-sort bit (0x40 at `NiAVObject` +0xac in 2.3) is clear. **Every other object
  (opaque, alpha-tested only, no-sorter, or under an `NiSortAdjustNode` with `SORTING_OFF`) is drawn at once,
  in scene traversal order**, before any sorted object of the same group.
- **Sort key** (`NiAlphaAccumulator::Sort`, `NiAlphaAccumulator.cpp:68-110`): depth = world bound centre ·
  camera world direction. The queue is sorted by depth with an unstable quicksort
  (`NiBackToFrontAccumulator::SortObjectsByDepth`, `NiBackToFrontAccumulator.cpp:111`) and drawn from the
  largest depth down: **far to near by bound centre**, ties in unspecified order
  (`NiBackToFrontAccumulator::FinishAccumulating` 0x006d6990, `NiBackToFrontAccumulator.cpp:52-76`).
- **Sort items.** Each `NiGeometry` is one item with its own world bound:
  - a mesh: the bound of its vertices (computed when the NIF loads, then transformed);
  - an `NiParticleSystem`: the bound written by `NiPSysBoundUpdateModifier::Update` 0x00815930 (vtable
    0x00cdc610 +0x48) when the system has that modifier: the AABB of the particle positions (sampled every
    `skip` particles), centre = AABB centre, radius = half diagonal; otherwise the bound stored in the file;
  - Chaos: one item per sequence (see `uosa_chaos.md`, "Draw order").
- **`NiSortAdjustNode::OnVisible`** 0x006e3130 (vtable 0x00cccbe0 +0x80, `NiSortAdjustNode.cpp:33-43`):
  with mode (+0xe0) `SORTING_OFF` it sets the no-sort bit on its subtree.

## NiStandardMaterial texture combine

The client renders effects with `NiStandardMaterial` (vtable 0x00ccbd88, found through its NiRTTI 0x00ddf704;
see "Effect camera and lighting"), not with the fixed-function `NiD3DDefaultShader`. **Confirmed** on the
binary: the constructor 0x006b6980 leaves `m_bSaturateShading` and `m_bSaturateTextures` (+0x3bc..+0x3be)
false (`NiStandardMaterial.cpp:40-48`), and the map handlers sit at the 2.6 slots:

| Slot | Function | SDK |
|---|---|---|
| +0xc8 | `HandlePostLightTextureApplication` 0x006c1600 | `NiStandardMaterial.cpp:5099` |
| +0xd0 | `HandleParallaxMap` 0x006beb90 | `:3996` |
| +0xd4 | `HandleBaseMap` 0x006bede0 | `:4036` |
| +0xd8 | `HandleNormalMap` 0x006bee70 | `:4065` |
| +0xdc | `HandleDarkMap` 0x006bf400 | `:4125` |
| +0xe0 | `HandleDetailMap` 0x006bf990 | `:4296` |
| +0xe4 | `HandleGlossMap` 0x006bfa60 | `:4308` |
| +0xe8 | `HandleBumpMap` 0x006bfaa0 | `:4317` |
| +0xec | `HandleGlowMap` 0x006bfc60 | `:4344` |
| +0xf0 | `HandleCustomMaps` 0x006bfca0 | `:4354` |
| +0xf4 | `HandleEnvMap` 0x006bfcb0 | `:4374` |
| +0xf8 | `HandleDecalMap` 0x006bffb0 | `:4483` |
| +0xfc | `HandleProjLightMap` 0x006bfff0 | `:4494` |
| +0x100 | `HandleProjShadowMap` 0x006c01d0 | `:4533` |
| +0x104 | `HandleLighting` 0x006c1b30 | `:5245` |

With no light reaching the effect (every effect NIF but `00005540_shadowstrike.nif`), the pixel shader the
material generates computes (`HandlePreLightTextureApplication` `:4891`, `HandleColorAccumulation` `:3224`,
`InsertTexture` `:4135`):

```text
Kd.rgb = emissive source                       // material emissive, or the vertex colour with SRC_EMISSIVE
Kd.a   = vertex alpha                           // vertex colours present and (E, SRC_EMISSIVE) or (E_A_D, SRC_AMB_DIFF)
       | material alpha                         // otherwise
tex    = dark.rgb                               // dark map first: it multiplies the later maps
tex   *= base.rgb                               // base map (MULTIPLY); opacity *= base.a
tex   *= detail.rgb * 2                         // detail map; alpha untouched
tex    = lerp(tex, decal.rgb, decal.a)          // each decal map, before the material colour; alpha untouched
out.rgb = Kd.rgb * tex + env + glow.rgb         // env from an NiTextureEffect, glow added in the same pass
out.a   = saturate(Kd.a * base.a)               // only the final alpha is clamped; colour is clamped by the target
```

`APPLY_REPLACE`: `out.rgb = tex (+ glow)`, `out.a = base.a`. Maps that are absent drop out (a dark map with no
base map multiplies only the material colour). No stage is saturated before the end; the colour is clamped
only by the render target.

Differences from `lib/ec-effects/particle-sim/src/render.rs` (as reviewed in the visual report):

1. The decal lerp is applied **before** the `Kd` multiply.
2. The detail map never contributes alpha, also when there is no base map.
3. No intermediate saturation.
4. Env maps: `00005036_magicreflection.nif`, `splat_blue_sizeb.nif` and `splat_blue_sizec.nif` carry an
   `NiTextureEffect` environment map that is added to their colour.
5. The glow map is one additive term in the same pass, not a second draw.

**"No lights gives black" re-verified.** With `E_A_D` lighting and no emissive source the result is
`0 * tex + glow`, so these drawables show only their glow map, or nothing visible with additive blending /
black with alpha blending. Survey of the shipped NIFs (`effects-re3/survey2.txt`): 207 drawables have a black
emissive source; 174 of them have a glow map and show only the glow, 33 have neither and draw black (11
opaque meshes such as `splat_blue_sizeb/c.nif`, `00009970_airelemental.nif`, `10000790_sandvortex.nif`; the
rest blended, e.g. `00007193_oilflask.nif`, `000273_fetid_essence.nif`). Of the blended ones, One/One and
SrcAlpha/One add nothing; SrcAlpha/InvSrcAlpha darkens what is behind. No drawable uses `E` +
`SRC_EMISSIVE` without vertex colours.

## Destination alpha

**Confirmed.** `NiDX9Renderer::InitializePresentParams` 0x007565b0 receives `FBFMT_UNKNOWN` from the UO
set-up and replaces it with the nearest display-mode format (`NiDX9DeviceDesc::GetNearestFrameBufferFormat`):
X8R8G8B8 on any 32-bit desktop. So in the world pass **DestAlpha reads 1** (`D3DBLEND_DESTALPHA` = 1,
`INVDESTALPHA` = 0). The hued effects render into an `NiRenderedTexture` (`Create` 0x0067f480, vtable
0x00cbff70) of 512 × 512 A8R8G8B8 cleared to (0, 0, 0, 0), where the destination alpha is the alpha
accumulated by the effect's own draws. No separate alpha blend state or colour write mask is set, so alpha
blends with the same factors as colour.

## Effect time origin

**Confirmed.** `UOParticleEffectNif` (vtable 0x00c9e0cc):

- `Start` 0x0041ab40 sets elapsed (+0x14) = 0, active (+0x24) and started (+0x2c).
- `Init` 0x0041a8e0 runs `Update(0)` on the scene first, so every controller's first `ComputeScaledTime`
  sees t = 0 (`NiTimeController.cpp:92-175`).
- `Update` 0x0041ab90: elapsed += dt; countdown (+0x40) -= 1. When not looping (+0x25) and duration (+0x28)
  <= elapsed, it stops without a scene update. Otherwise, unless hidden: when looping with a duration,
  elapsed = fmod(elapsed, duration); then `NiAVObject::Update(elapsed)` (vtable +0x64) and vtable +0x98.
- Controllers use `ComputeScaledTime` with the APP_TIME delta, so the scaled time telescopes to
  `elapsed × frequency + phase`; when elapsed wraps, the delta is negative and the scaled time goes back.
  A looping controller [-0.0667, 3.0] in an effect of duration 3.0 never reaches [3.0, 3.0667).
- Hued effects advance elapsed by dt × 0.5.

`lib/ec-effects/particle-sim` (`keys.rs` `ComputeScaledTime`, `instance.rs` elapsed %= duration) matches.

## Unreferenced marker NIFs

**Probable (high).** The eight hash-only NIFs of `EffectTexture.uop` (`0bb3e38a43964673`, `11b3e30c743b0256`,
`8666a645e0b6553a`, `9c97240a70eabd8e`, `e507689dcaa26e64`, `f46baaa128fd8807`, `f82d4cca4638f957`,
`fb998db1b3677a30`) have no name in the string dictionary, and nothing names them: no definition in
`EffectDefinitionCollection.uop` or `tileart.uop` (a named reference would resolve the hash), no string in
`UOSA.exe` (its only `.nif` strings are Rain/Snow, `LabelGeometry.nif` and the fixtures/regions formats), no
UI XML. Waypoints reach effects only through effect ids (`UOWaypoints::SetWaypointEffect` 0x005d29e0). The
client never plays them, so their emitter set-up is moot; their textures `03000017_star.nif` and
`03000018_circle_g.nif` are otherwise unreferenced.

## Animation frame effects

Frame effects of `AnimationSequence.uop` actions are played on the animated object by
`UOAnimationFrameSet::PlayFrameEffects` 0x0083390f through `IUOEffectDefinition::PlayOnObject` 0x0041a610
(the +4 slot of every definition vtable; the interface slot is `_purecall`). PlayOnObject requires the
object's effects component (id 4), instantiates and registers the definition with the object (so `hued`
particles take the object's hue), adds it with `UOObjectComponentEffects::AddEffect` 0x00443e90, sets its
position to the object's tile position and starts it. When frames fire is in `uosa_animation.md`, "Frame
effects and skipped frames".

## Rust parser specification

This section describes the bytes to read; the field meanings are in the tables above.

```
TileartEffects   := u8 count, count x EffectDefinition      // last field of a version-4 tileart record
CollectionEntry  := u32 count, count x EffectDefinition      // whole payload of an EffectDefinitionCollection.uop entry

EffectDefinition := u32 type_key, Body(type_key)
EffectList       := u32 count, count x EffectDefinition
EffectMap        := u32 count, count x { u32 key, EffectList }

Body(0  Particle)       := u32 name_string_index, f32 offset_x, f32 offset_y, f32 camera_offset, f32 scale,
                           u8 attach_mode, f32 duration, u8 loop, u8 field_3c, u8 hued         // 28 bytes
Body(1  Sound)          := u32 sound_id, f32 value_08                                          // 8 bytes
Body(2  RoundRobin)     := EffectList
Body(7  PositionOffset) := 8 x { i8 x, i8 y }                                                  // 16 bytes, index = direction 0..7
Body(10 Wait)           := EffectList children, u32 min_delay_ms, u32 max_delay_ms, u8 aligned
Body(11 LOD)            := EffectMap levels
Body(12 PropertyFlag)   := u8 mode, u64 mask, u32 property, EffectList default_children, EffectMap cases
Body(15 Earthquake)     := f64 amplitude, f64 duration, f64 frequency                          // 24 bytes
Body(16 Random)         := u32 count, count x { f32 weight, EffectList }
Body(17 UserPreference) := u32 mask, EffectList children
```

Suggested Rust representation: a `enum EffectDefinition` with one variant per key, and
`Vec<EffectDefinition>` for lists. Store maps as `Vec<(u32, Vec<EffectDefinition>)>` in file order. The
client uses a `std::map`, so lookups take the last entry for a key, and iteration is in ascending key order.

Validation, in terms of what the client does and what a parser should do:

- **Unknown type key.** Any key other than 0, 1, 2, 7, 10, 11, 12, 15, 16 or 17 makes the client clear the
  parse context's ok flag and return a null definition. It reads no body, so the rest of the stream is
  meaningless. Return an error carrying the key and the offset.
- **Wait aligned and PropertyFlag mode.** These are bytes that must be 0 or 1. The client stores
  `value == 1` and clears the ok flag for other values, but keeps reading. Return an error, or warn and
  store `value == 1`. The data never violates this.
- **Particle flags.** loop and hued are booleans meaning non-zero. attach_mode and field_3c are raw bytes;
  keep them as u8. The client doesn't check them when parsing (field_3c is checked when the effect starts).
- **Particle name.** This is a `string_dictionary.uop` index, not an inline string. Resolve it later with the
  repo's string dictionary. All indices in the data are in range.
- **Counts.** Counts aren't bounded by the client. Before allocating, check each count against the bytes
  left; the smallest definition is 4 + 8 = 12 bytes for Sound, and a list or map element is at least 4
  bytes.
- **Recursion.** The client doesn't limit recursion. The data nests at most 1 level deep. Use a depth limit
  (for example a named constant of 16) to reject corrupt input.
- **End of payload.** No version field or trailing padding exists. In tileart, the effects end the record
  exactly. A collection entry ends exactly after its definitions. Treat leftover bytes as an error, as the
  verifier does.
- **f64 and f32 values.** These are read raw, with no range checks.

## Names and types applied in Ghidra

`NameEffectCode.java --apply` was run on this build:

- **Functions renamed:** 116 DEFAULT functions, 28 of them marked probable. All have source ANALYSIS, the
  tag `UO_EFFECTS` and a plate comment.
- **Already named:** `UOEffects::ParseEffectDefinition` already had that name and was kept.
- **Global:** `g_UOEffectDefinitionFactories` was already labelled.
- **Types:** 19 structure types created in `/UOEffects`:
  - `MsvcString`, `MsvcVector`, `MsvcMap`;
  - the ten definition classes;
  - `UORandomEffectEntry`;
  - `BinaryUOEffectDefinitionCollection`;
  - the packed wire records `EffectParticleRecord` (0x1c), `EffectSoundRecord` (0x8),
    `EffectPositionOffsetRecord` (0x10) and `EffectEarthquakeRecord` (0x18).
- **Idempotence:** a second `--apply` reported `Changes: none`.

The named groups are:

| Group | Functions |
|---|---|
| `UOEffects` | ParseEffectDefinition (kept), ParseEffectList, ParseEffectMap, ParseEffectDefinitionCollection |
| `<X>BinaryBaseFactory` | CanHandle, Create (shared with the derived factory) |
| `<X>BinaryFactory` | NewInstance, Init, ScalarDeletingDestructor |
| definition classes | ScalarDeletingDestructor, Instantiate, GetType |
| runtime effects | `UOParticleEffectNif`/`Chaos`/`UOHuedParticleEffectNif`, `UOSoundEffect`, `UOPositionOffset`, `UOEarthquakeEffect`: Init, Update, Start/Stop/IsFinished, Get/SetPosition where they are unique |
| collection | `UOEffectDefinitionCollectionBinaryFactory::Create`, `BinaryUOEffectDefinitionCollection::{GetMemorySize, ScalarDeletingDestructor, GetCount, GetDefinition, AddDefinition (probable)}` |

Some slots were left unnamed because of shared code. These are `UOParticleEffectNif` +0x0/+0x4/+0x10/+0x1c/+0x20
(0x0041ab40, 0x0041ab60, 0x0041ab70, 0x0041ad90, 0x0041ade0). Each is referenced by a second vtable.

The following are not renamed:

- the registration function `FUN_004af590`;
- the other readers `FUN_0083c0ba` and `FUN_004b93d0`.

### Round 3 (unknown fields)

`NameEffectCode.java` now renames the `UOParticleEffectDefinition` +0x30 field and the packed record field to
`attach_mode` (constant `PARTICLE_ATTACH_PROJECTILE_ONLY = 2`), documents `field_3c` and the sound
`value_08`, and refreshes its own single-line plate comments when the evidence changed (comments edited by
hand are kept). `--apply` updated 3 types and refreshed 2 plate comments
(`UOParticleEffectBinaryBaseFactory::Create`, `UOSoundEffectDefinition::Instantiate`); a second `--apply`
reported no changes. `UOObject::CreateComponents` (named by `NameTileartCode.java`) now mentions the attach
mode test in its plate comment.

### Round 4 (run-time use)

`NameEffectCode.java` step 7 (`runtimeUse`) names the effect-id lookup, the active-effect list and the
graphical-effect packet chain from locators (no fixed addresses):

| Evidence | Finds |
|---|---|
| only user of the string `Missing Effect Definition %d\n` | `UOEffects::InstantiateEffectById` |
| callee of the lookup whose single callee makes the Instantiate virtual call (+8) | `UOEffects::InstantiateAndRegisterByValue`, `UOEffects::InstantiateAndRegister` |
| address taken by InstantiateAndRegister and used by exactly two other functions, one with an Update (+0x14) virtual call | `g_UOActiveEffects`, `UOEffects::UpdateActiveEffects`, `UOEffects::ReleaseActiveEffects` |
| callers of the lookup: scalar 10001 / callee of `UOProjectileEffect` +0x14 / virtual call +0x30 / the remaining one | `UOEffects::ShowTargetIndicator`, `UOProjectileEffect::SpawnExplosion`, `UOEffects::PlayEffectAtTile`, `UOWaypoints::SetWaypointEffect` (probable) |
| single caller of PlayEffectAtTile | `UOEffects::PlayGraphicalEffect` |
| dispatcher callees whose first instruction reads a global the dispatcher also reads / that write the `UOProjectileEffectDefinition` / `UOTileEffectDefinition` vftables | `UOEffects::IsParticleFiltered` + `g_UOParticleFilter`, `UOEffects::MakeProjectileEffect`, `UOEffects::MakeTileEffect` |
| the three callers of the dispatcher, ordered by the largest packet field offset they read (0x1e, 0x24, 0x36) | `UOEffects::PlayGraphicalEffect_0x70` / `_0xC0` / `_0xC7` |
| single caller of each copier; the call before the copier in it | `UOObjectManager::Receive_0x70_GraphicalEffect` / `Receive_0xC0_HuedEffect` / `Receive_0xC7_ParticleEffect`; `UOPacket::Read_0x70_GraphicalEffect` / `Read_0xC0_HuedEffect` / `Read_0xC7_ParticleEffect` |
| the single global read by the LOD / UserPreference Instantiate | `g_UOParticleLODLevel`, `g_UOUserEffectFilter` |

New vtable slots: `UOEffectContainer` (Start, Stop, SetVisible (probable), IsFinished, Update, Release,
SetPosition, GetKind (probable), ScalarDeletingDestructor (probable), Init (probable)),
`UOEffectContainerDefinition::Instantiate`, `UOEffectCollectionDefinition::Instantiate`,
`UOParticleEffectChaos` (Start, Stop, IsFinished (probable), Release (probable), SetLifetimeTicks),
`UOProjectileEffect` (Update, Init; probable), `UOTileEffect` (IsFinished (probable), Update, Init),
`UOObjectComponentEffects::HandleNotify` (probable). `UOParticleEffectNif::SetLifetimeTicks` 0x0041af00 is
shared with a second vtable and stays unnamed. The plate comments of the RoundRobin, LOD, Random and
Chaos Update functions were refreshed (the old Chaos Update text called +0x25 the loop flag; it is the
stopping flag).

`virtualCallOffsets` now also recognises `MOV reg2, [vtable + disp]; CALL reg2`.

`--apply` renamed 45 functions (tags and plate comments set), created 4 labels and 2 namespaces
(`UOWaypoints`, `UOPacket`), and refreshed 4 plate comments, with no anomalies. A second `--apply`
reported `Changes: none`. The program was saved.

### Round 5 (hued effects, object playback)

`NameEffectCode.java` step `huedEffects` (after `runtimeUse`) names from locators:

| Evidence | Finds |
|---|---|
| RTTI vtables | `UOObject` slots +0x54/+0x58 `SetHue`/`GetHue`, +0x138/+0x13c `Set`/`GetRenderMode`, +0x140/+0x144 `Set`/`GetSuppressTileartEffects` |
| +4 slot shared by every definition vtable (`IUOEffectDefinition` +4 is `_purecall`) | `IUOEffectDefinition::PlayOnObject` 0x0041a610 |
| callee of PlayOnObject also called by `UOTileEffect::Init`, making the +8 virtual call | `UOObjectComponentEffects::AddEffect` 0x00443e90 |
| writer of the `UOHuedParticleEffectNif` vftable among the callees of `UOParticleEffectDefinition::Instantiate` | `UOHuedParticleEffectNif::UOHuedParticleEffectNif` 0x0041d9c0 |
| Init callee using `Build\Hues\Hues.dds` / calling `SetViewFrustum` / called before the quad | `BuildHueQuad` 0x0041df40, `CreateOffscreenCamera` 0x00472160, `PlaceOffscreenCamera` 0x004722a0 |
| Update callee calling `NiDrawVisibleArray`; vtable +0x14 | `UOHuedParticleEffectNif::RenderToTexture` 0x0041ebc0, `::Update` 0x0041edc0 |
| single global read by the hued constructor and Init; string `UOWorldViewer::Init()` | `g_UOWorldViewer` 0x00e37224, `UOWorldViewer::Init` 0x0046fa70, `CreateCamera` 0x00471580, `SetCameraFrustum` 0x004712f0 |
| vtable writers that use the string `UOEffectsShader` | `UOEffectsShaderHLSL` / `UOEffectsShaderFFP` constructors, `UOEffectsShaderHLSL::UpdatePipeline` (probable) |

Types: `UOProjectileEffectDefinition` (0x40) and `UOTileEffectDefinition` (0x24) created in `/UOEffects`;
`UOParticleEffectDefinition.hued` comment updated. `effects_variables.spec` adds the hued constructor
(ESI effect, EDI object), Update, RenderToTexture and PlayOnObject signatures. `--apply` renamed 20
functions, created 1 label and 2 types and updated 1; `ApplyVariableSpecs` made 4 changes; second runs
reported no changes. The animation side (`NameAnimationCode.java` `frameEffects`) is listed in
`uosa_animation.md`. The program was saved.

### Round 6 (effect rendering, texture loading)

`NameEffectCode.java` step `effectRendering` (after `huedEffects`) names from locators:

| Evidence | Finds |
|---|---|
| table `UOParticleEffectNif::Release` indexes as `[reg*4 + table]` whose entry 7 the hued Init reads | `g_UOEffectRenderGroups` 0x00e32e28 |
| vtable holding `NiDX9Renderer::GetRTTI` (+8); +0x58 and +0x60 checked against the `RenderToTexture` virtual calls | `NiDX9Renderer::SetBackgroundColor` 0x00761580 (NiColorA) and 0x00761520 (NiColor), `GetBackgroundColor` 0x007614b0, `UseLegacyPipelineAsDefaultMaterial` 0x0075f800 (probable) |
| single caller of `NiDX9SourceTextureData::CreateSurfFromRendererData`; the one byte global it compares | `NiDX9SourceTextureData::Create` 0x00770b90, `NiTexture_ms_bMipmapByDefault` 0x00e31ea3 |
| vtable written by `~NiDevImageConverter` that holds `GenerateMipmapLevels`; its other writer; slots +0xc, +0x14 | `NiDevImageConverter::NiDevImageConverter` 0x00693cf0, `ReadImageFile` 0x006942d0, `ConvertPixelData` 0x00694700 |
| reader vtable written by that constructor whose +4 pushes `".nif"` | `NiNIFImageReader` vtable 0x00ccc1d8: `ScalarDeletingDestructor` 0x006ccd40 (probable), `CanReadFile` 0x006ccde0, `ReadFile` 0x006cce00, `ReadHeader` 0x006cceb0 |

Slot functions Ghidra had not created (0x00761580, 0x00761520, 0x007614b0, 0x006942d0, 0x006ccde0,
0x006cceb0) are created by `--apply`. `effects_variables.spec` adds the `GetBackgroundColor` and
`NiNIFImageReader::ReadFile` prototypes. `--apply` created 6 functions, renamed 12 (tags and plate comments
set), created 2 labels and 1 namespace; `ApplyVariableSpecs` set 2 prototypes; second runs of both reported
no changes. `ApplyVariableSpecs` notes the two `SetBackgroundColor` overloads (same name, as in Gamebryo).
The program was saved.

Identified but not renamed (Gamebryo functions whose BSim match is ambiguous, or helpers):
`NiDynamicEffect::AttachAffectedNode` 0x006a1900 (BSim proposes `AttachUnaffectedNode`; the call from
`NiNode::AttachEffect` 0x006768c0 settles it), `NiMatrix3::operator*` 0x00668f10, the renderer set-up
`FUN_00754fb0`, the render-target begin/end wrappers `FUN_00416030` / `FUN_004160d0`, the render group
builder `FUN_0044e4d0` and drawer `FUN_00470430` (named `DrawWorldRenderGroups` in round 7).

### Round 7 (draw order, materials, render targets, Chaos resources)

`NameEffectCode.java` step `drawOrderAndMaterials` (after the Chaos step). Gamebryo classes have no MSVC
RTTI, so their vtables come from NiRTTI: the class name string, the static `NiRTTI` holding it, the small
`GetRTTI` returning that object, and the vtable slot +8 holding `GetRTTI` (`niRttiVtable`).

| Evidence | Finds |
|---|---|
| caller of `NiDrawVisibleArray` reading render groups 7 and 9 | `DrawWorldRenderGroups` 0x00470430 (probable) |
| user of `"Data/Effects"` reading render group 7 | `UOEffects::InitParticleSystemRoot` 0x004216b0 (probable) |
| `NiAlphaAccumulator` vtable writer called by `CreateObject`; its UO caller | `NiAlphaAccumulator::NiAlphaAccumulator` 0x00698f70, `UOGraphics::CreateRendererAndSorter` 0x0058a950 (probable) |
| NiRTTI vtables: `NiSortAdjustNode` +0x80, `NiPSysBoundUpdateModifier` +0x48; MSVC RTTI `UOShaderOverrideSorter` +0x50 | `NiSortAdjustNode::OnVisible` 0x006e3130, `NiPSysBoundUpdateModifier::Update` 0x00815930, `UOShaderOverrideSorter::RegisterObjectArray` 0x00455ed0 |
| NiRTTI vtable `NiStandardMaterial` +0xd0..+0x100 | the 13 handlers in "NiStandardMaterial texture combine" |
| `NiRenderedTexture` vtable writer called by `BuildHueQuad` | `NiRenderedTexture::Create` 0x0067f480 |
| common caller of `GetNearestFrameBufferFormat` and `GetMultiSampleType` | `NiDX9Renderer::InitializePresentParams` 0x007565b0 |
| `CreateSequenceRenderer` callees: the one calling an `NiSourceTexture` vtable writer that calls `CreateRendererData`; the recursive one; the one with a single caller making a +0x64 call | `ParticleSystemManager::LoadTexture` 0x008a8630, `NiSourceTexture::Create` 0x0068a8b0, `ParticleSet::UnshareAnimatedGeometry` 0x008b2820, `ParticleSystemManager::LoadMesh` 0x008a8440 |
| MSVC RTTI `ParticleBillboard` +0x14, `MeshParticle` +0x4 | `ParticleBillboard::SetBound` 0x008c6300, `MeshParticle::SetActiveCount` 0x008c6520 |
| address-taken global in `LoadTexture` holding {6, 3, 2}; the byte it reads with MOVZX | `NiSourceTexture_ms_kDefaultPrefs` 0x00dde184, `g_UOTextureLoadDirectHint` 0x00e31e9b |

`--apply` created 7 functions (0x006e3130 and six `NiStandardMaterial` handlers), renamed 28 (tags and plate
comments set), created 2 labels and 1 namespace; `ApplyVariableSpecs` set the `SetBound` and
`SetActiveCount` prototypes; second runs of both reported no changes. The program was saved.

### Round 8 (texture filtering)

`NameEffectCode.java` step `textureFiltering` (after `drawOrderAndMaterials`; see "Texture filtering and
anisotropy").

| Evidence | Finds |
|---|---|
| user of `"Creation failed: Device does not support single-pass multi-texture"` | `NiDX9Renderer::InitializeDeviceCaps` 0x00756990 |
| the one run of 6 × 3 + 4 × 2 dwords that function writes | `NiDX9Renderer_ms_akD3DFilterMapping` 0x00e16648, `NiDX9Renderer_ms_akD3DAddressMapping` 0x00e16690 |
| other function reading the filter table's words 0 and 2 / the address table's words 0 and 1 | `NiD3DTextureStage::SetFilterMode` 0x0074d530, `NiD3DTextureStage::SetClampMode` 0x0074d4e0 |
| the data holding the 13 `{state, value}` pairs ADDRESSU..DMAPOFFSET with MAXANISOTROPY 1, then {-1, -1} | `NiDX9RenderState_ms_akInitialSamplerStateSettings` 0x00da8690 |

`--apply` renamed 3 functions and created 3 labels (tags and comments set); the program was saved.

## Proposed Rust changes

Not applied. The changes are for `lib/uocf/src/enhanced/effect_definition.rs` as of 914634cf, plus one
line in `ENHANCED_CLIENT_FILES.md`. The earlier items (attach_mode, field_3c, value_08) are already in.

1. **Effect id constants** (module level, documented as client behaviour):

   ```rust
   /// Collection id = 0xC7 packet effect / explode effect field - EFFECT_ID_PACKET_BIAS.
   pub const EFFECT_ID_PACKET_BIAS: u32 = 1;
   /// Types 2/3 packet effect value that plays nothing (not special as an explode effect).
   pub const PACKET_EFFECT_NONE: u16 = 9501;
   /// Types 2/3: plays the classic item effect without its tileart effects (0 plays it with them);
   /// type 0: the projectile item without its tileart effects. Not special as an explode effect.
   pub const PACKET_EFFECT_CLASSIC: u16 = 9502;
   /// Explode effect 0: classic explosion item, played for this many ticks.
   pub const EXPLOSION_ITEM_ID: u16 = 0x36CB;
   pub const EXPLOSION_TICKS: u32 = 10;
   /// Collection id substituted for a missing id (Placeholder01.ems).
   pub const EFFECT_ID_FALLBACK: u32 = 0;
   /// Collection id of packet type 1 (lightning).
   pub const EFFECT_ID_LIGHTNING: u32 = 666;
   /// Collection id of the current-target indicator.
   pub const EFFECT_ID_TARGET_INDICATOR: u32 = 10001;
   ```

   Optionally add `pub fn collection_id_from_packet_effect(effect: u16) -> Option<u32>` for types 2/3. It
   returns `None` for 0, `PACKET_EFFECT_NONE` and `PACKET_EFFECT_CLASSIC`, and `effect -
   EFFECT_ID_PACKET_BIAS` otherwise; the explode effect maps every non-zero value.
2. **`parse_effect_definition_collection` doc:** "All top-level definitions play at the same time (one
   container); there is no sequencing. A missing entry falls back to `EFFECT_ID_FALLBACK`."
3. **`RandomEffect` / `RandomEffectEntry::weight`:** `pub const RANDOM_WEIGHT_TOTAL: f32 = 100.0;`. Weight
   doc: "Percent (confirmed): the client draws r in [0, RANDOM_WEIGHT_TOTAL) and plays the first entry with
   r < weight after subtracting the previous weights. A sum below RANDOM_WEIGHT_TOTAL leaves a chance of
   nothing." Remove "(probable)" from the struct doc.
4. **`RoundRobinEffect` doc:** confirmed. Add "The cycle index belongs to the definition: it is shared by
   every object of the same graphic and every play of the same collection entry."
5. **`WaitEffect` doc:** replace "fires its children on a timer" with "Throttle (confirmed): when
   instantiated, plays its children only if the per-definition next time has passed. It then sets the next
   time to now + `min_delay_ms` (next multiple of `min_delay_ms` when `aligned`) + uniform [0, max - min]
   ms. It never fires again by itself." `aligned` becomes confirmed.
6. **`LodEffect` doc:** "Plays the entry for the particle LOD setting (0..=`PARTICLE_LOD_MAX`, client
   default `PARTICLE_LOD_DEFAULT`; UI Low/Medium/High = 1..3), falling back to the next lower key; nothing
   if no key is <= the setting (confirmed)." Add `pub const PARTICLE_LOD_MAX: u32 = 3;` and
   `pub const PARTICLE_LOD_DEFAULT: u32 = 3;`.
7. **`UserPreferenceEffect` doc:** "(confirmed)". Note that the client only ever sets bit 0, so the other
   bits are never set by the client.
8. **`PositionOffsetEffect` doc:** add "Played as an effect it does nothing in this client build; the
   client reads the definition from a mount's sequence frame effects to place the rider."
9. **`ParticleEffect::duration` doc:** add "Independent of the packet duration, which the client applies as
   an update-tick countdown (single-definition entries only)."
10. **`ParticleEffect::name_index` doc:** add `pub const PARTICLE_RESOURCE_DIR: &str = "data/effects/";`,
    with the doc "the file is the `EffectTexture.uop` entry `PARTICLE_RESOURCE_DIR` + lowercase name;
    `.ems` files are Chaos text scripts naming further `.nif`/`.tga` files there."
11. **`ENHANCED_CLIENT_FILES.md`:** under `EffectDefinitionCollection.uop`, add one bullet: "Entry id =
    0xC7 packet effect - 1; all top-level definitions play together; particle files are in
    `EffectTexture.uop` under `data/effects/`."

12. **Hued particles** (for the effect renderer): `HUED_EFFECT_TIME_SCALE: f32 = 0.5`,
    `HUE_LUMINANCE_WEIGHTS: [f32; 3] = [0.2125, 0.7154, 0.0721]`, `HUED_EFFECT_ATLAS_COLUMN_STRIDE: f32 =
    255.0`, `HUE_ATLAS_ROWS: u32 = 1024`, and the rule "only `.nif` particles, only with an object hue
    != 0; output colour = hue gradient[luminance], alpha unchanged, alpha-blended". Whether to copy the
    255-stride and texel-edge quirks or sample the intended entry is a renderer decision. (Now in
    `lib/ec-effects/particle-sim/src/hued.rs`; the 1024 atlas width it and `lib/uocf` assume is confirmed,
    see "Hue atlas".)
13. **Packet hue:** the classic effect object hue is `packet hue + 1` (0 stays 0).

14. **Effect renderer (`lib/ec-effects/particle-sim`)**, from "Draw order", "NiStandardMaterial texture combine" and
    "Destination alpha":
    - combine as `Kd.rgb × (dark × base × 2·detail, then decal lerp) + env + glow`, alpha
      `saturate(Kd.a × base.a)`; no intermediate saturation; detail never affects alpha;
    - draw every effect drawable of one frame in one list: unsorted ones (no blending, or no-sorter) first
      in scene order, then the blended ones far to near by world bound centre · view direction;
    - DestAlpha = 1 in the world pass; the hued target accumulates alpha from (0, 0, 0, 0).

No format change is needed: the parser is already correct for all 269 entries.

## Uncertainties and open questions

- **Particle attach mode:** the difference between 0 (arrows) and 1 is not visible in the client.
  Collection particles are all 1, and attach_mode is not tested on the collection path.
- **field_3c:** what a non-zero value was meant for (1 is rejected by the Nif effect, any non-zero value by
  the Chaos effect). Always 0 in the data.
- **Sound value_08:** not read by the client (settled); its intended use (probably a gain) is unknown.
- **Particle offsets:** settled (tile offset like `PlayEffectAtTile`; see the Particle field table).
- **UserPreference bits:** bit 0 is footsteps (settled). The client sets no other bit, so other mask bits
  could only come from a hand-edited settings file.
- **Unnamed entries:** 2 collection entries have no recovered name (`firefield02.ems`,
  `poisonfield_rotated.ems`).
- **Ids with no caller:** no code path selects 70000-70006 (weather). The weather uses its own
  `UORainEffectDefinition` / `UOSnowEffectDefinition` classes, and packet ids stop at 65534. 10002-10004,
  18000-18006 and 9999/10000 have no constant caller either; they are reachable only as packet effects.
- **Waypoint table:** the source of the waypoint type → effect id table (`FUN_005d49d0`, `[this+0x34]`) was
  not traced; ids 20000-20011 are inferred from the names.
- **Double registration:** composite effects registered twice in `g_UOActiveEffects` (see "Runtime
  semantics"). This is from the code only; it was not observed at run time.
- **LOD key 0:** whether settings value 0 (below the UI's Low) is ever used; the data has no LOD
  definitions.
- **Missing resources:** the four particle names with no `EffectTexture.uop` entry (see "Particle resource
  resolution").
- **Other readers:** `FUN_004b93d0`/`FUN_0083c0ba` (readers of the format) were not decoded.
  `FUN_0041a610` is now `IUOEffectDefinition::PlayOnObject`.
- **Hue atlas sampling:** the map flags (BILERP + WRAP), the coordinate math, the absence of any hue
  mask and the atlas width (1024, shipped `Hues.dds` and hard-coded shader constants; see "Hue atlas") are
  confirmed. The resulting 50/50 texel-edge blend and the 255-texel column stride follow from them; a frame
  capture of a hued effect with a hue >= 1024 would show them.
- **Anisotropic filtering:** settled: no effect map asks for it and the renderer never raises
  `D3DSAMP_MAXANISOTROPY` above 1 (see "Texture filtering and anisotropy").
- **Hued quad, shader map flags, target clear colour:** settled (see "Hued NIF effects").
- **Lights inside effect NIFs** (**settled**): the client applies the two `NiDirectionalLight`s of
  `00005540_shadowstrike.nif` to that file's own meshes (the only shipped file with lights).
  `UOParticleEffectNif::Init` runs `Update(0)`, `UpdateProperties` (`FUN_00663f10`, call at 0x0041aa4a) and
  `NiAVObject::UpdateEffects` (`FUN_006641f0`, call at 0x0041aa53) on the scene root before attaching it to
  the render group, so the effect state (`NiNode` effect lists less each light's unaffected nodes:
  `NiNode::UpdateEffectsDownward` `FUN_00676480`, vtable +0x74; `PushLocalEffects` `FUN_00676120`) is built
  once there. `lib/ec-effects/particle-sim` compiles the lights (`NifLight`, NIF desc and codec version 2) and adds them,
  per vertex, to the meshes they reach; particles (no normals) get only ambient lights.
- **Draw order, material combine, destination alpha, time origin:** settled (see their sections). The
  sort is unstable, so objects with equal depth may swap between frames in the client as well.
- **Marker NIFs:** the eight hash-only NIFs are unreferenced (probable, high; see "Unreferenced marker
  NIFs").
- **World camera tilt:** settled (C = Z(-π/2) · X(0.001 - π/4) · Z(-0.05)).
- **Hued constructor depth bias:** object vtable +0x11c / +0x124 (scaled by -0.001 and -1e-5) were not
  identified.
