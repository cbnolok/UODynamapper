# Classic Client: which UOP frame bodies can be requested

`classic_7.0.103.0_client.exe`, shipped data of the plain `_Ultima Online Classic` folder. Static only: nothing ran.
Three labels: **proven** (read in the code, or parsed from the shipped files), **likely** (inferred from them),
**not determined**. Function names are those set by `NameClassicFrameCode.java`.

## Short answer

The flag does not decide what is drawn from UOP. It is tested on the object, and the entry requested comes from the
tile definition in `tileart.uop`.

| Body | `mobtypes.txt` | Requested from UOP with shipped data? | Requested at all? |
|---|---|---|---|
| 1525 | no 0x10000 | **Yes**: a gargoyle wearing tile 42002 (**proven**) | yes |
| 197, 198 | no 0x10000 | No shipped row or table leads there (**proven**) | only if a server sends tile 197 or 198 as a layer item of a flagged mobile (**proven** for the request, not run) |
| 267 | no 0x10000 | same as 197 | same as 197, tile 267 |

Over all 277 bodies with UOP frames, the ones reached neither by a flagged mobile's own body nor by a wearable tile on
a flagged wearer are exactly 197, 198 and 267 (**proven** on the shipped files).

## The path of a request

Every UOP frame request ends in `GetUopFrameSet` (0x00543890): its callers are `GetUopFrame`, `UopFrameSetHasFrames`
and `PreloadWeaponUopFrameSets`, and the asset type 0x19000000 appears as an operand only there (0x00543b80,
0x00543c37). **Proven**, for a literal operand; a computed type would not show in that scan.

`GetUopFrameSet(id, object, action, frame, retry, direction)`:

1. Tile definition of `id`, with no 10000000 added (0x0054399c). None: empty result.
2. Placeholder of the object's body, `word[object +0x118] + 10000000` (0x00543a05). None: empty result.
3. `TileDefinition_GetAppearance` on the placeholder (0x00543a8b), overridden by a non-zero
   `TileDefinition_GetAnimationAppearance` (0x00543aac); 10000000 is subtracted if at or above it (0x00543ab4).
4. The object's sequence maps (action, frame, direction). The key is `(appearance << 8) + action`, type 0x19000000
   (0x00543b47..0x00543b92).

So the UOP body is the appearance, not `id`. The appearance is the tile id, or property 8 of the tile, or the first
matching body or gender filter; a body filter matches the placeholder's tile id or its property 8.

There is a second gate. `GetUopFrame` (0x00543e50) and `UopFrameSetHasFrames` return nothing unless the object's
`CUOAssetAnimation` (object +0x108) has its sequence and choreography, and `AssetAnimation_Construct` (0x005baf50)
fills them only when `BodyHasUopSequence` (0x005978e0): flags & 0x30000. Eight bodies have 0x20000 and not 0x10000:
400-403 and 605-608. Inside `GetUopFrameSet` the flag is not tested at all. **Proven**.

## 1. What the fields hold and what each caller passes

**+0x38** is the raw graphic. On a mobile it is the server's body, unconverted: `CMobile_Construct` 0x005bb4c7,
`CMobile_SetBody` 0x005bb200, reached from `Handle_0x78_MobileIncoming` and `Handle_0xD3_MobileIncoming`. On an item it
is the tile id (`FUN_005b00f0`, `FUN_005b0770`, `FUN_005b82d0`). On a corpse it is 0x2006 (`CCorpse_Construct`). **Proven**.
Handlers other than 0x78, 0xD3 and 0x2E that may set a body were **not read**.

**+0x3E** is the item amount, and on a corpse the corpse's body: `CCorpse_Construct` 0x005bb86a, `FUN_005bb8b0` 0x005bbba6
(which creates a `CCorpse` when the graphic is 0x2006, with the amount as body), `FUN_005bca70` 0x005bcece and
0x005bcfa6. `FUN_005b00f0` sets it from tiledata. **Proven** for the writes; that the corpse's amount comes straight from
the world-item packet is **likely** (`FUN_005bb8b0`'s callers were not read).

`GetMobileFrame` (0x005bc880) is used only on corpses (**likely**: its callers are `DrawCorpse`, `CCorpse_ComputeBounds`,
both in the `CCorpse` vtable, and the corpse branch of `ObjectFrameHitTest`). `DrawMobile` was misnamed.

`GetObjectLayerFrame` (0x005441e0) has two routes to `GetUopFrame`:

- Object flagged (0x0054424b reads +0x38, 0x00544250 tests it): `GetUopFrame(id)` at 0x0054430a, with no test of `id`
  and no fallback to MUL (0x0054430f jumps out).
- Object not flagged, id flagged (0x00544321): `IsMountBody(id)` at 0x0054433b, then `GetUopFrame` at 0x005443da; a
  flagged id that isn't a mount body gives no frame (0x00544375). An unflagged id can't take this route.
- Object and id not flagged: `GetMobileMulFrame` (0x0044e8e0).

Ids passed by the callers (`CMobile_Draw` 0x00452f90, `CMobile_DrawVariant` 0x00453ba0, `CMobile_ComputeBounds`
0x004517a0, `CMobile_ComputeBoundsVariant` 0x00451d50, `ObjectFrameHitTest` 0x0044fd50, `FUN_0044d980`,
`MountedFrameBounds` 0x00450f90); **proven** for each site read:

| Case | Id | The test sees |
|---|---|---|
| The mobile itself | +0x38; six ghost bodies are replaced in the id only (0x192→400, 0x193→401, 0x25F→0x25D, 0x260→0x25E, 0x2B6→0x29A, 0x2B7→0x29B) | raw +0x38 |
| Layer item, wearer flagged | the item's +0x38, its graphic | wearer's +0x38 |
| Layer item, wearer not flagged | tiledata anim id of the graphic, through `body.def`, `FUN_00449650` where called and `Equipconv` (`FUN_00562480`) | wearer's +0x38, then the id at 0x00544321 |
| Mount layer 0x19 | `FUN_005d3380` of the mount's graphic | as above |

`FUN_00562480` also runs on the flagged id in `CMobile_ComputeBounds` and `CMobile_ComputeBoundsVariant`; `CMobile_Draw`
and `CMobile_DrawVariant` skip it there. No `Equipconv.def` row names 197, 198, 267 or 1525.

`DAT_00751878`, `DAT_00751880` and `DAT_00751884` are `art.def`, `body.def` and `corpse.def`: `LoadDefFiles` stores them
at its +0x18, +0x20 and +0x24 and its `this` is 0x00751860 (0x005ff26f). **Proven**.

## 2. Tile 42002 and body 1525

Tile 42002 (`tileart.uop`): property 8 = 1522, and a body filter 10000400, 10000401, 10000605, 10000606 → 1522,
10000666, 10000667 → 1525. Tiledata: wearable, layer 22, anim 1522.

| Wearer | Id | Source |
|---|---|---|
| 666, 667; 694, 695 through their property 8 | 42002 → appearance 1525 | UOP `build/animationlegacyframe/001525/NN.bin` (55 entries); no MUL fallback. **Proven** |
| Unflagged, e.g. 400 | tiledata anim 1522 → `BodyUsesUopFrames` false | MUL through `Bodyconv.def` (1522 → 597). **Proven** |
| Corpse with a flagged body | tiledata anim 1522 → `GetMobileFrame(1522)` → tile 1522 has no property 8 → UOP body 1522, which has no frames | no frame. **Proven** as a request; the visible result was not run |

The `Bodyconv.def` row of 1525 (→ 600) is reached only when the id itself is 1525 on the MUL side: no tiledata anim is
1525 and no `body.def` or `corpse.def` row names it, so with shipped data only a mobile whose own body is 1525 gets it.

No other tile row names 1525. Tile 1525 itself (a roof piece, not wearable) resolves to it by default, so it has the
server-sent route of section 3. **Proven**.

## 3. Bodies 197, 198, 267

The test runs before the conversion. `GetMobileFrame` tests +0x3E, and `ConvertCorpseBodyForMul` (0x0044f6d0, called from
0x005bc9eb) runs only on the MUL side. `GetObjectLayerFrame` tests, then `GetMobileMulFrame` applies `body.def` and the
bodyconv table. The id tested for a mobile or corpse is therefore the server's own. **Proven**. Editing `Body.def` can't
move a body to UOP.

- A mobile or corpse with body 197, 198 or 267: unflagged, not in `IsMountBody`, so MUL. **Proven**.
- A flagged wearer's layer item: the id is the item's graphic, and tiles 197, 198, 267 exist with no property 8 and no
  filter, so the appearance is the tile id. They are a stone wall, a stone arch and a marble wall, not wearable.
  `Handle_0x78_MobileIncoming`, `Handle_0xD3_MobileIncoming`, `Handle_0x2E_WornItem` and `Item_EquipOnLayer` don't check the
  tile's wearable flag or layer (the fourth caller, `FUN_004675e0` for opcode 0x27, re-equips a saved item), so a server that sends one of these graphics on a layer of a flagged mobile causes the
  request. **Proven** as far as the request; whether a usable picture results was not run. That is the one route found,
  and it is why these bodies are not called unreachable.
- Shipped tables: no tile row's property 8 or filter names them, no tiledata anim is one of them, and `Body.def`,
  `Corpse.def` and `Equipconv.def` don't name them. **Proven**.
- A corpse's equipment can't request them: `DrawCorpse` keeps only ids 400..1999. **Proven**.
- Editing `mobtypes.txt` to flag them: the placeholder, the sequence and the frames all ship for the three, so they
  would **likely** draw from UOP. Not run.

## 4. `FUN_005b96a0`: `PreloadWeaponUopFrameSets`

Sole caller `Item_EquipOnLayer` (0x005b9b10), when the item's tiledata flags have 2 (`CDynamicObj` vtable +0x40 is
0x005b1b20) and the layer is 1 or 2. It takes the wearer's placeholder (0x005b971c) and the item's tile (0x005b9787),
computes the appearance, and calls `GetUopFrameSet(appearance, wearer, action, 0, no retry, direction)` for actions 9..14
(0x005b982d..0x005b98ad), keeping non-empty sets in the wearer's map at +0x28C. **Proven**. No frame is decoded or drawn
here, and the only readers of the map are `FUN_005b8b20` and `FUN_005b9ea0`, so it is **likely** a keep-loaded preload.
What those two do with the sets was not traced.

It doesn't test 0x10000. The wearer needs its placeholder, a sequence that maps the action, and for the frame set to
exist, so a 0x20000 human can load UOP frames through it. **Proven**. Of the 1011 tiles with flag 2 and a property 8,
262 point at bodies with frames, all of them flagged. The one tile with flag 2 that resolves to an unflagged body is
42002 → 1525 for gargoyle wearers, and only if a server sends it on layer 1 or 2, not its tiledata layer. It can't reach
197, 198 or 267 with shipped tiledata: those tiles lack flag 2. **Proven**, with one edge: the flags are read at graphic +
the item's signed offset +0x3C, which 0x2E can supply. That offset was not explored.

## 5. Verdict

| Claim | Label |
|---|---|
| The flag is tested on the object, and the id requested isn't tested on the flagged route | proven |
| The tested id is the server's own: no conversion before the test | proven (0x78, 0xD3 read; other body-setting handlers not read) |
| The UOP body is the tile's appearance, not the id passed | proven |
| A flagged body with a redirecting tile draws another entry (647→603, 650→610, 769→585, 1027..1030→654..657, 1032..1034→659..661) | proven by code and data, not run |
| 1525 is requested from UOP by a gargoyle wearing tile 42002 | proven |
| The `Bodyconv.def` route of 1525 is unused by shipped items | proven |
| 197, 198, 267 are not requested through any shipped table | proven |
| A server can make them requested by sending those graphics as layer items on a flagged mobile | proven for the request, not run |
| They are unreachable | false, per the line above |
| Any other route to 197, 198, 267 under shipped data | none found in the callers traced; not determined beyond them |
| `PreloadWeaponUopFrameSets` is a preload | likely |
| Editing `mobtypes.txt` would draw 197, 198, 267 from UOP | likely |
| `FUN_0044d980`'s exact role, `FUN_005d6f50` and `FUN_005d7610`'s purpose (they draw from MUL for an unflagged body beside certain items, likely sitting) | not determined |

## Method

Probes in `~/ghidra_scripts/` (never committed client output): `ClassicUopTraceDump20261103.java`,
`ClassicUopTraceFieldWrites20261103.java`, `ClassicUopTraceData20261103.java`, `ClassicUopTraceVtables20261103.java`,
`ClassicUopTraceConst20261103.java`. Shipped data was parsed read-only with scratch Python on top of
`../ghidra-animation/verify_animation.py`, following the tileart layout in `../ghidra-tileart/results/uosa_tileart.md`.
