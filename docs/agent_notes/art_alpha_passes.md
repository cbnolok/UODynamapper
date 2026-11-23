# Art alpha passes and the transparent-pass edge colour

## The two passes

Statics, ground statics and mobiles (`art/main.wesl`, `art/ground.wesl`) draw each sprite twice, with two
materials built in `render/statics/statics_draw.rs`:

- the opaque pass, `AlphaMode::Mask(0.5)` (`PASS_MODE_OPAQUE`): Bevy's alpha-mask phase, depth written, no blend; the
  shader keeps the texels whose visible alpha (texel alpha times instance opacity) is at least the cutoff, 0.5;
- the transparent pass, `AlphaMode::Blend` (`PASS_MODE_TRANSPARENT`): sorted, no depth write, Bevy's
  `BlendState::ALPHA_BLENDING` (`src * a + dst * (1 - a)`, straight colour; bevy_pbr 0.20 `mesh.rs`, `BLEND_ALPHA`);
  the shader keeps the texels from 0.0001 up to the cutoff.

Each texel lands in exactly one pass. Item surfaces, blend surfaces, filtered wide water and explicitly translucent
art (instance opacity below one, `classic_static_translucency.md`) raise the cutoff to 0.9999, so everything not fully
opaque blends. The fractional alpha the transparent pass sees at ordinary sprite edges comes from the EC art's own
authored alpha, BC7's edge texels, and KR-like's client filter (`texture_filter = 1`), whose taps blend premultiplied
since 693a1e76.

### Which taps filter sprite art (checked 2026-11-23)

No path takes a straight-alpha hardware-linear sample of sprite art or mobile frames, so no tap blends a transparent
texel's colour (the packages' black) into an edge:

- **Pixel-art filter** (`texture_filter = 0`: Classic, Enhanced and UOD-Painterly; statics in `art/main.wesl`, ground
  art in `art/ground.wesl`, mobiles through `sprite_page_tap` and `mobile_frame_tap`). The art page atlases
  (`create_art_atlas_image`, statics_draw.rs) and the mobile frame atlases (`atlas_image`, frame_atlas.rs) set no
  sampler, so they get Bevy's default one, nearest (`ImagePlugin::default_nearest()`, app.rs; bevy_render's
  `GpuImage` resolves `ImageSampler::Default` to it). The tap reads the centre of the texel the uv lies in, kept
  inside the sprite's rectangle (`pixel_art_atlas_nearest_uv`, pixel_art_filters.wesl), so it is nearest at every
  zoom; `enable_linear_filtering` (on in UOD-Painterly) reaches land and EC item surfaces only. The SPAN 2x frames
  and the RIFE in-betweens are frames in the same atlases, drawn the same way; the crossfade of two frames
  (`mobile_frames_mix`) blends premultiplied.
- **Client filter** (`texture_filter = 1`: KR-like): `client_filtered_rect` and `mobile_frame_client_filtered` load the
  texels and blend them premultiplied, the emulated mip box included.
- **Filtered wide water** (zoom out): premultiplied taps.

Evidence: Enhanced, UOD-Painterly and Classic captures at Britain 1495,1697 (zoom 0.4 and 0.73, CC and EC art, SPAN
on and off) with the iq point replaced by the plain uv match exactly for EC art and at zoom 0.73. For CC art at zoom
0.4, 1.4 % of the pixels differ, as single pixel columns along the roofs where float rounding puts the point in the
neighbouring texel; with a linear sampler every magnified texel's interior would have changed. The Classic art's
edges stay stair-stepped and binary in every style but KR-like.

Until 2026-11-23 these taps went through `pixel_art_atlas_uv_iq`, the land's seam filter, which under a nearest
sampler only moved the point within its texel and, through its float rounding, took the left or upper texel where a
screen pixel's centre lies exactly on a texel edge. `pixel_art_atlas_nearest_uv` rounds the texel position to 1/256
of a texel, as the sampler's sub-texel precision does, takes the texel it lies in (the right or lower one on an edge)
and taps its centre, so the draw and the 1x1 pick pass (`pick_clip`, art/picking.wesl), whose interpolated uvs differ
in their last bits, take the same texel. At zoom 0.4 (2.5 pixels a texel) the Classic roofs at Britain 1495,1697
change in 1.5 % of their pixels, every one in the columns whose centres lie on texel edges (one in five), each now the
texel to its right: the texels show 2, 3, 2, 3 pixels wide instead of 3, 2.

The old filter also left the sprite's rectangle. A mirrored mobile card puts the frame's right edge (`uv_max`) on its
screen-left side, where the rasterizer keeps a pixel centred on the card's edge; that pixel's uv clamped to `uv_max`
and the seam filter left it there, so the nearest tap took the column right of the frame: its gutter. The frame
uploads write the frames alone, so a gutter keeps the texels of the frames its page held before it emptied, and the
column showed them as a pale line the frame's height beside the wing tips of the Enhanced daemon and kirin walking
east at zoom 0.4, on the frames whose card edge lay on pixel centres, one or two frames at a time (the
`strides-curated` videos of 2026-11-22). Recorded on the walking field (renderer-reference-validation) after other
bodies had filled the pages, the daemon's east walk and run showed it in 37 and 39 frames with the old filter, in
none with the new one, and in none walking west, southwest or northeast with either (crossfade; the same run with
RIFE x4 in-betweens and two runs of the kirin's east walk showed no line with either filter: whether a card's edge
meets the pixel centres depends on the frame's size and anchor and on where the walker stands within its pixel).
`frame_atlas/nearest_uv_tests.rs` checks on the GPU that no tap at any zoom or on the edges reads past the frame.

## The edge factor, removed 2026-11-22

From be44e69c ("Start implementing double pass art pipeline", no rationale recorded) until 2026-11-22 the transparent
pass wrote `rgb * a * 2` for ordinary sprite edges (main.wesl; ground.wesl for every non-EC-water texel). Through the
straight blend that gives `2a² C + (1 - a) D` instead of `a C + (1 - a) D`: the colour and background weights sum below
one, so every fringe below alpha one half lost up to `C / 8` of its colour (at `a = 1/4`) to black, meeting the
straight weight only at the cutoff. Its likely intent was continuity at the cutoff: either a premultiplied output that
forgot its blend state, or a coverage remap (`C`, `min(2a, 1)`), which gives `2a C + (1 - 2a) D` and so a harder,
brighter edge.

The clients blend straight in one pass: the KR client's `UOSprite::GetSpriteAlphaProperty` uses the state block at
0x00c933ec, SRCALPHA / INVSRCALPHA with an alpha test GREATER
(`docs/client_research/ghidra-kr-atmosphere/results/uokr_atmosphere.md`), so nothing in them darkens an edge. The
transparent pass now writes straight RGB everywhere; for an edge texel the two passes together are then the clients'
blend with the cut at one half.

## Measurements

EC trees at map 1, 1488,1693 and 1545,1680, zoom 1, afternoon, the dev 5a3a7dee binary with three shader trees. The
fringe is where the factor and straight frames differ by more than 3 levels; the ring the pixels within 2 of it.

| Capture | Fringe px | Fringe / ring luminance: factor | straight | remap |
|---|---|---|---|---|
| 1488,1693 Enhanced | 3775 | 0.848 | 0.944 | 1.058 |
| 1488,1693 KR-like | 14492 | 0.858 | 0.945 | 1.014 |
| 1545,1680 Enhanced | 576 | 0.881 | 0.964 | 1.054 |
| 1545,1680 KR-like | 8674 | 0.865 | 0.947 | 1.008 |

Straight brightens the changed pixels by 5.8 to 6.7 levels on average, 43 at most; the remap moves twice as many
pixels and outlines the canopies with a brighter rim. Side by side at 2x the factor draws a dark contour round every
canopy and straight a soft one.

The EC client screenshot (`Schermata_20261113_073611`, a broadleaf over cobbles and a pine over grass, read only) does
not show the same trees, so it was compared by statistics free of alignment: fitting each pixel of the foliage edge as
`a C + b D` from its local foliage and background, `a + b` has a median of 0.79 and 0.88 there, against 0.88 to 1.15
for every variant of ours. The fit is dominated by the art's own dark shading at the canopy edge and the screenshot's
scale, and it hardly separates our three variants (0.876 against 0.882 in the same box), so the screenshot cannot
decide between them; the clients' blend state does.

## Regression harness

Full tier, before (dev 5a3a7dee's assets) and after, same binary, 506 captures: 110 identical, 73 within tolerance,
323 changed, every changed pixel (over 4 levels of luminance) brighter, all at the edges of foliage, statics and
mobiles: the slope-fallen-logs, yew-forest-canopy and hill-tree-line-rising-ground forests most (up to 4.7 % of the
frame in KR-like, 2.4 % in Enhanced, at most 49 levels). Classic art drawn by Classic, Enhanced and UOD-Painterly is
unchanged, its alpha binary under their filters; KR-like's client filter gives it fractional edges, which brighten.
EC art changes in all four styles.
