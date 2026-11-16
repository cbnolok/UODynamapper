# Kingdom Reborn's own world: the cave arches only its facets hold

Why the Kingdom Reborn client draws a rock arch over the Minoc mine's mouth when the Classic and Enhanced clients,
and Dynamapper, draw only the rock framing a black opening; and how much KR's facets differ from the Classic map and
statics Dynamapper packs. Data only: no executable was run or decompiled for this study. The sector format is the one
traced in [`ghidra-facets`](../ghidra-facets/results/uokr_facets.md) (`UOKR.exe` 2.58.0.6).

Clients read (2026-11-16, read only):

- KR: `UO - Old KR` (`UOKR.exe` 2.58.0.6), `facet0.uop`..`facet4.uop` and `StaticChunk.uop`;
- Enhanced: `_Ultima Online Enhanced`, `facet0.uop`..`facet4.uop`;
- Classic: `Ultima Online Classic_7_0_20_0` (`mapN.mul`, `staidxN.mul`, `staticsN.mul`), and the newest Classic
  folder `_Ultima Online Classic` for the Minoc statics and the item art.

[`kr_facet_statics.py`](kr_facet_statics.py) reproduces every number below:

```sh
python3 -I -B docs/client_research/kr-facet-statics/kr_facet_statics.py area 1 2553 497 2564 505
python3 -I -B docs/client_research/kr-facet-statics/kr_facet_statics.py census 1     # about 10 minutes for facets 0 and 1
```

## The arch is in KR's facet statics

The KR "cave entrance" art is the ids 0x3B60-0x3B83: the pieces of one rock arch, each standing on its square. Where
a client's statics place the whole set, the arch is drawn (Enhanced and KR art; the Classic art of every one of
these ids is an empty 14-byte entry, in `art.mul` 7.0.20 and in the newest `artLegacyMUL.uop`, so Classic never
draws an arch).

At the Minoc mine's mouth (Trammel, facet 1):

| Source | Cave entrance pieces at 2553..2564, 497..505 |
|---|---|
| KR `facet1.uop` | 40: the whole arch at z 2 on 2555..2562, 498..502 (0x3B60-0x3B83), and the 12 pieces below |
| Enhanced `facet1.uop` | 12: 0x3B63, 0x3B65 and 0x3B66 at 2557..2559, 501..502, and a row at 2555..2563, 503..504 |
| Classic `statics1.mul` (7.0.20 and newest) | the same 12 as Enhanced |

Felucca (facet 0) has the same arch in KR, at z 0 and one square east (2556..2563, 498..502); Enhanced and Classic
have only 0x3B63, 0x3B65 and 0x3B66 there.

KR changed the mouth around the arch too. Under it the cave floor is the cave rock 0x053B-0x054C where Classic has
0x2198 and 0x0547, and the land is lower at three squares: 2560,499 z 17 (Classic 23), 2560,500 z 16 (20) and
2561,500 z 0 (18); 2561,499 is land 0x0236 z 14 (Classic 0x022C z 20) and 2560,501 land 0x0076 (Classic 0x0071).

The cave north of Britain (1262..1268, 1250..1255) has the whole arch in all three clients, which is why Dynamapper
already draws that one.

Not the source:

- `StaticChunk.uop`: none of its 145 entries holds a piece of the arch.
- The art: the pieces are the same ids at Britain, where Dynamapper draws the arch from the Enhanced and KR art;
  the three ids Classic and Enhanced place at Minoc (0x3B63, 0x3B65, 0x3B66) have blank KR art (found in the
  2026-11-15 cave work), so on their own they draw no arch.
- A client rule: the sectors' statics account for the whole arch, so no substitution rule is needed to explain it.
  No executable code was traced for this study.

So the Enhanced Client draws no arch at Minoc (its facets have the Classic statics there; there is no Enhanced
screenshot of the mine to confirm it), and neither does ClassicUO, which draws the Classic files: KR and Enhanced
differ here, and the arch is KR-only data.

## How far KR's world differs from Classic's

Per facet, KR sectors against Classic 7.0.20 MUL (`census`): statics compared per square as a multiset of
(graphic, z); "repeats" are squares holding the same statics with a different count, mostly KR's water statics
0x1796-0x179C stacked up to 19 times on one square.

| Facet | Land z differs | Land id differs | Statics differ | Differ only by repeats | Statics only in KR | Only in Classic | Cave entrance pieces KR / Classic |
|---|---|---|---|---|---|---|---|
| 0 Felucca | 188,462 | 368,802 | 30,878 | 49,121 | 11,655 | 52,097 | 980 / 740 |
| 1 Trammel | 213,544 | 607,874 | 49,314 | 48,247 | 32,058 | 47,022 | 979 / 773 |
| 2 Ilshenar | 201 | 465 | 161 | 3,860 | 120 | 141 | 250 / 250 |
| 3 Malas | 120 | 185 | 194 | 1,543 | 239 | 35 | 295 / 131 |
| 4 Tokuno | 378 | 329 | 525 | 1,631 | 39 | 997 | 141 / 141 |

- Every KR static graphic and hue fits in 16 bits, so the `staticsN.uddp` chunk record holds them losslessly.
- KR's world is another edition of the map: it lacks statics Classic has (the most frequent: 0x1797 water and
  0x07C9 rock on every facet, Felucca's 0x21A3 and 0x0518-0x0524) and has its own, among them more cave
  entrance pieces: 240 on Felucca, 206 on Trammel and 164 on Malas, about 30 to an arch, the arches KR sets over
  cave mouths that Classic leaves open.
- Land differs in id (the legacy Classic id KR stores beside its material) on 0.4-0.6 million squares of facets 0
  and 1, and in z on about 0.2 million. KR draws its land from its own sectors (material, subtype and z,
  `ghidra-facets`), so KR's ground at those squares is not Classic's.
- KR has no facet 5 (Ter Mur).

## What drawing it takes

Dynamapper's `mapN.uddp` and `staticsN.uddp` are packed from Classic files only, and the KR packages
(`tex_land_kr.uddp`, `tex_art_kr.uddp`, `tex_aux_kr.uddp`, `tilemeta_kr.uddp`) carry textures and tile metadata,
not world data. The arch therefore needs KR's world data (statics, and the land z the arch sits on) as a new map
and statics source. That is a scope decision, proposed in the report of 2026-11-16 rather than built.
