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
tile definition request returns null. tiledata.mul is still loaded. This study did not trace what happens when
tiledata.mul is missing.

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
| `0x004124aa` | `ParseTileEffects` (probable) | u8 count of effects, added through vtable +0x144 |

The whole definition is built and kept. What matters is which parts are read back (section 3).

## 3. Readers

Only two functions request asset type 6:

- `GetTileDefinition` 0x00406001 (`OR EAX,0x6000000` at 0x00406051; the name is from `ghidra-classic-frames`)
- `RequestTileDefinitionUnused` 0x005f0780 (0x005f080c), which nothing references.

`GetTileDefinition` has 13 calling functions. Every definition vtable slot they call is listed below.

| Slot (UOSA name) | Callers | Use | Status |
|---|---|---|---|
| +0xbc `GetAppearance`, +0xc0 `GetAnimationAppearance` | `GetAppearance` 0x005435d0 and `GetAnimationAppearance` 0x00543000 (tile, body placeholder), called from `DrawPaperdollGumpLayers` 0x0051a4a0 (0x0051abe5, 0x0051ac20, 0x0051ae99, 0x0051aecb), `0x004cde80`, `0x005189e0` | Paperdoll and equipment gumps. If the wearer's body does not use UOP frames and the tile's animation appearance is 0, the gump comes from the tiledata anim (`DAT_00afb748 + id*0x30 + 0x10`). Otherwise the tileart appearance is used. Then `0x005624a0(body, anim)` → gump id, +10000 for a female wearer. | confirmed |
| +0xbc, +0xc0 | `GetUopFrameSet` 0x00543890, `PreloadWeaponUopFrameSets` 0x005b96a0 | Appearance of equipment on UOP-frame bodies | confirmed |
| +0xb8 `GetAlternateAnimation` | `CMobile_UpdateUopMoveAction` 0x005b6e40 (item on layer +0x12c) | UOP move action | confirmed |
| +0xb4 `GetBodyType` | `0x005b8720`, `0x005baa30` on the placeholder `10000000 + body`, from `AssetAnimation_Construct` | UOP animation sequence | confirmed |
| +0xe4 `HasSitting` | `UOClassicSitting::GetSeatGraphic` 0x005d6810 | Chair detection | confirmed |
| +0xe8 (word 0) | `UOClassicSitting::FindSeat` 0x005d6d30 (bit 0x10 test), `SnapSeatedDirection` 0x005d5620 (table `DAT_00744d84`) | Allowed seated directions | confirmed |
| +0xf4 (word 3) | `UOClassicSitting::DrawSeatOverlay` 0x005d6930 | Tested only for non-zero (`NEG/SBB/NEG`), see section 4 | confirmed |
| +0xf8 / +0xfc (words 4 / 5) | `DrawSeatOverlay` 0x005d6930, `HitTestSeatedMobile` 0x005d5450 | y shift of the seated frame buffer: word 5 for directions 0 and 6, otherwise word 4 | confirmed |
| +0x13c `GetSpriteDescriptor` | `0x00402781` (in a vtable at 0x006b8064) | The Classic slot is `0x00406146` = `XOR EAX,EAX; RET 4`, so the call always gets 0 | confirmed, dead |

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
are parsed but never read. The one override is the paperdoll and equipment gump appearance in section 3, which
tiledata cannot express (body-dependent appearance filters, UOP bodies).

## 4. Sitting (data only tileart has)

**Confirmed** from the disassembly of 0x005d6930, 0x005d6d30 and 0x005d5450.

- **Seat detection.** `FindSeat` 0x005d6d30 applies to a human body (400..1999, MobTypes human) under several state
  conditions. It walks the cell's object list for an item within |dz| < 2 that `GetSeatGraphic` accepts
  (+0xe4 HasSitting). A seat whose word 0 (+0xe8) has bit 0x10 set is skipped unless the mobile's +0x1a8 is non-zero
  (0x005d6ee5). It returns 1 for seated and keeps the seat graphic in
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
is 15), so the 0x10 test never applies to shipped data. What the mobile's +0x1a8 holds was not traced.

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
| appearance (props1 key 8) against the tiledata anim | 1,177 present, 65 differ. The Classic paperdoll uses the tiledata anim unless the body is UOP or there is an animation filter. |
| appearance filters | by body 313, by gender 39, animation by body 20 (ids 41609..). The animation filters are what moves a MUL-body paperdoll to the tileart appearance. |

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
- **tileart.uop still matters for non-art Classic packages**, because tiledata.mul does not have this data:
  - sitting: words 0, 3, 4 and 5
  - the paperdoll and equipment appearance for UOP bodies and animation filters
  - the body type and alternate animation of UOP animation
- string_dictionary.uop is needed only to resolve tileart names and texture paths. Classic reads neither for art.

## Open points

- `ParseTileEffects` 0x004124aa: the effect definitions are parsed and attached, but whether Classic ever plays a
  tile's effects was not traced.
- The mobile field +0x1a8 that lets `FindSeat` accept a seat with word 0 bit 0x10 (no shipped seat has it).
- The class of `0x00402781` (the dead +0x13c caller).
- What the client does without tiledata.mul (not traced, see section 1).
