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
- **Palettes change far less often than frames.** Files store one with each frame, 256 entries or fewer, but a smoke
  run on every 100th body measured 1.00 palette per action in Enhanced and 3.73 in Classic (1.46 distinct by content).
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

What is not safe yet:

- **An entry that fails to decode is dropped silently** (`:842`). It should be reported per entry, and no custom art
  should disappear without a line in the log.
- **Runs outside the frame's declared height are dropped silently** (`classic/anim.rs`). The client's behaviour for
  those hasn't been compared.
- **No test converts a MUL with custom entries.** Needed: an added body in an unused slot, a replaced body with its old
  bytes orphaned, an idx longer than the shipped one, use of palette index 0, frames larger than any shipped.
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
- Current zstd 19 costs (Classic / Enhanced): 746 / 879 MB/s; a whole body takes a median of 0.9 / 5.0 ms, at most
  197 / 363 ms; a per-action batch shows its first frame in a median of 34 / 96 µs, at most 6 / 10 ms.
- Upload to the GPU is a separate cost, and those numbers leave it out.
- Decoding belongs on worker threads, so a slow job delays a mobile's first appearance rather than the frame rate.

### Project rules

- No backward compatibility: our packages have not shipped. Format layout is ours to choose, and it doesn't need to
  look like the client's. Only the values we need have to survive.
- No magic numbers, named constants only. Don't change the format without the user's confirmation.
- Never execute client binaries or write into client folders. Client behaviour comes from static analysis.
- The test machine has 27 GB of RAM. Benchmarks run one at a time with an 18 GB cap.

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
| Pixel difference from the previous frame | 14–55 % larger | Palettes differ per frame, and limbs move, so differences are noisy. |
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
| Rank coding with previous-frame context, the smallest lossless result (Enhanced 0.878× BC7, 0.81× shipped) | **Too slow as measured** (*provisional*): 14 MB/s Classic, 40 MB/s Enhanced against 746 / 879 for zstd 19. |
| Rank coding from a predicted colour, with a coarse lookup | Enhanced sample only 2.8 % over the smallest (Classic 3.5 %), and the fastest rank variant, but still 20 / 127 MB/s (*provisional*). Classic is ruled out; Enhanced depends on the fast path in section 6. |
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
hasn't given a verdict yet; the contact sheets are in `uodynamapper-scratch/anim-codec/halved/`.

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

## 6. Still untested

Measure-only, queued after the timing rerun. The ranking is crowd speed first, size second.

1. **Crowd scenario:** 300 distinct (body, action) pairs by real distribution, plus a worst case, on 1 and 4 threads,
   for whole-body, per-action and per-direction units. Reports time until drawable, the worst job, upload bytes and
   VRAM.
2. **Fast prediction path:** ordered palette, residuals, context-split streams, and table-driven decoding instead of
   per-pixel rANS and palette sorting.
3. **Motion-compensated prediction, lossless:** the moved previous frame as the predicted colour. Earlier tests only
   covered unmoved references and exact matches.
4. **Why the floor is 5.5–5.7 bits:** the same smoke run shows no frame-to-frame noise signature, so per-frame palette
   noise looks like the wrong explanation. The full run on every 10th body decides it.
5. **Classic UOP against Enhanced:** whether they're the same renders, so one could derive from the other when both are
   installed.
6. **A real video codec (AV1 4:4:4), lossy:** the likely largest saving and the biggest risk to colour and hue.

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

**This rule is not proven complete.** Reading the callers on 2026-11-02 showed:

- `GetMobileFrame` (0x005bc880) tests one id, the object's field +0x3E, but requests the frames of its argument.
- `FUN_005441e0`, the other caller of `GetUopFrame`, first tests field +0x38 of its object the same way, and then
  requests the frames of its own id argument.
- So a flagged object can request an id that isn't flagged. Body 1525 is the likely case: it's equipment for a flagged
  wearer, so it is probably drawn, and the rule counts it as lost. This has not been traced.
- `FUN_005b96a0` also requests frame sets for actions 9–14 without this test, which looks like preloading. Not traced.

**What is safe to say:** a mobile whose own body is 197 or 198 draws from a MUL, since their `Bodyconv.def` rows point
there. Whether anything else requests those two UOP bodies is not proven. To confirm it, trace what the callers
(`FUN_0044fd50`, `FUN_0044d980`, `FUN_00450f90`, `FUN_004517a0`, `FUN_00451d50`, `FUN_00452f90`, `FUN_00453ba0`) pass
in, then check that the UOP frames of 197 and 198 are no use to a client that edits `mobtypes.txt`.

Other limits:

- The rule describes `classic_7.0.103.0_client.exe` with its shipped `mobtypes.txt`. A shard or mod that edits that
  file can reach these bodies.
- Bodies 197 and 198 are among the largest in Classic, so the 197 ms worst-case decode above may come from art the
  client never draws.
- Enhanced is much smaller and is a separate rule: about 3.3 MB (bodies 344, 679, 827).

**Default:** keep every entry the idx or UOP table holds. Custom animations (need 8) rule out dropping art by default:
a shard can route any body. Leaving out unreached art can only be an opt-in converter flag, working from the files of
the install being converted, never from a fixed list of official bodies. Whether to offer the flag at all is open.

## 8. Decisions

**Made by the user:** decode speed outranks size. Runtime BC7 is off for now. Upscale before conversion. Format is free
to change, with no backward compatibility. Official art may use restricted colour; custom art needs 32 bits.

**Open:**

| Question | Depends on |
|---|---|
| Storage per client | Crowd scenario, fast prediction path, video codec bound |
| Load unit: per action or per direction | Crowd scenario |
| Whether any lossy tier is acceptable, and where the quality floor sits (80 suggested) | Contact sheets; the user's eye |
| Halved frames: pursue with a better interpolator, or drop | Contact sheets |
| Runtime interpolation for high frame rates | A separate study on the full frame set |
| Whether to offer an opt-in flag for unreached art | Section 7's trace |
| What "fast enough" means in numbers | The user: no budget has been set beyond "hundreds on screen, no stutter" |
