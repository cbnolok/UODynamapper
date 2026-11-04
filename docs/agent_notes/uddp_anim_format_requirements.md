# UDDP mobile animation format: needs, constraints, desires, failed attempts

Status (2026-11-02): **no format chosen yet.** This file says what the format has to do and what we already tried, so
nobody repeats a dead end. The measurements and their method are in
[`proposals/2026-11-01-mobile-anim-codec.md`](proposals/2026-11-01-mobile-anim-codec.md), and the benchmark is
`lib/udd-conv/tests/integration/mobile_anim_codec.rs` with `mobile_anim_codec/shrink*`.

Reading the numbers:

- "× BC7" compares with the benchmark's BC7 + zstd estimate: 782.5 MB Classic, 1823.3 MB Enhanced, per body with no
  atlas or gutters. It flatters BC7 slightly, because real packs are larger:

  | | Benchmark estimate | Pack from today's packer (`--bc7 --zstd`, not installed) | Installed today |
  |---|---:|---:|---:|
  | Classic | 782.5 MB | 856.6 MB | 985.6 MB (30 Jun) |
  | Enhanced | 1823.3 MB | 1987.9 MB | 6904 MB (30 Oct) |

  The installed Enhanced package predates compression: 2291 of its 2522 entries are stored raw, and 73 % of its atlas
  area is empty.
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

### Minimum GPU

The bar is the GTX 900 and 10 series and AMD Polaris (the user, 2026-11-04). BC1, BC3, BC7, storage buffers and compute can
all be used without a fallback. None of the formats discussed here needs anything newer than that. Vulkan versions per GPU,
and Intel and AMD integrated GPUs, aren't checked yet.

Phones are out of scope. A WebGPU survey of Android reports BC support at 0 % on ARM Mali, about 25.5 % on Adreno 6xx and
100 % on Adreno 7xx and 8xx. An Apple forum answer says iOS devices generally lack it. A phone mode would need ASTC or ETC2.

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
12.4 %, which is the basis of the rule's table further down:

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
shares suggest. Against the rectangle at zstd 19, on every 10th body:

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
- **Do clearly better than the 20-year-old clients.** Classic already does (0.64× of the referenced shipped data). Enhanced
  does not yet (0.98×).

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
| Trained dictionaries, large bodies as a reference | ×1.000–1.025 |
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

All six questions now have answers. Full tables are in sections 7 and 8 of
[the codec proposal](proposals/2026-11-01-mobile-anim-codec.md). Timings: 4 threads, boost off, EPP `balance_power`, no
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
| A coding profile per stream, for tiny and for large or upscaled art | No size bucket flips a result. Choosing a residual per stream adds nothing on Classic, where plain wins 95 % of streams, and 0.3 % on Enhanced, where left nearly always wins | Not needed to choose a residual. Coders per class are still being measured |
| Canonical body palette plus temporal index deltas | The premise of per-frame palette noise is wrong. A master palette gives −2 %. Master palette + XOR with the previous frame is 24 % larger on Classic and 13.9 % on Enhanced | Rejected |
| Dirty rectangles against the previous frame | Static pixels are only 4–5 % of index bits outside idle, about 20 % in idle, which caps the saving | Rejected |
| Static palette sort at no runtime cost | Sorting alone does nothing for LZ. A tight left residual now decodes at 0.83× zstd 19's speed on Enhanced, but painted only, with no sort or residual, is 1.5 % smaller at the same speed, so the gain came from dropping transparent pixels. Classic stays 7 % larger | Rejected |
| Shape and fill as separate planes | Enhanced already splits them. Every-pixel coverage tables save 32 % of that plane, about 6 % of the total, but fail five directions on creatures at 580 ms. Coverage in row spans saves 6.3 % of the plane at 0.95× the speed | Tables rejected; row spans still a candidate |
| Cross-action dictionary of 4 × 4 patches | Nothing to gain across bodies; 8 × 8 exact copies are rare. Even per body, the 4 × 4 bound counting codebook and block ids is 11.8 % / 20.9 % larger than zstd 19 on the index plane | Closed |
| Scanline RLE plus FSE, no LZ, "at memory bandwidth" | With semi-adaptive tANS it is 9.7 % / 13.7 % larger than zstd 19 and slower: 151 / 322 ms on creatures against 41 / 120, so Enhanced fails the line. zstd's history isn't its bottleneck | Closed |

**Still not measured:** GPU upload, disk reads, a crowd weighted by how often actions are really played, and the row-span and
painted-only rows of the Enhanced worst crowd. That job stopped at the 18 GB watchdog, and a rerun of those rows is planned. The `timing` pass failed on memory twice and wasn't retried; `s1` and `s2` time the same
streams, so the ranking doesn't need it.

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

Runtime VRAM counts as well: wasted space there is undesirable (the user, 2026-11-04). Figures so far are pixels × format,
without atlas waste or tables, and the real measurement is pending.

A lossy BC7 mode for low-end devices is wanted, to reduce disk and VRAM (the user, 2026-11-04). It is chosen at pack time;
runtime BC7 stays off. What it must deliver is still being measured. So far, per (action, direction), BC7 + zstd 7 is larger
on disk than the lossless candidates (≈874 / ≈1904 MB). Its VRAM saving is on Enhanced only (5.11 GB against 9.83, hue masks
not counted; Classic 2.52 against 2.42). Its creature-crowd decode is faster (29 / 51 ms against 41 / 120 for zstd 19).

**Open:**

| Question | Recommendation | Depends on |
|---|---|---|
| Storage per client | Indexed slot planes, lossless, zstd 19, hue in the shader; LZ4 HC is the alternative at about 22 % more disk | The user |
| Load unit | One stream per (action, direction), faced direction first | The user |
| Where the low-end BC7 tier's quality floor sits, and which RDO level it uses | 80 suggested as a floor | Rows being measured; contact sheets; the user's eye |
| Whether the low-end tier uses BC1 (DXT1), BC7 or a mix | BC1 is the only format that halves VRAM on both clients, at 0.5 bytes per pixel. It keeps 3 colours in blocks with transparency, loses Enhanced's anti-aliased edges and needs a smaller hue mask. Classic stays lossless R8 unless BC1 proves acceptable, since BC7 saves it nothing | Rows being measured |
| Which GPU layout holds frames in VRAM | Decide after the measurement | Resident bytes per layout; atlas waste; shader cost on a GPU |
| Runtime interpolation for high frame rates | A separate study | The full frame set |
| Whether to offer an opt-in flag for unreached art | Keep all | Section 7's trace |
