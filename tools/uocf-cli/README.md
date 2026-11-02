# uocf-asset-cli

`uocf-asset-cli` contains asset-level tooling built on top of `uocf`.

## Commands

- `export-anim-patch`: export animation payloads as single-entry `.vd` files or
  Michelangelo/UOAnimTool `.uop` patch streams.
- `hues mul-to-uop` / `hues uop-to-mul`: convert Classic `hues.mul` to an Enhanced
  Client `Hues.uop` (EA's layout; `--hue-ramp linear` (default, EA's) or `flat`) and
  back. Packages written by `mul-to-uop` carry `build/hues/huesmul_metadata.bin`, so
  `uop-to-mul` gives back the identical `hues.mul`; EA's packages give exact colours
  with defaulted table start/end, names and group headers.
- `classicanim export` / `classicanim import`: Classic mobile frames of one body and
  action, from an `AnimationFrame*.uop` `animationlegacyframe` entry or the five direction
  entries of an `anim*.idx` / `anim*.mul` pair, as RGBA PNGs plus `classicanimation.kdl`
  (anchors, frame numbers, palettes), and back into copies of the files (absolute output
  paths; MUL: `--output` idx and `--mul-output`, changed entries appended; UOP: a changed
  frames-per-direction needs `--sequence` and `--sequence-output`). Alpha 0 or 255 only.
- `classic-image export` / `classic-image import`: one Classic art (statics: item id +
  0x4000) or gump id as a PNG, from `artidx.mul`/`art.mul`, `artLegacyMUL.uop`,
  `gumpidx.mul`/`gumpart.mul` or `gumpartLegacyMUL.uop` (`--container` when both are
  there), and an image back into copies in `--output-dir`: encoded as the shipped records
  (land 44x44 with an opaque diamond; alpha 0 or 255; 5 bits per channel; opaque black as
  0x0001), a MUL record appended to the data copy, a UOP entry replaced or added;
  `--allow-resize` for another size than the record's.
- `facet convert`: one facet between Classic (`mapN.mul` and/or `mapNLegacyMUL.uop`, with
  `staidxN.mul`/`staticsN.mul`), Enhanced Client and Kingdom Reborn `facetN.uop`, in any
  direction. Classic difs are baked into the map (`--difs`, a later id winning). The target
  size comes from `--target-client` (its executable's size table) or `--size WxH`; a smaller
  size is refused when content lies outside it unless `--crop`, a larger one is filled with
  land 0x244 at z 0 (the shipped maps' void; `--fill-land`/`--fill-z`). `--target-client`
  also picks the Classic container and checks the ids against its `tiledata.mul`. A facet
  package with data the Classic files cannot hold (KR materials and subtypes other than the
  land map's, graphics or hues above 0xFFFF, missing sectors) is refused unless
  `--allow-lossy`; stale delimiters only warn, with their count, since the delimiter rule
  rebuilds them. For a bigger map in an old client, patch its size table with
  `uo-client-patcher-gui` instead.
  With `--to classic` and a `--target-client` that draws the world map images (7.0.x:
  its executable names `facetNN.mul`), the facet's image is written too. The client reads
  it without bounds checks, tile (x, y) at column x of row y, so an image of another size
  than the map draws the wrong tiles. `--facet-image`:
  - `update` (default): the target's image when the map is the target's, byte for byte;
    the tiles that differ from the target's map re-rendered by the slope rule otherwise;
    the whole image rendered when the sizes differ or the target has none;
  - `keep`: the target's image as it is;
  - `regenerate`: the whole image rendered;
  - `none`: no image.

  `keep` and `none` are refused when the target's image is not the map's size. A whole
  rendered image is an approximation: about 94 % of pixels match the shipped style. The
  rule takes the radarcol colour of the highest static (else the land) and shifts every
  channel by the land slope. Bit-15 (black) pixels of unchanged tiles are kept. A
  variant edition leaves the image alone unless `regenerate`: the shipped images follow
  the standard edition.
- `facet patch-classic`: edit one cell of one Classic edition (land, z, statics added after
  the cell's, removed or changed) and write it in the same container to `-o`. When the
  folder has the facet's `facetNN.mul` (with `radarcol.mul` and `tiledata.mul`), the
  edited tile is re-rendered into a copy in `-o`, as are the tiles left of and above it
  when its land z changed. Only those rows change; the pixels touched are reported.
- `mul-convert extract|pack`: maps go through the typed map writer (4,096 blocks per UOP
  entry and the trailing block, as shipped); `pack --uop-template-dir` keeps the layout of
  the shipped packages, so an unchanged map packs back byte for byte.

Supported animation sources:

- `classic-mul`: reads raw blocks from `anim*.idx` / `anim*.mul`.
- `cc-animation-frame`: reads Classic Client `AnimationFrame*.uop` entries.
- `ec-animation-frame`: reads Enhanced Client `AnimationFrame*.uop` entries.

Example:

```sh
uocf-asset-cli export-anim-patch \
  --source classic-mul \
  --idx anim.idx \
  --mul anim.mul \
  --block 7 \
  --format vd \
  --output body_7.vd
```

Use `--format michelangelo-uop` for the old Michelangelo/UOAnimTool patch
stream. This is not a modern Mythic UOP package.
