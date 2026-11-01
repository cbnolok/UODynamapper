# Data-track open items: proposals (2026-11-01)

The user's decisions and what came of them are in [Decisions](#decisions-2026-11-01) at the end;
the proposals below are as written. Four `docs/TODO.md` items needed a decision from the user:

- (a) cluster tiny art records;
- (b) the 1:1 tiled world export with WebP profiles;
- (c) the converter side of the terrain metadata palettes;
- (d) the open BC7 RDO item.

Measurements were taken on the installed packages in `/home/claudio/test/uddp` unless a line
says otherwise. The candidate packs used for the RDO numbers are in
`/home/claudio/test/uddp-candidates/rdo-measure-20261101/`. The probes were scratch Python
scripts (Python 3.14 `compression.zstd`, the same libzstd at the packer's level 7) and a
scratch crate. No repository code was added.

## Summary

| Item | Gain measured | Runtime change | Recommendation |
| --- | --- | --- | --- |
| (a) cluster tiny records | no tiny art records exist any more; map0 + statics0 together −4.2 MB of 26.8 MB (−16 %) | new grouped-record reader | don't cluster; optionally give statics a zstd dictionary (−0.76 MB, no runtime change) |
| (b) 1:1 tiled world export | new feature | offscreen capture loop in the renderer | do it only with a stated use case; phase 1 is PNG through the control channel, then libwebp |
| (c) terrain metadata palettes, converter side | 4 B → 2 B per tile: −64 MiB VRAM for Britannia (8 pages) | land shader indirection on 63 metadata reads | don't do the converter side; defer the whole item until VRAM telemetry asks for it |
| (d) BC7 RDO | default λ 0.05: art and land packages get **larger** (+0.4 % to +1.8 %); EC mobiles get 1.1 % smaller in 19.5× the pack time (1580 s against 81 s) | none | drop the "continue RDO reduction" item; keep RDO opt-in or remove it |

## (a) Cluster tiny art records into 32..96 KB payload groups

### What it means today

The item predates atlas paging. Static art is no longer stored as one record per art id:

- `tex_art_cc.uddp` is 46 RGBA atlas pages (16.4 MB raw each), plus 2 metadata files;
- `tex_art_ec.uddp` is 9 pages plus 3 metadata files.

No art package has a tiny record.

The packages that do have many small records are:

| Package | Entries | Raw size histogram | Stored now |
| --- | --- | --- | --- |
| `statics0.uddp` | 28,672 | 18,493 empty; 968 under 256 B; 1,709 at 256 B..1 KB; 6,375 at 1..4 KB; 1,118 at 4..16 KB; 9 at 16..32 KB | 8.00 MB payload + 459 KB index (`ZstdNoDict`, no dictionary) |
| `map0.uddp` | 28,672 + 1 | every chunk is 4,096 B (32×32 tiles × `Rg16u`) | 17.89 MB payload + 459 KB index (`ZstdTypeDict`, 16 KB dictionary) |
| `effects_ec.uddp` | 683 | 440 under 16 KB | 8.04 MB |

The current per-entry overhead is:

- **Index.** Each entry costs 16 B (dense) or 24 B (path). That is 459 KB in statics0, or 5.4 % of
  the package, and 296 KB of it is spent on the 18,493 empty blocks.
- **Frame.** An empty zstd frame is 9 B.
- **Lost context.** Each record is compressed without its neighbours as context. statics0
  without a dictionary is 7.9 % larger compressed record by record than in 32 KB groups. With a
  trained dictionary the gap drops to 2.9 %.

### Measured options (facet 0)

| Option | statics0 total | map0 total |
| --- | --- | --- |
| now | 8.46 MB | 18.35 MB |
| per record, freshly trained 16 KB dictionary | 7.70 MB (−9.0 %) | 17.39 MB (−5.2 %) |
| 32 KB groups + dictionary + 4 B per-record sub-index | 7.16 MB (−15.4 %) | 15.41 MB (−16.0 %) |
| 64 KB groups + dictionary | 7.24 MB | 15.46 MB |
| 96 KB groups + dictionary | 7.30 MB | 15.53 MB |

Notes on these numbers:

- **The shipped map0 dictionary is weak.** Without any dictionary, map0 would be 17.30 MB of
  payload against the 17.89 MB shipped. map0 was packed on 2026-06-30, so a repack with the
  current packer may already recover the 5 %.
- **Group size.** 32 KB groups beat 64 and 96 KB, because the dictionary already supplies the
  shared context.

Decode costs:

| Unit | Decode time |
| --- | --- |
| one record now | 5..6 µs |
| one 32 KB group | 22 µs (map) / 37 µs (statics) |
| one 96 KB group | 59 µs / 96 µs |

### Gain

Clustering saves about 4.2 MB per facet across map and statics. The installed set is 8.1 GB, of
which 6.9 GB is `mobile_anim_ec.uddp`, so this is 0.05 % of it. The map/statics load path is not
I/O bound today.

### Options

1. **Don't cluster** (recommended).
2. **Statics dictionary only.**
   - Change `cc_statics.rs` from `CompressionFlag::ZstdNoDict` to `Auto` or `ZstdDict`. The
     container, reader and validator already handle `ZstdTypeDict`.
   - Saves 0.76 MB per facet, with no runtime change.
   - Effort: about an hour plus a repack and `tool validate`.
3. **Grouped records.**
   - Needs a new grouped payload layout: a group index plus a per-record offset sub-index.
   - The reader must decode a group and cache it; `decoded_entry_cache` exists but is keyed per
     locator.
   - The map/statics loaders in dynamapper and `uddp-inspector-gui` must use the grouped reader,
     and `tool validate` and `extract` must learn the layout.
   - Effort: 2..3 days including the tests.
   - The runtime changes are reader-side only, but every random block load decodes 8..24
     records.

**Recommendation:** don't do the clustering. Do option 2 if the user wants the 9 % on statics,
and repack map0 with the current packer. Then remove the TODO line, or reword it to "statics
dictionary". The neighbouring TODO lines "atlas packing with guillotiere" and "RGBA8 / BC7 page
modes" describe what the art packers already do.

## (b) 1:1 tiled world export pipeline with WebP output profiles

### What it means today

Nothing exists beyond the control channel's `capture`. That command writes one window-sized PNG
through Bevy's screenshot path, and only to an absolute `.png` path. `docs/dev_wiki/TECHNICAL_REFERENCE.md`
mentions full-map 1:1 offline export as the reason for the LOD meshes.

At the client scale of 44 px per tile, Britannia (7168×4096 tiles) has these sizes:

| View | Pixels |
| --- | --- |
| isometric, content | 29.4 M tiles × 968 px² ≈ 28 Gpx |
| isometric, bounding box | 248k × 248k ≈ 61 Gpx (the corners are empty) |
| top-down | 315k × 180k ≈ 57 Gpx |

The export has to be tiled: WebP is limited to 16,383 px per side. 256 px tiles give about
430k non-empty tiles at the full level, plus about 33 % for the overview pyramid.

### Measured encoder costs

The sample is six 1024×512 crops of renderer captures (Classic, Enhanced and KR presets). The
zoom levels are mixed, so treat the numbers as ranges.

| Encoder | Bytes per pixel (6 crops) | Time per Mpx (one thread) |
| --- | --- | --- |
| PNG (the capture) | 0.83..2.30 | - |
| libwebp lossless, method 4 (ImageMagick 7 / libwebp 1.6.0) | 0.17..1.88 | ~0.75 s |
| libwebp lossless, method 6 | 0.17..1.87 | ~1.0 s |
| libwebp lossy q90 | 0.16..0.53 | ~0.2 s |
| libwebp lossy q75 | 0.06..0.32 | ~0.15 s |
| `image` 0.25 + `image-webp` 0.2.4 lossless (pure Rust; scratch probe, round trip checked) | 1.15..2.24 | ~8 ms |
| libjxl lossless, effort 7 (already a dependency) | about libwebp lossless (one crop) | - |

The libwebp times include ImageMagick start-up.

The pure-Rust lossless encoder is very fast. Its files are 1.2× to 7.6× larger than libwebp
lossless, and often larger than the PNG.

For the 28 Gpx isometric world, these rates give:

| Output | Size | Encode time |
| --- | --- | --- |
| lossless | ≈ 5..50 GB | ≈ 6 CPU-hours (method 4) |
| lossy q90 | ≈ 5..15 GB | ≈ 1.5 CPU-hours |

The pyramid and the render/readback time come on top.

### Dependencies

| Option | Lossless | Lossy | Build cost | Notes |
| --- | --- | --- | --- | --- |
| `image` feature `webp` (`image-webp`) | yes | **no** | pure Rust, small | 1.2..7.6× larger lossless files than libwebp |
| `webp` crate (libwebp-sys) | yes | yes | C build via `cc` (bundled libwebp) or the system libwebp | full quality, method, `exact` and alpha controls |
| PNG through `image` (already a dependency) | yes | - | none | baseline; larger than libwebp lossless |
| JPEG XL through `jpegxl-rs` (already a dependency) | yes | yes | none new | browsers other than Safari mostly don't decode it, so it doesn't suit a web map |

I know of no mature pure-Rust lossy WebP encoder.

### Runtime changes (renderer session)

The renderer would need:

- a fixed-scale camera, with presets at 1:1;
- deterministic time: water scroll, lights, time of day and animations frozen;
- an offscreen render target of fixed size with GPU readback (not the window);
- a tile walker that waits for land, static and texture residency per tile, using the control
  channel's settle rule;
- overlap and crop, so that sprites taller than a tile are not cut;
- resume support for a run that lasts hours.

The tooling side is the tile writer (z/x/y layout), encoder profiles, the pyramid downsampler, a
manifest, and optionally a small Leaflet/OpenLayers viewer page.

### Effort

| Phase | Work | Effort |
| --- | --- | --- |
| 1 | the control channel drives teleport + capture over a grid; PNG tiles; an offline WebP pass | about 1 day, no new dependency |
| 2 | in-app offscreen capture and readback, plus deterministic time | 3..5 days, renderer session |
| 3 | the `webp` crate encoder profiles (lossless/lossy, quality, method), pyramid and manifest | 1..2 days |

**Recommendation:** start only once the user names the use case: a web map, archival, editor
reference, or radar replacement. The use case decides the projection (isometric client view or
top-down), the presets, the lossless/lossy choice and whether statics and lights are drawn. Do
phase 1 first. Take libwebp through the `webp` crate for the profiles. The pure-Rust encoder is
lossless only and gives files no smaller than PNG, so it isn't worth a dependency.

## (c) Terrain Metadata VRAM Optimization: converter-side palettes

### What it means today

The land metadata atlas is `Rg16Uint`, 4 B per tile:

- `r` holds the tile id;
- `g` holds Z+128, the texture source mode, the land flags and the wet bit.

A page is 2048² tiles (16 MiB). Britannia needs 8 pages (128 MiB VRAM), and the CPU mirror holds
the same 4 B per tile. The map package stores the same `Rg16u` layout, and the runtime fills
`g`'s high byte from the tile id and the preset.

Palette cardinality, measured on `map0.uddp` per 2048² page:

| Page | Unique ids | Unique (id, Z) | Top 256 ids cover |
| --- | --- | --- | --- |
| (0,0) | 552 | 7,363 | 99.86 % |
| (0,1) | 561 | 5,055 | 99.88 % |
| (1,0) | 561 | 5,203 | 99.86 % |
| (1,1) | 460 | 6,589 | 99.95 % |
| (2,0) | 617 | 10,147 | 99.79 % |
| (2,1) | 859 | 21,143 | 99.02 % |
| (3,0) | 613 | 8,360 | 99.82 % |
| (3,1) | 318 | 1,817 | 99.99 % |

The whole map has 1,496 ids and 38,770 (id, Z) pairs.

The local cardinality is:

| Area | Unique ids (median / p90 / p99 / max) |
| --- | --- |
| 32×32 chunk | 4 / 40 / 77 / 117 |
| 8×8 block | 4 / 12 / 23 / 41 |

So:

- An 8-bit per-page id index does not fit: every page has 318..859 ids.
- Z needs its own 8 bits to stay lossless.
- The smallest lossless layouts are 2 B per tile.

### Options

| Option | Texel | Palette | Lookup cost |
| --- | --- | --- | --- |
| C1: per-page (id, Z) palette | `R16Uint` index | ≤ 21,143 entries × 4 B per page (85 KB) | every metadata and height read becomes two dependent reads |
| C2: per-chunk id palette + direct Z | `Rg8Uint` (index, Z) | ≤ 117 entries per 32×32 chunk, plus a chunk → palette offset table | height reads stay direct; id reads need two dependent reads |

Neither option goes below 2 B per tile. The gain is the same for both:

- −64 MiB VRAM and −64 MiB CPU mirror for Britannia;
- half the metadata upload bytes after teleports and zoom-outs.

The cost is in the land shader, which has 63 `atlas_read_meta`/`atlas_read_height` call sites
(neighbour reads for transitions, coverage, liquid banks and normals). Each gains a dependent
fetch. That change belongs to the deferred renderer performance work, which is to be measured
with RenderDoc/rocprofv3.

Converter side:

- Precomputed palettes would tie the map package to the runtime page size
  (`TILE_ATLAS_TILES_PER_PAGE`).
- The runtime fills pages chunk by chunk. An append-only palette built while filling is
  lossless and needs no converter change.
- Live terrain edits (deferred) would also append at runtime.

The converter side therefore adds coupling and buys nothing the runtime can't build in the same
pass.

### Effort

| Part | Effort |
| --- | --- |
| C1 or C2 in the runtime and shaders | 3..5 days, renderer session, including the preset visual checks |
| converter side | about 1 day, not needed |

**Recommendation:** don't do the converter side. Defer the whole item until frame and VRAM
telemetry on the target GPUs shows atlas memory or upload pressure. 128 MiB is small next to the up
to 160 MiB of RGBA land texture arrays. If it is done, C2 keeps height reads direct. Keep the
lossless rule in the Validation list either way.

## (d) The open RDO item

The open item is "Continue RDO algorithmic reduction after timings isolate it". RDO is opt-in
(`--bc7-rdo`, default λ 0.05). The mobile packers and `tex_aux_ec` default to λ 0, and no
installed package uses it.

Measured with the current release `udd-cli`, BC7 + Zstd:

| Package | Plain BC7 | RDO λ 0.05 (lookback 64) | Other RDO settings |
| --- | --- | --- | --- |
| `tex_land_cc` (5 pages) | 10.92 MB, 0.3 s | 11.12 MB (+1.8 %), 13.3 s | lookback 256: 11.07 MB, 50 s; λ 0.5: 10.77 MB (−1.4 %); λ 2.0: 9.42 MB (−13.8 %) |
| `tex_art_cc` (57 pages) | 38.71 MB, 1.2 s | 38.84 MB (+0.4 %), 22.6 s | |
| `mobile_anim_ec` (2,291 pages) | 1.988 GB, 81 s (BC7 encode 10.6 s) | 1.965 GB (−1.1 %), 1580 s (RDO stage 1478 s) | |

Quality, as PSNR of each RDO page against the plain BC7 page with the same layout (`tex_land_cc`):

| λ | PSNR |
| --- | --- |
| 0.05 | 43..53 dB |
| 0.5 | 40..48 dB |
| 2.0 | 33..41 dB (visible on terrain) |

So at the default λ, RDO costs quality and time and gives back almost no bytes: at best 23 MB on the 2 GB EC mobile package, for 25 more minutes of packing. It saves bytes only at a
λ that shows. The stage timing already isolates it: plain BC7 encode is 10.6 s of the 81 s EC
mobile pack, so the encoder is no longer the bottleneck. The larger stages are bucket selection
(9.6 s), page build/blit (17.5 s) and page registration (10.8 s).

Side finding: the installed `mobile_anim_ec.uddp` (6.9 GB, packed 2026-10-30) stores most BC7
pages uncompressed. The current packer with `--bc7 --zstd` writes 1.99 GB, because 73 % of the
atlas area is empty and zstd removes it. A repack would save about 4.9 GB on disk, at the cost of
one zstd decode per 4 MB page at load (milliseconds).
That pack passes `udd-cli tool validate`:
`/home/claudio/test/uddp-candidates/rdo-measure-20261101/mobile_anim_ececmob__bc7___zstd_.uddp`
(plain CC and EC clients, today's packer).

**Recommendation:** close the TODO item without further work. Then choose one:

- keep `--bc7-rdo` as a documented opt-in whose default does not pay off;
- remove RDO, its options and the `wide` dependency from the encoder path.

Repack `mobile_anim_ec` with `--zstd`; this is not RDO, but it is the real size win.

## Questions for the user

1. (a) Drop the clustering item? Do you want the statics dictionary (−9 %, no runtime change)
   and a map0 repack?
2. (b) What is the world export for: a web map, archival, editor reference or radar? Which
   projection (isometric client view or top-down), which presets, and with statics and lights?
   Lossless, lossy or both? Is a C build dependency (libwebp through the `webp` crate) acceptable?
3. (c) Agree to drop the converter-side palettes and defer the runtime part to the renderer
   performance session?
4. (d) Close the RDO item. Keep RDO as an opt-in, or remove it? Repack the installed
   `mobile_anim_ec.uddp` with `--zstd` (6.9 GB → 2.0 GB)?

## Decisions (2026-11-01)

### (a) Map and statics: retrieval speed, not size

The user: "We are talking about saving a small amount of MB, I'd keep what allows faster runtime
retrieval and rendering." The clustering item is dropped (removed from `docs/TODO.md`). Map and
statics packages were measured for runtime retrieval instead.

The benchmark is `lib/udd-conv/tests/integration/map_statics_retrieval.rs` (ignored; run it with
`--release` and `UDD_PACKAGE_DIR`). It repacks the installed `map0`/`statics0` entries with each
codec and reads them through `UddpReader` as dynamapper does: `load` (mmap), decoded cache off,
`read_file_by_dense_id`. It also reads them from `load_in_memory` and through
`read_file_by_dense_id_cow`.

The read orders are:

- id order;
- a shuffled order;
- a 24×24-chunk region at the map centre, as a view load would read it.

Each read order is timed two ways:

- **warm** is the fastest of 5 passes with the file in the page cache, per chunk;
- **cold** drops the file's pages first (`posix_fadvise(DONTNEED)`) and covers opening the package
  plus reading every chunk in the order once.

The numbers below are with the reader change described next.

**map0** (28,672 chunks of 4 KB):

| Codec | Size | Warm per chunk (mmap / cow) | Warm region | Cold id order (mmap / in-memory) | Cold shuffled (mmap / in-memory) | Cold region (mmap / in-memory) |
| --- | --- | --- | --- | --- | --- | --- |
| installed (zstd 7 + dictionary) | 18.36 MB | 3.92 / 3.90 us | 4.14 us | 116 / 121 ms | 149 / 129 ms | 5.4 / 9.0 ms |
| **stored** | 117.90 MB | **0.21 / 0.01 us** | **0.07 us** | **28 / 44 ms** | 850 / **46 ms** | **3.5** / 37 ms |
| zstd 1 | 20.58 MB | 4.44 us | 5.06 us | 131 ms | 160 / 153 ms | 4.7 / 8.5 ms |
| zstd 3 | 20.41 MB | 4.56 us | 5.06 us | 136 ms | 162 / 148 ms | 5.0 / 8.6 ms |
| zstd 7 | 17.76 MB | 3.39 us | 3.64 us | 102 ms | 127 / 114 ms | 3.9 / 7.1 ms |
| zstd 19 | 15.96 MB | 3.43 us | 3.35 us | 105 ms | 126 / 114 ms | 3.8 / 6.2 ms |
| zstd 7 + dictionary | 18.36 MB | 3.74 us | 3.99 us | 117 ms | 144 / 128 ms | 4.0 / 7.4 ms |

**statics0** (28,672 chunks; 18,834 empty):

| Codec | Size | Warm per chunk (mmap / cow) | Warm region | Cold id order (mmap / in-memory) | Cold shuffled (mmap / in-memory) | Cold region (mmap / in-memory) |
| --- | --- | --- | --- | --- | --- | --- |
| installed (zstd 7) | 8.46 MB | 1.31 / 1.31 us | 0.62 us | 39 / 41 ms | 51 / 43 ms | 1.5 / 3.3 ms |
| **stored** | 23.71 MB | **0.06 / 0.01 us** | **0.01 us** | **8 / 8 ms** | 25 / **8 ms** | 1.5 / 6.0 ms |
| zstd 1 | 8.82 MB | 1.30 us | 0.60 us | 40 ms | 49 / 42 ms | 1.3 / 2.7 ms |
| zstd 3 | 8.74 MB | 1.31 us | 0.62 us | 40 ms | 49 / 43 ms | 1.3 / 2.6 ms |
| zstd 7 | 8.46 MB | 1.31 us | 0.64 us | 42 ms | 50 / 44 ms | 1.3 / 2.6 ms |
| zstd 19 | 8.17 MB | 1.46 us | 0.72 us | 45 ms | 53 / 46 ms | 1.3 / 2.5 ms |
| zstd 7 + dictionary | 7.71 MB | 1.34 us | 0.60 us | 44 ms | 51 / 44 ms | 1.2 / 2.4 ms |

What the numbers say:

- **Stored retrieves fastest** wherever the data is in memory: 16..50× faster warm, and zero-copy
  through `_cow`. It is also fastest cold in id order and for a mapped region.
- The one loss is reading the whole map in a shuffled order from a cold mmap: 850 ms against
  127 ms, because 118 MB of page faults stand in for 18 MB. No view does that; a cold region is
  3.5 ms against 3.9 ms.
- The zstd levels differ little from each other. The dictionary does not make map chunks smaller,
  and it is 10 % slower than plain zstd 7.

**The reader fix.** Before this change the reader built a new Zstd context for every payload. For
the dictionary codec it also digested the dictionary again for every payload. That made the
installed map read at 7.90 us per chunk, against 3.45 us at plain zstd 7.

`udd-container` now keeps:

- one decompression context per thread (`support::zstd_decompress`);
- the type dictionaries, prepared once at open (`UddpReader::prepared_dicts`).

The installed map now reads at 3.9 us per chunk without a repack. Every Zstd package benefits.

**Decision:**

- The map and statics packers store chunks by default (`udd_conv::chunk_compression::ChunkCompression`).
- `--zstd[=LEVEL]` on `pack-map`, `pack-statics` and `pack-map-statics` now compresses chunks with
  plain Zstd. It used to be accepted and ignored.
- The dictionary path (`CompressionFlag::Auto` on map chunks) is gone from the map packer.
- `scripts/uddconv` passes `--zstd` to `pack-map-statics` only when it is asked. Its other
  non-texture packages keep the default `--zstd`.
- The size cost is about +100 MB per Britannia-sized facet for the map and +15 MB for statics.

The candidates are packed from the plain CC client by today's packer, without difs. Both pass
`udd-cli tool validate`. The sha256 values are in `SHA256SUMS` in the folder.

`/home/claudio/test/uddp-candidates/map-statics-stored-20261101/`:

- `stored/map0.uddp` (117,899,352 B) and `stored/statics0.uddp` (23,665,808 B): the new default;
- `zstd7/map0.uddp` (17,744,768 B) and `zstd7/statics0.uddp` (8,444,739 B): `--zstd`, for
  comparison.

The stored and the zstd 7 packages have the same payloads (`tool diff`: 0 changed). Against the
installed packages, which were packed on 2026-06-30, 40 map chunks and 53 statics chunks differ.
The zstd 7 build differs in the same chunks, so the codec is not the cause; the June packages
predate later packer or source changes. With `--include-map-difs --include-static-difs` the counts
are 40 and 183, so the June packages were not built with difs either.

Suggested for the renderer session: read map and statics chunks with
`read_file_by_dense_id_cow`. A stored chunk is then borrowed from the mapping, not copied.

### (b) World export

It is for web map tiles, as lossy WebP through libwebp. The user is answering the remaining design
questions; nothing is started. The TODO item records this.

### (c) Terrain metadata palettes

Deferred to the renderer performance work. The item moved to "Deferred" in `docs/TODO.md`, with the
cardinality numbers and a link here.

### (d) RDO

The item is closed. RDO stays an opt-in (`--bc7-rdo`), and the numbers are in `docs/TODO.md`.

The renderer is asked, in `agents_comm.txt`, to install the `--bc7 --zstd` EC mobile candidate
after a visual check. The candidate is
`/home/claudio/test/uddp-candidates/rdo-measure-20261101/mobile_anim_ececmob__bc7___zstd_.uddp`:

- size: 1,987,863,904 B;
- sha256: `b6472838bc9573281052fd6a93e4f6f201d6135da4d77c198229120db5ffdec5`;
- package hash64: `0x5d21415ff80f2810`.

It replaces the 6.9 GB installed package.
