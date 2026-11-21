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

- **Only one page is LZ4 HC at 1×:** the default packages are lossless RGBA, where LZ4 HC costs 44–131 %, so they come
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
- **A full version** in `uot-gpu-compute`, called by udd-conv, has to choose between two semantics:
  - port the CPU's exact search (the hash, the continuation and REP0 state that changes within a block's search, the
    ultrasmooth scales, modes other than 6 and 4 on the CPU), which costs a sequential pass per block;
  - define its own deterministic RDO, whose output differs from the CPU's and depends on the chunking.

  With RDO off by default, it pays only for someone who opts in at λ 1 or more.

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
<look-back>` times the GPU prototype against the CPU RDO on the same pages.
