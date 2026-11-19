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

`tex_art_ec.uddp` and `tex_land_ec.uddp` repacked in 128-row strips hold byte-identical pages and grow by 8.5 %
(60.2 to 65.4 MB and 21.1 to 23.0 MB). Read through `udd-container`, which decodes the frames of a payload of 1 MiB or
more in parallel, an art page takes 8–9 ms instead of 62–86 ms, and a land page about 4 ms instead of 25–31 ms.

The strip height is a multiple of 8 rows, so a strip holds whole block rows of BC7 and of 4×4 and 8×8 ASTC pages, and a
decoded strip could go straight to its rows of a block-compressed texture. The frames are stored one after another as
plain Zstd, so the package format does not change, and a reader that does not split them decodes them in order.

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

It runs every codec and the 128-row strips. The PNG filter, the other strip heights and the dictionaries were one-off
runs with the numbers above.
