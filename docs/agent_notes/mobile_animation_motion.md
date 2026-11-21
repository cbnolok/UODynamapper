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
- A sequence (a clip's frames and their in-betweens) too big for one 1024² atlas page is stored in pieces over
  several pages, so large Enhanced creatures (the daemon at ×4) get their in-betweens too. It stays one unit: losing
  any of its pages drops the whole sequence, which is made again when drawn. A clip alone still needs one page.
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

The table is `dynamapper/assets/data/mobile_strides.txt`, bundled into the binary (`render/mobiles/pace.rs`), written
by the GPU port of the prototype, the ignored test `render::mobiles::pace::measure::write_the_stride_table` (command
in the module doc and the table's header; about 40 minutes for both packages on a Radeon 880M). It is keyed by source,
body and action: walk and run of the peace and war groups and, Enhanced, the mounts' mounted moves, so a war mode
needs no new table. The stride is the median of the side facings that measured (at least two), even when they
disagree: the clamp bounds a poor number, and rejecting a spread over 60% of the median turned away the clips the
user judged from the prototype videos (Enhanced wyrm 46, stygian 826) and most Enhanced creatures. The report lists
each facing and the spread for inspection. A clip whose median is not positive or under a quarter tile has no entry
and keeps the client timing: bodies without planted feet (gazers, slimes, wisps) and the smallest creatures measure a
few hundredths of a tile, and the clamp would play them at twice their speed. The 2026-11-21 table paces 882 of 1914
Classic clips with frames and 1627 of 4125 Enhanced ones. Riders never use a stride of their own: the mount's clip is
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
