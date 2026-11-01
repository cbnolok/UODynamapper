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
- `facet convert`: one facet between Classic (`mapN.mul` and/or `mapNLegacyMUL.uop`, with
  `staidxN.mul`/`staticsN.mul`), Enhanced Client and Kingdom Reborn `facetN.uop`, in any
  direction. Classic difs are baked into the map (`--difs`, a later id winning). The target
  size comes from `--target-client` (its executable's size table) or `--size WxH`; a smaller
  size is refused when content lies outside it unless `--crop`, a larger one is filled with
  land 0x244 at z 0 (the shipped maps' void; `--fill-land`/`--fill-z`). `--target-client`
  also picks the Classic container and checks the ids against its `tiledata.mul`. A facet
  package with data the Classic files cannot hold (stale delimiters, KR materials) is refused
  unless `--allow-lossy`. For a bigger map in an old client, patch its size table with
  `uo-client-patcher-gui` instead.
- `facet patch-classic`: edit one cell of one Classic edition (land, z, statics added after
  the cell's, removed or changed) and write it in the same container to `-o`.
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
