# UOSA.exe animation sequences and definitions

The Enhanced Client (`UOSA.exe`, Ghidra project `UOSA`) drives animations from two asset files:

- `AnimationSequence.uop`: one entry per mobile body. It holds the action table (frame count, fps, alias),
  per-direction pixel offsets, per-frame effects and skipped frames, and the action groups that map
  movement, packet animation types and state transitions to actions.
- `AnimationDefinition.uop`: one entry per animated item (tile). It holds frame sets of tile ids with a
  frame interval. It has nothing to do with mobile bodies or body redirects.

The mobile pixels come from `AnimationFrame1..6.uop`, one entry per (body, action). This document gives
the layout of both files as the client parses them, the verification against the shipped data, the
run-time semantics, the relation to the frame files and packets, the frame pixel decoding (palette, RLE,
alpha, hue), how a server body becomes a sequence and a frame key, and the mismatches with the repo parsers.

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
| `stored_id` | +0x10 (constructor 0x00836694, second argument) | the sequence's own body identity: `BuildLayers` checks equipment body types against it and logs it as "sq" (see "Stored id") | confirmed (code) |
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

Three entries store an id different from their name: 146 -> 704, 1308 -> 29, 1309 -> 1254. The stored id
is kept at sequence +0x10. It is **not** the frame key: the frame files are keyed by the tileart appearance
of the mobile (see "Body to sequence and frames"). Bodies 1308 and 1309 have their own frame files (the
same action sets as 29 and 1254), and no creature placeholder uses sequence 146 (placeholder 146 has body
type 124). The stored id is read by `UOObjectComponentAnimationMob::BuildLayers` 0x004387f0:

- an equipment layer is drawn only when the equipment definition's body type (tileart +0xe4) equals the
  wearer sequence's stored id, else it logs "Not letting a %u(sq %u) wear a %u(sq %u)";
- the cached mount animation is rebuilt when the mount's `GetBodyType` differs from its sequence's
  stored id.

The 5485 frame files the sequences reference under their stored id all exist, and every
`AnimationFrame` header's animation id equals the body in its own entry name (31877 of 31877). The binary
contains no `bodyconv`, `mobtypes` or `body.def` strings: the body redirects come from `tileart.uop`.

## Relation to AnimationFrame*.uop

The frame file of (body, action) is keyed by the mobile's appearance id (see "Body to sequence and
frames"; usually the body itself) and the resolved action:

- Header u32 at +0x0c (animation id) = the body in the entry name (31877 of 31877). The client never reads
  it. **confirmed** (data).
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
- 559 bodies have frame files but no sequence of their own. 557 of them are animated through the sequence
  of their tileart body type (401 -> sequence 400, for example). Only 344 and 827 have no creature
  placeholder, so the client cannot show them.

Resolving "server body S, action A, direction D" as the client does:

1. Definition = tileart record `10000000 + S`. Sequence = `build/animationsequence/%08d.bin` of its
   body type (+0x20 in the record). Frames body F = its appearance (props1 key 8, else S; minus 10000000
   when above it). When F has no frames, body 0 is used.
2. Group = `SelectAnimationGroup`; A comes from the packet or movement lookups above.
3. `R = ResolveAction(A)`; frame count `fc` and fps from R.
4. `mirrored = D < 3`; `d = mirrored ? 6 - D : D`.
5. Frame file `build/animationframe/%06d/%02d.bin` of (F, R); frames numbered
   `(d - 3) * fc + 1 .. (d - 3) * fc + fc`; drop skipped frames; flip horizontally when mirrored.
6. Interval `1 / fps` (16 fps when the action is missing); `fc = 1` for stand actions when idle animation
   is off.

## AnimationFrame*.uop pixel decoding

### Asset

- `UOAnimationFrameSetFactory::Create` 0x0084217e (factory vtable 0x00cdf0b4 +4, asset type 0x10) creates
  a `UOMobFrameSet` (vtable 0x00cdf0c8). `UOMobFrameSet::UOMobFrameSet` 0x00842078 reads u32 +4
  (signature `AMOU`), +8 (version, 1 in every file) and +0xc (total size, equal to the entry size in every
  file), and keeps the blob at +0x10. **confirmed**.
- The client reads these header fields only: +0x18 colours count and +0x1c colours offset (palette
  builder), +0x20 frames count and +0x24 frames offset (`FindFrameEntry` 0x00430f60, a binary search on
  `(u16 action, u16 number)`, ascending). No reader was found for +0x0c (animation id) or the +0x10..+0x17
  box. The box always contains the union of the frame boxes (31877 of 31877) but equals it in only 4495
  files. **confirmed** (code: no other reader found).
- Frame entry, 16 bytes: `u16 action, u16 number, i16 x0, i16 y0, i16 x1, i16 y1, u32 offset`. The pixel
  data starts at `entry address + offset`. width = x1 - x0, height = y1 - y0. The box is anchor-relative:
  pixel (0, 0) of the frame is drawn at (x0, y0) from the mobile's anchor point (body 400 action 0 frame 1:
  x -15..15, y -74..-10). **confirmed**.

### Palette

`UOAnimations::BuildFramePalette` 0x0042b270 (ECX = hue, EAX = blob; confirmed at instruction level) turns
the `colours_count` 4-byte entries into 32-bit words for a BGRA8888 surface:

```
entry = [b0, b1, b2, b3]
r, g, b, a = b0, b1, b2, 0xff
if hue != 0 and (table = GetHueTable(hue)) and b3 == 0:
    t = table[b0]                       // u32: R | G << 8 | B << 16 | A << 24
    r, g, b, a = t & 0xff, t >> 8 & 0xff, t >> 16 & 0xff, t >> 24
word = a << 24 | r << 16 | g << 8 | b   // B, G, R, A in memory
```

- Byte 3 is a "keep colour" flag, never alpha: 0 = the colour takes the hue, non-zero = it keeps its
  colour. Values in the data: only 0 (29248 files) or 0 and 1 (2629 files). Body 400 has 182 hueable
  near-grey skin colours and 74 brown flag-1 colours (the loincloth). **confirmed**.
- Every palette colour is opaque. The hue table is indexed by the red byte b0 (hueable colours are
  near-grey, so R is their brightness).
- Hue 0x4001 (shadow) is special: every colour becomes black (the flag byte b3 is ignored), with an alpha
  computed from the whole palette. Read at the instruction level (0x0042b2c5..0x0042b3f0); **confirmed**:

  ```
  // constants: 0.34f 0x00d05294, 0.07f 0x00d05290, 0.59f 0x00d0528c, 0.5 0x00c95604,
  //            0.2 0x00d04f28, 0.8 0x00d051cc, 130 0x00d05288, 60 0x00d04f60, 1.0 FLD1 0x0042b3bb
  inv[i] = 0xff - trunc((b0 * 0.34 + b2 * 0.07) + b1 * 0.59)     // the 0xff immediate at 0x0042b354
  min = min(inv) (starts at 0xff), max = max(inv) (starts at 0), mean = sum(inv) / count (unsigned DIV, 0x0042b39b)
  below = mean - min, above = max - mean, d = inv - min           // u32
  x = 0                                  when below == 0 or above == 0
  x = 0.5                                when d == below (inv == mean)
  x = ((1 - t) * 0.2 + t + 1) * 0.5      when d >  below, t = (d - below) / above
  x = t * 0.8 * 0.5                      when d <  below, t = d / below
  alpha = trunc(x * 130 + 60)            // FISTP with RC = truncate; 60..190
  word = alpha << 24                     // black
  ```

  Precision: the x87 code runs with the precision control of the D3D9 device thread. The device is created
  without `D3DCREATE_FPU_PRESERVE` (behaviour flags 0x28), so Direct3D sets single (24-bit) precision; the
  only other `_controlfp` caller is the CRT `__setdefaultprecision`. Both palette callers (`UpdateFrame`
  0x00437506 on the main loop, the paperdoll builders 0x0050b780 / 0x0050ba30 / 0x0050c550) run there, so
  the client computes in f32 (**probable**). It matters: over the 31877 shipped palettes (7,441,059
  colours), f32 and f64 evaluation give different alphas for 361,221 colours in 26,613 palettes (max
  difference 16). The main cause is the luminance of a pure grey g, which truncates to g in f32 and to g - 1
  in f64. `lib/uocf/src/enhanced/hues.rs` evaluates in f32 in the client's operation order.
- What selects 0x4001: `UOMobileStatus::OnBondedStatus` 0x00630ba0, the 0xBF sub-command 0x19 callback
  (bonded pet status), stores the packet's flag byte at byte +0x6d of the mobile status (component 5) and,
  when it is set, calls `SetHue(0x4001)` (object vtable +0x54). `UOObjectManager::ApplyGraphicAndHue`
  0x005ea110 (called by the 0x78 / 0x20 / 0x77 object handlers) replaces the packet hue with 0x4001 while
  that byte is set (0x005ea647). So the shadow hue is the dead bonded pet look. **confirmed** (code),
  **probable** (meaning).
- `UOHues::GetHueTable` 0x00427ad0 maps a hue to a table key: 0x4001 -> 0x9c3; no 0xc000 bits -> hue - 1;
  0x8000 set -> (hue & 0x3fff) - 1; only 0x4000 set -> the hue itself, so no table and no hue. The palette
  builder skips hue 0. The classic 0x8000 "partial hue" bit is masked away: in the EC, partial hueing is the
  per-colour flag. **confirmed**.
- `UOHues::LoadHueTables` 0x004274a0 reads `data\definitions\hues\huenames.csv` and, for each number N,
  `hue%04d.bmp` through `UOHues::LoadHueBitmap` 0x004271f0, stored under key N - 1. So server hue H uses
  `hue{H:04}.bmp`. The bitmap must be 32 bpp, 256 x 2. The table is the second stored row (top image row),
  B,G,R,A bytes -> `A << 24 | B << 16 | G << 8 | R`. Hues.uop holds 3000 bitmaps (1..3000), the csv and 2 other entries.
  Every table entry has alpha 255. **confirmed**.
- The palette is rebuilt when the layer hue (+0x20) differs from the cached one (+0x1c)
  (`UpdateFrame` 0x004374e0).

### RLE

`UOAnimations::DecodeFrameRle` 0x0042b620 (EAX = frame entry, then palette words, canvas, pitch; confirmed
at instruction level):

```
dst = canvas + y0 * pitch + x0 * 4               // anchor-relative, the canvas origin is the anchor
for each of height rows:                         // a skip may run past the row end; the excess carries over
    while dst < row_end:
        c = *src++
        if c < 0x80: dst += c                    // transparent, left untouched
        else:
            F = *src++
            if F >> 4:  *dst = lerp(*dst, pal[*src++], F >> 4); dst++
            repeat c - 0x80: *dst++ = pal[*src++]                 // solid, opaque copy
            if F & 15:  *dst = lerp(*dst, pal[*src++], F & 15); dst++
lerp(d, s, f) = per byte, all four channels: (s * f + d * (16 - f)) >> 4
```

Because a skip carries over to the next row, the row loop is the same as a linear pixel index
`0..width * height` over the frame box. Edge factors 1..15 occur in both nibbles.

- `UOAnimations::DecodeFrameRleMirrored` 0x0042b810 is the same loop, writing right to left from column
  `-x0 - 1`: pixel x of the frame lands at `-1 - x`, so a mirrored frame covers `[-x1, -x0)`.
- `UOAnimations::DecodeFrameMask` 0x0042ba20 runs the same RLE into a byte mask and marks every coded
  pixel, edges included. Its caller `FUN_0050bd00` also calls `DecodeFrameRle`. **probable**: hit testing.

### Layers and the atlas

`UOAnimations::BuildFrameAtlas` 0x00431a10:

- `UOAnimations::GetLayerBounds` 0x0042b170 computes the union of the layer boxes. It uses `[-x1, -x0]`
  for mirrored layers, subtracts the layer offsets (+0x18, +0x1c) and adds a 1-pixel margin.
- The atlas is a `NiPixelData` with a power-of-2 size (at least 4) and format BGRA8888 (`DAT_00dded60`:
  32 bits, B,G,R,A of 8 bits). It is zeroed (`memset`). An empty layer set gives a 64 x 64 surface.
- Layers are drawn in list order (body first, then equipment). Layer +0xc chooses the mirrored decoder, and
  `**(layer + 0x10)` is the palette. Edge pixels of an upper layer blend over the lower layer's pixels,
  alpha included.

Over the zeroed atlas, a single layer gives: solid = (R, G, B, 255); edge with factor f =
(R*f >> 4, G*f >> 4, B*f >> 4, 255*f >> 4). That is premultiplied over black. The straight-alpha
equivalent is (R, G, B, 255*f >> 4).

### Frame-set loading per layer

`UOAnimations::LoadLayerFrameSet` 0x00439b70 (owner, out, id, any-action flag, animation definition
shared_ptr):

- the id loses 10000000 when it is >= 10000000;
- the action comes from `UOAnimations::GetLayerAction` 0x00439a70 (frame-set time -> global frame ->
  `FrameToAction`);
- then `GetAnimationFrameSet(id, action)`;
- with the flag set and no file, it takes the first of actions 0..0x33 that exists;
- without an animation definition (no sequence) the result is an empty frame set: nothing is drawn.

`BuildLayers` passes the flag 1 and `UpdateFrame` passes 0. **confirmed** (the id at instruction level).

### Data check

`verify_animation.py --frames ... --pixels` walks every frame of the 31877 files (1,572,711 frames):

- every stream ends exactly where the next frame's data starts, or at the end of the file;
- no run passes the frame box;
- every palette index is below the colour count;
- 1571702 frames have edge pixels.

The scratch decoder `probe_frames.py` (scratch folder, not in the repo) was run on bodies 1, 2, 9, 260,
400 and 401. It decodes by the client algorithm, on a magenta background:

- the silhouettes are opaque and the edges are blended;
- no stray pixels appear;
- the human skin is grey (hueable), the loincloth is brown (flag 1);
- creatures with all-zero flags show their natural colours.

## Body to sequence and frames

### Code path

1. Server body S arrives in 0x78 / 0x20 / 0x77 (and 0x1B for the player). The handlers
   (`FUN_005e9ae0`, `FUN_005e9db0`, `FUN_005eb220`, `UOPacket::Handle_0x1B_LoginConfirm` 0x00611950) add
   10000000. `UOObjectManager::ApplyGraphicAndHue` 0x005ea110 compares the result with the object type (vtable +0x60). It then takes
   `UOTileArt::GetObjectDefinition(10000000 + S)` and calls `SetBodyType` (object vtable +0x6c, 0x005b3070)
   with the definition's `GetBodyType` (+0xb4 -> tileart record +0x20, default 400).
   `UOObject::InitFromDefinition` copies the same field. **confirmed**.
2. `UOObject::GetBodyType` 0x005b3080 (vtable +0x70, object +0x50) is the sequence id
   `BuildMobileAnimation` requests. **confirmed**.
3. `UOObjectComponentAnimationMob::BuildLayers` 0x004387f0 builds the body layer (confirmed at instruction
   level, 0x00438ad1..0x00438c53):
   - frames id = `GetAppearance` (definition vtable +0xbc: props1 key 8, else the tile id, then the
     non-animation appearance filters);
   - a non-zero `GetAnimationAppearance` (+0xc0, filter subtype 2) replaces it;
   - then `LoadLayerFrameSet(id, flag 1)`;
   - when that fails, it retries with id 0 (body 0); when that fails too, the mobile gets no layers.
4. Equipment (26 layers, `0x1a`): frames id = the item definition's `GetAppearance` /
   `GetAnimationAppearance` for the wearer (minus 10000000). The record `10000000 + id` must exist, and its
   body type must equal the wearer sequence's stored id, else the item is not drawn.
5. Ghost bodies 402, 403, 607, 608, 694, 695 (checked on the definition's tile id minus 10000000) are
   drawn with hue 0x3e3. So is any mobile whose mobile flags have bit 0x80 (0x004388e8:
   `UOObject::GetMobileFlags` 0x005b3e80, object vtable +0xa0, returns object +0x6c, `>> 7 & 1`). Object
   +0x6c is the flags byte of 0x77 / 0x78, stored by `UOObject::SetMobileFlags` 0x005b3e90 (vtable +0xa4);
   bit 0x80 is the "hidden" bit of those packets. Equipment of such a mobile gets 0x3e3 too (0x00439267).
   Other mobiles use their hue (object vtable +0x58). **confirmed** (code), **probable** (bit 0x80 =
   hidden).

The body type is plain data: no hardcoded table, gender or race substitution, or Lua mapping exists in
this path. Race (props1 key 9) and gender (key 10) are set on the human placeholders. They are read
elsewhere:

- `UOAnimations::BuildEquipmentLayerOrder` 0x00468810 builds the two 26-entry equipment draw orders
  (object +0xf8 and +0x160) from the definition's race and gender (`HasRace` +0xc8 / `GetRace` +0xd0,
  default 1 at 0x00842df1; `HasGender` +0xcc / `GetGender` +0xd4, default 0 at 0x00842e68).
  `UOAnimations::GetDrawLayerItemSerial` 0x005b77c0 walks that order for `BuildLayers`: entry i -> layer
  +0xf8[i]; layer 0x19 is the mount; layer 0x1e is skipped.
- `AppearanceFilterByOwnerGender::Apply` 0x0084b485 reads the owner definition's `GetGender` to choose an
  item appearance.

### Data (tileart.uop creature placeholders)

`verify_animation.py --tileart` and the scratch `probe_bodies*.py`:

| Check | Result |
|---|---|
| placeholders (tile id >= 10000000) | 1143 |
| body type has a sequence | 1143 of 1143 |
| body type == body | 363; 780 map to another sequence (401 -> 400, 10 -> 9, 146 -> 124) |
| appearance (props1 key 8) differs from the body | 172 (402 -> 400, 403 -> 401, 607 -> 605, 146 -> 124) |
| frames exist under the appearance | 1091; 52 have none and fall back to body 0 (433, 484, 507, ...; human or gargoyle body types) |
| frame bodies reached by an appearance | 919 of 926; 344 and 827 have no placeholder |
| sequences no placeholder uses | 10, 146, 567, 679, 833, 842, 1551 |

Examples:

- 401 (female human): body type 400, appearance 401, gender 1 -> sequence 400, frames 401.
- 605 / 606 (elves): sequence 400, own frames.
- 667: sequence 666, own frames.

### Other things in the path the repo does not handle

- Per-colour hue flag (palette byte 3) and hue tables by red byte (see "Palette").
- Shadow / translucent hue 0x4001 and the ghost hue 0x3e3.
- Mirrored directions are mirrored around the anchor (x -> -1 - x). They are not flipped inside the frame
  box.
- Equipment layers come from the item's appearance, filtered by the wearer's body (appearance filters
  0/1/2), and their edges blend over the body.
- Mount: `BuildLayers` gives the mount item's animation the rider's state, with group 2 when the rider
  sequence has it (else group 1), then `GetMoveAction` and `PlayAction`. The rider's sequence offsets
  (per direction, overridden per action by `GetOffsetX/Y` unless -0x80, plus `GetExtraOffsetX/Y` unless
  -0x8000) place the attachments.
- Frame effects (`UpdateFrame`, effect type 7 PositionOffset checked at 0x0043725c) are placed at: (x +
  offset, y + offset - 15) when the effect has offsets (+0x2c); else, when seated on a chair (component
  +0xb4), at the chair definition's negated seat offsets (`GetSeatOffsetX/Y`, definition vtable +0xec /
  +0xf0: sitting record fields 1 and 2); else at (x, y - 15). The earlier reading of +0xec / +0xf0 as mount
  offsets was wrong: they belong to the chair.
- `UOObjectComponentAnimationMob::RefreshMoveAction` 0x00437e60 (vtable +0x68) recomputes the movement
  action with `GetMoveAction(group, 0, alternate)`, where alternate comes from
  `GetLayer2AlternateAnimation` 0x004386a0 (the layer-2 item's `GetAlternateAnimation`, object vtable
  +0x78). In group 6 (seated) it keeps the chair and turns to its sitting directions (definition +0xe8).
  The body's own `alternate_animation` placeholder field is not read on this path.
- `UOObjectComponentAnimationMob::CollectWeaponAttackActions` 0x004394a0, called by `BuildLayers` when an
  item with tiledata flag 2 (weapon) changes, collects which of actions 9..14 have frames for the weapon's
  appearance (set at component +0xe0). It does not concern the mount.
- Shadow frames: no separate shadow asset or decoder was found in this path.

### Creature record fields

Fields of the `10000000 + body` tileart records, the code that reads them for a mobile, and whether
dynamapper needs them. Data from the reference client (1143 placeholders; the scratch
`placeholder_fields.py` counts also include 2 non-creature records).

| Field (tileart) | Definition getter | Reader in the mobile path | Data | dynamapper |
|---|---|---|---|---|
| body type (+0x20, definition +0xe4) | `GetBodyType` +0xb4 | `ApplyGraphicAndHue` -> object +0x50: the sequence | == body for 363, other sequence for 780 | yes (`sequence_id`, packed) |
| sequence stored id | (sequence +0x10) | `BuildLayers` wear check: item body type must equal it | differs from the sequence id for 2 bodies | yes (`sequence_stored_id`, new in v5) |
| appearance (props1 8) | `GetAppearance` +0xbc | `BuildLayers` frames id | 184 records set it | yes (`appearance_body_id`, packed) |
| animation appearance | `GetAnimationAppearance` +0xc0 | `BuildLayers` overrides the appearance | filter-driven | yes (resolved by the packer) |
| race (props1 9) | `HasRace` +0xc8 / `GetRace` +0xd0 (default 1) | `BuildEquipmentLayerOrder` | 13 records: 0, 1, 2 | yes (`race`, new in v5) |
| gender (props1 10) | `HasGender` +0xcc / `GetGender` +0xd4 (default 0) | `BuildEquipmentLayerOrder`, owner-gender appearance filter | 18 records: 0, 1 | yes (`gender`, new in v5) |
| alternate animation (definition +0xe8) | `GetAlternateAnimation` +0xb8 | only through the layer-2 item (`GetLayer2AlternateAnimation`) | set on 8 placeholders (323, 635, 1027, 1028, 1033, 1041, 1042, 1047) | no (item property) |
| world / legacy / primary art scale | +0x2c / +0x30 / +0x84 | none found in `UOObjectComponentAnimationMob` | world 1.4 on 1076, legacy 1.0 on 1111 | no (open question) |
| light scale / shader / offsets | +0x40 / +0xa0 | none found for mobiles | 1.0 / 0 on all | no |
| flags1 PartialHue (0x40000) / AlphaBlend (0x800000000) | object +0x38/+0x3c | none found in the animation path | 10 / 3 records | no |
| radar colour | +0xdc | not in the mobile path | | no |
| sitting directions / seat offsets | +0xe8 / +0xec / +0xf0 | `RefreshMoveAction`, `UpdateFrame` (of the chair) | chairs only | no (not a creature field) |

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
| `body_id` | the requested body | sequence id = tileart body type of `10000000 + body`; frames under the tileart appearance; stored id is the wear-check identity | missing |
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
| `load_animation_sequences` (~1076) | `parse_ec`, keyed by the requested body | Layout above; keyed by the sequence entry name; a body reaches its sequence through the tileart body type and its frames through the tileart appearance | wrong |
| Bodies without an own sequence | dropped (559 frame bodies) | animated with the body type's sequence (557 of them) | missing |
| Frame identity | `source_frame_index` = entry position in the frame file | the entry's second u16 (1-based number) gives direction and frame; gaps exist | wrong (10 files) |
| Records | one record per parsed "direction", or one action 0 / direction 0 record with all frames | 5 stored directions per action, directions 0..2 mirrored, alias actions share frames, fps per action | missing |

### `lib/uocf/src/enhanced/animationframe.rs`

| Item | Repo | Client | Severity |
|---|---|---|---|
| `FrameEntry::frame` | "frame" | action id | naming only |
| `FrameEntry::unknown` | unknown | 1-based local frame number (`(dir - 3) * frame_count + frame + 1`) | naming only |
| Palette byte 3 | alpha (`u32::from_le_bytes`, `blend_pixel`) | "keep colour" flag (0 = hueable); every colour is opaque | wrong: every decoded frame is transparent |
| Edge pixels | `c * f >> 4` on all four bytes, alpha from byte 3 | coverage f/16 over the destination; alpha 255 * f >> 4 | wrong |
| `center_x` / `center_y` | header x0 - frame x0 (relative to the header box) | anchor = (-x0, -y0) from the frame's top-left; the header box is unused | wrong (constant shift per file) |
| Width / height | `abs(x1 - x0)` | `x1 - x0` (never negative in the data) | harmless |
| Hue | none | per-colour flag + hue table indexed by the red byte; specials 0x4001, 0x3e3 | missing |

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
3. In `mobile_anim_ec.rs`, key sequences by entry name (body type) and frames by the tileart appearance
   (item 8); derive
   direction and frame from `FrameEntry` number; emit records for directions 3..7 and mark 0..2 as mirrors;
   emit alias actions as references to the resolved action; carry fps and skipped frames.
4. Rename `FrameEntry::frame` -> `action` and `unknown` -> `frame_number` (1-based), with doc comments.
5. Replace `classic::anim::AnimationDefinition` with an item animation parser:
   `{ stored_id, start_set, second_set, value: f32, sets: BTreeMap<u32, FrameSet> }`,
   `FrameSet { next: Option<u32>, frame_interval: f32, frames: Vec<(u32 tile_id, Vec<EffectDefinition>)> }`,
   keyed by item id. Remove `resolve`; nothing in the EC data maps bodies through this file.
6. Document both files in `ENHANCED_CLIENT_FILES.md` and link this note.
7. `AnimationFrame::decode_frame` (`lib/uocf/src/enhanced/animationframe.rs`), with named constants
   `RLE_RUN_FLAG = 0x80`, `EDGE_FACTOR_BITS = 4`, `EDGE_FACTOR_MASK = 0x0f`, `EDGE_FACTOR_ONE = 16`,
   `OPAQUE_ALPHA = 0xff`, `PALETTE_KEEP_COLOUR_BYTE = 3`:
   - palette word = `[b0, b1, b2, OPAQUE_ALPHA]` (RGBA8888 bytes). Never use byte 3 as alpha;
   - solid pixel: copy the word; edge pixel with factor f: RGB = palette RGB and A = `255 * f >> 4`
     (straight alpha). That is the client's premultiplied-over-black result, `(c * f >> 4, 255 * f >> 4)`,
     un-premultiplied. Keep straight alpha so the red byte stays usable as the hue key on edges;
   - decode over the linear pixel index `0..width * height` (as now), width = `x1 - x0`, and fail when a run
     passes the box or the stream ends early, instead of silently clamping;
   - return a per-pixel hue mask next to the RGBA: `hueable = palette[i][3] == 0` (1 bit or 1 byte per
     pixel). The hue key of a hueable pixel is its red byte;
   - `center_x = -x0`, `center_y = -y0` (the anchor inside the frame image); drop the header-box
     subtraction. Mirrored directions: draw the frame at x from `-x1` to `-x0` around the anchor
     (pixel x -> `-1 - x`), so the mirrored `center_x = x1`;
   - rename the header fields: +0x0c `animation_id` (unused by the client), +0x10..+0x17 `bounds`
     (unused; contains every frame box).
8. Hue support (new, e.g. `lib/uocf/src/enhanced/hues.rs`): read `Hues.uop`
   (`data/definitions/hues/huenames.csv`, `hue%04d.bmp`, 32 bpp, 256 x 2, second stored row,
   B,G,R,A per entry). `hue_table(hue)` follows `GetHueTable`: 0 -> none; 0x4001 -> shadow; mask 0x3fff
   when 0x8000 is set; 0x4000 alone -> none; table of `hue{H:04}.bmp`. Hued colour of a hueable pixel =
   `table[red]` (keep the edge alpha). Hue 0x4001 = black with the palette-statistics alpha of "Palette"
   (done: `ShadowPaletteStats`; the package stores min / mean / max per frame, format v5).
9. Body mapping (new, from `tileart.uop`, no hardcoded table): for each placeholder `10000000 + body`,
   `sequence_id = body_type` (+0x20), `frames_body = props1[8]` (else body; minus 10000000 when above), and
   body 0 when no frames exist for it. In the packer (`mobile_anim_ec.rs`) and the package
   (`lib/udd-assets/src/mobile_anim_ec.rs`):
   - emit one animation per server body: the frames of `frames_body` with the actions, fps, aliases and
     offsets of `sequence_id`;
   - stop dropping frame-only bodies (557 become drawable);
   - share the pixel data when several bodies use the same frames body (402/403 reuse 400/401);
   - record ghost bodies 402, 403, 607, 608, 694, 695 as "draw with hue 0x3e3".
   The tileart parser already reads body type and props1 (see `../ghidra-tileart`).
10. Creature fields (done, package format v5): body records carry the sequence stored id (the equipment
    wear check), race and gender (equipment draw order); frame records carry the shadow palette
    statistics. Still open: the mount item -> mount body mapping, mobile flag 0x80 (hue 0x3e3) from the
    server flags, and the bonded flag (hue 0x4001) from 0xBF 0x19 belong to dynamapper's server state, not
    to the package.

## Uncertainties and open questions

- Transition key 3 (group 0 -> action 19/20 in 50 bodies) is never produced by the two callers of
  `PlayTransitionAction`, which pass 0, 1, 4 or 5. It may be unused data or come from a caller not found.
- The exact predicate that selects groups 4 and 5 in `SelectAnimationGroup` was not decoded.
- The skipped-frame index base (0-based frame within the direction) is inferred from the frame-set helper
  0x00833f3d, not traced through it.
- Header i8 pairs of the sequence and the `AnimationDefinition` `second_set` / `value` fields have no
  decoded consumer; all data values are trivial (zero).
- The 0x4001 formula is exact; its evaluation precision (f32, from the D3D9 FPU state) is probable, not
  observed at run time.
- Bit 0x80 of the 0x77 / 0x78 flags (hue 0x3e3) is read as "hidden" from the packet's known meaning; the
  client code only tests the bit.
- No reader of the art scales (+0x2c / +0x30 / +0x84) was found in the mobile drawing path; the scene graph
  may apply them elsewhere.
- The mount item -> mount body mapping and the rider offsets per action are not in the package.
- The texture blend mode of the mobile atlas (premultiplied or straight alpha) was not traced. The atlas
  bytes over black are premultiplied.
- `FUN_0050bd00` (`DecodeFrameRle` + `DecodeFrameMask`) is probably the hit-test / paperdoll path; not
  traced.
- The sequence stored id is the wear-check identity; no other reader of sequence +0x10 was searched for.
- `cacheSize` eviction of these assets was not traced.
- `AnimationDefinition` entry 10000400 has no consumer found.
