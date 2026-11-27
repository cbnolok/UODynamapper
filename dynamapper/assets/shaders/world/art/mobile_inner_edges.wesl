// ============================================================================
// art::mobile_inner_edges — Lines and contrast at the strong colour edges inside a mobile's body, where the
// Classic frames' pixel art draws a dark line (an arm across the torso, between the legs, along a wing or a plate)
// and the Enhanced frames' rendered art has only a change of colour. The frames hold no depth or normals, so the
// edges are the discontinuities of the colour, taken on the 3x3 window the sharpening reads (art/mobile_sharpen.wesl)
// and, on an edge only, two taps two texels away along its axis, on the frame's own texel grid: on a 2x frame (SPAN)
// a texel of the 2x frame, so the line stays one texel wide there as at 1x. Each off at 0 in the frame's filter
// (`MobileFrameFilter`, render/mobiles/sharpening.rs, from runtime_specs/mobiles/Sharpening.kdl):
// - the lines darken the texel on the darker side of an edge, one texel wide (the gradient's maximum along its axis),
//   to a darker shade of its own colour;
// - the contrast pushes the texels on both sides of an edge apart, the dark side darker and the light side lighter.
// An edge's strength is the colour step across the texel (the Sobel gradient's length, as the RGB distance over its
// largest, so 1 is black to white), at most the weaker of its two neighbours' steps along the edge, so that an edge
// must go on past the texel: a short fleck of texture (fur, scales, a spot) does not make one. It starts at the
// frame's `inner_threshold` and is whole at 1.5 times it, so a gentle gradient takes nothing and an edge fades in
// rather than popping. The colours are taken on their sRGB-encoded values, as the clients' 8-bit art; a transparent
// neighbour lends the centre's colour, so the silhouette (the outline's and the contour's) makes no interior edge,
// and the partly transparent rim is left as it is.
// ============================================================================

import package::shaders::world::art::art_bindings::MobileFrameFilter;
import package::shaders::world::srgb::{encode_srgb, decode_srgb};
import package::shaders::world::art::mobile_edges::mobile_edge_is_fringe;
import package::shaders::world::art::mobile_sharpen::{MOBILE_SHARPEN_WINDOW_TAPS, MOBILE_SHARPEN_CENTRE_TAP,
    MOBILE_SHARPEN_NORTH_TAP, MOBILE_SHARPEN_WEST_TAP, MOBILE_SHARPEN_EAST_TAP, MOBILE_SHARPEN_SOUTH_TAP,
    MOBILE_SHARPEN_NORTH_WEST_TAP, MOBILE_SHARPEN_NORTH_EAST_TAP, MOBILE_SHARPEN_SOUTH_WEST_TAP,
    MOBILE_SHARPEN_SOUTH_EAST_TAP};

// The taps beyond the window's cross along an edge's axis: before it (west or north) and after it (east or south),
// this many texels from the fragment's.
const MOBILE_INNER_FAR_TAPS: u32 = 2u;
const MOBILE_INNER_FAR_BEFORE: u32 = 0u;
const MOBILE_INNER_FAR_AFTER: u32 = 1u;
const MOBILE_INNER_FAR_REACH: f32 = 2.0;
// The Sobel weights: the row (or column) through the centre counts twice its two neighbours, over their sum.
const MOBILE_INNER_SOBEL_CENTRE_WEIGHT: f32 = 2.0;
const MOBILE_INNER_SOBEL_NORM: f32 = 0.25;
// The length of a step from black to white in RGB, which the colour step divides by.
const MOBILE_INNER_RGB_RANGE: f32 = 1.7320508;
// Rec. 601 luma on the encoded values: which side of an edge is the darker.
const MOBILE_INNER_LUMA = vec3<f32>(0.299, 0.587, 0.114);
// The step past the threshold, as a share of it, at which an edge is whole.
const MOBILE_INNER_RAMP: f32 = 0.5;
// The share of its colour a texel of the line keeps at strength 1.
const MOBILE_INNER_LINE_SHADE: f32 = 0.45;
// The contrast at strength 1: the push on each side of an edge is this times the centre's luma less the mean of the
// four taps along the edge's axis, softly limited to `MOBILE_INNER_CONTRAST_LIMIT` so that a hard edge is not
// clipped to black and white.
const MOBILE_INNER_CONTRAST_GAIN: f32 = 3.0;
const MOBILE_INNER_CONTRAST_LIMIT: f32 = 0.3;
const MOBILE_INNER_AXIS_TAPS: f32 = 4.0;

// An interior edge through a fragment: its weight (0 none, 1 whole) and its axis, the direction across it, (1, 0)
// for an edge running north to south and (0, 1) for one running west to east.
struct MobileInnerEdge {
    weight: f32,
    axis: vec2<f32>,
}

// Whether the frame's interior lines or contrast are drawn: off, the fragment skips their work.
fn mobile_inner_edges_on(frame_filter: MobileFrameFilter) -> bool {
    return frame_filter.inner_lines > 0.0 || frame_filter.inner_contrast > 0.0;
}

// The offset, in texels, of the far tap `side` across the edge `edge`.
fn mobile_inner_far_offset(edge: MobileInnerEdge, side: u32) -> vec2<f32> {
    return select(-edge.axis, edge.axis, side == MOBILE_INNER_FAR_AFTER) * MOBILE_INNER_FAR_REACH;
}

// The colour step `step` (sRGB-encoded) as a share of black to white.
fn mobile_inner_step(step: vec3<f32>) -> f32 {
    return length(step) / MOBILE_INNER_RGB_RANGE;
}

fn mobile_inner_luma(encoded: vec3<f32>) -> f32 {
    return dot(encoded, MOBILE_INNER_LUMA);
}

// The window's taps (straight, linear) encoded, a transparent one lending the centre's colour.
fn mobile_inner_encoded(window: array<vec4<f32>, MOBILE_SHARPEN_WINDOW_TAPS>)
    -> array<vec3<f32>, MOBILE_SHARPEN_WINDOW_TAPS> {
    var taps = window;
    let e = encode_srgb(taps[MOBILE_SHARPEN_CENTRE_TAP].rgb);
    var t: array<vec3<f32>, MOBILE_SHARPEN_WINDOW_TAPS>;
    for (var tap = 0u; tap < MOBILE_SHARPEN_WINDOW_TAPS; tap++) {
        t[tap] = mix(e, encode_srgb(taps[tap].rgb), clamp(taps[tap].a, 0.0, 1.0));
    }
    return t;
}

// The interior edge through the centre of the window `window` (straight, linear, before the sharpening), by the
// frame's threshold: a weight of 0 where there is none or the centre is the rim.
fn mobile_inner_edge(window: array<vec4<f32>, MOBILE_SHARPEN_WINDOW_TAPS>, frame_filter: MobileFrameFilter)
    -> MobileInnerEdge {
    var taps = window;
    if (mobile_edge_is_fringe(taps[MOBILE_SHARPEN_CENTRE_TAP].a)) {
        return MobileInnerEdge(0.0, vec2<f32>(0.0));
    }
    let t = mobile_inner_encoded(window);
    let a = t[MOBILE_SHARPEN_NORTH_WEST_TAP];
    let b = t[MOBILE_SHARPEN_NORTH_TAP];
    let c = t[MOBILE_SHARPEN_NORTH_EAST_TAP];
    let d = t[MOBILE_SHARPEN_WEST_TAP];
    let g = t[MOBILE_SHARPEN_SOUTH_WEST_TAP];
    let h = t[MOBILE_SHARPEN_SOUTH_TAP];
    let i = t[MOBILE_SHARPEN_SOUTH_EAST_TAP];
    let k = t[MOBILE_SHARPEN_EAST_TAP];
    // The Sobel gradient: the central differences of the three rows (columns), the centre's counting twice.
    let across_x = (c - a + MOBILE_INNER_SOBEL_CENTRE_WEIGHT * (k - d) + i - g) * MOBILE_INNER_SOBEL_NORM;
    let across_y = (g - a + MOBILE_INNER_SOBEL_CENTRE_WEIGHT * (h - b) + i - c) * MOBILE_INNER_SOBEL_NORM;
    let step_x = mobile_inner_step(across_x);
    let step_y = mobile_inner_step(across_y);
    // The edge's axis: the larger step's. Along it lie the texels either side of the edge; across it, the two
    // neighbours whose own steps must carry the edge on.
    let horizontal = step_x >= step_y;
    let along = select(min(mobile_inner_step(g - a), mobile_inner_step(i - c)),
        min(mobile_inner_step(c - a), mobile_inner_step(i - g)), horizontal);
    let strength = min(length(vec2<f32>(step_x, step_y)), along);
    let threshold = frame_filter.inner_threshold;
    let weight = clamp((strength - threshold) / (threshold * MOBILE_INNER_RAMP), 0.0, 1.0);
    return MobileInnerEdge(weight, select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horizontal));
}

// `color` (straight, linear: the window's centre after the sharpening) with the interior lines and contrast of the
// edge `edge` through the window `window` and the far taps `far` across it (straight, linear, both before the
// sharpening), by the frame's filter. The alpha stays.
fn mobile_inner_edges(color: vec4<f32>, window: array<vec4<f32>, MOBILE_SHARPEN_WINDOW_TAPS>,
    far: array<vec4<f32>, MOBILE_INNER_FAR_TAPS>, edge: MobileInnerEdge, frame_filter: MobileFrameFilter)
    -> vec4<f32> {
    let t = mobile_inner_encoded(window);
    let e = t[MOBILE_SHARPEN_CENTRE_TAP];
    var far_taps = far;
    let far_before = mix(e, encode_srgb(far_taps[MOBILE_INNER_FAR_BEFORE].rgb),
        clamp(far_taps[MOBILE_INNER_FAR_BEFORE].a, 0.0, 1.0));
    let far_after = mix(e, encode_srgb(far_taps[MOBILE_INNER_FAR_AFTER].rgb),
        clamp(far_taps[MOBILE_INNER_FAR_AFTER].a, 0.0, 1.0));
    let horizontal = edge.axis.x > 0.0;
    let near_before = select(t[MOBILE_SHARPEN_NORTH_TAP], t[MOBILE_SHARPEN_WEST_TAP], horizontal);
    let near_after = select(t[MOBILE_SHARPEN_SOUTH_TAP], t[MOBILE_SHARPEN_EAST_TAP], horizontal);
    var encoded = encode_srgb(color.rgb);
    if (frame_filter.inner_lines > 0.0) {
        // One texel wide: the texel's step along the axis is a maximum against its neighbours' there, a tie going to
        // the texel on the darker side, so a sharp edge's line lies on its dark side and a soft one's in its middle.
        let own = mobile_inner_step(near_after - near_before);
        let before_step = mobile_inner_step(e - far_before);
        let after_step = mobile_inner_step(far_after - e);
        let before_darker = mobile_inner_luma(near_before) <= mobile_inner_luma(near_after);
        let crest = select(own > after_step && own >= before_step, own > before_step && own >= after_step,
            before_darker);
        if (crest) {
            encoded *= 1.0 - frame_filter.inner_lines * edge.weight * (1.0 - MOBILE_INNER_LINE_SHADE);
        }
    }
    if (frame_filter.inner_contrast > 0.0) {
        let mean = (far_before + near_before + near_after + far_after) / MOBILE_INNER_AXIS_TAPS;
        let push = frame_filter.inner_contrast * MOBILE_INNER_CONTRAST_GAIN * edge.weight * mobile_inner_luma(e - mean);
        encoded += vec3<f32>(push / (1.0 + abs(push) / MOBILE_INNER_CONTRAST_LIMIT));
    }
    return vec4<f32>(decode_srgb(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0))), color.a);
}
