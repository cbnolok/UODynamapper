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
- BC1 with RDO was measured later, on the upscaled pages: see "BC1 with RDO" below.

Every page in 128-row strips makes `tex_art_ec.uddp` and `tex_land_ec.uddp` 8.5 % larger (60.2 to 65.4 MB and 21.1 to
23.0 MB). The converter keeps strips only where they pay (`PAGE_STRIPS_WORTH` in `udd-conv`, from the rule below):
a page of at least 12 MiB raw whose one frame is at least 3 MiB and whose strips add at most 10 %. That puts 5 of the
9 art pages and 1 of the 6 land pages in strips; the packages grow by 4.3 % and 2.2 % (62.8 and 21.6 MB), with
byte-identical pages. Read through `udd-container`, all art pages take 367 ms instead of 759, all land pages 130 ms
instead of 159; a page in strips takes 8–9 ms instead of 62–86. Classic pages and gumps stay one frame, but for the
densest Classic land page (4.07 MiB in one frame, +9.8 % in strips, repacked on 2026-11-19).

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
  has one-bit alpha, and RDO gains it little (below).
- **MMPX keeps Classic art cheap at every scale:** 1.34× at 2× and 1.91× at 4× as lossless RGBA, so a 4× `tex_art_cc`
  would be about 93 MB on disk. BC7 is smaller only at 4× (1.36×), where it saves VRAM too. xBRZ invents colours and
  costs 8× at 3×.
- **Classic land suffers most from Lanczos,** 11.5× at 2×, since its flat 44 × 44 tiles compress well at 1×; BC7 RDO
  1 keeps it at 3.4–10×.
- **LZ4 HC suits block pages:** on Enhanced art 4× BC7 RDO 1 it is 20 % larger than Zstd 19 (18.7 against 15.6 MiB)
  and decodes on one thread in 11 ms, as fast as Zstd 19 in strips on 8 threads (11.2 ms) and 8× as fast as Zstd 19 on
  one. It became codec 3 (see "LZ4 HC within 20 %" below).

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

### LZ4 HC within 20 %

LZ4 HC is Meta32 codec 3 (`Codec::Lz4Hc`, the `lz4` crate's level 12, `LZ4_HC_LEVEL`): one LZ4 block with no size
prefix, the raw size from the locator. The writer stores a page as LZ4 HC when that is at most 20 % larger than the page
in one Zstd frame, and otherwise as Zstd, in strips when they pay (`CompressionFlag::ZstdOrLz4HcLevel`; the converter's
`atlas_page_compression`, `PAGE_LZ4_HC_MAX_GROWTH_PERCENT`; the user, 2026-11-19: "let's use 20% as threshold"). It
compresses both side by side. A patch re-stores an LZ4 HC payload as LZ4 HC; LZ4 HC is deterministic at a fixed
level, so the rebuilt package is byte for byte the one patched to.

Every page of the 2026-11-19 repack, one thread, best of 5 (Zstd 19 one frame against LZ4 HC 12):

| Package | Pages | LZ4 HC larger by | Decode, Zstd 19 → LZ4 HC | LZ4 HC kept |
|---|---:|---:|---:|---:|
| `tex_art_ec`, BC7 (`--bc7`), 8 MiB pages | 10 | +16.6–32.4 % | 9.5–13.0 → 2.9–3.7 ms | 1 |
| `tex_land_ec`, BC7, 4 MiB pages | 6 | +25.5–49.7 % | 2.3–6.4 → 0.8–1.8 ms | 0 |
| `tex_art_ec`, RGBA (default) | 9 | +52.3–73.1 % | 28–83 → 10–34 ms | 0 |
| `tex_art_cc`, RGBA (default) | 46 | +43.8–130.5 % | 5.1–12.3 → 3.0–6.0 ms | 0 |

- **Only one page is LZ4 HC at 1×:** the default packages were lossless RGBA (BC7 since 2026-11-22), where LZ4 HC costs 44–131 %, so they come
  out byte for byte as before (`tex_land_cc` and `gumps_ec` changed only by the strips rule and the gump packer, which
  they predated). Of the BC7 pages, most of the art lands just above the limit (+21–25 %), and the land further.
- **The rule pays on upscaled block pages,** where LZ4 HC was +20 % on 4× Enhanced BC7 RDO 1 (above), and on the
  animation blocks (`uddp_anim_format_requirements.md`: +14–17 % for the uniform crowds at 2× and 3×).

### BC1 with RDO

`uot_texture_codecs::bc1::rdo` ports the entropy reduction transform of bc7enc_rdo (`ert.cpp`) to BC1's 8-byte blocks,
with the BC7 port's cost model, `rdo_bc_encoder.cpp`'s BC1 smooth-block scale (λ / 8), 64 blocks of look-back, and
the error over RGBA rather than RGB, since our BC1 uses punch-through alpha. The same bands, Zstd 19 one frame, MiB
and PSNR over the opaque texels:

| Atlas | Scale | BC1 | RDO 0.5 | RDO 1 | RDO 2 |
|---|---|---|---|---|---|
| Enhanced art | 1× | 0.68, 35.7 dB | 0.65, 35.3 | 0.63, 34.4 | 0.60, 32.7 |
| | 2× | 2.25, 37.5 | 2.20, 37.1 | 2.14, 36.3 | 2.04, 34.7 |
| | 4× | 7.48, 41.1 | 7.18, 40.6 | 6.92, 39.9 | 6.53, 38.5 |
| Enhanced land | 4× | 3.06, 41.2 | 2.91, 40.7 | 2.82, 40.1 | 2.68, 39.1 |
| Classic art | 1× | 0.20, 33.2 | 0.18, 32.6 | 0.17, 31.8 | 0.15, 30.3 |
| | 2× MMPX | 0.40, 38.0 | 0.41, 37.3 | 0.41, 36.3 | 0.40, 34.5 |
| | 4× MMPX | 0.48, 51.2 | 0.55, 49.8 | 0.56, 49.2 | 0.57, 48.3 |
| Classic land | 2× | 1.03, 37.2 | 1.03, 36.9 | 1.03, 36.5 | 1.01, 35.6 |
| | 4× | 3.42, 41.3 | 3.43, 40.8 | 3.34, 40.2 | 3.20, 39.3 |

- **Not worth it.** RDO 1 saves 4–8 % on interpolated pages (Enhanced art 4×: 7.5 %) for 0.8–1.3 dB; RDO 2 saves
  9–13 % for 2–3 dB. BC7 RDO 1 saves 12–29 % against plain BC7. An 8-byte block has half the bytes a BC7 block has
  for a run to replace, and its endpoints and indices are denser, so a run from a nearby block costs more error.
- **It makes MMPX pages larger** (up to +19 % at 4×), as BC7 RDO does: the runs it picks from nearby blocks break the
  longer runs of identical blocks the upscale left.
- It is slow: 28–38 s on rayon's whole pool for a 4× band of Enhanced art, so about 2 minutes a page.

The converter does not use it; plain BC1 stays the BC1 form. `upscaled_pages <dir> [ec|cc|both] bc1` reruns these rows.

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

## BC7 RDO on mobile animation pages

Why packing the mobile animations took 16 minutes (Classic) and 34 minutes (Enhanced) with `--bc7-rdo`, and what the
RDO gives back (2026-11-21). `--bin anim_rdo` reruns the numbers below.

### Where the time goes

The Classic BC7 pack (1210 pages, `--bc7-rdo --zstd=9`, λ 0.05, look-back 64) took 983 s: BC7 encode 18.5 s, RDO
920 s. The RDO is 95 % of the pack, about 50 times the encode. Three things add up:

1. **Each block tries thousands of candidates.** The RDO is the ERT of bc7enc_rdo (`ert.cpp`): for each block, every
   earlier block of the same BC7 mode within the look-back, every run length 16 to 3, and every byte offset, so 105
   candidates per earlier block. On a Classic page, one block that is not fully transparent ran about 2650 candidate
   checks, and 94 % of them were decode trials (84 % mode 6, 16 % mode 4). The bound on the trial error stops 99.97 %
   of the decodes early, but the rate test (the candidate's bits alone already cost more than the best so far) almost
   never skips one at λ 0.05, since the bits weigh little at that λ.
2. **The look-back is 64 blocks,** 8 times bc7enc_rdo's default window of 128 bytes (8 blocks). The cost grows with
   it: on 20 Classic pages, look-back 8 took 27 s of CPU, 16 took 44 s and 64 took 132 s.
3. **It used 8 of the 20 threads.** The packer encoded a chunk of 20 pages one page after another, and each page's
   RDO split into row chunks run in parallel. Fully transparent blocks (61 % of a Classic page) cost one decode, so
   the chunks of a sparse page finish at very different times, and the pool idled until the slowest one ended.

Fully transparent blocks were already cheap: the encoder writes all of them exactly, so the RDO's zero-error skip
(`Bc7RdoOptions::sparse_atlas`) catches every one.

bc7enc_rdo differs in ways that matter less here: it searches the earlier blocks of every mode, has no error bound or
rate pruning, and updates its match-continuation and REP0 state after each block, where our port updates it during
the search.

### What it gives back

20 Classic pages (56.8 Mpx) and 16 Enhanced pages (48.0 Mpx), sampled evenly. Size is the blocks with Zstd 9 (the
packer's `--zstd=9`), against plain BC7. PSNR is against the RGBA page, RGB over the texels with alpha > 0. CPU is
seconds of the RDO on the whole pool.

| Look-back | λ | Classic size | Classic PSNR | Enhanced size | Enhanced PSNR | CPU, Classic |
|---|---|---|---|---|---|---|
| plain BC7 | | 13.69 MiB | 33.59 dB | 12.02 MiB | 31.95 dB | 4.6 s (encode) |
| 8 | 0.05 | +0.1 % | 33.59 | +0.0 % | 31.95 | 27 s |
| | 0.5 | −0.9 % | 33.11 | −0.8 % | 31.72 | 27 s |
| | 1 | −2.6 % | 32.33 | −2.0 % | 31.28 | 27 s |
| | 2 | −6.3 % | 30.55 | −4.8 % | 30.12 | 26 s |
| 16 | 0.05 | +0.2 % | 33.59 | −0.2 % | 31.95 | 45 s |
| | 0.5 | −0.8 % | 33.03 | −1.6 % | 31.66 | 44 s |
| | 1 | −2.8 % | 32.13 | −3.1 % | 31.14 | 44 s |
| | 2 | −6.9 % | 30.21 | −6.4 % | 29.86 | 44 s |
| 64 | 0.05 (the old default) | +0.2 % | 33.59 | −0.3 % | 31.96 | 132 s |
| | 0.25 | −0.5 % | 33.32 | −1.3 % | 31.82 | 125 s |
| | 0.5 | −1.8 % | 32.85 | −2.4 % | 31.56 | 119 s |
| | 1 | −4.8 % | 31.76 | −4.7 % | 30.89 | 122 s |
| | 2 | −10.4 % | 29.65 | −9.1 % | 29.42 | 109 s |

- **At λ 0.05 the RDO saves nothing.** It changes 15–22 % of the blocks, but the package is no smaller; it costs 30
  times the encode. The whole gain of the Classic BC7 package against RGBA8888 (869 MB against 921 MB) comes from BC7.
- **It saves bytes only at a λ that costs quality:** λ 1 saves about 5 % for 1–2 dB, λ 2 about 10 % for 2.5–4 dB.
  The other atlas notes found the same on art and land (`2026-11-01-data-track-open-items.md`, item d).
- **A longer look-back pays per dB:** at λ 1, look-back 64 saves 4.7–4.8 % where 16 saves 2.8–3.1 %, for 0.25–0.4 dB
  more, at 3 times the CPU.

**RDO is off by default** everywhere: the packers' option defaults are λ 0 (`NO_BC7_RDO_LAMBDA`), `udd-conv-gui`
starts with it unticked, and `udd-cli pack` applies it only with `--bc7-rdo`. `--bc7-rdo` without `--bc7-rdo-lambda`
still uses `DEFAULT_BC7_RDO_LAMBDA` 0.05, which the table shows does nothing; λ 1 with look-back 64 is the setting
worth offering, after a visual check on sprites (`UDDP_FORMATS.md` asks for one).

### Making it faster

The same 20 Classic pages, wall time of the RDO on the whole pool (Ryzen AI 9 365: 10 cores, 20 threads), with
identical blocks in every row:

| Step | λ 0.05, look-back 8 | λ 0.05, look-back 64 | λ 1, look-back 64 | Threads busy |
|---|---|---|---|---|
| pages one after another (before) | 3.75 s | 15.90 s | 16.14 s | 7–9 |
| pages in parallel | 1.78 s | 9.11 s | 7.95 s | 16–18 |
| pages in parallel, AVX2 trials | 1.15 s | 6.23 s | 5.07–5.68 s | 15–18 |

Wall times on a full pool vary by 10–20 % from run to run, since 20 pages on 20 threads end unevenly. CPU seconds
are steadier:

| Decode trials | λ 0.05, look-back 8 | λ 1, look-back 8 | λ 0.05, look-back 64 | λ 1, look-back 64 |
|---|---|---|---|---|
| scalar | 29.9 s | 29.8 s | 154.8 s | 135.2 s |
| AVX2 | 19.4 s | 18.5 s | 104.3 s | 90.3 s |
| AVX-512 | 20.2 s | 18.8 s | 110.6 s | 91.7 s |

- **Pages in parallel:** the packers encode the chunk's pages (one per thread) as parallel tasks. A page's RDO still
  splits into the same row chunks, sized from the pool's thread count, which a nested task sees unchanged, so the
  blocks are those of the serial loop (`page_chunk_encodes_in_parallel_with_the_bytes_of_a_serial_loop`, Classic and
  Enhanced). CPU seconds rise 10 %, since the second thread of a core adds less than a core.
- **The blocks depend on the thread count.** That was so before: the row chunks restart the look-back and the match
  state, and their size comes from the pool. A machine with another thread count writes other blocks for the same
  settings (the same size and quality to within noise). Fixed chunks would make the output portable, at the cost of
  changing today's bytes.
- **AVX2 decode trials** (`bc7/rdo_simd.rs`): the mode 6 and mode 4 trials, 99 % of the decodes, interpolate and sum
  four texels per instruction and test the bound after each group of four, where the scalar decoder tests it after
  each texel. The errors are non-negative integers, so both cross the bound together and give the same answer: the
  blocks are bit-identical at every level (`simd_block_errors_match_the_scalar_decoder` on random blocks with random
  bounds, `rdo_writes_the_same_blocks_at_every_simd_level`, and the digests of all four settings above). The RDO
  picks AVX2 (with BMI2) at run time and falls back to the scalar code without it; the whole search loop is compiled
  for the level, so the scalar parts gain too (`ceil` becomes one instruction). CPU drops by a third; the rest is the
  search itself (enumerating candidates, the hash of tried segments, the rate tests), 37 % of the instructions under
  callgrind before.
- **AVX-512 is no faster.** Eight texels per instruction took 1–6 % more CPU than AVX2: this Zen 5 laptop runs AVX-512
  on 256-bit units, and a trial ends after three texels on average, so a wider group mostly decodes texels it does not
  need. It stays behind `limit_bc7_rdo_simd(Bc7RdoSimd::Avx512)`; the default stops at AVX2.

### On the GPU (prototype)

`--bin rdo_gpu_proto` (`rdo_gpu_proto.wgsl`) runs the fixed-offset search in a compute shader. One workgroup of 64
threads per row chunk (the CPU's chunks) walks the chunk's blocks in order. For each mode 6 or mode 4 block, the threads
share the candidates: earlier blocks of the same mode in the look-back, × run length, × offset. Each thread keeps its own
best and its own error bound, and the lowest cost wins, with the earliest candidate on a tie. Each submission walks 256
blocks of every chunk, about 75 ms of GPU, so the desktop keeps drawing. The shader leaves out:

- the hash of tried segments;
- the match-continuation and REP0 prices, which the CPU updates during a block's search;
- the ultrasmooth scales.

Its blocks therefore differ from the CPU's.

Same 20 Classic pages, Radeon 880M (RADV), against the parallel AVX2 CPU RDO on all 20 threads:

| Setting | CPU wall | GPU wall | CPU size | GPU size | CPU PSNR | GPU PSNR |
|---|---|---|---|---|---|---|
| λ 0.05, look-back 8 | 1.42 s | 0.22 s | +0.1 % | +0.1 % | 33.59 | 33.59 |
| λ 1, look-back 8 | 1.28 s | 0.25 s | −2.6 % | −3.1 % | 32.33 | 32.30 |
| λ 0.05, look-back 64 | 6.28 s | 1.22 s | +0.2 % | +0.1 % | 33.59 | 33.59 |
| λ 1, look-back 64 | 6.19 s | 1.35 s | −4.8 % | −6.1 % | 31.76 | 31.59 |

- **It is 4.6 to 6.4 times faster** and leaves the CPU free. For the whole Classic package (1210 pages), λ 1 at
  look-back 64 would take about 80 s instead of about 6 minutes.
- **Its size and quality are on the CPU's curve.** Without the continuation and REP0 prices it takes a few more
  matches: 1.3 % smaller for 0.17 dB at λ 1.
- **A full version** in `uot-gpu-compute`, called by udd-conv, had to choose between two semantics:
  - port the CPU's exact search (the hash, the continuation and REP0 state that changes within a block's search, the
    ultrasmooth scales, modes other than 6 and 4 on the CPU), which costs a sequential pass per block;
  - define its own deterministic RDO, whose output differs from the CPU's and depends on the chunking.

  It took the second (next section).

### On the GPU (uot-gpu-compute)

`uot_gpu_compute::bc7_rdo::Bc7RdoGpu` (`shaders/block_encode/bc7_rdo.wgsl`) is the full version (2026-11-22). Its
blocks differ from the CPU's, and it defines its own deterministic search:

- **The CPU's price model and error scales.** It uses the codecs' match length and distance prices, the literal
  price, and the continuation and REP0 prices. The CPU computes the per-block error scales over the whole page
  (`bc7::rdo::block_error_scales`): the smooth block scale, and the ultrasmooth regions scaled by λ.
- **Every mode.** The shader decodes all eight BC7 modes. Blocks without a mode are left alone, and with
  `skip_zero_mse_blocks` so are exact blocks.
- **The lowest cost wins, ordered.** Each block takes the cheapest of its candidates: an earlier block of the same
  mode in the look-back, × run length 16 to 3, × offset. On a tie it takes the lowest candidate number: the nearest
  block, then the longest run, then the lowest offset. The 64 threads of a workgroup share the candidates. A thread
  drops a candidate only when it cannot cost less than the thread's own best, which already has a lower number:
  either the rate alone reaches that best, or the error passes a bound kept above the exact one by a margin. A
  minimum over the costs, then over the numbers, picks the winner. The result therefore does not depend on how the
  threads split the candidates.
- **Fixed chunks.** One workgroup walks each chunk of whole block rows: at least 2048 blocks, and at least the
  look-back. The chunks are the page's, not the pool's, so the blocks depend only on the page, λ and the look-back.
  They do not depend on the workgroup size, the pages batched together, the blocks per submission, or the CPU's
  thread count (the CPU RDO's output does).
- **Match state as in ert.cpp and the BC1 RDO.** The last match's distance (REP0), and the block a run from offset 0
  continues, are set after each matched block. A block without a match resets the continuation, and skipped blocks
  leave both alone. The CPU BC7 RDO instead updates this state during the search.
- **No segment hash.** The CPU's hash skips a trial already tried from a nearer block. On the GPU the same trial from
  a farther block costs at least as much (the distance price never falls), and the tie goes to the nearer block, so
  the hash would only save time.
- **Not supported.** Relative movement and the two-match search; parameters asking for them are an error.

Its tests (`bc7_rdo/tests.rs`, ignored without a GPU) compare it with a CPU walk of the same search, which tries
every candidate with no bound and uses the codecs' decoder. The fixtures are random blocks of every mode with near
copies and invalid and exact blocks, plus encoded sprite pages of one and three chunks, at λ 1 to 4 and look-back 8
and 64. They match block for block (4 to 72 % of the blocks changed). A batch, a second run, and three threads calling
at once give the same blocks.

Same pages as above, Radeon 880M (RADV, Mesa 26.2.7), against the parallel AVX2 CPU RDO on all 20 threads. The GPU
wall time includes the CPU's error scales and the upload; the two GPU runs and the concurrent callers (the packers'
way, one call per page from the rayon threads) gave the same blocks in every row:

| Pages | Look-back | λ | CPU wall | GPU wall | CPU size | GPU size | CPU PSNR | GPU PSNR |
|---|---|---|---|---|---|---|---|---|
| Classic | 8 | 1 | 1.19 s | 0.66 s | −2.6 % | −3.2 % | 32.33 | 32.20 |
| | | 2 | 0.99 s | 0.56 s | −6.3 % | −7.2 % | 30.55 | 30.33 |
| | 64 | 1 | 5.42 s | 1.65 s | −4.8 % | −6.3 % | 31.76 | 31.50 |
| | | 2 | 4.60 s | 1.71 s | −10.4 % | −12.7 % | 29.65 | 29.27 |
| Enhanced | 8 | 1 | 0.99 s | 0.52 s | −2.0 % | −2.6 % | 31.28 | 31.21 |
| | | 2 | 0.90 s | 0.46 s | −4.8 % | −5.7 % | 30.12 | 29.97 |
| | 64 | 1 | 4.53 s | 1.39 s | −4.7 % | −6.2 % | 30.89 | 30.74 |
| | | 2 | 4.20 s | 1.44 s | −9.1 % | −11.4 % | 29.42 | 29.14 |

- **It lies on or slightly above the CPU's size/PSNR curve.** At a given λ it takes a few more matches: 0.6 to 2.3 %
  smaller, for 0.07 to 0.38 dB. Along the CPU's line between λ 1 and 2, the CPU at the GPU's PSNR would be 0.3 to
  1.5 % larger. So at equal quality the GPU is as small or a little smaller, and λ means slightly more on the GPU. The
  prototype, without the continuation and REP0 prices and the ultrasmooth scales, landed in the same place (−6.1 %,
  31.59 dB). Those prices and scales mostly move the choices, not the curve.
- **It is 2.7 to 3.3 times faster at look-back 64 and 1.8 to 2 times at look-back 8.** At look-back 8 the CPU's error
  scales, the upload and a GPU walk of fixed length weigh more. The CPU stays free during the walk.
- **Compile:** the walk's pipeline takes 91 ms on a cold Mesa cache and 4 ms warm. The module is created without
  naga's loop bounding (`network_shader_module`); every loop counts up to a constant, a uniform or a chunk's end.
- **Visual check at λ 1, look-back 64:**
  `/home/claudio/rust-target/uodynamapper-scratch/gpu-rdo/gpu-rdo_{cc,ec}_l1_lb64.mp4` and their `zoom_` 5×
  versions toggle plain BC7, the CPU RDO, the GPU RDO and the source on a crop of a Classic and an Enhanced page.
  The two RDOs look alike: both flatten some sprite texture, with no block artefacts of their own.

**In the packers** (`udd_conv::atlas::bc7_rdo_device`): when RDO is on, the mobile animation (Classic, Enhanced),
art, land and aux packers run it on the GPU if a wgpu adapter is there, otherwise on the CPU. If the GPU fails, Auto
warns and runs the page on the CPU. `udd-cli pack --bc7-rdo-device auto|cpu|gpu` chooses the device; `gpu` fails
without an adapter. The packers start one device on the first page they RDO. The pages encoded at once call it from
their threads and share its batches. RDO stays off by default.

A full repack of the Classic mobile animations (`udd-cli pack pack-mobile-anims --bc7-rdo --bc7-rdo-lambda 1
--bc7-rdo-lookback-blocks 64 --zstd=9`, 1210 pages, release build, all 20 threads):

| RDO device | Total | Page encode | Package |
|---|---|---|---|
| GPU, first run | 169.7 s | 124.7 s | 810 486 376 bytes |
| GPU, second run | 164.2 s | 122.5 s | 810 486 376 bytes, the same file |
| CPU | 397.5 s | 353.5 s | 823 320 535 bytes |

The GPU package is 1.6 % smaller, in line with the pages above, and the two GPU runs wrote the same file byte for byte.
With the walk on the GPU, the CPU's BC7 encode, which runs on the same threads, also takes less time (536 against 1475
thread seconds).

### BC7 encode on the GPU

`uot_gpu_compute::page_encode::PageEncoderGpu` encodes a whole page held on the CPU with the GPU BC7 encoder
(`block_encode::MobileBlockEncoder`, block_compression's port of the ISPC kernels), on a device of its own
(2026-11-22). A partial block repeats the page's edge texels, as the CPU encoder's does. A block of one colour is the
CPU encoder's solid block, made on the CPU; the other blocks go up side by side, 1024 to a row, in bands of at most 128
Ki blocks, one submission each, read back before the next, and are put back in their places (since the optimizations
below; before, the bands were whole block rows of the page). The pages encoded at once share the encoder,
whose frames wait in it for its next record, so a band records and submits under a lock and waits for its blocks
outside it. It is `PageEncoderGpu` now, with `encode_bc1` and `encode_blocks(format)` beside `encode(quality)`.

The pages against the packers' CPU encoder (AnalyticalWide), all encoded at once from the 20 rayon threads as the
packers do, on the Radeon 880M (`--bin bc7_gpu`), before the optimizations below (their table has the UltraFast
rows now). PSNR is RGB over the texels with alpha > 0; sizes are the blocks at Zstd 9; GPU times are the faster of two
runs, which gave the same blocks:

| Pages | Encoder | Wall | Zstd 9 | RGB PSNR | Alpha PSNR |
|---|---|---|---|---|---|
| Classic mobile animations, 20 (56.8 Mpx) | CPU | 0.26 s | 13.69 MiB | 33.59 dB | 64.17 dB |
| | GPU UltraFast | 0.33 s | +0.5 % | 33.32 dB | 60.50 dB |
| | GPU VeryFast | 1.63 s | +4.1 % | 34.94 dB | 54.33 dB |
| | GPU Fast | 2.54 s | +9.6 % | 35.92 dB | 57.66 dB |
| | GPU Basic | 5.02 s | +9.8 % | 36.18 dB | 57.84 dB |
| Enhanced mobile animations, 16 (48.0 Mpx) | CPU | 0.28 s | 12.02 MiB | 31.95 dB | 37.54 dB |
| | GPU UltraFast | 0.26 s | +0.5 % | 31.65 dB | 37.43 dB |
| | GPU VeryFast | 1.36 s | +4.8 % | 33.31 dB | 37.96 dB |
| | GPU Fast | 2.13 s | +9.8 % | 33.88 dB | 37.96 dB |
| | GPU Basic | 4.20 s | +10.1 % | 34.17 dB | 37.96 dB |
| Classic art, 20 (20.5 Mpx) | CPU | 0.09 s | 4.15 MiB | 33.36 dB | 58.72 dB |
| | GPU UltraFast | 0.13 s | +0.5 % | 33.15 dB | 59.35 dB |
| | GPU VeryFast | 0.60 s | +5.7 % | 35.33 dB | 54.14 dB |
| | GPU Fast | 0.91 s | +10.0 % | 36.26 dB | 55.80 dB |
| | GPU Basic | 1.78 s | +10.3 % | 36.54 dB | 55.90 dB |
| Enhanced art, 16 (16.7 Mpx) | CPU | 0.14 s | 9.05 MiB | 37.86 dB | 46.48 dB |
| | GPU UltraFast | 0.09 s | +0.0 % | 37.85 dB | 46.22 dB |
| | GPU VeryFast | 0.50 s | +1.8 % | 39.23 dB | 45.78 dB |
| | GPU Fast | 0.74 s | +7.7 % | 40.45 dB | 45.89 dB |
| | GPU Basic | 1.42 s | +8.6 % | 40.57 dB | 46.00 dB |
| Enhanced land, 6 (19.5 Mpx) | CPU | 0.10 s | 12.31 MiB | 41.30 dB | 52.78 dB |
| | GPU UltraFast | 0.12 s | +1.1 % | 41.64 dB | 51.52 dB |
| | GPU VeryFast | 0.60 s | +1.6 % | 42.28 dB | 51.07 dB |
| | GPU Fast | 0.94 s | +13.9 % | 44.25 dB | 52.01 dB |
| | GPU Basic | 1.94 s | +14.9 % | 44.40 dB | 52.06 dB |

The art and land pages come from lossless packs of the clients (`--raw --zstd=9`), the mobile animations from the
RGBA8888 test packages. Slow, the last profile, was left out: Basic already costs 15–20 times the CPU's time.

- **UltraFast is the CPU's quality.** In RGB it is 0.2–0.3 dB below the CPU on the sprites and the Classic art, level
  on the Enhanced art and 0.3 dB above on the land, with blocks 0–1 % larger after Zstd. Alpha moves by at most 1.3
  dB, except on the Classic sprites, from 64 to 60 dB, where both are far above the RGB error. VeryFast and above gain 0.6–3
  dB, but cost 2–15 % in size and 5–20 times the time: that is a choice of quality, not of device. The packers use
  UltraFast (`GPU_BC7_QUALITY` in `udd_conv::atlas::bc_encode_device`).
- **It was not faster.** At UltraFast the GPU took the CPU's wall time on 20 threads, give or take 0.05 s: the band
  uploads and readbacks and the one queue weighed as much as the kernel, which spent most of its time on transparent
  blocks. What it gave was CPU threads free for the rest of the pack. Below, it encodes a whole pack's pages 2.3 to
  2.9 times as fast.
- **Compile:** creating the encoder compiled every profile's pipeline, block_compression's one pipeline holding every
  mode: about 20 s on a cold Mesa cache, 40–50 ms warm. With our kernels (below, "GPU block encoding") creating it
  compiles nothing (60 ms, the device's start) and each format compiles at its first page: UltraFast in 5.2 s cold,
  47 ms warm, BC1 in 0.24 s cold, 7 ms warm (`page_encode` test `a_page_of_several_bands_encodes_close_to_its_texels`).

**In the packers** (`udd_conv::atlas::bc_encode_device`): `udd-cli pack --bc-encode-device auto|cpu|gpu` chooses
where the mobile animation (Classic, Enhanced), art, land and aux packers encode their BC7 pages and where the single
Classic gumps' BC1 (`gumps_cc`, and the Classic colour space gumps of a lossy `gumps_ec`; our BC1 keeps their
punch-through transparency, below, "GPU block encoding") is encoded. Auto, the default, encodes both on the GPU when
a wgpu adapter is there and on the CPU without one (the user, 2026-11-22). The BC7 first stayed on the CPU, whose pages
were 1–2.5 % smaller after Zstd; optimized (below), the GPU's whole BC7 packs come out 0.5 % smaller than the CPU's
over the three art and land packs, the encode 2.3 to 2.9 times as fast, with the CPU's threads free for the rest of
the pack, whose time stays level (reading the sources and the Zstd 19 weigh more), so Auto's BC7 moved to the GPU.
If the GPU fails, Auto warns and the CPU encodes the image. `gpu` puts both on the GPU and fails without an adapter. The RDO, when on, runs after the encode on
its own device (`--bc7-rdo-device`). The other BC7 images (the Enhanced gumps, effects, radar) are not atlas pages and
stay on the CPU, as the effects' BC1 does. The converter GUI has both choices on its BC7 cards, and the encode device
as BC1 Encode on the gump cards packing lossy.

Full packs with their default formats (BC7 without RDO, Zstd 19; release build, 20 threads, a warm shader cache),
the PSNR over every image against a lossless pack of the same sources, matched by id (`bc7_gpu compare`). GPU before
is the encoder before the optimizations below, GPU after with them:

| Pack | Device | Time | Package | RGB PSNR | Alpha PSNR |
|---|---|---|---|---|---|
| `pack-art` (Classic, 39 386 images, 232 pages) | CPU | 12.3 s; 10.2–10.6 s | 43 247 629 bytes | 33.04 dB | 57.22 dB |
| | GPU before | 8.9 s | 43 579 679 bytes (+0.8 %) | 32.70 dB | 58.29 dB |
| | GPU after | 10.4–11.5 s | 43 269 306 bytes (+0.05 %) | 32.68 dB | 61.95 dB |
| `pack-ec-textures` art (13 305 images, 86 pages) | CPU | 14.7 s; 17.4–22.0 s, with the land | 39 897 142 bytes | 45.56 dB | 49.64 dB |
| | GPU before | 14.4 s, with the land | 40 277 734 bytes (+1.0 %) | 46.25 dB | 49.21 dB |
| | GPU after | 17.2–23.1 s, with the land | 39 619 362 bytes (−0.7 %) | 45.92 dB | 49.55 dB |
| `pack-ec-textures` land (446 images, 6 pages) | CPU | | 11 082 859 bytes | 50.20 dB | 57.71 dB |
| | GPU before | | 11 365 022 bytes (+2.5 %) | 52.76 dB | 55.79 dB |
| | GPU after | | 10 862 281 bytes (−2.0 %) | 51.79 dB | 57.81 dB |

Before, the GPU packs were 1–2.5 % larger; after, the three packages together are 0.5 % smaller than the CPU's
(93 750 949 bytes against 94 227 630): the Classic art level, the Enhanced art 0.7 % and the land 2 % smaller. In RGB
the GPU stays 0.36 dB below the CPU on the Classic art and above it on the Enhanced (0.36 dB on the art, 1.6 on the
land); its alpha is level or better. The pack times are the whole commands: the first CPU time and GPU before were
taken on a quiet machine, the ranges after over three runs of each device, alternating, at loads of 7 to 18 from other
agents' builds and previews. They do not separate the devices: a pack's time goes to reading and composing the sources
and to the Zstd 19 of the pages (3.5 to 8 s per package), and its BC7 encode is about a second on the CPU's threads,
half a second on the GPU (the dumped pages below).

### Optimizing the GPU BC7 encode

Before giving Auto's BC7 to the GPU (the user, 2026-11-22: "let's wait for the wgpu version and check if that could be
further optimized"), where its time and its Zstd bytes went, at UltraFast (`--bin bc7_gpu stats`, `variants`,
`fields`, `profile`):

- **The kernel ran on empty blocks.** 43 to 67 % of a sprite or art page's blocks are transparent and up to 1 % are one
  colour; the band upload, the kernel and the readback paid for them as for any other. One page at a time on one
  thread, the kernel took 0.28 s of the Classic mobile pages' 0.34 s of GPU time (timestamp queries).
- **The opaque blocks cost the Zstd bytes.** Swapping each kind of block for the CPU's: the transparent and one-colour
  blocks move the size by at most 0.1 %, the all-opaque blocks of a page with alpha by 0.2 to 1.1 % (the most on the
  land, 76 % of whose blocks are opaque). Two reasons: in modes 6 and 7 the GPU's p-bit is 0 wherever it fits the colour
  better, which decodes alpha 254 (252 in mode 7) on texels of alpha 255 and makes the p-bits and alpha endpoints noise
  to Zstd; and it encodes 3 to 9 % of those blocks in the dual-plane modes 4 and 5 on the sampled pages, 34 % on the
  dumped land pages, where the CPU's AnalyticalWide puts them mostly through its RGB path, mode 6 with alpha 255 (modes
  4 and 5 for 0.7 to 6 % on the sampled pages, none on the dumped land). The opaque profile on those blocks (no alpha
  at all) is larger still: its mode 6 p-bits are free.

What changed (`block_encode.rs`, `bc7.wgsl`, `page_encode.rs`):

1. **One-colour blocks are the CPU's solid block** (mode 5 from the CPU encoder's optimal-endpoint table, appended to
   the shader as `MODE5_SOLID_ENDPOINTS`): `page_encode` makes them on the CPU and never sends them, and the kernel
   codes them the same way for the renderer's frames. A transparent block is one of them.
2. **Packed bands.** `page_encode` sends only the other blocks, side by side, 1024 to a row, and puts them back in
   place; the kernel and the transfers shrink with the page's empty area.
3. **Opaque blocks are encoded as the CPU encodes them**: in a page with alpha, a block whose 16 alphas are 255 skips
   modes 4 and 5 and keeps p-bits 1 in modes 6 and 7, so it decodes alpha 255. On the dumped land pages, mode 6 alone
   gives up 0.6 dB of the GPU's 2.2 dB lead over the CPU in RGB and takes 4 % off its Zstd bytes.

Every other block is still block_compression's byte for byte (`our_kernels_encode_as_block_compression`, which also
checks the opaque blocks' RGB error against the CPU's: 0.99 times it at UltraFast, 0.45 to 0.81 times at the slower
qualities; `uniform_and_opaque_blocks_are_the_cpu_encoders`). A second encode gives the same blocks.

Pages sampled from lossless packs (as above, Zstd 9), and the pages the packers hand their BC7 encoder in a full pack
(`dumped`: Bc7Oriented layouts, slots on whole blocks, unlike the lossless packs; Zstd 19, as the packages). RGB PSNR
CPU / GPU; wall on 20 threads, the faster of two runs:

| Pages | CPU wall | GPU before | GPU after | Size vs CPU, before | after | RGB PSNR CPU / GPU after |
|---|---|---|---|---|---|---|
| Classic mobile, 20 sampled | 0.27–0.37 s | 0.31 s | 0.19 s | +0.5 % | −0.2 % | 33.59 / 33.31 dB |
| Enhanced mobile, 16 sampled | 0.26–0.29 s | 0.26 s | 0.12 s | +0.5 % | +0.0 % | 31.95 / 31.65 dB |
| Classic art, 20 sampled | 0.08–0.09 s | 0.13 s | 0.07 s | +0.5 % | +0.2 % | 33.36 / 33.13 dB |
| Enhanced art, 16 sampled | 0.14 s | 0.10 s | 0.09 s | +0.0 % | −0.6 % | 37.86 / 37.83 dB |
| Enhanced land, 6 sampled | 0.07–0.09 s | 0.11 s | 0.10 s | +1.1 % | −0.2 % | 41.30 / 41.59 dB |
| Classic art, 232 dumped | 1.16 s | | 0.49 s | | +0.2 % | 33.04 / 32.69 dB |
| Enhanced art, 89 dumped | 0.75 s | | 0.33 s | | −0.9 % | 42.19 / 42.24 dB |
| Enhanced land, 8 dumped | 0.26 s | | 0.09 s | | −2.0 % | 50.01 / 51.55 dB |

The small sampled sets are a few hundred milliseconds of work, where the GPU's queue and the machine's load (other
agents' builds) weigh most; on the dumped pages of a whole pack the GPU is 2.3 to 2.9 times as fast. Alpha PSNR moves by
at most 0.1 dB, but on the Classic sprites and art, where it rises by 4 to 5 dB.

Kernel and transfers, one page at a time on one thread (`profile ... packed`, timestamp queries):

| Pages | Kernel before | Kernel after | Upload + readback after |
|---|---|---|---|
| Classic mobile, 20 | 0.280 s | 0.121 s (470 Mpx/s) | 0.11 s |
| Enhanced mobile, 16 | 0.239 s | 0.092 s | 0.06 s |
| Classic art, 20 | 0.103 s | 0.046 s | 0.05 s |
| Enhanced art, 16 | 0.083 s | 0.064 s | 0.08 s |
| Enhanced land, 6 | 0.098 s | 0.055 s | 0.08 s |

Making the bands' texels on the CPU (finding the solid blocks, packing the others) costs 0.1 to 0.5 s of one thread on
these pages, which the packers spread over their 20.

**Compile:** unchanged. UltraFast with alpha compiles cold in 6.4 and 6.5 s after, 6.2 and 7.5 s before (alternating
on the same loaded machine), and the app's warm-up is the same; see shader_compile_progress.md. Nothing tried cut it
but the bounds checks, since turned off (below, "Bounds checks off").

Tried and not taken:

- **No bounds checks** (`ShaderRuntimeChecks::bounds_checks: false`): the kernel 1.6 times as fast (0.079 s on the
  Classic mobile pages), the same blocks on every profile, the cold compile 0.5 s shorter. Left on at first: an
  out-of-range index would be undefined behaviour on the GPU instead of a clamped read, and the kernel's indexing was
  not audited for it. Taken since, after the audit below.
- **No loop bounding**: block_compression 0.9.1 turned it back on because RADV/Navi gave wrong BC7 without it.
- **Gathering the band's texels straight from the page** instead of through a block list, and comparing a block's
  texels as `u32`s: slower or level.
- **Overlapping the transfers with the kernel**: the upload and readback are about a tenth of the wall on 20 threads;
  not worth a second band in flight.

### Bounds checks off

Both kernels compile without bounds checks since 2026-11-22 (`KERNEL_CHECKS` in `block_encode.rs`: `bounds_checks:
false`; loop bounding and the integer division checks stay, as in block_compression, whose 0.9.1 found RADV/Navi
encoding BC7 wrongly without loop bounding). On Vulkan, wgpu 30 has naga clamp every index of a local array, vector or
matrix (`Restrict`), and the buffer and texture accesses unless the device has robustBufferAccess2 and
robustImageAccess; with the checks off all four go unchecked. RADV has both robustness features, so the 1.6 times comes
from the local arrays' clamps in the BC7 kernel's inner loops; a driver without them leaves the buffer and texture
accesses to the guards below.

Without the checks an index out of range is undefined behaviour, so every index of `bc7.wgsl` and `bc1.wgsl` was
audited. Where the range depends on the inputs (the settings buffer, the texture's and the output's sizes, a texel's
value), the kernel now clamps or guards it itself:

| Index | Where | Why it stays in range |
|---|---|---|
| The block, `array<f32, 64>` | `16 * channel + texel` | Channel < 4: loops to the literal 4 or to `settings_channels()`, `min(settings.channels, 4)` (new). Texel < 16: loops to 16, or `y * 4 + x` with both < 4 |
| Endpoints `ep`, `qep`, `array<_, 24>` | `8 * subset + 4 * end + channel` | Subset < 3 (`pairs` ≤ 3, from the mode, a literal), end < 2, channel < 4 as above: at most 23 |
| Endpoints by texel in `block_quant` | `ep[8 * j + 4 + p]`, j the texel's subset from the pattern | The pattern table's 2-subset entries hold subsets 0 and 1 only, its 3-subset entries 0 to 2: no subset 3 in any entry (checked over the table), so at most 23 |
| Quantized weights, `vec2<u32>` | `qblock[k0 >> 3]`, k0 a skip nibble | The skip table's nibbles are at most 15, so the index is at most 1 |
| Weight tables, `array<i32, 16>` | `get_unquant_value(bits, i)` | `bits` is 2, 3 or 4 (literals); i is clamped to `1..levels - 1` before, then i - 1 and i: at most 15 |
| Partition tables (pattern, mask, skips), 128 entries | `part_id` | From `part_list[k] & 63`, plus 64 in the 3-subset modes: at most 127. `best_part_id` starts at −1, so the tables are read through `partition_entry` (new), masked to 127 |
| Partition lists, `array<i32, 64>` | Loops to `part_count`, `partial_count` | Both come from the settings' thresholds: clamped to `PARTITIONS` (64) in `bc7_enc_mode01237` and `partial_sort_list` (new) |
| Mode tables (`refine_iterations`, `pairs`) | `[mode]` | The mode is a literal 0 to 7 at every call |
| Dual-plane rotation (modes 4 and 5) | Channel p | Loops to `settings_channels()` (new); `candidate_block` swaps only for rotation < 3 |
| The packed block, `data: array<u32, 5>` | `put_bits`: `pos / 32` and `pos / 32 + 1` | Every mode writes exactly 128 bits, so the second word is at most 4 |
| `MODE5_SOLID_ENDPOINTS`, 256 entries | A solid block's channel value | `channel_byte` (new) clamps it to 0..255, whatever the texture's format gives |
| The source texture | `textureLoad` at the block's texels | `texel_coordinates` (new) clamps to the texture's last row and column. In range anyway: page_encode's bands have sides that are multiples of 4 (`Band::pack`), the renderer's scratch texture is `BC7_SCRATCH_SIDE` square |
| The output buffer | `store_data`, `store_data_2` | Return when the block's last word is past `arrayLength` (new). In range anyway: `add_frame_at` asserts on the CPU that `offset + encoded_bytes` fits the buffer, `y % 4 == 0` and STORAGE usage; page_encode's output is exactly `encoded_bytes(band)` |
| The frame uniforms | Dynamic offset `index * stride` | Into a buffer of `frames * stride`; wgpu validates every dynamic offset on the CPU |
| BC1's local arrays | Block, colours, indices | Indexed in loops of constant bounds equal to their lengths |

The remaining `settings.channels == 3u` and `== 4u` tests are comparisons, not indices.

Checked (Radeon 880M, RADV):

- **The same blocks.** The ignored GPU tests (`BLOCK_ENCODE_PACKAGES=/home/claudio/test/uddp`): every profile's blocks
  are block_compression's checked kernel's byte for byte (0 of 28 312 blocks differ with alpha, 0 of 89 087 opaque),
  the solid and all-opaque blocks the CPU encoder's, BC1 0 of 62 629 blocks off the crate's, its alpha exact
  (`bc1_keeps_the_cpu_encoders_alpha`). `bc7_gpu digest` on 3 pages of each of the five sets (2 of the land) gives
  the same digest before and after for all 11 formats (55 of 55).
- **The kernel**, one page at a time on one thread (`bc7_gpu profile ... ultrafast packed`, two rounds alternating,
  the faster of each), below.

| Pages | Kernel before | Kernel after | Speed-up |
|---|---|---|---|
| Classic mobile, 20 | 0.130 s | 0.079 s (716 Mpx/s) | 1.6 × |
| Enhanced mobile, 16 | 0.095 s | 0.060 s (796 Mpx/s) | 1.6 × |
| Classic art, 20 | 0.051 s | 0.030 s (677 Mpx/s) | 1.7 × |
| Enhanced art, 16 | 0.070 s | 0.041 s (406 Mpx/s) | 1.7 × |
| Enhanced land, 6 | 0.056 s | 0.041 s (476 Mpx/s) | 1.4 × |

The page's wall on one thread drops less (Classic mobile 0.42 to 0.36 s): making the bands' texels on the CPU weighs
more than the kernel now.

- **Cold compile** (`time_every_format_compile` with an empty `MESA_SHADER_CACHE_DIR`, two rounds alternating, load 8
  to 11 from other agents): UltraFast 7.9 and 5.5 s before, 4.8 and 5.3 s after; UltraFast opaque 1.4 and 1.1 s
  before, 1.5 and 1.1 s after; BC1 0.24 s both; VeryFast 16.7 to 21.8 s, Basic 22.0 to 26.2 s and Slow 25.6 to 36.6 s
  on both sides. Level within the noise, UltraFast perhaps half a second shorter.
- **The renderer**: the regression harness's quick tier with the BC7 package set (`bc7-atlas/uddp-bc7`), a build
  before and after, in the four styles: of 95 captures, 76 identical and 19 within tolerance (at most 2 levels), none
  changed.

### The BC7 packages without RDO, and their strides

The test packages repacked with the code above (`pack-mobile-anims` / `pack-ec-mobile-anims --bc7 --zstd=9`, RDO
off, 2026-11-21):

| Package | Pack before (`--bc7-rdo`, λ 0.05, look-back 64) | Pack now | Size before | Size now |
|---|---|---|---|---|
| Classic | 983 s | 80 s (page encode 28 s) | 868.9 MB | 867.4 MB |
| Enhanced | 2048 s | 170 s (page encode 68 s) | 1932.4 MB | 1939.4 MB |

The stage log's BC7 input, encode, RDO and flatten times are now thread time summed over the pages encoded at once (398
s of encode inside Classic's 28 s), and the log says so.

`udd-cli tool strides measure changed` on the two new packages measured every clip, since a repack keeps no strides:

| Package | Clips | Measured | Sharing another's frames | With a stride | Without | Time |
|---|---|---|---|---|---|---|
| Classic | 1914 | 1706 | 208 | 880 | 1034 | 128 s |
| Enhanced | 4125 | 3498 | 627 | 1617 | 2508 | 261 s |

Hashing took 1–2 s, and the whole run took 398 s on the Radeon 880M.

Against the strides of the RGBA8888 packages (`tool strides export` of both, joined on body and action;
[`compare_strides.py`](atlas_page_compression/compare_strides.py)):

| Package | Strides in both | Median | p90 | p99 | > 10 % | > 25 % | Largest | Only RGBA / only BC7 |
|---|---|---|---|---|---|---|---|---|
| Classic | 877 | 0.81 % (0.007 tiles) | 3.8 % | 11.4 % | 11 | 5 | 47 % | 5 / 3 |
| Enhanced | 1613 | 1.24 % (0.014 tiles) | 4.7 % | 14.5 % | 44 | 6 | 65 % | 14 / 4 |

- **The measurement itself barely moves.** Per facing, over the clips with a stride in both, the median difference is
  0.76 % (Classic) and 1.27 % (Enhanced), and p90 is 3.5 % and 5.7 %. The mean signed difference is −0.001 and −0.005
  tiles, so BC7 does not bias the strides.
- **The RGBA strides are rounded.** Those packages store their strides to 0.01 tiles; the new ones keep the facings'
  median exactly. That rounding accounts for part of the small differences.
- **The worst cases are clips whose facings disagree.** The median of three facings that differ by two or three times
  flips to another facing on a small change. For example, Classic body 875 action 0 has south 0.54, south-west 1.11
  and west 0.15. BC7 moved south to 0.29, and the stride went from 0.54 to 0.29. Enhanced bodies 33, 35 and 36
  action 22 went from 0.59 to 0.35 when south-west dropped from 0.59 to 0.25. The run-time clamp (half to twice the
  clip's own cycle) bounds what such a clip does on screen.
- **A few clips cross the acceptance tests.** About 1 % of the strides exist in only one package: a facing measured or
  not, or a stride near the 0.25-tile minimum.

## Formats for the Classic pages

Which formats keep the Classic mobile animation frames and the Classic art best (2026-11-22, `--bin
classic_formats`). Both are binary in alpha: the sampled pages hold alpha 0 and 255 only, and every opaque texel is a
1555 colour widened as `c << 3` (uocf `rgba8888_word_from_bgra5551`); the animation frames are 256-colour palettes of
those per animation, the art has no palette. The transparent texels are (0, 0, 0, 0), all of them. Blocks of the
Classic mobile pages: 61 % transparent, 21 % all opaque, 17 % mixed (art 67, 19, 15 %).

**How the renderer reads them.** A sprite's tap (`client_bilinear_rect`, `mobile_frame_bilinear`) blends the four
texels straight, not premultiplied, then cuts at alpha one half (the opaque pass) and draws the rest blended. The RGB
of a transparent texel therefore reaches every edge pixel: next to a transparent black texel the edge darkens. Against
a premultiplied tap (sum of alpha x RGB over sum of alpha, where a transparent texel lends no colour) the lossless
pages' edges are at 22.2 to 22.8 dB: the dark fringe is in the data and the filter, whatever the format. (The
renderer has taken the premultiplied tap since, see "What to do"; the measurements below are of the straight one.)

Measured on the pages `bc7_gpu` samples (Classic mobile 20 pages, 56.8 Mpx; Classic art 20, 20.5 Mpx; Enhanced mobile
16, 48.0 Mpx, for contrast), each page compressed alone. Opaque PSNR is RGB over the texels of alpha 255 only. Flips
are texels whose alpha lands on the other side of 128; ghosts are transparent texels decoded with some alpha, which
the blended pass draws faintly. Edges are the 2x2 taps with a transparent texel and a source alpha of at least one
half, RGB, against the lossless page's straight tap and against the premultiplied one. Sizes in MiB:

| Classic mobile, 20 pages | VRAM bpp | Zstd 9 | Zstd 19 | Opaque PSNR | Flips | Ghosts | Edges vs lossless | vs premultiplied |
|---|---|---|---|---|---|---|---|---|
| RGBA8888 (lossless) | 32 | 17.49 | 14.50 | lossless | 0 | 0 | lossless | 22.44 dB |
| 1555 (lossless) | 16 | 15.32 | 13.51 | lossless | 0 | 0 | lossless | 22.44 dB |
| R8 palette indices (lossless), without the palettes | 8 | 12.87 | 12.20 | lossless | 0 | 0 | lossless | 22.44 dB |
| BC7 CPU (AnalyticalWide) | 8 | 13.69 | 13.18 | 33.59 dB | 0 | 596 | 37.75 dB | 22.28 dB |
| BC7 GPU UltraFast | 8 | 13.66 | 13.15 | 33.31 dB | 0 | 60 | 37.77 dB | 22.29 dB |
| BC7 GPU VeryFast | 8 | 14.14 | 13.66 | 34.93 dB | 0 | 771 | 39.33 dB | 22.35 dB |
| BC7 GPU Fast | 8 | 15.02 | 14.62 | 35.91 dB | 0 | 751 | 39.44 dB | 22.35 dB |
| BC7 GPU Basic | 8 | 15.07 | 14.67 | 36.13 dB | 0 | 739 | 39.47 dB | 22.35 dB |
| BC7 GPU Slow | 8 | 15.11 | 14.70 | 36.20 dB | 0 | 734 | 39.48 dB | 22.35 dB |
| UltraFast, all-opaque blocks opaque UltraFast | 8 | 13.80 | 13.30 | 33.32 dB | 0 | 60 | 37.77 dB | 22.29 dB |
| UltraFast, all-opaque blocks opaque Basic | 8 | 14.55 | 14.09 | 35.44 dB | 0 | 60 | 38.07 dB | 22.30 dB |
| BC7 CPU, transparent texels nearest colour | 8 | 14.79 | 14.33 | 34.21 dB | 0 | 97 | 22.87 dB | 29.20 dB |
| BC7 CPU, transparent texels block mean | 8 | 14.32 | 13.84 | 34.25 dB | 0 | 0 | 22.29 dB | 29.95 dB |
| UltraFast, transparent texels nearest colour | 8 | 14.77 | 14.31 | 33.87 dB | 0 | 1 | 22.87 dB | 29.18 dB |
| UltraFast, transparent texels block mean | 8 | 14.30 | 13.82 | 33.91 dB | 0 | 0 | 22.30 dB | 29.95 dB |
| BC1 CPU (dds, the gumps' encoder) | 4 | 8.22 | 7.71 | 30.09 dB | 0 | 0 | 35.60 dB | 22.27 dB |
| BC1 GPU | 4 | 8.23 | 7.72 | 29.58 dB | 0 | 0 | 34.98 dB | 22.24 dB |
| BC2 (dds) | 8 | 9.48 | 8.51 | 29.99 dB | 0 | 0 | 34.90 dB | 22.13 dB |
| BC3 (dds) | 8 | 9.51 | 8.56 | 29.99 dB | 0 | 0 | 34.90 dB | 22.13 dB |
| BC3, transparent texels nearest colour | 8 | 10.29 | 9.32 | 30.71 dB | 0 | 0 | 22.79 dB | 28.83 dB |

| Classic art, 20 pages | VRAM bpp | Zstd 9 | Zstd 19 | Opaque PSNR | Flips | Ghosts | Edges vs lossless | vs premultiplied |
|---|---|---|---|---|---|---|---|---|
| RGBA8888 (lossless) | 32 | 5.20 | 4.58 | lossless | 0 | 0 | lossless | 22.23 dB |
| 1555 (lossless) | 16 | 4.70 | 4.25 | lossless | 0 | 0 | lossless | 22.23 dB |
| BC7 CPU | 8 | 4.15 | 4.04 | 33.36 dB | 1 | 987 | 37.46 dB | 22.07 dB |
| BC7 GPU UltraFast | 8 | 4.16 | 4.04 | 33.13 dB | 0 | 333 | 37.58 dB | 22.08 dB |
| BC7 GPU VeryFast | 8 | 4.38 | 4.26 | 35.31 dB | 2 | 459 | 39.98 dB | 22.16 dB |
| BC7 GPU Fast | 8 | 4.57 | 4.47 | 36.25 dB | 2 | 458 | 40.07 dB | 22.16 dB |
| BC7 GPU Basic | 8 | 4.59 | 4.49 | 36.47 dB | 2 | 458 | 40.12 dB | 22.17 dB |
| BC7 GPU Slow | 8 | 4.60 | 4.50 | 36.57 dB | 2 | 448 | 40.13 dB | 22.17 dB |
| UltraFast, all-opaque blocks opaque UltraFast | 8 | 4.17 | 4.08 | 33.14 dB | 0 | 333 | 37.58 dB | 22.08 dB |
| UltraFast, all-opaque blocks opaque Basic | 8 | 4.35 | 4.25 | 35.12 dB | 0 | 333 | 37.82 dB | 22.08 dB |
| BC7 CPU, transparent texels nearest colour | 8 | 4.50 | 4.38 | 34.35 dB | 0 | 18 | 22.50 dB | 30.31 dB |
| BC7 CPU, transparent texels block mean | 8 | 4.40 | 4.28 | 34.43 dB | 0 | 0 | 22.37 dB | 30.78 dB |
| UltraFast, transparent texels block mean | 8 | 4.41 | 4.29 | 34.14 dB | 0 | 0 | 22.38 dB | 30.79 dB |
| BC1 CPU | 4 | 2.57 | 2.42 | 31.12 dB | 0 | 0 | 37.64 dB | 22.08 dB |
| BC1 GPU | 4 | 2.56 | 2.42 | 30.66 dB | 0 | 0 | 37.10 dB | 22.08 dB |
| BC2 | 8 | 2.86 | 2.65 | 30.55 dB | 0 | 0 | 35.76 dB | 21.96 dB |
| BC3 | 8 | 2.88 | 2.66 | 30.55 dB | 0 | 0 | 35.76 dB | 21.96 dB |

The Enhanced mobile pages, whose alpha is soft (7.7 % of the texels between 0 and 255), for contrast: BC7 UltraFast
11.68 MiB at Zstd 19 (67 % of the lossless 17.34) at 31.86 dB, VeryFast 12.29 MiB at 33.81 dB, Basic 13.03 MiB at
35.05 dB. 1555 and BC1 cut their soft alpha (575 811 edge taps change side); BC3 keeps it at 9.08 MiB but 28.74 dB.
On these pages flips and ghosts count soft alpha moving, not errors of a binary edge.

What it says:

- **Binary alpha costs BC7 nothing at the cutoff.** Every BC7 variant flips at most 2 of 77 Mpx across one half. The
  ghosts (60 to 987 transparent texels decoded with a little alpha) are the only alpha error that shows, faintly, in
  the blended pass. Weighting alpha by the threshold's side would have nothing to win there; making the ghosts 0
  would.
- **On disk BC7 barely beats lossless.** At Zstd 19 the Classic BC7 mobile pages are 91 % of the RGBA8888 ones and 97 %
  of 1555's; the art 88 % and 95 %. BC7's gain on these pages is the VRAM (8 bpp against 32 or 16), not the package.
- **The palette indices beat BC7 and are lossless** (below): 12.20 MiB against BC7's 13.18 at Zstd 19, at BC7's 8 bpp.
- **UltraFast is the efficient BC7.** VeryFast gives +1.6 dB (mobile) and +2.2 dB (art) for +3.9 % and +5.4 % at Zstd
  19; Fast to Slow give +2.6 to 3.4 dB for +11 %. The all-opaque blocks on the opaque profile's modes (0 to 3 and 6)
  give nothing at UltraFast (+0.01 dB for +1 %, and its mode 6 alpha then decodes 254 on 6.6 M texels). At Basic they
  give +2.1 dB (mobile) and +2.0 dB (art) for +7 % and +5 %, no better per byte than VeryFast on the whole page. As
  a second dispatch (`classic_formats split`: the alpha profile on the page with its all-opaque blocks made
  transparent, which page_encode then skips as solid, and the opaque profile on those blocks packed 256 to a row;
  quiet machine, best of 2), the opaque UltraFast costs what the whole page does: 0.288 against 0.269 s on one
  thread, 0.058 against 0.092 s from 20 threads (Classic mobile; art 0.113 against 0.109 s, 0.028 against 0.035 s).
  The opaque Basic takes 0.72 s and 0.49 s (art 0.28 and 0.19 s), 2.7 and 5 times the whole page's time, about
  VeryFast's on the whole page (0.53 s from 20 threads), which gains more per byte (2.4 % per dB on the Classic
  mobile pages, against 3.3 %).
- **Filling the transparent texels' RGB** with the block's mean opaque colour (as a "don't care" texel would pull the
  endpoints nowhere) or the nearest opaque colour gives +0.6 to +1.1 dB on the opaque texels. The edges against the
  premultiplied tap go from 22 to 30 dB and the ghosts go away, for +5.0 to +6.2 % at Zstd 19 with the block mean and
  +8 to +9 % with the nearest colour. The bleed it removes is the same on the lossless pages: it comes from the renderer's straight tap over
  black texels. A real "don't care" in the kernel (the transparent texels' RGB out of the endpoint fit and the error,
  alpha weighted by the cutoff's side) was not prototyped. The block mean approximates its fit, and its size cost
  suggests that Zstd pays for the transparent texels' indices, not their colours.
- **BC1** is half of BC7's size at Zstd 19 (7.71 against 13.18 MiB) for 3.2 to 3.5 dB less colour, with the alpha
  exact (punch-through: no flips, no ghosts, transparent black).
- **BC2 and BC3** keep the alpha exact too, but their colour block is BC1's four-colour one: no better colour than BC1
  (−0.1 to −0.6 dB) for 10 % more bytes and twice the VRAM. BC2's explicit 4-bit alpha and BC3's BC4 alpha do equally
  well for binary alpha.
- **BC4 and BC5** hold one and two channels, so they carry no RGB and are of no use for these pages. BC4 cannot carry
  palette indices either, being lossy.

### Palette indices for the Classic animation frames

`classic_formats palettes` over every page of the RGBA8888 `mobile_anim_cc.uddp` (1210 pages) finds 140 050
animations (body, action and direction) with frames, and 115 232 distinct palettes. An animation uses 75 colours at
the median, and 255 at p99 and at most. So an R8 index with 0 for transparent holds every one (no texel past 255
colours). The directions do not share a palette: merged per body and action, 3 901 of the 22 109 palettes pass 255
colours. All the palettes take 27.95 MB as 1555 (24.53 MB deduplicated), 15.80 MB at Zstd 19.

The sampled pages as R8 indices (each frame's texels index its animation's palette, in the order the frames scan
them) take 12.87 MiB at Zstd 9 and 12.20 MiB at Zstd 19, 6 and 7 % under BC7. Scaled from the 20 pages to the
package's 1210, that is about 738 MiB of indices plus 15 MiB of palettes, against about 797 MiB of BC7 at Zstd 19:
about 5 % smaller and lossless, at BC7's 8 bpp in VRAM.

What the renderer would need (a proposal, not made; a UDDP format change needs the user's confirmation):

- An `R8Uint` page array for the Classic frames beside the RGBA8 and BC7 atlases, and a palette per animation: a
  storage buffer of 1555 or RGBA8 words with an offset per animation. That is 28 MB as 1555 for all of them, or only
  the animations whose pages are resident.
- The palette's offset per mobile instance, beside its page layer, and per crossfade frame, since the next frame's
  animation can differ.
- The tap: `textureLoad` of the four indices, each looked up in the palette (index 0 transparent), then the bilinear
  blend in the shader, as `client_bilinear_rect` already does with four loads; nearest is one load. Indices get no
  hardware filtering.
- Hues: the Classic hue tables map the colour, which the palette lookup gives as before.
- The RIFE and SPAN in-between frames are made as RGBA on the GPU and stay BC7, in their own atlas as now.

### BC7 frames as they show, at 1x and after SPAN

Whether the BC7 UltraFast of the Classic animation frames shows, and whether SPAN 2x makes it worse (2026-11-22,
`--bin classic_span`, Radeon 880M). The frames are encoded as the packer does (whole lossless pages on the GPU), then
cut out. SPAN 2x is uot-gpu-compute's `FrameUpscaler` with the alpha snapped, as the renderer runs it on Classic frames,
and its output is encoded BC7 UltraFast as the renderer does (`MOBILE_BLOCK_FORMAT`). PSNR is RGB over the texels the
reference has opaque; flips are texels whose alpha crosses one half. 200 frames from 20 pages of `mobile_anim_cc.uddp`:

| 1x input | 1x vs lossless | flips | SPAN 2x vs SPAN of lossless | flips | × MSE of 1x | game's 2x (SPAN, then BC7) vs SPAN of lossless | game's 2x vs the lossless package's | flips |
|---|---|---|---|---|---|---|---|---|
| lossless | lossless | 0 | lossless | 0 | - | 35.86 dB | lossless | 0 |
| BC7 UltraFast (the packages) | 33.14 dB | 0 | 32.35 dB | 0 | 1.20 | 31.36 dB | 32.11 dB | 0 |
| BC7 VeryFast | 34.69 dB | 0 | 33.87 dB | 6 | 1.21 | 32.28 dB | 33.34 dB | 6 |
| BC1 | 29.45 dB | 0 | 29.10 dB | 0 | 1.08 | 28.61 dB | 28.95 dB | 0 |

Per frame (median, tenth percentile, worst): UltraFast at 1x 33.0, 27.2, 24.4 dB; the game's 2x from the BC7 package
30.3, 26.3, 23.7 dB and from the lossless one 34.2, 29.6, 25.8 dB. The seven frames of the sheets, at 1x: human 35.7,
monk's robe 30.8, orc 34.7, horse 38.9, dragon 38.1, fire elemental 29.4, skeleton 32.0 dB (UltraFast).

- **SPAN does not amplify the BC7 error.** Its 2x output from a BC7 frame is 1.2 times the 1x error in MSE (0.8 dB),
  and no alpha flips. It neither sharpens the blocks' noise into structure nor smooths it away.
- **The 2x frame is BC7 anyway.** Its own UltraFast encode costs 35.9 dB from a lossless input; from the BC7 package
  the two errors add to 31.4 dB. Lossless source frames would buy those 4.5 dB on the upscaled frames, as they buy
  33.1 dB to lossless at 1x.

In game (Classic style, Classic sources, Britain, the player as human 400, orc 17 and human on horse 200, standing; the
RGBA8 against the BC7 `mobile_anim_cc.uddp`, everything else the same): at zoom 1, 0.46 to 1.05 % of the pixels around
the mobile differ by more than 6 of 255, mean difference 0.16 to 0.44; magnified 4x, 0.94 to 2.13 % with SPAN off and
2.35 to 4.53 % with SPAN 2x, mean 0.32 to 1.32. Every difference is on the mobile (none on the art, the same package).

Verdict, by eye on the two states side by side (the stills of the flicker videos) and on the amplified differences:

- **At 1x it does not show.** At zoom 1 the two packages' mobiles look the same; the per-frame sheets at 3x nearest
  show UltraFast and lossless the same too. The difference appears only amplified 8x.
- **Magnified 4x it does not show either.** Side by side the human, the orc and the rider look the same. Amplified,
  the error sits on the sharp colour steps (the dark outlines against a bright fill, the fire elemental's rim) more
  than on flat shading. No edge moves (no flips) and no block shows as a block. A toggle between the two may let
  the eye catch a slight shimmer on those steps; a single view does not.
- **After SPAN the same, a little more.** The SPAN frame from BC7 differs from the one from lossless in fine noise
  along the same steps (31.4 against 35.9 dB from the lossless package), not in any artefact SPAN made of the blocks.
- **So the palette indices are worth it for exactness and size, not for a visible flaw.** They remain the better
  format (lossless, 5 % smaller than BC7 on disk, same VRAM), and they would lift the SPAN frames by those 4.5 dB, but
  BC7 UltraFast is not visibly worse at any zoom the renderer offers, with SPAN or without.

### What to do

- Keep the GPU's BC7 UltraFast for the Classic art and as the frames' fallback. It matches the CPU's size and
  quality, nothing gains more per byte, and the opaque-profile split does not pay.
- For the Classic mobile frames, use R8 palette indices: lossless, 5 % smaller than BC7 on disk, at BC7's VRAM, with
  the renderer work above. It is the largest quality step on offer (from BC7's 33.6 dB to lossless) at no size cost.
- The edge bleed came from the straight tap, not the format. Since 2026-11-22 the sprite taps under the client filter
  (`client_filtered_rect`, `mobile_frame_client_filtered`) blend premultiplied and return straight alpha, which removes
  the dark fringe for every format at no size cost, where filling the texels would cost 5 %. With it, a "don't care"
  encode of the transparent texels would be safe.
- Use BC1 only where half the size matters more than 3.5 dB (it is what the Classic gumps use), and BC2 or BC3 never
  for these pages.

## Atmosphere maps and light textures

`tex_aux_ec.uddp` stores the textures under its reserved keys lossless, RGBA8888 at their own size, whatever the other
textures take: the three atmosphere maps of `TerrainTexture.uop` (entries 20 to 22, day, night and night sight;
the renderer samples only the night map, entry 21) and its 29 light textures (25 at 256², 4 at 128²). The packer
encodes its missing-texture image as the other textures. The client ships the 32 as DXT5 (the light textures) and
DXT1 (the three maps), without mips, so the lossless texels are the decoded DXT and a BC7 re-encode adds its error on
top of that. None of them is in the renderer's auxiliary array: the night composite
(`client_night.rs`) and the light sprites (`client_light_sprites.rs`) each upload their own `Rgba8Unorm` image
through `load_terrain_texture_rgba`, which decodes BC7 on the CPU. Two other textures are always lossless: the
Classic `light.mul` lights in `world_lights.uddp` (55 lights, one signed byte per texel, from which the renderer
builds hued `Rgba8Snorm` textures, so no texture exists at pack time) and the Kingdom Reborn grunge of
`tex_land_ec.uddp`.

Measured on the test packages of 2026-11-16 with `--bin aux_maps` (2026-11-22). BC7 is the packers' encoder
(`AnalyticalWide`, RDO off) and the ISPC kernels of `block_compression` at `alpha_slow`, the ones uot-gpu-compute
runs on the GPU. BC4 keeps the alpha with the texture's mean colour, and BC5 the alpha and the luminance. Sizes are
Zstd 19 of the payload. PSNR and the largest error are taken against the lossless texels: RGB where alpha is above 0,
and the colour times alpha, which the light buffer adds.

| Group | Lossless (Zstd) | BC7 | ISPC BC7 | BC4 | BC5 | VRAM RGBA / BC7 / BC4 | BC7 error | ISPC error | BC4 / BC5 error |
|---|---|---|---|---|---|---|---|---|---|
| Atmosphere maps (3) | 97.3 KiB | 85.6 | 117.9 | — | 43.4 | 768 / 192 / 96 KiB | 51.0 dB, max 7 | 62.5 dB, max 3 | 30.0 / 36.9 dB, max 40 / 27 |
| Night map alone | 18.7 KiB | 19.7 | 28.0 | — | 12.1 | 256 / 64 / 32 KiB | 51.3 dB, max 4 | 68.6 dB, max 2 | — |
| Light textures (29) | 500.6 KiB | 460.2 | 431.4 | 336.7 | 414.4 | 6464 / 1616 / 808 KiB | 60.7 dB, max 4.6 (colour 54.7, max 8) | 60.2 dB, max 4.7 | 44.4 / 44.5 dB, max 48 / 43 |
| Missing texture | 2.0 KiB | 2.3 | 3.1 | — | 1.8 | 256 / 64 / 32 KiB | 58.3 dB, max 7 | 63.5 dB, max 6 | — |
| `light.mul` (55) | 328.0 KiB | — | — | 382.3 | — | 11,325 / — / 1433 KiB | — | — | BC4 0.24 steps RMS, max 3 of 31 |

- **Size.** BC7 saves 52 KiB of the 6 MB package: 12 KiB on the maps and 40 KiB on the lights. Several textures grow:
  the night map, light 1 (7.9 to 14.3 KiB), light 17 (2.9 to 6.8 KiB) and lights 37 to 40 (7.4 KiB each to 10 to 14).
- **VRAM.** Nothing changes unless the night composite and the light sprites upload BC7 blocks, and today they decode
  them. With those uploads, the saving would be 4.7 MiB if every light texture were loaded at once (the sprites load
  a light's texture only when a light in view uses it) and 192 KiB for the night map.
- **Error.** BC7 keeps the lights within 4.6 levels of light added. On the night map, a near-black texture that the
  composite scales by four, BC7 shifts the mean by +0.65, +0.53 and +0.20 levels, from (3.21, 4.41, 39.22) to (3.86,
  4.94, 39.42). That is 20 % more red and 12 % more green: below the 1/255 tolerance of the `NIGHT_MAP_MEAN` check,
  but a bias where the original texels round to 3 and 4. The ISPC kernels shift it by much less (68.6 dB, max 2).
  The glow colours of the land and static lights, the alpha-weighted mean of their textures, move by at most one level.
- **BC4 and BC5 do not fit.** The light textures' colour is not constant (a hot core and a coloured rim), so keeping
  the alpha alone or the luminance loses up to 48 levels of added light. The atmosphere maps are colour gradients.
  `light.mul` is smaller as Zstd bytes than as BC4, and its runtime textures (`Rgba8Snorm`, one per light, hue and
  style: the VRAM column counts one each) carry the hue, so BC4 would have to be encoded at load.
- **Mixed formats.** These textures never reach the auxiliary array, so storing them BC7 does nothing for the
  mixed-format fallback of a BC7 package. The fallback came from `AuxLayerFormat::for_slots` counting their RGBA
  slots. It now counts the slots the array holds (all but the reserved keys, plus the missing texture), and the Kingdom
  Reborn section takes the Enhanced options. A BC7 package then keeps its array BC7: 651 textures of 256 KiB each as
  RGBA, against 64 KiB as BC7. With the fix, a copy whose array textures (Kingdom Reborn section included) are BC7
  loads without the warning and draws the 12 views of Britain's lamp posts, the lava and Deceit within 1 level of
  the same textures decoded to RGBA8.

**On screen the loss does not show.** The quick tier's night and cave views (45 captures, all four styles) were
taken with one binary and two copies of the package (`--bin aux_bc7_variant`): BC7 textures in the array, with the
maps and light textures lossless or BC7. Classic matches exactly, since it draws neither. In the Enhanced, KR-like
and UOD-Painterly styles, at most 0.013 % of the pixels differ by more than 4 levels (Deceit's candles, largest 8).
Elsewhere the largest difference is 6 levels (Britain's slate roofs at light level 22 and the lava), and most views
stay within 3 or 4 levels. The harness still lists 34 of them as changed, because the glow colours move the
static-light and light-sprite digests. Side by side, and toggled in a video, the frames cannot be told apart. The difference, amplified 16 times, is the night map's 4×4 blocks tiled over
the ground, and it is one-sided: BC7 brightens the night frames by 0.2 to 0.5 levels on average (Britain's lamp posts
+0.30, +0.23 and +0.13 in the Enhanced style; Deceit +0.38, +0.34 and +0.15). BC7 therefore costs nothing visible
here, and it gains 52 KiB on disk and no VRAM unless the renderer uploads blocks.

**Decision (the user, 2026-11-22): the atmosphere maps and the light textures stay lossless.** The BC7 option for them
was not merged; the measurements above stay as the record of what it would give.

A lossless copy of the client's own texels would be the DXT1 and DXT5 blocks as shipped: bit-exact, 0.5 to 1 byte
per texel in VRAM, and no re-encode. `PagePixelFormat` has only RGBA8888 and BC7, though, so that needs a format
change.

## GPU block encoding (uot-gpu-compute)

`uot_gpu_compute::block_encode::MobileBlockEncoder` encodes RGBA8 textures to BC7 or BC1 blocks on the GPU, with our
copies of block_compression 0.11.0's kernels (`shaders/block_encode/bc7.wgsl`, `bc1.wgsl`; MIT, CREDITS.md). The
renderer uses it for its BC7 in-betweens, the converter's BC encode device (`page_encode::PageEncoderGpu`) for its BC7
pages and BC1 gumps (2026-11-22). The API:

- `MobileBlockFormat::{Bc7(Bc7Quality), Bc7Opaque(Bc7Quality), Bc1}`, `Bc7Quality::{UltraFast, VeryFast, Fast, Basic,
  Slow}` (block_compression's `alpha_*` and `opaque_*` settings, `Bc7Profile::of`). `texture_format()`,
  `block_bytes()`, `encoded_bytes(width, height)` and `compile_name()` on the format; `padded_size` for the block grid.
- `MobileBlockEncoder::new(device, queue)` (or `compile(device, queue, report)`) compiles `WARM_BLOCK_FORMAT`
  (`Bc7(UltraFast)`), `lazy(device)` nothing; `compile_format(format, report)` compiles another ahead of use,
  `is_compiled(format)` asks. Each format is its own pipeline, compiled once, the first time it is wanted.
- `add_frame(format, source_view, width, height, output_buffer, offset)` and `add_frame_at(..., y, ...)` (a frame at
  row `y` of the source, a multiple of 4, for BC7 and BC1 alike) queue frames of any formats, compiling a format not
  compiled yet on the caller's thread; `record(pass)` records them into a compute pass, with a uniform buffer of that
  record's own, so several records may share a submission. The blocks are written row-major into the storage buffer
  from `offset` (a multiple of 4); `record_copy_to_layer` copies a frame's blocks into a texture layer.

**The same blocks as the crate, but two kinds.** Every BC7 profile (5 qualities, alpha and opaque) encodes the crate's
blocks byte for byte: 0 of 188,169 blocks differ on the first and middle page of `mobile_anim_cc`/`_ec` and
`tex_art_cc`/`_ec` (512² crops) and Classic and Enhanced frames. Since 2026-11-22 two kinds of block are encoded as the
CPU encoder does instead, for blocks Zstd compresses better ("Optimizing the GPU BC7 encode", above): a block of one
colour is the CPU's solid block, and an all-opaque block of a frame with alpha skips modes 4 and 5 and keeps p-bits 1
in modes 6 and 7 (`our_kernels_encode_as_block_compression` counts and checks both). Only the modes a profile tries
are WGSL constants; the numeric settings
stay in the crate's storage buffer, since as constants up to 0.3 % of the blocks came out different
(shader_compile_progress.md, "Our block encoder").

**BC1 punch-through.** The packages' BC1 is the CPU's (uot-texture-codecs, the dds crate): CC gumps, Classic EC gumps
and the EC effects; no atlas page or mobile frame is BC1, so the renderer has no GPU BC1 user. The crate's BC1 is colour
only, in the four-colour mode, and ignores a frame's row. Ours keeps its four-colour encode for the blocks all opaque
(byte for byte the crate's: 0 of 62,629 differ, but for 415 blocks whose two endpoints come out equal, which a
decoder reads in the three-colour mode, where the crate's index 3 would be transparent black: ours writes indices 0
there, the one colour of the block), and encodes a block with a
texel of alpha below 128 in the three-colour mode, index 3 transparent black, as the CPU's does; an empty block is
endpoints 0 and indices all 3. Against the CPU's on Classic frames, Classic page crops and frames on a block's edge:
every texel decodes opaque or transparent as the CPU's, every transparent one black. The colour of the opaque texels is
0.57 to 0.75 dB below the CPU's in the four-colour blocks (ISPC's endpoint search, as the crate has it) and 0.52 to
0.63 dB in the three-colour ones (`bc1_keeps_the_cpu_encoders_alpha`).

**BC1 gumps on the GPU.** The 3397 single gumps of a lossless `gumps_cc` (83.1 Mpx, the paperdoll body gumps left
out), encoded side by side from the 20 rayon threads as the writer does (`udd_conv` test
`the_gpu_bc1_gumps_match_the_cpus`, `LOSSLESS_GUMPS_CC` set; debug build of the tests, optimized):

| Encoder | Encode | Zstd 19 | RGB PSNR, opaque texels | Alpha |
|---|---|---|---|---|
| CPU (dds crate) | 3.95 to 5.22 s | 18 690 536 bytes | 32.59 dB | exact |
| GPU | 0.37 s (0.65 s with the cold compile) | 18 658 508 bytes (−0.2 %) | 32.07 dB | exact |

The whole `pack-gumps` (lossy, Zstd 19, the same build) takes 16.7 s on the CPU and 12.4 to 12.7 s on the GPU, cold
or warm Mesa cache, for a package of 20 318 441 bytes against 20 350 514; two GPU packs are the same file, and it
validates. The GPU is 10 times faster at the encode for half a dB less colour, the ISPC endpoint search's; its one bit
of alpha is the CPU's on every texel. Raising the kernel's `refine_iterations` might close the gap, but it would leave
block_compression's blocks, which the opaque blocks match today.

**Compile times** (Radeon 880M, RADV, cold Mesa cache): BC7 UltraFast 5.4 to 8 s, Slow with alpha 22 s (the crate's
one pipeline for every quality was 22 to 25 s), BC1 0.25 s; warm 5 to 15 ms each. Every profile in
shader_compile_progress.md. The converter's encoder compiles only the formats its pages take, at their first page.

## Reproducing

[`atlas_page_compression/`](atlas_page_compression/) is the benchmark, a standalone crate that reads its pages from a
package folder:

```sh
cd docs/agent_notes/atlas_page_compression
CARGO_TARGET_DIR=/path/to/scratch cargo run --release -- /path/to/uddp
```

It runs every codec and the 128-row strips. `--bin strip_levels` runs the levels and codecs in strips (it reads
`tex_land_ec.uddp` too), `--bin block_pages <dir> [lanczos|mmpx] [ec|cc|both]` the BC1 and BC7 pages, and `--bin upscaled_pages <dir>
[ec|cc|both] [all|bc1]` the pages at 1× to 4× (`bc1` only BC1 and BC1 with RDO). The PNG filter, the other strip heights and
the dictionaries were one-off runs with the numbers above. `--bin anim_rdo <dir> <cc|ec> <pages> [λ,...] [look-back,...]
[stats|parallel]` runs the BC7 RDO on pages sampled from an RGBA8888 `mobile_anim_cc.uddp` or `mobile_anim_ec.uddp`:
`stats` adds the RDO's counters for one page on one thread, `parallel` runs the pages at once as the packers do, and
`ANIM_RDO_SIMD=scalar|avx2|avx512` sets the highest instruction set. `--bin rdo_gpu_proto <dir> <cc|ec> <pages> <λ>
<look-back>` times the GPU prototype against the CPU RDO on the same pages, and `--bin rdo_gpu <dir> <cc|ec> <pages>
<λ,...> <look-back,...> [<crop dir> <page> <x> <y> <width> <height>]` the GPU RDO (with its compile time; set
`MESA_SHADER_CACHE_DIR` to an empty folder for a cold one), writing the crops of the visual check as raw RGBA.
`--bin aux_maps <dir> [<dump dir>]` measures the atmosphere maps, light textures and `light.mul` lights of
`tex_aux_ec.uddp` and `world_lights.uddp` in `<dir>`, and with a dump folder writes each texture and its BC7 decode as
raw RGBA. `--bin aux_bc7_variant <tex_aux_ec.uddp> <output> reserved|array|all` writes the copy of the rendered
check: the atmosphere maps and light textures BC7, the array's textures BC7, or both.
`--bin bc7_gpu <dir> <mobile_cc|mobile_ec|art_cc|art_ec|land_ec> <pages> [profile,...]` times the GPU BC7 encode
against the CPU's on pages of a lossless package (pack the art and land with `--raw --zstd=9`), and `--bin bc7_gpu
compare <art_cc|art_ec|land_ec> <lossless dir> <BC7 dir>` gives the PSNR of a packed BC7 package's
images against a lossless package's, matched by id (the two lay their pages out differently). Set `dumped` reads
`<width>x<height>_<n>.rgba` pages from the directory (raw RGBA8, a packer's BC7 encoder input dumped by a temporary
hook), and `BC7_GPU_ZSTD` sets the Zstd level (9 by default; 19 as the packages). `--bin bc7_gpu stats <dir> <set>
<pages> [profile]` sorts the blocks by kind and gives each kind's modes and Zstd share on both encoders, `variants` the
size with some kinds swapped for the CPU's or the opaque profile's, `fields` the alpha endpoints and p-bits of the
opaque blocks' mode 6, and `profile <dir> <set> <pages> [profile] [packed]` the texels, upload, kernel and readback time
of one page at a time (`packed` as `page_encode` sends its bands). `--bin bc7_gpu digest <dir> <set> <pages>` hashes
every GPU format's blocks on the pages, to compare two builds of the kernels. `--bin classic_formats <dir> <mobile_cc|art_cc|mobile_ec>
<pages> [variant,...]` measures the formats of "Formats for the Classic pages" on a lossless package's pages, `texels`
their alpha and transparent texels, `palettes <dir> <pages>` the Classic animations' palettes and the pages as indices,
and `split` the GPU's UltraFast against the two dispatches. `--bin classic_span psnr <dir> <pages> <frames>` gives
"BC7 frames as they show" on frames sampled from an RGBA8888 `mobile_anim_cc.uddp`, and `--bin classic_span frames
<dir> <out dir> <name>=<body>:<action>:<direction>:<frame> ...` writes each named frame lossless, BC7 UltraFast,
VeryFast and BC1, and SPAN 2x of the lossless and the UltraFast frame, as raw RGBA (`<name>_<variant>_<w>x<h>.rgba`);
`python3 -I -B classic_span_sheets.py <out dir> <sheets dir>` draws their sheets and the flicker pictures.
