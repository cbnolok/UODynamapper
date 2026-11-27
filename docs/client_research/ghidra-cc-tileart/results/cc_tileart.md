# Classic Client: tileart.uop against tiledata.mul

The program is `classic_7.0.103.0_client.exe` (PE version 7.0.102.3, SHA-256 `4d857f2f…690a`). Names are those
applied by `NameClassicTileartCode.java`, or by earlier studies where noted. Definition vtable slots use the UOSA
slot names from `../ghidra-tileart/results/uosa_tileart.md`, because the Classic `UOObjectDefinitionBase` vtable has
the same 85-slot layout.

## 1. Loading

**Confirmed.**

| Address | Name | What it does |
|---|---|---|
| `0x00417ec4` | `UOClassicFiles::MountUopPackages` (probable) | Mounts a fixed list of UOP packages if present: facet0-6, `tileart.uop` (string 0x006db554, ref 0x00418119), `string_dictionary.uop` (0x006db560, ref 0x00418169), AnimationDefinition, TerrainDefinition, EffectDefinitionCollection, AnimationSequence, Texture, LegacyTexture, … A missing file is skipped without an error. |
| `0x005f0e20` | `UOClassicTileArt::InitAssetSystem` (probable) | Mounts the packages, loads `Build/StringDictionary/string_dictionary.bin` (0x006bc29c), and registers the asset factories by type. Type 6 is `UOObjectDefinitionBinaryBaseFactory` (`0x00411a1e`); 0xc, 0x13, 0x19, 10 and the effect factories follow. |
| `0x0044b3e0` → `0x00542ca0` → `0x005f0e20` | | A lazy singleton (`DAT_00a57f28`) with 38 callers, among them `GetArtRecord` 0x005d7c00, `CMobile_Draw` and the paperdoll. It is created on first use, with no setting or version gate. |
| `0x00411b1d` | `UOClassicTileArt::TileDefinitionFactory_Create` (probable) | Slot +4 of the factory vtable 0x006dafd4. Errors: "Trouble loading object definition asset id #%d", "... from binary file %s". |
| `0x00411c6e` | `UOClassicTileArt::ParseTileDefinition` | See section 2. |
| `0x005b2ce0` | `UOClassicFiles::LoadTiledata` | Reads tiledata.mul, called from `0x005b5820`: 0x4000 land entries and statics up to 0x2ff940 / 0x30 = 65500 entries. It runs with or without tileart. `0x005b2870` and `0x005b2bd0` write it back (the god-client edit packet, from `0x004694b0`). |

Both loads are unconditional. If tileart.uop or string_dictionary.uop is missing, the mount skips it and every
tile definition request returns null. tiledata.mul is still loaded. Section 7 lists what degrades when either file
is missing.

## 2. Parser: same version 4 layout as UOSA, everything kept

**Confirmed.** `ParseTileDefinition` 0x00411c6e reads a u16 version and compares it with 4 at 0x00411caf. Any other
version is reported as an error. The rest is the UOSA version 4 record in the same order:

- name index, resolved in the string dictionary (`[0x01f7fde8]+0x154`, 0x1c-byte entries)
- tile id → +0x20
- the fixed fields +0x54..+0x68, body type +0xe4, alternate animation +0xe8, light +0x6c..+0x78, house bits
  (through vtable +0x14c), flags1 +0x80, flags2 +0x88, facing +0x90, Enhanced window +0x24, legacy window +0x3c

The sub-parsers run in this order:

| Address | Name | Format |
|---|---|---|
| `0x00412376` | `ParseTileProperties1` | u8 count × {u8 key, u32 value} |
| `0x00412410` | `ParseTileProperties2` | same |
| `0x004122e5` | `ParseTileStackAliases` | u32 count × {u32, u32} |
| `0x0041256f` | `ParseTileAppearanceFilters` | "Invalid animation appearance filter type" |
| `0x00412683` | `ParseTileSitting` | u8 0/1, then 6 × i32 ("The sitting data count is not 1 or 0") |
| `0x00412818` | `ParseTileRadarColor` | 4 bytes, stored through vtable +0xe0 |
| `0x00415d71` × 4 | | the four shader blocks, through setters +0xa4/+0xa8/+0xac/+0xb0 |
| `0x004124aa` | `ParseTileEffects` (probable) | u8 count of effects, added through vtable +0x144, which drops them (section 3.4) |

The whole definition is built and kept, except the effects. What matters is which parts are read back (section 3).

## 3. Readers

Only two functions request asset type 6:

- `GetTileDefinition` 0x00406001 (`OR EAX,0x6000000` at 0x00406051; the name is from `ghidra-classic-frames`)
- `RequestTileDefinitionUnused` 0x005f0780 (0x005f080c), which nothing references.

`GetTileDefinition` has 13 calling functions. Every definition vtable slot they call is listed below.

| Slot (UOSA name) | Callers | Use | Status |
|---|---|---|---|
| +0xbc `GetAppearance`, +0xc0 `GetAnimationAppearance` | `GetAppearance` 0x005435d0 and `GetAnimationAppearance` 0x00543000 (tile, body placeholder), called only from `DrawPaperdollGumpLayers` 0x0051a4a0 (0x0051abe5, 0x0051ac20, 0x0051ae99, 0x0051aecb), `BuildItemGumpForRace` 0x004cde80 and `0x005189e0` | Paperdoll and equipment gumps: section 3.1 | confirmed |
| +0xbc, +0xc0 | `GetUopFrameSet` 0x00543890, `PreloadWeaponUopFrameSets` 0x005b96a0 | Appearance of equipment on UOP-frame bodies (section 3.1, world column) | confirmed |
| +0xd4 `GetGender` (0x00405ee2) | `GenderFilter_Apply` 0x0041723c, on the placeholder | Gender filter key: property 1 key 10 | confirmed |
| +0xb8 `GetAlternateAnimation` | `CMobile_UpdateUopMoveAction` 0x005b6e40 (the item on layer 2, mobile +0x12c) | UOP move action: section 3.3 | confirmed |
| +0xb4 `GetBodyType` | `GetBodyTypeSequence` 0x005b8720 on the placeholder `10000000 + body`, from `AssetAnimation_Construct` and `MountEntry_LoadUopSequence` | UOP animation sequence: section 3.2 | confirmed |
| +0xe4 `HasSitting` | `UOClassicSitting::GetSeatGraphic` 0x005d6810 | Chair detection | confirmed |
| +0xe8 (word 0) | `UOClassicSitting::FindSeat` 0x005d6d30 (bit 0x10 test), `SnapSeatedDirection` 0x005d5620 (table `DAT_00744d84`) | Allowed seated directions | confirmed |
| +0xf4 (word 3) | `UOClassicSitting::DrawSeatOverlay` 0x005d6930 | Tested only for non-zero (`NEG/SBB/NEG`), see section 4 | confirmed |
| +0xf8 / +0xfc (words 4 / 5) | `DrawSeatOverlay` 0x005d6930, `HitTestSeatedMobile` 0x005d5450 | y shift of the seated frame buffer: word 5 for directions 0 and 6, otherwise word 4 | confirmed |
| +0x13c `GetSpriteDescriptor` | `0x00402781`, slot 0 of `UOAnimationFrameSet` (vtable 0x006b8064, RTTI `.?AVUOAnimationFrameSet@@`) | The Classic slot is `0x00406146` = `XOR EAX,EAX; RET 4`, so the call always gets 0. See section 3.5 | confirmed, dead |
| +0x144 `AddEffect` | `ParseTileEffects` 0x004124aa | Drops the effect: section 3.4 | confirmed |

No caller in this program calls these slots on a definition:

- flags1/flags2 (+0x54..+0x60)
- properties: weight, height, layer (+0x64..+0x74)
- the Enhanced and legacy windows and draw offsets (+0x14/+0x18)
- shader and texture names, scales, light, radar colour (+0xd8..+0xe0)
- stack aliases (`GetGraphicForAmount` +0xc4)
- house bits, race, gender, old id, rotation, facing

The name is used only during the parse.

### Art drawing never uses tileart

**Confirmed.** `GetArtRecord` 0x005d7c00 (`../ghidra-classic-art`) picks the land loader below id 0x4000 and the
static loader above it. Both read art.mul or `artLegacyMUL.uop`, and neither reads tileart. No art id, alias or draw
offset comes from tileart. The only tileart effect on how a static looks is the chair-back overlay (section 4),
which redraws the seat item's own art.

### tiledata.mul wins everywhere both exist

**Confirmed.** Flags, height, weight, layer and anim come from the in-memory tiledata table: statics are 0x30 bytes
at `DAT_00afb748`, with flags u64 @0, weight @8, layer @9, anim @0x10 and height @0x16. Tileart flags and properties
are parsed but never read. The one override is the paperdoll and equipment gump appearance in section 3.1, which
tiledata cannot express (body-dependent appearance filters, UOP bodies).

### 3.1 Paperdoll and equipment gumps

**Confirmed** from the disassembly of `DrawPaperdollGumpLayers` 0x0051a4a0 (worn layers 0x0051ab60..0x0051ac27, the
held item 0x0051ae68..0x0051aed2), with the same code in `BuildItemGumpForRace` 0x004cde80 (0x004cdf48..0x004cdf8d)
and `0x005189e0` (0x00518c7c..0x00518cc1).

**The paperdoll body.** `DrawPaperdollGumpLayers` converts the mobile's body (+0x38) through `body.def` (the table at
`DAT_00751880`, `0x00446fa0`) and switches on the result (0x0051a6d4..0x0051a8cf):

| Body after `body.def` | Paperdoll body | Female |
|---|---|---|
| 400, 0x2e8 | 400 | no |
| 401, 0x2e9 | 401 | yes |
| 605 / 606 | 605 / 606 | 606 |
| 666 / 667 | 666 / 667 | 667 |
| 987 | 400 or 401 by the mobile's female bit | |
| 0x4e5 (1253) | 1253 | no |
| any other | by race (mobile +0x284): 1 → 400/401, 2 → 605/606, 3 → 666/667 | mobile +0xcc bit 1 |

So the paperdoll body is always one of 400, 401, 605, 606, 666, 667, 1253. `0x005189e0` takes the same race rule but
passes the raw +0x38 to `EquipConvGump`; `BuildItemGumpForRace` takes race and gender from its caller (two packet
handlers, 0x0045fa40 and 0x0045fdc0, under the dispatcher 0x00483af0; which packets was not identified).

**The rule, per item on a layer** (`graphic` = the item's tile id, `P` = the placeholder definition
`10000000 + paperdoll body`):

```
if (!BodyUsesUopFrames(body)) {                                   // 0x005978a0: mobtypes.txt flag 0x10000
    if (GetAnimationAppearance(graphic, body) != 0)               // 0x00543000 -> tile vtable +0xc0, 0x0051abe5
        anim = GetAppearance(graphic, body);                      // 0x005435d0 -> tile vtable +0xbc, 0x0051ac20
    else
        anim = tiledata[graphic].anim;                            // DAT_00afb748 + graphic*0x30 + 0x10
} else
    anim = GetAppearance(graphic, body);                          // 0x0051ac20
gump = EquipConvGump(body, anim);                                 // 0x005624a0
if (female && GumpSize(gump + 10000) != 0) gump += 10000;         // 0x00567d40 at 0x0051ac40, 0x0051ac57
if (layer == 21 && own mobile && DAT_0098faa4 in 1..3) gump = 0x777b..0x777d;    // backpack style
if (!gump record (file 7, 0x0056e000) && gump.def has gump) {     // 0x0051acd3, gump.def table DAT_00751864
    if (hue == 0) hue = gump.def hue;                             // 0x00452a90
    gump = gump.def target;                                       // 0x00446fa0
    if (gump < 60000 && female && GumpSize(gump + 10000) != 0) gump += 10000;
}
```

`EquipConvGump(body, anim)` 0x005624a0 looks up the `Equipconv.def` row (body, anim) (`FUN_005623d0`, map at
0x00a58d64): gump column > 0 → that gump; −1 → converted id + 50000; 0 or no row → anim + 50000. So the "item id +
50000 / 60000" rule is really anim + 50000, plus 10000 for a female when that gump exists, and the anim is the
tiledata anim or the tileart appearance as above. `gump.def` comes last and only replaces a gump id that has no
record.

**The tileart values.** Both are read on the item's definition with the placeholder `P` as the owner:

- `GetAppearance` (+0xbc, 0x00405bf1): the stored tile id, replaced by the tile's property 1 key 8 when present, then
  the first filter of type 0 or 1 that matches.
- `GetAnimationAppearance` (+0xc0, 0x00405ce4): 0, then the first type 2 filter that matches. In the paperdoll it is
  only a switch; its value never becomes the gump.
- A missing tile or placeholder definition gives 0 for both: a MUL body falls back to tiledata, a UOP body gets
  anim 0.

**The appearance filters** are lists on the item's tileart record:

| Type | Apply | Matches when | Records |
|---|---|---|---|
| 0, by body | `BodyFilter_Apply` 0x0041717f | the key equals `P`'s property 8, else `P`'s stored id (10000400 …) | 313 items |
| 1, by gender | `GenderFilter_Apply` 0x0041723c | the gender byte equals `P`'s gender, property 1 key 10 (`GetGender` +0xd4): 0 for 400, 605, 666, 1253; 1 for 401, 606, 667 | 39 items |
| 2, animation by body | `AnimationBodyFilter_Apply` 0x00417287 | as type 0 | 20 items, 41609..41800 |

The keys are body placeholders only (shipped: 10000400, 10000401, 10000605, 10000606, 10000666, 10000667). Race is
not a key: the placeholders' property 9 holds it, and no filter reads it. Ghost placeholders 694 and 695 carry
property 8 = 10000666 / 10000667, so they match the gargoyle keys.

**Per body source** (the item worn; "world" is the layer item drawn on the mobile, from
[uop_reachability.md](../../ghidra-classic-frames/results/uop_reachability.md#1-what-the-fields-hold-and-what-each-caller-passes)):

| Wearer | Paperdoll anim | World layer frames |
|---|---|---|
| MUL frames, `mobtypes.txt` without 0x10000: humans 400-403, elves 605-608 (0x20000), and every other MUL body, which the paperdoll maps to 400/401/605/606 by race | tiledata anim, unless a type 2 filter of the item matches `P`; then `GetAppearance` | tiledata anim → `body.def`, `Equipconv.def` (`FUN_00562480`) → `GetMobileMulFrame` (`body.def`, `Bodyconv.def`) → MUL; tileart is not read |
| A body remapped by `body.def` | the paperdoll body is taken after `body.def`, then the rule above for that body | the world tests the raw body for UOP and converts only on the MUL side |
| `Bodyconv.def` | not used by the paperdoll | picks the MUL file of the converted anim |
| `Equipconv.def` | `(paperdoll body, anim)` → gump | `(body, anim)` → converted anim |
| UOP frames (0x10000): 666, 667, 694, 695, 1253 and the 0x10000 monsters | always `GetAppearance` (property 8, else the tile id, overridden by a type 0/1 filter); tiledata anim never | `GetUopFrameSet`: `GetAppearance` overridden by a non-zero `GetAnimationAppearance` (its value is used here) |

So the human paperdoll does read tileart: it asks `GetAnimationAppearance` for every layer item, which loads the
item's and placeholder 10000400's definitions. The answer changes the outcome only for the items with a matching
type 2 filter (14 for humans and elves, section 5), and on the world the same human draws the tiledata anim for
those items. A gargoyle's paperdoll never reads tiledata's anim.

### 3.2 Body type (+0xb4)

**Confirmed.** Read on placeholders only, by `GetBodyTypeSequence` 0x005b8720: the placeholder's body type selects
the `AnimationSequence.uop` asset (type 0xc, `0x00409410`), cached by body type (`DAT_00afb760`). Two callers:

- `AssetAnimation_Construct` 0x005baf50, for a mobile whose body has `BodyHasUopSequence` (flags & 0x30000), beside
  the choreography (`0x005baa30`). That is the 300 UOP bodies (0x10000) and the eight MUL-frame bodies with 0x20000:
  400-403 and 605-608, all with body type 400. For those eight the sequence only serves `GetUopFrameSet` through
  `PreloadWeaponUopFrameSets` (a keep-loaded preload, see uop_reachability.md section 4); their frames and move
  actions stay MUL. For the 0x10000 bodies it is the move action and frame timing of section 3.3. 179 of the 298
  placeholders of 0x30000 bodies name another body type (162 → 666, e.g. 320-344; 16 → 400, e.g. 401-403 and
  605-608; one → 1549). The 10 such bodies without a placeholder get no sequence (1038, 1039, 1409, 1492, 1496,
  1503, 1505, 1508, 1509, 1553).
- `MountEntry_LoadUopSequence` 0x005d3e90, built on first use by `GetMountEntry` 0x005d4020: the mount table at
  0x00744940 (41 entries of 0x18 bytes, mount body at +0x12, indexed by the mount item graphic through
  `MountItemToIndex` 0x005d3c10). Only when the mount body has 0x10000 (`BodyUsesUopFrames`).

A MUL body without 0x20000 never reads its body type.

### 3.3 Alternate animation (+0xb8)

**Confirmed.** Read only by `CMobile_UpdateUopMoveAction` 0x005b6e40, on the item on layer 2 (mobile +0x12c), and
passed to `FrameSetLookup_GetMoveAction(state, alternate)`. That function runs only from `GetObjectLayerFrame`
0x005441e0 for a 0x10000 body with no server animation playing, so the flag changes UOP bodies' stand, walk and run
actions and nothing else. 22 item records set it, all two-handed layer 2 weapons (2301, 2308-2310, 2312, 5182-5183,
16488, 16493, 16495-16496, 16501, 18608-18613, 18616-18617, 18628-18629). Eight placeholder records also set it;
nothing reads it there.

### 3.4 Tile effects are dropped

**Confirmed.** The tile definitions are `UOObjectDefinitionBase` objects (RTTI `.?AVUOObjectDefinitionBase@@`,
vtable 0x006da914, set by `0x00405426`, which the factory's create `0x004119be` calls). Its +0x144 slot,
`TileDefinition_AddEffect` 0x00404ffa, only releases the shared pointer it receives (`0x00402455`) and stores
nothing. `ParseTileEffects` reads the effects and hands each one there, so no tile effect survives the parse and the
Classic client never plays one.

### 3.5 The +0x13c caller

**Confirmed.** `0x00402781` is slot 0 of `UOAnimationFrameSet` (vtable 0x006b8064, constructors 0x005b73b0 and
0x005b9110). For a frame index it returns the cached entry (0x14-byte entries at +0x2c); when the entry is empty it
fetches the tile definition named by the entry (+8) and stores its +0x13c(0), which is always 0 in Classic. Nothing
calls slot 0 directly; a virtual caller was not searched.

## 4. Sitting (data only tileart has)

**Confirmed** from the disassembly of 0x005d6930, 0x005d6d30 and 0x005d5450.

- **Seat detection.** `FindSeat` 0x005d6d30 applies to a human body (400..1999, MobTypes human) under several state
  conditions. It walks the cell's object list for an item within |dz| < 2 that `GetSeatGraphic` accepts
  (+0xe4 HasSitting). A seat whose word 0 (+0xe8) has bit 0x10 set is skipped unless the mobile's +0x1a8 is non-zero
  (0x005d6ee5). +0x1a8 is the standing flag, set once more than 2000 ms have passed since the last step (+0x1ac)
  ([classic_frames.md](../../ghidra-classic-frames/results/classic_frames.md), `NameClassicMoveCode.java`). So a
  bit 0x10 seat takes only a mobile that stands still. It returns 1 for seated and keeps the seat graphic in
  `DAT_00afff38`.
- **Callers of `FindSeat`.** `CMobile_Draw` (0x004530a7), `CMobile_DrawVariant`, `ObjectFrameHitTest`,
  `CMobile_ComputeBounds(Variant)`, `CMobile_UopSequenceGroup` and `0x0044d980`.
- **Drawing a seated mobile.** A seated MUL-frame body is drawn by `DrawSeatedMobile` 0x005d6f50, and a UOP body by
  `0x005d6800`.
- **Direction.** `SnapSeatedDirection` 0x005d5620 snaps the mobile's direction to word 0. It is used by the draw
  path and the hit test (`0x005d5b00`).
- **y shift.** `DrawSeatOverlay` 0x005d6930 builds a 64×128 seated frame buffer and moves it in y by word 5
  (+0xfc) for directions 0 and 6, otherwise by word 4 (+0xf8). `HitTestSeatedMobile` 0x005d5450 applies the same
  shift.
- **Chair-back overlay, directions 0 and 6 only.** The seat item on the same cell (|dz| ≤ 1, same graphic) is
  redrawn over the mobile, clipped at the item's horizontal centre:
  - The clip edge moves by `shift` = 1 when word 3 (+0xf4) is non-zero, otherwise 0. The value of word 3 is not used.
  - Graphics 0x121a-0x121b force `shift` 3.
  - 0x2de5-0x2de6 and 0x9c57..0x9c62 force 0, so no overlay.
  - For direction 0 the right edge is clipped; for direction 6, the left.

So Classic reads sitting words 0, 3 (as a flag), 4 and 5. UOSA reads words 0-3, and its words 4-5 have no reader.
Words 1 and 2 (UOSA's seat x/y offsets) are not read by Classic. No shipped word 0 reaches bit 0x10 (the largest
is 15), so the 0x10 test never applies to shipped data.

## 5. Measured on the plain Classic folder

Measured with `../measure_cc_tileart.py` on 2026-11-27, on `_Ultima Online Classic` with tileart.uop
5,693,165 bytes, string_dictionary.uop 388,457 and tiledata.mul 3,188,736.

### Records

| Measure | Result |
|---|---|
| tileart records | 37,755: 36,611 items (ids 0..44918) and 1,144 placeholders (10000000+). One more record is named outside the ids the client looks up. |
| Records whose stored tile id differs from their entry name | 25 (39777→39676, 43957..43963→43956, 43965..43974→43964, 10000584→100000584, …). The client keys by entry name. |
| tiledata statics | 65,536 in the file. The client reads 65,500. |
| tileart items outside tiledata, or at or past the client's bound | 0 |
| Non-empty tiledata entries without a tileart record | 0 |
| tileart items whose tiledata entry is empty | 2,410 (e.g. 0, 506-510, 576, 651-656). Classic has no data for them, and their art is the client's concern. |

### Values the client takes from tiledata.mul, compared

These tileart values are never read by Classic. They show what a tileart-only source would get wrong.

| Measure | Result |
|---|---|
| flags1 ≠ tiledata flags | 10,843 of 36,611 |
| Bits set only in tiledata | 0x4000 (article "a") 8,209; 0x4000000 886; 0x80000 795; 0x20 553; 0x1000000 (animation) 539; 0x8000 (article "an") 522; 0x4 275; 0x8 271; 0x80 65; … |
| Bits set only in tileart | 0x400000000 1,658; 0x800000000 440; 0x200000000 98; 0x40 29; 0x40000 25; 0x2000 22; 0x10 19; 0x1000000 15; … |
| height (props1 key 3) | 95 differ; 8 have no key where tiledata is non-zero |
| weight (props1 key 0) | 70 differ; 35 missing |
| layer (props1 key 6) | 7 differ (17069-17072, 19358-19360); 1,737 missing |
| appearance (props1 key 8) against the tiledata anim | 1,177 present, 65 differ (e.g. 3570: anim 980, key 8 981; 3721: 648 / 628). None of the 65 has a type 2 filter, so no MUL-frame paperdoll sees them; all 65 reach 666/667 (64 by the key 8 value, one through a body filter) and 1253. |
| appearance filters | by body 313, by gender 39, animation by body 20 (ids 41609..). The animation filters are what moves a MUL-body paperdoll to the tileart appearance. |

### Paperdoll gumps per body

The paperdoll rule of section 3.1 replayed over the 1,126 wearable tiledata items (flag 0x400000, ids below 65500),
with `mobtypes.txt`, `Equipconv.def` and the 4,456 gumps of `gumpartLegacyMUL.uop`. "Tiledata only" is the same rule
with the tiledata anim in place of every tileart value. Every wearable item has a tileart record and every paperdoll
body a placeholder.

| Body | `mobtypes.txt` | Type 2 filter matches | anim ≠ tiledata anim | Gump differs (a gump present) |
|---|---|---|---|---|
| 400 | 0x20000, MUL | 14 | 8 | 8 (8) |
| 401 | 0x20000, MUL | 14 | 14 | 14 (14) |
| 605 | 0x20000, MUL | 14 | 8 | 8 (8) |
| 606 | 0x20000, MUL | 14 | 14 | 14 (14) |
| 666 | 0x10000, UOP | not used | 198 (110 by filter, 64 by key 8, 24 by tile id) | 195 (170) |
| 667 | 0x10000, UOP | not used | 249 | 240 (215) |
| 1253 | 0x10000, UOP | not used | 89 (65 by key 8, 24 by tile id) | 89 (64) |

The 14 items of a MUL-frame body are 41609-41611, 41613-41619 (whips, holiday boots and hats) and 41793-41796
(skull sword and staff). Their type 2 filter returns the tiledata anim itself, so it acts as a switch to the
appearance: the male whips 41610/41611/41618/41619 take 1490/1494 instead of 1486, the skull sword 9986 instead of
618 (gump 59986, not 50618), and for women all 14 take their female appearance (e.g. 41609 → 61487 instead of
51486). Every other wearable gets the tiledata anim on these bodies. Type 2 filters keyed to gargoyles (41620,
41622, 41797-41800) are never read by the paperdoll, since 666/667 use UOP frames.

### Art data (what the tex_art_cc fallback uses)

| Measure | Result |
|---|---|
| Legacy (shader block 1) texture id ≠ entry id | 23: 5723→5711, 39777→39676, 39854→39852, 40146→40147, 42091→41516, 42092→41517, 43957..43963→43956, 43965..43974→43964. All 23 have their own art record, and Classic draws that (e.g. 5723 is 44×56 in art.mul; 5711 is 88×111). |
| Legacy window ≠ (0, 0, art width, art height) | 3,216. These are Enhanced legacy textures of another size, e.g. 644: art 43×44, window 56×56, x offset −6. |
| Legacy x offset = −((w − 44) // 2) for art wider than a tile, else 0 | 31,502 of the 33,304 art-sized windows. It centres wide art on the tile, which Classic gets from the art width alone. |
| Legacy y offset non-zero | 181 (−1 ×141, −2 ×31, −10 ×5, 5 ×2, 20 ×2) |
| Items with sitting data | 170. The word distributions are identical to UOSA's: word 3 {0: 140, 1: 14, ±6: 12, 3: 3, −2: 1}; word 4 {0: 67, 4: 24, 3: 24, …, −45: 5}; word 5 {0: 40, 10: 33, 4: 25, 8: 24, …}. |
| Alternate animation set | 30 records (2301, 2308-2312, 5182-5183, 16488, …) |
| Item body types | 400 ×36,484, 666 ×127. Classic reads the body type only on placeholders. |
| Items with stack aliases | 2. Classic never reads them. |

## 6. Consequences for the converter

- **`tex_art_cc`.** Requiring tiledata.mul for Classic art loses nothing the Classic client uses: the client never
  reads tileart for art. The `TileartUop` fallback is not Classic-faithful:
  - 23 slots would show another item's art.
  - 3,216 offsets describe textures of another size.
  - About 1,800 offsets are Enhanced-authored adjustments that Classic does not apply.

  Done 2026-11-27 (the user's decision): the fallback is removed, and a Classic folder without tiledata.mul is an
  error naming the folder. A Classic art package of the plain Classic folder is byte-identical to before.
- **tileart.uop still matters for non-art Classic packages**, because tiledata.mul does not have this data:
  - sitting: words 0, 3, 4 and 5
  - the paperdoll appearance: for UOP bodies (666, 667, 1253) always, for MUL-frame bodies on the 14 items with a
    matching type 2 filter
  - the body type and alternate animation of UOP animation (0x10000 bodies, UOP mounts)
- string_dictionary.uop is needed only to resolve tileart names and texture paths. Classic reads neither for art.
- **The Classic gump paperdoll.** Done 2026-11-27: Dynamapper's Classic gump paperdoll follows section 3.1
  (`uocf::classic::paperdoll_gump`). `tilemeta.uddp` carries the type 2 filters (`metadata/item_definitions.bin`
  kind 2) and the Classic folder's `mobtypes.txt` UOP-frame bodies, `Equipconv.def` and `gump.def`
  (`metadata/classic_paperdoll.bin`). Replayed over the 7 paperdoll bodies and the 1,126 wearables (7,882 pairs) of
  the plain Classic folder against this script's rule, the anim is the same for all and the gump for all but the 109
  whose gump is missing and has a `gump.def` row, which the script does not apply (e.g. 1253 on 12652: 50760 → 50524).
  Where `gump.def` lists several targets the client picks one at random; Dynamapper takes the first. The paperdoll
  bodies come from the profile (`PaperdollProfiles.kdl`), so `body.def` is not applied. The Enhanced gump paperdoll
  keeps the tiledata anim and the profile offsets.

## 7. Missing files

**tiledata.mul missing.** Confirmed from the code, not run. `FUN_005653f0` opens files: a name listed in the
client's path table (`DAT_00a59d9c`, filled elsewhere) that fails to open is fatal (`0x005f1340`, "fatal: ", exit);
otherwise it searches the data folders and returns null. `LoadTiledata` then reads nothing and reports nothing. Its
two tables were allocated with `malloc` (0x005b5870, 0x005b587f) and are never cleared, so every flag, height,
weight, layer, anim and name is whatever the allocation held. Fresh blocks of this size are usually zero pages,
which would make every tile flagless with height 0 and anim 0 (**probable**). Nothing falls back to tileart: no code
reads tileart flags or properties.

**tileart.uop (or string_dictionary.uop) missing.** Confirmed from the code, not run. Every tile definition request
returns null, and:

- Art, flags, heights and the MUL world drawing are unchanged.
- Sitting: `GetSeatGraphic` returns 0, so no mobile sits and no chair back is drawn over it.
- MUL-frame paperdolls: `GetAnimationAppearance` is 0, so all items take the tiledata anim (the 14 items above show
  their tiledata gump).
- UOP-frame paperdolls (666, 667, 1253): `GetAppearance` is 0, so every item's anim is 0 and its gump 50000 before
  the female and `gump.def` steps.
- UOP animation: no placeholders, so `GetUopFrameSet` is empty and 0x10000 bodies and their layer items draw nothing
  (the flagged route has no MUL fallback); `AssetAnimation_Construct` gets no sequence, so no UOP move action or
  timing; the alternate animation is off; UOP mounts get no sequence.

## Open points

- Which packets reach `BuildItemGumpForRace` 0x004cde80, and what `0x005189e0` is for (it keeps gumps 50001..115000
  per layer; probably the paperdoll hit test).
- `0x00515df0`, called with the gump id between the female step and the record check in `DrawPaperdollGumpLayers`
  (0x0051ac6f), was not read; it may change the gump. Dynamapper's Classic paperdoll does not apply it.
- Whether anything calls `UOAnimationFrameSet` slot 0 virtually.
- The missing-file behaviour of section 7 was read, not run.
