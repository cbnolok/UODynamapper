# ghidra-ec-paperdoll-gump

This study asks how the Enhanced Client picks the gump each worn item draws on its old-style 2D gump paperdoll
(`LegacyPaperdollTexture`, `UOPaperdoll::BuildImage` 0x0050c790). It also asks how it picks the body gump, the
positions, the hue and the draw order. It is the Enhanced counterpart of the Classic rule of
[ghidra-cc-tileart](../ghidra-cc-tileart/README.md); the 3D paperdoll is in
[ghidra-paperdoll](../ghidra-paperdoll/README.md).

**Answer** (results sections 2 to 4, confirmed):

- **Number.** An item's number is tileart props2 Paperdoll (key 11), else props2 Appearance (key 8), else its own
  graphic. props1, the appearance filters and tiledata are not read.
- **Gump.** The gump is number + 50,000. A female body first tries number + 60,000 and falls back to + 50,000.
  There is no `gump.def`.
- **equipconv.csv.** `data/gamedata/equipconv.csv` could convert the number, the gump and the hue per body, but it
  ships in no package, so it never applies.
- **Missing record.** An item whose number names no record at 10,000,000 + number is not drawn.
- **Partial hue.** It is flags2 bit 18 of that record, which no shipped record sets.
- **Body gump.** It comes from a fixed body table (400 -> 12, 401 -> 13, 605 -> 14, 606 -> 15, 666 -> 666,
  667 -> 665, ghosts alike).
- **Canvas and positions.** The canvas is the body gump's size (260x237), and every layer is drawn at 0,0. No
  profile offsets exist in the client.
- **Order.** The draw order is a hard-coded table with item-specific moves, ClassicUO's paperdoll order at heart.
  Only the backpack, the mount and a ghost's robe are left out.

**On the shipped data** (section 5):

- No gump changes. Every props2 Appearance equals the tiledata anim (1,102 EC and 1,240 EC fp wearables).
- The 24 wearables without one (tiledata anim 0) are numbered by their graphic and find no gump.
- The client leaves out 47 (fp 59) items whose appearance record is missing: gargish jewellery, talismans, samurai
  armour, epaulets.
- It hues every worn item fully, 89 (fp 173) partial-hue items among them.
- It crops 9 gargish two-handers to 260x237.

Dynamapper's Enhanced gump paperdoll now numbers items by the client's rule (`uocf::enhanced::paperdoll_gump`). The
missing records, partial hue, canvas, draw order and take-off are open decisions (results section 6).

Findings, addresses and measurements: [results/uosa_paperdoll_gump.md](results/uosa_paperdoll_gump.md).

## Program

The program is in the Ghidra project `UOSA`. It was imported read-only and never executed. It was auto-analysed,
named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/UOSA.exe` | `_Ultima Online Enhanced/UOSA.exe` 4.0.102.3 | `5a5e9dc536c836d1bf10b6f86bb88f61a6ae078f519d4060d3bc47aecbebad50` |

The client folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binary or anything extracted
from it.

## NameEcPaperdollGumpCode.java

A GhidraScript in category `ECResearch`, built like `ghidra-paperdoll/NamePaperdollCode.java`.

```
NameEcPaperdollGumpCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="UOSA.exe", script_name="<abs path>/NameEcPaperdollGumpCode.java",
                  args="<scratch>/uosa_paperdoll_gump_names.md --apply")
```

- It names fixed addresses of the program above, selected by SHA-256. Any other program is refused.
- Every address is checked first with `calls:`, `calledby:`, `ref:` (a vtable slot), `string:` and `const:` (such
  as 0xc350 = 50,000, 0x2710 = 10,000, 0x40000, and the order tables 0x00c9b688, 0x00c9b66c, 0x00c9b6a4). A failed
  check is reported and nothing is named.
- `OUTPUT_FILE` is created with CREATE_NEW.
- Without `--apply` the script only reports.
- With `--apply` it renames DEFAULT functions into `PaperdollTextures`, `LegacyPaperdollTexture`, `UOPaperdoll` and
  `UOGameData`. It adds the tag `UO_PAPERDOLL_GUMP` and sets the plate comment
  `NameEcPaperdollGumpCode: UO_PAPERDOLL_GUMP <name> [(probable)]: <evidence>`.
- Names from another source are kept: `UOPaperdoll::BuildImage` (and its plate comment) and
  `SelectPaperdollBodyGumpForBody` (user-defined).

Results of the 2026-11-28 runs:

- `--apply` created the tag and renamed 11 functions, tagged 12 and set 11 plate comments. There were no
  mismatches or conflicts, and two names were kept.
- A second `--apply` reported `changes: none`, and the program was saved.

The probes were throwaway inline scripts and are not kept. They used the assembly, decompiler output,
cross-references and the `UOObjectDefinition` vtable 0x00ca5b4c.

## measure_ec_paperdoll_gump.py

```
python3 -I -B measure_ec_paperdoll_gump.py "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced" \
    "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic"
python3 -I -B measure_ec_paperdoll_gump.py "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Enhanced fp" \
    "/mnt/dati/_proj_local/_uo_clients/_Ultima Online Classic fp"
```

- It reads `tileart.uop`, `string_dictionary.uop` and `Interface.uop` of the Enhanced folder and `tiledata.mul` of
  the Classic one, all read-only. The Enhanced folders ship no tiledata.mul, and tilemeta.uddp takes the anim from
  the Classic folder.
- It replays the client's choice for every wearable, male and female, against dynamapper's earlier rule.
- It reuses the UOP and tileart readers of `../ghidra-cc-tileart/measure_cc_tileart.py`.
- Gump entries packed with zlib + BWT (EC fp `Interface.uop`) count as present, with an unknown size.
- It takes about 11 s (EC) and 8 s (EC fp).

The output of 2026-11-28 is quoted in the results.
