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

What each client times a clip by: Classic MUL bodies take a frame every 80 ms in ClassicUO. The client itself advances
them once a mobile tick, which runs when more than 75 ms passed, and only while a step runs: 4 frames a walk step and
2 a run step, so its cycles are locked to the ground ([Human walk and run](#human-walk-and-run-2026-11-22)). Classic UOP
bodies (`mobtypes.txt` flag 0x10000) take
1000 / fps of the action in their `AnimationSequence.uop` entry, aliases resolved (`CMobile_SetUopAction` 0x005b6580,
Classic 7.0.103.0, [classic_frames.md](../client_research/ghidra-classic-frames/results/classic_frames.md#uop-bodies));
those entries are the Enhanced client's, which times every body by them (`GetActionFps`). `mobile_anim_cc.uddp`
stores that fps on the UOP animation records; MUL records and UOP bodies without an entry keep 80 ms.

Choice (user, from `pace_compare.py` videos): **measured stride** over the Enhanced clip timing and over borrowing the
Classic body's cycle. A walk/run cycle lasts stride_tiles × step_secs (on a screen-diagonal facing √2 times that,
[below](#planted-foot-measurement-2026-11-22)), clamped to a third to three times the clip's own cycle speed (half to
twice until 2026-11-22); "client timing" stays as a setting.

Measuring the stride, first (`stride_measure.py`; replaced on 2026-11-22 by the
[planted foot](#planted-foot-measurement-2026-11-22)): RIFE flow between consecutive frames at t = 0.5; in the lowest
20% of the silhouette, the most backward motion (10th percentile) along the facing's screen direction is the planted
foot, i.e. the body's travel; summed over the cycle and divided by the tile's screen length in that facing. Only the
side facings (server directions 4, 5, 6) agree with each other (within ~25% on horses, wyrms, ogres, daemons); facings
toward or away from the camera and all small creatures (dogs, rats, cats) give unusable numbers, which the clamp
absorbs. So the strides are measured offline over every body into a table, not guessed per frame at run time.

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

`udd-cli tool strides measure changed|full FILE...` writes them (`just measure-mobile-strides`; `--body N`
repeatable, `--output DIR`, `--report DIR`), by the planted foot since 2026-11-22 and before by the GPU port of the
prototype (`--method flow`), on the measurement core of
`lib/support/uot-gpu-compute` (`strides.rs`: plain RGBA frames and anchors) driven over the packages by
`udd_conv::animation::mobile_strides`, which the inspector's strides panel and the converter GUI's measure step run too.
It blends the in-between with Practical-RIFE's bilinear warp, as the prototype did, not the in-betweens' nearest one:
the in-between's alpha picks the feet, and the nearest warp's other edges moved Classic body 1's west walk from 1.919
to 1.935 tiles.
Dynamapper only reads the strides. It hashes every
clip on all threads (13 s for both packages), then measures only what it must: `changed` the clips with new
or other frames than their stride's (a clip sharing the frames of a kept one takes its measurement), dropping the
strides of clips gone; `full` everything again (by the flow about 40 minutes on a Radeon 880M, by the planted foot
about two and a half on the CPU). It rewrites a package only when
its strides changed, copying the other entries as stored (3 s for the 2.7 GB Enhanced package, warm cache), and prints
what it measured, unchanged, changed, new, removed and rejected. By hand, `udd-cli tool strides export|import|status`
writes a package's strides as a tab-separated table (`source body action south south-west west stride hash`), imports
one (rows whose hash no longer matches the frames are skipped; rows without a hash need `--trust-unhashed`), and lists
the stale ones. A repack with `udd-cli pack-mobile-anims` / `pack-ec-mobile-anims` keeps the strides of the package it
replaces (or of `--strides-from`) whose frames still hash the same, or are made from the same client frames: the
packers record each clip's hash next to the hash of the client frames it was made from, decoded before the frame filter,
trim, scale or encode (`metadata/stride_sources.bin`, UDDP_FORMATS.md §3.1), so a filter change in
`MobileFrameFilters.kdl` carries the strides over to the new hashes. Clips whose client frames changed, and new clips,
get none until measured. `udd-cli tool strides sources FILE... --ccdir|--ecdir` records the entry in a package packed
before it existed (3 s for the Classic test package, 16 s plus 9 s writing for the Enhanced one). The test set got it
so on 2026-11-23, its exported strides unchanged; a repack of copies with other filters on a few bodies (Classic 1, 9,
200 median or highlight; Enhanced dragon 12/59 back to highlight, 1 none, 200 median) kept all 1914 and 4125 strides,
18 and 14 of them carried to new hashes, the exported values equal.

The stride cache (2026-11-25, `udd_conv::animation::stride_cache`) spares a fresh pack the measuring: the conversion
tables ship `MobileStrides.tsv`, every stride measured on the packages of the client versions at hand, keyed by
package kind, body, action and the clip's source hash, at full `f32` precision (Rust's shortest round-trip text), so
it holds for any pixel format, Zstd level, RDO, frame filter or scale, as the carry-over does, and a modified client's
clips just miss. Every packer (`pack-mobile-anims`, `pack-ec-mobile-anims`, the GUI jobs, Quick Convert) fills from
it in `keep_repacked_strides`, on the source hashes it computes anyway. Precedence, per clip: a stride carried from
the package replaced (or `--strides-from`, or Quick Convert's destination, carried after the pack into the staged
copy) wins, since it was kept for these very frames and may be a measurement the user chose; the cache fills the
clips still without one; the measure option then measures what neither gave (`Changed clips`, which Quick Convert
runs, skips the hashing when the package's records cover every clip of its `stride_sources.bin`), or `All clips`
everything again. `--stride-cache` (CLI) or the routing folder's copy (GUI, when it has one) replaces the shipped
table. `udd-cli tool strides cache-export PACKAGE... [--ccdir] [--ecdir] [--merge TABLE]... [--output]` writes the
table from measured packages (`measure full` first), the client frames read from the client folder when given, else
from the sources the package records; the same key with other values is an error, and with `--merge` alone it merges
tables. The shipped table (2026-11-25, 6817 rows, 387 KB) merges four client folders, each packed with Quick Convert's
settings (BC7, no RDO, Zstd 9) and measured in full: Classic 1914 clips and Classic "fp" (anim6.mul,
AnimationFrame6.uop) 2073, of which 1725 share their client frames, Enhanced 4125 and Enhanced "fp" 4398, 3968 shared;
the shared ones measured the same to the bit, so the merge met no conflict. 718 Classic and 1616 Enhanced rows hold no
stride (measured, none fit), so those clips are not measured again either. A fresh pack of each folder with the table
then took every stride from it (0 misses; `measure changed` measured nothing in 0.1 to 0.5 s) and exported strides
byte for byte equal to the measured package's; the stride part of the pack is the source hashing, 1.6 to 6.7 s Classic
and 7 to 12 s Enhanced at idle priority, against 65 to 92 s and 129 to 182 s of measuring in full.
The first strides, measured into the table that was bundled in the binary until 2026-11-21, were moved into the
packages with `strides import --trust-unhashed` from the measurement report, hashed on the packages they were measured
on, without measuring again. The stride is the median of the side facings that measured (at least two), even when they
disagree: the clamp bounds a poor number, and rejecting a spread over 60% of the median turned away the clips the
user judged from the prototype videos (Enhanced wyrm 46, stygian 826) and most Enhanced creatures. Each record keeps
each facing's stride, and the generator prints their spread for inspection. A clip whose median is not positive or
under a quarter tile is recorded without a stride and keeps the client timing: bodies without planted feet (gazers,
slimes, wisps) and the smallest creatures measure a few hundredths of a tile, and the clamp would play them at twice
their speed. The 2026-11-21 strides pace 882 of 1914 Classic clips with frames and 1627 of 4125 Enhanced ones; the
planted foot's 1339 and 2649. Riders never use a stride of their own: an Enhanced mount's clip is
paced from the mount's stride (a Classic mount's from its run's own time, see below) and the rider shows the same point
of that cycle, with the by-eye Enhanced mounted cycles as the fallback. The cycle position is a phase accumulator (`MobileCycleClock`), so a cycle length that changes
mid-walk doesn't make the frame jump.

### Classic mounts gallop (2026-11-22)

The user found the Classic horse's legs too fast on a mounted walk (human 400 on horse 200). Measured on the recording
and the clip: the ground went by at 5.0 tiles/s (a mounted step is 0.2 s, a run's step on foot), and the horse played
its walk, a trot, at 5 cycles a second, its stride (0.90 tiles) times the step clamped to twice its own speed. The
trot's hooves, followed by hand on the side facing (they move back 6–12 px a frame), cover about 1.05 tiles a cycle,
so 5 cycles a second kept them on the ground: the walk art, made for a horse walking on foot (a tile in 0.4 s, its
own 0.4 s cycle), can only keep up with a mounted walk by scurrying. Both clients play it at 80 ms a frame (2.5 cycles
a second, ClassicUO `ProcessAnimation`; the Classic client advances the frame once a mobile tick of over 75 ms and
mounted changes only the step time, [classic_frames.md](../client_research/ghidra-classic-frames/results/classic_frames.md#pacing-and-idle)),
and the hooves slide forward at half the ground's speed.

Matched to the ground, a ridden Classic mount on the move now plays its run (a gallop; the animal layout's action 1)
and the rider its fast ride (24), on a mounted walk too, one cycle in the run clip's own time (0.4 s) on a mounted
walk and in half of it on a mounted run (`MobileCycle::LeadClipTimes`): the gallop's planted hooves travel 18–25 px a
frame on the side facing, 2.0–2.8 tiles a cycle, the two tiles of a mounted walk's 0.4 s. At client timing nothing
changes (walk at a mounted walk, the clients' frame time). The measured strides no longer pace Classic mounts: the
flow loses the hooves of five-frame clips, which jump 10–25 px a frame (per-pair travel of the horse's trot on the side
facing 7.0, 8.4, -2.2, 6.0, 16.8 px; of its gallop 0.9, 17.5, 2.2, 12.5, 14.6, summing to 1.08 tiles where the hooves
travel more than twice that). Enhanced mounts keep their measured strides (ten-frame clips); clamped to half their
own speed they slide on the move (horse 200: trot 1.44 tiles in 0.56 s against 5 tiles/s).

The port must feed RIFE exactly what the prototype did: the grey backdrop (RGB over 0.5 grey) covers the whole canvas
the clip's frames share, and only the padding up to a multiple of 64 stays black. With the grey on each frame's own
rectangle only, the flow near the feet changed and Classic body 9's walk came out 2.66 tiles instead of 2.79; with the
canvas grey the port matches the prototype to 0.01 tiles.

### Human walk and run (2026-11-22)

The user found the walking human's legs too fast for how far it moved. The flow measures feet short, and the human
clips are the worst. Each stride below was found by hand on the south-west facing (straight left, 44 px a tile), by
following the planted foot's ground contact from frame to frame on the dumped frames. The Classic walk's contact
moves back 8, 8, 7, 3, 2 px a frame, the run's 9.5, and the Enhanced walk's and run's 6.6 and 8.5 (frames of 9 at 9
fps). A template match of the ground blobs gave the same within 0.1–0.2 tiles. In the table, slip is cycles/s × the
hand stride − the ground speed: positive, the legs run ahead of the ground; negative, the feet slide forward.

| Clip | Ground | Flow stride | By hand | Before: c/s, slip | After: c/s, slip |
|---|---|---|---|---|---|
| Classic 400/401 walk (0) | 2.5 t/s | 0.78 / 0.87 | 1.3 | 2.50 (clamped), +0.75 | 1.92, 0 |
| Classic 400/401 run (2) | 5.0 | 1.40 / 1.55 | 2.15 | 2.50 (clamped), +0.37 | 2.33, 0 |
| Enhanced 400/401 walk (1) | 2.5 | 1.16 / 1.24 | 1.4 | 2.00 (clamped), +0.30 | 1.79, 0 |
| Enhanced 400/401 run (3) | 5.0 | 1.24 / 1.19 | 1.75 | 2.00 (clamped), −1.50 | 2.86, 0 |
| Enhanced 400 war walk (15) | 2.5 | 1.09 | 1.3 | 2.00 (clamped), +0.10 | 1.92, 0 |

The flow measures short on most bodies, though by different amounts. Unchanged by this fix (flow / by hand,
side facing): Classic orc 17 walk 0.88 / 1.3 (legs 0.75 t/s ahead), ogre 1 1.92 / 2.25, daemon 9 2.79 / 2.8; Enhanced
orc 17 1.55 / 2.4 (+1.37), wolf 225 1.04 / 1.55, ogre 1 1.83 / 2.55, daemon 9 2.73 / 3.13, horse 200 walk 1.44 / 1.36.
The big bodies come close and the small and middle ones measure 0.6–0.8 of their stride. The humans' clips now take
strides measured by hand (now the runtime spec `runtime_specs/mobiles/Strides.kdl`, [curated strides](#curated-strides-2026-11-22)). The table covers the Classic walk and run and
the Enhanced walk, run and war walk of bodies 400–403, 605 and 606; the elves' flow numbers match the humans' to 1%,
and so do their contacts. A hand stride replaces the package's stride for the clip, still checked against the package
record's frame hash, and paces the cycle outside the clamp, which only bounds a measured number: the Enhanced
run's 0.35 s cycle is a third of its own. The [planted-foot measurement](#planted-foot-measurement-2026-11-22)
reproduces these within about 0.1 tiles except the Enhanced run, so they stay; the diagonal facings 4 and 6 measured
shorter than 5 because the flow divided their travel by a diagonal step's 31.1 px instead of a side step's 44.

The clients (Ghidra): Classic 7.0.103.0 locks the frames to the steps. The frame counter +0x1a0 moves once a tick,
only while a step runs, and a step lasts the `animinfo.mul` ticks, 4 walking and 2 running for every body; missing
or 0 entries also read 4 and 2 (`AnimInfo_Load` 0x0056a970). A 10-frame cycle therefore covers 2.5 tiles walking and
5 running, and the human's legs move at about half the ground's pace. ClassicUO's 80 ms a frame gives 1.25 cycles a
second. Enhanced 4.0.102.3 plays 1 / fps of the sequence, and nothing on the way reads the movement
([classic_frames.md](../client_research/ghidra-classic-frames/results/classic_frames.md#walk-pace),
[uosa_animation.md](../client_research/ghidra-animation/results/uosa_animation.md)). Neither client matches feet to
the ground, so client timing stays as it is and keeps its own speed.

### Planted-foot measurement (2026-11-22)

`udd-cli tool strides measure` now follows the planted foot by default (`--method planted-foot`; `--method flow` is
the GPU flow above), deterministically on all CPU threads: both test packages in 46 s (Classic) and 89 s (Enhanced)
instead of about 40 minutes of RIFE. Code: `lib/support/uot-gpu-compute/src/strides/planted_foot.rs` (no GPU, kept
beside the flow because it shares the clip and measurement types). For each pair of frames on the anchor-aligned
canvas (the last with the first too), every opaque pixel of the ground band (the lowest 15% of the silhouette across
the travel; a diagonal facing's band runs 30 degrees off the horizontal) finds where its 5x5 patch went in the next
frame among the backward shifts (side facing: along the screen x, ±1.5 px vertically; diagonal: within a 10–60
degree cone). Pixels whose best patch is within a mean RGBA difference of 20 vote for its backward x; the pair's
travel is the most voted shift (at least 4 votes), refined by the votes' mean over it and its two neighbours. The
still body votes for no shift and the forward swing for none backward, so the most voted one is the largest region
sliding rigidly back: the planted foot. A facing's stride is the median pair travel × the frames, over the side
tile's screen x along the facing (it needs a travel on 30% of the pairs); the clip's is the median of the facings.

Strides are now in side tiles: the planted foot's screen travel a cycle over the 44 px of a step along the screen's
horizontal, in every facing. The art moves the feet as far on the screen in every facing, but a step along a screen
diagonal (server directions 0, 2, 4, 6) covers 31.1 px, so the app (`step_side_tiles` in `render/mobiles/pace.rs`)
takes √2 times as many steps for a cycle on those facings. The flow divided a diagonal facing's travel by 31.1 px
and the app used every stride as tiles per step, which is why the diagonal facings measured "shorter" and why a
creature walking east played its cycle √2 too fast.

Where the hand numbers came from: they were not measured by a tool. An agent dumped the clips' frames, read the
planted foot's ground contact off the images frame by frame on the south-west facing (eye and pixel counting), and
cross-checked with a throwaway template match of the ground blobs; several Enhanced numbers are that match's
medians, and it misread the Classic horse's trot. They carry about ±0.1–0.2 tiles. The prototype
(`planted_proto.py`, scratch) tried blob tracking and the vote; the vote was ported, and its band, tolerance, votes,
per-pair reduction (median, mean, interquartile mean) and sub-pixel refinement were tuned in Rust over 92 settings
against the hand table (`stride_hand_compare.py`). Reproduce: `udd-cli tool strides measure full
mobile_anim_cc.uddp mobile_anim_ec.uddp --report DIR` (or `--body N` for these bodies with `--output DIR`), then
`python3 stride_hand_compare.py DIR/*.tsv`.

| Clip | Hand | Planted foot (S, SW, W → stride) | Δ | Flow |
|---|---|---|---|---|
| Classic 400 walk (0) | 1.30 | 1.33 1.34 1.68 → 1.34 | +0.04 | 0.78 |
| Classic 400 run (2) | 2.15 | 1.96 2.10 2.43 → 2.10 | −0.05 | 1.40 |
| Classic 401 walk (0) | 1.30 | 1.29 1.34 1.67 → 1.34 | +0.04 | 0.87 |
| Classic 401 run (2) | 2.15 | 1.97 2.04 2.18 → 2.04 | −0.11 | 1.55 |
| Enhanced 400 walk (1) | 1.40 | 1.38 1.40 1.35 → 1.38 | −0.02 | 1.16 |
| Enhanced 400 run (3) | 1.75 | 1.74 2.14 1.83 → 1.83 | +0.08 | 1.24 |
| Enhanced 400 war walk (15) | 1.30 | 1.20 1.21 1.17 → 1.20 | −0.10 | 1.09 |
| Enhanced 401 walk (1) | 1.40 | 1.42 1.43 1.35 → 1.42 | +0.02 | 1.24 |
| Enhanced 401 run (3) | 1.75 | 2.03 2.15 1.76 → 2.03 | +0.28 | 1.19 |
| Classic horse 200 trot (0) | 1.05 | 1.08 1.38 1.73 → 1.38 | +0.33 | 0.90 |
| Classic horse 200 gallop (1) | 2.40 | 2.40 2.27 5.16 → 2.40 | 0.00 | 1.08 |
| Classic dog 217 walk (0) | 0.95 | 0.64 0.52 0.64 → 0.64 | −0.31 | none |
| Classic wolf 225 walk (0) | 1.00 | 1.11 0.69 0.93 → 0.93 | −0.07 | 0.52 |
| Classic orc 17 walk (0) | 1.30 | 1.31 1.49 2.12 → 1.49 | +0.19 | 0.88 |
| Classic ogre 1 walk (0) | 2.25 | 2.86 2.43 2.22 → 2.43 | +0.18 | 1.92 |
| Classic daemon 9 walk (0) | 2.80 | 3.01 3.34 3.04 → 3.04 | +0.24 | 2.79 |
| Enhanced horse 200 walk (22) | 1.36 | 1.59 1.36 1.47 → 1.47 | +0.11 | 1.44 |
| Enhanced horse 200 run (24) | 3.80 | 3.48 4.02 5.35 → 4.02 | +0.22 | 2.33 |
| Enhanced dog 217 walk (22) | 0.91 | 1.03 0.88 0.98 → 0.98 | +0.07 | 0.68 |
| Enhanced wolf 225 walk (22) | 1.55 | 1.54 1.36 1.49 → 1.49 | −0.06 | 1.04 |
| Enhanced orc 17 walk (22) | 2.40 | 2.51 2.48 2.33 → 2.48 | +0.08 | 1.55 |
| Enhanced ogre 1 walk (22) | 2.55 | 2.88 2.37 2.54 → 2.54 | −0.01 | 1.83 |
| Enhanced daemon 9 walk (22) | 3.13 | 2.71 2.98 2.75 → 2.75 | −0.38 | 2.73 |

Mean |Δ| 0.13 (flow 0.50), 12 of 23 within 0.1 (flow 2), 18 within 0.25, largest 0.38; the BC7 test package gives
0.133. Where it disagrees: the hand numbers are the south-west facing alone while the stride is the facings' median
(the Classic orc's SW 1.49 against the hand's 1.3 is a real disagreement, its S 1.31 agrees; the Classic trot's SW
1.38 against 1.05, which the hand table already doubted); the Enhanced human run has a planted foot in only about
half its frame pairs and their travels spread 3–12 px (the hand count read 8.5 px a frame, the vote 9–10.5); the
Classic dog's feet are a few pixels, so the band holds little else than the body; the Enhanced daemon's wings reach
into the band. Five-frame Classic runs (wolf, dog) jump 20–40 px a frame and mostly find no planted foot on two
facings: they get no stride and keep the client timing, about right for their two-tile gallops, where the flow's
0.4–0.6 played them at the clamp's fastest.

Decisions: planted foot is the default (packers, `udd-cli tool strides measure`, the inspector and the converter GUI);
the human hand strides stay ([curated strides](#curated-strides-2026-11-22)), since the Enhanced run does not reproduce; the clamp is now a third to three times
the clip's own speed (`MIN_OWN_CYCLE_FRACTION`, `MAX_OWN_CYCLE_FRACTION`): the planted foot only needs a guard
against outliers, and a Classic monster runs its walk at the run's 0.2 s steps (the orc: 0.37 of its own cycle on a
side facing), which half would hold back. Re-measured in place with `full`: `/home/claudio/test/uddp` (1339 of 1914
Classic clips with frames have a stride, 2649 of 4125 Enhanced; the flow's 882 and 1627) and the BC7 atlas test
packages (1349, 2613).

The Classic orc (the user found it too fast on the walk-pace clip): recorded walking and running east, a screen
diagonal, on the test packages before and after (`video_pace.py` on the recordings: the ground's scroll and the
silhouette's period). The ground was right, 2.47 tiles/s walking and 4.5–4.7 running (the 0.4 and 0.2 s steps; the
run climbs a slope), and the run used the right clip: Classic monsters run their walk (action 0) at the run's steps,
as ClassicUO does. The legs were not: walk and run both played 2.50 cycles a second, the flow's 0.88 tiles × 0.4 s
held at the clamp's 0.4 s (twice the clip's own 0.8 s) and its run likewise. The planted foot covers 1.49 side tiles
a cycle, 2.11 tiles along a diagonal, so the legs went at 5.3 tiles/s over a 2.5 tiles/s ground: twice too fast
walking, 1.06 times running. Now the walk plays 1.19 cycles a second and the run 2.38 (measured; 1.18 and 2.37
expected), the legs at the ground's speed. The Enhanced orc (walk 22, run 24) went from 1.61 and 2.60 cycles a second
to 0.71 (expected; the walk's period outlasts the recording's segment) and 1.54 (measured). The recordings' foot
tracker (`video_feet.py`) reads the planted foot moving with the ground in every segment, before and after alike: it
locks on the ground showing through the mask, so slip is computed from the measured cycles and the strides instead.
Small creatures (wolf, dog) have too little colour for the silhouette period; their rates follow from the strides:
the Classic wolf's walk from 4.8 cycles a second (the flow's 0.52 tiles at the clamp) to 1.9, its run from 5 to the
clip's own 2.5.

#### Re-measure (2026-11-22, evening)

Every package set on the machine with stride records was looked for: `/home/claudio/test/uddp` and the BC7 atlas
test packages (`/home/claudio/rust-target/uodynamapper-scratch/bc7-atlas/uddp`); the client folders hold none, and
the other `.uddp` files are maps, art and the KR delta. Both sets were already in planted-foot units (Classic 400
walk 1.34, against the flow's 0.78), and `udd-cli tool strides measure full` over them changed no record, so nothing
was written. The stride TSVs before it are in
`/home/claudio/test/uddp-backups/strides-before-curated-remeasure-*/`.

#### Curated strides (2026-11-22)

The runtime spec `dynamapper/assets/runtime_specs/mobiles/Strides.kdl` (until 2026-11-23 `CURATED_STRIDES` in
`render/mobiles/pace.rs`, formerly `HAND_STRIDES`) holds the clips whose stride stays hand-configured because the
tool does not follow them, each with its reason as a comment above it. An entry is a stride in side tiles, paced
outside the clamp, or client timing: the clip plays at its own frame time. Like a measured stride, a curated one
needs the package's stride record and the frame hash; a client-timing entry needs neither.

The file is read strictly through `SpecFiles` (`CuratedStrides::decode` in `pace.rs`), so a user copy at
`cache/runtime_specs/mobiles/Strides.kdl` takes its place without a rebuild; the copy is checked once a second and
read again when saved, and the clips' checks start over when what is in force changed. Its header documents the
format; in short, keyed by the in-game (server) body ids and the package (`cc` Classic, `ec` Enhanced):

    stride <body> [<body> ...] package="cc"|"ec" action=<action> tiles=<tiles>
    client-timing <body> [<body> ...] package="cc"|"ec" action=<action>

`action` is the clients' table number of the movement (below 100, `CURATED_ACTION_LIMIT`), `tiles` 0.1 to 12
(`CURATED_STRIDE_TILES`). A body's action is listed once per package; an unknown node or property, a missing one, a
value out of range, no body or a body listed twice is an error naming the line. The `pace` tests read the bundled
file (`include_str!`) and check every entry's value, that each has a comment above it, and the 17 entries.

How the list was made: `udd-cli tool strides inspect PACKAGE --body B --action A` prints each side facing's pairs
(travel, votes, band pixels) and the facing's stride; `--strip DIR` draws the frames side by side with the ground band
blue, the voting pixels red and a bar the pair's travel long under each frame; `--facing F --pair P ...` counts the
stride on chosen pairs alone. A survey ran it on about 70 common bodies in both sources (humans, elves, gargoyles,
mounts, orc, ogre, troll, daemon, lich, skeleton, zombie, ettin, dragons, wolves, dog, cat, horses, llama, bears, deer,
birds) and flagged a clip with few planted pairs (under half), a large spread across facings (over 0.5 of the
median) or pairs, five frames, or a voting region off the feet. Each flagged clip's strip was looked at: the pairs
whose red sits on one foot touching the band in both frames were counted with `--pair`, and the foot's x was read off
an enlarged south-west strip with a 5 px ruler as the independent count (for the Classic dog and wolf also the
silhouette's ground contacts, matched frame to frame). The value is the south-west count, the facing the human counts
used; an entry is kept when it differs from the tool by more than a tenth, or the tool has no foot to follow.

| Clip | Curated | Tool (S, SW, W → stride) | How it was counted, and why the tool is not trusted |
|---|---|---|---|
| Classic humans, elves walk (0) | 1.30 | 1.33 1.34 1.68 → 1.34 | Kept from the first hand count. |
| Classic humans, elves run (2) | 2.15 | 1.96 2.10 2.43 → 2.10 | Kept; half the pairs lose the foot. |
| Enhanced humans, elves walk (1) | 1.40 | 1.38 1.40 1.35 → 1.38 | Kept. |
| Enhanced humans, elves run (3) | 1.75 | 1.74 2.14 1.83 → 1.83 | Kept; about half the pairs show no planted foot, theirs spread 3–12 px. |
| Enhanced humans, elves war walk (15) | 1.30 | 1.20 1.21 1.17 → 1.20 | Kept. |
| Classic horses 177, 200, 204, 226, 228 trot (0) | 1.05 | 1.08 1.38 1.73 → 1.38 | The leading hoof's contact x on SW: 30, 40, 49, 61, 67 px over frames 0–4, 9.25 px a frame (the old hand count, confirmed). The trot's diagonal hooves swap within a pair: three of five pairs match another hoof (26.7 and 17.2 px). |
| Classic horses gallop (1) | 2.25 | 2.40 2.27 5.16 → 2.40 (4.11 on 226 and 177, and on 200 in the BC7 set) | SW pairs 0 and 1 (`--pair` 2.25), the front hoof at 24, 44, 64 px: 20 px a frame. The other pairs match another hoof (59.5 px), and the S facing's median of two pairs flips between 2.4 and 4.1 with the package. |
| Classic wolves 23, 25, 27, 34, 98, 225 walk (0) | 0.70 | 1.11 0.69 0.93 → 0.93 | SW pairs 1–4 (`--pair` 0.69); the contacts' shifts sum to 31 px (0.70). Five frames, 3 px paws: pairs of four votes read 8 and 11 px. |
| Classic dog 217 walk (0) | 0.52 | 0.64 0.52 0.64 → 0.64 | SW pairs 1–4 (`--pair` 0.52); the contacts' shifts sum to 24 px (0.55). 2 px paws: S and W find two and three of five pairs. The old hand count, 0.95, was wrong. |
| Classic flying run (19): eagle 5, bird 6, daemons 9 and 10, dragons 12 and 59, harpy 30, wyrms 49 and 106, drakes 60 and 61, wyvern 62 | client timing | 10–16 tiles from one facing, or none | A flight: no foot touches the band, the votes sit on a wing tip. |
| Enhanced daemons 9, 10 run (24) | 6.8 | 5.15 5.80 1.65 → 5.15 | Pairs 4 and 5 on every facing (S 6.54, SW 6.82, W 7.34), the planted foot's blob; the wings sweep through the band in pairs 1–3 and on the west facing. |
| Enhanced kirin 132 run (24) and ridden run (30) | 6.9 | 5.86 4.22 7.76 → 5.86 | Pairs 1 and 6 (S 6.30, SW 6.88, W 7.76); the front hoof's heel on SW at 48 then 78 px, 30 px. The hoof touches down for one pair; the lift-off pairs read 2–8 px. |
| Enhanced ostards 210, 218, 219 run (24) | 6.2 | 5.86 – 9.21 → 7.54 | SW pairs 7, 8 (6.18), S 6, 7 (6.28), W 7, 8 (6.84). Two of ten pairs on SW, so the stride was the mean of S and W. |
| Enhanced flying run (24): dragons 12 and 59, harpy 30, wyrms 49 and 106, drakes 60 and 61, wyvern 62 | client timing | 0.43–5.1, or none | A flight, as above; the white wyrm hovers. |
| Classic cows 216, 231 and bulls 232, 233 walk (0) | 1.0 | 1.46 0.53 3.49 → 1.46 (S 0.32–1.78, W 3.0–3.5 across the four) | Counted 2026-11-23: the front hoof's contact on SW moves back 8–10 px a frame, 9 px on average over the five frames (1.0); the bulls are drawn a little larger, in the cows' poses frame for frame (the tool's SW 0.53–0.56 on all four). The hooves step out of phase: while a front hoof moves, a rear one holds its canvas x for three frames, and the SW votes sit on it (0.53). The south facing's 30 degree band misses the front hooves, and the west facing's pairs match one hoof to the next. |
| Enhanced orc captain 7, and 138, 139 (its frames) run (24) | 1.9 | 3.10 1.90 7.11 → 3.10 | Read 2026-11-23 off the SW strip: a bounding run, in the air for frames 5 and 6. Three SW pairs are stance pairs (0–1, 2–3, 3–4: the trailing foot in the band in both frames, 3–12 px) against two that sweep the swinging leg through the band (1–2 and 4–5, 22–33 px), so the facing's median is a stance pair's, 12 px a frame (1.9). On S and W the sweeping pairs reach the median (3.1 and 7.1; `inspect` reads W 4.4 today). |

Looked at and left to the tool: the ridden ostards' run (30, 7.0: three pairs per facing, all agreeing within 0.2),
the ostards' walk (22, 3.37: five steady pairs per facing), the Enhanced trolls' run (S 2.98 and W 3.10 agree with
SW's two pairs, 3.11), the Enhanced bears' walk (0.75–0.79 on every facing), the Classic cat (walk 0.35, contacts
0.34–0.45), the Enhanced horses' run (4.0, against the old hand count 3.8). Left with no stride, at the clients'
timing, because no pair shows a clear planted foot: the five-frame Classic runs of the dog, cat and wolves, and the
Enhanced runs of the skeletons (one 25 px pair on SW, the next lifting), wolves, dog, cat, cow and hind.

The Classic orc (17, 7, 41, 138–140) and lich (24, 78, 79, 82) walks read about 0.6 tiles longer on the west facing
(orc S 1.31, SW 1.49, W 2.12; lich 1.12, 1.46, 2.01), and the Classic people's walk too (1.68 against 1.34). The
cause (2026-11-23): on the pairs around the cycle's wrap the lifted foot, toe down, matches the other foot's toe about
20 px behind it, a second "planted" foot; a facing has only about five stance pairs, so one or two of these lift its
median a rank. The mid-stance west pairs agree with the south ones (people 5.0–6.6 px against 4.4–6.4). The clip's
stride is the median of the three facings, the south-west's here (1.49 and 1.46), so nothing changes in game and no
entry is needed. Two fixes in the matcher were measured over all bodies of both test packages and dropped, because
they moved good results more than they mended these: requiring the match to hold back from the second frame within
1 px (west/south 1.157 to 1.100 on the Classic clips, but 162 Classic and 233 Enhanced strides moved by 0.2 or more,
150 lost, and the hand counts' mean distance from 0.130 to 0.222 tiles), and requiring the matched spot to hold still
in the second frame (the reverse match at least half the travel: the orc's west 2.12 to 1.53, the lich's 2.01 to
1.63, but 70 Classic and 125 Enhanced strides moved by 0.2 or more, the Classic alligators' run (202, 133) on the
south facing from 1.31 to 0.37 among them, 139 lost, and the hand counts from 0.130 to 0.135, 11 within a tenth
against 12).

The Classic armed walk and run (1, 3) measure as the unarmed ones (1.34, 2.10) and the war walk (15) 0.68, but the
renderer plays neither and the packages have no records for them: they would need records and the renderer's armed
and war modes. Left as they are.

Reproduce an entry with `inspect --strip` and the `--pair` lists in the table (frame numbers start at 0 and a pair
is named by its first frame). The survey and the sheet, ruler and contact scripts were throwaway scratch.

What changes in game, a cycle on a side facing at the steps of the movement (×√2 on a diagonal): the Classic horse's
walk from 1.38 to 1.05 tiles a cycle, its run from 2.40 (4.11 on bodies 226 and 177) to 2.25, the Classic wolves'
walk from 0.93 to 0.70 and the dog's from 0.64 to 0.52, the Enhanced daemon's run from 5.15 to 6.8, the kirin's from
5.86 to 6.9 on foot and ridden, the ostards' run on foot from 7.54 to 6.2, and the flying runs now flap at the
clients' frame time. Recorded headless through the control socket on the test packages, before and after, walking
and running along a screen diagonal and along the screen's horizontal: Classic (horse, wolf, dog, dragon), Enhanced
(daemon, kirin on foot and ridden, ostard, dragon), KR-like (ostard) and UOD-Painterly (daemon), side-by-side mp4s in
`/home/claudio/rust-target/uodynamapper-scratch/strides-curated/`. The silhouette-period tracker (`video_pace.py`)
locks on the recordings' six-frame repeats here, so the rates are the strides' rather than measured.

The tracker is now `udd-devtools-cli video-pace <video> --half N` (`--halves`, `--shifts <tsv>`), the script ported
to Rust: an `ffmpeg` pipe of raw frames, the frames, pairs, match offsets and autocorrelation lags in parallel. On
the Enhanced ostard recording it prints the script's numbers to the digit; on the daemon's the ground matches and
two near-flat autocorrelations (0.01 to 0.13) settle on another peak, its sums in f64 against the script's f32. It
measures a view of 1941 frames in about 75 s against the script's six minutes, and gives a stretch too short for a
period none instead of failing, as the script did on the ridden kirin. The regression
harness (quick tier, all four styles): 99 captures, 76 identical and 23 within tolerance, none changed.

The cattle's and the orc captain's entries (2026-11-23): the Classic cow's walk from 1.46 to 1.0 tiles a cycle (the
bulls' from 0.85 and 0.56, the brown cow's from 1.78), the Enhanced orc captain's run from 3.10 to 1.9. Recorded the
same way, Classic for the cow and Enhanced for the captain, with the Classic orc and lich walking west unchanged:
side-by-side mp4s in `/home/claudio/rust-target/uodynamapper-scratch/strides-rest/`.

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

`world_rendering.mobile_upscale` is `off` (default), `balanced` or `always` (since 2026-11-23; settings written as
`span_x2` read as `balanced`): in Preferences, and `set mobile_upscale` on the control socket. Off by default: it costs GPU time and VRAM the zoomed-out view never shows, and it is a look the user
has not chosen yet for every style. Code: `render/mobiles/upscales.rs` (main world) and `upscales/render.rs`; the
network and its frame steps are uot-gpu-compute's `upscale`.

- **Gate**: the view's magnification is the window's scale factor over `RenderZoom` (zoom 0.5 on a scale-1 display is
  2×). Upscales start at 1.5× and stop below 1.35×, so a zoom around the threshold does not flap
  (`UPSCALE_ON_MAGNIFICATION`, `UPSCALE_OFF_MAGNIFICATION`). Below it the 1× frames draw as before, and the queued
  jobs stop.
- **Which bodies (per body, 2026-11-23)**: Always upscales every body; Balanced the bodies
  `runtime_specs/mobiles/Upscale.kdl` picks (below), by default the rule that follows. A body upscales whole or not at all, so its sharpness does not jump between
  actions (it did with a rule per clip: a Classic dragon 12 upscaled its walk but not its attack). A body upscales
  in Balanced only when every sequence it can show fits the size rule's canvas (the file's `max_canvas`, 260×260
  by default and at most, with the 2-pixel border, sides rounded up to 8): the clip of every action and direction in its package (`mobile_anim_cc` for
  Classic, `mobile_anim_ec` for Enhanced), or in a RIFE mode each clip's RIFE sequence, whose in-betweens sit on the
  card of two neighbouring frames on one anchor, wider or taller than either. Each sequence has a canvas of its own,
  so a wide clip and a tall one don't add up. A rider and its mount are two bodies, each with its own rule. The
  sizes come from the frame records (`MobileAnimClips::body_actions`, `clip_extents`), with no pixels read, once per
  body drawn while upscaling is on (about 0.2 ms, 0.7 ms at most). In Balanced a body over the rule plays at 1× at every zoom; its
  RIFE in-betweens have no size limit and are made as for any body. A future filter on the frames (a denoise) should
  likewise take every body. Excluded in Balanced at 260 with the 2026-11-22 packages:
  - Classic, 49 of 1030 bodies with frames (54 in a RIFE mode): 9 10 12 13 38 40 43 46 47 49 59 60 61 62 77
    102–106 131 162–164 172 175 180 197 198 735 755 790 797 798 826 1248 1288 1417–1422 1427 1431 1531 1532 1545
    1760; a RIFE mode adds 189 285 796 999 1433.
  - Enhanced, 60 of 1145 (76 in a RIFE mode): 8 9 10 12 38 40 43 46 49 59 62 66 102–106 123 128 129 157 172 173 175
    176 180 197 198 242 265 273 276 728 735 777 782 794 797 798 804 807 808 826 1068 1071 1246 1248 1288 1400 1404
    1417–1422 1427 1431 1432 1434; a RIFE mode adds 47 76 256 285 308 312 666 667 694 695 741 746 780 785 1405 1433.
  - So the dragons 12 and 59, the wyrms 46, 49 and 180, the daemons 9, 10, 38, 40 and 43, the wyvern 62, the giant
    beetle 735 and the great bodies 197, 198, 826, 1248 and 1288 play at 1× in both packages; the drakes 60 and 61
    only in Classic, 1400 and 1404 only in Enhanced. The blue fiend 11, the horse 200 and the human 400 upscale in
    both. Every excluded body had some clips upscaled before (none had all), so the rule only took the 2× away from
    the actions of bodies that were mixed.
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
  budget spreads over several frames; its workspace stays with it until it ends. No workspace is larger than 260×260
  (90 MB, ~30 ms): a larger sequence upscales in 256×256 tiles (below).
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
  ~290 MB of workspace). Tiling a large frame over a small canvas would lift the cap (done 2026-11-23, below). Since 2026-11-23 the daemon 9
  above plays at 1× too: its body is over the cap (above).
- **Looks**: at 2.5× the 2× frames are sharper and smoother at the edges, Classic edges stay hard, BC7 and RGBA8 look
  the same, the placement matches the 1× frames, and at zoom 1 nothing is upscaled. Side-by-side videos (RIFE x4,
  walking then standing): `uodynamapper-scratch/span-ingame/videos/*-off-vs-span.mp4`.
- The regression harness with upscale off (quick tier) matches the commit before: 76 identical, 15 within tolerance.

### Modes, Upscale.kdl and tiles (2026-11-23)

The user's decisions: three modes, Off, Balanced (the per-body rule above, now from a file) and Always (every body,
whatever its size); large bodies upscale in tiles without growing the workspace.

- **`runtime_specs/mobiles/Upscale.kdl`** (`upscales/spec.rs`, strict, every error naming its line; a copy under
  `cache/runtime_specs/mobiles/` takes its place, read at startup). `default max_canvas=<side>` is the size rule
  (8 to 260, 260 without it; the code's `UPSCALE_MAX_CANVAS_SIDE` is both the fallback and the bound, since a side
  past the workspace would only mean tiles). `body <id> [<last id>] [package="cc"|"ec"] upscale="always"|"never"` lists
  in-game (server) body ids, a range, in one package or both; a package's entry comes before one for both, and a body
  listed twice in the same scope is an error. The bundled file holds only `default max_canvas=260`, so Balanced is
  today's rule.
- **Always ignores the `body` entries**: it is the "upscale everything" choice, and a `never` that still held would
  make it "Balanced without the size rule", a fourth mode nobody asked for; a body the user wants at 1x everywhere is a
  Balanced `never`. Written in the file's header and the Preferences hover text.
- **One decision point**: `MobileUpscales::body_upscales(key, packages, rife, mode)` and its `body_choice`; the size
  rule's canvases are found only when Balanced asks for an unlisted body. A package that carries a body's 2× frames
  (pack-time upscaling, planned) answers before it.
- **Tiles** (uot-gpu-compute `upscale::tiles`): a canvas larger than its workspace is cut into windows of the
  workspace's size (256×256, `UPSCALE_TILE_SIDE`, the largest multiple of 8 within 260×260), each keeping as its core
  only what lies at least the network's reach inside its inner edges: 21 pixels, one per 3×3 convolution (1 + 3×6 + 1
  + 1 layers); the bicubic alpha's reach of 2 is within it. A window's source is the frame read at an origin of its
  own (`set_frame_at`; the pixel shader reads `origin_x/y` instead of the border). The cores tile the canvas exactly,
  and the GPU test `tiles_stitch_into_the_whole_canvas` (150×118 frame, 64² workspace, two regions) matches the whole
  canvas's upscale bit for bit on the shader cores and the matrix units: 0 of 75152 texels differ.
- **Parts** (an atlas limit): an upscaled sequence's frames each fit an atlas page at 2×, so a frame at most 510 canvas
  pixels wide or tall (BC7; 511 RGBA8: the page's room less its gutter, halved). A sequence wider than that is stored
  in parts split at the same anchor-relative columns in every frame, each part its own atlas entry and drawn as a card
  of its own (`Vec<Clip>` per sequence); a frame taller than that plays at 1× with a warning (no body's is: every
  frame is at most 480 tall). Classic 12, 49, 59, 105, 106, 180, 198, 826, 1531, 1532, 1545 and 1760 and Enhanced 12,
  59, 197, 198, 797 and 826 have frames wider than 506 pixels (frames up to 640×480), so they draw in two parts. At a
  part's edge each card's filter clamps at its edge instead of blending across it, a sub-pixel seam at most.
- **BC7 frames made in several passes**, or from a part's columns, are put together in an RGBA8 1024² texture per
  workspace (cleared before the first pass, copied into the BC7 scratch rows after the last) and encoded once; RGBA8
  passes copy straight into the atlas. The compute budget counts each pass as a frame step, and the BC7 encode only
  on the last.

- **The log** (debug) names each upscaled sequence's canvas, parts and passes when it is reserved, and its time to 2×.

Measured 2026-11-23 on the Radeon 880M (profiling build with the span trace, 1900×1000, zoom 0.5 = 2×, frame limiter
off, crossfade, the 2026-11-22/23 packages, west of Yew at 2111,790; scripts, captures and traces in
`uodynamapper-scratch/upscale-spec/`):

- **Balanced**: human 400 on horse 200 upscale (0.02–0.04 s to 2×); the Classic dragon 12 gets no job and plays at 1×.
  With a user copy listing `body 12 package="cc" upscale="always"`, Balanced upscales the Classic dragon and leaves the
  Enhanced one (not listed for `ec`) at 1×.
- **Always, what tiles**: the clips a player can show (stand, walk, run) mostly fit 260² whole, even on the largest
  bodies: the Classic dragon's stand (192×216 canvas) and walk (200×224). Tiled on 256² windows: the Classic dragon's
  run (21 passes for 10 frames), Classic 197 and 198 standing (4 passes a frame), Classic and Enhanced 826 and
  Enhanced 197 and 198 standing (48 passes for 12 frames). No clip reachable there needs parts (the frames wider than
  506 pixels are in other actions); parts are covered by the unit tests, and the captures of every listed wide body
  standing show no seam between tiles.
- **Time to 2×** (unloaded): the Classic dragon's tiled run 1.6 s per direction at ~160 fps; Classic 826 standing
  3.7 s, Classic 197 1.7 s; in the Enhanced style (~16 ms frames) Enhanced 197, 198 and 826 standing 8.6–8.9 s,
  Enhanced dragon 12 (whole) 2.4 s. The budget is per frame, so the time follows the frame rate: with other agents'
  GPU load (frames ~20 ms) the dragon's run took 10.3 s and Enhanced 197 13.7 s.
- **Frame time**, one session alternating Balanced and Always three times (Classic dragon standing 8 s, running east
  and west 6.7 s, standing 10 s), from the trace's frame spans: the same in both, median 6.0–6.3 ms, p95 7.3–8.2 ms,
  p99 8.0–9.6 ms, none over 33 ms, also in the window where Always made the tiled run (453 SPAN submissions, median
  1.87 ms, max 2.17 ms on the GPU). Classic is CPU-bound here. In the Enhanced style, GPU-bound, Enhanced 197's
  upscale raised the median from 14.3 to 16.1 ms (p99 16.2 to 18.1 ms) while it ran: the 2 ms budget.
- **Switching** between Balanced and Always keeps the atlases (both have the upscaled pages), so 2× frames made in one
  stay for the other.
- **Looks**: the four styles draw the 2× dragon at the 1× placement; Classic edges stay hard. Side-by-side clip of the
  Classic dragon running, 1× against its 2× tiles (crossfade on, so both show a fading wing between frames):
  `uodynamapper-scratch/upscale-spec/dragon-1x-vs-tiled-2x.mp4`.

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

## Faster networks: GPU kernels and CPU inference (measured 2026-11-22)

Whether RIFE and SPAN run faster with more aggressive GPU kernels, and whether a CPU inference (AVX2 or AVX-512)
could add throughput, on the Radeon 880M (RADV, Mesa 26.2.7, wave64) and the Ryzen AI 9 365 (4 Zen 5 + 6 Zen 5c
cores, 20 threads, AVX-512, boost off: 2.0 GHz). GPU times are timestamp queries of uot-gpu-compute's ignored
`benchmark` tests; CPU times are wall time of a scratch prototype (`uodynamapper-scratch/cpu-gpu-nets/`) and of
PyTorch 2 on oneDNN. Other agents' jobs loaded the CPU during the CPU runs (load average up to 33), so the
multi-thread CPU numbers are pessimistic; the one-thread ones are steady.

**Shipped: SPAN on the matrix units** (`ConvUnits::Matrix`, `shaders/upscale/conv_matrix.wgsl`). Where the device
has wgpu's experimental cooperative matrices with 16×16×16 fp16 products into f32 sums (AMD WMMA on RDNA 3, NVIDIA
tensor cores), SPAN's 3×3 and 1×1 convolutions run as an implicit GEMM on them: a workgroup of 256 threads (4 wave64
subgroups) makes 64 pixels × 48 channels, staging 16 input rows (channel × tap) at a time in workgroup memory as fp16,
pixel-major for A and channel-major for B, rows padded by 4 halves; the next step's loads go to registers while the
current one multiplies; bias, SiLU and the attention residual are the epilogue. The device is asked for the features
where the adapter has them (`own_device_and_units`; Bevy's render device has all adapter features already), and the
adapter's cooperative-matrix properties decide (`ConvUnits::of`), since wgpu enables the feature for any shape.
Elsewhere the f32 shader-core kernels run as before. dynamapper's cost model needs no change: its `CostScale` learns
the faster convolutions from the timestamps.

SPAN, one frame (the canvas, with its border), ms:

| canvas | GPU f32 (before) | GPU matrix (shipped) | PyTorch CPU fp32 1 / 4 / 10 threads | Rust AVX2 1T / AVX-512 1T | Rust AVX-512 4T / 10T |
|---|---:|---:|---:|---:|---:|
| 48×40 | 1.63 | 0.80 (2.0×) | — | — | — |
| 64² (Rust: 68²) | — | — | 82 / 30 / 27 | 197 / 132 | 127 / 148 |
| 80×85 | 5.0 | 2.53 (2.0×) | — | — | — |
| 128² | 7.8–8.2 | 5.57 (1.4×) | 277 / 81 / 76 | 758 / 474 | 414 / 385 |
| 256² | 28.4–29.6 | 20.8 (1.4×) | 1135 / 350 / 273 | 2698 / 1851 | 1223–1252 / 818–862 |

- Throughput: f32 shader cores 600–1000 GMAC/s, matrix 1240–1400 GMAC/s (the kernel is bound by its loads and
  barriers, not by WMMA). CPU: the Rust kernels reach 10–12 GMAC/s a thread with AVX2 and 15–20 with AVX-512 (about
  half the 2.0 GHz core's FMA peak), PyTorch 10 threads about 100.
- Error against PyTorch's fp32 (dog 45×36, horse 80×85): matrix 0.20 and 0.29/255 max (fp16 operands, f32 sums), f32
  shader cores 0.0004/255, Rust CPU 0.0004/255. The reference test allows 0.5/255 for the matrix path.
- Cold compile (empty Mesa cache): the matrix pipelines take 110–290 ms for their 6 compiles against 800–890 ms for
  the f32 upscale pipelines' 10, so the warm-up is about 0.6 s shorter (1.02 s in all with the matrix path).
- In game (dev build, 1900×1000, zoom 0.4, the Enhanced daemon 9 walking then standing, the four styles, RGBA8888
  and BC7 packages; the same build with the shader cores forced, alternated with it): with crossfade (SPAN alone) its
  two actions reach 2× in 1.1–1.8 s instead of 1.7–3.1 s in Enhanced, KR-like and Painterly, about 1.7× sooner, in
  fewer submissions; the cost scale settles at 0.7–1.4 instead of 1.0–1.5. In the Classic style the scale sits near 2
  with either units and the times (0.1–5.7 s) show no clear change; with RIFE ×2 the time to 2× follows RIFE's own
  (unchanged) and stays 0.1–8.7 s either way. The budget holds as before (median 1.45–1.6 ms timed, rare peaks of
  2.5–5 ms), and frame time is the same: 60 fps (vsync) in Classic, Enhanced and KR-like, 18–22 ms in Painterly, with
  one 34 ms and one 25 ms sample of the matrix runs that the other package's run did not repeat. Captures look as the
  f32 path's (scripts in `uodynamapper-scratch/cpu-gpu-nets/ingame/`).
- Tuning on 256²: 16 rows a step (32: 21.0–21.4 ms, 64: 23.4, 96: 32.6); 128 threads as fast (20.6), 512 slower
  (23–25); rows padded by 4 halves (2: 4× slower from misaligned loads, 8: slower). Without padding (bank conflicts)
  38.9 ms; the first, row-major A/B kernel 85 ms. Same results under wave32 (`RADV_PERFTEST=cswave32`).

RIFE v4.25, one in-between (Head encode of both frames included), ms; CPU Rust is its convolutions alone (random
weights, same shapes), PyTorch the whole network:

| frame | GPU f32 (unchanged) | PyTorch CPU fp32 1 / 4 / 10 threads | Rust AVX2 1T / AVX-512 1T | Rust AVX-512 4T / 10T |
|---|---:|---:|---:|---:|
| 64² | 1.39 | 19.6 / 15.2 / 11.8 | 11.7 / 7.9 | 9.0 / 7.8 |
| 128² | 2.00 | 48.9 / 27.7 / 21.1 | 35.8 / 23.1 | 21.9 / 17.9 |
| 256² | 4.78 | 154 / 65 / 63 | 125 / 79 | 79–176 / 236–322 |
| 576×512 | 17.0 | 608 / 286 / 245 | 550 / 321 | 338–430 / 354–685 |

- RIFE on the GPU is dispatch-bound below 256²: about 120 dispatches an in-between at about 10 µs each are 1.2 of the
  1.3 ms at 64². At 576×512 the five blocks take 0.86 / 1.36 / 2.38 / 4.38 / 5.46 ms, and the last two (64 and 32
  channels over most pixels) run at 300–400 GMAC/s, the f32 kernels' worst case.
- The Rust convolutions do not scale past a few threads at RIFE's low-resolution blocks (a few rows each, one rayon
  task per row) and under the other jobs' load; one thread is the steady figure.

**bf16 / fp16 on the CPU: rejected on quality.** PyTorch's bf16 autocast is 1.1–1.8× faster than fp32 but its error
is far past the 0.64/255 colour tolerance: SPAN 2.4–3.0/255 max (0.36/255 mean); RIFE on the reference frames (dog,
wyrm, t = ¼, ½, ¾) 1.5–33/255 max on the merged frame and up to 0.47 px of flow. fp16 operands with f32 sums (the
matrix path) are fine for SPAN; fp16 sums were not tried: the error grows with the 432 products a 3×3 output
over 48 channels adds up.

**Evaluated, not shipped:**

- *Shared-memory tiling, register blocking, channel-last layouts, workgroup tuning*: the matrix kernel is all of them
  (tuning above). The f32 kernels already tile in workgroup memory and block outputs in registers; they were not
  retuned.
- *Packed fp16 dot products / fp16 shader-core arithmetic*: on RDNA 3 WMMA is the packed fp16 path (1.4–2× above);
  a `dot`-of-`vec2<f16>` kernel would be for devices with fp16 but no matrix units, where it would also need its
  own error check. Not written.
- *Subgroup operations* beyond the cooperative matrices (which are subgroup-scoped): not needed, as the kernels
  share inputs through workgroup memory.
- *Winograd F(2,3)*: 2.25× fewer multiplies for SPAN's 3×3 convolutions, but the matrix kernel is load-bound, not
  multiply-bound, and the transforms add loads and fp16 error; on the f32 path it would be a third kernel family. Not
  written.
- *Fusing small dispatches* (RIFE's 120 an in-between): the gain is up to ~1 ms of the 1.4 ms at 64², which the 2 ms
  budget already absorbs (several sequences a frame); a persistent kernel per block is a large change. Not done.
- *RIFE on the matrix units*: estimated from its blocks' rates, 576×512 would go from 14.8 to about 10 ms an
  in-between (blocks 3 and 4 at SPAN's matrix rate); its convolutions take 15 or 28 input channels, have strides of 2
  and transposed convolutions of 52×4 outputs, so it needs its own tiles and split-K.
  The follow-up if RIFE on large frames matters; small frames would not gain.

**A CPU worker: not worth it.** Even with 10 cores the CPU is 8–14× slower than this iGPU (PyTorch, the better of
the two CPU paths), and the game has no cores to spare at those rates: the background pool also decodes, unpacks
(Zstd) and builds statics, whose spikes are the walks' CPU cost. On 4 threads, about what the pool could give,
a CPU worker as fast as oneDNN would add about 3 SPAN frames of 256² a second (the Rust prototype 0.8), or about 3.5
RIFE in-betweens of 576×512, against the GPU's 8–16 or 10–20 in its 2 ms a frame at 80–160 fps: a fifth to a third
more, for 4 cores kept busy. Raising the GPU budget from 2 to 2.5 ms a frame gives as much for nothing on the CPU.
A weaker GPU (no matrix units, an older iGPU) is still several times this CPU, and one without compute is not a
dynamapper target (wgpu). The udd tools' ahead-of-time path runs on the GPU too.

**The swap model would take a slower worker as is.** A clip plays plain until its whole RIFE sequence is made, then
switches to it (the sequence spans the clip's cycle, so it lands on the same point); a layer switches to its 2×
frames only when all are made and only as its sequence starts over (`UpscaleSwitch`). Nothing waits on a job, so a
CPU job would only show its result later. What it would take: the source frames in RAM (they only live in the GPU
atlas now, so a second decode of the package's frames), the networks in Rust (SPAN's convolutions are the ~400 lines
of the prototype; RIFE adds its warps, transposed convolutions, flow and mask merging), the results uploaded into the
atlas and BC7-encoded there, and a scheduler sharing jobs between the two. For at most a third more throughput,
which a larger GPU budget gives too, it is not worth that.

## RIFE on the matrix units, and the compute budget (measured 2026-11-22)

**Shipped: RIFE's block convolutions on the matrix units** (`shaders/rife/conv_matrix.wgsl`, the parts shared with the
shader-core kernel in `conv_layer.wgsl`). SPAN's implicit GEMM (above) carried over: 256 threads, 16 rows (input
channel × tap) a step staged as fp16 in workgroup memory, the next step loaded while the current one multiplies; a
tile is 16 × subgroups pixels (64 on wave64) by 16 to 64 output channels (one to four 16×16 matrices), the narrowest
that covers the layer's outputs. Strided, residual and the last pixel-shuffled transposed convolutions run there, with
the split-K of the shader-core kernel and its reduction. `ConvUnits::of` decides per device, as for SPAN, and the
shader-core kernels serve devices without matrix units. These stay in f32 on the shader cores on every device:

- *The Head* (the encoder of both frames, whose features every block reads): in fp16 the merged frame moved up to
  2.35/255 from PyTorch, past the 2/255 tolerance, and the Head ran no faster (16 output channels: gather-bound).
- *Grids under 8 pixels* (block 0 of 64² and 128² frames, 1 to 4 pixels by 192 channels): 0.021 ms on the shader
  cores, 0.028 on the matrix units.
- The strides (`strides.rs`) keep the shader cores too, so a measured stride does not depend on the device.

Error against PyTorch's fp32 on the reference frames (dog, wyrm, t = ¼, ½, ¾): flow at most 0.018 px (tolerance
0.05), merged frame at most 1.17/255 (2/255), mask logit at most 0.034, which the reference test allows up to 0.05 on
the matrix units (0.01 on the shader cores): through the sigmoid, at most 0.0125 of the blend's weight.

One in-between (the Head's encode included), ms, `rife::tests::benchmark` on the Radeon 880M; the quieter of two
alternated rounds:

| frame | dev 5c1b85d7 | shader cores now | matrix units | pair of 3 t, shader cores → matrix |
|---|---:|---:|---:|---:|
| 64² | 1.42 | 1.41 | 1.06 (1.33×) | 4.08 → 3.02 |
| 128² | 2.05 | 2.01 | 1.73 (1.17×) | 5.74 → 4.88 |
| 256² | 4.96 | 4.82 | 4.31 (1.12×) | 13.6 → 12.1 |
| 576×512 | 17.72 | 17.49 | 14.57 (1.20×) | 48.0 → 39.3 |

- At 576×512 the five blocks take 0.79 / 1.35 / 2.09 / 3.55 / 4.10 ms against 0.87 / 1.38 / 2.44 / 4.55 / 5.61; the
  convolutions as a whole 8.3 ms against 12.2. The ResConvs run at 1.1 to 1.4 TMAC/s, SPAN's matrix rate; the
  pixel-shuffled last convolution at 0.5 (its writes are strided).
- What is left is not convolution: the pixel steps (warps, the blocks' input resampling: 1.25 ms for block 3's, 0.82
  for block 4's, 0.71 for block 2's at 576×512, the upsamples, the merge 0.38) and the Head (2.3 ms at 576×512). They
  are the next lever for large frames; small frames stay dispatch-bound (about 120 dispatches an in-between).
- 32 rows a step instead of 16: no faster at 576×512, slower at 64² and 128². Not kept.
- dynamapper's cost model is unchanged: in game the timed cost scale settled at 0.94 to 0.98 with the matrix units
  against 1.03 to 1.27 with the shader cores forced, so the estimates stay within a few percent and the budget holds
  (timed median 1.93 to 2.01 ms a frame, either units).

Time to a RIFE ×2 sequence in game (dev build with the timings logged at info, 1900×1000, zoom 1, RGBA8888 packages,
the same build with the shader cores forced alternated with it, one round; seconds from the clip's first frame drawn to
its sequence made; Classic dog 217 and horse 200 in Classic art, Enhanced daemon 9 and stygian 826 in Enhanced art,
each walking then standing):

| style | units | dog | horse | daemon | stygian |
|---|---|---|---|---|---|
| Classic (6.2 ms a frame) | matrix | 0.04 | 0.05 0.29 0.44 | 0.14 0.29 | 0.68 0.28 |
| | shader cores | 0.04 | 0.05 0.30 0.43 | 0.17 0.32 | 0.92 0.32 |
| Enhanced (17 ms) | matrix | 0.08 | 0.10 0.46 0.60 | 0.36 0.71 | 1.88 0.70 |
| | shader cores | 0.09 | 0.13 0.52 0.70 | 0.42 0.80 | 2.13 0.81 |
| KR-like (20 ms) | matrix | 0.10 | 0.12 0.55 0.74 | 0.44 0.87 | 2.64 0.85 |
| | shader cores | 0.10 | 0.14 0.63 0.85 | 0.51 0.96 | 2.24 0.85 |
| UOD-Painterly (36–42 ms) | matrix | 0.20 | 0.21 0.89 1.22 | 0.70 1.39 | 1.24 |
| | shader cores | 0.21 | 0.34 1.43 | 2.02 1.13 | 2.19 2.23 |

The sequences come 10 to 20 % sooner on the matrix units in most cells, which is the in-between's own gain at these
sizes: the budget, not the network, paces them (2 ms a frame), so the wait is the frames it takes and grows with the
style's frame time. One KR-like stygian walk came later (2.64 s against 2.24); the Painterly rows scatter by more
than the units explain (one round on a shared machine). Frame times matched within 3 % with either units, except
Painterly's (36 against 42 ms median, as noisy as its waits).
The regression harness (quick tier, 99 captures, against dev 5c1b85d7's baseline): 72 identical, 26 within tolerance,
1 changed, the wide Painterly shoreline at 0.013 % (a few water pixels), which the alpha-weighted taps' run against
the same baseline shows too.

**The compute budget stays at 2 ms a frame** (`MOBILE_COMPUTE_FRAME_BUDGET_MS`). Evaluated: 2.5 and 2.25 ms, in a
busy scene, with the matrix units. The player is the only mobile so far, so the scene makes the work: RIFE ×2 and
SPAN 2× on, zoom 0.4, 1900×1000, RGBA8888 packages, the player turned into 35 bodies one after the other (Classic art in
the Classic style, Enhanced in the others), each walking east 1.5 s and west 1.5 s, about 75 to 135 sequences and 50
to 120 upscales wanted in two minutes. The same measurement build took the budget from the environment, and the
budgets alternated, three rounds (2.25 two). The frame time is each frame's own, over the busy two minutes; "over
idle" is its median less the idle 20 s after it. Means of the rounds' figures, ms (Classic without one 2.5 run that
the machine's load slowed throughout, idle included):

| style | budget | median | p95 | p99 | over idle | RIFE made, median wait | 2× made, median wait |
|---|---|---:|---:|---:|---:|---|---|
| Classic | 2.0 | 6.29 | 8.48 | 9.92 | +0.41 | 74, 0.08 s | 117, 0.18 s |
| | 2.25 | 6.22 | 8.62 | 9.92 | +0.33 | 75, 0.08 s | 120, 0.20 s |
| | 2.5 | 6.35 | 8.68 | 10.03 | +0.45 | 75, 0.08 s | 118, 0.17 s |
| Enhanced | 2.0 | 13.88 | 16.29 | 18.31 | +1.37 | 124, 0.17 s | 89, 1.0 s |
| | 2.25 | 14.17 | 16.81 | 19.06 | +2.02 | 124, 0.15 s | 98, 0.77 s |
| | 2.5 | 14.57 | 17.27 | 19.48 | +1.95 | 128, 0.14 s | 98, 0.75 s |
| KR-like | 2.0 | 14.12 | 16.81 | 18.86 | +1.03 | 124, 0.17 s | 90, 0.94 s |
| | 2.25 | 14.49 | 17.48 | 19.83 | +1.21 | 122, 0.16 s | 88, 0.9 s |
| | 2.5 | 14.59 | 17.42 | 19.52 | +1.05 | 126, 0.14 s | 95, 0.86 s |
| UOD-Painterly | 2.0 | 23.37 | 26.32 | 28.04 | +0.70 | 127, 0.30 s | 53, 1.30 s |
| | 2.25 | 23.02 | 26.20 | 28.37 | +0.92 | 129, 0.26 s | 61, 1.22 s |
| | 2.5 | 23.54 | 26.67 | 28.84 | +1.46 | 131, 0.24 s | 69, 1.21 s |

The longest wait for a 2× frame set was 8 to 12 s with any budget (a large body's walk, queued behind the others).

- The threshold for raising it was the median unchanged and the p99 within the run-to-run noise, round by round.
  2.5 ms fails it in Enhanced and KR-like, where the GPU sets the frame: the median rose in every round (Enhanced
  +0.56 to +0.86 ms, 4 to 6 %, about 72 to 69 fps; KR-like +0.40 to +0.61, 3 to 4 %), while the 2.0 runs agreed
  within 0.3 ms (Enhanced) and 0.8 ms (KR-like), and the p99 rose by 0.7 to 1.2 ms. That is the extra half
  millisecond of compute on the frames that run it. 2.25 ms costs about half as much (+0.3 to +0.4 ms median).
- What it buys is less: about 10 % more 2× sets made in the two minutes in Enhanced and KR-like (30 % in Painterly,
  which makes the fewest), a 2× wait of 0.75 instead of 1.0 s in Enhanced, RIFE's waits 0.03 to 0.06 s shorter.
- Classic runs at 155 to 165 fps with the CPU setting the frame, and nothing changes there: the work keeps up at
  2 ms already. Painterly's frame time is the noisiest (the 2.0 runs spread over 2 ms) and shows no clear change.
- So the constant stays. The step cost estimates need no retuning: with the matrix units RIFE's timed scale settles
  at 0.93 to 1.00 and SPAN's at 0.77 to 1.19, and the timed work at 1.96 to 1.99 ms a frame (median) with a 2.0
  budget, 2.21 to 2.23 at 2.25, 2.44 to 2.47 at 2.5.
  The lever, if one is wanted, is a budget that follows the GPU's headroom (larger where the CPU sets the frame, as
  in Classic), not a larger constant.

## RIFE on large frames (measured 2026-11-23)

Every dispatch of `rife::tests::benchmark` timed (matrix units, Radeon 880M), the dev build of c82351c6 alternated
with the changes over two rounds. At 576×512 one t's pixel steps took 5.6 ms of 13.1 (block inputs 2.8, upsamples
1.3, merge 0.35, the split sums' reductions 0.9) and its convolutions 7.5; at 64² every dispatch is 0.003 to 0.016 ms,
so small frames stay dispatch-bound and nothing below changes them.

- **The downscaling blocks' input, a thread per corner.** A block input reads the four frame pixels each of its
  pixels resizes from, and each read is 69 scalar loads (the warps of both frames' colour and features, the mask, the
  state's features, the flow). One thread walking the four corners in turn waited on its loads, worst at the coarse
  blocks, which have few pixels: block 3's input took 1.08 ms at 576×512, block 2's 0.62. Now the blocks of scale
  16 to 2 run a thread per corner, and the four sums meet in workgroup memory, added in the corners' order as before
  (`block_input_corners`); block 4, at scale 1, weighs one corner and keeps a thread per pixel. Block inputs 2.78 →
  1.62 ms (block 3 1.08 → 0.45, block 2 0.62 → 0.26, block 1 0.29 → 0.12).
- **The matrix units' split-K counts threads.** A layer with few tiles splits its sum until it makes about 96
  workgroups, which on the matrix units, of 256 threads, was four times the shader cores' threads. Each split adds a
  reduction, and at 576×512 the reductions came to 0.87 ms a t, 0.63 at 256². Now the matrix units aim for 24
  workgroups (`MATRIX_TARGET_WORKGROUPS`): reductions 0.12 ms at 576×512, 0.22 at 256², with no slower convolution.
  12 and 48 measured no better (one round on a loaded machine).
- Tried and dropped: the frames' colour and the Head's features packed per pixel into two vectors of four, so a warp
  tap reads two loads instead of seven planes (the Head's last convolution writing them interleaved): block inputs
  2.88 → 1.65 ms with the corners, as without the packing, while the Head's last convolution slowed 0.45 → 0.65 ms
  for its strided writes. The upsample of block 4 with one tap instead of four: 0.40 → 0.39 ms.

One in-between, ms (encode + one t), and a pair of three in-betweens:

| frame | before | after | pair of 3 t, before → after |
|---|---:|---:|---:|
| 64² | 1.04 | 1.03 | 2.96 → 2.93 |
| 128² | 1.68 | 1.41 | 4.73 → 3.92 |
| 256² | 4.23 | 3.16 | 11.8 → 8.6 |
| 576×512 | 14.2 | 11.5 | 38.2 → 30.3 |

The shader cores (devices without matrix units) gain the block inputs alone: 576×512 17.0 → 16.0 ms, 256² 4.8 → 4.6.
The largest single dispatches at 576×512 are now block 4's last convolution (0.90 ms), block 4's input (0.77) and
the Head's last convolution (0.46 a frame), all within the 2 ms budget, so a frame needs no slicing to keep it.
Error against PyTorch stays within the reference test's tolerances and where it was (flow at most 0.017 px, mask
logit 0.034, merged frame 1.1/255, against 0.018, 0.034 and 1.2/255 before): the corners' sums are added in the
order they were, and only the matrix units' fewer split ranges move the rounding.

dynamapper's cost model for a RIFE step now counts the pixel steps and reductions by their threads (1.0 ms per
million) and the convolutions' multiply-adds at the rate of the units they run on (1.85 ms per GMAC on the shader
cores, 1.15 on the matrix units), fitted to every dispatch of the benchmark; it used to count the multiply-adds alone
at the shader cores' rate, so a 576×512 block input, 0.8 ms, was estimated at 0.009.

## A budget that follows the GPU's headroom: not shipped (measured 2026-11-23)

Tried: the 2 ms budget grows by 0.25 ms a frame, up to 6 ms more, while the compute has steps held back and the GPU
waited more than 1.75 ms between frames, and falls back to 2 ms the first frame it waited less than 1.5 ms. The wait
was measured with two timestamps per frame while the compute runs: an empty pass before the compute (Queue) and one
after the present (Cleanup), each in a submission of its own. A wgpu-hal Vulkan pass timestamp is written at the bottom
of the pipe, so the frame's end stamp latches when its work is done, and the next frame's start stamp when that frame
reaches the GPU; the wait is the gap.

That gap is not the GPU's headroom here (Radeon 880M, RADV, XWayland under KWin). Busy view of Britain, one round,
frame time median / p95 / p99 in ms, the gap's median, and how often the budget had grown:

| style, limiter | fixed 2 ms | headroom | gap (median, p90) | frames grown | GPU busy (fdinfo) |
|---|---|---|---|---:|---:|
| Classic, off | 6.30 / 8.34 / 9.55 | 6.12 / 7.99 / 9.07 | 0.03, 0.04 ms | 0 % | |
| Classic, 60 fps | 16.66 / 17.76 / 27.32 | 16.66 / 17.89 / 27.54 | 2.1, 3.0 ms | 46 % | 43 to 49 % |
| Enhanced, off | 14.03 / 15.54 / 16.86 | 14.08 / 15.56 / 16.96 | 0.03, 0.05 ms | 0 % | |
| Enhanced, 60 fps | 16.66 / 18.13 / 27.01 | 16.66 / 18.15 / 27.25 | 0.04, 0.07 ms | 0 % | |

At 60 fps Classic keeps the GPU busy less than half the frame (the process's `drm-engine-gfx` time in
/proc/self/fdinfo: 43 to 49 %, about 8 ms of 16.7), yet the gap between its frames is 2 ms; Enhanced at 60 fps, which
renders in 14 ms uncapped, shows no gap at all. The span between a frame's two stamps holds more than its work: most
likely the wait of its render submission for the swapchain image the compositor gives back, which Vulkan does on the
GPU's queue. So the timestamps see the GPU as busy while it waits on the compositor, the budget grew in Classic at
60 fps alone and by 0.2 ms on average, and the sequences came no sooner (2x upscales: max 8.0 against 9.2 s in
Classic at 60 fps, 8.7 against 10.3 in Enhanced, within the runs' spread). Frame times did not move either way. The
second round and the KR-like and UOD-Painterly runs were not made once the signal was shown not to work.

Signals for a later try, each with its catch:

- The process's GPU engine time in /proc/self/fdinfo (`drm-engine-gfx`, nanoseconds, per DRM client): it excludes
  the semaphore waits (the scheduler runs a job once its dependencies are met), but sums overlapping jobs (111 % in
  Enhanced uncapped), exists only on Linux, and is averaged over jobs that straddle frames.
- Timestamps inside the frame's own passes (Bevy's render diagnostics), which run after the swapchain wait: a sum of
  the passes is the GPU's work, at the cost of a query per pass.
- Either one measures time at the clock the GPU chose: amdgpu's governor ran Classic at 60 fps at 1.5 to 1.7 GHz of
  2.9, so the work stretches to fill part of the headroom and what is left looks smaller than it is (the safe side).

Uncapped Classic is CPU-bound, and a larger budget there costs CPU (recording the dispatches) as well as GPU, so a
grown budget should also want the frame limiter to have slept.

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
  `MobileBlockEncoder` (BC7 UltraFast) encodes them all in one pass, one `record` (each record has a uniform buffer of
  its own; block_compression's compressor wrote them through the queue, one `compress` per submission), each at its
  scratch row (`add_frame_at`; our BC7 and BC1 kernels both take a row). Their blocks are copied into the atlas a block row at a
  time (a multi-row copy needs 256-byte rows). A pair's first frame is copied block by block through a 1 MiB bounce
  buffer, since a copy within one texture layer is not allowed. A frame takes the BC7 steps the scratch rows and the
  bounce buffer hold and leaves the rest to the next. The cost model adds 0.25 ms + 4 ms per megapixel for the encode
  of each in-between, inside the same 2 ms budget; the measured scale corrects the rest. The BC7 work holds 6 MiB
  more in the render world (scratch 4 MiB, blocks 1 MiB, bounce 1 MiB).
- **BC1** (`MobileBlockFormat::Bc1`, punch-through alpha) is in `uot-gpu-compute` with the other formats, compiled
  only when first used: no atlas uses it, so dynamapper never compiles it.

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
  Intel's ISPC kernels, at first through the crate on wgpu "30" (our 30.0.1, no second wgpu). Since 2026-11-22
  uot-gpu-compute runs its own copies of the crate's BC7 and BC1 kernels (`shaders/block_encode/`), one pipeline per
  format and BC7 profile, with the profile's modes as constants and its blocks the crate's byte for byte; the crate is
  a dev-dependency, the reference of the tests. `block_encode.rs`
  encodes the sRGB bytes through an `Rgba8Unorm` view, so a `*UnormSrgb` BC texture decodes to the same bytes and
  samples like the RGBA8 atlas; the GPU decode matched the CPU one exactly for BC7. The crate's compressor compiled
  all its pipelines (38.6 s cold on RADV in the app, 21 s alone, nearly all of it BC7; 25 ms with Mesa's cache warm);
  our BC7 UltraFast alone takes 6.6 s. The warm-up compiles it off the frame path, on a thread of its own and only
  while a loaded package is BC7 (shader_compile_progress.md has each pipeline's time).
- **Loop bounding**: 0.9.1 turned it back on for BC7 (off gave wrong output on RADV/Navi). Off was 1.3–2× faster on
  the 880M with the same output; our copy keeps it on, as the crate does.
- **BC1**: colour only and four-colour mode in the crate, which also ignores a frame's row. Our `bc1.wgsl` encodes
  blocks with transparent texels (alpha below 128) in the three-colour mode, index 3 transparent black, so Classic
  alpha stays exactly 0/255 as with the CPU's BC1, and takes a row like BC7.

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

## Ground, collision and draw order of a walking body (2026-11-22)

The user saw the stygian dragon (Enhanced 826) sink into the ground while it walked, and the human, on foot and
mounted, walk through Britain's planter and black statue as if on air. Measured and fixed:

- **Enhanced layer offsets point up** (`render/mobiles.rs`, `layer_screen_offset_px`): `mobile_layer_offsets` gives
  the client's layer positions (the extra offset, the rider offset and the -15 shift), and they were drawn with y down.
  826's walk is action 19, its flying walk with flying animations on (the client's default): frames reaching 100-190
  px below their anchor (the stand's reach 41) and the package's one extra offset, +125. Read as y down, the dragon's
  feet were 207-300 px below its cell, so nearer land covered all but its wings. Read as y up its frames end up to 43 px
  above the cell centre, the downbeat's wing tips 50 px below it, and it flies whole. The same flip lowers every Enhanced body by 30 px: on the same cell the KR-like human's feet stood
  about 28 world px above the Classic human's, and now meet them within 4 px. The sign is measured on these two
  cases, not traced in the client's draw.
- **Draw order while sliding** (`render/mobiles/draw.rs`, `mobile_depth_cell`): a sliding body was ordered from its
  fractional position, a row behind the cell it steps into, so that cell's land (one row nearer, at its z - 2) could
  cover the part of the body below its anchor until the step ended. It is now ordered from the nearer cell of its step
  in each map axis (the ceiling of its position), as ClassicUO's `View.CalculateDepthZ` orders a walking mobile. The
  card itself still slides. This alone did not cure the dragon, whose frames were 200 px underground.
- **Collision** (`world/walk_z.rs`, `body_step`): a drawn body took a step the client blocks anyway, onto the nearest
  floor, since the marker was free to go anywhere. It now stops and faces the step, as the client does: at the
  planter's flower beds (Impassable, z 0, 5 high) and at the statue's pieces (Impassable, z 6) on its pedestals
  (Surface and Bridge, 10 high: a walker stands on them at z 5, half the height, as ClassicUO's pathfinder and the
  servers count a Bridge). A body the client lets out in no direction, as one set inside a rock, still steps as
  asked, so the viewer is never stuck. The marker without a body stays free.
- **Height over a step** (`world/player_movement.rs`, `slide_towards`): the body's height jumped to the next cell's z
  as the step began, so it floated over its cell stepping up and sank into the next one stepping down. It now follows
  the slide evenly, as ClassicUO's `Mobile.ProcessSteps` moves a stepping mobile's `Offset.Z`.

Not read by the walk: multis (houses placed by players) and dynamic items; Britain's buildings are statics and block.
The visual harness hides the player and cannot walk a body, so unit tests guard these fixes, not fixtures. The before |
after videos are in `uodynamapper-scratch/mobile-ground/`: `stygian-before-after.mp4` (KR-like, Enhanced,
UOD-Painterly), `human-foot-before-after.mp4` and `human-horse-before-after.mp4` (Classic, KR-like) and
`human-styles-before-after.mp4` (Enhanced, UOD-Painterly).

### The legs of a ridden Enhanced horse under the land in front (2026-11-22)

After the layer offsets were made to point up, the user saw the legs of a ridden horse in Enhanced hidden by the
land for some frames of its walk. Measured on the recordings (rider 400 on horse 200, walking east): in about half of
each stride the front hooves were cut along a cell edge, where they reached right of and below the cell's centre.
The same happened in KR-like and UOD-Painterly, which draw the Enhanced frames, and not in Classic.

- **Cause.** A mobile's card had one depth: its depth cell's row, priority z + 1 (`render/mobiles/draw.rs`). The
  land of a nearer row (row + 1 at its z - 2) is nearer than the whole card. Its diamond starts at the anchor's
  height, one cell right or left of it, so it covered every pixel of the card below the anchor that lies over it.
  The Enhanced frames reach well below their anchor (a ridden horse's hooves 20-40 pixels, more after the -15 shift
  was made to lower the layers), the Classic frames hardly at all. Standing still, the hooves stay over the mobile's
  own cell and show. Walking, the front hooves reach past it, as far as cell x + 2 while the depth cell moves one
  cell at a time, so they show or hide with the stride.
- **What the clients do.** The Enhanced client (UOSA `MobAnimTriShape::RenderImmediate`, traced in
  [uosa_animation.md](../client_research/ghidra-animation/results/uosa_animation.md#drawing-a-mobile-the-card-and-its-ground-strips))
  draws the frame down to 8 pixels above its anchor at the mobile, then 4 strips of one diagonal cell each, every one
  moved one cell nearer in x and y. ClassicUO slices the Classic frame at its anchor the same way, two rows a slice.
  Both draw the lower part as near as the land it lies over.
- **Fix.** The Enhanced frames' fragments below the cut take the depth of their strip
  (`MobileGroundStrips`, art/depth.wesl `mobile_ground_strip`, applied in art/main.wesl with `MOBILE_FRAMES`): row + 2
  per strip, the same priority z and tie level, so the rider stays over its mount. Below the fourth strip the client
  draws nothing; here the fourth strip takes it, so no frame loses pixels. The depth is per fragment, not per card, so
  the card, its crossfade, its client filter, local lights and shadows are untouched. Classic frames keep one card:
  they do not reach far enough below the anchor, and ClassicUO's slices would change their order against statics.
  A unit test walks every pixel of the strips' reach, at rest and sliding: no flat land at the mobile's z covers it.
- **Kept.** The stygian dragon's flight and the human's height are unchanged. A pixel is never ordered farther than
  before, so the strips only show more of a body. Land rising in front still covers it (a cell of the strip's rows
  more than 3 z above the body, or any cell of a nearer row), as in the client. On a slope this can cover one strip
  and not the one below it, leaving a straight edge across a low wing (the stygian dragon walking up the slope west
  of Britain). Before the fix the land hid both strips there.
- **Statics.** Britain's planter, statue and pedestal still cover a horse stopped behind them, in all four styles,
  as before. A strip is as near as the land of its rows, so a static one row in front whose art reaches below the
  cut would draw behind the legs there, as in both clients.
- **Evidence.** Before | after videos in `uodynamapper-scratch/ec-mount-legs/` (scripts in `scripts/`):
  `horse-enhanced-before-after.mp4` (walk and run in six directions, slopes, the planter, the statue and its
  pedestal), `horse-other-styles-before-after.mp4` (KR-like, UOD-Painterly, Classic unchanged),
  `stygian-before-after.mp4` and `foot-before-after.mp4` (the human on foot and an unridden horse).
