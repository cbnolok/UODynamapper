# UOSA.exe animation sequences and definitions

The Enhanced Client (`UOSA.exe`, Ghidra project `UOSA`) drives animations from two asset files:

- `AnimationSequence.uop`: one entry per mobile body. It holds the action table (frame count, fps, alias),
  per-direction pixel offsets, per-frame effects and skipped frames, and the action groups that map
  movement, packet animation types and state transitions to actions.
- `AnimationDefinition.uop`: one entry per animated item (tile). It holds frame sets of tile ids with a
  frame interval. It has nothing to do with mobile bodies or body redirects.

The mobile pixels come from `AnimationFrame1..6.uop`, one entry per (body, action). This document gives
the layout of both files as the client parses them, the verification against the shipped data, the
run-time semantics, the relation to the frame files and packets, and the mismatches with the repo parsers.

Conventions:

- All values are little-endian and unaligned. The stream is read field by field through
  `UOAssets::StreamRead` 0x00a6fe55.
- Addresses are function entries in `UOSA.exe` SHA-256
  5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50.
- Confidence: **confirmed** means read in the assembly and checked against the data. **probable** means
  inferred from how the value is used. **unknown** means no reader was found or the use is unclear.
- Data figures come from `verify_animation.py` run on the shipped client files.

## Load path

Both files go through the generic asset manager (`g_UOAssetManager` 0x00e37220), the same path as the
other `build/` assets:

| Step | Function |
|---|---|
| Request `{id, type << 24}` (synchronous) | `UOAnimations::GetAnimationSequence` 0x00837479 (type 0x0c), `UOAnimations::GetAnimationDefinition` 0x00834253 (type 0x0b) |
| Request queued in a `UOResourceRequest` | `UOAnimations::RequestAnimationSequence` 0x004cb850, `UOAnimations::RequestAnimationDefinition` 0x004cb7e0 |
| Cache lookup, resolvers, blob load | `UOAssets::GetAsset` 0x00a6cfc8 -> `ResolveAsset` 0x00a6d346 -> `TryResolvers` 0x00a749ed -> `LoadAssetBlob` 0x00a6d38f -> `CreateAssetFromBlob` 0x00a72861 |
| Factory Create (vtable +4) | `UOAnimationSequenceBinaryFactory::Create` 0x0083bfb4, `UOAnimationDefinitionBinaryFactory::Create` 0x004b8bc0 |
| Parser | `UOAnimationSequence::Parse` 0x0083c0ba, `UOAnimationDefinition::Parse` 0x004b8d80 |

- Entry names: `build/animationsequence/%08d.bin` and `build/animationdefinition/%08d.bin`, hashed with
  Jenkins lookup3 `hashlittle2` after lowercasing (`UOP::HashFileName` 0x0098d487). **confirmed**: every
  entry of both files matches a name of that form; no other name form exists.
- `AnimationFrame` assets are type 0x10 with id `body * 0x100 + action`, named
  `build/animationframe/%06d/%02d.bin` (`UOAnimations::GetAnimationFrameSet` 0x004316f0). **confirmed**.
- A missing entry makes the request end with status 1. `BuildMobileAnimation` then aborts and the mobile has
  no sequence-driven animation. An item without an `AnimationDefinition` gets a static tile component
  (`UOProjectileEffect::LoadAnimationDefinition` 0x0044b8e0, `UOTileEffect::LoadAnimationDefinition`
  0x00466700). **confirmed**.
- Sequences are requested when a mobile's animation component initializes
  (`UOObjectComponentAnimationMob::Initialize` 0x004341c0 -> `UOAnimations::BuildMobileAnimation` 0x00433a40).
  The builder keeps a map keyed by body type (`DAT_00e38fcc`) holding the sequence and the built
  `UOAnimationDefinition`, so each body is parsed once. `UOAnimations::HasSeatedGroup` 0x005b4040 also
  requests the sequence of an unmounted mobile (the player) to test for group 6. **confirmed**.
- `cacheSize` (settings) is parsed but its effect on these assets was not traced. `AssetLoaderConfig.xml`
  is absent from the client folder.

## AnimationSequence.uop

### Layout

Parsed by `UOAnimationSequence::Parse` 0x0083c0ba into a `UOAnimationSequence` (0xc4 bytes, vftable
0x00cde930). Every one of the 370 entries is consumed to its last byte.

```
u32   stored_id                                    // see "Stored id"
8 x { i8 x, i8 y }                                 // sequence-level offsets, per direction 0..7
8 x { i16 x, i16 y }                               // sequence-level extra offsets, per direction
u32   action_count
action_count x {
    u32   action                                   // action id (0..~70)
    u32   frame_count                              // frames per direction
    i32   alias                                    // -1 = own frames; else action id to borrow
    f32   fps
    8 x { i8 x, i8 y }                             // per direction; -128 = unset
    8 x { i16 x, i16 y }                           // per direction; -0x8000 = unset
    u32   effect_frame_count
    effect_frame_count x {
        u32   frame                                // frame index within the direction
        u32   effect_count; effect_count x EffectDefinition   // same format as EffectDefinitionCollection
    }
    u32   skipped_count
    skipped_count x u32 frame                      // frames the player passes over
}
u32   group_count
group_count x {                                    // FrameSetLookup
    i8    key                                      // 0 war, 1 peace, 2 mounted, 3 flying, 4/5 state, 6 seated
    i8    parent                                   // -1 none; else key of the group searched next
    u32   default_action
    u32   n; n x { i8 key, u32 action }            // transition actions
    u32   n; n x { u8 move, u32 action, u32 alternate }   // 0 stand, 1 walk, 2 run
    u32   n; n x { i8 type, u8 sub, u32 m, m x u32 action }   // packet animation types (0xE2)
}
```

| Field | Stored at | Run-time use | Confidence |
|---|---|---|---|
| `stored_id` | +4 | body id under which the frames are stored (see below) | confirmed (data), probable (code) |
| header i8 pairs | object | all zero in the data; read by Parse; no reader found besides the copy | unknown |
| header i16 pairs | object | default for the action i16 pair when the action's is -0x8000 (`UpdateFrame` 0x00436d40); all zero in the data | probable |
| `action` | map key (+0xb8, `std::map<u32, record>`) | action id | confirmed |
| `frame_count` | record +0x20 | frames per direction; 0 in every alias record | confirmed |
| `alias` | record +0x24 | redirect to another action of the same body (`ResolveAction`) | confirmed |
| base frame | record +0x28 | not in the file: assigned by `NextBaseFrame` 0x00836d13 | confirmed |
| `fps` | record +0x2c | frame interval = 1 / fps; 0.0 in every alias record | confirmed |
| action i8 pairs | record +0x48 (x[8]), +0x68 (y[8]) | per-direction pixel offset of what rides on the body (rider on a mount); -128 -> 0 | probable |
| action i16 pairs | record +0x88 (x[8]), +0xa8 (y[8]) | extra per-direction offset added to the i8 one; -0x8000 -> header value | probable |
| effect frames | record +0x30 (`map<u32, collection>`) | effects started when the frame is shown (sounds in the data) | confirmed |
| skipped frames | record +0x3c (`map<u32, bool>`) | frame time is advanced past these frames (`Update` 0x00434c90) | confirmed (code), probable (index base) |
| group `key` / `parent` | group map +4 (`map<i8, FrameSetLookup>`), lookup +0x28 | group chain searched by every lookup | confirmed |
| `default_action` | lookup +0x2c | fallback of the movement and typed lookups | confirmed |
| transitions | lookup +4 (`map<i8, u32>`) | action played when the mobile changes state | probable |
| moves | lookup +0x10 (`map<u8, {action, alternate}>`) | stand / walk / run action of the group | confirmed |
| typed | lookup +0x1c (`map<{i8, u8}, vector<u32>>`) | action for a 0xE2 type/sub-type; also 0x6E 200..269 | confirmed |

Record handling in Parse:

- `alias == -1` -> `AddAction` 0x00836b80 stores frame count, fps and base frame. A duplicate action id is
  logged unless the existing record is empty.
- `alias != -1` -> `AddAliasAction` 0x0083695a stores only the alias. The alias target does not have to
  exist or be parsed yet.
- Effect frames go through `AddFrameEffects` 0x00837288; skipped frames through `AddSkippedFrame`
  0x008372e1. Both use `ActionAt` 0x00837b26 (map operator[]), so they work for alias records too.
- Groups are created with `GetOrCreateGroup` 0x00837167; `parent != -1` links `GetGroup(parent)`
  0x00837136.

### Base frames and directions

`NextBaseFrame` 0x00836d13 returns `max(1, base + 5 * frame_count)` over the records added so far, in file
order. Each plain action owns `5 * frame_count` consecutive global frame numbers, starting at 1. The 5
stored directions are 3..7 in UO direction order (0 north, 1 north-east, 2 east, 3 south-east, 4 south,
5 south-west, 6 west, 7 north-west).

`UOAnimationSequence::GetDirectionFrames` 0x00836d96 (confirmed at instruction level):

```
action    = ResolveAction(action)
if !HasAction(action): return 0
mirrored  = dir < 3
if mirrored: dir = 6 - dir                      // 0 -> 6, 1 -> 5, 2 -> 4
first     = base + (dir - 3) * frame_count
count     = frame_count
if !g_UOIdleAnimation and IsStandAction(action): count = 1
```

Directions 0, 1 and 2 are drawn as horizontal mirrors of 6, 5 and 4. The per-direction offsets are stored
for all 8 directions; the mirrored ones carry their own values (body 114 action 22: dir 2 = (-6, 0), dir 4
= (6, 0)).

`UOAnimationSequence::FrameToAction` 0x0083706b maps a global frame back to the record whose base <= frame
and the local frame `frame - base + 1`. That 1-based number is the second u16 of the `AnimationFrame` frame
entry (see "Relation to AnimationFrame").

### Action resolution

`UOAnimationSequence::ResolveAction` 0x00836e8a (confirmed at instruction level):

- `alias == -1` -> the action itself.
- `alias != -1` -> the alias, when the target exists and has `alias == -1`. Only one level is followed.
- Action 19 is treated as an alias of 0 when `g_UOFlyingAnimations` is 0.
- A missing action, a missing target or an alias pointing at another alias is logged. The result then falls
  back to the lowest action id with `alias == -1`.

`GetActionFps` 0x00836e24 returns the fps of the resolved action, or 16.0 when it is missing. When
`g_UOFlyingAnimations` is 0, actions 61..63 use the fps of action - 60. `GetFrameCount` 0x008375a7
returns the frame count of the resolved action.

Data: 5546 plain records, 5368 alias records. 5290 aliases point at a plain action. 78 point at another
alias, and the client resolves those to the body's lowest plain action:

- bodies 1401 and 1434: every action aliases 25 or 22, which alias 31 or 29;
- bodies 1417..1422, 1484, 1485: actions 13/14 alias 12, which aliases 5 or 4;
- body 1431 action 19 -> 0 -> 24; body 1485 action 11 -> 5 -> 4.

No alias target is missing.

### Groups (FrameSetLookup)

`UOAnimations::SelectAnimationGroup` 0x00437a70 picks the group key for a mobile:

| Key | Condition |
|---|---|
| 2 | mounted, when the sequence has group 2 |
| 4 / 5 | a mobile state flag (the predicate was not fully decoded) |
| 3 | flying: `g_UOSettingFlyingAnimations` and the component flying state |
| 0 | war mode |
| 6 | seated |
| 1 | otherwise (peace) |

Lookups search the selected group, then its parent chain:

- `FrameSetLookup::GetMoveAction` 0x008361ed (u8 move, bool alternate): movement map entry action or
  alternate, else `default_action`. Move keys are 0 stand, 1 walk, 2 run. **confirmed**.
- `FrameSetLookup::GetTypedAction` 0x00836142 (i8 type, u8 sub, u32 variant): typed map at (type, sub),
  then (type, 0xff), then the parent chain, else `default_action`; picks `list[variant % size]`.
  `g_UOFrameSetRestriction < 2` forces variant 0; `== 0` forces sub 0xff. **confirmed**.
- `FrameSetLookup::GetTransitionAction` 0x008360f9 (i8 key): transition map at key along the chain.
  `PlayTransitionAction` 0x0043a6e0 calls it from 0x005b84e0 / 0x005b8690 when the mobile's state flag
  (+0x5c) or flying mode changes, with key = (flag == 1 ? 0 : 1) + (flying ? 4 : 0). **probable**.
- `FrameSetLookup::IsStandAction` 0x00836232: action equals the stand (move 0) action or alternate of any
  group. Used by `GetDirectionFrames` for the idle setting.

Data:

- 370 sequences: groups 0 and 1 always; 2 in 37, 3 in 50, 6 in 4, 4 and 5 in one (body 666).
- Move keys are always 0..2; `alternate == action` in 2483 of 2491 entries.
- Typed entries: every listed action exists in its sequence. Typed lists have 0..20 choices; 101 are empty
  (the lookup then falls back to `default_action`).
- Transitions: (group 1, key 0 -> 28) in 332 bodies, (group 0, key 3 -> 19/20) in 50 bodies. Body 666 has
  (0, 4 -> 60), (1, 5 -> 60), (2, 5 -> 60), (4, 0 -> 61), (5, 1 -> 61). The client never passes key 3 (see
  open questions).

### Frame effects and skipped frames

- Effect frames: 56 entries in 22 actions, each a `UserPreference` wrapping one `Sound` (body 400 action 0:
  frame 1 sound 299, frame 5 sound 300, the footstep pair).
  The frame key is always below the frame count. `GetFrameEffects` 0x00837311 returns the collection;
  `UOAnimationFrameSet` vtable +8 (0x00833bc0) starts it when the frame is shown.
- Skipped frames: 69 actions, always the single frame 0. `UOObjectComponentAnimationMob::Update`
  0x00434c90 asks `IsFrameSkipped(action, index)` with the frame index computed from the frame set at the
  current time; when true it adds one frame interval and recomputes. The index is 0-based within the
  direction, so the first frame of these actions is never shown. **probable** (index base inferred from the
  frame-set helper 0x00833f3d).

### Stored id

Three entries store an id different from their name: 146 -> 704, 1308 -> 29, 1309 -> 1254. The frames of
those bodies exist only under the stored id (`animationframe/000704/...`); every `AnimationFrame` header's
animation id equals the stored id of the sequence that uses it (5485 of 5485). This is the only body
redirect in the EC data path: the binary contains no `bodyconv`, `mobtypes` or `body.def` strings.

## Relation to AnimationFrame*.uop

The frame file of (body, action) is keyed by the sequence's stored id and the resolved action:

- Header u32 at +0x0c (animation id) = stored id. **confirmed** (data).
- Frame entry `{u16 action, u16 number, i16 x0, i16 y0, i16 x1, i16 y1, u32 offset}`: the first u16 is the
  action id (5485 of 5485 files), the second is the 1-based local frame number `1..5 * frame_count`.
  **confirmed** (data), matching `FrameToAction`.
- Local frame number `n` belongs to direction `3 + (n - 1) / frame_count`, frame `(n - 1) % frame_count`.
- Frames can be missing: 10 files have gaps in the numbers (for example body 742 action 23 starts at 2;
  body 291 action 28 has only 1, 9, 17, 25, 33, the first frame of each direction). The number, not the
  entry position, gives the frame.
- 5384 of 5485 files have exactly `5 * frame_count` entries. Body 679 has 50 frames for actions whose
  sequence frame count is 1; the client shows only the first frame of each 10. 61 plain actions have no
  frame file (bodies 842, 778, 666, 0, 241, 1253).
- 561 bodies have frame files but no sequence. The client cannot animate them through this path.

Resolving "body B, action A, direction D" as the client does:

1. Sequence = `build/animationsequence/%08d.bin` of B (the body type); frames body = stored id.
2. Group = `SelectAnimationGroup`; A comes from the packet or movement lookups above.
3. `R = ResolveAction(A)`; frame count `fc` and fps from R.
4. `mirrored = D < 3`; `d = mirrored ? 6 - D : D`.
5. Frame file `build/animationframe/%06d/%02d.bin` of (stored id, R); frames numbered
   `(d - 3) * fc + 1 .. (d - 3) * fc + fc`; drop skipped frames; flip horizontally when mirrored.
6. Interval `1 / fps` (16 fps when the action is missing); `fc = 1` for stand actions when idle animation
   is off.

## AnimationDefinition.uop

### Layout

Parsed by `UOAnimationDefinition::Parse` 0x004b8d80 and `ParseFrameSets` 0x004b8ec0 into a
`UOAnimationDefinition` (0x48 bytes, vftable 0x00c9f744). All 1102 entries are consumed to the last byte.

```
u32   stored_id                // +4
u32   start_set                // +8, key of the first frame set
u32   second_set               // +0xc, key of a second frame set
f32   value                    // +0x14, must be >= 0
u32   set_count
set_count x {
    u32   key
    i32   next                 // -1 none; else key of the set played after this one
    f32   frame_interval       // seconds per frame
    u32   frame_count
    frame_count x {            // UOAnimationDefinitionBinaryFactory::ReadFrame 0x004b93d0
        u32   tile_id          // item graphic shown for the frame
        u8    effect_count
        effect_count x EffectDefinition
    }
}
```

| Field | Run-time use | Confidence |
|---|---|---|
| `stored_id` | kept at +4; 3 entries differ from their name (7905 -> 7897, 7923 -> 7916, 18153 -> 18150) | confirmed (data), unknown (use) |
| `start_set`, `second_set` | `SetStartFrameSets` 0x00833e2c requires both keys to exist; the tile component starts at the start set | confirmed |
| `value` | stored at +0x14, must be >= 0; 0.0 in all data | unknown |
| `key` | frame set map (+0x18) key; `AddFrameSet` 0x00833d81, `GetFrameSet` 0x00833d0d | confirmed |
| `next` | `SetNext` 0x00833ac9 links set +0x38 | confirmed |
| `frame_interval` | `SetFrameInterval` 0x00833b71 through the interned timing object `GetSharedTiming` 0x00833718 | confirmed |
| `tile_id` | tile drawn for the frame | confirmed |
| effects | started when the frame is shown | confirmed (code); none in the data |

Data: one set per entry, key 0, `next == -1`, `start_set == second_set == 0`, `value == 0.0`, no frame
effects. Intervals range from 0.0 to 0.9375 s (0.25 s is the most common); frame counts from 2 to 64. 1101 entries are item ids
(109..44909). One entry, 10000400, is keyed like a mobile (10000000 + body 400) and lists tiles 22, 3634,
3635 at 0.0625 s; no consumer of that id was found.

Consumers (all synchronous requests of type 0x0b keyed by item id):

- `UOObjectComponentAnimationTile::Initialize` 0x0042c710 -> `LoadAnimationDefinition` 0x0042c490;
- `UOProjectileEffect::LoadAnimationDefinition` 0x0044b8e0 and `UOTileEffect::LoadAnimationDefinition`
  0x00466700 (animated projectile and tile effects).

The mobile builder also creates `UOAnimationDefinition` objects (constructor 0x00430750) in memory, with
one frame set per action and direction (frames `base + i`, interval `1 / fps`). Those are never read from
`AnimationDefinition.uop`.

## Packets

| Packet | Reader | Handler | Behaviour |
|---|---|---|---|
| 0x6E CharacterAnimation | `UOPacket::Read_0x6E_CharacterAnimation` 0x005bb070: u32 serial, u16 action, u16 frame count, u16 repeat count, u8 forward, u8 repeat, u8 delay | `Receive_0x6E` 0x005c02c0 -> `OnCharacterAnimation` 0x004381b0 | `PlayAction(MapCharacterAnimationAction(action))`. Frame count, repeat count, forward, repeat and delay are ignored. |
| 0xE2 NewCharacterAnimation | `UOPacket::Read_0xE2_NewCharacterAnimation` 0x005bbac0: u32 serial, u16 type, u16 sub type, u8 variant | `Receive_0xE2` 0x005c0420 -> `OnNewCharacterAnimation` 0x00438220 | group = `SelectAnimationGroup`; action = `GetTypedAction(type, sub, variant)`; type 0 in war mode may use the weapon's attack action (0x004384e0); then `PlayAction`. |

`UOAnimationSequence::MapCharacterAnimationAction` 0x008370c5 (confirmed at instruction level): actions
200..269 become `GetTypedAction(group 1, type 11, sub 0xff, variant = the packet action)` when the sequence
has groups; without groups 200..209 -> 16 and 210..269 -> 17. Other actions pass through unchanged and are
then resolved through the alias table.

Movement uses `GetMoveAction` of the selected group (stand / walk / run, alternate for the second variant).

## Settings

Parsed by `UOSettings::ParseOptimizationSettings` 0x0048c450 (Lua `SystemData.Settings.Optimization`) and
copied into run-time globals in the packet registration function 0x005bc540:

| Setting | Stored | Run-time copy | Effect |
|---|---|---|---|
| `idleAnimation` (bool, default 1) | 0x00e3361c | 0x00da6feb | 0 -> stand actions show one frame |
| `frameSetRestriction` (u32, 0..2) | 0x00e33620 | 0x00e32168 | < 2 -> typed variant 0; 0 -> sub type ignored |
| `cacheSize` (u32) | 0x00e33624 | - | not traced |
| `flying_animations` (bool, default 1) | 0x00e33628 | 0x00da7994 | 0 -> action 19 resolves to 0; actions 61..63 use the fps of action - 60; no flying group |

## Mismatches with the repo parsers

### `lib/uocf/src/animation_sequence.rs`

| Item | Repo | Client | Severity |
|---|---|---|---|
| Entry name | tries `data/animationsequence/%06d.bin` first | only `build/animationsequence/%08d.bin` exists | wrong (dead lookup) |
| `parse_cc` header | reads i8 pairs and i16 pairs "until equal" | exactly 8 i8 pairs and 8 i16 pairs; the loop stops after the first (0,0) pair, so every later field is misaligned | wrong |
| `parse_cc` action record | u32 action, 3 ignored u32, two "until equal" loops, then `{u32, x_count, x_count x u32}` as directions | u32 action, u32 frame count, i32 alias, f32 fps, 8+8 fixed pairs, `{u32 frame, effect list}` frame effects, then the skipped frame list | wrong |
| `parse_cc` tail | one u32 count + u32 list after all actions | the u32 skipped list is per action; the tail is the group table | wrong |
| `parse_ec` | starts at offset 0 with `u8, u8, i32, u32` and three counted lists per "action" | resembles the group record (`i8 key, i8 parent, u32 default`, then three counted lists) with an extra u32, read at offset 0 instead of after the action table | wrong |
| `DirectionData::frame_indices` | per-direction list of frame indices read from the file | the file has no per-direction frame list: direction frames are computed (`base + (dir - 3) * frame_count`, mirroring for 0..2) | wrong (model) |
| `body_id` | the requested body | frames live under the stored id (146 -> 704, 1308 -> 29, 1309 -> 1254) | missing |
| alias, fps, offsets, frame effects, skipped frames, groups | ignored | see Layout | missing |
| `ec_action_count` | first u32 = action count | first u32 = stored id | wrong |

### `lib/uocf/src/classic/anim.rs` `AnimationDefinition`

| Item | Repo | Client | Severity |
|---|---|---|---|
| Purpose | body redirect table `u32 version, u32 count, count x (u32 from, u32 to)` | item animation frame sets keyed by item id | wrong |
| `resolve(body)` | redirects bodies | no body redirect exists in this file; body redirects come from the sequence stored id and action aliases | wrong |

### `lib/udd-conv/src/mobile_anim_ec.rs`

| Item | Repo | Client | Severity |
|---|---|---|---|
| `load_animation_sequences` (~1076) | `parse_ec`, keyed by the requested body | Layout above; keyed by the sequence entry name, frames by the stored id | wrong |
| Frame identity | `source_frame_index` = entry position in the frame file | the entry's second u16 (1-based number) gives direction and frame; gaps exist | wrong (10 files) |
| Records | one record per parsed "direction", or one action 0 / direction 0 record with all frames | 5 stored directions per action, directions 0..2 mirrored, alias actions share frames, fps per action | missing |

### `lib/uocf/src/enhanced/animationframe.rs`

| Item | Repo | Client | Severity |
|---|---|---|---|
| `FrameEntry::frame` | "frame" | action id | naming only |
| `FrameEntry::unknown` | unknown | 1-based local frame number (`(dir - 3) * frame_count + frame + 1`) | naming only |

### `lib/uocf/src/enhanced/ENHANCED_CLIENT_FILES.md`

Neither file is described. Missing.

## Proposed Rust changes

1. Replace `animation_sequence.rs` with one EC parser that follows the Layout exactly and fails on trailing
   bytes:
   - `AnimationSequence { stored_id, offsets: [(i8, i8); 8], extra_offsets: [(i16, i16); 8],
     actions: BTreeMap<u32, SequenceAction>, groups: BTreeMap<i8, ActionGroup> }`;
   - `SequenceAction { frame_count, alias: Option<u32>, fps, offsets, extra_offsets,
     frame_effects: BTreeMap<u32, Vec<EffectDefinition>>, skipped_frames: BTreeSet<u32>, base_frame }`,
     with `base_frame` computed in file order like `NextBaseFrame`;
   - `ActionGroup { parent: Option<i8>, default_action, transitions: BTreeMap<i8, u32>,
     moves: BTreeMap<u8, (u32, u32)>, typed: BTreeMap<(i8, u8), Vec<u32>> }`;
   - reuse `effect_definition::parse_effect_definition` for the frame effects;
   - named constants for the 8 directions, 5 stored directions, mirror sum 6, first stored direction 3,
     unset markers -128 / -0x8000, sub type 0xff, default fps 16.0.
   Drop `parse_cc`, the 6-digit path and `ec_action_count`.
2. Add `resolve_action(action, flying_enabled)` (one alias level, action 19 rule, fallback to the lowest
   plain action) and `direction_frames(action, dir) -> (first_number, count, mirrored)`, plus group lookups
   `move_action`, `typed_action(group, type, sub, variant, restriction)` and `map_character_animation`.
3. In `mobile_anim_ec.rs`, key sequences by entry name (body type) and frames by `stored_id`; derive
   direction and frame from `FrameEntry` number; emit records for directions 3..7 and mark 0..2 as mirrors;
   emit alias actions as references to the resolved action; carry fps and skipped frames.
4. Rename `FrameEntry::frame` -> `action` and `unknown` -> `frame_number` (1-based), with doc comments.
5. Replace `classic::anim::AnimationDefinition` with an item animation parser:
   `{ stored_id, start_set, second_set, value: f32, sets: BTreeMap<u32, FrameSet> }`,
   `FrameSet { next: Option<u32>, frame_interval: f32, frames: Vec<(u32 tile_id, Vec<EffectDefinition>)> }`,
   keyed by item id. Remove `resolve`; nothing in the EC data maps bodies through this file.
6. Document both files in `ENHANCED_CLIENT_FILES.md` and link this note.

## Uncertainties and open questions

- Transition key 3 (group 0 -> action 19/20 in 50 bodies) is never produced by the two callers of
  `PlayTransitionAction`, which pass 0, 1, 4 or 5. It may be unused data or come from a caller not found.
- The exact predicate that selects groups 4 and 5 in `SelectAnimationGroup` was not decoded.
- The skipped-frame index base (0-based frame within the direction) is inferred from the frame-set helper
  0x00833f3d, not traced through it.
- Header i8 pairs of the sequence and the `AnimationDefinition` `second_set` / `value` fields have no
  decoded consumer; all data values are trivial (zero).
- How `GetBodyType` (object vtable +0x70) maps the server body to the sequence id was not traced. No
  classic remap file is loaded, so it is probably the body id itself.
- `cacheSize` eviction of these assets was not traced.
- `AnimationDefinition` entry 10000400 has no consumer found.
