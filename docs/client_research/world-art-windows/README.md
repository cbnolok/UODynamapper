# World-art windows: why EC's tileart cuts the creature statuettes

For 215 items, most of them creature statuettes ("... Frame", monsters, horses: 0x20D5 Dog Frame, 0x210D Elf Female,
0x2108 mountain goat), the Enhanced Client's tileart record gives a smaller world-art window and another draw offset
than Kingdom Reborn's for the same texture, so Dynamapper's Enhanced style draws them cut off where KR-like draws the
whole figure. Does the real Enhanced Client draw them cut off, or do we read the wrong field?

**Answer.** Neither. The Enhanced Client never draws these items' world art: their records lack `UseNewArt`, so it
draws the TileArtLegacy (Classic) image with the legacy window, which holds the whole statuette. Their enhanced
window is a field that client never reads. It is a leftover: the legacy window copied in, sometimes with one or two
values from KR's window. No field of the EC record describes the whole world-art figure. Kingdom Reborn, the
client that does draw this art, reads the enhanced window of its own record, and that window fits the figure (in 190
of the 215 it is exactly the image's alpha bounds). We read the right field (the one both clients' mode-1 code reads)
from the one client whose record holds no real value for it.

Clients (2026-11-17, read only, nothing executed):

- Enhanced: `_Ultima Online Enhanced`, `UOSA.exe` 4.0.102.3 (PE FileVersion; SHA-256 `5a5e9dc5...ebad50`),
  `tileart.uop`, `string_dictionary.uop`, `Texture.uop`, `LegacyTexture.uop`;
- Kingdom Reborn: `UO - Old KR`, `UOKR.exe` 2.58.0.6 (PE FileVersion; the folder's `buildinfo.txt` says 2.53.0.2),
  `tileart.uop`, `string_dictionary.uop`.

[`world_art_windows.py`](world_art_windows.py) reproduces every count below (about 15 seconds):

```sh
python3 -I -B docs/client_research/world-art-windows/world_art_windows.py \
  '/path/to/_Ultima Online Enhanced' '/path/to/UO - Old KR' [--list]
```

## 1. What the records hold

Fields of one record (`lib/uocf/src/enhanced/metadata/tileart.rs`, `docs/client_research/ghidra-tileart`) that bear on
the sprite: `has_world_art` (+0x0a), `world_art_scale` (+0x0c), `legacy_art_scale` (+0x10), `flags2` (+0x41,
`UseNewArt` 0x80000), the enhanced window (+0x4d, `ec_img_offset`: x_start, y_start, x_end, y_end, x_off, y_off),
the legacy window (+0x65, `cc_img_offset`), shader block 0 (WorldArt image) and block 1 (TileArtLegacy image).
KR's record (version 3) has the same fields except `uses_enhanced_tileart`.

Example records (windows as x_start, y_start, x_end, y_end, x_off, y_off; scale is `world_art_scale`):

| Item, scale | EC enhanced window | EC legacy window | KR enhanced window | EC window cuts |
|---|---|---|---|---:|
| 0x20D5 Dog Frame, 0.8 | `0,0,44,44,0,0` | `0,0,44,44,0,0` | `0,0,28,65,21,-9` | 20.9 % |
| 0x210D Elf Female, 0.55 | `0,0,44,51,0,0` | `0,0,44,51,0,0` | `0,0,29,69,18,-9` | 27.0 % |
| 0x2108 mountain goat, 0.6 | `0,0,44,44,0,-10` | `0,0,44,44,0,0` | `0,0,20,49,22,-10` | 3.5 % |
| 0x20E5 Serpent Frame, 0.35 | `0,5,44,47,0,0` | `0,0,44,47,0,0` | `0,5,21,78,20,-1` | 40.3 % |
| 0x20FA reaper, 0.4 | `0,0,58,55,-7,0` | `0,0,58,55,-7,0` | `0,0,91,89,-2,-11` | 56.9 % |
| 0x20E8 Slime Frame, 1.4 | `0,1,60,56,-8,0` | `0,0,60,56,-8,0` | `0,1,50,39,0,-11` | 0 % |
| 0x2135 desert ostard, 0.6 | `0,0,44,44,0,0` | `0,0,44,44,0,0` | `0,0,109,114,-4,-2` | 100 % (drawn blank) |

"Cuts" is the share of the world-art image's alpha mass outside the window (mode 1: inclusive ends).

Census of the 15,270 EC static records with world art (static namespace by stored tile id):

| | Records |
|---|---:|
| same enhanced window as KR's record | 12,799 |
| no KR record (items added after KR) | 2,163 |
| KR record without world art | 92 |
| **other window, same texture and same `world_art_scale`** | **215** |
| other window and texture (0x34D2 water: a water-shader camera quad in EC, a blank sprite in KR) | 1 |

All 215 lack `UseNewArt` (so do almost all records: it is set on 795 of 36,589). In 150 the EC enhanced window *is*
the EC legacy window; in the other 65 it is the legacy window with one or two of KR's values, mostly KR's `y_start`
(the Serpent Frame), sometimes KR's `y_off` (the mountain goat). 149 of them end at x = 44, the Classic tile width.
All 215 have `legacy_art_scale` 1.5. The legacy window does fit the TileArtLegacy image (it holds all of that image's alpha in 213 of the 215; 0x2100 wisp and 0x3789 death
vortex are blank). The KR window fits the world-art image: exactly its alpha bounds in 190, holding all of it in 18
more, cutting under 1.5 % in 5; two are blank images. The EC window cuts more than 2 % of the figure in 194; in the
other 21 it holds the figure inside a larger window with other offsets (the Slime Frame; the bird 0x20EE, a 16x11
image in a 44x44 window where KR has `0,0,15,10,24,-21`).

So no EC field gives the figure: the enhanced window is the copied legacy window, the legacy window describes the
Classic image, and the scales agree with KR's. The 215 are the 211 of class B1 of the KR art comparison (statuettes,
identical pixels, `kr-art-diff`), the bird (B4), the desert ostard (B3) and the blank wisp and death vortex (E).

The same copy appears in 251 EC-only records (no KR record) whose window equals their legacy window. Ten of them have a
world-art image, each another item's: glow 0xA1FE-0xA201 (the lantern 2597, held whole), ship 0xA432/0xA440 (ship
15996, 30.6 % cut), orchid 0xA645-0xA648 (Campion Flowers 3209, 52.9 % cut). No client draws those records' world
art with an authored window.

## 2. What our converter and renderer use

`TileArtEntry::process` (`lib/uocf/src/enhanced/metadata/tileart.rs`) builds `ArtData::ec_texture` from block 0 and
the enhanced window; `ArtDefinition::from_package` (`tile_database.rs`) stores the same window as
`ArtSpriteDescriptor::world_window`. The `tex_art_ec` converter (`lib/udd-conv/src/textures/tex_art_ec.rs`,
`selected_art_source`, `selected_art_window`, `build_sprite_geometry`) crops the block-0 image to that window (ends
+1) and takes its offsets as the slot's draw offset, and `world_art_scale` as the pixel scale: the client's mode 1,
exactly. The Enhanced style draws every item with a `tex_art_ec` slot this way whatever `UseNewArt` says (a
deliberate project choice, `docs/dev_wiki/ASSET_PIPELINE.md`: the slot is the KR art). The KR-like style takes the
slot of the KR section instead when the KR records differ (`lib/udd-conv/src/textures/kr_items.rs`,
`compare_world_art`), which is why the 211 statuettes look whole there.

## 3. What the clients' code does

### Enhanced Client, `UOSA.exe` 4.0.102.3

`UOTileArt::BuildSpriteDescriptor` `0x004592e0` (decompiled again for this study; first traced in
`ghidra-tileart` and `ghidra-legacy-art-scale`), reached for every world item through
`UOObjectDefinition::GetSpriteDescriptor` `0x004c9240` (vtable +0x13c, also called from `FUN_0042ef10`) and
`UOTileArt::GetCachedSpriteDescriptor` `0x00457a70`:

```
if (use_legacy)                       // g_UOWorldViewer +0x54, never set nonzero
    mode 0
else if (definition->HasFlags2(0x80000 UseNewArt))       // vtable +0x5c
    mode 1: block 0 (vt +0x94), world_art_scale (vt +0x2c), GetEnhancedImageWindow (vt +0x14), ends + 1,
            ends = 44 when HasWorldArt (vt +0x1c) is 0
else if (UsesEnhancedTileArt)         // 0 in every record
    mode 2: block 2, legacy scale x 2/3, legacy window x 1.5
else
    mode 0: block 1 (vt +0x98), legacy_art_scale (vt +0x30), GetLegacyImageWindow (vt +0x18)
```

The enhanced window is read only in mode 1. Without `UseNewArt` the client draws block 1 (TileArtLegacy) with the
legacy window and `legacy_art_scale` (1.5 in all 215): the Classic statuette, whole. There is no setting that turns mode 1 on for other records.

### Kingdom Reborn, `UOKR.exe` 2.58.0.6

The descriptor builder `FUN_00441dc0` (sole caller of the quad builder `FUN_00442890`, which calls
`UOSprite::BuildSpriteQuad` `0x00442430`) does the same with one more switch:

```
if (!use_legacy && (!DAT_00c93b31 || definition->HasFlags2(0x80000)))   // vtable +0x58
    world art: block 0 (vt +0x90), world_art_scale (vt +0x28), enhanced window (vt +0x14), ends + 1,
               ends = 44 when HasWorldArt (vt +0x1c) is 0
else
    legacy: block 1 (vt +0x94), legacy_art_scale (vt +0x2c), legacy window (vt +0x18)
```

`DAT_00c93b31` is the `legacy` attribute of the `Art` settings node (stored at `0x00476b7c` in the settings reader `FUN_004769f0`; strings
`Art` `0x00b54ce4` and `legacy` `0x00b54cdc`), 0 by default (written 0 in `FUN_0047b660` `0x0047b9e7` and
`FUN_0047bac0` `0x0047bb06`; `ApplyResolutionSettings` copies it at `0x0047e9be`). So KR draws the world art of every item that
has some, with its enhanced window, unless the player picks legacy art; then it behaves as the Enhanced Client always
does. The Enhanced Client dropped the switch and kept only the `UseNewArt` path, so the enhanced windows of the other
records went unread; those of the statuettes hold copies of the legacy window.

## 4. Client screenshots

None of the user's Enhanced (`~/Immagini/Schermate/Schermata_20261113_*`, `_20261116_140951`) or KR captures
(`uokr/`, 24) shows one of the 215 items. The statues in the KR captures (Britain, the mage tower) are other ids.

## 5. Dynamapper at the reaper statuette

Map 1 (Trammel) holds a few of the 215 in its statics: the Slime Frame 0x20E8 at 21 places around 5574..5575,
3015..3020 (z 25..27), the reaper 0x20FA at 5594, 3018, 34 and the sea serpent 0x20FB at 4557, 2340, 0. One
harness capture (tree `eba1a2c0`, Enhanced ec/ec and KR-like ec/kr, player at 5588, 3018, 30, zoom 1.5; kept outside
the repository, under `uodynamapper-scratch/visual-baselines/labels/ec-statuette`) shows the slime frames whole
and in the same place in both styles (no shift is visible at zoom 1.5): the Slime Frame is one of the 21 whose EC
window holds the figure, and its larger window makes up for its other offsets. The reaper stands under the tree canopy
east of the player and does not show in either capture. The cut statuettes (the dog, the elf, the reaper) were
compared from the images and windows above, not from captures.

## 6. Is the Enhanced Client's behaviour a bug?

Not in the code: the Enhanced Client draws these items whole, from their Classic image. The cut windows are dead data
in its tileart, harmless to it; they only matter to a reader that draws the world art the Enhanced Client never
draws, as Dynamapper's Enhanced style does. Someone who turned `UseNewArt` on for these records (the way KR art is
enabled in the EC) would see them cut: a data defect of the EC's tileart, not a rule to copy.

## 7. Proposed fix (not implemented)

Take the window and draw offset of the art from the client that draws that art. For an item whose EC record lacks
`UseNewArt`, the EC never draws its world art; KR does, with KR's window. So when Kingdom Reborn's record names the
same block-0 texture with the same `world_art_scale` and another enhanced window, the Enhanced packages use KR's
enhanced window. The records with `UseNewArt` keep EC's (the EC draws those; none of them differs from KR's anyway).

- Where: one step that patches the loaded EC sources right after `load_tex_art_ec_sources`, before anything is
  decoded, for example `adopt_kr_world_windows(ec: &mut TexArtEcLoadedSources, kr: &TexArtEcLoadedSources)` in
  `lib/udd-conv/src/textures/kr_items.rs`, replacing `ArtData::ec_texture`'s window and offsets and
  `ArtSpriteDescriptor::world_window` for those ids. Call it wherever the EC art and EC tilemeta are packed from
  loaded EC sources (`convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources*` in `tex_art_ec.rs` and its
  callers, `compute_tex_art_ec_crop_adjustments_from_loaded_sources`, and
  `metadata::tilemeta::build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources`), so that the slot crops, draw
  offsets, `sprite_geometry.json` and tilemeta's `ec_start_x/y` (which follow the trims) agree. The KR records come
  from the KR client the pack already rebuilds from the delta in git (`kr_sections::KrSectionWork`); it must be ready
  before the outer EC art and tilemeta are packed. Without a KR source the step does nothing.
- What it changes: 215 records of 37,756 (36,589 in the static namespace by stored id). Visible: the 211 statuettes
  drawn whole and in KR's place; the desert ostard 0x2135, now blank, drawn; the bird 0x20EE moved onto its perch.
  The blank wisp 0x2100 and death vortex 0x3789 do not show either way. No other record changes: the other 12,799
  shared records already have KR's window, and the 2,163 without a KR record keep theirs.
- Side effect: `compare_world_art` then finds the 211 statuettes identical to EC's (same pixels, window and
  descriptor), and probably the bird and the ostard too (their textures are the same), so the KR section of
  `tex_art_ec` loses 211 to 213 slots and the KR-like style draws the same pixels
  from the outer package. No format change (the descriptor values change, not the layout); `tex_art_ec` and
  `tilemeta` need repacking together.
- Not covered: the 10 EC-only records that borrow another item's world art with a copied window (glow, ship,
  orchid). Nothing authored fits them. Options: leave them, or give them no `tex_art_ec` slot so they draw their
  Classic art (the EC draws their TileArtLegacy image). User's call.

A runtime alternative (Enhanced reading the KR section's slot for these ids) would leave the outer package's wrong
crops in place and tie the Enhanced style to the KR section; the converter fix is smaller and removes the cause.
