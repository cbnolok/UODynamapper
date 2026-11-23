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
opaque blends. The fractional alpha the transparent pass sees at ordinary sprite edges comes from the texture filter:
Enhanced's `pixel_art_atlas_uv_iq` through the hardware bilinear sampler, KR-like's client filter (premultiplied taps
since 693a1e76), and BC7's edge texels.

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
