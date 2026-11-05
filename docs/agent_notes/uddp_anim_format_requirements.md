# UDDP mobile animation format: needs, constraints, desires, failed attempts

Status (2026-11-05): **measuring is finished, and the user has made the first format decisions (section 8). Nothing is
implemented yet, and the lossy rows are still open: fractional zoom and upscaled sprites are being measured first.** This file says what the format has to do and what we
already tried, so nobody repeats a dead end. Section 9 lists every option set aside, and why. The measurements and their method are in
[`proposals/2026-11-01-mobile-anim-codec.md`](proposals/2026-11-01-mobile-anim-codec.md), which ends with the recommendation
and the list of decisions, and the benchmark is
`lib/udd-conv/tests/integration/mobile_anim_codec.rs` with `mobile_anim_codec/shrink*`.

Reading the numbers:

- "× BC7" compares with the benchmark's BC7 + zstd estimate: 782.5 MB Classic, 1823.3 MB Enhanced, per body with no
  atlas or gutters. It flatters BC7 slightly, because real packs are larger:

  | | Benchmark estimate | Pack from the packer of the time (`--bc7 --zstd`, not installed) | Installed in early November, since removed |
  |---|---:|---:|---:|
  | Classic | 782.5 MB | 856.6 MB | 985.6 MB (30 Jun) |
  | Enhanced | 1823.3 MB | 1987.9 MB | 6904 MB (30 Oct) |

  That Enhanced package predated compression: 2291 of its 2522 entries were stored raw, and 73 % of its atlas area holds no
  painted pixel. The packer's own waste is 26.5 %; the rest is the empty space inside padded frame boxes.
- Timings are one thread, boost off, EPP power, unless marked otherwise. Numbers marked *provisional* come from a run
  whose CPU state changed before it ended.

## 1. Needs

Stated by the user, most important first.

| # | Need | Note |
|---|---|---|
| 1 | **Frame decoding comes before size.** The renderer must handle hundreds of different NPCs and actions on one screen. | "size is very important but even more important is frame decoding" |
| 2 | **No stutter when new mobs appear.** | The official Enhanced Client's defect: a scene with new mobs and animations stutters a little. |
| 3 | **Size is very important.** Upscaled art makes it matter more. | Monster animations are huge, and full-frame atlases are wasteful. |
| 4 | **Colour fidelity.** | Stated for the web map ("my concern is color fidelity"); we apply it here too. Official frames stay lossless unless the user picks a lossy tier after seeing it. |
| 5 | **32-bit colour for custom animations.** | The restricted palette format is only for official art, so a package must be able to say which kind a batch holds. |
| 6 | **Upscaled sprites**, using the filters we already have, chosen at conversion or inspection time. | The user prefers upscaling before conversion, since that works with more filters. |
| 7 | **Dynamapper reads only UDDP, never UOP.** | Any animation data that lives in other client files, such as `tileart.uop`, must be carried into the package. |
| 8 | **Custom animations added to the MUL files must survive conversion.** | Stated 2026-11-02. See "Custom animations in MUL files" below. |
| 9 | **A custom or override table for bodies and worn items.** | New wearers such as dwarves, and custom frames for existing items on them (lower, smaller), are planned. The placeholder rule alone can't say those. Stated 2026-11-05. |

Also needed, derived from the above and from how the art works:

- Hue applied in the shader, which needs the palette and each entry's keep-colour flag to survive to the GPU.
- Enhanced's 16 coverage levels preserved, including the client's edge alpha.
- Every frame's anchor and size, and an existence index per (body, action, direction), so asking whether a frame
  exists never needs a decode.
- Frame numbers kept stable: `AnimationSequence` frame effects and skipped frames refer to them.
- **Classic equipment needs the wearer's source and the tile's appearance.** The client tests the UOP flag on the object
  drawn, not on the id it requests, and a flagged wearer's layer item takes its UOP body from the tile's `tileart.uop`
  appearance (a gargoyle in tile 42002 draws UOP body 1525, an unflagged id, with no MUL fallback). So the Classic package
  must carry that tile-to-appearance data, as the Enhanced one does, plus `IsMountBody`'s list. Its reader's `animation()`
  follows the rule for a mobile's own body only. See
  [`uop_reachability.md`](../../scripts/dev-helpers/ec-research/ghidra-classic-frames/results/uop_reachability.md).

## 2. Constraints

### The data

| | Classic | Enhanced |
|---|---:|---:|
| Bodies, frames | 1022, 996,620 | 926, 1,572,711 |
| Trimmed pixels | 2419 Mpx | 4917 Mpx |
| Shipped size | 1235.0 MB (`AnimationFrame*.uop`, `anim*.mul`, `animdata.mul`, `animinfo.mul`), of which 1060.3 MB is referenced | 1984.5 MB (`AnimationFrame*.uop`) |
| Pixel | one index | index + coverage (0–16 sixteenths); 19 % of pixels partly covered |
| Bits per opaque pixel, as measured | 5.66 | 6.89 |
| Stored VRAM, whole set | R8 2.42 GB (BC7 2.52 GB) | RG8 9.83 GB (BC7 5.11 GB) |

- **174.7 MB of the shipped MUL files is unreferenced:** the `anim*.idx` files reference only 521.0 MB of the 695.7 MB
  of `anim*.mul`. So comparisons with shipped Classic use the 1060.3 MB. Also, `animdata.mul` (4.49 MB) is static-tile
  animation data, and the `anim*.idx` files (6.09 MB) aren't in the total. What those bytes hold is in
  "Orphaned art" below.
- **Palettes change far less often than frames.** Files store one with each frame, 256 entries or fewer, but on every
  10th body there are 1.00 palettes per action in Enhanced and 3.41 in Classic (1.63 distinct by content).
  Across a body they do differ: a median of 3784 distinct colours, and only 5.7 % of bodies fit one 256-entry palette.
- **Enhanced can't be 8 bits per pixel without loss:** (index, coverage) pairs fit 255 entries in under 1 % of bodies.
- **Classic index 0 is free in shipped art only:** no shipped pixel uses it, so we can use it for transparent there. A
  custom animation may use it, so the format can't assume this for every MUL entry.
- **Frames are small:** about 3100 trimmed pixels on average for Enhanced, so an action averages about 154 kpx. A few
  bodies are enormous: Enhanced body 1288 has 1600 frames and 139.6 Mpx (279 MB as RG8).
- **Frame counts:** Enhanced and Classic `AnimationFrame*.uop` use a median of 10 frames per direction. Classic
  `anim.mul` uses a median of 5 (Enhanced has 1.70× as many frames on shared body and action pairs). 96 % of Enhanced actions play at
  9 fps.
- **Five directions are stored** and the other three are mirrored.
- **Frames are redrawn, not shifted.** See the failed attempts.

### Custom animations in MUL files

Shard and mod makers add or replace animations with MUL tools. The converter must keep what they added.

What the converter does today (`lib/udd-conv/src/mobile_anim_cc.rs`):

- It walks every idx slot of `anim.mul` to `anim6.mul` with data, whether or not a def file reaches it, so an added
  entry is collected.
- It reads `Body.def`, `Bodyconv.def` and `mobtypes.txt` from the install being converted, so edited routing is kept.

- An entry that can't be read gets a warning naming its file, slot, body, action, direction and reason. The total is in
  the build summary and the CLI output. Reading walks the runs at planning, so a bad entry can't abort the pixel stage.
- A MUL frame grows to hold every run. The client places runs from the anchor and clips only to the screen, never
  reading the frame's size (`DrawFrameRle`, 7.0.103.0). Each pixel stays at its offset from the anchor. No shipped frame
  needs this: 0 of 526,895 MUL frames and 0 of 470,415 UOP frames have a run outside their size.
- When a package holds a MUL and a UOP entry for one action, the reader takes the one the client would use: UOP when
  the `mobtypes.txt` row has flag 0x10000, otherwise MUL. The other source only fills in when that one has none.
- `lib/udd-conv/tests/integration/mobile_anim_custom_mul.rs` covers an added body, a replaced body with its old bytes
  orphaned, an idx longer than the shipped one, palette index 0 drawn as opaque black, a 1500x900 frame, runs outside the
  declared size, trimming, and unreadable entries.

What is still not safe:

- **Custom UOP frames:** `AnimationFrame*.uop` frames are still clipped to their declared size, and a frame wider or
  taller than 1024 is blanked silently.
- **Very large frames:** above 2048 in either dimension an entry is refused, with a warning. Frames near that limit may
  not fit the 2048 atlas page with its gutter; this is untested.
- **Leaving out art is unsafe by default,** because a shard can draw any body it routes. See section 7.

### Orphaned art in the shipped MUL files

Measured 2026-11-02 (`uodynamapper-scratch/mul-gaps/scan.py`, read-only). No idx entry points at these bytes, and the
client reads frames only through the idx, so it shouldn't reach them. All 174.66 MB parses as complete animation
entries with no bytes left over: 69,289 entries and 414,461 frames, mostly in blocks of 175 entries (35 actions × 5
directions).

| Compared with referenced entries | Size |
|---|---:|
| Byte-identical to a referenced entry | 12.61 MB |
| Different frames, same palette as a referenced entry (likely an older version of that body) | 19.92 MB |
| No match | 142.13 MB |

The largest blocks:

| File, offset | Size | Entries, frames | Contents |
|---|---:|---:|---|
| `anim2.mul` 0x8B678CF | 57.8 MB | 44,625, 267,282 | Very small sprites, mostly grey |
| `anim5.mul` 0x29F45B8 | 33.0 MB | 200, 880 | Large, detailed red and purple dragons |
| `anim4.mul` 0x3D402CB | 24.4 MB | 3500, 21,000 | Robed figures; about half identical to bodies 505–515 |
| `anim3.mul` 0x2C83146 | 10.6 MB | 2975, 17,850 | Armour pieces and robes; many share palettes with bodies 455 and 458 |
| `anim3.mul` 0x1D1EF47 | 8.9 MB | 2800, 16,799 | Same kind of art; many share palettes with body 468 |

- These have no body id: the idx was the only record of it. Only a shared palette or neighbouring data hints at one.
- We can't tell from this why they're orphaned. Art left behind when patches rewrote bodies is a guess. The MUL
  tools that add custom animations can leave the same kind of gap.
- They haven't been compared with the `AnimationFrame*.uop` art, so "no match" doesn't mean the art exists nowhere else.

The converter reads through the idx, so none of this reaches a package. That's correct, and it makes the 0.64× shipped
figure the fair one.

### Decoding and loading

- A frame is decoded once per load, then drawn from the atlas. Drawing a crowd costs GPU time; decoding only happens
  the first time an action or body appears.
- Current zstd 19 costs (Classic / Enhanced): 769 / 1021 MB/s on one thread, boost off, EPP `balance_power`. The largest
  whole body takes 204 / 404 ms, the largest action at most 21 ms, and the largest (action, direction) 4.5 ms (6.1 ms for
  `Spider_Female`, whose largest action has a single direction).
- Upload to the GPU is a separate cost, and those numbers leave it out.
- Decoding belongs on worker threads, so a slow job delays a mobile's first appearance rather than the frame rate.

### Project rules

- No backward compatibility: our packages have not shipped. Format layout is ours to choose, and it doesn't need to
  look like the client's. Only the values we need have to survive.
- No magic numbers, named constants only. Don't change the format without the user's confirmation.
- Never execute client binaries or write into client folders. Client behaviour comes from static analysis.
- The test machine has 27 GB of RAM. Benchmarks run one at a time with an 18 GB cap.

### Minimum GPU and platforms

The user's requirements (2026-11-04):

- Desktop cards: the GTX 900 and 10 series and AMD Polaris are the bar. Vulkan versions per GPU aren't checked yet.
- Integrated GPUs count, and must hold at least 60 fps. The test machine's Radeon 890M is well above the low end, so it can
  compare layouts but can't prove this.
- The ideal reach is web, mobile, Apple, Windows and Linux.

No block format reaches all of these. Vulkan, WebGPU and Metal each expose BC1 to BC7 as one feature, so BC1 reaches no
device that BC7 misses. Support reported by browsers to web3dsurvey, read 2026-11-04 (the page shows no data date, so treat
these as indicative):

| Platform | BC1 to BC7 | ETC2 | ASTC |
|---|---:|---:|---:|
| Windows | 100 % | 0.02 % | 0.02 % |
| macOS | 88.33 % | 95.82 % | 95.82 % |
| Linux | 78.7 % | 71.87 % | 64.71 % |
| Chrome OS | 81.72 % | 99.78 % | 98.34 % |
| iOS | 57.28 % | 99.92 % | 99.92 % |
| Android | 34.87 % | 99.94 % | 99.94 % |

By vendor, BC is 100 % on AMD, Intel and NVIDIA, 79.6 % on Apple, 73.43 % on Qualcomm and 0 % on ARM, Samsung, Huawei and
Imagination. ASTC is 0 % on AMD and NVIDIA and 19.66 % on Intel.

R8 and RG8 planes need no block format, so a lossless indexed layout is the one path that runs everywhere. A low-end pack
needs a format per platform, at the same VRAM:

| Bytes per pixel | Desktop | Phones and Apple |
|---:|---|---|
| 0.5 | BC1 | ETC2 RGB8A1 |
| 1 | BC7 | ASTC 4×4 |

At the same size these formats aren't smaller, and their quality gain is modest. Arm's published figures, on general images:
ASTC 8 bits per pixel is comparable to BC7, and ASTC 6×6 (3.56 bits per pixel) beats BC1 at 4 by about 1.5 dB and ETC2 by
about 0.7 dB. The 2007 ETC2 paper puts ETC2 about 0.8 dB above S3TC on opaque images. ASTC's real gain for us is that 6×6 keeps
full alpha below BC1's size, which may keep Enhanced's anti-aliased edges. None of this has been measured on our sprites, and
big blocks round tiny frames up.

Block sizes round tiny frames up, so those nominal rates hold only on large frames. Bytes per frame, with no atlas or gutter:

| Frame | R8 | BC1 | ASTC 6×6 | ASTC 8×8 |
|---|---:|---:|---:|---:|
| 20×23, typical equipment | 460 | 240 | 256 | 144 |
| 85×94, typical monster | 7,990 | 4,224 | 3,840 | 2,112 |

Choosing a codec per client or per size class gains nothing for lossless disk. On Classic the same coder wins every class that
stays inside the crowd line (702.5 MB either way). On Enhanced the best mix is 1767.4 MB against 1810.8 MB for one coder, or 2.4 %,
and it takes 232 ms of the 250 ms line. The clients do differ in layout: Classic has to store its shape, and Enhanced's is
free in coverage.

Size may matter for the low-end tier. Bodies above a median cropped frame of 10,000 px are 5.2 % of Classic's streams and 62.7 %
of its pixels, and 6.3 % and 59.4 % on Enhanced. Block formats hurt tiny frames most, so block-coding only those bodies could keep
equipment exact. Pixels × format, whole set, before padding:

| VRAM | Classic | Enhanced |
|---|---:|---:|
| Lossless everywhere | 2.42 GB | 9.83 GB |
| Large bodies BC7, the rest lossless | no gain | 6.91 GB |
| Large bodies BC1, the rest lossless | 1.66 GB | 5.46 GB |
| Large bodies ASTC 8×8, the rest lossless (phones and Apple) | 1.28 GB | 4.73 GB |
| Large bodies BC1, the rest BC7 | not useful | 3.46 GB |

Basis Universal is the one single-file route: it transcodes one file to BC, ETC2 or ASTC at load. It adds a transcode step
on top of zstd, and its small mode (ETC1S) is described by its authors as low to medium quality. It hasn't been measured on
our data.

The renderer isn't set up for most of this. The web and Android features are commented out in `dynamapper/Cargo.toml`, and
the workspace's direct `wgpu` dependency names only Vulkan. WebGL2 has no storage buffers, which the renderer already uses,
so the web path would need WebGPU.

### Shader paths and filtering

The user's concern (2026-11-04): a per-sprite branch in one shader can reserve registers for the path not taken and slow the
other path. Vendor limits show how: several Mali Valhall GPUs halve their threads above 32 work registers, and Intel drops from
SIMD16 to SIMD8 at 64 registers. So each pixel format should get its own pipeline, compiled at load and only for the formats
the pack holds. Bevy already compiles at runtime, and `shader_defs` remove the other path before compiling (`gpu_picking.rs`).
A mixed pack pays a pipeline switch wherever draw order forces a change of format. Compiling at load also avoids the mid-game
stutter the user dislikes in the original client.

Palette indices can't use hardware filtering, which the renderer relies on: `art/main.wesl` samples colour once through the
hardware sampler after `pixel_art_atlas_uv_iq`. Fetches per pixel with that filter on:

| Layout | Fetches |
|---|---:|
| BC1, BC7, ASTC | 1 |
| Classic R8 | 5 (1 gather, 4 palette reads) |
| Enhanced RG8 | 6 (2 gathers, 4 palette reads) |

Palette reads come from a small table, so fetch counts don't predict time. This hasn't been measured, and it could count against
indexed frames as the default on integrated GPUs.

### What the original clients ship

Enhanced and Kingdom Reborn frames aren't block-compressed on disk: they are AMOU palettes and RLE. Their textures are DDS,
mostly DXT5 rather than DXT1. DXT5 (BC3) costs the same VRAM as BC7. Which format they use in VRAM after decoding a frame
hasn't been traced. Share of each package's decompressed DDS bytes:

| Package | Enhanced DXT5 / DXT1 | Kingdom Reborn DXT5 / DXT1 |
|---|---:|---:|
| `LegacyTexture.uop` | 100.0 % / 0.0 % | 100.0 % / 0.0 % |
| `Texture.uop` | 85.4 % / 14.6 % | 96.2 % / 3.8 % |
| `TerrainTexture.uop` | 95.3 % / 4.7 % | 89.8 % / 10.2 % |

### Frame and stream sizes

Most frames are tiny, but most pixels are in the few large ones. So tiny frames set the number of streams and what each
stream's overhead costs, while large frames set the disk total and the worst crowd. Measured 2026-11-04 from the shipped
files, using each frame's declared box. Stream counts assume 5 directions per entry in the UOP files.

| | Classic MUL | Classic UOP | Enhanced |
|---|---:|---:|---:|
| Frames | 526,895 | 469,725 | 1,572,711 |
| Median width × height | 18 × 21 | 73 × 20 | 24 × 27 |
| Median frame area, px | 391 | 1406 | 680 |
| Frames that fit 128 × 64 | 88.9 % | 85.0 % | 81.3 % |
| Pixels in those frames | 26.9 % | 31.2 % | 19.1 % |
| Streams per (action, direction) | 92,108 | 39,615 | 152,529 |
| Median frames per stream | 5 | 10 | 10 |
| Median stream area, px | 2336 | 17,408 | 7563 |
| Streams up to 4096 px | 66.4 % | 7.7 % | 33.5 % |
| Pixels in those streams | 9.4 % | 0.4 % | 2.0 % |

**Those medians describe equipment, not creatures.** Equipment is about 80 % of all frames, and it pulls every median down.
Split by the body's `mobtypes.txt` type (Enhanced ships no such file, so its bodies use Classic's table by id). Pixel shares
in this table count declared boxes. Counted by cropped boxes, Classic UOP is monster 52.0 %, animal 33.0 % and equipment
12.4 %, which is the basis of the rule's table further down. A later `mobtypes.txt` split of Classic MUL that also counts
bodies the table doesn't list gives equipment 75.0 % / 19.5 % and not listed 9.4 % / 3.5 %; the rows below leave those out:

| Kind | Share of frames | Share of pixels | Median frame | Median area, px | Fit 128 × 64 |
|---|---:|---:|---:|---:|---:|
| Enhanced monster | 11.0 % | 52.1 % | 85 × 94 | 7857 | 18.1 % |
| Enhanced animal | 4.9 % | 23.2 % | 91 × 102 | 8439 | 19.7 % |
| Enhanced human | 1.3 % | 2.1 % | 44 × 70 | 2925 | 27.5 % |
| Enhanced equipment | 81.7 % | 21.3 % | 20 × 23 | 484 | 94.4 % |
| Classic MUL monster | 10.2 % | 67.8 % | 80 × 86 | 6767 | 22.2 % |
| Classic MUL animal | 3.3 % | 6.0 % | 41 × 48 | 2044 | 66.0 % |
| Classic MUL human | 1.8 % | 1.7 % | 30 × 57 | 1710 | 95.9 % |
| Classic MUL equipment | 77.2 % | 20.4 % | 16 × 19 | 312 | 98.4 % |
| Classic UOP monster | 9.2 % | 42.1 % | 101 × 78 | 7743 | 38.7 % |
| Classic UOP animal | 9.0 % | 27.8 % | 109 × 82 | 8549 | 29.9 % |
| Classic UOP equipment | 80.0 % | 27.2 % | 70 × 16 declared, 15 × 16 cropped | 1110 declared | 97.9 % |

**Telling equipment apart from a client's own data.** `mobtypes.txt` is Classic's table, and the Classic converter already
reads it. Enhanced doesn't ship it, but `tileart.uop` can say the same. A body's creature placeholder (tile
`10000000 + body`) has a body type, which is the action map it plays:

- It plays its own map: a creature.
- It plays a wearer's map (400, 666 or 1253): a human or elf or gargoyle if the placeholder sets race (key 9), else equipment.

**This is the rule to use: the user's call on 2026-11-04, as the strongest and most consistent of the three.** A wearer's map
isn't hard-coded: it is the body type of any placeholder that sets race. That comes out as 400, 666 and 1253, and only body
1253 plays 1253, so it changes no body's class. Bodies with no placeholder take `mobtypes.txt`. Both clients' `tileart.uop`
hold the same counts: 1143 placeholders, 13 setting race, 496 without race on 400 and 170 on 666.

| Source (pixels are cropped boxes) | Equipment: bodies, frames, pixels | NPC: bodies, frames, pixels | No class | No placeholder |
|---|---|---|---|---|
| Enhanced, 926 bodies, 4917 Mpx | 558, 82.86 %, 21.55 % | 368, 17.14 %, 78.45 % | 0 | 344 and 827 |
| Classic UOP, 276 bodies, 1473 Mpx | 154, 80.08 %, 12.36 % | 122, 19.92 %, 87.64 % | 0 | 344 |
| Classic MUL, 746 bodies, 946 Mpx | 396, 76.69 %, 21.43 % | 247, 15.27 %, 76.25 % | 103 bodies, 8.04 %, 2.32 % | 23 bodies, 16.36 Mpx |

The 103 are `anim2+.mul` bodies with no `Bodyconv.def` line, so they have no body id to look up.

Against `mobtypes.txt`, Classic UOP has no disagreement. The largest in the others:

- Equipment by the rule: 987 (`HUMAN`, a GM robe's anim id), 260 (`MONSTER`, a bone armor's appearance), 994 and 991 (`HUMAN`),
  and without a line 970, 641 and 573.
- NPC by the rule: 841 and 840 (`EQUIPMENT`), and without a line 105.
- Inside NPCs: 990 is `HUMAN` and plays its own map with no race, though a Wearable tile points at it. It is 1.1 Mpx in
  Enhanced and is the one result here that looks doubtful.

**The `Wearable` flag alone isn't enough.** Wearable tiles and their appearance or anim id agree with `mobtypes.txt` on 93–97 %
of pixels, but point at some real creatures: 10 monster-sized bodies such as 735 (272 × 238, its tile is a "shirt") and the
gargoyle bodies 666 and 667 (a "shortsword" and a "war mace"). 735 plays its own action map, and 666 and 667 are wearers
themselves. So those tile anim ids look like old values that collide with reused body ids, which is an inference. In Classic
`Equipconv.def` also has to be followed: it reaches 21 of the 42 `EQUIPMENT` MUL bodies no tile points at. The other 21 are
unexplained.

**Both clients already hold per-wearer rows, with no fixed list of races.** Enhanced: `tileart.uop`'s appearance filters, on 346 of 1126 wearable tiles.
A row is (owner placeholder, appearance), and `AppearanceFilterByOwnerBodyType::Apply` takes the first match without testing race. Classic: `Equipconv.def`,
104 rows of (body, equipment id, converted id, gump, hue). The wear check compares action maps only (`BuildLayers`, "Not letting a %u(sq %u) wear a %u(sq %u)").
For draw order, race is one of four keys in a rule table where -1 is a wildcard (`FUN_00468e30`, read from the decompile; where that table loads from isn't
traced). Only `lib/uocf` reads these rows today: no package carries item-to-wearer data.

**Empty space inside the frames.** Whole-frame cropping is already in the converters and in every bench figure, and it has
nothing more to give. What remains is inside the cropped box: only 31–44 % of it is painted, and about half lies outside
each row's first and last painted pixel. Sampled on every 10th entry; "others" is everything `mobtypes.txt` doesn't call
equipment. 80 of the sampled Classic UOP frames have no bytes after the header, so they count as empty and are left out.

| | Cropped ÷ declared | Row spans ÷ cropped | Painted ÷ cropped | Frames with an empty row inside | Median cropped frame |
|---|---:|---:|---:|---:|---:|
| Enhanced equipment | 1.000 | 0.454 | 0.389 | 15.7 % | 20 × 23 |
| Enhanced others | 1.000 | 0.475 | 0.366 | 2.5 % | 77 × 89 |
| Classic MUL equipment | 1.000 | 0.503 | 0.442 | 13.8 % | 16 × 19 |
| Classic MUL others | 1.000 | 0.495 | 0.371 | 4.8 % | 38 × 59 |
| Classic UOP equipment | 0.337 | 0.397 | 0.309 | 23.8 % | 15 × 16 |
| Classic UOP others | 0.883 | 0.519 | 0.410 | 1.8 % | 77 × 80 |

Sparse frames take a large share of the rectangles. Among Enhanced bodies worn by a tile, sampled the same way, frames under
20 % painted are 12.7 % of frames and 7.1 % of painted pixels but 24.6 % of box area. The sparsest large bodies, 1055 to 1058,
are 15–19 % painted and 30–35 % row spans, so their rows hold separate islands that row spans can't remove.

A coder fed the rectangle spends bytes and decode time on those row transitions, but removing them saves far less than these
shares suggest. On every body, against the rectangle at zstd 19: row spans −3.6 % on Classic and −2.9 % on Enhanced, both
Enhanced planes in row spans −6.6 %, painted pixels only −8.4 %, and painted pixels with coverage in row spans −9.6 %. The
same rows on every 10th body, with the sparse-frame shares, which move with the sample:

- **Row spans:** 3.5 % smaller on Classic, 2.9 % on Enhanced.
- **Both Enhanced planes in row spans:** 6.7 % smaller.
- **Painted pixels only:** 2.2 % smaller on Classic, 8.6 % on Enhanced. Classic has to store the shape this removes, while
  Enhanced's coverage plane already holds it.
- **Frames under 20 % painted:** only 2.5–4 % of the bytes.
- **Splitting a frame's box at empty rows and columns:** no change in bytes. It saves 3–4 % of the atlas overall: 12–19 % of
  equipment's and nothing of creatures'.

Section 9 of [the codec proposal](proposals/2026-11-01-mobile-anim-codec.md) has the tables.

The same bias is in the crowd numbers: the "uniform" crowd draws every (body, action) pair equally, so it is mostly
equipment. A crowd of creatures is about 4× its pixels. See section 6.

## 3. Desires

Wanted but not required, in the order the user raised them.

- **Use the repetition between neighbouring frames.** The obvious redundancy should pay off somewhere.
- **Use what we know about the data**, rather than general-purpose compression alone, as the original clients did with
  restricted colour.
- **Smooth animation at high frame rates.** The world scrolls smoothly while the art changes at about 9 fps. Treat the
  stored frames as key frames and interpolate the movement between them, probably at runtime.
- **Fewer stored frames if interpolation can rebuild them** (the halving idea below).
- **Runtime BC7** as a later option, possibly for upscaled animations, to save VRAM. Off for now by the user's decision.
- **Do clearly better than the 20-year-old clients.** On every body, Classic does: 0.67× of the referenced shipped data.
  Enhanced is only slightly better: 0.94× at the best crowd speed, 0.91× when disk leads, and it pays with RG8's VRAM.

## 4. Failed attempts

Each of these was measured, so repeat one only with a new reason. All are lossless unless marked. Failure is relative
to the needs: some of these may still be useful elsewhere.

### Using the repetition

| Attempt | Result | Why it failed |
|---|---|---|
| 8×8 block copy from the previous frame | 0.5 % larger | Exact shifts are rare: 3 % of blocks (Classic), under 1 % (Enhanced). |
| Pixel difference from the previous frame | 14–55 % larger | Limbs move, so differences are noisy. Palettes aren't the cause: Enhanced has one per action and still gets larger. |
| Exact reference frames, including mirrors | Only 17–21 % of pixels match under the best reference | Not enough to pay for signalling it. |
| Stand action as the reference for other actions | About −1.5 % against per-(action, direction), no better than plain per-action | Poses differ too much. |
| Frame order by direction | Classic −0.8 %, Enhanced +1.1 % | |
| Previous frame's colour as the rank reference | Worse than predicting from neighbours | Same cause as the difference attempt. |

### Between bodies

| Attempt | Result |
|---|---|
| Solid zstd with `--long=31` over everything | ×0.990 Classic, ×0.995 Enhanced (the upper bound on this idea) |
| Trained zstd dictionaries, large bodies as a reference | ×1.000–1.025 |
| Exact duplicate frames; recoloured copies are also counted | About 24 MB Classic, 12 MB Enhanced, none across files |
| Equipment layers sharing the wearer's shape | ×0.99 or no gain |
| Same (body, action, direction) in two `AnimationFrame*.uop` files | 0.0 MB |

### Stronger general-purpose and image codecs

Compared with zstd 19 on 25 bodies, index only. Their speeds are from a contended machine.

| Codec | Size | Decode | Verdict |
|---|---:|---:|---|
| zstd 22 | at most 1 % smaller | | Not worth it |
| zstd long-distance matching | under 1 % | | Not worth it |
| xz | 0.94–0.97× | about 58 MB/s | Too slow for the gain |
| brotli | 0.94–0.96× | about 130 MB/s | Too slow for the gain |
| WebP lossless strips | 0.97–1.06× | about 100 MB/s | No gain |
| JPEG XL lossless | 1.00–1.50× | 5–11 MB/s | No gain, far too slow |

### Palette and colour tricks

| Attempt | Result |
|---|---|
| Sorting the palette | Helps only with residuals plus an entropy coder: about −11 % of the Enhanced index plane. Classic gets larger. |
| One palette per body or one per Enhanced (index, coverage) | Doesn't fit 256 entries: 5.7 % and under 1 % of bodies. |
| Rank coding, left pixel as reference | Classic worse, Enhanced −4 % |
| Rank coding with previous-frame context, the smallest lossless result (Enhanced 0.878× BC7, 0.81× shipped) | **Too slow:** 14–15 MB/s Classic, 40–44 MB/s Enhanced against 769 / 1021 for zstd 19. It takes 580 / 1010 ms on the uniform crowd, against the 250 ms line. |
| Rank coding from a predicted colour, with a coarse lookup | Enhanced sample only 2.8 % over the smallest (Classic 3.5 %), and the fastest rank variant, but still only 20 / 50 MB/s. It doesn't rescue rank coding. |
| Coverage predicted from edge distance | ×0.956 of coverage, about 11 MB (estimate) |

### Lossy, measured but not chosen

SSIMULACRA2 is median / minimum unless stated.

| Attempt | Saving | Quality |
|---|---|---|
| Near-lossless index, Enhanced, threshold 4 | −6 % of index | 94.8 / 88.6 |
| Near-lossless index, Enhanced, threshold 8 | −18 % | 91.1 / 80.7 |
| Near-lossless index, Classic, threshold 8 | −10 % | 88.4 / 72.4 |
| Near-lossless index, Classic, threshold 16 | about −26 % | 81.8 / 57.6 |
| Coverage reduced to 8 levels | −22 % of coverage | 94 / 33: damages at least one body |
| Coverage reduced to 4 levels | about −45 % | 88 / −26 |

**Halving the frames and interpolating them back (lossy; every 10th body):**

- It doesn't halve. Keeping first, last and frame-effect frames leaves about 61 %, so the size is ×0.59 (Enhanced) and
  ×0.57 (Classic UOP). Choosing frames per action was no better than dropping them evenly.
- A halved set really plays at about 5 fps.
- Rebuilding the dropped Enhanced frames:

| Rebuild | SSIMULACRA2 median | p5 | CPU per action, median / max |
|---|---:|---:|---|
| Hold previous | 50.3 | −57.8 | none |
| Crossfade (off palette) | 52.0 | −50.5 | 0.5 / 91 ms |
| Block warp, computed at load | 40.7 | −56.1 | 12.4 / 2130 ms |
| Block warp, stored vectors | 80.6 | 29.1 | 0.1 / 9.4 ms |

Stored vectors are the only workable rebuild, and they tear on limbs and tails. They score 39.6 on large frames and 57
on die actions. Idle holds up, scoring 91 even with a plain hold. The action labels are still assumptions. The user
decided on 2026-11-04 to keep all frames.

### Load and GPU ideas

| Attempt | Result |
|---|---|
| Runtime BC7 at load | 22–23 Mpx/s, 19–43× the decode time. Too slow for first draw; off by the user's decision. |
| GPU BC7 encoder | Not built: Classic gains nothing, and only Enhanced VRAM would benefit. |
| Whole-body batches for crowds (the original choice) | Rough estimate from measured times for 300 distinct items, one thread, before upload, until the crowd scenario replaces it: 1.5 s at the median and 12 s at the p95, holding about 1.3 GB of VRAM, against 30 ms, 0.8 s and about 90 MB per action. |
| Upscaled frames as lossless RGBA8 | At best 1.34× / 1.60× BC7 at 2× |

## 5. What still looks right

Nothing here is final.

- **zstd 19 on indexed planes.** Classic whole body is 677 MB: 0.79× of a real BC7 pack and 0.64× of the referenced
  shipped data. Enhanced is 1951 MB: 0.98× of a real BC7 pack and 0.98× shipped, so it needs help.
- **Per-action streams, grouped per body.** Classic is 749 MB, 0.87× of a real BC7 pack. Enhanced is about 1970 MB
  (an estimate from the sample), about 0.99× of one. That is what we pay for crowds, and it's still no larger than BC7.
- **Hue in the shader**, on R8 and RG8 atlas pages.
- **Upscaling:** store 1× and a filter id and upscale on load, since the filters are deterministic. On one thread the
  worst action takes EPX 27 / 52 ms, MMPX 59 / 144 ms, hqx 129 / 214 ms, xBRZ 161 / 387 ms (not a fully quiet machine),
  so it needs a background step. MMPX stored indexed costs 0.37× / 0.44× of 2× BC7.
- **Keeping everything by default.** Leaving out unreached art is at most opt-in. See section 7.

## 6. Measured since

**Final rows (2026-11-05), every body, lossless, zstd 19, one stream per (action, direction).** Sections 9–17 and the
recommendation of [the codec proposal](proposals/2026-11-01-mobile-anim-codec.md) hold every row and the method.

| Row | Disk | × real BC7 pack / shipped | Creature crowd, five directions / faced | VRAM, whole set |
|---|---:|---:|---:|---:|
| Classic: row spans, duplicates as pointers | 706.5 MB | 0.82 / 0.67 | 37.0 / 7.5 ms | R8 2474.5 MB |
| Enhanced, speed first: both planes in row spans, duplicates as pointers | 1869.6 MB | 0.94 / 0.94 | 100.3 / 20.5 ms | RG8 9891.3 MB |
| Enhanced, disk first: painted only + coverage spans, duplicates as pointers | 1808.7 MB | 0.91 / 0.91 | 129.0 / 26.3 ms | RG8 9891.3 MB |

These layouts fail the line on the worst crowd with five directions (about 260–550 ms) and pass it when each NPC loads its
faced direction first (about 55–112 ms), so faced-first loading is part of the recommendation.

The close call is Enhanced on desktop: BC3 + colour under transparent pixels is 1539.0 MB at SSIMULACRA2 84.52 / 66.43 / 46.73
(median, p5, min) and half the VRAM of RG8, but 204 of 926 bodies score under 80, hue has to work from colour, which no real
renderer does yet, and phones need another pack.

The six questions below came first. Their tables are in sections 7 and 8 of the proposal. Timings: 4 threads, boost off, EPP `balance_power`, no
GPU upload or disk reads. Crowds are 300 distinct (body, action) pairs: "uniform" treats every existing pair as equally
likely, and "worst" is the 300 largest (918 / 1231 Mpx, Classic / Enhanced). The line is 250 ms.

| Candidate, per (action, direction) | Uniform ms | Worst ms | Faced only, worst ms | Disk × shipped |
|---|---:|---:|---:|---:|
| LZ4 HC | 3.9 / 9.7 | 80 / 174 | 17 / 35 | 0.88 / 1.23 |
| BC7, zstd 7, as stored (lossy) | 9.6 / 15.3 | 184 / 231 | not run | 0.82 / 0.96 |
| zstd 19 | 14.7 / 31.1 | 278 / 496 | 58 / 103 | 0.72 / 1.01 |
| Luma, MED, zstd 19 | 93 / 128 | 2186 / 2624 | 448 / 539 | 0.78 / 0.92 |
| Luma, MED, tANS | 147 / 200 | 3287 / 4005 | 684 / 825 | 0.77 / 0.91 |
| Block motion, tANS | 179 / 239 | 3846 / 4732 | 805 / 973 | 0.74 / 0.86 |
| Rank + previous | 580 / 1010 | 9112 / 11704 | 1876 / 2381 | 0.67 / 0.80 |

Disk figures for lossless rows are estimates from every 10th body, and Classic compares with the 1060.3 MB the idx files
reference.

1. **Crowds:** zstd 19 has 8–17× to spare on the uniform crowd. On the worst crowd it passes only when each NPC loads its
   faced direction first. Whole-body units fail the uniform crowd for every lossless candidate except LZ4 HC.
2. **Fast prediction path:** table-driven decoding works, but it is 6–10× slower than plain zstd 19 in Classic, where it is
   also larger, and 4–6× slower in Enhanced for 9–10 % less disk.
3. **Block motion, lossless:** −4.8 % Classic and −7.8 % Enhanced overall, mostly in idle (−22 % / −24.5 %) and fidget.
4. **Why the floor is 5.5–5.7 bits:** it isn't per-frame palette noise. Static surfaces take only 19–22 % of index bits in
   idle actions and 4–5 % in others, frame-to-frame changes lack the correlation of noise (−0.23 to +0.05, against about
   −0.5), and Enhanced has one palette per action anyway. Pixels that change or move cost 5.9–6.4 bits, and static ones
   3.6–5.5 from their neighbours but 2.8–3.7 from the previous frame.
5. **Classic UOP against Enhanced:** not the same renders as stored, so deriving one from the other saves nothing. 0 %
   share a shape, 1.8 % share RGB555, and coding one against the other costs +38.5 % / +11.8 %. Different action or
   direction numbering isn't ruled out, though matching frame counts make it unlikely.
6. **AV1 4:4:4, lossy:** CRF 30 gives 0.57–0.68× BC7 at SSIMULACRA2 84–86, but under 2 % of pixels keep their colour and no
   hue-flagged pixel stays exact. It needs RGBA8 VRAM or a requantise step, and would miss the line at an estimated
   851 / 985 ms on the uniform crowd.

**The uniform crowd flatters every candidate.** It is mostly equipment (section 2). Measured on 2026-11-04, decode only, all
five directions on 4 threads:

| Crowd | Classic Mpx | Enhanced Mpx | zstd 19, Classic / Enhanced | Luma + MED, Classic / Enhanced |
|---|---:|---:|---:|---:|
| Uniform, 300 pairs | 27.1 | 49.9 | 15 / 31 ms | 93 / 128 ms |
| Creatures, 300 pairs | 105.6 | 216.6 | 40–43 / 110–120 ms | 305 / 530 ms |
| Dressed NPCs, 50 × (human + 5 equipment) | 10.1 | 15.6 | 6.5 / 16.5 ms | 37 / 55 ms |

The estimate scaled from the uniform rows was about 223 Mpx and 90–140 ms for Enhanced creatures, so it held. Luma + MED fails
the line on creatures with all five directions and passes with the faced direction only, at 60 and 112 ms. Repeats of zstd 19
drift by up to about 5 %, so smaller gaps are ties.

**Outside proposals checked against this data (2026-11-04),** with the rows in section 9 of
[the codec proposal](proposals/2026-11-01-mobile-anim-codec.md):

| Proposal | What the data says | Status |
|---|---|---|
| Concatenate each (action, direction) as one indexed stream, whole-frame reuse only, no block modes | This is the recommendation | Agreed |
| Whole-body streams for Classic | Smallest, but the largest takes 204 ms, against 11 ms for the largest action stream; fails the uniform crowd except with LZ4 HC | Rejected |
| A coding profile per stream, for tiny and for large or upscaled art | No size bucket flips a result. Choosing a residual per stream adds nothing on Classic, where plain wins 95 % of streams, and 0.3 % on Enhanced, where left nearly always wins | Not needed to choose a residual. Coders per class gain nothing on Classic and 2.4 % on Enhanced, with almost no margin on the line |
| Canonical body palette plus temporal index deltas | The premise of per-frame palette noise is wrong. A master palette gives −2 %. Master palette + XOR with the previous frame is 24 % larger on Classic and 13.9 % on Enhanced | Rejected |
| Dirty rectangles against the previous frame | Static pixels are only 4–5 % of index bits outside idle, about 20 % in idle, which caps the saving | Rejected |
| Static palette sort at no runtime cost | Sorting alone does nothing for LZ. A tight left residual now decodes at 0.83× zstd 19's speed on Enhanced, but painted only, with no sort or residual, is 1.5 % smaller at the same speed, so the gain came from dropping transparent pixels. Classic stays 7 % larger | Rejected |
| Shape and fill as separate planes | Enhanced already splits them. Every-pixel coverage tables save 32 % of that plane, about 6 % of the total, but fail five directions on creatures at 580 ms. Coverage in row spans saves 6.3 % of the plane at 0.95× the speed | Tables rejected; row spans are a final candidate |
| Cross-action dictionary of 4 × 4 patches | Nothing to gain across bodies; 8 × 8 exact copies are rare. Even per body, the 4 × 4 bound counting codebook and block ids is 11.8 % / 20.9 % larger than zstd 19 on the index plane | Closed |
| Scanline RLE plus FSE, no LZ, "at memory bandwidth" | With semi-adaptive tANS it is 9.7 % / 13.7 % larger than zstd 19 and slower: 151 / 322 ms on creatures against 41 / 120, so Enhanced fails the line. zstd's history isn't its bottleneck | Closed |

**Still not measured:** any low-end GPU, Mali or Apple GPU cost, hue from colour or a second shader path in a real renderer,
upload over PCIe to a discrete card, ETC2 RGBA8, BC3 in the GPU test, BC3 + colour's decode, disk reads, and a crowd weighted by
how often actions are really played. GPU upload and shader cost were measured headless on one Radeon 880M (section 15), as ratios
that say nothing about low-end hardware. The `timing` pass failed on memory twice and wasn't retried; `s1` and `s2` time the
same streams, so the ranking doesn't need it.

TLP used to re-apply boost and `balance_performance` during timed runs. `/etc/tlp.conf` now sets boost off and EPP
`balance_power` on mains, so new timings are in that state and don't compare in absolute terms with the older EPP
`power` tables. Each run carries its own zstd 19 baseline, so ratios still hold.

## 7. Never-drawn art: how it was determined, and how far to trust it

**Method.** Static only: nothing ran.

1. Read the client's per-body test in Ghidra: `BodyUsesUopFrames` (0x005978a0) is true when the body's `mobtypes.txt`
   row has flag 0x10000.
2. Parse the shipped `mobtypes.txt`, `Body.def` and `Bodyconv.def`.
3. Walk every body id. Flagged ids reach the UOP entry of that id; every other id reaches a MUL through the def files.
4. Whatever no id reaches counts as never drawn, under any expansion setting or with all enabled.

**Result as computed:** 85.0 MB of 657.6 MB of Classic index data under any expansion setting, or 91.1 MB with all
enabled. 73.5 MB of the 85 comes from 4 unflagged UOP bodies (compressed sizes below are shipped, not index-only):

| Body | `mobtypes.txt` | Shipped size |
|---|---|---|
| 198 | Chaos Variant | 44.9 MB |
| 197 | Order Variant | 39.5 MB |
| 267 | Troglodyte | 0.8 MB |
| 1525 | EQUIP_Tabbard_Gargoyle | 0.8 MB |

**This rule is wrong for one of the four, and was too simple.** Traced on 2026-11-03 in the code and in parses of the
shipped data; nothing ran. Details and addresses: `scripts/dev-helpers/ec-research/ghidra-classic-frames/results/uop_reachability.md`.

- The flag is tested on the object, not on the id it requests. On a flagged object, `GetObjectLayerFrame` (0x005441e0)
  requests its id untested, and for a layer item the id is the item's graphic. Proven by the code read.
- The UOP body isn't that id: it is the appearance of the id's `tileart.uop` row on the object. That is the id, unless
  property 8 or a filter says otherwise. Proven by the code read. Step 3 above is therefore not what the client does: flagged
  body 647, for one, resolves to 603.
- **1525 is drawn from UOP.** Tile 42002 resolves to 1525 for wearers 666, 667, 694 and 695. Proven by the code read and
  the shipped files. The 85.0 MB counts it as lost; that is about 0.8 MB of shipped size. The total was not recomputed.
- **197, 198 and 267 are not requested through any shipped table.** Proven on the shipped files: of the 277 bodies with
  UOP frames, they are the only ones reached neither by a flagged mobile's own body nor by a wearable tile on a flagged wearer.
- **They are not unreachable.** The packet handlers don't check that a layer item is wearable, so a server that sends tile
  197, 198 or 267 (a wall, an arch and a wall) on a layer of a flagged mobile causes the request. Proven as far as the request;
  not run. A mobile or corpse whose own body is one of them draws from a MUL.
- The conversion doesn't change this: `Body.def`, `Corpse.def` and `Bodyconv.def` apply after the test, on the MUL side, so
  the id tested is the server's own. Proven by the code read.
- `FUN_005b96a0` is `PreloadWeaponUopFrameSets`, likely a preload: actions 9-14 of a weapon-flag item on layer 1 or 2, with no
  test of 0x10000. It can load UOP frames for a human wearer, with shipped data only of flagged bodies, apart from 1525 if a
  server puts tile 42002 on layer 1 or 2.

**What is safe to say:** with the shipped files and a server sending wearable items, 197, 198 and 267 are not drawn from
UOP. Whether another route exists beyond the callers traced is not determined. Editing `mobtypes.txt` to flag them would
likely draw them: the placeholder, the sequence and the frames all ship. So their frames aren't useless to such a client.

Other limits:

- The rule describes `classic_7.0.103.0_client.exe` with its shipped `mobtypes.txt`. A shard or mod that edits that
  file can reach these bodies.
- Bodies 197 and 198 are among the largest in Classic, so the worst-case whole-body decode may come from art the client
  never draws.
- Enhanced is much smaller and is a separate rule: about 3.3 MB (bodies 344, 679, 827).

**Default:** keep every entry the idx or UOP table holds. Custom animations (need 8) rule out dropping art by default:
a shard can route any body. Leaving out unreached art can only be an opt-in converter flag, working from the files of
the install being converted, never from a fixed list of official bodies. Whether to offer the flag at all is open.

## 8. Decisions

**Made by the user:** decode speed outranks size. Runtime BC7 is off for now. Upscale before conversion. Format is free
to change, with no backward compatibility. All frames are kept: no halved tier. Official art may use restricted colour; custom art needs 32 bits.

The user set the line: 300 new actions drawable in about 250 ms on 4 threads is fast enough, and faster is better.

Runtime VRAM counts as well: wasted space there is undesirable (the user, 2026-11-04). Section 11 of the proposal has the
measured layouts, with tables and palettes counted: Classic / Enhanced MB are R8 or RG8 2474.5 / 9891.3, BC7 2533.4 / 5131.6,
BC1 1274.7 / 2578.4, row spans in linear memory 1386.2 / 4990.6. Whole atlas pages aren't a sensible unit: they turn 3.0 MB of
a dressed crowd's frames into 804 MB.

A lossy block mode for low-end devices is wanted, to reduce disk and VRAM (the user, 2026-11-04). It is chosen at pack time;
runtime BC7 stays off. The final rows: on Classic, BC1 above 2500 px with lossless R8 below is 523.5 MB, with a median of 81.77
on the coded bodies, and every BC7 row is larger than lossless. On Enhanced, BC3 + colour is 1539.0 MB at 84.52 and BC7 with fix C
is 1900.7 MB at 85.40, at the same VRAM; BC1's one-bit alpha takes its min to −47.93.

The repo's BC7 encoder had a real bug at the converter's flags: mode 4 with index flag 0 packed each weight plane into the other's
field. The user approved that fix, and it landed on 2026-11-05. Any BC7 figure from before that date and not labelled with a fixed
encoder reads the bug: section 12 of the proposal has the details. Fix C, the search change that lets alpha have its own plane, landed
the same day ([`bc7_alpha_plane_search.md`](bc7_alpha_plane_search.md)). The colour step is still undecided.

**Made by the user on 2026-11-05:**

- **Both a lossless and a lossy path are supported.** Which one a package uses is chosen at pack time.
- **Enhanced's lossless layout is both planes in row spans,** the faster of the two zstd rows: 1869.6 MB at 100.3 / 20.5 ms
  against painted + coverage spans at 1808.7 MB and 129.0 / 26.3 ms. 61 MB over gigabytes isn't worth being slower.
- **Compile only the shader pipelines a package needs, at startup.** A package's contents don't change at runtime, so its
  formats can be read once and the other paths never compiled. A package with one format gets one pipeline with no format
  branch. A package holding two, such as a low-end mix or custom BC7 art beside indexed frames, needs both paths live, and there
  a shared shader with a branch was level with grouped pipelines at 100/0 and 90/10 (proposal, section 15).
- **The 60 fps bar for integrated GPUs applies at fullscreen.** See `renderer_release_performance.md` for where this stands.
- **The lossless compressor is LZ4 HC.** On the chosen layouts, expanded for VRAM, with the creature crowd on 4 threads, five directions /
  faced: Enhanced is 2268.0 MB before duplicates at 36.4 / 7.5 ms and 169.5 / 34.9 ms in the worst crowd, against zstd 19's 1881.1 MB
  at 100.3 / 20.5 ms and 451.1 / 92.2 ms, so 2.75× as fast for 21 % more disk. Classic is 888.4 MB at 12.6 / 2.7 ms against zstd 19's
  733.4 MB at 37.0 / 7.5 ms, about 2.9× as fast. The gap reaches 3–4× only if LZ4 HC's frames stay as spans in VRAM.
  These are level 12, the highest, with no dictionary. LZ4's levels change only the compressor, so the level should cost pack time and nothing at
  run time (not yet compared between levels). Every dictionary figure in this file is zstd's: LZ4 with one hasn't been measured.
  **This moves the lossy comparisons,** which were all made against zstd 19's sizes. Enhanced's BC7 with fix C, 1900.7 MB, was slightly larger than
  lossless and is now 0.84× it; BC3 goes from 0.78× to 0.65×. Classic's BC1 is 0.58× lossless, and its BC7 with fix C, 918.4 MB, is still
  larger. The block rows also decode slower than lossless now, because they use zstd: Enhanced BC3 is 50.4 / 10.2 ms and BC7 with fix C 50.9 / 10.3 ms, against 36.4 / 7.5 ms.
- **The BC7 encoder takes fix C,** which landed the same day.
- **Classic equipment and wearer animations are lossless in every pack,** so block formats reach only creatures. Wearers are only the humans, elves and
  gargoyles that can equip items. The classes come from the placeholder rule under "Telling equipment apart", and all 1143 placeholders match between
  Enhanced's and Classic's `tileart.uop` on body type and race:
  - **Wearer:** the placeholder sets race. 13 bodies: humans 400–403, elves 605–608, gargoyles 666, 667, 694, 695, and 1253.
  - **Equipment:** no race, on a wearer's map (400, 666, 1253). This includes bodies `mobtypes.txt` lists as HUMAN, such as 183–186.
  - **Creature, may take a block format:** any other map, so orcs (17), lizardmen (35) and ratmen (42). Body 990 is listed HUMAN but plays its own map.
  - **No placeholder:** `mobtypes.txt`, then any worn tile that reaches the body. What nothing classifies stays lossless.
  No PartialHue tile in `tiledata.mul` reaches a body this calls a creature. A dressed character's layers share one format, so the format changes
  only between characters and creatures. This is the default for official data: the table in need 9 overrides it.
- **A custom table overrides the class rule and adds per-wearer frames** (need 9; the user, 2026-11-05). It's built with the format, not before. One strict
  KDL file goes to the packers, with these rows (the ids are made up):

  ```kdl
  // Like a mobtypes row, plus what mobtypes can't say.
  body 2100 kind="wearer" race=3 gender=0 actions=400
  body 2200 kind="equipment" actions=400

  // Same item, other frames, on one wearer. Like an Equipconv row.
  wear tile=5141 on=2100 body=2200 gump=72200
  ```

  - **Priority:** the table, then the client's own rows and the placeholder rule, then `mobtypes.txt`, then worn tiles, then lossless.
  - **Packages store the result,** so the renderer never reads the rule, the table or a `.uop` file. Changing the table means repacking.
  - **It can override official bodies too,** and an optional `coding="lossless"` forces that on one body. Race is an open number, not a list of three.
  - **These fail the pack:** an unknown key, a duplicate row, a body with no frames, and equipment whose map differs from its wearer's, which the client
    only skips silently.
  - **Frames aren't its job.** They still come from UOP, MUL or custom 32-bit art (need 5).
  - **A new wearer starts on an existing map,** such as dwarves on 400. An item with no version for it then draws the human one, as the client's wear check
    allows. Giving the wearer its own map hides such items instead, so no extra setting is needed.
- **A server's 0x8000 on a block-coded creature gets an approximate grey test, logged once** (the user, 2026-11-05). Its tolerance isn't set yet.
- **Classic data that can take partial hue stays lossless: exact greys can't be lost.** The client hues only pixels whose 5-bit channels match
  (`(c & 0x3FF) == c >> 5`, [traced](../../scripts/dev-helpers/ec-research/ghidra-classic-frames/results/classic_frames.md)), with the red byte as the key. On every Classic body BC1
  breaks that test on 30.60 % of grey pixels at 5 bits and 84.31 % at 8, and changes the key on 56.19 %. What it covers:
  - **By tile flag,** in `tiledata.mul`: 107 worn tiles with PartialHue reach 76 animation ids, and 12708 tiles that aren't worn flag their art.
    Their paperdoll gumps probably follow: traced in the Enhanced client (`UOPaperdoll::BuildImage`), not in Classic. BC7 for `gumps_cc` needs this check.
  - **By the hue's 0x8000 bit, from the server:** ServUO's `RandomSkinHue` sets it, and ClassicUO treats it as partial on any sprite. So bodies that
    take skin hue stay lossless too. A server can set the bit on any body, so files alone can't close this set.
  - Enhanced has no such test: it masks the classic bit away and takes hueability from the palette's flag, which `mobile_anim_ec` packs as a mask.

**Open:**

| Question | Recommendation | Depends on |
|---|---|---|
| Lossy rows on the PC preset | The user's proposal: Enhanced BC1 for equipment and BC3 for other animations, BC7 for land textures or gumps, Classic BC1 on creatures only, converging on one or two lossy formats if the mix needs too many branches or pipelines. Against LZ4 HC's lossless size, BC3 and BC7 with fix C both save disk as well as VRAM on Enhanced, so BC7 is no longer ruled out by size | Fractional zoom, upscaled sprites and the cost of alternating formats in painter order, being measured |
| Load unit | One stream per (action, direction), faced direction first | The user |
| Where the low-end tier's quality floor sits, and which RDO level it uses | 80 as a floor, no RDO unless a target needs it | The user's eye: `blocks-preview/` crops, including BC3 against BC7 |
| Whether indexed frames stay the default once filtering is counted | Yes: the indexed filter costs 1.43–1.57× BC7 on the Radeon 880M, 2.4–3.1 % of a 60 fps frame in a crowd at 1.5× | The user; one AMD iGPU only |
| Whether the low-end tier block-codes only large bodies, and at what size | Classic: BC1 above 2500 px, with a median of 81.77 on coded bodies, measured before equipment and humanoids were kept lossless and being re-measured. Enhanced: the same mix with BC7 fix C is 1931.1 MB against 1539.0 MB for BC3 + colour on every body | The user; a second shader path and hue from colour are unbuilt |
| Whether phones and web get their own low-end packs or one Basis Universal pack | Stay on the universal R8/RG8 pack, and ASTC 4×4 only if VRAM forces it. No Basis row was both smaller and better than the plain format beside it | The user |
| LZ4 HC with a dictionary | Not measured. LZ4 reads only a dictionary's last 64 KiB and has no entropy stage, so zstd's −3.0 % per body doesn't carry over. The rows being run: trained per body, the body's standing stream, one shared per package | The bench, then the user |
| Whether the low-end tier uses BC1, BC3 or BC7 | Classic BC1, on creatures only. Enhanced BC3 + colour or BC7 with fix C, not BC1 | The user |
| Which GPU layout holds frames in VRAM | R8/RG8 with faced-first loading, not whole atlas pages. Row spans in linear memory only if a device proves too small | The user; no low-end device measured |
| Whether lossy packs take the colour step | Not for what it shows at 1×: about +5 % on disk, and on the sheets it isn't visible at game size on NPCs (the user agreed by eye, 2026-11-05). It clears small specks on a few bodies when zoomed, and halves the pixels under 80 on BC3. **Fractional zoom decides it:** the seam filter's hardware sample (`art/main.wesl`) reads straight RGB, so at seams beside transparent pixels it mixes in the colour stored under them. That is transparent black without the step, and every sheet so far is at 1× with no filter. Storing premultiplied colour is the alternative under test | The user; the zoom test |
| Runtime interpolation for high frame rates | A separate study | The full frame set |
| Whether to offer an opt-in flag for unreached art | Keep all | Section 7's trace |

## 9. Options set aside, and why

Section 4 has the earlier failed attempts, and section 6 the outside proposals. This adds what the final rounds set aside. Figures
are from [the codec proposal](proposals/2026-11-01-mobile-anim-codec.md), on every body unless marked; decode is the creature crowd on
4 threads, five directions / faced. Repeat one only with a new reason.

**Decided by the user:**

| Option | Figures | Why not |
|---|---|---|
| Enhanced painted + coverage spans | 1808.7 MB, 129.0 / 26.3 ms | 61 MB doesn't justify being slower (2026-11-05) |
| Enhanced coders per class (section 10's assignment) | 1761.7 MB, 232.3 / 57.7 ms | Same decision, and almost no margin on the line |
| One shader with a format branch on every install | 8–26 % slower when every sprite takes the cheap path | Pipelines compiled at startup from the package's formats cost nothing. The branch stays for mixed packages only (2026-11-05) |
| zstd 19 as the lossless compressor | Enhanced 1881.1 MB before duplicates, 100.3 / 20.5 ms, worst crowd 451.1 / 92.2 ms. LZ4 HC is 2268.0 MB, 36.4 / 7.5 ms, worst 169.5 / 34.9 ms | 2.75× the decode speed is worth 21 % more disk, and LZ4 HC is the only one to pass the worst crowd with five directions (2026-11-05) |
| Lossless only, or lossy only | | Both paths are supported (2026-11-05) |
| Classic BC1 on every body | 514.2 MB at 82.90 | Loses exact greys on what can take partial hue (2026-11-05) |
| Classic BC1 with masks for partial hue | One-bit masks 279.2 MB, with BC1 1537.9 MB of VRAM against 2419.0 MB for R8 | Not chosen: the user keeps this data lossless (2026-11-05). A mask would keep which pixels take the hue, but BC1 still moves their grey level, which picks the hue's colour |
| Halved frames | See section 4 | All frames are kept (2026-11-04) |
| Runtime BC7 | See section 4 | Off for now; possibly later for upscaled animations |

**Set aside by the measurements, with nobody asking for them:**

| Option | Figures | Why not |
|---|---|---|
| Plain rectangle, Enhanced | 2013.8 MB before duplicates, 120.1 / 23.3 ms | Larger and slower than row spans |
| Index in spans with a plain coverage plane, Enhanced | 1955.7 MB before duplicates, 108.6 / 22.3 ms | Larger and slower than both planes in spans |
| Plain rectangle, Classic | 38.7 / 7.7 ms against 37.0 / 7.5 | Row spans are level on speed and 3.6 % smaller |
| Whole-body load unit | Fails the line with every lossless candidate but LZ4 HC | Per (action, direction) costs +12 % / +3 % on disk and lets a crowd load only what it draws |
| Loading all five directions at once | Worst crowd about 260–550 ms | Faced first passes at about 55–112 ms, with 5× less decode and VRAM |
| Luma, MED + block motion, tANS | Classic ≈ 784 MB, 1.03× zstd 19, 524.0 / 125.3 ms | Larger and slower than zstd 19 |
| Rank + previous frame, rANS | Classic ≈ 712 MB, 1762 / 361.2 ms | Smallest, and fails the line |
| Split boxes | Bytes don't move; under 1.5 MB of a town's atlas | Not worth a second box |
| BC7 as the default on disk | Classic RDO 1 823.2 MB at 80.20 against 733.4 MB. Enhanced fix C 1900.7 MB at 85.40 | Larger than lossless, and lossy. Measured against zstd 19: against LZ4 HC (888.4 / 2268.0 MB) both are smaller, so size no longer rules it out on its own, and it stays aside for being lossy |
| BC7 fix C with RDO 1, Enhanced | 1728.9 MB at 81.03, 558 of 926 at 80 | BC3 beats it on every score at 0.85× the disk |
| BC1, Enhanced | 929.5 MB at 73.75 / 52.00 / −47.93, 170 of 926 at 80 | One-bit alpha: 10.361 % of painted pixels disappear |
| BC3, Classic | Every 10th: 583.9 MB at 83.25, BC1 504.2 MB at 83.17 | Classic's alpha is one bit, so it gains nothing at twice the VRAM |
| R8 index + BC4 coverage | 1.47× BC7's VRAM; 0.9971 % of pixels change level | Not lossless, and not small |
| BC7 mode 7 on (fix B) | Sample, with fix C + colour: Enhanced 88.12 at 1994 MB against 87.36 at 1943 MB | Small gain; the converter doesn't set those flags. Bugs B remain in those paths (proposal, section 12) |
| ISPC BC7 | Sample: Enhanced 86.94 / 74.10 / 70.34 at 1988 MB, about 40 worker seconds a Mpx against 0.14 | Near fix C's median at about 280× the encode cost; `intel_tex_2`'s licence wasn't verified |
| ASTC 8×8 | Every 10th, Classic / Enhanced: 313.5 / 649.7 MB, 59.86 / 57.59 | Far below the 80 line |
| AV1 per (action, direction) | Classic CRF 20: 0.0 % of hue-flagged pixels exact | Fails the crowd line and colour fidelity |
| Whole atlas pages as the load unit | 3.0 MB of a dressed crowd's frames holds 804 MB | Long-lived pages filled by streams instead |
| One upload per frame | 4–11× longer than one upload per load | The only thing that brings a crowd near the line |
| A hue read per filter tap | 1.62–1.64× a plain index draw, against 1.10–1.15× for a palette row hued in advance, at 1× | Hue the palette row in advance |

**Parked: not chosen now, and may return if a need appears:**

| Option | Figures | Returns if |
|---|---|---|
| Row spans held in linear memory | 0.97× BC7's VRAM, lossless, 1.99–2.13× BC7 on the GPU | A device proves too small for RG8 |
| zstd dictionary per body | Classic −3.0 %, 711.1 MB; reads 25.6 MB instead of 6.2 MB for a faced crowd | The user wants it for Classic |
| BC1 with block reuse, λ 1 | Classic, every 10th: 445.8 MB at 81.05 | The crops pass the user's eye |
| ASTC 4×4 | Every 10th, Classic / Enhanced: 1022.2 / 2128.9 MB at 90.41 / 87.62 | VRAM forces it on a phone; larger than lossless on disk |
| ASTC 6×6, ETC2 RGB8A1 | 75.28 / 73.08 and 74.05 / 71.63, both under the 80 line. ETC2 loses 11.582 % of Enhanced's painted pixels | The user accepts them by eye |
| Basis Universal | No row both smaller and better than the plain format beside it | One file for every GPU matters more than size |
| xBRZ filtered at load | Too slow even for faced directions: 268 / 510 ms, estimated | Stored instead, if it's wanted |
