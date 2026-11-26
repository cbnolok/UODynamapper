# BC7 animation pages: lossless supercompression

Can the BC7 pages of the mobile animation packages be stored smaller without changing a single block, and what would
it cost the game to decode them (2026-11-25)? The brief proposed regrouping, dictionaries, byte shuffles, BC7-aware
field splits and block reuse in front of LZ4; the container's codec for these pages is Zstd, so everything here is
measured against Zstd 9 as the packages store it, and against LZ4 HC. Nothing in the production packers or the format
changed: `udd-bc7-supercompress` (`tools/udd-devtools-cli/src/bin/bc7_supercompress/`) only reads packages.

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
