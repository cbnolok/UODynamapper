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
| **The recommendation** | **Indexed slot planes, zstd 19, one stream per (action, direction), faced direction first.** Lossless. About 761 MB Classic (0.89× a real BC7 pack, 0.72× shipped) and 2000 MB Enhanced (1.01×, 1.01×). The user decides; section 8 has the alternatives. |
| **300 new actions on 4 threads** (the user's line: about 250 ms; section 8) | Classic / Enhanced: 14.7 / 31.1 ms uniform, or 3.1 / 7.0 ms faced direction only. The 300 largest pairs take 278 / 496 ms, or 58 / 103 ms faced only. |
| Faster, at a size cost | LZ4 HC: 3.9 / 9.7 ms and 80 / 174 ms on the largest, about 22 % larger on disk. |
| Smaller, at a speed cost | Enhanced only: luma + MED + zstd 19 is −9 % and 128 ms; block motion + tANS is −14 % and 239 ms. Rank coding is the smallest (0.80× shipped) and fails the line at 1010 ms. |
| Why is the floor so high? | It isn't per-frame palette noise. Static pixels take 4–5 % of the index bits in most actions; the rest is fine spatial detail at about 6 bits. Enhanced has one palette per action. |
| Redundancy between bodies or clients | None. A solid `--long=31` stream saves 1 %. Classic UOP and Enhanced aren't the same renders (0 % same shape, 1.8 % same RGB555). |
| A real video codec | AV1 CRF 30: 0.57–0.68× BC7 at SSIMULACRA2 84–86, LOSSY. Under 2 % of pixels keep their colour, the palette and hue are gone, and it fails the line at about 0.9 s. |
| Disk, Classic, whole-body units | **Indexed R8, whole batch, zstd 19: 658 MB, 0.84× today's BC7.** The deduplicated palette table adds 19.8 MB. Lossless. |
| Disk, Enhanced, whole-body units | **Indexed RG8, planar, zstd 19: 1921 MB, 1.05× today's BC7.** The deduplicated palette table adds 29.5 MB. Lossless. |
| zstd level, LDM | Level 19 is 7 % smaller than level 9 (Classic) and 6 % smaller (Enhanced). Long-distance matching gains under 1 %. |
| 8×8 block copy, frame difference | **No gain.** Block copy is 0.5 % larger, and the difference from the previous frame is 14–55 % larger. |
| VRAM, Classic | R8 needs 2.42 GB for the whole set, and BC7 2.52 GB. **R8 is no larger than BC7, and it is lossless with hue in the shader.** |
| VRAM, Enhanced | RG8 needs 9.83 GB and BC7 5.11 GB. RG8 is twice the size of BC7. |
| Enhanced at 8 bpp | **No.** 19 % of pixels have partial coverage (per-body median 32 %), and the (index, coverage) pairs fit 255 entries in under 1 % of bodies. |
| Runtime BC7 | About **22–23 Mpx/s per thread**, 19–43× the decode time. That is too slow for the first-draw path, but feasible as a background step after the indexed frames are drawn. It is only worth it for Enhanced. **The user has decided: no runtime BC7 for now,** possibly later as an option for upscaled animations. |
| GPU BC7 | Not worth building now. Classic gains nothing from it, and pack-time BC7 already covers RGBA art. Reconsider only if Enhanced VRAM becomes the limit. |
| 2× upscale | MMPX costs 1.38–1.48× the 1× size and stays indexed. xBRZ/hqx requantized to 256 colours cost 2.9–3.7×, about the same as BC7 (3.1–4.2×), with SSIMULACRA2 above 91 at the median. |
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

- `MOBILE_ANIM_CODEC_SHRINK_PARTS` picks the parts: `a1,a2,a2p,b,w,c,c7,d,e,f,g,k,k1..k8,r,s1,s3,s4,s4q,s5,s6`.
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
  uncompressed. 2291 entries are stored raw and only 225 with zstd, so most BC7 pages were never compressed; 73 % of
  the atlas area is empty. The real pack above has the same 7682.9 MB of pages, all with zstd, and it passed
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
- **A table-driven coverage path.** rANS on coverage saves 5 % of the total but brings the whole decode down to 150 MB/s
  (`z19 + rANS coverage` is 1.024 × BC7). A faster coder for it is untested.
- **Reruns.** The `s1` Enhanced worst case ran separately with its memory fixed: BC7 frames freed before candidates, 6
  encoding threads. Its BC7 rows are from the first run, in the same CPU state. That rebuild includes the custom MUL
  fixes, which change no shipped frame. Two single-thread Python jobs overlapped the earlier `b` and `e2` jobs for about
  a minute each, which doesn't change a ratio. In the first timed `b` table the base rows read 590 and 879 MB/s, against
  769 and 1021 here, so those two were discarded.

## Recommendation

The ranking is crowd speed first and size second, with the user's line of 300 new actions in about 250 ms on 4 threads.
Section 8 has the numbers. **Nothing here is a format change: the user decides.**

1. **Store official animations indexed, lossless, zstd 19, as one stream per (action, direction).**
   - **Classic:** R8 slot plane. **Enhanced:** RG8 as two planes, indices and then coverages. **Palettes:** one
     deduplicated table, with hue in the shader.
   - **Load the direction an NPC faces first, and the other four when it turns.**
   - **Speed on 4 threads, Classic / Enhanced:** uniform crowd 3.1 / 7.0 ms for the faced direction and 14.7 / 31.1 ms
     for all five. The worst crowd is 58 / 103 ms for the faced direction. All five directions miss there, at 278 / 496
     ms, on a crowd of 918 / 1231 Mpx where upload would dominate anyway. Turning costs at most 6.6 / 3.7 ms on the
     uniform crowd and 16.4 ms on the worst one.
   - **Disk:** about 761 MB for Classic (0.97× BC7, 0.89× a real pack, 0.72× referenced shipped) and about 2000 MB for
     Enhanced (1.10×, 1.01×, 1.01×), against the whole-body 677 and 1951 MB.
   - **VRAM:** 5.8 / 21.1 MB for a uniform crowd's faced directions, 28.3 / 100 MB for all five.
   - **It's the fastest candidate that's smaller than a real BC7 pack for Classic and level with it for Enhanced.**
2. **The choices the user may want to take differently,** each one a measured trade against item 1:
   - **LZ4 HC instead of zstd 19:** 3–4× faster, passes the worst crowd with all directions (80 / 174 ms), about 22 %
     larger on disk. A per-stream codec id would allow it per body.
   - **Enhanced only, luma + MED + zstd 19:** −9 % on disk (0.92× shipped), 4× slower (128 ms), still passes the uniform
     crowd. Block motion with tANS is −14 % (0.86× shipped), 7.7× slower (239 ms), and has almost no margin.
   - **Not worth considering:** rank coding fails the line, even though it's the smallest (0.80× shipped).
3. **Keep everything by default, and don't assume shipped data.** See "Custom MUL animations" in section 8: no dropping
   of unreached art, index 0 flagged per batch, and no trained tables.
4. **Keep custom 32-bit art and colour-making upscales on BC7 at pack time,** or requantize them into the indexed path
   when the SSIMULACRA2 floor is acceptable. **No runtime BC7 for now, by the user's decision;** possibly later as an
   option for upscaled animations.
5. **The runtime:**
   - an existence index per (body, action, direction), so existence is never answered from data;
   - 4 decoding worker threads, with the current direction first;
   - long-lived R8/RG8 atlas pages with hue in the shader;
   - measure upload next: no run here includes it.
6. **Upscaling:** store 1× plus a filter id and filter when a direction loads, for EPX, MMPX and hqx. Store xBRZ if it's
   wanted: it's too slow to filter at load (268 / 510 ms estimated even for faced directions). MMPX stored indexed is
   the alternative when disk allows.
7. **Lossy tiers aren't recommended.** Near-lossless index, reduced coverage, merged palettes, halved frames and AV1 are
   measured for the user to judge. AV1 is the only large saving, and it fails both the crowd line and colour fidelity.

## Decisions for the user

| Question | Options | Recommendation |
| --- | --- | --- |
| Storage of official animations | indexed slot planes with zstd 19; with LZ4 HC; with luma + MED; keep BC7 | **Indexed zstd 19:** lossless, hue in the shader, 0.97× / 1.10× BC7, passes the crowd line |
| Load unit | whole body (a), per action, or per (action, direction) | **Per (action, direction):** on the uniform crowd, whole-body units fail the line with every lossless candidate but LZ4 HC. Streams cost +12 % / +3 % on disk against whole-body zstd 19 |
| What to load first | all directions, or the faced one | **The faced one:** 4–5× less decode and VRAM, and only this passes the worst crowd with zstd 19 |
| Enhanced size against speed | zstd 19 (1.01× shipped, 31 ms); luma + MED (0.92×, 128 ms); block motion (0.86×, 239 ms) | **zstd 19,** since speed outranks size and it isn't close; the user may value the 9 % |
| Enhanced VRAM | RG8 (16 bpp); background runtime BC7 (off by the user's decision); split 8+4 bpp pages (not measured) | **RG8,** with faced directions keeping crowds small |
| Colour-making upscales | requantized indexed (i); RGBA (ii); BC7 (iii) | **(i)** where SSIMULACRA2 stays high; the user picks a floor (e.g. 80) below which an animation falls back to (iii) |
| Lossy tiers | near-lossless, coverage levels, merged palettes, halved frames, AV1 | **None by default.** The user judges from the sheets in `uodynamapper-scratch/anim-codec/halved/` and `av1/` |
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
