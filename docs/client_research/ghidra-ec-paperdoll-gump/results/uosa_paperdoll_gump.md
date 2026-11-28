# The Enhanced Client's gump paperdoll: which gump each worn item draws

Program `/UOSA.exe` 4.0.102.3, SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`. Every
result is **confirmed** (read in the assembly or the decompiler output) unless marked **probable**. The
measurements are the output of [`../measure_ec_paperdoll_gump.py`](../measure_ec_paperdoll_gump.py) of 2026-11-28.

The old-style paperdoll is the third paperdoll texture class, `PaperdollTextures::LegacyPaperdollTexture` (vtable
0x00ca92d8). The two others draw `Paperdoll.uop` frames ([ghidra-paperdoll](../../ghidra-paperdoll/README.md)):
`ECPaperdollTexture` (0x00ca9298) and `CCPaperdollTexture` (0x00ca92b8). Its Build, slot 2, is the function the
earlier studies call `UOPaperdoll::BuildImage`, 0x0050c790. It draws Interface.uop gumps on the CPU through
`UOPaperdoll::DrawGumpLayer` 0x00509b30, whose hue rules are in
[uosa_paperdoll.md](../../ghidra-paperdoll/results/uosa_paperdoll.md#partial-hue-on-the-gump-paperdoll).

## 1. Functions

| Address | Name | Role |
|---|---|---|
| 0x0050c790 | `UOPaperdoll::BuildImage` | `LegacyPaperdollTexture` slot 2: composes the paperdoll |
| 0x0050aef0 | `PaperdollTextures::ResolvePaperdollMobile` (probable name) | the mobile, its body and hue; shared with `ECPaperdollTexture::Build` 0x0050b340 |
| 0x0050d1c0 | `SelectPaperdollBodyGumpForBody` (name kept) | body -> body gump, ghost and female flags |
| 0x0050d010 | `LegacyPaperdollTexture::GetLayers` | slot 5: the worn layers in draw order |
| 0x00501340 | `UOPaperdoll::BuildLegacyLayerOrder` | the draw order |
| 0x005012c0 | `UOPaperdoll::MoveLayerNextTo` (probable name) | order tweak |
| 0x00501240 | `UOPaperdoll::MoveLayerAfter` (probable name) | order tweak |
| 0x005099f0 | `UOPaperdoll::GetLayerItemAppearance` | props1 Appearance of a layer's graphic |
| 0x00403050 | `UOGameData::LoadGameData` (probable name) | "Loading game data...": the `data/gamedata` tables |
| 0x00402d80 | `UOGameData::LoadEquipConvCsv` | reads `data/gamedata/equipconv.csv` |
| 0x00404220 | `UOGameData::FindEquipConv` | finds an (item, body) row |
| 0x004084f0, 0x00404150 | `UOGameData::EquipConvLowerBound`, `InsertEquipConv` (probable names) | the map's lower bound and insert |
| 0x0051af10 | `UOGumpArt::RequestGumpArtTexture` (earlier name) | the gump texture; its cache key is `id & 0xffff` |
| 0x005bda40 | `UOTileArt::GetObjectDefinition` (earlier name) | `GetAsset(id, type 0x06000000)`: the record named `build/tileart/%08d.bin`, or none |

Definition reads go through the `UOObjectDefinition` vtable 0x00ca5b4c: `HasFlags2` +0x5c (0x004c95c0, tests the
u64 at +0x88, the record's flags2), `HasProperty1`/`GetProperty1` +0x64/+0x68, `HasProperty2`/`GetProperty2`
+0x6c/+0x70.

## 2. The gump of a worn item

`BuildImage` loops over the layers `GetLayers` returns (section 4). For each layer:

1. **The item.** The item of the layer's serial (`FUN_005bd9d0`); no item, no layer. Its graphic is the item's
   graphic (vtable +0x60). On the face layer (15) the item's display graphic (+0x68) is taken instead when it is
   not zero (0x0050ca8c).
2. **The definition and the number.** The item graphic's definition (`GetObjectDefinition`). When it exists:
   - props2 key 11 (Paperdoll) present: number = its value, definition = the record of value + 10,000,000
     (0x0050cb29);
   - else props2 key 8 (Appearance) present: the same with its value (0x0050cb73);
   - else the number is the graphic and the definition the item's own record.

   The number loses 10,000,000 when above it (0x0050cbbc). props1 is not read here: the props1 Appearance the 3D
   paperdoll and the world read plays no part. The appearance filters (`GetAppearance`, vtable +0xbc) are not
   read either, so neither the wearer's body nor its gender changes the number.
3. **equipconv.csv.** `FindEquipConv(number, body)` (0x0050cbf9). A row's convert value, when positive, replaces
   the number and the definition (the record of convert + 10,000,000, then one more step through its props2 11, props2
   8, props1 11 or props1 8). The row's gump and hue, when positive, replace the gump and the item's hue.
   `LoadEquipConvCsv` reads `data/gamedata/equipconv.csv` (a header line, then body, item, convert, gump, hue).
   The file is in no package of the EC, EC fp or KR folders: a scan of every `.uop` for the entry's hash finds
   nothing, while `data/gamedata/skilldata.csv`, the control, is found in `GameData.uop`. So the map is empty and
   this step never applies to the shipped data.
4. **No definition, no layer.** When the final definition is null the layer is skipped (`TEST ESI,ESI` at
   0x0050cda5).
5. **Partial hue.** `HasFlags2(0x40000, 0)` of the **final** definition (0x0050cdad): flags2 bit 18, not the
   tiledata flags and not the item record's flags.
6. **The gump.** The equipconv gump when set. Otherwise number + 50,000. When the body is female, number + 60,000 is
   tried first (`LEA EBP,[EDI+0x2710]`, 0x0050ce18). Its `DrawGumpLayer` returning false (no gump, or a 256x256
   one) falls back to number + 50,000 (0x0050ce2f). There is no `gump.def` and no other fallback: a missing gump
   draws nothing.
7. **The hue.** The item's hue (vtable +0x58), or the equipconv hue.

`RequestGumpArtTexture` keys its cache by `id & 0xffff`. **probable**: a number + 60,000 past 65,535 asks for a
16-bit gump id that wrapped around.

The rule depends on the body only through the female flag and the empty equipconv map.

## 3. The body, its gump and the canvas

- **Mobile and body** (`ResolvePaperdollMobile`). It reads the mobile of serial +0x48, its graphic (+0x50) and its
  hue (+0x54). If the body's definition has props1 Paperdoll (11) or Appearance (8), the body takes that value
  (+ 10,000,000). Other bodies take the race table at 0x00c9b12c, indexed [race * 2 + female] (race 0 and 1
  human 400/401, 2 elf 605/606, 3 gargoyle 666/667).
- **Body hue.** Bodies 987 and 1253 are drawn with hue 1002 (0x3ea). The others take the mobile's hue.
- **Body gump** (`SelectPaperdollBodyGumpForBody`):

  | Body | Gump | Flags |
  |---|---|---|
  | 400, 744 | 12 | |
  | 401, 745 | 13 | female |
  | 402 / 403 | 12 / 13 | ghost (/ female) |
  | 605 / 606 | 14 / 15 | (female) |
  | 607 / 608 | 14 / 15 | ghost (/ female) |
  | 666 / 667 | 666 / 665 | (female) |
  | 694 / 695 | 666 / 665 | ghost (/ female) |
  | 1253 | 51253 | |
  | 987, any other | 12, or 13 when the player is female | **probable**: the gender of the player (`FUN_0042eff0` flags +0xa0 bit 1), not of the mobile shown |

- **Canvas.** The paperdoll image is the body gump's size. All shipped body gumps are 260x237.
- **Positions.** The body is drawn first at 0,0, with partial true. Each layer is copied at 0,0, its rows clipped
  to the canvas. There is no per-body or per-race offset. Dynamapper's `PaperdollProfiles.kdl` offsets (all 0) and
  canvases (260x300, gargoyles 300x340) are not a client concept.
- **Ghosts** skip the robe layer (22).

## 4. Layer list and draw order

`GetLayers` collects the graphic of every worn layer and orders the layers with `BuildLegacyLayerOrder`. The
paperdoll then skips the backpack (21) and the mount (25), and the robe (22) on a ghost body. Nothing else is
taken off: there is no robe or cloak hiding, and every remaining worn layer is drawn.

The order (bottom first) is a hard-coded table, not `wearables.xml`:

| Table | Order | Taken when |
|---|---|---|
| 0x00c9b688 | 20 5 4 3 24 19 13 17 8 14 15 7 23 22 12 10 11 16 18 6 1 2 21 9 25 | default |
| 0x00c9b66c | 20 5 4 3 24 13 17 8 14 15 19 7 23 22 12 10 11 16 18 6 1 2 21 9 25 | arms (19) appearance 975, 528, 947 or 989, or chest (13) appearance 538 |
| 0x00c9b6a4 | 20 13 5 4 3 24 17 8 14 15 19 7 23 22 12 10 11 16 18 6 1 2 21 9 25 | chest appearance 921..925 |

That is: cape, torso, legs, feet, leg armour (24), arms, chest, above chest, ring, bracelet, face, hands, skirt,
robe, waist, neck, hair, beard, earrings, head, the two hands, then backpack, talisman and mount.

The appearances are props1 Appearance (`GetLayerItemAppearance` 0x005099f0), not props2. Then come the moves.
`MoveLayerNextTo(a, b)` (0x005012c0) puts b just before a when b came after it, but just after a when b came before
it. **probable** that the asymmetry is an oversight; it only orders b next to a. `MoveLayerAfter(a, b)` (0x00501240)
puts b just after a.

- torso (5) worn and legs (4) appearance 920: `MoveLayerNextTo(5, 4)`;
- legs appearance 512, 491 or 506: feet (3) before legs;
- legs appearance 1299 or 1300 with feet worn, or feet worn and legs appearance 996: `MoveLayerNextTo(3, 4)`;
- above chest (17) appearance 568: `MoveLayerAfter(12, 17)`; then robe (22) appearance 1256..1259 or 1506..1509:
  `MoveLayerNextTo(10, 22)`;
- cape (20) appearance 896 or 1523: `MoveLayerAfter(22, 20)`;
- head (6) appearance 513 or 425 and neck (10) appearance 456, 471 or 472: `MoveLayerAfter(6, 10)`, and the
  order is final;
- head appearance 1513 or 1514 with robe appearance 1506..1509: `MoveLayerNextTo(22, 6)`.

The default table is ClassicUO's paperdoll order; the moves are the Enhanced Client's own.

## 5. Measurements

`python3 -I -B measure_ec_paperdoll_gump.py ENHANCED_DIR CLASSIC_DIR`. Wearables are the items tiledata flags
wearable (0x400000). The old rule is dynamapper's before 2026-11-28: tiledata anim + 50,000, a female profile
anim + 60,000 falling back to + 50,000, partial hue from the item's flags.

| | EC (`_Ultima Online Enhanced` + Classic tiledata) | EC fp (`... fp` pair) |
|---|---:|---|
| wearables | 1,126 | 1,264 |
| number from props2 Paperdoll / props2 Appearance / own id | 0 / 1,102 / 24 | 0 / 1,240 / 24 |
| props1 Appearance without props2 Appearance | 0 | 0 |
| number != tiledata anim | 24, the own-id items (anim 0) | 24, the same |
| props2 Appearance whose 10,000,000 + value record is missing (drawn by the old rule, not by the client) | 47 | 59 |
| partial hue of the final definition != tiledata partial hue | 89 | 173 |
| 256x256 gumps | 0 | not measured (BWT-packed Interface.uop) |
| male: drawn by the client / the old rule | 949 / 996 | 1,077 / 1,136 |
| female: drawn by the client / the old rule | 988 / 1,033 | 1,116 / 1,173 |
| a gump other than the old rule's, both drawn | 0 | 0 |

- **Same gumps.** Every props2 Appearance equals the tiledata anim. Where both rules draw, they draw the same gump
  for every item, male and female, in both folders. The client's rule changes no gump on the shipped data.
- **Own-id items.** The 24 own-id items (5703, 7110, 7111, 9594, ..., 22145) have no tiledata anim and no props2
  Appearance. The client numbers them by their graphic: number + 50,000 is no gump, so nothing is drawn. For a
  woman, number + 60,000 passes 65,535 (**probable** wrap). 7110 and 7111 (scale shields) then find gumps 1574
  (326x320) and 1575 (214x72), unrelated gumps the client would draw over the paperdoll. The old rule rejected these
  items ("no paperdoll anim_id") and failed the whole paperdoll.
- **No appearance record.** For 47 items (59 in fp) the record at 10,000,000 + props2 Appearance is missing, and
  the client draws nothing for them although their gump exists. They are backpacks (layer 21, never drawn anyway),
  wand 3570, talismans (4246, 12120..12123, 40485..40492), ring 7945, amazon armour 9817/9818, the samurai mempo,
  hatsuburi, kabuto and ninja mitts (10100..10216), hide and leather gloves (11125, 12230, 12652, 12668), the
  gargish necklace, bracelet, ring and earrings (16912..16915, 19722, 19723), epaulets (39301, 39302, 41674,
  41675), and the octopus and fellowship necklaces (41801, 41802, 42025, 42026).
- **Partial hue.** No appearance record (10,000,000 and up) has any flags2 bit: of 1,144, none has bit 18. Only
  10 have flags1 bit 18. So the client draws every worn item of the gump paperdoll fully hued, except the 24 own-id
  items, which read their own record. The 89 (fp 173) differences are all items flagged partial hue in tiledata
  and in their own tileart record (flags1 and flags2): hats (5907..5916), the jester's cap and suit, skullcaps, wands,
  pole arms, kite shields, the fancy dress, bead necklace, bracelet, the GM robe and more. Their non-grey parts
  take the item's hue in the client.
- **Body gumps.** 12, 13, 14, 15, 665 and 666 are 260x237 in the EC folder. Of the gumps the client draws, 931 are
  260x237 and 18 are 260x300: the gargish weapons 50930..50940 (scythe, scepter, bladed staff, pike, double bladed
  staff, lance, crescent blade, composite bow, paladin sword). Their bottom 63 rows fall off the client's
  260x237 canvas.

## 6. Against dynamapper's Enhanced gump paperdoll

| | Client | Dynamapper before 2026-11-28 |
|---|---|---|
| number | props2 Paperdoll, else props2 Appearance, else the graphic | tiledata anim (equal on the shipped data; anim 0 failed the paperdoll) |
| gump | number + 50,000; female + 60,000 first | the profile's offsets (50,000; female 60,000 then 50,000): the same |
| female | from the body (`SelectPaperdollBodyGumpForBody`) | from the profile |
| missing appearance record | layer not drawn | drawn |
| partial hue | flags2 bit 18 of the appearance record (never set) | the item's flags (tiledata partial hue) |
| canvas, positions | body gump size (260x237), all at 0,0 | profile canvas (260x300, 300x340), offsets 0 |
| body gump | by body (table above) | typed in the dialog |
| draw order | the hard-coded table and moves of section 4 | `WearableRules.kdl` (wearables.xml) with its overrides |
| taken off | backpack, mount, a ghost's robe | robe and dress take-off decisions (`StoredDressDecisions`) |

Since 2026-11-28 dynamapper numbers each Enhanced worn item by the client's rule
(`uocf::enhanced::paperdoll_gump::legacy_gump_number`): props2 Paperdoll, else props2 Appearance, else the item's
own id. The profile offsets still give the gumps. The other rows are open, because following the client there
draws worse paperdolls than the data allows:

- **missing appearance records**: the client hides 45 items (57 in fp) whose gump exists, the backpacks aside;
- **partial hue**: the client hues every worn item fully because the records it reads carry no flag;
- **canvas**: the client crops the gargish two-handers;
- **female wrap**: the client draws unrelated gumps for two shields;
- **draw order and take-off**: the client's order differs from the `WearableRules.kdl` one (torso under legs,
  feet over legs, neck over the robe, rings and bracelets over the arms, earrings near the top), and it takes off
  nothing.

## 7. Open points

- equipconv.csv: what a server or a patch shipped in it is unknown; no copy was found.
- `FUN_0042eff0`, which gives the default body gump's gender, is taken to be the player mobile (**probable**).
- Interface.uop of EC fp packs its gumps with zlib + BWT (compression 3), which the script does not undo: its gump
  sizes were not measured.
