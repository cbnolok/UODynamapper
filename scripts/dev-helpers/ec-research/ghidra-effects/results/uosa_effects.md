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
| 2 | f32 | offset_x | +0x20 | added to the effect x | probable |
| 3 | f32 | offset_y | +0x24 | subtracted from the effect z, so it acts as a vertical offset | probable |
| 4 | f32 | camera_offset | +0x28 | scaled by k = -camera_offset * 0.12856 (0x00d050a0) and applied along the camera vector (`DAT_00e37224`+0x24..+0x2c) | probable |
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

The positional math is in `UOParticleEffectNif::Init` 0x0041a8e0. The axis names follow the code, not a
proven world orientation.

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
- **Server effect ids.** The collection id is the 0xC0/0xC7 packet effect field minus 1; see "Keying".
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
| `UOEffects::PlayEffectAtTile` | 0x00421130 | from `UOEffects::PlayGraphicalEffect` (packets 0x70/0xC0/0xC7): `effect - 1`, or 666 for lightning |
| `UOProjectileEffect::SpawnExplosion` | 0x0044ca90 | `explode effect - 1` of a moving effect (0xC7); 0 selects the classic tile effect 0x36CB for 10 ticks instead |
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
| 0 moving | `UOEffects::MakeProjectileEffect` 0x00420a90: a `UOProjectileEffectDefinition` (item id, hue + 1, speed, serials, explode flags, explode effect and sound, render mode). It has its own particle filter test. |
| 1 lightning | particle filter on the source serial, then `PlayEffectAtTile(666, ticks 0)` (`lightning.ems`) |
| 2 fixed location, 3 fixed on mobile | Gets the source object (type 3 without an object plays nothing), then tests `UOEffects::IsParticleFiltered(serial, location = type == 2)`. Effect 9501 (0x251D) plays nothing. Effect 0 or 9502 (0x251E) plays the classic item effect `UOEffects::MakeTileEffect` 0x00420e10, if the item id is non-zero. Any other effect plays `PlayEffectAtTile(effect - 1)`, with the packet duration as lifetime ticks for type 2 and 0 for type 3; type 3 then attaches the effect to the object's effects component (`FUN_00443e90`). |
| 4 | screen fade: a `UOFadeEffectDefinition` from the item id |

Consequences:

- **Packet effect value.** `effect - 1` is the collection id (verified by name: packet effect 5002 = id 5001
  `00005001_clumsy`, and the like). Ids reachable from packets are 0..65534.
- **Effect 0.** Packet effect 0 is not id -1: it means "no EC effect, use the item id".
- **9501 and 9502.** Packet 9501 and 9502 never reach ids 9500/9501 through this path. Collection 9501
  (`00009501_Null.nif`) and 9502 (`fizzle.ems`) are reached as packet values 9502 and 9503.
- **Hue and render mode.** `PlayEffectAtTile` passes object 0, so collection effects ignore the packet hue
  and render mode. A `hued` particle needs an object hue, so it is never hued on this path. PropertyFlag
  has no object to read either.
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
- **PositionOffset:** `UOPositionOffset` (0x50 bytes) holds the 8 offsets but nothing reads them. There is no
  dynamic_cast of its type descriptor (ProbeEffColl05), and it reports finished at once. It is inert in this
  build, and no data uses it.
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
| Object | the world object: hue, `hued` and PropertyFlag work | none for packet effects (no hue, no PropertyFlag); the target or waypoint object for the indicator and waypoints |
| attach_mode | `CreateComponents` skips particles with attach_mode 2 (`PARTICLE_ATTACH_PROJECTILE_ONLY`). `UOTileEffect::Init` and `UOProjectileEffect::Init` instantiate every particle. | not tested (all collection particles are 1) |
| Ownership | registered in `g_UOActiveEffects` and added to the object's `UOObjectComponentEffects` (component id 4, vftable 0x00c9fb44; `FUN_00443e90` AddEffect). The component sets visibility and position, follows the object (`HandleNotify` message 1) and stops everything when the component is cleared (+0x8); it never updates the effects. | registered in `g_UOActiveEffects`; type 3, the indicator and waypoints are also added to the object's component |
| Lifetime | the object's: started once; ends when the effect finishes or the component is cleared | the effect's own end, or the packet duration in ticks (type 2, single definition) |

So a classic packet effect (effect field 0, or 0x70) still gets Enhanced Client particles, through the
tileart effects of its item id. RoundRobin and Wait state in tileart effects advances per object creation
and is shared by all objects of the same graphic.

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

## Proposed Rust changes

Not applied. The changes are for `lib/uocf/src/enhanced/effect_definition.rs` as of 914634cf, plus one
line in `ENHANCED_CLIENT_FILES.md`. The earlier items (attach_mode, field_3c, value_08) are already in.

1. **Effect id constants** (module level, documented as client behaviour):

   ```rust
   /// Collection id = packet (0xC0/0xC7) effect field - EFFECT_ID_PACKET_BIAS.
   pub const EFFECT_ID_PACKET_BIAS: u32 = 1;
   /// Packet effect value that plays nothing.
   pub const PACKET_EFFECT_NONE: u16 = 9501;
   /// Packet effect values that play the classic item (tileart) effect instead of a collection entry.
   pub const PACKET_EFFECT_CLASSIC: u16 = 9502;   // 0 behaves the same
   /// Collection id substituted for a missing id (Placeholder01.ems).
   pub const EFFECT_ID_FALLBACK: u32 = 0;
   /// Collection id of packet type 1 (lightning).
   pub const EFFECT_ID_LIGHTNING: u32 = 666;
   /// Collection id of the current-target indicator.
   pub const EFFECT_ID_TARGET_INDICATOR: u32 = 10001;
   ```

   Optionally add `pub fn collection_id_from_packet_effect(effect: u16) -> Option<u32>`. It returns `None`
   for 0, `PACKET_EFFECT_NONE` and `PACKET_EFFECT_CLASSIC`, and `effect - EFFECT_ID_PACKET_BIAS`
   otherwise.
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
8. **`PositionOffsetEffect` doc:** add "Inert in this client build: the runtime object stores the offsets
   but nothing reads them."
9. **`ParticleEffect::duration` doc:** add "Independent of the packet duration, which the client applies as
   an update-tick countdown (single-definition entries only)."
10. **`ParticleEffect::name_index` doc:** add `pub const PARTICLE_RESOURCE_DIR: &str = "data/effects/";`,
    with the doc "the file is the `EffectTexture.uop` entry `PARTICLE_RESOURCE_DIR` + lowercase name;
    `.ems` files are Chaos text scripts naming further `.nif`/`.tga` files there."
11. **`ENHANCED_CLIENT_FILES.md`:** under `EffectDefinitionCollection.uop`, add one bullet: "Entry id =
    0xC0/0xC7 packet effect - 1; all top-level definitions play together; particle files are in
    `EffectTexture.uop` under `data/effects/`."

No format change is needed: the parser is already correct for all 269 entries.

## Uncertainties and open questions

- **Particle attach mode:** the difference between 0 (arrows) and 1 is not visible in the client.
  Collection particles are all 1, and attach_mode is not tested on the collection path.
- **field_3c:** what a non-zero value was meant for (1 is rejected by the Nif effect, any non-zero value by
  the Chaos effect). Always 0 in the data.
- **Sound value_08:** not read by the client (settled); its intended use (probably a gain) is unknown.
- **Particle offset axes:** the axis names are inferred from the Nif Init math, not from a rendered check.
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
- **Other readers:** `FUN_0041a610` and `FUN_004b93d0`/`FUN_0083c0ba` (other InstantiateAndRegister users
  and readers of the format) were not decoded.
