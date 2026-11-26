# BC7 animation pages: lossless supercompression

Can the BC7 pages of the mobile animation packages be stored smaller without changing a single block, and what would
it cost the game to decode them (2026-11-25)? The brief proposed regrouping, dictionaries, byte shuffles, BC7-aware
field splits and block reuse in front of LZ4; the container's codec for these pages is Zstd, so everything here is
measured against Zstd 9 as the packages store it, and against LZ4 HC. Nothing in the production packers or the format
changed: `udd-bc7-supercompress` (`tools/udd-devtools-cli/src/bin/bc7_supercompress/`) only reads packages. What
changing the blocks would buy is the third study, [Lossy conditioning](#lossy-conditioning). Since 2026-11-26 the
field split is the packers' default, joined per clip behind a 6-page cache: see [Adopted](#adopted-2026-11-26).

## LZ4 HC on the animation pages

The animation pages are plain Zstd 9 (`MOBILE_ANIM_PAGE_COMPRESSION`, `lib/udd-conv/src/atlas/page_compression.rs`).
The rule that stores a page as one LZ4 HC 12 block when that is at most 20 % larger than one Zstd frame
(`CompressionFlag::ZstdOrLz4HcLevel`, `PAGE_LZ4_HC_MAX_GROWTH_PERCENT`) is applied only by
`atlas_page_compression`, to the gump, art and land pages; it was never measured on these. Here it is on every page of
the two packages, each round trip checked byte for byte:

| Pages stored as | Classic MiB | vs Zstd 9 | Decode p50 / p95 / p99 ms | 4 workers, pages/s | Enhanced MiB | vs Zstd 9 | Decode p50 / p95 / p99 ms | 4 workers, pages/s |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **Zstd 9 (as stored)** | 815.70 | | 1.85 / 3.65 / 3.89 | 1879–1960 | 1970.03 | | 2.25 / 4.06 / 4.42 | 1698–1722 |
| the 20 % rule | 892.03 | +9.36 % | 0.95 / 3.57 / 3.85 | 2702–3101 | 2166.88 | +9.99 % | 0.99 / 4.00 / 4.34 | 2763–2944 |
| LZ4 HC 12, every page | 970.28 | +18.95 % | 0.78 / 1.67 / 1.76 | 4447–4601 | 2301.55 | +16.83 % | 0.91 / 1.70 / 1.80 | 4243–4338 |

- **The rule flips most pages:** 833 of 1210 Classic and 1893 of 2473 Enhanced pages (69 and 77 %) come out at most
  20 % larger as LZ4 HC 12 and would be stored so, for +77 and +197 MiB. The median page then decodes as fast as LZ4,
  but the tail does not move: the slowest pages to decode are among those LZ4 would grow by more than 20 %, which stay
  Zstd, so p95 and p99 stay Zstd's.
- **LZ4 HC 12 on every page** decodes 2.4 times as fast at every percentile for +155 and +332 MiB.
- Decode is one thread decompressing a page from memory into a new 4 MiB buffer, the fastest of 5 runs per page; the
  pages/s run all pages 3 times over 4 threads (the range is the 3 passes). Two runs of Classic, and Enhanced in full
  and on every 4th page, agree within 3 % except where another job got in the way (one Classic Zstd 9 run
  had p50 2.32 ms); the full tables are under [Decode](#decode).

**In the game.** A clip decodes its 1.28 (Classic) or 1.02 (Enhanced) pages on one of the 4 background workers, and
the package is memory-mapped, so a page's first read faults it in from disk on that same worker before decompressing.
Measured here: a random direct read of a page-sized span takes 0.4 to 0.7 ms on this NVMe (about 0.8 ms per stored
MiB), and the drive streams 1.8 to 1.9 GB/s. The page's mean decode (from the single-thread MB/s) is 1.93 ms
(Classic) and 2.19 ms (Enhanced) at Zstd 9, 0.80 and 0.87 ms at LZ4 HC 12, 1.19 and 1.26 ms with the rule. So:

| 100 clips appearing at once (128 Classic pages, 102 Enhanced) | Zstd 9 | the rule | LZ4 HC 12 |
|---|---:|---:|---:|
| package already in the OS page cache, Classic / Enhanced | 67 / 60 ms | 44 / 36 ms | 28 / 24 ms |
| package cold on disk, Classic / Enhanced (estimate) | 79 / 72 ms | 58 / 52 ms | 60 / 56 ms |

- **Decode is the bottleneck at Zstd 9,** cold or warm: a cold read is 0.54 / 0.64 ms of a 2.5 / 2.8 ms page, and the 4
  workers then ask the disk for about 1.1 GiB/s, well within it.
- **With LZ4 the disk becomes the limit when cold:** 4 workers at LZ4's speed would read 2.2 to 2.3 GiB/s, more than
  the drive gives, so cold it is only about 1.3 times as fast as Zstd 9 (estimated: the read adds to the decode on
  the worker; the game's page faults read ahead 128 KiB at a time, which may be slower than the direct reads
  measured). Warm, after a body has been seen once, LZ4 is 2.4 times as fast. Cold, the rule does as well as LZ4 for
  half its size cost.
- **In frames:** at 60 fps 100 clips arrive over about 4 frames at Zstd 9 and 2 with LZ4, on otherwise idle workers;
  the workers are shared with the terrain and statics work of the same priority. 99 % of pages decode within 3.9 /
  4.4 ms (Zstd 9) on a worker, and none of it is on the main thread.

Whether 9 to 19 % more disk is worth halving the time a crowd takes to appear is the user's call; this note does not
change the packer (see the recommendation).

## Baseline

The two packages of `/home/claudio/test/uddp`, read only:

| Package | Bytes | SHA-256 |
|---|---:|---|
| `mobile_anim_cc.uddp` | 866 401 793 | `19f8856e6e653770db60b4b8afc7d2bb6d6e8745bb0cae78eb38b6f695ecf4ee` |
| `mobile_anim_ec.uddp` | 2 132 361 516 | `51dbf94049e7f99d16129d81f308c63dec973abe5b9b8a716f30a9ad851281fa` |

`udd-bc7-supercompress audit <package> cc|ec`:

| Package | Entries | Codec | Count | Raw MiB | Stored MiB | Share of the file |
|---|---|---|---:|---:|---:|---:|
| Classic | BC7 pages | Zstd 9 (codec 1) | 1210 | 3312.12 | 815.70 | 98.7 % |
| | metadata, 2x pages | Zstd / none | 6 | 26.10 | 10.53 | 1.3 % |
| Enhanced | BC7 pages | Zstd 9 (codec 1) | 2473 | 7110.82 | 1970.03 | 96.9 % |
| | hue masks (`masks/*.r8`) | Zstd | 306 | 996.55 | 36.97 | 1.8 % |
| | metadata, 2x pages | Zstd / none | 7 | 73.42 | 26.51 | 1.3 % |

- **Every page is one Zstd 9 frame**, no strips, no dictionary, no LZ4: compressing each page again with the harness's
  `none+zstd:9` gives the stored bytes exactly (815.70 and 1970.03 MiB), so the harness reproduces the packer.
- **A page is 2048 × 2048 texels** (the atlas extent, `DEFAULT_ATLAS_PAGE_WIDTH/HEIGHT`) of raw BC7 blocks, row
  major, of its used extent: 4 MiB raw for most pages (p50 3.9 and 4.0 MiB; the smallest 160 KiB), 644 KiB (Classic)
  and 822 KiB (Enhanced) stored at the median, 2.2 and 2.3 MiB at most.
- **The page is the unit the game reads.** `dynamapper/src/render/mobiles/clips.rs::decode_clip` runs on the
  background pool (`Priority::Visible`, at most 4 workers, `WORKERS_MAX` in `app/background.rs`) and decodes every page
  a clip (body, action, direction) touches, whole; the packages are opened with the page cache off
  (`AtlasCacheOptions::disabled()`), so a page is decompressed again for every clip that uses it. A clip touches 1.28
  pages on average in Classic (at most 5) and 1.02 in Enhanced (at most 4); a page serves 21 (Classic) and 54
  (Enhanced) clips at the median.
- **The blocks** (`blocks`, every 4th Classic and every 8th Enhanced page): 61 % (Classic) and 63 % (Enhanced) of
  the blocks are fully transparent, and every one of them is the same 16 bytes (a mode 5 block); the visible blocks are
  mode 6 (53 and 47 %), mode 4 (39 and 46 %) and mode 5 (8 and 7 %). No other mode occurs. 18 % (Classic) and 6 %
  (Enhanced) of the visible blocks repeat a visible block earlier in their page; 3.7 and 1.1 % repeat one of an
  earlier sampled page.

To reproduce (release build, then the commands of each table):

```sh
cargo build --release -p udd-devtools-cli --features dev-tool-tests,bc7-supercompress --bin udd-bc7-supercompress
B=/home/claudio/rust-target/uodynamapper/release/udd-bc7-supercompress   # the workspace target dir (.cargo/config.toml)
$B audit /home/claudio/test/uddp/mobile_anim_cc.uddp cc
$B blocks /home/claudio/test/uddp/mobile_anim_cc.uddp cc --every 4
$B sizes /home/claudio/test/uddp/mobile_anim_cc.uddp cc --methods stored,none+zstd:9,none+lz4hc:12,none+rule:9:12:20
$B timing /home/claudio/test/uddp/mobile_anim_cc.uddp cc --methods stored,none+lz4hc:12 --reps 5 --parallel-passes 3
```

A method is `stored` or `<transform>+<backend>`; `sizes` and `timing` take `--every N` (every N-th page), `--group N`
(N consecutive pages as one payload) and `--dict-kib` (dictionaries train on the even pages, the odd ones are
measured, the dictionary's bytes are added to the total). Transforms: `none`, `split<E>[t<N>]` (byte planes of
E-byte elements, over tiles of N blocks), `modesort`, `fields`, `fields-shuf`,
`pred-<none|prev|up|med>-<packed|bytes|planes>[-xi]` ([Prediction](#prediction-on-the-fields-layout-xubc7-style)).
Backends: `zstd:L[:long]`, `xz:P[e]` (one raw LZMA2 stream at xz preset P),
`zstd-chunks:L:KiB`, `zstd-dict:L`, `zstd-dict-chunks:L:KiB`, `lz4hc:L`, `lz4hc-chunks:L:KiB`, `lz4hc-dict:L`,
`lz4hc-dict-chunks:L:KiB`, `rule:Z:L:P` (the builder's rule: LZ4 HC at L when within P % of Zstd at Z).
The unit tests (`cargo test -p udd-devtools-cli --features dev-tool-tests,bc7-supercompress --bin
udd-bc7-supercompress`) round-trip every transform over every mode, the reserved encoding and tails of 0 to 15 bytes,
and reject truncated, extended and corrupt streams (a mode map value out of range, a map whose counts don't match,
set padding bits).

Machine: Ryzen AI 9 365 (10 cores, 20 threads), 27 GB, SK hynix NVMe; rustc 1.99.1, zstd 1.5.7 (zstd-sys and the
tool), lz4 1.10.0. Sizes were run while other jobs used the machine (encode thread-seconds are indicative only);
timings were run on a quiet machine (below).

## Experiments

Sizes against the pages as stored (Zstd 9). "Full" is every page of both packages; the sweeps ran on every 4th page
(303 Classic, 619 Enhanced; the dictionaries on every 2nd odd page, below). Every round trip was byte-exact. A
transform that changes the stream's length is charged 4 bytes a page for it. Encode is thread-seconds on a busy
machine, for scale only. Decode times are in the next section.

**Phase 1: level, window, chunks, grouping** (`none+<backend>`):

| Method | Classic | Enhanced | Pages | Encode thread-s (Classic) | Random access |
|---|---:|---:|---|---:|---|
| Zstd 3 | +6.57 % | +6.47 % | full | 23 | page |
| Zstd 6 | +1.73 % | +1.36 % | full | 61 | page |
| **Zstd 9 (stored)** | 815.70 MiB | 1970.03 MiB | full | 81 | page |
| Zstd 12 | −0.86 % | −0.30 % | full | 153 | page |
| Zstd 15 | −1.20 % | −0.37 % | full | 434 | page |
| Zstd 19 | −3.61 % | −2.82 % | full | 1488 | page |
| Zstd 22 | −4.02 % | −3.09 % | 1/4 | 1217 (1/4) | page |
| Zstd 9, long (LDM, 128 MiB window) | +0.78 % | +0.52 % | 1/4 | 47 (1/4) | page |
| Zstd 19, long | −3.78 % | −2.87 % | 1/4 | 946 (1/4) | page |
| Zstd 9, 2 pages a frame | −0.67 % | −0.57 % | 1/4 | 51 (1/4) | 2 pages |
| Zstd 19, 2 pages a frame | −4.59 % | −3.51 % | 1/4 | 952 (1/4) | 2 pages |
| Zstd 9, 4 pages a frame | −0.99 % | −0.95 % | 1/4 | 34 (1/4) | 4 pages |
| Zstd 19 long, 4 pages a frame | −5.18 % | −4.14 % | 1/4 | 773 (1/4) | 4 pages |
| Zstd 9, frames of 256 / 64 / 16 KiB | +10.3 / +16.0 / +22.9 % | +7.3 / +11.8 / +16.1 % | 1/4 | | chunk |
| LZ4 HC 9 | +19.32 % | +17.15 % | full | 140 | page |
| LZ4 HC 12 | +18.95 % | +16.83 % | full | 590 | page |
| LZ4 HC 12, blocks of 256 / 64 / 16 KiB | +20.9 / +26.2 / +35.9 % | +17.9 / +21.5 / +27.4 % | 1/4 | | chunk |
| The 20 % rule (Zstd 9 or LZ4 HC 12) | +9.36 % | +9.99 % | full | 787 | page |

**Phase 2: dictionaries.** Trained with `ZDICT` (zstd's `from_continuous`) on 64 KiB samples of every 4th even page,
measured on the odd pages. Zstd sizes its tables for the dictionary when one is loaded: with the level's own
parameters a whole page came out 3 to 18 % larger than without a dictionary (the zstd 1.5.7 tool with `-D` does the
same). Pinning the level-9 large-input parameters (`--zstd=wlog=22,clog=21,hlog=22,slog=6,mml=5,tlen=16,strat=5`,
single thread) fixes that in the tool. Through the `zstd` crate with the same parameters, a 112 KiB dictionary still
made the pages 1.5 to 2.1 % larger, a 1 MiB one 0.8 to 1.3 % smaller (unexplained; the crate's frames are the same
size as the tool's without a dictionary), so the first rows are the tool's: the harness dumps the dictionary it
trained and every 8th odd page, and the tool compresses each page with and without it.

```sh
$B sizes /home/claudio/test/uddp/mobile_anim_cc.uddp cc --every 8 --dict-kib 1024 --dump dump --dump-pages 1000 \
    --methods stored,none+zstd-dict:9
P9=--zstd=wlog=22,clog=21,hlog=22,slog=6,mml=5,tlen=16,strat=5
for p in dump/page_*.bc7; do
    zstd -q -9 --single-thread $P9 -c $p | wc -c                             # plain
    zstd -q -9 --single-thread $P9 -D dump/dictionary_none.bin -c $p | wc -c # with the dictionary
done
```


| Method | Classic | Enhanced | Pages |
|---|---:|---:|---|
| Zstd 9 + 112 KiB dictionary, whole page | −0.14 % (−0.13 % with the dictionary) | −0.05 % (−0.04 %) | 76 / 155 odd |
| Zstd 9 + 1 MiB dictionary, whole page | −0.72 % (−0.60 % on the full package) | −0.46 % (−0.41 %) | 76 / 155 odd |
| the same through the crate, dictionary included | −1.31 % | −0.81 % | 1/2 odd |
| Zstd 9, 64 KiB frames, + 1 MiB dictionary | +11.1 % (+15.7 % without) | +8.7 % (+11.6 % without) | 1/2 odd |
| LZ4 HC 12, whole page + 16 / 112 / 1024 KiB dictionary | +18.97 / +18.99 / +19.46 % (+19.04 % without) | +16.65 / +16.64 / +16.85 % (+16.69 %) | 1/2 odd |
| LZ4 HC 12, 64 KiB blocks + 112 KiB dictionary | +19.91 % (+25.96 % without) | +17.00 % (+21.27 %) | 1/2 odd |

**Phase 3: byte planes** (the brief's reversible shuffle; `split1` is the byte shuffle of the 16 bytes of each block,
`split<E>` takes E-byte elements, `t<N>` restarts every N blocks):

| Method | Classic | Enhanced |
|---|---:|---:|
| split1 + Zstd 9 | +34.52 % | +27.47 % |
| split2 + Zstd 9 | +21.73 % | +16.77 % |
| split4 + Zstd 9 | +11.70 % | +8.74 % |
| split8 + Zstd 9 | +1.23 % | +1.01 % |
| split1, tiles of 64 / 1024 blocks + Zstd 9 | +49.76 / +47.63 % | +39.53 / +38.10 % |
| split4, tiles of 64 blocks + Zstd 9 | +19.91 % | +15.82 % |
| split1 / split4 + Zstd 19 | +27.98 / +6.43 % | +21.30 / +4.63 % |
| split1 / split4 + LZ4 HC 12 | +49.81 / +25.85 % | +40.07 / +20.51 % |

**Phase 4: BC7 mode and field split.** `modesort` writes a mode map (a byte a block), then the blocks grouped by mode.
`fields` writes the map, then for each mode the bits between the mode bits and the indices (partition, rotation, index
selector, endpoints, p-bits), byte-padded, block after block, then each mode's index bits, byte-padded, then any block
of the reserved encoding whole. `fields-shuf` also byte-transposes each mode's records. Pages are 4 to 7 % longer after
`fields` (the map and the padding); the inverse checks the map, the lengths and the padding bits.

| Method | Classic | Enhanced | Encode thread-s (Classic, 1/4) |
|---|---:|---:|---:|
| modesort + Zstd 9 | −2.64 % | −2.48 % | 47 |
| **fields + Zstd 9** | **−4.20 %** | **−3.99 %** | 48 |
| fields-shuf + Zstd 9 | +11.19 % | +7.37 % | 54 |
| Zstd 19 (same pages) | −3.75 % | −2.84 % | 851 |
| modesort + Zstd 19 | −6.86 % | −6.12 % | 712 |
| **fields + Zstd 19** | **−8.81 %** | **−8.07 %** | 819 |
| fields-shuf + Zstd 19 | +6.31 % | +3.23 % | 1029 |
| fields + LZ4 HC 12 | +5.06 % (against +19.29 % for LZ4 HC 12 alone) | +4.79 % (+16.78 %) | 596 |
| modesort + LZ4 HC 12 | +10.27 % | +8.97 % | 619 |

Level 22 on the same sample: `fields` + Zstd 22 −9.03 % (Classic) and −8.33 % (Enhanced), against −4.02 and −3.09 %
for Zstd 22 alone: no better than level 19 for half again its encode.

On every page of both packages:

| Method | Classic MiB | vs stored | Enhanced MiB | vs stored | Encode thread-s (C / E) |
|---|---:|---:|---:|---:|---:|
| stored (Zstd 9) | 815.70 | | 1970.03 | | 81 / 167 |
| modesort + Zstd 9 | 794.07 | −2.65 % | 1921.62 | −2.46 % | 92 / 181 |
| **fields + Zstd 9** | **780.21** | **−4.35 %** | **1891.49** | **−3.99 %** | 94 / 191 |
| Zstd 19 | 786.23 | −3.61 % | 1914.48 | −2.82 % | 1488 / 2988 |
| **fields + Zstd 19** | **743.61** | **−8.84 %** | **1811.60** | **−8.04 %** | 1581 / 3092 |

The pages are 96.9 to 98.7 % of the files, so the packages shrink by almost as much: about 35 and 78 MiB at level 9,
72 and 158 MiB at level 19.

**Phase 5** was not built: every fully transparent block of both packages is already the same 16 bytes, so there is
nothing to canonicalise, and exact repeats are what Zstd's matches already code (see below).

## Decode

`timing` on every page of both packages, the machine otherwise running another agent's GPU work (load 15 to 20 on 20
threads; the fastest of 5 runs per page filters most of it). Classic was run twice, Enhanced once in full and once on
every 4th page; the second figure, in brackets, is the other run, and the pages/s ranges span both runs' passes.
"Inverse" is the share of the decode spent undoing the transform; "Mem" what a worker holds beyond the 4 MiB page it
hands on.

| Method | Classic p50 / p95 / p99 ms | Inverse p50 | 4 workers pages/s | Enhanced p50 / p95 / p99 ms | Inverse p50 | 4 workers pages/s | Mem |
|---|---:|---:|---:|---:|---:|---:|---:|
| Zstd 9 (stored) | 1.85 / 3.65 / 3.89 (1.86 / 3.71 / 3.94) | | 1879–1960 | 2.25 / 4.06 / 4.42 (2.24 / 4.11 / 4.46) | | 1582–1722 | Zstd context |
| LZ4 HC 12 | 0.78 / 1.67 / 1.76 (0.79 / 1.67 / 1.76) | | 4447–4601 | 0.91 / 1.70 / 1.80 (0.92 / 1.72 / 1.81) | | 4087–4338 | none |
| the 20 % rule | 0.95 / 3.57 / 3.85 (0.96 / 3.58 / 3.86) | | 2702–3101 | 0.99 / 4.00 / 4.34 (0.98 / 4.02 / 4.38) | | 2763–2944 | Zstd context |
| Zstd 19 | 2.00 / 3.91 / 4.22 (1.99 / 3.90 / 4.21) | | 1706–1842 | 2.48 / 4.63 / 5.08 (2.44 / 4.65 / 5.03) | | 1462–1543 | Zstd context |
| fields + Zstd 9 | 4.24 / 7.67 / 8.29 (4.22 / 7.63 / 8.30) | 2.29 | 903–946 | 4.86 / 7.86 / 8.61 (4.86 / 8.01 / 8.51) | 2.51 | 850–880 | + 4.2 MiB |
| fields + Zstd 19 | 4.59 / 7.60 / 8.07 (4.56 / 7.54 / 8.02) | 2.27 | 885–902 | 5.35 / 8.23 / 8.73 (5.38 / 8.32 / 8.87) | 2.52 | 783–817 | + 4.2 MiB |
| fields + LZ4 HC 12 | 2.98 / 4.99 / 5.32 (3.46 / 6.66 / 7.03) | 2.27 | 1332–1394 (869–949) | 3.34 / 5.04 / 5.35 (3.42 / 5.20 / 5.69) | 2.51 | 1269–1297 | + 4.2 MiB |

- **Zstd 19 costs 8 to 10 % at p50** and 7 to 14 % at p95 against level 9: smaller pages, but its longer matches and
  larger tables decode slower.
- **The field split's inverse costs 2.3 to 2.5 ms a page,** as much as the Zstd decode itself: it walks 262 144 blocks,
  joining each from two records with a branch on its mode (about 9 ns a block). It was made 2.4 times faster than the
  first version (one instance per mode, masked 16-byte loads, the transformed stream decoded into a buffer kept per
  worker); a table-driven version without the branch was 50 % slower, as a page's modes come in runs the branch
  predicts. A SIMD join might halve it; it cannot approach the 0.1 ms the brief's 5 % gate would allow.
- **Zstd itself decodes the split stream about as fast:** about 1.95 against 1.85 ms (Classic, level 9, the p50 less
  the inverse's), the stream being 4 to 7 % longer.
- Encode, for the full packages (thread-seconds, busy machine): Zstd 9 81 / 167, Zstd 19 1488 / 2988, fields + Zstd 9
  94 / 191, fields + Zstd 19 1581 / 3092, LZ4 HC 12 590 (Classic).
- Every method reads the page it needs and nothing else: random access stays the page.

## Negative results

- **Higher Zstd levels buy 3 to 4 % for 15 to 25 times the encode** (19: −3.6 / −2.8 %; 22: −4.0 / −3.1 % on the
  sample), which `MOBILE_ANIM_PAGE_ZSTD_LEVEL`'s note already recorded for 19.
- **Long-distance matching does nothing:** at level 9 the window (4 MiB) already covers a whole page, and level 19's
  is 8 MiB; LDM's coarser matches make level 9 0.5 to 0.8 % larger.
- **Pages share little:** two pages in one frame save 0.6 to 0.7 % at level 9 (four pages 1 %), and a clip would then
  decode two to four pages to read one. Only 1 to 4 % of a page's visible blocks occur in an earlier page.
- **Chunking a page costs 7 to 36 %**, and the page is already the unit the game reads: nothing reads less than a page.
- **Dictionaries give at most 1.3 %** on whole pages (1 MiB of dictionary for a 2 GB package), nothing for LZ4 HC (its
  16-bit offsets reach only the first 64 KiB of a 4 MiB page), and help only the chunked layouts, which stay 9 to 20 %
  larger than one frame a page. A dictionary needs the same transform as the pages it serves, which made it a poor
  partner for the field split too: on top of `fields` + Zstd 9 it adds 0.24 points (−4.62 against −4.38 %, dictionary
  included; [Crowds](#crowds-under-the-loader-order)).
- **Byte shuffles make every page larger** (+1 to +50 %), with Zstd and with LZ4 HC, whole or tiled: BC7 fields cross
  byte boundaries, so a byte plane mixes the bits of several fields, and the shuffle breaks the 16-byte repeats Zstd
  codes cheaply (61 % of the blocks are the same transparent block). Transposing the field records (`fields-shuf`)
  is worse for the same reason.
- **Exact block reuse** would replace repeats Zstd already codes as matches: 6 to 18 % of the visible blocks repeat in
  their page, about 2 to 7 % of all blocks, each costing Zstd a few bytes as a match; a reference would cost about as
  much. Not built.

## BC7 RDO

RDO is lossy, so it is not one of the transforms above; it is measured here as the brief asked, and to see whether
the field split survives it. Four packages packed from this commit's `udd-cli` into scratch, from the clients under
`/mnt/dati/_proj_local/_uo_clients/`, on the CPU (`--bc-encode-device cpu --bc7-rdo-device cpu`), while other jobs
ran:

```sh
udd-cli pack pack-mobile-anims --ccdir "<Classic>" --output cc_off.uddp --bc-encode-device cpu
udd-cli pack pack-mobile-anims --ccdir "<Classic>" --output cc_rdo1.uddp --bc-encode-device cpu \
    --bc7-rdo --bc7-rdo-lambda 1 --bc7-rdo-device cpu --zstd=9
# and pack-ec-mobile-anims --ecdir "<Enhanced>" for ec_off and ec_rdo1
```

| Package | RDO off | λ 1, look-back 64 | Saving | Pack time off / on |
|---|---:|---:|---:|---:|
| Classic | 867 478 192 | 823 407 732 | −5.08 % | 82 / 1493 s |
| Enhanced | 1 939 014 497 | 1 839 384 403 | −5.14 % | 553 / 3076 s |

(These Enhanced packages have 2291 pages against the 2473 of the test package: the packer changed since; each pair is
compared with itself.) The Classic saving matches the earlier full repack (823 320 535 bytes,
`atlas_page_compression.md`), whose GPU RDO saved 6.6 % in 164 s. On every 4th page of each package, against its own
stored pages:

| Pages | fields + Zstd 9 | Zstd 19 | fields + Zstd 19 | LZ4 HC 12 |
|---|---:|---:|---:|---:|
| Classic, RDO off | −3.14 % | −3.74 % | −7.87 % | +19.05 % |
| Classic, RDO λ 1 | −2.33 % | −4.50 % | −7.72 % | +16.39 % |
| Enhanced, RDO off | −3.65 % | −2.81 % | −7.77 % | +16.96 % |
| Enhanced, RDO λ 1 | −2.59 % | −3.58 % | −7.36 % | +15.30 % |

- **The field split keeps most of its gain on RDO pages:** RDO copies whole blocks and runs of bytes from the
  previous 64 blocks, which the split partly breaks apart, so at level 9 it loses about a quarter of its gain; at level
  19 almost none.
- **They add:** RDO λ 1 and `fields` + Zstd 19 together are 12.4 % (Classic) and 12.1 % (Enhanced) below the plain
  pages at Zstd 9 on these samples, the lossless part being 7.7 and 7.4 % of it.

## Not built (phases 6 and 7)

Both were measured later as lossy steps: see [Lossy conditioning](#lossy-conditioning).

- **6, temporal XOR or prediction against a reference frame.** The packer trims each frame to its silhouette and
  places it at a 4-pixel-aligned spot of the page; the block grid follows the trimmed box, which moves with the
  silhouette from frame to frame, so the same body part of two frames rarely falls on the same grid, and a page holds
  frames of many clips. Only 1 to 4 % of a page's visible blocks occur in an earlier page and 6 to
  18 % earlier in the same page, which Zstd already codes. Aligning frames to blocks would change the packer's layout
  and the page count, and a reference chain would make a clip read more than its own pages. Not worth building
  before the field split (phase 4) is in.
- **7, an RDO that models the coder.** The RDO (`uot_texture_codecs::bc7::rdo`, a port of bc7enc_rdo's `ert.cpp`) prices
  a block by the matches it makes in the previous 64 blocks, an LZ-agnostic model; the pages are Zstd, not LZ4. λ 1
  saves 5.1 % on both packages (above) for 1 to 2 dB (`atlas_page_compression.md`), and stays off by default for that
  loss. A model of Zstd's sequence costs could get more at the same distortion, but any RDO is lossy, needs the visual
  check `UDDP_FORMATS.md` asks for, and interacts with the field split, which still saves 2.3 to 7.7 % on RDO pages
  (above).
- **8, endpoint and weight prediction (XUBC7-style)** was built and measured in the second study
  ([Prediction](#prediction-on-the-fields-layout-xubc7-style)): every predictor loses 6.5 to 10 points against
  `fields`. DCT residuals are lossy and out of scope.

## Size classes, per-clip joins and crowds

A second study (2026-11-26) of the same packages:
- Do small and large bodies compress and decode differently?
- What would a codec per size class cost?
- What does joining only a clip's blocks cost, against inverting the whole page?
- What does a crowd wait for under the game's loader order, and would a cache of decoded pages help?
- Does XUBC7-style prediction on the `fields` layout pay?

The new subcommands are `classes`, `bodies` and `crowd`, and `sizes` / `timing` gained the `pred-*` transforms
(below). Timings ran on a quiet machine, sizes while other jobs ran.

**Already done before this study:**
- BC7 RDO λ 1 (lossy): −5.1 % on both packages ([BC7 RDO](#bc7-rdo)).
- Conditioning ahead of the coder: `modesort` −2.6 / −2.5 % and `fields` −4.4 / −4.0 % at Zstd 9, `fields` −8.8 /
  −8.0 % at Zstd 19.
- Losing transforms: byte planes and `fields-shuf` ([Negative results](#negative-results)).

### Size classes

A body's size is its largest stored frame side: the larger of width and height, over every frame of the body. The
bounds are 96, 192 and 384 px. Most bodies are small, but most of the bytes are in large ones:
- Bodies by size, deciles: Classic 16 24 37 46 57 72 97 140 214 640 px; Enhanced 18 28 40 58 71 92 120 160 239 640 px.
- By frame blocks, the median block belongs to a body of 219 (Classic) or 213 px (Enhanced).

Each page's stored bytes are shared among its frames by their slot blocks (content and gutter):

| Class | Classic bodies | Frames | Clips | Stored MiB | Share | Enhanced bodies | Frames | Clips | Stored MiB | Share |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ≤ 96 px | 517 | 821 440 | 113 874 | 182.5 | 22.4 % | 632 | 1 374 872 | 231 000 | 490.0 | 24.9 % |
| 97–192 px | 138 | 106 250 | 15 828 | 161.0 | 19.7 % | 212 | 197 141 | 53 344 | 392.9 | 19.9 % |
| 193–384 px | 64 | 43 276 | 6 530 | 240.8 | 29.5 % | 128 | 106 897 | 31 192 | 677.7 | 34.4 % |
| > 384 px | 20 | 25 530 | 3 851 | 231.3 | 28.4 % | 27 | 25 185 | 6 568 | 409.5 | 20.8 % |

**The named bodies:**

| Body | Classic side | Enhanced side |
|---|---|---|
| 217 dog | 48 px (≤ 96) | 70 px (≤ 96) |
| 400 human | 77 px (≤ 96) | not stored (Enhanced humans are paperdoll) |
| 28 giant spider | 105 px (97–192) | not stored |
| 24 lich | 247 px (193–384) | 110 px (97–192) |
| 86 ophidian | 195 px (193–384) | 207 px (193–384) |
| 12 dragon | 508 px (> 384) | 528 px (> 384) |
| 9 daemon | 438 px (> 384) | 388 px (> 384) |
| 826 stygian | 640 px (> 384) | 640 px (> 384) |
| 1248 Exodus | 407 px (> 384) | 412 px (> 384) |

**The packer, replayed.** A body's or a class's own pages need a layout from the packer, so `repack.rs` replays it:
- It uses the packer's guillotiere allocator, its buckets in area order and its prefix search (doubling, then
  bisection).
- It chooses the bucket with the best fill, and places frames largest first.
- Each allocation is the content and gutter, padded to 4 × 4 blocks.

The packer encodes every block on its own, so a replayed page is built by copying each frame's slot (content and
gutter) from its stored page, with the transparent block as background.

How close the replay is:
- **Classic:** it is exact. Replaying the whole package gives its 1210 pages; 0 of 996 496 frames are misplaced and
  173 of 173 sampled pages are identical.
- **Enhanced:** it does not match the stored pages, because the test package was written by an older packer. The
  replay gives 2486 pages against 2473, and 2423 of 2472 page boundaries fall in the same place, but no page is
  identical. Enhanced replayed layouts are what the current packer would do as far as Classic shows the replay to be
  right. Their baseline is the replay of all frames as one group: 1969.3 MiB at Zstd 9, against 1970.0 stored.
  Enhanced "stored layout" figures use the stored pages themselves.

### Per body

Each named body's frames are laid out alone by the replay; the Classic human takes 4 pages, the stygian 79. A clip is
one (action, direction) of the body.

"Clip ms" covers decoding every page the clip touches, then either cutting its frames' blocks from the whole page or,
"per clip", joining only those blocks (next section). It is single thread, the fastest of 5 runs. The percentages
compare each method's size with Zstd 9.

Classic:

| Body (side) | Pages / clips | Zstd 9 ratio | Zstd 19 | LZ4 HC 12 | xz 6 | fields + Zstd 9 | fields + Zstd 19 | fields + LZ4 HC 12 | fields + xz 6 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 217 (48) | 2 / 56 | 5.75 | −3.7 % | +11.0 % | −12.0 % | +2.0 % | −6.0 % | +11.8 % | −11.1 % |
| 400 (77) | 4 / 175 | 4.08 | −6.2 % | +19.4 % | −14.3 % | −3.8 % | −10.4 % | +4.1 % | −15.8 % |
| 28 (105) | 3 / 130 | 3.95 | −4.0 % | +17.7 % | −11.9 % | −2.6 % | −8.3 % | +4.9 % | −13.6 % |
| 24 (247) | 4 / 205 | 4.43 | −1.9 % | +18.8 % | −8.8 % | −3.8 % | −8.2 % | +4.4 % | −13.3 % |
| 86 (195) | 2 / 65 | 5.40 | −2.0 % | +18.0 % | −9.4 % | −5.5 % | −9.4 % | +3.6 % | −14.8 % |
| 12 (508) | 14 / 215 | 4.82 | −2.5 % | +18.0 % | −9.6 % | −6.1 % | −9.7 % | +1.6 % | −15.1 % |
| 9 (438) | 17 / 245 | 3.99 | −2.1 % | +20.3 % | −8.6 % | −6.6 % | −9.4 % | +3.3 % | −14.6 % |
| 826 (640) | 79 / 136 | 3.88 | −1.9 % | +12.3 % | −8.3 % | −6.2 % | −8.9 % | +1.7 % | −14.2 % |
| 1248 (407) | 41 / 50 | 3.94 | −2.0 % | +14.2 % | −9.1 % | −5.1 % | −8.1 % | +3.7 % | −13.4 % |

| Body | Zstd 9 MB/s, clip ms p50 | Zstd 19 | LZ4 HC 12 MB/s, ms | xz 6 MB/s, ms | fields + Zstd 9, whole / per clip | fields + Zstd 19, whole / per clip | fields + LZ4 HC 12, whole / per clip | fields + xz 6, per clip |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 217 | 2247, 0.16 | 0.19 | 8211, 0.04 | 117, 3.27 | 0.41 / 0.21 | 0.45 / 0.25 | 0.28 / 0.07 | 3.44 |
| 400 | 1381, 1.58 | 1.64 | 3228, 0.77 | 90, 22.6 | 3.42 / 1.76 | 3.46 / 1.81 | 2.26 / 0.59 | 22.9 |
| 28 | 1440, 1.58 | 1.72 | 3493, 0.71 | 84, 25.8 | 3.40 / 1.80 | 3.49 / 1.87 | 2.21 / 0.61 | 25.6 |
| 24 | 1702, 2.94 | 3.23 | 3781, 1.38 | 93, 51.7 | 6.07 / 3.01 | 6.76 / 3.68 | 4.35 / 1.28 | 51.4 |
| 86 | 1963, 2.23 | 2.44 | 4722, 0.99 | 113, 38.2 | 5.25 / 2.51 | 5.52 / 2.76 | 3.83 / 1.09 | 37.3 |
| 12 | 1936, 2.17 | 2.34 | 4660, 0.93 | 102, 38.1 | 4.65 / 2.55 | 5.06 / 2.96 | 3.40 / 1.05 | 37.7 |
| 9 | 1693, 1.53 | 1.59 | 4181, 0.63 | 84, 29.6 | 3.02 / 1.65 | 3.61 / 2.24 | 1.91 / 0.56 | 27.8 |
| 826 | 1692, 2.82 | 2.95 | 4760, 1.08 | 81, 56.4 | 5.17 / 3.27 | 5.88 / 3.73 | 3.55 / 1.46 | 54.5 |
| 1248 | 1615, 2.94 | 3.23 | 4372, 1.12 | 81, 55.8 | 5.63 / 4.08 | 6.17 / 4.59 | 3.83 / 2.34 | 55.0 |

Enhanced (no 400, 401 or 28):

| Body (side) | Pages / clips | Zstd 9 ratio | Zstd 19 | LZ4 HC 12 | xz 6 | fields + Zstd 9 | fields + Zstd 19 | fields + LZ4 HC 12 | fields + xz 6 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 217 (70) | 2 / 232 | 3.19 | −3.3 % | +12.2 % | −10.7 % | −1.3 % | −7.7 % | +4.8 % | −12.2 % |
| 24 (110) | 3 / 464 | 2.62 | −2.7 % | +12.3 % | −8.9 % | −4.2 % | −8.1 % | +2.3 % | −12.5 % |
| 86 (207) | 8 / 232 | 5.77 | −1.8 % | +14.9 % | −8.4 % | −3.3 % | −8.2 % | +5.2 % | −12.2 % |
| 12 (528) | 30 / 464 | 4.80 | −1.3 % | +12.4 % | −7.3 % | −7.5 % | −9.8 % | +0.0 % | −13.9 % |
| 9 (388) | 15 / 464 | 4.14 | −1.8 % | +12.6 % | −7.7 % | −6.1 % | −9.1 % | +1.2 % | −13.6 % |
| 826 (640) | 86 / 248 | 3.55 | −1.6 % | +10.3 % | −6.7 % | −6.3 % | −8.6 % | +0.6 % | −12.9 % |
| 1248 (412) | 42 / 232 | 3.58 | −1.6 % | +12.2 % | −7.8 % | −5.3 % | −8.0 % | +2.3 % | −12.7 % |

| Body | Zstd 9 MB/s, clip ms p50 | Zstd 19 | LZ4 HC 12 MB/s, ms | xz 6 MB/s, ms | fields + Zstd 9, whole / per clip | fields + Zstd 19, whole / per clip | fields + LZ4 HC 12, whole / per clip | fields + xz 6, per clip |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 217 | 1218, 0.91 | 1.00 | 3526, 0.34 | 68, 15.5 | 1.75 / 0.97 | 1.91 / 1.13 | 1.07 / 0.29 | 16.0 |
| 24 | 1103, 1.87 | 2.08 | 2849, 0.72 | 57, 36.6 | 3.50 / 2.03 | 4.06 / 2.59 | 2.19 / 0.73 | 36.9 |
| 86 | 2210, 0.96 | 1.12 | 5111, 0.45 | 120, 16.8 | 2.53 / 1.23 | 2.69 / 1.41 | 1.81 / 0.55 | 17.3 |
| 12 | 2087, 1.92 | 2.11 | 5321, 0.79 | 100, 37.6 | 4.33 / 2.33 | 4.75 / 2.72 | 3.13 / 1.13 | 36.5 |
| 9 | 1729, 2.50 | 2.70 | 4427, 1.02 | 88, 48.4 | 5.22 / 2.78 | 5.78 / 3.33 | 3.62 / 1.19 | 46.9 |
| 826 | 1694, 2.69 | 2.89 | 4914, 0.97 | 75, 57.7 | 5.07 / 3.41 | 5.91 / 4.39 | 3.29 / 1.72 | 55.1 |
| 1248 | 1530, 2.91 | 3.16 | 4367, 1.04 | 74, 55.8 | 5.27 / 3.87 | 6.06 / 4.62 | 3.52 / 1.97 | 54.9 |

- **No class is special.** The plain coders' size gains are larger on small bodies: Zstd 19 saves 3 to 6 % on the
  Classic dog, human and spider but 2 % on the large bodies. The field split goes the other way: at Zstd 9 it saves 5
  to 7.5 % on bodies over 384 px, against −2 to +2 % on the dog. Decode speed per byte is within 1.5 times across
  classes, the same way for every method.
- **xz 6 and 9 give the same bytes here,** because the dictionary is clamped to the 4 MiB page. With the field split
  xz saves 11 to 16 %, but it decodes at 55 to 120 MB/s, 15 to 21 times slower than Zstd 9: 23 ms for a human's clip
  and 55 ms for a large body's. It is not a candidate for anything the game reads at run time.
- **Joining per clip removes most of the field split's cost on small and medium bodies:** a human clip takes 1.76 ms
  with `fields` + Zstd 9, against 3.42 with the whole-page inverse and 1.58 at Zstd 9 alone. On the largest bodies, whose clips cover much of their pages, it
  gains less: Exodus takes 4.08 ms against 2.94.
- **`fields` + LZ4 HC 12, per clip, is the fastest:** 0.3 to 0.5 times Zstd 9's clip time (0.5 to 0.8 on the
  largest bodies), for +0 to +5 % in size (+12 % only for the Classic dog).

### Joining a clip's blocks only

`transforms::fields_extract` joins, from a `fields` stream, only the blocks of a list of rectangles: a clip's frames,
each its content padded to blocks. A block's records are found from its mode's running count:
- The count at each row's start comes from counting the mode map's rows before it.
- The count at the block comes from that row up to the block.

The checks:
- The whole map is checked: its values, and the stream length its counts give.
- Padding bits are checked only in the blocks joined.
- Unit tests check that the join equals cutting the rectangles from the whole inverse, and that malformed streams are
  rejected as the inverse rejects them.

**Counting the map was the cost.** One `filter().count()` pass per mode took 1.17 ms of a 1.28 ms join on a
512 × 512-block page, about 4.5 ns an entry. Counting in byte counters 32 lanes wide (`COUNT_LANES`) takes 0.12 ms,
and the join 0.17 ms, against 2.1 ms for the whole inverse.

So a clip now costs the split stream's decode (about 5 % slower than the plain page at Zstd 9) plus 0.1 to 0.3 ms. A
clip covering most of a page costs up to the whole inverse.

### Crowds under the loader order

`crowd` models what a crowd waits for. The model:
- **Mobiles.** Each crowd is 34 mobiles: random bodies (Classic 673, Enhanced 993), or "people" (Classic humans 400
  and 401 only). Each mobile faces a random direction.
- **Clips.** Each mobile asks for its movement set (stand, walk, run, as `movement_action` / `move_action` resolve
  them), facing direction first, then the next direction for every mobile, and so on through all 8.
- **Deduplication.** Repeated clips are merged.
- **The resulting crowd** is 96 to 101 faced clips and 760 to 784 clips in all at the median.
- **Workers.** Requests run first come, first served on 4 workers (`WORKERS_MAX`).
- **Per clip.** A clip decodes every page it touches, then cuts or joins its blocks.
- **Cache.** With an LRU of N decoded pages, a cached page is not decoded again. A page another worker is still
  decoding is waited for.
- **Page costs** are measured on every page with 4 threads busy, the fastest of 3.
- **"Cold"** adds 0.8 ms per stored MiB on a page's first read in the crowd (above).

The columns:
- **First drawable:** when the last of the 34 mobiles has its faced movement set.
- **All preloaded:** when every direction is decoded.
- **Decode ms:** the sum over workers.

Each figure is the median of 20 crowds. Random crowds, Classic (stored layout):

| Config | MiB | vs today | Cache | Page decodes | Decode ms | First drawable, warm / cold | Mobile p50, warm | All preloaded, warm / cold |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **today: Zstd 9** | 815.7 | | 0 | 766 | 2358 | 76.6 / 83.2 | 37.6 | 591 / 598 |
| | | | 8 | 332 | 1006 | 58.4 / 70.2 | 29.0 | 457 / 469 |
| | | | 32 | 324 | 924 | 55.9 / 67.3 | 29.0 | 420 / 433 |
| Zstd 19 | 786.2 | −3.6 % | 0 | 766 | 2414 | 76.9 / 83.6 | 38.1 | 605 / 612 |
| | | | 8 | 332 | 994 | 57.4 / 68.3 | 29.1 | 449 / 462 |
| LZ4 HC 12 | 970.3 | +19.0 % | 0 | 766 | 1018 | 33.0 / 41.7 | 15.9 | 255 / 264 |
| | | | 8 | 332 | 428 | 24.9 / 39.4 | 12.4 | 194 / 209 |
| fields + Zstd 9, whole page | 780.2 | −4.4 % | 0 | 766 | 4725 | 153.2 / 158.8 | 75.0 | 1183 / 1191 |
| fields + Zstd 9, per clip | | | 0 | 766 | 2575 | 83.0 / 89.2 | 41.1 | 645 / 654 |
| | | | 8 | 332 | 1131 | 63.1 / 74.9 | 31.9 | 492 / 504 |
| fields + Zstd 19, whole page | 743.6 | −8.8 % | 0 | 766 | 4718 | 152.2 / 157.7 | 75.8 | 1182 / 1189 |
| | | | 8 | 332 | 1973 | 114.6 / 125.8 | 57.4 | 890 / 902 |
| **fields + Zstd 19, per clip** | | | 0 | 766 | 2539 | 81.7 / 88.7 | 41.3 | 637 / 643 |
| | | | 8 | 332 | 1124 | 61.4 / 72.7 | 31.5 | 487 / 499 |
| fields + LZ4 HC 12, whole page | 855.1 | +4.8 % | 0 | 766 | 3140 | 100.8 / 107.9 | 49.9 | 787 / 794 |
| **fields + LZ4 HC 12, per clip** | | | 0 | 766 | 991 | 31.8 / 39.3 | 15.7 | 248 / 256 |
| | | | 8 | 332 | 474 | 24.6 / 37.5 | 12.4 | 191 / 204 |

Random crowds, Enhanced (stored layout):

| Config | MiB | vs today | Cache | Page decodes | Decode ms | First drawable, warm / cold | Mobile p50, warm | All preloaded, warm / cold |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **today: Zstd 9** | 1970.0 | | 0 | 804 | 2540 | 83.1 / 94.0 | 43.7 | 637 / 648 |
| | | | 8 | 455 | 1366 | 66.7 / 82.7 | 35.4 | 521 / 538 |
| | | | 32 | 447 | 1361 | 65.4 / 81.1 | 34.6 | 509 / 526 |
| Zstd 19 | 1914.5 | −2.8 % | 0 | 804 | 2809 | 92.7 / 101.9 | 48.0 | 703 / 714 |
| LZ4 HC 12 | 2301.6 | +16.8 % | 0 | 804 | 1059 | 34.8 / 47.3 | 17.9 | 265 / 279 |
| | | | 8 | 455 | 567 | 28.1 / 47.1 | 14.7 | 217 / 237 |
| fields + Zstd 9, whole page | 1891.5 | −4.0 % | 0 | 804 | 5004 | 163.2 / 174.2 | 85.0 | 1253 / 1264 |
| fields + Zstd 9, per clip | | | 0 | 804 | 2807 | 92.0 / 101.9 | 48.7 | 703 / 714 |
| fields + Zstd 19, whole page | 1811.6 | −8.0 % | 0 | 804 | 5165 | 168.7 / 179.3 | 88.5 | 1294 / 1303 |
| | | | 8 | 455 | 2855 | 137.0 / 151.3 | 72.7 | 1064 / 1080 |
| **fields + Zstd 19, per clip** | | | 0 | 804 | 2987 | 97.7 / 107.2 | 51.2 | 749 / 759 |
| | | | 8 | 455 | 1709 | 79.4 / 93.7 | 42.3 | 616 / 632 |
| fields + LZ4 HC 12, whole page | 2064.9 | +4.8 % | 0 | 804 | 3278 | 107.1 / 118.3 | 55.2 | 821 / 832 |
| **fields + LZ4 HC 12, per clip** | | | 0 | 804 | 1100 | 36.0 / 47.3 | 18.8 | 275 / 287 |
| | | | 8 | 455 | 658 | 29.8 / 46.9 | 15.6 | 231 / 249 |

People crowds (Classic: 34 humans, 45 faced clips and 48 in all at the median):

| Config | Cache 0: page decodes, first drawable warm / cold | Cache 8 |
|---|---:|---:|
| Zstd 9 | 48, 42.0 / 42.0 ms | 2, 3.5 / 4.4 ms |
| fields + Zstd 19, whole page | 48, 86.3 / 86.6 ms | 2, 7.4 / 8.2 ms |
| fields + Zstd 19, per clip | 48, 44.5 / 45.1 ms | 2, 5.6 / 6.4 ms |
| fields + LZ4 HC 12, per clip | 48, 16.3 / 16.3 ms | 2, 3.0 / 3.9 ms |

**Is fields + Zstd 19 feasible under the loader order?**
- **With the whole-page inverse, no:** it doubles every figure. The first drawable goes from 77 to 152 ms (Classic)
  and from 83 to 169 ms (Enhanced), and a full preload from 0.6 to 1.2 s.
- **Joined per clip, yes:** it costs 7 % on the first drawable and 8 % on the preload in Classic, and 18 % on both in
  Enhanced. That is 5 and 15 ms more before the last of 34 mobiles is drawable, for −8.8 and −8.0 % of disk.
- **With an 8-page cache,** per clip it is faster than today's Zstd 9 without one: 61 against 77 ms (Classic), 79
  against 83 ms (Enhanced).

**A dictionary is a poor partner for it:**
- With the dictionary included, `fields` + Zstd 9 with a dictionary is −4.62 % on 76 odd Classic pages, against
  −4.38 % without. ZDICT kept 150 KiB of the 1 MiB asked for.
- So the dictionary adds 0.24 points on top of the split, and costs a dictionary entry and a format dependency.

**fields + LZ4 HC 12, per clip, decodes as fast as LZ4 HC alone** (991 against 1018 ms, Classic). It is only +4.8 %
on disk in both packages, a quarter of LZ4 HC's +19.0 / +16.8 %, and 2.4 times as fast as today, warm. Cold, the
cached figures show the disk at work, since every page's first read is still paid: 37.5 ms (Classic) and 46.9 ms
(Enhanced) to the first drawable, against 70.2 and 82.7 ms for Zstd 9 with the same cache.

### A decoded-page cache

The reader opens the packages with the page cache off (`AtlasCacheOptions::disabled()`). A page serves 21 (Classic)
and 54 (Enhanced) clips at the median, and a crowd's clips of one body share pages across its 8 directions:
- **Page decodes:** an 8-page LRU of decoded pages cuts a random crowd's page decodes from 766 to 332 (Classic, −57 %)
  and from 804 to 455 (Enhanced, −43 %). Decode time falls by as much at every codec.
- **First drawable:** it falls by 23 to 25 % (Classic) and 17 to 20 % (Enhanced). The first drawable needs only the
  faced clips, which share fewer pages.
- **Larger caches:** 16 and 32 pages save at most 3 % more decodes and 8 % more decode time than 8.
- **People crowds:** 48 decodes become 2, and the first drawable falls from 42 to 3.5 ms.

The memory is 8 decoded pages: 32 MiB (4 MiB each), or about 34 MiB with the split stream held for per-clip joins.

This argues for a reader change, independent of the codec: a bounded LRU of about 8 decoded pages, shared by the
workers, where a page being decoded is waited for rather than decoded twice. The existing cache (`AtlasPageCache`,
`lib/udd-assets/src/common.rs`) is an unbounded map with neither eviction nor an in-flight wait, so turning it on is
not that change.

### Codec per size class (tiering)

The classes layout replays the packer on each class's frames separately, which gives each class pages of its own. It
costs nothing:
- **Classic:** 1264 pages against 1210, 816.2 MiB at Zstd 9 against 815.7.
- **Enhanced:** 2489 pages against the one-group replay's 2486, 1969.8 against 1969.3 MiB.
- **Decode:** a crowd's decode is within 4 %: 2266 against 2358 ms (Classic), 2579 against 2560 (Enhanced).

The tiers below list the method for each class, ≤ 96 / 97–192 / 193–384 / > 384 px; "per clip" applies to each
`fields` method. Random crowds, no cache:

| Tiering | Classic MiB | vs today | Decode ms | First drawable warm / cold | Enhanced MiB | vs today | Decode ms | First drawable warm / cold |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| today (stored layout, Zstd 9) | 815.7 | | 2358 | 76.6 / 83.2 | 1970.0 | | 2540 | 83.1 / 94.0 |
| classes layout, Zstd 9 everywhere | 816.2 | +0.1 % | 2266 | 73.0 / 79.9 | 1969.8 | +0.0 % | 2579 | 84.4 / 94.5 |
| A: f+Z19 / f+Z19 / f+LZ4 / f+LZ4 | 800.8 | −1.8 % | 2365 | 76.8 / 83.1 | 1930.4 | −2.0 % | 2728 | 89.0 / 99.2 |
| B: f+Z19 / f+Z19 / f+Z19 / f+LZ4 | 771.8 | −5.4 % | 2502 | 81.2 / 87.3 | 1853.0 | −5.9 % | 2973 | 97.1 / 107.1 |
| C: f+Z19 / Z9 / Z9 / LZ4 (plain) | 836.7 | +2.6 % | 2415 | 78.4 / 84.8 | 1983.4 | +0.7 % | 2790 | 90.1 / 101.9 |
| f+Z19 everywhere | 744.2 | −8.8 % | 2555 | 82.8 / 88.3 | 1811.6 | −8.0 % | 3016 | 99.0 / 108.1 |
| f+LZ4 everywhere | 855.7 | +4.9 % | 999 | 32.2 / 39.3 | 2065.3 | +4.8 % | 1127 | 37.0 / 48.6 |

With an 8-page cache, every row's decode falls by 52 to 59 % (Classic) and 44 to 50 % (Enhanced). The cached figures
are in the harness output, the same for every tier. All preloaded, warm, without a cache:
- Classic: 568 (Zstd 9), 593 (A), 628 (B), 640 (f+Z19) and 250 ms (f+LZ4).
- Enhanced: 647, 684, 744, 757 and 283 ms.

**Tiering gives the points between the two uniform choices,** f+LZ4 (+4.9 %, 999 ms) and f+Z19 (−8.8 %, 2555 ms),
Classic:
- Tier A matches today's decode for −1.8 % (Enhanced −2.0 % for +7 %).
- Tier B saves 5.4 % for +6 % (Enhanced −5.9 % for +17 %).
- Tier C, with plain codecs, is worse than today on both counts.

No class has a trade-off of its own (above), so a tier is a way to pick an intermediate point, at the price of a
codec choice per class in the packer. It is not a better point than the uniform choices reach.

### Prediction on the fields layout (XUBC7-style)

`pred-<predictor>-<layout>[-xi]` is the `fields` layout with each block's endpoint values replaced by residuals:
- **The residual** is taken against a block of the same mode, modulo the field's width.
- **Predictors:** `none` (the values themselves, to measure a layout alone); `prev` (the previous block of the mode);
  `up` (the block above when it has the same mode, else `prev`); `med` (LOCO-I's median of left, above and
  above-left when all three share the mode, else above, left or `prev`).
- **Layouts:** `packed` (in the fields' own bits); `bytes` (header and p-bits in whole bytes, then one zigzag byte per
  value); `planes` (`bytes` transposed per mode).
- **`-xi`** also XORs each index record with the predicting block's.

The inverse runs in raster order. The unit tests cover every variant: round trips at widths 1 to 64, zigzag at every
field width, and rejection of malformed maps, lengths and set padding bits.

Results, against the pages as stored (Zstd 9), on every 4th Classic and every 8th Enhanced page:

| Method | Classic, Zstd 9 | Enhanced, Zstd 9 | Classic, Zstd 19 | Enhanced, Zstd 19 | Classic, LZ4 HC 12 | Enhanced, LZ4 HC 12 |
|---|---:|---:|---:|---:|---:|---:|
| `fields` | −4.20 % | −3.95 % | −8.81 % | −8.04 % | +5.06 % | +4.86 % |
| `pred-none-bytes` (no prediction) | −4.22 % | **−4.92 %** | **−9.26 %** | **−8.73 %** | +7.27 % | +7.28 % |
| `pred-none-planes` | −0.51 % | −1.30 % | | | | |
| `pred-prev-planes` (best predictor) | +2.99 % | +2.60 % | −1.06 % | −0.65 % | +18.71 % | +18.91 % |
| `pred-up-bytes` | +4.07 % | +3.75 % | −1.63 % | −0.25 % | +17.59 % | +18.70 % |
| `pred-med-bytes` | +4.41 % | +4.22 % | | | | |
| any `-xi` (index XOR) | +8.1 to +17.2 % | +6.3 to +17.0 % | | | | |

Every prediction loses:
- **Every predictor is 6.5 to 10 points worse than `fields`** at Zstd 9 (24 variants measured), and 7 to 8 points worse
  at Zstd 19.
- **XORing the indices costs another 11 to 12 points.**
- **Why, probably:** neighbouring blocks of the same mode rarely have close endpoints. The packer trims and places frames
  independently (see phase 6), and a block's "previous block of the mode" is often in another frame. A residual is
  then as wide as the value it replaced, and it breaks the exact repeats Zstd was coding.
- **The decode** of the prototype's inverse is 43 ms a page (`pred-prev-planes`, Classic, single thread), against
  2.5 ms for `fields`. It is unoptimised, so this is not its floor.
- **Per clip:** a predicted block depends on earlier blocks outside its frame, so a per-clip join would have to
  restart the prediction at each frame's rectangle, which loses more.

One layout gain without prediction: storing each endpoint value in its own byte (`pred-none-bytes`) is 0 to 1.0
points smaller than `fields` with Zstd (the same for Classic at Zstd 9), but 2.2 to 2.4 points larger with LZ4 HC:
- Classic: −9.26 against −8.81 % at Zstd 19.
- Enhanced: −4.92 against −3.95 % at Zstd 9, and −8.73 against −8.04 % at Zstd 19.

It keeps per-clip joins possible, as no block depends on another. Its prototype inverse is 32 ms; written like
`fields`' inverse it should cost about as much (not measured). It is a refinement of the field split's layout, worth
measuring in full only if the field split is adopted with Zstd.

Commands:

```sh
$B sizes /home/claudio/test/uddp/mobile_anim_cc.uddp cc --every 4 \
    --methods stored,fields+zstd:9,pred-none-packed+zstd:9,pred-prev-planes+zstd:9,pred-up-bytes-xi+zstd:9   # and the rest
$B timing /home/claudio/test/uddp/mobile_anim_cc.uddp cc --every 4 --reps 3 --parallel-passes 1 \
    --methods none+zstd:9,fields+zstd:9,pred-none-bytes+zstd:9,pred-prev-planes+zstd:9
```

To reproduce:

```sh
$B classes /home/claudio/test/uddp/mobile_anim_cc.uddp cc --bounds 96,192,384
$B bodies /home/claudio/test/uddp/mobile_anim_cc.uddp cc --bounds 96,192,384 --bodies 217,400,28,24,86,12,9,826,1248 \
    --methods none+zstd:9,none+zstd:19,none+lz4hc:12,none+xz:6,fields+zstd:9,fields+zstd:19,fields+lz4hc:12,fields+xz:6 --reps 5
$B crowd /home/claudio/test/uddp/mobile_anim_cc.uddp cc --layout stored --bounds 96,192,384 --mix random,people \
    --config today=none+zstd:9 --config f19c=fields+zstd:19@clip --config f4c=fields+lz4hc:12@clip   # and the others
$B crowd /home/claudio/test/uddp/mobile_anim_cc.uddp cc --layout classes --bounds 96,192,384 \
    --config tA=fields+zstd:19@clip/fields+zstd:19@clip/fields+lz4hc:12@clip/fields+lz4hc:12@clip
$B crowd /home/claudio/test/uddp/mobile_anim_ec.uddp ec --layout classes --bounds 1000000 --lru 0,8 --config one9=none+zstd:9
$B sizes /home/claudio/test/uddp/mobile_anim_cc.uddp cc --every 8 --dict-kib 1024 --methods stored,fields+zstd:9,fields+zstd-dict:9
```

`crowd` takes `--lru` (cache sizes, default 0,8,16,32), `--crowds` (20), `--mobiles` (34), `--mix` (random,
people), `--seed` and `--reps` (3). A configuration is `name=method[@clip]`, or one method per class separated by
`/` with `--layout classes`; `@clip` joins a `fields` method per clip.

## Recommendation

### Adopted (2026-11-26)

The user took the field split with Zstd and the cache ("okay for the codec 4", "okay for zstd 19 where its
compression ratio/decompression time makes sense. I'd keep lz4 hc support for now", "okay for the decoded-page cache,
the smallest it makes sense"):
- **Format.** Container codec 4 (`Bc7FieldsZstd`) is the split of this study's layout and checks
  (`udd_container::bc7_fields`), then one Zstd frame; codec 5 (`Bc7FieldsLz4Hc`) is the split, then one LZ4 HC block.
  The codec's high bit is Meta32 bit 17, and a reader refuses reserved bits (`docs/dev_wiki/UDDP_FORMATS.md`).
- **Packers.** The mobile animation packers (and the 2x pages of `--prescale`) default to codec 4 at Zstd 17 (19 until the same evening)
  (`MOBILE_ANIM_PAGE_COMPRESSION`); `--page-coding fields-lz4hc` writes codec 5 and `--page-coding zstd` plain Zstd.
  The GUI's mobile cards have the same choice; Quick Convert packs codec 4 at 17.
- **Reader.** `udd_assets::animation::mobile_pages` keeps a split page split and joins only a clip's blocks
  (`FieldsPage`), in an LRU of `MOBILE_PAGE_CACHE_PAGES` = 6 decoded pages shared by the loader's workers, which wait
  for a page another worker is decoding. Every frame and every clip of both repacked packages comes out byte for
  byte as from the plain Zstd 9 packages (`lib/udd-assets/tests/mobile_pages_split.rs`).

Measured on 2026-11-26 with the machine loaded by other agents (load 19 to 36 on 20 threads), so the decode figures
are slower than the study's above, the split's more than Zstd 9's:

| Package, repacked from the client | Plain Zstd 9 | Split + Zstd 19 | | Pack, Zstd 9 | Pack, split + Zstd 19 |
|---|---:|---:|---:|---:|---:|
| Classic (`_Ultima Online Classic`) | 826.3 MiB | 754.2 MiB | −8.72 % | 56 s | 128 s |
| Enhanced (`_Ultima Online Enhanced fp`) | 2033.7 MiB | 1875.3 MiB | −7.79 % | 147 s | 342 s |

- **Pack time and Quick Convert.** Zstd 19 makes the two mobile packs 72 and 194 s longer, 4.5 minutes in all. The
  user's Create everything of 2026-11-23 (Zstd 9) wrote its packages over about 9 minutes, so it now takes about 13
  to 14, inside the 10 to 15 minutes asked for. Level 17 would save about half of that time for 0.5 to 0.6 points:

  | Level, split (every 8th page) | Classic | Encode thread-s | Enhanced | Encode thread-s |
  |---|---:|---:|---:|---:|
  | plain Zstd 9 | 0 | 12.0 | 0 | 23.7 |
  | 9 | −4.36 % | 11.2 | −3.95 % | 24.0 |
  | 15 | −5.36 % | 48.3 | −4.41 % | 96.9 |
  | 17 | −8.30 % | 84.6 | −7.40 % | 165.7 |
  | 18 | −8.45 % | 109.8 | −7.68 % | 222.2 |
  | 19 | −8.83 % | 179.0 | −8.04 % | 366.0 |

  The user took 17 the same day ("you can keep 17 then, if the extra time is halved for only a little less compression
  efficiency"): `MOBILE_ANIM_PAGE_ZSTD_LEVEL` is 17, so the packers, the GUI cards and Quick Convert default to it.
  The package sizes and pack times in the table above are level 19's; expect about 0.4 to 0.6 points less saving and
  about half the extra time.

- **Cache size.** `crowd --lru 0,2,4,6,8,16`, 20 crowds of 34 mobiles in the loader's order. A random crowd's page
  decodes reach their floor at 2 pages (Classic 766, 340, 335, 332, 332, 332; Enhanced 804, 457, 455, 455, 455,
  447), but an Enhanced crowd of humans needs 6 (49, 30, 6, 5, 5, 5), so the cache holds 6, at most 27 MiB split.
- **Crowd timings,** first drawable and all preloaded, warm / cold, in ms (medians of 20 crowds):

  | Crowd | Zstd 9, no cache (before) | Zstd 9, 6 pages | Split + Zstd 19 per clip, 6 pages (now) |
  |---|---:|---:|---:|
  | Classic random, first drawable | 74.9 / 81.6 | 56.5 / 68.4 | 75.1 / 86.2 |
  | Classic random, all preloaded | 585 / 594 | 439 / 452 | 587 / 599 |
  | Enhanced random, first drawable | 85.8 / 96.7 | 70.3 / 86.1 | 90.7 / 105.6 |
  | Enhanced random, all preloaded | 664 / 674 | 547 / 564 | 704 / 720 |
  | Classic people, first drawable | 42.9 / 43.1 | 3.7 / 4.6 | 5.8 / 6.6 |
  | Enhanced people, first drawable | 44.3 / 44.9 | 8.1 / 9.7 | 13.3 / 14.6 |

  With the cache, the split at Zstd 19 is as fast as Zstd 9 was without it in Classic and 6 % slower in Enhanced
  (5 ms to the last of 34 mobiles), and a crowd of people is 3 to 7 times faster.

### The study's recommendation (2026-11-25)

**Not worth it as a default; nothing in the packers or the format changes.** Against the brief's gates (at least 3 %
smaller on the full package, at most 5 % slower to decode):

| Candidate | Classic | Enhanced | Decode p50, Classic / Enhanced | 4 workers pages/s | Format change | Verdict |
|---|---:|---:|---:|---:|---|---|
| Zstd 9 (today) | 815.70 MiB | 1970.03 MiB | 1.85 / 2.25 ms | 1880–1960 / 1580–1720 | | |
| dictionaries, 1 MiB | −0.60 % | −0.41 % | | | dictionary entry | below 3 % |
| Zstd 19 | −3.61 % | −2.82 % | +8 / +10 % | −8 / −11 % | none (packer level) | fails decode, Enhanced below 3 % |
| fields + Zstd 9 | −4.35 % | −3.99 % | +129 / +116 % | −52 / −49 % | new codec id | fails decode |
| fields + Zstd 19 | −8.84 % | −8.04 % | +148 / +138 % | −54 / −52 % | new codec id | fails decode |
| BC7 RDO λ 1 (lossy) | −5.08 % | −5.14 % | | | none | lossy; off by default |

- **The one transform that pays is the BC7 field split** (`fields`): each mode's endpoint fields and index bits
  stored as separate streams ahead of the coder, exactly reversible. It saves 4.0 to 4.4 % at Zstd 9 and 8.0 to 8.8 %
  at Zstd 19 (35 / 78 MiB and 72 / 158 MiB), and still 2.3 to 7.7 % on RDO pages. Its whole-page inverse doubles a
  page's decode, which is what the gates above measure.
- **Joined per clip, the field split is affordable** (second study). In a 34-mobile crowd under the loader order:
  - `fields` + Zstd 19 costs 7 % (Classic) and 18 % (Enhanced) on the time to the first drawable, for −8.8 / −8.0 %.
  - `fields` + LZ4 HC 12 decodes 2.4 times as fast as today, for +4.8 % on both packages. That is a quarter of plain
    LZ4 HC's size cost, at plain LZ4 HC's speed.

  Either would be one versioned proposal, not built: a `udd_container::uddp::Codec` value (the next free one, 4)
  meaning "BC7 field split, then one Zstd frame" (or one LZ4 HC block), for mobile animation pages only. It would use
  the harness's layout (the mode map; each mode's field records; each mode's index records; reserved blocks; the tail)
  and its checks (map values, stream lengths, padding bits). The reader would hand on a clip's blocks through
  `fields_extract` instead of whole pages, which moves work into `stored_clip` and the reader. It is a design for the
  user to decide, and its trade is a choice of point:
  - `fields` + Zstd 19 for disk (−8 to −9 %, a crowd 7 to 18 % slower).
  - `fields` + LZ4 HC 12 for speed (+5 %, 2.4 times faster).
- **A bounded cache of about 8 decoded pages** is the cheapest gain and is independent of the codec. It halves a
  crowd's page decodes (−57 % Classic, −43 % Enhanced) and cuts the time to the first drawable by 17 to 25 %, for
  32 MiB. It is a reader change; the existing `AtlasPageCache` is unbounded, and off for these packages. With it,
  `fields` + Zstd 19 per clip is faster than today is without it.
- **Not worth it:**
  - Tiering the codec by body size: no class has a trade-off of its own, so tiers only give points between the
    uniform choices.
  - xz, 15 to 21 times slower to decode.
  - XUBC7-style prediction: 6.5 to 10 points worse than `fields`.
  - A dictionary on top of `fields`: +0.24 points.
- **The LZ4 HC question** is the opposite trade to the field split:
  - The 20 % rule costs +9.4 / +10.0 % (77 / 197 MiB) for 1.5 to 1.7 times the pages/s warm and about 1.3 times cold.
  - LZ4 HC 12 everywhere costs +19 / +17 % for 2.4 times warm.
  - `fields` + LZ4 HC 12 per clip gets that speed for +4.8 %.
- **Not worth pursuing either:** higher levels alone, long mode, grouping pages, chunking, dictionaries, byte shuffles,
  transposed field records, block reuse (all under [Negative results](#negative-results)), phase 6. The unpredicted
  byte layout `pred-none-bytes` (0 to 1 point better than `fields` with Zstd) is worth measuring only if the field
  split is adopted with Zstd.

## Lossy conditioning

Third study (2026-11-26): how much smaller can the pages get if the BC7 blocks may change, for how much quality, and
is there a better trade than the library RDO? Measurements only: no packer, format or renderer change.

### Method

`udd-bc7-supercompress lossy` (`lossy.rs`, block model in `bc7block.rs`) reads two packages packed from this commit's
`udd-cli` on the CPU, from the clients under `/mnt/dati/_proj_local/_uo_clients/`:
- a BC7 package with RDO off (Classic 867 478 192 bytes, 1210 pages; Enhanced 1 939 014 497 bytes, 2291 pages, the
  sizes of [BC7 RDO](#bc7-rdo));
- the same packer with `--raw`: RGBA8888 pages with the same layout, the source pixels.

For each page it rebuilds the packer's pixels (content, extruded gutters, transparent background), encodes them with
the converter's encoder (`encode_to_bc7`, `AnalyticalWide`), then runs each variant on the blocks. `--check` confirms
the pipeline: on 21 Classic pages the plain encode equals the package's blocks bit for bit, and the rebuilt pixels
equal the RGBA package's. Every candidate is measured against the source frames, not against the plain encode.

Sizes are each page as one frame under each method (`none+zstd:9` is today's format). Quality, per size class of the
body (≤ 96 / 97–192 / 193–384 / > 384 px, as above):
- **PSNR** over the visible texels (source alpha > 0), RGB.
- **Alpha:** texels whose alpha differs from the source, and **silhouette** texels, whose coverage (alpha 0 or not)
  differs. Every step here except the library RDO keeps every texel's alpha exactly as the plain encode decodes it,
  so its silhouette changes are 0 by construction. (The plain encode itself already changes coverage on 71 texels of
  the Classic sample and on 1.28 M of the Enhanced one, whose alpha is graded.)
- **SSIMULACRA2** on every 50th placement, the frame composited over grey 128 on a canvas of at least 64 px.
- **Flicker:** the RMS of the change between the decodes of consecutive frames of a clip, over the texels whose
  source is identical and visible at the same place relative to the anchor. The source does not change there, so
  every level of it is coding noise that moves; the plain encode has 3.6 (Classic) and 3.7 (Enhanced).

Samples are every 60th page: 21 Classic and 39 Enhanced pages (22 and 40 in the aligned layout).

### The steps

- **`rdo:<λ>:<look-back>`:** `uot_texture_codecs::bc7::rdo` (the bc7enc_rdo port), one window over the page, as
  the packer's `--bc7-rdo` (the packer splits a page into row chunks; the difference is a few blocks at chunk edges).
- **`zrdo:<λ>:<rms>`, Zstd-priced block reuse:** a visible block may become an earlier block of the page, whole. The
  candidates are the block at the last match offset, the blocks to the left and above, the co-located blocks of the
  clip's adjacent frames (±1 block, relative to the anchor), and the last 8 blocks of the page with the same mode and
  a similar mean (a hash, so the reach is the whole page). A candidate is accepted only if it keeps the alpha of every
  texel and its RMS error over the visible texels is at most `rms` levels above the plain encode's. Among them, the
  cheapest by error + λ × bits wins, the bits from a model of Zstd's sequences: 7 bits a literal byte, 12 bits plus
  log2 of the offset for a new match, 2 bits for a match that continues the previous one at the same offset.
- **`fields:<λ>:<rms>`, field-split-aware reuse:** priced for the `fields` layout. A block may take the endpoint
  fields of an earlier block of its mode (its indices picked again), or its index bits (its endpoints fitted again
  by least squares), so that one of its two records repeats in its mode's stream.
- **`lrdo` and `lfields`:** the same two steps priced for the LZ4 block format: raw literals (8 bits a byte plus a
  length byte per 255), a token and a 2-byte offset per new match, a length byte per 255 bytes to extend one, no
  repeat offsets, and candidates beyond the 64 KiB window dropped.
- **`coarse:<bits>:<rms>`, XUBC7-lossy-like endpoint quantisation:** each colour endpoint rounded to a multiple of
  `1 << bits` in its stored precision (alpha endpoints and p-bits kept), the indices picked again; kept per block
  within `rms`.
- **`flat:<rms>`:** an opaque block within `rms` of its mean becomes a solid mode 6 block with zero indices.
- **`reuse:<rms>[:<radius>]`, temporal reuse:** on the anchor-aligned layout (below), a block takes the block of the
  previous or next frame of its clip at the same place relative to the anchor (or ±`radius` blocks), when that block
  is earlier on the page and the error stays within `rms`; a tie goes to the last offset used, so runs stay one match.

Steps chain with `+`, each against the plain encode's error, so a chain does not compound its bound.

### Results on the samples

Δ against the plain encode under the same method; PSNR and SSIMULACRA2 as differences; flicker as measured.

Classic (plain: 34.00 dB, SSIMULACRA2 81.18, flicker 3.61; `rdo:1:64` and `rdo:1:256` without SSIMULACRA2):

| Variant | Zstd 9 | Zstd 19 | fields + Zstd 19 | fields + LZ4 HC 12 | Δ PSNR dB | Δ SSIMULACRA2 | Flicker | Silhouette texels changed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `rdo:0.5:64` | −1.26 % | −1.82 % | −1.55 % | −0.96 % | −0.81 | −2.07 | 4.50 | +5034 |
| `rdo:1:64` | −4.30 % | | −4.13 % | | −2.03 | | 5.74 | +8167 |
| `rdo:1:256` | −7.53 % | | −6.63 % | | −2.31 | | 6.04 | +8488 |
| `rdo:2:64` | −10.08 % | −11.10 % | −9.15 % | −6.28 % | −4.33 | −9.40 | 8.11 | +13406 |
| `rdo:4:64` | −17.79 % | −18.97 % | −16.15 % | −11.42 % | −7.24 | −15.37 | 11.46 | +25305 |
| `rdo:1:1024` | −13.07 % | −13.23 % | −10.50 % | −8.90 % | −2.63 | −4.83 | 6.36 | +4228 |
| `rdo:1:4096` | −17.80 % | −17.69 % | −13.40 % | −12.27 % | −2.83 | −4.94 | 6.51 | +2172 |
| `zrdo:10:3` | −3.26 % | −3.14 % | −2.66 % | −2.52 % | −0.11 | −0.11 | 3.76 | +0 |
| `zrdo:30:3` | −5.06 % | −4.96 % | −4.10 % | −3.74 % | −0.24 | −0.28 | 3.92 | +0 |
| `zrdo:100:6` | −15.15 % | −15.14 % | −12.55 % | −11.17 % | −1.59 | −1.63 | 5.52 | +0 |
| `fields:10:3` | −2.72 % | −2.74 % | −3.73 % | −3.11 % | −0.17 | −0.27 | 3.58 | +0 |
| `fields:30:3` | −7.44 % | −7.70 % | −9.43 % | −7.86 % | −0.77 | −1.18 | 3.83 | +0 |
| `fields:100:3` | −8.97 % | −9.38 % | −11.36 % | −9.49 % | −1.21 | −2.28 | 4.03 | +0 |
| `fields:100:6` | −13.18 % | −13.90 % | −16.80 % | −14.29 % | −2.07 | −3.77 | 4.54 | +0 |
| `coarse:1:2` | −4.77 % | −5.30 % | −5.03 % | −4.28 % | −0.40 | −1.06 | 3.95 | +0 |
| `coarse:2:2` | −10.28 % | −11.16 % | −10.67 % | −9.28 % | −0.61 | −2.46 | 4.08 | +0 |
| `coarse:2:4` | −11.06 % | −12.27 % | −11.63 % | −10.29 % | −1.03 | −4.18 | 4.33 | +0 |
| `coarse:3:4` | −15.27 % | −16.68 % | −15.53 % | −14.24 % | −1.68 | −5.77 | 5.04 | +0 |
| `flat:2` | −0.65 % | −0.65 % | −0.54 % | −0.53 % | −0.02 | −0.04 | 3.65 | +0 |
| `flat:4` | −3.05 % | −3.08 % | −2.76 % | −2.64 % | −0.23 | −0.32 | 3.93 | +0 |
| `flat:8` | −9.46 % | −9.62 % | −8.89 % | −8.33 % | −1.48 | −1.22 | 5.06 | +0 |
| `coarse:2:2+flat:4` | −12.63 % | −13.57 % | −12.83 % | −11.29 % | −0.80 | −2.70 | 4.34 | +0 |
| `coarse:2:2+zrdo:30:3` | −14.22 % | −15.04 % | −13.85 % | −12.14 % | −0.79 | −2.63 | 4.29 | +0 |
| `coarse:2:2+fields:30:3` | −11.24 % | −12.16 % | −13.63 % | −11.62 % | −1.10 | −2.55 | 4.08 | +0 |

Enhanced (plain: 31.73 dB, SSIMULACRA2 82.15, flicker 3.75):

| Variant | Zstd 9 | Zstd 19 | fields + Zstd 19 | fields + LZ4 HC 12 | Δ PSNR dB | Δ SSIMULACRA2 | Flicker | Silhouette texels changed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `rdo:1:64` | −5.28 % | −6.06 % | −4.80 % | −3.38 % | −1.03 | −2.05 | 4.90 | +233307 |
| `rdo:1:256` | −8.07 % | −8.60 % | −7.06 % | −5.42 % | −1.22 | −2.20 | 5.12 | +208668 |
| `rdo:2:64` | −9.96 % | −10.91 % | −8.81 % | −6.16 % | −2.52 | −4.18 | 6.52 | +383282 |
| `zrdo:30:3` | −4.27 % | −4.14 % | −3.57 % | −3.20 % | −0.09 | −0.09 | 3.80 | +0 |
| `zrdo:100:6` | −10.41 % | −10.33 % | −8.89 % | −7.73 % | −0.60 | −0.47 | 4.50 | +0 |
| `fields:30:3` | −5.91 % | −5.90 % | −6.14 % | −5.10 % | −0.15 | +0.00 | 3.79 | +0 |
| `fields:100:3` | −7.11 % | −7.15 % | −7.38 % | −6.11 % | −0.35 | −0.23 | 3.87 | +0 |
| `coarse:1:2` | −3.86 % | −4.21 % | −4.30 % | −3.65 % | −0.52 | −1.65 | 4.10 | +0 |
| `coarse:2:2` | −7.51 % | −7.94 % | −8.09 % | −6.98 % | −0.58 | −2.75 | 4.18 | +0 |
| `coarse:2:4` | −9.14 % | −9.76 % | −9.91 % | −8.71 % | −1.23 | −5.01 | 4.65 | +0 |
| `coarse:3:4` | −10.88 % | −11.47 % | −11.20 % | −10.26 % | −1.16 | −3.55 | 4.60 | +0 |
| `flat:4` | −2.63 % | −2.65 % | −2.40 % | −2.25 % | −0.10 | −0.09 | 3.88 | +0 |
| `flat:8` | −7.89 % | −8.01 % | −7.58 % | −6.99 % | −0.76 | −0.42 | 4.55 | +0 |
| `coarse:2:2+flat:4` | −9.53 % | −10.01 % | −9.96 % | −8.68 % | −0.66 | −2.81 | 4.29 | +0 |
| `coarse:2:2+zrdo:30:3` | −10.73 % | −11.10 % | −10.77 % | −9.36 % | −0.65 | −2.79 | 4.18 | +0 |
| `coarse:2:4+zrdo:30:3` | −12.11 % | −12.67 % | −12.39 % | −10.91 % | −1.27 | −5.02 | 4.64 | +0 |
| `coarse:2:2+fields:30:3` | −8.76 % | −9.12 % | −9.45 % | −7.97 % | −0.57 | −1.93 | 4.11 | +0 |

**The library RDO** buys size with a lot of error, and changes silhouettes:
- At λ 1, look-back 64 (the packer's) it saves 4.3 % (Classic) and 5.3 % (Enhanced) for 2.0 and 1.0 dB and 2 points
  of SSIMULACRA2 (Enhanced). λ 2 and 4 save 10 and 18 % for 4.3 and 7.2 dB.
- **A longer look-back pays more than a higher λ:** 256, 1024 and 4096 blocks save 7.5, 13.1 and 17.8 % (Classic,
  Zstd 9) for 2.3, 2.6 and 2.8 dB. The whole page (up to 262 144 blocks) was not run: the step takes 5.3, 13, 56 and
  137 s a page on one thread for those four windows, against 0.26 s for the encode itself, so a whole-page window
  would take hours a page.
- **It moves alpha:** 2 000 to 25 000 more silhouette texels on 21 Classic pages, 209 000 to 383 000 on 39 Enhanced
  ones. The brief's rule is that silhouettes do not change; the RDO would need an alpha constraint first.
- Its saving survives the coders unevenly: at look-back 4096, −17.8 % under Zstd 9 is −13.4 % under `fields` +
  Zstd 19, since a whole-block copy breaks into one match per record stream.

**Coder-aware block reuse (`zrdo`) gets the RDO's saving for a tenth of its error.** `zrdo:30:3` saves 5.1 / 4.3 %
(about what λ 1 saves) for 0.24 / 0.09 dB, no silhouette change, and flicker barely up. λ above 30 changes nothing:
the error bound decides how many blocks may move, the price only which candidate. `zrdo:100:6` saves 15.2 / 10.4 %
for 1.6 / 0.6 dB, still a quarter of the RDO's error at that size.

**The field-split-aware variant (`fields`)** pays under `fields` + Zstd 19: −9.4 % (Classic) for 0.8 dB, and −6.1 %
(Enhanced) for 0.15 dB with no SSIMULACRA2 change, more than `zrdo` there. Under plain Zstd it is weaker.

**Endpoint coarsening is the cheapest per dB but the most visible to SSIMULACRA2.** `coarse:2:2` saves 10.3 / 7.5 %
for 0.6 dB, at 0.15 s a page, but costs 2.5 to 2.8 points of SSIMULACRA2 (flat gradients step), where `zrdo` at the
same PSNR costs a fraction of a point. One bit is weak (−4 to −5 %), three bits cost 1.7 to 2.7 dB. With `zrdo:30:3`
after it, `coarse:2:2+zrdo:30:3` is the strongest chain at that quality: −14.2 / −10.7 % for 0.8 / 0.65 dB. Further
steps on top add little (`flat`, `fields`: under a point).

**Flat blocks (`flat`)** are rare: within 4 levels, 4.6 % (Classic) and 8.5 % (Enhanced) of the visible blocks, for
−3.1 / −2.6 %.

**An LZ4-priced RDO is no better than the Zstd-priced one under LZ4 HC 12.** On the same samples, under LZ4 HC 12
(Classic columns first, then Enhanced):

| Variant | LZ4 HC 12 | fields + LZ4 HC 12 | Δ PSNR dB | LZ4 HC 12 | fields + LZ4 HC 12 | Δ PSNR dB |
|---|---:|---:|---:|---:|---:|---:|
| `zrdo:30:3` | −3.45 % | −3.74 % | −0.24 | −2.69 % | −3.20 % | −0.09 |
| `lrdo:30:3` | −3.54 % | −2.30 % | −0.16 | −2.78 % | −2.12 % | −0.06 |
| `zrdo:100:6` | −12.13 % | −11.17 % | −1.59 | −7.46 % | −7.73 % | −0.60 |
| `lrdo:100:6` | −12.45 % | −8.78 % | −1.33 | −7.73 % | −6.01 % | −0.48 |
| `fields:30:3` | −7.71 % | −7.86 % | −0.77 | −4.64 % | −5.10 % | −0.15 |
| `lfields:30:3` | −7.76 % | −7.15 % | −0.68 | −4.76 % | −4.43 % | −0.13 |
| `lfields:100:3` | −9.73 % | −9.17 % | −1.18 | −5.86 % | −5.60 % | −0.33 |
| `lfields:100:6` | −14.29 % | −14.08 % | −2.02 | −8.09 % | −8.04 % | −0.72 |
| `coarse:2:2+lrdo:30:3` | −11.42 % | −11.00 % | −0.73 | −7.71 % | −8.55 % | −0.63 |
| `coarse:2:2+lfields:30:3` | −11.45 % | −11.36 % | −1.06 | −7.13 % | −7.70 % | −0.57 |

The LZ4 model takes fewer candidates (a new match costs 24 bits, twice Zstd's, and the window stops at 4096 blocks),
so at the same bound it gets a fraction of a point more per dB under plain LZ4 HC and less under `fields` + LZ4 HC,
where the Zstd-priced `fields` still wins. Whatever the coder, the bound sets the trade; the model only picks among
candidates the bound allows.

### Temporal reuse on an anchor-aligned layout

The packer places each frame's trimmed box at a 4-pixel-aligned spot, so a body part falls on a different block grid
in each frame. `--layout aligned` replays the packer (as `repack.rs` does) with each frame padded on the left and top
by its anchor's phase, so that every anchor falls on a block corner and every frame of a clip shares one grid relative
to its anchor.

**What the alignment costs, full packages (plain encode, no reuse):**

| Package | Pages | Blocks | Zstd 9 | fields + Zstd 19 | PSNR dB |
|---|---:|---:|---:|---:|---:|
| Classic, stored layout | 1210 | 217 063 107 | 816.67 MiB | 752.47 MiB | 33.21 |
| Classic, aligned | 1299 (+7.4 %) | 225 183 380 (+3.74 %) | 812.83 MiB (−0.47 %) | 751.49 MiB (−0.13 %) | 33.26 |
| Enhanced, stored layout | 2291 | 428 210 672 | 1798.33 MiB | 1658.59 MiB | 31.37 |
| Enhanced, aligned | 2343 (+2.3 %) | 444 352 947 (+3.77 %) | 1792.69 MiB (−0.31 %) | 1658.23 MiB (−0.02 %) | 31.42 |

The padding is transparent blocks, which cost Zstd next to nothing, and the aligned pages come out slightly smaller:
frames of a clip now share their block grid, so more of their blocks repeat. The real cost is the page count: 7.4 %
more Classic pages (each a 2048 × 2048 allocation to the reader, and more pages for a clip to touch) and 2.3 % more
Enhanced ones. The layout would be a packer change (the anchor's phase as padding before the allocator).

On the aligned samples (Δ against the aligned plain encode):

Classic (plain: 33.01 dB, SSIMULACRA2 91.68, flicker 3.45):

| Variant | Zstd 9 | Zstd 19 | fields + Zstd 19 | fields + LZ4 HC 12 | Δ PSNR dB | Δ SSIMULACRA2 | Flicker | Silhouette texels changed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `reuse:0` | −0.04 % | −0.04 % | −0.03 % | −0.03 % | +0.00 | +0.00 | 3.42 | +0 |
| `reuse:1` | −0.79 % | −0.77 % | −0.63 % | −0.45 % | −0.01 | −0.01 | 3.38 | +0 |
| `reuse:2` | −1.98 % | −1.95 % | −1.63 % | −1.13 % | −0.05 | −0.03 | 3.40 | +0 |
| `reuse:4` | −4.72 % | −4.72 % | −3.91 % | −2.67 % | −0.23 | −0.09 | 3.62 | +0 |
| `reuse:8` | −10.47 % | −10.59 % | −8.80 % | −5.74 % | −1.17 | −0.36 | 4.61 | +0 |
| `reuse:2:0` | −1.67 % | −1.64 % | −1.40 % | −0.98 % | −0.04 | −0.03 | 3.37 | +0 |
| `reuse:4:0` | −3.55 % | −3.53 % | −3.03 % | −2.06 % | −0.16 | −0.08 | 3.43 | +0 |
| `coarse:2:2` | −9.98 % | −10.83 % | −10.30 % | −8.65 % | −0.51 | −0.95 | 3.88 | +0 |
| `fields:30:3` | −7.88 % | −8.21 % | −9.46 % | −7.60 % | −0.62 | −0.47 | 3.52 | +0 |
| `coarse:2:2+reuse:2` | −11.41 % | −12.26 % | −11.49 % | −9.47 % | −0.54 | −0.96 | 3.80 | +0 |
| `coarse:2:2+reuse:2+fields:30:3` | −11.65 % | −12.63 % | −13.63 % | −11.25 % | −0.90 | −0.88 | 3.72 | +0 |

Enhanced (plain: 31.40 dB, SSIMULACRA2 83.05, flicker 3.56):

| Variant | Zstd 9 | Zstd 19 | fields + Zstd 19 | fields + LZ4 HC 12 | Δ PSNR dB | Δ SSIMULACRA2 | Flicker | Silhouette texels changed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `reuse:0` | −0.14 % | −0.13 % | −0.11 % | −0.12 % | +0.00 | −0.01 | 3.48 | +0 |
| `reuse:1` | −1.71 % | −1.62 % | −1.44 % | −1.37 % | −0.01 | −0.03 | 3.35 | +0 |
| `reuse:2` | −2.94 % | −2.83 % | −2.48 % | −2.21 % | −0.04 | −0.04 | 3.35 | +0 |
| `reuse:4` | −5.27 % | −5.16 % | −4.48 % | −3.69 % | −0.14 | −0.16 | 3.44 | +0 |
| `reuse:8` | −9.58 % | −9.53 % | −8.27 % | −6.47 % | −0.65 | −0.73 | 3.93 | +0 |
| `reuse:2:0` | −2.66 % | −2.55 % | −2.26 % | −2.04 % | −0.03 | −0.04 | 3.33 | +0 |
| `coarse:2:2` | −8.24 % | −8.84 % | −8.85 % | −7.61 % | −0.59 | −3.32 | 4.05 | +0 |
| `coarse:2:2+reuse:2` | −10.23 % | −10.75 % | −10.52 % | −9.12 % | −0.61 | −3.34 | 3.82 | +0 |

`reuse:<rms>:0` takes only the co-located block, the default also its 8 neighbours.

- **Exact reuse is nearly nil:** with the grid aligned, 0.20 % (Classic) and 0.35 % (Enhanced) of the visible blocks
  of the full packages equal the co-located block of an adjacent frame (±1 block), for −0.08 / −0.14 % under Zstd 9
  (`reuse:0`). A body moves by fractions of a block
  and is shaded again in every frame.
- **Reuse within an error bound** is the cleanest of the lossy steps per dB: `reuse:2` saves 2.0 / 2.9 % for 0.05 /
  0.04 dB, `reuse:4` 4.7 / 5.3 % for 0.23 / 0.14 dB, and it lowers flicker (a reused block cannot flicker). It does
  not reach the other steps' sizes: `reuse:8` −10.5 / −9.6 % costs 1.2 / 0.65 dB.
- **By action**, idle clips are where it works. Share of each action's visible blocks that `reuse:2` takes on the
  full packages (movement clips as `movement_action` / `move_action` resolve walk and run, idle ones stand; "other"
  is every other action, attacks, casts, deaths and fidgets):

  | Package | Walk / run | Stand | Other | Whole package, Zstd 9 | fields + Zstd 19 | Δ PSNR dB |
  |---|---:|---:|---:|---:|---:|---:|
  | Classic | 2.5 % | 18.4 % | 3.7 % | −2.65 % | −2.18 % | −0.06 |
  | Enhanced | 2.2 % | 12.4 % | 4.2 % | −2.88 % | −2.46 % | −0.04 |

  A walking or attacking body moves and is shaded again in every frame; a standing one breathes in place, and a
  fifth of its blocks can stay. Exact reuse follows the same pattern at a tenth of the rate (stand 1.0 / 0.8 %).
- On top of `coarse:2:2` it adds 1.4 / 2.0 points.

### Full packages

Classic:

| Variant | Zstd 9 MiB | Zstd 19 MiB | fields + Zstd 19 MiB | fields + LZ4 HC 12 MiB | PSNR dB | Δ dB | Δ SSIMULACRA2 | Flicker |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain | 816.67 | 787.24 | 752.47 | 862.15 | 33.21 |  |  | 4.15 |
| `zrdo:30:3` | 769.76 (−5.74 %) | 743.00 (−5.62 %) | 717.15 (−4.69 %) | 825.61 (−4.24 %) | 33.01 | −0.21 | −0.25 | 4.41 |
| `fields:30:3` | 750.99 (−8.04 %) | 721.79 (−8.31 %) | 678.03 (−9.89 %) | 791.47 (−8.20 %) | 32.60 | −0.61 | −0.80 | 4.32 |
| `zrdo:100:6` | 691.89 (−15.28 %) | 667.22 (−15.25 %) | 656.18 (−12.80 %) | 764.31 (−11.35 %) | 31.91 | −1.30 | −1.08 | 5.77 |
| `coarse:2:2+zrdo:30:3` | 698.33 (−14.49 %) | 666.28 (−15.37 %) | 645.95 (−14.16 %) | 755.79 (−12.34 %) | 32.51 | −0.71 | −2.15 | 4.80 |

Enhanced:

| Variant | Zstd 9 MiB | Zstd 19 MiB | fields + Zstd 19 MiB | fields + LZ4 HC 12 MiB | PSNR dB | Δ dB | Δ SSIMULACRA2 | Flicker |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| plain | 1798.33 | 1748.62 | 1658.59 | 1885.96 | 31.37 |  |  | 4.79 |
| `zrdo:30:3` | 1708.84 (−4.98 %) | 1664.16 (−4.83 %) | 1588.54 (−4.22 %) | 1814.95 (−3.77 %) | 31.27 | −0.10 | −0.13 | 4.83 |
| `fields:30:3` | 1682.04 (−6.47 %) | 1635.10 (−6.49 %) | 1544.51 (−6.88 %) | 1778.16 (−5.72 %) | 31.21 | −0.15 | −0.20 | 4.81 |
| `zrdo:100:6` | 1592.11 (−11.47 %) | 1549.38 (−11.39 %) | 1494.02 (−9.92 %) | 1722.37 (−8.67 %) | 30.74 | −0.62 | −0.73 | 5.64 |
| `coarse:2:2+zrdo:30:3` | 1584.18 (−11.91 %) | 1533.59 (−12.30 %) | 1461.14 (−11.91 %) | 1690.32 (−10.37 %) | 30.72 | −0.64 | −2.85 | 5.30 |

Per size class (full packages, Δ PSNR dB / Δ SSIMULACRA2 / flicker):

Classic (plain: ≤ 96 px 31.64 dB / 86.43,
97–192 px 33.38 dB / 78.00, 193–384 px 33.31 dB / 78.01, > 384 px 34.29 dB / 84.38):

| Variant | ≤ 96 px | 97–192 px | 193–384 px | > 384 px |
|---|---:|---:|---:|---:|
| `zrdo:30:3` | −0.14 / −0.15 / 4.74 | −0.22 / −0.30 / 4.25 | −0.23 / −1.15 / 4.42 | −0.24 / −1.41 / 4.22 |
| `fields:30:3` | −0.32 / −0.58 / 4.63 | −0.64 / −1.48 / 4.17 | −0.64 / −2.49 / 4.29 | −0.86 / −2.13 / 4.20 |
| `zrdo:100:6` | −0.73 / −0.60 / 5.54 | −1.27 / −1.68 / 5.54 | −1.44 / −5.43 / 6.09 | −1.73 / −6.73 / 5.81 |
| `coarse:2:2+zrdo:30:3` | −0.60 / −1.95 / 5.11 | −0.75 / −2.86 / 4.64 | −0.69 / −3.34 / 4.79 | −0.82 / −3.49 / 4.67 |

Enhanced (plain: ≤ 96 px 29.79 dB / 84.65,
97–192 px 31.36 dB / 75.45, 193–384 px 32.04 dB / 78.26, > 384 px 32.20 dB / 83.61):

| Variant | ≤ 96 px | 97–192 px | 193–384 px | > 384 px |
|---|---:|---:|---:|---:|
| `zrdo:30:3` | −0.09 / −0.07 / 4.45 | −0.10 / −0.19 / 5.11 | −0.10 / −0.55 / 4.98 | −0.10 / −1.22 / 4.82 |
| `fields:30:3` | +0.04 / −0.05 / 4.49 | −0.08 / −0.52 / 5.03 | −0.23 / −1.23 / 4.93 | −0.40 / −2.16 / 4.81 |
| `zrdo:100:6` | −0.35 / −0.31 / 4.81 | −0.59 / −1.21 / 5.80 | −0.78 / −3.79 / 6.05 | −0.81 / −7.06 / 6.05 |
| `coarse:2:2+zrdo:30:3` | −0.56 / −2.66 / 4.87 | −0.64 / −3.59 / 5.59 | −0.68 / −3.62 / 5.48 | −0.72 / −4.28 / 5.31 |

The larger bodies lose more SSIMULACRA2 than PSNR: their smooth shading shows endpoint steps and reused blocks more
than the small sprites' texture does. The package figure is weighted by placements, most of them small, and it hides
where `zrdo:100:6` hurts: on bodies above 192 px it loses 3.8 to 7.1 points, more than `coarse:2:2+zrdo:30:3`
(3.3 to 4.3), and its flicker rises by 1.2 to 2.0 levels against 0.6 to 0.8. `zrdo:30:3` stays within 1.4 points
everywhere.

### Decode and crowds

The lossy steps change only which bytes the blocks hold, so a reader decodes them as it does today. `crowd`
(above) was run on each candidate's stored pages (`--pages`, the blocks the full runs kept), with the configs of
[Crowds](#crowds-under-the-loader-order): today's Zstd 9 per page, and `fields` + Zstd 19 and `fields` + LZ4 HC 12
joined per clip, each without and with an 8-page decoded cache. Timed alone (the benchmark marker held), same
machine as the earlier tables.

Random crowds, Classic (median of 20; MiB and Δ against the plain encode under the same config):

| Pages | Config | MiB | Cache | Page decodes | Decode ms | First drawable, warm / cold | Mobile p50, warm | All preloaded, warm / cold |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| plain | Zstd 9 | 816.7 | 0 | 766 | 2390 | 77.3 / 83.8 | 39.0 | 599 / 606 |
|  |  |  | 8 | 332 | 996 | 58.0 / 69.8 | 29.4 | 454 / 465 |
|  | fields + Zstd 19, per clip | 752.5 | 0 | 766 | 2687 | 86.5 / 93.1 | 43.7 | 673 / 680 |
|  |  |  | 8 | 332 | 1180 | 66.2 / 76.4 | 33.0 | 515 / 527 |
|  | fields + LZ4 HC 12, per clip | 862.1 | 0 | 766 | 1136 | 36.9 / 43.9 | 18.5 | 285 / 292 |
|  |  |  | 8 | 332 | 559 | 28.3 / 41.3 | 14.3 | 220 / 234 |
| `zrdo:30:3` | Zstd 9 | 769.8 (−5.7 %) | 0 | 766 | 2365 | 76.9 / 83.3 | 36.7 | 593 / 600 |
|  |  |  | 8 | 332 | 971 | 56.5 / 67.6 | 28.1 | 439 / 451 |
|  | fields + Zstd 19, per clip | 717.2 (−4.7 %) | 0 | 766 | 2738 | 88.6 / 94.4 | 42.9 | 686 / 692 |
|  |  |  | 8 | 332 | 1215 | 66.6 / 77.0 | 33.3 | 529 / 539 |
|  | fields + LZ4 HC 12, per clip | 825.6 (−4.2 %) | 0 | 766 | 1127 | 36.5 / 43.3 | 17.8 | 282 / 290 |
|  |  |  | 8 | 332 | 535 | 28.0 / 40.5 | 14.1 | 218 / 230 |
| `coarse:2:2+zrdo:30:3` | Zstd 9 | 698.3 (−14.5 %) | 0 | 766 | 2298 | 74.1 / 80.1 | 37.0 | 576 / 582 |
|  |  |  | 8 | 332 | 958 | 56.1 / 66.7 | 27.6 | 435 / 446 |
|  | fields + Zstd 19, per clip | 645.9 (−14.2 %) | 0 | 766 | 2618 | 83.7 / 89.5 | 41.1 | 657 / 663 |
|  |  |  | 8 | 332 | 1156 | 63.3 / 72.8 | 31.8 | 501 / 511 |
|  | fields + LZ4 HC 12, per clip | 755.8 (−12.3 %) | 0 | 766 | 1152 | 37.3 / 43.9 | 18.5 | 289 / 296 |
|  |  |  | 8 | 332 | 546 | 28.6 / 40.3 | 14.4 | 223 / 235 |
| `zrdo:100:6` | Zstd 9 | 691.9 (−15.3 %) | 0 | 766 | 2160 | 70.2 / 75.7 | 34.3 | 542 / 548 |
|  |  |  | 8 | 332 | 922 | 52.4 / 63.5 | 26.3 | 407 / 418 |
|  | fields + Zstd 19, per clip | 656.2 (−12.8 %) | 0 | 766 | 2759 | 89.4 / 94.5 | 44.4 | 691 / 696 |
|  |  |  | 8 | 332 | 1233 | 67.3 / 77.3 | 33.8 | 526 / 536 |
|  | fields + LZ4 HC 12, per clip | 764.3 (−11.3 %) | 0 | 766 | 1178 | 37.7 / 44.5 | 18.6 | 295 / 302 |
|  |  |  | 8 | 332 | 557 | 28.8 / 40.8 | 14.5 | 225 / 237 |

Page decode ms p50 / p95, Classic, 4 workers busy (decompress and inverse, whole page):

| Pages | Method | ≤ 96 px | 97–192 px | 193–384 px | > 384 px |
|---|---|---:|---:|---:|---:|
| plain | Zstd 9 | 2.70 / 4.70 | 2.76 / 4.06 | 2.59 / 3.71 | 1.23 / 3.33 |
|  | fields + Zstd 19 | 6.18 / 8.58 | 6.41 / 7.71 | 5.80 / 7.35 | 2.81 / 6.45 |
|  | fields + LZ4 HC 12 | 4.63 / 6.46 | 4.38 / 5.84 | 3.80 / 5.59 | 1.96 / 4.57 |
| `zrdo:30:3` | Zstd 9 | 2.83 / 4.36 | 2.78 / 4.41 | 2.40 / 4.14 | 1.22 / 3.35 |
|  | fields + Zstd 19 | 6.26 / 9.36 | 6.43 / 8.05 | 5.81 / 8.27 | 3.02 / 7.17 |
|  | fields + LZ4 HC 12 | 4.67 / 5.65 | 4.34 / 5.07 | 3.84 / 4.82 | 1.75 / 4.38 |
| `coarse:2:2+zrdo:30:3` | Zstd 9 | 2.69 / 4.43 | 2.61 / 4.97 | 2.48 / 4.08 | 1.21 / 3.29 |
|  | fields + Zstd 19 | 6.18 / 8.25 | 6.19 / 7.53 | 5.66 / 7.06 | 2.76 / 6.55 |
|  | fields + LZ4 HC 12 | 4.62 / 6.44 | 4.35 / 5.50 | 3.82 / 4.69 | 1.69 / 4.42 |
| `zrdo:100:6` | Zstd 9 | 2.62 / 4.05 | 2.65 / 3.47 | 2.21 / 3.41 | 1.12 / 2.82 |
|  | fields + Zstd 19 | 6.47 / 10.47 | 6.32 / 8.94 | 5.77 / 8.77 | 2.81 / 7.03 |
|  | fields + LZ4 HC 12 | 4.70 / 6.38 | 4.45 / 5.97 | 3.94 / 5.77 | 2.11 / 5.19 |

Random crowds, Enhanced (median of 20; MiB and Δ against the plain encode under the same config):

| Pages | Config | MiB | Cache | Page decodes | Decode ms | First drawable, warm / cold | Mobile p50, warm | All preloaded, warm / cold |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| plain | Zstd 9 | 1798.3 | 0 | 821 | 2840 | 92.4 / 103.0 | 46.7 | 711 / 723 |
|  |  |  | 8 | 455 | 1478 | 74.4 / 91.2 | 37.6 | 585 / 603 |
|  | fields + Zstd 19, per clip | 1658.6 | 0 | 821 | 3270 | 104.7 / 114.4 | 52.2 | 818 / 829 |
|  |  |  | 8 | 455 | 1739 | 85.4 / 99.7 | 43.6 | 657 / 672 |
|  | fields + LZ4 HC 12, per clip | 1886.0 | 0 | 821 | 1176 | 38.1 / 49.7 | 19.6 | 295 / 307 |
|  |  |  | 8 | 455 | 682 | 31.3 / 48.8 | 15.8 | 246 / 265 |
| `zrdo:30:3` | Zstd 9 | 1708.8 (−5.0 %) | 0 | 821 | 2565 | 84.6 / 94.2 | 43.3 | 644 / 655 |
|  |  |  | 8 | 455 | 1351 | 68.1 / 84.0 | 35.0 | 529 / 547 |
|  | fields + Zstd 19, per clip | 1588.5 (−4.2 %) | 0 | 821 | 3034 | 99.1 / 109.2 | 51.0 | 761 / 771 |
|  |  |  | 8 | 455 | 1702 | 80.5 / 95.9 | 41.0 | 631 / 645 |
|  | fields + LZ4 HC 12, per clip | 1814.9 (−3.8 %) | 0 | 821 | 1196 | 38.9 / 49.5 | 20.1 | 300 / 312 |
|  |  |  | 8 | 455 | 699 | 31.8 / 48.8 | 16.4 | 249 / 266 |
| `coarse:2:2+zrdo:30:3` | Zstd 9 | 1584.2 (−11.9 %) | 0 | 821 | 2757 | 89.5 / 98.1 | 45.4 | 690 / 701 |
|  |  |  | 8 | 455 | 1428 | 72.5 / 86.1 | 36.6 | 560 / 576 |
|  | fields + Zstd 19, per clip | 1461.1 (−11.9 %) | 0 | 821 | 2994 | 98.2 / 107.8 | 49.2 | 752 / 760 |
|  |  |  | 8 | 455 | 1646 | 79.5 / 93.3 | 40.9 | 626 / 640 |
|  | fields + LZ4 HC 12, per clip | 1690.3 (−10.4 %) | 0 | 821 | 1240 | 40.4 / 50.4 | 20.4 | 311 / 322 |
|  |  |  | 8 | 455 | 714 | 33.0 / 48.7 | 16.5 | 258 / 275 |
| `zrdo:100:6` | Zstd 9 | 1592.1 (−11.5 %) | 0 | 821 | 2622 | 84.4 / 94.4 | 44.3 | 657 / 666 |
|  |  |  | 8 | 455 | 1363 | 69.6 / 84.4 | 35.7 | 532 / 548 |
|  | fields + Zstd 19, per clip | 1494.0 (−9.9 %) | 0 | 821 | 3026 | 97.6 / 106.1 | 50.9 | 759 / 769 |
|  |  |  | 8 | 455 | 1639 | 79.9 / 93.8 | 40.6 | 616 / 632 |
|  | fields + LZ4 HC 12, per clip | 1722.4 (−8.7 %) | 0 | 821 | 1434 | 46.3 / 57.5 | 23.6 | 360 / 371 |
|  |  |  | 8 | 455 | 824 | 37.7 / 54.0 | 19.7 | 294 / 312 |

Page decode ms p50 / p95, Enhanced, 4 workers busy (decompress and inverse, whole page):

| Pages | Method | ≤ 96 px | 97–192 px | 193–384 px | > 384 px |
|---|---|---:|---:|---:|---:|
| plain | Zstd 9 | 3.49 / 5.75 | 3.40 / 4.83 | 2.39 / 4.06 | 1.31 / 3.46 |
|  | fields + Zstd 19 | 7.28 / 10.30 | 7.11 / 8.46 | 5.82 / 7.90 | 3.11 / 7.28 |
|  | fields + LZ4 HC 12 | 4.86 / 5.58 | 4.43 / 5.13 | 3.79 / 4.61 | 1.77 / 4.11 |
| `zrdo:30:3` | Zstd 9 | 3.28 / 4.44 | 3.29 / 4.02 | 2.36 / 3.81 | 1.28 / 3.34 |
|  | fields + Zstd 19 | 6.94 / 9.00 | 6.96 / 8.55 | 5.74 / 7.94 | 3.01 / 7.07 |
|  | fields + LZ4 HC 12 | 4.90 / 5.96 | 4.50 / 5.88 | 3.88 / 4.85 | 1.78 / 4.19 |
| `coarse:2:2+zrdo:30:3` | Zstd 9 | 3.38 / 4.99 | 3.33 / 5.01 | 2.45 / 4.84 | 1.31 / 3.53 |
|  | fields + Zstd 19 | 6.98 / 9.06 | 6.82 / 9.12 | 5.71 / 8.08 | 3.07 / 7.19 |
|  | fields + LZ4 HC 12 | 4.97 / 6.29 | 4.60 / 5.94 | 3.96 / 5.56 | 1.81 / 4.54 |
| `zrdo:100:6` | Zstd 9 | 3.28 / 4.79 | 3.27 / 4.00 | 2.27 / 3.76 | 1.25 / 3.23 |
|  | fields + Zstd 19 | 6.97 / 8.69 | 6.86 / 8.20 | 5.71 / 7.87 | 3.06 / 6.74 |
|  | fields + LZ4 HC 12 | 5.50 / 7.21 | 5.18 / 6.63 | 4.22 / 6.05 | 2.30 / 5.36 |

**Decode does not get slower, with one exception:**
- Under Zstd 9 and `fields` + Zstd 19 the conditioned pages decode as fast as the plain ones or up to 10 % faster:
  more of each page is matches, less is literals for Zstd's entropy stage. The first drawable moves by −9 to +3 %
  (Classic) and −9 to −3 % (Enhanced).
- Under `fields` + LZ4 HC 12 they are within 6 %, except `zrdo:100:6` on Enhanced: 22 % slower (1434 against 1176 ms,
  first drawable 46 against 38 ms). A whole-block match is one short LZ4 sequence, and 15 % of the visible blocks
  became one. Its Classic run shows +4 %, so this is the one figure worth timing again before relying on it.
- These packages were packed again from this commit, so the plain figures differ from the
  [Crowds](#crowds-under-the-loader-order) tables (Enhanced: 821 page decodes against 804, first drawable 92 against
  83 ms); compare within these tables. Differences under about 5 % are run-to-run noise.

The trade between the configs is the one [Crowds](#crowds-under-the-loader-order) measured; conditioning adds its
disk saving to whichever is chosen and nothing to the load.

### Encode time

On one thread per page (20 pages at once, the times summed over the pages' threads):
- the plain encode: 0.26 s a Classic page, 0.17 s an Enhanced one (the sample pages; 0.06 s averaged over all
  Classic pages, most of which are small);
- `coarse` 0.15 s, `flat` 0.02 s, `reuse` 0.2 s (0.03 s at radius 0), `zrdo` / `lrdo` 0.6 s, `fields` / `lfields`
  0.8 s a page;
- the library RDO 5.3 s (look-back 64), 13 s (256), 56 s (1024), 137 s (4096) a page.

A full Classic run of three variants with every measure took 19 minutes on 20 threads; the steps alone add about a
minute (Classic) and two (Enhanced) to a repack, against the 25 and 51 minutes of the packer's RDO (λ 1).

### Clips

Side-by-side videos, nearest-upscaled, anchored, looped, in
`/home/claudio/rust-target/uodynamapper-scratch/bc7-lossy/clips/`:

Each video is `<body>-<action or movement>-<direction>_<encodes>.mp4`, panels left to right: the source frames, the
plain encode, then the variants (`<encodes>` counts the plain encode and the variants).

- Classic, 4×, panels source | plain | `zrdo:30:3` | `coarse:2:2+zrdo:30:3` | `zrdo:100:6` | `rdo:1:64`:
  - `400-walk-2_5.mp4`, the human male walking;
  - `400-5-3_5.mp4`, his fidget (action 5; his stand clip is a single frame);
  - `17-walk-3_5.mp4`, an orc walking, a mid-size body.
- Enhanced, 3×, panels source | plain | `zrdo:30:3` | `coarse:2:2+zrdo:30:3` | `zrdo:100:6` (the library RDO is left
  out: it takes 5 s a page for each of these large pages and is dominated on every measure):
  - `12-walk-2_4.mp4`, a dragon walking, a large body;
  - `9-walk-2_4.mp4` and `9-stand-3_4.mp4`, a daemon walking and standing.
- The same six clips with `fields:30:3` alone, panels source | plain | `fields:30:3`, at the same scales: the files
  ending `_2.mp4`.

What to look at: banding on the large bodies' smooth shading under `coarse` and `zrdo:100:6`, and the shimmer of
`rdo:1:64` at the silhouette, which no other step moves.

### Visual comparison

In the clips above the candidates could not be told from the plain encode by eye, so the ladder here goes on until
the damage is plain. Every figure is a full pass over the whole package (every page, not a sample), stored as the field
split + Zstd 17 (codec 4, today's default); the saving counts the whole file (pages and the rest), from Classic
768.83 MiB and Enhanced 1719.96 MiB plain. The in-game steps were written as real packages and their sizes match.

| Step | Classic | Enhanced | Δ PSNR dB C / E | Δ dB > 384 px C / E | Flicker C / E |
|---|---:|---:|---:|---:|---:|
| plain | 768.83 MiB | 1719.96 MiB | (33.21 / 31.37 dB) | | 4.15 / 4.80 |
| `fields:30:3` (mild) | −9.6 % | −6.6 % | −0.61 / −0.15 | −0.86 / −0.40 | 4.32 / 4.81 |
| `zrdo:100:6` | −12.6 % | −9.7 % | −1.30 / −0.62 | −1.73 / −0.81 | 5.77 / 5.64 |
| `coarse:2:2+zrdo:30:3` (strong) | −13.9 % | −11.7 % | −0.71 / −0.64 | −0.82 / −0.72 | 4.80 / 5.30 |
| `coarse:2:4+zrdo:100:6` | −21.4 % | −17.7 % | −1.98 / −1.63 | −2.34 / −1.68 | 6.24 / 6.41 |
| `coarse:3:8+zrdo:100:8` | −29.5 % | −23.6 % | −3.76 / −3.05 | −4.36 / −3.26 | 7.93 / 7.79 |
| `coarse:4:8+zrdo:100:12` | −35.2 % | −25.6 % | −5.09 / −3.41 | −6.23 / −4.38 | 9.61 / 8.52 |
| `coarse:4:16+zrdo:300:16` | −46.8 % | −34.2 % | −8.11 / −6.24 | −9.20 / −7.17 | 13.34 / 11.78 |
| `coarse:5:32+zrdo:1000:32` | −64.5 % | −47.6 % | −13.10 / −11.13 | −14.23 / −12.34 | 22.03 / 19.12 |

Where each step starts to show, as I read stills taken from the videos below (my observations, not a user verdict):

- **`fields:30:3`, `zrdo:100:6`, `coarse:2:2+zrdo:30:3`:** nothing I can find in the picture at 1x, 2x or 4x on any
  body, in either client; they show only in the amplified difference rows (`zrdo:100:6`'s is the brightest of the
  three on the Classic dragon: scattered whole blocks taken from elsewhere).
- **`coarse:2:4+zrdo:100:6`:** at 4x a faint 4×4 block grid on smooth areas (the Classic dragon's and daemon's wing
  membranes) and a slight green tint on the Enhanced human's half-transparent loincloth; nothing at 1x.
- **`coarse:3:8+zrdo:100:8`:** the first step I would call visible on a close look: 4×4 tiles on the wing membranes
  and bellies of both clients' dragon and daemon at 2x and 4x, a few off-hue specks (yellow-green on the Enhanced
  daemon's wing edge). At normal zoom in the game I cannot pick it out from the plain encode.
- **`coarse:4:8+zrdo:100:12`:** visible at 1x on the large bodies: green and pink block patches on the Classic dragon,
  a tiled belly on the Enhanced dragon, a pink tint on the Enhanced human's leg.
- **`coarse:4:16+zrdo:300:16`:** obvious in the game at normal zoom: blotchy colour blocks over the dragon and daemon,
  shading gone to tiles, pale patches on the Enhanced dragon's back.
- **`coarse:5:32+zrdo:1000:32`:** broken: a posterised palette, flat grey and olive blocks across the Classic dragon
  and daemon, the Enhanced bodies a mosaic. Silhouettes stay exact at every step.

The small bodies hide it longest: on the Classic human the picture changes only from `coarse:4:16` (a yellow patch on
the hip at 4x). The ranking by eye follows the ladder except `zrdo:100:6`, which sits with the two milder steps despite
its larger PSNR loss.

The videos, in `/home/claudio/rust-target/uodynamapper-scratch/bc7-lossy-visual/clips/`, the Enhanced dragon and
daemon first:

- `NN-<client>-<body>-<name>-blink.mp4`, the primary evidence: held frames, every panel flipping every 0.5 s
  between the plain encode and one step (the first panel between the source and the plain encode, a blue or orange
  mark saying which shows), at 2x and then 4x (or the largest scale that shows the whole body).
- `NN-<client>-<body>-<name>-ladder.mp4`: the source, the plain encode and every step side by side, playing the walk
  and an idle clip at 4x and 2x with |version − source| × 8 per channel under each panel (black is exact), then at
  1x. Large bodies are cropped to the panel at 2x and 4x (the title gives the crop).
- In order: 01 Enhanced dragon (12), 02 Enhanced daemon (9), 03 Classic dragon, 04 Classic daemon, 05/06 lich (24),
  07/08 human (400), 09/10 dog (217), Enhanced before Classic. Classic idles are actions 17 (monsters), 5 (human)
  and 10 (dog); the Enhanced ones `stand`.
- `ingame/<style>-<body>.mp4`: the game at normal zoom (1x, renderer defaults, Classic style with Classic art and
  Enhanced style with Enhanced art, map 1 at 2111,790), the body walking east and standing, recorded once per
  conditioned package and shown six up: plain, `fields:30:3`, `coarse:2:2+zrdo:30:3`, `coarse:3:8+zrdo:100:8`,
  `coarse:4:16+zrdo:300:16`, `coarse:5:32+zrdo:1000:32`. The runs are separate, so the panels drift apart by a few
  frames.

Every file is under 29 MiB: H.264 4:4:4 at CRF 8, raised (shown in the encoder log, 12 to 16 for the large bodies'
ladders, 18 to 21 for the in-game videos) until it fits.

To reproduce (`$B` as below; the in-game recordings used scratch scripts: a private headless KWin, the renderer
control socket, isolated settings and `x11grab`):

```sh
$B lossy cc_rgba.uddp cc_bc7.uddp cc --methods fields+zstd:17 --variants fields:30:3,coarse:3:8+zrdo:100:8 \
    --store store/cc                                   # full pass, every page stored
$B condition cc_bc7.uddp cc store/cc/coarse_3_8_zrdo_100_8 mobile_anim_cc.uddp   # same codec, rest copied
udd-cli tool validate mobile_anim_cc.uddp
$B lossy cc_rgba.uddp cc_bc7.uddp cc --every 100000 --methods fields+zstd:17 --variants <the ladder> \
    --clip 12:walk:2 --clip 12:17:2 --clip 9:walk:2 --clip 9:17:2 --ladder-dir clips --ladder-prefix classic \
    --note "fields:30:3=package −9.6 %" --body-name 12=dragon --body-name 9=daemon
```

`condition` rewriting the plain encode's pages reproduces the package byte for byte.

### Verdict

Every candidate below keeps silhouettes and alpha exact and decodes as fast as the plain encode (one exception
under LZ4 HC, in 3); they differ in size and visible error. Figures are full packages under `fields` + Zstd 19
per clip (codec 4), Classic / Enhanced.

1. **Mild: `fields:30:3` (or `zrdo:30:3` for a non-split codec).** −9.9 / −6.9 % for −0.61 / −0.15 dB and −0.8 /
   −0.2 SSIMULACRA2; `zrdo:30:3` gives −4.7 / −4.2 % there for −0.21 / −0.10 dB. Under Zstd 9 or LZ4 HC without the
   split, `zrdo:30:3` is the one (−5.7 / −5.0 % at Zstd 9), about what the packer's RDO saves today at a tenth of its
   error. Both cost 0.6 to 0.8 s a page. The least likely to be seen; worth a look on the large Enhanced bodies,
   where `fields:30:3` loses 2.2 points.
2. **Strong: `coarse:2:2+zrdo:30:3`.** −14.2 / −11.9 % (−106 / −197 MiB) for −0.71 / −0.64 dB, but −2.2 / −2.9
   SSIMULACRA2 (−3.3 to −4.3 on bodies above 192 px): endpoint steps on smooth shading. Needs the visual decision.
3. **Alternative: `zrdo:100:6`.** −12.8 / −9.9 % for −1.30 / −0.62 dB. Its package SSIMULACRA2 is better than the
   strong chain's (−1.1 / −0.7), but on bodies above 192 px it is worse (−3.8 to −7.1) and its flicker the highest
   of the three; under `fields` + LZ4 HC 12 its Enhanced crowd decoded 22 % slower. At most the strong chain's size
   (smaller only on Classic under Zstd 9) and worse where it shows; kept for the visual comparison only.

Not recommended:
- **The library RDO** (any λ or look-back): dominated by `zrdo` at every size, changes silhouettes, and a long
  look-back costs minutes a page.
- **An LZ4-priced RDO:** within a fraction of a point of the Zstd-priced steps under LZ4 HC 12 and worse under the
  field split. The error bound, not the coder model, sets the trade.
- **Temporal reuse on an aligned layout:** −2.2 / −2.5 % under codec 4 for 0.05 dB, almost all from idle clips,
  against a packer layout change and 7.4 % more Classic pages. On top of `coarse:2:2` it adds 1.4 / 2.0 points.
- **Flat blocks:** rare (−2.6 to −3.1 % within 4 levels).

Which deserve a visual decision: the mild `fields:30:3` / `zrdo:30:3` (whether anyone can tell), the strong
`coarse:2:2+zrdo:30:3`, and `zrdo:100:6` against it. Any of them would be a packer step after the encode, before the
page codec, with no format or reader change.

To reproduce:

```sh
B=/home/claudio/rust-target/uodynamapper/release/udd-bc7-supercompress
udd-cli pack pack-mobile-anims --ccdir "<Classic>" --output cc_bc7.uddp --bc-encode-device cpu
udd-cli pack pack-mobile-anims --ccdir "<Classic>" --output cc_rgba.uddp --bc-encode-device cpu --raw --zstd=3
# and pack-ec-mobile-anims --ecdir "<Enhanced>" for ec_bc7 and ec_rgba
$B lossy cc_rgba.uddp cc_bc7.uddp cc --every 60 --check --ssim-every 50 \
    --variants rdo:1:64,zrdo:30:3,fields:30:3,coarse:2:2,coarse:2:2+zrdo:30:3,flat:4   # and the others
$B lossy cc_rgba.uddp cc_bc7.uddp cc --layout aligned --every 60 --ssim-every 50 --variants reuse:0,reuse:2,reuse:4
$B lossy cc_rgba.uddp cc_bc7.uddp cc --ssim-every 50 --variants zrdo:30:3,coarse:2:2+zrdo:30:3 --store store/cc
$B crowd cc_bc7.uddp cc --bounds 96,192,384 --lru 0,8 --mix random --pages store/cc/zrdo_30_3 \
    --config today=none+zstd:9 --config f19c=fields+zstd:19@clip --config f4c=fields+lz4hc:12@clip
$B lossy cc_rgba.uddp cc_bc7.uddp cc --every 100000 --variants zrdo:30:3,coarse:2:2+zrdo:30:3,zrdo:100:6,rdo:1:64 \
    --methods none+zstd:9 --clip 400:walk:2 --clip 400:5:3 --clip 17:walk:3 --clip-dir clips
$B lossy ec_rgba.uddp ec_bc7.uddp ec --every 100000 --variants zrdo:30:3,coarse:2:2+zrdo:30:3,zrdo:100:6 \
    --methods none+zstd:9 --clip 12:walk:2 --clip 9:walk:2 --clip 9:stand:3 --clip-scale 3 --clip-dir clips
```
