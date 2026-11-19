# Atlas page compression

Why an Enhanced art page took 75–90 ms to load, which codecs could replace Zstd 19 for the lossless atlas pages, and
what compressing a page in independently decoded strips gains and costs (2026-11-19).

## Where the time goes

Profiling the KR Britain walk showed 20 % of the CPU in Zstd, on the Enhanced art pages. A page of `tex_art_ec.uddp`
is 4096×2048 RGBA8888, 32 MiB raw, stored with plain Zstd 19 (`LOSSLESS_PAGE_ZSTD_LEVEL`). Reading one took 72–90 ms,
against 2 ms for copying the same bytes, so the decode is the whole cost. Three things add up:

1. **Enhanced art compresses poorly.** Painted art changes colour at almost every pixel, so Zstd decodes many short
   matches and literals: 4.5–5.3:1 at 430–450 MB/s, against 12:1 at 1.5 GB/s for the Classic art pages.
2. **Level 19 decodes slower than lower levels** on these pages (75 ms against 50 ms at level 7), for files about 20 %
   smaller.
3. **A page was one Zstd frame**, which only one thread can decode.

## Codecs compared

One page, decoded on one thread (best of 3), on a 20-thread machine. Encode times are single-threaded. Runs vary by
about 15 % between sessions (zstd 19 on Enhanced page 0 measured 75 ms and 91 ms).

**Enhanced art page 0** (32.0 MiB raw):

| Codec | Size (MiB) | Ratio | Decode (ms) | MB/s | Encode (ms) |
|---|---:|---:|---:|---:|---:|
| memcpy | | | 1.7 | | |
| zstd 1 | 11.01 | 2.9 | 56 | 594 | |
| zstd 3 | 10.04 | 3.2 | 57 | 593 | |
| zstd 7 | 9.27 | 3.5 | 50 | 676 | 633 |
| zstd 12 | 8.96 | 3.6 | 49 | 686 | |
| **zstd 19 (shipped)** | **7.16** | **4.5** | **75** | **450** | **17 374** |
| lz4 | 17.16 | 1.9 | 26.5 | 1266 | 104 |
| lz4 hc 9 | 11.98 | 2.7 | 24.4 | 1374 | 1119 |
| lz4 hc 12 | 11.91 | 2.7 | 24.2 | 1384 | 3785 |
| zlib-ng 6 | 9.66 | 3.3 | 104 | 322 | 589 |
| zlib-ng 9 | 9.54 | 3.4 | 99 | 338 | 1933 |
| libdeflate 9 | 9.60 | 3.3 | 71 | 472 | 828 |
| libdeflate 12 | 8.92 | 3.6 | 71 | 475 | 10 374 |
| brotli q9, window 22 | 8.17 | 3.9 | 168 | 200 | 7249 |
| brotli q11, window 24 | 6.26 | 5.1 | 187 | 179 | 88 024 |
| bzip2 9 | 8.17 | 3.9 | 1429 | 23 | 2885 |
| xz 6 | 5.91 | 5.4 | 405 | 83 | 20 337 |
| xz 9 | 5.80 | 5.5 | 420 | 80 | 23 379 |

**Enhanced art page 4** (32.0 MiB raw):

| Codec | Size (MiB) | Ratio | Decode (ms) |
|---|---:|---:|---:|
| zstd 7 | 7.81 | 4.1 | 59 |
| **zstd 19** | **6.02** | **5.3** | **78** |
| lz4 hc 9 | 10.38 | 3.1 | 33.5 |
| zlib-ng 9 | 8.29 | 3.9 | 105 |
| xz 9 | 4.73 | 6.8 | 365 |

**Classic art page 0** (15.6 MiB raw):

| Codec | Size (MiB) | Ratio | Decode (ms) | MB/s |
|---|---:|---:|---:|---:|
| **zstd 19** | **1.31** | **11.9** | **10** | **1567** |
| zstd 7 | 1.77 | 8.8 | 11 | 1529 |
| lz4 hc 12 | 2.63 | 5.9 | 5.2 | 3149 |
| zlib-ng 9 | 2.25 | 6.9 | 20 | 803 |
| libdeflate 12 | 2.09 | 7.5 | 18 | 906 |
| brotli q11, window 24 | 1.17 | 13.3 | 31 | 532 |
| bzip2 9 | 1.48 | 10.6 | 351 | 47 |
| xz 9 | 1.15 | 13.6 | 87 | 188 |

- **LZ4 HC** is the only codec that decodes faster: 2–3× faster, for files 1.7–2× larger. Level 12 compresses no better
  than level 9.
- **zlib-ng, libdeflate** are as slow as Zstd 19 or slower, for files 25–35 % larger.
- **Brotli q11** is 12 % smaller at 2.5× the decode time and 5× the encode time. At q9 it is larger and slower than Zstd.
- **xz** is about 20 % smaller and decodes 5–8× slower.
- **bzip2** decodes 20–35× slower.
- **A PNG-style filter before Zstd** (each byte minus the pixel to the left or above) made the pages 10 % (left) to
  100 % (above, Classic) larger and saved at most a few milliseconds: the atlas has large transparent areas and repeated
  sprites, and the filter breaks the long matches Zstd lives on.
- Not measured: Oodle Kraken and Leviathan (the right trade-off, but proprietary), lossless image codecs such as WebP and
  JPEG XL lossless (smaller, decoding at 50–200 MB/s), GPU decompression (GDeflate, nvCOMP; not available through wgpu).

## Strips

Every codec trades size for decode speed, and Zstd 19 has the best balance of the ones above. What makes the page slow
is that one thread decodes it. Compressed as independent Zstd frames, one per strip of rows, a page decodes on several
threads. The cost is size: level 19 earns its ratio with an 8 MiB window, and a strip cannot match bytes outside itself.

Zstd 19, decoded on 20 threads (best of 5):

| Page | One frame | 128-row strips (16) | 256-row strips (8) | 512-row strips (4) |
|---|---:|---:|---:|---:|
| Enhanced art page 0 | 7.16 MiB, 75–90 ms | +9.2 %, 9.4 ms | +6.8 %, 13.4 ms | +4.0 %, 23.3 ms |
| Enhanced art page 4 | 6.02 MiB, 78–96 ms | +12.0 %, 10.4 ms | +9.0 %, 14.1 ms | +5.2 %, 25.5 ms |
| Classic art page 0 | 1.31 MiB, 10 ms | +35.5 %, 2.1 ms | +25.0 %, 2.5 ms | +16.2 %, 4.4 ms |
| Classic art page 10 | 1.45 MiB, 10 ms | +23.7 %, 2.1 ms | +16.4 %, 2.4 ms | +10.7 %, 4.1 ms |

At level 7 the strips cost much less (+1.1 % and +2.6 % on the two Enhanced pages in 16 strips), because level 7 does
not reach far back either; it was this first measurement that suggested strips were almost free.

### Levels and codecs in strips

A strip costs level 19 more than lower levels, but lower levels start further behind, so in 128-row strips every
level from 3 to 15 is still 26–47 % larger than one level 19 frame. Size is against one Zstd 19 frame; "CPU" is every
strip decoded on one thread, which is what a page costs the machine whatever the threads do; the thread columns are
the wall time of one page (best of 5).

| 128-row strips | EC art 0 | EC art 4 | EC land 0 | CC art 0 | CPU, EC art 0 | 4 threads | 8 threads |
|---|---:|---:|---:|---:|---:|---:|---:|
| One zstd 19 frame (today) | 7.16 MiB | 6.02 MiB | 2.88 MiB | 1.31 MiB | 73 ms | | |
| zstd 3 | +40.6 % | +41.5 % | +37.4 % | +69.2 % | 54 ms | 15.0 ms | 7.7 ms |
| zstd 7, 9, 12, 15 | +29–31 % | +32–33 % | +26–28 % | +46–52 % | 50 ms | 13 ms | 7.3 ms |
| zstd 16 | +14.7 % | +18.2 % | +14.6 % | +40.5 % | 65 ms | 17.2 ms | 9.2 ms |
| zstd 17 | +9.0 % | +13.9 % | +8.9 % | +39.9 % | 72 ms | 18.8 ms | 10.5 ms |
| **zstd 19** | **+9.2 %** | **+12.0 %** | **+9.7 %** | **+35.5 %** | **77 ms** | **20.0 ms** | **10.8 ms** |
| zstd 22 | +9.1 % | +11.9 % | +9.7 % | +35.4 % | 77 ms | 19.9 ms | 11.0 ms |
| zstd 19, minimum match 6 | +18.8 % | +20.9 % | +19.6 % | +36.7 % | 62 ms | 16.4 ms | 8.7 ms |
| zstd 19, raw literals | +10.5 % | +14.7 % | +11.0 % | +39.2 % | 74 ms | 19.4 ms | 10.7 ms |
| lz4 hc 12 | +67.3 % | +72.3 % | +63.8 % | +101.9 % | 24.5 ms | 6.3 ms | 3.3 ms |
| libdeflate 12 | +25.4 % | +30.3 % | +20.0 % | +59.4 % | 71 ms | 17.9 ms | 9.2 ms |
| brotli q11, window 22 | −5.9 % | −2.4 % | −6.7 % | +22.3 % | 204 ms | 51.5 ms | 29.5 ms |
| xz 9 | −12.1 % | −7.0 % | −12.5 % | +20.1 % | 460 ms | 115 ms | 63 ms |

- **Levels 7 to 15 are one setting** for these pages: same size, same speed. They save a third of the CPU for 30 %
  more disk. Level 16 (the first optimal-parsing level) is the knee, and level 17 matches 19 within 2 % at 6–8 % less
  CPU. 18 and 22 add nothing over 19.
- **Zstd parameters** don't beat a level: a longer minimum match is level 16 again, and raw literals are slower and larger.
- **Brotli and xz** are the only codecs smaller than today in strips, at 3× and 6× the CPU.
- **Strips don't lower the CPU**, they spread it: a page costs the same 72–77 ms of decode on any number of threads.
  They help when cores are idle, and do nothing for a loader whose threads are all busy, such as a crowd of
  animations, which already decodes many streams side by side.

The reader decodes a payload's frames on a pool of its own, of at most 8 threads, built on first use, so no decode
starts threads and the cap holds whatever else uses rayon. On 8 threads, 16 strips (128 rows) take 10–11 ms for an
Enhanced art page, and 8 strips (256 rows, one per thread) about 13–14 ms.

## Block-compressed pages

The same pages encoded to BC1 and BC7 (the packer's `AnalyticalWide` encoder; RDO λ 1 uses the default look-back), at
their size and upscaled 2× with Lanczos3 over the whole page, which stands in for per-sprite upscaling. 16 strips,
8 threads.

**Enhanced art page 0** (4096×2044):

| Format | Raw | zstd 19, one frame | Decode | Strips | 8 threads | lz4 hc 12 | Decode |
|---|---:|---:|---:|---:|---:|---:|---:|
| RGBA (today) | 32.0 MiB | 7.16 MiB | 73 ms | +9.2 % | 10.8 ms | 11.97 MiB | 24 ms |
| BC7 | 8.0 MiB | 5.36 MiB | 14 ms | +5.0 % | 1.9 ms | 6.18 MiB | 3.8 ms |
| BC7, RDO 1 | 8.0 MiB | 4.91 MiB | 14 ms | +2.9 % | 1.8 ms | 5.64 MiB | 3.5 ms |
| BC1 | 4.0 MiB | 2.58 MiB | 7 ms | +4.2 % | 1.0 ms | 2.90 MiB | 2.0 ms |
| RGBA 2× | 127.8 MiB | 69.93 MiB | 312 ms | +2.5 % | 64 ms | 92.93 MiB | 73 ms |
| BC7 2× | 31.9 MiB | 21.46 MiB | 65 ms | +4.9 % | 8.4 ms | 25.17 MiB | 15 ms |
| BC7 2×, RDO 1 | 31.9 MiB | 18.73 MiB | 62 ms | +2.6 % | 7.6 ms | 21.60 MiB | 13 ms |
| BC1 2× | 16.0 MiB | 8.46 MiB | 34 ms | +6.4 % | 4.2 ms | 10.59 MiB | 7.9 ms |

**Classic art page 0** (2024×2024):

| Format | Raw | zstd 19, one frame | Decode | Strips | 8 threads | lz4 hc 12 | Decode |
|---|---:|---:|---:|---:|---:|---:|---:|
| RGBA (today) | 15.6 MiB | 1.31 MiB | 10 ms | +35.5 % | 2.4 ms | 2.65 MiB | 5.1 ms |
| BC7 | 3.9 MiB | 1.14 MiB | 4.0 ms | +23.9 % | 0.6 ms | 1.58 MiB | 1.2 ms |
| BC7, RDO 1 | 3.9 MiB | 1.06 MiB | 4.5 ms | +15.0 % | 0.7 ms | 1.39 MiB | 1.2 ms |
| BC1 | 2.0 MiB | 0.67 MiB | 2.5 ms | +23.9 % | 0.4 ms | 0.92 MiB | 0.7 ms |
| RGBA 2× | 62.5 MiB | 15.32 MiB | 70 ms | +16.8 % | 12.0 ms | 24.15 MiB | 20 ms |
| BC7 2× | 15.6 MiB | 3.94 MiB | 15 ms | +29.4 % | 2.7 ms | 6.07 MiB | 4.8 ms |
| BC7 2×, RDO 1 | 15.6 MiB | 3.65 MiB | 16 ms | +16.9 % | 2.4 ms | 5.03 MiB | 4.3 ms |
| BC1 2× | 7.8 MiB | 1.77 MiB | 7.8 ms | +34.0 % | 1.5 ms | 2.94 MiB | 2.5 ms |

- **Block data decodes 4–5× faster than RGBA** for the same art, because it is a quarter (BC7) or an eighth (BC1) of
  the bytes and Zstd finds little to match in it. Enhanced BC7 is also smaller on disk than Enhanced RGBA (5.4 against
  7.2 MiB), and Classic BC7 nearly as small.
- **Strips cost block data little on Enhanced** (+3–6 %), and as much as RGBA on Classic (+15–34 %), whose pages are
  mostly empty and repeated sprites.
- **RGBA 2× is out of reach lossless:** an Enhanced page grows to 70 MiB and 312 ms. Upscaled pages need a block format.
- RDO λ 1 saves 8–13 % under Zstd for the same decode time.

**Classic art page 0 upscaled with MMPX** (the filter chosen for Classic's pixel art, which adds no colours), 2×:

| Format | Raw | zstd 19, one frame | Decode | Strips | 8 threads | lz4 hc 12 | Decode |
|---|---:|---:|---:|---:|---:|---:|---:|
| RGBA 2× (lossless) | 62.5 MiB | 1.97 MiB | 20 ms | +19.9 % | 3.6 ms | 4.30 MiB | 13 ms |
| BC7 2× | 15.6 MiB | 1.99 MiB | 9.0 ms | +42.2 % | 2.1 ms | 3.67 MiB | 3.9 ms |
| BC7 2×, RDO 1 | 15.6 MiB | 2.39 MiB | 13.6 ms | +21.3 % | 1.9 ms | 3.65 MiB | 3.9 ms |
| BC1 2× | 7.8 MiB | 1.28 MiB | 6.7 ms | +39.3 % | 1.5 ms | 2.34 MiB | 2.5 ms |

- **MMPX keeps Classic lossless at 2×:** 1.5× the 1× page on disk (1.97 against 1.31 MiB) and 20 ms, against
  15.3 MiB and 70 ms after Lanczos. BC7 is no smaller, so an MMPX-upscaled Classic page has no reason to be lossy.
- **RDO makes MMPX pages larger** (+20 %): the upscale repeats identical blocks, which plain BC7 encodes identically
  and RDO's choices break apart.
- BC1 has no RDO in our encoder (`dds`), so BC1 with RDO was not measured.

Every page in 128-row strips makes `tex_art_ec.uddp` and `tex_land_ec.uddp` 8.5 % larger (60.2 to 65.4 MB and 21.1 to
23.0 MB). The converter keeps strips only where they pay (`PAGE_STRIPS_WORTH` in `udd-conv`, from the rule below):
a page of at least 12 MiB raw whose one frame is at least 3 MiB and whose strips add at most 10 %. That puts 5 of the
9 art pages and 1 of the 6 land pages in strips; the packages grow by 4.3 % and 2.2 % (62.8 and 21.6 MB), with
byte-identical pages. Read through `udd-container`, all art pages take 367 ms instead of 759, all land pages 130 ms
instead of 159; a page in strips takes 8–9 ms instead of 62–86. Classic pages and gumps stay one frame.

The strip height is a multiple of 8 rows, so a strip holds whole block rows of BC7 and of 4×4 and 8×8 ASTC pages, and a
decoded strip could go straight to its rows of a block-compressed texture. The frames are stored one after another as
plain Zstd, and a reader that does not split them decodes them in order. Bit 16 of the payload's Meta32 marks them
(the user, 2026-11-19), so the reader splits only the payloads the writer chose to store in strips, and a patch stores
them in strips again.

## Upscaled pages, 1× to 4×

Page 0 of each atlas, cut to its first 512 rows so the 4× page fits in memory (a quarter of an art page's rows,
0.26–0.28 of a land page's), upscaled and stored four ways. Enhanced art, both land atlases: Lanczos3. Classic art:
MMPX at 2× and 4×, xBRZ at 3×, which has no MMPX. Zstd 19 one frame and 128-pixel-row strips, LZ4 HC 12, best of 3.
"× 1×" is the size against the 1× RGBA page in one Zstd 19 frame; times are for the band, so a whole page takes about
4× (Enhanced land 3.5×).

| Atlas | Form | 2×: × 1×, 1 thread ms, strips | 3× | 4× |
|---|---|---|---|---|
| Enhanced art | RGBA | 9.5, 96, +3.6 % | 18.9, 225, +3.4 % | 33.2, 374, +3.0 % |
| | BC1 | 1.20, 8, +4.7 % | 2.46, 22, +5.6 % | 4.00, 40, +6.0 % |
| | BC7 | 3.00, 16, +4.0 % | 6.36, 51, +6.2 % | 10.6, 91, +7.2 % |
| | BC7 RDO 1 | 2.60, 15, +2.0 % | 5.29, 45, +3.2 % | 8.35, 86, +3.7 % |
| Enhanced land | RGBA | 7.8, 39, +6.3 % | 15.8, 101, +6.9 % | 27.4, 171, +6.8 % |
| | BC1 | 0.90, 3, +5.6 % | 1.83, 8, +7.5 % | 3.01, 14, +8.3 % |
| | BC7 | 2.68, 7, +4.3 % | 5.58, 20, +8.3 % | 9.35, 43, +10.2 % |
| | BC7 RDO 1 | 2.28, 6, +2.1 % | 4.59, 16, +4.7 % | 7.27, 34, +5.9 % |
| Classic art | RGBA | 1.34, 5, +12.7 % | 8.02 (xBRZ), 27, +11.9 % | 1.91, 12, +6.9 % |
| | BC1 | 1.05, 2, +14.6 % | 3.37, 5, +10.4 % | 1.24, 4, +25.6 % |
| | BC7 | 1.62, 3, +20.3 % | 6.21, 9, +12.2 % | 1.36, 6, +18.7 % |
| | BC7 RDO 1 | 1.73, 3, +10.7 % | 6.08, 10, +7.4 % | 1.40, 6, +18.0 % |
| Classic land | RGBA | 11.5, 35, +19.2 % | 24.2, 87, +13.2 % | 44.7, 174, +11.1 % |
| | BC1 | 1.67, 4, +21.0 % | 4.17, 11, +7.7 % | 5.56, 16, +26.4 % |
| | BC7 | 3.93, 7, +18.6 % | 9.53, 21, +8.7 % | 14.2, 37, +17.4 % |
| | BC7 RDO 1 | 3.44, 7, +12.9 % | 7.34, 17, +4.2 % | 10.0, 31, +8.2 % |

At 1×, against the same RGBA page: BC1 0.26–0.56, BC7 0.69–1.20, BC7 RDO 1 0.59–1.10; strips cost 1–6 % on block
pages and 5–24 % on RGBA (Classic land the most).

- **Interpolated upscales can't stay lossless.** Lanczos makes Enhanced art 9.5× larger at 2× and 33× at 4×, and its
  4× page takes about 1.5 s on one thread. Extrapolated from the bands, a whole `tex_art_ec` would grow from 60 MB to
  about 570 MB at 2× RGBA and 2 GB at 4×; `tex_land_ec` from 21 MB to 165 / 580 MB.
- **BC7 with RDO 1 is the smallest BC7 on every interpolated page,** 12–29 % below plain BC7 from 2× on. `tex_art_ec`
  would be about 155 MB at 2×, 320 MB at 3×, 500 MB at 4×; `tex_land_ec` 48 / 97 / 154 MB. BC1 halves that again, but
  has one-bit alpha and no RDO in our encoder.
- **MMPX keeps Classic art cheap at every scale:** 1.34× at 2× and 1.91× at 4× as lossless RGBA, so a 4× `tex_art_cc`
  would be about 93 MB on disk. BC7 is smaller only at 4× (1.36×), where it saves VRAM too. xBRZ invents colours and
  costs 8× at 3×.
- **Classic land suffers most from Lanczos,** 11.5× at 2×, since its flat 44 × 44 tiles compress well at 1×; BC7 RDO
  1 keeps it at 3.4–10×.
- **LZ4 HC suits block pages:** on Enhanced art 4× BC7 RDO 1 it is 20 % larger than Zstd 19 (18.7 against 15.6 MiB)
  and decodes on one thread in 11 ms, as fast as Zstd 19 in strips on 8 threads (11.2 ms) and 8× as fast as Zstd 19 on
  one. The codec value 3 is free in Meta32 if a block package wants it.

### When strips pay

Strips help where a page takes over about 25 ms on one thread and cost under about 10 %. From the table, scaled to
whole pages:

- **Yes:** Enhanced art and land RGBA at every scale (1×: 31–76 ms, +5–7 %); every Enhanced page from 2× on, RGBA or
  BC (BC7 RDO 1 at 2×: 21–60 ms, +2 %); Classic land BC7 RDO 1 from 3× on.
- **No:** every 1× block page (at most 14 ms); Classic art at every scale and form but xBRZ (at most 48 ms with MMPX,
  for +7–26 %); Classic land RGBA at 1× (14 ms, +24 %).
- **Costly but needed:** Classic land RGBA upscaled (132–660 ms) pays 11–19 % for strips; BC7 RDO 1 avoids both.

The writer decides this for itself (`CompressionFlag::ZstdStripsWhenWorthLevel`): it compresses the strips (in
parallel, so cheap beside the one frame) and keeps them when the page is large and they cost less than the limit.

## Trained dictionaries

A Zstd dictionary trained on the strips of the other pages (one in three 64 KiB samples) and used on the 128-row strips
of the pages left out, at level 19:

| Pages | 112 KiB dictionary | 1 MiB dictionary | One-thread decode |
|---|---:|---:|---:|
| Enhanced art 0 and 4 (14.55 MiB in strips) | +0.9 % | −2.6 % | 5–6 % slower |
| Classic art 0 and 1 (3.85 MiB in strips) | +6.1 % | +1.4 % | 7–15 % slower |

A dictionary helps payloads of a few kilobytes, which have no history of their own. A 2 MiB strip builds its history
in its first few kilobytes, so the dictionary saves at most 2.6 % and slows every decode. The animation study found the
same on large bodies (`uddp_anim_format_requirements.md`: ×1.000–1.025).

## Reproducing

[`atlas_page_compression/`](atlas_page_compression/) is the benchmark, a standalone crate that reads its pages from a
package folder:

```sh
cd docs/agent_notes/atlas_page_compression
CARGO_TARGET_DIR=/path/to/scratch cargo run --release -- /path/to/uddp
```

It runs every codec and the 128-row strips. `--bin strip_levels` runs the levels and codecs in strips (it reads
`tex_land_ec.uddp` too), `--bin block_pages <dir> [lanczos|mmpx] [ec|cc|both]` the BC1 and BC7 pages, and `--bin upscaled_pages <dir>
[ec|cc|both]` the pages at 1× to 4×. The PNG filter, the other strip heights and
the dictionaries were one-off runs with the numbers above.
