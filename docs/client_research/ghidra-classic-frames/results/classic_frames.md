# classic_7.0.103.0_client.exe: Classic animation frames

Program `classic_7.0.103.0_client.exe` (SHA-256 `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a`).
**confirmed** = read in the code; **probable** = inferred from the code and the shipped data.

## Functions

| Address | Name | Notes |
|---|---|---|
| 0x00449c90 | `UOClassicAnim::DrawFrameRle` | draws a frame's runs; hue modes; mirror |
| 0x00449c30 | `UOClassicAnim::DarkenPixel` | probable name: the 0x4000 hue modes darken the destination |
| 0x00449aa0 | `UOClassicAnim::DrawFrameShadowRle` | the same runs as a shadow (0x3DEF blend) |
| 0x0044a4c0 | `UOClassicAnim::FrameRleHitTest` | pixel under the cursor |
| 0x005d4dc0 | `UOClassicAnim::DrawFrameRleToBuffer` | probable name: a 64x128 buffer, anchor (32, 80), same hue modes |
| 0x00543500 | `UOClassicAnim::GetMulFrame` | `anim*.mul` entry → frame |
| 0x00543e50 | `UOClassicAnim::GetUopFrame` | `animationlegacyframe` entry → frame |
| 0x00542c40 | `UOClassicAnim::FindFrameEntry` | binary search of the AMOU table |
| 0x005bc880 | `UOClassicAnim::GetMobileFrame` | MUL or UOP by the object's +0x3E; **probably** corpses only, kept name |
| 0x00452ad0 | `UOClassicAnim::DrawCorpse` | `CCorpse` vtable slot 2; was misnamed `DrawMobile` |
| 0x005441e0 | `UOClassicAnim::GetObjectLayerFrame` | the mobile layers' route: UOP or MUL, see [uop_reachability.md](uop_reachability.md) |
| 0x0056c170 | `UOClassicFiles::ReadMulIndexEntry` | probable name: idx lookup / length |
| 0x00543890 | `UOClassicAnim::GetUopFrameSet` | probable name: requests the frame set asset |
| 0x00413752 | `UOMobFrameSet::Construct` | the asset object (vtable 0x006db10c) |
| 0x0041385a | `UOAnimationFrameSetFactory::CreateAsset` | "Failed to get animation frame set" |
| 0x005a4a00 | `GameLoop_Tick` | calls `TickMobiles` when more than 75 ms passed |
| 0x005bc160 | `TickMobiles` | step frames, standing, frames in place; see [Pacing and idle](#pacing-and-idle) |
| 0x005b2310 | `CMobile_StepTicks` | ticks a step lasts, from `animinfo.mul` |
| 0x005b2f20 | `IsStandAtOnceBody` | bodies 0x50, 0x51 |
| 0x005b2e80 | `CMobile_AnimatesInPlace` | the frame counter runs while not stepping |
| 0x005bbf20 | `CMobile_TryIdleFidget` | the idle fidget |
| 0x005b6e40 | `CMobile_UpdateUopMoveAction` | a UOP body's move action, from `GetObjectLayerFrame` |
| 0x005b6270 | `CMobile_UopSequenceGroup` | the sequence group of a UOP body |
| 0x005b6580 | `CMobile_SetUopAction` | sets the action, frame count and frame time |
| 0x005b6cc0 | `CMobile_PlayAnimation` | a server-style animation |
| 0x00408114 | `FrameSetLookup_GetMoveAction` | the move action of a sequence group |
| 0x00597ae0 | `MobTypes_IsPeopleGroup` | `mobtypes.txt` flag 0x400 |
| 0x00597a00 | `MobTypes_IsLowGroup` | flag 0x40 |
| 0x005979d0 | `MobTypes_IsLowGroupExtended` | flag 0x20 |
| 0x00597960 | `MobTypes_CanFly` | flag 0x8, with the flying gate `DAT_00742cf8` |
| 0x00597870 | `MobTypes_IdleAt8` | flag 0x4 |
| 0x005977e0 | `MobTypes_IsHuman` | type 3 (HUMAN) |
| 0x00597780 | `MobTypes_Flag2` | flag 0x2 |
| 0x005976b0 | `MobTypes_GroupType` | the type; 4 (EQUIPMENT) without a row |
| 0x0044d250 | `MobTypes_EnsureTable` | allocates the 0x800-row table |

The move-code names (from `GameLoop_Tick` down) are in the global namespace; `NameClassicMoveCode.java` checks them.

## The frame

1. **Palette.** 256 `u16` words before the frames (MUL: at the entry start; UOP: at the start of every frame's
   data). The client draws `palette[index]` for every pixel of a run, bit 15 and a word of 0 included (0 draws
   black): there is no transparent index or colour. **confirmed** (`DrawFrameRle`).
2. **Header.** `i16` anchor x, `i16` anchor y, `u16` width, `u16` height. **confirmed**.
3. **Runs.** A `u32` header: `x = (int)hdr >> 22`, `y = (hdr << 10) >> 22` (both signed, 10 bits),
   length `hdr & 0xFFF`; then `length` palette indices. The pixel drawn is anchor-relative: image pixel
   `(x, y)` has offsets `(x - anchor x, y - anchor y - height)`. **confirmed** for the decoding, **probable** for
   the image mapping (it matches every shipped frame and uocf's decoder).
4. **End.** The `u32` 0x7FFF7FFF. Drawing stops at the first run below the clip box, so runs go top to bottom.
   Mirroring reads the runs backwards from the marker. **confirmed**.
5. **Hue.** A hue of 0x3FFF bits takes hue table entry hue - 1 (`DAT_00a6b810`, stride 0x2C); bit 0x8000 hues
   only the greys (`(c & 0x3FF) == c >> 5`). **confirmed**.
6. **A colour of 0 is an opaque pixel on every path** (the four functions that read the runs, the only code
   with the 0x7FFF7FFF constant). **confirmed**:
   - `DrawFrameRle`, plain (hue & 0x3FFF = 0, normal and mirrored): writes `palette[index]`, so 0 is black.
   - full hue (no 0x8000 / 0x4000): writes hue entry `[(c >> 10) + (hue - 1) * 0x2C]` (the shift at
     0x007469e8 is 10): 0 takes the hue's colour 0, as any colour of red 0 (0x0001 too).
   - partial hue (0x8000): 0 is a grey, so it takes the hue's colour 0; 0x0001 is no grey and is drawn as is.
   - 0x4000 (with 0x8000: the greys only, others written as they are): `DarkenPixel` darkens the destination
     by level 0x17 - (c >> 10); 0 darkens most. No pixel is skipped.
   - `DrawFrameShadowRle` halves the destination under every run pixel; it never reads the index.
   - `FrameRleHitTest` tests run coverage only; it never reads the index.
   - `DrawFrameRleToBuffer`: the plain, full and partial hue modes of `DrawFrameRle` (hue clamped: above 2999
     is 0), with the same results for 0.

## The sources

1. **MUL.** `GetMulFrame` reads the idx entry, then at the entry start the palette, a `u32` frame count and
   `u32` offsets relative to the end of the palette; an offset of 0 is a missing frame (skipped). **confirmed**.
2. **UOP.** `GetMobileFrame` takes the MUL path when `BodyUsesUopFrames(body)` is false, else `GetUopFrame`, which
   requests the frame set of (body, action), searches its table with `FindFrameEntry` by (action, frame number)
   and decodes at the entry's data offset; a width or height of 0 gives no frame. The container fields read are
   the frame count (+0x20), the table offset (+0x24) and each entry's key and data offset. **confirmed**.
   - **The per-body test.** `BodyUsesUopFrames` (0x005978a0) is true when the body has a `mobtypes.txt` row whose
     flags have 0x10000. `LoadMobTypesFile` (0x00597d70) reads `"%d %s %x"` per line, cuts at `#`, and keeps ids
     1..0x7FF with a known type. A body out of range, or with no row, takes the MUL path. **confirmed**. Within
     `GetMobileFrame` there is no fallback from one source to the other. **confirmed**.
   - **What the test does not prove.** It is tested on the object, not on the id requested, and on the flagged route
     the id isn't tested at all, so an unflagged UOP entry can be drawn. Body 1525 is: a gargoyle wearing tile 42002
     requests it. Bodies 197, 198 and 267 are requested only if a server sends those tile graphics as layer items.
     `PreloadWeaponUopFrameSets` (0x005b96a0, **probably** a preload) doesn't test it either. Trace and verdict:
     [uop_reachability.md](uop_reachability.md). **confirmed**.
   - **Remapping.** Only the MUL path goes through `body.def`, `corpse.def` and `bodyconv.def`, after the test
     (`ConvertCorpseBodyForMul` 0x0044f6d0, `GetMobileMulFrame` 0x0044e8e0). The UOP path keys on the appearance of the
     id's tile definition on the object's placeholder: vtable +0xbc (`TileDefinition_GetAppearance`), or +0xc0 when not
     0. That is the tile id unless property 8 or a filter says otherwise, so it isn't always the body: flagged body 647
     resolves to 603. **confirmed**.
3. **Unread fields.** The header box, the colour table and the entry boxes of the AMOU container are not read
   on this path. `UOMobFrameSet::Construct` reads three `u32` from its stream, not identified. **probable** that
   nothing else reads them.

## For a writer

- The shipped encoder: runs row by row, left to right, each a maximal span of opaque pixels split at 0xFFF; no
  bytes after the marker; MUL frames contiguous after the offset table. The uocf writer reproduces every shipped
  frame byte for byte (`classic_animation_frame_encoding` real-file tests).
- Transparency is only "not in a run": an RGBA pixel of alpha 0 is left out, every other pixel is opaque.
- uocf's decoders draw a colour of 0 opaque black, as the client does (since 615cebf5; they drew it
  transparent before). ClassicUO draws it transparent, and the partial hue hues 0 but not 0x0001: new palettes
  store black as 0x0001.

## Directions

`GetMobileMulFrame` and `GetObjectLayerFrame` take the stored direction from the table at 0x007087dc, indexed by
`direction & 7` (0 north .. 7 north-west): [3, 2, 1, 0, 1, 2, 3, 4], and mirror the frame when the table at
0x007087fc, [1, 1, 1, 0, 0, 0, 0, 0], says so. World directions 3..7 are stored at 0..4; 0, 1 and 2 show 6, 5 and 4
mirrored. **confirmed**. ClassicUO's `GetAnimDirection` gives the same mapping.

## Movement actions

### MUL bodies

`GetMobileMulFrame` picks the action of a mobile with no server animation (argument 4 is -1) from the
`mobtypes.txt` type of the body after `body.def` (`MobTypes_GroupType`; no row is 4, EQUIPMENT, whatever the id),
then reads frame `counter % n` of `anim*.mul` entry `(body - base) * stride + action * 5 + stored direction +
offset`. The counter is the mobile's +0x1a0 (see [Pacing and idle](#pacing-and-idle)). **confirmed**.

| Layout | Bodies | Entry | Stand | Walk | Run |
|---|---|---|---|---|---|
| monster | type 0 | `body * 110` | 1 (1 frame); 8 (6) with flag 0x4 | 0 (10) | 0; 19 with `MobTypes_CanFly` (10) |
| sea monster | type 1 | `body * 110` | 2 (1) | 0 (10) | 1 (10) |
| animal | type 2; type 0 with flag 0x40 | `(body - 200) * 65 + 22000` | 2 (1) | 0 (5) | 1 (5) |
| people | types 3, 4 and above; type 0 with flag 0x400 | `(body - 400) * 175 + 35000` | 4 (1) | 0 (10) | 2 (10) |

- A type-0 body with flag 0x400 plays action 0 for every movement (10 frames). An animal with flag 0x20 takes the
  monster rules, flags 0x400 and 0x40 included.
- A monster flying (mobile +0xcc & 4) plays 19 whatever it does.
- People: only a HUMAN (`MobTypes_IsHuman` on the body at +0x38) with a mount (+0x188) rides: walk 23, run 24
  (5 frames), stand 25 (1); EQUIPMENT never does. War mode walks with 15 and stands with 7 or 8 (the two-handed
  list at 0x00708828); a layer-2 item outside the list at 0x0070887c walks with 1 and runs with 3.
- No action is tested for frames: idle-at-8, 19 and the animal run 1 are played whether the body has them. A
  missing frame takes the layout's stand action in `GetObjectLayerFrame`. **probable**.

`MobileAnimCcPackage::movement_action` keeps ClassicUO's rules (`GetGroupForAnimation`) for these bodies, by choice:
equipment bodies ride there, a body without a row takes a layout from its id range (below 200 monster, below 400
animal, else people), and 19, 8 and the animal run 1 are taken only when the body has them (else 0, 1 and 2).

### UOP bodies

For a body with `mobtypes.txt` flag 0x10000, `GetObjectLayerFrame` calls `CMobile_UpdateUopMoveAction` when no
server animation plays. Movement 0 when standing (+0x1a8), else 1 walk or 2 run (+0x1a4); the action is
`FrameSetLookup_GetMoveAction(group, movement, alternate)` of the body's `AnimationSequence.uop` entry: the movement
entry of the first group of the parent chain that has one (its alternate action when the layer-2 item's tile
definition asks), else the default action of the chain's last group. `CMobile_SetUopAction` runs when the action
changes; the frame time is 1000 / fps of the sequence (62.0 ms for a non-UOP action); the counter wraps at the frame
count. **confirmed**. The fps is `GetActionFps` (0x00408d83: the resolved action's, 16 for none) with the flying
switch `DAT_0074dca5` on, as both callers of `FUN_00407f52` set it to 1; a UOP body without a sequence entry keeps
the frame time it had. **confirmed**. The package stores that fps on the UOP animation records of these bodies.

`CMobile_UopSequenceGroup` picks the group: 2 (mounted) when the sequence has group 2 and the mobile rides
(+0x1fc); a flier (gate and +0xcc & 4) 4 in war, 5 in peace; 0 in war (unless vtable +0xc0); 3 for a running
`MobTypes_CanFly` body; 6 when `FUN_005d6d30`; else 1 (peace). **confirmed**.

The package keeps the stand, walk and run actions of groups 0, 1 and 2 (`move_actions`, alternate off), so the
renderer plays them; without a sequence entry it falls back to ClassicUO's fixed UOP actions.

The move actions (stand/walk/run, `-` for a missing group) of the 300 UOP rows of `mobtypes.txt` in the 7.0.103.0
client folder, as packed (2026-11-21); 181 rows have no sequence entry, 176 of them EQUIPMENT:

| Type | War | Peace | Mounted | Bodies |
|---|---|---|---|---|
| monster | 1/0/0 | 1/0/0 | - | 714, 742, 1244, 1248 |
| monster | 1/0/24 | 25/22/24 | - | 43 bodies: 130, 270, 286, 287 .. 1485 |
| monster | 1/0/24 | 1/0/24 | - | 1428 |
| monster | 1/1/1 | 1/1/1 | - | 743, 1245 |
| monster | 1/1/1 | 25/25/25 | - | 692, 1247 |
| monster | 1/1/1 | 25/26/26 | - | 1246 |
| monster | 1/19/19 | 25/19/19 | - | 826 |
| monster | 1/22/1 | 25/22/1 | - | 832 |
| monster | 1/22/24 | 25/22/24 | - | 1026 |
| monster | 25/25/25 | 25/25/25 | - | 725 |
| animal | 1/0/24 | 25/22/24 | - | 37 bodies: 1069, 1285, 1286, 1287 .. 1542 |
| animal | 1/0/24 | 25/22/24 | 31/29/30 | 17 bodies: 1254, 1255, 1309, 1401 .. 1552 |
| animal | 1/22/24 | 25/22/24 | - | 1308, 1408 |
| animal | 1/24/24 | 25/24/24 | - | 1431 |
| animal | 25/22/25 | 25/22/25 | - | 831 |
| animal | 31/0/24 | 31/22/24 | 31/29/30 | 432 |
| human | 4/0/24 | 4/0/24 | 25/0/2 | 990 |
| human | 7/15/1 | 37/1/0 | 25/23/24 | 1253 |
| human | 7/15/3 | 37/1/3 | 25/23/24 | 666 |
| no sequence | | | | 176 EQUIPMENT rows; humans 667, 694, 695; animals 1409, 1553 |

ClassicUO's fixed UOP actions (walk 22, run 24, stand 25) match only the most common monster and animal rows; body
714, for one, stands with 1 and walks and runs with 0.

## Pacing and idle

- `GameLoop_Tick` runs `TickMobiles` when more than 75 ms passed since the last run (compared with 0x4c).
  **confirmed**.
- While a step runs, `TickMobiles` advances the frame counter +0x1a0 once a tick and counts down +0x19c, the
  step's ticks from `CMobile_StepTicks`: the `animinfo.mul` byte of the body at 0x00a59e30 (walk) or 0x00a59e31
  (run, +0x1a4), 2 bytes a body, body 0x32 for one above 1999; mounted (+0x204), the walk byte + (+0x214 / 11) * 2.
  **confirmed**.
- More than 2000 ms after the last step (+0x1ac), the mobile stands (+0x1a8) and the counter is reset to 3; bodies
  0x50 and 0x51 (`IsStandAtOnceBody`) stand at once. Standing, the counter runs only when
  `CMobile_AnimatesInPlace`: a flier with the flying gate, bodies 0xd, 0xf, 0x10, 0x23e, 0x2b4, 0x57e, 0x593,
  0x597, 0x598, 0x599 standing, or a body from 400 with a layer-2 item of tile flag 0x800000 (bodies below 400
  return before that test). A MUL stand action of 1 frame shows frame 0 anyway. **confirmed**.
- A flier with no step ticks left while the flying gate `DAT_00742cf8` is off stands at once; in peace with nothing
  playing, its +0x1ac moves back 15000 ms, which makes the idle fidget due. **confirmed** for the code,
  **probable** for the purpose.
- `CMobile_TryIdleFidget`: nothing when +0x208 is 1, the mobile rides (+0x188), the count at 0x00afb744 is above 20
  or 0x00a58bbc is set. More than 15000 ms after the last step, r = rand % (count * 4 + 120); below 2 a fidget
  plays (action, frames): monster 18 (r 0) or 17, 5; sea monster 4 with 20 or 3 with 15; animal 10 with 3 (r 0
  without flag 0x2) or 9 with 5; human or equipment 6 (r 0) or 5, 5. A UOP body plays a type-5 action of its
  sequence group. **confirmed**.

## Walk pace

How far a walk or run cycle carries the mobile, for Dynamapper's pace of the cycles. Found on 2026-11-22 with
`ProbeClassicWalkPace.java` (the field accesses) and decompiles of the functions named.

- The frame shown on the move is the counter +0x1a0 modulo the action's frame count: `GetObjectLayerFrame`
  0x005441e0 for a MUL action, and for a UOP body, after `CMobile_UpdateUopMoveAction` 0x005b6e40, the counter
  modulo the frame count at +0x224. **confirmed**.
- The counter advances once a tick, and only while a step runs (`TickMobiles`, above). A step lasts
  `CMobile_StepTicks` ticks, the `animinfo.mul` bytes. `AnimInfo_Load` 0x0056a970 reads 2000 two-byte entries and
  puts 4 (walk) and 2 (run) in an entry that is missing or 0. The installed `animinfo.mul` (4000 bytes) holds 4 and
  2 for every body. **confirmed**.
- So a step plays 4 frames walking and 2 running, whatever the body and its clip. A 10-frame walk cycle covers 2.5
  tiles and a 10-frame run covers 5; a 5-frame animal clip covers half that. At the servers' 400 and 200 ms steps
  that is about ten frames a second. The step's draw offsets (+0x1f0, +0x1f4) are the ticks left times 22 or 44,
  over the step's ticks, in the same ticks. Nothing scales the frames by the body's size or the clip's stride.
  **confirmed**.
- A server animation on a UOP body (`CMobile_SetUopAction` 0x005b6580) keeps its own clock. The frame time 1000 /
  fps is at +0x238 (62 ms without one). 0x00454d30 reads it and steps the frame +0x220 (0x00454730) each time that
  many milliseconds have passed. The move actions don't use this clock. **confirmed** for the code; **probable**
  that the list it walks (0x0075a8fc) holds the mobiles playing such an action.

By hand on the side facing, the human walk's planted foot travels about 1.35 tiles a cycle and the run's about 2.15.
The Classic client's legs therefore move about half as fast as the ground needs. ClassicUO's 80 ms a frame across
the steps (`CHARACTER_ANIMATION_DELAY`) gives 1.25 walk cycles a second, which is also short of the ground.

