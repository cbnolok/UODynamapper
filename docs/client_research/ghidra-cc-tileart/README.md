# ghidra-cc-tileart

This study asks what the Classic Client reads from `tileart.uop` and `string_dictionary.uop`, and whether any of it
replaces or adds to `tiledata.mul`. The question is about Classic art packages: `tex_art_cc`
(`lib/udd-conv/src/textures/tex_art_cc.rs`) fell back to tileart.uop slot aliases and draw offsets when the source
had no tiledata.mul. Following this study the fallback is gone (the user, 2026-11-27): the packer requires the
tiledata.mul of the Classic folder the art comes from (`find_tex_art_cc_tiledata`).

**Answer.** For Classic art, tileart.uop is redundant whenever tiledata.mul is present:

- The client never takes an art id, alias or draw offset from tileart. It never reads tileart flags, heights,
  weights or radar colours either.
- It loads tiledata.mul unconditionally and reads all of those from it.
- tileart.uop feeds only non-art features: UOP animation appearance and body type, the alternate animation, the
  paperdoll appearance, and the sitting data.

The tileart fallback would also not reproduce the Classic client: 23 items would draw another item's art, and the
legacy offsets describe the Enhanced Client's legacy textures, not art.mul.

**Who reads what, by body** (results section 3.1-3.3, all confirmed):

- The deciding test is `mobtypes.txt` flag 0x10000 (`BodyUsesUopFrames`) on the paperdoll body, which is always one
  of 400, 401, 605, 606, 666, 667, 1253 (by body after `body.def`, else by race).
- A MUL-frame body (humans, elves, and every other body through its race) takes an item's paperdoll anim from
  tiledata.mul. Tileart decides only when the item has an "animation by body" filter (type 2) keyed to the body's
  placeholder: then the tileart appearance replaces it. 14 shipped wearables have such a filter for humans and
  elves; the gump changes for 8 of them on men and all 14 on women. The 65 items whose tileart appearance differs
  from the tiledata anim never reach these bodies. The world drawing of the same items never reads tileart.
- A UOP-frame body (gargoyles 666/667 and ghosts, 1253) always takes the tileart appearance and never the tiledata
  anim: 198 to 249 wearables differ from tiledata for gargoyles.
- Then, for both, `Equipconv.def` (body, anim) gives the gump (else anim + 50000), +10000 for a woman when that gump
  exists, and `gump.def` replaces a gump that has no record.
- Body type and alternate animation are read only for UOP animation: the body type of 0x10000 and 0x20000 bodies'
  placeholders and UOP mounts, the alternate animation on the layer 2 weapon of a 0x10000 body.
- Tile effects are dropped at parse time; Classic never plays them.

Findings, addresses and measurements are in [results/cc_tileart.md](results/cc_tileart.md). Each result is marked
**confirmed** (read in the code) or **probable** (inferred).

## Program

The program is in the Ghidra project `UOSA`. It was imported read-only and never executed. It was auto-analysed,
named with `--apply` and saved.

| Ghidra program | Source | PE version resource | SHA-256 |
|---|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` | FileVersion / ProductVersion `7, 0, 102, 3` | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |

The Ghidra program name calls this file 7.0.103.0, as the earlier `ghidra-classic-*` studies did before their labels
were corrected. Its PE version resource says 7.0.102.3 (read from `VS_FIXEDFILEINFO` and the `StringFileInfo`
strings, without running the file). The SHA-256 is the same file.

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binary or anything extracted
from it.

## NameClassicTileartCode.java

A GhidraScript in category `ECResearch`, built like `ghidra-classic-art/NameClassicArtCode.java`.

```
NameClassicTileartCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe", script_name="<abs path>/NameClassicTileartCode.java",
                  args="<scratch>/cc_tileart_names.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256. Any other program is refused.
- Every address is checked first with `calls:`, `calledby:`, `ref:`, `string:` and `const:`. A failed check is
  reported and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW.
- Without `--apply` the script only reports.
- With `--apply` it renames DEFAULT functions into `UOClassicTileArt`, `UOClassicSitting`, `UOClassicFiles`,
  `UOClassicPaperdoll` and `UOClassicAnim`. It
  adds the tag `UO_CLASSIC_TILEART` and sets the plate comment
  `NameClassicTileartCode: UO_CLASSIC_TILEART <name> [(probable)]: <evidence>`.
- Names from another source are kept. `GetTileDefinition` 0x00406001 keeps `UOClassicAnim::GetTileDefinition` from
  `ghidra-classic-frames`.

Results of the 2026-11-27 runs:

- `--apply` created the tag and renamed and tagged 22 functions, with 22 plate comments. There were no mismatches
  or conflicts, and one name was kept.
- A second `--apply` reported `changes: none`, and the program was saved.
- The follow-up `--apply` (filter Apply functions, `GetGender`, `AddEffect`, `EquipConvGump`, the body type and
  mount sequence functions) renamed and tagged 11 more functions with 11 plate comments and refreshed 3 comments.
  A second run reported `changes: none`, and the program was saved.

The probes were throwaway inline scripts and are not kept. They used decompiler output, cross-references and
definition vtable slot offsets.

## measure_cc_tileart.py

```
python3 -I -B measure_cc_tileart.py "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic"
```

The script reads `tileart.uop`, `string_dictionary.uop`, `tiledata.mul` and, if present, `artLegacyMUL.uop`,
`mobtypes.txt`, `Equipconv.def` and `gumpartLegacyMUL.uop`, all read-only. It prints what the Classic client could
take from tileart next to tiledata.mul and the art sizes, and replays the paperdoll gump rule for each paperdoll body
against a tiledata-only rule.

- Records are keyed by their entry name `build/tileart/%08d.bin`, as the client looks them up, not by the stored
  tile id.
- The parse stops before the effects.
- It takes about 7 s.

The output of 2026-11-27 is quoted in the results.
