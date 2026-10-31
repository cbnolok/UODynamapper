# Facet variants (`facetNx.uop`, `mapNx`): what differs in the data

This note covers the data only: what differs between the base map packages and their `x` variants in every
client install we have, where, and what kind of change it is. The client code that swaps them is in
[uosa_facets.md](uosa_facets.md#packages-and-mounting): `UOFacet::ApplyFacetVariantFlag` mounts `facetNx.uop`
for N = 0, 1, 2, 5 when bit 0x8000 of the 0xA9 flags differs from the mounted state; the Classic side
(`g_UseFacetVariants`, the second facet table) is in [uosa_facets.md](uosa_facets.md#facetnx-variants-and-the-classic-x-maps)
and [classic_facets.md](../../ghidra-classic-facets/results/classic_facets.md). This note adds the variant contents, the Classic `mapNx`
files, the cross-client comparison and the lineage across three generations of data.

Confidence: every number below comes from a full comparison of the shipped files (high), unless it is marked
as an interpretation.

## Installs and files

| Label | Folder | What it is |
|---|---|---|
| EC-2024 | `_Ultima Online Enhanced` | facet packages dated 2024-06-06 |
| EC-2026 | `_Ultima Online Enhanced fp` | `facet0/1.uop` 2026-05-19, `facet0x.uop` 2026-04-09 (file times), new `facet00x.dds` |
| KR | `UO - Old KR` | `facet0..4.uop` (UOP v4, data headers dated 2009) |
| CC-2024 | `_Ultima Online Classic` | `map*LegacyMUL.uop` 2024-06-06, `statics/staidx 1x, 2x, 5x` 2018-03-29 |
| CC-2026 | `_Ultima Online Classic fp` | `map0/1` 2026-05-19, `map0x` and statics 2026-05-12 (file times) |
| CC-2011 | `Ultima Online Classic_7_0_20_0` | plain MUL; `map0x/staidx0x/statics0x` 2011-06-30, `map1x...` 2011-04-19, bases 2011-10-26 |

Variant files present:

| Install | Variants | Not present |
|---|---|---|
| EC-2024 | `facet0x`, `facet1x`, `facet2x`, `facet5x` `.uop` | no `facetNNx.dds`; no `facet3x`, `facet4x` |
| EC-2026 | the same four, plus `facet00x.dds` | no `facet01x.dds` |
| KR | none | KR has no `x` packages and no facet 5 |
| CC-2024, CC-2026 | `map0x/1x/2x/5xLegacyMUL.uop`, `staidx0x/1x/2x/5x.mul`, `statics0x/1x/2x/5x.mul` | no `x` of `facet0N.mul` (atlas images) or `radarcol.mul` (colour table, shared) |
| CC-2011 | `map0x.mul`, `map1x.mul`, `staidx0x/1x`, `statics0x/1x` | no `map2x`, `map5x` |

## Method

- Scripts (Python, run with `-I`) and the full outputs are kept outside the repository, in
  `~/rust-target/uodynamapper-scratch/facet-variants/` (`ec_diff.py`, `classic_diff.py`, `overlap.py`,
  `analyze2.py`, `area_cmp.py`, `dds_diff.py`, `bwt.py`). Diff mask PNGs are there too (one pixel per 8x8
  squares; red land id, green statics, blue z, colours add up; grey delimiters only; the dark blue column is
  x = 5120). Nothing in `lib/` or `tools/` was changed.
- EC/KR packages: entries by name hash (`build/sectors/facet_%02d/%08d.bin`); compressed bytes first, then
  decompressed bytes, then every square decoded and compared: z, land id, delimiters, statics as a multiset of
  (graphic, z, hue). A square counts as "land", "z" or "statics" if that field differs; "delimiter only" if
  only its delimiters do.
- Classic: map blocks in the base package's entry order (`build/mapNlegacymul/%08d.dat`; the variants use the
  base names, see below), land id and z per tile; statics per block as a multiset of
  (graphic, x, y, z, hue).
- Cross-client: `uocf-cli facet from-classic` built EC packages from each Classic map + staidx + statics
  (output in `/home/claudio/test/uop/fv`), which were then compared with the shipped EC packages square by
  square. The encoder writes the Classic land ids and statics as they are, so any land, z or static difference
  is a data difference.
- Regions: diff squares grouped on an 8x8 grid, merging cells up to 2 cells (16 squares) apart; the bounding
  box is the box of the differing squares. Static and land names are from `tiledata.mul`, region names from the
  FacetDefinition records of the EC-2026 `facets.uop` (cliloc names from the CC-2026 `Cliloc.enu`, decompressed
  with a port of `zlib_bwt_codec`). "blank" is land 0x244 (580), the black filler of unused map areas.
- Region tables: A is the base, B the variant. `blank only in A/B` counts squares whose land is 0x244 on that
  side only; `+B/-B` are statics added to / removed from the variant; `?` is a static with no tiledata name.
  Region names come from the EC-2026 `facets.uop` for every table, the CC-2011 ones included.
- ASCII grids: one character per 128x128 squares, `.` no difference, `o` 1..63 squares, `O` 64..1023,
  `#` 1024 or more. The column header marks x = 0, 1024, ..., 6144 (thousands digit); rows are y.

## Package basics

- **Names, ids, header.** Every EC variant holds the same entry hashes as its base (`facet_00`, not
  `facet_00x`), the same 7168 (facets 0, 1), 900 (2) or 1280 (5) sectors plus `data.map`, and the same header
  facet byte and sector id in every sector. The physical entry order differs, so `facet2x`/`facet5x` differ
  from their bases from byte 12 (first block offset) on while every sector is byte-equal.
- **Sizes.** `data.map` is the same in base and variant (4096, 7168 for facets 0 and 1; 1600, 2304 for 2;
  4096, 1280 for 5). `facets.uop` has one FacetDefinition per facet id and no variant records, so both
  packages share one width, height and region list.
- **Packing.** EC-2024 variants are UOP v5, 1000 entries per block, with the 135..137-byte data headers of the
  base packages. EC-2026 `facet0x.uop` is UOP v4, 100 entries per block, 12-byte data headers
  (`u16 3, u16 8, u64 FILETIME`) with no signature, every sector stamped 2024-10-10 and `data.map`
  2023-05-09. CC-2026 `map0xLegacyMUL.uop` has the same layout and the same 2024-10-10 stamp, so both came
  from one build. uocf writes v5, so neither is ours.
- **Classic packages.** `mapN(x)LegacyMUL.uop`: 113 uncompressed entries, 112 of 802816 bytes and one of 196,
  so 458753 blocks (7168x4096 plus one trailing block). The variants use the base entry names
  (`build/map0legacymul/%08d.dat`; no hash of `map0xlegacymul` matches). CC-2011 MULs are exactly 458752
  blocks. Base and variant `staidx` always have the same size.
- **Byte identity.** CC-2024: `map2(x)` and `map5(x)` decode identically; `statics/staidx 2x, 5x` are
  byte-identical to the bases. EC-2026 `facet1x.uop` is byte-identical to EC-2024 `facet1x.uop`; EC-2026
  `facet2(x)`, `facet3`, `facet4`, `facet5(x)` are byte-identical to EC-2024. CC-2026 `statics1x`/`staidx1x`
  are the 2018 files of CC-2024.

## Summary

Variant compared with its base (`+`/`-` statics are added to / removed from the variant):

| Pair | Units differing | Squares | Land | z | Statics squares | Statics +/- | Delimiters only | Regions |
|---|---|---|---|---|---|---|---|---|
| EC-2024 facet0 / 0x | 191 / 7168 sectors (167 at x < 5120) | 42,358 | 35,314 | 9,178 | 4,633 | +6,590 / -717 | 521 | 138 |
| EC-2024 facet1 / 1x | 209 / 7168 sectors (186 at x < 5120) | 37,462 | 35,033 | 1,369 | 2,725 | +3,138 / -885 | 271 | 110 |
| EC-2024/2026 facet2 / 2x, 5 / 5x | 0 | 0 | | | | | | |
| EC-2026 facet0 / 0x | 619 / 7168 sectors (381 at x < 5120) | 680,654 | 532,720 | 387,685 | 130,089 | +92,758 / -99,855 | 1,072 | 171 |
| EC-2026 facet1 / 1x | 231 / 7168 sectors (189 at x < 5120) | 69,368 | 65,140 | 1,372 | 8,425 | +3,246 / -6,988 | 367 | 115 |
| CC-2024 map0 / 0x | 1,344 / 458,752 blocks, statics only | 12,792 | 0 | 0 | 12,792 | +5,754 / -14,075 | | 147 |
| CC-2024 map1 / 1x | 439 blocks, statics only | 2,722 | 0 | 0 | 2,722 | +3,138 / -885 | | 99 |
| CC-2024/2026 map2 / 2x, 5 / 5x | 0 | 0 | | | | | | |
| CC-2026 map0 / 0x | 15,534 blocks | 679,435 | 532,720 | 387,685 | 129,937 | +92,758 / -99,855 | | |
| CC-2026 map1 / 1x | 940 blocks, statics only | 8,420 | 0 | 0 | 8,420 | +3,246 / -6,988 | | |
| CC-2011 map0 / 0x | 5,308 blocks | 295,884 | 275,272 | 48,785 | 33,438 | +72,524 / -31,820 | | 43 |
| CC-2011 map1 / 1x | 159 blocks | 3,897 | 2,560 | 443 | 1,841 | +1,918 / -966 | | 15 |

For scale, Felucca and Trammel (EC-2024 facet0 / facet1) differ in 1,202,351 squares (land 983,005, z 640,600,
statics 128,448). No pair differs only in the header byte: the header is always the base facet's.

## EC-2024 facet0 / facet0x

```
     0       1       2       3       4       5       6       
   0 ...............oo......Ooo..............................
 128 ........OOOOoooo....o....oO.............................
 256 .........OO..oo.........................................
 384 .......OOOO..o...o.......o..............................
 512 .......o.OOOO............o............................##
 640 ........OOOO...........................................O
 768 .....................o.....................#............
 896 ......................oo................................
1024 ..oO.o...........Oo..............o..o...................
1152 ..Oo.........................o...oo.....................
1280 .o.Oo........................o....oo....................
1408 oo..o.....o...............................o.............
1536 ....O.............................................o.....
1664 .....o......o...........................................
1792 ....oO.....oo...........................................
1920 ........................................................
2048 .....o........o.......o.................................
2176 ....oo.........O......o.................................
2304 ....................ooo.....O...........................
2432 ..........................o.............................
2560 .............oo.........................................
2688 ........O.....oo........................................
2816 ................................................##......
2944 ..............oOo...............................Oo......
3072 ........................................................
3200 ........................................................
3328 ........Oo..............................................
3456 ........Oo....o.........................................
3584 ........OOOO............................................
3712 .........oOO..............................o.............
3840 ..........OO............................................
3968 ..........Oo............................................
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 7005,518..7166,650 | 19502 | 19143 | 5651 | 0 | 0 | 19143 | +0/-0 |  |  |  |
| 1 | 6154,2848..6313,2944 | 13083 | 12955 | 0 | 949 | 10904 | 667 | +1720/-0 | 0x3609 ?, 0x1854 skull with candle, 0x0A22 lantern |  | Alchemist's Lab; Hidden Library; Secret Room |
| 2 | 5536,804..5586,853 | 1334 | 0 | 6 | 1334 | 0 | 0 | +1675/-0 | 0x21A3 nodraw |  | Despise  |
| 3 | 1040,3362..1151,3697 | 1148 | 923 | 161 | 55 | 0 | 0 | +47/-22 | 0x17AD water, 0x03A5 pier, 0x17A5 water | 0x17A1 water, 0x17A5 water, 0x17B2 water |  |
| 4 | 1228,3601..1455,3783 | 670 | 389 | 222 | 70 | 0 | 0 | +51/-39 | 0x17AE water, 0x179A water, 0x1799 water | 0x17A7 water, 0x17AA water, 0x17A1 water |  |
| 5 | 548,1600..622,1646 | 530 | 336 | 167 | 223 | 0 | 0 | +233/-106 | 0x21A3 nodraw, 0x326C swamp, 0x326A swamp | 0x0CAC grasses, 0x0CBB grasses, 0x3511 waterfall |  |
| 6 | 1408,512..1571,754 | 528 | 55 | 466 | 94 | 0 | 0 | +83/-17 | 0x179B water, 0x179C water, 0x17A5 water | 0x17A5 water, 0x17AE water, 0x17AD water |  |
| 7 | 1081,512..1186,704 | 510 | 114 | 248 | 262 | 0 | 0 | +285/-9 | 0x1798 water, 0x179A water, 0x1797 water | 0x0CA7 rushes, 0x0CD7 cedar needles, 0x0CD6 cedar tree |  |
| 8 | 1088,2730..1151,2813 | 434 | 14 | 68 | 371 | 0 | 0 | +471/-12 | 0x3270 swamp, 0x3273 swamp, 0x3277 swamp | 0x0CE0 walnut tree, 0x0CE1 walnut leaves, 0x0CCE leaves |  |
| 9 | 1344,150..1510,239 | 302 | 38 | 259 | 40 | 0 | 0 | +34/-6 | 0x1798 water, 0x179C water, 0x1799 water | 0x1799 water |  |

- **Lost Lands slot A, 7005,518..7166,650** (south of the Heartwood region, in no FacetDefinition region):
  the base holds a terrain-only area (land 1089 stone x3516 and void 506..511, z 0, no statics); the variant
  has it blank, exactly as both 2011 maps do.
- **Lost Lands slot B, 6154,2848..6313,2944:** the variant has void land and 1,720 statics where the base is
  blank or cave: 0x3609 (no name) x1501, 0x1854 skull with candle, 0x0A22 lantern, 0x24BB paper lantern,
  0x32F2 pedestal. The EC-2026 `facets.uop` later names regions inside this box (Secret Room
  6280,2864..6315,2899, Hidden Library 6152,2872..6173,2908, Alchemist's Lab 6192,2880..6223,2911), but
  neither EC-2026 facet0 package holds this content any more (EC-2026 facet1x, the 2024 file, still does).
- **Despise, 5536,804..5586,853:** the variant adds 1,675 0x21A3 nodraw statics (also in CC-2024 statics0x).
- **Britannia (x < 5120):** 167 sectors with small edits: coast and water (water statics 0x1797..0x17B2,
  0x179A..0x179C, land water 168..171 against shore 76..101), sand 26 against snow 379..391 around Jhelom
  (1040..1519, 3362..4033), swamp statics 0x3270..0x3277 (1088..1151, 2730..2813), a pier 0x03A5. The variant
  has the snow, the base has sand.

## EC-2024 facet1 / facet1x

```
     0       1       2       3       4       5       6       
   0 ...............oo......ooo..............................
 128 ........ooOOoooo....o....oo.............................
 256 .........Oo..oo.........................................
 384 .......ooOo..o...o.......o..............................
 512 .......o.OoOo............o............................##
 640 ........OOOO............................................
 768 .....................O..................................
 896 ..oo..................oo................................
1024 ..oO.o...........oo..............o..o...................
1152 ..oo.........................o...oo.....................
1280 .o.oo......oo................o....oo....................
1408 oo..o.....o...............................o.............
1536 ....O........O....................................o.....
1664 .....o.....oo...........................................
1792 ....oo.....oo...........................................
1920 ........................................................
2048 .....o........o.......o.................................
2176 ....oo.........o......o.................................
2304 ....................ooo.....O...........................
2432 ..........................o.............................
2560 .............oo............oo...........................
2688 ........o.....oo........................................
2816 ................................................##......
2944 ..............oOo...............................Oo......
3072 ........................................................
3200 ........................................................
3328 ........Oo..............................................
3456 ........Oo....o.........................................
3584 ........OOOO............................................
3712 .........ooo.............................Oo...OO........
3840 ..........oO............................................
3968 ..........Oo............................................
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 7008,518..7164,637 | 18840 | 18840 | 234 | 0 | 0 | 18840 | +0/-0 |  |  |  |
| 1 | 6154,2848..6313,2944 | 13083 | 12955 | 0 | 949 | 10904 | 667 | +1720/-0 | 0x3609 ?, 0x1854 skull with candle, 0x0A22 lantern |  |  |
| 2 | 5990,3717..6034,3789 | 1095 | 919 | 71 | 366 | 0 | 0 | +96/-444 | 0x0CE6 willow tree, 0x0CE7 willow leaves, 0x0CDD oak tree | 0x19BA iron ore, 0x19B9 iron ore, 0x19AB fire | Lost Lands |
| 3 | 1234,3601..1455,3783 | 462 | 389 | 10 | 56 | 0 | 0 | +61/-0 | 0x17A7 water, 0x17A1 water, 0x17AE water |  |  |
| 4 | 1307,156..1472,270 | 235 | 102 | 94 | 57 | 0 | 0 | +56/-2 | 0x179C water, 0x1798 water, 0x1797 water | 0x0CE4 walnut leaves, 0x0CE3 walnut tree |  |
| 5 | 561,1611..576,1641 | 223 | 105 | 0 | 223 | 0 | 0 | +233/-106 | 0x21A3 nodraw, 0x326C swamp, 0x326A swamp | 0x0CAC grasses, 0x0CBB grasses, 0x3511 waterfall |  |
| 6 | 1046,3362..1146,3502 | 215 | 210 | 0 | 0 | 0 | 0 | +0/-0 |  |  |  |
| 7 | 1378,3954..1489,4032 | 208 | 172 | 15 | 19 | 0 | 0 | +20/-0 | 0x17A2 water, 0x177B rock, 0x17B0 water |  |  |

The same slots A and B and the same Britannia edits as facet 0, plus a Lost Lands spot at 5990,3717..6034,3789
(the base has iron ore 0x19B9/0x19BA and fire 0x19AB, the variant trees) and a Britain spot at
1672,1584..1679,1601.

## EC-2026 facet0 / facet0x

```
     0       1       2       3       4       5       6       
   0 ...............o..........................o........O#...
 128 ...............................O.............Oo....O#...
 256 ..........................o....OO...........O.#..o......
 384 ...................oo.........OO#..........#.....O......
 512 ..........o........oO#O.......o##O......................
 640 ..o..OOOo.o.........##O.........O#......................
 768 ..oooo....O.o.o.........................................
 896 ..O.oO.....O.....................O.O....O...............
1024 ...OO.............................OOO...................
1152 ...o.....oO.oo....................oo....................
1280 .........o........................O.O...ooo...Oo........
1408 ...........Oo.....................##....##O...oO........
1536 ...oo...o.O.O.....................##..............o.....
1664 ......##o.O#............................oO..............
1792 .....O#OOO.....OO.......................##o.....o#......
1920 .....##O.oO....OO...O#......OO..........#O......O##.....
2048 ....oo........Oo....OO.....O##..................O##....o
2176 ......O.............OO......##.....................oo.##
2304 ......O####.........O.....o#O.....................O##.##
2432 .......########............##.....................O##.oO
2560 .......########...........##o.....................O#O...
2688 ...........###.Oo.........##............................
2816 ...........o##OO..........o#..................##.O#.....
2944 ..............Oo..............................#######...
3072 .............###............................o...#####...
3200 .............O##.............................O..#####...
3328 ..............#......OOO...........O............####....
3456 .............o#......OOO........................#.......
3584 ...................O.............o......o.......##o.....
3712 ..........oo....................................##......
3840 ...................o.....................O......##......
3968 ........................................o.......##......
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 5939,2824..6593,3362 | 159087 | 145646 | 150948 | 3879 | 141391 | 0 | +6011/-874 | 0xA98C crystal wall, 0xA98B floor tile, 0xA98A crystal wall | 0x0D57 tree, 0x0D44 tree, 0x0D42 tree | Alchemist's Lab; Castle Blackthorn Dungeon; Hidden Library; Lost Lands; Quaderact; Secret Room |
| 1 | 853,2333..1919,2879 | 135154 | 91944 | 85565 | 42328 | 0 | 0 | +13280/-40350 | 0x179A water, 0x179C water, 0x179B water | 0x0CAF grasses, 0x0CB0 grasses, 0x0CB5 grasses |  |
| 2 | 6160,3491..6356,3999 | 84041 | 59873 | 83471 | 1430 | 59873 | 0 | +1724/-0 | 0x08E3 flowstone, 0x08E6 flowstone, 0x21A3 nodraw |  | Fiery Caves; The Manticore Passage |
| 3 | 3520,1992..3818,2312 | 31738 | 23798 | 8771 | 15734 | 0 | 0 | +21669/-5992 | 0x1797 water, 0x05B4 tile roof, 0x05B5 tile roof | 0x07C9 wooden plank, 0x07CD wooden plank, 0x2199 nodraw |  |
| 4 | 6976,2175..7141,2432 | 31613 | 30011 | 0 | 5570 | 0 | 30011 | +0/-5939 |  | 0xAE46 Tracks Straight Sout, 0xAE47 Tracks Straight East, 0xAE31 Wall Mine Beams 01 E | Orc Cave |
| 5 | 6260,1952..6446,2126 | 24594 | 14156 | 24391 | 1041 | 14156 | 0 | +1236/-0 | 0x08E6 flowstone, 0x1777 rock, 0x1778 rock |  | Crimson Stronghold; Titanweave Spider Lair |
| 6 | 6672,3043..6780,3239 | 21473 | 21473 | 6 | 2850 | 21473 | 0 | +4719/-0 | 0x04D7 wooden planks, 0x77ED BoardBatten WallCorn, 0x77EA BoardBatten WallHalf |  | Quaderact; Trinsic Flagship |
| 7 | 5123,1403..5379,1515 | 21042 | 16203 | 11212 | 2325 | 0 | 6847 | +2375/-4 | 0x0241 dungeon wall, 0x0242 dungeon wall, 0x0A0C wall torch | 0x079F cave steps, 0x079E cave steps | Dungeon Outpost |
| 8 | 1698,3060..1967,3503 | 20328 | 6529 | 7627 | 9708 | 0 | 0 | +659/-13443 | 0x07C9 wooden plank, 0x0C4C flowers, 0x0C4A flowers | 0x0CA3 fern, 0x0CA1 large fern, 0x0CA2 fern |  |
| 9 | 3352,2628..3566,2869 | 20085 | 19885 | 708 | 2921 | 0 | 0 | +1448/-2413 | 0x04A9 wooden boards, 0x0222 palisade, 0x0221 palisade | 0x0CAF grasses, 0x0CB0 grasses, 0x0CB5 grasses |  |
| 10 | 2602,582..2833,737 | 13582 | 12472 | 61 | 4561 | 0 | 0 | +3381/-4531 | 0x049C grey slate tile, 0x05C4 wooden shingles, 0x05A4 thatch roof | 0x0CAF grasses, 0x0CB0 grasses, 0x0CB5 grasses |  |
| 11 | 708,1706..988,2015 | 13199 | 6621 | 136 | 7470 | 0 | 0 | +64/-8968 | 0x0CAD grasses, 0x0CDB oak leaves, 0x0CDA oak tree | 0x0CAF grasses, 0x0CB0 grasses, 0x0CB6 grasses |  |
| 12 | 3455,2355..3671,2533 | 12129 | 10478 | 1264 | 2597 | 0 | 0 | +4230/-327 | 0x21A3 nodraw, 0x0CDB oak leaves, 0x0CE0 walnut tree | 0x0CAF grasses, 0x0CB6 grasses, 0x0CB5 grasses |  |
| 13 | 6516,2300..6774,2419 | 11155 | 9366 | 739 | 2326 | 9366 | 0 | +2375/-4 | 0x0241 dungeon wall, 0x0242 dungeon wall, 0x0A0C wall torch | 0x1796 water, 0x17AD water, 0x1798 water |  |
| 14 | 6516,2494..6774,2613 | 11152 | 9366 | 739 | 2322 | 9366 | 0 | +2375/-0 | 0x0241 dungeon wall, 0x0242 dungeon wall, 0x0A0C wall torch |  |  |
| 15 | 5129,1809..5271,2040 | 8048 | 7133 | 0 | 2386 | 1337 | 42 | +2651/-0 | 0x001B stone wall, 0x001C stone wall, 0x00C9 stone wall |  | Library Cellar; Ocllo Crypt; Ocllo Mines; Orc Cave; Rat's Nest |

- **New areas only in the variant** (blank in the base): 5939,2824..6593,3362 (crystal walls 0xA98A..0xA98D,
  floor tile 0xA98B), 6160,3491..6356,3999 (cave: rock 228, dirt, flowstone 0x08E3/0x08E6), 6260,1952..6446,2126
  (cave), 6672,3043..6780,3239 (water and a wooden structure, BoardBatten walls 0x77EA/0x77ED),
  6516,2300..6774,2419 and 6516,2494..6774,2613 (stone and dungeon walls 0x0241/0x0242). The EC-2026
  `facets.uop` defines regions over them, in facet 0 only: Quaderact, Trinsic Flagship, The Manticore Passage,
  Fiery Caves, Crimson Stronghold, Titanweave Spider Lair.
- **Mixed edits in existing dungeons** (content added and removed on both sides): 5123,1403..5379,1515
  (Dungeon Outpost region), 5129,1809..5271,2040 (Ocllo Crypt, Ocllo Mines, Library Cellar, Rat's Nest),
  5894,267..6005,373 (Ice, Ironheart Depths). These regions are also new in EC-2026 and in facet 0 only.
- **Only in the base:** the "Orc Cave" mine at 6976,2175..7141,2432 (cave land, mine tracks 0xAE46/0xAE47,
  mine beams 0xAE31..0xAE34). The base got it between 2024 and 2026 (with a small Trinsic building edit at
  1840,2819..1866,2838); this is the whole EC-2024 -> EC-2026 change of facets 0 and 1 (18 and 22 sectors).
  Its region is in both facet 0 and facet 1.
- **Britannia:** large edits around Trinsic and the swamp west of it (853..1919, 2333..2879: forests, water
  added, grasses removed), Magincia (3520..3818, 1992..2312: roofs and water added), the Ocllo area
  (3352..3671, 2355..2869), 2602..2833,582..737 (new roofs: slate, shingles, thatch), 708..988,1706..2015
  (grasses removed), 4429..4583,1464..1591 (stone pavers 0x051F, slate roofs) and the jungle south of Trinsic
  (1698..1967, 3060..3503).
- Slots A and B equal the base here: the EC-2024 variant content at B is gone and A is no longer blank.

## EC-2026 facet1 / facet1x

```
     0       1       2       3       4       5       6       
   0 ...............oo......ooo..............................
 128 ........ooOOoooo....o....oo.............................
 256 .........Oo..oo.........................................
 384 .......ooOo..o...o.......o..............................
 512 .......o.OoOo............o............................##
 640 ........OOOO............................................
 768 .....................O..................................
 896 ..oo..................oo................................
1024 ..oO.o...........oo..............o..o...................
1152 ..oo.........................o...oo.....................
1280 .o.oo......oo................o....oo....................
1408 oo..o.....o...............................o.............
1536 ....O........O....................................o.....
1664 .....o.....oo...........................................
1792 ....oo.....oo...........................................
1920 ........................................................
2048 .....o........o.......o...............................Oo
2176 ....oo.........o......o...............................##
2304 ....................ooo.....O.........................##
2432 ..........................o...........................oO
2560 .............oo............oo...........................
2688 ........o.....Oo........................................
2816 ..............O.................................##......
2944 ..............oOo...............................Oo......
3072 ........................................................
3200 ........................................................
3328 ........Oo..............................................
3456 ........Oo....o.........................................
3584 ........OOOO............................................
3712 .........ooo.............................Oo...OO........
3840 ..........oO............................................
3968 ..........Oo............................................
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 6976,2175..7141,2432 | 31613 | 30011 | 0 | 5570 | 0 | 30011 | +0/-5939 |  | 0xAE46 Tracks Straight Sout, 0xAE47 Tracks Straight East, 0xAE31 Wall Mine Beams 01 E | Orc Cave |
| 1 | 7008,518..7164,637 | 18840 | 18840 | 234 | 0 | 0 | 18840 | +0/-0 |  |  |  |
| 2 | 6154,2848..6313,2944 | 13083 | 12955 | 0 | 949 | 10904 | 667 | +1720/-0 | 0x3609 ?, 0x1854 skull with candle, 0x0A22 lantern |  |  |
| 3 | 5990,3717..6034,3789 | 1095 | 919 | 71 | 366 | 0 | 0 | +96/-444 | 0x0CE6 willow tree, 0x0CE7 willow leaves, 0x0CDD oak tree | 0x19BA iron ore, 0x19B9 iron ore, 0x19AB fire | Lost Lands |
| 4 | 1234,3601..1455,3783 | 462 | 389 | 10 | 56 | 0 | 0 | +61/-0 | 0x17A7 water, 0x17A1 water, 0x17AE water |  |  |
| 5 | 1307,156..1472,270 | 235 | 102 | 94 | 57 | 0 | 0 | +56/-2 | 0x179C water, 0x1798 water, 0x1797 water | 0x0CE4 walnut leaves, 0x0CE3 walnut tree |  |

`facet1x.uop` is the EC-2024 file, so this is the EC-2024 difference plus the base's new Orc Cave and
Trinsic edit, which the variant lacks.

## Classic CC-2024 map0 / map0x and map1 / map1x

The terrain is identical (0 map blocks differ in either pair); only statics differ.

map0 / map0x:

```
     0       1       2       3       4       5       6       
   0 ...............oo......o................................
 128 ........oooooooo....o.....o.............................
 256 .........oo..o.....O....................................
 384 .......oooo..o...o.......o..............................
 512 .......o.Oooo............o..............................
 640 ...oo...Oooo............................................
 768 ...OO................o.....................#............
 896 ......................o.................o...............
1024 ..oo.o...........o...............o..o...................
1152 ..oo.........................o...oo.....................
1280 .o.oo........................o.....o....................
1408 oo..o.....o...............................o.............
1536 ....O......O......................................o.....
1664 .....o..................................................
1792 ....oO.....o..........................................##
1920 ...oo.................................................OO
2048 ....Oo................................................O.
2176 ....o..........o......o.................................
2304 ....................oo......o....................o......
2432 ..........................o.............................
2560 .............oo.........................................
2688 ........O.....oo........................................
2816 ........................................................
2944 ..............ooo...............................#O......
3072 ........................................................
3200 ........................................................
3328 ........oo..............................................
3456 ........oo..............................................
3584 ........oooo............................................
3712 .........ooo..............................o.............
3840 ..........oo............................................
3968 ..........oo............................................
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 6912,1792..7167,1982 | 4534 | 0 | 0 | 4534 | 0 | 0 | +0/-5467 |  | 0x179F water, 0x17A3 water, 0x179D water |  |
| 1 | 6200,2990..6280,3026 | 1558 | 0 | 0 | 1558 | 0 | 0 | +0/-3635 |  | 0x3609 ?, 0x04AB wooden boards, 0x04AA wooden boards |  |
| 2 | 5536,804..5586,853 | 1334 | 0 | 0 | 1334 | 0 | 0 | +1675/-0 | 0x21A3 nodraw |  | Despise  |
| 3 | 495,766..544,850 | 1274 | 0 | 0 | 1274 | 0 | 0 | +323/-1372 | 0x179B water, 0x179C water, 0x1799 water | 0x07C9 wooden plank, 0x03A6 rope, 0x03A5 pier |  |
| 4 | 2493,323..2540,374 | 916 | 0 | 0 | 916 | 0 | 0 | +269/-1029 | 0x179A water, 0x1799 water, 0x1798 water | 0x07C9 wooden plank, 0x03A9 rope, 0x03A6 rope |  |
| 5 | 1114,2736..1151,2813 | 371 | 0 | 0 | 371 | 0 | 0 | +471/-12 | 0x3270 swamp, 0x3273 swamp, 0x3277 swamp | 0x0CE0 walnut tree, 0x0CE1 walnut leaves, 0x0CCE leaves |  |
| 6 | 460,2029..588,2117 | 368 | 0 | 0 | 368 | 0 | 0 | +179/-309 | 0x0CB0 grasses, 0x0CAF grasses, 0x0CB6 grasses | 0x1B08 floor cracks, 0x1B02 floor cracks, 0x1B01 floor cracks |  |
| 7 | 6978,2048..7032,2107 | 308 | 0 | 0 | 308 | 0 | 0 | +0/-340 |  | 0x04C3 wooden logs, 0x21A3 nodraw, 0x080D suspension bridge |  |

map1 / map1x:

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 6210,2882..6270,2942 | 949 | 0 | 0 | 949 | 0 | 0 | +1720/-0 | 0x3609 ?, 0x1854 skull with candle, 0x0A22 lantern |  |  |
| 1 | 5990,3717..6034,3789 | 366 | 0 | 0 | 366 | 0 | 0 | +96/-444 | 0x0CE6 willow tree, 0x0CE7 willow leaves, 0x0CDD oak tree | 0x19BA iron ore, 0x19B9 iron ore, 0x19AB fire | Lost Lands |
| 2 | 561,1611..576,1641 | 221 | 0 | 0 | 221 | 0 | 0 | +233/-106 | 0x21A3 nodraw, 0x326C swamp, 0x326A swamp | 0x0CAC grasses, 0x0CBB grasses, 0x3511 waterfall |  |
| 3 | 1081,598..1186,704 | 88 | 0 | 0 | 88 | 0 | 0 | +89/-2 | 0x1798 water, 0x179B water, 0x1797 water | 0x0CA7 rushes |  |
| 4 | 1672,1584..1679,1601 | 73 | 0 | 0 | 73 | 0 | 0 | +39/-116 | 0x0ADA ?, 0x0AE0 ?, 0x0AE2 ? | 0x0E34 blank scroll%s%, 0x9FB4 Town_Cryer, 0xA0E2 red poppies |  |
| 5 | 443,1022..492,1101 | 62 | 0 | 0 | 62 | 0 | 0 | +64/-4 | 0x17A7 water, 0x179E water, 0x179D water | 0x0CE4 walnut leaves, 0x0CE3 walnut tree, 0x0CCE leaves |  |

CC-2026 map1 / map1x (map1x equals map1; statics1x is the 2018 file, so it also lacks the Orc Cave mine):

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 6976,2175..7141,2432 | 5570 | 0 | 0 | 5570 | 0 | 0 | +0/-5939 |  | 0xAE46 Tracks Straight Sout, 0xAE47 Tracks Straight East, 0xAE31 Wall Mine Beams 01 E | Orc Cave |
| 1 | 6210,2882..6270,2942 | 949 | 0 | 0 | 949 | 0 | 0 | +1720/-0 | 0x3609 ?, 0x1854 skull with candle, 0x0A22 lantern |  |  |
| 2 | 5990,3717..6034,3789 | 366 | 0 | 0 | 366 | 0 | 0 | +96/-444 | 0x0CE6 willow tree, 0x0CE7 willow leaves, 0x0CDD oak tree | 0x19BA iron ore, 0x19B9 iron ore, 0x19AB fire | Lost Lands |
| 3 | 561,1611..576,1641 | 221 | 0 | 0 | 221 | 0 | 0 | +233/-106 | 0x21A3 nodraw, 0x326C swamp, 0x326A swamp | 0x0CAC grasses, 0x0CBB grasses, 0x3511 waterfall |  |
| 4 | 1840,2819..1866,2838 | 128 | 0 | 0 | 128 | 0 | 0 | +108/-152 | 0x0AD1 carpet, 0x0AEB ?, 0x0204 plaster wall | 0x0203 plaster wall, 0x076C sandstone, 0x04AA wooden boards |  |
| 5 | 1081,598..1186,704 | 88 | 0 | 0 | 88 | 0 | 0 | +89/-2 | 0x1798 water, 0x179B water, 0x1797 water | 0x0CA7 rushes |  |

CC-2026 map0 / map0x is the EC-2026 facet0 / facet0x difference exactly (same land, z and static counts;
see the cross-client table).

## Classic CC-2011 map0 / map0x and map1 / map1x

```
     0       1       2       3       4       5       6       
   0 ..........................................o.............
 128 ..........................................oO............
 256 ...............o........................................
 384 ...................oo...................................
 512 ........................................................
 640 ......................................................O.
 768 ..o.........#O.....oo......................o............
 896 ............##.....o..............oo............##......
1024 .................................o.o....o.......####....
1152 ................................................####....
1280 ................................................####....
1408 ....#O............................#O.................###
1536 ....##o...o.......................##......#..........###
1664 ..........o...............................#..........###
1792 ...........o............................................
1920 ............................OO..........................
2048 ...........................O##..........................
2176 ............................##.....O....................
2304 ............................O......#....................
2432 ........Oo..............................................
2560 ........##................o.............................
2688 ..........o..................#..........................
2816 ...........o............................................
2944 ........................................................
3072 ........................................................
3200 .............................................O..........
3328 ........................................................
3456 ........................................................
3584 ...............................................O........
3712 ...............................................#........
3840 ........................................................
3968 ........................................................
```

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 6272,1088..6591,1407 | 102400 | 102400 | 6625 | 2824 | 0 | 0 | +0/-2825 |  | 0x1797 water, 0x17A3 water, 0x17A4 water | Island of the Soulbinder |
| 1 | 6848,1408..7149,1709 | 91204 | 91204 | 0 | 0 | 0 | 91204 | +0/-0 |  |  |  |
| 2 | 6184,960..6399,1159 | 28752 | 19536 | 0 | 0 | 2640 | 16896 | +0/-0 |  |  | Island of the Soulbinder |
| 3 | 3537,2018..3809,2312 | 28279 | 23715 | 8403 | 4763 | 0 | 0 | +2907/-6134 | 0x1797 water, 0x1799 water, 0x179B water | 0x07C9 wooden plank, 0x07CD wooden plank, 0x2199 nodraw |  |
| 4 | 571,1504..695,1630 | 10965 | 5493 | 10921 | 5766 | 0 | 0 | +16403/-14 | 0x049B floor tile, 0x0521 stone pavers Dark, 0x0081 stone wall | 0x0CB1 grasses, 0x0CAC grasses, 0x179E water |  |
| 5 | 1085,2544..1213,2687 | 8800 | 7838 | 5917 | 5545 | 0 | 0 | +20944/-2385 | 0x0495 marble floor, 0x0518 cobblestones, 0x0517 cobblestones | 0x051D stone pavers, 0x00C5 stone wall, 0x071E stone |  |
| 6 | 1572,846..1685,1013 | 8732 | 4235 | 7575 | 6507 | 0 | 0 | +22438/-4 | 0x055C stone roof, 0x297D wooden floor, 0x2965 wooden floor | 0x0CB0 grasses, 0x0CB6 grasses, 0x0CB5 grasses |  |
| 7 | 4432,1484..4506,1587 | 6073 | 5929 | 2084 | 2441 | 0 | 0 | +466/-3329 | 0x1796 water, 0x179A water, 0x1797 water | 0x0519 stone pavers Dark, 0x0175 sandstone wall, 0x1797 water |  |
| 8 | 5427,1546..5502,1697 | 5546 | 5546 | 0 | 0 | 0 | 0 | +0/-0 |  |  |  |
| 9 | 6144,1216..6183,1343 | 5120 | 5120 | 0 | 0 | 0 | 5120 | +0/-0 |  |  |  |

map1 / map1x:

| # | bbox (x0,y0..x1,y1) | squares | land | z | statics sq | blank only in A | blank only in B | statics +B/-B | top +B | top -B | FacetDefinition regions overlapping |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 3654,2146..3741,2259 | 2724 | 2533 | 293 | 764 | 0 | 0 | +697/-151 | 0x0861 wooden fence, 0x0860 wooden fence, 0x085E wooden fence | 0x0110 marble wall, 0x010F marble wall, 0x0119 marble wall |  |
| 1 | 4428,3677..4467,3718 | 582 | 4 | 5 | 578 | 0 | 0 | +673/-267 | 0x0CA3 fern, 0x0C9F fern, 0x0CA2 fern | 0x0C88 snowdrops, 0x0C83 campion flowers, 0x0CD5 leaves |  |
| 2 | 5819,3230..5840,3271 | 418 | 0 | 125 | 352 | 0 | 0 | +382/-377 | 0x07C9 wooden plank, 0x03AA pier, 0x07CB wooden bridge | 0x07C9 wooden plank, 0x03AA pier, 0x07CB wooden bridge | Lost Lands |
| 3 | 6740,663..6752,666 | 29 | 0 | 0 | 29 | 0 | 0 | +22/-34 | 0x293F clay wall, 0x293C clay wall, 0x293B clay wall | 0x293F clay wall, 0x07BE platform, 0x293C clay wall | Event Arenas |
| 4 | 6740,712..6752,715 | 29 | 0 | 0 | 29 | 0 | 0 | +22/-34 | 0x293F clay wall, 0x293C clay wall, 0x293B clay wall | 0x293F clay wall, 0x07BE platform, 0x293C clay wall | Event Arenas |
| 5 | 6077,3704..6089,3707 | 29 | 0 | 0 | 29 | 0 | 0 | +22/-34 | 0x293F clay wall, 0x293C clay wall, 0x293B clay wall | 0x293F clay wall, 0x07BE platform, 0x293C clay wall | Lost Lands |

The CC-2011 map0x has the Island of the Soulbinder (6272,1088..6591,1407) as filler 430 instead of water,
blanks 6848,1408..7149,1709 and 6144,1216..6183,1343, and its own Magincia (3537..3809, 2018..2312). None of it
reached a later base: CC-2024 map0 equals CC-2011 map0 there (Soulbinder 102,400 of 102,400 squares,
6848..7149 91,204 of 91,204, Magincia 28,173 of 28,279).

## Cross-client: EC packages against the Classic files

EC package against the facet `uocf-cli facet from-classic` builds from the Classic map + staidx + statics
(squares; stale delimiters, a known property of the shipped EC data, are counted apart):

| EC package | Classic source | Land | z | Statics squares | Delimiters only |
|---|---|---|---|---|---|
| EC-2024 facet0 | CC-2024 map0 | 0 | 0 | 0 | 13,884 |
| EC-2024 facet1 | CC-2024 map1 | 0 | 0 | 0 | 22,848 |
| EC-2024 facet0x | CC-2024 map0x | 35,314 | 9,178 | 10,105 | 13,613 |
| EC-2024 facet0x | CC-2024 map0 | 35,314 | 9,178 | 4,633 | 13,823 |
| EC-2024 facet1x | CC-2024 map1x | 35,033 | 1,369 | 0 | 22,807 |
| EC-2026 facet0 | CC-2026 map0 | 0 | 0 | 0 | 14,827 |
| EC-2026 facet0x | CC-2026 map0x | 0 | 0 | 0 | 26,598 |
| EC-2026 facet1 | CC-2026 map1 | 0 | 0 | 0 | 23,855 |
| EC-2026 facet1x | CC-2026 map1x | 65,140 | 1,372 | 0 | 22,961 |

- The base packages always match Classic exactly. EC-2026 facet0x matches CC-2026 map0x exactly.
- EC-2024 facet1x has exactly the statics of CC-2024 statics1x (the static delta is +3,138 / -885 in both
  clients), but its terrain differs from CC-2024 map1x (which equals map1) in 35,033 squares.
- EC-2024 facet0x matches neither Classic map0 nor map0x: its terrain differs as above, and its statics
  differ from statics0x in 10,105 squares. Of the 4,633 squares whose statics differ from the base, 3,671
  also differ in CC-2024 statics0x.

## Lineage: older or newer?

Comparing square by square (z, land, statics) on the squares where base and variant differ:

- **Felucca and Trammel agree.** Where EC-2024 facet0 and facet0x differ (41,824 squares), facet1 equals
  facet0 in 40,160 (96%) and facet1x equals facet0x in 29,509 (71%). Where facet1 and facet1x differ
  (37,188), facet0 equals facet1 in 35,448 (95%) and facet0x equals facet1x in 30,795 (83%). The variant is
  a second edition of both facets, not a Felucca-against-Trammel difference.
- **EC-2024 variants keep 2011 data.** Of the 41,824 squares, facet0x equals CC-2011 map0 in 29,509: all
  7,990 squares at x < 5120 and 21,519 at x >= 5120 (mostly slot A). The base equals CC-2011 in 10,904 (slot B,
  where the variant added content) and neither in 1,411. Facet 1: 24,850 equal CC-2011 map1 (4,109 at
  x < 5120), 10,904 base, 1,434 neither. So the EC-2024 variants were built from older data and not
  refreshed when the base was edited, except for a few additions of their own.
- **EC-2026 facet0x is new content, not old.** It differs from CC-2011 map0 in 910,565 squares (EC-2024
  facet0x: 244,030; EC-2024 facet0: 263,152), but its Magincia is the CC-2011 map0x Magincia: where the
  EC-2026 base and CC-2011 map0x differ in that box, facet0x equals CC-2011 map0x in 17,679 squares, the base
  in 394, neither in 10,207. The Magincia layout of the `x` line survives from CC-2011 map0x to the 2024-10
  build and never enters the base.

## Felucca against Trammel

EC facet0 against facet1 differs exactly as Classic map0 against map1 (EC-2024 and CC-2024: land 983,005, z
640,600 squares, statics +118,262 / -94,241 in both; the EC sectors also differ in the header facet byte).
That follows from the base packages matching Classic. The variants differ from each other a little more (EC-2024
facet0x / facet1x: 1,212,052 squares; EC-2026: 1,838,496, because facet0x got the new areas and facet1x did
not).

## Radar images

EC-2026 ships `facet00x.dds` next to `facet00.dds` (same size and header). Level 0 differs in 46,897 of
1,835,008 DXT1 blocks; 46,773 of them (99.7%) lie on 4x4 blocks touched by the facet0 / facet0x square diff,
and 7,637 touched blocks have equal radar pixels. It is the radar of the variant. The UOSA build traced in
[uosa_facets.md](uosa_facets.md#facetnx-variants-and-the-classic-x-maps) opens only `facet%02d.dds`, so that
client shows the base radar on the variant map. EC-2024 has no `facet00x.dds`
although its facet0x differs too, and no install has `facet01x.dds`. EC-2026 also changed `facet01.dds`.

## Conclusions

- The `x` packages are a second edition of the Felucca and Trammel maps (facets 0 and 1), selected
  per server. Facets 2 and 5 ship variants with identical data. Both facet editions share most of their
  changes, so the variant is not a Felucca-only thing in the EC-2024 and Classic data.
- The editions differ in whole areas present in one and blank (0x244) in the other, mostly in the Lost Lands
  dungeon slots (x >= 5120), and in town, coast and terrain edits in Britannia. Each generation differs in
  where: 2011 Soulbinder and Magincia, 2024 slots A and B, 2026 six new Lost Lands areas (facet 0 only) and
  large Britannia edits.
- Maintenance is uneven. Classic map0x/map1x terrain followed the base in 2024 while the EC-2024 variants kept
  2011 terrain; EC-2026 facet0x and CC-2026 map0x were rebuilt together (2024-10-10) and match exactly, while
  facet1x, statics1x/staidx1x and the facet 2/5 variants were not touched since 2018-2024. Neither variant
  follows the base's later additions (Orc Cave mine).
- Interpretation (medium): the flag selects a map edition for a group of shards that differs from the
  standard one, with content the standard shards do not get (EC-2026: the new Lost Lands dungeons whose
  regions are already in `facets.uop`) and without some content they do get. The emulators' name for bit
  0x8000, "unlock / new Felucca areas", fits EC-2026, where only facet0x changed and the new regions are in
  facet 0 only. Which shards set the flag is not in the data. A tool that reads a facet must be told which
  edition the server uses: the same coordinates can be a dungeon in one and blank in the other.

## Open questions

- When the server sets 0x8000 and which shards do (code side and server side).
- Why EC-2024 facet0x matches neither Classic map0 nor map0x, while EC-2024 facet1x has the Classic
  statics1x: the EC variants were evidently built from a different snapshot.
- Whether a newer EC build than the one traced reads `facet00x.dds` (the traced UOSA does not; there is no
  variant radar for facet 1, and none at all in EC-2024).
- What slot A (7005,518..7166,650, terrain only, no statics, no region) is for.
