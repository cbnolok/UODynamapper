# Web map export: design proposal (2026-11-01)

Status: **approved 2026-11-01, tool built**; the renderer's export mode is the renderer session's work (see
[Decisions](#decisions-2026-11-01)). The questions the user answered are kept in
[Questions for the user](#questions-for-the-user).

The [renderer control pilot](../../reviews/2026-11-01-webmap-control-pilot.md)
is complete: placement is correct, but translated overlaps are not pixel-identical.
The differences reproduce on terrain alone and survive full settling; repeated
captures at the same location agree exactly. Export acceptance still needs stable
terrain sampling and the offscreen job implementation.

The user's decisions so far, from proposal (b) of the
[data-track proposals](2026-11-01-data-track-open-items.md#b-world-export):

- the output is web map tiles;
- lossy WebP through libwebp;
- isometric, as rendered, at client scale (44 px per tile);
- one preset: all statics, no lights (fixed daylight).

The JPEG XL question the user added is answered by measurement in
[JPEG XL, WebP and AVIF](#jpeg-xl-webp-and-avif-measured). In short, JPEG XL makes files 22–34 % smaller at
the same quality. Edge does not decode it yet, Chrome and Firefox have only decoded it since October 2026,
and it greys saturated dark colours. **WebP stays the serving format.**

## Decisions (2026-11-01)

The user approved every recommendation:

| Question | Decision |
| --- | --- |
| Format | Lossy WebP, **method 4, quality 85, sharp YUV**. **No JPEG XL set.** |
| Capture | An **offscreen export mode** in the renderer (`DYNAMAPPER_EXPORT_JOB`), after a **control-channel pilot**. |
| Facets | **Britannia first.** |
| Tile size | **512 px.** |
| Viewer | **Leaflet 1.9.4 vendored** into the output. |
| Coordinate readout | **z = 0** approximation in v1. |
| Location | A new crate, **`tools/webmap-export-cli`**. |
| Downsampling | **sRGB 2 × 2 box** average. |

What the implementation settled on top of the proposal:

- **TOML, not JSON**, for the job, the frame sidecars and the tool's state, read strictly (unknown or
  missing keys are errors, ENGINEERING.md). The schemas are in
  [Contract for the renderer session](#contract-for-the-renderer-session).
- `plan` writes `state/plan.toml` once and refuses an existing plan or a non-empty directory; `run`
  resumes from it. Other parameters mean a new output directory, so there is no `--restart`.
- Frame ids are `f{i:04}_{j:04}`, the frame's column and row in the 4096 px grid.
- `--lossless` is not built: no use for lossless WebP tiles came up.
- Britannia's tallest static rises 591 px, so `Z_TOP` = 1152 px (not 512). The box is 247,808 ×
  249,472 px, still 10 levels (0–9). 1,815 frames touch the facet; a frame is planned if the
  separating-axis test finds its kept area can show the ground diamond moved up by `Z_TOP` or down by
  the bottom margin.
- The `pilot` command is the control-channel pilot. Because the renderer session uses the running app,
  it is asked to run it (agents_comm.txt).

## Summary

| Topic | Proposal |
| --- | --- |
| Capture | A new offscreen **export mode** in the renderer, driven by a job file. It renders 4160 × 4160 frames to an offscreen target, keeps a 4096 × 4096 centre, and writes raw RGBA frames to a spool directory. The control channel is kept only for a prototype. |
| Geometry | UO projection at 44 px/tile (`px = (x − y + H)·22`, `py = (x + y)·22 − 4z + Z_TOP`). Frames are placed on a pixel grid with no seams. A 32 px crop margin covers screen-space effects. Chunk loading is driven by the frame rectangle. |
| Tiling | Leaflet `CRS.Simple`, `{z}/{x}/{y}.webp`, **512 px tiles**. The base level is the highest zoom (9 for Britannia). Lower zooms are downsampled from lossless data and never re-rendered. Two overzoom levels use pixelated scaling. |
| Tool | A new crate `tools/webmap-export-cli`. It plans frames, runs the renderer, slices the tiles, builds the pyramid, encodes, and writes the viewer. It is resumable, with frame-level done markers and atomic writes. Encoding is parallel with rayon. |
| Format | WebP, method 4, **quality 85 with sharp YUV**: mean SSIMULACRA2 74.9, minimum 65.7, 1.57 bits per pixel. A quality flag is exposed. JPEG XL is an optional second output, not the default. |
| Size and time (Britannia) | About 145k tiles and an estimated 6–9 GB. Encoding takes about 4 min on 20 threads. Capture is estimated at 15–60 min and is measured in phase 1. |
| libwebp | `libwebp-sys` 0.14.4 bundles libwebp 1.6.0 and builds it with `cc`, so no cmake and no bindgen. A clean build takes 4 s. Linux gnu and musl static builds pass here. Only the new crate depends on it, so no feature gate is needed. The hakari diff is expected to be empty. |
| Effort | Renderer export mode 2–4 days (renderer session), tool 2–3 days, pilot and seam tests 1 day. |

## Capture

### Control channel or export mode

| | Control channel (exists today) | Offscreen export mode (proposed) |
| --- | --- | --- |
| How | `teleport` → poll `status` until no metadata, art pages or uploads are pending → `capture` saves the primary window | The renderer reads a job, renders each frame into an offscreen `Image` target, waits until the frame is ready, and writes it |
| Frame size | Limited to the window, which the compositor and monitor can clamp | Any size up to the GPU limit (wgpu guarantees 8192); 4160² RGBA8 is 69 MB |
| Content | Includes whatever the window shows: the egui UI, cursor overlays (the dock crop below shows the terrain cursor's green dot), the player and fog | Only the export preset |
| Placement | The camera follows the player at the player's Z; teleport takes whole tiles, so sub-tile pixel alignment is indirect | Exact frame rectangle in base-level pixels |
| Throughput | One socket round trip per frame, a PNG encode in the renderer, and 30 s capture timeouts | Raw frames, pipelined with the tool |
| Unix only | Yes (Unix datagram socket) | No |

**Proposal:** the export mode. A prototype through the control channel is possible today for a
1–2 frame pilot, to measure render time and catch seams early. It is not the production path.

### Projection and placement

The renderer's orthographic camera reproduces UO's projection at zoom 1.0:

- `UO_TILE_PIXEL_SIZE` is 44.
- `ORTHO_SIZE_FACTOR` and `TILE_SIZE_FACTOR` are in `dynamapper/src/core/render/scene/camera.rs`.

The export uses UO's formula, in base-level pixels, for a facet of W × H tiles:

```
px = (x − y + H) · 22
py = (x + y) · 22 − 4·z + Z_TOP
```

- **Fixed Z.** The camera sits on the z = 0 plane. The projection is orthographic, so the camera's Z only
  shifts the picture.
  - Terrain and statics at height z are drawn 4·z px higher.
  - `Z_TOP` = 4 · 127 + the tallest static art height (read from the art package when the frames are
    planned), rounded up to 64 px.
  - The same margin goes at the bottom for z down to −128.
- **The bounding box.**
  - Britannia (map0, 7168 × 4096 tiles): 247,808 px of diamond, plus the Z margins, about 249k px square.
  - The diamond holds 28.4 Gpx of content: 29.4 M tiles × 968 px².
- **Frames.**
  - Frame (i, j) keeps base-level pixels `[i·K, (i+1)·K) × [j·K, (j+1)·K)` with **K = 4096**. That is 8 base
    tiles of 512 px, so one frame also yields whole tiles at levels 8, 7 and 6.
  - The frame is rendered with a margin **M = 32 px** on every side, so F = 4160, and the margin is cropped.
  - Coverage is 97 % efficient.
  - Britannia needs about 1,700 frames that touch the diamond, about 1,900 with the edge frames.
- **No seams by construction.** Every pixel's colour must depend only on the world, not on where the frame
  sits. That requires the following:
  - **Position-independent painter order.** The renderer's depth classes and bias already sort by world
    position.
  - **No screen-space post effects** inside the kept area. With FXAA or SMAA and the fog pass off, M = 0
    would suffice. The 32 px margin covers any filter that remains.
  - **The camera centre on an exact pixel**, so the rasterisation grid is identical in every frame.
  - **Chunk loading driven by the frame rectangle**, not by the window and player.
    - Today `sys_update_worldmap_chunks_to_render` derives the chunk set from the window size, the zoom
      and the camera chunk.
    - For export, the set must hold every chunk whose sprites can reach the frame. That is the frame's
      ground footprint, extended toward +x+y by `(Z_TOP + bottom margin) / 22` tiles, so tall or raised
      statics whose base lies outside the frame are still drawn.
- **Tall statics across tile edges.** Tall statics crossing a tile edge need no overlap logic at tile level.
  Tiles are cut from frames that already contain every sprite that touches them. Only the frame needs the
  extended chunk set above.
- **Facet edges.**
  - Outside the map the export draws opaque black: no wraparound, no void-tile texture.
  - Frames that lie entirely outside the diamond, plus the Z margins, are not rendered. Tiles that would be
    all black are not written (see [Empty tiles](#empty-tiles)).
- **Acceptance test.**
  - Render two frames that overlap by 512 px. Their overlap must be **bit-identical**.
  - Render the same frame twice, with another frame in between. The two must also be bit-identical.

### Contract for the renderer session

The renderer session would add an export mode with these properties. The tool depends only on this
contract.

1. **Activation.**
   - `DYNAMAPPER_EXPORT_JOB=<absolute path to job.toml>`, an opt-in like `DYNAMAPPER_CONTROL_SOCKET`.
   - No interactive state is touched.
   - Rendering must work without a visible or focused window. A hidden window or a windowless app is the
     renderer's choice.
2. **Job.** The tool writes `job.toml`; the renderer only reads it, strictly:

   ```toml
   format_version = 1
   facet = 0
   map_width_tiles = 7168
   map_height_tiles = 4096
   texture_source = "cc"          # or "ec"
   tile_half_width_px = 22        # the projection, repeated so the renderer need not hard-code it
   z_step_px = 4
   z_top_px = 1152
   frame_size_px = 4160           # F, margin included
   frame_margin_px = 32           # M, cropped by the tool
   spool_dir = "/abs/out/spool"
   max_spool_frames = 2

   [[frames]]                     # in render order
   id = "f0021_0000"
   left_px = 85984                # top-left of the rendered frame in base-level pixels; may be negative
   top_px = -32
   ```
3. **Preset**, fixed for the export:
   - every static drawn;
   - fixed daylight, static lights off, dynamic lights off;
   - fog, weather and post-process anti-aliasing off;
   - no player, mobiles, cursor, overlays or egui;
   - no roof or Z-cut hiding (none exists today; if one is added, export disables it);
   - animated materials (water, lava) frozen at one fixed time;
   - zoom exactly 1.0, orthographic.
4. **Placement.**
   - The base-level pixel convention above.
   - The renderer reports in the frame's sidecar one tile it drew and the base-level pixel where it put that
     tile's top vertex at z = 0; the tool checks it against its own projection.
5. **Readiness.** A frame is captured only when every chunk for its rectangle is spawned and every land
   page, art page and metadata upload it needs is complete. These are the same signals `status` exposes
   today (`metadata_pending`, `pending_pages`, the art collection stats).
6. **Output.**
   - `<spool>/<frame id>.rgba`: raw RGBA8, F × F, rows top to bottom.
   - `<spool>/<frame id>.toml`, the sidecar:

     ```toml
     format_version = 1
     id = "f0021_0000"
     left_px = 85984              # as in the job
     top_px = -32
     width_px = 4160
     height_px = 4160
     check_tile = [10, 20]        # a tile in the frame
     check_tile_px = [89892, 1812] # where its top vertex at z = 0 landed, in base-level pixels
     render_ms = 850
     packages = [{ name = "map0.uddp", hash64 = "0x0123456789abcdef" }]   # every package used
     ```

   - Both are written as `*.tmp` and renamed; the sidecar is renamed last, as the commit marker.
   - The tool reads the pixels, writes the frame's tiles, appends the id to `state/frames.done`, then deletes
     the `.rgba` and then the `.toml`. Every frame must list the same packages as the first.
7. **Back-pressure.** The renderer waits while the spool holds `max_spool_frames` (2) sidecars the tool has not consumed (the tool
   deletes consumed frames). This keeps memory and disk bounded at about 140 MB.
8. **Determinism.** The same rectangle gives the same bytes, whatever frame came before.
9. **Exit.** The renderer exits with status 0 after the last frame's sidecar is consumed or written; on an
   error it exits non-zero. `run` treats an exit with frames missing as an error, and a rerun resumes.

## Tiling scheme

- **Leaflet `CRS.Simple`, with a z/x/y pyramid.**
  - Tile URL: `tiles/{z}/{x}/{y}.webp`.
  - The base level is `N = ceil(log2(bbox / 512))`: 9 for Britannia (249k / 512 = 486 < 2⁹), and less for
    the small facets.
  - Leaflet pixel coordinates at zoom N are the base-level pixels. The viewer uses
    `map.unproject([px, py], N)` and sets the layer's `bounds`, so tile x and y are never negative.
- **Tile size: 512 px**, recommended. Leaflet supports `tileSize: 512`.
  - Britannia at 256 px would need about 578k files instead of 145k.
  - Each file also carries a fixed header, and the codec's block prediction works better on larger tiles.
  - The cost is a little more transfer when panning at the edge of the view.
- **Zoom levels.**
  - Levels N…N−3 (9…6) are cut and downsampled from each lossless frame: one frame gives 64 + 16 + 4 + 1
    tiles.
  - Level N−3 is also kept as a **lossless intermediate** (zstd raw RGB). That is 64 × 64 tiles for
    Britannia, about 1 GB compressed.
  - Levels N−4…0 are built from the intermediate after all frames are done.
  - No level is ever built from lossy tiles, so there is no generational loss.
  - Downsampling is a 2 × 2 box average in sRGB, the convention for map tiles. Gamma-correct averaging is
    an option for one line of code (see the questions).
  - Two **overzoom** levels (`maxZoom = N + 2`, `maxNativeZoom = N`) use
    `image-rendering: pixelated` on the tiles, so pixel art scales up crisply.
- **Isometric coordinates to tile x/y** (the cursor readout). The inverse at z = 0 is:

  ```
  u = px / 22 − H,   v = (py − Z_TOP) / 22
  x = floor((u + v) / 2),   y = floor((v − u) / 2)
  ```

  - The viewer shows `x, y` (and the facet) as the mouse moves.
  - On raised terrain the readout is off by about `z / 11` tiles. That is about 4 tiles at z = 40.
  - An exact readout needs heights in the viewer. One way: per-chunk height blocks fetched lazily, 1 KB
    each. See the questions.
- **Empty tiles.**
  - Tiles that are entirely outside the diamond, or entirely black, are not written.
  - The viewer sets `errorTileUrl` to one shared black tile and limits requests with `bounds`.
  - This saves about 120k files for Britannia's bounding box.

## The tool

- **Where it lives:** a new crate, `tools/webmap-export-cli`. Recommended over a `udd-cli` subcommand
  because:
  - it orchestrates another program (it spawns `dynamapper` with the job), while `udd-cli` works on
    packages only;
  - it keeps libwebp out of `udd-cli` and the GUIs without a feature gate.

  It reads the facet size and the tallest art height through `udd-container`.
- **Commands.**
  - `webmap-export-cli plan`: facet, output and tile options → `state/plan.toml`; `run` writes `job.toml`.
  - `webmap-export-cli run`: spawns the renderer with the job, consumes frames, writes the base levels.
  - `webmap-export-cli pyramid`: the low levels from the intermediate.
  - `webmap-export-cli viewer`: `index.html`.
  - `run` performs all four steps when given a fresh output directory.
- **Resumable.**
  - `state/frames.done` is append-only. A frame id is added only after all its tiles and its intermediate
    tile are written. Every write goes to a `.tmp` file first and is renamed.
  - A rerun gives the renderer only the frames not marked done.
  - The pyramid step is idempotent: it overwrites from the intermediate.
  - The parameters (quality, tile size, region) are stored in `state/plan.toml`, the package revisions in
    `state/packages.toml`. A new plan in the same directory is refused.
- **Parallelism.**
  - The renderer produces frames one at a time, since the GPU is the serial part.
  - The tool slices and encodes each frame's 85 tiles on a rayon pool while the next frame renders. A
    bounded queue of 2 frames keeps memory bounded.
  - The pyramid step encodes tiles in parallel too.
- **Encoder.**
  - A small safe wrapper of about 80 lines over `libwebp-sys`: `WebPConfig` with quality, method and
    `use_sharp_yuv`, ARGB import, and a memory writer.
  - The `webp` crate is not used. Its latest release, 0.3.1, still pins `libwebp-sys` 0.9 (libwebp 1.3.1).
  - Profiles: `--quality` (default 85), `--method` (default 4), `--sharp-yuv` (default on),
    `--lossless`.
  - This covers the TODO item "Add WebP output profile controls".
- **Viewer.**
  - `index.html`, a little JS for the CRS, coordinate readout and facet bounds, and `tiles/`.
  - **Leaflet 1.9.4 vendored** into the output (BSD-2-Clause, about 150 KB with its CSS), so the export is
    self-contained and works offline. Opened from `file://`, plain `<img>` tiles load.
  - The alternative is unpkg or cdnjs with an SRI hash: a smaller output that needs the network. See the
    questions.

### Size and time (Britannia, map0)

| | Estimate | Basis |
| --- | --- | --- |
| Base level | 28.4 Gpx → about 108k tiles of 512² | geometry above |
| All levels | about 145k tiles (+⅓) | pyramid |
| Disk, WebP q85 sharp YUV | **6–9 GB** | 1.57 bpp geometric mean over the 11 crops (0.21–3.87 bpp). Water and black make up much of the map and cost 0.2–0.6 bpp, so the base level is likely 4–6 GB. Downsampled levels are denser per pixel. |
| Disk, JXL d1.5 (if wanted) | about 22 % less than the line above | measurements below |
| Encoding | about 65 CPU-minutes → **about 4 min on 20 threads** | 27 ms per 512² tile, single thread, measured |
| Capture | 1,900 frames × 0.5–2 s = **15–60 min** | not measured yet; phase 1 replaces this with a number |
| Spool and intermediate | about 140 MB spool, about 1 GB intermediate | contract item 7, level 6 lossless |

The other facets are smaller. Using the same formula: Ilshenar 3.6 Gpx, Malas 5.1, Tokuno 2.0,
Ter Mur 5.1.

## libwebp build check

- **Crate.**
  - `libwebp-sys` 0.14.4 bundles **libwebp 1.6.0** (`ENC_*_VERSION` 1.6.0), the same as the system
    library.
  - It compiles with the `cc` crate from the vendored C sources, with a pre-generated `ffi.rs`. Bindgen
    and libclang are needed only under the opt-in `generate-bindings` feature, and cmake is not used.
  - Its build dependencies (`cc` with `parallel`, `glob`, `pkg-config`) are already in `Cargo.lock`, and
    `cc` with the `parallel` feature is already in `lib/workspace-hack`.
- **Checked here** (scratch crate `webp-build-check`, which prints `WebPGetEncoderVersion()`):

  | Target | Result |
  | --- | --- |
  | x86_64-unknown-linux-gnu | builds, runs, prints 1.6.0; libwebp linked statically; clean release build 4.0 s |
  | x86_64-unknown-linux-musl (clang `--target`) | builds a static-pie binary, runs, prints 1.6.0. CI's Alpine image uses gcc, which is cc's default there. |
  | x86_64-pc-windows-gnu | **not verifiable here**: no `x86_64-w64-mingw32-gcc` and no MinGW sysroot (clang stops on `string.h`) |
  | aarch64-unknown-linux-gnu | **not verifiable here**: no aarch64 sysroot (`gnu/stubs-32.h`) |

- **Reasoned for the rest of CI.**
  - The source is portable C (it is Chromium's and Android's WebP).
  - The build script handles MSVC (`is_like_msvc` → `-D_CRT_SECURE_NO_WARNINGS`) and aarch64 (NEON), and
    macOS uses the system clang through `cc`.
  - The x86_64 and ARM64 Windows MSVC builds, both macOS builds and Linux ARM64 should build without
    changes.
  - MinGW, if that CI job is ever re-enabled, is gcc with plain C. It does not have jpegxl-src's ClangCL
    problem.
  - The first CI run of the tool settles all of this.
- **Feature gate.**
  - Not needed. Only `tools/webmap-export-cli` depends on `libwebp-sys`, so no other tool pulls it in.
  - If the encoder is ever moved into a shared crate, it should go behind a `webp` feature there.
- **Hakari.**
  - With one consumer, `libwebp-sys` is not hoisted into the workspace hack, and its build dependencies are
    already unified.
  - The expected `cargo hakari generate --diff` is empty. It is checked when the crate is added, which this
    proposal does not do.
- **Compared with JPEG XL.**
  - libjxl is already in the workspace through `udd-container` (`jpegxl-rs` 0.14, `vendored-jpegxl` on
    CI). Its vendored build is a cmake C++ build of several minutes, and it is why the MinGW job is
    disabled.
  - A JXL output in the tool would add no new C dependency.
  - libwebp is a new but small one: about 4 s, plain C, no cmake.

## JPEG XL, WebP and AVIF (measured)

### Method

- **Crops.** 11 crops of 512 × 512, cut from the renderer captures in `/tmp/uodynamapper-ec-followup`:
  - `town_classic`: a dense Classic town with trees;
  - `forest_kr`: KR art, trees and buildings;
  - `snow_edge`, `snow_flat`;
  - `lava`;
  - `dungeon_black`: a Classic coast with a large black void area;
  - `water_coast`;
  - `dock_sprites`: barrels, an anchor and a 1-px rope over water;
  - `swamp`;
  - `bridge`;
  - `road_town`.

  There was no cave or night capture. The black-area crop stands in for cave black. Lights are off in the
  export preset anyway.
- **Probe.** A scratch crate in `/home/claudio/rust-target/uodynamapper-scratch/webmap-codecs` (single
  thread; the best of 3 runs for timing).
  - **WebP:** libwebp 1.6.0 through `libwebp-sys` 0.14.4, with ARGB import:
    - method 4;
    - method 6;
    - method 4 with sharp YUV (`use_sharp_yuv`).
    - Quality 40, 50, 60, 70, 75, 80, 85, 90, 95.
  - **JPEG XL:** `jpegxl-rs` 0.14, the workspace's crate, linked here to the system libjxl 0.12.0. CI
    vendors 0.11.x.
    - Distance 0.5, 0.75, 1, 1.5, 2, 2.5, 3, 4, 5.
    - Effort 3 (Falcon), 5 (Hare), 7 (Squirrel).
  - **AVIF:** `avifenc`/`avifdec` 1.4.2 (aom 3.13.3, dav1d 1.5.3), speed 6, one thread, q 40…95.
    - Its timings include starting the process and writing a PNG, so they are an upper bound and are not
      comparable to the in-process timings.
- **Metrics.**
  - **SSIMULACRA2**, from the pure-Rust `ssimulacra2` 0.5.1 crate. Its reference points are 90 for visually
    lossless, 70 for high quality and 50 for medium.
  - PSNR.
  - **Edge PSNR**: PSNR over the pixels whose Sobel luma gradient is above 96, which are the hard sprite
    edges.
  - Butteraugli and the `ssimulacra2`/`cjxl` binaries are not installed, so butteraugli is not reported.
- **Matching.** For each crop and codec, the bpp is interpolated at a target SSIMULACRA2. The results are
  geometric means over the crops where every codec reaches the target.

### Results

**Bits per pixel at matched SSIMULACRA2.** Each cell is the geometric mean, with the ratio to WebP
method 4 in brackets.

| SSIMULACRA2 (crops) | WebP m4 | WebP m6 | WebP m4 sharp YUV | JXL e3 | JXL e5 | JXL e7 | AVIF s6 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 50 (6) | 1.175 (1.00) | 1.079 (0.92) | 1.222 (1.04) | 0.844 (0.72) | 0.854 (0.73) | 0.828 (0.70) | 0.845 (0.72) |
| 60 (10) | 1.227 (1.00) | 1.149 (0.94) | 1.223 (1.00) | 0.841 (0.69) | 0.858 (0.70) | 0.850 (0.69) | 0.845 (0.69) |
| 70 (11) | 1.439 (1.00) | 1.385 (0.96) | 1.309 (0.91) | **0.955 (0.66)** | 0.984 (0.68) | 0.994 (0.69) | 0.986 (0.68) |
| 80 (6) | 1.409 (1.00) | 1.358 (0.96) | 1.464 (1.04) | **0.964 (0.68)** | 1.005 (0.71) | 1.014 (0.72) | 1.052 (0.75) |
| 85 (4) | 1.688 (1.00) | 1.643 (0.97) | 1.798 (1.07) | **1.156 (0.68)** | 1.170 (0.69) | 1.180 (0.70) | 1.258 (0.75) |

Against WebP with sharp YUV, the recommended WebP setting, JXL e3 is 27 % smaller at 70 and 34 % smaller at
80.

**The curve per setting.** Each row gives the mean and minimum SSIMULACRA2 over the 11 crops, the
geometric-mean bpp, and the encode time per 512² tile.

| Setting | SSIMULACRA2 mean / min | bpp | Encode ms |
| --- | --- | --- | --- |
| WebP m4 q75 | 57.5 / 44.3 | 0.863 | 16.5 |
| WebP m4 q85 | 70.3 / 60.4 | 1.412 | 19.5 |
| WebP m4 q90 | 76.2 / 65.2 | 1.974 | 20.5 |
| WebP m4 q95 | 81.2 / 71.2 | 2.900 | 22.8 |
| WebP m4 sharp YUV q80 | 68.1 / 55.8 | 1.209 | 25.3 |
| **WebP m4 sharp YUV q85** | **74.9 / 65.7** | **1.571** | **26.4** |
| WebP m4 sharp YUV q90 | 81.5 / 76.7 | 2.208 | 28.1 |
| WebP m4 sharp YUV q95 | 87.0 / 84.2 | 3.184 | 30.6 |
| JXL e3 d2 | 70.8 / 64.4 | 0.999 | 5.1 |
| **JXL e3 d1.5** | **75.7 / 69.2** | **1.230** | **5.5** |
| JXL e3 d1 | 83.3 / 78.0 | 1.846 | 7.2 |
| JXL e7 d1.5 | 76.3 / 67.9 | 1.351 | 66.1 |
| AVIF s6 q70 | 76.2 / 69.3 | 1.303 | 131.5 (CLI) |
| AVIF s6 q80 | 82.0 / 77.0 | 1.781 | 150.4 (CLI) |

**Per crop at the two recommended settings.** Each cell is bpp, then SSIMULACRA2.

| Crop | WebP sharp YUV q85 | JXL e3 d1.5 | AVIF q70 |
| --- | --- | --- | --- |
| town_classic | 3.39 / 77.7 | 2.63 / 74.0 | 2.67 / 75.1 |
| forest_kr | 3.87 / 78.9 | 2.97 / 69.2 | 2.96 / 75.0 |
| road_town | 3.16 / 78.5 | 2.39 / 77.3 | 2.45 / 77.1 |
| lava | 3.28 / 78.5 | 2.59 / 76.2 | 2.49 / 75.9 |
| bridge | 2.76 / 74.7 | 1.87 / 74.3 | 1.99 / 72.6 |
| dock_sprites | 1.98 / 69.1 | 1.28 / 71.8 | 1.35 / 69.3 |
| swamp | 1.32 / 69.8 | 1.01 / 73.4 | 1.08 / 75.6 |
| dungeon_black | 1.23 / 82.4 | 1.13 / 82.2 | 1.11 / 83.4 |
| snow_edge | 0.93 / 74.4 | 0.73 / 77.3 | 0.85 / 78.6 |
| water_coast | 0.62 / 65.7 | 0.50 / 76.8 | 0.51 / 73.8 |
| snow_flat | 0.21 / 74.6 | 0.20 / 79.8 | 0.27 / 81.4 |

**Decode time per 512² tile**, native, single thread (median):

- WebP: 2.2 ms;
- JXL (libjxl): 3.7–4.3 ms;
- AVIF: not comparable (CLI, 26–77 ms with process start).

Browsers decode with their own libraries; Chrome's JXL decoder is the Rust `jxl-rs`. Neither format is a
bottleneck at about 20 tiles per view.

### Artefacts that matter for pixel art

The decoded crops were inspected at 4× with nearest-neighbour scaling, at matched SSIMULACRA2 of about 70.

- **Hard sprite edges and 1-px details** (the barrel outlines, the anchor, the 1-px rope over water). All
  three formats keep the black outlines and the rope at these settings. There was no visible ringing on
  the outlines. AVIF slightly softens the rope edge.
  - Edge PSNR at matched SSIMULACRA2 70 is lower for JXL: 33.8 dB, against 36.2 for WebP and 36.8 for AVIF.
  - So JXL spends fewer bits on hard edges than on texture, but the difference was not visible at 1:1 or
    at 4×.
- **Saturated dark colours: JXL's visible defect.**
  - The water's red channel is 0 everywhere.
  - JXL (XYB colour space) adds red noise there at every distance tried, which turns the blue water texture
    greyish and speckled at zoom. The mean red goes from 0 to 3.5 at d1.5 and is still 3.0 at d0.5.
  - WebP with sharp YUV gives a mean red of 1.0. AVIF gives 0.5.
- **Texture smoothing.**
  - AVIF smooths the water's noise texture: the standard deviation drops from 5.3 to 4.7.
  - WebP keeps the texture; JXL adds to it (6.1).
  - Without sharp YUV, WebP's 4:2:0 chroma cannot reach high quality on red or saturated art. Plain WebP
    q95 tops out at a mean of 81 and a minimum of 71. Sharp YUV reaches 87 / 84 and fixes swamp, lava and
    forest at the same quality.
- **Banding on gradients.**
  - UO art has no smooth gradients once lights are off. The flattest area is `snow_flat`, a 179–213 grain.
  - Contrast-stretched 10×, every codec flattens the grain:
    - WebP into soft blotches;
    - JXL d2.5 into visible 8 × 8 blocks (much less at d1.5);
    - AVIF into smears.
  - None of it is visible without the stretch, at these settings.

### Browser support (late 2026)

| Browser | JPEG XL in `<img>` | Source |
| --- | --- | --- |
| Chrome | **155+**, default on (decoder: Rust `jxl-rs`). Chrome 145–154 had it behind `#enable-jxl-image-format`. | [Chrome for Developers, "Shipping JPEG XL in Chrome", 2026-10-06](https://developer.chrome.com/blog/jpeg-xl-in-chrome); [caniuse](https://caniuse.com/jpegxl) |
| Firefox | **158+** (released 2026-10-13), default on. 90–157 had it behind a pref. | [Firefox 158 release notes](https://www.firefox.com/en-US/firefox/158.0/releasenotes/); [caniuse](https://caniuse.com/jpegxl) |
| Safari | **17+** (September 2023): macOS Monterey and later, iOS/iPadOS 17. caniuse marks it "partial" without saying which features are missing; still images work. | [WebKit, "WebKit Features in Safari 17.0"](https://webkit.org/blog/14445/webkit-features-in-safari-17-0/); [caniuse](https://caniuse.com/jpegxl) |
| Edge | **Not working.** caniuse lists Edge up to 155 as unsupported. The press says Edge 155 (2026-10-08) announced JXL, but hands-on reports find it not working. | [caniuse](https://caniuse.com/jpegxl); [PiunikaWeb, 2026-10-09](https://piunikaweb.com/2026/10/09/microsoft-edge-155-jpeg-xl-support/) |

caniuse's global total is 60 % (43 % full, 17 % partial), because Chrome and Firefox only turned it on a month
ago. WebP works in every current browser, and so does AVIF (Edge since 121).

Options if JPEG XL is wanted anyway:

- **A client-side WASM decoder** (for example jxl-oxide's WASM build) in a Leaflet `GridLayer` that draws
  into canvases.
  - It works everywhere, but adds a few hundred KB of WASM and a worker pool, decodes 20–40 tiles per pan in
    JS-land, and replaces plain `<img>` tiles.
  - Not worth it for a format that is only 25–30 % smaller.
- **Two tile sets** (JXL plus WebP fallback). Leaflet tiles cannot use `<picture>`. Instead, the viewer
  decodes a tiny JXL data URI once at start-up and picks the URL template.
  - Disk is about 1.8 × WebP-only, not 2 ×, since JXL is smaller.
  - Encoding cost is small (JXL e3 is 5 ms per tile).
- **JXL for archival, WebP for serving.** The lossless level-6 intermediate already serves the
  "re-encode without re-rendering" purpose for the low levels. A full lossless archive of the base level
  would be about 20–40 GB, re-rendering is cheap, and the export is reproducible from the packages. So no
  archive is proposed.

### Recommendation

- **Serve WebP**: libwebp 1.6.0, method 4, **quality 85, sharp YUV**.
  - Mean SSIMULACRA2 75, minimum 66, 1.57 bpp, 26 ms per tile.
  - `--quality 90` is the "very high" setting: 81.5 / 76.7 at 2.21 bpp, +40 % disk.
  - Method 6 saves only 3–4 % for 2.5 × the encode time, so it is not worth it.
- **JPEG XL wins on compression**: 22–34 % smaller at matched SSIMULACRA2. Effort 3 is both the fastest
  (5 ms) and as good as efforts 5 and 7.
  - It does not win for serving today:
    - Edge does not decode it;
    - Chrome and Firefox users older than a month do not either;
    - it greys saturated dark colours, which UO water is full of.
  - It costs nothing to offer as an **optional second output** (`--format webp,jxl`, d1.5 e3), since libjxl
    is already in the workspace, with the viewer picking the format at start-up.
  - Revisit the default when caniuse shows Edge support.
- **AVIF** matches JXL's size and works everywhere.
  - But it smooths textures, encodes 20 × slower, and would add libavif and an AV1 encoder (or the
    pure-Rust `ravif` with rav1e) as a new dependency.
  - Not proposed unless disk size becomes the constraint.

### Colour fidelity (re-measured)

This section asks a different question from the one above: which format keeps the colours, at any size
and whatever the browser support.

**Method.**

- **Crops.** The same 11 crops of 512 × 512 at the same coordinates.
- **Probe.** The probe crate was rebuilt in
  `/home/claudio/rust-target/uodynamapper-scratch/webmap-codecs` with the same libraries:
  - libjxl 0.12.0 through `jpegxl-rs` 0.14.0;
  - libwebp 1.6.0 through `libwebp-sys` 0.14.4;
  - `avifenc`/`avifdec` 1.4.2.
- **Settings.**
  - **JXL XYB**: the default lossy mode, as above.
  - **JXL no-XYB**: lossy with `uses_original_profile = true`, so libjxl codes the RGB values without
    converting them to XYB. Both VarDCT and modular were tried.
  - **JXL lossless**: efforts 1, 3 and 7.
  - **WebP lossless**: `lossless = 1`, `exact = 1`, quality 75, method 4.
  - **WebP lossy**: the current setting (method 4, sharp YUV, q85), plus q90 and q95.
  - **AVIF**: 4:4:4 (`-y 444`), speed 6, one thread.
  - The probe checks that every lossless output matches its source bit for bit.
- **Metrics.**
  - **SSIMULACRA2**: the mean and the minimum over the crops.
  - **Error per channel**: the mean absolute error, the mean signed error (bias) and the largest absolute
    error, in 8-bit units, over all 11 crops.
  - **Water mean red**: the mean decoded red over the `water_coast` pixels whose source red is exactly 0.
    The 3.5 quoted above came from a different selection of pixels, so the two numbers are not comparable.
  - **ΔE2000 on dark saturated pixels.**
    - A pixel counts when its largest channel is between 16 and 128 and its (max − min) / max is at least
      0.5.
    - That gives 1.31 M of the 2.88 M pixels, mostly water, swamp, shadows and dark foliage. `snow_flat`
      has none.
    - The colour difference is CIEDE2000 between the source and decoded colours in CIELAB, converted from
      sRGB with a D65 white. The implementation reproduces Sharma's test pair 1 (2.0425).
    - Each row gives the mean, the 95th percentile and the maximum, pooled over all those pixels.
- **Timing.**
  - Each tile is encoded and decoded on one thread; the best of 3 runs is kept.
  - **Boost off, EPP power**: the cores stay at 2.0 GHz, on a Ryzen AI 9 365.
  - Every encoder is about 5 × slower than in the table above, which ran with boost on, so the two tables
    cannot be compared.
  - The AVIF figures come from the CLI, so they include starting the process and writing the PNG.

**Results.**

| Setting | Total KB (11 crops) | bpp (geo mean) | SSIMULACRA2 mean / min | MAE R / G / B | Bias R / G / B | Max error R / G / B | Water mean red | ΔE2000 dark sat. mean / p95 / max | Encode ms | Decode ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| JXL XYB e3 d1.5 | 554 | 1.230 | 75.7 / 69.2 | 4.08 / 2.89 / 5.34 | +1.28 / +0.12 / +1.33 | 114 / 65 / 162 | 2.45 | 2.47 / 6.35 / 43.5 | 32.5 | 19.8 |
| JXL XYB e3 d1 | 802 | 1.846 | 83.3 / 78.0 | 3.32 / 2.05 / 4.74 | +1.18 / +0.10 / +1.18 | 107 / 48 / 154 | 2.43 | 2.15 / 5.66 / 38.4 | 36.1 | 20.5 |
| JXL XYB e3 d0.5 | 1268 | 3.071 | 89.9 / 87.1 | 2.42 / 1.24 / 3.91 | +0.96 / +0.06 / +0.92 | 79 / 34 / 130 | 2.08 | 1.71 / 4.63 / 29.3 | 43.0 | 20.0 |
| JXL no-XYB VarDCT e3 d2 | 1488 | 3.408 | 77.7 / 69.3 | 2.25 / 2.71 / 4.31 | +0.12 / +0.02 / +0.49 | 44 / 28 / 116 | 0.01 | 2.21 / 5.79 / 34.2 | 45.0 | 22.7 |
| JXL no-XYB VarDCT e3 d1.5 | 1674 | 3.894 | 80.3 / 73.4 | 2.23 / 2.25 / 3.95 | +0.12 / +0.02 / +0.44 | 45 / 22 / 108 | 0.01 | 2.02 / 5.27 / 33.0 | 38.8 | 23.1 |
| JXL no-XYB VarDCT e3 d1 | 2115 | 5.069 | 84.0 / 78.2 | 2.20 / 1.45 / 3.22 | +0.12 / +0.01 / +0.32 | 45 / 15 / 60 | 0.01 | 1.67 / 4.40 / 26.2 | 49.2 | 25.4 |
| JXL no-XYB VarDCT e3 d0.5 | 2835 | 7.048 | 86.4 / 78.7 | 2.19 / 0.76 / 2.31 | +0.12 / +0.01 / +0.19 | 45 / 8 / 34 | 0.00 | 1.31 / 3.52 / 20.5 | 57.5 | 26.8 |
| JXL no-XYB VarDCT e7 d1 | 2103 | 5.340 | 61.6 / 19.6 | 5.06 / 1.46 / 3.38 | +0.50 / +0.01 / +0.37 | 121 / 19 / 119 | 0.00 | 2.52 / 7.64 / 33.1 | 270.2 | 21.4 |
| JXL no-XYB modular e3 d1.5 | 1933 | 4.510 | 77.5 / 71.4 | 3.16 / 2.80 / 3.48 | +0.42 / +0.05 / +0.40 | 74 / 106 / 66 | 0.71 | 2.28 / 5.93 / 45.1 | 150.2 | 90.9 |
| JXL no-XYB modular e3 d1 | 2396 | 5.695 | 83.7 / 80.3 | 2.53 / 2.09 / 2.77 | +0.36 / +0.04 / +0.30 | 41 / 103 / 41 | 0.60 | 1.95 / 5.19 / 32.9 | 142.4 | 83.7 |
| JXL no-XYB modular e3 d0.5 | 3415 | 8.487 | 89.8 / 87.0 | 1.69 / 1.31 / 1.82 | +0.25 / +0.03 / +0.16 | 21 / 103 / 19 | 0.47 | 1.37 / 3.66 / 19.7 | 133.4 | 80.5 |
| JXL lossless e1 | 4366 | 11.895 | lossless | 0 | 0 | 0 | 0 | 0 | 4.8 | 27.8 |
| JXL lossless e3 | 4299 | 11.608 | lossless | 0 | 0 | 0 | 0 | 0 | 142.5 | 93.0 |
| JXL lossless e7 | 3059 | 8.214 | lossless | 0 | 0 | 0 | 0 | 0 | 816.9 | 146.6 |
| **WebP lossless q75 m4 exact** | **2708** | **7.052** | **lossless** | **0** | **0** | **0** | **0** | **0** | **489.6** | **9.1** |
| WebP m4 sharp YUV q85 (current) | 728 | 1.571 | 74.9 / 65.7 | 2.89 / 2.34 / 3.87 | +0.27 / +0.26 / +0.20 | 98 / 55 / 144 | 0.42 | 2.12 / 5.41 / 34.7 | 134.6 | 12.2 |
| WebP m4 sharp YUV q90 | 932 | 2.208 | 81.5 / 76.7 | 2.51 / 1.87 / 3.51 | +0.27 / +0.25 / +0.20 | 97 / 53 / 144 | 0.44 | 1.89 / 5.00 / 35.0 | 126.6 | 14.7 |
| WebP m4 sharp YUV q95 | 1249 | 3.184 | 87.0 / 84.2 | 2.20 / 1.43 / 3.19 | +0.29 / +0.20 / +0.23 | 97 / 51 / 145 | 0.44 | 1.68 / 4.62 / 34.1 | 142.4 | 19.1 |
| AVIF 4:4:4 s6 q70 | 568 | 1.303 | 76.2 / 69.3 | 2.78 / 2.71 / 3.61 | +0.29 / −0.03 / +0.22 | 74 / 47 / 116 | 0.44 | 2.13 / 5.44 / 35.8 | 531 (CLI) | 195 (CLI) |
| AVIF 4:4:4 s6 q80 | 759 | 1.781 | 82.0 / 77.0 | 2.27 / 2.09 / 3.12 | +0.26 / −0.02 / +0.20 | 65 / 35 / 94 | 0.43 | 1.86 / 4.80 / 30.0 | 620 (CLI) | 208 (CLI) |
| AVIF 4:4:4 s6 q90 | 1143 | 2.777 | 88.2 / 85.4 | 1.68 / 1.37 / 2.45 | +0.24 / −0.04 / +0.19 | 35 / 19 / 47 | 0.40 | 1.47 / 3.83 / 26.1 | 802 (CLI) | 214 (CLI) |

The probe also writes a table per crop and setting to `out/per_crop.tsv`.

**What the numbers show.**

- **XYB tints the colours, and lowering the distance does not fix it.**
  - Red and blue drift upwards on average (bias +0.9 to +1.3) at every distance; the other codecs stay
    within ±0.3.
  - In the water, red rises from 0 to 2.1–2.5 whatever the distance. WebP and AVIF give about 0.4.
  - Even at d0.5 the largest blue error is 130.
  - XYB is the reason. It is not a matter of how many bytes are spent.
- **JXL without XYB removes the tint, but is not usable either.**
  - The water red stays at 0.00–0.01, and the bias is +0.12.
  - **VarDCT has an error floor in red.** The red error stays flat as the distance drops. In
    `town_classic` the red MAE is 3.64 at d1.5, 3.62 at d1 and 3.61 at d0.5, and the largest red error is
    41–45 throughout, while green improves 3.5-fold and blue 1.7-fold. The same holds in every crop with red
    content. That is a measurement; why libjxl does it was not investigated.
  - **Effort 7 is broken in this mode.** The red MAE doubles and the SSIMULACRA2 minimum falls to 19.6
    (`lava`).
  - **Modular** has no floor, but it keeps a single green error of 103–106 in `dock_sprites` at every
    distance. It is also 4 × slower and 3 × larger than XYB for the same SSIMULACRA2.
  - libjxl's lossy encoder is tuned for XYB. Without it, JXL also loses its size advantage: d1 is 5.07 bpp,
    against 2.78 for AVIF q90.
- **Current WebP q85.**
  - **A sharp-edge error of about 97 in red and 144 in blue that higher quality never reduces.** q85, q90
    and q95 all keep it. It is the 4:2:0 chroma on 1-px colour edges, which sharp YUV narrows but cannot
    remove.
  - Otherwise it is accurate on dark colours: ΔE2000 has a mean of 2.1 and a p95 of 5.4, and the water
    red is 0.4.
- **AVIF 4:4:4 is the most faithful lossy setting.**
  - At q90 (2.78 bpp) it has the lowest ΔE2000 of all the lossy settings (mean 1.47, p95 3.8) and the
    smallest maximum errors (35 / 19 / 47).
  - WebP q95 is larger (3.18 bpp) and still keeps its 97 / 145 edge errors.
  - AVIF q70 matches WebP q85 on ΔE2000 at 17 % fewer bytes, with smaller maximum errors.
  - Its costs are those given above: a new encoder dependency, slow encoding, and smoothed textures.
- **Lossless: WebP beats JPEG XL on this art.**
  - WebP lossless is 7.05 bpp, against 8.21 for JXL e7.
    - It is 14 % smaller and encodes in 490 ms, against 817.
    - It decodes in 9 ms, against 147. That is the fastest decode of any setting here, lossy included.
  - JXL e1 encodes fastest (4.8 ms) but is 69 % larger than WebP lossless.
  - Every lossless output matched its source exactly.

**AVIF and WebP in one process (2026-11-04).** The AVIF timings above include starting the process and writing a PNG,
so they overstate its cost, its decode most of all. These come from Pillow 13 with libavif 1.4.2 and libwebp 1.6.0, on one
thread, per 512² tile, averaged over the 11 crops, encode best of 3 and decode best of 7. CPU state: boost off, EPP
`balance_power`, with one other job running on another core. File sizes match the table above exactly where the setting
is the same. Pillow doesn't expose sharp YUV, so the WebP lossy rows leave it out, which makes their encode faster than
the tool's; it doesn't affect decoding.

| Setting | Total KB | Encode ms | Decode ms |
| --- | ---: | ---: | ---: |
| WebP q85 m4, no sharp YUV | 663 | 47.8 | 8.3 |
| WebP q95 m4, no sharp YUV | 1162 | 58.4 | 11.9 |
| WebP lossless m4 | 2708 | 263.6 | 7.8 |
| AVIF 4:4:4 s6 q70 | 568 | 301.5 | 10.9 |
| AVIF 4:4:4 s6 q90 | 1143 | 473.4 | 17.4 |
| AVIF 4:4:4 s8 q90 | 1168 | 162.9 | 16.9 |
| AVIF 4:4:4 s4 q90 | 1176 | 3108.5 | 18.1 |

So AVIF decodes 1.3–2.1× slower than WebP, not the 15× the CLI figures suggested. Speed 8 encodes 2.9× faster than speed
6 for 2 % more bytes; its quality hasn't been scored.

**Disk for Britannia**, scaled from the 6–9 GB estimate for q85 by the bpp ratio:

| Format | Disk |
| --- | --- |
| WebP sharp YUV q85 | 6–9 GB |
| AVIF 4:4:4 q90 | 11–16 GB |
| WebP lossless | 27–40 GB |
| JXL lossless e7 | 31–47 GB |

Encoding WebP lossless takes about 3.6 × the q85 time per tile. That is roughly 15 minutes on 20 threads
at full clock.

**Recommendation on fidelity.**

- **Exact colours: lossless WebP** (`lossless = 1`, `exact = 1`, method 4, quality 75).
  - Every pixel stays bit for bit, so the water red is 0 and ΔE is 0.
  - Every browser shows it, and its decoding is the fastest measured here.
  - It is smaller than lossless JXL at any effort.
  - Its price is disk: about 4.5 × the current tiles, 27–40 GB for Britannia.
  - It needs only the `--lossless` switch the tool left out. The tool already uses libwebp, so there is no
    new dependency.
- **If that disk is too much: AVIF 4:4:4 q90.**
  - It has the best ΔE2000 and the smallest maximum errors among the lossy settings, at 1.8 × the current
    disk.
  - It adds an AV1 encoder dependency and slow encoding.
  - It is still the most faithful option below lossless.
- **Not JPEG XL lossy, in either mode.**
  - XYB tints the colours.
  - Without XYB it has a red error floor, effort 7 breaks, and it is not smaller than AVIF.
  - JXL lossless is larger and slower than WebP lossless.
- **The current WebP q85 stays the choice for size.**
  - Its known limit is the 1-px colour edge, which no quality setting fixes.

**Decided by the user on 2026-11-04, with these numbers: WebP q85 stays.**

## Phases and effort

1. **Pilot** (half a day, tool side). A control-channel prototype captures a 2 × 2 frame block around
   Britain at 1280 × 800 windows. It measures render and ready time per frame, the seam behaviour, and the
   bpp on a real region, and replaces the estimates above.
2. **Renderer export mode** (renderer session, 2–4 days): the contract above, with the seam and
   determinism tests.
3. **Tool** (2–3 days):
   - plan, run, pyramid and viewer;
   - the libwebp wrapper;
   - resume;
   - unit tests for the projection, inverse, tile cutting and pyramid alignment;
   - an end-to-end test on a small facet region.
4. **Full Britannia run and check** (1 day, mostly waiting): disk, time, a visual pass, and a CI build of
   the tool on all targets.

## Risks

- **Renderer non-determinism shows up as seams**: animation time, asynchronous uploads, culling that
  depends on the window. The bit-identical overlap test catches it, and contract items 3, 5 and 8 address
  it.
- **Capture time is unmeasured.** If a frame takes much more than 2 s, a full run takes hours. It is still
  resumable.
- **About 145k files and 6–9 GB per Britannia**, which matters for hosting. Static hosting with directory
  listings disabled is fine. Some hosts limit the file count.
- **The coordinate readout is approximate on hills** until heights ship to the viewer.
- **libwebp on MinGW, ARM64 and macOS is reasoned, not run here.** The first CI build of the tool confirms
  it.
- **Publishing.** The tiles are renders of EA's game art. Whether and where they are published is the
  user's call.

## Questions for the user

1. **Tile size**: 512 px (recommended: 145k files for Britannia) or 256 px (578k files)?
2. **Quality**: WebP q85 with sharp YUV (recommended, about 6–9 GB) or q90 (very high, about 40 % more)?
   Also: add the optional JPEG XL second set now (+80 % disk, served to Chrome 155+, Firefox 158+ and
   Safari 17+, with a start-up check), or keep WebP only (recommended)?
3. **Viewer**: vendor Leaflet 1.9.4 into the output (recommended, works offline) or load it from a CDN
   with SRI?
4. **Capture path**: build the offscreen export mode in the renderer (recommended; it is renderer-session
   work), with the control-channel pilot first to measure? Or settle for control-channel capture?
5. **Coordinate readout on hills**: accept the z = 0 approximation in v1 (recommended), or ship per-chunk
   heights to the viewer for an exact readout (+1 day)?
6. **Facets**: all six, or Britannia only first?
7. **Location**: a new crate `tools/webmap-export-cli` (recommended) or a `udd-cli` subcommand?
8. **Downsampling**: an sRGB box average (the map-tile convention, recommended) or gamma-correct?
