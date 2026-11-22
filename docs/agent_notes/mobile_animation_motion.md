# Mobile animation motion: decisions and reasons (2026-11-21)

The client animations are short (Classic animals 5 walk frames at 80 ms, Enhanced 6–12 at 9 fps), so on a 60–255 Hz
display each pose stays on screen for 5–28 frames. This note records what was chosen to make them move smoothly, what
was rejected, and why. The user judged every visual choice from videos and stills; the scripts that made them are in
[`mobile_animation_motion/`](mobile_animation_motion/) (they read clip dumps from the scratch dumper
`anim-interp/rs/src/bin/dump.rs` and need PyTorch; paths inside are the 2026-11-21 scratch layout).

Evidence priority for animation behaviour: visuals first, then the official Classic client for UOP bodies and ClassicUO
for MUL bodies, the Enhanced client last. A bigger animation package is acceptable only if the benefit is clearly
visible.

## Stopping

`MobileStop` (`world_rendering.mobile_stop`): **half a step later** (default) or **at once**. A walk keeps playing for
half a step after the last step, so tapping a direction doesn't snap to standing between steps; "at once" stands as the
step ends unless a direction is still held. The user asked for both as a run-time switch after comparing them.

## Between frames

| Method | Verdict | Why |
|---|---|---|
| No blend (hold) | rejected as default | pops between poses; "doesn't look so bad", but the fade is smoother |
| Crossfade over the whole gap | rejected | thin limbs look ghosted/doubled (the "thin horse leg" was this, not the art) |
| **Crossfade over the middle 50% of the gap** (smoothstep, `CROSSFADE_SPAN`) | **kept, default** | user: "cleaner, with less halo"; poses stay solid longer |
| DIS / Farnebäck optical flow warp | rejected | limbs bend and smear: "extremely weird, movement extremely unnatural" |
| Block matching | rejected | no better than the fade in the measurement study |
| **RIFE v4.25** (Practical-RIFE, MIT) | **kept as options ×2 / ×4** | single solid limbs at believable halfway positions, no doubling; "convinces me very much" |

The crossfade runs in the art sprite shader (both frames sampled on one card, blended premultiplied in sRGB like the
CPU version it replaced): no CPU cost or uploads at any refresh rate.

### RIFE: generated at run time on the GPU, not stored

- Stored in-betweens would double the packages for ×2 (Classic 0.9 → ~1.8 GB, Enhanced 2.7 → ~5.4 GB) and quadruple
  them for ×4. The run-time cost is small, so files don't grow.
- wgpu compute shaders on Bevy's own device (Vulkan/DX12/Metal), not OpenCL (patchy, deprecated on macOS), not ncnn or
  ONNX Runtime (separate devices and thread pools), not burn/CubeCL (its own wgpu version).
- Weights embedded as fp16, 11.3 MB; fp16 vs fp32 changes the output by at most 0.64/255. The port matches PyTorch
  within 0.00025 px of flow and 0.014/255 of colour.
- Cost on a Radeon 880M per in-between: 1.4 ms (64²), 2.0 (128²), 4.8 (256²), 17 ms (576×512), about half what it
  was before naga's loop bounding was turned off (wgsl_loop_bounding_blocks_unrolling.md). PyTorch on one CPU
  thread: 19 / 49 / 145 / 618 ms — the CPU is a fallback at best.
- Work is rationed per frame; a clip plays as the 50% fade until its in-betweens are ready.
- A clip, or a sequence (a clip's frames and their in-betweens), too big for one 1024² atlas page is stored in pieces
  over several pages, so large Enhanced creatures (the daemon at ×4) get their in-betweens too, and the daemon's
  stand facing east, whose frames alone exceed a page, shows at all. It stays one unit: losing any of its pages drops
  it whole, and it is made again when drawn.
- The 50% fade still runs between the shorter frames. Expected visible change per display rate: at 30 fps ×2 adds
  ~1 blended frame per pose change, at 144 fps ~6–8.

### RIFE details

- **Nearest sampling at the final warp** (keeping RIFE's flow): bilinear made the in-betweens of small sprites softer
  than the real frames; nearest keeps dithering and pixel outlines crisp (`rife_crisp_warp.py`).
- **Classic alpha snapped at 50%**: Classic art has binary alpha, so a half-transparent pixel is always an artefact.
- **Known limit**: when a thin limb changes shape completely between frames (Classic horse's foreleg curled →
  extended), RIFE finds no correspondence and the leg turns thin and half transparent. A finer last scale, 2× input and
  taking each pixel whole from one frame do not fix it (`rife_leg_fixes.py`), and a whole-sprite ghost score does not
  single out the bad pairs (the failure is local). Generative in-betweening (ToonCrafter, FCVG, ARVFI) is too slow and
  inconsistent for this; large motion and occlusion are still open problems in the literature.

## Walk pace

The original clients pace walk cycles by time, whatever the walking speed: Classic shows a frame every 80 ms
(ClassicUO `Mobile.ProcessAnimation`), Enhanced plays at 9 fps. Classic's art was made for that (a 5-frame animal cycle
is one 0.4 s step), but big Enhanced creatures skate visibly, and in-betweens make it more visible because the feet
never plant.

What each client times a clip by: Classic MUL bodies take a frame every 80 ms in ClassicUO; the client itself advances
them once a mobile tick, which runs when more than 75 ms passed. Classic UOP bodies (`mobtypes.txt` flag 0x10000) take
1000 / fps of the action in their `AnimationSequence.uop` entry, aliases resolved (`CMobile_SetUopAction` 0x005b6580,
Classic 7.0.103.0, [classic_frames.md](../client_research/ghidra-classic-frames/results/classic_frames.md#uop-bodies));
those entries are the Enhanced client's, which times every body by them (`GetActionFps`). `mobile_anim_cc.uddp`
stores that fps on the UOP animation records; MUL records and UOP bodies without an entry keep 80 ms.

Choice (user, from `pace_compare.py` videos): **measured stride** over the Enhanced clip timing and over borrowing the
Classic body's cycle. A walk/run cycle lasts stride_tiles × step_secs, clamped to 0.5–2× the clip's own cycle speed;
"client timing" stays as a setting.

Measuring the stride (`stride_measure.py`): RIFE flow between consecutive frames at t = 0.5; in the lowest 20% of the
silhouette, the most backward motion (10th percentile) along the facing's screen direction is the planted foot, i.e.
the body's travel; summed over the cycle and divided by the tile's screen length in that facing. Only the side facings
(server directions 4, 5, 6) agree with each other (within ~25% on horses, wyrms, ogres, daemons); facings toward or
away from the camera and all small creatures (dogs, rats, cats) give unusable numbers, which the clamp absorbs. So the
strides are measured offline over every body into a table, not guessed per frame at run time.

A rider and its mount keep one shared cycle, paced by the mount's stride. The sequences already aim for this: a mount's
ridden run is set to 12 fps instead of 9 so that its bounce stays in step with the rider (UOP animation editor's
remark, 2026-11-21, on body 1409, which the installed clients don't have), and the riders' mounted walks and runs are
12 fps too (709 bodies in the Enhanced package). A sequence frame count can exceed the
stored frames (the package fills the gap with empty placeholders); the app drops empty frames, so neither the
in-betweens nor the stride sum see them.

The strides live in the animation packages, `metadata/strides.bin` of `mobile_anim_cc.uddp` and `mobile_anim_ec.uddp`
(layout in [UDDP_FORMATS.md](../dev_wiki/UDDP_FORMATS.md); reader in `udd_assets::animation::mobile_strides`), so the
udd tools see them too. A record is keyed by body and action: walk and run of the peace and war groups and, Enhanced,
the mounts' mounted moves, so a war mode needs no new strides. Each carries the XXH64 of the frames it was measured
on: for the side facings 4, 5, 6 in order, presence, mirroring, and each kept frame's size, anchor and RGBA (skipped and
empty frames dropped; the fps is not hashed). A stride never applies to other frames: the app (`render/mobiles/pace.rs`)
hashes a clip's side facings on the background pool the first time it asks for its stride and keeps the client timing
until the hash matches, for good when it doesn't (one debug log per clip). The check costs 5–30 ms of one worker per
clip, once a session, nearly all of it reading the atlas pages (the hash itself is under 2 ms on a 17 MB clip). A
package without the entry loads fine and keeps the client timing for every clip.

The GPU port of the prototype writes them: `udd-cli tool strides measure changed|full FILE...` (`just
measure-mobile-strides`; `--body N` repeatable, `--output DIR`, `--report DIR`), on the measurement core of
`lib/support/uot-gpu-compute` (`strides.rs`: plain RGBA frames and anchors) driven over the packages by
`udd_conv::animation::mobile_strides`, which the inspector's strides panel and the converter GUI's measure step run too.
It blends the in-between with Practical-RIFE's bilinear warp, as the prototype did, not the in-betweens' nearest one:
the in-between's alpha picks the feet, and the nearest warp's other edges moved Classic body 1's west walk from 1.919
to 1.935 tiles.
Dynamapper only reads the strides. It hashes every
clip on all threads (13 s for both packages), then measures on the GPU only what it must: `changed` the clips with new
or other frames than their stride's (a clip sharing the frames of a kept one takes its measurement), dropping the
strides of clips gone; `full` everything again (about 40 minutes on a Radeon 880M). It rewrites a package only when
its strides changed, copying the other entries as stored (3 s for the 2.7 GB Enhanced package, warm cache), and prints
what it measured, unchanged, changed, new, removed and rejected. By hand, `udd-cli tool strides export|import|status`
writes a package's strides as a tab-separated table (`source body action south south-west west stride hash`), imports
one (rows whose hash no longer matches the frames are skipped; rows without a hash need `--trust-unhashed`), and lists
the stale ones. A repack with `udd-cli pack-mobile-anims` / `pack-ec-mobile-anims` keeps the strides of the package it
replaces (or of `--strides-from`) whose frames still hash the same; changed and new clips get none until measured.
The first strides, measured into the table that was bundled in the binary until 2026-11-21, were moved into the
packages with `strides import --trust-unhashed` from the measurement report, hashed on the packages they were measured
on, without measuring again. The stride is the median of the side facings that measured (at least two), even when they
disagree: the clamp bounds a poor number, and rejecting a spread over 60% of the median turned away the clips the
user judged from the prototype videos (Enhanced wyrm 46, stygian 826) and most Enhanced creatures. Each record keeps
each facing's stride, and the generator prints their spread for inspection. A clip whose median is not positive or
under a quarter tile is recorded without a stride and keeps the client timing: bodies without planted feet (gazers,
slimes, wisps) and the smallest creatures measure a few hundredths of a tile, and the clamp would play them at twice
their speed. The 2026-11-21 strides pace 882 of 1914 Classic clips with frames and 1627 of 4125 Enhanced ones. Riders never use a stride of their own: the mount's clip is
paced from the mount's stride and the rider shows the same point of that cycle, with the by-eye Enhanced mounted
cycles as the fallback. The cycle position is a phase accumulator (`MobileCycleClock`), so a cycle length that changes
mid-walk doesn't make the frame jump.

The port must feed RIFE exactly what the prototype did: the grey backdrop (RGB over 0.5 grey) covers the whole canvas
the clip's frames share, and only the padding up to a multiple of 64 stays black. With the grey on each frame's own
rectangle only, the flow near the feet changed and Classic body 9's walk came out 2.66 tiles instead of 2.79; with the
canvas grey the port matches the prototype to 0.01 tiles.

## Upscaling when zoomed

- **Classic**: no neural upscaler — those models are trained on photos/anime, and Classic's hard one-pixel art is what
  the pixel-art scalers (xBR, MMPX, … see CREDITS.md) are built for.
- **Enhanced/KR**: Real-ESRGAN's compact models (realesr-animevideov3, realesr-general-x4v3, BSD-3) were rejected:
  painted look and a maze pattern on fine texture; bicubic looked better (`upscale_realesrgan.py`). Their
  real-world degradation + GAN training invents detail. Faithful models trained on pixel loss (SPAN 4xpix /
  4xmssim pretrains, 2.2 M parameters, CC-BY-4.0, `upscale_span.py`) look like a sharper bicubic without invented
  texture. **Chosen: 4xmssim_span** (user: looks better, most of all on small animations, where bicubic blurs the
  little detail there is). Folded for inference (each Conv3XC into one 3x3 conv) it has 426k parameters, 0.86 MB as
  fp16, within 0.08/255 of the original. Alpha is upscaled bicubically, not through the network.
- 4× frames would cost 16× the atlas memory (tight on a 4 GB card), so the game stores 2× results, only for clips on
  screen while zoomed above ~1.5×.

### SPAN 2× in game (`mobile_upscale`, 2026-11-21)

`world_rendering.mobile_upscale` is `off` (default) or `span_x2`: in Preferences, and `set mobile_upscale` on the
control socket. Off by default: it costs GPU time and VRAM the zoomed-out view never shows, and it is a look the user
has not chosen yet for every style. Code: `render/mobiles/upscales.rs` (main world) and `upscales/render.rs`; the
network and its frame steps are uot-gpu-compute's `upscale`.

- **Gate**: the view's magnification is the window's scale factor over `RenderZoom` (zoom 0.5 on a scale-1 display is
  2×). Upscales start at 1.5× and stop below 1.35×, so a zoom around the threshold does not flap. Below it the 1×
  frames draw as before, and the queued jobs stop.
- **What upscales**: the sequence a layer shows at 1×: its clip, or in a RIFE mode the clip's RIFE sequence once that
  is made (RIFE first: the upscale of a sequence still being generated would be thrown away). Crossfade and Off
  upscale the clip, and the crossfade blends the 2× frames as it blends the 1× ones. Classic frames get their 2× alpha
  snapped at 0.5 in the finish step, so their edges stay hard.
- **Atlas**: 2× frames are atlas entries of their own (`AtlasEntry::Upscaled`) in a pool of 16 extra pages per atlas
  (`AtlasPool::Upscaled`, layers 8–23), in the atlas's format: BC7 atlases get them encoded on the GPU by the warmed
  BC7 UltraFast encoder, shared with the RIFE in-betweens (`bc7_encoding.rs`). Only upscales empty upscaled pages, so
  1× frames are never evicted for them, and an evicted upscale just plays at 1× again. The pages exist only while the
  setting is on: switching it remakes the atlases (everything reloads, as after a package switch).
- **Drawing**: each 2× frame is its canvas cut to the frame plus the 2-pixel border the network sees, so the halo past
  the frame's edge shows: the placement grows by the border and its anchor moves by it, and the card spans 2 texels
  per art pixel (`Clip::texel_scale`), the same world size as the 1× frame.
- **Switching**: a layer switches to 2× only when the whole sequence is made, and only as the sequence starts over
  (its frame index going back), so one cycle never mixes 1× and 2×; it switches back to 1× the same way when zoomed
  out, and at once if the 2× frames were evicted.
- **Order**: the sequences drawn in the last 2 s, the nearest the player first (in steps of a tile), then the largest
  on screen; after them, jobs already started whose sequence was drawn in the last 10 s, so a mobile that stops and
  walks on finds its walk made rather than started over (without this, the daemon's RIFE x4 walk often never reached
  2× in the test's walk-stand-walk). A job dropped after that starts over when listed again.
- **Budget**: one per-frame budget of 2 ms for both RIFE and SPAN (`compute_budget.rs`), not two: what matters is the
  frame's total GPU time, and when RIFE is idle (sequences made, or crossfade) SPAN gets all of it. RIFE spends first,
  because the upscales of a RIFE mode wait for its sequences. Each frame step (prepare, the network's 22 dispatches,
  finish with its encode) has a cost estimate from uot-gpu-compute's benchmark fit (0.027 ms a dispatch plus 1 ms per
  10⁹ multiply-adds, ~0.43 M per canvas pixel), corrected by timestamp queries (`CostScale`). A frame larger than the
  budget spreads over several frames; its workspace stays with it until it ends. Canvases above 260×260 (90 MB of
  workspace, ~30 ms) are not upscaled.
- **Workspaces**: one per canvas size (rounded up to 8, so a body's actions share one) and atlas texture, made on the
  background pool, freed after 20 s unused (~1360 bytes per canvas pixel: 5.6 MB at 64², 89 MB at 256²).

Measured 2026-11-22 on the Radeon 880M (dev build, 1900×1000, zoom 0.4 = 2.5×, frame limiter off), the four styles
with RGBA8888 and BC7 packages, Classic human 400 on horse 200, Classic dog 217, Enhanced daemon 9 and stygian 826,
crossfade, RIFE x2 and x4 (scripts and captures in `uodynamapper-scratch/span-ingame/`):

- **Budget**: SPAN submissions timed by timestamp queries: median 1.5–1.7 ms, p95 2.1–2.5 ms, max 2.9–5.0 ms (rare: a
  step's estimate missing on a large frame). The cost scale settles at 1.05–1.3 over the benchmark fit.
- **Time to 2×** (from the sequence's first draw to all its frames made, RIFE's own time excluded): Classic dog and
  human/horse 0.03–0.4 s (x4: 0.1–1.8 s); Enhanced daemon 0.9–3.6 s at 1×, 2–10 s for its RIFE x2/x4 sequences. The
  budget is per frame, so a style with slower frames (Painterly, ~20 ms) makes them slower.
- **Frame time**: within noise of upscale off (Classic 5.8 vs 5.6 ms, Enhanced 12.8 vs 12.2 ms, KR and Painterly
  the same or lower).
- **VRAM** (sysfs `mem_info_vram_used`, the whole card): turning the upscales off frees 96 MiB with RGBA8 packages (the
  16 upscaled pages, 64 MiB, plus the workspaces); with BC7 the pages are 16 MiB and the drop is within the driver's
  64 MiB blocks. The card sat at 3.4–3.6 GiB of 4 with the desktop.
- **Not upscaled**: the stygian (frames up to 512×413, above the 260×260 canvas cap; a 516×417 canvas would need
  ~290 MB of workspace). Tiling a large frame over a small canvas would lift the cap.
- **Looks**: at 2.5× the 2× frames are sharper and smoother at the edges, Classic edges stay hard, BC7 and RGBA8 look
  the same, the placement matches the 1× frames, and at zoom 1 nothing is upscaled. Side-by-side videos (RIFE x4,
  walking then standing): `uodynamapper-scratch/span-ingame/videos/*-off-vs-span.mp4`.
- The regression harness with upscale off (quick tier) matches the commit before: 76 identical, 15 within tolerance.

## Ahead of time instead of at run time (measured 2026-11-22)

What SPAN 2× and the RIFE in-betweens would cost made by the converter instead of the game, measured by
`ahead_of_time_motion_costs` (ignored, `lib/udd-conv/src/animation/ahead_of_time_motion.rs`, with
`uot_gpu_compute::frame_upscales`) on the Radeon 880M. Method: each package's stored clips, split into 8 strata of
equal 1× texels, 40 random clips from each; every option's frames of a sample packed at the package's page size and
gutter, BC7-encoded without RDO by the packer's encoder and Zstd 9'd; each stratum scaled by its texels over its
sample's. Sizes are the ratio to the sample's 1× times the BC7 packages (Classic 867 MB, Enhanced 1.94 GB; the
sample's own 1× estimate is 3–9 % above them). The 1× frames stay in any package with SPAN, since the game draws
them below 1.5×; the RIFE sequences replace the clips (×4 is 3.2–3.7×, not 4×, since in-betweens are trimmed).
Times are GPU time on this machine, by clips, plus 2–25 minutes of packing, BC7 and Zstd on all threads. The upscaler
submits and reads back each frame alone, 1.6–1.8× the game's cost model; a batched path could approach that
(SPAN alone: Classic 32, Enhanced 56 minutes).

| option | Classic package | Enhanced package | both | conversion (both) |
|---|---:|---:|---:|---:|
| 1× (BC7 + Zstd 9) | 0.87 GB | 1.94 GB | 2.8 GB | — |
| SPAN 2× (+ the 1× frames) | 4.4 GB (5.0×) | 9.3 GB (4.8×) | 13.7 GB | 2.5 h |
| RIFE ×2 | 1.6 GB (1.9×) | 3.8 GB (2.0×) | 5.5 GB | 2.1 h |
| RIFE ×4 | 2.7 GB (3.2×) | 7.2 GB (3.7×) | 9.9 GB | 4.9 h |
| SPAN 2× + RIFE ×2 (+ the ×2 sequences) | 8.2 GB (9.5×) | 18.4 GB (9.5×) | 26.6 GB | 7.0 h |
| SPAN 2× + RIFE ×4 (+ the ×4 sequences) | 14.2 GB (16.4×) | 34.3 GB (17.7×) | 48.5 GB | 14.5 h |

In game, frames made ahead of time take the same atlas pages as the ones made at run time (the 2× frames 4× the
texels, in the upscaled pool), so VRAM barely changes: they save the SPAN workspaces (5.6–89 MB each while in use,
within the driver's 64 MiB blocks with BC7) and the RIFE scratch (6 MiB). What they save is GPU time (up to the 2 ms
budget a frame while sequences are made) and the time to 2× or to the in-betweens (seconds) for a read and decode of
about 4× (SPAN) to 13–14× (SPAN with RIFE ×4) the clip's bytes, 2.3–4.5 ms a 4 MiB BC7 page on one thread. They also fix the
choice: the in-between mode (crossfade, ×2, ×4) and SPAN on or off would be the package's, unless it stored each,
and in-betweens in a package need a format field the clips lack (their frame timing).

Kept at run time: RIFE and SPAN both fit the shared 2 ms budget with frame time within noise, and the packages stay
2.8 GB. Ahead of time, RIFE ×2 alone (+2.7 GB) is the only option of the size of one more package, and it would
still remove the run-time choice. A frame too large for the run-time SPAN canvas (above 256², the stygian) would be
the case for a few upscales made ahead of time.

## Sharpening

The CPU FSR 1.1 EASU and RCAS are in `lib/support/uot-image-postprocess/src/upscaling/fsr.rs`; a WESL port of EASU
(unwired) is in [`proposals/fsr-upscaling/`](proposals/fsr-upscaling/). There is no GPU RCAS; the land shader has its
own unsharp mask (`apply_sharpening`).

## Block-compressed frame atlas (2026-11-21)

The frame atlas takes its pixel format from the animation package it shows: a `Rgba8888` package's frames go to an
`Rgba8UnormSrgb` atlas exactly as before, a BC7 package's to a `Bc7RgbaUnormSrgb` one, and RIFE's in-betweens to the
atlas of their clip, in its format. The format is one value of the page-manifest header, read once
(`MobileAnimCcPackage::pixel_format`, `MobileAnimEcPackage::pixel_format`, `MobileAnimClips::page_pixel_format`).

- **One atlas per format, not per source** (`frame_atlas.rs`, `AtlasFormat`). Both packages of one format share one
  atlas, so `Rgba8888` packages keep today's single 32 MiB atlas and its behaviour. A Classic package and an Enhanced
  one of different formats make two, each with its own 8 pages of 1024² and its own eviction: RGBA8 32 MiB, BC7
  8 MiB. Per source would double the atlases of the common case (both packages of one format) for nothing.
- **Sampling**: the art material binds the RGBA8 atlas as `art_atlas` and the BC7 one as `art_atlas_enhanced` (each
  the other when there is one). A BC7 rect's instance layer carries `ART_LAYER_ENHANCED_ATLAS_BIT`
  (`AtlasRect::instance_layer`); the crossfade's next frame reads the same atlas as its frame (`main.wesl`).
- **Rects**: BC7 rects start on multiples of 4, are padded to whole blocks and lie a block (4 texels) apart; RGBA8 ones
  keep the 1-texel gutter. An empty page is zeros, a transparent block.
- **Source frames**: `MobileAnimClips::stored_clip` hands a clip's frames as stored. A BC7 frame's blocks
  (`StoredFramePixels::Bc7Blocks`, out of the page memo, no decode) are written into the atlas as they are, a block row
  per `bytes_per_row`. Every frame of the converter's `Bc7Oriented` packing starts on a block corner, and its extruded
  gutter is outside the copied blocks.
- **In-betweens** (`rife_in_betweens/render.rs`): an in-between of a BC7 clip goes from the staging rows to rows of its
  own in a 1024² `Rgba8Unorm` scratch texture, cleared around it. After the frame's steps the warm-up's
  `MobileBlockEncoder` (BC7 UltraFast) encodes them all in one pass, the one `compress` of the submission (the crate
  writes its uniforms through the queue), each at its scratch row through `texture_y_offset` (BC7 honours it;
  BC1–BC5 don't, so `add_frame_at` refuses a row for them). Their blocks are copied into the atlas a block row at a
  time (a multi-row copy needs 256-byte rows). A pair's first frame is copied block by block through a 1 MiB bounce
  buffer, since a copy within one texture layer is not allowed. A frame takes the BC7 steps the scratch rows and the
  bounce buffer hold and leaves the rest to the next. The cost model adds 0.25 ms + 4 ms per megapixel for the encode
  of each in-between, inside the same 2 ms budget; the measured scale corrects the rest. The BC7 work holds 6 MiB
  more in the render world (scratch 4 MiB, blocks 1 MiB, bounce 1 MiB).
- **BC1** stays in `uot-gpu-compute` (with its punch-through pass) behind the crate's `bc1` feature, which no app
  turns on: no atlas uses it.

Measured on the Radeon 880M (RADV), Britain, four styles, Classic human 400 on horse 200, dog 217, Enhanced daemon 9
and stygian 826, crossfade / RIFE ×2 / ×4:

- **Packages** (`--bc7-rdo --zstd=9`, λ 0.05, look-back 64; RDO is now off by default, see
  [atlas_page_compression.md](atlas_page_compression.md#bc7-rdo-on-mobile-animation-pages)): Classic 869 MB
  (RGBA8888 921 MB), 1210 pages, 983 s to pack; Enhanced 1.93 GB (2.70 GB), 2291 pages, 2048 s (RDO 1884 s). Strides are keyed to the frames' pixels, so none
  carried over (0 of 1914 and 0 of 4125): until `measure-mobile-strides` runs on the BC7 packages, clips step at the
  client timing.
- **RIFE GPU time per frame**, encode included (timestamp queries, a frame with in-between work): the same with both
  formats. KR style: median 1.75–2.1 ms, max 2.2–2.7 ms (one 4.3 ms outlier on the stygian in both), cost scale
  1.05–1.25 BC7 and 1.07–1.45 RGBA8. The encode fits in the 2 ms budget, which then makes fewer in-betweens a frame.
- **VRAM**: the atlas is 8 MiB instead of 32 (both packages BC7), plus the 6 MiB of the BC7 in-betweens: 18 MiB
  less. With one package of each format both atlases exist: 40 + 6 MiB. The process's DRM fdinfo can't show it: the
  driver sees the allocator's blocks (steps of 64 MiB), and both runs showed 946.9 MiB at the start; the BC7 run
  took one more 64 MiB block when RIFE began (KR style peak 1263 MiB against 1203).
- **Looks**: RGBA8 and BC7 captures side by side are the same at 1× and 2×, the in-betweens included; no block
  misplaced, no seams at the rects' edges. Mixed packages (Classic BC7, Enhanced RGBA8) draw from both atlases at once.

The evaluation that chose BC7 UltraFast is `lib/support/uot-gpu-compute/src/block_encode/tests.rs`
(`evaluate_block_formats`, ignored); its sheets and summary are in `uodynamapper-scratch/bc-eval/`.

- **Encoder**: [block_compression](https://github.com/hasenbanck/block_compression) 0.11.0 (MIT), a WGSL port of
  Intel's ISPC kernels, on wgpu "30" (our 30.0.1, no second wgpu), feature `bc7` (and `bc15` with uot-gpu-compute's `bc1`). `block_encode.rs`
  encodes the sRGB bytes through an `Rgba8Unorm` view, so a `*UnormSrgb` BC texture decodes to the same bytes and
  samples like the RGBA8 atlas; the GPU decode matched the CPU one exactly for BC7. Creating the compressor compiles
  all its pipelines (38.6 s cold on RADV in the app, 21 s alone, nearly all of it BC7; 25 ms with Mesa's cache warm),
  so the warm-up makes it off the frame path, on a thread of its own and only while a loaded package is BC7
  (shader_compile_progress.md has each pipeline's time).
- **Loop bounding**: 0.9.1 turned it back on for BC7 (off gave wrong output on RADV/Navi). Off was 1.3–2× faster on
  the 880M with the same output; it would need a vendored bc7.wgsl.
- **BC1**: colour only, four-colour mode in the crate; our `bc1_punch_through.wgsl` re-encodes blocks with transparent
  texels in the three-colour mode, so Classic alpha stays exactly 0/255.

Evaluation results on the Radeon 880M. Times are GPU per frame, encoded alone. "Shown" is the error on colour ×
alpha. "Halo" counts texels transparent in the source that decode with alpha.

| frames | format | GPU ms | RGB PSNR (α>0) | shown PSNR / max | alpha max | halo (max α) | B/px |
|---|---|---:|---:|---:|---:|---:|---:|
| Classic (12, ~3k px) | BC1 + punch-through | 0.007 | 30.0 | 33.8 / 52 | 0 | 0 | 0.5 |
| Classic | BC7 UltraFast | 0.27 | 37.4 | 41.3 / 24 | 1 (opaque → 254) | 0 | 1 |
| Classic | BC7 Fast / Slow | 1.9 / 12.4 | 37.9 / 38.1 | 41.8 / 20 | 4 | 0 | 1 |
| Enhanced (20, ~43k px) | BC7 UltraFast | 0.28 | 33.9 | 38.6 / 80 | 45 | 7090 (32) | 1 |
| Enhanced | BC7 Fast | 2.0 | 35.8 | 40.3 / 91 | 112 (one block) | 5097 (40) | 1 |
| Enhanced | BC7 Slow | 13.4 | 36.1 | 40.5 / 91 | 112 | 5072 (40) | 1 |
| Enhanced | CPU BC7 Analytical (ours, wall ms) | 0.40 | 33.9 | 38.6 / 76 | 48 | 6275 (32) | 1 |
| in-between 64² / 128² (Classic) | BC7 UltraFast | 0.27 / 0.27 | 34.9 / 38.8 | 42.3 / 46.8 | 1 | 0 | 1 |
| in-between 64² / 128² | BC1 + punch-through | 0.008 / 0.009 | 27.1 / 31.5 | 34.6 / 39.5 | 0 | 0 | 0.5 |
| in-between 256² (Enhanced) | BC7 UltraFast / Fast | 0.53 / 3.7 | 33.9 / 35.3 | 43.6 / 44.7 | 42 / 47 | 223 / 173 (16) | 1 |

Observations:

- The halos and the larger alpha errors on Enhanced come from BC7 itself: our CPU encoder shows the same.
- Fast and slower add 0.5–2 dB for 7–50× the time.
- The time of small frames is latency, not size: 64² and 128² cost the same.
- BC1's colour noise on Classic's dithered art is visible at 4× in the sheets, and BC7 UltraFast's is much smaller.
  The user chose BC7 for Classic too.
