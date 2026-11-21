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

The GPU port of the prototype writes them: `dynamapper measure-strides changed|full` (`just measure-mobile-strides`;
`--source cc|ec`, `--packages DIR` defaulting to the app's `udd_path`, `--output DIR`, `--report DIR`). It hashes every
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
- If adopted: 4× frames cost 16× the atlas memory (tight on a 4 GB card), so store 2× results, only for clips on screen
  while zoomed above ~1.5×.

## Sharpening

The CPU FSR 1.1 EASU and RCAS are in `lib/support/uot-image-postprocess/src/upscaling/fsr.rs`; a WESL port of EASU
(unwired) is in [`proposals/fsr-upscaling/`](proposals/fsr-upscaling/). There is no GPU RCAS; the land shader has its
own unsharp mask (`apply_sharpening`).

## Block-compressed frame atlas (evaluation, 2026-11-21)

The frame atlas is `Rgba8UnormSrgb`: 8 pages of 1024², 32 MiB. BC7 would make it 8 MiB and BC1 4 MiB. Nothing is
wired yet. The evaluation is `render/mobiles/block_encode/tests.rs` (`evaluate_block_formats`, ignored), and its
sheets and summary are in `uodynamapper-scratch/bc-eval/`.

- **Encoder**: [block_compression](https://github.com/hasenbanck/block_compression) 0.11.0 (MIT), a WGSL port of
  Intel's ISPC kernels. It uses wgpu "30", so it builds against our 30.0.1 with no second wgpu. We use its `bc15` and
  `bc7` features. `block_encode.rs` wraps it:
  - It encodes the sRGB bytes through an `Rgba8Unorm` view. An `*UnormSrgb` BC texture then decodes to the same bytes
    and samples like the RGBA8 atlas. The GPU decoding matched the CPU decoding exactly for BC7, and within 2/255 for
    BC1.
  - Frames are padded to 4×4 with transparent texels.
  - Blocks are copied one row of blocks at a time, because a multi-row copy needs 256-byte rows.
- **Crate limits**:
  - Its BC1 is colour only, in the four-colour mode, so it has no punch-through alpha. If both endpoints come out equal,
    the block decodes index 3 as transparent.
  - Our `bc1_punch_through.wgsl` pass runs after it: blocks with transparent texels are re-encoded in the three-colour
    mode, and equal endpoints are fixed. With it, Classic alpha stays exactly 0/255, with no halo and no holes.
  - Its BC1–BC5 colour loads ignore `texture_y_offset`, so each frame sits at the origin of its own source texture.
  - It writes its uniforms through the queue when it records, so use one `compress` per submission.
  - Creating the compressor compiles all its pipelines. The first time on RADV that took 38.6 s; with Mesa's shader
    cache warm it took 25 ms. It must be created off the frame path.
- **Loop bounding**:
  - 0.5.0 turned it off for BC7 to fix an AMD iGPU crash. 0.9.1 turned it back on, because off gave wrong alpha/basic
    output on RADV/Navi.
  - On the 880M, off was 1.3–2× faster (UltraFast 0.27 → 0.21 ms, Fast 1.9 → 1.3, Slow 12.4 → 6.1). It matched the
    bounded output except 2–4 texels out of 863k, with the same metrics.
  - The speed-up needs a vendored bc7.wgsl, because the crate offers no switch.

Results on the Radeon 880M. Times are GPU per frame, encoded alone. "Shown" is the error on colour × alpha. "Halo"
counts texels transparent in the source that decode with alpha.

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
  The user should judge it from the sheets.

**Recommendation (to confirm)**:

- **Format and quality**: BC7 UltraFast for all sources, in one `Bc7RgbaUnormSrgb` atlas array. BC1 for Classic only
  as a low-VRAM option, which needs its own array, since one texture has one format.
- **Generated frames** (RIFE in-betweens, SPAN): encode them on the GPU, where they already are. An in-between costs
  0.27–0.53 ms on top of RIFE's 1.4–4.8 ms. The in-between path would write to a scratch `Rgba8Unorm` texture
  instead of the staging rows.
- **Decoded frames**: the CPU encoder (0.1–0.4 ms wall per frame, multithreaded, about UltraFast quality) or a
  GPU encode after the upload.
- **Atlas changes**:
  - Rects start on multiples of 4 and are padded to whole blocks.
  - The empty-page fill stays zero: a zero BC7 block is transparent black. A zero BC1 block is opaque black, so BC1
    pages need `BC1_TRANSPARENT_BLOCK`.
  - `FRAME_GUTTER` 1 becomes a block wherever filtering can reach past a frame.

**Option: upload BC7 pages without decoding.** These packages are `Rgba8888` pages, while the converter's default is
BC7, and the option needs BC7 packages. Their layout already fits:

- Packing is `Bc7Oriented`: every one of the 996k Classic and 1.55M Enhanced frames starts on a block corner.
- Extents are aligned to 4, and the gutter is 4 texels (extruded edge pixels).

The frame's 4-aligned rect of blocks (`uot_texture_codecs::bc7::extract_bc7_subrect`) then copies straight into a BC7
atlas layer. It needs:

- a public udd-assets call for a frame's raw blocks (page reads and `extract_atlas_subrect_rgba` are crate-private);
- the atlas rect rules above.

The extruded gutter is outside the frame's rect and isn't copied. No decode or encode is left, and the quality is the
converter's BC7.
