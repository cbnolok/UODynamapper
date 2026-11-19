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

It runs every codec and the 128-row strips. `--bin strip_levels` runs the levels and codecs in strips (it reads
`tex_land_ec.uddp` too), and `--bin block_pages` the BC1 and BC7 pages. The PNG filter, the other strip heights and
the dictionaries were one-off runs with the numbers above.
