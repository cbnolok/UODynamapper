# Enhanced frame speckle

Where the darker specks on bright parts of the Enhanced creature animations come from (the user's report: the
`enhanced-ec-dragon` clip, darker orange spots on the dragon's bright back and tail), how widespread they are, and what
filtering at pack time or at run time could do about them (2026-11-23). Study only; nothing in the production paths
changed. The bench crate is [`ec_frame_quantization/`](ec_frame_quantization/).

## Summary

- **The specks are in the client's frames; our pipeline does not add them.**
  - The package's frames are the source frames texel for texel. `mobile_anim_ec.uddp` stores RGBA8888 pages, and the
    measures of the package frame and the source frame are identical.
  - SPAN 2x and RIFE carry the specks over, and BC7 UltraFast of their output keeps them.
- **They are not palette quantization, and not dithering.**
  - Each `AnimationFrame` entry has one 256-colour palette shared by all of an action's frames.
  - A typical speck lies about 4 palette steps from its neighbours. About 30 palette colours lie nearer the
    neighbours' colour than the speck's own colour does, so a quantizer would have picked one of those.
  - Dither-like windows are 12–16 times rarer than in a re-quantized control (2–4 times in KR).
- **On the dragon they are clipped highlights.** The Enhanced render of dragon 59 is overexposed:
  - 35 % of its interior texels have the red channel at 250 or above, against 4.7 % in the KR client's render of the
    same model;
  - where red is clipped, only green and blue still carry the skin texture's shading, so the texture's brightness
    steps show as hue steps: pale yellow next to saturated orange, read as "darker orange spots where the image is
    brighter";
  - 54 % of the dragon's outliers are in clipped areas.
  - The KR render shows the same skin blotches in the same places, but softly: 21 outliers per 1000 interior texels
    against 57.
- **Elsewhere the "specks" are mostly texture.** Bodies like the scaled dragons 197 and 198, the stone golem 829 and
  the turtle 1288 owe their outlier counts to fine rendered texture (scales, stone, leaves), which a despeckle filter
  would wrongly treat as noise.
- **How widespread:**
  - Over all actions of the 868 Enhanced bodies with frames, the median body has 27 outliers per 1000 interior
    texels.
  - Clipped areas have twice the density of the rest (67 against 34 per 1000).
  - 212 bodies have more than 10 % of their texels clipped, and 70 bodies more than 25 %. Among the large bodies, the
    most clipped are 286 (63 %), 1411 (44 %), 256 (35 %), 59 (35 %), 180 (31 %) and 797 (29 %).
  - KR's frames clip less overall (6.3 % against 7.4 %).
- **The official client** draws the same frames; nothing in it is known to filter them. The user's Enhanced
  screenshots show no dragon, only a small ridden horse at a scaled zoom, so they cannot show whether the specks look
  the same there.
- **Recommendation:** a *highlight-only selective median* at pack time in udd-conv.
  - It replaces a texel by its 3x3 neighbours' channel median only when the texel stands out from them, and only
    where at least 3 of the 9 window texels are clipped.
  - It changes 4–8 % of the dragon frames' texels, 0–1 % on mostly unclipped bodies, and none on 794 and 1288.
  - It removes the dragon's pale/orange specks and halves its outlier count (64 to 35 per 1000), while keeping the
    edges (edge ratio 0.99–1.0).
  - Its cost on the other overexposed bodies varies: 286, 63 % clipped, gets softer (edge 0.98).
  - It costs nothing at run time.
  - The unrestricted selective median removes more specks, but it flattens the texture of the scaled and stony
    bodies.
  - The ML option (a denoising SPAN) does not help: see below.

## Evidence

### The source frames

`palette <client> <body> <action>` on the dragon (server body 12 shows `AnimationFrame` body 59; `source-body`):

| | EC 59 action 22 | KR 59 action 22 |
|---|---:|---:|
| palette colours (all hueable) | 256 | 256 |
| distinct values per channel (R, G, B) | 104, 142, 124 | 123, 146, 130 |
| palette spacing, median (p10–p90) | 12.4 (8.5–19.4) | 10.1 (7.3–16.9) |
| outliers per 1000 interior texels | 53.1 | 24.4 |
| outlier's distance to its neighbours' median, in palette spacings, median (p10–p90) | 4.2 (2.2–9.0) | 4.6 (2.2–11.4) |
| palette colours nearer that median than the outlier, median | 31 | 50 |
| dither-like 3x3 windows: stored / re-quantized to 64 colours, nearest / Floyd-Steinberg | 0.07 % / 1.09 % / 0.87 % | 0.74 % / 1.80 % / 2.71 % |

- The channel values are full 8-bit: multiples of 8 make up 11.8 % of them, against 12.5 % for uniform values.
- The palette's spacing is small next to the specks' contrast.
- A dither test fails on its controls first: the stored frames have far fewer two-colour intermixed windows than the
  same frames re-quantized to 64 colours, with or without error diffusion.

**The specks are the render's own.** `visuals/dragon59_back_ec_vs_kr_4x.png` puts the EC back at 4x next to the KR
render of the same frame at the same size. The KR frames are 1.36 times larger, so the KR crop is scaled less. The KR
skin has the same blotches, at lower contrast and without the hue flips.

**The clipping.**
- On the walk frame (59:22, stored direction 1, frame 2), 30 % of the opaque texels have red at 250 or above.
- On the flying top view (59:0:4:2) it is 51 %; on the KR walk frame it is 9.6 %.
- In the back's speck neighbourhoods, red is 254 throughout while green runs 147–215 and blue 107–164. One texel
  is pale yellow (254, 213, 156) and its neighbour saturated orange (254, 147, 107).
- Their neighbours spread 15–50 apart in RGB. The bright field is therefore not smooth with a few specks in it: it is
  grainy throughout.

### Survey

`survey <client> 0 <tsv>` counts over every action of every body; it takes about 3 minutes per client on 20 threads.
An outlier is an interior texel at least 20 (RGB distance) from its 8 neighbours' channel median and at least 3 times
the neighbours' own median distance to it.

| | EC | KR |
|---|---:|---:|
| bodies with frames | 868 | 538 |
| outliers per 1000 interior texels: p10 / median / p90 of bodies | 8.6 / 27.2 / 65.2 | 12.9 / 37.2 / 79.3 |
| texel-weighted | 36.8 | 35.6 |
| interior texels with a clipped channel (≥ 250) | 7.4 % | 6.3 % |
| outliers per 1000 in clipped areas / elsewhere | 67.1 / 34.4 | 63.3 / 33.7 |
| bodies with more than 10 % / 25 % clipped | 212 / 70 | 131 / 40 |

| Body | EC clipped | EC outliers ‰ | of them in clipped areas | KR clipped | KR outliers ‰ |
|---|---:|---:|---:|---:|---:|
| 59 (red dragon) | 35.3 % | 56.9 | 54 % | 4.7 % | 21.4 |
| 197 (dragon) | 18.0 % | 85.0 | 34 % | 1.3 % | 55.8 |
| 198 (dragon) | 10.8 % | 85.0 | 23 % | 2.1 % | 55.2 |
| 123 (winged) | 17.4 % | 61.1 | 23 % | 14.3 % | 69.2 |
| 200 (horse) | 11.5 % | 15.9 | 21 % | 0.4 % | 12.0 |
| 829 (golem) | 1.0 % | 74.7 | 4 % | | |
| 794 (leaf dragon) | 1.7 % | 58.6 | 7 % | 0.0 % | 55.6 |
| 1288 (turtle) | 0.0 % | 57.0 | 0 % | | |

The EC render clips more than the KR render in only 135 of the 502 bodies both clients have. The worst Enhanced
cases are therefore individual overexposed models, the red dragon among them, not a general Enhanced trait.

## Options

All options were prototyped on the frames listed under [Visuals](#visuals). Each filter changes only the solid texels
(coverage 16 of 16) and reads only solid neighbours of the same hue class, so the antialiased edge and the hue mask
stay as they are. Abbreviations in the table: ‰ is outliers per 1000 interior texels, "lp PSNR" is the PSNR of the
low-passed (binomial 5x5) frame against the low-passed source (it changes when the filter shifts the colour, not
when it removes a speck), and "edge" is the Sobel energy on the source's strongest tenth of edges, relative to the
source (1.0 keeps the edges).

| Option | Dragon 59 back ‰ (source 49.2) | Dragon 59 top ‰ (65.3) | 198 ‰ (77.6) | 829 ‰ (73.6) | 1288 ‰ (53.8) | Texels changed | lp PSNR / edge (dragon) | Visual |
|---|---:|---:|---:|---:|---:|---:|---|---|
| (a) palette-aware de-quantization (Gaussian over near palette entries, clamped to 1 palette step) | 58.4 | 75.8 | 96.1 | 100.5 | 54.8 | 88–98 % | 48.1 dB / 0.998 | specks stay: they are 4 steps out, not 1 |
| (b) bilateral (σs 1.2, σr 16) | 50.3 | 59.8 | 88.7 | 90.3 | 46.7 | 95–99 % | 52.3 dB / 0.997 | specks stay |
| (b) guided (luma, r 2, ε 12²) | 39.9 | 40.9 | 80.7 | 85.7 | 35.7 | all | 39.0 dB / 0.953 | blurs edges, specks fade only |
| (b) selective median, everywhere | 17.8 | 23.1 | 36.1 | 32.4 | 21.8 | 7–11 % | 38.9 dB / 0.984 | specks gone; flattens scales and stone (198: full PSNR 20.2) |
| **(d) selective median, clipped highlights only** | **33.0** | **33.8** | **71.8** | **72.3** | **53.8** | **0–8 %** | **41.7 dB / 0.999** | **dragon specks gone, textured bodies unchanged** |
| (c) SPAN 4xNomosUni_span_multijpg instead of mssim | | | | | | (2x) | | sharper, keeps the specks |

Notes on the options:

- **(a) Palette-aware de-quantization** assumes the specks are within one or two palette steps of the true colour.
  They are 4 steps out (median), so clamping to the quantization cell leaves them in place. Smoothing their
  neighbours only makes them stand out more, and the outlier count rises.
- **(b) Edge-preserving smoothing** cannot tell a speck from the texture. With a range σ small enough to keep the
  edges, the bilateral keeps the specks. The guided filter blurs everything a little, edges included (edge ratio
  0.93–0.95).
- **A dark-speck filter for smooth areas** was tried and dropped. It replaced a texel only when it was much darker
  than its neighbours and those neighbours were within 9 of each other. It changed 0.1–1.2 % of the texels but missed
  the dragon's specks, because the bright field around them is not smooth (see the clipping evidence).
- **(d) The clipped-highlight median** targets the mechanism. It applies the selective median (the outlier test with
  distance 12 and spread factor 2.5) only where at least 3 of the 9 window texels have a channel ≥ 250. On bodies
  without clipped highlights it does nothing (794 and 1288 are unchanged).
- **(c) ML.**
  - The game's SPAN is 4xmssim_span_pretrain (CC-BY-4.0). At 2x it already softens the specks a little, but keeps them
    visible (crop column "SPAN 2x" in the sheets).
  - 4xNomosUni_span_multijpg is the same architecture and licence. It is trained on JPEG and multiscale degradations,
    so it could replace the weights with no code change. [`fold_span.py`](ec_frame_quantization/fold_span.py) folds
    it, input normalization included, into our tensor layout with numpy. The script reproduces the shipped mssim file
    to fp16 rounding (largest relative difference 3e-4), and it found that the checkpoint's own `eval_conv` tensors
    are stale.
  - In game Nomos is sharper and keeps the specks: its 2x outlier density is about twice that of mssim's (8.9 against
    4.7 ‰ on the dragon). It is trained to remove JPEG blocking, not texture aliasing.
  - The dedicated 1x de-quantizers are out:
    - 1x-DitherDeleterV3-Smooth (WTFPL): ESRGAN 64nf 23 blocks, about 17 M MACs per pixel, 40 times SPAN's 0.43 M
      MACs per canvas pixel. One 215x167 frame would take about 600 ms of a 2 ms budget, and it targets dithering,
      which these frames do not have.
    - 1x-Spongebob-De-Quantize: CC-BY-NC-4.0.
    - 1x-DeNoise-realplksr-otf (CC-BY-4.0) would need a RealPLKSR port.
  - None of them tells highlight clipping from texture any better than the targeted median does.

### Cost and interactions

- **Pack time.** One thread runs the filters at 0.2–0.5 s per megapixel of frame box: highlight and median 0.21 s,
  guided 0.22 s, palette 0.24 s, bilateral 0.41 s. The EC animations have 1.5 G solid texels, so 20 threads take
  about 30 s. The highlight test rejects most texels before the median, so it runs at most at the median's cost.
- **Run time.** Zero for a pack-time filter.
  - A compute version would also be cheap (a 3x3 pass is a few µs per frame). It would still have to run on every
    frame before the atlas upload and before SPAN and RIFE, which the stride and atlas paths do not have a hook for.
    Pack time is the natural place.
- **BC7, SPAN and RIFE.** See the `measure` table below. The package pages are RGBA8888, so the 1x frames are not
  BC7-encoded; only SPAN and RIFE output is (UltraFast). The highlight filter changes too few texels to matter to
  either.
- **Hue.**
  - All dragon colours are hueable, and a hueable texel's red byte is its hue key. The filter writes medians, so in a
    clipped area red stays at 250–254 and the hue keys barely move.
  - The hue mask itself is not touched.
  - A production version should restrict the median to texels of the same hue class (the prototype does) and leave
    the mask as is.

`measure <client> 59,197,286,123,829,1288 1 4` covers the first action of each body (40–100 frames). For each filter it
measures the frames at 1x; BC7 UltraFast of the frames packed into 1024² pages (PSNR against the filtered frames, and
the pages' Zstd 9 bytes); SPAN 2x of 4 frames per action (only frames up to 256 texels a side, so none of the dragons);
and the RIFE x2 in-betweens of the first stored direction:

| Body | Filter | 1x ‰ | lp PSNR / edge | BC7 PSNR | BC7+Zstd bytes | SPAN 2x ‰ / edge | RIFE in-betweens ‰ |
|---|---|---:|---|---:|---:|---|---:|
| 59 | source | 63.7 | | 33.30 | 1 458 053 | | 65.8 |
| 59 | **highlight** | 35.0 | 42.1 dB / 0.990 | 33.64 | 1 453 627 | | 34.5 |
| 59 | median | 23.1 | 39.7 dB / 0.970 | 33.79 | 1 451 174 | | 25.2 |
| 197 | source | 79.5 | | 28.68 | 1 906 366 | | 80.4 |
| 197 | **highlight** | 70.7 | 38.4 dB / 0.969 | 29.27 | 1 903 985 | | 70.4 |
| 197 | median | 34.8 | 32.6 dB / 0.933 | 30.19 | 1 899 165 | | 38.2 |
| 286 | source | 74.0 | | 30.74 | 408 743 | 9.0 / 1.000 | 65.8 |
| 286 | **highlight** | 38.7 | 37.2 dB / 0.981 | 31.32 | 402 849 | 6.7 / 0.915 | 37.4 |
| 286 | median | 31.0 | 36.1 dB / 0.957 | 31.47 | 402 804 | 5.5 / 0.818 | 29.1 |
| 123 | source | 68.1 | | 37.54 | 347 010 | 27.2 / 1.000 | 65.1 |
| 123 | **highlight** | 54.5 | 42.4 dB / 1.000 | 37.58 | 345 846 | 23.4 / 0.971 | 56.5 |
| 123 | median | 31.6 | 36.9 dB / 0.950 | 37.76 | 345 514 | 17.0 / 0.715 | 36.2 |
| 829 | source | 75.0 | | 32.43 | 632 034 | 16.3 / 1.000 | 79.3 |
| 829 | **highlight** | 74.6 | 49.1 dB / 0.997 | 32.44 | 632 083 | 16.2 / 0.995 | 78.8 |
| 829 | median | 33.9 | 33.1 dB / 0.916 | 33.11 | 632 142 | 9.2 / 0.763 | 39.2 |
| 1288 | source / highlight | 52.2 | unchanged | 35.33 | 3 948 567 | 5.0 / 1.000 | 47.7 |
| 1288 | median | 21.7 | 39.4 dB / 0.968 | 35.75 | 3 896 978 | 3.4 / 0.876 | 21.9 |

Reading the table:

- **BC7.** Any filter gains a little BC7 PSNR: 0.0–0.6 dB for the highlight median, more for the smoothing filters
  (the guided filter gains 4–6 dB). The compressed size moves by under 1.5 %.
- **SPAN.** SPAN's output follows its input. On the filtered frames its edge ratio against SPAN of the source falls
  with the unrestricted median (0.72–0.88), and much less with the highlight median, except on 286, whose texels are
  63 % clipped.
  - Since e612dfe7 on dev, bodies with a sequence past a 260 canvas play at 1x in every action, so the reported
    dragon and the other large bodies are not upscaled in game at all. At 1x nearest, a pack-time filter is the only
    lever.
- **RIFE.** The in-betweens inherit the 1x frames' speck density almost exactly (dragon 65.8 to 34.5 ‰), and the
  filter adds no flicker of its own: the motion videos show none.

## Recommendation

1. **Pack-time highlight median in udd-conv** (`lib/udd-conv/src/animation/mobile_anim_ec.rs`, between
   `decode_frame` and the blit), on by default for Enhanced frames.
   - It needs no format change: the package keeps RGBA8888 frames, only with different pixels, so a repack is
     enough.
   - It adds no run-time cost, and it fixes the red dragon and the other overexposed models (286, 1411, 256, 180, 797
     …) without touching textured bodies.
   - Constants as in the prototype:
     - clip level 250;
     - at least 3 clipped texels in the 3x3 window;
     - outlier distance 12 and spread factor 2.5.
2. **Do not** apply a general denoiser (bilateral, guided, palette or a 1x ML model). The general "specks" are the
   models' rendered texture, which KR shows too.
3. **Do not swap SPAN's weights** for Nomos's: they are sharper and keep the specks.
4. Worth asking the user: whether the stronger unrestricted median (it also softens textured bodies) is preferred on
   the dragons alone. A per-body list would allow that, but the highlight filter makes it unnecessary for the reported
   case.

## Visuals

In `/home/claudio/rust-target/uodynamapper-scratch/ec-undither/visuals/` (not in git):

- `<name>_sheet.png`, one per body. Each row is one variant: source, package, highlight, median, bilateral, guided,
  palette, and Nomos (whose 1x is the source, with the Nomos SPAN in the last column). Each row shows the frame at 1x
  and 2x, a crop at 3x and, for frames up to 256 texels a side, the SPAN 2x crop at 3x of the source scale. Bodies:
  - `dragon59` (walk, back and tail) and `dragon59tail` (flying top view);
  - `b198` (scaled dragon), `b829` (golem), `b794` (leaf dragon), `b123` (winged rider), `b1288` (turtle).
- `<name>_flicker.mp4`: source, highlight, source, median, 0.7 s each, 1x plus a 3x crop. For `dragon59`,
  `dragon59tail`, `b123` and `b198`.
- `dragon_walk_motion.mp4` and `dragon_fly_motion.mp4`: the clip playing as source, highlight and median side by
  side, first at 1x (3x nearest), then the SPAN 2x frames, then RIFE x2.
- `dragon59_back_ec_vs_kr_4x.png` and `dragon59_walk_ec3x_vs_kr2x.png`: the EC render next to the KR render.

## Reproduction

The crate builds outside the workspace (`[workspace]` is empty), with its own target:

    cd docs/agent_notes/ec_frame_quantization
    CARGO_TARGET_DIR=<scratch>/target cargo build --release
    B=<scratch>/target/release/ec-frame-quantization-bench
    EC="<Enhanced client folder>"; KR="<KR client folder>"
    $B survey "$EC" 0 survey_ec.tsv          # and "$KR"
    $B palette "$EC" 59 22                   # palette, outlier distances, dither controls
    $B pair "$EC" "$KR" 59 22
    $B source-body <package dir> 12
    $B frames "$EC" <package dir> bodies dragon59=59:22:1:2 dragon59tail=59:0:4:2 b198=198:0:1:3 ...
    $B clip "$EC" clip source,highlight,median dragon_walk=59:22:1 dragon_fly=59:0:4
    $B measure "$EC" 59,197,286,123,829,1288 1 4 measure.tsv
    $B timing "$EC" 59,198,829 1
    python3 -I -B sheets.py sheet bodies visuals dragon59 75,5,110,70 source,package,highlight,median
    python3 -I -B sheets.py flicker bodies visuals dragon59 75,5,110,70 highlight,median
    python3 -I -B sheets.py motion clip visuals dragon_walk 10 highlight,median
    python3 -I -B fold_span.py 4xNomosUni_span_multijpg.safetensors span_4x_nomos_f16.safetensors

To try the Nomos weights, copy the folded file over `lib/support/uot-gpu-compute/models/span_4x_mssim_f16.safetensors`,
build into a separate target, and restore the file with `git checkout`. GPU runs (`frames`, `clip`, `measure`) hold
the shared preview lock.

Model sources:
- 4xmssim_span_pretrain and 4xNomosUni_span_multijpg: <https://openmodeldb.info/models/4x-NomosUni-span-multijpg>
- <https://huggingface.co/Phips/2xNomosUni_span_multijpg_ldl>
- <https://openmodeldb.info/models/1x-DitherDeleterV3-Smooth>
- <https://openmodeldb.info/models/1x-Spongebob-De-Quantize>
- <https://openmodeldb.info/models/1x-DeNoise-realplksr-otf>
