# UOSA.exe effect definitions

The Enhanced Client (`UOSA.exe`, Ghidra project `UOSA`) stores effect definitions in two places:

- at the end of every `tileart.uop` record: a u8 count, then that many definitions;
- in each entry of `EffectDefinitionCollection.uop`: a u32 count, then that many definitions.

One recursive reader decodes both: `UOEffects::ParseEffectDefinition`, through a table of type-keyed
factories. This document gives the format the client parses, what the fields mean, the verification
against the shipped data, and a parser specification for the repo.

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
| 6 | u8 | field_30 | +0x30 | stored as u32. No reader was found. Data values: 1, 2 and 0. | unknown |
| 7 | f32 | duration | +0x34 | seconds. If the effect doesn't loop, it stops once elapsed >= duration. If it loops and duration != 0, elapsed = fmod(elapsed, duration). | confirmed |
| 8 | u8 | loop | +0x38 | normalized with SETNZ, so any non-zero value means true | confirmed |
| 9 | u8 | field_3c | +0x3c | the Nif Init fails when the value is 1, and the Chaos Init requires 0. Always 0 in the data. | probable |
| 10 | u8 | hued | +0x40 | non-zero together with a non-zero object hue gives `UOHuedParticleEffectNif` | confirmed |

Particle bytes aren't validated when they are parsed.

`UOParticleEffectDefinition::Instantiate` 0x0040e490 chooses the runtime class:

- a name containing `.ems` gives `UOParticleEffectChaos`;
- otherwise, object hue != 0 and hued give `UOHuedParticleEffectNif`;
- otherwise, `UOParticleEffectNif`.

The positional math is in `UOParticleEffectNif::Init` 0x0041a8e0. The axis names follow the code, not a
proven world orientation.

### 1 Sound (body 8 bytes, Create 0x0083b590)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u32 | sound_id | +0x04 | `UOSoundEffect::Start` plays this id at the effect position, with distance attenuation, when sound is enabled | confirmed |
| 2 | f32 | value_08 | +0x08 | copied to `UOSoundEffect`+0x14. Start doesn't read it. 1.0 in the data. Possibly a volume. | unknown |

### 2 RoundRobin (Create 0x0083b7f0)

| # | Type | Name | Meaning | Confidence |
|---:|---|---|---|---|
| 1 | u32 | count | number of children | confirmed |
| 2 | count x definition | children | each child is appended through Init (vtable +0x18). Instantiate takes the child at index +0x14 and advances that index. | confirmed / probable |

The Create calls ParseEffectDefinition directly, but the bytes are the same as an effect list.

### 7 PositionOffset (body 16 bytes, Create 0x0083b8f9)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1..8 | 8 x {i8 x, i8 y} | offsets[direction] | x at +0x04+4d, y at +0x24+4d (sign-extended to i32) | per-direction (0..7) position offset copied into `UOPositionOffset` | confirmed |

### 10 Wait (Create 0x0083ba55)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | list | children | +0x04 | effects fired by the timer | confirmed |
| 2 | u32 | min_delay_ms | +0x14 | default 2000 | confirmed |
| 3 | u32 | max_delay_ms | +0x18 | default 4000 | confirmed |
| 4 | u8 | aligned | +0x1c | stored as `value == 1`. Any value other than 0 or 1 clears the context's ok byte. When set, the next fire time is aligned to a multiple of min_delay_ms. | confirmed / probable |

`Instantiate` 0x0040d9b0 uses QueryPerformanceCounter milliseconds. It fires when now >= next (+0x20), then
schedules the next time at a random point in [min, max].

### 11 LOD (Create 0x0083b9a6)

| # | Type | Name | Meaning | Confidence |
|---:|---|---|---|---|
| 1 | map | levels | LOD level -> children. Instantiate runs the entry for the global effect LOD level. | confirmed / probable |

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
| 2 | count x {f32 weight, list} | entries | Instantiate picks one entry, weighted by these values | confirmed / probable |

### 17 UserPreference (Create 0x0083bcc1)

| # | Type | Name | Stored at | Meaning | Confidence |
|---:|---|---|---|---|---|
| 1 | u32 | mask | +0x04 | the children are skipped when `(mask & user effect setting DAT_00e33584) == mask`, or when mask == 1 and object vt+0xa0 < 0 | probable |
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
  - field_30: 1 x550, 2 x19, 0 x3;
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
  - field_30: always 1;
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
- **Lookup by effect id.** `FUN_004220c0` (not renamed) looks up an effect id:
  - When the id is missing, it logs `Missing Effect Definition %d` and falls back to id 0.
  - A collection with one definition returns that definition.
  - Otherwise, all the definitions are wrapped in a `UOEffectContainerDefinition`.
- **Server effect ids.** The collection ids are therefore the server's effect ids.
- **Other readers.** Two other readers of the same format exist:
  - `FUN_0083c0ba`, a larger asset that embeds effect collections per entry;
  - `FUN_004b93d0`, which reads u32 id, u8 count and that many definitions.

  Neither was decoded further.

## Rust parser specification

This section describes the bytes to read; the field meanings are in the tables above.

```
TileartEffects   := u8 count, count x EffectDefinition      // last field of a version-4 tileart record
CollectionEntry  := u32 count, count x EffectDefinition      // whole payload of an EffectDefinitionCollection.uop entry

EffectDefinition := u32 type_key, Body(type_key)
EffectList       := u32 count, count x EffectDefinition
EffectMap        := u32 count, count x { u32 key, EffectList }

Body(0  Particle)       := u32 name_string_index, f32 offset_x, f32 offset_y, f32 camera_offset, f32 scale,
                           u8 field_30, f32 duration, u8 loop, u8 field_3c, u8 hued            // 28 bytes
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
- **Particle flags.** loop and hued are booleans meaning non-zero. field_30 and field_3c are raw bytes; keep
  them as u8. The client doesn't check them when parsing.
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
- the effect id lookup `FUN_004220c0`;
- the other readers `FUN_0083c0ba` and `FUN_004b93d0`.

## Uncertainties

- What field_30 means: no reader was found for definition +0x30 or for the runtime copy.
- Whether field_3c is a "Chaos only" flag. Its value is always 0 in the data.
- Sound value_08: it is stored but no reader was seen. A volume is plausible.
- The axis names of the particle offsets are inferred from the Nif Init math, not from a rendered check.
- UserPreference mask semantics: the bit meanings of `DAT_00e33584` (the user effect settings) weren't
  traced.
- The 2 collection entries whose names weren't recovered.
