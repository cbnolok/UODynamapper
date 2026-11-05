# Mobile animation storage: measured candidates (2026-11-01)

Status: **measurement only, for the user's decision**. No package format or renderer change has been made. What the
format has to do is in [the format requirements](../uddp_anim_format_requirements.md); this file holds the measurements.

The benchmark is `lib/udd-conv/tests/integration/mobile_anim_codec.rs` (`#[ignore]`). It reads every body of the
canonical clients, `_Ultima Online Classic` and `_Ultima Online Enhanced`, and keeps everything in memory:

```sh
UO_CC_CLIENT_DIR=<Classic client> UO_EC_CLIENT_DIR=<Enhanced client> \
  cargo test --release -p udd-conv --test integration mobile_anim_codec:: -- --ignored --nocapture
```

- `MOBILE_ANIM_CODEC_PASSES=timing` (or `sizes`, `upscale`, comma-separated) runs only those passes.
- `MOBILE_ANIM_CODEC_STRIDE=n` measures every n-th body only.
- The timing pass prints the CPU state (turbo boost and the CPUs' energy preference) and the load average at its
  start and end. If the CPU state changes during the pass, it discards the timings.

The unit is the user's choice (a): a **body batch**, meaning every action, direction and frame of one body from one
source. Each Classic `anim*.mul` and each Classic `AnimationFrame*.uop` counts as a source. For Enhanced, all the
`AnimationFrame*.uop` files together are one source. Frames are trimmed to their non-transparent box and stored back
to back per direction, in frame order. Frame metadata (size, anchor, palette reference) is 14 bytes per frame in
every variant, so it is left out of the payload sizes: 14 MB for Classic and 22 MB for Enhanced.

- **Classic pixels** are one palette index each; index 0 means transparent in shipped art, since no shipped pixel uses
  it. A custom MUL entry may use it, and the client draws it as opaque black. See "Custom MUL animations" in section 8.
- **Enhanced pixels** are an index plus a coverage value from 0 to 16 (sixteenths), stored as RG8.
  - **Interleaved** stores the two bytes of each pixel side by side.
  - **Planar** stores all the indices of the batch first, then all the coverages.

## Summary

| Question | Answer |
| --- | --- |
| **The recommendation** | **Indexed, lossless, zstd 19, one stream per (action, direction), faced direction first.** Classic in row spans, 706.5 MB with duplicates as pointers (0.82 × a real BC7 pack, 0.67 × shipped). Enhanced with both planes in row spans, 1869.6 MB (0.94 ×, 0.94 ×), or painted + coverage spans at 1808.7 MB (0.91 ×, 0.91 ×) if disk leads. **Fix the BC7 encoder whatever the format.** A low-end pack is a separate option, below. The user decides; the Recommendation section has the reasons. |
| **300 new actions on 4 threads** (the user's line: about 250 ms; section 8) | Every recommended row passes on creatures and dressed NPCs, five directions and faced. Uniform crowd: 14.7 / 31.1 ms, or 3.1 / 7.0 ms faced. The 300 largest pairs take 278 / 496 ms with all five directions, and 58 / 103 ms faced. Timings are boost off, EPP `balance_power`. |
| **The crowd that decides: creatures** (section 9) | 300 pairs of non-equipment bodies, 105.6 / 216.6 Mpx. Zstd 19 takes 38.7 / 108.0 ms in the quiet rerun (40.6 / 120.1 in the first run), or 7.7 / 21.7 ms faced. Luma + MED and block motion fail with all five directions (305 to 920 ms). A town of 50 dressed NPCs is 5.8 / 14.1 ms. |
| **Layouts that beat the rectangle** (sections 9, 16; every body, decoded back) | No residual, no tables, no shipped statistics. **Classic, row spans:** 733.4 MB, −3.6 %, at the same crowd speed. **Enhanced, both planes in row spans:** 1881.1 MB, −6.6 %, and faster than the rectangle. **Enhanced, painted only + coverage spans:** 1819.9 MB, −9.6 %, about 7 % slower. |
| **Coders per class** (section 10) | On Classic a class rule adds nothing: row spans with a dictionary per body is 702.5 MB sampled as one coder, and a per-stream id reaches 702.4. On Enhanced it saves 2.4 %: the assignment is 1761.7 MB on every body, 0.875 ×, at 232 ms on creatures, so it has almost no margin. |
| **The tight residual decode** (section 9) | Enhanced left is 717 MB/s, 0.83 × zstd 19 in the same run. Painted only, with no residual, is smaller at the same speed, so the residual isn't needed. |
| Faster, at a size cost | LZ4 HC: 3 to 4 × faster, and passes the worst crowd with all five directions. 22 % larger on disk (929.7 / 2459.3 MB). |
| Smaller, at a speed cost | Enhanced only: every-pixel coverage tANS is 1718.2 MB, −15 %, and fails five directions at about 590 ms if used on every class. Rank coding is the smallest and fails the line. |
| Why is the floor so high? | It isn't per-frame palette noise. Static pixels take 4–5 % of the index bits in most actions; the rest is fine spatial detail at about 6 bits. Enhanced has one palette per action. |
| Redundancy between bodies or clients | None. A solid `--long=31` stream saves 1 %. Classic UOP and Enhanced aren't the same renders (0 % same shape, 1.8 % same RGB555). |
| A real video codec | AV1 CRF 30: 0.57–0.68 × BC7 at SSIMULACRA2 84–86, LOSSY. Under 2 % of pixels keep their colour, the palette and hue are gone, and it fails the line at about 0.9 s. |
| Disk, every body, per stream | Lossless plain planes at zstd 19: **761.1 MB Classic, 2013.8 MB Enhanced,** palette table in (19.8 / 32.4 MB). Whole-body units would be 677.4 / 1950.8 MB, but they fail the line. |
| zstd level, LDM | Level 19 is 7 % smaller than level 9 (Classic) and 6 % smaller (Enhanced). Long-distance matching gains under 1 %. |
| 8×8 block copy, frame difference | **No gain.** Block copy is 0.5 % larger, and the difference from the previous frame is 14–55 % larger. |
| **The BC7 encoder** (section 12) | **The repo's encoder has a real bug that reaches every package built with `--bc7`.** `encode_mode4` packs the 3-bit plane into the 2-bit field. Fix A is approved and landed after this benchmark: it lifts Enhanced's median from 60.21 to 69.99. Fix C, a search change the user approved and which landed on 2026-11-05, takes it to 85.40 and Classic's from 67.39 to 87.51. Fix C changes disk by −0.7 % on Enhanced and +2.6 % on Classic. |
| **Does a lossy row beat lossless at 80?** (section 16; every body) | **BC7: Classic no,** RDO 1 is 823.2 MB at 80.20, against 733.4 MB. **Enhanced narrowly, and only by median:** fix C with RDO 1 is 1728.9 MB at 81.03, 0.98 × the assignment, with 368 of 926 bodies under 80. **BC3 and BC1 do better.** Enhanced's BC3 is 1463.1 MB at 81.72, 0.83 ×. BC3 + colour is 1539.0 MB at 84.52, 0.87 ×, with 204 of 926 under 80. Classic's BC1 is 514.2 MB at 82.90, 0.70 × row spans, with 327 of 1022 under 80. |
| **VRAM** (section 11) | The whole set: Classic R8 2.42 GB, BC7 2.52 GB, BC1 1.26 GB. Enhanced RG8 9.83 GB, BC7 and BC3 5.11 GB, BC1 2.56 GB. **What's loaded matters first:** a creature crowd's faced frames are 25.8 / 79.9 MB against 132.7 / 396.6 MB for five directions. Whole pages undo that: 3.0 MB of frames holds 804 MB of Enhanced pages. |
| **GPU cost** (section 15; ratios on one AMD GPU) | The indexed filter is 1.43 to 1.57 × BC7, 0.05 to 0.15 ms more at the crowd's pixels, and row spans are 1.84 to 2.13 ×. The crowd's faced pass takes at most 2.5 % of a 60 fps frame with the indexed filter and 3.3 % with row spans. **This GPU can't prove low-end results.** |
| **A low-end pack** (sections 13, 14, 17) | **Classic BC1 is the good one:** half the VRAM, 514.2 MB at 82.90 on every body, or 445.8 MB at 81.05 with block reuse on the sample. **Enhanced can't use BC1,** whose one-bit alpha removes edges (min −47.93). **BC3 + colour is the row to look at:** BC7's VRAM, 1539.0 MB at 84.52, 0.81 × BC7 with fix C and level with it at the 80 line (722 against 712 of 926). It has a third of fix C's edge error and lower colour accuracy (29.76 against 31.34 dB). **No phone format under BC7's bytes reaches 80:** ASTC 6×6 is 75.28 / 73.08 and ETC2 RGB8A1 is 74.05 / 71.63. ASTC 4×4 does at 90.41 / 87.62, but it's larger than lossless on disk. |
| Enhanced at 8 bpp | **No.** 19 % of pixels have partial coverage (per-body median 32 %), and the (index, coverage) pairs fit 255 entries in under 1 % of bodies. |
| Runtime BC7 | About **22–23 Mpx/s per thread**, 19–43 × the decode time. **The user has decided: no runtime BC7 for now,** possibly later as an option for upscaled animations. |
| Basis Universal (section 14) | Its one gain is a single file for every GPU, against one pack per platform group. No Basis row is both smaller and better than the plain block format beside it. Every load pays a transcode, so with all five directions on creatures nothing passes the line on Enhanced, and on Classic only ETC1S to BC1, which has no alpha. |
| 2× upscale | MMPX costs 1.38–1.48 × the 1× size and stays indexed. xBRZ/hqx requantized to 256 colours cost 2.9–3.7 ×, about the same as BC7 (3.1–4.2 ×), with SSIMULACRA2 above 91 at the median. |
| Motion | An exact 8×8 shift between neighbouring frames is rare: 3 % of blocks (Classic) and under 1 % (Enhanced). Keyframe interpolation would have to be approximate. |
| Load unit | A whole-body batch takes a median of 0.9 ms (Classic) or 5.0 ms (Enhanced) to decode, and up to 197 / 363 ms. A per-action batch shows its first frame in a median of 34 / 96 µs, with a maximum of 6 / 10 ms (boost off, EPP power). In the newer CPU state one action stream takes a median of 20 / 70 µs (section 8). |

## Timing conditions

Sections 1–6 time one thread of an AMD Ryzen AI 9 365, with **boost off and EPP power**. Each time is the fastest of 3
runs, and the files are already in memory. **Section 8 uses boost off, EPP balance_power, platform profile performance,**
the state `/etc/tlp.conf` now keeps, so its absolute times don't compare with these. Each run has its own zstd 19.

The tables come from a timing-only run (`MOBILE_ANIM_CODEC_PASSES=timing`) on a quiet machine, after the
renderer's web-map export had ended. No other agent was running.

| run | load average at the start of the timing pass (1 / 5 / 15 min) | at its end |
|---|---|---|
| **quiet, timing only (these tables)** | 3.83 / 2.24 / 5.93 | 1.04 / 3.42 / 5.57 |
| full run, contended by the web-map export (discarded) | 15.13 / 20.33 / 28.23 | 1.04 / 4.80 / 13.16 |

- **The quiet run's starting load is the benchmark's own.** The load was 0.43 when it was launched.
  - Without the size pass, the benchmark first loads every body in parallel to rank them by pixel count.
  - It also prepares each chunk of 20 sampled bodies' payloads in parallel between timings.
  - The timed steps themselves run alone, on a one-thread pool.
- **The quiet medians are 3–20 % lower than the contended run's.** Classic block copy is 20 % lower, most Classic
  steps 10–15 % and every Enhanced step under 10 %. No conclusion changed.

**The last reruns (sections 9, 13, 14 and 17, on the night of 2026-11-05)** ran in that same state, on the first L3 cluster (CPUs 0-3 and 10-13). A log recorded both clusters' load every 30 s. The other cluster stayed at 1.0 to 1.2 % busy on average per job, with a peak of 4 %, so these rows count as quiet. Rows from an earlier window, when builds ran at idle priority on the other cluster, moved by at most 2.4 % on five directions and changed no pass or fail. **Runs differ more than rows do,** so compare a row only with the zstd 19 row of its own table. Size rows aren't timed and may share cores.

## 1. Sizes

The whole set is 1022 Classic bodies (996,620 frames, 2419 Mpx trimmed) and 926 Enhanced bodies (1,572,711 frames,
4917 Mpx trimmed).

"BC7 + zstd 7" estimates today's packer. It compresses each frame padded to whole 4×4 blocks with the container's
zstd level 7, and it doesn't count the atlas pages or gutters. Real packs from today's packer are about 9 % larger:
856.6 MB for Classic and 1987.9 MB for Enhanced, measured and since deleted. The installed packages are older and
larger still; see "Baselines" in section 7.

### Classic

| variant | total MB | vs BC7 | per body median KB | p95 KB | max KB |
|---|---:|---:|---:|---:|---:|
| BC7 + zstd 7 (today's packer) | 782.5 | 1.00 | 219.4 | 3025.3 | 44644.6 |
| RGBA8 + zstd 7 | 1012.9 | 1.29 | 287.7 | 4034.9 | 57830.5 |
| RGBA8 + zstd 19 | 809.3 | 1.03 | 241.7 | 3063.6 | 48175.7 |
| indexed, per frame + zstd 9 | 863.9 | 1.10 | 278.4 | 3395.0 | 44091.8 |
| indexed, per frame + zstd 19 | 834.2 | 1.07 | 268.6 | 3266.2 | 42769.4 |
| indexed, batch + zstd 3 | 753.6 | 0.96 | 220.9 | 2731.5 | 43929.9 |
| indexed, batch + zstd 9 | 704.1 | 0.90 | 205.8 | 2572.5 | 41772.9 |
| **indexed, batch + zstd 19** | **657.6** | **0.84** | 192.0 | 2413.3 | 38879.2 |
| indexed, batch + zstd 19 + LDM | 653.1 | 0.83 | 192.2 | 2412.0 | 38099.2 |
| indexed + 8×8 block copy + zstd 9 | 708.2 | 0.91 | 208.7 | 2584.5 | 41889.9 |
| indexed + 8×8 block copy + zstd 19 | 661.2 | 0.85 | 193.7 | 2426.6 | 38989.6 |
| indexed + difference + zstd 9 | 925.1 | 1.18 | 271.0 | 3463.9 | 52845.9 |
| indexed + difference + zstd 19 | 893.8 | 1.14 | 261.6 | 3312.0 | 51061.8 |
| indexed, per-action batch + zstd 19 | 729.5 | 0.93 | 214.3 | 2787.0 | 40262.9 |
| BC7, per-action batch + zstd 7 | 874.5 | 1.12 | 266.1 | 3352.8 | 44819.4 |

Palettes: 101,703 are stored (52.1 MB), but only 38,616 are distinct. As RGB555 they take 19.8 MB, or 39.5 MB as
256-entry RGBA8 rows. A body uses a median of 2 distinct palettes, at most 175.

### Enhanced

| variant | total MB | vs BC7 | per body median KB | p95 KB | max KB |
|---|---:|---:|---:|---:|---:|
| BC7 + zstd 7 (today's packer) | 1823.3 | 1.00 | 841.2 | 7069.2 | 62510.7 |
| RGBA8 + zstd 7 | 2832.2 | 1.55 | 1529.3 | 10459.6 | 74244.4 |
| RGBA8 + zstd 19 | 2328.9 | 1.28 | 1223.9 | 8765.5 | 60083.4 |
| indexed, per frame + zstd 9 | 2505.1 | 1.37 | 1332.2 | 8991.1 | 64971.8 |
| indexed, per frame + zstd 19 | 2429.4 | 1.33 | 1301.2 | 8600.4 | 61370.1 |
| indexed, batch + zstd 3 | 2299.2 | 1.26 | 1124.4 | 8730.2 | 66779.1 |
| indexed, batch + zstd 9 | 2149.6 | 1.18 | 1063.3 | 8068.4 | 60384.7 |
| indexed, batch + zstd 19 | 2031.2 | 1.11 | 1009.2 | 7701.4 | 54434.4 |
| indexed, batch + zstd 19 + LDM | 2029.7 | 1.11 | 1009.0 | 7700.7 | 54449.5 |
| **indexed, batch planar + zstd 19** | **1921.3** | **1.05** | 964.1 | 7286.1 | 52990.1 |
| indexed + 8×8 block copy + zstd 9 | 2157.5 | 1.18 | 1067.6 | 8097.8 | 60481.0 |
| indexed + 8×8 block copy + zstd 19 | 2038.3 | 1.12 | 1021.2 | 7721.2 | 54524.8 |
| indexed + difference + zstd 9 | 2894.8 | 1.59 | 1373.6 | 11600.8 | 82240.6 |
| indexed + difference + zstd 19 | 2821.5 | 1.55 | 1328.2 | 11296.9 | 78579.2 |
| index plane only + zstd 19 (coverage dropped: lower bound) | 1584.1 | 0.87 | 727.6 | 6067.4 | 49594.8 |
| indexed, per-action batch + zstd 19 (interleaved) | 2037.9 | 1.12 | 1017.7 | 7705.3 | 54429.8 |
| BC7, per-action batch + zstd 7 | 1903.5 | 1.04 | 910.8 | 7213.3 | 62679.2 |

The Enhanced rows are interleaved unless they say planar. The planar split saves 5 %. The coverage plane alone costs
337 MB compressed (planar minus index plane only).

Palettes: 31,877 are stored, of which 31,629 are distinct (29.5 MB, or 32.4 MB as 256-entry RGBA8 rows). A body uses
a median of 43 distinct palettes, at most 67.

### Per-action batches

Splitting each body into one zstd stream per action costs disk space, mostly for Classic, whose actions are small:

| | whole body | per action | cost |
|---|---:|---:|---:|
| Classic indexed zstd 19 | 657.6 MB | 729.5 MB | +10.9 % |
| Enhanced indexed zstd 19 (interleaved) | 2031.2 MB | 2037.9 MB | +0.3 % |
| Classic BC7 zstd 7 | 782.5 MB | 874.5 MB | +11.8 % |
| Enhanced BC7 zstd 7 | 1823.3 MB | 1903.5 MB | +4.4 % |

### Resident bytes (pixels only, trimmed frames, no atlas waste)

| format | Classic total MB | Classic per body median / p95 / max KB | Enhanced total MB | Enhanced per body median / p95 / max KB |
|---|---:|---|---:|---|
| R8 | 2419.0 | 642 / 10165 / 140753 | 4916.9 | 2190 / 20754 / 139645 |
| RG8 | 4837.9 | 1285 / 20331 / 281506 | 9833.8 | 4381 / 41509 / 279291 |
| RGBA8 | 9675.9 | 2570 / 40661 / 563012 | 19667.6 | 8761 / 83017 / 558581 |
| BC7 | 2517.5 | 710 / 10412 / 142101 | 5106.4 | 2415 / 21138 / 141049 |

Classic needs R8; Enhanced needs RG8 unless coverage is dropped.

The largest bodies are Classic `AnimationFrame1.uop` body 197 (1380 frames, 140.8 Mpx) and Enhanced body 1288
(1600 frames, 139.6 Mpx). As one whole-body batch, body 1288 occupies 279 MB in RG8.

## 2. Load cost per body batch

This uses 200 bodies per client, spread by pixel count, on one thread. **Boost off, EPP power, quiet machine**
(see [Timing conditions](#timing-conditions)). MB/s counts the decoded output.

### Classic (boost off, EPP power)

| step | median µs | p95 µs | max µs | MB/s |
|---|---:|---:|---:|---:|
| BC7 + zstd 7 (zstd only) | 667 | 8727 | 145899 | 1051 |
| RGBA8 + zstd 7 | 1927 | 40137 | 468413 | 1144 |
| indexed, per frame + zstd 9 | 2913 | 12646 | 119089 | 526 |
| indexed, batch + zstd 9 | 685 | 8866 | 163688 | 921 |
| indexed, batch + zstd 19 | 914 | 10642 | 197394 | 746 |
| indexed, batch + zstd 19 + LDM | 898 | 10698 | 217233 | 725 |
| indexed + block copy + zstd 19 (zstd + rebuild) | 1572 | 29480 | 569309 | 295 |
| indexed + difference + zstd 19 (zstd + rebuild) | 2363 | 30766 | 530414 | 273 |
| indexed batch → RGBA8 expand (no zstd) | 471 | 5792 | 69223 | 6468 |
| runtime BC7 (Analytical): expand + encode | 40373 | 398903 | 4185815 | 22.1 Mpx/s |
| runtime BC7 (AnalyticalWide): expand + encode | 39331 | 383064 | 3975973 | 23.0 Mpx/s |

### Enhanced (boost off, EPP power)

| step | median µs | p95 µs | max µs | MB/s |
|---|---:|---:|---:|---:|
| BC7 + zstd 7 (zstd only) | 2513 | 18739 | 165740 | 1040 |
| RGBA8 + zstd 7 | 7226 | 87436 | 592213 | 1058 |
| indexed, per frame + zstd 9 | 11337 | 39036 | 221637 | 698 |
| indexed, batch + zstd 9 | 4118 | 34765 | 351436 | 1000 |
| indexed, batch + zstd 19 | 5048 | 39508 | 363321 | 879 |
| indexed, batch + zstd 19 + LDM | 5040 | 40203 | 386514 | 864 |
| indexed + block copy + zstd 19 (zstd + rebuild) | 9073 | 95298 | 865798 | 409 |
| indexed + difference + zstd 19 (zstd + rebuild) | 11271 | 118302 | 938334 | 344 |
| indexed batch → RGBA8 expand (no zstd) | 4986 | 33064 | 199554 | 2183 |
| runtime BC7 (Analytical): expand + encode | 130485 | 717982 | 3908371 | 22.1 Mpx/s |
| runtime BC7 (AnalyticalWide): expand + encode | 126059 | 683008 | 3689146 | 22.9 Mpx/s |

Only the interleaved layout was timed. Planar is one zstd stream of about the same size, so it should decode at
about the same speed (inferred, not measured).

### Load unit: whole body or one action (boost off, EPP power)

The first frame is drawable once the batch that holds it is decoded. For a per-action batch, that is the body's
lowest action.

| client | unit | stored as | compressed MB (sample) | per batch decode median / p95 µs | first frame drawable median / p95 / max µs |
|---|---|---|---:|---|---|
| Classic | body | indexed zstd 19 | 134.5 | 914 / 10642 | 914 / 10642 / 197394 |
| Classic | action | indexed zstd 19 | 148.2 | 18 / 382 | 34 / 1039 / 6110 |
| Classic | body | BC7 zstd 7 | 160.6 | 667 / 8727 | 667 / 8727 / 145899 |
| Classic | action | BC7 zstd 7 | 178.0 | 17 / 251 | 37 / 756 / 4109 |
| Enhanced | body | indexed zstd 19 | 440.0 | 5048 / 39508 | 5048 / 39508 / 363321 |
| Enhanced | action | indexed zstd 19 | 441.8 | 54 / 1229 | 96 / 2554 / 9861 |
| Enhanced | body | BC7 zstd 7 | 387.2 | 2513 / 18739 | 2513 / 18739 / 165740 |
| Enhanced | action | BC7 zstd 7 | 405.2 | 36 / 490 | 54 / 1144 / 4515 |

These are decode times only. A whole-body batch also has to be uploaded before its first frame shows: up to
279 MB (RG8) for the largest Enhanced body, and 41.5 MB at p95.

### Baseline: map and statics chunks in the same state

`map_statics_retrieval` was re-run right after the timing pass, in the same state: boost off, EPP power, load
average 0.74 / 3.20 / 5.45 at the start and 2.34 / 3.02 / 5.02 at the end. Times are warm, per chunk, read through
`UddpReader` from an mmap. The decoded sizes come from the stored variant: 117.9 MB for 28,673 map chunks
(4.1 KB each) and 23.7 MB for 28,672 statics chunks (0.8 KB each).

| package | codec | size MB | sequential µs | random µs | ≈ MB/s decoded (sequential) |
|---|---|---:|---:|---:|---:|
| map0 | installed (size of zstd 19) | 15.94 | 8.67 | 9.03 | 474 |
| map0 | zstd 7 | 17.74 | 8.25 | 8.63 | 498 |
| map0 | lz4 | 58.35 | 5.30 | 5.72 | 776 |
| statics0 | installed (size of zstd 7) | 8.44 | 3.28 | 3.34 | 252 |
| statics0 | zstd 19 | 8.15 | 3.57 | 3.70 | 232 |
| statics0 | lz4 | 16.96 | 0.87 | 0.90 | 950 |

The 4 KB map chunks decode at about 500 MB/s, and the 1 KB statics chunks at about 250 MB/s. The larger
animation batches reach 750–880 MB/s at zstd 19. So an animation batch costs no more per byte than the map
streaming already does, and less for whole-body batches.

### What the official client does, and what the design needs

From [the UOSA loading study](../../../scripts/dev-helpers/ec-research/ghidra-ec-anim-loading/results/uosa_anim_loading.md)
(commit f8c14c5c), the EC client stutters because all of this work is synchronous and on the main thread:

- **Whole-entry inflate.** Each new (body, action) inflates its whole UOP entry with zlib inside the frame. With boost
  off this takes a median of 0.1–4 ms, a p95 of 1–12 ms and at most 57 ms, plus disk time. A mobile entering view
  needs one entry per layer.
- **Weapon probe.** Equipping a weapon fully loads actions 9–14 just to test whether they exist.
- **Per-composite D3D9 texture.** Every new composite frame gets its own CPU RLE decode plus a CreateTexture/LockRect.
- **Small LRU.** A 26 MB composite LRU thrashes in crowds.

So the design needs:

- decoding off the main thread;
- existence answered from an index, never from data;
- long-lived atlas pages with hue in the shader, never per-frame textures.

The indexed formats provide the last point directly: hue is a palette swap, and R8/RG8 pages need no compositing.
With decoding off the main thread, a large unit costs latency (pop-in), not frame time. A whole Enhanced body is
40 ms at p95 and 363 ms at most to its first frame, against 2.6 ms and 10 ms for one action.

## 3. Runtime BC7

The test encodes BC7 from the indexed batch with our encoder at its fastest settings: no RDO, both front-ends. The
encoders have no other speed setting. It takes about **22–23 Mpx/s on one thread** (boost off, EPP power): a median
of 39 ms per Classic body and 126 ms per Enhanced body, with a maximum of 4.0 s / 3.7 s. That is 32× (Classic) and
19× (Enhanced) the batch zstd 19 decode (the benchmark's ratio over the sample totals; the medians give 43× and
25×). The expand to RGBA8 alone is 0.5 ms / 5.0 ms at the median.

Is it viable while streaming?

- **Not on the first-draw path.** One mid-size Enhanced body would stall the frames that wait for it by 100+ ms.
- **Feasible as a background step**: draw the indexed pages first, then re-encode and swap them in once they are
  warm. Spread over 8 worker threads, a crowd of 20 median Enhanced bodies (44 Mpx) takes about 0.25 s.
- **It only pays off for Enhanced.** For Classic, BC7 is no smaller than R8 (2.52 vs 2.42 GB), so runtime BC7 gains
  nothing. For Enhanced it halves RG8.
- **BC7 bakes one palette.** A runtime BC7 page loses hue-in-shader for its hueable pixels, unless those pixels stay
  indexed or each hue is encoded separately.

**GPU compute BC7: not worth trying now.** Published compute encoders are one to two orders of magnitude faster
than 23 Mpx/s per thread, but the only gain on the table is halving Enhanced VRAM. The cheaper option is not
measured: an R8 index page plus 4-bit coverage packed two pixels per byte in a half-width R8 page, which comes to
12 bpp with no codec. RGBA art (custom 32-bit art, colour-making upscales) is already handled by BC7 at pack time.
Reconsider a GPU encoder only if Enhanced VRAM becomes the binding limit in practice.

## 4. Enhanced palettes and coverage

- **Palette colours per entry:** median 256, p95 256, max 256. 86.1 % of entries use all 256 colours.
- **Distinct colours of a whole body:** median 3784, p95 10530, max 16141. Only 5.7 % of bodies fit 256.
- **Partial coverage:** 19.03 % of the 1.84 G coded pixels; per body median 32.2 %, p95 81.7 %, max 100 %.
- **8 bpp with (index, coverage) pairs as the palette, index 0 transparent:**
  - Every entry of the body fits 255 pairs in only 0.9 % of bodies.
  - One palette for the whole body fits in 0.6 % (distinct pairs per body: median 38,860, p95 86,233, max 151,280).

**Enhanced can't fit 8 bits per pixel without loss.** Dropping coverage, the lower bound, saves 337 MB on disk and
halves VRAM, but loses the 16-level anti-aliased edges, which are 19 % of pixels.

## 5. Upscale before conversion (2×, 25 bodies per client)

The 25 bodies per client are spread by pixel count. The 1× baselines are:

- **Classic:** indexed batch zstd 19 13.9 MB, BC7 zstd 7 16.2 MB, RGBA8 zstd 19 17.3 MB.
- **Enhanced:** indexed 49.9 MB, BC7 45.6 MB, RGBA8 57.6 MB.

The colour-making filters' requantized variant (i) uses the existing median cut: 256 colours per animation entry,
with the hueable/keep-colour split. Its SSIMULACRA2 is measured against the unquantized upscale, on mosaics of up to
64 frames over grey 128.

### Classic

| filter | stored as | MB | × 1× indexed | × 1× same format | quality |
|---|---|---:|---:|---:|---|
| nearest | indexed, batch zstd 19 | 18.7 | 1.34 | 1.34 | exact |
| EPX (Scale2x) | indexed, batch zstd 19 | 25.3 | 1.81 | 1.81 | exact |
| MMPX | indexed, batch zstd 19 | 20.6 | 1.48 | 1.48 | exact |
| xBRZ | (i) requantized RG8 zstd 19 | 50.5 | 3.62 | | SSIMULACRA2 median 93.3, p5 88.4, min 74.5 |
| xBRZ | (ii) RGBA8 zstd 19 | 75.3 | 5.40 | 4.36 | lossless |
| xBRZ | (iii) BC7 zstd 7 | 55.4 | 3.97 | 3.42 | BC7 |
| hqx | (i) requantized RG8 zstd 19 | 52.2 | 3.74 | | SSIMULACRA2 median 93.5, p5 87.7, min 72.5 |
| hqx | (ii) RGBA8 zstd 19 | 96.2 | 6.89 | 5.56 | lossless |
| hqx | (iii) BC7 zstd 7 | 59.0 | 4.23 | 3.63 | BC7 |

### Enhanced

| filter | stored as | MB | × 1× indexed | × 1× same format | quality |
|---|---|---:|---:|---:|---|
| nearest | indexed, batch zstd 19 | 64.4 | 1.29 | 1.29 | exact |
| EPX (Scale2x) | indexed, batch zstd 19 | 82.3 | 1.65 | 1.65 | exact |
| MMPX | indexed, batch zstd 19 | 68.9 | 1.38 | 1.38 | exact |
| xBRZ | (i) requantized RG8 zstd 19 | 142.5 | 2.85 | | SSIMULACRA2 median 92.1, p5 84.5, min 65.0 |
| xBRZ | (ii) RGBA8 zstd 19 | 253.4 | 5.08 | 4.40 | lossless |
| xBRZ | (iii) BC7 zstd 7 | 154.3 | 3.09 | 3.38 | BC7 |
| hqx | (i) requantized RG8 zstd 19 | 147.3 | 2.95 | | SSIMULACRA2 median 91.3, p5 84.7, min 64.5 |
| hqx | (ii) RGBA8 zstd 19 | 357.2 | 7.16 | 6.21 | lossless |
| hqx | (iii) BC7 zstd 7 | 163.3 | 3.27 | 3.58 | BC7 |

What the tables show:

- **Palette-preserving filters stay exactly indexed** (0 pixels off the palette). Storing nearest is pointless: the
  shader can do it for free.
- **Block copy doesn't help upscaled frames either:** it is 0.5–1.5 % larger than the batch.
- **Requantized colour filters cost about what BC7 does** (2.9–3.7× against 3.1–4.2×), keep the indexed pipeline and
  hue in the shader, and score 91–93.5 at the median.
- **The worst bodies drop to 65–75:** Classic `anim5.mul` body 20 under xBRZ, Enhanced body 273. Their gradients don't
  fit 256 colours. The BC7 error was not scored.
- **xBRZ and hqx create partial coverage.** The requantized upscales have partial coverage on a median 8.5 % (xBRZ)
  to 16 % (hqx) of Classic pixels, so an upscaled Classic body needs RG8, not R8.

## 6. Motion between neighbouring frames

This uses the same 25 bodies per client. Each non-empty 8×8 block after a direction's first frame is matched against
the previous frame, aligned on the anchor, searching ±16 px. Shifts are tried in order of length.

| | blocks | same place | exact shift | no exact match | shift length median / p95 |
|---|---:|---:|---:|---:|---|
| Classic, all 64 pixels | 452,241 | 1.4 % | 1.7 % | 97.0 % | 4.2 / 15.8 |
| Classic, opaque pixels only | | 1.6 % | 3.9 % | 94.5 % | 5.0 / 15.7 |
| Enhanced, all 64 pixels | 1,102,615 | 0.6 % | 0.3 % | 99.2 % | 6.1 / 16.6 |
| Enhanced, opaque pixels only | | 0.6 % | 0.6 % | 98.8 % | 7.0 / 17.1 |

Shift lengths are spread out. The Chebyshev distance of the shifted blocks:

- **Classic:** 1: 20.7 %, 2: 15.2 %, 3–4: 20.0 %, 5–8: 22.7 %, 9–16: 21.4 %.
- **Enhanced:** 1: 10.9 %, 2: 11.1 %, 3–4: 21.5 %, 5–8: 26.3 %, 9–16: 30.3 %.

Neighbouring frames are redrawn, not translated, so exact block motion is no basis for keyframe interpolation. An
interpolator would need approximate, optical-flow-style matching, and its output would be new colours (RGBA, or
requantized).

## 7. Can the animations shrink more? (`shrink` pass, 2026-11-02)

The benchmark's `shrink` pass (`mobile_anim_codec/shrink*`) answers the user's question "can't we shrink them more?",
and later "can what we know about the data help?". It changes no format. Every lossless variant is decoded again and
compared with the original pixels: a failure would stop the run, and none did.

```sh
MOBILE_ANIM_CODEC_PASSES=shrink MOBILE_ANIM_CODEC_SHRINK_PARTS=b,k2 MOBILE_ANIM_CODEC_SHRINK_STRIDE=10 \
  UO_CC_CLIENT_DIR=<Classic client> UO_EC_CLIENT_DIR=<Enhanced client> \
  cargo test --release -p udd-conv --test integration mobile_anim_codec:: -- --ignored --nocapture
```

- `MOBILE_ANIM_CODEC_SHRINK_PARTS` picks the parts: `a1,a2,a2p,b,w,c,c7,d,e,f,g,k,k1..k8,r,s1,s3,s4,s4q,s5,s6`, and `t0,t1,tt,tc,tu,ts,t9,tk0,tk` (sections 9 and 10).
- `MOBILE_ANIM_CODEC_SHRINK_ONLY` keeps candidates by name; `MOBILE_ANIM_CODEC_SHRINK_TIME=1` times them.
- "Every 10th body" means bodies sorted by pixel count: 103 Classic bodies (250 Mpx, 78.6 MB of BC7 + zstd 7) and 93
  Enhanced bodies (480 Mpx, 175.3 MB). Those sample ratios are only as good as the spread by size. `w` rows are the
  whole set.
- **Crowd options:** `MOBILE_ANIM_CODEC_SHRINK_CROWD_SCENARIOS` (`cc-uniform,cc-worst,ec-uniform,ec-worst`),
  `MOBILE_ANIM_CODEC_SHRINK_CROWD_NO_BC7`, and `MOBILE_ANIM_CODEC_SHRINK_QUALITY_CLIENT` for one client of `s4q`.
- **Memory:** `systemd-run` isn't usable here, so each job ran under `ulimit -v` at 18 GiB with a watchdog on its
  summed RSS. The last chain, approved by the user, used 22 GB virtual with an 18 GB resident watchdog. Allocation
  failures at 15 GiB resident stopped the `timing` pass twice and the `s1` Enhanced worst case once; section 8 says what
  became of them. Two parallel runs had been killed earlier for low memory, and after that jobs ran strictly one at a
  time.
- **Timings:** section 7's decode figures are marked where they're from a contended machine. Quiet figures are in
  [section 8](#8-the-crowd-decode-speed-first).

**Decided by the user:** no runtime BC7 for now, possibly later as an option for upscaled animations.

### Baselines: what "today" means

| | Classic | Enhanced |
|---|---:|---:|
| Benchmark's BC7 + zstd 7, no atlas (the "× BC7" of all tables) | 782.5 MB | 1823.3 MB |
| A real pack with today's packer, `--bc7 --zstd` ("× real pack") | 856.6 MB | 1987.9 MB |
| The installed package | 985.6 MB (30 Jun) | 6904.5 MB (30 Oct) |
| The client's files ("× shipped") | 1235.0 MB | 1984.5 MB |

- **The 6904 MB package is an old pack, not today's packer.** Its file table holds 2522 entries and 7682.9 MB
  uncompressed. 2291 entries are stored raw and only 225 with zstd, so most BC7 pages were never compressed. 73 % of
  the used page area holds no painted pixel, but only 26.5 % is outside the padded frames; section 11 reconciles the
  two. The real pack above has the same 7682.9 MB of pages and hue masks, all with zstd, and it passed
  `udd-cli tool validate`.
- **The benchmark's BC7 baseline is about 8–9 % under a real pack,** because it leaves out atlas pages and gutters. To
  read any × BC7 here as × real pack, multiply by 0.913 for Classic and 0.917 for Enhanced. The real packs were measured
  on 2026-11-01 and have since been deleted; the figures stand.
- **Earlier sections are corrected:** section 1 used to call 857 MB and 1.99 GB "installed". They are the real packs.
- **The shipped totals** cover `AnimationFrame*.uop`, plus for Classic `anim*.mul` (695.7 MB), `animdata.mul` (4.5 MB)
  and `animinfo.mul`. `animdata.mul` is static tile animation data rather than mobile frames, and is negligible. The
  `anim*.idx` files (6.1 MB) are left out.
- **174.7 MB of the shipped MUL files is unreferenced:** the `anim*.idx` files reference only 521.0 of the 695.7 MB of
  `anim*.mul`. All of it parses as complete animation entries, with no bytes left over; see "Orphaned art" in
  [the format requirements](../uddp_anim_format_requirements.md). So comparisons with shipped Classic use the 1060.3 MB
  the benchmark reads too. Classic's 677.4 MB is 0.64× of that, and 0.55× of the 1235.0 MB.

**How much boost off costs:** the same map chunk decodes in 3.4 µs with boost on and 8.67 µs with boost off, EPP power.
That is about 2.55×, a rough factor from one operation only.

### The floor: bits per opaque pixel

Whole set, index plane, zstd 19 per body: **Classic 5.66 bits, Enhanced 6.89 bits** (of 8). 929.9 of 2419 Mpx are
opaque in Classic and 1840.2 of 4917 Mpx in Enhanced. Over the every-10th-body sample the strongest lossless coders
(rank coding, block motion with tANS) end at 5.4–5.7 bits. Section 8 says why (s4).

### A. Redundancy that isn't being used

| Test | Classic | Enhanced |
|---|---:|---:|
| Exact duplicate frames, in a body (estimated compressed saving) | 23.8 MB | 12.3 MB |
| Duplicate directions / actions / whole bodies | 18.1 / 17.5 / 0.3 MB | 12.2 / 12.2 / 5.9 MB |
| Recoloured copies (raw), all in one body | 4.0 / 2.6 / 4.2 MB | none |
| Duplicates across files | 0.0 | 0.0 |
| One solid zstd 19 stream with `--long=31` | ×0.990 | ×0.995 |
| The same at level 22 | ×0.985 | not run |
| Trained dictionary, 112 KiB / 1024 KiB | ×1.025 / ×1.012 | ×1.005 / ×1.003 |
| 5 largest bodies as raw reference for 98 others | ×1.000 | ×1.000 |
| zstd 22 with a large window, per body / per action | 0.848 / 0.940 × BC7 | 1.076 / 1.089 × BC7 |

So there is no redundancy across bodies to harvest, as the solid stream is the upper bound. Non-zero indices under
transparent pixels: 0. Duplicate-frame removal would give at most 3.6 % of Classic and 0.8 % of Enhanced.

### B. Making the image smooth for the coder (every 10th body, × BC7, with coverage or shape, no palettes)

| Candidate | Classic | Enhanced |
|---|---:|---:|
| zstd 19, body / action (today's proposal) | 0.856 / 0.941 | 1.078 / 1.091 |
| Luma-sorted palette, zstd 19 | 0.888 | 1.076 |
| Colour-path-sorted palette, zstd 19 | 0.891 | 1.077 |
| Luma, MED / left / up residual, zstd 19 | 0.934 / 0.936 / 0.946 | 0.982 / 1.002 / 1.004 |
| Original order, MED, rANS | 1.021 | 1.094 |
| Luma, MED, rANS with 2D contexts, tables per body | 0.922 | 0.926 |
| The same, colour-path order | 0.936 | 0.968 |
| The same, tables per action | 0.949 | 0.957 |
| With the previous frame as a context | 0.918 | 0.920 |
| With the previous frame as the prediction (unmoved) | 1.012 | 0.978 |
| Master palette per body, zstd 19 | 0.841 | 1.059 |

Sorting alone does nothing for LZ. Prediction alone costs Classic what zstd's long matches gave it. The earlier lead
("palette order is noise, and that's the largest lever") is about −14 % of Enhanced at best, and only with an entropy
coder that costs per-pixel decode.

### C. General-purpose and image-codec bounds (25 bodies, × zstd 19 per body, index only)

Decode speeds here come from a contended machine: read them as an order of magnitude.

| Codec | Classic | Enhanced | decode MB/s |
|---|---:|---:|---:|
| xz -9e | 0.938 | 0.970 | 56–60 |
| brotli 11, large window | 0.941 | 0.956 | about 130 |
| WebP lossless, original palette, per frame / strip | 1.209 / 1.057 | 1.142 / 0.973 | 69–107 |
| WebP lossless, sorted palette, per frame / strip | 1.149 / 1.010 | 1.117 / 0.969 | 99–109 |
| JPEG XL e7, original, frame / strip (12 bodies) | 1.478 / 1.263 | 1.357 / 1.245 | 5–11 |
| JPEG XL e7, sorted, frame / strip | 1.284 / 1.109 | 1.158 / 1.034 | 8–9 |
| JPEG XL e9, sorted strip | 1.090 | 1.001 | 5–10 |

A proper image codec doesn't beat zstd by much on palette indices, and it costs 10–100× in decode.

### D. Coverage (Enhanced)

| Variant (93 bodies) | MB | × zstd 19 |
|---|---:|---:|
| zstd 19 | 33.87 | 1 |
| Shape + partial values split, rANS | 24.40 | 0.720 |
| With neighbours in the context | 25.54 | 0.754 |
| With distance to the shape's edge | 23.32 | 0.689 |
| With the previous frame's coverage / edge and previous / also colour | 25.40 / 23.56 / 24.01 | 0.750 / 0.696 / 0.709 |

**Lossy** (25 bodies, `16 levels` is 8.40 MB zstd, 6.42 rANS): 8 levels 6.88 / 5.02 MB, SSIMULACRA2 94.3 median, 33.2
minimum; 4 levels 5.38 / 3.54 MB, 88.5 median, −25.9 minimum. The minimum shows a level cut can damage a body badly.

### E. Near-lossless index (LOSSY, a tier for the user to judge)

A pixel takes its left, upper or previous neighbour's index when the colour is within a threshold.

| | threshold | zstd 19 MB | path rANS MB | largest channel error | SSIMULACRA2 median / min |
|---|---:|---:|---:|---:|---:|
| Classic | lossless | 12.22 | 12.04 | 0 | 100 / 100 |
| Classic | 8 | 10.83 | 10.82 | 8 | 88.4 / 72.4 |
| Classic | 16 | 8.99 | 8.95 | 16 | 81.8 / 57.6 |
| Enhanced | lossless | 34.50 | 31.79 | 0 | 100 / 100 |
| Enhanced | 4 | 31.92 | 29.82 | 4 | 94.8 / 88.6 |
| Enhanced | 8 | 27.52 | 26.16 | 8 | 91.1 / 80.7 |

### F. Upscaled content, 2× (25 bodies: BC7 of xBRZ 55.4 / 154.3 MB, Classic / Enhanced)

| Content | Coder | Classic MB (× 2× BC7) | Enhanced MB (× 2× BC7) | lossless of the upscale? |
|---|---|---:|---:|---|
| MMPX indexed | zstd 19 per body | 20.6 (0.372) | 67.8 (0.439) | yes |
| MMPX indexed | zstd 19 per action | 22.8 (0.412) | 70.2 (0.455) | yes |
| MMPX indexed | luma, MED, rANS | 21.1 (0.381) | 69.6 (0.451) | yes |
| MMPX indexed | rank + previous, action | 22.6 (0.408) | 87.3 (0.566) | yes |
| xBRZ requantized | zstd 19 per body | 49.2 (0.887) | 135.6 (0.879) | no, 256 colours |
| xBRZ requantized | zstd 19 per action | 53.2 (0.961) | 139.4 (0.904) | no |
| xBRZ requantized | luma, MED, rANS | 47.1 (0.849) | 126.9 (0.822) | no |
| xBRZ requantized | rank + previous, action | 46.6 (0.840) | 120.1 (0.778) | no |
| xBRZ RGBA8 | YCoCg-R + MED + rANS | 96.1 (1.734) | 285.1 (1.848) | yes |
| xBRZ RGBA8 | WebP m6, per frame / strip | 84.3 / 74.1 (1.520 / 1.336) | 270.7 / 247.1 (1.755 / 1.602) | yes |
| xBRZ RGBA8 | JPEG XL e7, per frame | 98.6 (1.779) | 285.8 (1.852) | yes |

**The zero-storage option: store 1× plus a filter id, and upscale when an action is loaded.** The filters are
deterministic, so the output is the same as storing it. Whether it's fast enough depends on the filter; the timings are
in [the crowd section](#8-the-crowd-decode-speed-first).

Decoding those planes, on 8 bodies with quiet timings, one thread, MB/s, Classic / Enhanced: MMPX indexed zstd 19 per
action 1351 / 1713, with luma + MED + rANS 60 / 102, with rank coding 32 / 58; xBRZ requantized zstd 19 per action 1177
/ 1038; xBRZ RGBA8 through YCoCg-R + MED + rANS 91 / 75, WebP strips 318 / 244, JPEG XL 13 / 12.

### Knowledge of the data (`k`)

Bytes saved against the current best for that client. Items that do badly are kept here in a line, so nobody tries
them again.

| Item | Result | Decode cost | Load complexity |
|---|---|---|---|
| k1 Art a mobile's own body can't reach | Classic 85.0 of 657.6 MB of index data (91.1 with all expansions on); Enhanced about 3.3 MB. **See the warning below** | none | a converter option |
| k2a Rank from the left pixel, zstd 19 | Classic worse (70.72 vs 67.29 MB); Enhanced −4 % (148.65 vs 155.10) | a table per palette | medium |
| k2b Rank from a MED colour, rANS | Classic 66.92 (−0.5 %); Enhanced 130.39 (−16 %) | 15 / 44 MB/s: see section 8 | a table per palette, and a colour lookup |
| k2c As b, plus the previous frame as context | Classic 66.63 (−1.0 %); Enhanced 129.67 (−16 %), the smallest without motion | high | as b |
| k2c As b, previous frame as reference | Classic 74.32; Enhanced 139.99: worse | | |
| k3 Stand action as a zstd reference | Classic 73.93 vs 73.91 per action: nothing. Enhanced 157.31 vs 155.14: worse | needs the stand action | high, and of no use |
| k3 Slots remapped to a body master palette | Classic body 66.06 (−1.8 %), action 73.66; Enhanced 151.82 / 155.13 (−2 % / 0) | none | a palette table per body |
| k3 Shared rANS tables per body | luma 69.09 / 137.97; path 70.21 / 145.21; master 70.07 / 148.27 | | |
| k4 Frame order by direction | Classic −0.8 % (z19); Enhanced +1.1 % (z19), −0.6 % (rANS) | none | none |
| k5 Better reference frames | Exact matches among opaque pixels: left 14–19 %, previous 12–14 %, best earlier 17–21 %, mirror 4–7 %. Not worth coding | | |
| k6 Coverage from its structure | ×0.956 of the current rANS coverage, about −10.8 MB (an estimate from 93 bodies) | 36 MB/s of coverage, as for the current rANS | an edge distance per frame |
| k7 Equipment layers | Their shape share is 0.5–2.4 % (Classic), 1.9–6.6 % (Enhanced); wearer as context ×0.999 / ×0.989 | | no value |

**k1, the client's rule, and a warning.** `BodyUsesUopFrames` (`0x005978a0` in `classic_7.0.103.0_client.exe`) is true
when the body's `mobtypes.txt` row has flag 0x10000. 300 bodies are flagged; all others go to a MUL through `Body.def`
and `Bodyconv.def`, and within `GetMobileFrame` there's no fallback. 27 flagged bodies have no UOP entry and draw
nothing.

**The 85 MB is "not reached by a mobile's own body", not "never drawn".** `GetMobileFrame` tests the object's field
+0x3E but requests the frames of its argument, and `FUN_005441e0` tests field +0x38 and then requests its own id
argument, so a flagged object can request an unflagged id. Body 1525 (`EQUIP_Tabbard_Gargoyle`, unflagged, has UOP
frames) is the likely case. `FUN_005b96a0` also requests actions 9–14 without the test, probably as preloading. The
callers' arguments aren't traced. Bodies 197 and 198 make up nearly all of the 85 MB. See section 7 of
[the format requirements](../uddp_anim_format_requirements.md).

| source | bodies | MB of index data | not reached, any expansion setting: bodies / MB | every expansion on: bodies / MB |
|---|---:|---:|---:|---:|
| `anim.mul` | 270 | 96.4 | 3 / 1.5 | 6 / 7.6 |
| `anim2.mul` | 157 | 63.4 | 80 / 4.8 | 80 / 4.8 |
| `anim3.mul` | 90 | 20.8 | 22 / 3.8 | 22 / 3.8 |
| `anim4.mul` | 138 | 29.2 | 2 / 0.0 | 2 / 0.0 |
| `anim5.mul` | 91 | 35.4 | 5 / 1.4 | 5 / 1.4 |
| `AnimationFrame1.uop` | 17 | 87.0 | 3 / 73.1 | 3 / 73.1 |
| `AnimationFrame2.uop` | 105 | 47.9 | 0 | 0 |
| `AnimationFrame3.uop` | 130 | 184.9 | 1 / 0.4 | 1 / 0.4 |
| `AnimationFrame4.uop` | 25 | 92.7 | 0 | 0 |

Enhanced: bodies 344 (2.22 + 0.55 MB of index and coverage) and 827 (0.43 + 0.23 MB) have no placeholder, and 90 % of
body 679 (0.56 + 0.21 MB) isn't shown, since its sequences show the first of each 10 frames.

### k8. Halved frames, rebuilt by interpolation (LOSSY, a tier for the user to judge)

93 Enhanced and 28 Classic UOP bodies. **What the client's timing implies:** of 5546 plain actions in 370 sequences,
96.0 % play at 9 fps, 2.5 % at 12 and 1.3 % at 6. Ten frames at 9 fps last 1.11 s (cycle median 1.11 s, p95 1.56 s, max
3.33 s), so a halved set really plays at about 5 fps.

- **It doesn't halve.** Actions of 4 frames or fewer stay whole (135 of 157,511 Enhanced frames), first and last frames
  stay, and effects protect 3468 Enhanced and 739 Classic UOP frames. That keeps 61.8 % of frames (58.5 % of pixels) in
  Enhanced and 60.8 % (56.9 %) in Classic UOP.
- **Size, × the full set:** zstd 19 ×0.605 and rank coding ×0.592 for Enhanced; ×0.564 and ×0.574 for Classic UOP.
  Stored vectors add 2.31 / 0.84 MB and keep masks almost nothing. Choosing per action the frames that interpolate
  worst is no better: ×0.611 / ×0.597, scoring 80.5 against 80.6.

| Rebuild, Enhanced, uniform | SSIMULACRA2 median | p5 | min | CPU per frame, median / p95 / max ms | per action |
|---|---:|---:|---:|---|---|
| Hold the previous frame (what a player would see) | 50.3 | −57.8 | −800.1 | none | none |
| Crossfade (off palette) | 52.0 | −50.5 | −493.2 | 0.05 / 1.69 / 2.50 | 0.5 / 67.8 / 91.0 |
| Block warp at load | 40.7 | −56.1 | −500.3 | 0.93 / 40.2 / 62.5 | 12.4 / 1688 / 2130 |
| Block warp, stored vectors | 80.6 | 29.1 | −63.2 | 0.01 / 0.17 / 0.29 | 0.1 / 6.4 / 9.4 |

- Stored vectors by action kind, Enhanced median: idle 91.8 (a plain hold scores 90.9), fidget 83.0, attack 77.0, run
  75.3, walk 73.8, impact 72.7, spell 70.0, die 57.4. By frame size: small 84.8, medium 52.7, large 39.6.
- Classic UOP medians: hold 52.5, crossfade 54.6, warp at load 45.6, stored 72.7 (p5 28.3); walk 63.9, idle 83.2, large
  frames 17.7.
- Worst scorers: Enhanced bodies 1248 and 105, Classic `AnimationFrame3` bodies 1248 and 1417.
- The block warp tears on limbs, by the coordinator's look at two sheets. Contact sheets, for the user:
  `uodynamapper-scratch/anim-codec/halved/` (real frame, each rebuild, a difference image).
- The action kinds come from `0xE2` type names as ClassicUO has them, so they are assumptions.

## 8. The crowd: decode speed first

Measured 2026-11-03 (`s1`–`s6`). The user moved the priority: the renderer must handle hundreds of different NPCs and actions on one screen, so decode
speed now matters more than size, and size is still very important. **The pass line is the user's: 300 new actions
drawable in about 250 ms on 4 threads.** Faster is better.

All timings in this section are on **boost off, EPP balance_power, platform profile performance**, the state TLP now
keeps. They don't compare in absolute terms with the EPP power tables above, so every run carries its own zstd 19.
Times leave out disk reads and GPU upload, and no run measured upload.

### The ranking

Candidates are per (action, direction) streams, sorted by the uniform crowd on 4 threads. "Facing" loads only the
direction each NPC is facing. "One stream" is one action stream with its tables on one thread (rank coding's is per
action and direction), standing in for the time to the first drawable frame. Decode MB/s is one thread, per action
for zstd 19, and otherwise per action and direction. Disk includes the palette table. It is estimated from the
every-10th-body sample, scaled by the measured whole-set zstd 19 row, so read it as within about 1 %. BC7's disk is the
measured per-action row. Shipped is 1060.3 MB for Classic, the referenced bytes, and 1984.5 MB for Enhanced.

**Classic**

| # | Candidate | Uniform crowd ms | Worst crowd ms | Facing only, uniform / worst ms | Decode MB/s, one thread | One stream, median / max ms | Disk MB | × BC7 | × real pack | × shipped | Lossless |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | Slot plane, LZ4 HC | **3.9** | **80.3** | 0.9 / 17.2 | 2776 | 0.00 / 3.77 | ≈ 931 | 1.19 | 1.09 | 0.88 | yes |
| 2 | BC7, zstd 7 (today, stored) | **9.6** | **183.8** | not run | not in this run | worst job 4.7 | 874.5 | 1.12 | 1.02 | 0.82 | no |
| 3 | Slot plane, zstd 19 | **14.7** | 277.5 | **3.1 / 58.3** | 769 | 0.02 / 11.15 | ≈ 761 | 0.97 | 0.89 | 0.72 | yes |
| 4 | Luma, MED, zstd 19 | **93.4** | 2186 | 21.1 / 448 | 98 | 0.09 / 100.89 | ≈ 828 | 1.06 | 0.97 | 0.78 | yes |
| 5 | Luma, MED, 10 streams, zstd 19 | **125.7** | 2873 | 27.6 / 593 | 70 | 0.12 / 136.99 | ≈ 840 | 1.07 | 0.98 | 0.79 | yes |
| 6 | Luma, MED, 10 streams, tANS | **146.8** | 3287 | 38.1 / 684 | 65 | 0.28 / 154.73 | ≈ 813 | 1.04 | 0.95 | 0.77 | yes |
| 7 | Luma, MED + block motion, tANS | **178.6** | 3846 | 47.1 / 805 | 55 | 0.38 / 195.38 | ≈ 784 | 1.00 | 0.92 | 0.74 | yes |
| 8 | Rank + previous frame, rANS (reference) | 579.5 | 9112 | 125.8 / 1876 | 12 | 0.68 / 100.94 | ≈ 712 | 0.91 | 0.83 | 0.67 | yes |

**Enhanced**

| # | Candidate | Uniform crowd ms | Worst crowd ms | Facing only, uniform / worst ms | Decode MB/s, one thread | One stream, median / max ms | Disk MB | × BC7 | × real pack | × shipped | Lossless |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | Slot + coverage planes, LZ4 HC | **9.7** | **174.2** | 2.2 / 35.3 | 3214 | 0.02 / 6.18 | ≈ 2446 | 1.34 | 1.23 | 1.23 | yes |
| 2 | BC7, zstd 7 (today, stored) | **15.3** | **231.1** | not run | not in this run | worst job 2.1 | 1903.5 | 1.04 | 0.96 | 0.96 | no |
| 3 | Slot + coverage planes, zstd 19 | **31.1** | 496.4 | **7.0 / 103.2** | 1021 | 0.07 / 16.91 | ≈ 2000 | 1.10 | 1.01 | 1.01 | yes |
| 4 | Luma, MED, zstd 19 | **127.9** | 2624 | 28.1 / 539 | 213 | 0.30 / 95.43 | ≈ 1823 | 1.00 | 0.92 | 0.92 | yes |
| 5 | Luma, MED, 10 streams, zstd 19 | **168.9** | 3451 | 36.8 / 707 | 157 | 0.40 / 130.76 | ≈ 1845 | 1.01 | 0.93 | 0.93 | yes |
| 6 | Luma, MED, 10 streams, tANS | **199.9** | 4005 | 53.6 / 825 | 141 | 0.59 / 149.33 | ≈ 1816 | 1.00 | 0.91 | 0.91 | yes |
| 7 | Luma, MED + block motion, tANS | **239.2** | 4732 | 65.0 / 973 | 118 | 0.79 / 186.18 | ≈ 1712 | 0.94 | 0.86 | 0.86 | yes |
| 8 | Rank + previous frame, rANS (reference) | 1010 | 11704 | 216.4 / 2381 | 24 | 1.79 / 106.30 | ≈ 1597 | 0.88 | 0.80 | 0.80 | yes |

Bold passes the 250 ms line.

- **On the uniform crowd every lossless candidate passes except rank coding.** Slot zstd 19 has 8–17× of margin; block
  motion has almost none on Enhanced.
- **On the worst crowd only LZ4 HC and stored BC7 pass with all five directions.** Slot zstd 19 passes when only the
  faced direction loads. That crowd is 918 and 1231 Mpx, up to 2.46 GB of VRAM, and upload would dominate it.
- **Prediction costs far more than it saves.** Luma + MED saves 9 % of Enhanced and adds 4×; on Classic it is both
  larger and 6× slower than plain zstd 19, because it gives up zstd's long matches.
- **Zstd 19 and LZ4 HC decode through the same plane, so they trade directly:** LZ4 HC is 3–4× faster and about 22 %
  larger on disk.

### s1. The crowd scenarios

Two crowds per client: 300 (body, action) pairs drawn uniformly from the 28,057 Classic and 31,877 Enhanced pairs that
exist, and the 300 largest pairs. "Uniform" means every existing pair is equally likely, which isn't how often a game
shows them; a real server's frequency hasn't been measured. Each cell is one thread / 4 threads, in ms until all are
drawable, with the 4-thread figure in bold when it passes. Action modes decode only the wanted actions, so "read" is
what the load reads. Tables are rebuilt per job, which is conservative; building them all once costs 6–61 ms.

**Classic, uniform:** 300 pairs of 251 bodies, 27.1 Mpx trimmed; a pair's Mpx: median 0.013, p95 0.32, max 5.14.

| Candidate | body | action | action + direction | facing direction only | worst job, action, ms | read for an action load, MB |
|---|---:|---:|---:|---:|---:|---:|
| BC7, zstd 7 | 515 / **168.0** | 30 / **9.6** | 36 / **9.6** | not run | 4.9 | 10.7 |
| slot plane, zstd 19 | 686 / 280.0 | 42 / **14.3** | 49 / **14.7** | 10 / **3.1** | 8.5 | 8.7 |
| slot plane, LZ4 HC | 224 / **77.5** | 12 / **3.7** | 12 / **3.9** | 2 / **0.9** | 2.2 | 10.7 |
| luma, MED, zstd 19 | 5028 / 1501.8 | 294 / **93.4** | 302 / **93.4** | 62 / **21.1** | 58.5 | 9.7 |
| luma, MED, 10 streams, zstd 19 | 6575 / 1948.6 | 391 / **122.7** | 409 / **125.7** | 85 / **27.6** | 76.7 | 9.7 |
| luma, MED, 10 streams, tANS | 7550 / 2222.3 | 492 / **146.2** | 500 / **146.8** | 143 / **38.1** | 89.1 | 9.8 |
| luma, MED + motion, tANS | 9027 / 2737.6 | 595 / **176.0** | 608 / **178.6** | 180 / **47.1** | 107.3 | 9.1 |
| rank + previous, rANS (reference) | 38882 / 10471.4 | 1978 / 499.7 | 2302 / 579.5 | 494 / **125.8** | 221.8 | 9.1 |

**Classic, worst (the 300 largest pairs):** 300 pairs of 56 bodies, 917.9 Mpx trimmed; a pair's Mpx: median 1.870, p95 8.04, max 12.00.

| Candidate | body | action | action + direction | facing direction only | worst job, action, ms | read for an action load, MB |
|---|---:|---:|---:|---:|---:|---:|
| BC7, zstd 7 | 1198 / 381.9 | 732 / **188.1** | 727 / **183.8** | not run | 8.9 | 300.2 |
| slot plane, zstd 19 | 1660 / 661.8 | 1079 / 285.9 | 1103 / 277.5 | 226 / **58.3** | 13.2 | 256.0 |
| slot plane, LZ4 HC | 604 / **209.4** | 319 / **83.8** | 319 / **80.3** | 65 / **17.2** | 3.8 | 312.1 |
| luma, MED, zstd 19 | 12537 / 3691.4 | 8700 / 2186.3 | 8730 / 2185.8 | 1792 / 448.4 | 103.0 | 278.4 |
| luma, MED, 10 streams, zstd 19 | 16193 / 4707.4 | 11447 / 2854.4 | 11455 / 2873.3 | 2350 / 592.6 | 137.3 | 271.8 |
| luma, MED, 10 streams, tANS | 18280 / 5337.8 | 13123 / 3279.4 | 13182 / 3287.2 | 2726 / 683.8 | 155.3 | 272.6 |
| luma, MED + motion, tANS | 21199 / 6179.7 | 15397 / 3844.3 | 15443 / 3845.9 | 3223 / 804.5 | 195.8 | 254.3 |
| rank + previous, rANS (reference) | 52110 / 14671.4 | 34084 / 8535.0 | 36404 / 9111.8 | 7488 / 1876.1 | 395.3 | 244.1 |

**Enhanced, uniform:** 300 pairs of 241 bodies, 49.9 Mpx trimmed; a pair's Mpx: median 0.037, p95 0.81, max 3.06.

| Candidate | body | action | action + direction | facing direction only | worst job, action, ms | read for an action load, MB |
|---|---:|---:|---:|---:|---:|---:|
| BC7, zstd 7 | 937 / 250.8 | 48 / **13.9** | 53 / **15.3** | not run | 2.2 | 18.0 |
| slot plane, zstd 19 | 1914 / 502.3 | 95 / **27.3** | 111 / **31.1** | 22 / **7.0** | 4.3 | 18.4 |
| slot plane, LZ4 HC | 683 / **188.5** | 31 / **9.9** | 32 / **9.7** | 7 / **2.2** | 1.6 | 22.3 |
| luma, MED, zstd 19 | 8617 / 2243.8 | 436 / **126.9** | 445 / **127.9** | 92 / **28.1** | 23.3 | 16.9 |
| luma, MED, 10 streams, zstd 19 | 10974 / 2843.1 | 563 / **162.7** | 589 / **168.9** | 122 / **36.8** | 31.1 | 16.9 |
| luma, MED, 10 streams, tANS | 12581 / 3261.3 | 687 / **197.1** | 700 / **199.9** | 186 / **53.6** | 35.7 | 17.1 |
| luma, MED + motion, tANS | 15209 / 3969.2 | 836 / **234.4** | 850 / **239.2** | 234 / **65.0** | 40.3 | 15.9 |
| rank + previous, rANS (reference) | 52361 / 13205.7 | 2314 / 642.8 | 3816 / 1010.2 | 808 / **216.4** | 97.0 | 15.5 |

**Enhanced, worst (the 300 largest pairs):** 300 pairs of 41 bodies, 1231.1 Mpx trimmed; a pair's Mpx: median 2.953, p95 8.94, max 11.28.

| Candidate | body | action | action + direction | facing direction only | worst job, action, ms | read for an action load, MB |
|---|---:|---:|---:|---:|---:|---:|
| BC7, zstd 7 | 1620 / 466.5 | 939 / **243.0** | 912 / **231.1** | not run | 9.7 | 407.3 |
| slot plane, zstd 19 | 3387 / 1033.6 | 1908 / 522.5 | 1970 / 496.4 | 399 / **103.2** | 20.6 | 404.4 |
| slot plane, LZ4 HC | 1500 / 426.0 | 642 / **180.5** | 644 / **174.2** | 139 / **35.3** | 6.3 | 487.0 |
| luma, MED, zstd 19 | 15378 / 3979.4 | 10356 / 2621.9 | 10410 / 2623.9 | 2125 / 539.2 | 107.6 | 394.0 |
| luma, MED, 10 streams, zstd 19 | 20013 / 5264.8 | 13687 / 3424.9 | 13746 / 3450.5 | 2809 / 706.5 | 151.8 | 388.4 |
| luma, MED, 10 streams, tANS | 22969 / 5984.6 | 15771 / 3979.4 | 15869 / 4004.6 | 3256 / 825.3 | 173.8 | 389.0 |
| luma, MED + motion, tANS | 26938 / 7214.1 | 18688 / 4751.8 | 18792 / 4731.6 | 3886 / 973.1 | 217.2 | 368.0 |
| rank + previous, rANS (reference) | 62247 / 15841.8 | 42198 / 10565.4 | 46558 / 11704.0 | 9482 / 2381.0 | 427.0 | 339.4 |

| VRAM at the end, MB, stored / as RGBA8 | body | action | facing only |
|---|---:|---:|---:|
| Classic, uniform | 502 / 1893 | 28.3 / 109 | 5.8 / 21.9 |
| Classic, worst | 1265 / 5048 | 919 / 3672 | 190 / 758 |
| Enhanced, uniform | 1929 / 3838 | 100 / 200 | 21.1 / 41.6 |
| Enhanced, worst | 3488 / 6974 | 2462 / 4924 | 504 / 1007 |

Those are the bytes to upload, too. BC7 holds about the same for Classic and half for Enhanced (uniform action 51.8 MB,
worst action 1245 MB). Pages and gutters aren't counted.

**Turning an NPC,** which loads its other four directions on one thread (median / p95 / max ms):

| Candidate | Classic uniform | Classic worst | Enhanced uniform | Enhanced worst |
|---|---:|---:|---:|---:|
| Slot plane, LZ4 HC | 0.01 / 0.1 / 1.6 | 0.51 / 2.2 / 3.0 | 0.03 / 0.4 / 1.2 | 1.27 / 4.1 / 5.2 |
| Slot plane, zstd 19 | 0.04 / 0.4 / 6.6 | 1.70 / 7.2 / 11.3 | 0.11 / 1.3 / 3.7 | 3.83 / 13.5 / 16.4 |
| Luma, MED, zstd 19 | 0.15 / 2.4 / 45.5 | 13.9 / 58.6 / 87.3 | 0.30 / 6.1 / 18.8 | 19.4 / 73.6 / 85.0 |
| Rank + previous, rANS | 3.12 / 17.1 / 182.9 | 61.9 / 228.6 / 349.7 | 6.56 / 36.5 / 92.8 | 93.7 / 311.1 / 347.2 |

If every NPC turned, the stored bytes would be 22.6, 729, 79.4 and 1959 MB, in the order of those columns.

### s2. The fast entropy path, and s3 block motion

Every 10th body, whole plane totals with shape or coverage, no palettes. The single thread decode is a whole sample.

| Candidate | Classic: index + shape MB | × BC7 | MB/s | One stream, median / max ms | Enhanced: index + coverage MB | × BC7 | MB/s | One stream, median / max ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| zstd 19, body | 67.29 + 0.00 | 0.856 | 771 | 0.81 / 118.47 | 155.10 + 33.87 | 1.078 | 998 | 5.29 / 183.76 |
| zstd 19, action | 73.91 + 0.00 | 0.941 | 769 | 0.02 / 11.15 | 155.14 + 36.04 | 1.091 | 1021 | 0.07 / 16.91 |
| slot plane, LZ4 HC, body | 88.17 + 0.00 | 1.122 | 2236 | 0.18 / 49.78 | 180.22 + 48.52 | 1.305 | 2704 | 1.66 / 88.08 |
| slot plane, LZ4 HC, action | 90.51 + 0.00 | 1.152 | 2830 | 0.00 / 3.77 | 180.79 + 51.10 | 1.323 | 3230 | 0.02 / 6.18 |
| luma, MED residual, zstd 19, action | 70.74 + 7.67 | 0.998 | 100 | 0.09 / 100.89 | 138.73 + 36.04 | 0.997 | 217 | 0.30 / 95.43 |
| luma, left residual, zstd 19, action | 71.38 + 7.67 | 1.006 | 161 | 0.07 / 63.09 | 142.23 + 36.04 | 1.017 | 361 | 0.20 / 56.99 |
| luma, MED, 4 streams, zstd 19, action | 69.67 + 7.67 | 0.984 | 75 | 0.11 / 137.22 | 137.77 + 36.04 | 0.992 | 164 | 0.38 / 131.64 |
| luma, MED, 10 streams, zstd 19, action | 69.74 + 7.67 | 0.985 | 74 | 0.12 / 136.99 | 137.90 + 36.04 | 0.992 | 164 | 0.40 / 130.76 |
| luma, MED, 10 streams, zstd 19, body | 65.16 + 7.06 | 0.919 | 75 | 8.48 / 1108.04 | 136.37 + 33.87 | 0.971 | 164 | 28.15 / 1069.03 |
| luma, left, 10 streams, zstd 19, action | 71.53 + 7.67 | 1.008 | 96 | 0.11 / 104.32 | 142.19 + 36.04 | 1.017 | 214 | 0.30 / 100.62 |
| slot, MED, 10 streams, zstd 19, action | 76.98 + 7.67 | 1.077 | 76 | 0.12 / 125.75 | 163.62 + 36.04 | 1.139 | 171 | 0.39 / 120.82 |
| luma, MED, 4 streams, tANS, action | 70.08 + 7.67 | 0.990 | 66 | 0.19 / 154.46 | 139.46 + 36.04 | 1.001 | 144 | 0.50 / 148.85 |
| luma, MED, 10 streams, tANS, action | 69.25 + 7.67 | 0.979 | 66 | 0.28 / 154.73 | 138.18 + 36.04 | 0.994 | 144 | 0.59 / 149.33 |
| luma, MED, 10 streams, tANS, body | 69.10 + 7.06 | 0.969 | 65 | 9.87 / 1246.44 | 138.00 + 33.87 | 0.980 | 143 | 32.30 / 1214.43 |
| luma, MED, 10 streams, tANS, action+direction | 69.75 + 8.10 | 0.991 | 65 | 0.18 / 42.54 | 138.80 + 36.77 | 1.002 | 141 | 0.21 / 40.41 |
| luma, left, 10 streams, tANS, action | 71.43 + 7.67 | 1.007 | 83 | 0.26 / 121.56 | 143.83 + 36.04 | 1.026 | 182 | 0.49 / 118.87 |
| slot, MED, 10 streams, tANS, action | 77.04 + 7.67 | 1.078 | 67 | 0.28 / 144.42 | 167.51 + 36.04 | 1.161 | 150 | 0.59 / 139.10 |
| luma, MED + block motion, zstd 19, action | 67.40 + 7.67 | 0.955 | 62 | 0.17 / 176.92 | 129.13 + 36.04 | 0.942 | 135 | 0.54 / 166.52 |
| luma, MED + block motion, tANS, action | 65.94 + 7.67 | 0.937 | 57 | 0.38 / 195.38 | 127.40 + 36.04 | 0.932 | 121 | 0.79 / 186.18 |
| slot plane, LZ4 HC, action+direction | 92.33 + 0.00 | 1.175 | 2776 | 0.00 / 0.89 | 183.53 + 54.13 | 1.356 | 3214 | 0.00 / 1.39 |
| luma, MED residual, zstd 19, action+direction | 71.31 + 8.10 | 1.011 | 98 | 0.02 / 25.41 | 139.55 + 36.77 | 1.006 | 213 | 0.05 / 24.73 |
| luma, MED, 10 streams, zstd 19, action+direction | 72.57 + 8.10 | 1.027 | 70 | 0.03 / 37.34 | 141.71 + 36.77 | 1.018 | 157 | 0.06 / 35.18 |
| luma, MED + block motion, tANS, body | 65.63 + 7.06 | 0.925 | 56 | 12.15 / 1424.62 | 127.02 + 33.87 | 0.918 | 121 | 40.26 / 1369.82 |
| luma, MED + block motion, tANS, action+direction | 66.85 + 8.10 | 0.954 | 55 | 0.25 / 54.79 | 128.58 + 36.77 | 0.943 | 118 | 0.29 / 51.81 |
| rank + previous, action+direction | 66.68 + 3.41 | 0.892 | 12 | 0.68 / 100.94 | 129.72 + 24.45 | 0.880 | 24 | 1.79 / 106.30 |

- **Table-driven coding doesn't reach zstd's speed once prediction is in the way.** The predictor runs per pixel in
  every one of these, and that cost is the difference: zstd over 10 split streams is slower than zstd over one, and
  the split gains under 1.5 % in size.
- **tANS is about the same size as zstd 19 on split streams** (0.979 against 0.985 Classic, 0.994 against 0.992
  Enhanced), and slower.
- **Slot order instead of luma order makes the index plane 10–21 % larger** and is barely faster, so once there's a
  predictor, sorting pays for itself.

**s3, by action kind:** a stored vector per block, with the moved previous frame as prediction (index plane, action
streams, MB). This is the untested form of "use the repetition". It's real but concentrated in idle: the vectors
themselves cost under 1 % and a third of blocks use one.

| Action kind | Classic Mpx | no motion | with motion | change | Enhanced Mpx | no motion | with motion | change |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| idle | 14.7 | 4.00 | 3.11 | −22.2 % | 53.3 | 16.30 | 12.31 | −24.5 % |
| fidget | 6.3 | 1.83 | 1.57 | −14.1 % | 47.3 | 13.79 | 12.39 | −10.2 % |
| walk | 15.6 | 4.06 | 3.76 | −7.3 % | 54.6 | 15.04 | 14.10 | −6.2 % |
| run | 3.9 | 1.08 | 1.03 | −4.6 % | 21.9 | 5.99 | 5.74 | −4.2 % |
| attack | 20.6 | 5.27 | 5.18 | −1.7 % | 72.3 | 22.04 | 21.31 | −3.3 % |
| die | 18.6 | 4.92 | 4.80 | −2.5 % | 60.5 | 16.68 | 16.21 | −2.8 % |
| spell | 17.1 | 4.08 | 3.98 | −2.6 % | 35.9 | 9.77 | 9.54 | −2.3 % |
| impact | 2.3 | 0.63 | 0.60 | −5.5 % | 19.1 | 5.40 | 5.20 | −3.8 % |
| other typed | 25.3 | 6.26 | 6.03 | −3.7 % | 86.8 | 24.58 | 23.32 | −5.1 % |
| no sequence | 125.5 | 37.01 | 35.75 | −3.4 % | 28.3 | 8.48 | 7.13 | −15.8 % |
| **all** | 249.9 | 69.14 | 65.80 | −4.8 % | 479.9 | 138.07 | 127.25 | −7.8 % |

The action kinds are the `0xE2` type names as ClassicUO has them, so they're assumptions. Applying the vectors takes
about 16 % off tANS's decode speed (65 → 55 and 141 → 118 MB/s per action and direction).

### Rank coding's decode, step by step

Measured with the pass's other steps taken out, per action, one thread, MB/s of output.

| Variant | Classic total / index MB | shape / palette tables / pixel loop ms | Whole ms | MB/s | Opaque Mpx/s in the loop | Enhanced total / index MB | shape / palette tables / pixel loop ms | Whole ms | MB/s | Opaque Mpx/s in the loop |
|---|---:|---|---:|---:|---:|---:|---|---:|---:|---:|
| Rank + previous | 70.01 / 66.64 | 2630 / 8867 / 6628 | 18132 | 13.8 | 14.7 | 154.09 / 129.68 | 5782 / 5620 / 12428 | 23837 | 40.3 | 14.7 |
| Rank from MED | 70.31 / 66.93 | 2623 / 8796 / 5630 | 17051 | 14.7 | 17.3 | 154.81 / 130.40 | 5784 / 5533 / 10542 | 21862 | 43.9 | 17.3 |
| Coarser 16³ colour lookup | 72.46 / 69.09 | 2634 / 4618 / 5485 | 12740 | 19.6 | 17.7 | 158.47 / 134.06 | 5782 / 2972 / 10313 | 19071 | 50.3 | 17.7 |

The cheaper path halves the palette tables but not the per-pixel loop, costs 2.8–3.2 % of the index plane and stays
at 20 / 50 MB/s. So a smaller table doesn't rescue it, and rank coding fails the crowd as a reference row only.

### s4. Why the lossless floor is about 5.5 bits

Every 10th body. **The hypothesis was per-frame palette noise on top of one surface. It's wrong.** A static pixel
is one solid in both frames, inside a static region.

| | Classic idle | Classic other | Enhanced idle | Enhanced other |
|---|---:|---:|---:|---:|
| Palettes per action (by content) | 3.41 (1.63) | | 1.00 (1.00) | |
| Used entries per palette, median | 88 | | 256 | |
| Median distance to the nearest other colour, largest channel | 8 | | 5 | |
| Opaque Mpx | 6.7 | 90.5 | 21.2 | 161.6 |
| On a static surface, of opaque | 24.1 % | 5.8 % | 25.1 % | 6.9 % |
| Static: same index in the next frame | 62.6 % | 59.3 % | 55.5 % | 48.4 % |
| Static: largest channel change of 1–2 | 0 | 0 | 0 | 0 |
| Change correlated with the same pixel's previous change (frame noise: about −0.5) | −0.116 | −0.229 | +0.052 | −0.036 |
| Change correlated with the next pixel (surface change: positive) | 0.179 | 0.329 | 0.251 | 0.377 |
| 2×2 checkers per 1000 solid pixels | 8.0 | 7.1 | 4.4 | 4.7 |
| Bits, MED with contexts, static pixels | 4.40 | 3.63 | 5.46 | 4.64 |
| Bits, from the previous frame's index, static pixels | 2.79 | 3.09 | 3.29 | 3.73 |
| Bits, MED with contexts, the others | 6.05 | 5.92 | 6.43 | 6.16 |
| Share of the index bits on static pixels | 18.8 % | 3.7 % | 22.1 % | 5.3 % |

- **The per-frame noise isn't there.** The anti-correlation is far from −0.5, changes cluster in space, and a static
  pixel that changes moves by at least 3 levels. Only 13–17 % of those land on the nearest palette entry.
- **Enhanced has one palette per action,** so a per-action shared palette has nothing to give it. Classic's 1.63
  distinct per action leaves little.
- **Static surfaces are too small a share to set the floor:** about 4–5 % of the bits in most actions and about 20 %
  in idle, which is where motion gains.
- **What's left is spatial.** The other pixels cost about 6 bits under a good predictor: fine detail quantised to a
  palette whose neighbouring entries are 5–8 levels apart, with one- and two-pixel ripples at 31–40 per 1000. This is a
  reading of those numbers, not a separate measurement, and rendered noise and shading would look the same.

**The lossy test (LOSSY, a tier for the user to judge):** near colours with the same hue flag and greyness merged within each palette ("merged") and static
surfaces made to repeat ("repeat"), at a colour error up to the threshold, 25 bodies, action streams, index only.
Threshold 2 changes nothing, nor does 4 for Classic, whose colours are 8 levels apart.

| Client | Variant | zstd 19 MB | tANS MB | Pixels changed | Worst channel error | SSIMULACRA2 median / p5 / min |
|---|---|---:|---:|---:|---:|---:|
| Classic | lossless | 15.23 | 14.06 | 0.0 % | 0 | 100.0 / 100.0 / 100.0 |
| Classic | repeat, up to 8 | 15.00 | 14.12 | 8.0 % | 8 | 94.0 / 86.4 / 76.5 |
| Classic | merged palette, up to 8 | 12.03 | 13.30 | 58.8 % | 8 | 89.2 / 77.4 / 59.1 |
| Classic | merged palette and repeat, up to 8 | 12.02 | 13.32 | 58.8 % | 8 | 89.3 / 77.1 / 58.6 |
| Classic | repeat, up to 16 | 14.45 | 14.23 | 16.2 % | 16 | 91.2 / 80.9 / 73.3 |
| Classic | merged palette, up to 16 | 10.12 | 12.37 | 76.1 % | 16 | 84.6 / 64.4 / 46.7 |
| Classic | merged palette and repeat, up to 16 | 10.07 | 12.42 | 76.1 % | 16 | 84.8 / 64.8 / 47.9 |
| Enhanced | lossless | 38.23 | 35.03 | 0.0 % | 0 | 100.0 / 100.0 / 100.0 |
| Enhanced | repeat, up to 4 | 37.92 | 35.14 | 4.1 % | 4 | 97.7 / 93.8 / 86.7 |
| Enhanced | merged palette, up to 4 | 35.43 | 34.42 | 20.6 % | 4 | 96.7 / 90.3 / 82.3 |
| Enhanced | merged palette and repeat, up to 4 | 35.40 | 34.46 | 20.6 % | 4 | 96.7 / 90.1 / 82.1 |
| Enhanced | repeat, up to 8 | 37.01 | 35.43 | 11.8 % | 8 | 95.9 / 89.1 / 76.1 |
| Enhanced | merged palette, up to 8 | 29.86 | 32.84 | 55.9 % | 8 | 91.9 / 81.0 / 73.1 |
| Enhanced | merged palette and repeat, up to 8 | 29.78 | 32.99 | 55.9 % | 8 | 91.9 / 80.9 / 70.3 |
| Enhanced | repeat, up to 16 | 34.80 | 36.03 | 23.1 % | 16 | 93.3 / 78.6 / 49.0 |
| Enhanced | merged palette, up to 16 | 22.92 | 29.67 | 81.4 % | 16 | 85.3 / 63.5 / 34.4 |
| Enhanced | merged palette and repeat, up to 16 | 22.84 | 30.04 | 81.4 % | 16 | 85.4 / 62.4 / 42.9 |

Merging colours gives the savings, mostly for zstd: −22 % of Enhanced's index plane at threshold 8, with 56 % of pixels
changed and SSIMULACRA2 91.9 / 81.0 / 73.1. Repeating static surfaces gives only 1–3 %, or 9 % at 16, which agrees with
the table above.

### s5. Classic UOP against Enhanced

"None across files" covered files within one client only, so this compared the same (body, action, direction, frame)
across clients: 277 bodies in both, 456,949 shared frames. **They aren't the same renders as stored, and neither can be
derived from the other.**

- The frame counts match, as measured before (1.003×), but same box and anchor: 0.0 % of shared frames; same shape:
  0.0 %.
- Pixels opaque in both, of those opaque in either: 35.2 %. Mean overlap per frame is 0.077 at the anchor and 0.106 at
  the best offset up to 4 px (better elsewhere in 48.2 % of frames).
- On pixels opaque in both: same RGB555 1.8 %, within one 5-bit step on every channel 12.1 %, mean largest error 7.54
  steps. Same index 0.4 %.
- Enhanced's coverage does predict Classic's shape a little: Classic draws 46.8 % of the pixels where Enhanced is
  solid, against 10.8 % where it's empty.
- Coded with the other as the reference, per body, zstd 19: Classic UOP 417.7 → 578.5 MB (+38.5 %); Enhanced 718.5 →
  803.3 MB (+11.8 %). The best body by colour match (483, 13.0 % same RGB555) still gets +25.6 %.

So it saves nothing with both packages installed, and nothing with one alone. A real shift over 4 px, or different
action or direction numbering between the two, would also give these numbers. The anchors were checked against both
readers, but that second cause wasn't ruled out, and the frame counts make it unlikely.

### s6. A real video codec (LOSSY, a bound)

AV1 4:4:4 per (action, direction), libaom `cpu-used` 6 through ffmpeg, 12 bodies per client spread by size, full range.
Hue flags and shape or coverage are separate lossless planes, and they're counted (0.78 MB Classic; 4.30 MB Enhanced).
Palettes aren't stored, since the output is colour. A video has to code the whole canvas, not the trimmed frame: × 1.57
pixels for Classic and × 1.88 for Enhanced. Decode is ffmpeg with libdav1d on one thread, with its 32 ms start-up taken
off each stream.

| Client | CRF | MB with planes | × zstd 19 action | × BC7 | SSIMULACRA2 median / p5 / min | Exact colour | On the palette | Mean / worst channel error | Hue-eligible grey stays grey | Hue-flagged exact | Canvas Mpx/s | Action median / p95 / max ms |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Classic | 10 | 10.20 | 1.338 | 1.504 | 91.3 / 83.7 / 78.5 | 1.8 % | 1.9 % | 3.87 / 54 | 63.1 % | 0.0 % | 9.9 | 4.4 / 30.3 / 62.9 |
| Classic | 20 | 6.66 | 0.874 | 0.982 | 87.6 / 77.1 / 71.7 | 0.8 % | 1.0 % | 6.39 / 112 | 60.6 % | 0.0 % | 11.4 | 4.2 / 23.8 / 49.8 |
| Classic | 30 | 4.60 | 0.604 | 0.679 | 84.1 / 71.2 / 62.0 | 0.4 % | 0.9 % | 9.15 / 129 | 57.5 % | 0.0 % | 12.5 | 4.6 / 18.0 / 41.4 |
| Classic | 40 | 3.03 | 0.398 | 0.447 | 79.3 / 61.5 / 48.9 | 0.2 % | 0.8 % | 13.46 / 181 | 49.5 % | 0.0 % | 13.8 | 4.1 / 15.4 / 30.8 |
| Classic | 50 | 2.10 | 0.275 | 0.310 | 73.9 / 50.8 / 19.8 | 0.1 % | 0.9 % | 18.79 / 193 | 50.2 % | 0.0 % | 17.0 | 3.5 / 14.0 / 26.2 |
| Enhanced | 10 | 23.20 | 1.023 | 1.106 | 92.8 / 88.0 / 77.0 | 1.1 % | 1.3 % | 3.89 / 107 | 81.1 % | 0.0 % | 16.0 | 10.3 / 52.8 / 60.2 |
| Enhanced | 20 | 16.03 | 0.707 | 0.764 | 90.0 / 81.7 / 69.6 | 0.5 % | 1.0 % | 6.18 / 129 | 79.3 % | 0.0 % | 19.4 | 8.9 / 40.4 / 45.2 |
| Enhanced | 30 | 11.86 | 0.523 | 0.565 | 86.4 / 74.4 / 61.0 | 0.3 % | 1.0 % | 8.85 / 170 | 78.0 % | 0.0 % | 23.8 | 8.9 / 28.9 / 33.4 |
| Enhanced | 40 | 8.68 | 0.383 | 0.414 | 80.9 / 63.6 / 49.6 | 0.2 % | 0.9 % | 12.99 / 211 | 77.1 % | 0.0 % | 29.3 | 7.3 / 24.5 / 27.0 |
| Enhanced | 50 | 6.95 | 0.307 | 0.331 | 75.3 / 50.4 / 31.1 | 0.1 % | 0.9 % | 17.87 / 245 | 77.4 % | 0.0 % | 34.9 | 6.7 / 17.1 / 23.7 |

By action kind, Enhanced median / p5 / min: at CRF 10, attack 92.7 / 88.4 / 80.4, die 92.3 / 82.3 / 78.9, idle 93.2 /
87.4 / 77.0, walk 92.6 / 82.0 / 78.8; at CRF 30, attack 86.9 / 75.5 / 61.9, die 83.4 / 68.9 / 61.0, idle 88.5 / 75.3 /
65.1, walk 85.9 / 70.2 / 62.3; at CRF 50, attack 76.6 / 51.8 / 33.9, die 65.9 / 45.2 / 32.7, idle 80.5 / 57.1 / 40.7,
walk 73.6 / 48.4 / 32.6. Classic actions with a sequence are 4–5 points under Enhanced's at CRF 10 and 10–12 at CRF
30.

**This is where decades of progress show up,** and it's the only large saving here: 0.57–0.68× BC7 at SSIMULACRA2 84–86. What it costs:

- **Colour fidelity:** under 2 % of pixels keep their exact colour at any level tried, and about 1 % land on a palette
  entry. So the palette and the index plane are gone. The worst single channel errors are 54–245.
- **Hue:** hue applies through the palette in the shader today, and there's no palette left. No hue-flagged pixel
  keeps its exact colour, and a fifth or more of grey pixels stop being grey, so a hued mobile can't be recoloured the same
  way. It would need a different hue method.
- **VRAM:** the output is colour, so it needs RGBA8 (4× Classic's R8, 2× Enhanced's RG8), or a requantise or a BC7 pass
  after decode, which adds loss and time.
- **Crowd speed:** estimated from these rates, not run through `s1`, at CRF 30 the uniform crowd takes 851 / 985 ms on
  4 threads for Classic / Enhanced, and 626 / 672 ms at CRF 50. **It fails the line,** the first drawable frame costs
  decoding from the start of the stream, and the worst crowd would take many seconds. A linked decoder would remove
  ffmpeg's start-up but not the per-pixel rates.

Worst scorers, for the user: `uodynamapper-scratch/anim-codec/av1/`. Each sheet shows the frame before, the real frame,
CRF 10–50 and a difference row. The worst are Enhanced bodies 250, 1469, 654 and 1421, and Classic `anim2.mul` bodies
22, 29 and 62 and `AnimationFrame3.uop` body 659.

### Upscaling when an action loads (item 13)

One thread, at 2×, deterministic, so the output equals what storing it would give. Times are for one action (median /
p95 / max ms). The last two columns are estimates for the uniform crowd on 4 threads, all directions / facing only,
in ms, from those rates with ideal scaling, added on top of decode.

| Filter | Classic Mpx/s of input | Classic action | Classic whole body max | Enhanced Mpx/s | Enhanced action | Enhanced whole body max | Classic crowd estimate | Enhanced crowd estimate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| EPX (Scale2x) | 65.4 | 0.3 / 5.0 / 28.0 | 241 | 66.7 | 0.6 / 11.7 / 52.8 | 477 | 104 / 21 | 187 / 39 |
| MMPX | 32.3 | 0.6 / 9.8 / 60.0 | 505 | 33.5 | 1.0 / 23.6 / 146.8 | 1028 | 210 / 42 | 372 / 78 |
| hqx | 13.4 | 1.2 / 25.8 / 303.4 | 1016 | 16.0 | 2.3 / 46.3 / 225.2 | 1926 | 506 / 102 | 780 / 162 |
| xBRZ | 5.1 | 3.5 / 69.5 / 282.4 | 2989 | 5.1 | 7.6 / 158.0 / 725.1 | 5716 | 1328 / 268 | 2446 / 510 |

So with the faced direction only, EPX, MMPX and hqx fit the line on the uniform crowd, on top of decode and with no
extra disk; xBRZ doesn't. Measured on 2× content (8 bodies, zstd 19 per action), stored MMPX indexed decodes at 1351 /
1713 MB/s, 6–10× the pixels per second of filtering, at 0.41 / 0.52 of xBRZ's BC7 at 2×. VRAM is 4× in every case.

### The largest bodies (G14–G16)

Zstd 19, one thread unless stated. "Kind" is the `mobtypes.txt` label, or its type only when it has no name. The rest
of the 20 per client is in `uodynamapper-scratch/anim-codec/e2_g.log` and repeats the pattern.

| Body | Kind | Mpx | Frames | Resident MB | MB body / action / action + direction | Whole body ms | 4 threads, action streams ms | Largest action ms | Largest action + direction ms |
|---|---|---:|---:|---:|---|---:|---:|---:|---:|
| AnimationFrame1.uop body 197 | MONSTER: Order Variant | 140.8 | 1380 | 140.8 | 33.64 / 34.76 / 35.26 | 203.6 | 44.8 | 12.1 | 2.7 |
| AnimationFrame1.uop body 198 | MONSTER: Chaos Variant | 133.4 | 1360 | 133.4 | 38.88 / 40.26 / 40.72 | 206.0 | 44.5 | 9.2 | 2.5 |
| AnimationFrame4.uop body 826 | MONSTER: Stygian Dragon | 121.6 | 1360 | 121.6 | 30.60 / 31.86 / 32.34 | 191.4 | 42.7 | 9.0 | 2.5 |
| AnimationFrame3.uop body 1288 | ANIMAL: Dragon_Turtle | 77.2 | 1600 | 77.2 | 26.14 / 26.97 / 27.16 | 173.8 | 36.3 | 8.4 | 2.0 |
| AnimationFrame3.uop body 1248 | MONSTER: Baddie 01 | 75.0 | 1010 | 75.0 | 19.14 / 19.42 / 19.54 | 120.6 | 28.3 | 11.1 | 3.2 |
| EC body 1288 | ANIMAL: Dragon_Turtle | 139.6 | 1600 | 279.3 | 52.99 / 53.01 / 53.24 | 404.1 | 86.0 | 20.7 | 4.0 |
| EC body 197 | MONSTER: Order Variant | 138.3 | 1380 | 276.7 | 46.67 / 46.85 / 47.18 | 294.0 | 57.4 | 15.8 | 3.0 |
| EC body 198 | MONSTER: Chaos Variant | 133.5 | 1360 | 267.0 | 50.21 / 50.36 / 50.60 | 285.8 | 56.7 | 12.5 | 2.9 |
| EC body 826 | MONSTER: Stygian Dragon | 133.5 | 1360 | 267.0 | 45.46 / 45.60 / 46.20 | 304.5 | 63.0 | 14.0 | 3.4 |
| EC body 1248 | MONSTER: Baddie 01 | 76.5 | 1010 | 153.0 | 26.55 / 26.59 / 26.79 | 187.0 | 42.3 | 17.0 | 4.5 |

Rank coding takes 4.7 s for the largest Classic body and 6.3 s for the largest Enhanced body, or 1.2 and 1.6 s on 4
threads with action streams. **So the 197 / 363 ms the user found too slow comes from whole-body units, and action
streams fix it:** the largest action in any of these takes 21 ms and the largest (action, direction) 4.5 ms, except
`Spider_Female`, whose largest action has one direction. Bodies 197 and 198 of `AnimationFrame1.uop`, which lead Classic,
aren't reached by a mobile's own body; see the warning in section 7.

### Custom MUL animations

This is the user's requirement, in [the format requirements](../uddp_anim_format_requirements.md).

- **Keep everything by default.** Everything the idx or UOP table holds goes in the package. Leaving out unreached art
  is at most an opt-in flag, working from the files of the install being converted, never from a fixed list of bodies.
- **Palette index 0 is opaque black when a custom entry uses it, because the client draws it.** "Index 0 means
  transparent" holds for shipped Classic art only, so the format has to detect or flag it per batch. The Classic slot
  plane above relies on that for shipped art. A batch that uses index 0 needs a shape plane, or a free slot remapped to
  transparent.
- **No candidate here depends on shipped statistics.** The zstd and LZ4 candidates use no tables of ours. tANS and rANS
  build their tables per body from the data, and rank coding builds them per palette. A trained dictionary would depend
  on them, but it was rejected in section 7 (×1.003–1.025). A motion search range limits the gain only, not validity.
- **What was run:** the round-trip checks cover shipped frames. They haven't been run on custom entries with arbitrary
  palettes and frame sizes. The design has no bound on either, and `mobile_anim_custom_mul.rs` tests the converter, not
  these coders.

### What wasn't measured

- **The `timing` pass in this CPU state:** it failed twice with `memory allocation failed` at 15 GiB resident, at the
  same early point, so it's recorded as not measured and wasn't retried. The ranking doesn't need it, since `s1`, `b`
  and `s2` time the same streams. The EPP power tables of section 2 stay as they were.
- **GPU upload, disk reads and BC7's own `s2` row,** and any frequency-weighted crowd from a real server.
- **A table-driven coverage path:** now measured, in item 6 of section 9. tANS by context keeps rANS's saving and reaches
  only 183 to 188 MB/s for the whole decode, against rANS's 150.
- **Reruns.** The `s1` Enhanced worst case ran separately with its memory fixed: BC7 frames freed before candidates, 6
  encoding threads. Its BC7 rows are from the first run, in the same CPU state. That rebuild includes the custom MUL
  fixes, which change no shipped frame. Two single-thread Python jobs overlapped the earlier `b` and `e2` jobs for about
  a minute each, which doesn't change a ratio. In the first timed `b` table the base rows read 590 and 879 MB/s, against
  769 and 1021 here, so those two were discarded.

## 9. Tiny and large streams, creatures and towns (`t`)

Measured 2026-11-04, for the user's decision on the format. No format or shipped code changes. The user has decided to
keep all frames, so no halved tier appears here. The census behind this section is in section 2 of
[the format requirements](../uddp_anim_format_requirements.md), and its section 6 lists the outside proposals that
these rows answer.

| Part | What it measures |
| --- | --- |
| `t0` | Kinds and exact duplicate streams, over the whole set (no sampling) |
| `t1` | Sizes on every 10th body: stream-size buckets, dictionaries, per-stream choice, coverage tables, row spans and painted-only pixels |
| `tt` | One-thread decode of the same sample |
| `tc` | The creature and dressed-NPC crowds, for this part's coders |
| `tu` | The uniform and worst crowds, for the coders that held up |
| `ts` | The older candidates of section 8 in the creature and dressed-NPC crowds |
| `t9` | Sparse frames, and frames split into up to 2 boxes |
| `tk0`, `tk` | The placeholder rule and coders per class: [section 10](#10-coders-per-class-tk) |

`MOBILE_ANIM_CODEC_TINY_ONLY` keeps coders by exact name, separated by `;`.

- **Lossless check:** every coder here decodes again and compares with the original pixels. None failed.
- **Quiet timings:** `tt`, `tc`, `tu` and `ts` ran one at a time on **boost off, EPP balance_power, platform profile
  performance**, with the bench as the machine's only CPU load (checked with `ps`), and Ghidra stopped. They
  compare with section 8's state, not with the EPP power tables. Repeats of zstd 19 show the drift: 667, 672 and 690
  MB/s for Classic, 866, 872 and 902 for Enhanced; 40.2 to 43.1 ms and 109.5 to 120.1 ms on the creature crowd. **Read
  differences under about 5 % as ties.** The one job that didn't finish is named under the Enhanced table.
- **Size jobs** ran at nice 19 and only their bytes are used.
- **MB/s** always divides the rectangle's bytes by the time, even for a "kept as spans" row that doesn't write them.
- **Kinds:** every part of this section splits by Classic's `mobtypes.txt`, by id. Enhanced ships none, so its split
  borrows Classic's, and Classic MUL goes through `Bodyconv.def` for anim2-5. Only `tk` uses the placeholder rule; these
  parts ran before the rule was settled and keep their split. "Creatures" means monster, animal, sea monster and human.
- **Disk** in bold is measured on every body, with every stream decoded back (section 16). A row marked ≈ is still the estimate from section 8: the sample scaled by the whole-set zstd 19 row, plus the palette table. The estimates ran up to 0.8 % low against the rows measured since.
  Read it as within about 1 %. **× real pack** is against 856.6 / 1987.9 MB and **× shipped** against 1060.3 / 1984.5 MB.

### The crowds

The uniform crowd draws every pair equally, so it is mostly equipment. Equipment is about 80 % of frames at a median of
20×23 in Enhanced, while Enhanced monsters are 85×94 and hold 52 % of pixels.

| Crowd | Classic: pairs, bodies, Mpx | Enhanced: pairs, bodies, Mpx |
| --- | ---: | ---: |
| **Creatures:** 300 distinct (body, action) pairs from non-equipment bodies | 300, 207, 105.6 | 300, 196, 216.6 |
| **Dressed NPCs:** 50 × (one human + 5 equipment bodies), one action | 300, 263, 10.1 | 300, 259, 15.6 |
| The same, each distinct pair once | 263, 263, 6.7 | 259, 259, 9.8 |
| Uniform (section 8) | 300, 251, 27.1 | 300, 241, 49.9 |
| Worst: the 300 largest pairs (section 8) | 300, 56, 917.9 | 300, 41, 1231.1 |

The scaling estimate for the Enhanced creature crowd was about 223 Mpx, with zstd 19 at 90 to 140 ms and luma + MED
missing the line with all five directions. **Measured:** 216.6 Mpx, zstd 19 at 110 to 120 ms, luma + MED at 530 ms with
all five directions and 112 ms faced only.

### The ranking, by the creature crowd

Sorted by the creature crowd with all five directions on 4 threads. Bold passes the 250 ms line on 4 threads. Decode MB/s
is one thread, per (action, direction). "Kept as spans" leaves out the expansion to the rectangle the atlas needs, so
it counts only if the renderer keeps spans. Duplicates add one table lookup per stream, too small to time.
Older rows take their uniform and worst columns from section 8 and their creature and dressed-NPC columns from `ts`,
where zstd 19 read 42.6 and 115.2 ms, so their ratios hold. All rows are lossless unless the name says otherwise.

**Classic**

| # | Candidate | Creatures, five directions, 1 / 4 threads ms | Creatures, faced, 1 / 4 | Dressed NPCs, five directions, 1 / 4 | Dressed NPCs, faced, 1 / 4 | Uniform, 4 threads, five / faced | Worst, 4 threads, five / faced | Decode MB/s, one thread | Disk MB | × zstd 19 | × real pack | × shipped |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Row spans (a), LZ4 HC, kept as spans | 35.9 / **10.3** | 7.8 / **2.2** | 6.3 / **1.8** | 1.4 / **0.4** | **3.2** / **0.8** | **65.1** / **14.5** | 2955 | **888.4** | 1.17 | 1.04 | 0.84 |
| 2 | Slot plane, LZ4 HC | 41.9 / **12.0** | 8.7 / **2.5** | 5.4 / **1.6** | 1.1 / **0.4** | **3.8** / **0.9** | **86.1** / **17.9** | 2685 | **929.7** | 1.22 | 1.09 | 0.88 |
| 3 | Row spans (a), LZ4 HC, expanded | 44.1 / **12.6** | 9.2 / **2.7** | 10.2 / **2.7** | 2.3 / **0.7** | **4.2** / **1.0** | **77.1** / **17.1** | 2171 | **888.4** | 1.17 | 1.04 | 0.84 |
| 4 | BC7, zstd 7 (today, stored; not lossless) | 109.7 / **29.1** | 22.5 / **5.9** | 23.7 / **5.6** | 4.5 / **1.3** | **9.6** / not run | **183.8** / not run | not in this run | ≈ 874 | 1.15 | 1.02 | 0.82 |
| 5 | Row spans (a), zstd 19, kept as spans | 133.2 / **35.7** | 26.7 / **7.3** | 22.0 / **5.7** | 4.7 / **1.2** | **13.6** / **2.9** | 250.4 / **51.6** | 753 | **733.4** | 0.96 | 0.86 | 0.69 |
| 6 | Row spans (a), zstd 19, expanded | 140.7 / **40.2** | 30.3 / **8.3** | 26.0 / **6.7** | 5.4 / **1.5** | **14.2** / **3.2** | 262.2 / **55.4** | 687 | **733.4** | 0.96 | 0.86 | 0.69 |
| 7 | Row spans (a), zstd 19, expanded, duplicates pointed at one blob | 140.7 / **40.2** | 30.3 / **8.3** | 26.0 / **6.7** | 5.4 / **1.5** | **14.2** / **3.2** | 262.2 / **55.4** | 687 | **706.5** | 0.93 | 0.82 | 0.67 |
| 8 | Row spans (a), best dictionary per body, expanded | 146.0 / **39.3** | 34.3 / **9.2** | 28.0 / **7.2** | 8.9 / **2.3** | **14.4** / **3.7** | 281.6 / **60.7** | 702 | **711.1** | 0.93 | 0.83 | 0.67 |
| 9 | Row spans (a), best dictionary per body, duplicates (2 + 3 + 8a) | 146.0 / **39.3** | 34.3 / **9.2** | 28.0 / **7.2** | 8.9 / **2.3** | **14.4** / **3.7** | 281.6 / **60.7** | 702 | **692.3** | 0.91 | 0.81 | 0.65 |
| 10 | Slot plane, zstd 19 | 150.2 / **40.6** | 30.9 / **8.4** | 24.6 / **6.5** | 5.2 / **1.4** | **13.5** / **2.9** | 276.3 / **56.5** | 667 | **761.1** | 1.00 | 0.89 | 0.72 |
| 11 | Slot plane, best dictionary per body | 150.0 / **40.7** | 34.1 / **9.3** | 25.1 / **6.4** | 7.3 / **1.9** | not run | not run | 690 | ≈ 751 | 0.99 | 0.88 | 0.71 |
| 12 | Slot plane, best dictionary per body, duplicates (2 + 3) | 150.0 / **40.7** | 34.1 / **9.3** | 25.1 / **6.4** | 7.3 / **1.9** | not run | not run | 690 | ≈ 724 | 0.95 | 0.84 | 0.68 |
| 13 | Painted only (b), runs apart, zstd 19, expanded | 155.0 / **44.0** | 32.9 / **9.0** | 31.1 / **7.9** | 6.3 / **1.7** | **15.1** / **3.5** | 277.4 / **57.1** | 624 | **746.8** | 0.98 | 0.87 | 0.70 |
| 14 | Luma, MED, zstd 19 | 1102 / 305.0 | 222.4 / **59.5** | 142.4 / **37.4** | 29.8 / **7.9** | **93.4** / **21.1** | 2186 / 448.0 | 98 | ≈ 828 | 1.09 | 0.97 | 0.78 |
| 15 | Luma, MED, 10 streams, zstd 19 | 1455 / 402.9 | 294.5 / **78.7** | 200.7 / **52.5** | 42.4 / **10.9** | **125.7** / **27.6** | 2873 / 593.0 | 70 | ≈ 840 | 1.10 | 0.98 | 0.79 |
| 16 | Luma, MED, 10 streams, tANS | 1678 / 458.7 | 379.7 / **105.4** | 256.7 / **65.5** | 92.8 / **24.7** | **146.8** / **38.1** | 3287 / 684.0 | 65 | ≈ 813 | 1.07 | 0.95 | 0.77 |
| 17 | Luma, MED + block motion, tANS | 1952 / 524.0 | 454.1 / **125.3** | 323.2 / **86.3** | 122.7 / **30.8** | **178.6** / **47.1** | 3846 / 805.0 | 55 | ≈ 784 | 1.03 | 0.92 | 0.74 |
| 18 | Rank + previous frame, rANS (reference) | 6754 / 1762 | 1383 / 361.2 | 1683 / 426.2 | 362.8 / **92.5** | 579.5 / **125.8** | 9112 / 1876 | 12 | ≈ 712 | 0.94 | 0.83 | 0.67 |

**Enhanced**

| # | Candidate | Creatures, five directions, 1 / 4 threads ms | Creatures, faced, 1 / 4 | Dressed NPCs, five directions, 1 / 4 | Dressed NPCs, faced, 1 / 4 | Uniform, 4 threads, five / faced | Worst, 4 threads, five / faced | Decode MB/s, one thread | Disk MB | × zstd 19 | × real pack | × shipped |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | Painted only + coverage row spans, LZ4 HC, kept as spans | 91.5 / **24.1** | 18.8 / **5.2** | 11.4 / **3.1** | 2.4 / **0.7** | **6.8** / **1.5** | **111.5** / **23.1** | 4380 | **2192.3** | 1.09 | 1.10 | 1.10 |
| 2 | Painted only (b), LZ4 HC, kept as spans | 100.2 / **26.8** | 20.9 / **5.7** | 11.0 / **2.9** | 2.3 / **0.7** | **7.0** / **1.6** | **125.4** / **26.1** | 4242 | ≈ 2225 | 1.10 | 1.12 | 1.12 |
| 3 | Both planes in row spans (c), LZ4 HC, kept as spans | 109.8 / **28.0** | 22.5 / **5.8** | 12.5 / **3.3** | 2.6 / **0.8** | **7.7** / **1.7** | **129.6** / **27.2** | 3896 | **2268.0** | 1.13 | 1.14 | 1.14 |
| 4 | Slot + coverage planes, LZ4 HC | 135.8 / **35.9** | 28.4 / **7.4** | 14.8 / **3.9** | 3.2 / **0.9** | **9.9** / **2.1** | **173.1** / **35.6** | 3047 | **2459.3** | 1.22 | 1.24 | 1.24 |
| 5 | Both planes in row spans (c), LZ4 HC, expanded | 131.7 / **36.4** | 28.4 / **7.5** | 21.5 / **5.7** | 4.5 / **1.2** | **11.1** / **2.6** | **169.5** / **34.9** | 2580 | **2268.0** | 1.13 | 1.14 | 1.14 |
| 6 | BC7, zstd 7 (today, stored; not lossless) | 201.8 / **51.4** | 41.0 / **10.5** | 30.2 / **7.5** | 6.0 / **1.6** | **15.3** / not run | **231.1** / not run | not in this run | ≈ 1904 | 0.95 | 0.96 | 0.96 |
| 7 | Painted only (b), LZ4 HC, expanded | 258.6 / **67.9** | 53.0 / **14.3** | 39.8 / **10.1** | 8.4 / **2.2** | **19.2** / **4.2** | 278.5 / **57.2** | 1459 | ≈ 2225 | 1.10 | 1.12 | 1.12 |
| 8 | Painted only + coverage row spans, LZ4 HC, expanded | 262.2 / **68.0** | 53.9 / **14.7** | 44.1 / **11.2** | 9.4 / **2.5** | **20.6** / **4.5** | 280.9 / **58.0** | 1368 | **2192.3** | 1.09 | 1.10 | 1.10 |
| 9 | Painted only + coverage row spans, zstd 19, kept as spans | 332.6 / **85.8** | 69.4 / **18.2** | 45.4 / **11.1** | 9.0 / **2.4** | **22.9** / **4.8** | 383.4 / **77.9** | 1175 | **1819.9** | 0.90 | 0.92 | 0.92 |
| 10 | Painted only (b), zstd 19, kept as spans | 349.9 / **90.3** | 73.0 / **19.0** | 44.6 / **11.2** | 8.9 / **2.4** | **23.3** / **5.0** | 397.9 / **81.6** | 1151 | **1845.3** | 0.92 | 0.93 | 0.93 |
| 11 | Both planes in row spans (c), zstd 19, kept as spans | 361.9 / **92.0** | 72.4 / **18.7** | 47.0 / **12.0** | 10.1 / **2.6** | **24.3** / **5.2** | 416.1 / **84.8** | 1084 | **1881.1** | 0.93 | 0.95 | 0.95 |
| 12 | Both planes in row spans (c), zstd 19, expanded | 389.3 / **100.3** | 78.7 / **20.5** | 55.1 / **14.1** | 11.2 / **2.9** | **28.1** / **6.0** | 451.1 / **92.2** | 960 | **1881.1** | 0.93 | 0.95 | 0.95 |
| 13 | Row spans (a), zstd 19, expanded | 421.4 / **108.6** | 84.8 / **22.3** | 60.6 / **15.3** | 12.3 / **3.1** | **31.2** / **6.9** | 474.9 / **97.4** | 895 | **1955.7** | 0.97 | 0.98 | 0.99 |
| 14 | Slot + coverage planes, zstd 19 | 442.5 / **120.1** | 94.7 / **23.3** | 57.9 / **16.5** | 12.5 / **3.3** | **29.4** / **6.3** | 491.4 / **99.8** | 866 | **2013.8** | 1.00 | 1.01 | 1.01 |
| 15 | Painted only + coverage row spans, zstd 19, expanded | 505.3 / **129.0** | 101.3 / **26.3** | 75.7 / **20.0** | 15.9 / **4.4** | **36.8** / **7.8** | 548.0 / **112.2** | 736 | **1819.9** | 0.90 | 0.92 | 0.92 |
| 16 | Painted only + coverage row spans, zstd 19, expanded, duplicates (2 + 8) | 505.3 / **129.0** | 101.3 / **26.3** | 75.7 / **20.0** | 15.9 / **4.4** | **36.8** / **7.8** | 548.0 / **112.2** | 736 | **1808.7** | 0.90 | 0.91 | 0.91 |
| 17 | Painted only (b), zstd 19, expanded | 504.5 / **130.3** | 103.5 / **27.6** | 77.1 / **19.5** | 15.8 / **4.1** | **35.7** / **7.6** | 545.8 / **111.9** | 754 | **1845.3** | 0.92 | 0.93 | 0.93 |
| 18 | Luma, left, zstd 19, tight (item 4) | 519.6 / **132.9** | 104.5 / **27.0** | 80.4 / **21.9** | 16.7 / **4.6** | **38.3** / **8.6** | 541.1 / **110.4** | 717 | ≈ 1871 | 0.93 | 0.94 | 0.94 |
| 19 | Luma, left, tight, coverage row spans | 513.5 / **137.9** | 109.9 / **28.5** | 83.1 / **22.5** | 18.2 / **4.7** | **39.9** / **9.0** | 540.7 / **110.2** | 699 | ≈ 1847 | 0.92 | 0.93 | 0.93 |
| 20 | Plain or left per stream, tight (item 5) | 512.4 / **138.3** | 108.7 / **28.1** | 74.8 / **20.3** | 16.4 / **4.3** | **36.8** / **8.5** | 546.3 / **110.8** | 722 | ≈ 1864 | 0.93 | 0.94 | 0.94 |
| 21 | Zstd 19, coverage runs zstd 19, partial tANS by context | 749.9 / **202.9** | 216.7 / **55.0** | 176.4 / **45.1** | 97.8 / **25.3** | not run | not run | 531 | ≈ 1982 | 0.98 | 1.00 | 1.00 |
| 22 | Luma, left, tight, coverage runs zstd 19, partial tANS by context, duplicates (2 + 4 + 6) | 824.0 / **212.5** | 229.9 / **59.3** | 196.9 / **51.7** | 103.0 / **27.1** | not run | not run | 469 | ≈ 1829 | 0.91 | 0.92 | 0.92 |
| 23 | Painted only, coverage runs zstd 19, partial tANS by context, duplicates | 813.8 / **218.3** | 227.2 / **58.0** | 190.5 / **50.2** | 101.0 / **25.4** | not run | not run | 485 | **1794.0** | 0.89 | 0.90 | 0.90 |
| 24 | Luma, MED, zstd 19 | 2080 / 530.4 | 416.6 / **111.5** | 209.4 / **54.5** | 43.1 / **12.0** | **127.9** / **28.1** | 2624 / 539.0 | 213 | ≈ 1823 | 0.91 | 0.92 | 0.92 |
| 25 | Painted only, coverage every pixel tANS by context, duplicates | 2316 / 591.2 | 532.6 / **137.2** | 319.7 / **81.1** | 138.2 / **34.6** | not run | not run | 183 | **1707.6** | 0.85 | 0.86 | 0.86 |
| 26 | Luma, left, tight, coverage every pixel tANS by context, duplicates (2 + 4 + 6) | 2334 / 596.0 | 534.9 / **136.7** | 325.6 / **83.9** | 139.4 / **37.1** | not run | not run | 181 | ≈ 1740 | 0.86 | 0.88 | 0.88 |
| 27 | Luma, MED, 10 streams, zstd 19 | 2688 / 679.9 | 531.3 / **136.4** | 276.1 / **76.0** | 59.6 / **15.5** | **168.9** / **36.8** | 3451 / 707.0 | 157 | ≈ 1845 | 0.92 | 0.93 | 0.93 |
| 28 | Luma, MED, 10 streams, tANS | 3081 / 781.3 | 655.5 / **176.1** | 350.0 / **91.3** | 113.6 / **29.6** | **199.9** / **53.6** | 4005 / 825.0 | 141 | ≈ 1816 | 0.90 | 0.91 | 0.92 |
| 29 | Luma, MED + block motion, tANS | 3641 / 919.7 | 784.9 / **200.2** | 431.0 / **111.1** | 147.9 / **37.7** | **239.2** / **65.0** | 4732 / 973.0 | 118 | ≈ 1712 | 0.85 | 0.86 | 0.86 |
| 30 | Rank + previous frame, rANS (reference) | 11698 / 2961 | 2362 / 597.4 | 2546 / 646.4 | 547.2 / **146.5** | 1010 / **216.4** | 11704 / 2381 | 24 | ≈ 1597 | 0.79 | 0.80 | 0.80 |

**The uniform and worst columns come from two runs.** The first `tu` job stopped at the 18 GB resident watchdog after 14 of its 26 Enhanced row sets, because coded streams of earlier rows stay cached for reuse. A narrowed quiet rerun covers the row-span, painted and dictionary rows, and those rows use it: the cluster log has the other cluster at 1 % busy through it, peak 2 %. **Its zstd 19 rows are the anchors for them:** Classic 13.5 / 2.9 ms uniform and 276.3 / 56.5 ms worst, Enhanced 29.4 / 6.3 and 491.4 / 99.8. The first run's zstd 19 agrees: 15.0 / 3.2, 276.9 / 57.2, 30.8 / 6.8 and 493.8 / 102.4. No row changed between pass and fail.

**What the ranking by creatures changes:**

- **Pass or fail:** on the uniform crowd every lossless candidate passes with all five directions except rank coding. On
  the creature crowd, **luma + MED, its 10-stream form and block motion all fail with all five directions** (Classic
  305 to 524 ms, Enhanced 530 to 920 ms) and pass only faced (60 to 125 ms, 112 to 200 ms). Rank coding fails both ways.
  Enhanced's every-pixel coverage tables fail the same way (about 590 ms, 137 faced).
- **Dressed NPCs are easy.** A town is 10 to 16 Mpx, and everything but rank coding passes with all five directions. A
  town only separates candidates with tables per body, since their setup doesn't shrink with the pixels: coverage by
  context costs 2.7× zstd 19 here against 1.7× on creatures.
- **Row-based layouts move up.** Classic's row spans and Enhanced's both-planes spans are as fast as the rectangle or
  faster, so they sit above plain zstd 19 and are smaller. No residual row is faster than zstd 19.

### 1. By stream size and kind: do tiny and large streams want different codecs?

| Client | Kind | Stream size | Streams | Mpx | Zstd 19 MB | LZ4 HC ÷ zstd 19 | Luma + left ÷ zstd 19 | Zstd headers and tables, % of zstd 19 |
|---|---|---|---:|---:|---:|---:|---:|---:|
| Classic | equipment | up to 4 KiB | 7690 | 11.6 | 4.70 | 1.362 | 1.103 | 11.2 % |
| Classic | equipment | up to 16 KiB | 2254 | 16.2 | 5.20 | 1.340 | 1.114 | 5.1 % |
| Classic | equipment | up to 64 KiB | 176 | 6.3 | 1.01 | 1.380 | 1.035 | 2.1 % |
| Classic | equipment | up to 256 KiB | 24 | 2.0 | 0.28 | 1.240 | 0.996 | 1.1 % |
| Classic | equipment | all sizes | 10144 | 36.1 | 11.19 | 1.350 | 1.099 | 7.3 % |
| Classic | creatures | up to 4 KiB | 313 | 0.5 | 0.24 | 1.379 | 1.145 | 10.7 % |
| Classic | creatures | up to 16 KiB | 632 | 6.6 | 2.83 | 1.210 | 1.065 | 3.3 % |
| Classic | creatures | up to 64 KiB | 1061 | 34.4 | 12.53 | 1.193 | 1.067 | 1.4 % |
| Classic | creatures | up to 256 KiB | 459 | 53.9 | 16.25 | 1.189 | 1.064 | 0.7 % |
| Classic | creatures | larger | 133 | 109.1 | 29.13 | 1.225 | 1.066 | 1.1 % |
| Classic | creatures | all sizes | 2598 | 204.5 | 60.99 | 1.209 | 1.066 | 1.2 % |
| Classic | everything | up to 4 KiB | 8550 | 12.5 | 5.13 | 1.366 | 1.106 | 11.3 % |
| Classic | everything | up to 16 KiB | 3125 | 25.0 | 9.03 | 1.284 | 1.097 | 4.3 % |
| Classic | everything | up to 64 KiB | 1391 | 47.1 | 15.16 | 1.206 | 1.060 | 1.5 % |
| Classic | everything | up to 256 KiB | 488 | 56.2 | 16.61 | 1.190 | 1.062 | 0.7 % |
| Classic | everything | larger | 133 | 109.1 | 29.13 | 1.225 | 1.066 | 1.1 % |
| Classic | everything | all sizes | 13687 | 249.9 | 75.06 | 1.230 | 1.070 | 2.2 % |
| Enhanced | equipment | up to 4 KiB | 5718 | 10.2 | 7.61 | 1.344 | 0.921 | 9.7 % |
| Enhanced | equipment | up to 16 KiB | 5330 | 45.5 | 24.47 | 1.301 | 0.924 | 3.9 % |
| Enhanced | equipment | up to 64 KiB | 2116 | 60.5 | 25.52 | 1.285 | 0.963 | 1.6 % |
| Enhanced | equipment | up to 256 KiB | 4 | 0.3 | 0.02 | 1.447 | 0.877 | 3.8 % |
| Enhanced | equipment | all sizes | 13168 | 116.4 | 57.62 | 1.300 | 0.941 | 3.5 % |
| Enhanced | creatures | up to 4 KiB | 16 | 0.1 | 0.04 | 1.249 | 0.892 | 5.8 % |
| Enhanced | creatures | up to 16 KiB | 243 | 2.6 | 1.48 | 1.270 | 0.906 | 3.1 % |
| Enhanced | creatures | up to 64 KiB | 1077 | 41.8 | 21.89 | 1.206 | 0.910 | 1.0 % |
| Enhanced | creatures | up to 256 KiB | 1022 | 137.0 | 56.37 | 1.187 | 0.915 | 0.6 % |
| Enhanced | creatures | larger | 243 | 153.8 | 48.06 | 1.200 | 0.944 | 1.1 % |
| Enhanced | creatures | all sizes | 2601 | 335.2 | 127.85 | 1.196 | 0.925 | 0.9 % |
| Enhanced | everything | up to 4 KiB | 5734 | 10.3 | 7.66 | 1.343 | 0.921 | 9.7 % |
| Enhanced | everything | up to 16 KiB | 5573 | 48.0 | 25.94 | 1.299 | 0.923 | 3.9 % |
| Enhanced | everything | up to 64 KiB | 3193 | 102.2 | 47.41 | 1.249 | 0.938 | 1.3 % |
| Enhanced | everything | up to 256 KiB | 1058 | 144.4 | 58.71 | 1.186 | 0.914 | 0.6 % |
| Enhanced | everything | larger | 301 | 175.0 | 54.11 | 1.198 | 0.937 | 1.1 % |
| Enhanced | everything | all sizes | 15859 | 479.9 | 193.84 | 1.226 | 0.928 | 1.6 % |

- **No bucket flips a result.** Luma + left loses on Classic and wins on Enhanced in every bucket with more than a few
  streams. Tiny and large streams don't want different residuals.
- **LZ4 HC costs most where streams are tiny:** 1.34 to 1.37× zstd 19 up to 4 KiB, against 1.19 to 1.23× above 64 KiB.
  The time it saves is on large streams, so a mixed rule would use LZ4 HC for large streams and zstd 19 for small ones.
  [Section 10](#10-coders-per-class-tk) measures that by class.
- **Overhead is real only in the tiny bucket:** zstd's headers and tables are about 10 to 11 % up to 4 KiB, 0.6 to 1.1 %
  above 64 KiB, and 2.2 / 1.6 % overall. Equipment carries it: 7.3 / 3.5 % against creatures' 1.2 / 0.9 %.

### 2. Exact duplicate streams

Hashed over the whole set, by the exact plane of each (action, direction). Decode cost is one table lookup. Frame
metadata stays per stream. Bytes saved, where Enhanced's row spans and painted columns hold coverage in row spans:

| Client | Repeated streams | Mpx | Rectangle MB | Row spans MB | Painted only MB | Luma + left MB | Where they are |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Classic | 2540 of 138426 | 89.39 | 27.73 | 26.94 | 27.92 | 29.53 | **Within creature bodies:** 26.41 MB, 1876 streams |
| Enhanced (index + coverage) | 1238 of 158133 | 25.82 | 12.34 | 11.91 | 11.25 | 11.96 | **Equipment across bodies:** 8.32 MB; creatures within 1.46 and across 2.55 |

So a pointer saves **3.6 % of Classic** and **0.6 % of Enhanced**. It's the cheapest row here, and per-stream zstd can't
see it. It needs a hash per stream at pack time and a stream table that can point two entries at one blob.

### 3. Recovering Classic's per-stream cost

Whole-body zstd 19 is 677 MB against 761 MB per stream, a gap of 84 MB. Index plane only, dictionaries counted, each body
training only on its own streams, with no fixed tables or shipped statistics:

| Coder | Classic: equipment / creatures / total MB | Enhanced: equipment / creatures / total MB | Creature crowd, 4 threads, Classic / Enhanced ms |
| --- | ---: | ---: | ---: |
| Zstd 19 per stream | 11.19 / 60.99 / 75.06 | 43.74 / 106.61 / 157.06 | 40.6 / 120.1 |
| No magic, checksum or content size | 11.14 / 60.98 / 74.99 | 43.68 / 106.59 / 156.98 | 42.8 / 114.6 |
| Own dictionary, 1 KiB | 11.04 / 62.46 / 76.38 | 43.82 / 108.58 / 159.20 | 45.1 / 118.9 |
| Own dictionary, 4 KiB | 11.07 / 62.15 / 76.10 | 43.87 / 108.25 / 158.90 | not timed |
| Own dictionary, 16 KiB | 11.37 / 61.45 / 75.72 | 44.29 / 107.74 / 158.80 | 45.7 / 123.4 |
| Own dictionary, 64 KiB | 12.23 / 61.30 / 76.44 | 45.46 / 107.87 / 160.09 | not timed |
| Own dictionary, 256 KiB | 12.34 / 62.26 / 77.62 | 48.11 / 109.75 / 164.67 | 45.8 / 115.7 |
| Best of those per body, one byte per body | 10.97 / 60.27 / **74.11** | 43.62 / 106.59 / **156.93** | see below |

- **The gap is shared match history, not overhead.** Dropping every optional field saves 0.1 %. A fixed dictionary size
  loses on total, because it can't repay itself in small bodies. Only a choice per body gains: **−1.3 % on Classic** (65
  of 103 bodies take one, and 1.46 MB of dictionaries are counted) and −0.1 % on Enhanced (20 of 93).
- **On row spans it gains more:** Classic's row spans with the best dictionary are 70.57 MB, against 72.43 without.
- **What a crowd costs when each new body loads its dictionary:** loading is 1.47 ms on one thread for all 207 bodies of
  the Classic creature crowd (1 KiB), 1.87 ms for 205 (16 KiB) and 2.73 ms for 147 (256 KiB); Enhanced 1.26 ms for 196
  (1 KiB) and 3.37 ms for 181 (256 KiB). The load is not the cost. Decoding through a dictionary is: +11 to 13 % on
  Classic's crowd, about level on Enhanced.
- **Reading the "best dictionary" crowd rows:** the first runs chose a dictionary on each crowd body's few streams, where
  one rarely repays itself, so only 1 to 3 bodies used one and those rows were a lower bound. The creature and dressed
  columns above come from a quiet rerun that chooses over the whole body, as a real package would, with the dictionary's
  load included. **That rerun's own zstd 19 row is 38.7 and 7.7 ms on Classic's creatures, where the table's first run has 40.6 and 8.4, so read a dictionary row against 38.7.** The
  rows stay inside the line, but a dictionary is read once per body: on Classic's creature crowd the faced-only read goes
  from 6.4 MB to 19.0 MB (slot plane) and from 6.2 MB to 25.6 MB (row spans), and the faced time goes from 7.7 to 9.3 ms (slot plane) and from 7.5 to 9.2 ms (row spans).
  With dictionaries already loaded the row-span times are 39.5 and 8.6 ms. On Enhanced a dictionary saves 0.1 % and doesn't change the order.
- **Runs differ more than rows do.** Enhanced's zstd 19 on creatures is 120.1 ms in the table's run and 108.0 ms in the same rerun, both quiet by the cluster log and the load average. Row spans (a) is 0.96 × zstd 19 in the rerun and 0.90 × in the first. So a gap of a few percent between two rows from different runs means nothing here. Compare a row with the zstd 19 row of its own run.

### 4. A tight residual decode: the main question for Enhanced

The new decode reads the shape first, then runs a prefix sum along each opaque span (`left`) or a vector add against the
row above (`up`). The per-pixel `predict` stays as the reference.

| Enhanced, 4 threads unless marked | MB/s, one thread | × zstd 19 | Creatures, five / faced ms | Dressed NPCs, five / faced ms | Total MB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Zstd 19, same run | 866 | 1 | 120.1 / 23.3 | 16.5 / 3.3 | 193.83 |
| Left, per-pixel `predict` | 449 | 0.52 | 226.1 / 45.8 | 28.7 / 6.3 | 179.83 |
| **Left, tight** | **717** | **0.83** | **132.9 / 27.0** | **21.9 / 4.6** | 179.83 |
| Up, tight | 643 | 0.74 | 146.5 / 30.0 | 24.1 / 4.9 | 179.95 |

- **The answer: 0.83× zstd 19 per stream, and 0.80× per action** (772 against 970 MB/s). The 1021 MB/s of section 8 is an
  earlier run per action; this run's 970 is the fair partner. On the creature crowd the gap is only 1.11×.
- **Time on the sample's index plane, one thread:** zstd 0.282 s, reading the shape 0.236 s, the walk 0.316 s. Zstd has
  fewer bytes to write than for the rectangle, so two thirds of the time is now the shape and the walk.
- **`up` with a vector add is slower than `left`** on both clients (643 against 717 MB/s, 442 against 478), and it isn't
  smaller.
- **Classic:** 478 MB/s against 667, 55.6 ms on creatures against 40.6, and still 7 % larger. It stays out.
- **But the residual no longer earns its place on Enhanced.** Painted only, with no residual and the original palette
  order (item 8), is 1.5 % smaller at 177.10 MB and decodes at the same speed. What the residual appeared to gain came
  from dropping the transparent pixels.

### 5. One codec id per stream

Plain or left, whichever is smaller. Totals are whole streams: index, Classic's shape or Enhanced's coverage, and one id
byte per stream. A stream's palette follows its choice, which adds 0.12 / 0.19 MB on the sample's palette table.

| Client | Coder | Total MB | MB/s | Creatures, five / faced ms | Share picking left: equipment, creatures, not listed |
| --- | --- | ---: | ---: | ---: | --- |
| Classic | Plain or left | 75.03 (−0.0 %) | 658 | 41.1 / 8.4 | 5.1 %, 4.7 %, 5.9 % |
| Classic | Plain, left or up | 74.75 (−0.4 %) | 635 | 43.9 / 9.3 | |
| Enhanced | Plain or left | 179.24 (−7.5 %) | 722 | 138.3 / 28.1 | 87.7 %, 95.7 %, 100 % |
| Enhanced | Plain, left or up | 177.13 (−8.6 %) | 677 | 145.8 / 29.5 | |

**A per-stream id adds nothing on Classic,** since plain wins 95 % of streams. On Enhanced the choice is nearly always
left, so it's worth 0.3 % of the total over left alone, and three-way 1.5 %. Against painted only (item 8) it loses.

### 6. Table-driven coverage (Enhanced)

Tables per body are counted. Coverage MB/s is the plane alone, and "Tables" is the time to build every table of the sample.

| Coverage coder | Equipment / creatures / total MB | Coverage MB/s | Tables, ms | With zstd 19 index, total MB, MB/s, creatures ms | With tight left index, total MB, MB/s, creatures ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Zstd 19 | 13.88 / 21.24 / 36.77 | 1237 | - | 193.83, 866, 120.1 | 179.83, 717, 132.9 |
| LZ4 HC | total 54.13 | 3525 | - | - | - |
| Runs + partial values, tANS | total 40.77 | 578 | 3.08 | 197.83, 620, 153.1 | 183.83, 535, 182.1 |
| Runs with copy-up, tANS | total 40.09 | 579 | 3.19 | 197.16, 619, 154.9 | 183.15, 534, 183.1 |
| Runs in zstd 19, partial tANS | total 35.75 | 645 | 3.06 | 192.81, 655, 152.6 | 178.80, 560, 162.6 |
| Runs with copy-up in zstd 19, partial tANS | total 36.03 | 632 | 3.16 | 193.10, 645, 154.1 | 179.09, 558, 165.1 |
| **Every pixel, tANS by context** | **9.67 / 14.09 / 24.93** | **112** | 27.27 | 181.99, 188, 580.4 | 167.99, 181, 596.0 |
| Runs in zstd 19, partial tANS by context | total 33.61 | 455 | 24.49 | 190.67, 531, 202.9 | 176.66, 469, 212.5 |
| Row spans, zstd 19 | total 34.45 | 1171 | - | - | 177.51, 699, 137.9 |

- **tANS doesn't rescue the rANS row.** Every pixel by context keeps rANS's saving (24.93 against 24.40 MB, −32 % of the
  plane, −6 % of the total) but its coverage decode is 112 MB/s, and the whole row 183 to 188 MB/s against rANS's 150.
  **It fails five directions on creatures, at 580 ms.** Faced only it passes at 133 ms.
- **The light forms give almost nothing:** runs and partial values save at most 2.8 % of the plane, and cost about 25 %
  of the speed.
- **Row spans on coverage are the cheap win:** −6.3 % of the plane at 0.95× the speed.

### 7. Rows to close

One row each, so they don't come back. Sizes are whole streams against the rectangle at zstd 19 in the same run.

| Idea | Classic MB, MB/s, creatures ms | Enhanced MB, MB/s, creatures ms | Verdict |
| --- | ---: | ---: | --- |
| (a) Scanline RLE + semi-adaptive tANS, no LZ, no trained tables | 82.34 (+9.7 %), 190, 151.0 | 220.41 (+13.7 %), 331, 322.3 | **Closed:** larger and slower than zstd 19. Enhanced fails the line |
| (b) Per-body dictionary of exact 4×4 patches, cost-model bound counting codebook and block ids | +11.8 % on the index plane | +20.9 % on the index plane | **Closed:** the bound alone is already larger, so no decoder was written |
| (c) Master palette + XOR with the previous frame + zstd 19, per stream | 93.09 (+24.0 %), 609, 45.2 | 220.77 (+13.9 %), 834, 117.4 | **Closed:** about the same speed, and 14 to 24 % larger |

### 8. Row spans and painted-only pixels

The census says about 55 % of each rectangle lies outside the row's first and last painted pixel. Rows, per
(action, direction):

- **a. Row spans:** the index stream holds each row from its first to its last painted pixel. A side stream holds start
  and length, raw, against the previous row, or against the same row of the previous frame, whichever is smallest per
  stream, inside the same zstd frame. Interior holes stay as slot 0.
- **b. Painted only:** interior holes go too, and an exact shape stream describes them. Enhanced needs no extra shape,
  since its coverage plane is the shape. Classic's has to be stored. **This is the first measurement of this against the
  plain rectangle;** earlier shape rows all had a residual or a colour order on top.
- **c. Enhanced:** both planes in the same row spans, sharing one side stream.

| Client | Layout | Equipment MB | Creatures MB | Not listed MB | Total MB | × rectangle | MB/s, expanded / kept | Creatures, five ms, 4 threads, expanded / kept | Dressed NPCs, five ms, expanded / kept |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Classic | Rectangle, zstd 19 | 11.19 | 60.99 | 2.88 | 75.06 | 1 | 667 | 40.6 | 6.5 |
| Classic | **a, zstd 19** | 10.80 | 58.88 | 2.75 | **72.43** | **0.965** | **687 / 753** | **40.2 / 35.7** | **6.7 / 5.7** |
| Classic | b, runs apart, zstd 19 | 11.21 | 59.37 | 2.83 | 73.42 | 0.978 | 624 | 44.0 | 7.9 |
| Classic | b, runs in the frame, zstd 19 | | | | 74.57 | 0.993 | 620 / 832 | 42.6 / 34.6 | 7.9 / 5.5 |
| Classic | b, bits apart, zstd 19 | | | | 74.01 | 0.986 | 540 | 49.1 | 8.6 |
| Classic | **a, best dictionary per body** | 10.45 | 57.50 | 2.62 | **70.57** | **0.940** | 702 | see item 3 | |
| Classic | Rectangle, LZ4 HC | 15.11 | 73.72 | 3.50 | 92.33 | 1.230 | 2685 | 12.0 | 1.6 |
| Classic | a, LZ4 HC | 14.39 | 70.61 | 3.36 | 88.37 | 1.177 | 2171 / 2955 | 12.6 / 10.3 | 2.7 / 1.8 |
| Classic | b, runs in the frame, LZ4 HC | | | | 91.56 | 1.220 | 1569 / 4206 | 17.0 / 7.8 | 3.9 / 1.3 |
| Enhanced | Rectangle, zstd 19 | 57.62 | 127.85 | 8.36 | 193.83 | 1 | 866 | 120.1 | 16.5 |
| Enhanced | a, zstd 19 | 56.35 | 123.80 | 8.00 | 188.15 | 0.971 | 895 / 946 | 108.6 / 105.3 | 15.3 / 14.1 |
| Enhanced | **c, both planes, zstd 19** | 52.94 | 120.17 | 7.78 | **180.90** | **0.933** | **960 / 1084** | **100.3 / 92.0** | **14.1 / 12.0** |
| Enhanced | **b, zstd 19** | 52.26 | 117.33 | 7.52 | **177.10** | **0.914** | 754 / 1151 | 130.3 / 90.3 | 19.5 / 11.2 |
| Enhanced | **b + coverage in row spans, zstd 19** | 51.70 | 115.67 | 7.41 | **174.78** | **0.902** | 736 / 1175 | 129.0 / 85.8 | 20.0 / 11.1 |
| Enhanced | b + coverage every pixel tANS by context | 48.05 | 110.17 | 7.04 | 165.26 | 0.853 | 183 | 591.2 (fails) | 81.1 |
| Enhanced | Rectangle, LZ4 HC | 74.89 | 152.90 | 9.87 | 237.66 | 1.226 | 3047 | 35.9 | 3.9 |
| Enhanced | c, LZ4 HC | 66.82 | 142.67 | 9.17 | 218.67 | 1.128 | 2580 / 3896 | 36.4 / 28.0 | 5.7 / 3.3 |
| Enhanced | b, LZ4 HC | 65.83 | 139.67 | 8.96 | 214.47 | 1.106 | 1459 / 4242 | 67.9 / 26.8 | 10.1 / 2.9 |
| Enhanced | b + coverage in row spans, LZ4 HC | 64.93 | 137.57 | 8.79 | 211.29 | 1.090 | 1368 / 4380 | 68.0 / 24.1 | 11.2 / 3.1 |

By stream size, zstd 19, against the rectangle in the same bucket:

| Layout | Up to 4 KiB | Up to 16 KiB | Up to 64 KiB | Up to 256 KiB | Larger | All |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Classic, a | 0.979 | 0.960 | 0.964 | 0.963 | 0.965 | 0.965 |
| Classic, b, runs apart | 1.014 | 0.993 | 0.984 | 0.979 | 0.964 | 0.978 |
| Enhanced, a | 0.994 | 0.981 | 0.971 | 0.969 | 0.964 | 0.971 |
| Enhanced, c | 0.901 | 0.919 | 0.927 | 0.941 | 0.942 | 0.933 |
| Enhanced, b | 0.884 | 0.902 | 0.919 | 0.917 | 0.915 | 0.914 |
| Enhanced, b + coverage in row spans | 0.879 | 0.893 | 0.907 | 0.905 | 0.901 | 0.902 |

- **Best per client.** **Classic: a,** with the best dictionary if a pack can spend the time. **Enhanced: b + coverage in
  row spans** for bytes, **c** for speed, since it decodes 1.11× faster than the rectangle while 6.7 % smaller.
- **Why the two clients differ.** Enhanced's shape is free, because the coverage plane already holds it, so removing holes
  costs nothing and gains 8.6 %. Classic has to store it, and that eats the gain. Row spans cost Classic only their side
  stream: 2.29 MB alone in zstd 19 out of 72.43.
- **Side stream:** against the previous row wins most streams (70 % of Classic's equipment, 93 % of creatures; 76 % and
  90 % of Enhanced's). On its own, it's 3.2 % of Classic's streams and 2.2 % of Enhanced's.
- **The gain is alike for equipment and creatures,** at 0.965 each on Classic and 0.897 against 0.905 on Enhanced, so the
  empty space isn't an equipment effect.
- **The expansion is the cost.** On Enhanced with b, kept spans are 1.5× faster than expanded: 1151 against 754 MB/s.
  For c the expansion is much cheaper (960 against 1084), because a whole row of each plane is copied at once.
- **Combinations.** With duplicates (2): Classic a saves 26.94 MB more, Enhanced b + coverage spans 11.25. With dictionaries
  (3): Classic gains, 70.57; Enhanced doesn't (a + best dictionary 187.81 against 188.15). With a tight residual (4):
  it adds nothing, left + coverage spans is 177.51 against b + coverage spans at 174.78. With table-driven coverage (6): b +
  runs by context 173.94 MB at 218 ms on creatures, b + every pixel by context 165.26 MB failing at 591 ms.

### 9. Sparse frames and split boxes

Frames under 20 % painted, coded apart from the rest, so each set loses the matches between them. Sizes are the sample,
with coverage counted. The census's split (13 % of worn frames, 25 % of box area, 7 % of painted pixels on Enhanced) counts
every 10th entry; this one is the codec sample, by `mobtypes.txt`.

| Client, kind | Share of frames / box area / painted, % | Zstd 19 rectangle MB | a | b | Best row × rectangle | Split boxes MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Classic equipment, sparse | 11.5 / 31.1 / 10.0 | 1.343 | 1.265 | 1.381 | 0.942 (a) | 1.335 |
| Classic equipment, the rest | 88.5 / 68.9 / 90.0 | 9.917 | 9.618 | 9.929 | 0.970 (a) | 9.909 |
| Classic creatures, sparse | 0.8 / 1.8 / 0.8 | 0.553 | 0.523 | 0.538 | 0.946 (a) | 0.553 |
| Classic creatures, the rest | 99.2 / 98.2 / 99.2 | 60.450 | 58.379 | 58.855 | 0.966 (a) | 60.448 |
| Enhanced equipment, sparse | 10.2 / 17.1 / 4.0 | 3.835 | 3.686 | 3.198 | 0.804 (b + coverage spans 3.085) | 3.784 |
| Enhanced equipment, the rest | 89.8 / 82.9 / 96.0 | 53.939 | 52.853 | 49.174 | 0.904 (48.764) | 53.864 |
| Enhanced creatures, sparse | 2.8 / 4.7 / 2.2 | 3.341 | 3.186 | 2.996 | 0.872 (2.915) | 3.340 |
| Enhanced creatures, the rest | 97.2 / 95.3 / 97.8 | 124.569 | 120.734 | 114.431 | 0.906 (112.877) | 124.566 |

**Layouts help sparse frames more than the rest,** −15.9 % against −9.5 % on Enhanced and −5.7 % against −3.4 % on Classic,
**but sparse frames are few bytes:** 4 % of Enhanced's total and 2.5 % of Classic's. They need no rule of their own.

**Split boxes** cut a frame's box at fully empty rows and columns, into up to 2 boxes, when that saves at least 64 pixels.

| | Classic | Enhanced |
| --- | ---: | ---: |
| Total, zstd 19 MB, against the rectangle | 75.01 against 75.06 (−0.06 %) | 193.69 against 193.83 (−0.07 %) |
| Decode MB/s, one thread, expanded / kept / rectangle in the same run | 653 / 698 / 690 | 851 / 910 / 902 |
| Creatures, 4 threads, five directions, expanded / kept / rectangle ms | 42.5 / 39.1 / 40.2 | 116.0 / 108.2 / 109.5 |
| Dressed NPCs, 4 threads, five directions, expanded / kept / rectangle ms | 6.7 / 5.9 / 6.0 | 15.9 / 15.0 / 14.7 |
| Atlas area saved, all frames | 4.2 % | 3.0 % |
| Atlas area saved, equipment / creatures | 18.6 % / 0.2 % | 12.3 % / 0.0 % |
| Atlas area saved, sparse equipment | 41.3 % | 48.0 % |
| Atlas area saved with a 1-pixel gutter, all frames | 4.1 % | 2.9 % |
| Sample VRAM saved, MB (R8 / RG8) | 10.6 | 29.0 |
| Atlas area saved, crowds: creatures / dressed NPCs / uniform / worst, five directions | 0.5 / 6.9 / 2.8 / 0.3 % | 0.1 / 4.7 / 3.8 / 0.1 % |

- **It's an atlas row, not a disk row.** Bytes don't move, because an empty row or column costs a match or less, and the
  geometry takes the rest of what is saved. Decode is about 5 % slower expanded. Keeping the boxes is what saves the area.
- **Island bodies 1055 to 1058** are the case it's for: 31.3, 20.4, 31.3 and 20.4 % of atlas area saved, and 30.0, 34.7,
  30.0 and 34.7 % of the box inside row spans. Their bytes still move under 1 % (3.470 → 3.448, 4.954 → 4.940, 3.799 →
  3.777, 4.920 → 4.908 MB), while b + coverage spans takes 18.4 %, 14.4 %, 17.0 % and 14.6 % off.
- **It only matters for towns.** A dressed crowd saves 5 to 7 % of its atlas, which is 0.7 MB of VRAM on Classic and 1.5 MB
  on Enhanced, and a creature crowd nothing. The cost is a second box per frame in the atlas and draw path. **Not worth
  it for VRAM alone,** as far as these crowds go.

### Which per-stream ideas help creatures, and which only help equipment

| Idea | Equipment | Creatures |
| --- | --- | --- |
| Removing zstd's optional fields | −0.4 / −0.1 % | −0.0 / −0.0 % |
| Duplicates | Enhanced: 8.32 MB across bodies. Classic: 0.41 MB | **Classic: 26.41 MB within bodies.** Enhanced: 4.0 MB |
| Best dictionary per body | −2.0 / −0.3 % | −1.2 / −0.0 % |
| Row spans (a) | −3.5 / −2.2 % | −3.5 / −3.2 % |
| Painted only + coverage spans (Enhanced) | −10.3 % | −9.5 % |
| Luma + left (Enhanced) | −5.9 % | −7.5 % |
| Coverage tables (Enhanced coverage plane, every pixel) | −30 % | −34 % |
| Split boxes | **The atlas: 18.6 / 12.3 %** | Nothing |

Only split boxes and optional fields are equipment ideas. **Everything that moves the disk helps creatures as much, and
creatures are most of the bytes:** 81 % of Classic's and 66 % of Enhanced's. Duplicates are the one row that splits by
client: Classic's are creatures within a body, and Enhanced's are equipment across bodies.

### Custom art and shipped statistics

No row in this section depends on shipped statistics: dictionaries and tANS tables are built per body from that body's
own streams, and spans, shapes and duplicate pointers come from the pixels. Two things to carry into a format:

- **Classic row spans and painted-only treat index 0 as empty.** A custom MUL entry that draws index 0 as opaque black
  needs the per-batch flag of "Custom MUL animations" in section 8 to turn these layouts off, or a span built from the
  entry's own runs.
- **Enhanced painted-only drops the index under coverage 0.** No such pixel can be seen, and the round-trip check found
  no shipped pixel that needs one on any body coded here. A format that must return such a pixel exactly would need the
  rectangle for that batch.


## 10. Coders per class (`tk`)

The user asked whether equipment and NPCs, and small, medium and large NPCs, want different coders. `t1` answers that
only for three coders, by kind and by stream size, and stream size isn't body size. `tk` classes bodies by the placeholder
rule, and its crowds are drawn by that rule, so they can differ slightly from `tc`'s. A Classic body kind in the final format is a
separate decision.

### The placeholder rule, run as one rule (`tk0`)

The rule looks up the tile `10000000 + body` directly by body id in `tileart.uop`:

1. **A wearer's map** is the body type of any placeholder that sets race (key 9).
2. **Equipment:** a placeholder with no race whose body type is a wearer's map.
3. **NPC:** every other placeholder, including those that set race.
4. **No placeholder:** `mobtypes.txt`, by id.

**The derived wearer set is {400, 666, 1253}, not {400, 666}.** Both clients' `tileart.uop` have 1143 placeholders, of which
13 set race (400 to 403, 605 to 608, 666, 667, 694, 695, 1253). Body 1253 is its own map and no race-less placeholder
plays it, so it changes no class: 496 race-less placeholders play 400, 170 play 666, none play 1253.

All pixels here are cropped boxes, which is why Classic UOP's shares differ from counts over declared boxes.

**Enhanced:** 926 bodies, 1572711 frames, 4916.9 Mpx.

| Rule class | `mobtypes.txt` kind | Bodies | % frames | % pixels |
| --- | --- | ---: | ---: | ---: |
| equipment (no race, a wearer's map) | EQUIPMENT | 547 | 81.24 | 20.82 |
| equipment (no race, a wearer's map) | HUMAN | 1 | 0.15 | 0.14 |
| equipment (no race, a wearer's map) | MONSTER | 1 | 0.15 | 0.05 |
| equipment (no race, a wearer's map) | not listed | 7 | 1.00 | 0.39 |
| NPC, sets race | HUMAN | 7 | 1.09 | 1.89 |
| NPC, other map | ANIMAL | 120 | 4.90 | 23.23 |
| NPC, other map | EQUIPMENT | 2 | 0.11 | 0.29 |
| NPC, other map | HUMAN | 1 | 0.02 | 0.02 |
| NPC, other map | MONSTER | 231 | 10.80 | 52.01 |
| NPC, other map | SEA_MONSTER | 3 | 0.11 | 0.38 |
| NPC, other map | not listed | 4 | 0.10 | 0.64 |
| no placeholder | EQUIPMENT | 2 | 0.32 | 0.16 |

**Classic UOP:** 276 bodies, 469725 frames, 1472.8 Mpx.

| Rule class | `mobtypes.txt` kind | Bodies | % frames | % pixels |
| --- | --- | ---: | ---: | ---: |
| equipment (no race, a wearer's map) | EQUIPMENT | 153 | 79.52 | 12.13 |
| NPC, sets race | HUMAN | 3 | 1.61 | 2.57 |
| NPC, other map | ANIMAL | 59 | 9.02 | 33.02 |
| NPC, other map | HUMAN | 1 | 0.07 | 0.05 |
| NPC, other map | MONSTER | 59 | 9.22 | 52.00 |
| no placeholder | EQUIPMENT | 1 | 0.55 | 0.23 |

**Classic MUL:** 746 bodies, 526895 frames, 946.2 Mpx.

| Rule class | `mobtypes.txt` kind | Bodies | % frames | % pixels |
| --- | --- | ---: | ---: | ---: |
| equipment (no race, a wearer's map) | EQUIPMENT | 364 | 71.70 | 18.52 |
| equipment (no race, a wearer's map) | HUMAN | 3 | 0.60 | 0.71 |
| equipment (no race, a wearer's map) | MONSTER | 1 | 0.05 | 0.10 |
| equipment (no race, a wearer's map) | not listed | 7 | 1.39 | 1.22 |
| NPC, sets race | HUMAN | 4 | 0.80 | 0.72 |
| NPC, other map | ANIMAL | 58 | 3.28 | 6.00 |
| NPC, other map | EQUIPMENT | 2 | 0.40 | 0.06 |
| NPC, other map | HUMAN | 1 | 0.20 | 0.21 |
| NPC, other map | MONSTER | 178 | 10.18 | 67.22 |
| NPC, other map | SEA_MONSTER | 2 | 0.16 | 1.18 |
| no placeholder | EQUIPMENT | 21 | 2.95 | 0.88 |
| no placeholder | HUMAN | 1 | 0.20 | 0.06 |
| no placeholder | MONSTER | 1 | 0.05 | 0.80 |
| no Bodyconv.def line | not listed | 103 | 8.04 | 2.32 |

| Source | Equipment: bodies, % frames, % pixels | NPC: bodies, % frames, % pixels | Not listed |
| --- | ---: | ---: | ---: |
| Enhanced | 558, 82.86, 21.55 | 368, 17.14, 78.45 | none |
| Classic UOP | 154, 80.08, 12.36 | 122, 19.92, 87.64 | none |
| Classic MUL | 396, 76.69, 21.43 | 247, 15.27, 76.25 | 103 bodies, 8.04, 2.32 |

Equipment plays 400 in 404 Enhanced bodies and 666 in 152. Classic UOP is the reverse (666 in 152, 400 in 1), and Classic MUL
plays 400 in 371 and 666 in 4.

With no placeholder, these use `mobtypes.txt`:

- **Enhanced:** 2 bodies, as `ENHANCED_CLIENT_FILES.md` says: 344 (2600 frames, 6.24 Mpx) and 827 (2400, 1.71), both
  EQUIPMENT. The earlier list of seven came from mapping by appearance.
- **Classic UOP:** 1 body: 344, 2600 frames, 3.40 Mpx, EQUIPMENT.
- **Classic MUL:** 23 bodies with an id, 16886 frames, 16.36 Mpx: 21 EQUIPMENT, 988 HUMAN, 796 MONSTER (7.52 Mpx). Another
  103 bodies in `anim2+.mul` have no `Bodyconv.def` line, so no id at all (42362 frames, 21.97 Mpx): those stay "not listed".

**Largest disagreements with `mobtypes.txt`, where it has a line:**

| Source | Bodies | Mpx | Which |
| --- | ---: | ---: | --- |
| Enhanced | 4 | 23.16 | 841 (rule NPC, listed EQUIPMENT, 7.91 Mpx), 987 (equipment, HUMAN, 6.81), 840 (NPC, EQUIPMENT, 6.20), 260 (equipment, MONSTER, 2.24) |
| Classic UOP | 0 | 0 | None. Inside NPCs, 990 is listed HUMAN but sets no race |
| Classic MUL | 6 | 8.26 | 994 (equipment, HUMAN, 2.68), 987 (2.05) and 991 (2.00) likewise; 774 (anim file 1 body 42, MONSTER, plays 666, 0.93); 840 (NPC, EQUIPMENT, 0.53); 839 (NPC, body type 187, EQUIPMENT, 0.07) |

Where `mobtypes.txt` has no line, the rule still decides: 11 Enhanced bodies (50.33 Mpx, the largest 105, as an NPC at 28.26
Mpx, and 970, as equipment at 6.65) and 7 Classic MUL bodies, all equipment (11.58 Mpx, the largest 573 at 6.82). So the rule
and the file agree on all but 0.5 % of Enhanced's and 0.9 % of Classic MUL's pixels, and on everything in Classic UOP.

### Results

Everything timed here is quiet: boost off, EPP `balance_power`, platform profile performance, nothing else running. Size columns aren't affected by that. `tk` draws its crowds through the rule above, not `mobtypes.txt`, so its milliseconds don't match section 9's to the digit. The dictionary rows in `tk`'s crowds pick each body's dictionary over the crowd's streams, which flatters them; `t_tc3` redoes that over the whole body and section 9 has the result. Up to 400 Mpx per class are coded, and equipment at most every 10th body, so whole-set figures scale by pixels.

**The answer:** on Classic a class rule adds nothing. On Enhanced it gains 2.4 % over the best single coder inside the line, by spending the time the small and medium classes leave free, and it leaves almost no margin.

| client | one coder inside the line | best assignment inside the line | best per stream with an id byte, coders that pass alone | no line at all: one coder | no line: per stream with an id byte |
|---|---|---|---|---|---|
| Classic | `a row spans, best dictionary per body`, 702.5 MB, 0.934 x zstd 19 | the same coder in all four classes, 702.5 MB | 702.4 MB, 0.934 | same coder, 702.5 MB | 681.6 MB, 0.907 at 25 MB/s |
| Enhanced | `b painted, coverage runs zstd 19, partial tANS by context`, 1810.8 MB, 0.896 | equipment, small, medium: `b painted, coverage every pixel tANS by context`; large: `b painted, coverage row spans, zstd 19`; 1767.4 MB, 0.874 | 1802.8 MB, 0.892 | rank, 1584.3 MB, 0.784 at 26 MB/s | 1554.8 MB, 0.769 at 30 MB/s |

- **Worst stream's decode time:** 4.30 ms on Classic's assignment and 3.74 ms on Enhanced's, on one thread. Those are below 16.7 ms, so no single stream stalls a frame even on the slower coders.
- **The crowd line, real, 4 threads, creatures five / faced, dressed NPCs five / faced:** Classic 48.0 / 10.6 / 6.5 / 1.4 ms. Enhanced 232.3 / 57.7 / 81.7 / 34.7 ms, which is 7 % under the line, on this machine's boost-off clock. That's the assignment's limit, not a comfortable pick: the search stops at the first that passes, and the next size down fails.
- **A class rule against a per-stream id:** on Enhanced, the class assignment beats the per-stream choice by 35 MB because the per-stream column may only use coders that pass alone, while an assignment may use a coder on one class that would fail on all of them. With no line, the per-stream id beats a class rule by 2.2 % on Classic (681.6 against 697.2 MB) and 1.8 % on Enhanced (1554.8 against 1584.0 MB), at decode speeds that fail the crowds.
- **Why large bodies stay on zstd 19:** they hold 59–63 % of the pixels, so they're the class that has to stay fast. Every-pixel tANS on them is what fails the line.
- **The uniform and worst crowds weren't part of this search.** On Enhanced's worst crowd every zstd row already misses the line (475–541 ms in section 9, about 4 times the creature crowd), so this assignment would too.

### Classic: bodies, streams, pixels

"Measured" is what this part coded.

| class | bodies | streams | Mpx | painted Mpx | bodies measured | streams measured | Mpx measured | painted Mpx measured |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| equipment | 550 | 104770 | 384.8 | 144.7 | 55 | 10145 | 38.6 | 13.0 |
| NPC small | 95 | 7072 | 61.9 | 27.5 | 95 | 7072 | 61.9 | 27.5 |
| NPC medium | 173 | 12235 | 432.5 | 176.7 | 87 | 6152 | 218.6 | 87.5 |
| NPC large | 101 | 7151 | 1517.8 | 569.9 | 26 | 1781 | 398.0 | 147.6 |
| not listed | 103 | 7198 | 22.0 | 11.1 | 103 | 7198 | 22.0 | 11.1 |

#### Classic: equipment

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| a row spans, best dictionary per body | 10.18 | 0.935 | 6.27 | 655 | 0.06 |
| a row spans, zstd 19 | 10.50 | 0.965 | 6.46 | 570 | 0.05 |
| rank + previous, rANS (reference) | 10.59 | 0.973 | 6.52 | 6 | 3.12 |
| split boxes, zstd 19 | 10.88 | 1.000 | 6.70 | 644 | 0.04 |
| zstd 19 | 10.88 | 1.000 | 6.70 | 631 | 0.04 |
| b painted, runs apart, zstd 19 | 11.00 | 1.011 | 6.77 | 513 | 0.05 |
| luma, MED + motion, tANS | 11.74 | 1.079 | 7.23 | 88 | 1.08 |
| luma, MED, 10 streams, tANS | 11.81 | 1.085 | 7.27 | 108 | 0.80 |
| luma, MED, zstd 19 | 11.95 | 1.098 | 7.36 | 138 | 0.37 |
| luma, up, tight | 12.06 | 1.108 | 7.42 | 355 | 0.06 |
| luma, left, tight | 12.25 | 1.125 | 7.54 | 395 | 0.06 |
| luma, MED, 10 streams, zstd 19 | 13.83 | 1.270 | 8.51 | 97 | 0.60 |
| a row spans, LZ4 HC | 14.30 | 1.314 | 8.80 | 1670 | 0.01 |
| split boxes, LZ4 HC | 15.00 | 1.378 | 9.24 | 3381 | 0.01 |
| LZ4 HC | 15.05 | 1.383 | 9.27 | 3454 | 0.01 |
| b painted, runs in the frame, LZ4 HC | 15.71 | 1.443 | 9.67 | 1392 | 0.02 |

#### Classic: NPC small

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| a row spans, best dictionary per body | 22.42 | 0.934 | 6.52 | 817 | 0.05 |
| a row spans, zstd 19 | 23.12 | 0.963 | 6.73 | 701 | 0.05 |
| rank + previous, rANS (reference) | 23.22 | 0.967 | 6.76 | 6 | 4.23 |
| b painted, runs apart, zstd 19 | 23.76 | 0.989 | 6.91 | 641 | 0.05 |
| zstd 19 | 24.01 | 1.000 | 6.99 | 787 | 0.04 |
| split boxes, zstd 19 | 24.01 | 1.000 | 6.99 | 782 | 0.04 |
| luma, MED + motion, tANS | 24.48 | 1.019 | 7.12 | 68 | 1.19 |
| luma, MED, zstd 19 | 25.61 | 1.067 | 7.45 | 127 | 0.44 |
| luma, MED, 10 streams, tANS | 25.76 | 1.073 | 7.49 | 82 | 0.95 |
| luma, up, tight | 26.08 | 1.086 | 7.59 | 442 | 0.06 |
| luma, left, tight | 26.21 | 1.092 | 7.63 | 495 | 0.06 |
| luma, MED, 10 streams, zstd 19 | 27.60 | 1.150 | 8.03 | 86 | 0.72 |
| a row spans, LZ4 HC | 28.76 | 1.198 | 8.37 | 2308 | 0.02 |
| split boxes, LZ4 HC | 30.07 | 1.252 | 8.75 | 4179 | 0.01 |
| LZ4 HC | 30.07 | 1.252 | 8.75 | 4253 | 0.01 |
| b painted, runs in the frame, LZ4 HC | 30.69 | 1.278 | 8.93 | 1846 | 0.02 |

#### Classic: NPC medium

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| a row spans, best dictionary per body | 65.11 | 0.916 | 5.95 | 1060 | 0.51 |
| rank + previous, rANS (reference) | 66.92 | 0.942 | 6.12 | 11 | 17.98 |
| a row spans, zstd 19 | 68.44 | 0.963 | 6.26 | 969 | 0.52 |
| b painted, runs apart, zstd 19 | 69.98 | 0.985 | 6.40 | 895 | 0.52 |
| luma, MED + motion, tANS | 71.04 | 1.000 | 6.49 | 61 | 8.24 |
| split boxes, zstd 19 | 71.04 | 1.000 | 6.49 | 1071 | 0.47 |
| zstd 19 | 71.05 | 1.000 | 6.49 | 1071 | 0.47 |
| luma, MED, zstd 19 | 75.34 | 1.060 | 6.89 | 116 | 4.29 |
| luma, MED, 10 streams, tANS | 76.07 | 1.071 | 6.95 | 73 | 7.92 |
| luma, left, tight | 76.22 | 1.073 | 6.97 | 756 | 0.58 |
| luma, up, tight | 76.58 | 1.078 | 7.00 | 661 | 0.72 |
| luma, MED, 10 streams, zstd 19 | 77.18 | 1.086 | 7.06 | 81 | 6.75 |
| a row spans, LZ4 HC | 82.37 | 1.159 | 7.53 | 3517 | 0.16 |
| split boxes, LZ4 HC | 85.55 | 1.204 | 7.82 | 5275 | 0.14 |
| LZ4 HC | 85.57 | 1.204 | 7.82 | 5297 | 0.15 |
| b painted, runs in the frame, LZ4 HC | 86.42 | 1.216 | 7.90 | 2805 | 0.17 |

#### Classic: NPC large

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| rank + previous, rANS (reference) | 108.64 | 0.925 | 5.89 | 22 | 122.59 |
| a row spans, best dictionary per body | 110.02 | 0.937 | 5.96 | 991 | 4.30 |
| a row spans, zstd 19 | 112.96 | 0.962 | 6.12 | 1006 | 4.26 |
| luma, MED + motion, tANS | 113.12 | 0.964 | 6.13 | 60 | 55.19 |
| b painted, runs apart, zstd 19 | 114.05 | 0.972 | 6.18 | 973 | 3.79 |
| zstd 19 | 117.39 | 1.000 | 6.36 | 993 | 4.27 |
| split boxes, zstd 19 | 117.39 | 1.000 | 6.36 | 992 | 4.22 |
| luma, MED, 10 streams, tANS | 121.19 | 1.032 | 6.57 | 70 | 48.05 |
| luma, MED, 10 streams, zstd 19 | 121.26 | 1.033 | 6.57 | 79 | 42.57 |
| luma, MED, zstd 19 | 122.93 | 1.047 | 6.66 | 107 | 31.66 |
| luma, left, tight | 123.29 | 1.050 | 6.68 | 812 | 4.78 |
| luma, up, tight | 125.82 | 1.072 | 6.82 | 748 | 4.64 |
| a row spans, LZ4 HC | 131.99 | 1.124 | 7.16 | 4037 | 1.22 |
| b painted, runs in the frame, LZ4 HC | 134.96 | 1.150 | 7.32 | 3297 | 1.39 |
| split boxes, LZ4 HC | 138.38 | 1.179 | 7.50 | 4323 | 1.31 |
| LZ4 HC | 138.38 | 1.179 | 7.50 | 4360 | 1.30 |

#### Classic: not listed

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| a row spans, best dictionary per body | 8.64 | 0.912 | 6.22 | 571 | 0.15 |
| rank + previous, rANS (reference) | 9.01 | 0.951 | 6.49 | 5 | 7.44 |
| a row spans, zstd 19 | 9.23 | 0.974 | 6.64 | 525 | 0.13 |
| zstd 19 | 9.47 | 1.000 | 6.82 | 602 | 0.12 |
| split boxes, zstd 19 | 9.48 | 1.001 | 6.82 | 596 | 0.12 |
| b painted, runs apart, zstd 19 | 9.50 | 1.003 | 6.84 | 472 | 0.13 |
| luma, MED + motion, tANS | 10.18 | 1.074 | 7.33 | 58 | 2.32 |
| luma, MED, 10 streams, tANS | 10.33 | 1.090 | 7.43 | 69 | 2.03 |
| luma, up, tight | 10.38 | 1.096 | 7.48 | 310 | 0.17 |
| luma, MED, zstd 19 | 10.40 | 1.097 | 7.48 | 104 | 1.25 |
| luma, left, tight | 10.50 | 1.108 | 7.56 | 342 | 0.15 |
| a row spans, LZ4 HC | 11.47 | 1.211 | 8.26 | 1757 | 0.03 |
| luma, MED, 10 streams, zstd 19 | 11.60 | 1.224 | 8.35 | 68 | 1.64 |
| split boxes, LZ4 HC | 11.96 | 1.263 | 8.61 | 3303 | 0.02 |
| LZ4 HC | 11.97 | 1.264 | 8.62 | 3449 | 0.02 |
| b painted, runs in the frame, LZ4 HC | 12.16 | 1.283 | 8.75 | 1458 | 0.03 |

#### Classic: each coder alone in the crowds, 4 threads, ms

| coder | creatures, five directions | creatures, faced direction | dressed NPCs, five directions | dressed NPCs, faced direction | all inside 250 ms |
|---|---:|---:|---:|---:|---|
| zstd 19 | 51.5 | 11.5 | 6.2 | 1.3 | yes |
| LZ4 HC | 15.2 | 3.2 | 1.4 | 0.4 | yes |
| luma, left, tight | 65.5 | 14.0 | 11.8 | 2.4 | yes |
| luma, up, tight | 70.2 | 15.1 | 12.1 | 2.6 | yes |
| a row spans, zstd 19 | 48.2 | 10.5 | 6.7 | 1.5 | yes |
| a row spans, LZ4 HC | 15.3 | 3.3 | 2.6 | 0.6 | yes |
| a row spans, best dictionary per body | 48.9 | 11.1 | 6.7 | 1.5 | yes |
| b painted, runs apart, zstd 19 | 52.7 | 11.8 | 8.1 | 1.8 | yes |
| b painted, runs in the frame, LZ4 HC | 20.4 | 4.5 | 4.1 | 1.0 | yes |
| split boxes, zstd 19 | 52.7 | 11.1 | 6.2 | 1.3 | yes |
| split boxes, LZ4 HC | 14.8 | 3.3 | 1.4 | 0.4 | yes |
| luma, MED, zstd 19 | 374.6 | 78.5 | 35.6 | 7.6 | no |
| luma, MED, 10 streams, zstd 19 | 506.6 | 104.5 | 51.0 | 10.8 | no |
| luma, MED, 10 streams, tANS | 582.2 | 132.2 | 64.6 | 24.1 | no |
| luma, MED + motion, tANS | 674.7 | 156.0 | 80.8 | 32.2 | no |
| rank + previous, rANS (reference) | 2150.5 | 441.6 | 371.7 | 81.8 | no |

#### Classic: one coder, the best per class, the best per stream

| way | coders | measured bodies MB | palette table, whole set MB | whole set estimate MB | x zstd 19 | decode MB/s, one thread | largest stream ms |
|---|---|---:|---:|---:|---:|---:|---:|
| one coder | zstd 19 | 232.8 | 21.5 | 751.8 | 1.000 | 946 | 4.27 |
| one coder, any coder | a row spans, best dictionary per body | 216.4 | 21.5 | 702.5 | 0.934 | 946 | 4.30 |
| best per class, any coder | equipment: a row spans, best dictionary per body; NPC small: a row spans, best dictionary per body; NPC medium: a row spans, best dictionary per body; NPC large: rank + previous, rANS (reference); not listed: a row spans, best dictionary per body | 215.0 | 21.5 | 697.2 | 0.927 | 39 | 122.59 |
| best per stream, an id byte each, any coder | as the stream prefers | 209.3 | 21.9 | 681.6 | 0.907 | 25 | 74.58 |
| one coder, coders that keep all crowds inside the line alone | a row spans, best dictionary per body | 216.4 | 21.5 | 702.5 | 0.934 | 946 | 4.30 |
| best per class, coders that keep all crowds inside the line alone | equipment: a row spans, best dictionary per body; NPC small: a row spans, best dictionary per body; NPC medium: a row spans, best dictionary per body; NPC large: a row spans, best dictionary per body; not listed: a row spans, best dictionary per body | 216.4 | 21.5 | 702.5 | 0.934 | 946 | 4.30 |
| best per stream, an id byte each, coders that keep all crowds inside the line alone | as the stream prefers | 216.3 | 21.6 | 702.4 | 0.934 | 943 | 4.26 |

streams picking each, any coder: rank + previous, rANS (reference) 48.7 %; a row spans, best dictionary per body 44.5 %; luma, MED + motion, tANS 2.3 %; a row spans, zstd 19 1.9 %; zstd 19 1.0 %; luma, MED, 10 streams, tANS 0.6 %; split boxes, zstd 19 0.5 %; LZ4 HC 0.3 %; b painted, runs apart, zstd 19 0.2 %; luma, up, tight 0.0 %; luma, MED, zstd 19 0.0 %; luma, left, tight 0.0 %; a row spans, LZ4 HC 0.0 %.

streams picking each, coders that keep all crowds inside the line alone: a row spans, best dictionary per body 86.4 %; a row spans, zstd 19 6.0 %; zstd 19 3.8 %; split boxes, zstd 19 1.3 %; b painted, runs apart, zstd 19 1.0 %; LZ4 HC 0.8 %; luma, up, tight 0.6 %; a row spans, LZ4 HC 0.1 %; luma, left, tight 0.1 %; b painted, runs in the frame, LZ4 HC 0.0 %; split boxes, LZ4 HC 0.0 %.

#### Classic: the smallest assignment inside the line

65536 assignments in disk order.

| assignment | whole set estimate MB | x zstd 19 | prediction ms, worst check | real 4 threads ms, per check | passes |
|---|---:|---:|---:|---|---|
| equipment: a row spans, best dictionary per body; NPC small: a row spans, best dictionary per body; NPC medium: a row spans, best dictionary per body; NPC large: a row spans, best dictionary per body | 702.5 | 0.934 | 46.6 | 48.0 / 10.6 / 6.5 / 1.4 | yes |


| way | coders | measured bodies MB | palette table, whole set MB | whole set estimate MB | x zstd 19 | decode MB/s, one thread | largest stream ms |
|---|---|---:|---:|---:|---:|---:|---:|
| assignment | equipment: a row spans, best dictionary per body; NPC small: a row spans, best dictionary per body; NPC medium: a row spans, best dictionary per body; NPC large: a row spans, best dictionary per body | 216.4 | 21.5 | 702.5 | 0.934 | 946 | 4.30 |

### Enhanced: bodies, streams, pixels

"Measured" is what this part coded.

| class | bodies | streams | Mpx | painted Mpx | bodies measured | streams measured | Mpx measured | painted Mpx measured |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| equipment | 558 | 130028 | 1059.6 | 414.3 | 56 | 13187 | 105.6 | 42.7 |
| NPC small | 51 | 3740 | 64.4 | 26.4 | 51 | 3740 | 64.4 | 26.4 |
| NPC medium | 187 | 14364 | 873.8 | 354.7 | 63 | 4971 | 300.0 | 121.2 |
| NPC large | 130 | 10001 | 2919.1 | 1044.9 | 17 | 1395 | 413.1 | 144.9 |
| not listed | 0 | 0 | 0.0 | 0.0 | 0 | 0 | 0.0 | 0.0 |

#### Enhanced: equipment

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| rank + previous, rANS (reference) | 41.49 | 0.795 | 7.78 | 11 | 6.05 |
| b painted, coverage every pixel tANS by context | 43.08 | 0.825 | 8.08 | 198 | 1.05 |
| luma, left, tight, coverage every pixel tANS by context | 44.36 | 0.850 | 8.32 | 196 | 1.07 |
| luma, MED + motion, tANS | 45.05 | 0.863 | 8.45 | 140 | 1.56 |
| b painted, coverage runs zstd 19, partial tANS by context | 45.90 | 0.879 | 8.61 | 539 | 0.50 |
| b painted, coverage row spans, zstd 19 | 46.49 | 0.891 | 8.72 | 807 | 0.15 |
| b painted, zstd 19 | 47.07 | 0.902 | 8.83 | 891 | 0.15 |
| c both planes in row spans, zstd 19 | 47.71 | 0.914 | 8.95 | 932 | 0.11 |
| luma, left, tight, coverage row spans | 47.77 | 0.915 | 8.96 | 783 | 0.18 |
| luma, MED, zstd 19 | 48.05 | 0.921 | 9.01 | 264 | 0.66 |
| zstd 19, coverage every pixel tANS by context | 48.21 | 0.924 | 9.04 | 201 | 1.02 |
| luma, left, tight | 48.35 | 0.926 | 9.07 | 862 | 0.18 |
| luma, up, tight | 48.53 | 0.930 | 9.10 | 735 | 0.21 |
| luma, MED, 10 streams, tANS | 49.09 | 0.940 | 9.21 | 171 | 1.25 |
| luma, MED, 10 streams, zstd 19 | 51.01 | 0.977 | 9.57 | 186 | 0.97 |
| a row spans, zstd 19 | 51.02 | 0.977 | 9.57 | 892 | 0.12 |
| split boxes, zstd 19 | 52.02 | 0.997 | 9.75 | 949 | 0.12 |
| zstd 19 | 52.20 | 1.000 | 9.79 | 929 | 0.12 |
| b painted, coverage row spans, LZ4 HC | 58.21 | 1.115 | 10.92 | 1725 | 0.06 |
| b painted, LZ4 HC | 59.15 | 1.133 | 11.09 | 2230 | 0.06 |
| c both planes in row spans, LZ4 HC | 60.13 | 1.152 | 11.28 | 2624 | 0.03 |
| a row spans, LZ4 HC | 64.63 | 1.238 | 12.12 | 2914 | 0.03 |
| split boxes, LZ4 HC | 67.41 | 1.291 | 12.64 | 4641 | 0.02 |
| LZ4 HC | 68.04 | 1.303 | 12.76 | 4529 | 0.02 |

#### Enhanced: NPC small

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| b painted, coverage every pixel tANS by context | 24.80 | 0.818 | 7.52 | 197 | 2.65 |
| rank + previous, rANS (reference) | 25.09 | 0.827 | 7.60 | 15 | 17.06 |
| luma, left, tight, coverage every pixel tANS by context | 25.92 | 0.854 | 7.85 | 192 | 2.73 |
| luma, MED + motion, tANS | 26.34 | 0.868 | 7.98 | 142 | 6.91 |
| b painted, coverage runs zstd 19, partial tANS by context | 26.65 | 0.879 | 8.08 | 509 | 2.42 |
| b painted, coverage row spans, zstd 19 | 26.75 | 0.882 | 8.11 | 943 | 0.30 |
| b painted, zstd 19 | 27.23 | 0.898 | 8.25 | 1021 | 0.34 |
| c both planes in row spans, zstd 19 | 27.73 | 0.914 | 8.40 | 1131 | 0.18 |
| luma, left, tight, coverage row spans | 27.87 | 0.919 | 8.45 | 895 | 0.42 |
| zstd 19, coverage every pixel tANS by context | 27.90 | 0.920 | 8.46 | 204 | 2.50 |
| luma, MED, zstd 19 | 27.98 | 0.922 | 8.48 | 291 | 2.32 |
| luma, up, tight | 28.26 | 0.932 | 8.56 | 802 | 0.52 |
| luma, MED, 10 streams, tANS | 28.31 | 0.933 | 8.58 | 178 | 6.48 |
| luma, left, tight | 28.35 | 0.934 | 8.59 | 964 | 0.42 |
| luma, MED, 10 streams, zstd 19 | 29.07 | 0.958 | 8.81 | 199 | 4.99 |
| a row spans, zstd 19 | 29.51 | 0.973 | 8.94 | 1092 | 0.22 |
| zstd 19 | 30.34 | 1.000 | 9.19 | 1139 | 0.21 |
| split boxes, zstd 19 | 30.34 | 1.000 | 9.19 | 1141 | 0.20 |
| b painted, coverage row spans, LZ4 HC | 33.53 | 1.105 | 10.16 | 1922 | 0.21 |
| b painted, LZ4 HC | 34.21 | 1.128 | 10.37 | 2387 | 0.19 |
| c both planes in row spans, LZ4 HC | 34.91 | 1.151 | 10.58 | 3072 | 0.09 |
| a row spans, LZ4 HC | 37.26 | 1.228 | 11.29 | 3413 | 0.08 |
| split boxes, LZ4 HC | 39.02 | 1.286 | 11.83 | 4997 | 0.05 |
| LZ4 HC | 39.03 | 1.286 | 11.83 | 4965 | 0.05 |

#### Enhanced: NPC medium

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| rank + previous, rANS (reference) | 105.69 | 0.772 | 6.97 | 26 | 10.62 |
| luma, MED + motion, tANS | 115.92 | 0.847 | 7.65 | 116 | 4.13 |
| b painted, coverage every pixel tANS by context | 115.95 | 0.847 | 7.65 | 190 | 2.72 |
| luma, left, tight, coverage every pixel tANS by context | 117.08 | 0.855 | 7.73 | 187 | 2.78 |
| b painted, coverage runs zstd 19, partial tANS by context | 122.58 | 0.895 | 8.09 | 570 | 1.32 |
| luma, MED, 10 streams, tANS | 123.23 | 0.900 | 8.13 | 139 | 3.29 |
| b painted, coverage row spans, zstd 19 | 123.70 | 0.903 | 8.16 | 1063 | 0.51 |
| luma, MED, zstd 19 | 123.80 | 0.904 | 8.17 | 220 | 1.78 |
| luma, left, tight, coverage row spans | 124.83 | 0.912 | 8.24 | 999 | 0.59 |
| luma, MED, 10 streams, zstd 19 | 124.89 | 0.912 | 8.24 | 156 | 2.76 |
| b painted, zstd 19 | 125.57 | 0.917 | 8.29 | 1064 | 0.52 |
| luma, up, tight | 126.65 | 0.925 | 8.36 | 847 | 0.68 |
| luma, left, tight | 126.69 | 0.925 | 8.36 | 986 | 0.59 |
| zstd 19, coverage every pixel tANS by context | 127.32 | 0.930 | 8.40 | 197 | 2.63 |
| c both planes in row spans, zstd 19 | 127.70 | 0.933 | 8.43 | 1363 | 0.31 |
| a row spans, zstd 19 | 132.80 | 0.970 | 8.76 | 1297 | 0.39 |
| zstd 19 | 136.93 | 1.000 | 9.04 | 1265 | 0.39 |
| split boxes, zstd 19 | 136.93 | 1.000 | 9.04 | 1266 | 0.38 |
| b painted, coverage row spans, LZ4 HC | 148.47 | 1.084 | 9.80 | 2458 | 0.30 |
| b painted, LZ4 HC | 150.64 | 1.100 | 9.94 | 2697 | 0.31 |
| c both planes in row spans, LZ4 HC | 153.31 | 1.120 | 10.12 | 4654 | 0.12 |
| a row spans, LZ4 HC | 159.54 | 1.165 | 10.53 | 5040 | 0.11 |
| split boxes, LZ4 HC | 164.90 | 1.204 | 10.88 | 6366 | 0.08 |
| LZ4 HC | 164.92 | 1.204 | 10.88 | 6369 | 0.08 |

#### Enhanced: NPC large

| coder | MB | x zstd 19 | bits per painted pixel | decode MB/s, one thread | largest stream ms |
|---|---:|---:|---:|---:|---:|
| rank + previous, rANS (reference) | 113.48 | 0.775 | 6.26 | 46 | 86.46 |
| b painted, coverage every pixel tANS by context | 127.13 | 0.868 | 7.02 | 210 | 22.50 |
| luma, MED + motion, tANS | 127.47 | 0.870 | 7.04 | 131 | 40.08 |
| luma, left, tight, coverage every pixel tANS by context | 130.43 | 0.891 | 7.20 | 209 | 22.40 |
| b painted, coverage runs zstd 19, partial tANS by context | 132.09 | 0.902 | 7.29 | 789 | 5.06 |
| b painted, coverage row spans, zstd 19 | 132.95 | 0.908 | 7.34 | 1159 | 3.74 |
| luma, MED, 10 streams, tANS | 133.04 | 0.908 | 7.34 | 152 | 32.25 |
| luma, MED, 10 streams, zstd 19 | 133.71 | 0.913 | 7.38 | 174 | 27.85 |
| b painted, zstd 19 | 134.95 | 0.921 | 7.45 | 1164 | 3.78 |
| luma, MED, zstd 19 | 135.57 | 0.926 | 7.48 | 231 | 21.36 |
| luma, left, tight, coverage row spans | 136.26 | 0.930 | 7.52 | 1138 | 3.85 |
| c both planes in row spans, zstd 19 | 137.60 | 0.940 | 7.60 | 1483 | 3.48 |
| luma, left, tight | 138.26 | 0.944 | 7.63 | 1142 | 3.87 |
| luma, up, tight | 138.51 | 0.946 | 7.65 | 1034 | 4.02 |
| zstd 19, coverage every pixel tANS by context | 138.64 | 0.947 | 7.65 | 215 | 22.28 |
| a row spans, zstd 19 | 141.22 | 0.964 | 7.79 | 1411 | 3.52 |
| zstd 19 | 146.46 | 1.000 | 8.08 | 1339 | 3.61 |
| split boxes, zstd 19 | 146.47 | 1.000 | 8.08 | 1330 | 3.64 |
| b painted, coverage row spans, LZ4 HC | 156.68 | 1.070 | 8.65 | 2536 | 1.85 |
| b painted, LZ4 HC | 159.05 | 1.086 | 8.78 | 2570 | 1.83 |
| c both planes in row spans, LZ4 HC | 161.84 | 1.105 | 8.93 | 5299 | 1.08 |
| a row spans, LZ4 HC | 166.38 | 1.136 | 9.18 | 5179 | 1.10 |
| split boxes, LZ4 HC | 172.90 | 1.180 | 9.54 | 5172 | 1.11 |
| LZ4 HC | 172.91 | 1.181 | 9.54 | 5253 | 1.10 |

#### Enhanced: each coder alone in the crowds, 4 threads, ms

| coder | creatures, five directions | creatures, faced direction | dressed NPCs, five directions | dressed NPCs, faced direction | all inside 250 ms |
|---|---:|---:|---:|---:|---|
| zstd 19 | 102.4 | 21.0 | 15.2 | 3.2 | yes |
| LZ4 HC | 31.5 | 6.7 | 4.1 | 0.9 | yes |
| luma, left, tight | 124.1 | 25.4 | 21.9 | 4.5 | yes |
| luma, up, tight | 131.4 | 27.0 | 23.3 | 4.8 | yes |
| a row spans, zstd 19 | 93.9 | 19.4 | 14.8 | 3.1 | yes |
| a row spans, LZ4 HC | 31.3 | 6.6 | 5.2 | 1.1 | yes |
| luma, left, tight, coverage row spans | 118.3 | 24.3 | 21.8 | 4.5 | yes |
| luma, left, tight, coverage every pixel tANS by context | 540.3 | 126.2 | 83.9 | 34.9 | no |
| zstd 19, coverage every pixel tANS by context | 520.2 | 122.1 | 78.0 | 33.7 | no |
| b painted, zstd 19 | 114.4 | 23.6 | 18.8 | 3.8 | yes |
| b painted, coverage row spans, zstd 19 | 113.9 | 23.5 | 19.4 | 4.0 | yes |
| b painted, coverage every pixel tANS by context | 536.9 | 125.5 | 81.7 | 34.5 | no |
| b painted, coverage runs zstd 19, partial tANS by context | 195.3 | 55.4 | 48.6 | 25.5 | yes |
| b painted, LZ4 HC | 59.6 | 12.3 | 10.4 | 2.2 | yes |
| b painted, coverage row spans, LZ4 HC | 60.9 | 12.7 | 11.6 | 2.5 | yes |
| c both planes in row spans, zstd 19 | 87.9 | 18.1 | 14.0 | 2.9 | yes |
| c both planes in row spans, LZ4 HC | 30.3 | 6.5 | 5.6 | 1.2 | yes |
| split boxes, zstd 19 | 98.0 | 20.1 | 14.5 | 3.0 | yes |
| split boxes, LZ4 HC | 30.7 | 6.4 | 3.9 | 0.8 | yes |
| luma, MED, zstd 19 | 451.9 | 91.4 | 52.2 | 10.7 | no |
| luma, MED, 10 streams, zstd 19 | 598.0 | 120.5 | 70.4 | 14.4 | no |
| luma, MED, 10 streams, tANS | 688.0 | 149.3 | 88.6 | 28.3 | no |
| luma, MED + motion, tANS | 820.1 | 181.0 | 109.1 | 36.5 | no |
| rank + previous, rANS (reference) | 2698.3 | 551.9 | 617.9 | 134.4 | no |

#### Enhanced: one coder, the best per class, the best per stream

| way | coders | measured bodies MB | palette table, whole set MB | whole set estimate MB | x zstd 19 | decode MB/s, one thread | largest stream ms |
|---|---|---:|---:|---:|---:|---:|---:|
| one coder | zstd 19 | 365.9 | 33.3 | 2021.1 | 1.000 | 1234 | 3.61 |
| one coder, any coder | rank + previous, rANS (reference) | 285.7 | 33.3 | 1584.3 | 0.784 | 26 | 86.46 |
| best per class, any coder | equipment: rank + previous, rANS (reference); NPC small: b painted, coverage every pixel tANS by context; NPC medium: rank + previous, rANS (reference); NPC large: rank + previous, rANS (reference); not listed: zstd 19 | 285.5 | 33.3 | 1584.0 | 0.784 | 29 | 86.46 |
| best per stream, an id byte each, any coder | as the stream prefers | 278.7 | 35.6 | 1554.8 | 0.769 | 30 | 86.46 |
| one coder, coders that keep all crowds inside the line alone | b painted, coverage runs zstd 19, partial tANS by context | 327.2 | 33.3 | 1810.8 | 0.896 | 643 | 5.06 |
| best per class, coders that keep all crowds inside the line alone | equipment: b painted, coverage runs zstd 19, partial tANS by context; NPC small: b painted, coverage runs zstd 19, partial tANS by context; NPC medium: b painted, coverage runs zstd 19, partial tANS by context; NPC large: b painted, coverage runs zstd 19, partial tANS by context; not listed: zstd 19 | 327.2 | 33.3 | 1810.8 | 0.896 | 643 | 5.06 |
| best per stream, an id byte each, coders that keep all crowds inside the line alone | as the stream prefers | 323.7 | 40.7 | 1802.8 | 0.892 | 760 | 5.06 |

streams picking each, any coder: rank + previous, rANS (reference) 75.6 %; luma, MED + motion, tANS 12.2 %; b painted, coverage every pixel tANS by context 10.8 %; b painted, coverage row spans, zstd 19 0.6 %; b painted, zstd 19 0.4 %; b painted, coverage runs zstd 19, partial tANS by context 0.2 %; luma, left, tight, coverage every pixel tANS by context 0.1 %; luma, MED, 10 streams, tANS 0.0 %; c both planes in row spans, zstd 19 0.0 %; luma, up, tight 0.0 %; zstd 19 0.0 %; LZ4 HC 0.0 %; luma, left, tight 0.0 %; luma, left, tight, coverage row spans 0.0 %; b painted, LZ4 HC 0.0 %; luma, MED, zstd 19 0.0 %.

streams picking each, coders that keep all crowds inside the line alone: b painted, coverage runs zstd 19, partial tANS by context 60.3 %; luma, left, tight, coverage row spans 14.4 %; b painted, coverage row spans, zstd 19 10.5 %; b painted, zstd 19 7.0 %; luma, up, tight 6.7 %; luma, left, tight 0.8 %; b painted, LZ4 HC 0.2 %; c both planes in row spans, zstd 19 0.1 %; LZ4 HC 0.0 %; split boxes, zstd 19 0.0 %; zstd 19 0.0 %.

#### Enhanced: the smallest assignment inside the line

331776 assignments in disk order.

| assignment | whole set estimate MB | x zstd 19 | prediction ms, worst check | real 4 threads ms, per check | passes |
|---|---:|---:|---:|---|---|
| equipment: b painted, coverage every pixel tANS by context; NPC small: b painted, coverage every pixel tANS by context; NPC medium: b painted, coverage every pixel tANS by context; NPC large: b painted, coverage row spans, zstd 19 | 1767.4 | 0.874 | 229.7 | 232.3 / 57.7 / 81.7 / 34.7 | yes |


| way | coders | measured bodies MB | palette table, whole set MB | whole set estimate MB | x zstd 19 | decode MB/s, one thread | largest stream ms |
|---|---|---:|---:|---:|---:|---:|---:|
| assignment | equipment: b painted, coverage every pixel tANS by context; NPC small: b painted, coverage every pixel tANS by context; NPC medium: b painted, coverage every pixel tANS by context; NPC large: b painted, coverage row spans, zstd 19 | 316.8 | 33.3 | 1767.4 | 0.874 | 316 | 3.74 |
## 11. VRAM

Sizes only, from `MOBILE_ANIM_CODEC_SHRINK_PARTS=v` on the whole set (the tiny tests, not timed), run at idle priority. Everything the GPU holds is counted: pixels, per-frame and per-row tables, tile maps, palettes. A frame entry is 16 bytes in every layout. A palette is 1024 bytes, once per distinct palette in scope. A crowd counts each distinct stream once. BC7 and BC1 hold no palette, and their masks are in the hue table. **Nothing here decides the format; the user does.** Shader cost is in section 15.

**What the numbers say:**

- **Classic's R8 is the same size as BC7 (0.98×), and BC1 or row spans halve it** (0.52×, 0.56×). Enhanced's RG8 is the outlier: 1.93× BC7. Its second plane is what a block format gets back, because coverage is mostly 0 or 16.
- **Row spans in linear memory reach BC7's size on Enhanced (0.97× BC7) and 0.55× BC7 on Classic.** They carry a row table (smallest entry that works: 3 or 4 bytes for 99.3 % of frames, 127.8 or 160.0 bytes a frame on average, 141.2 or 172.6 bytes if entries must be 4 or 8 bytes for one fetch). They can't use a hardware sampler.
- **Painted only isn't worth its tables on Classic.** With a row entry and a 32-pixel word (3 fetches) it's larger than row spans; runs searched save 4 % at 3–8 fetches in the worst row of a body. On Enhanced it's 2 % under row spans with 3 fetches, 12 % with the search.
- **Dicing only beats row spans with no gutter**, and without a gutter hardware filtering doesn't work, so it has the same shader problem as row spans with more tables. With a 1 px gutter, 8×8 costs 0.94× and 0.95× of the rectangle: the gutter takes back almost all it saved.
- **R8 + BC4 coverage isn't lossless.** 7.61 % of 4×4 blocks don't round back to their 0–16 levels, 0.9971 % of pixels change level (0.01111 levels on average, worst 8 of 16), and 10548812 pixels change between transparent and painted. It's also 1.47× BC7.
- **Masks erase nothing on Enhanced, but they would on Classic.** Enhanced: 91.8 % of frames need no mask, and R8 masks add 457.5 MB (+9 % on BC7, +18 % on BC1). Classic: if a block format must hue greys under a partial hue, 79.4 % of frames are masked. R8 masks add 2132.9 MB, so BC1 + masks (3391.7 MB) is larger than plain R8 (2419.0 MB). Only one-bit masks (279.2 MB, lossy) keep BC1 under R8: 1537.9 MB. **Classic packs no mask today.**
- **Every layout's real saving comes second to what's loaded.** A faced-only load is 5× smaller than five directions and about 50× smaller than all actions of a body. Whole pages undo that: under today's reader API a dressed-NPC crowd's 3.0 MB of Enhanced frames holds 804 MB of pages, 269× as much.

### The atlas figures, reconciled

Earlier revisions of this document said the old package's pages are 7682.9 MB with 73 % of the atlas area empty, and also that Enhanced's BC7 pixels are 5106 MB, which would mean about 34 % empty. Both figures are true, and they answer different questions. On Enhanced, the simulated packer's used page area is 6946.6 Mpx, at one byte a pixel in BC7:

| Enhanced, today's packer | Mpx | share of used page area |
|---|---:|---:|
| used page area (2339 pages; full page area 7021.0 Mpx) | 6946.6 | 100 % |
| frames padded to 4×4 blocks | 5106.4 | 73.5 % |
| frame boxes | 4916.9 | 70.8 % |
| painted pixels | 1840.2 | 26.5 % |

- **"73 % empty" meant no painted pixel.** It counts transparent pixels inside a frame's box as empty. Those are in the 5106 MB too, so it's not the packer's waste. It's what row spans and painted-only layouts address.
- **The packer's own waste is 26.5 %** of used page area (1 − 5106.4 / 6946.6): gutters and packing holes.
- **The 7682.9 MB is BC7 pages plus R8 hue mask pages.** The simulation gives 6946.6 MB of BC7 on 2339 pages, plus 777.7 MB of masks on 231 pages: 7724.4 MB, 0.5 % over the real pack's file table (7682.9 MB in 2522 entries). The real pack has been deleted, so I didn't trace that difference. The 5106 MB counts neither gutters nor masks.
- **Classic:** 1258 pages, 3467.4 MB of BC7, frame boxes 69.8 %, padded 72.6 %, painted 26.8 %. `mobile_anim_cc` packs no hue mask, so the 3268.7 MB of masks the simulation prints (6736.2 MB together) is hypothetical.

**Would the renderer hold whole pages?** Under today's reader API, yes. `read_page_bytes`, `read_page_rgba` and `AtlasPageCache` work per page, and `read_frame_rgba` decodes the whole page, so an upload holds whole pages. `dynamapper/src` doesn't use these packages yet, so this is what the API implies, not an observed renderer. The page holds frames from sorted body and frame order, so a crowd touches many pages and holds much more than it draws (table below). A dynamic atlas fixes that, whatever the pixel format.

**Atlas waste with a dynamic allocator** (one frame at a time on 2048 pages, 1 px gutter): `guillotiere` 86.6 % frames, shelf 82.4 % on Classic, 86.7 % and 82.3 % on Enhanced. So a real allocator spends about 15 % on gutters and holes, half of today's packer. Treating a stream as one block costs 44 % more pages on Classic and 38 % more on Enhanced. **Churn** adds about 19 % on top of the largest fresh pack with `guillotiere` at 2048 (43 pages against 36 on Classic, 69 against 59 on Enhanced). LRU rounds that end on the small crowd hold pages kept for the large one, so the table's "live's share" of 2–4 % is retained pages, not holes, and a renderer could release them.

#### What the GPU holds, the whole set

Rectangle is R8 on Classic and RG8 on Enhanced. Each client: pixels, tables, palettes, total in MB, then total against its rectangle and against its BC7.

| layout | Classic pixels | tables | palettes | total | x rectangle | x BC7 | Enhanced pixels | tables | palettes | total | x rectangle | x BC7 | fetches | hardware sampler |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| rectangle | 2419.0 | 15.9 | 39.54 | 2474.5 | 1.000 | 0.977 | 9833.8 | 25.2 | 32.39 | 9891.3 | 1.000 | 1.928 | 1, then the palette | nearest only, a palette lookup after it |
| BC7, padded to 4 x 4 | 2517.5 | 15.9 | 0.00 | 2533.4 | 1.024 | 1.000 | 5106.4 | 25.2 | 0.00 | 5131.6 | 0.519 | 1.000 | 1 | yes |
| BC1, padded to 4 x 4 | 1258.7 | 15.9 | 0.00 | 1274.7 | 0.515 | 0.503 | 2553.2 | 25.2 | 0.00 | 2578.4 | 0.261 | 0.502 | 1 | yes |
| R8 index + BC4 coverage | - | - | - | - | - | - | 7470.1 | 25.2 | 32.39 | 7527.7 | 0.761 | 1.467 | 2, then the palette | nearest only, a palette lookup after it |
| row spans, linear memory | 1190.0 | 156.6 | 39.54 | 1386.2 | 0.560 | 0.547 | 4661.6 | 296.6 | 32.39 | 4990.6 | 0.505 | 0.973 | 2, then the palette | no: explicit fetches in the shader |
| painted only, row entry + 32-pixel word + pixel | 929.9 | 611.3 | 39.54 | 1580.8 | 0.639 | 0.624 | 3680.5 | 1175.1 | 32.39 | 4887.9 | 0.494 | 0.953 | 3, then the palette | no: explicit fetches in the shader |
| painted only, runs searched in the row | 929.9 | 365.3 | 39.54 | 1334.7 | 0.539 | 0.527 | 3680.5 | 689.0 | 32.39 | 4401.9 | 0.445 | 0.858 | row + log2(runs) + pixel, then the palette | no: explicit fetches in the shader |

**Dicing into tiles** (non-empty tiles plus the tile map; total MB, then against the rectangle). Every row needs a tile map fetch and then a tile fetch, and hardware filtering works only with the gutter.

| tile | dedupe | Classic, no gutter | Classic, 1 px gutter | Enhanced, no gutter | Enhanced, 1 px gutter |
|---|---|---:|---:|---:|---:|
| 8 x 8 | no dedupe | 1633.4 (0.660) | 2474.3 (1.000) | 6091.1 (0.616) | 9390.5 (0.949) |
| 8 x 8 | dedupe in a stream | 1610.4 (0.651) | 2438.4 (0.985) | 6047.9 (0.611) | 9323.1 (0.943) |
| 8 x 8 | dedupe in a body | 1547.3 (0.625) | 2315.7 (0.936) | 6128.8 (0.620) | 9387.2 (0.949) |
| 16 x 16 | no dedupe | 2090.4 (0.845) | 2624.8 (1.061) | 7924.7 (0.801) | 10002.2 (1.011) |
| 16 x 16 | dedupe in a stream | 2073.4 (0.838) | 2603.3 (1.052) | 7896.5 (0.798) | 9966.4 (1.008) |
| 16 x 16 | dedupe in a body | 1969.1 (0.796) | 2469.9 (0.998) | 7893.8 (0.798) | 9959.9 (1.007) |
| 32 x 32 | no dedupe | 3087.0 (1.248) | 3476.9 (1.405) | 11658.8 (1.179) | 13152.4 (1.330) |
| 32 x 32 | dedupe in a stream | 3067.6 (1.240) | 3454.9 (1.396) | 11636.9 (1.176) | 13127.7 (1.327) |
| 32 x 32 | dedupe in a body | 2934.5 (1.186) | 3304.6 (1.335) | 11625.3 (1.175) | 13114.5 (1.326) |

#### By class: total MB (x rectangle)

**Classic**

| layout | equipment | NPC small | NPC medium | NPC large | not listed |
|---|---:|---:|---:|---:|---:|
| rectangle | 430.5 (1.000) | 64.6 (1.000) | 436.5 (1.000) | 1519.7 (1.000) | 23.1 (1.000) |
| BC7, padded to 4 x 4 | 445.5 (1.035) | 67.4 (1.043) | 451.6 (1.035) | 1543.8 (1.016) | 25.0 (1.086) |
| BC1, padded to 4 x 4 | 229.0 (0.532) | 34.1 (0.527) | 226.5 (0.519) | 772.3 (0.508) | 12.9 (0.558) |
| row spans, linear memory | 284.9 (0.662) | 42.0 (0.650) | 249.8 (0.572) | 792.0 (0.521) | 17.5 (0.759) |
| painted only, row entry + 32-pixel word + pixel | 389.0 (0.903) | 53.1 (0.822) | 285.1 (0.653) | 830.6 (0.547) | 23.0 (0.996) |
| painted only, runs searched in the row | 340.7 (0.791) | 47.8 (0.739) | 249.1 (0.571) | 677.2 (0.446) | 20.0 (0.869) |
| dice 8 x 8, dedupe in a body, no gutter | 396.1 (0.920) | 53.7 (0.832) | 278.5 (0.638) | 797.8 (0.525) | 21.1 (0.915) |
| dice 8 x 8, dedupe in a body, 1 px gutter | 583.9 (1.356) | 81.1 (1.255) | 423.4 (0.970) | 1195.5 (0.787) | 31.9 (1.382) |

**Enhanced**

| layout | equipment | NPC small | NPC medium | NPC large |
|---|---:|---:|---:|---:|
| rectangle | 2166.7 (1.000) | 130.1 (1.000) | 1752.7 (1.000) | 5841.8 (1.000) |
| BC7, padded to 4 x 4 | 1183.5 (0.546) | 69.4 (0.534) | 909.1 (0.519) | 2969.5 (0.508) |
| BC1, padded to 4 x 4 | 602.2 (0.278) | 35.0 (0.269) | 455.7 (0.260) | 1485.6 (0.254) |
| R8 index + BC4 coverage | 1688.4 (0.779) | 100.2 (0.770) | 1332.4 (0.760) | 4406.7 (0.754) |
| row spans, linear memory | 1159.7 (0.535) | 71.0 (0.546) | 906.0 (0.517) | 2853.9 (0.489) |
| painted only, row entry + 32-pixel word + pixel | 1314.8 (0.607) | 76.2 (0.586) | 915.7 (0.522) | 2581.1 (0.442) |
| painted only, runs searched in the row | 1196.2 (0.552) | 70.5 (0.542) | 838.5 (0.478) | 2296.7 (0.393) |
| dice 8 x 8, dedupe in a body, no gutter | 1847.6 (0.853) | 101.2 (0.778) | 1144.5 (0.653) | 3035.5 (0.520) |
| dice 8 x 8, dedupe in a body, 1 px gutter | 2831.2 (1.307) | 155.7 (1.197) | 1764.9 (1.007) | 4635.4 (0.793) |

#### By crowd: total MB (x rectangle)

**Classic**

| layout | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| rectangle | 133.4 (1.000) | 26.1 (1.000) | 6.9 (1.000) | 1.6 (1.000) | 27.9 (1.000) | 5.8 (1.000) | 918.3 (1.000) | 189.5 (1.000) |
| BC7, padded to 4 x 4 | 135.8 (1.018) | 26.4 (1.013) | 7.0 (1.005) | 1.6 (0.954) | 28.4 (1.019) | 5.8 (0.995) | 929.8 (1.013) | 191.8 (1.012) |
| BC1, padded to 4 x 4 | 68.0 (0.510) | 13.2 (0.507) | 3.6 (0.517) | 0.8 (0.490) | 14.3 (0.513) | 2.9 (0.501) | 465.1 (0.506) | 95.9 (0.506) |
| row spans, linear memory | 70.7 (0.530) | 14.2 (0.546) | 5.2 (0.753) | 1.3 (0.770) | 16.7 (0.598) | 3.5 (0.610) | 473.0 (0.515) | 96.9 (0.511) |
| painted only, row entry + 32-pixel word + pixel | 77.1 (0.578) | 15.4 (0.593) | 7.0 (1.013) | 1.7 (1.011) | 19.2 (0.687) | 4.1 (0.701) | 488.5 (0.532) | 99.8 (0.527) |
| painted only, runs searched in the row | 64.0 (0.480) | 12.9 (0.494) | 6.3 (0.902) | 1.5 (0.909) | 16.4 (0.587) | 3.5 (0.601) | 390.5 (0.425) | 79.7 (0.421) |
| dice 8 x 8, dedupe in a body, no gutter | 76.3 (0.572) | 15.4 (0.589) | 7.2 (1.043) | 1.7 (1.034) | 19.4 (0.697) | 4.1 (0.712) | 480.0 (0.523) | 97.8 (0.516) |
| dice 8 x 8, dedupe in a body, 1 px gutter | 116.0 (0.870) | 23.4 (0.896) | 10.7 (1.533) | 2.5 (1.498) | 29.4 (1.055) | 6.2 (1.064) | 719.6 (0.784) | 147.1 (0.776) |

**Enhanced**

| layout | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| rectangle | 397.1 (1.000) | 80.2 (1.000) | 19.5 (1.000) | 5.8 (1.000) | 100.4 (1.000) | 21.1 (1.000) | 2462.7 (1.000) | 503.9 (1.000) |
| BC7, padded to 4 x 4 | 203.0 (0.511) | 40.9 (0.510) | 10.7 (0.550) | 3.0 (0.525) | 52.1 (0.518) | 10.8 (0.513) | 1245.4 (0.506) | 254.7 (0.505) |
| BC1, padded to 4 x 4 | 101.6 (0.256) | 20.5 (0.255) | 5.5 (0.280) | 1.5 (0.267) | 26.1 (0.260) | 5.4 (0.258) | 622.9 (0.253) | 127.4 (0.253) |
| R8 index + BC4 coverage | 300.2 (0.756) | 60.7 (0.757) | 15.2 (0.782) | 4.5 (0.785) | 76.4 (0.761) | 16.1 (0.764) | 1854.2 (0.753) | 379.4 (0.753) |
| row spans, linear memory | 192.4 (0.485) | 38.5 (0.480) | 12.1 (0.619) | 3.6 (0.625) | 47.7 (0.475) | 10.3 (0.485) | 1191.8 (0.484) | 244.3 (0.485) |
| painted only, row entry + 32-pixel word + pixel | 179.2 (0.451) | 36.0 (0.449) | 13.8 (0.708) | 4.0 (0.698) | 46.2 (0.460) | 9.8 (0.465) | 1048.4 (0.426) | 213.6 (0.424) |
| painted only, runs searched in the row | 161.2 (0.406) | 32.5 (0.405) | 12.8 (0.654) | 3.8 (0.652) | 41.3 (0.412) | 8.8 (0.416) | 917.9 (0.373) | 186.9 (0.371) |
| dice 8 x 8, dedupe in a body, no gutter | 214.9 (0.541) | 43.1 (0.537) | 18.7 (0.958) | 5.3 (0.921) | 57.5 (0.573) | 12.2 (0.576) | 1165.0 (0.473) | 235.8 (0.468) |
| dice 8 x 8, dedupe in a body, 1 px gutter | 331.5 (0.835) | 66.4 (0.827) | 28.7 (1.472) | 8.1 (1.399) | 88.6 (0.882) | 18.6 (0.881) | 1777.6 (0.722) | 360.9 (0.716) |

#### 4 x 4 blocks with a transparent pixel

A BC1 block with any transparent pixel drops to three colours, so this predicts its damage. "Among blocks with paint" leaves out wholly transparent blocks, which cost BC1 nothing.

| scope | Classic blocks M | any transparent | wholly transparent | among blocks with paint | Enhanced blocks M | any transparent | wholly transparent | among blocks with paint |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| whole set | 157.3 | 73.1 % | 52.3 % | 43.6 % | 319.2 | 73.7 % | 53.8 % | 43.1 % |
| equipment | 27.1 | 84.6 % | 43.2 % | 72.9 % | 72.7 | 81.0 % | 45.3 % | 65.3 % |
| NPC small | 4.2 | 77.9 % | 39.9 % | 63.2 % | 4.3 | 79.0 % | 44.5 % | 62.1 % |
| NPC medium | 28.1 | 73.2 % | 48.5 % | 47.9 % | 56.7 | 72.0 % | 50.0 % | 44.0 % |
| NPC large | 96.4 | 69.6 % | 56.8 % | 29.7 % | 185.5 | 71.2 % | 58.6 % | 30.6 % |
| not listed | 1.5 | 74.1 % | 33.0 % | 61.4 % | - | - | - | - |
| creatures, five | 8.5 | 69.5 % | 54.6 % | 32.8 % | 12.7 | 72.8 % | 57.1 % | 36.6 % |
| creatures, faced | 1.6 | 68.9 % | 53.5 % | 33.1 % | 2.6 | 73.0 % | 57.5 % | 36.5 % |
| dressed NPCs, five | 0.4 | 80.7 % | 35.5 % | 70.0 % | 0.7 | 76.0 % | 37.2 % | 61.7 % |
| dressed NPCs, faced | 0.1 | 79.4 % | 35.0 % | 68.3 % | 0.2 | 74.8 % | 38.1 % | 59.3 % |
| uniform, five | 1.8 | 70.5 % | 47.9 % | 43.3 % | 3.2 | 75.7 % | 57.2 % | 43.3 % |
| uniform, faced | 0.4 | 70.2 % | 47.6 % | 43.1 % | 0.7 | 76.1 % | 57.5 % | 43.7 % |
| worst, five | 58.1 | 68.9 % | 58.3 % | 25.4 % | 77.8 | 70.5 % | 60.9 % | 24.5 % |
| worst, faced | 12.0 | 69.2 % | 58.8 % | 25.4 % | 15.9 | 70.7 % | 61.2 % | 24.5 % |

#### What padding costs

x pixels is block pixels over true pixels. Bytes a pixel count a frame's own blocks with no gutter, over its true pixels.

**Classic**

| scope | frame area up to | frames | pixels Mpx | x at 4 x 4 | x at 6 x 6 | x at 8 x 8 | BC1 bytes a pixel | BC7 or ASTC 4 x 4 | ASTC 6 x 6 | ASTC 8 x 8 | 6 x 6 over BC1 | 8 x 8 over BC1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| whole set | 64 | 136019 | 4.4 | 1.566 | 2.143 | 2.398 | 0.783 | 1.566 | 0.953 | 0.599 | 1.217 | 0.766 |
| whole set | 256 | 261144 | 39.3 | 1.270 | 1.458 | 1.691 | 0.635 | 1.270 | 0.648 | 0.423 | 1.020 | 0.665 |
| whole set | 1024 | 346170 | 183.5 | 1.140 | 1.239 | 1.337 | 0.570 | 1.140 | 0.551 | 0.334 | 0.966 | 0.587 |
| whole set | all | 996620 | 2419.0 | 1.041 | 1.069 | 1.098 | 0.520 | 1.041 | 0.475 | 0.274 | 0.913 | 0.528 |
| equipment | 64 | 127569 | 4.1 | 1.568 | 2.139 | 2.417 | 0.784 | 1.568 | 0.951 | 0.604 | 1.212 | 0.770 |
| equipment | 256 | 239341 | 36.2 | 1.270 | 1.462 | 1.678 | 0.635 | 1.270 | 0.650 | 0.419 | 1.023 | 0.661 |
| equipment | all | 780234 | 384.8 | 1.125 | 1.215 | 1.304 | 0.563 | 1.125 | 0.540 | 0.326 | 0.960 | 0.580 |
| NPC small | all | 42120 | 61.9 | 1.079 | 1.134 | 1.188 | 0.539 | 1.079 | 0.504 | 0.297 | 0.934 | 0.551 |
| NPC medium | all | 80489 | 432.5 | 1.041 | 1.070 | 1.098 | 0.521 | 1.041 | 0.475 | 0.275 | 0.913 | 0.527 |
| NPC large | all | 51415 | 1517.8 | 1.017 | 1.028 | 1.039 | 0.508 | 1.017 | 0.457 | 0.260 | 0.899 | 0.511 |
| not listed | all | 42362 | 22.0 | 1.109 | 1.178 | 1.284 | 0.554 | 1.109 | 0.523 | 0.321 | 0.944 | 0.579 |

**Enhanced**

| scope | frame area up to | frames | pixels Mpx | x at 4 x 4 | x at 6 x 6 | x at 8 x 8 | BC1 bytes a pixel | BC7 or ASTC 4 x 4 | ASTC 6 x 6 | ASTC 8 x 8 | 6 x 6 over BC1 | 8 x 8 over BC1 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| whole set | 64 | 82862 | 3.0 | 1.546 | 1.997 | 2.361 | 0.773 | 1.546 | 0.888 | 0.590 | 1.148 | 0.763 |
| whole set | 256 | 359626 | 56.2 | 1.255 | 1.435 | 1.661 | 0.628 | 1.255 | 0.638 | 0.415 | 1.016 | 0.661 |
| whole set | 1024 | 546078 | 325.2 | 1.131 | 1.224 | 1.317 | 0.565 | 1.131 | 0.544 | 0.329 | 0.962 | 0.583 |
| whole set | all | 1572711 | 4916.9 | 1.039 | 1.065 | 1.093 | 0.519 | 1.039 | 0.473 | 0.273 | 0.912 | 0.526 |
| equipment | 64 | 82738 | 3.0 | 1.546 | 1.998 | 2.361 | 0.773 | 1.546 | 0.888 | 0.590 | 1.148 | 0.763 |
| equipment | 256 | 359222 | 56.2 | 1.255 | 1.435 | 1.661 | 0.628 | 1.255 | 0.638 | 0.415 | 1.016 | 0.661 |
| equipment | all | 1303213 | 1059.6 | 1.097 | 1.166 | 1.238 | 0.549 | 1.097 | 0.518 | 0.309 | 0.944 | 0.564 |
| NPC small | all | 33440 | 64.4 | 1.070 | 1.116 | 1.165 | 0.535 | 1.070 | 0.496 | 0.291 | 0.927 | 0.545 |
| NPC medium | all | 137062 | 873.8 | 1.038 | 1.063 | 1.089 | 0.519 | 1.038 | 0.473 | 0.272 | 0.911 | 0.525 |
| NPC large | all | 98996 | 2919.1 | 1.017 | 1.028 | 1.039 | 0.508 | 1.017 | 0.457 | 0.260 | 0.899 | 0.511 |

#### Hue masks

BC7 and BC1 can't hue by index, so a frame that's neither all hueable nor all kept needs a mask. On Enhanced the flag is the palette's keep-colour byte, and `mobile_anim_ec` packs that mask today. **Classic packs no mask today** (`mobile_anim_cc` has no `FRAME_HUE_*`): its row is what a block-coded Classic format would need, because under a partial hue only grey colours take it.

**Classic**

| scope | none | all | masked | R8 mask MB | BC4 mask MB | one-bit mask MB | BC7 MB | + R8 | + BC4 | + bits | BC1 MB | + R8 | + BC4 | + bits | rectangle MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| whole set | 16.2 % | 4.4 % | 79.4 % | 2132.9 | 1108.6 | 279.2 | 2517.5 | 4650.4 | 3626.1 | 2796.7 | 1258.7 | 3391.7 | 2367.3 | 1537.9 | 2419.0 |
| equipment | 17.6 % | 5.2 % | 77.2 % | 338.8 | 189.4 | 48.5 | 433.0 | 771.8 | 622.4 | 481.5 | 216.5 | 555.3 | 405.9 | 265.0 | 384.8 |
| NPC small | 17.6 % | 0.4 % | 82.0 % | 52.2 | 28.1 | 7.2 | 66.8 | 118.9 | 94.9 | 73.9 | 33.4 | 85.5 | 61.5 | 40.6 | 61.9 |
| NPC medium | 7.1 % | 0.4 % | 92.6 % | 392.1 | 204.2 | 51.5 | 450.3 | 842.4 | 654.6 | 501.8 | 225.2 | 617.3 | 429.4 | 276.7 | 432.5 |
| NPC large | 15.8 % | 0.1 % | 84.0 % | 1331.1 | 676.3 | 169.4 | 1543.0 | 2874.1 | 2219.3 | 1712.4 | 771.5 | 2102.6 | 1447.8 | 940.9 | 1517.8 |
| not listed | 5.4 % | 6.7 % | 87.9 % | 18.8 | 10.5 | 2.7 | 24.4 | 43.2 | 34.8 | 27.1 | 12.2 | 31.0 | 22.7 | 14.9 | 22.0 |
| creatures, five | 17.1 % | 0.2 % | 82.7 % | 112.5 | 57.5 | 14.4 | 135.6 | 248.1 | 193.1 | 150.1 | 67.8 | 180.3 | 125.3 | 82.2 | 132.7 |
| creatures, faced | 16.7 % | 0.0 % | 83.3 % | 21.9 | 11.2 | 2.8 | 26.4 | 48.2 | 37.5 | 29.2 | 13.2 | 35.1 | 24.4 | 16.0 | 25.8 |
| dressed NPCs, five | 16.4 % | 5.3 % | 78.3 % | 5.3 | 3.0 | 0.8 | 6.8 | 12.1 | 9.8 | 7.5 | 3.4 | 8.7 | 6.4 | 4.2 | 6.0 |
| dressed NPCs, faced | 15.1 % | 5.6 % | 79.3 % | 1.2 | 0.7 | 0.2 | 1.5 | 2.8 | 2.2 | 1.7 | 0.8 | 2.0 | 1.4 | 0.9 | 1.4 |
| uniform, five | 15.2 % | 4.2 % | 80.5 % | 25.7 | 13.3 | 3.4 | 28.2 | 53.9 | 41.6 | 31.6 | 14.1 | 39.8 | 27.5 | 17.5 | 27.1 |
| uniform, faced | 15.8 % | 4.1 % | 80.1 % | 5.1 | 2.6 | 0.7 | 5.7 | 10.8 | 8.4 | 6.4 | 2.9 | 7.9 | 5.5 | 3.5 | 5.5 |
| worst, five | 7.9 % | 0.0 % | 92.1 % | 859.2 | 435.0 | 108.9 | 929.5 | 1788.7 | 1364.5 | 1038.4 | 464.8 | 1324.0 | 899.8 | 573.6 | 917.9 |
| worst, faced | 8.0 % | 0.0 % | 92.0 % | 177.4 | 89.8 | 22.5 | 191.8 | 369.2 | 281.6 | 214.2 | 95.9 | 273.3 | 185.7 | 118.4 | 189.4 |

**Enhanced**

| scope | none | all | masked | R8 mask MB | BC4 mask MB | one-bit mask MB | BC7 MB | + R8 | + BC4 | + bits | BC1 MB | + R8 | + BC4 | + bits | rectangle MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| whole set | 0.2 % | 91.8 % | 8.0 % | 457.5 | 238.7 | 60.4 | 5106.4 | 5563.9 | 5345.1 | 5166.8 | 2553.2 | 3010.7 | 2791.9 | 2613.6 | 9833.8 |
| equipment | 0.2 % | 92.3 % | 7.5 % | 187.1 | 99.7 | 25.5 | 1162.6 | 1349.7 | 1262.4 | 1188.1 | 581.3 | 768.4 | 681.1 | 606.8 | 2119.2 |
| NPC small | 0.0 % | 69.4 % | 30.6 % | 23.4 | 12.5 | 3.2 | 68.9 | 92.3 | 81.4 | 72.1 | 34.5 | 57.9 | 47.0 | 37.7 | 128.8 |
| NPC medium | 0.0 % | 92.6 % | 7.4 % | 83.5 | 43.1 | 10.8 | 906.9 | 990.4 | 950.0 | 917.8 | 453.5 | 537.0 | 496.5 | 464.3 | 1747.6 |
| NPC large | 0.0 % | 92.7 % | 7.3 % | 163.5 | 83.4 | 20.9 | 2968.0 | 3131.4 | 3051.3 | 2988.9 | 1484.0 | 1647.5 | 1567.4 | 1504.9 | 5838.2 |
| creatures, five | 0.0 % | 88.2 % | 11.8 % | 14.7 | 7.6 | 1.9 | 202.8 | 217.5 | 210.3 | 204.7 | 101.4 | 116.1 | 109.0 | 103.3 | 396.6 |
| creatures, faced | 0.0 % | 87.9 % | 12.1 % | 3.2 | 1.6 | 0.4 | 40.8 | 44.0 | 42.5 | 41.3 | 20.4 | 23.6 | 22.1 | 20.8 | 79.9 |
| dressed NPCs, five | 0.3 % | 87.7 % | 12.0 % | 2.6 | 1.4 | 0.4 | 10.5 | 13.1 | 11.9 | 10.9 | 5.3 | 7.9 | 6.7 | 5.6 | 19.0 |
| dressed NPCs, faced | 0.3 % | 81.3 % | 18.3 % | 1.2 | 0.6 | 0.2 | 3.0 | 4.2 | 3.6 | 3.2 | 1.5 | 2.7 | 2.1 | 1.7 | 5.5 |
| uniform, five | 0.2 % | 93.1 % | 6.7 % | 2.2 | 1.2 | 0.3 | 51.8 | 54.0 | 53.0 | 52.1 | 25.9 | 28.1 | 27.1 | 26.2 | 99.9 |
| uniform, faced | 0.1 % | 93.2 % | 6.6 % | 0.4 | 0.2 | 0.1 | 10.8 | 11.2 | 11.0 | 10.8 | 5.4 | 5.8 | 5.6 | 5.5 | 20.8 |
| worst, five | 0.0 % | 97.8 % | 2.2 % | 19.2 | 9.7 | 2.4 | 1245.1 | 1264.3 | 1254.8 | 1247.6 | 622.6 | 641.7 | 632.3 | 625.0 | 2462.1 |
| worst, faced | 0.0 % | 97.8 % | 2.2 % | 3.8 | 1.9 | 0.5 | 254.6 | 258.4 | 256.5 | 255.1 | 127.3 | 131.1 | 129.2 | 127.8 | 503.5 |

#### Today's packer and whole pages

A renderer using today's reader API (`read_page_bytes`, `read_page_rgba`, `AtlasPageCache`) holds whole pages. The table counts the pages a crowd touches, simulating the packer's own order.

| crowd | load unit | Classic frames' own BC7 MB | pages touched | whole pages MB | x frames' own | Enhanced frames' own BC7 MB | pages touched | whole pages MB | x frames' own | Enhanced with its R8 masks MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| creatures | five directions | 135.6 | 260 | 892.8 | 6.58 | 202.8 | 326 | 1139.0 | 5.62 | 1274.1 |
| creatures | faced only | 26.4 | 209 | 759.5 | 28.81 | 40.8 | 270 | 982.8 | 24.06 | 1112.7 |
| dressed NPCs | five directions | 6.8 | 151 | 492.0 | 72.58 | 10.5 | 214 | 804.3 | 76.46 | 974.7 |
| dressed NPCs | faced only | 1.5 | 151 | 492.0 | 321.79 | 3.0 | 213 | 803.7 | 269.05 | 974.1 |
| uniform | five directions | 28.2 | 207 | 707.0 | 25.04 | 51.8 | 260 | 947.8 | 18.29 | 1051.1 |
| uniform | faced only | 5.7 | 198 | 682.5 | 119.42 | 10.8 | 244 | 898.0 | 83.25 | 1001.3 |
| worst | five directions | 929.5 | 580 | 1474.8 | 1.59 | 1245.1 | 724 | 1890.8 | 1.52 | 1958.4 |
| worst | faced only | 191.8 | 322 | 992.7 | 5.18 | 254.6 | 385 | 1169.0 | 4.59 | 1215.0 |

#### Dynamic allocators, a fresh pack

`guillotiere` 0.7 for guillotine (already a `udd-conv` dependency), a small shelf allocator written in the bench for comparison. A 1 px gutter on each side of each rectangle.

**Classic**

| what | allocator | page side | rectangles | pages | page Mpx | frame Mpx | frames' share | gutters and blocks' gaps | empty |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| whole set | guillotine | 2048 | each frame | 666 | 2793.4 | 2419.0 | 86.6 % | 4.7 % | 8.7 % |
| whole set | shelf | 2048 | each frame | 700 | 2936.0 | 2419.0 | 82.4 % | 4.5 % | 13.1 % |
| whole set | guillotine | 2048 | each stream as one block | 957 | 4013.9 | 2419.0 | 60.3 % | 15.8 % | 24.0 % |
| whole set | shelf | 2048 | each stream as one block | 1029 | 4315.9 | 2419.0 | 56.0 % | 14.7 % | 29.3 % |
| whole set | guillotine | 4096 | each frame | 160 | 2684.4 | 2419.0 | 90.1 % | 4.9 % | 5.0 % |
| whole set | shelf | 4096 | each frame | 175 | 2936.0 | 2419.0 | 82.4 % | 4.5 % | 13.1 % |
| whole set | guillotine | 4096 | each stream as one block | 201 | 3372.2 | 2419.0 | 71.7 % | 18.8 % | 9.5 % |
| whole set | shelf | 4096 | each stream as one block | 226 | 3791.7 | 2419.0 | 63.8 % | 16.7 % | 19.5 % |
| creatures, five directions | guillotine | 2048 | each frame | 35 | 146.8 | 132.7 | 90.4 % | 2.7 % | 6.9 % |
| creatures, five directions | shelf | 2048 | each frame | 39 | 163.6 | 132.7 | 81.1 % | 2.4 % | 16.5 % |
| creatures, five directions | guillotine | 2048 | each stream as one block | 47 | 197.1 | 132.7 | 67.3 % | 15.4 % | 17.3 % |
| creatures, five directions | shelf | 2048 | each stream as one block | 52 | 218.1 | 132.7 | 60.8 % | 13.9 % | 25.3 % |
| creatures, faced only | guillotine | 2048 | each frame | 7 | 29.4 | 25.8 | 87.9 % | 2.7 % | 9.5 % |
| creatures, faced only | shelf | 2048 | each frame | 9 | 37.7 | 25.8 | 68.3 % | 2.1 % | 29.6 % |
| creatures, faced only | guillotine | 2048 | each stream as one block | 10 | 41.9 | 25.8 | 61.5 % | 13.8 % | 24.7 % |
| creatures, faced only | shelf | 2048 | each stream as one block | 12 | 50.3 | 25.8 | 51.2 % | 11.5 % | 37.2 % |
| dressed NPCs, five directions | guillotine | 2048 | each frame | 2 | 8.4 | 6.0 | 71.5 % | 12.7 % | 15.8 % |
| dressed NPCs, five directions | shelf | 2048 | each frame | 2 | 8.4 | 6.0 | 71.5 % | 12.7 % | 15.8 % |
| dressed NPCs, five directions | guillotine | 2048 | each stream as one block | 3 | 12.6 | 6.0 | 47.7 % | 20.8 % | 31.5 % |
| dressed NPCs, five directions | shelf | 2048 | each stream as one block | 3 | 12.6 | 6.0 | 47.7 % | 20.8 % | 31.5 % |
| dressed NPCs, faced only | guillotine | 2048 | each frame | 1 | 4.2 | 1.4 | 32.4 % | 5.5 % | 62.1 % |
| dressed NPCs, faced only | shelf | 2048 | each frame | 1 | 4.2 | 1.4 | 32.4 % | 5.5 % | 62.1 % |
| dressed NPCs, faced only | guillotine | 2048 | each stream as one block | 1 | 4.2 | 1.4 | 32.4 % | 14.0 % | 53.6 % |
| dressed NPCs, faced only | shelf | 2048 | each stream as one block | 1 | 4.2 | 1.4 | 32.4 % | 14.0 % | 53.6 % |

**Enhanced**

| what | allocator | page side | rectangles | pages | page Mpx | frame Mpx | frames' share | gutters and blocks' gaps | empty |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|
| whole set | guillotine | 2048 | each frame | 1352 | 5670.7 | 4916.9 | 86.7 % | 4.5 % | 8.8 % |
| whole set | shelf | 2048 | each frame | 1425 | 5976.9 | 4916.9 | 82.3 % | 4.3 % | 13.5 % |
| whole set | guillotine | 2048 | each stream as one block | 1870 | 7843.3 | 4916.9 | 62.7 % | 16.1 % | 21.2 % |
| whole set | shelf | 2048 | each stream as one block | 1979 | 8300.5 | 4916.9 | 59.2 % | 15.2 % | 25.5 % |
| whole set | guillotine | 4096 | each frame | 328 | 5502.9 | 4916.9 | 89.4 % | 4.6 % | 6.0 % |
| whole set | shelf | 4096 | each frame | 354 | 5939.1 | 4916.9 | 82.8 % | 4.3 % | 12.9 % |
| whole set | guillotine | 4096 | each stream as one block | 414 | 6945.8 | 4916.9 | 70.8 % | 18.2 % | 11.0 % |
| whole set | shelf | 4096 | each stream as one block | 449 | 7533.0 | 4916.9 | 65.3 % | 16.8 % | 17.9 % |
| creatures, five directions | guillotine | 2048 | each frame | 52 | 218.1 | 198.3 | 90.9 % | 2.8 % | 6.3 % |
| creatures, five directions | shelf | 2048 | each frame | 58 | 243.3 | 198.3 | 81.5 % | 2.5 % | 16.0 % |
| creatures, five directions | guillotine | 2048 | each stream as one block | 65 | 272.6 | 198.3 | 72.7 % | 15.9 % | 11.3 % |
| creatures, five directions | shelf | 2048 | each stream as one block | 72 | 302.0 | 198.3 | 65.7 % | 14.4 % | 20.0 % |
| creatures, faced only | guillotine | 2048 | each frame | 11 | 46.1 | 39.9 | 86.5 % | 2.6 % | 10.8 % |
| creatures, faced only | shelf | 2048 | each frame | 12 | 50.3 | 39.9 | 79.3 % | 2.4 % | 18.3 % |
| creatures, faced only | guillotine | 2048 | each stream as one block | 14 | 58.7 | 39.9 | 68.0 % | 15.4 % | 16.6 % |
| creatures, faced only | shelf | 2048 | each stream as one block | 17 | 71.3 | 39.9 | 56.0 % | 12.7 % | 31.3 % |
| dressed NPCs, five directions | guillotine | 2048 | each frame | 3 | 12.6 | 9.5 | 75.7 % | 10.4 % | 13.9 % |
| dressed NPCs, five directions | shelf | 2048 | each frame | 4 | 16.8 | 9.5 | 56.8 % | 7.8 % | 35.4 % |
| dressed NPCs, five directions | guillotine | 2048 | each stream as one block | 4 | 16.8 | 9.5 | 56.8 % | 22.9 % | 20.4 % |
| dressed NPCs, five directions | shelf | 2048 | each stream as one block | 4 | 16.8 | 9.5 | 56.8 % | 22.9 % | 20.4 % |
| dressed NPCs, faced only | guillotine | 2048 | each frame | 1 | 4.2 | 2.7 | 65.1 % | 7.9 % | 27.0 % |
| dressed NPCs, faced only | shelf | 2048 | each frame | 1 | 4.2 | 2.7 | 65.1 % | 7.9 % | 27.0 % |
| dressed NPCs, faced only | guillotine | 2048 | each stream as one block | 2 | 8.4 | 2.7 | 32.6 % | 12.7 % | 54.7 % |
| dressed NPCs, faced only | shelf | 2048 | each stream as one block | 2 | 8.4 | 2.7 | 32.6 % | 12.7 % | 54.7 % |

#### Churn

Eight rounds, creature crowds and dressed NPCs in turn, five directions, a 1 px gutter. LRU evicts what the round doesn't need, and a page is added only when nothing can be evicted. The "worst round" is the small dressed crowd sitting on pages kept for creatures, so its live share and fragmentation are dominated by retained pages rather than by holes: read peak pages against the budget.

**Classic**

| allocator | page side | rectangles | budget x largest fresh pack | budget pages | peak pages | rounds over budget | streams evicted | worst round: pages | its fresh pack | its live Mpx | live's share of its pages | waste that is fragmentation and cache | peak MB, rectangle format |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| guillotine | 2048 | each frame | 1.00 | 36 | 43 | 6 | 7996 | 43 | 2 | 6.3 | 3.5 % | 98.8 % | 180.4 |
| shelf | 2048 | each frame | 1.00 | 41 | 45 | 6 | 7958 | 45 | 3 | 6.9 | 3.7 % | 96.9 % | 188.7 |
| guillotine | 2048 | each frame | 1.25 | 45 | 50 | 6 | 7378 | 50 | 2 | 6.3 | 3.0 % | 99.0 % | 209.7 |
| shelf | 2048 | each frame | 1.25 | 52 | 52 | 0 | 5586 | 52 | 3 | 6.7 | 3.1 % | 97.2 % | 218.1 |
| guillotine | 2048 | each stream as one block | 1.00 | 46 | 64 | 6 | 7383 | 64 | 3 | 6.3 | 2.4 % | 97.6 % | 268.4 |
| shelf | 2048 | each stream as one block | 1.00 | 52 | 67 | 6 | 7378 | 67 | 3 | 6.3 | 2.3 % | 97.7 % | 281.0 |
| guillotine | 2048 | each stream as one block | 1.25 | 58 | 67 | 6 | 7378 | 67 | 3 | 6.3 | 2.3 % | 97.7 % | 281.0 |
| shelf | 2048 | each stream as one block | 1.25 | 65 | 67 | 6 | 7351 | 67 | 3 | 6.7 | 2.4 % | 97.9 % | 281.0 |
| guillotine | 4096 | each frame | 1.00 | 9 | 13 | 6 | 8057 | 13 | 1 | 6.3 | 2.9 % | 95.1 % | 218.1 |
| shelf | 4096 | each frame | 1.00 | 11 | 11 | 0 | 5882 | 11 | 1 | 6.7 | 3.6 % | 94.4 % | 184.5 |
| guillotine | 4096 | each frame | 1.25 | 12 | 13 | 6 | 6063 | 13 | 1 | 6.7 | 3.1 % | 95.2 % | 218.1 |
| shelf | 4096 | each frame | 1.25 | 14 | 14 | 0 | 5408 | 14 | 1 | 6.7 | 2.9 % | 95.6 % | 234.9 |
| guillotine | 4096 | each stream as one block | 1.00 | 11 | 15 | 6 | 7383 | 15 | 1 | 6.3 | 2.5 % | 95.7 % | 251.7 |
| shelf | 4096 | each stream as one block | 1.00 | 13 | 17 | 6 | 7383 | 17 | 1 | 6.3 | 2.2 % | 96.3 % | 285.2 |
| guillotine | 4096 | each stream as one block | 1.25 | 14 | 17 | 6 | 6058 | 17 | 1 | 6.7 | 2.4 % | 96.4 % | 285.2 |
| shelf | 4096 | each stream as one block | 1.25 | 17 | 18 | 2 | 7323 | 18 | 1 | 6.3 | 2.1 % | 96.5 % | 302.0 |

**Enhanced**

| allocator | page side | rectangles | budget x largest fresh pack | budget pages | peak pages | rounds over budget | streams evicted | worst round: pages | its fresh pack | its live Mpx | live's share of its pages | waste that is fragmentation and cache | peak MB, rectangle format |
|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| guillotine | 2048 | each frame | 1.00 | 59 | 69 | 6 | 8043 | 69 | 3 | 9.2 | 3.2 % | 98.8 % | 578.8 |
| shelf | 2048 | each frame | 1.00 | 67 | 73 | 4 | 8041 | 73 | 3 | 9.2 | 3.0 % | 98.9 % | 612.4 |
| guillotine | 2048 | each frame | 1.25 | 74 | 75 | 6 | 5857 | 75 | 3 | 9.5 | 3.0 % | 99.0 % | 629.1 |
| shelf | 2048 | each frame | 1.25 | 84 | 84 | 0 | 5050 | 84 | 3 | 9.5 | 2.7 % | 99.1 % | 704.6 |
| guillotine | 2048 | each stream as one block | 1.00 | 73 | 95 | 6 | 8043 | 95 | 4 | 9.2 | 2.3 % | 98.1 % | 796.9 |
| shelf | 2048 | each stream as one block | 1.00 | 83 | 106 | 6 | 6647 | 106 | 4 | 8.5 | 1.9 % | 98.1 % | 889.2 |
| guillotine | 2048 | each stream as one block | 1.25 | 92 | 97 | 6 | 7295 | 97 | 4 | 9.2 | 2.3 % | 98.1 % | 813.7 |
| shelf | 2048 | each stream as one block | 1.25 | 104 | 109 | 4 | 7559 | 109 | 4 | 9.2 | 2.0 % | 98.3 % | 914.4 |
| guillotine | 4096 | each frame | 1.00 | 15 | 19 | 6 | 8043 | 19 | 1 | 9.2 | 2.9 % | 97.6 % | 637.5 |
| shelf | 4096 | each frame | 1.00 | 17 | 20 | 6 | 8043 | 20 | 1 | 9.2 | 2.8 % | 97.7 % | 671.1 |
| guillotine | 4096 | each frame | 1.25 | 19 | 20 | 6 | 5843 | 20 | 1 | 9.5 | 2.8 % | 97.8 % | 671.1 |
| shelf | 4096 | each frame | 1.25 | 22 | 22 | 0 | 4990 | 22 | 1 | 9.5 | 2.6 % | 98.0 % | 738.2 |
| guillotine | 4096 | each stream as one block | 1.00 | 19 | 25 | 6 | 6647 | 25 | 1 | 8.5 | 2.0 % | 98.0 % | 838.9 |
| shelf | 4096 | each stream as one block | 1.00 | 21 | 26 | 6 | 6667 | 26 | 1 | 8.5 | 1.9 % | 98.1 % | 872.4 |
| guillotine | 4096 | each stream as one block | 1.25 | 24 | 26 | 6 | 6496 | 26 | 1 | 8.5 | 1.9 % | 98.1 % | 872.4 |
| shelf | 4096 | each stream as one block | 1.25 | 27 | 28 | 2 | 7723 | 28 | 1 | 9.2 | 2.0 % | 98.4 % | 939.5 |

#### The load unit

MB with each layout's own palettes and tables. Streams and frames are per client.

| crowd | load unit | Classic rectangle | BC7 | BC1 | row spans | painted only | Enhanced rectangle | BC7 | BC1 | row spans | painted only |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| creatures | all actions of each body | 1410.8 | 1438.6 | 720.1 | 746.2 | 812.3 | 4561.7 | 2331.1 | 1166.8 | 2216.8 | 2064.4 |
| creatures | one action, five directions | 133.4 | 135.8 | 68.0 | 70.7 | 77.1 | 397.1 | 203.0 | 101.6 | 192.4 | 179.2 |
| creatures | one action, the faced direction | 26.1 | 26.4 | 13.2 | 14.2 | 15.4 | 80.2 | 40.9 | 20.5 | 38.5 | 36.0 |
| creatures | only the frames on screen, a stream's mean frame | 3.42 | 3.26 | 1.63 | 1.94 | 2.08 | 8.26 | 4.07 | 2.04 | 4.07 | 3.81 |
| dressed NPCs | all actions of each body | 241.8 | 247.6 | 126.6 | 156.3 | 202.9 | 1221.7 | 660.7 | 335.4 | 689.5 | 750.0 |
| dressed NPCs | one action, five directions | 6.9 | 7.0 | 3.6 | 5.2 | 7.0 | 19.5 | 10.7 | 5.5 | 12.1 | 13.8 |
| dressed NPCs | one action, the faced direction | 1.6 | 1.6 | 0.8 | 1.3 | 1.7 | 5.8 | 3.0 | 1.5 | 3.6 | 4.0 |
| dressed NPCs | only the frames on screen, a stream's mean frame | 0.45 | 0.23 | 0.12 | 0.39 | 0.45 | 0.90 | 0.35 | 0.18 | 0.65 | 0.70 |
| uniform | all actions of each body | 489.2 | 504.7 | 254.9 | 290.4 | 350.2 | 1936.8 | 1017.2 | 512.6 | 959.9 | 970.9 |
| uniform | one action, five directions | 27.9 | 28.4 | 14.3 | 16.7 | 19.2 | 100.4 | 52.1 | 26.1 | 47.7 | 46.2 |
| uniform | one action, the faced direction | 5.8 | 5.8 | 2.9 | 3.5 | 4.1 | 21.1 | 10.8 | 5.4 | 10.3 | 9.8 |
| uniform | only the frames on screen, a stream's mean frame | 0.87 | 0.63 | 0.32 | 0.62 | 0.69 | 2.40 | 1.09 | 0.55 | 1.31 | 1.27 |
| worst | all actions of each body | 1262.8 | 1280.9 | 640.7 | 650.5 | 677.6 | 3488.0 | 1766.7 | 883.6 | 1678.2 | 1486.9 |
| worst | one action, five directions | 918.3 | 929.8 | 465.1 | 473.0 | 488.5 | 2462.7 | 1245.4 | 622.9 | 1191.8 | 1048.4 |
| worst | one action, the faced direction | 189.5 | 191.8 | 95.9 | 96.9 | 99.8 | 503.9 | 254.7 | 127.4 | 244.3 | 213.6 |
| worst | only the frames on screen, a stream's mean frame | 15.94 | 16.07 | 8.04 | 8.12 | 8.26 | 40.31 | 20.23 | 10.12 | 19.21 | 16.70 |

#### Bodies and Mpx by a body's median cropped frame area

**Classic**

| class | up to 2500 | up to 10000 | up to 40000 | above |
|---|---:|---:|---:|---:|
| equipment | 538, 364.8 | 10, 15.3 | 2, 4.7 | 0, 0.0 |
| NPC small | 95, 61.9 | 0, 0.0 | 0, 0.0 | 0, 0.0 |
| NPC medium | 0, 0.0 | 173, 432.5 | 0, 0.0 | 0, 0.0 |
| NPC large | 0, 0.0 | 0, 0.0 | 88, 825.7 | 13, 692.1 |
| not listed | 97, 13.2 | 6, 8.8 | 0, 0.0 | 0, 0.0 |

**Enhanced**

| class | up to 2500 | up to 10000 | up to 40000 | above |
|---|---:|---:|---:|---:|
| equipment | 523, 772.9 | 35, 286.7 | 0, 0.0 | 0, 0.0 |
| NPC small | 51, 64.4 | 0, 0.0 | 0, 0.0 | 0, 0.0 |
| NPC medium | 0, 0.0 | 187, 873.8 | 0, 0.0 | 0, 0.0 |
| NPC large | 0, 0.0 | 0, 0.0 | 112, 1761.9 | 18, 1157.2 |

#### The low-end mix: VRAM

A per-body choice: a body whose median cropped frame area is above the threshold takes the block format, padded to its blocks; the rest stay R8 or RG8 with palettes. MB with frame entries, without masks, then with R8 masks over the block-coded bodies' masked frames. Classic packs no masks today, and an index layout needs none.

**Classic, total MB**

| layout | whole set | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all lossless | 2474.5 | 133.4 | 26.1 | 6.9 | 1.6 | 27.9 | 5.8 | 918.3 | 189.5 |
| all BC1 | 1274.7 | 68.0 | 13.2 | 3.6 | 0.8 | 14.3 | 2.9 | 465.1 | 95.9 |
| all ASTC 6 x 6 | 1165.5 | 61.3 | 11.9 | 3.5 | 0.8 | 13.1 | 2.6 | 417.0 | 86.0 |
| all ASTC 8 x 8 | 680.0 | 35.1 | 6.8 | 2.2 | 0.5 | 7.6 | 1.5 | 236.6 | 48.8 |
| above 2500: BC1 | 1502.6 | 69.5 | 13.5 | 6.7 | 1.6 | 16.9 | 3.5 | 465.1 | 95.9 |
| above 2500: ASTC 6 x 6 | 1403.7 | 62.9 | 12.3 | 6.7 | 1.6 | 15.8 | 3.3 | 417.0 | 86.0 |
| above 2500: ASTC 8 x 8 | 1011.9 | 37.3 | 7.3 | 6.6 | 1.5 | 11.4 | 2.4 | 236.6 | 48.8 |
| above 10000: BC1 | 1724.8 | 82.3 | 16.2 | 6.9 | 1.6 | 19.1 | 4.0 | 469.2 | 96.7 |
| above 10000: ASTC 6 x 6 | 1646.5 | 76.9 | 15.2 | 6.9 | 1.6 | 18.2 | 3.8 | 421.6 | 86.9 |
| above 10000: ASTC 8 x 8 | 1346.4 | 56.5 | 11.3 | 6.9 | 1.6 | 14.6 | 3.2 | 242.8 | 50.0 |
| above 40000: BC1 | 2132.1 | 106.2 | 20.9 | 6.9 | 1.6 | 23.5 | 4.9 | 596.2 | 122.5 |
| above 40000: ASTC 6 x 6 | 2095.5 | 103.3 | 20.4 | 6.9 | 1.6 | 23.0 | 4.8 | 561.9 | 115.4 |
| above 40000: ASTC 8 x 8 | 1959.7 | 92.5 | 18.4 | 6.9 | 1.6 | 21.2 | 4.4 | 434.1 | 88.8 |

**Classic, total MB with R8 masks**

| layout | whole set | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all BC1 | 3407.6 | 180.5 | 35.1 | 8.9 | 2.0 | 40.0 | 8.0 | 1324.2 | 273.3 |
| all ASTC 6 x 6 | 3298.4 | 173.8 | 33.8 | 8.8 | 2.0 | 38.7 | 7.7 | 1276.2 | 263.4 |
| all ASTC 8 x 8 | 2812.9 | 147.6 | 28.7 | 7.5 | 1.7 | 33.3 | 6.6 | 1095.8 | 226.2 |
| above 2500: BC1 | 3251.7 | 179.6 | 34.9 | 7.1 | 1.7 | 38.5 | 7.8 | 1324.2 | 273.3 |
| above 2500: ASTC 6 x 6 | 3152.8 | 173.0 | 33.7 | 7.1 | 1.7 | 37.3 | 7.5 | 1276.2 | 263.4 |
| above 2500: ASTC 8 x 8 | 2761.0 | 147.4 | 28.7 | 7.0 | 1.7 | 32.9 | 6.7 | 1095.8 | 226.2 |
| above 10000: BC1 | 3060.5 | 171.8 | 33.4 | 7.0 | 1.7 | 36.3 | 7.4 | 1324.6 | 273.5 |
| above 10000: ASTC 6 x 6 | 2982.3 | 166.4 | 32.3 | 7.0 | 1.7 | 35.4 | 7.2 | 1277.0 | 263.7 |
| above 10000: ASTC 8 x 8 | 2682.2 | 146.0 | 28.4 | 7.0 | 1.6 | 31.9 | 6.5 | 1098.2 | 226.8 |
| above 40000: BC1 | 2785.1 | 157.0 | 30.3 | 6.9 | 1.6 | 32.0 | 6.5 | 1222.5 | 253.1 |
| above 40000: ASTC 6 x 6 | 2748.6 | 154.1 | 29.8 | 6.9 | 1.6 | 31.5 | 6.4 | 1188.1 | 245.9 |
| above 40000: ASTC 8 x 8 | 2612.8 | 143.3 | 27.8 | 6.9 | 1.6 | 29.8 | 6.1 | 1060.3 | 219.4 |

**Enhanced, total MB**

| layout | whole set | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all lossless | 9891.3 | 397.1 | 80.2 | 19.5 | 5.8 | 100.4 | 21.1 | 2462.7 | 503.9 |
| all BC1 | 2578.4 | 101.6 | 20.5 | 5.5 | 1.5 | 26.1 | 5.4 | 622.9 | 127.4 |
| all BC7 | 5131.6 | 203.0 | 40.9 | 10.7 | 3.0 | 52.1 | 10.8 | 1245.4 | 254.7 |
| all ASTC 6 x 6 | 2352.7 | 91.7 | 18.5 | 5.2 | 1.4 | 23.8 | 5.0 | 557.9 | 114.1 |
| all ASTC 8 x 8 | 1368.2 | 52.4 | 10.6 | 3.2 | 0.9 | 13.9 | 2.9 | 316.2 | 64.6 |
| above 2500: BC1 | 3813.9 | 106.6 | 21.5 | 15.0 | 4.1 | 38.3 | 8.3 | 622.9 | 127.4 |
| above 2500: BC7 | 5902.3 | 206.2 | 41.6 | 16.6 | 4.7 | 59.6 | 12.7 | 1245.4 | 254.7 |
| above 2500: ASTC 6 x 6 | 3611.1 | 96.9 | 19.6 | 14.9 | 4.1 | 36.2 | 7.8 | 557.9 | 114.1 |
| above 2500: ASTC 8 x 8 | 2802.7 | 58.3 | 11.8 | 14.2 | 3.8 | 28.0 | 6.1 | 316.2 | 64.6 |
| above 10000: BC1 | 5535.1 | 171.9 | 34.5 | 19.5 | 5.8 | 53.4 | 11.4 | 630.8 | 128.8 |
| above 10000: BC7 | 7019.0 | 248.6 | 50.1 | 19.5 | 5.8 | 69.4 | 14.7 | 1250.7 | 255.6 |
| above 10000: ASTC 6 x 6 | 5384.9 | 164.2 | 32.9 | 19.5 | 5.8 | 51.8 | 11.1 | 566.1 | 115.5 |
| above 10000: ASTC 8 x 8 | 4809.6 | 134.4 | 26.9 | 19.5 | 5.8 | 45.5 | 9.8 | 325.5 | 66.3 |
| above 40000: BC1 | 8161.8 | 306.3 | 61.7 | 19.5 | 5.8 | 85.0 | 17.9 | 893.2 | 183.0 |
| above 40000: BC7 | 8747.0 | 337.1 | 68.0 | 19.5 | 5.8 | 90.2 | 19.0 | 1423.9 | 291.4 |
| above 40000: ASTC 6 x 6 | 8100.8 | 303.2 | 61.1 | 19.5 | 5.8 | 84.5 | 17.8 | 837.7 | 171.7 |
| above 40000: ASTC 8 x 8 | 7873.6 | 291.2 | 58.7 | 19.5 | 5.8 | 82.4 | 17.4 | 631.6 | 129.5 |
| above 2500: BC1, the rest BC7 | 3043.2 | 103.4 | 20.8 | 9.1 | 2.4 | 30.7 | 6.4 | 622.9 | 127.4 |
| above 10000: BC1, the rest BC7 | 3647.6 | 126.3 | 25.3 | 10.7 | 3.0 | 36.0 | 7.5 | 625.5 | 127.8 |
| above 40000: BC1, the rest BC7 | 4546.4 | 172.3 | 34.6 | 10.7 | 3.0 | 46.8 | 9.7 | 714.7 | 146.2 |

**Enhanced, total MB with R8 masks**

| layout | whole set | creatures, five | creatures, faced | dressed NPCs, five | dressed NPCs, faced | uniform, five | uniform, faced | worst, five | worst, faced |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| all BC1 | 3035.8 | 116.3 | 23.6 | 8.1 | 2.7 | 28.3 | 5.8 | 642.0 | 131.2 |
| all BC7 | 5589.0 | 217.7 | 44.1 | 13.3 | 4.2 | 54.2 | 11.2 | 1264.6 | 258.5 |
| all ASTC 6 x 6 | 2810.2 | 106.4 | 21.6 | 7.8 | 2.6 | 26.0 | 5.4 | 577.1 | 117.9 |
| all ASTC 8 x 8 | 1825.6 | 67.1 | 13.7 | 5.8 | 2.1 | 16.0 | 3.3 | 335.4 | 68.5 |
| above 2500: BC1 | 4203.3 | 120.0 | 24.4 | 16.8 | 4.8 | 39.9 | 8.5 | 642.0 | 131.2 |
| above 2500: BC7 | 6291.7 | 219.5 | 44.5 | 18.4 | 5.4 | 61.2 | 13.0 | 1264.6 | 258.5 |
| above 2500: ASTC 6 x 6 | 4000.5 | 110.2 | 22.4 | 16.6 | 4.8 | 37.8 | 8.1 | 577.1 | 117.9 |
| above 2500: ASTC 8 x 8 | 3192.1 | 71.7 | 14.7 | 16.0 | 4.5 | 29.6 | 6.4 | 335.4 | 68.5 |
| above 10000: BC1 | 5698.5 | 180.7 | 36.4 | 19.5 | 5.8 | 53.4 | 11.4 | 649.9 | 132.6 |
| above 10000: BC7 | 7182.5 | 257.5 | 52.0 | 19.5 | 5.8 | 69.4 | 14.7 | 1269.8 | 259.4 |
| above 10000: ASTC 6 x 6 | 5548.4 | 173.0 | 34.8 | 19.5 | 5.8 | 51.8 | 11.1 | 585.3 | 119.4 |
| above 10000: ASTC 8 x 8 | 4973.1 | 143.2 | 28.8 | 19.5 | 5.8 | 45.5 | 9.8 | 344.7 | 70.1 |
| above 40000: BC1 | 8161.8 | 306.3 | 61.7 | 19.5 | 5.8 | 85.0 | 17.9 | 893.2 | 183.0 |
| above 40000: BC7 | 8747.0 | 337.1 | 68.0 | 19.5 | 5.8 | 90.2 | 19.0 | 1423.9 | 291.4 |
| above 40000: ASTC 6 x 6 | 8100.8 | 303.2 | 61.1 | 19.5 | 5.8 | 84.5 | 17.8 | 837.7 | 171.7 |
| above 40000: ASTC 8 x 8 | 7873.6 | 291.2 | 58.7 | 19.5 | 5.8 | 82.4 | 17.4 | 631.6 | 129.5 |
| above 2500: BC1, the rest BC7 | 3500.7 | 118.2 | 24.0 | 11.7 | 3.6 | 32.9 | 6.8 | 642.0 | 131.2 |
| above 10000: BC1, the rest BC7 | 4105.1 | 141.0 | 28.5 | 13.3 | 4.2 | 38.2 | 7.9 | 644.7 | 131.7 |
| above 40000: BC1, the rest BC7 | 5003.9 | 187.0 | 37.8 | 13.3 | 4.2 | 49.0 | 10.1 | 733.8 | 150.1 |

#### How much of each crowd each class holds

Share of the scope's MB in R8 or RG8, then in BC1 (padding shifts it slightly). `of all these, above 40000` is a subset of the class rows. A crowd counts each distinct stream once.

**Classic**

| scope | class | streams | Mpx | R8 / RG8 | BC1 |
|---|---|---:|---:|---:|---:|
| whole set | equipment | 104770 | 384.8 | 384.8, 15.9 % | 216.5, 17.2 % |
| whole set | NPC small | 7072 | 61.9 | 61.9, 2.6 % | 33.4, 2.7 % |
| whole set | NPC medium | 12235 | 432.5 | 432.5, 17.9 % | 225.2, 17.9 % |
| whole set | NPC large | 7151 | 1517.8 | 1517.8, 62.7 % | 771.5, 61.3 % |
| whole set | not listed | 7198 | 22.0 | 22.0, 0.9 % | 12.2, 1.0 % |
| whole set | of all these, above 40000 | 931 | 692.1 | 692.1, 28.6 % | 349.8, 27.8 % |
| creatures, five | NPC small | 355 | 3.0 | 3.0, 2.2 % | 1.6, 2.4 % |
| creatures, five | NPC medium | 700 | 26.1 | 26.1, 19.7 % | 13.6, 20.0 % |
| creatures, five | NPC large | 445 | 103.6 | 103.6, 78.1 % | 52.6, 77.6 % |
| creatures, five | of all these, above 40000 | 55 | 55.0 | 55.0, 41.4 % | 27.8, 40.9 % |
| creatures, faced | NPC small | 71 | 0.6 | 0.6, 2.3 % | 0.3, 2.4 % |
| creatures, faced | NPC medium | 140 | 5.3 | 5.3, 20.6 % | 2.8, 20.9 % |
| creatures, faced | NPC large | 89 | 19.9 | 19.9, 77.1 % | 10.1, 76.6 % |
| creatures, faced | of all these, above 40000 | 11 | 10.3 | 10.3, 40.0 % | 5.2, 39.5 % |
| dressed NPCs, five | equipment | 1237 | 5.3 | 5.3, 88.3 % | 3.0, 88.8 % |
| dressed NPCs, five | NPC small | 25 | 0.3 | 0.3, 5.4 % | 0.2, 5.2 % |
| dressed NPCs, five | NPC medium | 15 | 0.4 | 0.4, 6.3 % | 0.2, 6.0 % |
| dressed NPCs, faced | equipment | 250 | 1.1 | 1.1, 80.0 % | 0.6, 81.0 % |
| dressed NPCs, faced | NPC small | 9 | 0.1 | 0.1, 8.4 % | 0.1, 8.1 % |
| dressed NPCs, faced | NPC medium | 6 | 0.2 | 0.2, 11.5 % | 0.1, 10.9 % |
| uniform, five | equipment | 1135 | 3.7 | 3.7, 13.7 % | 2.1, 14.9 % |
| uniform, five | NPC small | 85 | 1.0 | 1.0, 3.7 % | 0.5, 3.8 % |
| uniform, five | NPC medium | 130 | 4.5 | 4.5, 16.5 % | 2.3, 16.6 % |
| uniform, five | NPC large | 70 | 17.9 | 17.9, 66.0 % | 9.1, 64.5 % |
| uniform, five | not listed | 50 | 0.1 | 0.1, 0.2 % | 0.0, 0.3 % |
| uniform, five | of all these, above 40000 | 15 | 8.9 | 8.9, 33.0 % | 4.5, 32.1 % |
| uniform, faced | equipment | 233 | 0.8 | 0.8, 14.1 % | 0.4, 15.4 % |
| uniform, faced | NPC small | 17 | 0.2 | 0.2, 3.6 % | 0.1, 3.7 % |
| uniform, faced | NPC medium | 26 | 1.0 | 1.0, 17.5 % | 0.5, 17.5 % |
| uniform, faced | NPC large | 14 | 3.5 | 3.5, 64.6 % | 1.8, 63.0 % |
| uniform, faced | not listed | 10 | 0.0 | 0.0, 0.2 % | 0.0, 0.3 % |
| uniform, faced | of all these, above 40000 | 3 | 1.8 | 1.8, 33.1 % | 0.9, 32.1 % |
| worst, five | NPC medium | 25 | 8.4 | 8.4, 0.9 % | 4.3, 0.9 % |
| worst, five | NPC large | 1471 | 909.5 | 909.5, 99.1 % | 460.5, 99.1 % |
| worst, five | of all these, above 40000 | 676 | 650.9 | 650.9, 70.9 % | 329.0, 70.8 % |
| worst, faced | NPC medium | 5 | 1.6 | 1.6, 0.9 % | 0.8, 0.9 % |
| worst, faced | NPC large | 295 | 187.8 | 187.8, 99.1 % | 95.1, 99.1 % |
| worst, faced | of all these, above 40000 | 136 | 135.3 | 135.3, 71.5 % | 68.4, 71.3 % |

**Enhanced**

| scope | class | streams | Mpx | R8 / RG8 | BC1 |
|---|---|---:|---:|---:|---:|
| whole set | equipment | 130028 | 1059.6 | 2119.2, 21.6 % | 581.3, 22.8 % |
| whole set | NPC small | 3740 | 64.4 | 128.8, 1.3 % | 34.5, 1.3 % |
| whole set | NPC medium | 14364 | 873.8 | 1747.6, 17.8 % | 453.5, 17.8 % |
| whole set | NPC large | 10001 | 2919.1 | 5838.2, 59.4 % | 1484.0, 58.1 % |
| whole set | of all these, above 40000 | 1471 | 1157.2 | 2314.4, 23.5 % | 585.2, 22.9 % |
| creatures, five | NPC small | 215 | 3.4 | 6.8, 1.7 % | 1.8, 1.8 % |
| creatures, five | NPC medium | 735 | 44.0 | 87.9, 22.2 % | 22.8, 22.5 % |
| creatures, five | NPC large | 550 | 150.9 | 301.8, 76.1 % | 76.7, 75.7 % |
| creatures, five | of all these, above 40000 | 90 | 60.7 | 121.4, 30.6 % | 30.7, 30.3 % |
| creatures, faced | NPC small | 43 | 0.7 | 1.4, 1.7 % | 0.4, 1.8 % |
| creatures, faced | NPC medium | 147 | 8.7 | 17.3, 21.7 % | 4.5, 22.0 % |
| creatures, faced | NPC large | 110 | 30.6 | 61.2, 76.6 % | 15.6, 76.2 % |
| creatures, faced | of all these, above 40000 | 18 | 12.3 | 24.7, 30.9 % | 6.2, 30.6 % |
| dressed NPCs, five | equipment | 1242 | 8.5 | 17.0, 89.4 % | 4.7, 89.8 % |
| dressed NPCs, five | NPC small | 20 | 0.4 | 0.8, 4.2 % | 0.2, 4.1 % |
| dressed NPCs, five | NPC medium | 15 | 0.6 | 1.2, 6.4 % | 0.3, 6.1 % |
| dressed NPCs, faced | equipment | 250 | 1.7 | 3.5, 63.2 % | 1.0, 64.2 % |
| dressed NPCs, faced | NPC small | 20 | 0.4 | 0.8, 14.5 % | 0.2, 14.4 % |
| dressed NPCs, faced | NPC medium | 15 | 0.6 | 1.2, 22.3 % | 0.3, 21.4 % |
| uniform, five | equipment | 1237 | 10.5 | 21.0, 21.0 % | 5.8, 22.2 % |
| uniform, five | NPC small | 35 | 0.7 | 1.4, 1.4 % | 0.4, 1.4 % |
| uniform, five | NPC medium | 116 | 7.2 | 14.4, 14.5 % | 3.8, 14.5 % |
| uniform, five | NPC large | 100 | 31.5 | 63.0, 63.1 % | 16.0, 61.8 % |
| uniform, five | of all these, above 40000 | 20 | 10.3 | 20.6, 20.7 % | 5.2, 20.2 % |
| uniform, faced | equipment | 249 | 2.2 | 4.5, 21.4 % | 1.2, 22.6 % |
| uniform, faced | NPC small | 7 | 0.1 | 0.3, 1.3 % | 0.1, 1.3 % |
| uniform, faced | NPC medium | 24 | 1.5 | 3.1, 14.9 % | 0.8, 15.0 % |
| uniform, faced | NPC large | 20 | 6.5 | 13.0, 62.4 % | 3.3, 61.1 % |
| uniform, faced | of all these, above 40000 | 4 | 2.2 | 4.3, 20.8 % | 1.1, 20.3 % |
| worst, five | NPC medium | 5 | 5.3 | 10.6, 0.4 % | 2.7, 0.4 % |
| worst, five | NPC large | 1491 | 1225.8 | 2451.5, 99.6 % | 619.9, 99.6 % |
| worst, five | of all these, above 40000 | 1131 | 1050.0 | 2100.0, 85.3 % | 530.7, 85.3 % |
| worst, faced | NPC medium | 1 | 0.9 | 1.9, 0.4 % | 0.5, 0.4 % |
| worst, faced | NPC large | 299 | 250.8 | 501.7, 99.6 % | 126.8, 99.6 % |
| worst, faced | of all these, above 40000 | 227 | 214.5 | 429.1, 85.2 % | 108.4, 85.2 % |

## 12. The repo's BC7 encoder

**The BC7 rows in this document used a buggy encoder, and so does every package built with `--bc7`.** The repo's encoder can pack a block that decodes differently from what its own search scored. The trace below names the cause, and it runs on a verbatim scratch copy (`bc7-trace`, `lib/udd-image-codecs/src/bc7/{analytical,tables,rdo}.rs` copied unchanged). The copy reproduces the repo's real call byte for byte on the named sample: `cmp` on the decoded output of `lossy.rs`'s `Coder::Bc7` and `Coder::Bleed` rows against the copy's rows, on both clients. **The repo's encoder is unchanged.** The fixes below are a proposal for the user, because they change every package built with BC7.

**What it changes on animation,** SSIMULACRA2 median on the named sample, with zstd 19 scaled to the whole set:

| encoder | Classic | MB | Enhanced | MB |
|---|---:|---:|---:|---:|
| repo, as the converter calls it | 61.49 | 987 | 58.31 | 1857 |
| repo + colour under transparent pixels | 75.01 | 1064 | **28.77** | 1963 |
| fix A | 61.85 | 987 | 71.10 | 1855 |
| fix A + colour | **85.86** | 1064 | **77.91** | 1951 |
| fix A + B, mode 7 on + colour | 86.16 | 1111 | 79.25 | 2009 |
| **fix C** | **86.31** | 1008 | **86.12** | 1841 |
| fix C + colour | 85.94 | 1065 | 87.36 | 1943 |
| fix C + B, mode 7 on + colour | 86.20 | 1112 | 88.12 | 1994 |
| ISPC basic + colour | 86.44 | 1132 | 88.29 | 2041 |
| for scale: BC1, `dds` High | 80.62 | 564 | 71.45 | 877 |

**So BC1 against BC7 can't be decided from the repo's BC7 rows.** With fix C, BC7 beats BC1 by 6 points on Classic at 1.8× the disk, and by 15 on Enhanced at 2.1×. The gap to ISPC on Enhanced is most of it a search problem rather than a limit: fix A alone stays 10 points under ISPC, and fix C closes all but 0.8 of them on the median. p5 and the worst body stay 5 to 9 points under ISPC.

**On every body the result holds (section 16):** the repo's median is 67.39 on Classic and 60.21 on Enhanced, fix A 68.37 and 69.99, fix C 87.51 and 85.40, and fix C + colour 87.89 and 87.14. Bodies at 80 or better out of 1022 and 926 go from 191 and 93 to 782 and 712 with fix C.

**Sample:** every 40th body by pixels, 26 Classic bodies (3221 streams) and 24 Enhanced bodies (4459 streams), classes in the table further down. `q` at every 10th body (103 and 93 bodies) repeats the repo rows.

**Labels:** every BC7 row names its encoder.

- `repo` (`BC7`, `bc7` in `q`, `qc`, `qm`, and all earlier sections): the repo's encoder as the converter calls it. Flags `FLAG_PBIT_OPT_M6 | FLAG_USE_DUAL_PLANE | FLAG_USE_TRIVIAL_M6`.
- `repo + colour under transparent pixels` (`BC7, colour under transparent pixels`, `bc7-bleed`): the same, with each transparent pixel's colour replaced by its block's alpha-weighted mean before encoding.
- `fix A`, `fix A + B`, `fix C`: the verbatim copy with the fixes below. Fix C includes fix A. Scratch only.
- `ISPC basic`: Intel's ISPC compressor through `intel_tex_2` 0.5.0. Scratch only. It's about 700× slower than the repo's path on these one-block-wide strips, which leave its SIMD lanes empty, so that figure isn't its real speed.

### The cause

**Bug A: `encode_mode4` with index flag 0 swaps its weight fields.** This is a real bug, and the converter's flags reach it. BC7 mode 4 stores a 2-bit weight plane and a 3-bit plane in fixed places, and the index flag only says which plane is colour. The code packs the 3-bit plane into the 2-bit field when the flag is 0. The candidate's predicted error uses the intended weights, so it wins the search and then decodes wrong. `pack_mode4_or_5` also runs for opaque blocks with a decorrelated colour channel, so opaque art, land and the radar reach it, not only sprites. The one existing test, `mode4_fused_error_bounds`, calls it with index flag 1 only.

```diff
--- a/lib/udd-image-codecs/src/bc7/analytical.rs (encode_mode4)
-    let (p2, p3) = if index_flag == 1 {
-        (&w1, &w0)
-    } else {
-        (&w0, &w1)
-    };
+    let (p2, p3) = (&w1, &w0);
```

**Bugs B: real, but the converter's flags don't reach them.** They explain why "all modes" scored worse than the default.

- `encode_mode1` swaps the endpoints of all subsets, and the p-bits, when only the first subset's anchor is inverted.
- `encode_mode2` does the same.
- `pack_bc7_rgb`'s 3-subset search offers mode 0 partitions up to 63, in a 4-bit field.

Only `FLAG_USE_2SUBSETS` and `FLAG_USE_3SUBSETS` reach these, and `benches/bc7_encode.rs` is the only caller that sets them.

**Field test.** Each encoder entry point with random endpoints and weights, 20000 blocks a row, against the format's formula, through two independent decoders (`texture2ddecoder`, `dds`). They agree on every count.

| encoder call | blocks | differ | mean squared error a channel |
|---|---:|---:|---:|
| `encode_mode0`, partition under 16 | 5123 | 0 | 0 |
| `encode_mode0`, partition 16 or more (the search offers these) | 14877 | 14877 | 3585.1 |
| `encode_mode1`, first pixel's weight under 4 | 9969 | 0 | 0 |
| `encode_mode1`, first pixel's weight 4 or more | 10031 | 10031 | 1606.2 |
| `encode_mode2`, first pixel's weight under 2 | 10004 | 0 | 0 |
| `encode_mode2`, first pixel's weight 2 or more | 9996 | 9996 | 3066.3 |
| `encode_mode3` | 20000 | 0 | 0 |
| `encode_mode4`, index flag 1, rotations 0 and 1 | 20000 each | 0 | 0 |
| **`encode_mode4`, index flag 0, rotation 0** | 20000 | **20000** | 2924.7 |
| **`encode_mode4`, index flag 0, rotation 1** | 20000 | **20000** | 2906.7 |
| `encode_mode5`, `encode_mode6`, `encode_mode7` | 20000 each | 0 | 0 |

**Transparent black: a limit of the design, not a bit bug.** Error is plain RGBA squared error, so colour under alpha 0 counts as data. In a mixed block that does two things:

- It pulls mode 6's single 4D line toward black.
- It makes alpha look correlated with colour, so `detect_dp_channel` doesn't offer the separate alpha plane.

Replacing colour under alpha 0 with the block's mean before encoding removes both. An encoder could instead give those pixels zero weight, which is more work.

**Hand-made blocks,** each encoded and then decoded for real. Squared error over the block:

| block | repo | its `pack_mode4_or_5` on alpha | with fix A | ISPC basic |
|---|---:|---:|---:|---:|
| half transparent black, half one colour | 8, mode 6 | reports 0, decodes 8 | 8 | 0, mode 5 |
| half transparent black, two colours | **376052**, mode 4, 3-bit alpha weights | reports and decodes 120368 | 57056 | 104, mode 7 |
| the same, mean colour under the transparent pixels | 1632, mode 4, 3-bit colour weights | 1632 | 1632 | 368, mode 7 |
| one colour under coverage levels, colour bled | 486, mode 6 | **reports 1656, decodes 308136** | 486 | 493, mode 6 |
| the same, one transparent-black pixel | **57706**, mode 6 | reports 1642, decodes 392626 | 57706 | 1594, mode 4 |
| opaque, red against green and blue | 24, mode 5 | 112876 | 24 | 56, mode 5 |

**Why bleeding helps Classic and hurts Enhanced in the repo's encoder.** Bleeding decorrelates alpha, so the separate-alpha modes 4 and 5 start to win.

- Classic's alpha is 0 or 255, which fits 2-bit alpha weights, and those decode correctly. Only 0.53 % of its mixed blocks take the broken path.
- Enhanced's alpha is partial, so it wants 3-bit alpha weights: 37.66 % of its partial blocks take the broken path, at 17.88 dB and alpha RMS 119.16.
- **With fix A, bleeding helps both** (those blocks reach 31.81 dB and alpha RMS 5.17).

**Fix C: the search misses alpha's own plane.** This isn't a wrong bit but a wrong choice, and it mattered more than the bug on Enhanced. Two things stack up in `pack_bc7_rgba`:

- `detect_dp_channel` only offers mode 4 or 5's separate alpha plane when alpha's correlation with each of R, G and B is under 0.35. A sprite's edge has alpha falling with colour, so the plane isn't offered where it would help most.
- When it is offered, the two candidates are compared by `pack_mode4_or_5`'s estimate rather than by a real decode.

So fix C tries the plane on every block whose alpha varies, decodes both candidates, and keeps the closer one. That's also why the colour step looked so strong under fix A: it worked indirectly, by decorrelating alpha so the plane got offered. With fix C the colour step adds little. Measured with the exact lines proposed below: a scratch copy holding only those lines gives output byte-identical to the measured `bc7-fixc` row on the named sample, both clients (`cmp`). Opaque blocks return to `pack_bc7_rgb` first, so fix C doesn't touch them.

**What each step is worth on the named sample,** SSIMULACRA2 median / p5 / min, zstd 19 MB scaled to the whole set, and encode cost in worker seconds a Mpx at idle:

| encoder | Classic | MB | s/Mpx | Enhanced | MB | s/Mpx |
|---|---|---:|---:|---|---:|---:|
| repo | 61.49 / 49.59 / 38.33 | 987 | 0.05 | 58.31 / 45.85 / 44.21 | 1857 | 0.05 |
| fix A | 61.85 / 50.82 / 45.86 | 987 | 0.05 | 71.10 / 59.09 / 58.70 | 1855 | 0.05 |
| fix A + colour | 85.86 / 66.40 / 55.41 | 1064 | 0.09 | 77.91 / 66.72 / 62.51 | 1951 | 0.09 |
| **fix C** | 86.31 / 61.22 / 54.52 | 1008 | 0.14 | 86.12 / 65.25 / 65.08 | 1841 | 0.14 |
| fix C + colour | 85.94 / 66.41 / 55.44 | 1065 | 0.19 | 87.36 / 70.19 / 70.06 | 1943 | 0.21 |
| ISPC basic | 86.46 / 68.65 / 62.95 | 1085 | 39 (see Labels) | 86.94 / 74.10 / 70.34 | 1988 | 40 (see Labels) |
| ISPC basic + colour | 86.44 | 1132 | 42 | 88.29 / 80.18 / 75.74 | 2041 | 36 |

The colour step's worth after fix C: disk +5.7 % on Classic and +5.5 % on Enhanced; median −0.37 and +1.24; p5 +5.19 and +4.94; worst body +0.92 and +4.98. So it buys the tail rather than the median, at about the price of a step of RDO.

**What's left against ISPC is design, not bugs:**

- The converter's flags leave out the subset modes. Mode 7 alone adds 2 to 3 dB on opaque blocks, at 26 to 37× the encode time.
- Mode 7's path has no least-squares pass and seeds from an RGB axis only.
- `pack_mode4_or_5` reports `rgb_sse + a_sse / 2`, the mean of two passes rather than the true error, and evaluates mode 5 without the decoder's bit replication.
- Mode 4's and mode 5's quantisation of the plane is what fix C stops at: Enhanced's partial blocks reach 29.46 dB against ISPC's 30.62, with alpha RMS 7.25 against 7.12, so alpha is level and colour is what remains.

#### What reaches the packages: a sample of each

Each package's raw pages are encoded through the verbatim copy with the converter's flags, then scored against themselves over grey, in 512 px windows with at least 10 % painted. The "as built" row is the same call a `--bc7` build makes. Only the ISPC rows run on every 3rd page, so their zstd figure is not comparable with the others.

| package and sample | encoder | mode 4 with 3-bit alpha weights, % of blocks | their colour dB | zstd 19 MB | painted colour dB | opaque dB | mixed binary dB | mixed partial dB | alpha RMS | SSIMULACRA2 median | p5 | min |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `tex_art_ec`, 9 of 9 pages, 73.4 Mpx | as built today | 1.948 | 17.20 | 41.2 | 29.49 | 37.45 | 25.87 | 22.44 | 21.50 | 53.25 | 4.80 | -50.94 |
|  | fix A | 1.948 | 34.35 | 41.2 | 32.50 | 39.11 | 26.14 | 28.62 | 7.28 | 71.57 | 64.72 | 52.86 |
|  | **fix C** | 4.345 | 34.63 | 41.5 | 37.71 | 39.11 | 35.15 | 34.86 | 1.73 | 87.46 | 82.27 | 75.56 |
|  | fix A + colour | 2.999 | 34.92 | 40.9 | 36.46 | 39.11 | 36.24 | 30.51 | 3.04 | 83.50 | 72.90 | 64.04 |
|  | fix C + colour | 4.389 | 34.87 | 41.0 | 38.19 | 39.11 | 37.15 | 35.26 | 1.67 | 87.81 | 83.10 | 79.98 |
|  | fix A + B, mode 7 on + colour | 2.849 | 34.96 | 43.8 | 37.96 | 42.20 | 36.46 | 31.29 | 2.97 | 84.85 | 74.74 | 65.74 |
|  | ISPC basic | 4.395 | 37.07 | not comparable | 41.93 | 44.79 | 36.71 | 35.75 | 1.10 | 90.55 | 88.31 | 85.82 |
| `tex_art_cc`, 9 of 46 pages, 37.4 Mpx | as built today | 0.321 | 16.52 | 7.7 | 27.55 | 32.10 | 23.33 | NaN | 12.64 | 64.95 | 52.87 | 47.05 |
|  | fix A | 0.321 | 28.53 | 7.7 | 28.14 | 34.17 | 23.51 | NaN | 11.42 | 70.16 | 61.67 | 59.18 |
|  | **fix C** | 0.351 | 29.81 | 8.0 | 33.20 | 34.17 | 31.42 | NaN | 0.56 | 86.80 | 79.56 | 72.60 |
|  | fix A + colour | 0.359 | 31.49 | 8.4 | 33.59 | 34.17 | 32.39 | NaN | 0.73 | 86.38 | 71.60 | 65.08 |
|  | fix C + colour | 0.359 | 31.54 | 8.5 | 34.05 | 34.17 | 33.75 | NaN | 0.07 | 87.77 | 80.02 | 75.98 |
|  | fix A + B, mode 7 on + colour | 0.257 | 31.54 | 8.9 | 35.61 | 37.18 | 33.13 | NaN | 0.62 | 88.53 | 76.17 | 70.51 |
|  | ISPC basic | 3.906 | 40.23 | not comparable | 36.95 | 38.73 | 34.39 | NaN | 0.52 | 89.37 | 86.87 | 86.56 |
| `tex_land_cc`, 4 of 4 pages, 16.4 Mpx | as built today | 0.974 | 22.59 | 10.9 | 35.69 | 35.71 | 26.20 | NaN | 0.26 | 74.56 | 62.73 | 57.45 |
|  | fix A | 0.974 | 35.46 | 10.9 | 36.65 | 36.68 | 26.20 | NaN | 0.26 | 86.32 | 82.38 | 81.74 |
|  | **fix C** | 0.974 | 35.46 | 10.9 | 36.67 | 36.68 | 32.74 | NaN | 0.03 | 86.42 | 82.38 | 81.74 |
|  | fix A + colour | 0.975 | 35.46 | 10.9 | 36.68 | 36.68 | 35.92 | NaN | 0.01 | 86.42 | 82.38 | 81.74 |
|  | fix C + colour | 0.975 | 35.46 | 10.9 | 36.68 | 36.68 | 36.33 | NaN | 0.00 | 86.42 | 82.38 | 81.74 |
|  | fix A + B, mode 7 on + colour | 0.706 | 35.56 | 12.0 | 38.52 | 38.52 | 36.02 | NaN | 0.01 | 89.18 | 84.77 | 84.12 |
|  | ISPC basic | 1.802 | 38.57 | not comparable | 40.14 | 40.14 | 37.62 | NaN | 0.06 | 89.79 | 88.93 | 88.76 |
| `tex_land_ec`, 6 of 6 pages, 19.5 Mpx | as built today | 0.241 | 22.17 | 12.5 | 39.66 | 41.43 | 26.19 | 30.59 | 3.68 | 86.78 | 55.09 | 45.23 |
|  | fix A | 0.241 | 36.27 | 12.5 | 40.37 | 41.63 | 26.29 | 32.61 | 1.90 | 90.17 | 78.81 | 52.72 |
|  | **fix C** | 0.931 | 35.03 | 12.5 | 41.30 | 41.63 | 35.90 | 37.09 | 0.64 | 90.60 | 84.64 | 83.84 |
|  | fix A + colour | 0.284 | 36.00 | 12.5 | 40.36 | 41.63 | 28.89 | 32.24 | 1.40 | 90.34 | 78.81 | 58.38 |
|  | fix C + colour | 0.927 | 35.09 | 12.5 | 41.34 | 41.63 | 41.40 | 37.27 | 0.63 | 90.60 | 84.64 | 84.20 |
|  | fix A + B, mode 7 on + colour | 0.247 | 36.04 | 13.8 | 41.82 | 43.45 | 28.93 | 32.98 | 1.37 | 90.76 | 84.31 | 58.40 |
|  | ISPC basic | 1.389 | 37.86 | not comparable | 44.25 | 44.57 | 39.94 | 36.66 | 0.51 | 92.30 | 91.00 | 90.52 |
| `tex_aux_ec`, 45 of 503 pages, 0.7 Mpx | as built today | 2.706 | 21.62 | 0.4 | 34.58 | 50.39 | 22.43 | 31.53 | 17.93 | 87.34 | 43.91 | -0.34 |
|  | fix A | 2.706 | 47.12 | 0.3 | 39.23 | 50.39 | 24.66 | 37.71 | 2.79 | 91.30 | 74.32 | 70.29 |
|  | **fix C** | 13.914 | 48.64 | 0.4 | 49.33 | 50.39 | 41.83 | 48.63 | 1.06 | 95.09 | 91.90 | 89.86 |
|  | fix A + colour | 2.953 | 45.51 | 0.3 | 41.66 | 50.39 | 38.56 | 37.80 | 2.18 | 93.16 | 78.41 | 69.54 |
|  | fix C + colour | 13.479 | 47.83 | 0.4 | 48.96 | 50.39 | 40.36 | 48.11 | 1.09 | 93.65 | 91.90 | 89.76 |
|  | fix A + B, mode 7 on + colour | 2.948 | 45.51 | 0.3 | 41.66 | 50.40 | 38.57 | 37.80 | 2.18 | 93.16 | 78.41 | 69.54 |
|  | ISPC basic | 18.699 | 48.54 | not comparable | 52.58 | 56.78 | 43.56 | 47.17 | 1.14 | 96.31 | 93.69 | 90.39 |
| `effects_ec`, 19 of 177 pages, 0.3 Mpx | as built today | 0.960 | 18.37 | 0.2 | 32.35 | 38.95 | NaN | 29.92 | 12.62 | 86.20 | 46.87 | 31.62 |
|  | fix A | 0.960 | 34.42 | 0.2 | 34.31 | 40.94 | NaN | 31.87 | 6.52 | 91.46 | 76.61 | 51.07 |
|  | **fix C** | 3.635 | 32.74 | 0.2 | 39.72 | 40.94 | NaN | 38.80 | 2.69 | 92.92 | 80.65 | 76.61 |
|  | fix A + colour | 2.688 | 33.80 | 0.2 | 35.72 | 40.94 | NaN | 33.49 | 5.18 | 91.51 | 76.61 | 53.59 |
|  | fix C + colour | 5.028 | 33.32 | 0.2 | 39.92 | 40.94 | NaN | 39.13 | 2.34 | 92.94 | 80.63 | 76.61 |
|  | fix A + B, mode 7 on + colour | 2.583 | 33.73 | 0.2 | 37.27 | 43.41 | NaN | 34.90 | 4.97 | 92.13 | 79.45 | 60.54 |
|  | ISPC basic | 1.325 | 36.42 | not comparable | 45.22 | 46.98 | NaN | 42.67 | 1.10 | 94.65 | 84.43 | 84.43 |
| `cc_radar`, map 0 (always opaque), 1 of 1 pages, 29.4 Mpx | as built today | 1.386 | 19.05 | 3.7 | 35.97 | 35.97 | NaN | NaN | 0.00 | 54.51 | 13.96 | 0.16 |
|  | fix A | 1.386 | 36.19 | 3.7 | 40.76 | 40.76 | NaN | NaN | 0.00 | 86.15 | 76.16 | 72.18 |
|  | **fix C** | 1.386 | 36.19 | 3.7 | 40.76 | 40.76 | NaN | NaN | 0.00 | 86.15 | 76.16 | 72.18 |
|  | fix A + colour | 1.386 | 36.19 | 3.7 | 40.76 | 40.76 | NaN | NaN | 0.00 | 86.15 | 76.16 | 72.18 |
|  | fix C + colour | 1.386 | 36.19 | 3.7 | 40.76 | 40.76 | NaN | NaN | 0.00 | 86.15 | 76.16 | 72.18 |
|  | fix A + B, mode 7 on + colour | 0.559 | 36.35 | 4.1 | 45.30 | 45.30 | NaN | NaN | 0.00 | 89.92 | 80.98 | 78.54 |
|  | ISPC basic | 0.667 | 36.27 | not comparable | 47.21 | 47.21 | NaN | NaN | 0.01 | 91.52 | 82.49 | 79.20 |

**Real packages agree:** a real `--bc7` `tex_land_cc` build holds 9745 of its 1310720 blocks (0.743 %) in mode 4 with 3-bit alpha weights, in rotations 1 to 3. The sample has 0.974 %, on the raw pages' slightly different layout. `udd-cli tool extract` can't read a `--bc7` art package (`invalid BC7 block payload length: expected 4112784, got 4194304`, `tools/udd-conv-cli/src/extract.rs:513`), so the art packages' counts come from the verbatim copy only.

**`mobile_anim_ec` and `mobile_anim_cc`** are the mobile rows below. Their frames are aligned to 4, so the strips are the atlas's blocks. `docs/TODO.md:85` closed land BC7 RDO using this encoder, so that closure predates this finding.

#### Classic's animations, the named sample: colour PSNR over painted pixels

| encoder | zstd 7 MB | zstd 19 MB | worker s a Mpx, idle | all, dB | opaque, dB | mixed binary, dB | its alpha RMS |
|---|---:|---:|---:|---:|---:|---:|---:|
| repo encoder, as the converter calls it | 19.9 | 19.6 | 0.06 | 24.94 | 35.95 | 20.20 | 28.59 |
| repo encoder, colour under transparent pixels | 21.5 | 21.1 | 0.13 | 34.38 | 35.95 | 32.20 | 8.66 |
| repo encoder, all modes | 21.8 | 21.6 | 3.07 | 22.50 | 23.04 | 21.53 | 24.34 |
| fix A | 19.9 | 19.6 | 0.06 | 24.96 | 36.22 | 20.20 | 28.46 |
| fix A, colour under transparent pixels | 21.5 | 21.1 | 0.13 | 34.79 | 36.22 | 32.74 | 0.12 |
| fix A + B, mode 7 on | 21.8 | 21.6 | 2.23 | 26.31 | 37.95 | 21.54 | 24.25 |
| fix A + B, mode 7 on, colour under transparent pixels | 22.4 | 22.1 | 2.51 | 35.59 | 37.95 | 32.79 | 0.35 |
| fix A + B, all modes | 21.8 | 21.6 | 2.81 | 26.32 | 38.07 | 21.54 | 24.25 |
| fix A + B, all modes, colour under transparent pixels | 22.4 | 22.1 | 2.67 | 35.63 | 38.07 | 32.79 | 0.35 |
| fix A only, all modes (B's bugs left in) | 21.8 | 21.6 | 2.55 | 22.51 | 23.04 | 21.54 | 24.25 |
| ISPC BC7, basic | 21.8 | 21.5 | 38.83 | 35.56 | 38.76 | 32.29 | 0.42 |
| ISPC BC7, basic, colour under transparent pixels | 22.8 | 22.5 | 41.96 | 36.04 | 38.76 | 33.02 | 0.56 |
| **fix C** | 20.3 | 20.0 | 0.14 | 34.22 | 36.22 | 31.69 | 0.11 |
| fix C, colour under transparent pixels | 21.5 | 21.2 | 0.19 | 34.81 | 36.22 | 32.77 | 0.03 |
| fix C + B, mode 7 on, colour under transparent pixels | 22.4 | 22.1 | 2.07 | 35.60 | 37.95 | 32.82 | 0.32 |
| fix C, RDO 0.5 | 19.3 | 18.8 | 4.12 | 33.20 | 34.82 | 30.97 | 0.14 |
| fix C, RDO 1 | 18.4 | 17.9 | 3.62 | 31.82 | 32.93 | 30.11 | 0.24 |
| fix C, RDO 2 | 17.1 | 16.7 | 2.87 | 29.40 | 29.75 | 28.72 | 0.49 |
| fix C, colour under transparent pixels, RDO 0.5 | 20.4 | 19.8 | 3.74 | 33.75 | 34.82 | 32.08 | 0.12 |
| fix C, colour under transparent pixels, RDO 1 | 19.4 | 18.9 | 3.11 | 32.29 | 32.93 | 31.15 | 0.26 |
| fix C, colour under transparent pixels, RDO 2 | 18.0 | 17.5 | 2.83 | 29.71 | 29.75 | 29.61 | 0.56 |
| fix C + B, mode 7 on, colour under transparent pixels, RDO 1 | 21.0 | 20.5 | 3.94 | 33.05 | 34.26 | 31.22 | 0.39 |

#### Enhanced's animations, the named sample: colour PSNR over painted pixels

| encoder | zstd 7 MB | zstd 19 MB | worker s a Mpx, idle | all, dB | opaque, dB | mixed binary, dB | its alpha RMS | mixed partial, dB | its alpha RMS |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| repo encoder, as the converter calls it | 42.7 | 42.0 | 0.05 | 25.05 | 34.94 | 24.44 | 18.86 | 22.64 | 29.97 |
| repo encoder, colour under transparent pixels | 45.2 | 44.4 | 0.10 | 24.10 | 34.94 | 35.74 | 9.29 | 21.64 | 74.09 |
| repo encoder, all modes | 44.7 | 44.0 | 1.81 | 24.38 | 24.92 | 26.20 | 14.89 | 23.97 | 27.47 |
| fix A | 42.7 | 42.0 | 0.05 | 25.78 | 35.67 | 24.49 | 18.07 | 23.37 | 22.49 |
| fix A, colour under transparent pixels | 44.9 | 44.1 | 0.10 | 30.22 | 35.67 | 36.31 | 0.17 | 28.20 | 11.90 |
| fix A + B, mode 7 on | 44.6 | 44.0 | 1.31 | 27.40 | 38.85 | 26.25 | 14.43 | 24.93 | 19.40 |
| fix A + B, mode 7 on, colour under transparent pixels | 46.1 | 45.4 | 1.61 | 31.39 | 38.85 | 36.53 | 0.32 | 29.14 | 10.43 |
| fix A + B, all modes | 44.6 | 44.0 | 1.58 | 27.41 | 39.14 | 26.25 | 14.43 | 24.93 | 19.40 |
| fix A + B, all modes, colour under transparent pixels | 46.1 | 45.4 | 2.09 | 31.41 | 39.14 | 36.53 | 0.32 | 29.14 | 10.43 |
| fix A only, all modes (B's bugs left in) | 44.6 | 44.0 | 2.00 | 24.94 | 24.95 | 26.25 | 14.43 | 24.93 | 19.40 |
| ISPC BC7, basic | 45.6 | 45.0 | 39.70 | 32.86 | 40.22 | 35.43 | 0.59 | 30.62 | 7.12 |
| ISPC BC7, basic, colour under transparent pixels | 46.8 | 46.1 | 36.43 | 33.97 | 40.22 | 37.11 | 0.48 | 31.84 | 6.06 |
| **fix C** | 42.5 | 41.6 | 0.14 | 31.30 | 35.67 | 32.57 | 0.48 | 29.46 | 7.25 |
| fix C, colour under transparent pixels | 44.8 | 43.9 | 0.21 | 32.71 | 35.67 | 36.40 | 0.05 | 31.23 | 6.06 |
| fix C + B, mode 7 on, colour under transparent pixels | 45.8 | 45.1 | 1.63 | 33.39 | 38.85 | 36.57 | 0.26 | 31.37 | 6.09 |
| fix C, RDO 0.5 | 40.4 | 39.3 | 3.29 | 30.91 | 34.78 | 31.53 | 1.79 | 29.19 | 7.52 |
| fix C, RDO 1 | 38.9 | 37.8 | 2.88 | 30.24 | 33.34 | 30.24 | 2.76 | 28.71 | 7.95 |
| fix C, RDO 2 | 36.5 | 35.5 | 2.78 | 28.83 | 30.73 | 28.50 | 4.59 | 27.71 | 9.04 |
| fix C, colour under transparent pixels, RDO 0.5 | 42.4 | 41.3 | 3.86 | 32.18 | 34.78 | 34.28 | 1.34 | 30.80 | 6.33 |
| fix C, colour under transparent pixels, RDO 1 | 40.5 | 39.4 | 3.20 | 31.25 | 33.34 | 32.80 | 2.31 | 30.06 | 6.84 |
| fix C, colour under transparent pixels, RDO 2 | 37.6 | 36.5 | 3.70 | 29.41 | 30.74 | 30.77 | 3.36 | 28.56 | 8.23 |
| fix C + B, mode 7 on, colour under transparent pixels, RDO 1 | 42.1 | 41.1 | 4.00 | 31.80 | 35.10 | 32.95 | 2.25 | 30.22 | 6.82 |

#### The named sample's scores, on the animation mosaics

Every 40th body by pixels. MB are this sample's zstd 19 scaled to the whole set, so they run about 2 % under the every-10th rows: the repo's BC7 is 1857 MB here and 1890 MB there on Enhanced. `bc7-before` is byte-identical to `repo`, so it isn't repeated.

**Classic**

| encoder | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB |
|---|---:|---:|---:|---:|---:|---:|
| BC1 (`dds` High) | 563.9 | 80.62 | 63.85 | 52.47 | 15 of 26 | 28.73 |
| repo | 987.3 | 61.49 | 49.59 | 38.33 | 5 of 26 | 24.95 |
| repo + colour | 1064.2 | 75.01 | 60.81 | 49.17 | 6 of 26 | 34.60 |
| fix A | 987.3 | 61.85 | 50.82 | 45.86 | 5 of 26 | 24.96 |
| fix A + colour | 1064.1 | 85.86 | 66.40 | 55.41 | 19 of 26 | 34.79 |
| **fix C** | 1008.1 | 86.31 | 61.22 | 54.52 | 18 of 26 | 34.22 |
| **fix C, RDO 0.5** | 945.7 | 82.75 | 60.78 | 53.76 | 17 of 26 | 33.20 |
| **fix C, RDO 1** | 902.9 | 79.32 | 59.56 | 52.26 | 12 of 26 | 31.82 |
| fix C, RDO 2 | 838.7 | 70.86 | 51.68 | 49.44 | 1 of 26 | 29.40 |
| fix C + colour | 1065.4 | 85.94 | 66.41 | 55.44 | 19 of 26 | 34.81 |
| fix C + colour, RDO 0.5 | 999.1 | 83.02 | 64.31 | 55.19 | 17 of 26 | 33.75 |
| fix C + colour, RDO 1 | 952.5 | 80.68 | 60.29 | 54.56 | 14 of 26 | 32.29 |
| fix C + colour, RDO 2 | 883.6 | 71.88 | 53.33 | 52.09 | 4 of 26 | 29.71 |
| fix A + B, mode 7 on + colour | 1111.1 | 86.16 | 67.69 | 58.20 | 19 of 26 | 35.59 |
| fix C + B, mode 7 on + colour | 1112.1 | 86.20 | 67.70 | 58.22 | 19 of 26 | 35.60 |
| fix C + B, mode 7 on + colour, RDO 1 | 1033.5 | 81.13 | 66.55 | 57.22 | 15 of 26 | 33.05 |
| ISPC basic | 1085.3 | 86.46 | 68.65 | 62.95 | 19 of 26 | 35.56 |
| ISPC basic + colour | 1131.8 | 86.44 | 68.49 | 62.74 | 19 of 26 | 36.04 |
| ISPC basic + colour, RDO 1 | 1069.3 | 81.95 | 67.95 | 62.03 | 16 of 26 | 33.64 |

**Enhanced**

| encoder | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB |
|---|---:|---:|---:|---:|---:|---:|
| BC1 (`dds` High) | 876.9 | 71.45 | 28.49 | 16.78 | 5 of 24 | 30.28 |
| repo | 1856.8 | 58.31 | 45.85 | 44.21 | 2 of 24 | 25.11 |
| repo + colour | 1963.1 | 28.77 | -1.56 | -11.11 | 1 of 24 | 25.29 |
| fix A | 1855.4 | 71.10 | 59.09 | 58.70 | 6 of 24 | 25.78 |
| fix A + colour | 1950.8 | 77.91 | 66.72 | 62.51 | 10 of 24 | 30.29 |
| **fix C** | 1841.2 | 86.12 | 65.25 | 65.08 | 21 of 24 | 31.33 |
| **fix C, RDO 0.5** | 1738.0 | 83.58 | 64.14 | 64.12 | 20 of 24 | 30.96 |
| **fix C, RDO 1** | 1669.8 | 81.38 | 62.91 | 62.89 | 15 of 24 | 30.28 |
| fix C, RDO 2 | 1568.1 | 74.81 | 59.72 | 59.35 | 6 of 24 | 28.88 |
| fix C + colour | 1943.1 | 87.36 | 70.19 | 70.06 | 21 of 24 | 32.73 |
| fix C + colour, RDO 0.5 | 1824.6 | 85.83 | 70.64 | 69.27 | 21 of 24 | 32.21 |
| fix C + colour, RDO 1 | 1741.0 | 82.47 | 68.79 | 67.42 | 18 of 24 | 31.30 |
| fix C + colour, RDO 2 | 1614.1 | 76.27 | 65.78 | 62.59 | 6 of 24 | 29.48 |
| fix A + B, mode 7 on + colour | 2008.5 | 79.25 | 70.98 | 65.92 | 12 of 24 | 31.44 |
| fix C + B, mode 7 on + colour | 1993.8 | 88.12 | 77.14 | 74.16 | 22 of 24 | 33.42 |
| fix C + B, mode 7 on + colour, RDO 1 | 1816.4 | 83.68 | 72.10 | 71.60 | 19 of 24 | 31.85 |
| ISPC basic | 1988.4 | 86.94 | 74.10 | 70.34 | 22 of 24 | 32.89 |
| ISPC basic + colour | 2040.7 | 88.29 | 80.18 | 75.74 | 23 of 24 | 33.99 |
| ISPC basic + colour, RDO 1 | 1894.8 | 84.10 | 75.55 | 72.93 | 19 of 24 | 32.41 |

**The same sample by class: MB, then SSIMULACRA2 median / min, RGB PSNR dB.** The older rows don't count bodies by class; the every-10th tables below do. Class sizes in this sample:

- Classic: equipment 11 bodies (6.5 Mpx), NPC small 3 (0.8), NPC medium 7 (20.6), NPC large 2 (19.7), not listed 3 (0.5).
- Enhanced: equipment 16 (43.5 Mpx), NPC small 1 (1.0), NPC medium 4 (15.4), NPC large 3 (51.3).
- **Too few bodies to trust a class on its own in small and large,** so the every-10th rows carry the class comparison.

**Classic**

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score |
|---|---:|---|---:|---|---:|---|---:|---|
| BC1 (`dds` High) | 93.6 | 79.55 / 63.85, 28.24 | 12.7 | 74.59 / 52.47, 25.84 | 245.2 | 81.67 / 79.34, 29.37 | 201.9 | 80.33 / 80.33, 28.46 |
| repo | 138.3 | 68.10 / 50.82, 21.89 | 20.3 | 49.59 / 38.33, 21.19 | 441.8 | 57.90 / 54.14, 25.40 | 368.3 | 61.49 / 61.49, 26.18 |
| fix A | 138.3 | 68.59 / 50.82, 21.89 | 20.3 | 53.86 / 45.86, 21.54 | 441.8 | 60.04 / 54.14, 25.40 | 368.3 | 61.48 / 61.48, 26.18 |
| fix A + colour | 170.8 | 80.68 / 66.40, 33.67 | 23.2 | 81.73 / 55.41, 28.65 | 467.9 | 89.61 / 88.75, 35.44 | 382.4 | 89.44 / 89.44, 35.46 |
| **fix C** | 152.4 | 78.14 / 61.22, 32.54 | 21.4 | 81.43 / 54.52, 28.17 | 446.0 | 89.20 / 87.45, 34.89 | 369.7 | 89.25 / 89.25, 35.16 |
| **fix C, RDO 0.5** | 140.6 | 75.81 / 60.78, 31.53 | 20.4 | 80.92 / 53.76, 27.91 | 421.3 | 86.46 / 84.67, 33.76 | 346.5 | 86.11 / 86.11, 34.00 |
| **fix C, RDO 1** | 135.6 | 73.68 / 59.56, 30.44 | 19.9 | 78.03 / 52.26, 27.51 | 402.5 | 81.26 / 78.62, 32.24 | 329.2 | 81.15 / 81.15, 32.44 |
| fix C + colour | 171.4 | 80.69 / 66.41, 33.73 | 23.2 | 81.75 / 55.44, 28.66 | 468.2 | 89.61 / 88.79, 35.45 | 382.8 | 89.49 / 89.49, 35.47 |
| ISPC basic + colour | 175.9 | 81.58 / 68.49, 34.28 | 24.1 | 85.97 / 62.74, 30.60 | 502.7 | 90.34 / 89.33, 36.69 | 408.3 | 90.01 / 90.01, 36.70 |

**Enhanced**

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score |
|---|---:|---|---:|---|---:|---|---:|---|
| BC1 (`dds` High) | 387.4 | 75.44 / 66.29, 29.05 | 12.2 | 71.45 / 71.45, 26.26 | 133.0 | 58.92 / 49.41, 32.41 | 344.4 | 28.49 / 16.78, 31.39 |
| repo | 800.8 | 64.51 / 45.85, 23.81 | 25.1 | 58.31 / 58.31, 22.69 | 300.4 | 46.25 / 44.21, 25.00 | 730.5 | 53.49 / 47.40, 27.12 |
| fix A | 800.1 | 67.18 / 58.70, 24.30 | 25.1 | 62.41 / 62.41, 22.97 | 300.1 | 71.30 / 62.71, 26.49 | 730.1 | 71.10 / 65.22, 27.79 |
| fix A + colour | 856.6 | 77.91 / 62.51, 29.49 | 26.7 | 71.11 / 71.11, 25.91 | 315.6 | 78.13 / 71.72, 30.96 | 751.9 | 77.55 / 72.99, 31.21 |
| **fix C** | 794.3 | 86.25 / 65.08, 30.50 | 24.9 | 82.54 / 82.54, 26.89 | 299.0 | 86.12 / 80.83, 31.98 | 723.0 | 85.65 / 83.90, 32.34 |
| **fix C, RDO 0.5** | 741.1 | 83.43 / 64.12, 30.12 | 24.4 | 80.91 / 80.91, 26.83 | 287.8 | 84.56 / 78.83, 31.62 | 684.9 | 84.54 / 83.21, 31.93 |
| **fix C, RDO 1** | 712.9 | 80.50 / 62.89, 29.52 | 23.8 | 79.30 / 79.30, 26.61 | 278.9 | 81.38 / 77.21, 30.94 | 654.1 | 81.44 / 80.89, 31.11 |
| fix C + colour | 853.2 | 87.81 / 70.06, 31.86 | 26.5 | 83.68 / 83.68, 28.15 | 315.0 | 87.21 / 86.37, 34.61 | 748.4 | 87.36 / 84.60, 33.41 |
| ISPC basic + colour | 889.4 | 88.29 / 75.74, 32.74 | 27.5 | 85.78 / 85.78, 29.75 | 326.0 | 88.18 / 87.89, 35.30 | 797.9 | 88.34 / 87.20, 35.41 |

#### Every 10th body: the same encoders, on enough bodies per class

103 Classic and 93 Enhanced bodies; the same dump as the mix rows. Idle priority, sizes and scores only.

**Classic**

| encoder | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB |
|---|---:|---:|---:|---:|---:|---:|
| BC1 (`dds` High) | 504.2 | 83.17 | 65.67 | 41.97 | 71 of 103 | 30.13 |
| repo | 873.1 | 67.91 | 51.48 | 23.59 | 20 of 103 | 26.07 |
| repo + colour | 945.5 | 75.65 | 58.56 | 29.34 | 37 of 103 | 33.00 |

By class: MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB.

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score |
|---|---:|---|---:|---|---:|---|---:|---|
| BC1 (`dds` High) | 106.5 | 83.50 / 62.88, 35 of 54, 27.67 | 15.5 | 84.61 / 41.97, 5 of 9, 28.56 | 111.0 | 81.77 / 75.52, 16 of 21, 30.74 | 262.6 | 81.49 / 80.64, 8 of 8, 31.13 |
| repo | 166.6 | 71.40 / 32.24, 13 of 54, 22.20 | 25.4 | 66.05 / 25.18, 0 of 9, 23.23 | 192.9 | 59.75 / 44.10, 0 of 21, 26.25 | 473.7 | 63.40 / 60.59, 0 of 8, 28.57 |
| repo + colour | 195.5 | 78.12 / 47.79, 25 of 54, 29.98 | 28.6 | 74.79 / 29.34, 2 of 9, 33.13 | 208.6 | 72.46 / 63.18, 2 of 21, 33.00 | 496.8 | 72.38 / 66.46, 0 of 8, 34.99 |

**Enhanced**

| encoder | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB |
|---|---:|---:|---:|---:|---:|---:|
| BC1 (`dds` High) | 911.2 | 74.90 | 51.11 | -46.34 | 17 of 93 | 30.85 |
| repo | 1890.4 | 60.36 | 45.10 | 37.36 | 10 of 93 | 26.49 |
| repo + colour | 1980.7 | 29.13 | 1.81 | -19.00 | 4 of 93 | 26.52 |
| fix A + colour | 1970.0 | 77.20 | 64.19 | 52.17 | 32 of 93 | 31.39 |
| **fix C** | 1873.0 | 85.74 | 63.02 | 51.11 | 73 of 93 | 32.37 |
| **fix C, RDO 0.5** | 1773.8 | 84.12 | 62.58 | 51.02 | 69 of 93 | 31.93 |
| **fix C, RDO 1** | 1697.9 | 81.23 | 61.35 | 49.55 | 59 of 93 | 31.12 |
| fix C + colour | 1963.9 | 87.22 | 68.22 | 54.25 | 76 of 93 | 33.71 |
| fix C + colour, RDO 1 | 1762.5 | 83.00 | 66.06 | 52.71 | 66 of 93 | 32.02 |

By class: MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB.

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score |
|---|---:|---|---:|---|---:|---|---:|---|
| BC1 (`dds` High) | 268.5 | 75.41 / 20.55, 15 of 57, 30.37 | 9.5 | 66.99 / 62.39, 1 of 4, 29.17 | 163.5 | 73.12 / -46.34, 0 of 19, 29.79 | 469.8 | 72.37 / 55.03, 1 of 13, 31.57 |
| repo | 561.9 | 65.13 / 37.36, 10 of 57, 24.89 | 19.9 | 54.79 / 53.97, 0 of 4, 23.47 | 348.6 | 58.50 / 48.42, 0 of 19, 26.46 | 960.0 | 57.47 / 41.43, 0 of 13, 27.80 |
| repo + colour | 606.3 | 37.88 / -19.00, 4 of 57, 24.59 | 21.6 | 13.07 / -3.18, 0 of 4, 24.03 | 364.1 | 25.75 / 1.81, 0 of 19, 26.39 | 988.7 | 21.14 / 0.74, 0 of 13, 28.18 |
| fix A + colour | 600.6 | 78.25 / 52.17, 27 of 57, 30.18 | 21.4 | 73.76 / 70.93, 1 of 4, 27.55 | 362.6 | 76.94 / 72.03, 3 of 19, 30.96 | 985.5 | 76.75 / 71.32, 1 of 13, 32.55 |
| **fix C** | 554.8 | 85.58 / 51.11, 40 of 57, 31.07 | 20.0 | 81.92 / 79.14, 3 of 4, 28.16 | 345.4 | 85.95 / 77.62, 18 of 19, 31.84 | 952.8 | 86.18 / 78.07, 12 of 13, 33.70 |
| **fix C, RDO 0.5** | 512.0 | 83.46 / 51.02, 39 of 57, 30.66 | 19.2 | 81.22 / 73.73, 3 of 4, 28.06 | 332.0 | 84.05 / 77.42, 15 of 19, 31.51 | 910.5 | 84.87 / 77.66, 12 of 13, 33.14 |
| **fix C, RDO 1** | 489.4 | 81.41 / 49.55, 34 of 57, 30.01 | 18.6 | 79.59 / 73.15, 2 of 4, 27.66 | 319.5 | 80.77 / 76.77, 13 of 19, 30.83 | 870.4 | 81.23 / 76.28, 10 of 13, 32.11 |
| fix C + colour | 598.4 | 86.81 / 54.25, 41 of 57, 32.72 | 21.4 | 84.51 / 83.06, 4 of 4, 30.38 | 361.0 | 87.42 / 78.66, 18 of 19, 33.25 | 983.2 | 87.40 / 83.11, 13 of 13, 34.66 |
| fix C + colour, RDO 1 | 519.8 | 83.41 / 52.71, 36 of 57, 31.14 | 19.7 | 83.54 / 81.80, 4 of 4, 29.44 | 332.1 | 83.00 / 77.76, 16 of 19, 31.88 | 890.9 | 82.36 / 78.88, 10 of 13, 32.70 |

### The smallest fix: a proposal

**Fixes A and C are approved by the user and landed after this benchmark, so every "repo" row in this document is the encoder from before them, and no row mixes encoders. The colour step isn't decided.** On a real `pack-art --bc7` of `tex_art_cc`, against the lossless package sprite by sprite, fix A took painted RGB from 27.36 to 28.02 dB, alpha from 30.20 to 31.08 dB and the worst sprite from 7.17 to 13.12 dB, at the same file size. Fix C then took that package to 33.04 dB, alpha to 57.22 dB and the worst sprite to 13.62 dB, and `tex_art_ec` from 38.46 to 45.56 dB, so the search was what held it back: [`bc7_alpha_plane_search.md`](../bc7_alpha_plane_search.md).

1. **Fix A,** the edit above, plus a regression test: the field test, with the repo's own `decode_bc7_error_bounded` or a dev-dependency decoder as the reference, over every `encode_modeN`, both index flags and all rotations. No disk cost. It's the only change needed to stop blocks decoding differently from what the search scored at the converter's flags.
2. **Fix C, landed on 2026-11-05 with a regression test,** the lines below. On top of fix A it costs about 2 % of animation disk on Classic and saves 1 % on Enhanced against the repo's row, and encode goes from 0.05 to 0.14 worker seconds a Mpx. It leaves pixels alone, so nothing can come to depend on it.

   ```diff
   --- a/lib/udd-image-codecs/src/bc7/analytical.rs (end of pack_bc7_rgba)
        *block = best_block;
   +
   +    // Alpha's own plane can win on blocks the correlation test above doesn't offer it to, and `pack_mode4_or_5` only
   +    // estimates its error, so the two candidates are compared by decoding them.
   +    if mn[3] != mx[3] {
   +        let mut plane_block = [0u8; 16];
   +        pack_mode4_or_5(&mut plane_block, pixels, ALPHA_PLANE);
   +        if decoded_error(&plane_block, pixels) < decoded_error(block, pixels) {
   +            *block = plane_block;
   +        }
   +    }
    }
   +
   +const ALPHA_PLANE: usize = 3;
   +
   +fn decoded_error(block: &[u8; 16], pixels: &[Pixel; 16]) -> u64 {
   +    let bits = u128::from_le_bytes(*block);
   +    super::rdo::decode_bc7_error_bounded::<false>(bits, pixels, 0, false, u64::MAX, None).expect("the encoder writes valid modes")
   +}
   --- a/lib/udd-image-codecs/src/bc7/rdo.rs (line 1420)
   -fn decode_bc7_error_bounded<const COLLECT_STATS: bool>(
   +pub(super) fn decode_bc7_error_bounded<const COLLECT_STATS: bool>(
   ```

3. **Colour under transparent pixels,** only if the tail is worth about 5.5 % of disk. After `normalize_to_rgba8888` in the two BC7 entry points of `lib/udd-image-codecs/src/bc7.rs` (lines 707 and 861; 969 and 1094 are the uncompressed RGBA8 path and stay as they are), so the encoder and RDO's target see the same pixels:

   ```diff
   -    let rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent);
   +    let mut rgba_pixels = normalize_to_rgba8888(pixels, input_format, extent).into_owned();
   +    mean_colour_under_transparent(&mut rgba_pixels, extent.width() as usize, extent.height() as usize);
   ```

   The helper is about 30 lines: per 4×4 block, each pixel with alpha 0 takes the alpha-weighted mean colour of the block. It's kept with the bench's scratch copy (`bc7-trace/src/colour_step.rs`), and on a random 40000-block strip it matches the form measured here (`bleed_strip` in `lossy.rs`) on every block. Check first that nothing reads colour under alpha 0 from a decoded page. It must not go in without fix A: on the repo's encoder it's what drops Enhanced to 28.77.
4. **Bugs B** (three small edits, in the scratch copy's `make_fixes.py`), only if the subset flags will ever be used. `benches/bc7_encode.rs` sets them today, so any benchmark comparison through those flags has been reading corrupt blocks.

**Packages need a rebuild to benefit:** `tex_art_cc`, `tex_land_cc`, `tex_art_ec`, `tex_land_ec`, `tex_aux_ec`, `mobile_anim_ec`, `mobile_anim_cc`, `effects_ec`, and `cc_radar` when built as BC7. `--jxl` textures don't pass through this encoder.

**`docs/TODO.md:85` closed BC7 RDO on land before this was known.** Those land rows ran on an encoder with bug A, and RDO partly heals corrupt blocks by replacing them, so they should be read again after the fix.
## 13. Lossy block formats on disk (`q`, `qc`)

**Measure only: the user decides.** A lossy mode would be chosen at pack time for low-end devices, and runtime BC7 stays off. Streams are one per (action, direction), so faced-only loading still works. Each stream's blocks are encoded as a strip in stream order, so RDO's lookback is what zstd sees and it stays inside the stream. Frames are padded to 4×4 blocks; section 11 has the padding's cost.

**Sample:** every 10th body by pixels, 103 Classic and 93 Enhanced bodies. Sizes and scores aren't timed. Every "BC7" row in these tables is **the repo's encoder as the converter calls it, with the bugs in section 12**. The same rows from the fixed scratch copy are in section 12.

**Encoders:**

- BC1 and BC3: `dds` 0.2.0, `CompressionQuality::High`, no dithering. `dds` does punch-through alpha from RGBA (`encode/bc1.rs`, threshold one half), so the repo's `encode_bc1` wasn't enough: it takes RGB only, at Normal. BC3's alpha is BC4 through the same crate.
- BC7 RDO: the repo's ERT port of `bc7enc_rdo` (`rdo.rs`, BC7 only), lookback 64 blocks.
- **BC1 "block reuse":** no RDO exists for BC1 in the repo. `lossy.rs` has a small pass written for this measurement. Over the previous 64 blocks it tries a whole earlier block, earlier selectors, or earlier endpoints with recomputed selectors, minimising mean squared error per channel + λ × bits (literal byte 8, matched byte 2). The alpha pattern must survive. These are real measurements of this pass, not the best a BC1 RDO could do.

### What the numbers say

**Does any row beat the lossless candidates on disk at SSIMULACRA2 80 or above?**

| | lossless to beat | rows that do, median at 80 or above | smallest |
|---|---|---|---|
| **Classic** | row spans 733.4 MB, zstd 19 761.1 MB (every body) | BC1 (504.2 MB, 83.17), its reuse pass at λ 0.5 and 1, BC3 (583.9 MB, 83.25), BC3 + colour (614.1 MB, 83.70) | **BC1, block reuse λ 1: 445.8 MB, 81.05.** λ 2 falls to 78.45. |
| **Enhanced** | section 10's assignment 1761.7 MB, zstd 19 2013.8 MB (every body) | BC3 (1445.4 MB, 82.23), BC3 + colour (1521.8 MB, 84.66) | **BC3: 1445.4 MB, 82.23** |

**On every body (section 16),** Enhanced's BC3 is 1463.1 MB at 81.72 and BC3 + colour 1539.0 MB at 84.52, and Classic's BC1 is 514.2 MB at 82.90. The answers above don't change. This table stays on the sample because the reuse pass ran only there.

- **No repo BC7 row beats lossless on disk at 80 or above.** The only one at 80 is Classic with colour and RDO 0.5 (889.5 MB, 81.11). It's larger than lossless, and λ 1 and 2 fall under 80.
- **Fix C changes that on Enhanced, and only there, and only just.** On every body, fix C with RDO 1 is 1728.9 MB at 81.03, against the assignment's 1761.7 MB, with 558 of 926 bodies at 80. Section 16 has the table. On Classic it's 823.2 MB at 80.20 against 733.4 MB, so no BC7 row beats lossless at 80.
- **BC1 on Enhanced isn't at 80.** 911.2 MB, median 74.90, but p5 51.11 and min −46.34, because one-bit alpha removes anti-aliased edges (below).
- **Medians hide the worst bodies.** Classic's BC1 has p5 65.67 and min 41.97, and only 71 of 103 bodies reach 80. Enhanced's BC3 + colour has 74 of 93.

**Why not BC3, as the original clients ship?** On these sprites there's no reason yet, and it's a real candidate:

- **At the same VRAM as BC7,** BC3 from `dds` is 23 to 35 % smaller on disk than the repo's BC7 and scores higher than it on both clients, because BC1's colour modes aren't hit by the bug and BC4's alpha plane is independent of colour.
- **Classic:** BC3 adds nothing over BC1. 583.9 MB against 504.2 MB, at 83.25 against 83.17, since Classic's alpha is one bit and BC1 holds it. It's also twice the VRAM.
- **Enhanced:** BC3 stays the smallest at 80 or above in this sample (1445.4 MB, 82.23; with colour 1521.8 MB, 84.66) and has by far the best coverage: 0.186 levels against 0.51 to 0.61 for BC7 with fix C. Fix C with RDO 1 costs about 280 MB more at about the same score, so BC3 has the disk and the edges, and BC7 the colour, as the next line says. On every body it stays the smallest at 80: section 16 has 1463.1 MB at 81.72, with colour 1539.0 MB at 84.52, and coverage error 0.211 levels against 0.628 for fix C.
- BC3's colour is still BC1's: 4 colours a block on one line. Its RGB PSNR on Enhanced is 29.51 to 30.87 dB, against 31.33 to 32.73 dB for BC7 with fix C and 32.86 to 33.97 dB for ISPC, and its hue error is 1.98 to 2.29 levels against 1.55 to 1.80 for BC7 with fix C.

**Enhanced's coverage as one-bit alpha** (BC1, threshold at one half): 87.89 % of edge pixels move by a level or more, 3.488 levels on average and at worst 8 of 16, and 11.571 % of painted pixels disappear. BC3's BC4 plane moves 0.81 % of edge pixels, 0.186 levels on average. The repo's BC7 moves 37.62 %, and its opaque interiors suffer too: 18.287 % of solid pixels stop being solid.

**Hue:**

- **Enhanced:** the hue table's key is a hueable pixel's red byte. Every block format changes it on 83 to 92 % of hueable pixels. After a real hue, the mean channel error is 2.0 to 3.3 levels, and 14 to 26 % of hued pixels are off by 8 or more. BC1 and BC3 + colour are the best here, at 2.0 levels, because BC7's corrupt blocks count against it.
- **Classic:** the key is the red byte's top 5 bits. BC1 changes it on 50.70 %, moved 0.707 of 31 on average, with a mean error of 3.102 levels under a full hue. **Partial hue is the weak point:** 81.45 % of grey pixels stop being exactly grey in 8 bits and 41.54 % in 5 bits, so a shader can't tell grey from colour after block coding. That's why Classic would need the masks of section 11. An R8 mask is twice BC1's 0.5 bytes a pixel, a BC4 mask equals it, and a one-bit mask is an eighth of a byte.
- **Hue flags through a mask:** BC4 and back with a threshold at one half gives 0 wrong flags, on 84876771 Classic and 16533130 Enhanced painted pixels of masked frames. A one-bit mask has none by construction. Section 11 has mask sizes and the share of frames that need none.

**BC1's reuse pass is cheap and honest.** On Classic it saves 5.9 %, 11.6 % and 20.2 % at λ 0.5, 1 and 2, losing 0.8, 2.1 and 4.7 points. Its encode time is 0.9 worker s/Mpx. BC7's RDO saves more on the repo encoder, partly by replacing corrupt blocks, and costs 3 to 8 worker s/Mpx. On the fixed encoder, in section 12, it's cleaner and costs about the same.

**Decode (`qc`):** every block row passes the 250 ms line by a wide margin, because a block row is only zstd in front of the upload. The tables below are a quiet rerun, with the other cluster 1.1 % busy on average and 2 % at its peak, using the placeholder rule's crowds, so they differ a little from section 9's. Read them against each table's own zstd 19 row. On Enhanced creatures, at 4 threads over five directions, plain planes take 100.4 ms, BC7 49.8 ms, BC3 50.4 ms and BC1 28.8 ms. An earlier run with background builds on the other cluster moved no five-direction row by more than 2.4 %, so it changes nothing here.

#### Classic: disk, pack time and quality

| row | block MB | zstd 7 MB | zstd 19 MB | encode, worker s a Mpx | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BC7 (repo) | 2510.9 | 891.2 | 873.1 | 0.048 | 67.91 | 51.48 | 23.59 | 20 of 103 | 6.593 | 255 | 26.07 | 0.033 % | 6.701 % |
| BC7 (repo), RDO lambda 0.5 | 2510.9 | 805.2 | 782.2 | 7.769 | 65.16 | 48.58 | 31.30 | 4 of 103 | 6.723 | 248 | 26.17 | 0.010 % | 6.980 % |
| BC7 (repo), RDO lambda 1 | 2510.9 | 760.3 | 737.7 | 6.543 | 62.87 | 47.70 | 30.04 | 3 of 103 | 7.204 | 248 | 25.87 | 0.010 % | 7.469 % |
| BC7 (repo), RDO lambda 2 | 2510.9 | 696.6 | 675.1 | 4.154 | 60.75 | 44.09 | 26.93 | 4 of 103 | 8.185 | 248 | 25.19 | 0.010 % | 7.881 % |
| BC7 (repo), colour under transparent pixels | 2510.9 | 966.7 | 945.5 | 0.088 | 75.65 | 58.56 | 29.34 | 37 of 103 | 3.199 | 255 | 33.00 | 0.041 % | 0.128 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 2510.9 | 915.7 | 889.5 | 8.488 | 81.11 | 65.00 | 41.72 | 59 of 103 | 3.494 | 247 | 33.04 | 0.003 % | 0.157 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 2510.9 | 872.5 | 846.9 | 2.782 | 78.13 | 64.10 | 41.27 | 45 of 103 | 4.030 | 247 | 31.96 | 0.003 % | 0.202 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 2510.9 | 808.2 | 784.1 | 2.521 | 73.82 | 58.70 | 40.33 | 13 of 103 | 5.110 | 247 | 29.89 | 0.003 % | 0.271 % |
| BC7 (repo), colour under transparent pixels, all modes | 2510.9 | 1011.7 | 994.4 | 3.032 | 68.08 | 43.79 | 18.34 | 28 of 103 | 5.317 | 255 | 25.22 | 0.039 % | 0.189 % |
| BC1 | 1255.5 | 520.3 | 504.2 | 0.400 | 83.17 | 65.67 | 41.97 | 71 of 103 | 5.045 | 189 | 30.13 | 0.000 % | 0.000 % |
| BC1, block reuse lambda 0.5 | 1255.5 | 493.0 | 474.3 | 0.922 | 82.37 | 65.65 | 42.22 | 70 of 103 | 5.200 | 189 | 30.03 | 0.000 % | 0.000 % |
| BC1, block reuse lambda 1 | 1255.5 | 465.8 | 445.8 | 0.915 | 81.05 | 65.55 | 42.09 | 58 of 103 | 5.505 | 189 | 29.80 | 0.000 % | 0.000 % |
| BC1, block reuse lambda 2 | 1255.5 | 423.5 | 402.3 | 0.924 | 78.45 | 64.47 | 41.86 | 45 of 103 | 6.126 | 189 | 29.25 | 0.000 % | 0.000 % |
| BC3 (BC1 colour + BC4 coverage) | 2510.9 | 629.0 | 583.9 | 0.358 | 83.25 | 64.44 | 39.87 | 73 of 103 | 5.057 | 189 | 30.23 | 0.000 % | 0.000 % |
| BC3, colour under transparent pixels | 2510.9 | 657.9 | 614.1 | 0.352 | 83.70 | 65.90 | 42.06 | 77 of 103 | 4.797 | 189 | 30.69 | 0.000 % | 0.000 % |

#### Classic: by class (zstd 19 MB, then SSIMULACRA2 median / min, RGB PSNR dB)

| row | equipment MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC small MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC medium MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC large MB | SSIMULACRA2 median / min, RGB PSNR dB | not listed MB | SSIMULACRA2 median / min, RGB PSNR dB |
|---|---:|---|---:|---|---:|---|---:|---|---:|---|
| BC7 (repo) | 166.6 | 71.40 / 32.24, 22.20 | 25.4 | 66.05 / 25.18, 23.23 | 192.9 | 59.75 / 44.10, 26.25 | 473.7 | 63.40 / 60.59, 28.57 | 14.6 | 82.98 / 23.59, 21.38 |
| BC7 (repo), RDO lambda 0.5 | 134.2 | 67.22 / 41.33, 22.42 | 21.7 | 63.10 / 31.30, 23.31 | 171.3 | 59.46 / 43.67, 26.24 | 442.2 | 63.43 / 58.91, 28.55 | 12.7 | 76.54 / 34.60, 22.09 |
| BC7 (repo), RDO lambda 1 | 126.3 | 65.10 / 38.19, 22.23 | 20.4 | 60.72 / 30.04, 23.10 | 159.8 | 56.07 / 42.03, 25.90 | 419.1 | 61.69 / 57.84, 28.14 | 12.1 | 73.35 / 31.89, 21.94 |
| BC7 (repo), RDO lambda 2 | 117.0 | 62.06 / 30.80, 21.84 | 18.8 | 59.72 / 28.59, 22.63 | 143.7 | 52.03 / 38.75, 25.14 | 384.5 | 57.63 / 53.03, 27.18 | 11.2 | 70.68 / 26.93, 21.58 |
| BC7 (repo), colour under transparent pixels | 195.5 | 78.12 / 47.79, 29.98 | 28.6 | 74.79 / 29.34, 33.13 | 208.6 | 72.46 / 63.18, 33.00 | 496.8 | 72.38 / 66.46, 34.99 | 16.0 | 88.04 / 32.59, 25.36 |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 179.4 | 81.11 / 61.85, 30.49 | 27.1 | 82.36 / 41.72, 32.58 | 195.0 | 79.53 / 73.95, 32.78 | 472.8 | 79.00 / 77.66, 34.64 | 15.2 | 84.97 / 49.39, 26.70 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 171.1 | 77.67 / 60.84, 29.71 | 26.0 | 80.00 / 41.27, 31.48 | 184.4 | 77.32 / 71.70, 31.64 | 450.7 | 76.67 / 74.52, 33.30 | 14.7 | 81.08 / 48.04, 26.44 |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 160.8 | 74.72 / 54.79, 28.26 | 24.3 | 75.04 / 40.33, 29.32 | 168.7 | 70.94 / 58.60, 29.51 | 416.4 | 70.35 / 67.05, 30.83 | 14.0 | 77.40 / 45.17, 25.74 |
| BC7 (repo), colour under transparent pixels, all modes | 199.7 | 77.03 / 18.34, 25.71 | 29.5 | 65.11 / 27.45, 24.50 | 219.1 | 56.97 / 41.93, 26.32 | 529.7 | 53.24 / 46.24, 24.91 | 16.4 | 88.03 / 20.57, 21.65 |
| BC1 | 106.5 | 83.50 / 62.88, 27.67 | 15.5 | 84.61 / 41.97, 28.56 | 111.0 | 81.77 / 75.52, 30.74 | 262.6 | 81.49 / 80.64, 31.13 | 8.7 | 85.74 / 54.48, 25.51 |
| BC1, block reuse lambda 0.5 | 100.6 | 83.05 / 62.85, 27.64 | 15.1 | 84.36 / 42.22, 28.53 | 104.3 | 81.32 / 75.11, 30.62 | 246.0 | 80.93 / 80.63, 30.99 | 8.4 | 85.60 / 53.34, 25.51 |
| BC1, block reuse lambda 1 | 96.5 | 82.48 / 62.72, 27.56 | 14.6 | 82.11 / 42.09, 28.45 | 98.0 | 79.71 / 73.71, 30.35 | 228.5 | 79.51 / 76.05, 30.67 | 8.2 | 85.36 / 52.88, 25.47 |
| BC1, block reuse lambda 2 | 90.1 | 81.37 / 62.46, 27.34 | 13.6 | 78.64 / 41.86, 28.18 | 88.6 | 75.26 / 64.42, 29.75 | 202.1 | 74.10 / 67.87, 29.93 | 7.8 | 85.16 / 52.02, 25.37 |
| BC3 (BC1 colour + BC4 coverage) | 132.1 | 83.25 / 56.94, 27.63 | 19.1 | 84.93 / 39.87, 29.12 | 128.7 | 82.80 / 75.01, 30.78 | 293.4 | 82.14 / 81.71, 31.38 | 10.5 | 86.82 / 54.62, 24.88 |
| BC3, colour under transparent pixels | 143.8 | 83.87 / 62.95, 28.51 | 20.4 | 85.71 / 42.06, 29.52 | 135.5 | 83.43 / 75.98, 31.30 | 303.3 | 82.00 / 81.43, 31.53 | 11.3 | 86.33 / 54.92, 25.90 |

#### Classic: hue

| row | key changed | moved, mean | worst | full hue channel error, mean | worst | hued pixels off by 8 or more | partial hue error, mean | worst | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BC7 (repo) | 50.56 % | 0.962 | 31 | 4.189 | 224 | 46.022 % | 8.838 | 255 | 68.89 % | 47.50 % | 0.427 % | 8.511 % |
| BC7 (repo), RDO lambda 0.5 | 52.72 % | 0.983 | 31 | 4.276 | 224 | 47.912 % | 9.256 | 255 | 74.68 % | 52.67 % | 0.644 % | 8.826 % |
| BC7 (repo), RDO lambda 1 | 54.99 % | 1.041 | 31 | 4.527 | 224 | 50.105 % | 9.724 | 255 | 74.93 % | 53.24 % | 0.795 % | 8.823 % |
| BC7 (repo), RDO lambda 2 | 58.51 % | 1.168 | 31 | 5.084 | 224 | 53.691 % | 10.662 | 255 | 75.47 % | 54.00 % | 0.940 % | 8.824 % |
| BC7 (repo), colour under transparent pixels | 41.52 % | 0.504 | 31 | 2.148 | 224 | 36.365 % | 5.875 | 255 | 63.34 % | 46.26 % | 0.906 % | 8.695 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 44.16 % | 0.531 | 30 | 2.262 | 224 | 38.743 % | 6.347 | 255 | 67.32 % | 49.25 % | 1.014 % | 8.816 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 47.14 % | 0.595 | 30 | 2.541 | 224 | 41.605 % | 6.902 | 255 | 68.67 % | 50.49 % | 1.115 % | 8.748 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 51.70 % | 0.735 | 30 | 3.158 | 224 | 46.223 % | 7.944 | 255 | 69.71 % | 51.66 % | 1.269 % | 8.704 % |
| BC7 (repo), colour under transparent pixels, all modes | 41.87 % | 0.786 | 31 | 3.418 | 224 | 37.644 % | 7.641 | 255 | 57.28 % | 41.92 % | 1.219 % | 7.046 % |
| BC1 | 50.70 % | 0.707 | 22 | 3.102 | 160 | 46.511 % | 8.677 | 255 | 81.45 % | 41.54 % | 1.627 % | 5.337 % |
| BC1, block reuse lambda 0.5 | 52.24 % | 0.725 | 22 | 3.173 | 160 | 47.731 % | 8.900 | 255 | 83.45 % | 43.39 % | 1.813 % | 5.662 % |
| BC1, block reuse lambda 1 | 55.25 % | 0.765 | 22 | 3.336 | 160 | 50.257 % | 9.244 | 255 | 84.36 % | 44.43 % | 2.158 % | 6.124 % |
| BC1, block reuse lambda 2 | 59.96 % | 0.847 | 22 | 3.679 | 160 | 54.430 % | 9.894 | 255 | 85.69 % | 46.69 % | 2.558 % | 6.658 % |
| BC3 (BC1 colour + BC4 coverage) | 51.60 % | 0.713 | 22 | 3.087 | 160 | 46.674 % | 8.817 | 255 | 80.49 % | 41.74 % | 2.909 % | 7.290 % |
| BC3, colour under transparent pixels | 50.21 % | 0.677 | 22 | 2.959 | 160 | 45.850 % | 8.497 | 255 | 81.73 % | 43.30 % | 1.690 % | 5.566 % |

#### Enhanced: disk, pack time and quality

| row | block MB | zstd 7 MB | zstd 19 MB | encode, worker s a Mpx | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| BC7 (repo) | 5112.1 | 1924.2 | 1890.4 | 0.056 | 60.36 | 45.10 | 37.36 | 10 of 93 | 6.262 | 255 | 26.49 | 0.081 % | 3.927 % |
| BC7 (repo), RDO lambda 0.5 | 5112.1 | 1746.5 | 1701.1 | 5.708 | 64.42 | 50.90 | 39.67 | 9 of 93 | 6.433 | 255 | 26.67 | 0.070 % | 3.834 % |
| BC7 (repo), RDO lambda 1 | 5112.1 | 1646.1 | 1601.2 | 7.886 | 63.31 | 50.78 | 38.77 | 6 of 93 | 6.887 | 255 | 26.38 | 0.093 % | 3.952 % |
| BC7 (repo), RDO lambda 2 | 5112.1 | 1506.9 | 1464.8 | 4.793 | 60.48 | 50.43 | 36.51 | 4 of 93 | 7.815 | 255 | 25.69 | 0.145 % | 4.089 % |
| BC7 (repo), colour under transparent pixels | 5112.1 | 2016.3 | 1980.7 | 0.109 | 29.13 | 1.81 | -19.00 | 4 of 93 | 5.608 | 255 | 26.52 | 2.822 % | 9.440 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 5112.1 | 1812.7 | 1763.8 | 8.369 | 53.11 | 40.48 | 33.20 | 3 of 93 | 5.819 | 255 | 27.04 | 1.015 % | 7.328 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 5112.1 | 1730.8 | 1682.1 | 3.773 | 52.64 | 40.38 | 33.11 | 2 of 93 | 6.243 | 255 | 26.78 | 1.033 % | 7.412 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 5112.1 | 1603.5 | 1557.9 | 3.523 | 51.85 | 39.60 | 32.52 | 2 of 93 | 7.130 | 255 | 26.14 | 1.074 % | 7.582 % |
| BC7 (repo), colour under transparent pixels, all modes | 5112.1 | 2098.8 | 2067.1 | 2.511 | 28.02 | 1.73 | -18.33 | 4 of 93 | 7.018 | 255 | 24.09 | 2.708 % | 9.105 % |
| BC1 | 2556.0 | 939.2 | 911.2 | 0.358 | 74.90 | 51.11 | -46.34 | 17 of 93 | 4.750 | 194 | 30.85 | 11.571 % | 0.000 % |
| BC1, block reuse lambda 0.5 | 2556.0 | 881.4 | 852.6 | 0.846 | 74.72 | 51.05 | -46.34 | 16 of 93 | 4.932 | 226 | 30.73 | 11.571 % | 0.000 % |
| BC1, block reuse lambda 1 | 2556.0 | 835.2 | 805.5 | 0.850 | 74.19 | 50.98 | -46.36 | 16 of 93 | 5.214 | 226 | 30.49 | 11.571 % | 0.000 % |
| BC1, block reuse lambda 2 | 2556.0 | 765.7 | 734.6 | 0.844 | 71.90 | 50.66 | -46.39 | 16 of 93 | 5.789 | 226 | 29.90 | 11.571 % | 0.000 % |
| BC3 (BC1 colour + BC4 coverage) | 5112.1 | 1518.9 | 1445.4 | 0.392 | 82.23 | 63.06 | 52.69 | 62 of 93 | 5.451 | 223 | 29.51 | 0.149 % | 0.254 % |
| BC3, colour under transparent pixels | 5112.1 | 1593.1 | 1521.8 | 0.371 | 84.66 | 67.37 | 54.65 | 74 of 93 | 4.693 | 223 | 30.87 | 0.149 % | 0.254 % |

#### Enhanced: by class (zstd 19 MB, then SSIMULACRA2 median / min, RGB PSNR dB)

| row | equipment MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC small MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC medium MB | SSIMULACRA2 median / min, RGB PSNR dB | NPC large MB | SSIMULACRA2 median / min, RGB PSNR dB |
|---|---:|---|---:|---|---:|---|---:|---|
| BC7 (repo) | 561.9 | 65.13 / 37.36, 24.89 | 19.9 | 54.79 / 53.97, 23.47 | 348.6 | 58.50 / 48.42, 26.46 | 960.0 | 57.47 / 41.43, 27.80 |
| BC7 (repo), RDO lambda 0.5 | 477.0 | 66.53 / 39.67, 25.03 | 17.8 | 61.19 / 59.64, 23.55 | 321.1 | 63.80 / 59.69, 26.69 | 885.3 | 63.30 / 55.86, 28.01 |
| BC7 (repo), RDO lambda 1 | 446.2 | 65.31 / 38.77, 24.79 | 16.7 | 60.08 / 57.88, 23.34 | 303.5 | 63.01 / 58.07, 26.40 | 834.9 | 62.31 / 55.41, 27.64 |
| BC7 (repo), RDO lambda 2 | 409.8 | 62.16 / 36.51, 24.31 | 15.1 | 57.73 / 52.89, 22.89 | 278.2 | 60.08 / 53.40, 25.71 | 761.6 | 58.69 / 51.62, 26.74 |
| BC7 (repo), colour under transparent pixels | 606.3 | 37.88 / -19.00, 24.59 | 21.6 | 13.07 / -3.18, 24.03 | 364.1 | 25.75 / 1.81, 26.39 | 988.7 | 21.14 / 0.74, 28.18 |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 513.5 | 52.43 / 33.20, 25.36 | 19.0 | 51.62 / 43.83, 24.55 | 329.6 | 54.67 / 44.08, 26.93 | 901.7 | 54.55 / 49.48, 28.43 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 490.8 | 52.08 / 33.11, 25.18 | 18.2 | 51.37 / 43.04, 24.39 | 315.4 | 54.05 / 42.57, 26.69 | 857.6 | 53.84 / 48.93, 28.08 |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 459.2 | 51.85 / 32.52, 24.78 | 16.9 | 50.35 / 42.29, 23.96 | 292.8 | 51.77 / 41.36, 26.05 | 788.9 | 51.99 / 46.87, 27.18 |
| BC7 (repo), colour under transparent pixels, all modes | 623.0 | 37.14 / -18.33, 23.54 | 22.1 | 13.60 / -0.68, 23.63 | 377.7 | 24.20 / 1.73, 24.23 | 1044.3 | 20.51 / 0.75, 24.37 |
| BC1 | 268.5 | 75.41 / 20.55, 30.37 | 9.5 | 66.99 / 62.39, 29.17 | 163.5 | 73.12 / -46.34, 29.79 | 469.8 | 72.37 / 55.03, 31.57 |
| BC1, block reuse lambda 0.5 | 245.4 | 75.30 / 20.55, 30.25 | 9.3 | 66.97 / 62.37, 29.15 | 158.3 | 73.08 / -46.34, 29.73 | 439.6 | 72.31 / 54.93, 31.42 |
| BC1, block reuse lambda 1 | 232.2 | 74.70 / 20.55, 30.04 | 9.1 | 66.89 / 62.27, 29.07 | 152.6 | 72.83 / -46.36, 29.59 | 411.7 | 72.21 / 54.64, 31.11 |
| BC1, block reuse lambda 2 | 214.4 | 73.91 / 20.54, 29.56 | 8.6 | 66.55 / 61.86, 28.79 | 142.5 | 71.42 / -46.39, 29.20 | 369.0 | 69.64 / 53.72, 30.35 |
| BC3 (BC1 colour + BC4 coverage) | 471.9 | 82.46 / 52.69, 28.22 | 18.1 | 79.69 / 73.93, 27.08 | 272.6 | 81.54 / 74.03, 28.96 | 682.8 | 82.43 / 77.06, 30.75 |
| BC3, colour under transparent pixels | 507.3 | 84.71 / 54.65, 30.11 | 19.4 | 83.06 / 82.26, 29.35 | 286.3 | 84.66 / 75.26, 30.43 | 708.8 | 84.56 / 78.82, 31.57 |

#### Enhanced: hue

| row | key changed | key moved, mean | worst | hued channel error, mean | worst | hued pixels off by 8 or more |
|---|---:|---:|---:|---:|---:|---:|
| BC7 (repo) | 84.81 % | 6.801 | 255 | 2.676 | 240 | 19.009 % |
| BC7 (repo), RDO lambda 0.5 | 85.77 % | 6.921 | 255 | 2.719 | 240 | 19.766 % |
| BC7 (repo), RDO lambda 1 | 86.73 % | 7.386 | 255 | 2.902 | 240 | 21.798 % |
| BC7 (repo), RDO lambda 2 | 87.88 % | 8.385 | 255 | 3.298 | 240 | 25.952 % |
| BC7 (repo), colour under transparent pixels | 83.96 % | 6.058 | 255 | 2.371 | 240 | 15.198 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 85.25 % | 6.348 | 255 | 2.484 | 240 | 16.887 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 86.25 % | 6.787 | 255 | 2.657 | 240 | 18.929 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 87.45 % | 7.749 | 255 | 3.039 | 240 | 23.166 % |
| BC7 (repo), colour under transparent pixels, all modes | 82.93 % | 7.653 | 255 | 3.016 | 240 | 18.302 % |
| BC1 | 88.51 % | 5.165 | 193 | 2.012 | 193 | 14.133 % |
| BC1, block reuse lambda 0.5 | 89.77 % | 5.343 | 226 | 2.076 | 223 | 14.567 % |
| BC1, block reuse lambda 1 | 90.83 % | 5.646 | 226 | 2.190 | 223 | 15.870 % |
| BC1, block reuse lambda 2 | 91.98 % | 6.280 | 226 | 2.436 | 223 | 19.257 % |
| BC3 (BC1 colour + BC4 coverage) | 87.78 % | 5.891 | 220 | 2.294 | 215 | 17.483 % |
| BC3, colour under transparent pixels | 86.91 % | 5.100 | 222 | 1.983 | 222 | 13.992 % |

#### Enhanced: coverage at edges

| row | edge pixels' error, mean levels | worst | edge pixels off by a level or more | solid pixels no longer solid |
|---|---:|---:|---:|---:|
| BC7 (repo) | 1.261 | 15.1 | 37.62 % | 18.287 % |
| BC7 (repo), RDO lambda 0.5 | 1.257 | 15.1 | 38.48 % | 18.946 % |
| BC7 (repo), RDO lambda 1 | 1.285 | 15.1 | 39.63 % | 19.471 % |
| BC7 (repo), RDO lambda 2 | 1.345 | 15.1 | 41.77 % | 20.353 % |
| BC7 (repo), colour under transparent pixels | 2.373 | 15.1 | 42.05 % | 17.583 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 0.5 | 1.734 | 15.1 | 40.89 % | 15.191 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 1 | 1.756 | 15.1 | 41.85 % | 15.517 % |
| BC7 (repo), colour under transparent pixels, RDO lambda 2 | 1.809 | 15.1 | 43.66 % | 16.047 % |
| BC7 (repo), colour under transparent pixels, all modes | 2.311 | 15.1 | 40.99 % | 17.339 % |
| BC1 | 3.488 | 8.0 | 87.89 % | 0.000 % |
| BC1, block reuse lambda 0.5 | 3.488 | 8.0 | 87.89 % | 0.000 % |
| BC1, block reuse lambda 1 | 3.488 | 8.0 | 87.89 % | 0.000 % |
| BC1, block reuse lambda 2 | 3.488 | 8.0 | 87.89 % | 0.000 % |
| BC3 (BC1 colour + BC4 coverage) | 0.186 | 1.6 | 0.81 % | 0.381 % |
| BC3, colour under transparent pixels | 0.186 | 1.6 | 0.81 % | 0.381 % |

#### Classic, creatures: 300 pairs of 209 bodies, 132.7 Mpx trimmed; a pair's Mpx: median 0.156, p95 1.19, max 10.09

| candidate | load unit | read from disk MB | all drawable, 1 thread ms | 4 threads ms | within 250 ms on 4 threads | worst job ms | job p95 ms | VRAM stored MB | RGBA8 MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | action+direction | 41.2 | 178.4 | 50.0 | yes | 9.3 | 1.5 | 132.7 | 530.7 |
| plain planes, zstd 19 | facing direction only | 8.2 | 35.5 | 10.3 | yes | 1.9 | 0.3 | 25.8 | 103.2 |
| BC7 (repo), zstd 19 | action+direction | 47.8 | 140.1 | 39.0 | yes | 6.7 | 1.2 | 135.6 | 530.7 |
| BC7 (repo), zstd 19 | facing direction only | 9.5 | 28.0 | 8.0 | yes | 1.4 | 0.3 | 26.4 | 103.2 |
| BC7 (repo), RDO lambda 1, zstd 19 | action+direction | 41.2 | 135.7 | 37.9 | yes | 6.9 | 1.1 | 135.6 | 530.7 |
| BC7 (repo), RDO lambda 1, zstd 19 | facing direction only | 8.2 | 26.8 | 7.8 | yes | 1.5 | 0.2 | 26.4 | 103.2 |
| BC7 (repo), colour under transparent pixels, zstd 19 | action+direction | 50.0 | 137.4 | 38.2 | yes | 6.6 | 1.2 | 135.6 | 530.7 |
| BC7 (repo), colour under transparent pixels, zstd 19 | facing direction only | 10.0 | 27.4 | 7.9 | yes | 1.4 | 0.3 | 26.4 | 103.2 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | action+direction | 44.9 | 135.7 | 38.0 | yes | 6.8 | 1.1 | 135.6 | 530.7 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | facing direction only | 8.9 | 27.0 | 7.9 | yes | 1.4 | 0.2 | 26.4 | 103.2 |
| BC1, zstd 19 | action+direction | 26.5 | 83.6 | 23.3 | yes | 3.9 | 0.7 | 67.8 | 530.7 |
| BC1, zstd 19 | facing direction only | 5.3 | 17.1 | 5.3 | yes | 0.8 | 0.1 | 13.2 | 103.2 |
| BC1, block reuse lambda 1, zstd 19 | action+direction | 24.2 | 88.9 | 24.8 | yes | 4.1 | 0.7 | 67.8 | 530.7 |
| BC1, block reuse lambda 1, zstd 19 | facing direction only | 4.8 | 17.6 | 5.1 | yes | 0.9 | 0.1 | 13.2 | 103.2 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | action+direction | 29.9 | 128.2 | 35.6 | yes | 5.5 | 1.1 | 135.6 | 530.7 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | facing direction only | 6.0 | 25.6 | 7.3 | yes | 1.2 | 0.2 | 26.4 | 103.2 |

#### Classic, dressed NPCs: 300 pairs of 258 bodies, 9.7 Mpx trimmed; a pair's Mpx: median 0.020, p95 0.11, max 0.14

| candidate | load unit | read from disk MB | all drawable, 1 thread ms | 4 threads ms | within 250 ms on 4 threads | worst job ms | job p95 ms | VRAM stored MB | RGBA8 MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | action+direction | 3.6 | 22.9 | 5.9 | yes | 0.3 | 0.2 | 9.7 | 39.0 |
| plain planes, zstd 19 | facing direction only | 0.7 | 4.8 | 1.3 | yes | 0.1 | 0.0 | 2.0 | 8.0 |
| BC7 (repo), zstd 19 | action+direction | 4.8 | 23.8 | 6.1 | yes | 0.2 | 0.2 | 10.8 | 39.0 |
| BC7 (repo), zstd 19 | facing direction only | 1.0 | 4.9 | 1.3 | yes | 0.0 | 0.0 | 2.2 | 8.0 |
| BC7 (repo), RDO lambda 1, zstd 19 | action+direction | 3.7 | 20.8 | 5.4 | yes | 0.2 | 0.2 | 10.8 | 39.0 |
| BC7 (repo), RDO lambda 1, zstd 19 | facing direction only | 0.8 | 4.4 | 1.2 | yes | 0.0 | 0.0 | 2.2 | 8.0 |
| BC7 (repo), colour under transparent pixels, zstd 19 | action+direction | 5.6 | 22.6 | 5.8 | yes | 0.2 | 0.2 | 10.8 | 39.0 |
| BC7 (repo), colour under transparent pixels, zstd 19 | facing direction only | 1.1 | 4.7 | 1.3 | yes | 0.1 | 0.0 | 2.2 | 8.0 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | action+direction | 4.9 | 21.1 | 5.5 | yes | 0.2 | 0.2 | 10.8 | 39.0 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | facing direction only | 1.0 | 4.4 | 1.2 | yes | 0.0 | 0.0 | 2.2 | 8.0 |
| BC1, zstd 19 | action+direction | 3.0 | 15.8 | 4.1 | yes | 0.2 | 0.1 | 5.4 | 39.0 |
| BC1, zstd 19 | facing direction only | 0.6 | 3.4 | 0.9 | yes | 0.0 | 0.0 | 1.1 | 8.0 |
| BC1, block reuse lambda 1, zstd 19 | action+direction | 2.7 | 16.1 | 4.2 | yes | 0.2 | 0.1 | 5.4 | 39.0 |
| BC1, block reuse lambda 1, zstd 19 | facing direction only | 0.6 | 3.3 | 0.9 | yes | 0.0 | 0.0 | 1.1 | 8.0 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | action+direction | 3.6 | 25.5 | 6.5 | yes | 0.2 | 0.2 | 10.8 | 39.0 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | facing direction only | 0.7 | 5.3 | 1.4 | yes | 0.1 | 0.0 | 2.2 | 8.0 |

#### Enhanced, creatures: 300 pairs of 198 bodies, 198.3 Mpx trimmed; a pair's Mpx: median 0.366, p95 1.99, max 8.19

| candidate | load unit | read from disk MB | all drawable, 1 thread ms | 4 threads ms | within 250 ms on 4 threads | worst job ms | job p95 ms | VRAM stored MB | RGBA8 MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | action+direction | 78.5 | 395.1 | 100.4 | yes | 17.9 | 3.6 | 396.6 | 793.1 |
| plain planes, zstd 19 | facing direction only | 15.7 | 80.3 | 20.5 | yes | 3.1 | 0.8 | 79.9 | 159.7 |
| BC7 (repo), zstd 19 | action+direction | 73.0 | 196.8 | 49.8 | yes | 8.8 | 1.7 | 202.8 | 793.1 |
| BC7 (repo), zstd 19 | facing direction only | 14.6 | 39.5 | 10.1 | yes | 1.6 | 0.4 | 40.8 | 159.7 |
| BC7 (repo), RDO lambda 1, zstd 19 | action+direction | 63.8 | 195.9 | 49.6 | yes | 9.0 | 1.7 | 202.8 | 793.1 |
| BC7 (repo), RDO lambda 1, zstd 19 | facing direction only | 12.8 | 39.1 | 10.0 | yes | 1.6 | 0.4 | 40.8 | 159.7 |
| BC7 (repo), colour under transparent pixels, zstd 19 | action+direction | 75.6 | 196.9 | 50.0 | yes | 8.8 | 1.7 | 202.8 | 793.1 |
| BC7 (repo), colour under transparent pixels, zstd 19 | facing direction only | 15.1 | 39.6 | 10.1 | yes | 1.6 | 0.4 | 40.8 | 159.7 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | action+direction | 65.8 | 193.3 | 49.0 | yes | 8.9 | 1.7 | 202.8 | 793.1 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | facing direction only | 13.2 | 38.5 | 9.9 | yes | 1.6 | 0.4 | 40.8 | 159.7 |
| BC1, zstd 19 | action+direction | 35.5 | 113.6 | 28.8 | yes | 4.9 | 1.0 | 101.4 | 793.1 |
| BC1, zstd 19 | facing direction only | 7.1 | 22.7 | 5.8 | yes | 0.9 | 0.2 | 20.4 | 159.7 |
| BC1, block reuse lambda 1, zstd 19 | action+direction | 33.0 | 119.8 | 30.5 | yes | 5.5 | 1.0 | 101.4 | 793.1 |
| BC1, block reuse lambda 1, zstd 19 | facing direction only | 6.6 | 23.8 | 6.2 | yes | 1.0 | 0.2 | 20.4 | 159.7 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | action+direction | 53.8 | 199.1 | 50.4 | yes | 6.6 | 1.8 | 202.8 | 793.1 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | facing direction only | 10.8 | 39.9 | 10.2 | yes | 1.3 | 0.4 | 40.8 | 159.7 |

#### Enhanced, dressed NPCs: 300 pairs of 257 bodies, 15.8 Mpx trimmed; a pair's Mpx: median 0.033, p95 0.17, max 0.25

| candidate | load unit | read from disk MB | all drawable, 1 thread ms | 4 threads ms | within 250 ms on 4 threads | worst job ms | job p95 ms | VRAM stored MB | RGBA8 MB |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | action+direction | 9.9 | 57.0 | 14.6 | yes | 0.7 | 0.5 | 31.6 | 63.2 |
| plain planes, zstd 19 | facing direction only | 2.0 | 11.7 | 3.1 | yes | 0.2 | 0.1 | 6.4 | 12.7 |
| BC7 (repo), zstd 19 | action+direction | 8.7 | 31.8 | 8.1 | yes | 0.4 | 0.3 | 17.2 | 63.2 |
| BC7 (repo), zstd 19 | facing direction only | 1.8 | 6.6 | 1.8 | yes | 0.1 | 0.1 | 3.5 | 12.7 |
| BC7 (repo), RDO lambda 1, zstd 19 | action+direction | 7.1 | 30.0 | 7.7 | yes | 0.3 | 0.2 | 17.2 | 63.2 |
| BC7 (repo), RDO lambda 1, zstd 19 | facing direction only | 1.4 | 6.2 | 1.7 | yes | 0.1 | 0.1 | 3.5 | 12.7 |
| BC7 (repo), colour under transparent pixels, zstd 19 | action+direction | 9.4 | 31.5 | 8.1 | yes | 0.4 | 0.3 | 17.2 | 63.2 |
| BC7 (repo), colour under transparent pixels, zstd 19 | facing direction only | 1.9 | 6.6 | 1.7 | yes | 0.1 | 0.1 | 3.5 | 12.7 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | action+direction | 7.7 | 28.7 | 7.4 | yes | 0.3 | 0.2 | 17.2 | 63.2 |
| BC7 (repo), colour under transparent pixels, RDO lambda 1, zstd 19 | facing direction only | 1.5 | 5.9 | 1.7 | yes | 0.1 | 0.1 | 3.5 | 12.7 |
| BC1, zstd 19 | action+direction | 4.3 | 20.6 | 5.3 | yes | 0.2 | 0.2 | 8.6 | 63.2 |
| BC1, zstd 19 | facing direction only | 0.9 | 4.3 | 1.2 | yes | 0.0 | 0.0 | 1.7 | 12.7 |
| BC1, block reuse lambda 1, zstd 19 | action+direction | 3.8 | 20.8 | 5.4 | yes | 0.2 | 0.2 | 8.6 | 63.2 |
| BC1, block reuse lambda 1, zstd 19 | facing direction only | 0.8 | 4.4 | 1.2 | yes | 0.1 | 0.0 | 1.7 | 12.7 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | action+direction | 7.5 | 34.3 | 8.8 | yes | 0.4 | 0.3 | 17.2 | 63.2 |
| BC3 (BC1 colour + BC4 coverage), zstd 19 | facing direction only | 1.5 | 7.1 | 1.9 | yes | 0.1 | 0.1 | 3.5 | 12.7 |

## 14. Phones, Apple, Basis Universal and the low-end mix (`qm`, tools outside the workspace)

**Measure only: the user decides.** Third-party encoders ran from a scratch project under `uodynamapper-scratch/lossy-encoders`. Nothing from them is in the repo. Streams are one per (action, direction) throughout, and each stream's blocks are encoded as a strip, so any match search stays inside the stream.

**Encoders and settings:**

- **ETC2 RGB8A1:** etc2comp (Apache-2.0), effort 40. It holds image height in 16 bits, so a strip taller than 65535 pixels wraps. The tool encodes in pieces of 16000 blocks.
- **ETC2 "reuse":** no RDO encoder for ETC2 RGB8A1 was found. The tool has a small selector-reuse pass like the BC1 one. **These rows are a bound on what such a pass gives,** not the result of a shipping encoder.
- **ASTC:** ARM astc-encoder through `astcenc-rs` 0.2.0 (bindings Unlicense, encoder Apache-2.0), LDR RGBA profile, preset medium, default alpha weighting. A `thorough` row is in the Basis tables below.
- **All rows use linear, equal-weight error,** as the BC rows do, not perceptual weights.

### ASTC 6×6 against BC1

**Arm's published figure doesn't hold on these sprites.** Arm reports 3.56 bpp beating BC1's 4 bpp by about 1.5 dB on general images. Here ASTC 6×6 is **below** BC1 in RGB PSNR in every class on both clients: by 0.18 to 1.90 dB on Classic and by 2 to 4 dB on Enhanced. The table below has each class.

**What ASTC 6×6 does keep is Enhanced's edges, which is why the user asked about it:**

| Enhanced, every 10th | zstd 19 MB | median / p5 / min | at 80 or better | painted pixels gone | edge error, levels of 16 |
|---|---:|---|---:|---:|---:|
| BC1 | 911.2 | 74.90 / 51.11 / −46.34 | 17 of 93 | 11.571 % | 3.488 |
| ASTC 6×6 | 1065.0 | 73.08 / 56.49 / 39.20 | 16 of 93 | 0.406 % | 0.970 |
| ASTC 8×8 | 649.7 | 57.59 / 41.09 / 28.84 | 9 of 93 | 0.531 % | 1.489 |
| ASTC 4×4 | 2128.9 | 87.62 / 70.59 / 55.89 | 78 of 93 | 0.222 % | 0.323 |

- **Enhanced:** ASTC 6×6 trades colour for edges. Its median is about the same as BC1's, its worst bodies are much better, and it's 17 % larger on disk. Neither reaches 80.
- **Classic:** BC1 wins outright. It has 83.17 and 71 of 103 bodies at 80, against 75.28 and 33 of 103 for ASTC 6×6, at about the same disk (504.2 and 525.6 MB). Classic's alpha is one bit, so ASTC's full alpha buys nothing there.
- **Only ASTC 4×4 has a median at 80 or above on both clients,** and on disk it's larger than lossless on both (1022.2 MB and 2128.9 MB).

**Padding.** ASTC blocks round tiny sprites up, so 6×6's nominal 11 % VRAM saving over BC1 shrinks: 0.514 bytes a pixel against 0.547 on Enhanced's equipment, and 0.457 against 0.508 on large NPCs. At 8×8 the bytes are 0.307 to 0.260, about half of BC1's.

**RDO on the phone formats:**

- **astcenc has none.** Nothing in its headers offers rate-distortion optimisation.
- **Basis Universal's ASTC path has a windowed RDO** that reuses the config and endpoints of the left, upper and diagonal blocks, so it stays inside a frame. It has a gate: blocks under a minimum PSNR (default 35 dB, alpha 38 dB) aren't replaced. At 6×6 and 8×8 most of these blocks are under the gate, so the default barely changes the size. The Basis tables below include rows with the gate lowered.
- **ETC2:** only the bound above. λ 1 saves about 7 % on both clients for less than one SSIMULACRA2 point. λ 4 saves about 21 % and costs 3.6 points on Classic and 4.9 on Enhanced.

**One thing not explained:** etc2comp loses 0.101 % of painted pixels on Classic (worst channel error 255), where the format itself can hold them all. This wasn't traced. It may be how the encoder picks its transparent selector.

Encode cost in worker seconds per Mpx, all at pack time: ETC2 2.0 to 2.2, ETC2 reuse 3.1 to 3.4, ASTC 4×4 3.5 to 3.8, 6×6 3.8 to 4.6, 8×8 4.7 to 6.2.

#### Classic: every 10th body, disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 4 x 4 | 2510.9 | 1036.1 | 1022.2 | 90.41 | 77.63 | 70.36 | 93 of 103 | 1.985 | 124 | 37.91 | 0.000 % | 0.000 % |
| ASTC 6 x 6 | 1146.1 | 532.9 | 525.6 | 75.28 | 58.12 | 43.93 | 33 of 103 | 5.518 | 212 | 29.22 | 0.000 % | 0.086 % |
| ASTC 8 x 8 | 660.3 | 319.1 | 313.5 | 59.86 | 35.63 | 1.50 | 10 of 103 | 9.181 | 231 | 24.89 | 0.000 % | 0.651 % |
| BC1 (`dds`, High) | 1255.5 | 520.3 | 504.2 | 83.17 | 65.67 | 41.97 | 71 of 103 | 5.045 | 189 | 30.13 | 0.000 % | 0.000 % |
| ETC2 RGB8A1 | 1255.5 | 534.8 | 517.3 | 74.05 | 55.98 | 39.80 | 15 of 103 | 5.418 | 255 | 29.56 | 0.101 % | 0.014 % |
| ETC2 RGB8A1, reuse 1 (a bound) | 1255.5 | 498.1 | 482.1 | 73.37 | 55.84 | 40.03 | 14 of 103 | 5.733 | 255 | 29.36 | 0.101 % | 0.014 % |
| ETC2 RGB8A1, reuse 4 (a bound) | 1255.5 | 419.0 | 405.3 | 70.49 | 53.49 | 39.46 | 8 of 103 | 7.216 | 255 | 28.01 | 0.101 % | 0.014 % |

#### Classic: by class: zstd 19 MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB

| row | equipment MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC small MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC medium MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC large MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | not listed MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB |
|---|---:|---|---:|---|---:|---|---:|---|---:|---|
| ASTC 4 x 4 | 202.1 | 88.31 / 72.80, 45 of 54, 36.36 | 30.5 | 91.26 / 70.36, 8 of 9, 36.15 | 228.5 | 91.27 / 87.39, 21 of 21, 37.92 | 544.8 | 90.60 / 89.11, 8 of 8, 38.72 | 16.4 | 91.80 / 80.54, 11 of 11, 33.53 |
| ASTC 6 x 6 | 114.2 | 74.94 / 49.56, 18 of 54, 25.95 | 16.0 | 77.18 / 43.93, 2 of 9, 26.66 | 117.0 | 74.87 / 61.87, 4 of 21, 29.66 | 269.5 | 74.46 / 67.93, 2 of 8, 30.95 | 9.0 | 85.12 / 47.36, 7 of 11, 24.78 |
| ASTC 8 x 8 | 71.6 | 60.31 / 17.24, 7 of 54, 21.38 | 9.8 | 56.16 / 23.42, 0 of 9, 22.46 | 69.4 | 55.26 / 26.94, 0 of 21, 25.53 | 157.2 | 57.88 / 42.82, 0 of 8, 26.67 | 5.5 | 76.46 / 1.50, 3 of 11, 20.79 |
| BC1 (`dds`, High) | 106.5 | 83.50 / 62.88, 35 of 54, 27.67 | 15.5 | 84.61 / 41.97, 5 of 9, 28.56 | 111.0 | 81.77 / 75.52, 16 of 21, 30.74 | 262.6 | 81.49 / 80.64, 8 of 8, 31.13 | 8.7 | 85.74 / 54.48, 7 of 11, 25.51 |
| ETC2 RGB8A1 | 108.9 | 74.14 / 51.76, 10 of 54, 27.33 | 15.9 | 72.82 / 39.80, 0 of 9, 28.08 | 113.4 | 71.55 / 60.90, 3 of 21, 29.62 | 270.3 | 77.17 / 69.89, 1 of 8, 30.59 | 8.7 | 73.85 / 53.43, 1 of 11, 26.01 |
| ETC2 RGB8A1, reuse 1 (a bound) | 104.9 | 74.25 / 51.77, 10 of 54, 27.31 | 15.6 | 72.83 / 40.03, 1 of 9, 28.06 | 105.6 | 70.24 / 59.85, 2 of 21, 29.42 | 247.5 | 75.69 / 69.25, 0 of 8, 30.28 | 8.6 | 73.84 / 53.51, 1 of 11, 26.01 |
| ETC2 RGB8A1, reuse 4 (a bound) | 96.5 | 73.56 / 49.93, 7 of 54, 26.80 | 14.6 | 68.37 / 39.46, 0 of 9, 27.54 | 89.0 | 61.65 / 52.95, 0 of 21, 28.05 | 197.0 | 61.21 / 53.79, 0 of 8, 28.45 | 8.2 | 73.64 / 52.40, 1 of 11, 25.79 |

#### Classic: hue

| row | key changed | moved, mean | worst | full hue channel error, mean | worst | hued pixels off by 8 or more | partial hue error, mean | worst | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 4 x 4 | 35.48 % | 0.374 | 14 | 1.596 | 104 | 31.160 % | 4.189 | 255 | 40.69 % | 31.09 % | 2.452 % | 9.077 % |
| ASTC 6 x 6 | 56.02 % | 0.812 | 27 | 3.478 | 200 | 49.832 % | 8.845 | 255 | 42.37 % | 30.03 % | 9.441 % | 16.337 % |
| ASTC 8 x 8 | 66.61 % | 1.295 | 29 | 5.587 | 216 | 60.442 % | 13.069 | 255 | 36.54 % | 24.69 % | 17.084 % | 22.744 % |
| BC1 (`dds`, High) | 50.70 % | 0.707 | 22 | 3.102 | 160 | 46.511 % | 8.677 | 255 | 81.45 % | 41.54 % | 1.627 % | 5.337 % |
| ETC2 RGB8A1 | 54.98 % | 0.794 | 31 | 3.346 | 224 | 48.210 % | 8.203 | 255 | 31.55 % | 30.01 % | 8.720 % | 11.860 % |
| ETC2 RGB8A1, reuse 1 (a bound) | 58.05 % | 0.836 | 31 | 3.514 | 224 | 50.748 % | 8.622 | 255 | 33.38 % | 30.83 % | 9.365 % | 12.252 % |
| ETC2 RGB8A1, reuse 4 (a bound) | 65.84 % | 1.030 | 31 | 4.349 | 224 | 58.364 % | 10.184 | 255 | 37.75 % | 34.59 % | 10.277 % | 12.997 % |

#### Enhanced: every 10th body, disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 4 x 4 | 5112.1 | 2155.8 | 2128.9 | 87.62 | 70.59 | 55.89 | 78 of 93 | 2.638 | 186 | 35.14 | 0.222 % | 0.241 % |
| ASTC 6 x 6 | 2331.2 | 1080.9 | 1065.0 | 73.08 | 56.49 | 39.20 | 16 of 93 | 6.088 | 240 | 28.22 | 0.406 % | 0.758 % |
| ASTC 8 x 8 | 1346.2 | 659.5 | 649.7 | 57.59 | 41.09 | 28.84 | 9 of 93 | 9.271 | 255 | 24.69 | 0.531 % | 2.162 % |
| BC1 (`dds`, High) | 2556.0 | 939.2 | 911.2 | 74.90 | 51.11 | -46.34 | 17 of 93 | 4.750 | 194 | 30.85 | 11.571 % | 0.000 % |
| ETC2 RGB8A1 | 2556.0 | 958.3 | 926.9 | 71.63 | 49.79 | -46.41 | 10 of 93 | 4.979 | 255 | 30.45 | 11.582 % | 0.005 % |
| ETC2 RGB8A1, reuse 1 (a bound) | 2556.0 | 891.7 | 864.7 | 71.25 | 49.98 | -46.41 | 11 of 93 | 5.297 | 255 | 30.21 | 11.582 % | 0.005 % |
| ETC2 RGB8A1, reuse 4 (a bound) | 2556.0 | 759.6 | 737.4 | 66.69 | 48.90 | -46.50 | 10 of 93 | 6.738 | 255 | 28.70 | 11.582 % | 0.005 % |

#### Enhanced: by class: zstd 19 MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB

| row | equipment MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC small MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC medium MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB | NPC large MB | SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB |
|---|---:|---|---:|---|---:|---|---:|---|
| ASTC 4 x 4 | 651.5 | 86.67 / 55.89, 42 of 57, 34.00 | 22.3 | 83.52 / 83.22, 4 of 4, 30.94 | 383.2 | 87.66 / 84.33, 19 of 19, 34.46 | 1071.9 | 88.36 / 85.01, 13 of 13, 36.41 |
| ASTC 6 x 6 | 338.6 | 73.92 / 39.20, 14 of 57, 26.71 | 11.7 | 64.63 / 60.92, 0 of 4, 25.25 | 192.1 | 72.31 / 65.76, 1 of 19, 27.86 | 522.6 | 75.05 / 63.85, 1 of 13, 29.61 |
| ASTC 8 x 8 | 213.4 | 61.26 / 28.84, 9 of 57, 23.12 | 7.4 | 45.14 / 42.49, 0 of 4, 22.32 | 117.0 | 54.98 / 45.74, 0 of 19, 24.41 | 311.9 | 60.47 / 44.06, 0 of 13, 26.06 |
| BC1 (`dds`, High) | 268.5 | 75.41 / 20.55, 15 of 57, 30.37 | 9.5 | 66.99 / 62.39, 1 of 4, 29.17 | 163.5 | 73.12 / -46.34, 0 of 19, 29.79 | 469.8 | 72.37 / 55.03, 1 of 13, 31.57 |
| ETC2 RGB8A1 | 271.9 | 72.37 / 20.64, 10 of 57, 30.20 | 9.6 | 65.17 / 61.42, 0 of 4, 28.50 | 166.8 | 70.20 / -46.41, 0 of 19, 29.54 | 478.6 | 71.30 / 54.11, 0 of 13, 30.96 |
| ETC2 RGB8A1, reuse 1 (a bound) | 249.2 | 71.93 / 20.59, 11 of 57, 29.96 | 9.4 | 65.13 / 61.37, 0 of 4, 28.47 | 161.6 | 69.79 / -46.41, 0 of 19, 29.43 | 444.4 | 70.38 / 53.89, 0 of 13, 30.66 |
| ETC2 RGB8A1, reuse 4 (a bound) | 222.2 | 69.99 / 20.53, 10 of 57, 28.79 | 8.8 | 64.62 / 59.17, 0 of 4, 27.85 | 145.1 | 64.51 / -46.50, 0 of 19, 28.40 | 361.2 | 62.56 / 48.90, 0 of 13, 28.77 |

#### Enhanced: hue

| row | key changed | key moved, mean | worst | hued channel error, mean | worst | hued pixels off by 8 or more |
|---|---:|---:|---:|---:|---:|---:|
| ASTC 4 x 4 | 75.30 % | 2.760 | 161 | 1.072 | 158 | 5.101 % |
| ASTC 6 x 6 | 87.66 % | 6.512 | 210 | 2.548 | 206 | 19.856 % |
| ASTC 8 x 8 | 90.88 % | 9.993 | 253 | 3.927 | 240 | 31.684 % |
| BC1 (`dds`, High) | 88.51 % | 5.165 | 193 | 2.012 | 193 | 14.133 % |
| ETC2 RGB8A1 | 89.36 % | 5.402 | 255 | 2.110 | 240 | 15.056 % |
| ETC2 RGB8A1, reuse 1 (a bound) | 90.85 % | 5.745 | 255 | 2.233 | 240 | 16.362 % |
| ETC2 RGB8A1, reuse 4 (a bound) | 93.07 % | 7.333 | 255 | 2.851 | 240 | 24.667 % |

#### Enhanced: coverage at edges

| row | edge pixels' error, mean levels | worst | edge pixels off by a level or more | solid pixels no longer solid |
|---|---:|---:|---:|---:|
| ASTC 4 x 4 | 0.323 | 10.8 | 9.01 % | 2.295 % |
| ASTC 6 x 6 | 0.970 | 15.1 | 36.56 % | 6.079 % |
| ASTC 8 x 8 | 1.489 | 15.1 | 50.67 % | 9.676 % |
| BC1 (`dds`, High) | 3.488 | 8.0 | 87.89 % | 0.000 % |
| ETC2 RGB8A1 | 3.491 | 15.1 | 87.89 % | 0.016 % |
| ETC2 RGB8A1, reuse 1 (a bound) | 3.491 | 15.1 | 87.89 % | 0.016 % |
| ETC2 RGB8A1, reuse 4 (a bound) | 3.491 | 15.1 | 87.89 % | 0.016 % |

#### Arm's claim on these sprites: ASTC 6 x 6 against BC1, per class

| client | class | BC1 RGB PSNR dB | ASTC 6 x 6 RGB PSNR dB | difference dB | BC1 SSIMULACRA2 median | ASTC 6 x 6 median | difference | BC1 bodies at 80 | ASTC 6 x 6 bodies at 80 | ASTC 8 x 8 median |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Classic | equipment | 27.67 | 25.95 | -1.72 | 83.50 | 74.94 | -8.56 | 35 of 54 | 18 of 54 | 60.31 |
| Classic | NPC small | 28.56 | 26.66 | -1.90 | 84.61 | 77.18 | -7.43 | 5 of 9 | 2 of 9 | 56.16 |
| Classic | NPC medium | 30.74 | 29.66 | -1.08 | 81.77 | 74.87 | -6.90 | 16 of 21 | 4 of 21 | 55.26 |
| Classic | NPC large | 31.13 | 30.95 | -0.18 | 81.49 | 74.46 | -7.03 | 8 of 8 | 2 of 8 | 57.88 |
| Enhanced | equipment | 30.37 | 26.71 | -3.66 | 75.41 | 73.92 | -1.49 | 15 of 57 | 14 of 57 | 61.26 |
| Enhanced | NPC small | 29.17 | 25.25 | -3.92 | 66.99 | 64.63 | -2.36 | 1 of 4 | 0 of 4 | 45.14 |
| Enhanced | NPC medium | 29.79 | 27.86 | -1.93 | 73.12 | 72.31 | -0.81 | 0 of 19 | 1 of 19 | 54.98 |
| Enhanced | NPC large | 31.57 | 29.61 | -1.96 | 72.37 | 75.05 | +2.68 | 1 of 13 | 1 of 13 | 60.47 |

#### Padding: block pixels over true pixels, and what it does to the bytes a true pixel takes

| client | row | class | x pixels | raw bytes a pixel | stored bits a true pixel | stored bits a painted pixel |
|---|---|---|---:|---:|---:|---:|
| Classic | ETC2 RGB8A1 | equipment | 1.112 | 0.556 | 2.095 | 6.143 |
| Classic | ETC2 RGB8A1 | NPC small | 1.079 | 0.539 | 2.371 | 5.493 |
| Classic | ETC2 RGB8A1 | NPC medium | 1.038 | 0.519 | 1.891 | 4.329 |
| Classic | ETC2 RGB8A1 | NPC large | 1.014 | 0.507 | 1.496 | 3.907 |
| Classic | ASTC 4 x 4 | equipment | 1.112 | 1.112 | 3.889 | 11.400 |
| Classic | ASTC 4 x 4 | NPC small | 1.079 | 1.079 | 4.537 | 10.513 |
| Classic | ASTC 4 x 4 | NPC medium | 1.038 | 1.038 | 3.809 | 8.718 |
| Classic | ASTC 4 x 4 | NPC large | 1.014 | 1.014 | 3.015 | 7.874 |
| Classic | ASTC 6 x 6 | equipment | 1.197 | 0.532 | 2.198 | 6.445 |
| Classic | ASTC 6 x 6 | NPC small | 1.131 | 0.503 | 2.380 | 5.514 |
| Classic | ASTC 6 x 6 | NPC medium | 1.065 | 0.473 | 1.950 | 4.464 |
| Classic | ASTC 6 x 6 | NPC large | 1.024 | 0.455 | 1.491 | 3.895 |
| Classic | ASTC 8 x 8 | equipment | 1.270 | 0.318 | 1.378 | 4.039 |
| Classic | ASTC 8 x 8 | NPC small | 1.188 | 0.297 | 1.464 | 3.392 |
| Classic | ASTC 8 x 8 | NPC medium | 1.091 | 0.273 | 1.157 | 2.650 |
| Classic | ASTC 8 x 8 | NPC large | 1.034 | 0.258 | 0.870 | 2.271 |
| Enhanced | ETC2 RGB8A1 | equipment | 1.094 | 0.547 | 1.823 | 4.155 |
| Enhanced | ETC2 RGB8A1 | NPC small | 1.072 | 0.536 | 1.777 | 4.788 |
| Enhanced | ETC2 RGB8A1 | NPC medium | 1.038 | 0.519 | 1.637 | 3.667 |
| Enhanced | ETC2 RGB8A1 | NPC large | 1.017 | 0.508 | 1.336 | 3.950 |
| Enhanced | ASTC 4 x 4 | equipment | 1.094 | 1.094 | 4.369 | 9.957 |
| Enhanced | ASTC 4 x 4 | NPC small | 1.072 | 1.072 | 4.139 | 11.155 |
| Enhanced | ASTC 4 x 4 | NPC medium | 1.038 | 1.038 | 3.760 | 8.425 |
| Enhanced | ASTC 4 x 4 | NPC large | 1.017 | 1.017 | 2.992 | 8.846 |
| Enhanced | ASTC 6 x 6 | equipment | 1.157 | 0.514 | 2.271 | 5.174 |
| Enhanced | ASTC 6 x 6 | NPC small | 1.124 | 0.499 | 2.183 | 5.883 |
| Enhanced | ASTC 6 x 6 | NPC medium | 1.065 | 0.473 | 1.885 | 4.223 |
| Enhanced | ASTC 6 x 6 | NPC large | 1.029 | 0.457 | 1.459 | 4.313 |
| Enhanced | ASTC 8 x 8 | equipment | 1.227 | 0.307 | 1.431 | 3.261 |
| Enhanced | ASTC 8 x 8 | NPC small | 1.176 | 0.294 | 1.378 | 3.714 |
| Enhanced | ASTC 8 x 8 | NPC medium | 1.091 | 0.273 | 1.148 | 2.572 |
| Enhanced | ASTC 8 x 8 | NPC large | 1.040 | 0.260 | 0.871 | 2.574 |

#### Classic, creatures, 300 pairs, 132.7 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 41.2 | 179.4 | 50.4 | yes | 9.2 |
| plain planes, zstd 19 | planes | faced only | 8.2 | 35.9 | 10.4 | yes | 2.0 |
| etc2-a1 | blocks | five directions | 27.0 | 83.1 | 23.2 | yes | 4.0 |
| etc2-a1 | blocks | faced only | 5.4 | 16.5 | 4.7 | yes | 0.8 |
| etc2-a1-reuse-1 | blocks | five directions | 25.9 | 83.8 | 23.3 | yes | 3.9 |
| etc2-a1-reuse-1 | blocks | faced only | 5.1 | 16.5 | 4.8 | yes | 0.9 |
| astc-4 | blocks | five directions | 54.6 | 137.7 | 38.6 | yes | 6.9 |
| astc-4 | blocks | faced only | 10.9 | 27.3 | 7.9 | yes | 1.5 |
| astc-6 | blocks | five directions | 27.1 | 73.1 | 20.3 | yes | 3.3 |
| astc-6 | blocks | faced only | 5.4 | 14.4 | 4.2 | yes | 0.7 |
| astc-8 | blocks | five directions | 15.9 | 49.3 | 13.6 | yes | 2.0 |
| astc-8 | blocks | faced only | 3.2 | 9.7 | 2.9 | yes | 0.4 |

#### Classic, dressed NPCs, 300 pairs, 9.7 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 3.6 | 22.8 | 5.9 | yes | 0.3 |
| plain planes, zstd 19 | planes | faced only | 0.7 | 4.7 | 1.3 | yes | 0.1 |
| etc2-a1 | blocks | five directions | 3.0 | 15.2 | 4.0 | yes | 0.2 |
| etc2-a1 | blocks | faced only | 0.6 | 3.3 | 0.9 | yes | 0.0 |
| etc2-a1-reuse-1 | blocks | five directions | 2.9 | 15.2 | 3.9 | yes | 0.2 |
| etc2-a1-reuse-1 | blocks | faced only | 0.6 | 3.2 | 0.9 | yes | 0.0 |
| astc-4 | blocks | five directions | 5.7 | 22.7 | 5.8 | yes | 0.2 |
| astc-4 | blocks | faced only | 1.2 | 4.8 | 1.3 | yes | 0.1 |
| astc-6 | blocks | five directions | 3.2 | 14.5 | 3.7 | yes | 0.1 |
| astc-6 | blocks | faced only | 0.7 | 3.0 | 0.8 | yes | 0.0 |
| astc-8 | blocks | five directions | 2.0 | 11.8 | 3.0 | yes | 0.1 |
| astc-8 | blocks | faced only | 0.4 | 2.5 | 0.7 | yes | 0.0 |

#### Enhanced, creatures, 300 pairs, 198.3 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 78.5 | 402.3 | 102.2 | yes | 18.9 |
| plain planes, zstd 19 | planes | faced only | 15.7 | 81.8 | 20.8 | yes | 3.2 |
| etc2-a1 | blocks | five directions | 36.0 | 111.8 | 28.7 | yes | 5.1 |
| etc2-a1 | blocks | faced only | 7.2 | 22.2 | 5.7 | yes | 0.9 |
| etc2-a1-reuse-1 | blocks | five directions | 34.9 | 112.7 | 28.8 | yes | 5.0 |
| etc2-a1-reuse-1 | blocks | faced only | 7.0 | 22.5 | 5.8 | yes | 0.9 |
| astc-4 | blocks | five directions | 80.6 | 194.3 | 49.2 | yes | 8.9 |
| astc-4 | blocks | faced only | 16.1 | 39.1 | 10.0 | yes | 1.6 |
| astc-6 | blocks | five directions | 39.6 | 101.8 | 26.0 | yes | 4.1 |
| astc-6 | blocks | faced only | 7.9 | 20.3 | 5.2 | yes | 0.7 |
| astc-8 | blocks | five directions | 23.8 | 66.8 | 16.9 | yes | 2.5 |
| astc-8 | blocks | faced only | 4.8 | 13.3 | 3.5 | yes | 0.5 |

#### Enhanced, dressed NPCs, 300 pairs, 15.8 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 9.9 | 59.3 | 16.1 | yes | 1.0 |
| plain planes, zstd 19 | planes | faced only | 2.0 | 12.2 | 3.5 | yes | 0.2 |
| etc2-a1 | blocks | five directions | 4.3 | 20.2 | 5.2 | yes | 0.2 |
| etc2-a1 | blocks | faced only | 0.9 | 4.2 | 1.1 | yes | 0.1 |
| etc2-a1-reuse-1 | blocks | five directions | 4.1 | 19.7 | 5.1 | yes | 0.2 |
| etc2-a1-reuse-1 | blocks | faced only | 0.8 | 4.1 | 1.1 | yes | 0.1 |
| astc-4 | blocks | five directions | 9.8 | 30.9 | 7.9 | yes | 0.4 |
| astc-4 | blocks | faced only | 2.0 | 6.3 | 1.7 | yes | 0.1 |
| astc-6 | blocks | five directions | 5.1 | 18.0 | 4.6 | yes | 0.2 |
| astc-6 | blocks | faced only | 1.0 | 3.8 | 1.0 | yes | 0.1 |
| astc-8 | blocks | five directions | 3.2 | 13.1 | 3.4 | yes | 0.1 |
| astc-8 | blocks | faced only | 0.6 | 2.8 | 0.7 | yes | 0.0 |


### Basis Universal

**No Basis row is both smaller and better than the plain block format beside it.** Where one is smaller, it scores lower, and the reverse. Upstream Basis Universal (HEAD 073eb6a, Apache-2.0) ran from the scratch project, one file per (action, direction). Every 40th body (26 Classic, 24 Enhanced) for ETC1S, UASTC and XUBC7, and every 80th (13 and 12) for the windowed ASTC rows and XUASTC. MB are each sample scaled to the whole set, so compare a row with the BC1 or ASTC row of its own sample, not with the every-10th tables.

**Against the rows it would replace,** native target, on the same bodies. Median / p5 / min, then bodies at 80 or better:

| Client, sample | row | MB | score | at 80 |
|---|---|---:|---|---:|
| Classic, 40th | BC1, zstd 19 | 563.9 | 80.62 / 63.85 / 52.47 | 15 of 26 |
| Classic, 40th | ETC1S q50 | 584.7 | 69.22 / 28.13 / 20.15 | 6 of 26 |
| Classic, 40th | ETC1S q128 | 678.4 | 74.81 / 34.76 / 25.28 | 10 of 26 |
| Classic, 40th | UASTC, RDO 4 | 1111.6 | 81.00 / 66.13 / 60.56 | 16 of 26 |
| Classic, 40th | XUBC7 q75, RDO 10 | 1328.0 | 83.74 / 65.25 / 61.02 | 18 of 26 |
| Enhanced, 40th | BC1, zstd 19 | 876.9 | 71.45 / 28.49 / 16.78 | 5 of 24 |
| Enhanced, 40th | ETC1S q50 | 1106.4 | 68.02 / 51.62 / 49.07 | 6 of 24 |
| Enhanced, 40th | ETC1S q128 | 1343.1 | 75.57 / 55.15 / 52.86 | 9 of 24 |
| Enhanced, 40th | UASTC, RDO 4 | 2030.1 | 80.32 / 73.14 / 70.16 | 13 of 24 |
| Enhanced, 40th | XUBC7 q75, RDO 10 | 2485.3 | 83.91 / 71.26 / 67.55 | 21 of 24 |
| Classic, 80th | ASTC 6×6, astcenc, zstd 19 | 594.7 | 73.85 / 62.89 / 62.89 | 4 of 13 |
| Classic, 80th | ASTC 6×6, Basis, zstd 19 | 583.7 | 70.14 / 60.75 / 60.75 | 1 of 13 |
| Classic, 80th | ASTC 6×6, Basis RDO, 5 dB | 579.2 | 68.89 / 57.71 / 57.71 | 1 of 13 |
| Classic, 80th | XUASTC 6×6 q75 | 787.8 | 69.39 / 60.72 / 60.72 | 1 of 13 |
| Enhanced, 80th | ASTC 6×6, astcenc, zstd 19 | 1080.2 | 67.04 / 46.05 / 46.05 | 2 of 12 |
| Enhanced, 80th | ASTC 6×6, Basis, zstd 19 | 1067.9 | 66.08 / 41.62 / 41.62 | 2 of 12 |
| Enhanced, 80th | ASTC 6×6, Basis RDO, 5 dB | 1054.7 | 66.33 / 40.47 / 40.47 | 1 of 12 |
| Enhanced, 80th | XUASTC 6×6 q75 | 1400.8 | 66.15 / 41.80 / 41.80 | 1 of 12 |

- **ETC1S:** larger than BC1 with zstd on both clients, and lower in score at q50. Its worst bodies fall hard on Classic: p5 28.13 against BC1's 63.85. On Enhanced it keeps edges better than BC1, whose p5 is 28.49.
- **UASTC:** 83.05 on Classic and 82.68 on Enhanced at 1 byte a pixel in VRAM, below fix C's 86.31 and 86.12 on the same bodies. RDO 4, the strongest level run, saves about 3 %. It still leaves Classic 1.5 times above row spans, and Enhanced above every zstd 19 lossless row.
- **XUBC7:** the best Basis score, and larger than every lossless row on both clients.
- **XUASTC:** about 30 % larger than plain ASTC 6×6 with zstd, at a lower score. The excess sits on small streams: Enhanced equipment takes 3.004 bits a true pixel against 1.961, and large NPCs 1.898 against 1.630. Classic is 3.666 against 2.381, and 2.299 against 1.905.

**The overhead on small streams is why.** Each `.basis` file carries its own tables. ETC1S at q50, bytes a block by stream size, against raw BC1's 8:

| Stream size, blocks | Classic | Enhanced |
|---:|---:|---:|
| up to 16 | 37.61 | 62.11 |
| up to 64 | 17.70 | 21.08 |
| up to 256 | 9.16 | 11.22 |
| up to 1024 | 5.26 | 5.63 |
| up to 4096 | 3.37 | 4.05 |
| more | 2.74 | 2.41 |

**So does it only work as atlases? No, but it only pays in larger units.** `.basis` images in one file may differ in size, so it works without an atlas, and the rows here have none. One file a body is 1.507 times smaller on Classic and 1.409 times on Enhanced (`t_bodies_*`). Scaling the q50 rows by those ratios gives about 388 MB and 785 MB, which are estimates at q50's scores, not packed results. A file holds one set of tables and transcodes image by image, so a faced-only load would read the body's tables and one direction's images. That read wasn't measured.

**Transcoding moves quality too.** The score depends on the target as well as the file, since all four alpha targets are 1 byte a pixel and don't hold what the file held:

| Row, Classic / Enhanced medians at the 40th | native | to BC7 | to ASTC 4×4 | to BC3 | to ETC2 + EAC |
|---|---:|---:|---:|---:|---:|
| UASTC | 83.05 / 82.68 | 83.26 / 82.48 | 83.03 / 82.73 | 76.82 / 78.33 | 77.08 / 77.46 |
| ETC1S q128 | 74.81 / 75.57 | 71.31 / 71.39 | 72.14 / 73.64 | 71.60 / 74.89 | 74.81 / 75.56 |
| XUBC7 q75, RDO 10 | 83.74 / 83.91 | 83.74 / 83.91 | 75.05 / 76.07 | 77.22 / 78.76 | 67.17 / 71.87 |

- **BC1 as a target doesn't work here.** Upstream's `cTFBC1_RGB` is "Opaque only, no punchthrough alpha support yet", so every animation frame with transparency would have to take BC3's 1 byte a pixel. **That means Basis can't reach BC1's or ETC2 RGB8A1's half-byte VRAM with these sprites.** That rules out the point of the low-end tier on the desktop path.
- **XUASTC 6×6 q75 transcodes too,** at every 80th: Classic 69.39 native, 71.24 to BC7, 69.82 to BC3, 65.92 to ETC2 + EAC; Enhanced 66.15, 65.84, 63.76, 56.81. None reaches 80.

**ASTC RDO:** `astcenc` has none. Basis's windowed RDO is the only one reachable, and it only reuses config and endpoints from the left, upper and diagonal blocks, so it stays inside a frame and a stream. Its default gate (`m_ls_min_psnr` 35 dB) skips almost every block at 6×6 and 8×8, so the default RDO row was the same size as the row without it. With the gate lowered to 15 dB and 5 dB allowed, it saves 0.8 % on Classic and 1.2 % on Enhanced, at a lower score. **It isn't worth the encode time:** Basis's ASTC encoder costs about 35 worker seconds a Mpx against astcenc's 4.3 at preset medium, and astcenc's own rows score higher.

**`thorough` doesn't change the verdict:** at every 40th, ASTC 6×6 goes from 71.80 to 72.80 on Classic and 70.64 to 71.94 on Enhanced, at 1 % more disk and about twice the encode time. It stays below 80 and below BC1's Classic score.

**Does Basis pass the crowd line? Mostly not with five directions.** Quiet runs, with the other cluster 1 % busy. A cell is file read + transcode to that target, on 4 threads. Plain planes at zstd 19 in the same runs take 50.7 and 102.2 ms with five directions, and 10.5 and 20.8 ms faced.

| Crowd, target | ETC1S q50, Classic / Enhanced ms | UASTC RDO 1 | XUBC7 q75 |
|---|---:|---:|---:|
| Creatures, five directions, to BC1, no alpha | **226.1** / 314.0 | 448.0 / 587.1 | 2039.5 / 2512.9 |
| Creatures, five directions, to BC3 | 376.0 / 571.9 | 524.0 / 699.5 | 2117.9 / 2647.1 |
| Creatures, five directions, to BC7 | 1086.1 / 1464.7 | 508.7 / 690.9 | 1012.7 / 1194.1 |
| Creatures, five directions, to ETC2 + EAC | 251.4 / 374.1 | 470.1 / 651.7 | 2116.1 / 2773.6 |
| Creatures, five directions, to ASTC 4×4 | 446.8 / 676.2 | 254.1 / 330.1 | 2278.5 / 2783.6 |
| Creatures, faced, to BC1, no alpha | **46.2** / **63.6** | **92.3** / **117.0** | 415.2 / 504.0 |
| Creatures, faced, to BC3 | **74.9** / **116.9** | **107.6** / **141.4** | 430.8 / 531.9 |
| Creatures, faced, to BC7 | **220.4** / 294.5 | **102.4** / **138.8** | **208.2** / **241.7** |
| Creatures, faced, to ETC2 + EAC | **50.4** / **76.1** | **96.5** / **130.6** | 431.7 / 557.2 |
| Creatures, faced, to ASTC 4×4 | **89.7** / **138.6** | **52.2** / **66.7** | 463.4 / 560.4 |

- **With five directions on creatures, the only pass is Classic's ETC1S to BC1, and that target has no alpha.** ETC1S to ETC2 and UASTC to ASTC 4×4 sit right at the line on Classic, at 251.4 and 254.1 ms. Nothing passes on Enhanced.
- **Faced only, ETC1S and UASTC pass almost everywhere,** except ETC1S to BC7 on Enhanced. XUBC7 passes only to BC7, its own target.
- **Even each client's fastest row is 4.5 × and 3.1 × plain planes,** so a passing row uses most of the line before the upload starts.
- **Dressed NPCs pass everywhere on Classic.** On Enhanced with five directions, XUBC7 fails to every target but BC7; every other Enhanced row passes.
- **Not measured:** `xuastc-6-q75` in the crowd. It scored below plain ASTC 6×6 and was larger, so it wouldn't change anything.

#### Classic: every 40th body: disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 1163.0 | 588.8 | 580.7 | 71.80 | 53.05 | 44.73 | 7 of 26 | 6.847 | 199 | 28.07 | 0.000 % | 0.123 % |
| ASTC 6 x 6, thorough | 1163.0 | 594.1 | 586.4 | 72.80 | 53.79 | 46.42 | 8 of 26 | 6.645 | 186 | 28.37 | 0.000 % | 0.078 % |
| BC1 (`dds`, High) | 1269.5 | 576.6 | 563.9 | 80.62 | 63.85 | 52.47 | 15 of 26 | 6.590 | 141 | 28.73 | 0.000 % | 0.000 % |
| ETC1S q128, to ASTC 4 x 4 | 678.4 | 657.0 | 678.4 | 72.14 | 34.14 | 25.30 | 9 of 26 | 7.463 | 157 | 27.74 | 0.000 % | 0.000 % |
| ETC1S q128, to BC3 | 678.4 | 657.0 | 678.4 | 71.60 | 34.89 | 25.72 | 7 of 26 | 7.464 | 141 | 27.77 | 0.000 % | 0.000 % |
| ETC1S q128, to BC7 | 678.4 | 657.0 | 678.4 | 71.31 | 30.15 | 23.79 | 9 of 26 | 7.489 | 143 | 27.69 | 0.000 % | 0.000 % |
| ETC1S q128, to ETC2 + EAC | 678.4 | 657.0 | 678.4 | 74.81 | 34.76 | 25.28 | 10 of 26 | 7.305 | 136 | 28.00 | 0.000 % | 0.000 % |
| ETC1S q128, native | 678.4 | 657.0 | 678.4 | 74.81 | 34.76 | 25.28 | 10 of 26 | 7.305 | 136 | 28.00 | 0.000 % | 0.000 % |
| ETC1S q50, to ASTC 4 x 4 | 584.7 | 562.5 | 584.7 | 65.62 | 27.71 | 20.46 | 6 of 26 | 9.518 | 191 | 25.69 | 0.000 % | 0.033 % |
| ETC1S q50, to BC3 | 584.7 | 562.5 | 584.7 | 67.15 | 27.91 | 20.46 | 4 of 26 | 9.487 | 191 | 25.71 | 0.000 % | 0.006 % |
| ETC1S q50, to BC7 | 584.7 | 562.5 | 584.7 | 64.96 | 23.96 | 19.42 | 6 of 26 | 9.492 | 196 | 25.69 | 0.000 % | 0.040 % |
| ETC1S q50, to ETC2 + EAC | 584.7 | 562.5 | 584.7 | 69.22 | 28.13 | 20.15 | 6 of 26 | 9.382 | 191 | 25.82 | 0.000 % | 0.006 % |
| ETC1S q50, native | 584.7 | 562.5 | 584.7 | 69.22 | 28.13 | 20.15 | 6 of 26 | 9.382 | 191 | 25.82 | 0.000 % | 0.006 % |
| UASTC, to ASTC 4 x 4 | 2583.7 | 1165.6 | 1147.4 | 83.03 | 66.44 | 60.65 | 17 of 26 | 3.730 | 130 | 32.84 | 0.000 % | 0.002 % |
| UASTC, to BC3 | 2583.7 | 1165.6 | 1147.4 | 76.82 | 59.33 | 48.61 | 10 of 26 | 7.367 | 158 | 27.78 | 0.000 % | 0.002 % |
| UASTC, to BC7 | 2583.7 | 1165.6 | 1147.4 | 83.26 | 66.27 | 60.67 | 17 of 26 | 3.816 | 130 | 32.72 | 0.000 % | 0.002 % |
| UASTC, to ETC2 + EAC | 2583.7 | 1165.6 | 1147.4 | 77.08 | 46.68 | 33.52 | 9 of 26 | 6.463 | 167 | 28.70 | 0.000 % | 0.002 % |
| UASTC, native | 2583.7 | 1165.6 | 1147.4 | 83.05 | 66.46 | 60.68 | 17 of 26 | 3.715 | 130 | 32.86 | 0.000 % | 0.002 % |
| UASTC, RDO 1, to ASTC 4 x 4 | 2583.7 | 1164.1 | 1146.0 | 83.01 | 66.15 | 60.59 | 17 of 26 | 3.762 | 130 | 32.79 | 0.000 % | 0.002 % |
| UASTC, RDO 1, to BC3 | 2583.7 | 1164.1 | 1146.0 | 76.81 | 59.34 | 48.58 | 10 of 26 | 7.387 | 158 | 27.77 | 0.000 % | 0.002 % |
| UASTC, RDO 1, to BC7 | 2583.7 | 1164.1 | 1146.0 | 83.22 | 65.97 | 60.60 | 17 of 26 | 3.851 | 130 | 32.66 | 0.000 % | 0.002 % |
| UASTC, RDO 1, to ETC2 + EAC | 2583.7 | 1164.1 | 1146.0 | 77.09 | 46.63 | 33.51 | 9 of 26 | 6.475 | 167 | 28.69 | 0.000 % | 0.002 % |
| UASTC, RDO 1, native | 2583.7 | 1164.1 | 1146.0 | 83.04 | 66.17 | 60.62 | 17 of 26 | 3.746 | 130 | 32.81 | 0.000 % | 0.002 % |
| UASTC, RDO 4, to ASTC 4 x 4 | 2583.7 | 1129.6 | 1111.6 | 81.00 | 66.11 | 60.52 | 16 of 26 | 4.615 | 130 | 31.10 | 0.000 % | 0.003 % |
| UASTC, RDO 4, to BC3 | 2583.7 | 1129.6 | 1111.6 | 75.27 | 59.32 | 48.54 | 8 of 26 | 7.999 | 158 | 27.19 | 0.000 % | 0.002 % |
| UASTC, RDO 4, to BC7 | 2583.7 | 1129.6 | 1111.6 | 81.30 | 65.94 | 60.54 | 16 of 26 | 4.695 | 130 | 31.02 | 0.000 % | 0.003 % |
| UASTC, RDO 4, to ETC2 + EAC | 2583.7 | 1129.6 | 1111.6 | 75.66 | 46.66 | 33.48 | 7 of 26 | 6.993 | 167 | 28.19 | 0.000 % | 0.003 % |
| UASTC, RDO 4, native | 2583.7 | 1129.6 | 1111.6 | 81.00 | 66.13 | 60.56 | 16 of 26 | 4.603 | 130 | 31.11 | 0.000 % | 0.003 % |
| XUBC7 q75, RDO 10, to ASTC 4 x 4 | 1328.0 | 1177.9 | 1328.0 | 75.05 | 61.07 | 55.95 | 8 of 26 | 6.246 | 149 | 28.87 | 0.000 % | 0.647 % |
| XUBC7 q75, RDO 10, to BC3 | 1328.0 | 1177.9 | 1328.0 | 77.22 | 59.70 | 49.89 | 9 of 26 | 8.005 | 150 | 27.44 | 0.000 % | 0.002 % |
| XUBC7 q75, RDO 10, to BC7 | 1328.0 | 1177.9 | 1328.0 | 83.74 | 65.25 | 61.02 | 18 of 26 | 4.926 | 113 | 31.45 | 0.000 % | 0.002 % |
| XUBC7 q75, RDO 10, to ETC2 + EAC | 1328.0 | 1177.9 | 1328.0 | 67.17 | 39.91 | 28.05 | 5 of 26 | 8.801 | 171 | 26.25 | 0.000 % | 20.311 % |
| XUBC7 q75, RDO 10, native | 1328.0 | 1177.9 | 1328.0 | 83.74 | 65.25 | 61.02 | 18 of 26 | 4.926 | 113 | 31.45 | 0.000 % | 0.002 % |

#### Classic: every 40th body: sizes in this sample and encode time

| row | raw MB | zstd 7 MB | zstd 19 MB | stored MB | encode, worker s a Mpx |
|---|---:|---:|---:|---:|---:|
| astc-6 | 23.1 | 11.7 | 11.5 | 11.5 | 3.487 |
| astc-6-thorough | 23.1 | 11.8 | 11.6 | 11.6 | 6.560 |
| etc1s-q50 | 11.6 | 11.2 | 10.9 | 11.6 | 2.652 |
| etc1s-q128 | 13.5 | 13.0 | 12.8 | 13.5 | 2.236 |
| uastc | 51.3 | 23.1 | 22.8 | 22.8 | 5.911 |
| uastc-rdo-1 | 51.3 | 23.1 | 22.8 | 22.8 | 10.108 |
| uastc-rdo-4 | 51.3 | 22.4 | 22.1 | 22.1 | 10.305 |
| xubc7-q75-r10 | 26.4 | 23.4 | 23.1 | 26.4 | 2.602 |

#### Classic: every 40th body: hue

| row | key changed | moved, mean | worst | full hue channel error, mean | worst | hued pixels off by 8 or more | partial hue error, mean | worst | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 62.46 % | 0.943 | 25 | 4.132 | 184 | 58.138 % | 10.093 | 255 | 40.23 % | 29.15 % | 10.467 % | 14.185 % |
| ASTC 6 x 6, thorough | 61.55 % | 0.915 | 24 | 4.009 | 176 | 57.260 % | 9.894 | 255 | 34.58 % | 25.22 % | 10.927 % | 14.355 % |
| BC1 (`dds`, High) | 60.11 % | 0.875 | 18 | 3.861 | 136 | 56.427 % | 9.271 | 255 | 77.80 % | 41.70 % | 0.869 % | 2.650 % |
| ETC1S q128, to ASTC 4 x 4 | 66.53 % | 1.040 | 20 | 4.521 | 152 | 61.461 % | 9.287 | 255 | 41.75 % | 36.12 % | 4.096 % | 7.266 % |
| ETC1S q128, to BC3 | 65.81 % | 1.029 | 18 | 4.491 | 136 | 61.124 % | 10.275 | 255 | 72.75 % | 41.98 % | 2.912 % | 6.563 % |
| ETC1S q128, to BC7 | 66.07 % | 1.045 | 18 | 4.570 | 136 | 61.433 % | 9.291 | 255 | 50.73 % | 43.64 % | 2.686 % | 5.185 % |
| ETC1S q128, to ETC2 + EAC | 65.76 % | 1.011 | 17 | 4.383 | 128 | 60.623 % | 9.145 | 255 | 41.83 % | 39.26 % | 4.083 % | 5.472 % |
| ETC1S q128, native | 65.76 % | 1.011 | 17 | 4.383 | 128 | 60.623 % | 9.145 | 255 | 41.83 % | 39.26 % | 4.083 % | 5.472 % |
| ETC1S q50, to ASTC 4 x 4 | 71.41 % | 1.295 | 24 | 5.636 | 176 | 66.700 % | 11.227 | 255 | 43.95 % | 38.16 % | 4.310 % | 7.986 % |
| ETC1S q50, to BC3 | 70.98 % | 1.284 | 24 | 5.616 | 176 | 66.626 % | 12.108 | 255 | 72.54 % | 44.23 % | 3.188 % | 6.609 % |
| ETC1S q50, to BC7 | 71.10 % | 1.296 | 25 | 5.674 | 184 | 66.772 % | 11.178 | 255 | 52.16 % | 45.09 % | 2.855 % | 5.271 % |
| ETC1S q50, to ETC2 + EAC | 70.85 % | 1.271 | 24 | 5.525 | 176 | 66.102 % | 11.092 | 255 | 43.85 % | 41.39 % | 4.322 % | 5.620 % |
| ETC1S q50, native | 70.85 % | 1.271 | 24 | 5.525 | 176 | 66.102 % | 11.092 | 255 | 43.85 % | 41.39 % | 4.322 % | 5.620 % |
| UASTC, to ASTC 4 x 4 | 47.69 % | 0.573 | 15 | 2.466 | 112 | 43.140 % | 5.694 | 255 | 45.11 % | 31.94 % | 3.344 % | 7.106 % |
| UASTC, to BC3 | 62.62 % | 0.977 | 20 | 4.279 | 152 | 58.438 % | 10.092 | 255 | 76.63 % | 44.26 % | 2.204 % | 4.862 % |
| UASTC, to BC7 | 46.62 % | 0.565 | 15 | 2.428 | 112 | 42.147 % | 5.768 | 255 | 44.61 % | 31.42 % | 3.425 % | 6.997 % |
| UASTC, to ETC2 + EAC | 62.60 % | 0.921 | 20 | 4.002 | 144 | 57.693 % | 9.171 | 255 | 31.04 % | 30.01 % | 9.000 % | 9.738 % |
| UASTC, native | 49.41 % | 0.591 | 15 | 2.544 | 112 | 44.663 % | 5.683 | 255 | 45.14 % | 33.63 % | 3.343 % | 7.133 % |
| UASTC, RDO 1, to ASTC 4 x 4 | 47.94 % | 0.577 | 15 | 2.484 | 112 | 43.380 % | 5.748 | 255 | 46.02 % | 32.65 % | 3.336 % | 7.105 % |
| UASTC, RDO 1, to BC3 | 62.75 % | 0.979 | 20 | 4.287 | 152 | 58.545 % | 10.114 | 255 | 76.83 % | 44.35 % | 2.207 % | 4.873 % |
| UASTC, RDO 1, to BC7 | 46.85 % | 0.569 | 15 | 2.445 | 112 | 42.363 % | 5.826 | 255 | 45.52 % | 32.05 % | 3.417 % | 7.006 % |
| UASTC, RDO 1, to ETC2 + EAC | 62.66 % | 0.922 | 20 | 4.006 | 144 | 57.746 % | 9.185 | 255 | 31.10 % | 30.07 % | 9.020 % | 9.754 % |
| UASTC, RDO 1, native | 49.63 % | 0.595 | 15 | 2.560 | 112 | 44.875 % | 5.734 | 255 | 45.99 % | 34.38 % | 3.335 % | 7.138 % |
| UASTC, RDO 4, to ASTC 4 x 4 | 51.95 % | 0.675 | 15 | 2.920 | 112 | 47.379 % | 6.585 | 255 | 46.60 % | 32.34 % | 3.378 % | 7.039 % |
| UASTC, RDO 4, to BC3 | 65.03 % | 1.055 | 20 | 4.627 | 152 | 60.883 % | 10.710 | 255 | 77.26 % | 44.73 % | 2.234 % | 4.923 % |
| UASTC, RDO 4, to BC7 | 51.15 % | 0.669 | 15 | 2.890 | 112 | 46.632 % | 6.653 | 255 | 46.11 % | 31.96 % | 3.460 % | 6.972 % |
| UASTC, RDO 4, to ETC2 + EAC | 64.70 % | 0.983 | 20 | 4.275 | 144 | 59.828 % | 9.733 | 255 | 30.96 % | 29.92 % | 9.370 % | 10.094 % |
| UASTC, RDO 4, native | 53.32 % | 0.690 | 15 | 2.984 | 112 | 48.584 % | 6.576 | 255 | 46.62 % | 33.76 % | 3.378 % | 7.058 % |
| XUBC7 q75, RDO 10, to ASTC 4 x 4 | 60.29 % | 0.868 | 19 | 3.770 | 144 | 55.605 % | 8.182 | 255 | 54.32 % | 35.01 % | 2.665 % | 6.113 % |
| XUBC7 q75, RDO 10, to BC3 | 67.07 % | 1.063 | 19 | 4.664 | 136 | 62.768 % | 10.734 | 255 | 77.29 % | 42.99 % | 2.449 % | 5.108 % |
| XUBC7 q75, RDO 10, to BC7 | 55.12 % | 0.695 | 15 | 3.030 | 112 | 50.624 % | 6.798 | 255 | 51.37 % | 34.43 % | 2.139 % | 4.960 % |
| XUBC7 q75, RDO 10, to ETC2 + EAC | 69.53 % | 1.213 | 22 | 5.298 | 160 | 64.879 % | 11.723 | 255 | 26.61 % | 25.79 % | 12.234 % | 12.964 % |
| XUBC7 q75, RDO 10, native | 55.12 % | 0.695 | 15 | 3.030 | 112 | 50.624 % | 6.798 | 255 | 51.37 % | 34.43 % | 2.139 % | 4.960 % |

#### Enhanced: every 40th body: disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 2349.6 | 1076.3 | 1059.9 | 70.64 | 59.44 | 59.44 | 6 of 24 | 7.309 | 213 | 26.90 | 0.597 % | 0.882 % |
| ASTC 6 x 6, thorough | 2349.6 | 1087.0 | 1071.9 | 71.94 | 59.85 | 59.80 | 6 of 24 | 7.129 | 231 | 27.09 | 0.576 % | 0.815 % |
| BC1 (`dds`, High) | 2569.2 | 900.4 | 876.9 | 71.45 | 28.49 | 16.78 | 5 of 24 | 5.042 | 185 | 30.28 | 14.412 % | 0.000 % |
| ETC1S q128, to ASTC 4 x 4 | 1343.1 | 1301.8 | 1343.1 | 73.64 | 50.24 | 38.98 | 7 of 24 | 7.720 | 181 | 26.67 | 1.680 % | 4.368 % |
| ETC1S q128, to BC3 | 1343.1 | 1301.8 | 1343.1 | 74.89 | 50.27 | 42.73 | 7 of 24 | 7.687 | 181 | 26.78 | 1.896 % | 0.502 % |
| ETC1S q128, to BC7 | 1343.1 | 1301.8 | 1343.1 | 71.39 | 51.73 | 50.44 | 7 of 24 | 7.789 | 206 | 26.65 | 1.229 % | 6.939 % |
| ETC1S q128, to ETC2 + EAC | 1343.1 | 1301.8 | 1343.1 | 75.56 | 55.14 | 52.83 | 9 of 24 | 7.399 | 181 | 27.11 | 1.812 % | 0.642 % |
| ETC1S q128, native | 1343.1 | 1301.8 | 1343.1 | 75.57 | 55.15 | 52.86 | 9 of 24 | 7.402 | 181 | 27.11 | 1.702 % | 1.307 % |
| ETC1S q50, to ASTC 4 x 4 | 1106.4 | 1062.0 | 1106.4 | 64.25 | 47.13 | 31.58 | 5 of 24 | 9.629 | 255 | 24.95 | 1.782 % | 4.701 % |
| ETC1S q50, to BC3 | 1106.4 | 1062.0 | 1106.4 | 65.52 | 47.37 | 34.74 | 5 of 24 | 9.589 | 255 | 25.04 | 2.097 % | 0.856 % |
| ETC1S q50, to BC7 | 1106.4 | 1062.0 | 1106.4 | 62.68 | 46.17 | 41.91 | 6 of 24 | 9.645 | 229 | 24.99 | 1.313 % | 7.112 % |
| ETC1S q50, to ETC2 + EAC | 1106.4 | 1062.0 | 1106.4 | 68.01 | 51.61 | 49.06 | 6 of 24 | 9.378 | 255 | 25.19 | 2.025 % | 0.907 % |
| ETC1S q50, native | 1106.4 | 1062.0 | 1106.4 | 68.02 | 51.62 | 49.07 | 6 of 24 | 9.383 | 255 | 25.19 | 1.906 % | 1.493 % |
| UASTC, to ASTC 4 x 4 | 5205.1 | 2140.2 | 2102.7 | 82.73 | 73.51 | 70.20 | 17 of 24 | 4.450 | 165 | 30.79 | 0.450 % | 1.645 % |
| UASTC, to BC3 | 5205.1 | 2140.2 | 2102.7 | 78.33 | 63.78 | 62.95 | 9 of 24 | 6.908 | 213 | 27.72 | 0.883 % | 1.633 % |
| UASTC, to BC7 | 5205.1 | 2140.2 | 2102.7 | 82.48 | 73.54 | 70.19 | 18 of 24 | 4.518 | 165 | 30.71 | 0.447 % | 1.674 % |
| UASTC, to ETC2 + EAC | 5205.1 | 2140.2 | 2102.7 | 77.46 | 60.53 | 59.02 | 9 of 24 | 7.168 | 227 | 26.97 | 0.486 % | 3.089 % |
| UASTC, native | 5205.1 | 2140.2 | 2102.7 | 82.68 | 73.44 | 70.24 | 17 of 24 | 4.436 | 165 | 30.79 | 0.450 % | 1.645 % |
| UASTC, RDO 1, to ASTC 4 x 4 | 5205.1 | 2131.8 | 2094.2 | 82.63 | 73.36 | 70.21 | 17 of 24 | 4.505 | 165 | 30.73 | 0.440 % | 1.619 % |
| UASTC, RDO 1, to BC3 | 5205.1 | 2131.8 | 2094.2 | 78.27 | 63.84 | 63.02 | 9 of 24 | 6.947 | 213 | 27.70 | 0.893 % | 1.604 % |
| UASTC, RDO 1, to BC7 | 5205.1 | 2131.8 | 2094.2 | 82.47 | 73.37 | 70.19 | 17 of 24 | 4.574 | 165 | 30.66 | 0.437 % | 1.685 % |
| UASTC, RDO 1, to ETC2 + EAC | 5205.1 | 2131.8 | 2094.2 | 77.45 | 60.51 | 59.08 | 9 of 24 | 7.203 | 227 | 26.95 | 0.478 % | 3.048 % |
| UASTC, RDO 1, native | 5205.1 | 2131.8 | 2094.2 | 82.61 | 73.29 | 70.24 | 17 of 24 | 4.489 | 165 | 30.73 | 0.440 % | 1.619 % |
| UASTC, RDO 4, to ASTC 4 x 4 | 5205.1 | 2067.0 | 2030.1 | 80.35 | 73.20 | 70.13 | 13 of 24 | 5.112 | 165 | 29.98 | 0.457 % | 1.660 % |
| UASTC, RDO 4, to BC3 | 5205.1 | 2067.0 | 2030.1 | 76.48 | 63.78 | 62.94 | 8 of 24 | 7.375 | 213 | 27.34 | 0.906 % | 1.627 % |
| UASTC, RDO 4, to BC7 | 5205.1 | 2067.0 | 2030.1 | 80.25 | 73.22 | 70.11 | 14 of 24 | 5.173 | 165 | 29.92 | 0.454 % | 1.726 % |
| UASTC, RDO 4, to ETC2 + EAC | 5205.1 | 2067.0 | 2030.1 | 76.70 | 60.48 | 59.01 | 7 of 24 | 7.560 | 227 | 26.72 | 0.495 % | 3.086 % |
| UASTC, RDO 4, native | 5205.1 | 2067.0 | 2030.1 | 80.32 | 73.14 | 70.16 | 13 of 24 | 5.101 | 165 | 29.98 | 0.457 % | 1.660 % |
| XUBC7 q75, RDO 10, to ASTC 4 x 4 | 2485.3 | 2200.1 | 2485.3 | 76.07 | 64.85 | 63.39 | 8 of 24 | 6.725 | 190 | 27.72 | 0.572 % | 3.587 % |
| XUBC7 q75, RDO 10, to BC3 | 2485.3 | 2200.1 | 2485.3 | 78.76 | 63.08 | 62.64 | 10 of 24 | 7.335 | 187 | 27.62 | 0.643 % | 3.004 % |
| XUBC7 q75, RDO 10, to BC7 | 2485.3 | 2200.1 | 2485.3 | 83.91 | 71.26 | 67.55 | 21 of 24 | 5.025 | 180 | 30.77 | 0.432 % | 3.016 % |
| XUBC7 q75, RDO 10, to ETC2 + EAC | 2485.3 | 2200.1 | 2485.3 | 71.87 | 57.03 | 56.71 | 5 of 24 | 8.751 | 182 | 25.49 | 0.224 % | 10.476 % |
| XUBC7 q75, RDO 10, native | 2485.3 | 2200.1 | 2485.3 | 83.91 | 71.26 | 67.55 | 21 of 24 | 5.025 | 180 | 30.77 | 0.432 % | 3.016 % |

#### Enhanced: every 40th body: sizes in this sample and encode time

| row | raw MB | zstd 7 MB | zstd 19 MB | stored MB | encode, worker s a Mpx |
|---|---:|---:|---:|---:|---:|
| astc-6 | 53.1 | 24.3 | 24.0 | 24.0 | 3.935 |
| astc-6-thorough | 53.1 | 24.6 | 24.2 | 24.2 | 7.515 |
| etc1s-q50 | 25.0 | 24.0 | 23.6 | 25.0 | 3.095 |
| etc1s-q128 | 30.4 | 29.4 | 29.0 | 30.4 | 2.822 |
| uastc | 117.7 | 48.4 | 47.5 | 47.5 | 6.118 |
| uastc-rdo-1 | 117.7 | 48.2 | 47.4 | 47.4 | 8.607 |
| uastc-rdo-4 | 117.7 | 46.7 | 45.9 | 45.9 | 8.661 |
| xubc7-q75-r10 | 56.2 | 49.7 | 49.2 | 56.2 | 2.339 |

#### Enhanced: every 40th body: hue

| row | key changed | key moved, mean | worst | hued channel error, mean | worst | hued pixels off by 8 or more |
|---|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 91.25 % | 7.880 | 213 | 3.081 | 210 | 24.598 % |
| ASTC 6 x 6, thorough | 90.38 % | 7.697 | 231 | 3.010 | 225 | 23.994 % |
| BC1 (`dds`, High) | 89.73 % | 5.568 | 143 | 2.166 | 143 | 15.738 % |
| ETC1S q128, to ASTC 4 x 4 | 93.20 % | 8.487 | 181 | 3.308 | 174 | 27.034 % |
| ETC1S q128, to BC3 | 92.78 % | 8.465 | 181 | 3.297 | 174 | 27.395 % |
| ETC1S q128, to BC7 | 92.84 % | 8.778 | 181 | 3.424 | 174 | 28.532 % |
| ETC1S q128, to ETC2 + EAC | 92.94 % | 8.059 | 181 | 3.145 | 174 | 25.843 % |
| ETC1S q128, native | 92.94 % | 8.062 | 181 | 3.146 | 174 | 25.856 % |
| ETC1S q50, to ASTC 4 x 4 | 94.45 % | 10.489 | 253 | 4.106 | 240 | 34.271 % |
| ETC1S q50, to BC3 | 94.02 % | 10.449 | 255 | 4.087 | 240 | 34.545 % |
| ETC1S q50, to BC7 | 94.13 % | 10.660 | 223 | 4.177 | 223 | 35.368 % |
| ETC1S q50, to ETC2 + EAC | 94.27 % | 10.124 | 253 | 3.963 | 240 | 33.285 % |
| ETC1S q50, native | 94.27 % | 10.129 | 253 | 3.965 | 240 | 33.302 % |
| UASTC, to ASTC 4 x 4 | 81.94 % | 4.752 | 151 | 1.840 | 151 | 13.404 % |
| UASTC, to BC3 | 91.41 % | 7.610 | 173 | 2.966 | 166 | 24.236 % |
| UASTC, to BC7 | 82.23 % | 4.815 | 152 | 1.865 | 152 | 13.729 % |
| UASTC, to ETC2 + EAC | 92.51 % | 7.843 | 200 | 3.051 | 193 | 24.438 % |
| UASTC, native | 81.27 % | 4.734 | 151 | 1.833 | 151 | 13.387 % |
| UASTC, RDO 1, to ASTC 4 x 4 | 82.43 % | 4.825 | 151 | 1.867 | 151 | 13.678 % |
| UASTC, RDO 1, to BC3 | 91.47 % | 7.647 | 173 | 2.982 | 166 | 24.441 % |
| UASTC, RDO 1, to BC7 | 82.79 % | 4.890 | 152 | 1.894 | 152 | 14.003 % |
| UASTC, RDO 1, to ETC2 + EAC | 92.59 % | 7.876 | 200 | 3.063 | 193 | 24.594 % |
| UASTC, RDO 1, native | 81.77 % | 4.804 | 151 | 1.860 | 151 | 13.658 % |
| UASTC, RDO 4, to ASTC 4 x 4 | 84.71 % | 5.548 | 151 | 2.157 | 151 | 16.766 % |
| UASTC, RDO 4, to BC3 | 91.99 % | 8.147 | 173 | 3.181 | 166 | 26.764 % |
| UASTC, RDO 4, to BC7 | 84.93 % | 5.605 | 152 | 2.180 | 152 | 17.097 % |
| UASTC, RDO 4, to ETC2 + EAC | 93.12 % | 8.250 | 200 | 3.213 | 193 | 26.372 % |
| UASTC, RDO 4, native | 84.23 % | 5.534 | 151 | 2.152 | 151 | 16.752 % |
| XUBC7 q75, RDO 10, to ASTC 4 x 4 | 90.82 % | 7.282 | 190 | 2.838 | 184 | 22.447 % |
| XUBC7 q75, RDO 10, to BC3 | 92.27 % | 8.009 | 165 | 3.125 | 158 | 26.873 % |
| XUBC7 q75, RDO 10, to BC7 | 88.46 % | 5.425 | 153 | 2.115 | 153 | 15.923 % |
| XUBC7 q75, RDO 10, to ETC2 + EAC | 93.82 % | 9.593 | 180 | 3.760 | 172 | 30.522 % |
| XUBC7 q75, RDO 10, native | 88.46 % | 5.425 | 153 | 2.115 | 153 | 15.923 % |

#### Enhanced: every 40th body: coverage at edges

| row | edge pixels' error, mean levels | worst | edge pixels off by a level or more | solid pixels no longer solid |
|---|---:|---:|---:|---:|
| ASTC 6 x 6 | 1.122 | 15.1 | 42.66 % | 8.111 % |
| ASTC 6 x 6, thorough | 1.079 | 15.1 | 41.27 % | 7.360 % |
| BC1 (`dds`, High) | 3.914 | 8.0 | 89.67 % | 0.000 % |
| ETC1S q128, to ASTC 4 x 4 | 0.996 | 6.6 | 41.32 % | 13.244 % |
| ETC1S q128, to BC3 | 0.857 | 5.5 | 36.04 % | 1.879 % |
| ETC1S q128, to BC7 | 0.971 | 6.4 | 39.87 % | 20.349 % |
| ETC1S q128, to ETC2 + EAC | 0.843 | 5.3 | 35.02 % | 2.564 % |
| ETC1S q128, native | 0.845 | 5.3 | 35.00 % | 3.662 % |
| ETC1S q50, to ASTC 4 x 4 | 1.107 | 12.7 | 45.32 % | 13.892 % |
| ETC1S q50, to BC3 | 0.977 | 12.0 | 40.83 % | 2.789 % |
| ETC1S q50, to BC7 | 1.086 | 12.7 | 43.87 % | 21.255 % |
| ETC1S q50, to ETC2 + EAC | 0.962 | 12.0 | 39.98 % | 3.260 % |
| ETC1S q50, native | 0.966 | 12.0 | 39.73 % | 4.464 % |
| UASTC, to ASTC 4 x 4 | 0.696 | 9.8 | 27.01 % | 7.910 % |
| UASTC, to BC3 | 0.804 | 10.0 | 31.87 % | 6.117 % |
| UASTC, to BC7 | 0.702 | 10.0 | 27.49 % | 36.002 % |
| UASTC, to ETC2 + EAC | 0.741 | 9.7 | 28.89 % | 10.374 % |
| UASTC, native | 0.696 | 9.8 | 26.97 % | 7.765 % |
| UASTC, RDO 1, to ASTC 4 x 4 | 0.695 | 9.8 | 26.95 % | 8.148 % |
| UASTC, RDO 1, to BC3 | 0.807 | 10.0 | 31.87 % | 6.288 % |
| UASTC, RDO 1, to BC7 | 0.703 | 10.0 | 27.40 % | 39.613 % |
| UASTC, RDO 1, to ETC2 + EAC | 0.743 | 9.7 | 28.86 % | 10.575 % |
| UASTC, RDO 1, native | 0.695 | 9.8 | 26.92 % | 7.978 % |
| UASTC, RDO 4, to ASTC 4 x 4 | 0.724 | 9.8 | 28.46 % | 8.243 % |
| UASTC, RDO 4, to BC3 | 0.833 | 10.0 | 33.30 % | 6.367 % |
| UASTC, RDO 4, to BC7 | 0.731 | 10.0 | 28.89 % | 39.686 % |
| UASTC, RDO 4, to ETC2 + EAC | 0.770 | 9.7 | 30.34 % | 10.673 % |
| UASTC, RDO 4, native | 0.724 | 9.8 | 28.43 % | 8.074 % |
| XUBC7 q75, RDO 10, to ASTC 4 x 4 | 0.955 | 12.3 | 37.44 % | 10.543 % |
| XUBC7 q75, RDO 10, to BC3 | 0.658 | 10.4 | 20.89 % | 11.209 % |
| XUBC7 q75, RDO 10, to BC7 | 0.607 | 10.2 | 18.29 % | 13.289 % |
| XUBC7 q75, RDO 10, to ETC2 + EAC | 0.644 | 10.2 | 19.57 % | 26.503 % |
| XUBC7 q75, RDO 10, native | 0.607 | 10.2 | 18.29 % | 13.289 % |

#### Classic: every 80th body: disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 1162.2 | 603.3 | 594.7 | 73.85 | 62.89 | 62.89 | 4 of 13 | 6.548 | 174 | 27.92 | 0.000 % | 0.187 % |
| ASTC 8 x 8 | 676.9 | 366.6 | 360.3 | 60.60 | 33.61 | 33.61 | 0 of 13 | 10.980 | 212 | 23.56 | 0.000 % | 1.313 % |
| ASTC 6 x 6, Basis encoder | 1162.2 | 595.2 | 583.7 | 70.14 | 60.75 | 60.75 | 1 of 13 | 7.231 | 192 | 26.98 | 0.000 % | 0.196 % |
| ASTC 6 x 6, Basis RDO 2 dB, gate 20 dB | 1162.2 | 594.4 | 582.3 | 70.00 | 60.49 | 60.49 | 1 of 13 | 7.283 | 192 | 26.94 | 0.000 % | 0.197 % |
| ASTC 6 x 6, Basis RDO 5 dB, gate 15 dB | 1162.2 | 592.0 | 579.2 | 68.89 | 57.71 | 57.71 | 1 of 13 | 7.473 | 199 | 26.74 | 0.000 % | 0.217 % |
| ASTC 8 x 8, Basis encoder | 676.9 | 365.2 | 358.4 | 54.55 | 25.43 | 25.43 | 0 of 13 | 12.052 | 220 | 22.72 | 0.003 % | 1.107 % |
| ASTC 8 x 8, Basis RDO 2 dB, gate 20 dB | 676.9 | 363.6 | 356.6 | 54.55 | 24.38 | 24.38 | 0 of 13 | 12.147 | 220 | 22.67 | 0.003 % | 1.089 % |
| BC1 (`dds`, High) | 1269.3 | 586.7 | 572.2 | 83.03 | 70.68 | 70.68 | 8 of 13 | 6.366 | 132 | 28.71 | 0.000 % | 0.000 % |
| XUASTC 6 x 6 q75, to ASTC 6 x 6 | 787.8 | 748.6 | 787.8 | 69.38 | 60.72 | 60.72 | 1 of 13 | 7.849 | 192 | 26.63 | 0.000 % | 0.227 % |
| XUASTC 6 x 6 q75, to BC3 | 787.8 | 748.6 | 787.8 | 69.82 | 57.53 | 57.53 | 1 of 13 | 9.759 | 192 | 25.05 | 0.000 % | 0.204 % |
| XUASTC 6 x 6 q75, to BC7 | 787.8 | 748.6 | 787.8 | 71.24 | 59.01 | 59.01 | 1 of 13 | 8.325 | 191 | 26.20 | 0.000 % | 0.748 % |
| XUASTC 6 x 6 q75, to ETC2 + EAC | 787.8 | 748.6 | 787.8 | 65.92 | 43.87 | 43.87 | 1 of 13 | 10.275 | 213 | 24.42 | 0.000 % | 19.622 % |
| XUASTC 6 x 6 q75, native | 787.8 | 748.6 | 787.8 | 69.39 | 60.72 | 60.72 | 1 of 13 | 7.844 | 192 | 26.63 | 0.000 % | 0.227 % |

#### Classic: every 80th body: sizes in this sample and encode time

| row | raw MB | zstd 7 MB | zstd 19 MB | stored MB | encode, worker s a Mpx |
|---|---:|---:|---:|---:|---:|
| astc-6 | 11.6 | 6.0 | 5.9 | 5.9 | 4.241 |
| astc-8 | 6.8 | 3.7 | 3.6 | 3.6 | 5.447 |
| bastc-6 | 11.6 | 5.9 | 5.8 | 5.8 | 38.724 |
| bastc-6-rdo-2-min-20 | 11.6 | 5.9 | 5.8 | 5.8 | 35.615 |
| bastc-6-rdo-5-min-15 | 11.6 | 5.9 | 5.8 | 5.8 | 34.141 |
| bastc-8 | 6.8 | 3.6 | 3.6 | 3.6 | 37.631 |
| bastc-8-rdo-2-min-20 | 6.8 | 3.6 | 3.6 | 3.6 | 36.219 |
| xuastc-6-q75 | 7.9 | 7.5 | 7.4 | 7.9 | 41.485 |

#### Classic: every 80th body: hue

| row | key changed | moved, mean | worst | full hue channel error, mean | worst | hued pixels off by 8 or more | partial hue error, mean | worst | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 60.74 % | 0.928 | 22 | 4.113 | 160 | 57.064 % | 10.533 | 255 | 30.74 % | 23.74 % | 9.409 % | 11.711 % |
| ASTC 8 x 8 | 70.96 % | 1.488 | 27 | 6.587 | 200 | 67.606 % | 15.961 | 255 | 26.44 % | 21.81 % | 16.887 % | 18.493 % |
| ASTC 6 x 6, Basis encoder | 62.41 % | 1.011 | 24 | 4.477 | 176 | 58.745 % | 10.920 | 255 | 31.70 % | 25.82 % | 8.971 % | 11.246 % |
| ASTC 6 x 6, Basis RDO 2 dB, gate 20 dB | 62.63 % | 1.017 | 24 | 4.503 | 176 | 58.964 % | 10.977 | 255 | 31.73 % | 25.86 % | 8.987 % | 11.275 % |
| ASTC 6 x 6, Basis RDO 5 dB, gate 15 dB | 63.23 % | 1.040 | 25 | 4.608 | 184 | 59.571 % | 11.186 | 255 | 31.70 % | 25.85 % | 9.117 % | 11.383 % |
| ASTC 8 x 8, Basis encoder | 72.25 % | 1.616 | 26 | 7.155 | 192 | 68.989 % | 16.588 | 255 | 30.56 % | 25.33 % | 15.382 % | 17.200 % |
| ASTC 8 x 8, Basis RDO 2 dB, gate 20 dB | 72.45 % | 1.628 | 26 | 7.208 | 192 | 69.209 % | 16.716 | 255 | 30.27 % | 25.12 % | 15.675 % | 17.475 % |
| BC1 (`dds`, High) | 60.80 % | 0.884 | 16 | 3.952 | 120 | 57.506 % | 10.683 | 255 | 81.33 % | 40.05 % | 0.668 % | 3.465 % |
| XUASTC 6 x 6 q75, to ASTC 6 x 6 | 65.61 % | 1.081 | 24 | 4.793 | 176 | 61.870 % | 11.526 | 255 | 31.02 % | 25.11 % | 9.220 % | 11.224 % |
| XUASTC 6 x 6 q75, to BC3 | 70.79 % | 1.310 | 24 | 5.800 | 176 | 67.125 % | 13.844 | 255 | 82.18 % | 40.69 % | 1.789 % | 7.915 % |
| XUASTC 6 x 6 q75, to BC7 | 66.65 % | 1.139 | 24 | 5.037 | 176 | 62.885 % | 11.486 | 255 | 39.62 % | 31.03 % | 6.292 % | 9.300 % |
| XUASTC 6 x 6 q75, to ETC2 + EAC | 71.86 % | 1.407 | 27 | 6.219 | 200 | 68.098 % | 14.392 | 255 | 23.52 % | 23.00 % | 13.576 % | 13.798 % |
| XUASTC 6 x 6 q75, native | 65.61 % | 1.082 | 24 | 4.795 | 176 | 61.867 % | 11.518 | 255 | 30.98 % | 25.09 % | 9.219 % | 11.220 % |

#### Enhanced: every 80th body: disk at the whole-set scale and quality

| row | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | channel error mean | worst | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 2345.4 | 1093.0 | 1080.2 | 67.04 | 46.05 | 46.05 | 2 of 12 | 7.176 | 214 | 27.29 | 0.565 % | 0.811 % |
| ASTC 8 x 8 | 1356.0 | 671.0 | 662.1 | 55.65 | 33.87 | 33.87 | 1 of 12 | 10.649 | 251 | 23.92 | 0.672 % | 2.189 % |
| ASTC 6 x 6, Basis encoder | 2345.4 | 1081.9 | 1067.9 | 66.08 | 41.62 | 41.62 | 2 of 12 | 7.565 | 239 | 26.76 | 0.766 % | 0.780 % |
| ASTC 6 x 6, Basis RDO 2 dB, gate 20 dB | 2345.4 | 1078.9 | 1063.8 | 66.36 | 41.74 | 41.74 | 1 of 12 | 7.634 | 239 | 26.71 | 0.775 % | 0.792 % |
| ASTC 6 x 6, Basis RDO 5 dB, gate 15 dB | 2345.4 | 1072.2 | 1054.7 | 66.33 | 40.47 | 40.47 | 1 of 12 | 7.898 | 239 | 26.38 | 0.798 % | 0.882 % |
| ASTC 8 x 8, Basis encoder | 1356.0 | 668.2 | 658.8 | 53.53 | 30.37 | 30.37 | 1 of 12 | 11.083 | 254 | 23.54 | 0.822 % | 2.282 % |
| ASTC 8 x 8, Basis RDO 2 dB, gate 20 dB | 1356.0 | 666.2 | 656.2 | 52.90 | 30.36 | 30.36 | 1 of 12 | 11.168 | 254 | 23.49 | 0.825 % | 2.291 % |
| BC1 (`dds`, High) | 2565.3 | 976.6 | 955.9 | 72.85 | 52.73 | 52.73 | 3 of 12 | 5.605 | 151 | 29.90 | 9.311 % | 0.000 % |
| XUASTC 6 x 6 q75, to ASTC 6 x 6 | 1400.8 | 1340.7 | 1400.8 | 66.19 | 41.76 | 41.76 | 1 of 12 | 8.110 | 239 | 26.43 | 0.758 % | 1.016 % |
| XUASTC 6 x 6 q75, to BC3 | 1400.8 | 1340.7 | 1400.8 | 63.76 | 39.11 | 39.11 | 1 of 12 | 9.798 | 237 | 24.96 | 1.034 % | 0.712 % |
| XUASTC 6 x 6 q75, to BC7 | 1400.8 | 1340.7 | 1400.8 | 65.84 | 40.75 | 40.75 | 1 of 12 | 8.612 | 238 | 25.93 | 0.645 % | 3.214 % |
| XUASTC 6 x 6 q75, to ETC2 + EAC | 1400.8 | 1340.7 | 1400.8 | 56.81 | 35.85 | 35.85 | 1 of 12 | 10.596 | 250 | 24.12 | 0.176 % | 10.085 % |
| XUASTC 6 x 6 q75, native | 1400.8 | 1340.7 | 1400.8 | 66.15 | 41.80 | 41.80 | 1 of 12 | 8.103 | 239 | 26.43 | 0.758 % | 1.007 % |

#### Enhanced: every 80th body: sizes in this sample and encode time

| row | raw MB | zstd 7 MB | zstd 19 MB | stored MB | encode, worker s a Mpx |
|---|---:|---:|---:|---:|---:|
| astc-6 | 26.7 | 12.5 | 12.3 | 12.3 | 4.284 |
| astc-8 | 15.5 | 7.6 | 7.5 | 7.5 | 5.350 |
| bastc-6 | 26.7 | 12.3 | 12.2 | 12.2 | 35.209 |
| bastc-6-rdo-2-min-20 | 26.7 | 12.3 | 12.1 | 12.1 | 34.244 |
| bastc-6-rdo-5-min-15 | 26.7 | 12.2 | 12.0 | 12.0 | 33.203 |
| bastc-8 | 15.5 | 7.6 | 7.5 | 7.5 | 35.995 |
| bastc-8-rdo-2-min-20 | 15.5 | 7.6 | 7.5 | 7.5 | 39.320 |
| xuastc-6-q75 | 16.0 | 15.3 | 15.1 | 16.0 | 39.401 |

#### Enhanced: every 80th body: hue

| row | key changed | key moved, mean | worst | hued channel error, mean | worst | hued pixels off by 8 or more |
|---|---:|---:|---:|---:|---:|---:|
| ASTC 6 x 6 | 93.41 % | 7.901 | 203 | 3.132 | 199 | 25.321 % |
| ASTC 8 x 8 | 95.74 % | 11.913 | 243 | 4.735 | 236 | 39.190 % |
| ASTC 6 x 6, Basis encoder | 93.65 % | 8.308 | 223 | 3.292 | 216 | 26.711 % |
| ASTC 6 x 6, Basis RDO 2 dB, gate 20 dB | 93.74 % | 8.397 | 223 | 3.327 | 216 | 27.110 % |
| ASTC 6 x 6, Basis RDO 5 dB, gate 15 dB | 93.94 % | 8.717 | 229 | 3.455 | 222 | 28.245 % |
| ASTC 8 x 8, Basis encoder | 95.81 % | 12.362 | 242 | 4.914 | 234 | 40.309 % |
| ASTC 8 x 8, Basis RDO 2 dB, gate 20 dB | 95.86 % | 12.474 | 242 | 4.959 | 234 | 40.706 % |
| BC1 (`dds`, High) | 92.30 % | 6.343 | 142 | 2.520 | 142 | 19.720 % |
| XUASTC 6 x 6 q75, to ASTC 6 x 6 | 94.60 % | 9.030 | 223 | 3.581 | 216 | 30.230 % |
| XUASTC 6 x 6 q75, to BC3 | 95.46 % | 11.075 | 231 | 4.400 | 224 | 38.069 % |
| XUASTC 6 x 6 q75, to BC7 | 94.93 % | 9.659 | 227 | 3.835 | 226 | 32.638 % |
| XUASTC 6 x 6 q75, to ETC2 + EAC | 95.99 % | 12.156 | 220 | 4.828 | 213 | 40.378 % |
| XUASTC 6 x 6 q75, native | 94.57 % | 9.026 | 223 | 3.580 | 216 | 30.199 % |

#### Enhanced: every 80th body: coverage at edges

| row | edge pixels' error, mean levels | worst | edge pixels off by a level or more | solid pixels no longer solid |
|---|---:|---:|---:|---:|
| ASTC 6 x 6 | 1.286 | 14.2 | 49.18 % | 5.879 % |
| ASTC 8 x 8 | 1.940 | 15.1 | 65.61 % | 9.354 % |
| ASTC 6 x 6, Basis encoder | 1.367 | 15.1 | 52.54 % | 5.348 % |
| ASTC 6 x 6, Basis RDO 2 dB, gate 20 dB | 1.373 | 15.1 | 52.77 % | 5.254 % |
| ASTC 6 x 6, Basis RDO 5 dB, gate 15 dB | 1.416 | 15.1 | 53.83 % | 5.275 % |
| ASTC 8 x 8, Basis encoder | 2.055 | 15.1 | 68.25 % | 8.794 % |
| ASTC 8 x 8, Basis RDO 2 dB, gate 20 dB | 2.062 | 15.1 | 68.42 % | 8.699 % |
| BC1 (`dds`, High) | 3.801 | 8.0 | 89.20 % | 0.000 % |
| XUASTC 6 x 6 q75, to ASTC 6 x 6 | 1.410 | 15.1 | 54.12 % | 5.809 % |
| XUASTC 6 x 6 q75, to BC3 | 1.505 | 15.1 | 57.13 % | 4.390 % |
| XUASTC 6 x 6 q75, to BC7 | 1.554 | 15.1 | 56.84 % | 31.577 % |
| XUASTC 6 x 6 q75, to ETC2 + EAC | 1.460 | 14.9 | 52.36 % | 21.423 % |
| XUASTC 6 x 6 q75, native | 1.409 | 15.1 | 54.08 % | 5.769 % |

Classic ETC1S, a file a stream against a file a body:

```text
cc: etc1s-q50: 11.6 MB in one file a stream, 7.7 MB in one file a body (1.507 x), 26 bodies, 3221 streams
  up to 16 blocks: 102 streams, 0.0 MB, 37.61 bytes a block (raw BC1 is 8)
  up to 64 blocks: 560 streams, 0.4 MB, 17.70 bytes a block (raw BC1 is 8)
  up to 256 blocks: 1137 streams, 1.6 MB, 9.16 bytes a block (raw BC1 is 8)
  up to 1024 blocks: 914 streams, 2.2 MB, 5.26 bytes a block (raw BC1 is 8)
  up to 4096 blocks: 296 streams, 2.1 MB, 3.37 bytes a block (raw BC1 is 8)
  up to more blocks: 212 streams, 5.2 MB, 2.74 bytes a block (raw BC1 is 8)
```

Enhanced ETC1S, a file a stream against a file a body:

```text
ec: etc1s-q50: 25.0 MB in one file a stream, 17.8 MB in one file a body (1.409 x), 24 bodies, 4459 streams
  up to 16 blocks: 1 streams, 0.0 MB, 62.11 bytes a block (raw BC1 is 8)
  up to 64 blocks: 277 streams, 0.3 MB, 21.08 bytes a block (raw BC1 is 8)
  up to 256 blocks: 1278 streams, 2.1 MB, 11.22 bytes a block (raw BC1 is 8)
  up to 1024 blocks: 1568 streams, 5.1 MB, 5.63 bytes a block (raw BC1 is 8)
  up to 4096 blocks: 920 streams, 6.7 MB, 4.05 bytes a block (raw BC1 is 8)
  up to more blocks: 415 streams, 10.8 MB, 2.41 bytes a block (raw BC1 is 8)
```

#### Classic, creatures, 300 pairs, 132.7 Mpx in five directions (transcode from the file; quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 41.2 | 180.1 | 50.7 | yes | 9.7 |
| plain planes, zstd 19 | planes | faced only | 8.2 | 36.5 | 10.5 | yes | 2.0 |
| etc1s-q50 | bc1-opaque | five directions | 19.6 | 826.6 | 226.1 | yes | 32.5 |
| etc1s-q50 | bc1-opaque | faced only | 3.9 | 166.5 | 46.2 | yes | 6.4 |
| etc1s-q50 | bc3 | five directions | 19.6 | 1388.7 | 376.0 | no | 66.2 |
| etc1s-q50 | bc3 | faced only | 3.9 | 276.9 | 74.9 | yes | 12.4 |
| etc1s-q50 | bc7 | five directions | 19.6 | 3901.1 | 1086.1 | no | 228.5 |
| etc1s-q50 | bc7 | faced only | 3.9 | 775.4 | 220.4 | yes | 40.8 |
| etc1s-q50 | etc2-eac | five directions | 19.6 | 928.3 | 251.4 | no | 37.2 |
| etc1s-q50 | etc2-eac | faced only | 3.9 | 185.2 | 50.4 | yes | 6.9 |
| etc1s-q50 | astc-4 | five directions | 19.6 | 1656.4 | 446.8 | no | 83.1 |
| etc1s-q50 | astc-4 | faced only | 3.9 | 330.1 | 89.7 | yes | 16.6 |
| uastc-rdo-1 | bc1-opaque | five directions | 53.9 | 1598.3 | 448.0 | no | 84.8 |
| uastc-rdo-1 | bc1-opaque | faced only | 10.7 | 316.9 | 92.3 | yes | 18.7 |
| uastc-rdo-1 | bc3 | five directions | 53.9 | 1883.7 | 524.0 | no | 96.0 |
| uastc-rdo-1 | bc3 | faced only | 10.7 | 371.7 | 107.6 | yes | 21.4 |
| uastc-rdo-1 | bc7 | five directions | 53.9 | 1814.6 | 508.7 | no | 113.6 |
| uastc-rdo-1 | bc7 | faced only | 10.7 | 357.3 | 102.4 | yes | 20.0 |
| uastc-rdo-1 | etc2-eac | five directions | 53.9 | 1688.9 | 470.1 | no | 85.2 |
| uastc-rdo-1 | etc2-eac | faced only | 10.7 | 336.9 | 96.5 | yes | 18.0 |
| uastc-rdo-1 | astc-4 | five directions | 53.9 | 911.9 | 254.1 | no | 48.5 |
| uastc-rdo-1 | astc-4 | faced only | 10.7 | 181.2 | 52.2 | yes | 10.2 |
| xubc7-q75-r10 | bc1-opaque | five directions | 53.0 | 7309.3 | 2039.5 | no | 415.4 |
| xubc7-q75-r10 | bc1-opaque | faced only | 10.6 | 1443.7 | 415.2 | no | 82.6 |
| xubc7-q75-r10 | bc3 | five directions | 53.0 | 7600.6 | 2117.9 | no | 429.5 |
| xubc7-q75-r10 | bc3 | faced only | 10.6 | 1501.7 | 430.8 | no | 85.0 |
| xubc7-q75-r10 | bc7 | five directions | 53.0 | 3627.7 | 1012.7 | no | 188.9 |
| xubc7-q75-r10 | bc7 | faced only | 10.6 | 721.5 | 208.2 | yes | 40.6 |
| xubc7-q75-r10 | etc2-eac | five directions | 53.0 | 7588.2 | 2116.1 | no | 415.8 |
| xubc7-q75-r10 | etc2-eac | faced only | 10.6 | 1506.4 | 431.7 | no | 83.7 |
| xubc7-q75-r10 | astc-4 | five directions | 53.0 | 8140.0 | 2278.5 | no | 475.8 |
| xubc7-q75-r10 | astc-4 | faced only | 10.6 | 1610.4 | 463.4 | no | 94.5 |

#### Classic, dressed NPCs, 300 pairs, 9.7 Mpx in five directions (transcode from the file; quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 3.6 | 22.7 | 5.9 | yes | 0.3 |
| plain planes, zstd 19 | planes | faced only | 0.7 | 4.7 | 1.2 | yes | 0.1 |
| etc1s-q50 | bc1-opaque | five directions | 3.7 | 142.1 | 36.3 | yes | 1.4 |
| etc1s-q50 | bc1-opaque | faced only | 0.8 | 29.6 | 7.5 | yes | 0.3 |
| etc1s-q50 | bc3 | five directions | 3.7 | 197.7 | 50.1 | yes | 2.2 |
| etc1s-q50 | bc3 | faced only | 0.8 | 40.5 | 10.5 | yes | 0.5 |
| etc1s-q50 | bc7 | five directions | 3.7 | 359.4 | 90.4 | yes | 6.4 |
| etc1s-q50 | bc7 | faced only | 0.8 | 73.7 | 18.9 | yes | 1.4 |
| etc1s-q50 | etc2-eac | five directions | 3.7 | 158.8 | 40.7 | yes | 1.7 |
| etc1s-q50 | etc2-eac | faced only | 0.8 | 33.2 | 8.5 | yes | 0.4 |
| etc1s-q50 | astc-4 | five directions | 3.7 | 216.2 | 54.9 | yes | 2.4 |
| etc1s-q50 | astc-4 | faced only | 0.8 | 44.9 | 11.6 | yes | 0.5 |
| uastc-rdo-1 | bc1-opaque | five directions | 6.2 | 195.9 | 49.6 | yes | 2.5 |
| uastc-rdo-1 | bc1-opaque | faced only | 1.3 | 40.2 | 10.4 | yes | 0.6 |
| uastc-rdo-1 | bc3 | five directions | 6.2 | 239.6 | 61.0 | yes | 3.0 |
| uastc-rdo-1 | bc3 | faced only | 1.3 | 49.4 | 12.7 | yes | 0.7 |
| uastc-rdo-1 | bc7 | five directions | 6.2 | 201.9 | 51.2 | yes | 2.6 |
| uastc-rdo-1 | bc7 | faced only | 1.3 | 41.6 | 10.7 | yes | 0.5 |
| uastc-rdo-1 | etc2-eac | five directions | 6.2 | 229.3 | 58.0 | yes | 3.0 |
| uastc-rdo-1 | etc2-eac | faced only | 1.3 | 47.1 | 12.1 | yes | 0.6 |
| uastc-rdo-1 | astc-4 | five directions | 6.2 | 106.5 | 26.8 | yes | 1.3 |
| uastc-rdo-1 | astc-4 | faced only | 1.3 | 21.7 | 5.7 | yes | 0.3 |
| xubc7-q75-r10 | bc1-opaque | five directions | 8.6 | 694.5 | 177.7 | yes | 11.0 |
| xubc7-q75-r10 | bc1-opaque | faced only | 1.8 | 142.0 | 36.7 | yes | 2.3 |
| xubc7-q75-r10 | bc3 | five directions | 8.6 | 744.7 | 189.8 | yes | 11.6 |
| xubc7-q75-r10 | bc3 | faced only | 1.8 | 151.4 | 39.0 | yes | 2.3 |
| xubc7-q75-r10 | bc7 | five directions | 8.6 | 350.8 | 90.2 | yes | 6.0 |
| xubc7-q75-r10 | bc7 | faced only | 1.8 | 72.0 | 18.6 | yes | 1.2 |
| xubc7-q75-r10 | etc2-eac | five directions | 8.6 | 798.0 | 203.6 | yes | 11.8 |
| xubc7-q75-r10 | etc2-eac | faced only | 1.8 | 163.4 | 41.8 | yes | 2.4 |
| xubc7-q75-r10 | astc-4 | five directions | 8.6 | 693.2 | 177.7 | yes | 10.3 |
| xubc7-q75-r10 | astc-4 | faced only | 1.8 | 142.8 | 36.7 | yes | 2.2 |

#### Enhanced, creatures, 300 pairs, 198.3 Mpx in five directions (transcode from the file; quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 78.5 | 402.2 | 102.2 | yes | 18.1 |
| plain planes, zstd 19 | planes | faced only | 15.7 | 81.7 | 20.8 | yes | 3.1 |
| etc1s-q50 | bc1-opaque | five directions | 31.7 | 1242.9 | 314.0 | no | 38.3 |
| etc1s-q50 | bc1-opaque | faced only | 6.3 | 251.4 | 63.6 | yes | 6.8 |
| etc1s-q50 | bc3 | five directions | 31.7 | 2250.0 | 571.9 | no | 74.7 |
| etc1s-q50 | bc3 | faced only | 6.3 | 457.3 | 116.9 | yes | 13.4 |
| etc1s-q50 | bc7 | five directions | 31.7 | 5810.7 | 1464.7 | no | 252.4 |
| etc1s-q50 | bc7 | faced only | 6.3 | 1166.1 | 294.5 | no | 43.9 |
| etc1s-q50 | etc2-eac | five directions | 31.7 | 1480.8 | 374.1 | no | 39.4 |
| etc1s-q50 | etc2-eac | faced only | 6.3 | 298.7 | 76.1 | yes | 7.5 |
| etc1s-q50 | astc-4 | five directions | 31.7 | 2665.2 | 676.2 | no | 92.0 |
| etc1s-q50 | astc-4 | faced only | 6.3 | 546.1 | 138.6 | yes | 17.7 |
| uastc-rdo-1 | bc1-opaque | five directions | 80.8 | 2292.8 | 587.1 | no | 108.8 |
| uastc-rdo-1 | bc1-opaque | faced only | 16.2 | 469.0 | 117.0 | yes | 19.3 |
| uastc-rdo-1 | bc3 | five directions | 80.8 | 2756.4 | 699.5 | no | 124.3 |
| uastc-rdo-1 | bc3 | faced only | 16.2 | 559.4 | 141.4 | yes | 22.8 |
| uastc-rdo-1 | bc7 | five directions | 80.8 | 2732.7 | 690.9 | no | 111.4 |
| uastc-rdo-1 | bc7 | faced only | 16.2 | 549.1 | 138.8 | yes | 20.4 |
| uastc-rdo-1 | etc2-eac | five directions | 80.8 | 2573.5 | 651.7 | no | 109.4 |
| uastc-rdo-1 | etc2-eac | faced only | 16.2 | 517.3 | 130.6 | yes | 19.8 |
| uastc-rdo-1 | astc-4 | five directions | 80.8 | 1311.1 | 330.1 | no | 61.4 |
| uastc-rdo-1 | astc-4 | faced only | 16.2 | 263.3 | 66.7 | yes | 11.2 |
| xubc7-q75-r10 | bc1-opaque | five directions | 80.3 | 9976.1 | 2512.9 | no | 512.3 |
| xubc7-q75-r10 | bc1-opaque | faced only | 16.1 | 2000.9 | 504.0 | no | 92.3 |
| xubc7-q75-r10 | bc3 | five directions | 80.3 | 10503.4 | 2647.1 | no | 526.0 |
| xubc7-q75-r10 | bc3 | faced only | 16.1 | 2110.3 | 531.9 | no | 94.8 |
| xubc7-q75-r10 | bc7 | five directions | 80.3 | 4726.9 | 1194.1 | no | 259.4 |
| xubc7-q75-r10 | bc7 | faced only | 16.1 | 950.5 | 241.7 | yes | 47.3 |
| xubc7-q75-r10 | etc2-eac | five directions | 80.3 | 11042.3 | 2773.6 | no | 505.3 |
| xubc7-q75-r10 | etc2-eac | faced only | 16.1 | 2212.3 | 557.2 | no | 90.2 |
| xubc7-q75-r10 | astc-4 | five directions | 80.3 | 11088.3 | 2783.6 | no | 535.0 |
| xubc7-q75-r10 | astc-4 | faced only | 16.1 | 2231.5 | 560.4 | no | 95.8 |

#### Enhanced, dressed NPCs, 300 pairs, 15.8 Mpx in five directions (transcode from the file; quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| plain planes, zstd 19 | planes | five directions | 9.9 | 58.8 | 14.9 | yes | 0.7 |
| plain planes, zstd 19 | planes | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| etc1s-q50 | bc1-opaque | five directions | 5.5 | 197.3 | 50.4 | yes | 2.2 |
| etc1s-q50 | bc1-opaque | faced only | 1.1 | 39.8 | 10.4 | yes | 0.5 |
| etc1s-q50 | bc3 | five directions | 5.5 | 290.4 | 74.6 | yes | 3.7 |
| etc1s-q50 | bc3 | faced only | 1.1 | 59.4 | 15.3 | yes | 0.8 |
| etc1s-q50 | bc7 | five directions | 5.5 | 576.1 | 145.8 | yes | 7.7 |
| etc1s-q50 | bc7 | faced only | 1.1 | 117.1 | 29.9 | yes | 1.7 |
| etc1s-q50 | etc2-eac | five directions | 5.5 | 229.8 | 58.7 | yes | 2.8 |
| etc1s-q50 | etc2-eac | faced only | 1.1 | 46.9 | 12.2 | yes | 0.6 |
| etc1s-q50 | astc-4 | five directions | 5.5 | 312.8 | 80.3 | yes | 3.8 |
| etc1s-q50 | astc-4 | faced only | 1.1 | 64.2 | 16.5 | yes | 0.8 |
| uastc-rdo-1 | bc1-opaque | five directions | 10.2 | 288.1 | 74.3 | yes | 3.9 |
| uastc-rdo-1 | bc1-opaque | faced only | 2.0 | 58.4 | 15.1 | yes | 0.8 |
| uastc-rdo-1 | bc3 | five directions | 10.2 | 358.4 | 91.7 | yes | 4.8 |
| uastc-rdo-1 | bc3 | faced only | 2.0 | 72.6 | 18.8 | yes | 1.0 |
| uastc-rdo-1 | bc7 | five directions | 10.2 | 307.2 | 78.7 | yes | 4.2 |
| uastc-rdo-1 | bc7 | faced only | 2.0 | 62.0 | 16.0 | yes | 0.9 |
| uastc-rdo-1 | etc2-eac | five directions | 10.2 | 347.7 | 88.7 | yes | 4.5 |
| uastc-rdo-1 | etc2-eac | faced only | 2.0 | 70.5 | 18.1 | yes | 0.9 |
| uastc-rdo-1 | astc-4 | five directions | 10.2 | 155.0 | 39.7 | yes | 1.8 |
| uastc-rdo-1 | astc-4 | faced only | 2.0 | 31.6 | 8.1 | yes | 0.4 |
| xubc7-q75-r10 | bc1-opaque | five directions | 12.8 | 1046.5 | 270.1 | no | 16.4 |
| xubc7-q75-r10 | bc1-opaque | faced only | 2.6 | 211.3 | 55.1 | yes | 3.4 |
| xubc7-q75-r10 | bc3 | five directions | 12.8 | 1125.8 | 291.2 | no | 17.5 |
| xubc7-q75-r10 | bc3 | faced only | 2.6 | 227.3 | 59.2 | yes | 3.7 |
| xubc7-q75-r10 | bc7 | five directions | 12.8 | 518.0 | 134.5 | yes | 8.5 |
| xubc7-q75-r10 | bc7 | faced only | 2.6 | 105.4 | 27.7 | yes | 1.9 |
| xubc7-q75-r10 | etc2-eac | five directions | 12.8 | 1247.2 | 322.5 | no | 18.3 |
| xubc7-q75-r10 | etc2-eac | faced only | 2.6 | 251.1 | 65.3 | yes | 3.8 |
| xubc7-q75-r10 | astc-4 | five directions | 12.8 | 1062.0 | 274.9 | no | 15.5 |
| xubc7-q75-r10 | astc-4 | faced only | 2.6 | 215.4 | 55.7 | yes | 3.3 |


### The low-end mix: a block format above a size threshold

The `tk` results already answer this for lossless disk, where one coder is right. This part measures the mix for the low-end tier only. **The layout is a per-body choice:** a body whose median cropped frame area is above the threshold takes a block format for all its frames, and the rest stay lossless R8 or RG8. **Lossless here is plain planes at zstd 19, per stream,** not the row-span coder, in both the disk and decode rows.

**Reading the tables below:**

- **Classic's BC1 is the only format in the mix where most coded bodies reach 80:** 28 of 33 above 2500 px, 9 of 9 above 10000, 2 of 2 above 40000. Above 2500 px it has a median of 81.77 and a min of 75.52, takes 523.5 MB on disk (0.72 × all lossless), and halves the creature crowd's VRAM (69.2 MB against 132.7, five directions). Dressed NPCs barely benefit (8.6 MB against 9.7), because they're mostly small bodies.
- **Enhanced, BC1 doesn't work at any threshold.** Above 2500 px only 1 of 38 bodies reaches 80, with a p5 of −13.58 and a min of −46.34, from one-bit alpha.
- **Enhanced, ASTC 6×6 above 2500 px** keeps its edges (min 63.85, median 73.08) but only 2 of 38 bodies reach 80.
- **Enhanced, BC7 depends entirely on the encoder.** Above 2500 px, the repo's encoder as called gets 0 of 38 bodies to 80 (median 58.44). With fix C it's 36 of 38 (median 86.18, min 77.62) at 1931.1 MB, and with fix C and RDO 1 it's 28 of 38 (median 81.40, min 76.28) at 1805.4 MB. Those rows come from a scratch copy, not the repo encoder.
- **Enhanced, BC1 on large bodies with BC7 on the rest** works only at a high threshold. Above 10000 px with fix C on the rest it takes 1390.0 MB, with 62 of 93 bodies at 80 and a min of 51.11. Above 2500 px the medium bodies bring back the −46.34.
- **ASTC 8×8 never approaches 80 on these bodies,** with medians of 55 to 64. It's the smallest row by far, so it's the one to look at in the crops below before ruling it out.

**Crowd VRAM isn't the whole set's.** Large bodies are 59.8 % of Classic's whole-set pixels and 76 to 78 % of its creature crowd's, and none of its dressed crowd's. Equipment is 17 % of the set but 54 % of the dressed crowd. So a threshold saves on creatures and does almost nothing for dressed NPCs. Enhanced has the same shape: large is 76 % of creatures, and equipment is 54 % of dressed.

**Crops to look at** are in `/home/claudio/rust-target/uodynamapper-scratch/anim-codec/blocks-preview/`, with an `index.txt`: worst and median frames for BC1, ASTC 6×6 and ASTC 8×8 next to lossless, on large bodies and on a few equipment frames. They're extracted art, so they aren't committed.

**Bodies to look at first,** worst at each threshold:

| client | format | above 2500 | above 10000 | above 40000 |
|---|---|---|---|---|
| Classic | BC1 | anim2.mul 16 (75.5), anim4.mul 202 (78.5), anim.mul 24 (78.6) | AnimationFrame4.uop 1434 (80.6), AnimationFrame2.uop 1484 (81.1) | anim5.mul 198 (81.2), AnimationFrame3.uop 1248 (82.9) |
| Classic | ASTC 6×6 | anim2.mul 16 (61.9), AnimationFrame4.uop 1434 (67.9), anim2.mul 49 (69.1) | AnimationFrame4.uop 1434 (67.9), AnimationFrame2.uop 1484 (70.1) | anim5.mul 198 (74.4), AnimationFrame3.uop 1248 (77.9) |
| Classic | ASTC 8×8 | anim2.mul 16 (26.9), anim2.mul 49 (38.3), AnimationFrame2.uop 432 (41.2) | AnimationFrame4.uop 1434 (42.8), AnimationFrame2.uop 1484 (49.2) | anim5.mul 198 (57.9), AnimationFrame3.uop 1248 (64.2) |
| Enhanced | BC1 | 16 (−46.3), 777 (−13.6), 1245 (40.5) | 11 (55.0), 1248 (56.9), 315 (68.0) | 1248 (56.9), 106 (79.0) |
| Enhanced | ASTC 6×6 | 830 (63.9), 675 (65.8), 67 (65.8) | 830 (63.9), 432 (70.1), 1419 (71.8) | the table's last column |
| Enhanced | ASTC 8×8 | 830 (44.1), 1255 (45.7), 307 (46.6) | 830 (44.1), 432 (54.2), 1419 (55.4) | the table's last column |

**Not measured here:** hue from colour instead of index, in a real renderer. Section 15 has a headless measurement of the second shader path; it doesn't replace a renderer test.

#### Classic: the mix on every 10th body; scores and hue are the block-coded bodies only

| above | format | bodies | Mpx coded | disk MB | x all lossless | SSIMULACRA2 median | p5 | min | at 80 or better | RGB PSNR dB | channel error | painted pixels gone | hue key changed | key moved | hued error | hued worst | edge levels, mean | worst bodies |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 0 | BC1 (`dds`, High) | 103 | 249.9 | 504.2 | 0.694 | 83.17 | 65.67 | 41.97 | 71 of 103 | 30.13 | 5.045 | 0.000 % | 50.70 % | 0.707 | 3.102 | 160 | 0.000 | AnimationFrame4.uop body 831 (NPC small, median 525 px): 42.0; anim3.mul body 371 (not listed, median 1624 px): 54.5; anim4.mul body 667 (equipment, median 144 px): 62.9 |
| 0 | ASTC 6 x 6 | 103 | 249.9 | 525.6 | 0.723 | 75.28 | 58.12 | 43.93 | 33 of 103 | 29.22 | 5.518 | 0.000 % | 56.02 % | 0.812 | 3.478 | 200 | 0.000 | AnimationFrame4.uop body 831 (NPC small, median 525 px): 43.9; anim3.mul body 371 (not listed, median 1624 px): 47.4; anim4.mul body 667 (equipment, median 144 px): 49.6 |
| 0 | ASTC 8 x 8 | 103 | 249.9 | 313.5 | 0.431 | 59.86 | 35.63 | 1.50 | 10 of 103 | 24.89 | 9.181 | 0.000 % | 66.61 % | 1.295 | 5.587 | 216 | 0.000 | anim3.mul body 371 (not listed, median 1624 px): 1.5; anim4.mul body 515 (equipment, median 1504 px): 17.2; AnimationFrame4.uop body 831 (NPC small, median 525 px): 23.4 |
| 2500 | BC1 (`dds`, High) | 33 | 207.7 | 523.5 | 0.720 | 81.77 | 78.52 | 75.52 | 28 of 33 | 30.87 | 4.606 | 0.000 % | 48.38 % | 0.649 | 2.838 | 160 | 0.000 | anim2.mul body 16 (NPC medium, median 2964 px): 75.5; anim4.mul body 202 (NPC medium, median 5016 px): 78.5; anim.mul body 24 (NPC medium, median 2730 px): 78.6 |
| 2500 | ASTC 6 x 6 | 33 | 207.7 | 536.8 | 0.739 | 74.87 | 67.93 | 61.87 | 7 of 33 | 30.42 | 4.857 | 0.000 % | 53.50 % | 0.727 | 3.082 | 200 | 0.000 | anim2.mul body 16 (NPC medium, median 2964 px): 61.9; AnimationFrame4.uop body 1434 (NPC large, median 21952 px): 67.9; anim2.mul body 49 (NPC medium, median 6975 px): 69.1 |
| 2500 | ASTC 8 x 8 | 33 | 207.7 | 370.8 | 0.510 | 55.60 | 38.33 | 26.94 | 0 of 33 | 26.18 | 7.953 | 0.000 % | 63.92 % | 1.133 | 4.854 | 216 | 0.000 | anim2.mul body 16 (NPC medium, median 2964 px): 26.9; anim2.mul body 49 (NPC medium, median 6975 px): 38.3; AnimationFrame2.uop body 432 (NPC medium, median 6831 px): 41.2 |
| 10000 | BC1 (`dds`, High) | 9 | 150.2 | 580.5 | 0.799 | 81.60 | 80.64 | 80.64 | 9 of 9 | 31.11 | 4.487 | 0.000 % | 46.40 % | 0.619 | 2.678 | 96 | 0.000 | AnimationFrame4.uop body 1434 (NPC large, median 21952 px): 80.6; AnimationFrame2.uop body 1484 (NPC large, median 12750 px): 81.1; anim5.mul body 198 (NPC large, median 79305 px): 81.2 |
| 10000 | ASTC 6 x 6 | 9 | 150.2 | 587.3 | 0.808 | 74.46 | 67.93 | 67.93 | 2 of 9 | 30.93 | 4.634 | 0.000 % | 53.13 % | 0.701 | 2.915 | 200 | 0.000 | AnimationFrame4.uop body 1434 (NPC large, median 21952 px): 67.9; AnimationFrame2.uop body 1484 (NPC large, median 12750 px): 70.1; anim5.mul body 101 (equipment, median 16652 px): 73.7 |
| 10000 | ASTC 8 x 8 | 9 | 150.2 | 474.5 | 0.653 | 57.88 | 42.82 | 42.82 | 0 of 9 | 26.65 | 7.581 | 0.000 % | 63.68 % | 1.087 | 4.589 | 216 | 0.000 | AnimationFrame4.uop body 1434 (NPC large, median 21952 px): 42.8; AnimationFrame2.uop body 1484 (NPC large, median 12750 px): 49.2; anim5.mul body 101 (equipment, median 16652 px): 52.1 |
| 40000 | BC1 (`dds`, High) | 2 | 93.8 | 638.2 | 0.878 | 81.24 | 81.24 | 81.24 | 2 of 2 | 31.18 | 4.245 | 0.000 % | 41.87 % | 0.563 | 2.385 | 96 | 0.000 | anim5.mul body 198 (NPC large, median 79305 px): 81.2; AnimationFrame3.uop body 1248 (NPC large, median 76473 px): 82.9 |
| 40000 | ASTC 6 x 6 | 2 | 93.8 | 641.4 | 0.883 | 74.37 | 74.37 | 74.37 | 0 of 2 | 31.28 | 4.382 | 0.000 % | 50.30 % | 0.647 | 2.575 | 184 | 0.000 | anim5.mul body 198 (NPC large, median 79305 px): 74.4; AnimationFrame3.uop body 1248 (NPC large, median 76473 px): 77.9 |
| 40000 | ASTC 8 x 8 | 2 | 93.8 | 573.0 | 0.788 | 57.88 | 57.88 | 57.88 | 0 of 2 | 26.99 | 7.176 | 0.000 % | 60.91 % | 0.999 | 4.082 | 216 | 0.000 | anim5.mul body 198 (NPC large, median 79305 px): 57.9; AnimationFrame3.uop body 1248 (NPC large, median 76473 px): 64.2 |

#### Enhanced: the mix on every 10th body; scores and hue are the block-coded bodies only

| above | format | bodies | Mpx coded | disk MB | x all lossless | SSIMULACRA2 median | p5 | min | at 80 or better | RGB PSNR dB | channel error | painted pixels gone | hue key changed | key moved | hued error | hued worst | edge levels, mean | worst bodies |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 0 | BC1 (`dds`, High) | 93 | 479.9 | 911.2 | 0.441 | 74.90 | 51.11 | -46.34 | 17 of 93 | 30.85 | 4.750 | 11.571 % | 88.51 % | 5.165 | 2.012 | 193 | 3.488 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 533 (equipment, median 527 px): 20.5 |
| 0 | BC7 (repo as called) | 93 | 479.9 | 1890.4 | 0.916 | 60.36 | 45.10 | 37.36 | 10 of 93 | 26.49 | 6.262 | 0.081 % | 84.81 % | 6.801 | 2.676 | 240 | 1.261 | EC body 1481 (equipment, median 506 px): 37.4; EC body 520 (equipment, median 1073 px): 41.1; EC body 11 (NPC large, median 13392 px): 41.4 |
| 0 | ASTC 6 x 6 | 93 | 479.9 | 1065.0 | 0.516 | 73.08 | 56.49 | 39.20 | 16 of 93 | 28.22 | 6.088 | 0.406 % | 87.66 % | 6.512 | 2.548 | 206 | 0.970 | EC body 1481 (equipment, median 506 px): 39.2; EC body 868 (equipment, median 391 px): 46.7; EC body 463 (equipment, median 306 px): 52.0 |
| 0 | ASTC 8 x 8 | 93 | 479.9 | 649.7 | 0.315 | 57.59 | 41.09 | 28.84 | 9 of 93 | 24.69 | 9.271 | 0.531 % | 90.88 % | 9.993 | 3.927 | 240 | 1.489 | EC body 1481 (equipment, median 506 px): 28.8; EC body 520 (equipment, median 1073 px): 35.4; EC body 868 (equipment, median 391 px): 37.2 |
| 0 | BC7 (fix A + colour step) | 93 | 479.9 | 1970.0 | 0.954 | 77.20 | 64.19 | 52.17 | 32 of 93 | 31.39 | 3.837 | 0.472 % | 83.28 % | 4.106 | 1.595 | 220 | 0.514 | EC body 1530 (equipment, median 680 px): 52.2; EC body 1481 (equipment, median 506 px): 52.6; EC body 868 (equipment, median 391 px): 56.2 |
| 0 | BC7 (fix C) | 93 | 479.9 | 1873.0 | 0.907 | 85.74 | 63.02 | 51.11 | 73 of 93 | 32.37 | 3.625 | 0.531 % | 83.36 % | 3.865 | 1.497 | 221 | 0.543 | EC body 1481 (equipment, median 506 px): 51.1; EC body 1530 (equipment, median 680 px): 52.8; EC body 868 (equipment, median 391 px): 53.9 |
| 0 | BC7 (fix C + colour step) | 93 | 479.9 | 1963.9 | 0.951 | 87.22 | 68.22 | 54.25 | 76 of 93 | 33.71 | 3.192 | 0.443 % | 82.71 % | 3.405 | 1.315 | 220 | 0.453 | EC body 1530 (equipment, median 680 px): 54.3; EC body 1481 (equipment, median 506 px): 54.6; EC body 868 (equipment, median 391 px): 58.4 |
| 0 | BC7 (fix C, RDO 1) | 93 | 479.9 | 1697.9 | 0.823 | 81.23 | 61.35 | 49.55 | 59 of 93 | 31.12 | 4.438 | 0.735 % | 85.50 % | 4.654 | 1.804 | 232 | 0.597 | EC body 1481 (equipment, median 506 px): 49.6; EC body 1530 (equipment, median 680 px): 51.0; EC body 868 (equipment, median 391 px): 52.4 |
| 0 | BC7 (fix C + colour, RDO 1) | 93 | 479.9 | 1762.5 | 0.854 | 83.00 | 66.06 | 52.71 | 66 of 93 | 32.02 | 4.084 | 0.597 % | 85.17 % | 4.286 | 1.661 | 220 | 0.507 | EC body 1530 (equipment, median 680 px): 52.7; EC body 1481 (equipment, median 506 px): 53.9; EC body 868 (equipment, median 391 px): 57.1 |
| 2500 | BC1 (`dds`, High) | 38 | 404.9 | 1165.0 | 0.564 | 73.24 | -13.58 | -46.34 | 1 of 38 | 31.12 | 4.592 | 10.346 % | 87.94 % | 5.046 | 1.964 | 193 | 3.375 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 2500 | BC7 (repo as called) | 38 | 404.9 | 1942.0 | 0.941 | 58.44 | 42.94 | 41.43 | 0 of 38 | 27.36 | 5.501 | 0.073 % | 83.89 % | 6.050 | 2.375 | 240 | 1.081 | EC body 11 (NPC large, median 13392 px): 41.4; EC body 1248 (NPC large, median 78260 px): 42.9; EC body 830 (NPC large, median 11495 px): 45.1 |
| 2500 | ASTC 6 x 6 | 38 | 404.9 | 1268.2 | 0.614 | 73.08 | 65.76 | 63.85 | 2 of 38 | 29.06 | 5.517 | 0.313 % | 86.91 % | 5.961 | 2.328 | 206 | 0.847 | EC body 830 (NPC large, median 11495 px): 63.9; EC body 675 (equipment, median 2664 px): 65.8; EC body 67 (NPC medium, median 4050 px): 65.8 |
| 2500 | ASTC 8 x 8 | 38 | 404.9 | 935.7 | 0.453 | 56.60 | 45.74 | 44.06 | 0 of 38 | 25.53 | 8.376 | 0.419 % | 90.15 % | 9.154 | 3.593 | 240 | 1.302 | EC body 830 (NPC large, median 11495 px): 44.1; EC body 1255 (NPC medium, median 7056 px): 45.7; EC body 307 (NPC medium, median 5548 px): 46.6 |
| 2500 | BC1, the rest BC7 (repo as called) | 93 | 479.9 | 1113.3 | 0.539 | 70.21 | 41.07 | -46.34 | 11 of 93 | 28.49 | 5.595 | 8.583 % | 88.18 % | 6.058 | 2.371 | 240 | 2.963 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1481 (equipment, median 506 px): 37.4 |
| 2500 | BC1, the rest BC7 (repo) + colour step | 93 | 479.9 | 1147.0 | 0.556 | 56.87 | -3.18 | -46.34 | 5 of 93 | 28.35 | 5.374 | 9.506 % | 87.91 % | 5.823 | 2.274 | 240 | 3.302 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 815 (equipment, median 1054 px): -19.0; EC body 1041 (equipment, median 1540 px): -13.8 |
| 2500 | BC7 (fix A + colour step) | 38 | 404.9 | 1992.3 | 0.965 | 76.94 | 72.03 | 71.32 | 6 of 38 | 32.04 | 3.550 | 0.357 % | 82.61 % | 3.829 | 1.483 | 190 | 0.463 | EC body 830 (NPC large, median 11495 px): 71.3; EC body 1255 (NPC medium, median 7056 px): 72.0; EC body 1407 (NPC medium, median 9576 px): 74.1 |
| 2500 | BC7 (fix C) | 38 | 404.9 | 1931.1 | 0.936 | 86.18 | 78.07 | 77.62 | 36 of 38 | 33.08 | 3.335 | 0.405 % | 82.71 % | 3.578 | 1.382 | 221 | 0.478 | EC body 1407 (NPC medium, median 9576 px): 77.6; EC body 1248 (NPC large, median 78260 px): 78.1; EC body 307 (NPC medium, median 5548 px): 80.6 |
| 2500 | BC7 (fix C + colour step) | 38 | 404.9 | 1988.1 | 0.963 | 87.40 | 82.26 | 78.66 | 37 of 38 | 34.20 | 3.002 | 0.334 % | 82.13 % | 3.210 | 1.236 | 174 | 0.402 | EC body 1407 (NPC medium, median 9576 px): 78.7; EC body 1255 (NPC medium, median 7056 px): 82.3; EC body 307 (NPC medium, median 5548 px): 82.3 |
| 2500 | BC7 (fix C, RDO 1) | 38 | 404.9 | 1805.4 | 0.875 | 81.40 | 76.77 | 76.28 | 28 of 38 | 31.71 | 4.126 | 0.560 % | 84.83 % | 4.355 | 1.683 | 232 | 0.525 | EC body 1248 (NPC large, median 78260 px): 76.3; EC body 1407 (NPC medium, median 9576 px): 76.8; EC body 1255 (NPC medium, median 7056 px): 77.0 |
| 2500 | BC7 (fix C + colour, RDO 1) | 38 | 404.9 | 1845.4 | 0.894 | 82.39 | 78.62 | 77.76 | 31 of 38 | 32.44 | 3.859 | 0.452 % | 84.53 % | 4.065 | 1.570 | 174 | 0.448 | EC body 1407 (NPC medium, median 9576 px): 77.8; EC body 1255 (NPC medium, median 7056 px): 78.6; EC body 830 (NPC large, median 11495 px): 78.9 |
| 2500 | BC1, the rest BC7 (fix A + colour step) | 93 | 479.9 | 1142.7 | 0.554 | 75.60 | 52.58 | -46.34 | 27 of 93 | 30.67 | 4.710 | 8.739 % | 87.67 % | 5.119 | 1.995 | 220 | 2.675 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 2500 | BC1, the rest BC7 (fix C) | 93 | 479.9 | 1106.9 | 0.536 | 77.19 | 52.79 | -46.34 | 38 of 93 | 30.89 | 4.673 | 8.758 % | 87.66 % | 5.082 | 1.980 | 194 | 2.692 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 2500 | BC1, the rest BC7 (fix C + colour step) | 93 | 479.9 | 1140.8 | 0.553 | 78.03 | 54.58 | -46.34 | 40 of 93 | 31.25 | 4.502 | 8.729 % | 87.48 % | 4.913 | 1.913 | 220 | 2.659 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 2500 | BC1, the rest BC7 (fix C, RDO 1) | 93 | 479.9 | 1057.4 | 0.512 | 76.06 | 51.00 | -46.34 | 32 of 93 | 30.65 | 4.845 | 8.835 % | 88.08 % | 5.243 | 2.043 | 193 | 2.712 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 2500 | BC1, the rest BC7 (fix C + colour, RDO 1) | 93 | 479.9 | 1082.0 | 0.524 | 77.02 | 53.92 | -46.34 | 36 of 93 | 30.98 | 4.701 | 8.785 % | 88.00 % | 5.103 | 1.989 | 220 | 2.679 | EC body 16 (NPC medium, median 8979 px): -46.3; EC body 777 (NPC medium, median 3740 px): -13.6; EC body 1245 (NPC medium, median 4554 px): 40.5 |
| 10000 | BC1 (`dds`, High) | 13 | 279.7 | 1501.9 | 0.728 | 72.37 | 55.03 | 55.03 | 1 of 13 | 31.57 | 4.380 | 6.816 % | 88.21 % | 4.916 | 1.903 | 193 | 3.417 | EC body 11 (NPC large, median 13392 px): 55.0; EC body 1248 (NPC large, median 78260 px): 56.9; EC body 315 (NPC large, median 17316 px): 68.0 |
| 10000 | BC7 (repo as called) | 13 | 279.7 | 1992.1 | 0.965 | 57.47 | 41.43 | 41.43 | 0 of 13 | 27.80 | 5.193 | 0.099 % | 86.67 % | 5.853 | 2.291 | 240 | 1.177 | EC body 11 (NPC large, median 13392 px): 41.4; EC body 1248 (NPC large, median 78260 px): 42.9; EC body 830 (NPC large, median 11495 px): 45.1 |
| 10000 | ASTC 6 x 6 | 13 | 279.7 | 1554.7 | 0.753 | 75.05 | 63.85 | 63.85 | 1 of 13 | 29.61 | 5.263 | 0.305 % | 89.61 % | 5.816 | 2.265 | 203 | 0.968 | EC body 830 (NPC large, median 11495 px): 63.9; EC body 432 (NPC large, median 12250 px): 70.1; EC body 1419 (NPC large, median 32928 px): 71.8 |
| 10000 | ASTC 8 x 8 | 13 | 279.7 | 1344.1 | 0.651 | 60.47 | 44.06 | 44.06 | 0 of 13 | 26.06 | 7.948 | 0.406 % | 92.97 % | 8.919 | 3.494 | 240 | 1.484 | EC body 830 (NPC large, median 11495 px): 44.1; EC body 432 (NPC large, median 12250 px): 54.2; EC body 1419 (NPC large, median 32928 px): 55.4 |
| 10000 | BC1, the rest BC7 (repo as called) | 93 | 479.9 | 1400.2 | 0.678 | 63.36 | 46.69 | 37.36 | 11 of 93 | 27.49 | 5.893 | 3.558 % | 85.53 % | 6.352 | 2.490 | 240 | 2.025 | EC body 1481 (equipment, median 506 px): 37.4; EC body 520 (equipment, median 1073 px): 41.1; EC body 1530 (equipment, median 680 px): 45.6 |
| 10000 | BC1, the rest BC7 (repo) + colour step | 93 | 479.9 | 1461.8 | 0.708 | 34.62 | 4.01 | -19.00 | 5 of 93 | 27.38 | 5.487 | 5.288 % | 85.03 % | 5.908 | 2.307 | 240 | 2.668 | EC body 815 (equipment, median 1054 px): -19.0; EC body 1041 (equipment, median 1540 px): -13.8; EC body 90 (NPC small, median 735 px): -3.2 |
| 10000 | BC7 (fix A + colour step) | 13 | 279.7 | 2017.6 | 0.977 | 76.75 | 71.32 | 71.32 | 1 of 13 | 32.55 | 3.442 | 0.315 % | 85.52 % | 3.790 | 1.461 | 174 | 0.523 | EC body 830 (NPC large, median 11495 px): 71.3; EC body 243 (NPC large, median 26085 px): 74.4; EC body 315 (NPC large, median 17316 px): 75.5 |
| 10000 | BC7 (fix C) | 13 | 279.7 | 1985.0 | 0.962 | 86.18 | 78.07 | 78.07 | 12 of 13 | 33.70 | 3.193 | 0.362 % | 85.52 % | 3.475 | 1.334 | 221 | 0.552 | EC body 1248 (NPC large, median 78260 px): 78.1; EC body 830 (NPC large, median 11495 px): 82.0; EC body 1419 (NPC large, median 32928 px): 83.4 |
| 10000 | BC7 (fix C + colour step) | 13 | 279.7 | 2015.4 | 0.976 | 87.40 | 83.11 | 83.11 | 13 of 13 | 34.66 | 2.938 | 0.296 % | 85.07 % | 3.183 | 1.217 | 174 | 0.474 | EC body 830 (NPC large, median 11495 px): 83.1; EC body 1248 (NPC large, median 78260 px): 83.5; EC body 1419 (NPC large, median 32928 px): 83.8 |
| 10000 | BC7 (fix C, RDO 1) | 13 | 279.7 | 1902.6 | 0.922 | 81.23 | 76.28 | 76.28 | 10 of 13 | 32.11 | 4.014 | 0.497 % | 87.67 % | 4.304 | 1.655 | 232 | 0.599 | EC body 1248 (NPC large, median 78260 px): 76.3; EC body 830 (NPC large, median 11495 px): 77.2; EC body 1419 (NPC large, median 32928 px): 79.2 |
| 10000 | BC7 (fix C + colour, RDO 1) | 13 | 279.7 | 1923.1 | 0.932 | 82.36 | 78.88 | 78.88 | 10 of 13 | 32.70 | 3.815 | 0.405 % | 87.46 % | 4.086 | 1.569 | 174 | 0.521 | EC body 830 (NPC large, median 11495 px): 78.9; EC body 1248 (NPC large, median 78260 px): 79.6; EC body 1419 (NPC large, median 32928 px): 79.8 |
| 10000 | BC1, the rest BC7 (fix A + colour step) | 93 | 479.9 | 1454.4 | 0.705 | 76.97 | 56.87 | 52.17 | 32 of 93 | 30.95 | 4.321 | 3.837 % | 84.59 % | 4.702 | 1.829 | 220 | 1.501 | EC body 1530 (equipment, median 680 px): 52.2; EC body 1481 (equipment, median 506 px): 52.6; EC body 11 (NPC large, median 13392 px): 55.0 |
| 10000 | BC1, the rest BC7 (fix C) | 93 | 479.9 | 1390.0 | 0.673 | 84.65 | 56.42 | 51.11 | 62 of 93 | 31.43 | 4.235 | 3.872 % | 84.67 % | 4.627 | 1.799 | 194 | 1.520 | EC body 1481 (equipment, median 506 px): 51.1; EC body 1530 (equipment, median 680 px): 52.8; EC body 868 (equipment, median 391 px): 53.9 |
| 10000 | BC1, the rest BC7 (fix C + colour step) | 93 | 479.9 | 1450.5 | 0.703 | 86.37 | 58.37 | 54.25 | 64 of 93 | 32.17 | 3.924 | 3.818 % | 84.25 % | 4.312 | 1.675 | 220 | 1.456 | EC body 1530 (equipment, median 680 px): 54.3; EC body 1481 (equipment, median 506 px): 54.6; EC body 11 (NPC large, median 13392 px): 55.0 |
| 10000 | BC1, the rest BC7 (fix C, RDO 1) | 93 | 479.9 | 1297.3 | 0.628 | 80.24 | 56.39 | 49.55 | 50 of 93 | 30.87 | 4.636 | 4.006 % | 85.70 % | 4.984 | 1.939 | 193 | 1.558 | EC body 1481 (equipment, median 506 px): 49.6; EC body 1530 (equipment, median 680 px): 51.0; EC body 868 (equipment, median 391 px): 52.4 |
| 10000 | BC1, the rest BC7 (fix C + colour, RDO 1) | 93 | 479.9 | 1341.3 | 0.650 | 82.34 | 57.08 | 52.71 | 57 of 93 | 31.48 | 4.377 | 3.915 % | 85.48 % | 4.724 | 1.838 | 220 | 1.494 | EC body 1530 (equipment, median 680 px): 52.7; EC body 1481 (equipment, median 506 px): 53.9; EC body 11 (NPC large, median 13392 px): 55.0 |
| 40000 | BC1 (`dds`, High) | 2 | 113.5 | 1866.2 | 0.904 | 56.87 | 56.87 | 56.87 | 0 of 2 | 32.11 | 3.824 | 4.318 % | 82.57 % | 4.076 | 1.529 | 193 | 3.895 | EC body 1248 (NPC large, median 78260 px): 56.9; EC body 106 (NPC large, median 46512 px): 79.0 |
| 40000 | BC7 (repo as called) | 2 | 113.5 | 2048.5 | 0.992 | 42.94 | 42.94 | 42.94 | 0 of 2 | 29.32 | 4.014 | 0.175 % | 83.83 % | 4.227 | 1.607 | 240 | 1.117 | EC body 1248 (NPC large, median 78260 px): 42.9; EC body 106 (NPC large, median 46512 px): 60.4 |
| 40000 | ASTC 6 x 6 | 2 | 113.5 | 1885.7 | 0.913 | 77.08 | 77.08 | 77.08 | 1 of 2 | 31.02 | 4.217 | 0.217 % | 84.74 % | 4.462 | 1.683 | 197 | 0.921 | EC body 1248 (NPC large, median 78260 px): 77.1; EC body 106 (NPC large, median 46512 px): 81.8 |
| 40000 | ASTC 8 x 8 | 2 | 113.5 | 1802.0 | 0.873 | 63.78 | 63.78 | 63.78 | 0 of 2 | 27.32 | 6.480 | 0.283 % | 89.39 % | 6.868 | 2.615 | 240 | 1.463 | EC body 1248 (NPC large, median 78260 px): 63.8; EC body 106 (NPC large, median 46512 px): 74.3 |
| 40000 | BC1, the rest BC7 (repo as called) | 93 | 479.9 | 1708.1 | 0.827 | 60.65 | 45.60 | 37.36 | 10 of 93 | 26.70 | 6.242 | 0.979 % | 84.54 % | 6.792 | 2.669 | 240 | 1.497 | EC body 1481 (equipment, median 506 px): 37.4; EC body 520 (equipment, median 1073 px): 41.1; EC body 11 (NPC large, median 13392 px): 41.4 |
| 40000 | BC1, the rest BC7 (repo) + colour step | 93 | 479.9 | 1788.8 | 0.867 | 30.07 | 1.81 | -19.00 | 4 of 93 | 26.67 | 5.669 | 3.422 % | 83.80 % | 6.132 | 2.397 | 240 | 2.468 | EC body 815 (equipment, median 1054 px): -19.0; EC body 1041 (equipment, median 1540 px): -13.8; EC body 90 (NPC small, median 735 px): -3.2 |
| 40000 | BC7 (fix A + colour step) | 2 | 113.5 | 2056.9 | 0.996 | 78.79 | 78.79 | 78.79 | 1 of 2 | 34.30 | 2.766 | 0.194 % | 82.69 % | 2.928 | 1.087 | 171 | 0.545 | EC body 1248 (NPC large, median 78260 px): 78.8; EC body 106 (NPC large, median 46512 px): 82.2 |
| 40000 | BC7 (fix C) | 2 | 113.5 | 2048.0 | 0.992 | 78.07 | 78.07 | 78.07 | 1 of 2 | 34.60 | 2.671 | 0.225 % | 82.54 % | 2.823 | 1.045 | 221 | 0.548 | EC body 1248 (NPC large, median 78260 px): 78.1; EC body 106 (NPC large, median 46512 px): 89.3 |
| 40000 | BC7 (fix C + colour step) | 2 | 113.5 | 2056.9 | 0.996 | 83.48 | 83.48 | 83.48 | 2 of 2 | 35.64 | 2.509 | 0.187 % | 82.23 % | 2.652 | 0.976 | 154 | 0.499 | EC body 1248 (NPC large, median 78260 px): 83.5; EC body 106 (NPC large, median 46512 px): 90.5 |
| 40000 | BC7 (fix C, RDO 1) | 2 | 113.5 | 2018.5 | 0.978 | 76.28 | 76.28 | 76.28 | 1 of 2 | 33.12 | 3.334 | 0.289 % | 85.05 % | 3.471 | 1.277 | 232 | 0.599 | EC body 1248 (NPC large, median 78260 px): 76.3; EC body 106 (NPC large, median 46512 px): 85.7 |
| 40000 | BC7 (fix C + colour, RDO 1) | 2 | 113.5 | 2024.6 | 0.981 | 79.64 | 79.64 | 79.64 | 1 of 2 | 33.78 | 3.208 | 0.243 % | 84.95 % | 3.334 | 1.224 | 170 | 0.548 | EC body 1248 (NPC large, median 78260 px): 79.6; EC body 106 (NPC large, median 46512 px): 86.0 |
| 40000 | BC1, the rest BC7 (fix A + colour step) | 93 | 479.9 | 1779.3 | 0.862 | 77.12 | 64.05 | 52.17 | 31 of 93 | 31.07 | 4.069 | 1.366 % | 83.26 % | 4.367 | 1.696 | 220 | 0.799 | EC body 1530 (equipment, median 680 px): 52.2; EC body 1481 (equipment, median 506 px): 52.6; EC body 868 (equipment, median 391 px): 56.2 |
| 40000 | BC1, the rest BC7 (fix C) | 93 | 479.9 | 1691.2 | 0.819 | 85.58 | 56.87 | 51.11 | 72 of 93 | 31.95 | 3.876 | 1.418 % | 83.37 % | 4.147 | 1.607 | 194 | 0.827 | EC body 1481 (equipment, median 506 px): 51.1; EC body 1530 (equipment, median 680 px): 52.8; EC body 868 (equipment, median 391 px): 53.9 |
| 40000 | BC1, the rest BC7 (fix C + colour step) | 93 | 479.9 | 1773.2 | 0.859 | 87.07 | 66.39 | 54.25 | 74 of 93 | 33.02 | 3.475 | 1.339 % | 82.79 % | 3.722 | 1.439 | 220 | 0.742 | EC body 1530 (equipment, median 680 px): 54.3; EC body 1481 (equipment, median 506 px): 54.6; EC body 1248 (NPC large, median 78260 px): 56.9 |
| 40000 | BC1, the rest BC7 (fix C, RDO 1) | 93 | 479.9 | 1545.6 | 0.749 | 81.12 | 56.87 | 49.55 | 58 of 93 | 30.96 | 4.551 | 1.609 % | 84.96 % | 4.797 | 1.864 | 193 | 0.878 | EC body 1481 (equipment, median 506 px): 49.6; EC body 1530 (equipment, median 680 px): 51.0; EC body 868 (equipment, median 391 px): 52.4 |
| 40000 | BC1, the rest BC7 (fix C + colour, RDO 1) | 93 | 479.9 | 1604.1 | 0.777 | 82.77 | 62.39 | 52.71 | 65 of 93 | 31.73 | 4.222 | 1.480 % | 84.66 % | 4.457 | 1.731 | 220 | 0.791 | EC body 1530 (equipment, median 680 px): 52.7; EC body 1481 (equipment, median 506 px): 53.9; EC body 1248 (NPC large, median 78260 px): 56.9 |

#### Classic: resident MB under the mix: pixels padded to each format's block, no atlas waste, no tables, no hue masks

| layout | whole set, every direction (x 9.681) | creatures, five directions | creatures, faced only | dressed NPCs, five directions | dressed NPCs, faced only |
|---|---:|---:|---:|---:|---:|
| all lossless | 2419.0 | 132.7 | 25.8 | 9.7 | 2.0 |
| above 0: BC1 | 1255.5 | 67.8 | 13.2 | 5.4 | 1.1 |
| above 0: BC7 | 2511.0 | 135.6 | 26.4 | 10.8 | 2.2 |
| above 0: ASTC 6 x 6 | 1146.1 | 61.2 | 11.9 | 5.2 | 1.1 |
| above 0: ASTC 8 x 8 | 660.3 | 34.9 | 6.8 | 3.1 | 0.6 |
| above 2500: BC1 | 1434.2 | 69.2 | 13.5 | 8.6 | 1.8 |
| above 2500: BC7 | 2460.5 | 135.4 | 26.3 | 9.9 | 2.0 |
| above 2500: ASTC 6 x 6 | 1333.5 | 62.6 | 12.2 | 8.5 | 1.7 |
| above 2500: ASTC 8 x 8 | 935.7 | 37.0 | 7.2 | 8.0 | 1.6 |
| above 10000: BC1 | 1702.4 | 81.7 | 16.0 | 9.7 | 2.0 |
| above 10000: BC7 | 2439.7 | 134.3 | 26.1 | 9.7 | 2.0 |
| above 10000: ASTC 6 x 6 | 1627.0 | 76.3 | 15.0 | 9.7 | 2.0 |
| above 10000: ASTC 8 x 8 | 1341.0 | 56.0 | 11.1 | 9.7 | 2.0 |
| above 40000: BC1 | 1969.9 | 105.5 | 20.7 | 9.7 | 2.0 |
| above 40000: BC7 | 2428.9 | 133.2 | 25.9 | 9.7 | 2.0 |
| above 40000: ASTC 6 x 6 | 1921.9 | 102.6 | 20.1 | 9.7 | 2.0 |
| above 40000: ASTC 8 x 8 | 1743.7 | 91.8 | 18.1 | 9.7 | 2.0 |

#### Classic: each class's resident MB and share

| view | format | equipment | NPC small | NPC medium | NPC large | not listed |
|---|---|---:|---:|---:|---:|---:|
| whole set, every direction (x 9.681) | lossless | 415.7, 17.2 % | 53.7, 2.2 % | 479.9, 19.8 % | 1445.8, 59.8 % | 23.9, 1.0 % |
| whole set, every direction (x 9.681) | BC1 | 231.1, 18.4 % | 29.0, 2.3 % | 249.0, 19.8 % | 733.2, 58.4 % | 13.3, 1.1 % |
| whole set, every direction (x 9.681) | BC7 | 462.1, 18.4 % | 57.9, 2.3 % | 498.1, 19.8 % | 1466.3, 58.4 % | 26.5, 1.1 % |
| whole set, every direction (x 9.681) | ASTC 6 x 6 | 221.2, 19.3 % | 27.0, 2.4 % | 227.1, 19.8 % | 658.1, 57.4 % | 12.6, 1.1 % |
| whole set, every direction (x 9.681) | ASTC 8 x 8 | 132.0, 20.0 % | 15.9, 2.4 % | 131.0, 19.8 % | 373.7, 56.6 % | 7.7, 1.2 % |
| creatures, five directions | lossless | 0.0, 0.0 % | 3.0, 2.2 % | 26.1, 19.7 % | 103.6, 78.1 % | 0.0, 0.0 % |
| creatures, five directions | BC1 | 0.0, 0.0 % | 1.6, 2.4 % | 13.6, 20.0 % | 52.6, 77.6 % | 0.0, 0.0 % |
| creatures, five directions | BC7 | 0.0, 0.0 % | 3.2, 2.4 % | 27.2, 20.0 % | 105.3, 77.6 % | 0.0, 0.0 % |
| creatures, five directions | ASTC 6 x 6 | 0.0, 0.0 % | 1.5, 2.5 % | 12.4, 20.3 % | 47.3, 77.3 % | 0.0, 0.0 % |
| creatures, five directions | ASTC 8 x 8 | 0.0, 0.0 % | 0.9, 2.5 % | 7.2, 20.5 % | 26.9, 77.0 % | 0.0, 0.0 % |
| creatures, faced only | lossless | 0.0, 0.0 % | 0.6, 2.3 % | 5.3, 20.6 % | 19.9, 77.1 % | 0.0, 0.0 % |
| creatures, faced only | BC1 | 0.0, 0.0 % | 0.3, 2.4 % | 2.8, 20.9 % | 10.1, 76.6 % | 0.0, 0.0 % |
| creatures, faced only | BC7 | 0.0, 0.0 % | 0.6, 2.4 % | 5.5, 20.9 % | 20.2, 76.6 % | 0.0, 0.0 % |
| creatures, faced only | ASTC 6 x 6 | 0.0, 0.0 % | 0.3, 2.5 % | 2.5, 21.2 % | 9.1, 76.3 % | 0.0, 0.0 % |
| creatures, faced only | ASTC 8 x 8 | 0.0, 0.0 % | 0.2, 2.6 % | 1.5, 21.4 % | 5.2, 76.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | lossless | 5.3, 54.4 % | 2.0, 20.9 % | 2.4, 24.7 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | BC1 | 3.0, 55.8 % | 1.1, 20.4 % | 1.3, 23.8 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | BC7 | 6.0, 55.8 % | 2.2, 20.4 % | 2.6, 23.8 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | ASTC 6 x 6 | 2.9, 56.7 % | 1.0, 20.1 % | 1.2, 23.2 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | ASTC 8 x 8 | 1.8, 57.3 % | 0.6, 19.7 % | 0.7, 23.0 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | lossless | 1.1, 54.7 % | 0.4, 20.5 % | 0.5, 24.9 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | BC1 | 0.6, 56.0 % | 0.2, 20.2 % | 0.3, 23.8 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | BC7 | 1.2, 56.0 % | 0.4, 20.2 % | 0.5, 23.8 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | ASTC 6 x 6 | 0.6, 56.9 % | 0.2, 20.0 % | 0.2, 23.0 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | ASTC 8 x 8 | 0.4, 57.2 % | 0.1, 19.8 % | 0.1, 23.0 % | 0.0, 0.0 % | 0.0, 0.0 % |

#### Enhanced: resident MB under the mix: pixels padded to each format's block, no atlas waste, no tables, no hue masks

| layout | whole set, every direction (x 10.245) | creatures, five directions | creatures, faced only | dressed NPCs, five directions | dressed NPCs, faced only |
|---|---:|---:|---:|---:|---:|
| all lossless | 9834.0 | 396.6 | 79.9 | 31.6 | 6.4 |
| above 0: BC1 | 2556.1 | 101.4 | 20.4 | 8.6 | 1.7 |
| above 0: BC7 | 5112.2 | 202.8 | 40.8 | 17.2 | 3.5 |
| above 0: ASTC 6 x 6 | 2331.3 | 91.5 | 18.4 | 8.0 | 1.6 |
| above 0: ASTC 8 x 8 | 1346.3 | 52.2 | 10.5 | 4.8 | 1.0 |
| above 2500: BC1 | 3665.1 | 106.4 | 21.4 | 21.3 | 4.3 |
| above 2500: BC7 | 5792.4 | 205.9 | 41.5 | 25.0 | 5.0 |
| above 2500: ASTC 6 x 6 | 3461.1 | 96.6 | 19.5 | 21.1 | 4.2 |
| above 2500: ASTC 8 x 8 | 2638.2 | 58.0 | 11.7 | 19.6 | 4.0 |
| above 2500: BC1, the rest BC7 | 2984.9 | 103.2 | 20.8 | 13.5 | 2.7 |
| above 10000: BC1 | 5559.9 | 171.5 | 34.3 | 31.6 | 6.4 |
| above 10000: BC7 | 7017.1 | 248.2 | 49.8 | 31.6 | 6.4 |
| above 10000: ASTC 6 x 6 | 5413.0 | 163.7 | 32.7 | 31.6 | 6.4 |
| above 10000: ASTC 8 x 8 | 4848.0 | 134.0 | 26.7 | 31.6 | 6.4 |
| above 10000: BC1, the rest BC7 | 3655.1 | 126.0 | 25.3 | 17.2 | 3.5 |
| above 40000: BC1 | 8096.5 | 305.8 | 61.4 | 31.6 | 6.4 |
| above 40000: BC7 | 8684.7 | 336.5 | 67.7 | 31.6 | 6.4 |
| above 40000: ASTC 6 x 6 | 8035.5 | 302.7 | 60.8 | 31.6 | 6.4 |
| above 40000: ASTC 8 x 8 | 7807.3 | 290.7 | 58.3 | 31.6 | 6.4 |
| above 40000: BC1, the rest BC7 | 4524.0 | 172.0 | 34.6 | 17.2 | 3.5 |

#### Enhanced: each class's resident MB and share

| view | format | equipment | NPC small | NPC medium | NPC large | not listed |
|---|---|---:|---:|---:|---:|---:|
| whole set, every direction (x 10.245) | lossless | 2386.0, 24.3 % | 86.1, 0.9 % | 1630.7, 16.6 % | 5731.2, 58.3 % | 0.0, 0.0 % |
| whole set, every direction (x 10.245) | BC1 | 652.6, 25.5 % | 23.1, 0.9 % | 423.3, 16.6 % | 1457.1, 57.0 % | 0.0, 0.0 % |
| whole set, every direction (x 10.245) | BC7 | 1305.2, 25.5 % | 46.2, 0.9 % | 846.7, 16.6 % | 2914.2, 57.0 % | 0.0, 0.0 % |
| whole set, every direction (x 10.245) | ASTC 6 x 6 | 613.7, 26.3 % | 21.5, 0.9 % | 385.9, 16.6 % | 1310.2, 56.2 % | 0.0, 0.0 % |
| whole set, every direction (x 10.245) | ASTC 8 x 8 | 366.0, 27.2 % | 12.7, 0.9 % | 222.4, 16.5 % | 745.2, 55.4 % | 0.0, 0.0 % |
| creatures, five directions | lossless | 0.0, 0.0 % | 6.8, 1.7 % | 87.9, 22.2 % | 301.8, 76.1 % | 0.0, 0.0 % |
| creatures, five directions | BC1 | 0.0, 0.0 % | 1.8, 1.8 % | 22.8, 22.5 % | 76.7, 75.7 % | 0.0, 0.0 % |
| creatures, five directions | BC7 | 0.0, 0.0 % | 3.7, 1.8 % | 45.6, 22.5 % | 153.5, 75.7 % | 0.0, 0.0 % |
| creatures, five directions | ASTC 6 x 6 | 0.0, 0.0 % | 1.7, 1.9 % | 20.8, 22.7 % | 69.0, 75.4 % | 0.0, 0.0 % |
| creatures, five directions | ASTC 8 x 8 | 0.0, 0.0 % | 1.0, 1.9 % | 12.0, 22.9 % | 39.2, 75.2 % | 0.0, 0.0 % |
| creatures, faced only | lossless | 0.0, 0.0 % | 1.4, 1.7 % | 17.3, 21.7 % | 61.2, 76.6 % | 0.0, 0.0 % |
| creatures, faced only | BC1 | 0.0, 0.0 % | 0.4, 1.8 % | 4.5, 22.0 % | 15.6, 76.2 % | 0.0, 0.0 % |
| creatures, faced only | BC7 | 0.0, 0.0 % | 0.7, 1.8 % | 9.0, 22.0 % | 31.1, 76.2 % | 0.0, 0.0 % |
| creatures, faced only | ASTC 6 x 6 | 0.0, 0.0 % | 0.3, 1.9 % | 4.1, 22.2 % | 14.0, 75.9 % | 0.0, 0.0 % |
| creatures, faced only | ASTC 8 x 8 | 0.0, 0.0 % | 0.2, 2.0 % | 2.4, 22.4 % | 8.0, 75.6 % | 0.0, 0.0 % |
| dressed NPCs, five directions | lossless | 17.0, 53.9 % | 5.5, 17.5 % | 9.0, 28.6 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | BC1 | 4.7, 55.0 % | 1.5, 17.5 % | 2.4, 27.5 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | BC7 | 9.4, 55.0 % | 3.0, 17.5 % | 4.7, 27.5 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | ASTC 6 x 6 | 4.5, 55.8 % | 1.3, 16.8 % | 2.2, 27.4 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, five directions | ASTC 8 x 8 | 2.7, 56.7 % | 0.8, 16.6 % | 1.3, 26.7 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | lossless | 3.5, 54.2 % | 1.1, 17.4 % | 1.8, 28.4 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | BC1 | 1.0, 55.4 % | 0.3, 17.3 % | 0.5, 27.3 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | BC7 | 1.9, 55.4 % | 0.6, 17.3 % | 0.9, 27.3 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | ASTC 6 x 6 | 0.9, 56.2 % | 0.3, 16.6 % | 0.4, 27.2 % | 0.0, 0.0 % | 0.0, 0.0 % |
| dressed NPCs, faced only | ASTC 8 x 8 | 0.6, 57.1 % | 0.2, 16.4 % | 0.3, 26.5 % | 0.0, 0.0 % | 0.0, 0.0 % |

#### Classic, creatures, 300 pairs, 132.7 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| all lossless | memory | five directions | 41.2 | 180.5 | 50.6 | yes | 9.4 |
| all lossless | memory | faced only | 8.2 | 36.0 | 10.4 | yes | 2.0 |
| above 0: BC1 | memory | five directions | 26.5 | 84.2 | 23.4 | yes | 3.9 |
| above 0: BC1 | memory | faced only | 5.3 | 16.5 | 4.8 | yes | 0.8 |
| above 0: ASTC 6 x 6 | memory | five directions | 27.1 | 73.5 | 20.4 | yes | 3.3 |
| above 0: ASTC 6 x 6 | memory | faced only | 5.4 | 14.5 | 4.1 | yes | 0.7 |
| above 0: ASTC 8 x 8 | memory | five directions | 15.9 | 49.4 | 13.7 | yes | 2.0 |
| above 0: ASTC 8 x 8 | memory | faced only | 3.2 | 9.8 | 2.8 | yes | 0.4 |
| above 2500: BC1 | memory | five directions | 26.8 | 86.8 | 24.2 | yes | 3.9 |
| above 2500: BC1 | memory | faced only | 5.3 | 17.3 | 4.9 | yes | 0.8 |
| above 2500: ASTC 6 x 6 | memory | five directions | 27.4 | 76.3 | 21.2 | yes | 3.3 |
| above 2500: ASTC 6 x 6 | memory | faced only | 5.4 | 15.1 | 4.3 | yes | 0.7 |
| above 2500: ASTC 8 x 8 | memory | five directions | 16.6 | 53.2 | 14.6 | yes | 2.0 |
| above 2500: ASTC 8 x 8 | memory | faced only | 3.3 | 10.6 | 3.0 | yes | 0.4 |
| above 10000: BC1 | memory | five directions | 29.7 | 107.9 | 29.5 | yes | 3.9 |
| above 10000: BC1 | memory | faced only | 5.9 | 21.5 | 6.0 | yes | 0.8 |
| above 10000: ASTC 6 x 6 | memory | five directions | 30.0 | 99.8 | 27.2 | yes | 3.3 |
| above 10000: ASTC 6 x 6 | memory | faced only | 6.0 | 19.9 | 5.6 | yes | 0.7 |
| above 10000: ASTC 8 x 8 | memory | five directions | 21.8 | 82.8 | 22.1 | yes | 2.1 |
| above 10000: ASTC 8 x 8 | memory | faced only | 4.4 | 16.7 | 4.6 | yes | 0.4 |
| above 40000: BC1 | memory | five directions | 35.9 | 143.5 | 38.2 | yes | 3.9 |
| above 40000: BC1 | memory | faced only | 7.1 | 28.4 | 7.7 | yes | 0.8 |
| above 40000: ASTC 6 x 6 | memory | five directions | 36.0 | 140.6 | 36.9 | yes | 3.3 |
| above 40000: ASTC 6 x 6 | memory | faced only | 7.2 | 27.8 | 7.5 | yes | 0.7 |
| above 40000: ASTC 8 x 8 | memory | five directions | 32.2 | 131.5 | 34.0 | yes | 2.7 |
| above 40000: ASTC 8 x 8 | memory | faced only | 6.4 | 26.2 | 6.9 | yes | 0.5 |

#### Classic, dressed NPCs, 300 pairs, 9.7 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| all lossless | memory | five directions | 3.6 | 22.8 | 5.9 | yes | 0.3 |
| all lossless | memory | faced only | 0.7 | 4.7 | 1.2 | yes | 0.1 |
| above 0: BC1 | memory | five directions | 3.0 | 15.4 | 4.0 | yes | 0.2 |
| above 0: BC1 | memory | faced only | 0.6 | 3.2 | 0.9 | yes | 0.0 |
| above 0: ASTC 6 x 6 | memory | five directions | 3.2 | 14.4 | 3.7 | yes | 0.1 |
| above 0: ASTC 6 x 6 | memory | faced only | 0.7 | 3.0 | 0.8 | yes | 0.0 |
| above 0: ASTC 8 x 8 | memory | five directions | 2.0 | 11.6 | 3.0 | yes | 0.1 |
| above 0: ASTC 8 x 8 | memory | faced only | 0.4 | 2.4 | 0.7 | yes | 0.0 |
| above 2500: BC1 | memory | five directions | 3.4 | 20.6 | 5.3 | yes | 0.2 |
| above 2500: BC1 | memory | faced only | 0.7 | 4.3 | 1.1 | yes | 0.0 |
| above 2500: ASTC 6 x 6 | memory | five directions | 3.5 | 20.2 | 5.2 | yes | 0.2 |
| above 2500: ASTC 6 x 6 | memory | faced only | 0.7 | 4.2 | 1.1 | yes | 0.0 |
| above 2500: ASTC 8 x 8 | memory | five directions | 3.2 | 19.6 | 5.1 | yes | 0.2 |
| above 2500: ASTC 8 x 8 | memory | faced only | 0.7 | 4.2 | 1.1 | yes | 0.0 |
| above 10000: BC1 | memory | five directions | 3.5 | 22.8 | 5.9 | yes | 0.3 |
| above 10000: BC1 | memory | faced only | 0.7 | 4.8 | 1.2 | yes | 0.1 |
| above 10000: ASTC 6 x 6 | memory | five directions | 3.5 | 22.7 | 5.9 | yes | 0.3 |
| above 10000: ASTC 6 x 6 | memory | faced only | 0.7 | 4.7 | 1.3 | yes | 0.1 |
| above 10000: ASTC 8 x 8 | memory | five directions | 3.5 | 22.7 | 5.9 | yes | 0.3 |
| above 10000: ASTC 8 x 8 | memory | faced only | 0.7 | 4.7 | 1.3 | yes | 0.1 |
| above 40000: BC1 | memory | five directions | 3.6 | 22.8 | 5.9 | yes | 0.3 |
| above 40000: BC1 | memory | faced only | 0.7 | 4.8 | 1.3 | yes | 0.1 |
| above 40000: ASTC 6 x 6 | memory | five directions | 3.6 | 22.8 | 5.8 | yes | 0.3 |
| above 40000: ASTC 6 x 6 | memory | faced only | 0.7 | 4.8 | 1.3 | yes | 0.1 |
| above 40000: ASTC 8 x 8 | memory | five directions | 3.6 | 22.8 | 5.8 | yes | 0.3 |
| above 40000: ASTC 8 x 8 | memory | faced only | 0.7 | 4.8 | 1.3 | yes | 0.1 |

#### Enhanced, creatures, 300 pairs, 198.3 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| all lossless | memory | five directions | 78.5 | 401.4 | 102.0 | yes | 18.3 |
| all lossless | memory | faced only | 15.7 | 82.0 | 20.8 | yes | 3.1 |
| above 0: BC1 | memory | five directions | 35.5 | 114.1 | 29.0 | yes | 4.9 |
| above 0: BC1 | memory | faced only | 7.1 | 22.7 | 5.9 | yes | 0.9 |
| above 0: BC7 | memory | five directions | 73.0 | 197.8 | 50.2 | yes | 8.8 |
| above 0: BC7 | memory | faced only | 14.6 | 40.0 | 10.2 | yes | 1.6 |
| above 0: ASTC 6 x 6 | memory | five directions | 39.6 | 102.0 | 26.0 | yes | 4.1 |
| above 0: ASTC 6 x 6 | memory | faced only | 7.9 | 20.4 | 5.3 | yes | 0.7 |
| above 0: ASTC 8 x 8 | memory | five directions | 23.8 | 67.1 | 17.0 | yes | 2.5 |
| above 0: ASTC 8 x 8 | memory | faced only | 4.8 | 13.3 | 3.4 | yes | 0.5 |
| above 2500: BC1 | memory | five directions | 36.5 | 121.2 | 30.9 | yes | 4.9 |
| above 2500: BC1 | memory | faced only | 7.3 | 24.3 | 6.2 | yes | 0.9 |
| above 2500: BC7 | memory | five directions | 73.3 | 203.1 | 51.7 | yes | 8.9 |
| above 2500: BC7 | memory | faced only | 14.7 | 40.7 | 10.4 | yes | 1.6 |
| above 2500: ASTC 6 x 6 | memory | five directions | 40.5 | 109.9 | 27.8 | yes | 4.0 |
| above 2500: ASTC 6 x 6 | memory | faced only | 8.1 | 21.9 | 5.6 | yes | 0.7 |
| above 2500: ASTC 8 x 8 | memory | five directions | 25.0 | 75.7 | 19.1 | yes | 2.5 |
| above 2500: ASTC 8 x 8 | memory | faced only | 5.0 | 15.1 | 3.8 | yes | 0.4 |
| above 2500: BC1, the rest BC7 | memory | five directions | 36.3 | 117.0 | 29.7 | yes | 4.9 |
| above 2500: BC1, the rest BC7 | memory | faced only | 7.3 | 23.3 | 6.0 | yes | 0.9 |
| above 10000: BC1 | memory | five directions | 48.5 | 195.8 | 49.4 | yes | 4.9 |
| above 10000: BC1 | memory | faced only | 9.7 | 39.4 | 9.9 | yes | 0.9 |
| above 10000: BC7 | memory | five directions | 75.4 | 255.2 | 64.8 | yes | 8.9 |
| above 10000: BC7 | memory | faced only | 15.1 | 51.2 | 13.1 | yes | 1.7 |
| above 10000: ASTC 6 x 6 | memory | five directions | 51.1 | 187.7 | 47.4 | yes | 4.1 |
| above 10000: ASTC 6 x 6 | memory | faced only | 10.2 | 37.7 | 9.5 | yes | 0.7 |
| above 10000: ASTC 8 x 8 | memory | five directions | 39.7 | 163.2 | 41.1 | yes | 2.5 |
| above 10000: ASTC 8 x 8 | memory | faced only | 7.9 | 32.5 | 8.2 | yes | 0.4 |
| above 10000: BC1, the rest BC7 | memory | five directions | 46.2 | 139.2 | 35.1 | yes | 4.9 |
| above 10000: BC1, the rest BC7 | memory | faced only | 9.2 | 27.8 | 7.1 | yes | 0.9 |
| above 40000: BC1 | memory | five directions | 67.4 | 324.9 | 81.8 | yes | 5.5 |
| above 40000: BC1 | memory | faced only | 13.5 | 65.7 | 16.5 | yes | 1.2 |
| above 40000: BC7 | memory | five directions | 77.5 | 348.9 | 87.6 | yes | 8.8 |
| above 40000: BC7 | memory | faced only | 15.5 | 70.5 | 17.7 | yes | 1.6 |
| above 40000: ASTC 6 x 6 | memory | five directions | 68.0 | 321.7 | 81.0 | yes | 5.5 |
| above 40000: ASTC 6 x 6 | memory | faced only | 13.6 | 65.3 | 16.5 | yes | 1.2 |
| above 40000: ASTC 8 x 8 | memory | five directions | 63.6 | 312.9 | 78.9 | yes | 5.5 |
| above 40000: ASTC 8 x 8 | memory | faced only | 12.7 | 63.1 | 15.9 | yes | 1.2 |
| above 40000: BC1, the rest BC7 | memory | five directions | 62.9 | 176.4 | 44.2 | yes | 4.9 |
| above 40000: BC1, the rest BC7 | memory | faced only | 12.6 | 35.3 | 8.9 | yes | 0.9 |

#### Enhanced, dressed NPCs, 300 pairs, 15.8 Mpx in five directions (quiet rerun)

| row | to | load unit | read MB | 1 thread ms | 4 threads ms | within the line | worst job ms |
|---|---|---|---:|---:|---:|---|---:|
| all lossless | memory | five directions | 9.9 | 58.7 | 14.9 | yes | 0.7 |
| all lossless | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 0: BC1 | memory | five directions | 4.3 | 20.7 | 5.3 | yes | 0.2 |
| above 0: BC1 | memory | faced only | 0.9 | 4.3 | 1.2 | yes | 0.1 |
| above 0: BC7 | memory | five directions | 8.7 | 32.2 | 8.2 | yes | 0.4 |
| above 0: BC7 | memory | faced only | 1.8 | 6.6 | 1.7 | yes | 0.1 |
| above 0: ASTC 6 x 6 | memory | five directions | 5.1 | 18.0 | 4.7 | yes | 0.2 |
| above 0: ASTC 6 x 6 | memory | faced only | 1.0 | 3.8 | 1.0 | yes | 0.0 |
| above 0: ASTC 8 x 8 | memory | five directions | 3.2 | 13.3 | 3.3 | yes | 0.1 |
| above 0: ASTC 8 x 8 | memory | faced only | 0.6 | 2.7 | 0.7 | yes | 0.0 |
| above 2500: BC1 | memory | five directions | 7.7 | 42.9 | 10.8 | yes | 0.4 |
| above 2500: BC1 | memory | faced only | 1.5 | 8.8 | 2.3 | yes | 0.1 |
| above 2500: BC7 | memory | five directions | 9.6 | 47.3 | 12.0 | yes | 0.4 |
| above 2500: BC7 | memory | faced only | 1.9 | 9.7 | 2.5 | yes | 0.1 |
| above 2500: ASTC 6 x 6 | memory | five directions | 8.0 | 42.0 | 10.6 | yes | 0.4 |
| above 2500: ASTC 6 x 6 | memory | faced only | 1.6 | 8.6 | 2.2 | yes | 0.1 |
| above 2500: ASTC 8 x 8 | memory | five directions | 7.1 | 40.1 | 10.2 | yes | 0.4 |
| above 2500: ASTC 8 x 8 | memory | faced only | 1.4 | 8.2 | 2.1 | yes | 0.1 |
| above 2500: BC1, the rest BC7 | memory | five directions | 6.9 | 27.6 | 7.0 | yes | 0.2 |
| above 2500: BC1, the rest BC7 | memory | faced only | 1.4 | 5.8 | 1.5 | yes | 0.0 |
| above 10000: BC1 | memory | five directions | 9.9 | 58.1 | 14.9 | yes | 0.8 |
| above 10000: BC1 | memory | faced only | 2.0 | 11.9 | 3.1 | yes | 0.2 |
| above 10000: BC7 | memory | five directions | 9.9 | 57.6 | 14.8 | yes | 0.7 |
| above 10000: BC7 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 10000: ASTC 6 x 6 | memory | five directions | 9.9 | 58.1 | 14.9 | yes | 0.8 |
| above 10000: ASTC 6 x 6 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 10000: ASTC 8 x 8 | memory | five directions | 9.9 | 57.9 | 14.9 | yes | 0.7 |
| above 10000: ASTC 8 x 8 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 10000: BC1, the rest BC7 | memory | five directions | 8.7 | 32.1 | 8.2 | yes | 0.4 |
| above 10000: BC1, the rest BC7 | memory | faced only | 1.8 | 6.7 | 1.8 | yes | 0.1 |
| above 40000: BC1 | memory | five directions | 9.9 | 58.2 | 14.9 | yes | 0.8 |
| above 40000: BC1 | memory | faced only | 2.0 | 11.8 | 3.2 | yes | 0.2 |
| above 40000: BC7 | memory | five directions | 9.9 | 57.9 | 14.9 | yes | 0.7 |
| above 40000: BC7 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 40000: ASTC 6 x 6 | memory | five directions | 9.9 | 57.9 | 14.8 | yes | 0.7 |
| above 40000: ASTC 6 x 6 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 40000: ASTC 8 x 8 | memory | five directions | 9.9 | 58.0 | 14.9 | yes | 0.8 |
| above 40000: ASTC 8 x 8 | memory | faced only | 2.0 | 11.8 | 3.1 | yes | 0.2 |
| above 40000: BC1, the rest BC7 | memory | five directions | 8.7 | 32.6 | 8.2 | yes | 0.4 |
| above 40000: BC1, the rest BC7 | memory | faced only | 1.8 | 6.5 | 1.7 | yes | 0.1 |

## 15. GPU cost, headless (`gpu-cost`)

**Measure only, no renderer change.** A scratch Vulkan tool (`uodynamapper-scratch/gpu-cost`, on the workspace's `wgpu` 30.0) draws the creature crowd offscreen with timestamp queries. Nothing from it is in the repo.

**This GPU can't prove low-end results.** The adapter reports AMD Radeon 880M Graphics (RADV STRIX1), radv Mesa 26.2.6, not the 890M the brief named. It's above the integrated GPUs the 60 fps goal has to hold on, so read the ratios between layouts and not the milliseconds. Shares of a 60 fps frame are for this GPU only.

**What it draws:**

- The crowd's 300 faced streams from the every-10th dump, one frame each, packed in a single 8192 px wide atlas with a 4 px gutter. Classic's is 8192 × 3672, Enhanced's 8192 × 5532.
- Instanced quads into a 1920 × 1080 RGBA8 target with premultiplied blending and no depth test, at random places, so sprites overlap as a crowd does. Eight frame sets rotate through the passes, so a row isn't one lucky frame.
- 1 × puts a fragment's centre on a texel; 1.5 × is the fractional case. **The filter is the real one:** `world/pixel_art_filters.wesl`'s seam filter, with `art/main.wesl` taking one hardware sample through it.
- **Indexed layouts can't use the hardware filter,** so they gather and blend by hand: Classic R8 is 1 gather + 4 palette reads, Enhanced RG8 is 2 gathers + 4 palette reads. Partial coverage is blended premultiplied, as a sampler does on BC7's alpha. A hardware-sampled row is 1 sample.
- Every row is timed inside the same seven rounds of 16 passes, interleaved, so drift cancels. A cell is the median round. A shader per row is generated with the unused paths removed before compiling, as `shader_defs` would.

**Layouts,** 3.19 Mpx of Classic's faced creature crowd and 3.98 Mpx of Enhanced's at 1 ×; 7.17 and 8.96 Mpx at 1.5 ×. Each cell is the GPU pass, its ratio to that client's nearest indexed draw, and its share of a 60 fps frame on this GPU.

| layout | fetches | Classic, 1 × | Classic, 1.5 × | Enhanced, 1 × | Enhanced, 1.5 × |
|---|---:|---:|---:|---:|---:|
| index, load, 1 palette read | nearest: 1 texel fetch + 1 palette read | 0.152 ms, 1.00 ×, 0.9 % | not filtered | 0.207 ms, 1.00 ×, 1.2 % | not filtered |
| index, manual filter | Classic 1 gather + 4 palette reads; Enhanced 2 gathers + 4 palette reads | 0.175 ms, 1.15 ×, 1.0 % | 0.324 ms, 2.13 ×, 1.9 % | 0.229 ms, 1.10 ×, 1.4 % | 0.409 ms, 1.97 ×, 2.5 % |
| index, manual filter, 1 read when the fraction is zero | the same with a branch | 0.191 ms, 1.25 ×, 1.1 % | 0.400 ms, 2.64 ×, 2.4 % | 0.254 ms, 1.23 ×, 1.5 % | 0.523 ms, 2.52 ×, 3.1 % |
| row spans, nearest | nearest: 2 buffer reads + 1 palette read | 0.155 ms, 1.02 ×, 0.9 % | not filtered | 0.196 ms, 0.95 ×, 1.2 % | not filtered |
| row spans, manual filter | filter: 4 row-table reads + 4 pixels + 4 palette reads | 0.225 ms, 1.48 ×, 1.3 % | 0.423 ms, 2.79 ×, 2.5 % | 0.292 ms, 1.41 ×, 1.8 % | 0.555 ms, 2.68 ×, 3.3 % |
| BC1 | 1 hardware sample | 0.115 ms, 0.76 ×, 0.7 % | 0.207 ms, 1.37 ×, 1.2 % | 0.137 ms, 0.66 ×, 0.8 % | 0.249 ms, 1.20 ×, 1.5 % |
| BC7 | 1 hardware sample | 0.122 ms, 0.80 ×, 0.7 % | 0.214 ms, 1.42 ×, 1.3 % | 0.147 ms, 0.71 ×, 0.9 % | 0.261 ms, 1.26 ×, 1.6 % |
| RGBA8 expanded at upload | 1 hardware sample | 0.205 ms, 1.34 ×, 1.2 % | 0.272 ms, 1.79 ×, 1.6 % | 0.249 ms, 1.21 ×, 1.5 % | 0.331 ms, 1.60 ×, 2.0 % |

**Ratios that decide it,** hardware-sampled BC7 against the indexed filter, in the same pass:

| client | scale | index filter ÷ BC7 | index filter ÷ BC1 | row spans filter ÷ BC7 | row spans filter ÷ index filter | RGBA8 ÷ BC7 | index filter − BC7, this GPU |
|---|---:|---:|---:|---:|---:|---:|---:|
| Classic | 1 × | 1.43 × | 1.52 × | 1.84 × | 1.29 × | 1.68 × | 0.053 ms |
| Classic | 1.5 × | 1.51 × | 1.57 × | 1.98 × | 1.31 × | 1.27 × | 0.110 ms |
| Enhanced | 1 × | 1.56 × | 1.67 × | 1.99 × | 1.28 × | 1.69 × | 0.082 ms |
| Enhanced | 1.5 × | 1.57 × | 1.64 × | 2.13 × | 1.36 × | 1.27 × | 0.148 ms |

**Hue on every sprite:**

| path | Classic, 1 × | Classic, 1.5 × | Enhanced, 1 × | Enhanced, 1.5 × |
|---|---:|---:|---:|---:|
| index filter, a hue read per tap | 0.249 ms, 1.64 × | 0.469 ms, 3.09 × | 0.335 ms, 1.62 × | 0.619 ms, 2.99 × |
| index filter, a palette row hued in advance | 0.175 ms, 1.15 × | 0.324 ms, 2.13 × | 0.229 ms, 1.10 × | 0.409 ms, 1.97 × |
| row spans filter, a hue read per tap | 0.303 ms, 2.00 × | 0.567 ms, 3.74 × | 0.394 ms, 1.90 × | 0.734 ms, 3.55 × |
| row spans filter, a palette row hued in advance | 0.232 ms, 1.52 × | 0.435 ms, 2.87 × | 0.301 ms, 1.45 × | 0.566 ms, 2.73 × |
| BC1, hue read after the sample | 0.143 ms, 0.94 × | 0.259 ms, 1.70 × | 0.169 ms, 0.81 × | 0.310 ms, 1.49 × |
| BC1, hue read and an R8 mask sample | 0.163 ms, 1.07 × | 0.294 ms, 1.94 × | 0.192 ms, 0.93 × | 0.351 ms, 1.69 × |
| BC7, hue read after the sample | 0.154 ms, 1.02 × | 0.277 ms, 1.82 × | 0.187 ms, 0.90 × | 0.337 ms, 1.63 × |
| BC7, hue read and an R8 mask sample | 0.173 ms, 1.14 × | 0.307 ms, 2.02 × | 0.209 ms, 1.01 × | 0.375 ms, 1.81 × |
| RGBA8, hue read after the sample | 0.226 ms, 1.48 × | 0.337 ms, 2.22 × | 0.277 ms, 1.34 × | 0.415 ms, 2.00 × |
| RGBA8, hue read and an R8 mask sample | 0.251 ms, 1.65 × | 0.366 ms, 2.40 × | 0.306 ms, 1.48 × | 0.447 ms, 2.16 × |

**Branch against separate pipelines, on the same sprites:** a is a pipeline per format with sprites grouped; b is one shader with a per-instance branch; c is a pipeline per format with a switch on every draw, the worst case for a mixed pack in a blended pass. The mix is the share of sprites on the first format.

| client | pair | scale | mix | a ms | b ms | c ms | b ÷ a | c ÷ a |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| Classic | R8, load, 1 palette read + BC1, 1 hardware sample | 1 × | 100 / 0 | 0.152 | 0.151 | 0.170 | 0.993 | 1.118 |
| Classic | R8, load, 1 palette read + BC1, 1 hardware sample | 1 × | 90 / 10 | 0.152 | 0.149 | 0.168 | 0.980 | 1.105 |
| Classic | R8, load, 1 palette read + BC1, 1 hardware sample | 1 × | 50 / 50 | 0.147 | 0.142 | 0.158 | 0.966 | 1.075 |
| Classic | R8, load, 1 palette read + BC1, 1 hardware sample | 1 × | 0 / 100 | 0.114 | 0.123 | 0.130 | 1.079 | 1.140 |
| Classic | R8, 1 gather, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 100 / 0 | 0.324 | 0.325 | 0.367 | 1.003 | 1.133 |
| Classic | R8, 1 gather, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 90 / 10 | 0.324 | 0.320 | 0.362 | 0.988 | 1.117 |
| Classic | R8, 1 gather, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 50 / 50 | 0.295 | 0.291 | 0.329 | 0.986 | 1.115 |
| Classic | R8, 1 gather, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 0 / 100 | 0.207 | 0.229 | 0.258 | 1.106 | 1.246 |
| Classic | row spans, 2 rows, 4 pixels, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 100 / 0 | 0.424 | 0.437 | 0.441 | 1.031 | 1.040 |
| Classic | row spans, 2 rows, 4 pixels, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 90 / 10 | 0.419 | 0.431 | 0.435 | 1.029 | 1.038 |
| Classic | row spans, 2 rows, 4 pixels, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 50 / 50 | 0.350 | 0.388 | 0.378 | 1.109 | 1.080 |
| Classic | row spans, 2 rows, 4 pixels, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 0 / 100 | 0.207 | 0.251 | 0.259 | 1.213 | 1.251 |
| Enhanced | RG8, load, 1 palette read + BC7, 1 hardware sample | 1 × | 100 / 0 | 0.207 | 0.204 | 0.242 | 0.986 | 1.169 |
| Enhanced | RG8, load, 1 palette read + BC7, 1 hardware sample | 1 × | 90 / 10 | 0.208 | 0.203 | 0.240 | 0.976 | 1.154 |
| Enhanced | RG8, load, 1 palette read + BC7, 1 hardware sample | 1 × | 50 / 50 | 0.193 | 0.186 | 0.218 | 0.964 | 1.130 |
| Enhanced | RG8, load, 1 palette read + BC7, 1 hardware sample | 1 × | 0 / 100 | 0.146 | 0.157 | 0.181 | 1.075 | 1.240 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 100 / 0 | 0.409 | 0.414 | 0.464 | 1.012 | 1.134 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 90 / 10 | 0.409 | 0.411 | 0.462 | 1.005 | 1.130 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 50 / 50 | 0.363 | 0.365 | 0.424 | 1.006 | 1.168 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 0 / 100 | 0.262 | 0.290 | 0.350 | 1.107 | 1.336 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 100 / 0 | 0.409 | 0.414 | 0.464 | 1.012 | 1.134 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 90 / 10 | 0.407 | 0.409 | 0.461 | 1.005 | 1.133 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 50 / 50 | 0.353 | 0.359 | 0.404 | 1.017 | 1.144 |
| Enhanced | RG8, 2 gathers, 4 palette reads + BC1, 1 hardware sample | 1.5 × | 0 / 100 | 0.249 | 0.277 | 0.310 | 1.112 | 1.245 |
| Enhanced | row spans, 2 rows, 4 pixels, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 100 / 0 | 0.554 | 0.571 | 0.577 | 1.031 | 1.042 |
| Enhanced | row spans, 2 rows, 4 pixels, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 90 / 10 | 0.549 | 0.565 | 0.570 | 1.029 | 1.038 |
| Enhanced | row spans, 2 rows, 4 pixels, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 50 / 50 | 0.442 | 0.501 | 0.490 | 1.133 | 1.109 |
| Enhanced | row spans, 2 rows, 4 pixels, 4 palette reads + BC7, 1 hardware sample | 1.5 × | 0 / 100 | 0.261 | 0.330 | 0.348 | 1.264 | 1.333 |

**Registers,** from `RADV_DEBUG=shaderstats,nocache`, pixel shader only. AMD only: another vendor allocates differently.

**Classic:**

| pipeline | VGPRs allocated | pre-scheduling | SGPRs pre-scheduling | spilled | instructions | branches | VMEM | code bytes | subgroups per SIMD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| R8, 1 gather, 4 palette reads / no hue | 32 | 13 | 22 | 0 | 155 | 1 | 7 | 800 | 32 |
| R8, gather, 1 read when the fraction is zero / no hue | 32 | 14 | 18 | 0 | 198 | 4 | 9 | 1060 | 32 |
| row spans, 2 rows, 4 pixels, 4 palette reads / no hue | 32 | 14 | 10 | 0 | 258 | 7 | 14 | 1320 | 32 |
| BC1, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| BC7, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| RGBA8 expanded, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| R8, load, 1 palette read / no hue | 8 | 7 | 17 | 0 | 70 | 2 | 4 | 376 | 32 |
| row spans, 1 row, 1 pixel, 1 palette read / no hue | 8 | 7 | 14 | 0 | 112 | 4 | 6 | 572 | 32 |
| R8, load, 1 palette read / hue texture | 8 | 7 | 18 | 0 | 85 | 3 | 6 | 468 | 32 |
| row spans, 1 row, 1 pixel, 1 palette read / hue texture | 8 | 8 | 20 | 0 | 127 | 5 | 8 | 664 | 32 |
| R8, 1 gather, 4 palette reads / hue texture | 32 | 15 | 21 | 0 | 207 | 5 | 12 | 1124 | 32 |
| row spans, 2 rows, 4 pixels, 4 palette reads / hue texture | 32 | 16 | 21 | 0 | 318 | 11 | 19 | 1668 | 32 |
| BC1, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| BC1, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| BC7, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| BC7, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| RGBA8 expanded, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| RGBA8 expanded, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| branch: R8, load, 1 palette read + BC1, 1 hardware sample | 16 | 10 | 17 | 0 | 127 | 4 | 6 | 680 | 32 |
| branch: R8, 1 gather, 4 palette reads + BC1, 1 hardware sample | 24 | 14 | 16 | 0 | 194 | 3 | 9 | 1036 | 32 |
| branch: row spans, 2 rows, 4 pixels, 4 palette reads + BC1, 1 hardware sample | 24 | 14 | 18 | 0 | 310 | 9 | 16 | 1576 | 32 |

**Enhanced:**

| pipeline | VGPRs allocated | pre-scheduling | SGPRs pre-scheduling | spilled | instructions | branches | VMEM | code bytes | subgroups per SIMD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| RG8, 2 gathers, 4 palette reads / no hue | 32 | 13 | 22 | 0 | 155 | 1 | 8 | 808 | 32 |
| RG8, gathers, 1 read when the fraction is zero / no hue | 32 | 16 | 16 | 0 | 203 | 4 | 10 | 1056 | 32 |
| row spans, 2 rows, 4 pixels, 4 palette reads / no hue | 32 | 14 | 9 | 0 | 270 | 7 | 14 | 1376 | 32 |
| BC1, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| BC7, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| RGBA8 expanded, 1 hardware sample / no hue | 16 | 8 | 16 | 0 | 83 | 1 | 2 | 436 | 32 |
| RG8, load, 1 palette read / no hue | 8 | 7 | 17 | 0 | 72 | 2 | 4 | 388 | 32 |
| row spans, 1 row, 1 pixel, 1 palette read / no hue | 8 | 7 | 13 | 0 | 117 | 4 | 6 | 600 | 32 |
| RG8, load, 1 palette read / hue texture | 8 | 7 | 17 | 0 | 88 | 3 | 6 | 484 | 32 |
| row spans, 1 row, 1 pixel, 1 palette read / hue texture | 8 | 8 | 16 | 0 | 132 | 5 | 8 | 692 | 32 |
| RG8, 2 gathers, 4 palette reads / hue texture | 32 | 15 | 22 | 0 | 209 | 5 | 13 | 1152 | 32 |
| row spans, 2 rows, 4 pixels, 4 palette reads / hue texture | 32 | 16 | 21 | 0 | 328 | 11 | 19 | 1736 | 32 |
| BC1, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| BC1, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| BC7, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| BC7, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| RGBA8 expanded, 1 hardware sample / hue texture | 24 | 9 | 22 | 0 | 93 | 1 | 4 | 508 | 32 |
| RGBA8 expanded, 1 hardware sample / hue texture and R8 mask | 24 | 9 | 22 | 0 | 102 | 1 | 5 | 548 | 32 |
| branch: RG8, load, 1 palette read + BC7, 1 hardware sample | 16 | 10 | 17 | 0 | 129 | 4 | 6 | 688 | 32 |
| branch: RG8, 2 gathers, 4 palette reads + BC7, 1 hardware sample | 32 | 16 | 16 | 0 | 198 | 3 | 10 | 1036 | 32 |
| branch: RG8, 2 gathers, 4 palette reads + BC1, 1 hardware sample | 32 | 16 | 16 | 0 | 198 | 3 | 10 | 1036 | 32 |
| branch: row spans, 2 rows, 4 pixels, 4 palette reads + BC7, 1 hardware sample | 32 | 14 | 17 | 0 | 322 | 9 | 16 | 1632 | 32 |


**What the registers say:**

- **Does a branch reserve the path not taken? Yes, as the user expected.** Registers are allocated per shader, whichever path a sprite takes. BC1 alone uses 16 VGPRs allocated, 8 before scheduling. In one shader with nearest R8 that becomes 16 and 10; with Classic's gather, 24 and 14; with Enhanced's gather, 32 and 16. So the BC1 sprites in that shader run with registers their path doesn't need.
- **Read the pre-scheduling counts.** AMD allocates VGPRs in groups of 8, and the allocated figure depends on what the scheduler does. It doesn't even rise steadily: Classic's gather alone is 32 allocated and 13 before scheduling, and the same gather sharing a shader with BC1 is 24 and 14.
- **The manual filter costs registers:** 32 against 16 for a hardware sample. Hardware-sampled BC paths also take fewer instructions (83 against 155).
- **Nothing spills, and no test shader comes near a vendor limit.** For context only, Mali Valhall halves its threads above 32 work registers on several GPUs and Intel drops from SIMD16 to SIMD8 at 64. The 13 to 16 VGPRs before scheduling sit under both, but those are AMD's counts in AMD's compiler. **`malioc` would be needed to say anything about Mali, and it isn't installed.**

**Does the reserved register show in the timing? Only on the cheap path.** The table above has the same sprites three ways. In the quiet run:

- **When most sprites take the expensive path, the branch costs nothing:** b ÷ a is 0.98 to 1.03 at 100/0 and 90/10, on every pair.
- **When all sprites take the cheap path in a shader that also holds the expensive one,** the branch costs 8 % with R8 load, about 11 % with the gather, and 21 to 26 % with row spans.
- **At 50/50** the branch is level with grouped pipelines with load or gather (0.96 to 1.02), and 11 to 13 % behind with row spans.
- **A pipeline switch on every draw is usually the worst case:** c ÷ a is 1.04 to 1.34, and it's the larger of b and c on 26 of the 28 rows. Grouping needs sprites that can be drawn out of order, which a blended pass can't always do.

So a per-format shader only pays when one pipeline's sprites can be grouped and they're the cheap ones. In a mostly indexed pack, a shared shader loses nothing measurable here. These are ratios on one AMD GPU.

**The real art shader, for the starting point.** Method: `RADV_DEBUG=shaderstats,nocache gpu-cost real`, which compiles `dynamapper/assets/shaders/world/art/main.wesl` through `wesl` 0.6 and wgpu as it stands, from the repo, and reads the pixel shader's statistics. The file's last commit is `01003ab7`, and it's byte-identical to `ad2975dd` where it was first measured. Its only `@if` features are `STATIC_PICK` and `STATIC_PICK_MSAA`; everything else is a runtime uniform. **So the rows below aren't `shader_defs` that exist.** They're text rewrites in a scratch copy, with uniforms fixed as literals, standing in for features that could exist. No renderer change was made.

| `art/main.wesl`, pixel shader | VGPRs allocated | pre-scheduling | SGPRs pre-scheduling | spilled | instructions | branches | VMEM | code bytes | subgroups per SIMD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| the renderer's art/main.wesl: as it is, every @if off | 56 | 50 | 73 | 0 | 7754 | 85 | 81 | 40320 | 18 |
| the renderer's art/main.wesl: opaque pass only | 48 | 47 | 68 | 0 | 7486 | 73 | 70 | 38852 | 20 |
| the renderer's art/main.wesl: transparent pass only | 48 | 47 | 68 | 0 | 7494 | 73 | 70 | 38912 | 20 |
| the renderer's art/main.wesl: shadow pass only | 32 | 19 | 22 | 0 | 306 | 4 | 13 | 1688 | 32 |
| the renderer's art/main.wesl: billboards only (no flat quads, no wet), every pass | 40 | 38 | 56 | 0 | 2803 | 30 | 30 | 14188 | 24 |
| the renderer's art/main.wesl: billboards, transparent pass | 40 | 38 | 55 | 0 | 2549 | 24 | 19 | 12788 | 24 |
| the renderer's art/main.wesl: billboards, transparent pass, no water, surface profile or grunge | 40 | 38 | 51 | 0 | 1614 | 19 | 18 | 8468 | 24 |
| the renderer's art/main.wesl: the same, also no grading or tonemap | 40 | 38 | 51 | 0 | 1504 | 17 | 18 | 7884 | 24 |


- **Sprite fetches are small against this shader.** A filtered indexed sprite is 155 to 270 instructions and 8 to 14 VMEM operations. The art shader is 7754 instructions and 81 VMEM operations, of which billboards in the transparent pass use 2549 and 19. The filter's cost matters less than what that shader does around it, on any layout.
- **Splitting recovers registers and instructions.** One path per pass moves it from 56 to 48 VGPRs, and billboards only to 40 with 18 → 24 subgroups per SIMD. The shadow pass alone would be 32 VGPRs and 306 instructions.

**The other half of "drawable": getting the crowd to the GPU.** `gpu-cost upload` times the faced frames going up through `queue.write_texture`, once as a single write of the atlas and once frame by frame, until the queue is idle again. This GPU shares system memory, so a discrete card over PCIe can differ. Not measured there.

| client | layout | MB, one write | ms | MB/s | MB, frame by frame | ms | MB/s |
|---|---|---:|---:|---:|---:|---:|---:|
| Classic | R8 index | 30.1 | 3.82 | 7883 | 25.8 | 18.47 | 1397 |
| Classic | BC1 | 15.0 | 1.43 | 10538 | 13.2 | 15.36 | 858 |
| Classic | BC7 | 30.1 | 2.43 | 12357 | 26.4 | 16.82 | 1567 |
| Classic | RGBA8 | 120.3 | 8.70 | 13826 | 103.2 | 24.70 | 4176 |
| Classic | row spans: table and pixels, storage buffers | 14.7 | 1.04 | 14055 |  |  |  |
| Enhanced | RG8 index | 90.6 | 7.66 | 11829 | 79.9 | 30.16 | 2648 |
| Enhanced | BC1 | 22.7 | 2.04 | 11099 | 20.4 | 22.02 | 928 |
| Enhanced | BC7 | 45.3 | 3.64 | 12465 | 40.8 | 24.10 | 1695 |
| Enhanced | RGBA8 | 181.3 | 38.20 | 4745 | 159.7 | 58.89 | 2712 |
| Enhanced | row spans: table and pixels, storage buffers | 39.3 | 2.66 | 14783 |  |  |  |


**Does the crowd stay drawable?** Decode on 4 threads plus the upload, summed, against the user's line of about 250 ms. Decode is `qc`'s quiet rerun on the placeholder rule's creature crowd, the same crowd `gpu-cost` draws, so each row is zstd 19 per stream. The upload was measured for the faced frames only, so **the × 5 columns are an estimate, not a run.** A sum is the pessimistic case, since a real load could upload while other workers decode.

| client | what the GPU holds | decode, faced ms | one write, faced ms | **total, faced** | decode, five directions ms | one write × 5 | **total, five directions** | frame by frame × 5 | total, five directions |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Classic | R8 rectangle | 10.3 | 3.82 | **14.1** | 50.0 | 19.1 | **69.1** | 92.3 | 142.3 |
| Classic | BC1 | 5.3 | 1.43 | **6.7** | 23.3 | 7.1 | **30.4** | 76.8 | 100.1 |
| Classic | BC7 | 8.0 | 2.43 | **10.4** | 39.0 | 12.2 | **51.1** | 84.1 | 123.1 |
| Enhanced | RG8 rectangle | 20.5 | 7.66 | **28.2** | 100.4 | 38.3 | **138.7** | 150.8 | 251.2 |
| Enhanced | BC1 | 5.8 | 2.04 | **7.8** | 28.8 | 10.2 | **39.0** | 110.1 | 138.9 |
| Enhanced | BC7 | 10.1 | 3.64 | **13.7** | 49.8 | 18.2 | **68.0** | 120.5 | 170.3 |

- **Every row passes when uploads are batched,** with room to spare on faced loads: 28.2 ms is the slowest. With all five directions the slowest is Enhanced's RG8 at about 139 ms.
- **Frame by frame is the one way to reach the line.** Enhanced RG8 sums to about 251 ms with all five directions, and 150.8 ms of that is writes. The bytes aren't the cost: 2852 small writes take 30.16 ms where one write of the larger atlas takes 7.66 ms. So a crowd's frames should go up in one write per load, or a few.
- **Row spans go up fastest,** 1.04 / 2.66 ms for 14.7 / 39.3 MB. Their decode is the "kept as spans" row of section 9, faster than expanded. They pay for it in the shader instead: section 15's filter table.
- **This machine shares memory between CPU and GPU.** A discrete card over PCIe isn't measured, and it's where upload would differ most.

**What this section can't say:**

- **Low-end GPUs.** The Radeon 880M is above the integrated GPUs the 60 fps goal has to hold on. Read ratios between layouts. No millisecond figure here is a low-end result.
- **Other vendors.** Register counts are AMD's compiler on AMD's hardware. The vendor limits above are for context only. `malioc` would be needed to say anything about Mali, and it wasn't installed.
- **The real renderer.** No renderer code changed, and the crowd draws without the art shader's work around it. Hue from colour instead of index, and a second shader path, are measured here headless and not in a renderer.
- **Shaders on ETC2 or ASTC.** `gpu-cost` has no such rows. The hardware-sampled BC rows are the nearest stand-in, and that's an assumption.

## 16. The final candidates on every body (`wl`, `qw`)

The earlier sections use samples. This one runs the final candidates on **every body in the set,** so these totals replace the estimates. Nothing here is timed.

- **Lossless (`wl`):** each layout coded on every stream and **decoded back and compared,** on all 1022 Classic and 926 Enhanced bodies. Totals include the palette table.
- **BC7 (`qw`):** every stream encoded with one encoder, then scored. "Repo" is a byte-identical scratch copy of the encoder in `lib/udd-image-codecs/src/bc7`. The fixed rows are that copy with the lines in section 12. **The repo encoder was unchanged during these runs.**
- **BC3 and BC1 (`qw`):** the same streams through `dds` 0.2.0 at `CompressionQuality::High`, labelled by that encoder. The last subsection compares them with the BC7 rows.
- **What "score" covers:** SSIMULACRA2 runs per body on a mosaic of up to 64 frames (`SSIM_FRAMES_PER_BODY`), so a body's score samples its frames. Hue, coverage and PSNR cover every pixel.
- **RDO runs in the encoder's segments:** above 2048 blocks a stream is split for parallel work, so the longest streams search a little less than they could.

**Classic, lossless:** 1022 bodies, 138426 streams, 2419.0 Mpx, and a palette table of 19.8 MB counted in each total.

| layout | total MB | × zstd 19 | equipment | NPC small | NPC medium | NPC large | not listed | duplicates dropped MB | total without them MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | 761.1 | 1.000 | 120.7 | 24.0 | 141.1 | 446.1 | 9.5 | 27.7 | 733.4 |
| plain planes, LZ4 HC | 929.7 | 1.222 | 163.8 | 30.1 | 171.1 | 532.9 | 12.0 | 32.7 | 897.0 |
| **a: row spans, zstd 19** | 733.4 | 0.964 | 116.5 | 23.1 | 135.9 | 429.0 | 9.2 | 26.9 | 706.5 |
| a: row spans, LZ4 HC | 888.4 | 1.167 | 155.8 | 28.8 | 164.6 | 507.9 | 11.5 | 31.5 | 856.9 |
| a: row spans, best dictionary per body | 711.1 | 0.934 | 112.9 | 22.4 | 130.8 | 416.7 | 8.6 | 18.8 | 692.3 |
| b: painted only, runs apart, zstd 19 | 746.8 | 0.981 | 121.5 | 23.8 | 138.9 | 433.3 | 9.5 | 27.3 | 719.5 |

**Enhanced, lossless:** 926 bodies, 158133 streams, 4916.9 Mpx, and a palette table of 32.4 MB counted in each total.

| layout | total MB | × zstd 19 | equipment | NPC small | NPC medium | NPC large | not listed | duplicates dropped MB | total without them MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| plain planes, zstd 19 | 2013.8 | 1.000 | 506.7 | 30.3 | 394.9 | 1049.5 | 0.0 | 12.3 | 2001.5 |
| plain planes, LZ4 HC | 2459.3 | 1.221 | 665.2 | 39.0 | 475.4 | 1247.3 | 0.0 | 15.5 | 2443.8 |
| **a: row spans, zstd 19** | 1955.7 | 0.971 | 495.4 | 29.5 | 383.0 | 1015.4 | 0.0 | 12.0 | 1943.7 |
| b: painted only, zstd 19 | 1845.3 | 0.916 | 455.6 | 27.2 | 362.0 | 968.0 | 0.0 | 11.4 | 1833.9 |
| **b: painted only, coverage in row spans, zstd 19** | 1819.9 | 0.904 | 449.9 | 26.8 | 356.6 | 954.3 | 0.0 | 11.2 | 1808.7 |
| b: painted only, every coverage tANS by context | 1718.2 | 0.853 | 415.6 | 24.8 | 334.6 | 910.8 | 0.0 | 10.6 | 1707.6 |
| b: painted only, runs zstd 19, partial tANS by context | 1805.1 | 0.896 | 444.1 | 26.7 | 353.9 | 948.1 | 0.0 | 11.1 | 1794.0 |
| b: painted only, coverage in row spans, LZ4 HC | 2192.3 | 1.089 | 568.7 | 33.5 | 427.0 | 1130.6 | 0.0 | 13.7 | 2178.6 |
| **c: both planes in row spans, zstd 19** | 1881.1 | 0.934 | 462.6 | 27.7 | 368.1 | 990.2 | 0.0 | 11.5 | 1869.6 |
| c: both planes in row spans, LZ4 HC | 2268.0 | 1.126 | 588.4 | 34.9 | 441.4 | 1171.0 | 0.0 | 14.1 | 2253.9 |

**Section 10's assignment on every body** (equipment, small and medium with every coverage tANS by context; large with coverage in row spans): 1761.7 MB, 0.875 × zstd 19. There are no bodies outside the four classes on Enhanced.

**Classic, block formats on every body,** labelled by encoder:

| encoder | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| repo, as the converter calls it | 2517.5 | 911.6 | 894.7 | 67.39 | 46.76 | 0.72 | 191 of 1022 | 25.56 | 0.021 % | 7.406 % |
| fix A (approved, not merged) | 2517.5 | 911.8 | 894.9 | 68.37 | 51.05 | 35.31 | 193 of 1022 | 25.86 | 0.000 % | 7.405 % |
| fix A + colour | 2517.5 | 985.6 | 965.6 | 87.56 | 65.43 | 43.71 | 796 of 1022 | 33.60 | 0.000 % | 0.019 % |
| **fix C** (includes fix A) | 2517.5 | 936.8 | 918.4 | 87.51 | 63.77 | 38.67 | 782 of 1022 | 33.20 | 0.000 % | 0.002 % |
| fix C + colour | 2517.5 | 988.4 | 968.0 | 87.89 | 65.67 | 43.72 | 810 of 1022 | 33.77 | 0.000 % | 0.000 % |
| **fix C, RDO 1** | 2517.5 | 846.8 | 823.2 | 80.20 | 60.25 | 37.57 | 534 of 1022 | 31.35 | 0.000 % | 0.021 % |
| BC1, `dds` High | 1258.7 | 529.1 | 514.2 | 82.90 | 63.85 | 41.97 | 695 of 1022 | 29.48 | 0.000 % | 0.000 % |

**Classic, by class:** zstd 19 MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB.

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score | not listed MB | score |
|---|---:|---|---:|---|---:|---|---:|---|---:|---|
| repo, as the converter calls it | 167.2 | 70.95 / 19.21, 124 of 550, 22.87 | 29.2 | 60.76 / 0.72, 3 of 95, 23.06 | 167.8 | 60.94 / 36.88, 1 of 173, 25.23 | 518.2 | 62.94 / 30.91, 0 of 101, 27.10 | 12.3 | 81.39 / 23.59, 63 of 103, 22.62 |
| fix A (approved, not merged) | 167.2 | 71.21 / 35.31, 126 of 550, 23.02 | 29.2 | 62.36 / 35.32, 3 of 95, 23.19 | 167.8 | 63.27 / 36.88, 1 of 173, 25.45 | 518.3 | 66.18 / 49.27, 0 of 101, 27.56 | 12.3 | 81.40 / 40.72, 63 of 103, 23.24 |
| fix A + colour | 197.2 | 85.14 / 43.71, 368 of 550, 32.64 | 32.6 | 88.68 / 45.27, 85 of 95, 33.35 | 182.1 | 88.98 / 64.71, 166 of 173, 33.48 | 540.2 | 87.60 / 66.69, 97 of 101, 34.05 | 13.5 | 89.57 / 54.96, 80 of 103, 30.29 |
| **fix C** (includes fix A) | 177.8 | 84.85 / 38.67, 356 of 550, 31.63 | 30.1 | 88.89 / 42.93, 85 of 95, 32.90 | 173.0 | 88.89 / 76.28, 168 of 173, 33.33 | 524.9 | 88.00 / 71.10, 99 of 101, 33.82 | 12.5 | 89.80 / 47.18, 74 of 103, 29.18 |
| fix C + colour | 198.1 | 85.40 / 43.72, 377 of 550, 32.83 | 32.6 | 88.77 / 45.27, 85 of 95, 33.43 | 182.8 | 89.24 / 76.48, 169 of 173, 33.86 | 540.9 | 88.19 / 72.55, 99 of 101, 34.14 | 13.5 | 89.57 / 54.99, 80 of 103, 30.35 |
| **fix C, RDO 1** | 157.0 | 78.34 / 37.57, 223 of 550, 30.03 | 27.3 | 82.05 / 41.52, 62 of 95, 31.05 | 156.0 | 81.56 / 70.91, 120 of 173, 31.42 | 471.7 | 80.67 / 67.80, 64 of 101, 31.85 | 11.2 | 81.73 / 45.36, 65 of 103, 28.30 |
| BC1, `dds` High | 107.4 | 82.77 / 42.43, 339 of 550, 28.37 | 17.5 | 82.72 / 41.97, 67 of 95, 28.49 | 97.5 | 82.50 / 63.85, 131 of 173, 29.54 | 284.5 | 82.20 / 62.45, 80 of 101, 29.92 | 7.3 | 85.74 / 48.62, 78 of 103, 26.96 |

**Classic, bodies under 80:**

| encoder | equipment | NPC small | NPC medium | NPC large | not listed |
|---|---:|---:|---:|---:|---:|
| repo, as the converter calls it | 426 of 550, 92.7 % of its pixels | 92 of 95, 99.7 % of its pixels | 172 of 173, 99.9 % of its pixels | 101 of 101, 100.0 % of its pixels | 40 of 103, 88.4 % of its pixels |
| fix A (approved, not merged) | 424 of 550, 92.6 % of its pixels | 92 of 95, 99.7 % of its pixels | 172 of 173, 99.9 % of its pixels | 101 of 101, 100.0 % of its pixels | 40 of 103, 88.4 % of its pixels |
| fix A + colour | 182 of 550, 29.4 % of its pixels | 10 of 95, 5.1 % of its pixels | 7 of 173, 6.4 % of its pixels | 4 of 101, 2.2 % of its pixels | 23 of 103, 45.2 % of its pixels |
| **fix C** (includes fix A) | 194 of 550, 30.7 % of its pixels | 10 of 95, 5.1 % of its pixels | 5 of 173, 5.6 % of its pixels | 2 of 101, 1.5 % of its pixels | 29 of 103, 48.3 % of its pixels |
| fix C + colour | 173 of 550, 28.1 % of its pixels | 10 of 95, 5.1 % of its pixels | 4 of 173, 4.1 % of its pixels | 2 of 101, 1.5 % of its pixels | 23 of 103, 45.2 % of its pixels |
| **fix C, RDO 1** | 327 of 550, 54.8 % of its pixels | 33 of 95, 26.9 % of its pixels | 53 of 173, 28.8 % of its pixels | 37 of 101, 38.4 % of its pixels | 38 of 103, 74.2 % of its pixels |
| BC1, `dds` High | 211 of 550, 39.8 % of its pixels | 28 of 95, 35.8 % of its pixels | 42 of 173, 22.5 % of its pixels | 21 of 101, 16.6 % of its pixels | 25 of 103, 50.2 % of its pixels |

- Lowest bodies, **fix C** (includes fix A): anim4.mul body 655 (equipment, 38.67, median 308 px); anim4.mul body 671 (equipment, 39.80, median 280 px); AnimationFrame4.uop body 831 (NPC small, 42.93, median 525 px); anim4.mul body 204 (NPC small, 45.24, median 975 px); anim3.mul body 372 (not listed, 47.18, median 667 px); AnimationFrame2.uop body 326 (equipment, 50.40, median 160 px); anim4.mul body 653 (equipment, 52.97, median 104 px); AnimationFrame2.uop body 1485 (NPC small, 53.09, median 1085 px); anim5.mul body 208 (NPC small, 53.44, median 675 px); anim4.mul body 658 (equipment, 53.50, median 561 px); AnimationFrame2.uop body 332 (equipment, 53.62, median 160 px); anim.mul body 417 (equipment, 53.63, median 280 px); anim4.mul body 657 (equipment, 54.50, median 300 px); anim2.mul body 15 (NPC small, 54.52, median 945 px); AnimationFrame2.uop body 330 (equipment, 54.77, median 144 px)
- Lowest bodies, fix C + colour: anim4.mul body 655 (equipment, 43.72, median 308 px); AnimationFrame4.uop body 831 (NPC small, 45.27, median 525 px); anim4.mul body 671 (equipment, 46.23, median 280 px); anim4.mul body 204 (NPC small, 50.32, median 975 px); anim.mul body 417 (equipment, 54.58, median 280 px); anim3.mul body 372 (not listed, 54.99, median 667 px); anim2.mul body 15 (NPC small, 55.44, median 945 px); anim4.mul body 658 (equipment, 55.73, median 561 px); AnimationFrame2.uop body 1485 (NPC small, 55.85, median 1085 px); AnimationFrame3.uop body 661 (equipment, 56.48, median 420 px); anim3.mul body 595 (equipment, 56.61, median 132 px); AnimationFrame3.uop body 657 (equipment, 56.94, median 340 px); AnimationFrame2.uop body 326 (equipment, 56.97, median 160 px); anim4.mul body 653 (equipment, 57.05, median 104 px); anim5.mul body 208 (NPC small, 57.76, median 675 px)
- Lowest bodies, **fix C, RDO 1**: anim4.mul body 655 (equipment, 37.57, median 308 px); anim4.mul body 671 (equipment, 38.82, median 280 px); AnimationFrame4.uop body 831 (NPC small, 41.52, median 525 px); anim4.mul body 204 (NPC small, 45.04, median 975 px); anim3.mul body 372 (not listed, 45.36, median 667 px); AnimationFrame2.uop body 326 (equipment, 49.00, median 160 px); AnimationFrame2.uop body 1485 (NPC small, 51.22, median 1085 px); anim4.mul body 658 (equipment, 51.32, median 561 px); anim4.mul body 653 (equipment, 51.33, median 104 px); anim.mul body 417 (equipment, 52.01, median 280 px); anim2.mul body 15 (NPC small, 52.26, median 945 px); AnimationFrame2.uop body 330 (equipment, 52.36, median 144 px); AnimationFrame3.uop body 661 (equipment, 52.46, median 420 px); AnimationFrame2.uop body 332 (equipment, 52.78, median 160 px); anim4.mul body 657 (equipment, 52.90, median 300 px)
- Lowest bodies, BC1, `dds` High: AnimationFrame4.uop body 831 (NPC small, 41.97, median 525 px); anim4.mul body 655 (equipment, 42.43, median 308 px); anim4.mul body 671 (equipment, 45.61, median 280 px); anim4.mul body 204 (NPC small, 46.77, median 975 px); anim3.mul body 369 (not listed, 48.62, median 1539 px); AnimationFrame1.uop body 322 (equipment, 50.23, median 126 px); anim3.mul body 372 (not listed, 51.53, median 667 px); anim2.mul body 15 (NPC small, 52.47, median 945 px); anim.mul body 417 (equipment, 53.53, median 280 px); anim4.mul body 658 (equipment, 54.02, median 561 px); anim3.mul body 371 (not listed, 54.48, median 1624 px); AnimationFrame2.uop body 1485 (NPC small, 54.71, median 1085 px); AnimationFrame3.uop body 661 (equipment, 55.19, median 420 px); AnimationFrame3.uop body 657 (equipment, 55.29, median 340 px); anim3.mul body 595 (equipment, 55.86, median 132 px)

**Classic, hue:**

| encoder | key changed | moved, mean | full hue error, mean | hued pixels off by 8 or more | partial hue error, mean | grey stops being grey, 8 bit | 5 bit | colour becomes grey, 8 bit | 5 bit |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| repo, as the converter calls it | 53.16 % | 1.043 | 4.604 | 49.980 % | 8.844 | 48.56 % | 32.14 % | 0.300 % | 4.307 % |
| fix A (approved, not merged) | 52.57 % | 1.003 | 4.423 | 49.388 % | 8.634 | 48.55 % | 32.09 % | 0.298 % | 4.295 % |
| fix A + colour | 43.00 % | 0.513 | 2.250 | 39.291 % | 5.455 | 43.53 % | 30.31 % | 0.794 % | 4.486 % |
| **fix C** (includes fix A) | 44.18 % | 0.533 | 2.326 | 40.161 % | 5.707 | 42.23 % | 28.95 % | 1.841 % | 6.127 % |
| fix C + colour | 42.78 % | 0.506 | 2.222 | 39.109 % | 5.413 | 43.28 % | 30.14 % | 0.792 % | 4.449 % |
| **fix C, RDO 1** | 50.51 % | 0.657 | 2.864 | 46.184 % | 7.204 | 50.71 % | 34.63 % | 2.309 % | 6.285 % |
| BC1, `dds` High | 56.19 % | 0.805 | 3.566 | 52.569 % | 10.108 | 84.31 % | 30.60 % | 0.873 % | 3.190 % |

**Enhanced, block formats on every body,** labelled by encoder:

| encoder | block MB | zstd 7 MB | zstd 19 MB | SSIMULACRA2 median | p5 | min | bodies at 80 or better | RGB PSNR dB | painted pixels gone | transparent pixels painted |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| repo, as the converter calls it | 5106.4 | 1946.0 | 1914.7 | 60.21 | 42.94 | 4.39 | 93 of 926 | 25.89 | 0.060 % | 3.892 % |
| fix A (approved, not merged) | 5106.4 | 1945.2 | 1913.9 | 69.99 | 55.61 | 41.20 | 163 of 926 | 26.70 | 0.036 % | 3.890 % |
| fix A + colour | 5106.4 | 2027.4 | 1996.0 | 77.43 | 64.05 | 45.57 | 318 of 926 | 30.56 | 0.484 % | 3.491 % |
| **fix C** (includes fix A) | 5106.4 | 1936.2 | 1900.7 | 85.40 | 64.34 | 43.84 | 712 of 926 | 31.34 | 0.540 % | 2.642 % |
| fix C + colour | 5106.4 | 2025.1 | 1990.6 | 87.14 | 67.86 | 47.72 | 756 of 926 | 32.51 | 0.455 % | 3.097 % |
| fix C, RDO 0.5 | 5106.4 | 1852.5 | 1803.9 | 83.90 | 63.39 | 43.25 | 680 of 926 | 30.97 | 0.667 % | 2.853 % |
| **fix C, RDO 1** | 5106.4 | 1778.2 | 1728.9 | 81.03 | 62.05 | 42.09 | 558 of 926 | 30.26 | 0.742 % | 3.057 % |
| fix C + colour, RDO 1 | 5106.4 | 1848.2 | 1796.7 | 82.48 | 66.55 | 45.94 | 653 of 926 | 31.11 | 0.605 % | 3.375 % |
| BC1, `dds` High | 2553.2 | 955.6 | 929.5 | 73.75 | 52.00 | -47.93 | 170 of 926 | 29.75 | 10.361 % | 0.000 % |
| BC3, `dds` High | 5106.4 | 1533.5 | 1463.1 | 81.72 | 63.38 | 43.12 | 585 of 926 | 28.55 | 0.151 % | 0.260 % |
| BC3, `dds` High, colour step | 5106.4 | 1606.4 | 1539.0 | 84.52 | 66.43 | 46.73 | 722 of 926 | 29.76 | 0.151 % | 0.260 % |

**Enhanced, by class:** zstd 19 MB, then SSIMULACRA2 median / min, bodies at 80 or better, RGB PSNR dB.

| encoder | equipment MB | score | NPC small MB | score | NPC medium MB | score | NPC large MB | score |
|---|---:|---|---:|---|---:|---|---:|---|
| repo, as the converter calls it | 478.4 | 63.75 / 18.41, 92 of 558, 23.79 | 27.5 | 58.31 / 37.59, 0 of 51, 24.58 | 372.2 | 55.94 / 4.39, 0 of 187, 25.71 | 1036.6 | 59.37 / 30.63, 1 of 130, 27.26 |
| fix A (approved, not merged) | 478.1 | 70.93 / 41.20, 156 of 558, 24.42 | 27.4 | 66.93 / 54.96, 3 of 51, 24.99 | 372.0 | 69.09 / 50.34, 3 of 187, 26.61 | 1036.4 | 70.94 / 57.23, 1 of 130, 28.20 |
| fix A + colour | 514.7 | 79.11 / 45.57, 254 of 558, 29.19 | 29.8 | 75.44 / 65.45, 13 of 51, 29.35 | 387.4 | 76.16 / 58.75, 31 of 187, 30.36 | 1064.1 | 76.65 / 62.46, 20 of 130, 31.36 |
| **fix C** (includes fix A) | 472.3 | 85.49 / 43.84, 384 of 558, 29.86 | 27.2 | 85.53 / 64.34, 42 of 51, 30.48 | 369.9 | 85.74 / 70.79, 170 of 187, 31.40 | 1031.3 | 84.70 / 72.32, 116 of 130, 32.10 |
| fix C + colour | 512.4 | 87.21 / 47.72, 414 of 558, 31.43 | 29.6 | 87.72 / 68.76, 45 of 51, 32.42 | 386.4 | 87.61 / 71.40, 176 of 187, 32.77 | 1062.2 | 85.92 / 72.90, 121 of 130, 32.94 |
| fix C, RDO 0.5 | 437.3 | 83.76 / 43.25, 373 of 558, 29.54 | 26.3 | 83.88 / 63.75, 39 of 51, 30.23 | 355.2 | 84.44 / 70.63, 156 of 187, 31.06 | 985.1 | 83.48 / 71.02, 112 of 130, 31.68 |
| **fix C, RDO 1** | 419.4 | 81.14 / 42.09, 320 of 558, 28.99 | 25.5 | 81.27 / 62.76, 34 of 51, 29.72 | 341.8 | 81.42 / 70.07, 132 of 187, 30.37 | 942.1 | 80.12 / 68.24, 72 of 130, 30.86 |
| fix C + colour, RDO 1 | 448.3 | 82.92 / 45.94, 362 of 558, 30.16 | 27.7 | 83.44 / 67.90, 43 of 51, 31.26 | 354.7 | 82.68 / 70.84, 154 of 187, 31.37 | 966.0 | 81.29 / 68.79, 94 of 130, 31.46 |
| BC1, `dds` High | 229.1 | 74.80 / 20.55, 154 of 558, 29.27 | 13.3 | 72.25 / -47.93, 5 of 51, 29.27 | 177.2 | 71.98 / -46.34, 5 of 187, 29.98 | 509.8 | 72.91 / -47.35, 6 of 130, 29.87 |
| BC3, `dds` High | 418.0 | 82.34 / 43.12, 341 of 558, 27.32 | 24.6 | 81.50 / 62.82, 33 of 51, 27.29 | 291.7 | 81.70 / 63.89, 130 of 187, 28.70 | 728.8 | 81.16 / 63.02, 81 of 130, 29.14 |
| BC3, `dds` High, colour step | 450.7 | 85.17 / 46.73, 411 of 558, 29.09 | 26.4 | 84.97 / 67.12, 45 of 51, 29.65 | 306.3 | 84.52 / 65.05, 159 of 187, 30.10 | 755.6 | 83.48 / 65.08, 107 of 130, 29.95 |

**Enhanced, bodies under 80:**

| encoder | equipment | NPC small | NPC medium | NPC large |
|---|---:|---:|---:|---:|
| repo, as the converter calls it | 466 of 558, 96.6 % of its pixels | 51 of 51, 100.0 % of its pixels | 187 of 187, 100.0 % of its pixels | 129 of 130, 100.0 % of its pixels |
| fix A (approved, not merged) | 402 of 558, 89.2 % of its pixels | 48 of 51, 98.5 % of its pixels | 184 of 187, 98.7 % of its pixels | 129 of 130, 100.0 % of its pixels |
| fix A + colour | 304 of 558, 70.9 % of its pixels | 38 of 51, 72.6 % of its pixels | 156 of 187, 82.0 % of its pixels | 110 of 130, 87.1 % of its pixels |
| **fix C** (includes fix A) | 174 of 558, 23.8 % of its pixels | 9 of 51, 10.0 % of its pixels | 17 of 187, 8.9 % of its pixels | 14 of 130, 13.8 % of its pixels |
| fix C + colour | 144 of 558, 20.1 % of its pixels | 6 of 51, 4.0 % of its pixels | 11 of 187, 5.4 % of its pixels | 9 of 130, 9.2 % of its pixels |
| fix C, RDO 0.5 | 185 of 558, 26.2 % of its pixels | 12 of 51, 14.8 % of its pixels | 31 of 187, 16.1 % of its pixels | 18 of 130, 17.1 % of its pixels |
| **fix C, RDO 1** | 238 of 558, 40.2 % of its pixels | 17 of 51, 20.0 % of its pixels | 55 of 187, 27.7 % of its pixels | 58 of 130, 57.0 % of its pixels |
| fix C + colour, RDO 1 | 196 of 558, 31.1 % of its pixels | 8 of 51, 6.3 % of its pixels | 33 of 187, 17.1 % of its pixels | 36 of 130, 43.5 % of its pixels |
| BC1, `dds` High | 404 of 558, 91.9 % of its pixels | 46 of 51, 95.6 % of its pixels | 182 of 187, 97.4 % of its pixels | 124 of 130, 96.5 % of its pixels |
| BC3, `dds` High | 217 of 558, 39.0 % of its pixels | 18 of 51, 23.7 % of its pixels | 57 of 187, 28.3 % of its pixels | 49 of 130, 47.1 % of its pixels |
| BC3, `dds` High, colour step | 147 of 558, 22.8 % of its pixels | 6 of 51, 4.6 % of its pixels | 28 of 187, 15.0 % of its pixels | 23 of 130, 22.3 % of its pixels |

- Lowest bodies, **fix C** (includes fix A): EC body 871 (equipment, 43.84, median 552 px); EC body 329 (equipment, 49.15, median 286 px); EC body 1520 (equipment, 49.82, median 187 px); EC body 332 (equipment, 50.47, median 352 px); EC body 1480 (equipment, 51.11, median 506 px); EC body 1481 (equipment, 51.11, median 506 px); EC body 982 (equipment, 51.32, median 136 px); EC body 657 (equipment, 51.65, median 672 px); EC body 1565 (equipment, 51.69, median 336 px); EC body 917 (equipment, 51.83, median 240 px); EC body 1295 (equipment, 52.17, median 504 px); EC body 1529 (equipment, 52.79, median 680 px); EC body 1530 (equipment, 52.79, median 680 px); EC body 327 (equipment, 52.94, median 368 px); EC body 326 (equipment, 53.21, median 350 px)
- Lowest bodies, fix C + colour: EC body 871 (equipment, 47.72, median 552 px); EC body 329 (equipment, 53.19, median 286 px); EC body 1529 (equipment, 54.25, median 680 px); EC body 1530 (equipment, 54.25, median 680 px); EC body 1480 (equipment, 54.58, median 506 px); EC body 1481 (equipment, 54.58, median 506 px); EC body 327 (equipment, 56.28, median 368 px); EC body 674 (equipment, 56.55, median 630 px); EC body 1565 (equipment, 56.81, median 336 px); EC body 527 (equipment, 56.85, median 728 px); EC body 1430 (equipment, 57.65, median 238 px); EC body 982 (equipment, 57.96, median 136 px); EC body 657 (equipment, 57.96, median 672 px); EC body 868 (equipment, 58.37, median 391 px); EC body 1520 (equipment, 58.88, median 187 px)
- Lowest bodies, **fix C, RDO 1**: EC body 871 (equipment, 42.09, median 552 px); EC body 329 (equipment, 48.44, median 286 px); EC body 1480 (equipment, 49.55, median 506 px); EC body 1481 (equipment, 49.55, median 506 px); EC body 1520 (equipment, 49.73, median 187 px); EC body 332 (equipment, 50.16, median 352 px); EC body 657 (equipment, 50.41, median 672 px); EC body 1565 (equipment, 50.79, median 336 px); EC body 917 (equipment, 50.83, median 240 px); EC body 1529 (equipment, 51.00, median 680 px); EC body 1530 (equipment, 51.00, median 680 px); EC body 1295 (equipment, 51.14, median 504 px); EC body 326 (equipment, 51.23, median 350 px); EC body 982 (equipment, 51.42, median 136 px); EC body 327 (equipment, 51.68, median 368 px)
- Lowest bodies, BC1, `dds` High: EC body 261 (NPC small, -47.93, median 2107 px); EC body 273 (NPC large, -47.35, median 13392 px); EC body 16 (NPC medium, -46.34, median 8979 px); EC body 1247 (NPC large, -38.93, median 11880 px); EC body 168 (NPC medium, -36.85, median 8586 px); EC body 158 (NPC medium, -36.11, median 8910 px); EC body 165 (NPC medium, -34.88, median 6150 px); EC body 144 (NPC small, -23.10, median 924 px); EC body 123 (NPC large, -21.92, median 23364 px); EC body 756 (NPC medium, -16.24, median 3330 px); EC body 1246 (NPC large, -14.99, median 13108 px); EC body 777 (NPC medium, -13.58, median 3740 px); EC body 145 (NPC large, -13.50, median 22320 px); EC body 790 (NPC medium, -12.23, median 2989 px); EC body 740 (NPC medium, -4.87, median 7500 px)
- Lowest bodies, BC3, `dds` High: EC body 871 (equipment, 43.12, median 552 px); EC body 917 (equipment, 47.82, median 240 px); EC body 1520 (equipment, 48.44, median 187 px); EC body 329 (equipment, 49.22, median 286 px); EC body 982 (equipment, 49.34, median 136 px); EC body 322 (equipment, 49.74, median 264 px); EC body 940 (equipment, 50.73, median 1360 px); EC body 674 (equipment, 51.16, median 630 px); EC body 657 (equipment, 51.24, median 672 px); EC body 326 (equipment, 51.43, median 350 px); EC body 332 (equipment, 51.60, median 352 px); EC body 327 (equipment, 51.83, median 368 px); EC body 1565 (equipment, 52.37, median 336 px); EC body 1480 (equipment, 52.69, median 506 px); EC body 1481 (equipment, 52.69, median 506 px)
- Lowest bodies, BC3, `dds` High, colour step: EC body 871 (equipment, 46.73, median 552 px); EC body 329 (equipment, 53.70, median 286 px); EC body 940 (equipment, 54.09, median 1360 px); EC body 1529 (equipment, 54.65, median 680 px); EC body 1530 (equipment, 54.65, median 680 px); EC body 674 (equipment, 54.82, median 630 px); EC body 327 (equipment, 55.45, median 368 px); EC body 527 (equipment, 55.48, median 728 px); EC body 982 (equipment, 55.64, median 136 px); EC body 1520 (equipment, 55.79, median 187 px); EC body 1565 (equipment, 55.94, median 336 px); EC body 1480 (equipment, 56.08, median 506 px); EC body 1481 (equipment, 56.08, median 506 px); EC body 326 (equipment, 57.18, median 350 px); EC body 322 (equipment, 57.46, median 264 px)

**Enhanced, hue:**

| encoder | key changed | moved, mean | hued error, mean | worst | hued pixels off by 8 or more |
|---|---:|---:|---:|---:|---:|
| repo, as the converter calls it | 87.01 % | 7.659 | 3.011 | 240 | 21.690 % |
| fix A (approved, not merged) | 86.84 % | 7.192 | 2.825 | 231 | 20.665 % |
| fix A + colour | 85.52 % | 4.722 | 1.836 | 220 | 11.993 % |
| **fix C** (includes fix A) | 85.68 % | 4.523 | 1.754 | 223 | 11.616 % |
| fix C + colour | 85.05 % | 4.010 | 1.551 | 220 | 9.203 % |
| fix C, RDO 0.5 | 86.64 % | 4.807 | 1.866 | 232 | 12.899 % |
| **fix C, RDO 1** | 87.58 % | 5.317 | 2.068 | 232 | 15.426 % |
| fix C + colour, RDO 1 | 87.22 % | 4.892 | 1.902 | 239 | 13.476 % |
| BC1, `dds` High | 91.48 % | 5.972 | 2.335 | 214 | 17.610 % |
| BC3, `dds` High | 91.38 % | 6.809 | 2.660 | 234 | 21.388 % |
| BC3, `dds` High, colour step | 90.55 % | 5.935 | 2.316 | 222 | 17.536 % |

**Enhanced, coverage,** in levels of 16:

| encoder | edge pixels error, mean | worst | edge pixels off by a level or more | solid pixels no longer solid |
|---|---:|---:|---:|---:|
| repo, as the converter calls it | 1.521 | 15.1 | 45.90 % | 17.986 % |
| fix A (approved, not merged) | 1.408 | 15.1 | 43.76 % | 17.213 % |
| fix A + colour | 0.603 | 14.6 | 16.46 % | 10.187 % |
| **fix C** (includes fix A) | 0.628 | 13.6 | 21.13 % | 5.980 % |
| fix C + colour | 0.519 | 13.6 | 13.63 % | 5.141 % |
| fix C, RDO 0.5 | 0.653 | 14.1 | 22.77 % | 6.391 % |
| **fix C, RDO 1** | 0.684 | 14.1 | 24.48 % | 6.713 % |
| fix C + colour, RDO 1 | 0.574 | 14.1 | 17.18 % | 5.988 % |
| BC1, `dds` High | 3.677 | 8.0 | 88.12 % | 0.000 % |
| BC3, `dds` High | 0.211 | 1.7 | 0.91 % | 0.375 % |
| BC3, `dds` High, colour step | 0.211 | 1.7 | 0.91 % | 0.375 % |

### Does any fixed BC7 row beat lossless at the 80 line?

**Enhanced yes, narrowly and only by its median. Classic no.** Fix C is the scratch copy with the lines of section 12, and none of this is in the repo.

| Client | lossless to beat, every body | fixed BC7 row | zstd 19 MB | × lossless | median / p5 / min | at 80 or better |
|---|---|---|---:|---:|---|---:|
| Enhanced | section 10's assignment, 1761.7 MB | **fix C, RDO 1** | 1728.9 | 0.981 | 81.03 / 62.05 / 42.09 | 558 of 926 |
| Enhanced | painted + coverage spans, 1819.9 MB | fix C + colour, RDO 1 | 1796.7 | 0.987 | 82.48 / 66.55 / 45.94 | 653 of 926 |
| Enhanced | painted + coverage spans, 1819.9 MB | fix C, RDO 0.5 | 1803.9 | 0.991 | 83.90 / 63.39 / 43.25 | 680 of 926 |
| Enhanced | both planes in spans, 1881.1 MB | fix C, no RDO | 1900.7 | 1.010 | 85.40 / 64.34 / 43.84 | 712 of 926 |
| Classic | row spans, 733.4 MB | fix C, RDO 1 | 823.2 | 1.122 | 80.20 / 60.25 / 37.57 | 534 of 1022 |
| Classic | row spans, 733.4 MB | fix C, no RDO | 918.4 | 1.252 | 87.51 / 63.77 / 38.67 | 782 of 1022 |

- **Smallest λ:** on Enhanced, **λ 1 under the assignment,** and λ 0.5 under painted + coverage spans. Plain fix C, with no RDO, is 1 % over both planes in spans. λ 2 wasn't run on every body, because at every 40th it falls to a median of 74.81.
- **The median hides 40 %.** At λ 1, 368 of Enhanced's 926 bodies are under 80, and they hold 57 % of NPC large's pixels. Fix C without RDO has 214 under 80. "Beats lossless at the 80 line" is true of the median only, and the margin is 2 %.
- **Equipment is the weak class on every row,** with the lowest minimums. The lowest bodies are all small: Enhanced body 871 has a median frame of 552 px, and Classic's anim4.mul body 655 has 308 px.
- **Classic's BC7 can't reach lossless at 80.** At λ 1 it's 12 % larger than row spans and already at 80.20, so a higher λ would drop under the line before it got there.
- **What BC7 buys on Enhanced isn't disk.** It's half the VRAM (5106 MB against RG8's 9834 MB of pixels) and a faster crowd than the assignment; section 17 has both. What it costs is exact colour, exact coverage and hue by index.
- **The colour step:** on every body it adds 4.7 % on Enhanced and 5.4 % on Classic. It raises the median by 1.74 and 0.38, p5 by 3.52 and 1.90, and min by 3.88 and 5.05. Bodies at 80 go from 712 to 756 and from 782 to 810.
- **Fix A alone isn't enough.** It lifts Enhanced's median from 60.21 to 69.99 and Classic's from 67.39 to 68.37. Fix C takes them to 85.40 and 87.51 for 0.7 % less on Enhanced and 2.6 % more on Classic. Fix A + colour reaches 77.43 and 87.56.

### BC3 and BC1 on every body

**These rows change the answer for a low-end pack, not for the default. On Enhanced, BC3 is the smallest row at 80, at BC7's VRAM. On Classic, BC1 is the smallest, at half of it.** The encoder is `dds` 0.2.0, already a workspace dependency, at `CompressionQuality::High` with no dithering, on the same streams. Section 12's fixes don't touch it. These are size rows: they share cores with other work and aren't timed.

| Client | row, encoder | zstd 19 MB | × lossless | median / p5 / min | at 80 or better | RGB PSNR dB | edge error, levels | hued error |
|---|---|---:|---:|---|---:|---:|---:|---:|
| Enhanced | **BC3, `dds` High** | 1463.1 | 0.831 | 81.72 / 63.38 / 43.12 | 585 of 926 | 28.55 | 0.211 | 2.660 |
| Enhanced | **BC3 + colour, `dds` High** | 1539.0 | 0.874 | 84.52 / 66.43 / 46.73 | 722 of 926 | 29.76 | 0.211 | 2.316 |
| Enhanced | BC7, fix C, RDO 1 | 1728.9 | 0.981 | 81.03 / 62.05 / 42.09 | 558 of 926 | 30.26 | 0.684 | 2.068 |
| Enhanced | BC7, fix C | 1900.7 | 1.079 | 85.40 / 64.34 / 43.84 | 712 of 926 | 31.34 | 0.628 | 1.754 |
| Enhanced | BC7, fix C + colour | 1990.6 | 1.130 | 87.14 / 67.86 / 47.72 | 756 of 926 | 32.51 | 0.519 | 1.551 |
| Enhanced | BC1, `dds` High | 929.5 | 0.528 | 73.75 / 52.00 / −47.93 | 170 of 926 | 29.75 | 3.677 | 2.335 |
| Classic | **BC1, `dds` High** | 514.2 | 0.701 | 82.90 / 63.85 / 41.97 | 695 of 1022 | 29.48 | n/a | 3.566 |
| Classic | BC7, fix C, RDO 1 | 823.2 | 1.122 | 80.20 / 60.25 / 37.57 | 534 of 1022 | 31.35 | n/a | 2.864 |
| Classic | BC7, fix C | 918.4 | 1.252 | 87.51 / 63.77 / 38.67 | 782 of 1022 | 33.20 | n/a | 2.326 |

The lossless column compares Enhanced with section 10's assignment (1761.7 MB) and Classic with row spans (733.4 MB). Against Enhanced's both planes in spans (1881.1 MB), BC3 is 0.778 × and BC3 + colour 0.818 ×. Classic's source alpha is one bit, so it has no edge levels.

- **BC3 beats BC7's RDO row on every SSIMULACRA2 figure.** It's 0.85 × the disk of fix C with RDO 1, with a higher median, p5 and min, and 585 bodies at 80 against 558.
- **BC3 + colour against BC7 with fix C, at the same VRAM:** 0.81 × the disk, 0.88 lower on the median, higher on p5 and min, and 722 bodies at 80 against 712. It leads on equipment (411 against 384 of 558) and NPC small (45 against 42 of 51). Fix C leads on NPC medium (170 against 159 of 187) and NPC large (116 against 107 of 130), and NPC large is about half the disk.
- **BC3 gives up colour and keeps edges.** Against fix C, BC3 + colour is 1.58 dB lower in RGB PSNR, its hued error is 2.316 levels against 1.754, and the hue key changes on 90.55 % of hueable pixels against 85.68 %. Its edge error is a third of fix C's, 0.211 levels against 0.628, and 0.151 % of painted pixels disappear against 0.540 %. One score weighs those together, so the crops matter. **`blocks-preview/` holds them, on Enhanced only:** 30 sheets with lossless, BC3, BC3 + colour, fix C and fix C + colour side by side, from a scratch tool with the same picking rules, and 26 hued sheets. The hued sheets hue every painted pixel, because the strips hold no palette, so they overstate the hued area and their error figures don't compare with this table's.
- **The median still hides bodies.** BC3 has 341 of 926 under 80, and BC3 + colour 204: equipment 147 (22.8 % of its pixels), NPC small 6 (4.6 %), medium 28 (15.0 %), large 23 (22.3 %). Fix C has 214.
- **Classic's BC1 loses no pixels:** its alpha is one bit, so none disappear and none appear. It's 0.70 × row spans on disk, and 1258.7 MB of VRAM against R8's 2419.0 MB. Against BC7 with fix C it gives up 4.61 on the median and 87 bodies at 80, while p5 holds (63.85 against 63.77). 327 of 1022 bodies are under 80: equipment has 211 of them, with 39.8 % of its pixels. Its colour is the weak point, with hued error 3.566 against 2.326.
- **BC1 stays out on Enhanced:** 10.361 % of painted pixels disappear, 88.12 % of edge pixels move by a level or more, the min is −47.93, and only 170 of 926 bodies reach 80.
- **The every-10th sample held.** Enhanced's BC3 went from 1445.4 MB at 82.23 to 1463.1 MB at 81.72, BC3 + colour from 1521.8 MB at 84.66 to 1539.0 MB at 84.52, and Classic's BC1 from 504.2 MB at 83.17 to 514.2 MB at 82.90. Disk rose by 1 to 2 %.
- **Not run on every body:** Classic's BC3, which adds nothing over BC1 on the sample (section 13) at twice the VRAM, and the BC1 block-reuse pass, whose 445.8 MB stays an every-10th figure. **BC3 + colour's decode wasn't timed either.** On BC7 the colour step moved the crowd rows by under 1 ms.

## 17. One table per platform group

**Measure only: the user decides.** Every row keeps one stream per (action, direction), so faced-only loading works on all of them. Each cell comes from a log named in the sections above.

**How to read them:**

- **Disk** is zstd 19. "Every body" rows are section 16's. "Every 10th" rows are a sample scaled by its share of the pixels, and they run a little under the whole set; the BC rows have every body where it was run.
- **VRAM** is resident pixels with padding to whole blocks, with no atlas waste, tables or masks. Section 11 adds those.
- **Decode** is creatures, then dressed NPCs, in ms on 4 threads, five directions / faced only. A block row is zstd in front of the upload. **Every row passes the 250 ms line.**
- **Timing state:** boost off, EPP balance_power, platform profile performance, on the first L3 cluster. BC rows: quiet rerun. Read a row against the plain zstd 19 row of the same table.
- **Quality** is SSIMULACRA2 median / p5 / min on composited frames, per body, then bodies at 80 or better.
- **Hue** is the share of hueable pixels whose key changed, then the mean channel error after a real hue. Enhanced's key is the red byte; Classic's is its top 5 bits, under a full hue.

### Desktop: BC

GTX 900/10 series, Polaris and up, with integrated GPUs held to 60 fps. BC1, BC3, BC7 all take one hardware sample. **GPU cost:** in the same pass, BC7 is 0.71 to 0.80 × nearest index at 1 × and 1.26 to 1.42 × at 1.5 ×, and the indexed filter is 1.43 to 1.57 × BC7. Section 15 has the table; those are ratios on one AMD GPU.

| client | format, encoder | bodies | disk MB | VRAM whole set MB | VRAM creatures, five / faced MB | decode, creatures ms | decode, dressed ms | quality | at 80 | hue |
|---|---|---|---:|---:|---:|---:|---:|---|---:|---|
| Classic | BC1, `dds` High | every body | 514.2 | 1258.7 | 67.8 / 13.2 | 23.3 / 5.3 | 4.1 / 0.9 | 82.90 / 63.85 / 41.97 | 695 of 1022 | 56.19 %, 3.566 |
| Classic | BC1, block reuse λ 1 (`lossy.rs`) | every 10th | 445.8 | 1255.5 | 67.8 / 13.2 | 24.8 / 5.1 | 4.2 / 0.9 | 81.05 / 65.55 / 42.09 | 58 of 103 | 55.25 %, 3.336 |
| Classic | BC3, `dds` High | every 10th | 583.9 | 2510.9 | 135.6 / 26.4 | 35.6 / 7.3 | 6.5 / 1.4 | 83.25 / 64.44 / 39.87 | 73 of 103 | 51.60 %, 3.087 |
| Classic | BC7, repo encoder as it is | every body | 894.7 | 2517.5 | 135.6 / 26.4 | 39.0 / 8.0 | 6.1 / 1.3 | 67.39 / 46.76 / 0.72 | 191 of 1022 | 53.16 %, 4.604 |
| Classic | BC7, fix C (scratch copy) | every body | 918.4 | 2517.5 | 135.6 / 26.4 | 38.5 / 7.9 | 6.0 / 1.3 | 87.51 / 63.77 / 38.67 | 782 of 1022 | 44.18 %, 2.326 |
| Classic | BC7, fix C + colour (scratch copy) | every body | 968.0 | 2517.5 | 135.6 / 26.4 | 38.2 / 7.9 | 5.8 / 1.2 | 87.89 / 65.67 / 43.72 | 810 of 1022 | 42.78 %, 2.222 |
| Classic | BC7, fix C, RDO 1 (scratch copy) | every body | 823.2 | 2517.5 | 135.6 / 26.4 | 38.1 / 7.9 | 5.6 / 1.2 | 80.20 / 60.25 / 37.57 | 534 of 1022 | 50.51 %, 2.864 |
| Classic | anchor: plain planes, zstd 19 |  |  |  |  | 50.0 / 10.3 | 5.9 / 1.3 | lossless |  | exact |
| Enhanced | BC1, `dds` High | every body | 929.5 | 2553.2 | 101.4 / 20.4 | 28.8 / 5.8 | 5.3 / 1.2 | 73.75 / 52.00 / -47.93 | 170 of 926 | 91.48 %, 2.335 |
| Enhanced | BC3, `dds` High | every body | 1463.1 | 5106.4 | 202.8 / 40.8 | 50.4 / 10.2 | 8.8 / 1.9 | 81.72 / 63.38 / 43.12 | 585 of 926 | 91.38 %, 2.660 |
| Enhanced | BC3 + colour, `dds` High | every body | 1539.0 | 5106.4 | 202.8 / 40.8 | not run | not run | 84.52 / 66.43 / 46.73 | 722 of 926 | 90.55 %, 2.316 |
| Enhanced | BC7, repo encoder as it is | every body | 1914.7 | 5106.4 | 202.8 / 40.8 | 49.8 / 10.1 | 8.1 / 1.8 | 60.21 / 42.94 / 4.39 | 93 of 926 | 87.01 %, 3.011 |
| Enhanced | BC7, fix C (scratch copy) | every body | 1900.7 | 5106.4 | 202.8 / 40.8 | 50.9 / 10.3 | 8.4 / 1.8 | 85.40 / 64.34 / 43.84 | 712 of 926 | 85.68 %, 1.754 |
| Enhanced | BC7, fix C + colour (scratch copy) | every body | 1990.6 | 5106.4 | 202.8 / 40.8 | 50.4 / 10.2 | 8.2 / 1.7 | 87.14 / 67.86 / 47.72 | 756 of 926 | 85.05 %, 1.551 |
| Enhanced | BC7, fix C, RDO 1 (scratch copy) | every body | 1728.9 | 5106.4 | 202.8 / 40.8 | 50.9 / 10.2 | 8.1 / 1.7 | 81.03 / 62.05 / 42.09 | 558 of 926 | 87.58 %, 2.068 |
| Enhanced | anchor: plain planes, zstd 19 |  |  |  |  | 100.4 / 20.5 | 14.6 / 3.1 | lossless |  | exact |


- **VRAM of BC3 and BC7 is the same,** 1 byte a pixel; BC1 is half. Whole-set VRAM is the row's own padded block total. Creature VRAM is padded blocks on the same crowd as the decode column.
- **BC3 + colour's decode wasn't timed.** It's a size row on every body. On BC7 the colour step moved the crowd rows by under 1 ms.
- **BC1 drops Enhanced's edges:** section 13 has the coverage error. It loses painted pixels, and its p5 and min show it.
- **Hue:** a block format can't hue by index, so the shader hues from colour, and **no real renderer does that yet.** Section 11 has the masks: Enhanced needs one on 8.2 % of frames, and Classic would on 79.4 % if a block format must hue greys.

### Phones and Apple: ETC2 and ASTC

Every 10th body. **GPU cost isn't measured:** `gpu-cost` has no ETC2 or ASTC rows, and Mali or Apple figures would need those GPUs. VRAM for creatures is padded blocks, with ETC2 RGB8A1 at BC1's bytes and ASTC 4×4 at BC7's. Basis Universal is in section 14: no Basis row is both smaller and better than the plain block format beside it. ETC2 RGBA8, BC3's counterpart at 1 byte a pixel, wasn't measured; ASTC 4×4 is the row at those bytes.

| client | format, encoder | disk MB | VRAM whole set MB | VRAM creatures, five / faced MB | decode, creatures ms | decode, dressed ms | quality | at 80 | hue |
|---|---|---:|---:|---:|---:|---:|---|---:|---|
| Classic | ETC2 RGB8A1, etc2comp effort 40 | 517.3 | 1255.5 | 67.8 / 13.2 | 23.2 / 4.7 | 4.0 / 0.9 | 74.05 / 55.98 / 39.80 | 15 of 103 | 54.98 %, 3.346 |
| Classic | ETC2 RGB8A1, reuse 1 (**a bound**) | 482.1 | 1255.5 | 67.8 / 13.2 | 23.3 / 4.8 | 3.9 / 0.9 | 73.37 / 55.84 / 40.03 | 14 of 103 | 58.05 %, 3.514 |
| Classic | ASTC 4×4, astcenc medium | 1022.2 | 2510.9 | 135.6 / 26.4 | 38.6 / 7.9 | 5.8 / 1.3 | 90.41 / 77.63 / 70.36 | 93 of 103 | 35.48 %, 1.596 |
| Classic | ASTC 6×6, astcenc medium | 525.6 | 1146.1 | 61.2 / 11.9 | 20.3 / 4.2 | 3.7 / 0.8 | 75.28 / 58.12 / 43.93 | 33 of 103 | 56.02 %, 3.478 |
| Classic | ASTC 8×8, astcenc medium | 313.5 | 660.3 | 34.9 / 6.8 | 13.6 / 2.9 | 3.0 / 0.7 | 59.86 / 35.63 / 1.50 | 10 of 103 | 66.61 %, 5.587 |
| Classic | anchor: plain planes, zstd 19 |  |  |  | 50.4 / 10.4 | 5.9 / 1.3 | lossless |  | exact |
| Enhanced | ETC2 RGB8A1, etc2comp effort 40 | 926.9 | 2556.0 | 101.4 / 20.4 | 28.7 / 5.7 | 5.2 / 1.1 | 71.63 / 49.79 / -46.41 | 10 of 93 | 89.36 %, 2.110 |
| Enhanced | ETC2 RGB8A1, reuse 1 (**a bound**) | 864.7 | 2556.0 | 101.4 / 20.4 | 28.8 / 5.8 | 5.1 / 1.1 | 71.25 / 49.98 / -46.41 | 11 of 93 | 90.85 %, 2.233 |
| Enhanced | ASTC 4×4, astcenc medium | 2128.9 | 5112.1 | 202.8 / 40.8 | 49.2 / 10.0 | 7.9 / 1.7 | 87.62 / 70.59 / 55.89 | 78 of 93 | 75.30 %, 1.072 |
| Enhanced | ASTC 6×6, astcenc medium | 1065.0 | 2331.2 | 91.5 / 18.4 | 26.0 / 5.2 | 4.6 / 1.0 | 73.08 / 56.49 / 39.20 | 16 of 93 | 87.66 %, 2.548 |
| Enhanced | ASTC 8×8, astcenc medium | 649.7 | 1346.2 | 52.2 / 10.5 | 16.9 / 3.5 | 3.4 / 0.7 | 57.59 / 41.09 / 28.84 | 9 of 93 | 90.88 %, 3.927 |
| Enhanced | anchor: plain planes, zstd 19 |  |  |  | 102.2 / 20.8 | 16.1 / 3.5 | lossless |  | exact |


### Universal: R8 and RG8

R8 and RG8 exist on every GPU in scope, web and phones included, so this path needs no second pack. **GPU cost:** the shader must gather and blend by hand, at 1.43 to 1.57 × BC7 in the same pass; nearest at integer scale is 1.25 to 1.41 × BC7. Disk is every body with the palette table, with every stream decoded back and compared. VRAM is the expanded rectangle, on each table's own crowd; a row-span layout in linear memory is in section 11. **The crowds differ from the tables above:** these rows use `mobtypes.txt`'s crowds from section 9, 105.6 and 216.6 Mpx, and the assignment uses section 10's placeholder rule. Compare a row with its own client's plain row.

| client | layout on disk | disk MB, every body | VRAM whole set MB | VRAM creatures, five / faced MB | decode, creatures ms | decode, dressed ms | quality | hue |
|---|---|---:|---:|---:|---:|---:|---|---|
| Classic | plain planes, zstd 19 | 761.1 | 2419.0 | 106.8 / 21.4 | 38.7 / 7.7 | 5.8 / 1.3 | lossless | exact, by index |
| Classic | **row spans, zstd 19** | 733.4 | 2419.0 | 106.8 / 21.4 | 37.0 / 7.5 | 6.5 / 1.4 | lossless | exact, by index |
| Classic | row spans, best dictionary a body | 711.1 | 2419.0 | 106.8 / 21.4 | 39.3 / 9.2 | 7.2 / 2.3 | lossless | exact, by index |
| Enhanced | plain planes, zstd 19 | 2013.8 | 9834.0 | 433.4 / 85.2 | 120.1 / 23.3 | 16.5 / 3.3 | lossless | exact, by index |
| Enhanced | **both planes in row spans, zstd 19** | 1881.1 | 9834.0 | 433.4 / 85.2 | 100.3 / 20.5 | 14.1 / 2.9 | lossless | exact, by index |
| Enhanced | painted + coverage spans, zstd 19 | 1819.9 | 9834.0 | 433.4 / 85.2 | 129.0 / 26.3 | 20.0 / 4.4 | lossless | exact, by index |
| Enhanced | section 10's assignment (coverage tANS on all but large) | 1761.7 | 9834.0 | 396.6 / 79.9 | 232.3 / 57.7 | 81.7 / 34.7 | lossless | exact, by index |

## 18. LZ4 HC with a dictionary (scratch bench: `lz4dict`, `timed`)

**Measure only: the user decides.** Every dictionary figure above is zstd's. These are LZ4 HC's. The decoder only looks back 64 KiB, so no dictionary here is larger, and a stream gains nothing from one past its first 64 KiB of output.

The layouts are the ones in the crowd tables: Classic in row spans, Enhanced with both planes in row spans, expanded after decoding, on every body. Every row keeps one stream per (action, direction), each plane its own LZ4 block. HC level 12 unless a row says otherwise. Every stream under rows a to d was decoded back and compared byte for byte, dictionary in its own buffer. On every 10th body the same was done for levels 9, 10 and 11 and for the prefix path of rows b and c.

**How to read them:**

- **Disk** counts the palette table (19.8 and 32.4 MB, the same in every row) and the dictionaries' own bytes. The best-per-body row adds one byte per body for the choice. Row c's dictionary is a stored stream, so it adds nothing. Row d adds its dictionary once per package.
- **Won** is the share of bodies smaller with the dictionary, its bytes included, than without.
- **Decode** is the creature and dressed NPC crowds on 1 / 4 threads, in ms, including expansion. Trained and shared dictionaries are already in memory when the clock starts, so reading them is not in the time, as disk reads aren't in any crowd row. **Read** is the crowd's compressed bytes, dictionaries included: creatures five / faced, dressed five / faced.
- **Apart** keeps the dictionary in its own buffer. **Prefix** copies it directly before the output, and the copy is in the time. `LZ4_decompress_safe_usingDict` in `lz4.c` (lz4 1.10.0) picks its path from the addresses: `dictSize == 0` is the plain decoder; `dictStart + dictSize == dest` is the prefix path (`withPrefix64k` from 64 KiB − 1 up, `withSmallPrefix` below); anything else is `forceExtDict`. Rows b, c and d all decode through this call. Every row makes one fresh output allocation per plane, as the baseline's `lz4::block::decompress` does; a prefix row's is larger by the dictionary.
- **Timing state:** boost off, EPP balance_power, platform profile performance, on the first L3 cluster. **Read a row against row a of the same table.** Same crowds as section 9 (the same expanded bytes), but row a doesn't reproduce section 9's times, in either direction:

| client | row a here: creatures five, faced, dressed five, faced; 1 / 4 threads | section 9 |
|---|---|---|
| Classic | 49.4 / 14.2, 9.8 / 3.0, 9.4 / 2.4, 1.9 / 0.6 | 44.1 / 12.6, 9.2 / 2.7, 10.2 / 2.7, 2.3 / 0.7 |
| Enhanced | 114.3 / 29.4, 23.3 / 6.1, 20.1 / 5.2, 4.3 / 1.1 | 131.7 / 36.4, 28.4 / 7.5, 21.5 / 5.7, 4.5 / 1.2 |

This bench is a separate build with the repo's release profile but not the repo's `.cargo/config.toml` rustflags (no forced frame pointers, mold instead of wild). The cause of that drift isn't known. Between two runs of this bench, a row's one-thread time moved by a median of 0.9 % on Classic and 1.2 % on Enhanced. Don't mix these times with section 9's.

### Classic, row spans, 1022 bodies

| row | disk MB | × no dictionary | won | creatures, five | creatures, faced | dressed, five | dressed, faced | read MB |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| a, no dictionary | 888.4 | 1.0000 |  | 49.4 / 14.2 | 9.8 / 3.0 | 9.4 / 2.4 | 1.9 / 0.6 | 46.9 / 9.3, 4.8 / 1.0 |
| b, trained 4 KiB, apart | 875.8 | 0.9858 | 98.1 % | 57.4 / 16.3 | 11.7 / 3.6 | 11.9 / 3.0 | 2.4 / 0.6 | 47.7 / 10.5, 5.8 / 2.2 |
| b, trained 4 KiB, prefix | 875.8 | 0.9858 | 98.1 % | 49.4 / 14.0 | 10.0 / 3.0 | 9.2 / 2.4 | 1.9 / 0.5 | 47.7 / 10.5, 5.8 / 2.2 |
| b, trained 16 KiB, apart | 863.7 | 0.9722 | 86.5 % | 64.7 / 18.2 | 13.2 / 4.0 | 13.6 / 3.5 | 2.8 / 0.8 | 50.6 / 13.9, 8.8 / 5.4 |
| b, trained 16 KiB, prefix | 863.7 | 0.9722 | 86.5 % | 49.9 / 14.3 | 10.3 / 3.1 | 9.9 / 2.6 | 2.0 / 0.6 | 50.6 / 13.9, 8.8 / 5.4 |
| b, trained 64 KiB, apart | 855.6 | 0.9630 | 54.6 % | 71.2 / 19.9 | 15.0 / 4.4 | 14.3 / 3.6 | 2.7 / 0.8 | 62.7 / 27.2, 14.3 / 11.2 |
| b, trained 64 KiB, prefix | 855.6 | 0.9630 | 54.6 % | 51.8 / 14.8 | 11.0 / 3.3 | 11.2 / 2.9 | 2.3 / 0.7 | 62.7 / 27.2, 14.3 / 11.2 |
| b, best per body including none, apart | 851.1 | 0.9580 | 98.1 % | 71.4 / 19.9 | 15.5 / 4.5 | 14.5 / 3.8 | 2.9 / 0.8 | 60.7 / 25.0, 14.0 / 10.9 |
| b, best per body including none, prefix | 851.1 | 0.9580 | 98.1 % | 51.5 / 14.7 | 10.8 / 3.3 | 11.1 / 2.9 | 2.4 / 0.7 | 60.7 / 25.0, 14.0 / 10.9 |
| c, own standing stream, apart | 873.3 | 0.9829 | 100.0 % | 70.1 / 19.7 | 18.3 / 5.3 | 11.9 / 3.1 | 2.6 / 0.7 | 52.5 / 15.8, 5.0 / 1.2 |
| c, own standing stream, prefix | 873.3 | 0.9829 | 100.0 % | 56.6 / 16.1 | 15.6 / 4.6 | 10.1 / 2.6 | 2.2 / 0.6 | 52.5 / 15.8, 5.0 / 1.2 |
| d, one shared 64 KiB per plane | 872.0 | 0.9815 | 100.0 % | 56.1 / 15.9 | 11.4 / 3.4 | 12.3 / 3.2 | 2.4 / 0.7 | 46.5 / 9.3, 4.6 / 0.9 |
| for scale: zstd 19, no dictionary (disk from section 16) | 733.4 | 0.8255 |  | 167.0 / 46.8 | 33.0 / 9.7 | 24.4 / 6.3 | 5.1 / 1.4 | 39.6 / 7.9, 3.4 / 0.7 |

### Enhanced, both planes in row spans, 926 bodies

| row | disk MB | × no dictionary | won | creatures, five | creatures, faced | dressed, five | dressed, faced | read MB |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| a, no dictionary | 2268.0 | 1.0000 |  | 114.3 / 29.4 | 23.3 / 6.1 | 20.1 / 5.2 | 4.3 / 1.1 | 84.0 / 16.8, 11.0 / 2.2 |
| b, trained 4 KiB, apart | 2239.8 | 0.9876 | 99.7 % | 133.1 / 34.0 | 27.0 / 7.1 | 26.5 / 6.8 | 5.5 / 1.5 | 85.9 / 19.1, 13.1 / 4.6 |
| b, trained 4 KiB, prefix | 2239.8 | 0.9876 | 99.7 % | 114.5 / 29.4 | 23.3 / 6.2 | 19.8 / 5.1 | 4.2 / 1.1 | 85.9 / 19.1, 13.1 / 4.6 |
| b, trained 16 KiB, apart | 2222.7 | 0.9800 | 97.4 % | 146.8 / 37.4 | 30.2 / 8.0 | 29.5 / 7.5 | 5.9 / 1.6 | 92.5 / 26.3, 19.9 / 11.7 |
| b, trained 16 KiB, prefix | 2222.7 | 0.9800 | 97.4 % | 115.9 / 29.6 | 23.9 / 6.3 | 21.1 / 5.5 | 4.4 / 1.2 | 92.5 / 26.3, 19.9 / 11.7 |
| b, trained 64 KiB, apart | 2235.9 | 0.9858 | 69.0 % | 154.2 / 39.6 | 32.5 / 8.6 | 30.6 / 7.9 | 6.0 / 1.6 | 119.2 / 54.0, 41.6 / 33.9 |
| b, trained 64 KiB, prefix | 2235.9 | 0.9858 | 69.0 % | 121.6 / 31.0 | 25.4 / 6.8 | 25.3 / 6.5 | 5.7 / 1.5 | 119.2 / 54.0, 41.6 / 33.9 |
| b, best per body including none, apart | 2220.0 | 0.9788 | 99.7 % | 148.0 / 37.7 | 30.6 / 8.1 | 30.9 / 7.9 | 6.1 / 1.6 | 95.5 / 29.5, 32.2 / 24.3 |
| b, best per body including none, prefix | 2220.0 | 0.9788 | 99.7 % | 116.6 / 29.9 | 24.1 / 6.3 | 23.3 / 6.0 | 5.2 / 1.4 | 95.5 / 29.5, 32.2 / 24.3 |
| c, own standing stream, apart | 2234.8 | 0.9853 | 100.0 % | 163.5 / 41.8 | 47.2 / 12.4 | 27.3 / 7.1 | 6.8 / 1.8 | 99.2 / 33.8, 12.2 / 3.6 |
| c, own standing stream, prefix | 2234.8 | 0.9853 | 100.0 % | 136.6 / 35.1 | 42.1 / 10.9 | 22.2 / 5.7 | 5.9 / 1.5 | 99.2 / 33.8, 12.2 / 3.6 |
| d, one shared 64 KiB per plane | 2203.0 | 0.9713 | 100.0 % | 145.3 / 37.3 | 29.7 / 7.8 | 28.1 / 7.3 | 5.9 / 1.6 | 82.8 / 16.5, 10.4 / 2.1 |
| for scale: zstd 19, no dictionary (disk from section 16) | 1881.1 | 0.8294 |  | 342.1 / 87.0 | 69.5 / 18.0 | 53.6 / 13.5 | 10.9 / 2.8 | 71.0 / 14.2, 8.6 / 1.7 |

### Stream sizes and the 64 KiB window

Per plane stream, so a Classic body has one per (action, direction) and Enhanced two.

| client | plane streams | stored median / p95 KiB | raw median / p95 KiB | share of streams with raw over 64 KiB | share of stored bytes in those streams | share of raw bytes past offset 64 KiB |
|---|---:|---|---|---:|---:|---:|
| Classic | 138426 | 1.1 / 22.0 | 1.5 / 32.6 | 2.6 % | 48.9 % | 32.9 % |
| Enhanced | 316266 | 1.8 / 25.8 | 3.1 / 65.1 | 5.1 % | 48.6 % | 33.7 % |

### Rows c and d in detail

- **Row c, Classic.** The dictionary is the last 64 KiB of the third stored direction of the standing action, on 1017 bodies. 5 bodies have fewer than three, and use the last one. 0 have no standing stream; the rule for those would be the body's longest stream. That stream is stored without a dictionary and costs no disk, but the crowd must read and unpack it whether or not it draws it, and that is in the row's time and read columns. What the crowd reads only for this: creatures 6.3 MB at five directions (282 of 300 pairs), 6.6 MB faced (298 of 300); dressed NPCs 0.3 MB at five directions (300 of 300 pairs), 0.3 MB faced (300 of 300).
- **Row c, Enhanced.** The dictionary is the last 64 KiB of the third stored direction of the standing action, on 923 bodies. 3 bodies have fewer than three, and use the last one. 0 have no standing stream; the rule for those would be the body's longest stream. That stream is stored without a dictionary and costs no disk, but the crowd must read and unpack it whether or not it draws it, and that is in the row's time and read columns. What the crowd reads only for this: creatures 16.3 MB at five directions (274 of 300 pairs), 17.3 MB faced (292 of 300); dressed NPCs 1.4 MB at five directions (300 of 300 pairs), 1.4 MB faced (300 of 300).
- **Row d.** Trained per plane on every n-th stream, with n chosen to give about 200 MiB of samples. Classic has one dictionary of 64 KiB. Enhanced has two, one per plane, so 128 KiB. No per-body reads. Timed apart only.

### Row e: HC levels, no dictionary, every 10th body

Disk has no palette table. Encode is worker seconds summed over threads, on the same CPUs as the crowds. "LZ4 only" is the decompress calls alone on one thread, every stream in the sample, best of 3; "expanded" adds the span expansion. The crowd columns are the same run as the tables above, on the whole crowd, 1 / 4 threads.

| client | level | disk MB | × level 12 | encode worker s | LZ4 only ms | expanded ms | creatures, five | creatures, faced | dressed, five |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Classic | 9 | 70.5 | 1.0030 | 6.1 | 52.9 | 101.5 | 49.6 / 14.2 | 10.0 / 3.0 | 9.0 / 2.3 |
| Classic | 10 | 70.4 | 1.0017 | 9.2 | 53.0 | 101.8 | 49.6 / 14.2 | 10.0 / 3.0 | 9.0 / 2.3 |
| Classic | 11 | 70.3 | 1.0003 | 19.2 | 52.7 | 101.6 | 49.1 / 14.1 | 10.0 / 3.0 | 8.9 / 2.3 |
| Classic | 12 | 70.3 | 1.0000 | 17.7 | 53.0 | 102.1 | 49.5 / 14.0 | 9.9 / 3.0 | 9.0 / 2.3 |
| Enhanced | 9 | 264.2 | 1.0064 | 49.9 | 250.2 | 406.6 | 117.2 / 30.1 | 23.7 / 6.3 | 19.9 / 5.1 |
| Enhanced | 10 | 264.3 | 1.0068 | 65.6 | 250.1 | 409.2 | 117.4 / 30.0 | 23.6 / 6.3 | 19.5 / 5.1 |
| Enhanced | 11 | 263.0 | 1.0015 | 192.3 | 246.7 | 405.1 | 115.4 / 29.6 | 23.4 / 6.3 | 19.6 / 5.2 |
| Enhanced | 12 | 262.6 | 1.0000 | 175.8 | 245.7 | 403.5 | 115.0 / 29.3 | 23.3 / 6.2 | 19.5 / 5.1 |

LZ4's readme says every level shares one decoder, and the times agree. **Decode doesn't depend on the level here:** LZ4 only, the levels sit within 0.6 % of each other on Classic and 1.8 % on Enhanced, where level 12 is the fastest. **So level 12's pack time costs nothing at run time.** Level 12 also encodes faster than level 11 on both clients, and on Enhanced level 10 comes out larger than level 9.

### What the tables show

- **Classic, trained, creatures at five directions on one thread, over row a:** apart +16 / +31 / +44 % at 4 / 16 / 64 KiB; prefix +0 / +1 / +5 %.
- **Enhanced, trained, creatures at five directions on one thread, over row a:** apart +16 / +28 / +35 % at 4 / 16 / 64 KiB; prefix +0 / +1 / +6 %.
- **The 250 ms line:** every row here passes on 4 threads. The slowest cell in these tables is 41.8 ms, at five directions.
- **Sizes:** the smallest row is the best per body on Classic (×0.9580) and the one shared dictionary on Enhanced (×0.9713). A 64 KiB dictionary per body is larger than 16 KiB on Enhanced, because the dictionaries themselves come to 101.9 MB.
- **Against zstd:** zstd's shared dictionary gave ×1.003 to ×1.025 (section 7), so a shared dictionary helps LZ4 and doesn't help zstd. Zstd's best dictionary per body gave ×0.970 on Classic (711.1 against 733.4 MB). Zstd 19 with no dictionary stays smaller than every LZ4 row, at the time shown in each table's last row.
- **The window:** about half the stored bytes are in the few streams over 64 KiB, and a third of the raw bytes lie past offset 64 KiB, where no dictionary reaches.

## Recommendation

The ranking is crowd speed first and size second, with the user's line of 300 new actions in about 250 ms on 4 threads, and faster is better among rows that pass. Section 17 has one table per platform group. **Nothing here is a format change: the user decides.**

1. **Store official animations indexed, lossless, zstd 19, as one stream per (action, direction).** Every GPU in scope reads R8 and RG8, so one pack reaches web, phones, Apple, Windows and Linux. There's no quality question, and hue stays exact because it works on the index.
   - **Classic: row spans, with duplicate streams as pointers.** 706.5 MB on every body, 0.82 × a real BC7 pack and 0.67 × shipped. On creatures it takes 37.0 ms with five directions and 7.5 ms faced, level with the plain rectangle's 38.7 and 7.7 in the same run.
   - **Enhanced: both planes in row spans, with duplicates as pointers.** 1869.6 MB, 0.94 × a real BC7 pack and 0.94 × shipped. On creatures it takes 100.3 / 20.5 ms where the rectangle takes 120.1 / 23.3 in the same run, so it's both smaller and faster.
   - **If disk leads on Enhanced:** painted only + coverage spans is 1808.7 MB, 0.91 ×, at 129.0 / 26.3 ms. Section 10's assignment is 1761.7 MB before duplicates, 0.875 × plain zstd 19, at 232.3 / 57.7 ms. That's inside the line with almost no margin.
   - **Palettes:** one deduplicated table, with hue in the shader. **Load the direction an NPC faces first,** and the other four when it turns: 5 × less decode and VRAM.
   - **Why not BC7 on disk:** on Classic, every BC7 row is larger than lossless, at any quality (RDO 1: 823.2 MB at 80.20, against 733.4 MB). On Enhanced, BC7 with fix C is 1900.7 MB at 85.40, larger than every row above. Only RDO 1 gets under them, at 1728.9 MB, and 368 of 926 bodies are then under 80.
   - **The close call is Enhanced on desktop.** BC3 + colour is 0.82 × both planes in spans on disk and 0.52 × on VRAM, with about half the crowd decode. It takes one hardware sample, and BC1 and BC7 cost 0.60 to 0.64 × the indexed filter there; BC3 itself wasn't in the GPU test. It pays with fidelity: 204 of 926 bodies are under 80, hue has to work from colour, which no real renderer does yet, and phones need another pack. Lossless stays first because it needs one pack and has no quality question. If the user accepts those costs on desktop, it's the row in item 4.
2. **Fix the BC7 encoder, whatever the user decides about animations.** Fix A is in the repo. **Take fix C with it:** it's about 15 lines, it moves disk by −0.7 % on Enhanced and +2.6 % on Classic, and it costs 0.14 worker seconds a Mpx against 0.05. On the shipped samples it lifts `tex_art_ec` from 53.25 to 87.46, `tex_art_cc` from 64.95 to 86.80 and `tex_aux_ec` from 87.34 to 95.09. Section 12 has the exact lines. **The colour step is a separate choice:** on every body it adds 4.7 % on Enhanced and 5.4 % on Classic for +1.74 and +0.38 on the median, and +3.52 and +1.90 on p5. It needs a check first that nothing reads colour under alpha 0 from a decoded page.
3. **Enhanced's VRAM is the real price of item 1,** and the only one. RG8 is 1.93 × BC7: a creature crowd's faced frames are 79.9 MB against 40.8 MB, and five directions 396.6 MB against 202.8 MB. **Start with RG8 and faced-first loading,** because that's simple and lossless, and take one of these only when a target device proves too small:
   - **Row spans held in linear memory:** 0.97 × BC7, lossless, at 1.99 to 2.13 × BC7 on the GPU, in a shader that can't use a hardware sampler.
   - **The low-end pack below:** BC3 or BC7, at the bytes of BC7.
   - Not whole atlas pages, under any layout. They turn 3.0 MB of a dressed crowd's frames into 804 MB.
4. **Offer a low-end pack as a pack-time choice, not as the default.** It keeps the same streams, so faced-first loading still works. Runtime BC7 stays off.
   - **Desktop, Classic: BC1 above 2500 px, lossless R8 below.** 523.5 MB, 0.72 × plain lossless. Coded bodies have a median of 81.77 and a min of 75.52, with 28 of 33 at 80 or better. Whole-set VRAM goes to 1434.2 MB from 2419.0 MB, and the creature crowd's to 69.2 MB from 132.7 MB. Dressed NPCs barely change, because they're small bodies. All-BC1 is 514.2 MB at 82.90 on every body, with 695 of 1022 at 80 (504.2 MB on the sample the mix uses). Its min is 41.97, on small bodies the mix keeps exact.
   - **Desktop, Enhanced: BC3 + colour, or BC7 with fix C. Not BC1.** One-bit alpha takes BC1's min to −47.93, and only 170 of 926 bodies reach 80. At the same VRAM on every body, BC3 + colour is 1539.0 MB at 84.52 / 66.43 / 46.73, with 722 of 926 at 80. BC7 with fix C is 1900.7 MB at 85.40 / 64.34 / 43.84, with 712. **So the original clients' DXT5 is the better choice for this art on disk:** 0.81 × fix C's bytes, and its separate alpha block has a third of the edge error (0.211 levels against 0.628). BC7 keeps colour better (31.34 dB against 29.76, hued error 1.754 against 2.316) and has more large bodies at 80 (116 against 107 of 130). Plain BC3 is 1463.1 MB at 81.72, with 585 of 926. The user should look at both: `blocks-preview/` has them side by side.
   - **Phones and Apple: stay on the universal path unless VRAM forces otherwise.** ASTC 4×4 is the only phone row at 80 or better (90.41 / 87.62), and it's larger than lossless on disk (1022.2 / 2128.9 MB). ASTC 6×6 and ETC2 RGB8A1 halve the disk but score 75.28 / 73.08 and 74.05 / 71.63. Arm's figure of 6×6 beating BC1 by about 1.5 dB doesn't hold on these sprites: it's below BC1 in every class. Crops are in `blocks-preview/`; the user judges those by eye.
   - **What a block format costs besides quality:** hue must work from colour rather than from an index, and **no real renderer does that yet.** Section 13 has hue errors, and section 11 the masks. A second shader path is cheap when most sprites stay indexed, since a per-instance branch was level with grouped pipelines at 100/0 and 90/10.
5. **Keep everything by default, and don't assume shipped data.** See "Custom MUL animations" in section 8: no dropping of unreached art, index 0 flagged per batch, and no trained tables. Classic's row spans treat index 0 as transparent, so a batch where it's opaque needs the flag or the plain rectangle. Enhanced's painted-only layout has the same caveat for coverage 0.
6. **Keep custom 32-bit art and colour-making upscales on BC7 at pack time,** with the fixed encoder, or requantize them into the indexed path when the SSIMULACRA2 floor is acceptable. **No runtime BC7 for now, by the user's decision;** possibly later as an option for upscaled animations.
7. **The runtime:**
   - an existence index per (body, action, direction), so existence is never answered from data;
   - 4 decoding worker threads, with the current direction first;
   - long-lived R8/RG8 atlas pages, filled by streams rather than by whole pages;
   - **one upload per load, not one per frame:** frame by frame takes 4 to 11 × longer, and it's the only thing here that brings a crowd near the line (section 15);
   - **a palette row hued in advance, not a hue read per tap:** 1.10 to 1.15 × a plain index draw at 1 ×, against 1.62 to 1.64 ×;
   - nearest at integer scale, and the manual filter only at fractional scale;
   - **a shared shader by default,** with a per-format pipeline only where one format's sprites can be grouped and they're the cheap ones. Splitting the art shader itself recovers far more: in a scratch rewrite, billboards in the transparent pass go from 56 to 40 VGPRs and from 7754 to 2549 instructions.
8. **Upscaling:** store 1× plus a filter id and filter when a direction loads, for EPX, MMPX and hqx. Store xBRZ if it's wanted: it's too slow to filter at load (268 / 510 ms estimated even for faced directions). MMPX stored indexed is the alternative when disk allows.
9. **Other lossy tiers aren't recommended.** Near-lossless index, reduced coverage, merged palettes, halved frames and AV1 are measured for the user to judge. AV1 is the only large saving, and it fails both the crowd line and colour fidelity. Basis Universal's one gain is a single file for every GPU. No Basis row was both smaller and better than the plain block format beside it, and with five directions on creatures its transcode fails the line on all but one row.

**Still not measured:** any low-end GPU; Mali or Apple GPU cost (`malioc` isn't installed); hue from colour or a second shader path in a real renderer; upload over PCIe to a discrete card; ETC2 RGBA8, BC3's counterpart on phones at 1 byte a pixel.

## Decisions for the user

The user's answers so far, and every option set aside with its reason, are in sections 8 and 9 of
[the requirements](../uddp_anim_format_requirements.md).

| Question | Options | Recommendation |
| --- | --- | --- |
| Storage of official animations | lossless indexed with zstd 19; with LZ4 HC; keep BC7; BC3 | **Lossless indexed:** 0.82 × / 0.94 × a real BC7 pack, passes the crowd line, hue exact. LZ4 HC is 22 % larger and 3–4 × faster if the user prefers speed. On Enhanced desktop, BC3 + colour is the close call: 0.82 × the disk and 0.52 × the VRAM, but lossy |
| Stream layout (sections 9, 16) | rectangle; row spans; Enhanced both planes in row spans; Enhanced painted only + coverage in row spans; section 10's assignment | **Row spans on Classic** (−3.6 %). On Enhanced, **both planes in row spans** (−6.6 %, faster) if speed leads; **painted only + coverage spans** (−9.6 %, about 7 % slower) if disk leads; the assignment (−12.5 %) has almost no margin |
| Duplicate streams (section 9) | none; a pointer to one blob | **Pointer:** −3.7 % on Classic's row spans, −0.6 % on Enhanced, one table lookup |
| Dictionary per body (section 9) | none; best per body | **None by default.** On Classic row spans it's −3.0 % (711.1 MB), at pack time, and it reads 25.6 MB instead of 6.2 MB for a faced crowd. The user may want it for Classic |
| Split boxes (section 9) | one box per frame; up to 2 | **One.** Bytes don't move, and it saves 5 to 7 % of a town's atlas, under 1.5 MB |
| Load unit | whole body (a), per action, or per (action, direction) | **Per (action, direction):** whole-body units fail the line with every lossless candidate but LZ4 HC. Streams cost +12 % / +3 % on disk against whole-body zstd 19 |
| What to load first | all directions, or the faced one | **The faced one:** 5 × less decode and VRAM, and only this passes the worst crowd with zstd 19 |
| BC7 encoder (section 12) | fix A (applied); fix A + C; with the colour step | **Fix A + C.** The colour step is the user's call: about +5 % on disk for a better p5 and min |
| Enhanced VRAM (sections 11, 15) | RG8 (2 bytes a pixel); row spans in linear memory (0.97 × BC7, 2 × its GPU cost); a BC3 or BC7 pack (lossy) | **RG8 with faced-first loading,** with the others kept for a device that proves too small. Not measured on one |
| Low-end pack, Classic | none; BC1; BC1 above a size; BC1 with block reuse | **BC1 above 2500 px, lossless below** (523.5 MB, min 75.52). All-BC1 is 514.2 MB at 82.90 on every body. BC1 block reuse λ 1 is the smallest at 80 on the sample (445.8 MB, 81.05), and the user should view the crops |
| Low-end pack, Enhanced | none; BC3; BC3 + colour; BC7 fix C; BC7 fix C, RDO 1; ASTC 6×6 | **BC3 + colour or BC7 fix C,** at the same VRAM: 1539.0 MB at 84.52 against 1900.7 MB at 85.40. BC3 keeps edges, BC7 keeps colour. Not BC1: its one-bit alpha removes edges |
| Phones and Apple | R8/RG8; ASTC 4×4; ASTC 6×6; ETC2 RGB8A1; Basis | **R8/RG8.** ASTC 4×4 if VRAM forces it. The rest score under 80, so the user judges them by eye |
| Shader paths (section 15) | one pipeline per format; one shader with a branch | **One shader with a branch** in a mostly indexed pack: level at 100/0 and 90/10. It costs 8 to 26 % only when every sprite takes the cheap path |
| Colour-making upscales | requantized indexed (i); RGBA (ii); BC7 (iii) | **(i)** where SSIMULACRA2 stays high; the user picks a floor (e.g. 80) below which an animation falls back to (iii) |
| Other lossy tiers | near-lossless, coverage levels, merged palettes, halved frames, AV1, Basis | **None by default.** The user judges from the sheets in `uodynamapper-scratch/anim-codec/halved/` and `av1/` |
| Unreached art | keep all; an opt-in flag from the install's own files | **Keep all.** The flag waits for tracing the callers (section 7) |
| Keyframe interpolation | exact-shift motion; approximate flow | A separate study; block warp tears on limbs in the halved-frame sheets |

## Dependencies

New dev-dependencies of `udd-conv`, used only by this benchmark:

- `zstd`, the workspace version, for zstd levels and long-distance matching;
- `ssimulacra2` 0.5.1, without default features;
- `jpegxl-rs` 0.14 and `libwebp-sys` 0.14.4, for the image-codec bounds;
- `lz4` 1.28, for LZ4 HC, which `lz4_flex` doesn't have;
- `flate2`, the workspace version, for the contact sheets.

The lockfile gained ssimulacra2 and its dependencies (aligned-vec, av-data, byte-slice-cast, equator, equator-macro,
v_frame, yuvxyb, yuvxyb-math), and lz4 with lz4-sys. The AV1 bound uses `/usr/bin/ffmpeg` (libaom, libdav1d) as an
outside tool, and adds no dependency. The workspace-hack is unchanged (`cargo hakari generate
--diff` is clean), and `just check-dep-alignment` passes.
