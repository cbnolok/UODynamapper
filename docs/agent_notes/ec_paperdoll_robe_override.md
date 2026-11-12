# Enhanced paperdoll: what a robe hides

The Enhanced client draws its 3D paperdoll in the `WearableRules.kdl` order
(`uocf::enhanced::paperdoll::WearableRules`). Under a dress-layer item it still
draws the shirt, trousers, skirt and chest pieces beneath it, and their 3D
frames do not line up with the robe's: their edges show past it at the collar,
down the sides and at the hem. Rules worth knowing:

- `chest any` carries `override priority=11900 { when "belowchest" ... }`. The
  client drops the unknown slot, so the override is unconditional: a chest piece
  is always drawn at 11900, beneath any dress.
- Legs are hidden only under a few robes (30753; 9859, 9860, 9863, 12217, 12218,
  7939); plate legs are drawn at 15000 under those. Feet are hidden under plate
  legs 5137 and 5146, and under dress 30753.
- Arms and abovechest are 0 under any dress, with exceptions for epaulets and
  some abovechest items drawn over it (the tabard 0xA412 at 18100). Platemail
  arms 0x1410 (5136) have a rule of their own, which replaces the rule for any
  item and does not hide them under a dress: they are drawn beneath every robe.
  The skirt is not hidden under a dress.

Not every dress-layer item is a robe. Their Enhanced frames: epaulets 0x9985,
0x9986 and parrot epaulets 0xA2CA, 0xA2CB draw only the shoulders; "kimono"
0x2681, 0x2682 draws a sword at the hip and little else; hakama-shita 0x279C,
0x27E7 is a jacket; tabard 0xA412 leaves sleeves and legs; wedding dress 0x7821
is sleeveless; red dress 0x781E is short; gargish fancy robe 0x4002, 0x4003 is
open at the front. Dress-layer items with no frame at all: 0x2042, 0x2043,
0x2050, 0x25EE, 0x25EF, 0x279E, 0x279F, 0x27E9, 0x27EA (and 0x2783, 0x27CE on
male bodies).

## The packed decisions

`pack-ec-paperdoll` measures every dress-layer item with a frame on each player
body (human, elf, gargoyle; male, female), garment by garment
(`udd_conv::paperdoll_ec_dress`). Every garment of the slots torso, legs, skirt,
arms, chest and abovechest the client draws beneath the item is drawn with and
without it; the garment is kept when it loses 1,600 texels or more beyond 16
texels from the item's edges, and taken off otherwise. The decisions are stored
per item and body in `metadata/dress_decisions.v3.bin` of paperdoll_ec.uddp
(metadata version 3; docs/dev_wiki/UDDP_FORMATS.md): the slots they cover and the
garments each keeps. At run time every garment of those slots drawn beneath the
item is taken off unless kept; a garment drawn beneath it only in another outfit
(its draw order changed by the other items) was never measured and is taken off
too. A garment the packer cannot measure on the body stays: one without a
`tileart.uop` Layer (worn on its tiledata layer) or without a 3D frame for the
body (`paperdoll_overrides::measured_garment`); the gump paperdolls draw many of
those, such as the chainmail tunic 0x13BD and the leather leggings 0x13C9. The packing log has one line per item and body, naming the garments kept
(the first 8, the most in view first); `--report` writes one line per garment.

The band and threshold come from the reviewed hand-written rule. The raw count
of texels taken away does not separate the cases: a robe over plate armour loses
up to ~11k texels at a slit, collar and cuffs, all near the dress. Beyond the
band, on cloth and armour outfits over every dress, robes lost at most 911
texels, and items left in view at least 3,033.

Metadata version 2 decided a slot at a time, and the census of EC 2024 (61,120
garment measures) chose the largest garment to decide it:

| rule                         | in view taken off | edges kept | drawn differences from the review |
|------------------------------|------------------:|-----------:|----------------------------------:|
| largest garment              | 458               | 2,287      | 21 |
| all garments hidden          | 0                 | 7,477      | 214 |
| most garments hidden         | 797               | 563        | 27 |
| a quarter of them showing    | 627               | 854        | 24 |

"In view taken off" counts garments in plain view that go with their slot (mostly
the amazon armour 0x265D, 0x265E under robes; also 0x2659, 0x265A, ringmail tunic
0x13EC, 0x13ED, ninja shirt 0x2667, 0x2668, do-maru 0x2671, 0x2672). "Most
garments" also hides plate legs under the red dress 0x781E.

Against the reviewed hand-written file, the stored decisions differ on:

- Tabard 0xA412, all bodies: keeps torso (10 to 16 of 14 to 28 shirts show
  around it, e.g. Shirt_Tux 0x9EFB 4,857 texels beyond the band, fancy shirt
  0x1EFD 4,815; the reviewed reference 0x1517 is one of the few that do not).
- Male kimono 0x2782, 0x27CD on female bodies: keeps legs (Skirt_Long_Tiger_H
  0x7826 shows 2,160 beyond), so plate legs keep their edges there.
- Wedding dress 0x7821 on male bodies: keeps skirt (the hakama 0x279A, 0x27E5
  shows 2,132 beyond).
- Gargish fancy robe 0x4002, 0x4003 on female bodies: hides legs and skirt (no
  garment shows beyond; the review decided per item from the male bodies).

Deciding each garment (version 3) fixes all of the slot rule's cases: the 458
garments in plain view are kept and the 2,287 edge-only ones taken off (the
report marks each). EC 2024: 372 decisions, 113 keeping nothing beneath the
item, 259 some; 9,770 garments kept, 51,350 taken off.

The threshold now meets the grey zone between 911 and 3,033 texels that the
slot rule hid behind the largest garment: 1,118 garments are taken off with 912
to 1,599 texels beyond the band, under every kind of dress-layer item (the red
dress 0x781E 127, the wedding dress 0x7821 122, hakama-shita 64, epaulets 53,
robes too: up to 1,328 under the death shroud). Some are in view on the sheets:
the shirt 0x1517 under the strapless wedding dress loses its collar and sleeves
(1,531 beyond), the bone legs 0x1452, 0x1457 under the red dress (1,595), the
body sash 0x1541 under the parrot epaulets (1,551). No threshold separates
them from the robes' edges; overrides (or the paperdoll's right-click menu) keep
them.

## Dynamapper

`core/render/dialogs/paperdoll_overrides.rs` applies the decisions to both of
Dynamapper's paperdolls, under the user's `PaperdollOverrides.kdl`
(`udd_assets::paperdoll_overrides`, strict schema; the bundled file holds
comments only). A cover decides for its covering items, on its `bodies=` (body
frame numbers; every body without it). Inside it, `show items=` keeps those
garments and `hide items=` takes them off (garment rules); `hide "<slot>"`
takes off the rest of that slot (slot rules); `except items=` leaves covering
items alone. Precedence: garment rules beat slot rules, which beat the stored
decision; among the covers that apply, the last with a garment rule naming the
garment decides it, else the last with a slot rule (a cover with no rule keeps
everything), else the stored decision. A cover with garment rules alone leaves
the other garments to the covers before it. Slot rules are literal: they also
take off the garments the packer could not measure, which the stored decisions
leave on. Only what the client draws beneath the item is taken off; a taken-off
item counts as not worn and the client's order is taken again, so taking off
plate legs brings the shoes back. The uocf editor keeps the client's rules.

The paperdoll windows (3D and gumps) take a right-click
(`core/render/dialogs/paperdoll_menu.rs`, a context menu inside the window, no
viewport of its own): the item picked is the topmost drawn whose texel under the
pointer is at least half opaque (else the topmost drawn there at all). For a
garment drawn beneath the drawn dress-layer item the menu offers "Show under
this robe", "Hide under this robe" and "Back to measured", the current choice
checked; it lists the garments taken off beneath the robe, each a submenu with
the same choices. A choice is for that robe on the body frame drawn (none
without paperdoll_ec.uddp) and goes to the user's copy,
`cache/runtime_specs/paperdoll/PaperdollOverrides.kdl`: each runtime spec has
its user copy at its assets path under the cache dir (`configs::runtime_specs`,
`SpecFiles`), as the settings files have theirs in `cache/settings/`, and
Dynamapper reads the copy in place of the bundled file when it exists; so do
`PaperdollProfiles.kdl` and `WearableRules.kdl`. The menu first wrote
`cache/settings/PaperdollOverrides.kdl`; such a copy moves to the new place at
startup, with an info line, unless a copy is there already (then a warning says
the old one is not read).
`udd_assets::paperdoll_overrides::set_garment_rule` edits the KDL text by its spans, keeping comments and other covers, in a cover per robe
and body headed "chosen in the paperdoll window"; the file is written through a
temporary file renamed over it and every open paperdoll is drawn again. A copy
that does not decode at startup is neither read nor written (a warning, the
bundled file applies); a choice re-reads the copy from disk first, so one fixed
meanwhile is taken up. The same holds for the profiles and the wearable rules:
strict, a copy that does not read leaves the bundled file in force with a
warning in the log and in game; the bundled files are required. The uocf editor
and the paperdoll_ec.uddp packer build the bundled files in, so a user's
`WearableRules.kdl` changes Dynamapper's paperdolls only.

The gump paperdolls take the decision for the frame the 3D paperdoll draws for
the profile's body, and none without paperdoll_ec.uddp. The gump art shows the
same artifact with armour: the cloth lines up (a shirt shows at most at a robe's
open collar), but plate, chainmail and other armour gumps are drawn wider than a
robe and their dark outline shows around its shoulders and sleeves. The ignored
test `gump::robe_edges::what_the_gump_paperdolls_show_beneath_a_robe` (gumps_cc,
gumps_ec, paperdoll_ec, tilemeta) measures every garment under every dress on
the six profiles: of 17,166 Classic pairs drawn beneath, nothing shows on 1,464
and only edges (within 11 logical texels, the 3D band at the window's 0.7) on
10,749. The stored decisions take off 7,560 of those edge-only pairs and 3 in
plain view (784 texels beyond, the 3D threshold at 0.7 across and down): the
short tiger pants 0x7825 under the kimono 0x2681 on male bodies, 1,201 texels
(416 on the 3D frame); with gumps_ec.uddp, 12: those and the do-maru 0x2671
under the plain, fancy and wedding dresses, about 800 to 900 texels. With the
slot rule of version 2 they took off 8,616 edge-only pairs and 0 (gumps_ec: 6)
in plain view. Taking off the garments the packer did not measure (no Layer of
their own, or no 3D frame for the body) took off 302 more in plain view.

The 3D paperdoll is hued from hues.uddp, the single hue source; paperdoll_ec.uddp
no longer carries the tables of `Hues.uop` (format 3). The ignored tests in
`udd_conv::hues::client_tables_tests` prove what hues.uddp holds, table by table:
packed from the Enhanced `Hues.uop`, all 3,000 tables exactly (the top row of each
`hue%04d.bmp`); packed from Classic `hues.mul`, all 3,000 equal the ramp of its 32
colours, which matches the Enhanced table for 2,635 hues with the linear ramp and
599 with the flat one. The pixel test
`paperdoll_ec::tests::the_packed_paperdoll_draws_what_the_editor_composes` takes
`UDD_HUES`, a hues.uddp packed from the client's `Hues.uop`, and draws 0 texels
apart from the editor; a hues.uddp packed from `hues.mul` colours the 365 (linear)
hues whose ramp differs as the Classic client does.

The ignored test `paperdoll_overrides::measure::the_stored_decisions_take_away_little_outside_the_dress`
(`UDD_PAPERDOLL_EC`, `UDD_TILEMETA`, `UDD_HUES`, `TMPDIR`; `UDD_PAPERDOLL_OVERRIDES` measures
another file) draws every dress over cloth and armour outfits and over each
garment alone, and writes the ranked list and contact sheets' data. With the
stored decisions nothing is flagged. The most taken away beyond the band was 911
texels with the slot rule, as with the reviewed file; deciding each garment it is
1,551 (the body sash under the parrot epaulets), and 104 of the 1,800 cases
change, all under dresses that leave some garments of a slot in view.
