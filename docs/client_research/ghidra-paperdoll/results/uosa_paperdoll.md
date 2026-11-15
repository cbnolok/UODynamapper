# UOSA.exe and UOKR.exe: Paperdoll.uop

Programs `UOSA.exe` (SHA-256 `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50`) and `UOKR.exe`
(`1b802e5c00f3bf89a771778b9c6c57406be0703808eeb4cd983e2f5baa348a43`). **confirmed** = read in the code;
**probable** = inferred from the code and the shipped data.

## Functions

| Program | Address | Name | Notes |
|---|---|---|---|
| UOSA | 0x0050a2e0 | `PaperdollTextures::LoadPaperdollFrameSets` | assets 0x40, 0x41 of type 0x12 |
| UOSA | 0x0050b780 | `PaperdollTextures::FindPaperdollFrame` | `FindFrameEntry(set, gender, id)`, fallback (0, 400) |
| UOSA | 0x0050b340 | `ECPaperdollTexture::Build` | probable name: vtable 0x00ca9298 slot 2 |
| UOSA | 0x0050bfc0 | `CCPaperdollTexture::Build` | probable name: vtable 0x00ca92b8 slot 2 |
| UOSA | 0x0050b850 | `PaperdollTextures::FindLayerFrame` | worn item: Appearance chain, then `FindFrameEntry` |
| UOSA | 0x0050ba30 | `ECPaperdollTexture::GetLayerFrame` | probable name: vtable 0x00ca9298 slot 6 |
| UOSA | 0x0050c550 | `CCPaperdollTexture::GetLayerFrame` | probable name: vtable 0x00ca92b8 slot 6 |
| UOSA | 0x0050a240 | `PaperdollTextures::ReleasePaperdollFrameSets` | probable |
| UOSA | 0x00509b30 | `UOPaperdoll::DrawGumpLayer` | CPU draw of one gump layer, partial hue (section below) |
| UOSA | 0x0050b690 | (not renamed) | probable `ECPaperdollTexture::GetLayers`: vtable 0x00ca9298 slot 5 |
| UOSA | 0x00430f60 | `UOAnimations::FindFrameEntry` | (action, number), 32-bit compare |
| UOSA | 0x00842bb2 | `UOObjectDefinitionBase::GetAppearance` | the +0xbc of `FindLayerFrame` |
| UOSA | 0x00842d90 | `UOObjectDefinitionBase::GetGraphicForAmount` | sets the face item's display graphic (item 9) |
| UOSA | 0x005bc130 | `UOObject::GetObjectDefinition` | a tile id without a record becomes 99999999 (item 9) |
| UOSA | 0x005b3980 | (not renamed) | `UOObject` vtable +0x68: the display graphic +0x4c (item 9) |
| UOSA | 0x0084b3c4 | `AppearanceFilterByOwnerBodyType::Apply` | |
| UOSA | 0x0084b485 | `AppearanceFilterByOwnerGender::Apply` | |
| UOSA | 0x00468810 | `UOAnimations::BuildEquipmentLayerOrder` | the walking order (+0xf8) and the paperdoll one (+0x160) |
| UOSA | 0x00468e30 | (not renamed) | applies the `wearables.xml` overrides to a draw order |
| UOSA | 0x00468660 | (not renamed) | picks the `draworder` record of a key |
| UOSA | 0x00468230 | (not renamed) | probable `wearables.xml` loader: reads "draworderpriority" at 0x004682a9 |
| UOKR | 0x004d9f80 | `PaperdollTextures::LoadPaperdollFrameSets` | same requests |

Globals (not renamed): UOSA 0x00e33d3c male set (asset 0x40), 0x00e33d44 female set (asset 0x41); UOKR
0x00c93c78 and 0x00c93c8c.

## What the entries are

1. **Names.** The two entries are `build/paperdoll/000001/00.bin` (08400B837FE16A24) and
   `build/paperdoll/000001/01.bin` (0747D0B847EE5B4B), in EC, EC fp and KR alike. **confirmed** (the hashes;
   both names are in `Dictionary.dic`).
2. **Assets.** The asset type names are a table of string pointers whose index is the type
   (`AnimationDefinition` 0x0b, `AnimationSequence` 0x0c, `AnimationFrameSet` 0x10, `PaperdollFrameSet` 0x12).
   `LoadPaperdollFrameSets` requests ids 0x40 and 0x41 of type 0x12 once and fails while either is missing.
   **confirmed** (UOSA 0x0050a341, UOKR 0x004d9ffa). How the resolver turns 0x40 / 0x41 into `000001/00` and
   `000001/01` was not traced (`id / 64`, `id % 64` fits). **probable**.
3. **Format.** Both are AMOU v1 files, the `AnimationFrame` format: animation id 1, the 256-colour palette at
   +40, the 16-byte frame table at 1064, the same RLE. They go through `UOAnimations::FindFrameEntry` and
   `UOAnimations::BuildFramePalette`, like the mobile animations. **confirmed**.
4. **Lookup.** `FindPaperdollFrame` searches the set with action = the gender (+0x8c: 0 in `00.bin`, 1 in
   `01.bin`; every shipped frame has the action of its file) and frame number = the texture's id (+0x50, less
   10,000,000 when above it). A miss falls back to (action 0, number 400) of the male set, else nothing is drawn.
   The palette is built with the hue at +0x54. **confirmed**.
5. **Who.** The EC paperdoll texture (`ECPaperdollTexture`) takes the gender from the mobile's object
   definition (vtable +0xd4) and picks the female set for a non-zero value, the male one otherwise. The CC
   texture (`CCPaperdollTexture`) does the same from the object definition of its id (10,000,000 added when the
   id is below 10,000,001). `LegacyPaperdollTexture` does not use these sets. **confirmed** for the calls,
   **probable** for the class roles (RTTI of the vtables).
6. **Worn items.** `GetLayerFrame` takes the item of a paperdoll layer and its object definition; a
   `Paperdoll` property (11) redirects to the definition of value + 10,000,000. `FindLayerFrame` follows the
   `Appearance` property (8, same + 10,000,000) up to 5 levels, then searches (action = gender, number = a
   definition value read through vtable +0xbc). A miss is retried in the male set with action 0. The palette
   takes the item's hue. **confirmed** for the flow, **probable** for what +0xbc returns. Since read: +0xbc is
   `GetAppearance` (item 8). **confirmed**.
7. **Frame numbers.** Mobile bodies and item animation ids: the male set has 400 (human), 402, 605 (elf), 607,
   666 (gargoyle); the female set 401, 606, 608, 667 (and 400). EC: 578 male frames (numbers 105..9986), 577
   female (185..9985); KR: 313 and 333 (183..1019). **probable** (the ids match the body and tiledata animation
   numbers; the code only passes +0x50 through).

## Worn items on the EC paperdoll

8. **Frame number of an item.** `FindLayerFrame` (0x0050b850) gives up past depth 5. A definition with an
   `Appearance` property (8; `HasProperty` vtable +0x64, value +0x68) first tries the definition value +
   10,000,000, recursively, and returns its frame when one is found. Otherwise the number is `GetAppearance`
   (0x00842bb2) of the definition for the wearer: the `Appearance` value, else the tile id, unless one of the
   definition's appearance filters applies first (subtype 2, the animation filter, is skipped; the first filter
   that applies wins). `AppearanceFilterByOwnerBodyType::Apply` (0x0084b3c4) matches the wearer's `Appearance`,
   else its tile id (vtable +0x10), against each row's body type; `AppearanceFilterByOwnerGender::Apply`
   (0x0084b485) matches the wearer's gender (+0xd4). `FindFrameEntry` (0x00430f60) compares 32-bit numbers, so a
   number past 65,535 finds no 16-bit frame. **confirmed**.
9. **Which layers.** The layer list (0x0050b690, vtable slot 5) reads the 26 layer bytes of the mobile at +0x160,
   the paperdoll order, and skips layers 0, 0x15 (backpack) and 0x19 (mount). The face layer (0x0F) item takes
   the value of item vtable +0x68 in place of its tile id when that is non-zero (in `GetLayerFrame`).
   **confirmed** for the reads. Traced 2026-11-07: the items are `UOObject`s (RTTI vtable 0x00cb501c; there is no
   item class). Slot +0x60 (0x005b3960) returns the tile id at +0x48; slot +0x68 (0x005b3980) returns +0x4c and
   slot +0x64 (0x005b3990) sets it. `UOObject::InitFromDefinition` (0x005bdac0, at 0x005bdba3..0x005bdbcd) sets
   +0x4c to `GetGraphicForAmount` (0x00842d90, definition vtable +0xc4) of the object's definition for the amount
   (slot +0x80, 0x005b39d0: 1 unless flags1 bit 0x800 is set, then the u16 at +0x56): the last stack alias whose
   amount is at most that, else the tile id the record stores (+0x20, not its entry name). The only other writer
   is `UOLua::GumpManagerSetPlayerFaceType` (0x004f2e10, `FaceSelectionWindow.lua`), which sets it on the
   player's face item to preview a face. **confirmed** (decompiler, assembly, probe
   `~/ghidra_scripts/PdollFaceAltIdSitesA2fe.java`). So the face is drawn from the record of its display
   graphic. In EC 2024 the 26 face records (15172..15191, 22137..22142) have no stack alias and store their own
   id, and only 2 records of `tileart.uop` have stack aliases: the lookup by the item id finds the same record and
   frame (1000..1025 in both sets), so the shared lookup does not model it. **confirmed** (data check, 2026-11-07).

   The default record: `UOObject::GetObjectDefinition` (0x005bc130), which `InitFromDefinition` calls first,
   replaces a tile id that has no record with 99999999 and uses that record, so the object's tile id (+0x48) and
   display graphic (+0x4c) become 99999999 and `GetLayerFrame` finds that record. **confirmed** for the code;
   **probable** that worn items are created through `InitFromDefinition` (its callers 0x005ae0d0 and
   0x005ae540 were not read). The EC 2024 record 99999999 stores its own id and no property, filter or alias, so
   such an item finds no frame (its number is past 16 bits) and is not drawn. `uocf::enhanced::paperdoll`
   (`object_definition`, `DEFAULT_DEFINITION_ID`) takes it for an item; the wearer's own definition is not
   defaulted there (the mobile's path to it was not traced).
10. **Paperdoll order.** `UOAnimations::BuildEquipmentLayerOrder` (0x00468810) reads the wearer definition's race
    (`HasRace` +0xc8, `GetRace` +0xd0, 1 when absent) and gender (+0xcc, +0xd4, 0 when absent). The walking order
    (+0xf8) is keyed (race, gender, frameset, direction); the paperdoll one (+0x160) is keyed (race, gender, -1,
    9) at 0x00468b0a..0x00468b4e. Both go through 0x00468e30; a layer whose priority ends at 0 or less gets 0x1e,
    not drawn; the others their layer byte (0x00468b77..0x00468b90). **confirmed**. Direction 9 is the
    `paperdoll` direction of `wearables.xml` and race 1/2/3 human/elf/gargoyle. **probable** (string order).
11. **The rules.** 0x00468e30: 0x00468660 picks the first `draworder` record whose 4 key ints match (-1 on the
    record side matches anything; the walk order of the list was not traced). For each worn layer
    (0x004687b0), the rule map at this+0x88 (a `std::map` keyed by (layer, item)) gives the rule of the exact
    item, else the one for any item (-1). Each override matches on its 4 key fields (-1 = any); its conditions
    (layer, item) are OR-ed, one matching when that layer wears that item, or any item for -1; an override with
    no condition applies. The last matching override sets the priority. 0x0046d7b0 then sorts the (priority,
    layer) pairs. **confirmed**.
12. **Names.** The slot names are a table of string pointers at 0x00db9ba0 whose index is the layer: drag,
    righthand, lefthand, feet, legs, torso, head, hands, finger1, talisman, neck, hair, waist, chest, lwrist,
    face, facialhair, abovechest, ears, arms, cape, backpack, dress, skirt, feetlegs, riding; a null, then the
    genders (any, male, female) and the races (any, human, elf, gargoyle). The file is `Data/wearables.xml`
    (0x00ca0d78) of `MainMisc.uop`; `dynamapper/assets/runtime_specs/paperdoll/WearableRules.kdl` is its
    translation. **confirmed** (probe `~/ghidra_scripts/FindRawAddressBytesTextTables.java`).
13. **Override loader.** 0x00468230 reads an `override`: its key (0x00467b10), `draworderpriority` (strtol), then
    only the 26 attributes the slot table names (loop at 0x004682e2), each "any" (-1) or a comma list. Any other
    attribute is dropped: the shipped `<override draworderpriority="11900" belowchest="8097,8059,8189"/>` of
    `chest` keeps no condition, so it always applies, and as it follows the `dress` / `abovechest` hiding
    overrides it wins over them; only the later 12500 override (dress 39301, 39302, 41674, 41675) beats it.
    **confirmed** for the loader; the outcome follows item 11.
14. **Shipped data** (EC 2024 `tileart.uop`, `Paperdoll.uop`). The filters' body types are 10,000,000 + body
    (10,000,666 and 10,000,667, the gargoyles), matching the wearer definition's tile id. Of the 1125 records
    with a drawn Layer property, 983 find a frame for a human man and 1054 for a woman; about 70 have a frame
    in neither set (9801..9892, the talismans 12120..12123 among them) and the man misses the female-only
    pieces besides (7168..7181: frames 921..927 of the female set). **confirmed** (the inspector's ignored
    tests `shipped_bodies_and_wearables_find_their_frames`, `a_shipped_outfit_composes_over_the_body`).

The inspector's Gumps > Paperdoll tab with the EC source follows items 4..13
(`tools/uocf-inspector-gui/src/ui/ec_paperdoll.rs`). Not followed: the face layer's display graphic (item 9; it
finds the same record in the shipped data), and the walk order of the `draworder` records (item 11; the inspector
takes the `paperdoll` one).

## Partial hue on the gump paperdoll

`UOPaperdoll::BuildImage` (0x0050c790) composes the gump paperdoll on the CPU, one layer at a time, through
`UOPaperdoll::DrawGumpLayer` (0x00509b30; named 2026-11-06). Its arguments are (gump id, target `NiPixelData`,
layer map, layer byte, hue, partial, key r, g, b). **confirmed** (decompiler and assembly).

15. **Source.** The gump is `Data/Interface/Default/Textures/GumpArt/%08d.tga` of `Interface.uop`
    (`MakeGumpTextureLocation` 0x00519700, through 0x0051af10), read as the texture's source `NiPixelData`
    (+0x3c): uncompressed bytes, the first three of each texel taken as R, G, B. A 256x256 gump is skipped.
16. **Transparency.** `BuildImage` passes the sentinel key 0x00e32c80..88, so the key becomes the gump's first
    texel (0x00509c4d..0x00509c7e). A texel whose R, G, B equal the key is not drawn; the alpha byte is not read.
    After every layer, `BuildImage` sets each target texel's alpha to 255 when any of R, G, B is non-zero, else 0.
17. **The hue test.** With a hue, a texel is hued when partial is false, or when its bytes are exact greys:
    `CMP AL,DL` (R, G) at 0x00509d64 and `CMP AL,CL` (R, B) at 0x00509d6c, each jumping past the hue on
    inequality. No tolerance and no separate mask. The colour is `UOHues::GetHueTable(hue)[R]` (0x00427ad0,
    0x00509d9e: the red byte is the index); an entry alpha below 255 blends it over the target texel
    (0x00509ded..0x00509e51).
18. **Partial.** The body gump is drawn with partial true (skin hue on greys only); each worn layer passes
    `HasFlags2(0x40000, 0)` (PartialHue, tileart flags2 bit 18) of its appearance definition.

So a block-coded Enhanced paperdoll gump changes what the client hues: any texel that was an exact grey and is
no longer one loses the hue, and a coloured texel that rounds to a grey gains it.

The key's texel 0 (item 16) is the gump's top-left texel: every gumpart TGA of `Interface.uop` is stored bottom-up
(image descriptor bit 0x20 clear, bit 0x10 clear), and Gamebryo's `NiTGAReader` flips a bottom-up file as it reads
it (2.6 `NiTGAReader.cpp`, `m_bFlipVert`). Dynamapper's Enhanced and KR-like paperdoll follow items 15..18
(`dynamapper/src/core/render/dialogs/gump.rs`, `compose_decoded_layers`).

Dynamapper now bakes items 15 and 16 at packing time (`pack-ec-gumps`, `bake_enhanced_paperdoll_rule` in
`lib/udd-conv/src/gumps/ec_gumps.rs`, 2026-11-06): each drawn texel of a paperdoll gump is stored opaque and every other
texel transparent, so its paperdoll blends by alpha. It drops the unused glow of gumps 50322 and 60322, and leaves
a drawn pure black texel transparent over the layers under it rather than clearing them.

## For a writer

- The entries are ordinary AMOU files: `AmouFile` and the frame encoder write them back byte for byte
  (`paperdoll_frame_sets::paperdoll_frame_sets_round_trip`, EC and KR).
- A new paperdoll image is a frame `(gender, number)` in the matching set; the table must stay sorted by (action,
  number) for the client's binary search, and every frame of a set keeps its gender as the action.
