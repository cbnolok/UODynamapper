# Mobile animation storage: measured candidates (2026-11-01)

Status: **measurement only, for the user's decision**. No package format or renderer change has been made.

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

- **Classic pixels** are one palette index each; index 0 means transparent, since no shipped pixel uses it.
- **Enhanced pixels** are an index plus a coverage value from 0 to 16 (sixteenths), stored as RG8.
  - **Interleaved** stores the two bytes of each pixel side by side.
  - **Planar** stores all the indices of the batch first, then all the coverages.

## Summary

| Question | Answer |
| --- | --- |
| Disk, Classic | **Indexed R8, whole batch, zstd 19: 658 MB, 0.84× today's BC7.** The deduplicated palette table adds 19.8 MB. Lossless. |
| Disk, Enhanced | **Indexed RG8, planar, zstd 19: 1921 MB, 1.05× today's BC7.** The deduplicated palette table adds 29.5 MB. Lossless. |
| zstd level, LDM | Level 19 is 7 % smaller than level 9 (Classic) and 6 % smaller (Enhanced). Long-distance matching gains under 1 %. |
| 8×8 block copy, frame difference | **No gain.** Block copy is 0.5 % larger, and the difference from the previous frame is 14–55 % larger. |
| VRAM, Classic | R8 needs 2.42 GB for the whole set, and BC7 2.52 GB. **R8 is no larger than BC7, and it is lossless with hue in the shader.** |
| VRAM, Enhanced | RG8 needs 9.83 GB and BC7 5.11 GB. RG8 is twice the size of BC7. |
| Enhanced at 8 bpp | **No.** 19 % of pixels have partial coverage (per-body median 32 %), and the (index, coverage) pairs fit 255 entries in under 1 % of bodies. |
| Runtime BC7 | About **21 Mpx/s per thread**, 19–42× the decode time. That is too slow for the first-draw path, but feasible as a background step after the indexed frames are drawn. It is only worth it for Enhanced. |
| GPU BC7 | Not worth building now. Classic gains nothing from it, and pack-time BC7 already covers RGBA art. Reconsider only if Enhanced VRAM becomes the limit. |
| 2× upscale | MMPX costs 1.38–1.48× the 1× size and stays indexed. xBRZ/hqx requantized to 256 colours cost 2.9–3.7×, about the same as BC7 (3.1–4.2×), with SSIMULACRA2 above 91 at the median. |
| Motion | An exact 8×8 shift between neighbouring frames is rare: 3 % of blocks (Classic) and under 1 % (Enhanced). Keyframe interpolation would have to be approximate. |
| Load unit | A whole-body batch takes a median of 1.1 ms (Classic) or 5.5 ms (Enhanced) to decode, and up to 215 / 364 ms. A per-action batch shows its first frame in a median of 41 / 104 µs, with a maximum of 6 / 10 ms. **Provisional timings**, see [Timing conditions](#timing-conditions). |

## Timing conditions

All timings are on one thread of an AMD Ryzen AI 9 365, with **boost off and EPP power**. Each time is the fastest
of 3 runs, and the files are already in memory.

> **Provisional.** The run below shared the machine with the renderer's full Britannia web-map export.
> The load average was 15.13 / 20.33 / 28.23 at the start of the timing pass and 1.04 / 4.80 / 13.16 at its end.
> A timing-only re-run, started with a load under 2 after the export ends, will replace these figures.

## 1. Sizes

The whole set is 1022 Classic bodies (996,620 frames, 2419 Mpx trimmed) and 926 Enhanced bodies (1,572,711 frames,
4917 Mpx trimmed).

"BC7 + zstd 7" estimates today's packer. It compresses each frame padded to whole 4×4 blocks with the container's
zstd level 7, and it doesn't count the atlas gutters. The installed packages are a little larger: 857 MB for
Classic and 1.99 GB for Enhanced.

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

This uses 200 bodies per client, spread by pixel count, on one thread. **Boost off, EPP power; provisional
(contended, see [Timing conditions](#timing-conditions)).** MB/s counts the decoded output.

### Classic (boost off, EPP power; provisional)

| step | median µs | p95 µs | max µs | MB/s |
|---|---:|---:|---:|---:|
| BC7 + zstd 7 (zstd only) | 741 | 8773 | 151968 | 973 |
| RGBA8 + zstd 7 | 2189 | 42334 | 485563 | 1067 |
| indexed, per frame + zstd 9 | 3045 | 13943 | 119147 | 471 |
| indexed, batch + zstd 9 | 804 | 9039 | 171608 | 836 |
| indexed, batch + zstd 19 | 1060 | 10866 | 215037 | 670 |
| indexed, batch + zstd 19 + LDM | 1050 | 11046 | 229122 | 660 |
| indexed + block copy + zstd 19 (zstd + rebuild) | 1955 | 32448 | 590998 | 268 |
| indexed + difference + zstd 19 (zstd + rebuild) | 2661 | 35953 | 555802 | 244 |
| indexed batch → RGBA8 expand (no zstd) | 524 | 6027 | 69219 | 5725 |
| runtime BC7 (Analytical): expand + encode | 45289 | 438850 | 4252282 | 20.7 Mpx/s |
| runtime BC7 (AnalyticalWide): expand + encode | 44271 | 385578 | 4336922 | 20.9 Mpx/s |

### Enhanced (boost off, EPP power; provisional)

| step | median µs | p95 µs | max µs | MB/s |
|---|---:|---:|---:|---:|
| BC7 + zstd 7 (zstd only) | 2706 | 19013 | 161261 | 991 |
| RGBA8 + zstd 7 | 7490 | 86559 | 556270 | 1042 |
| indexed, per frame + zstd 9 | 11999 | 39593 | 223265 | 670 |
| indexed, batch + zstd 9 | 4539 | 35161 | 347600 | 963 |
| indexed, batch + zstd 19 | 5463 | 39952 | 363565 | 850 |
| indexed, batch + zstd 19 + LDM | 5465 | 40499 | 375510 | 838 |
| indexed + block copy + zstd 19 (zstd + rebuild) | 9848 | 95535 | 835029 | 400 |
| indexed + difference + zstd 19 (zstd + rebuild) | 11893 | 118200 | 903036 | 332 |
| indexed batch → RGBA8 expand (no zstd) | 5310 | 33366 | 205352 | 2098 |
| runtime BC7 (Analytical): expand + encode | 140676 | 719383 | 3902086 | 21.7 Mpx/s |
| runtime BC7 (AnalyticalWide): expand + encode | 135938 | 681998 | 3673335 | 22.6 Mpx/s |

Only the interleaved layout was timed. Planar is one zstd stream of about the same size, so it should decode at
about the same speed (inferred, not measured).

### Load unit: whole body or one action (boost off, EPP power; provisional)

The first frame is drawable once the batch that holds it is decoded. For a per-action batch, that is the body's
lowest action.

| client | unit | stored as | compressed MB (sample) | per batch decode median / p95 µs | first frame drawable median / p95 / max µs |
|---|---|---|---:|---|---|
| Classic | body | indexed zstd 19 | 134.5 | 1060 / 10866 | 1060 / 10866 / 215037 |
| Classic | action | indexed zstd 19 | 148.2 | 19 / 397 | 41 / 1059 / 6314 |
| Classic | body | BC7 zstd 7 | 160.6 | 741 / 8773 | 741 / 8773 / 151968 |
| Classic | action | BC7 zstd 7 | 178.0 | 20 / 270 | 40 / 749 / 4185 |
| Enhanced | body | indexed zstd 19 | 440.0 | 5463 / 39952 | 5463 / 39952 / 363565 |
| Enhanced | action | indexed zstd 19 | 441.8 | 57 / 1272 | 104 / 2568 / 10125 |
| Enhanced | body | BC7 zstd 7 | 387.2 | 2706 / 19013 | 2706 / 19013 / 161261 |
| Enhanced | action | BC7 zstd 7 | 405.2 | 37 / 500 | 56 / 1115 / 4548 |

These are decode times only. A whole-body batch also has to be uploaded before its first frame shows: up to
279 MB (RG8) for the largest Enhanced body, and 41.5 MB at p95.

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
40 ms at p95 and 364 ms at most to its first frame, against 2.6 ms and 10 ms for one action.

## 3. Runtime BC7

The test encodes BC7 from the indexed batch with our encoder at its fastest settings: no RDO, both front-ends. The
encoders have no other speed setting. It takes about **21–23 Mpx/s on one thread**: a median of 44 ms per Classic
body and 136 ms per Enhanced body, with a maximum of 4.3 s / 3.7 s. That is 32× (Classic) and 19× (Enhanced) the
batch zstd 19 decode (the benchmark's ratio over the sample totals; the medians give 42× and 25×). The expand to
RGBA8 alone is 0.5 ms / 5.3 ms at the median.

Is it viable while streaming?

- **Not on the first-draw path.** One mid-size Enhanced body would stall the frames that wait for it by 100+ ms.
- **Feasible as a background step**: draw the indexed pages first, then re-encode and swap them in once they are
  warm. Spread over 8 worker threads, a crowd of 20 median Enhanced bodies (44 Mpx) takes about 0.25 s.
- **It only pays off for Enhanced.** For Classic, BC7 is no smaller than R8 (2.52 vs 2.42 GB), so runtime BC7 gains
  nothing. For Enhanced it halves RG8.
- **BC7 bakes one palette.** A runtime BC7 page loses hue-in-shader for its hueable pixels, unless those pixels stay
  indexed or each hue is encoded separately.

**GPU compute BC7: not worth trying now.** Published compute encoders are one to two orders of magnitude faster
than 21 Mpx/s per thread, but the only gain on the table is halving Enhanced VRAM. The cheaper option is not
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

## Recommendation

1. **Store official animations indexed, lossless.**
   - **Classic:** R8 with index 0 transparent.
   - **Enhanced:** RG8 planar (all indices, then all coverages), with zstd 19 at pack time.
   - **Palettes:** one deduplicated table. Hue applies to the palette in the shader.
   - **The cost:** about 677 MB for Classic (with palettes; −16 % against BC7) and about 1.95 GB for Enhanced (+5 %).
     Both are lossless, against today's lossy BC7.
   - **Not worth it:** long-distance matching, block copy and frame differences.
2. **Keep custom 32-bit art and colour-making upscales on BC7 at pack time.** Alternatively, requantize them into
   the indexed path (option i) when the SSIMULACRA2 floor is acceptable.
3. **Load unit: store per-action zstd streams laid out back to back per body.** One read still fetches the whole
   body, so choice (a) holds at the I/O level. Decoding and upload then go action by action, the current action
   first.
   - **The gain:** the first frame is drawable in microseconds instead of up to 0.2–0.4 s, and only the actions in
     use need to be resident.
   - **The disk cost:** +0.3 % for Enhanced and +11 % for Classic.
   - **Not measured:** merging a body's small Classic actions into blocks of a minimum size would recover most of
     the Classic cost.
4. **The runtime:**
   - an existence index per (body, action, direction), so existence is never answered from data;
   - decoding on worker threads;
   - long-lived R8/RG8 atlas pages with hue in the shader.
   Runtime BC7 is an optional later background step for Enhanced only. No GPU encoder.
5. **Upscaling:** MMPX (1.4–1.5×) stays indexed and exact. xBRZ/hqx is stored requantized (≈3×) or as BC7 (≈3–4×).

## Decisions for the user

| Question | Options | Recommendation |
| --- | --- | --- |
| Storage of official animations | indexed (R8 / RG8 planar) zstd 19; or keep BC7 | **Indexed**: lossless, −16 % / +5 % on disk, hue in the shader |
| Load unit | whole-body batch (a), as chosen; or per-action streams grouped per body | **Per-action streams, grouped per body**: same single read, first frame drawable in µs, +11 % Classic disk |
| Enhanced VRAM | RG8 (16 bpp); background runtime BC7 (8 bpp, hue baked); split 8+4 bpp pages (not measured) | **RG8 first**, and measure resident use in crowds before adding either |
| GPU BC7 encoder | try or skip | **Skip** for now |
| Colour-making upscales | requantized indexed (i); RGBA (ii); BC7 (iii) | **(i)** where SSIMULACRA2 stays high; the user picks a floor (e.g. 80) below which an animation falls back to (iii) |
| Keyframe interpolation | exact-shift motion; approximate flow | Exact motion is too rare (≤ 3 %); approximate flow only, as a separate study |

## Dependencies

New dev-dependencies of `udd-conv`, used only by this benchmark:

- `zstd`, the workspace version, for zstd levels and long-distance matching;
- `ssimulacra2` 0.5.1, without default features.

The lockfile gained ssimulacra2 and its dependencies (aligned-vec, av-data, byte-slice-cast, equator, equator-macro,
v_frame, yuvxyb, yuvxyb-math). The workspace-hack is unchanged (`cargo hakari generate --diff` is clean), and
`just check-dep-alignment` passes.
