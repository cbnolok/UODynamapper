# ghidra-classic-map-image

This folder names the code that draws `facet00.mul`-`facet05.mul`, the Classic client's world map images, in the
2024 client. It also names UO Gold's course gump, which has no such image. It records the format that the uocf
reader follows (`lib/uocf/src/classic/facet_image.rs`).

Each result below is marked **confirmed** (read in the code) or **probable** (inferred from the code and the
shipped files).

## Programs

The programs are in the Ghidra project `UOSA`. They were imported read-only and never executed. They were
auto-analysed, then named with `--apply` and saved.

| Ghidra program | Source | SHA-256 |
|---|---|---|
| `/classic/classic_7.0.103.0_client.exe` | `_Ultima Online Classic/client.exe` (7.0.103.0) | `4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a` |
| `/classic/gold_client.exe` | `Ultima Online Gold/client.exe` (2004-11-11) | `aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2` |

The other clients were searched for the file names only: ML, 7.0.20, `uotd.exe`, UOKR and UOSA. The client
folders are under `/mnt/dati/_proj_local/_uo_clients/`. Do not commit the binaries or anything extracted from
them.

## NameClassicMapImageCode.java

A GhidraScript, category `ECResearch`, built like `ghidra-classic-ui-files/NameClassicUiFileCode.java`.

```
NameClassicMapImageCode.java OUTPUT_FILE [--apply]
run_ghidra_script(program="classic_7.0.103.0_client.exe" or "gold_client.exe",
                  script_name="NameClassicMapImageCode.java", args="<scratch>/names.md --apply")
```

- It names fixed addresses of the two programs above, selected by SHA-256, and refuses any other program.
- It checks every address first. If a check fails, it reports the failure and names nothing.
- Without `--apply`, it only reports.
- With `--apply`, it does three things:
  - it renames DEFAULT functions into `UOClassicMapImage`;
  - it tags them `UO_CLASSIC_MAP_IMAGE`;
  - it sets the plate comment `NameClassicMapImageCode: <tag> <name> [(probable)]: <evidence>`.

The 2026-11-02 run named, tagged and commented 5 functions in 2024 and 2 in Gold
([results/classic_map_image_names.md](results/classic_map_image_names.md)). Every check passed on the first
report. The second `--apply` reported `changes: none` on both programs, and both were saved.

## Findings

### Which clients name the files

| Client | `facetNN.mul` | Course gump background |
|---|---|---|
| UO Gold (2004), ML (2007) | not named, not shipped (ML ships a `facet00.dds` that no client names) | Gold: `multimap.rle` shading (confirmed) |
| 7.0.20 | named, shipped (`facet00`-`facet05`) | the facet image (probable: same strings, not traced) |
| 2024 (7.0.103.0), fp | named, shipped; fp and 2024 differ in `facet00` and `facet01` only | the facet image (confirmed) |
| UOKR, UOSA | `facetN.uop` names only; the 2D clients ship no `facetN.uop` | not traced |

In 2024 the `facet0.uop`..`facet5x.uop` strings sit in a UOP package registration list. That list is probably
shared VFS code; nothing showed that a 2D client opens them.

### The 2024 reader (confirmed)

`UOClassicMapImage::CourseGump_DrawFacetImage` (`0x004c28d0`) works like this:

- It is called from `CourseGump_Redraw` when the gump is dirty.
- It takes file id `0x38 + facet` from the client's file table. File ids 56-61 are `facet00.mul`..`facet05.mul`,
  each memory-mapped whole through `MulFile_RecordPointer` with index 0.
- It starts at byte 4. **The `u16` width and `u16` height are never read.**
- It walks the rows: a `u32` byte count, then runs of a `u8` pixel count and a `u16` colour. It skips whole rows
  up to the tile rectangle's top, then adds up the runs to its left edge.
- It steps through the source rectangle by Bresenham, so the tile sampled for each gump pixel is the nearest one.
- **A colour with bit 15 set is drawn as 0 (black).** Every other colour is drawn ORed with `0x8000`, unless a
  global flag is set.
- Nothing is bounds-checked. If a row's runs add up to less than the width, the draw reads on into the next row's
  byte count.

`CourseGump_Construct` (`0x004c2d10`, "Course gump") keeps these fields:

- the facet, at `+0x500c8`;
- the tile rectangle x1, y1, x2, y2, at `+0x500b0`..`+0x500bc`;
- the gump size, at `+0x500c0`, `+0x500c4`.

It also loads `radarcol.mul`, for the course lines and pins.

`CourseGump_SquareRegion` (`0x004c1e70`) widens the shorter side of the rectangle into a square. It clamps the
result to 0 and to the facet's wrap width and height.

So tile (x, y) is drawn from column x of row y: the image must be the facet's size in tiles.

### UO Gold (confirmed)

`CourseGump_Redraw` (`0x00459be0`) has two cases:

- With map gump `0x139d`, it shades `multimap.rle` into the tile rectangle on a 5120 x 4096 map scale (the
  covered tile count per pixel, through a palette).
- Otherwise it draws the gump.

Gold has no facet image. 7.0.x replaced the `multimap.rle` shading with the colour images.

### The shipped files (probable, from all 18 files of 2024, fp and 7.0.20)

- Every image is the facet's size in tiles: 7168 x 4096, 7168 x 4096, 2304 x 1600, 2560 x 2048, 1448 x 1448
  and 1280 x 4096.
- Every row adds up to the width. No file has bytes after the last row.
- No run has a zero count, and no two adjacent runs could merge (the encoder is greedy, at most 255 pixels a run).
  Even so, the reader keeps the runs as read.
- Only `facet02` (Ilshenar) has colours with bit 15 set: 822,295 pixels in every client, drawn black.

### Do the images match the map and radarcol data? (probable, measured on 2024)

They are close, but they are not exactly a fixed rule. The comparison was a scratch probe, not committed. It
loaded each facet's `mapN`, `staticsN` and `tiledata.mul`, then compared every pixel with three renders:

- **Plain radarcol render.** This is the uocf-inspector Maps recipe: the highest static by (z + height, z) when
  it reaches the land z, else the land tile. It matches only 0-47% of pixels. Other static choices (first, last,
  highest z) do no better.
- **Per-channel shift.** About 95% of drawn pixels are that render with the same shift on all three 5-bit
  channels, clamped to 0..31.
- **Slope rule.** The shift follows the land slope `s = z(x+1, y) - z(x, y+1)`, the slope the EC radar shades
  by: `|s| <= 2` gives -1, `s >= 3` gives +3, and `s <= -3` gives -4 (sometimes -11).

| Facet | Pixels the slope rule gives exactly | Within one 5-bit step |
|---|---|---|
| 0 Felucca | 93.9% | 96.2% |
| 2 Ilshenar | 83.9% | 94.1% |
| 3 Malas | 95.4% | 98.6% |
| 4 Tokuno | 94.4% | 96.7% |
| 5 Ter Mur | 90.4% | 94.4% |

The rest is not in the map data:
- the steep-slope variants;
- about 5% of pixels whose channels do not shift equally;
- Ilshenar's black regions (its 822,295 bit-15 pixels, and plain black where the map has land).

**Verdict:** an image regenerated from an edited map would be a close visual stand-in, not the shipped bytes.

The way to keep the bytes is to re-render only what an edit changes:
- the edited tiles;
- the tiles left of and above each one, since tile (x, y) reads the land z of (x+1, y) and (x, y+1);
- `FacetImageFile::set_pixels` then re-encodes only those rows.

Nothing of this is built.
