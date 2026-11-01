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
   takes the item's hue. **confirmed** for the flow, **probable** for what +0xbc returns.
7. **Frame numbers.** Mobile bodies and item animation ids: the male set has 400 (human), 402, 605 (elf), 607,
   666 (gargoyle); the female set 401, 606, 608, 667 (and 400). EC: 578 male frames (numbers 105..9986), 577
   female (185..9985); KR: 313 and 333 (183..1019). **probable** (the ids match the body and tiledata animation
   numbers; the code only passes +0x50 through).

## For a writer

- The entries are ordinary AMOU files: `AmouFile` and the frame encoder write them back byte for byte
  (`paperdoll_frame_sets::paperdoll_frame_sets_round_trip`, EC and KR).
- A new paperdoll image is a frame `(gender, number)` in the matching set; the table must stay sorted by (action,
  number) for the client's binary search, and every frame of a set keeps its gender as the action.
