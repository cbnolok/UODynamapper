# ghidra-housing

Names the code in `UOSA.exe` (Enhanced Client) that loads and uses the house customization pieces and the
multis of `MultiCollection.uop`:

- the `build/multicollection/housing.bin` asset factory and parser (`UOCustomHousingPieces`) and its
  piece queries (`HousingPieceFilter`);
- the `build/multicollection/%06d.bin` multi asset factory and parser (`UOMultiCollection`);
- the `UOHouseCustomization` object: creation, the piece tables built from `housing.bin`, the multi
  appearances, the `WindowData.CustomizedHousing` publishing and the piece selection.

Findings: [results/uosa_housing.md](results/uosa_housing.md) has:

- the layout of `housing.bin` and of the multi entries as the client reads them, field by field;
- the load path and timing;
- what each field does at run time: tile types, categories, groups, slots, feature masks, tooltips and
  entitlement clilocs, the Solid / Window / Multi lists, the two multi part flags and the part strings;
- the data verification and the comparison with the classic customization text files;
- the mismatches with `lib/uocf/src/enhanced/multis.rs` and the proposed Rust changes.

## NameHousingCode.java

GhidraScript, category `ECResearch`.

```
NameHousingCode.java OUTPUT_FILE [--apply]
```

- `OUTPUT_FILE` is created with CREATE_NEW. If the file already exists, the run stops before making any change.
- Without `--apply`, the script only reports. With `--apply` it:
  - renames DEFAULT (`FUN_...`) functions with source ANALYSIS, in the RTTI class namespace
    (`UOCustomHousingPiecesBinaryFactory`, `UOCustomHousingPieces`, `UOMultiCollectionBinaryFactory`,
    `UOMultiCollection`) or in the namespaces `UOHouseCustomization`, `HousingPieceFilter` and
    `UOResourceRequest`;
  - adds the tag `UO_HOUSING`;
  - sets a plate comment `NameHousingCode: UO_HOUSING <name> [(probable)]: <evidence>` when the comment is
    empty;
  - labels the global `g_UOHouseCustomization`; the server feature flags global is labeled
    `g_UOServerFeatureFlags` only when it has no name yet (it is `g_ServerFeatureFlags` in the current
    project, which is kept).
- Its own single-line plate comments are refreshed when the evidence text changes; comments edited by hand
  are kept.
- Names with another source type are never changed; they are listed under "Kept names". Functions already
  named by another script (for example the `UOHouseCustomization` placement and support functions named by
  `NameTileartCode.java`) are not touched.
- A second `--apply` reports `Changes: none`.

The script relies on names applied by other scripts: `UOAssets::StreamRead` and `g_UOAssetManager`
(tileart / effects) and `UOLua::SetHousingTileSelected` / `UOLua::SetHousingMultiSelected`
([`ghidra-lua-names`](../ghidra-lua-names)). Run those first.

Run it through ghidra-mcp:

```
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameHousingCode.java",
                  args="<scratch>/housing_report.md --apply", timeout_seconds=900)
```

### How functions are found

The script uses no fixed function addresses. Each function is found from evidence that doesn't depend on
its address:

| Evidence | Finds |
|---|---|
| RTTI vtables of `UOCustomHousingPiecesBinaryFactory` and `UOMultiCollectionBinaryFactory` (+4) | the two factory `Create` functions |
| Create callee that calls the constructor writing the `UOCustomHousingPieces` / `UOMultiCollection` vftable | `UOCustomHousingPieces::Parse`, `UOMultiCollection::Parse` and both constructors |
| The five Parse callees after the constructor with at most two callers (the one called three times must call `UOAssets::StreamRead`) | `InitPiece`, `ReadSlotList`, `PushPiece`, `TileTypeAt`, `AssignPieces` |
| The single-caller multi Parse callee after the constructor that uses the part size 0x1c | `UOMultiCollection::SetPart` |
| Sole user of the string `previewTile`; its single caller; the first call of that caller | `PublishCustomizedHousing`, `Initialize`, `BuildPieceTables` |
| The 15 single-caller callees of BuildPieceTables, in call order (the first must use the asset key 0x14000000) | the housing request, the `UOHouseCustomization` map accessors, the `UOCustomHousingPieces` queries, `HousingPieceFilter::Describe`, `LoadMultiAppearance` |
| GetSolidTiles callees 2..5 (at most 32 bytes each) | the four `HousingPieceFilter::Matches*` helpers |
| LoadMultiAppearance callees using the asset key 0x13000000 / the part size 0x1c with several callers | `UOResourceRequest::AddMultiCollectionKey`, `UOMultiCollection::GetPart` |
| The single single-caller callee of `UOLua::SetHousingTileSelected`; its callees writing the +0x87704 selection flag, split by whether `UOLua::SetHousingMultiSelected` also calls them | `SelectPieceById`, `SelectMulti`, `SelectTile` |
| The only function using the object size 0x877b8; the global it writes; its single-caller callee | `UOHouseCustomization::Create`, `g_UOHouseCustomization`, the constructor |
| The absolute global BuildPieceTables reads besides `g_UOAssetManager` | the 0xB9 server feature flags |

## verify_housing.py

Read-only checker that decodes `housing.bin` and every multi entry of `MultiCollection.uop` as the client
does and requires each one to end exactly where its last field ends. It prints, per tile type, the piece,
category and group counts, the feature masks and the slot distribution of the Solid / Window / Multi lists;
the categories with several tooltips or mixed feature masks; the entitlement cliloc each category gets; the
multi part flag pairs, z range and part strings; and checks that every Multi reference exists in the
collection. Optional inputs resolve the tile ids against `tileart.uop`, the part string indices against
`string_dictionary.uop` and the tooltip / entitlement clilocs against a classic `Cliloc.enu`.

```
E="<Enhanced Client folder>"; C="<Classic Client folder>"
python3 -I verify_housing.py --multicollection "$E/MultiCollection.uop" --tileart "$E/tileart.uop" \
    --strings "$E/string_dictionary.uop" --cliloc "$C/Cliloc.enu" [--dump]
```

The script exits with code 1 and prints `FORMAT ERROR: ...` on the first violation: a read past the end of
an entry, trailing bytes, an entry name that is neither `housing.bin` nor `%06d.bin`, or a string index
outside the dictionary. Standard library only; no client data is stored.
