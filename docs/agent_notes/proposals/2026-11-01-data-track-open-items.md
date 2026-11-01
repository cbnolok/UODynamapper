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

### (a) Map and statics: compressed, at the level that decodes fastest

The user's first answer: "We are talking about saving a small amount of MB, I'd keep what allows
faster runtime retrieval and rendering." The clustering item is dropped (removed from
`docs/TODO.md`), and map and statics packages were measured for runtime retrieval.

Stored chunks measured fastest, so 3a3c1621 made them the packers' default. The user then
corrected that: "100 MB per facet is a lot, compress that data." Since then, chunks are compressed
with plain Zstd by default, at a level per package chosen by decode speed. Stored is an opt-in.

**The benchmark** is `lib/udd-conv/tests/integration/map_statics_retrieval.rs` (ignored; run it
with `--release` and `UDD_PACKAGE_DIR`).

- It repacks the installed `map0`/`statics0` entries with each codec and reports the pack time.
- It reads them through `UddpReader` as dynamapper does: `load` (mmap), decoded cache off,
  `read_file_by_dense_id`. It also reads them from `load_in_memory` and through
  `read_file_by_dense_id_cow`.
- LZ4 (`lz4_flex`, a dev-dependency) is there for comparison only. Its payloads are stored LZ4
  blocks, decoded in the test; there is no container codec for it.

The read orders are:

- id order;
- a shuffled order;
- a 24×24-chunk region at the map centre, as a view load would read it.

Each read order is timed two ways:

- **warm** is the fastest of 5 passes with the file in the page cache, per chunk;
- **cold** drops the file's pages first (`posix_fadvise(DONTNEED)`) and covers opening the package
  plus reading every chunk in the order once.

The numbers below are with the reader fix described further down. Pack time is the builder's
(parallel) compression and write of the entries.

**map0** (28,672 chunks of 4 KB):

| Codec | Size | Pack | Warm per chunk (mmap / cow) | Warm region | Cold id order (mmap) | Cold shuffled (mmap / in-memory) | Cold region (mmap) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| installed (zstd 7 + dictionary) | 18.36 MB | - | 3.89 / 3.87 us | 3.98 us | 117 ms | 154 / 129 ms | 5.5 ms |
| stored | 117.90 MB | 0.06 s | 0.22 / 0.00 us | 0.07 us | 32 ms | 842 / 46 ms | 3.3 ms |
| LZ4 (comparison) | 58.42 MB | 0.14 s | 2.22 / 2.05 us | 2.29 us | 73 ms | 421 / 82 ms | 4.6 ms |
| zstd 1 | 20.58 MB | 0.08 s | 4.29 us | 4.83 us | 130..270 ms | 160..269 ms | 5.1..6.4 ms |
| zstd 3 | 20.41 MB | 0.06 s | 4.36 us | 4.84 us | 130 ms | 159 / 144 ms | 5.1 ms |
| zstd 7 | 17.76 MB | 0.29 s | 3.33 us | 3.58 us | 98 ms | 122 / 109 ms | 4.0 ms |
| zstd 9 | 17.79 MB | 0.46 s | 3.15 us | 3.18 us | 95 ms | 122 / 107 ms | 3.8 ms |
| zstd 12 | 17.11 MB | 0.81 s | 3.47 us | 3.57 us | 105 ms | 129 / 115 ms | 3.8 ms |
| zstd 15 | 16.74 MB | 1.28 s | 3.36 us | 3.33 us | 101 ms | 124 / 110 ms | 3.8 ms |
| **zstd 19 (the map default)** | **15.96 MB** | 2.49 s | 3.41 us | 3.34 us | 102 ms | 127 / 113 ms | 3.7 ms |
| zstd 22 | 15.96 MB | 2.54 s | 3.41 us | 3.35 us | 102 ms | 124 / 112 ms | 3.7 ms |
| zstd 7 + dictionary | 18.36 MB | 0.32 s | 3.68 us | 3.96 us | 115 ms | 138 / 124 ms | 4.2 ms |

**statics0** (28,672 chunks; 18,834 empty):

| Codec | Size | Pack | Warm per chunk (mmap / cow) | Warm region | Cold id order (mmap) | Cold shuffled (mmap / in-memory) | Cold region (mmap) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| installed (zstd 7) | 8.46 MB | - | 1.30 / 1.30 us | 0.62 us | 39 ms | 51 / 43 ms | 1.5 ms |
| stored | 23.71 MB | 0.01 s | 0.06 / 0.00 us | 0.01 us | 8 ms | 25 / 8 ms | 1.5 ms |
| LZ4 (comparison) | 16.99 MB | 0.04 s | 0.36 / 0.32 us | 0.17 us | 14 ms | 28 / 16 ms | 1.3 ms |
| zstd 1 | 8.82 MB | 0.02 s | 1.25 us | 0.60 us | 39 ms | 48 / 42 ms | 1.3 ms |
| zstd 3 | 8.74 MB | 0.02 s | 1.27 us | 0.60 us | 43 ms | 48 / 41 ms | 1.3 ms |
| **zstd 7 (the statics default)** | **8.46 MB** | 0.05 s | 1.35 us | 0.64 us | 42 ms | 53 / 43 ms | 1.3 ms |
| zstd 9 | 8.46 MB | 0.06 s | 1.35 us | 0.64 us | 43 ms | 50 / 44 ms | 1.3 ms |
| zstd 12 | 8.28 MB | 0.10 s | 1.42 us | 0.69 us | 43 ms | 52 / 45 ms | 1.4 ms |
| zstd 15 | 8.23 MB | 0.14 s | 1.46 us | 0.73 us | 44 ms | 53 / 47 ms | 1.5 ms |
| zstd 19 | 8.17 MB | 0.25 s | 1.45 us | 0.73 us | 44 ms | 52 / 46 ms | 1.3 ms |
| zstd 22 | 8.17 MB | 0.25 s | 1.46 us | 0.73 us | 44 ms | 53 / 46 ms | 1.3 ms |
| zstd 7 + dictionary | 7.71 MB | 0.16 s | 1.33 us | 0.60 us | 42 ms | 51 / 44 ms | 1.2 ms |

The zstd 1 map row ranges over two runs; one cold pass was an outlier.

**Choosing the level.** The rule: take the highest level whose decode is not slower, unless pack
time becomes unreasonable.

- **Map:** level 19. It is the smallest package (15.96 MB, −10 % against level 7). Its warm decode
  is level 7's (3.41 against 3.33 us), and over a view region it is faster (3.34 against 3.58 us).
  Level 22 gives the same bytes. Packing takes 2.5 s against 0.3 s for map0.
- **Statics:** level 7. Level 9 gives the same bytes at the same speed. From level 12 on, decode
  is 5..10 % slower for at most 3.4 % fewer bytes (8.17 MB at 19). This is also what the installed
  statics already use.
- **No dictionary:** on the map it is larger than plain zstd 7 and about 10 % slower. On statics
  it is 9 % smaller at the same speed, but that is 0.75 MB, and dropping it keeps one codec path.
- **Stored** stays an opt-in: `--stored`.

**LZ4, for comparison only.** It is not adopted and has no container codec.

| Package | LZ4 against the default | Warm per chunk | View region, cold |
| --- | --- | --- | --- |
| map | 3.7× the size (58.4 MB) | 1.5× faster (2.2 against 3.4 us) | slower (4.6 against 3.7 ms), from the extra bytes |
| statics | 2× the size (17.0 MB) | 3.7× faster (0.36 against 1.35 us) | same |

The absolute gain is small. A 576-chunk view region decodes in about 2 ms (map) and 0.4 ms
(statics) at the defaults, so LZ4 would save about 0.7 ms and 0.3 ms.

**The reader fix.** Before c4b94c4e the reader built a new Zstd context for every payload. For
the dictionary codec it also digested the dictionary again for every payload, so the installed map
read at 7.90 us per chunk. `udd-container` now keeps:

- one decompression context per thread (`support::zstd_decompress`);
- the type dictionaries, prepared once at open (`UddpReader::prepared_dicts`).

The installed map reads at 3.9 us per chunk without a repack. Every Zstd package benefits.

**What changed in the packers:**

- `udd_conv::chunk_compression::ChunkCompression` defaults to plain Zstd at the package's level:
  `MAP_CHUNK_ZSTD_LEVEL` 19 and `STATICS_CHUNK_ZSTD_LEVEL` 7.
- On `pack-map`, `pack-statics` and `pack-map-statics`, `--zstd=LEVEL` sets the level. A bare
  `--zstd` keeps the package's level, which is what `scripts/uddconv` passes by default; the
  scripts are back to how they were before 3a3c1621.
- `--stored` stores the chunks. It conflicts with `--zstd`.
- The dictionary path (`CompressionFlag::Auto` on map chunks) is gone from the map packer.

**Candidates.** `/home/claudio/test/uddp-candidates/map-statics-zstd-20261101/` holds a default
`pack-map-statics --map-id 0` build from the plain CC client, without difs. Both packages pass
`udd-cli tool validate`, and the sha256 values are in `SHA256SUMS`.

| Package | Size | sha256 | Note |
| --- | --- | --- | --- |
| `map0.uddp` | 15,943,908 B | `f87cf818...` | packing compressed for 2.5 s |
| `statics0.uddp` | 8,444,739 B | `38c29ee7...` | the same bytes as an explicit zstd 7 build |

Their payloads equal those of the stored build (`tool diff`: 0 changed). The stored candidates
have been trashed.

**Why 40 map and 53 statics chunks differ from the installed packages.** The June packages
(2026-06-30) were packed from the fp client, `_Ultima Online Classic fp`, like
`mobile_anim_cc`. A pack of map0/statics0 from the fp client with today's packer has 0 changed
payloads against them.

The plain client's `map0LegacyMUL.uop` is dated 2024-06-06 and its `statics0.mul` 2024-04-09. The
fp client's files are dated 2026-05-19 and 2026-05-12, so they carry newer map content:

- the 40 map chunks are in one block at x 6976..7135, y 2176..2431, in the dungeon area east of
  x 5120;
- 51 of the 53 statics chunks are in the same block (x 6976..7167, y 2144..2463);
- the other 2 are at x 1824..1887, y 2816..2847.

No packer change is involved.

Suggested for the renderer session: read map and statics chunks with
`read_file_by_dense_id_cow`. It only pays off for stored chunks, which are then borrowed from the
mapping instead of copied; for Zstd chunks it changes nothing.

### (b) World export

It is for web map tiles, as lossy WebP through libwebp. The design, with the JPEG XL measurements the user
asked for, is in the [web map export proposal](2026-11-01-web-map-export.md); it waits for the user's
answers there, and nothing is started. The TODO item records this.

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
