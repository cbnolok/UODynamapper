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
E-byte elements, over tiles of N blocks), `modesort`, `fields`, `fields-shuf`. Backends: `zstd:L[:long]`,
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
  partner for the field split too.
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

## Not built (phases 6 to 8)

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
- **8, endpoint and weight prediction (XUBC7-style).** The field split is the first step of it: once each mode's
  endpoints sit together, predicting a block's endpoints from the previous block of the same mode and storing the
  residuals modulo the field width is reversible and cheap to undo. Its gain is unknown; it would be measured on the
  `fields` layout with Zstd. DCT residuals are lossy and out of scope.

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
  stored as separate streams ahead of Zstd, exactly reversible. It saves 4.0 to 4.4 % at level 9 and 8.0 to 8.8 % at
  level 19 (35 / 78 MiB and 72 / 158 MiB), still 2.3 to 7.7 % on RDO pages, but its whole-page inverse doubles the
  page's decode, and decode, not the disk, is what a crowd waits for.
- **If the size matters more than the decode** (a download), the evidence supports one versioned proposal, not built:
  a `udd_container::uddp::Codec` value (the next free one, 4) meaning "BC7 field split, then one Zstd frame", for
  mobile animation pages only, with the harness's layout (the mode map; each mode's field records; each mode's index
  records; reserved blocks; the tail) and its checks (map values, stream lengths, padding bits). Before proposing it
  for a default, the inverse should run per clip instead of per page: a page serves 21 (Classic) and 54 (Enhanced)
  clips at the median, so a clip needs a few percent of its blocks, and the mode map's running counts give each
  block's record offsets; joining only the clip's blocks would cost roughly 0.1 to 0.2 ms instead of 2.3 (an
  estimate, not measured), leaving the split stream's Zstd decode (+5 % at level 9) as the cost. That moves work
  into `stored_clip` and the reader, so it is a design for the user to decide.
- **The LZ4 HC question** is the opposite trade: the 20 % rule costs +9.4 / +10.0 % (77 / 197 MiB) for 1.5 to 1.7
  times the pages/s warm and about 1.3 times cold; LZ4 HC 12 everywhere +19 / +17 % for 2.4 times warm. Zstd 9 stays
  unless the user wants crowds to appear faster at that cost.
- **Not worth pursuing:** higher levels alone, long mode, grouping pages, chunking, dictionaries, byte shuffles,
  transposed field records, block reuse (all under [Negative results](#negative-results)); phases 6 and 8 only on
  top of the field split, if it is adopted.
