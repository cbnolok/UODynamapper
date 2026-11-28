// ============================================================================
// art::mobile_inner_edges — Lines and contrast at the strong edges inside a mobile's body, where the Classic
// frames' pixel art draws a dark line (an arm across the torso, between the legs, along a wing or a plate) and the
// Enhanced frames' rendered art has only a change of colour. The frames hold no depth or normals, so the edges are
// the discontinuities of the colour. Each off at 0 in the frame's filter (`MobileFrameFilter`,
// render/mobiles/sharpening.rs, from runtime_specs/mobiles/Sharpening.kdl):
// - the lines darken the foot of an edge's dark side to a darker shade of its own colour, so the hue stays. They
//   read a 5x5 window of the luma at a step of the frame's texel scale (on a SPAN 2x frame two texels, so the line is
//   as wide on screen as at 1x), smoothed by a binomial 3x3 so that the texel-sized mottle of the rendered skin, fur
//   and scales makes no edge and a form's does. An edge (the smoothed luma's Sobel gradient) takes a line on the
//   texel where its dark side's luma starts to rise towards the light side, beside a step of the texels' own luma
//   (their spread over the 3x3 about it at least half the threshold), and only where the step across it goes on just
//   before and just after the texel along it, so that a fleck takes none; it ramps in from half the frame's
//   `inner_threshold` and is whole at the threshold, or wherever it goes on whole on both sides (a light
//   hysteresis, which keeps a long line from breaking into dashes). No texel within the window of the transparent
//   or partly transparent rim takes a line, so the silhouette is not lined twice. The taps are summed as they are
//   read, and the outer ones read only where the 3x3 leaves a line possible, which most texels do not.
// - the contrast pushes the texels on both sides of an edge apart, the dark side darker and the light side lighter,
//   on the 3x3 window the sharpening reads (art/mobile_sharpen.wesl) and, on an edge only, two taps two texels away
//   along its axis, on the frame's own texel grid. Its edge's strength is the colour step across the texel (the
//   Sobel gradient's length, as the RGB distance over its largest, so 1 is black to white), at most the weaker of
//   its two neighbours' steps along the edge, so that an edge must go on past the texel. It starts at the frame's
//   `inner_threshold` and is whole at 1.5 times it, so a gentle gradient takes nothing and an edge fades in rather
//   than popping; a transparent neighbour lends the centre's colour, so the silhouette makes no interior edge, and
//   the partly transparent rim is left as it is.
// Both take the colours on their sRGB-encoded values, as the clients' 8-bit art.
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
// Rec. 601 luma on the encoded values: the lines' edges, and which side of the contrast's edge is the darker.
const MOBILE_INNER_LUMA = vec3<f32>(0.299, 0.587, 0.114);
// The step past the threshold, as a share of it, at which the contrast's edge is whole.
const MOBILE_INNER_RAMP: f32 = 0.5;
// The share of its colour a texel of the line keeps at strength 1.
const MOBILE_INNER_LINE_SHADE: f32 = 0.45;
// The line's hysteresis: the share of the threshold at which an edge beside a whole one is whole, and below which no
// texel takes a line.
const MOBILE_INNER_LOW_SHARE: f32 = 0.5;
// The gradient's direction is an axis's when its component along it is over this many times the other
// (1 / tan 22.5°), else a diagonal's: the nearest of the eight neighbours.
const MOBILE_INNER_OCTANT_SLOPE: f32 = 2.4142135;
// A texel is the foot of an edge's dark side when its rise to the lighter neighbour is at least this times its fall
// to the darker one. The foot of a hard step rises just twice what it falls once smoothed, so the ratio is a little
// under 2 (1.5 adds short marks on the daemon's mottled skin and hooves).
const MOBILE_INNER_FOOT_RATIO: f32 = 1.9;
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
fn mobile_inner_contrast(color: vec4<f32>, window: array<vec4<f32>, MOBILE_SHARPEN_WINDOW_TAPS>,
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
    let mean = (far_before + near_before + near_after + far_after) / MOBILE_INNER_AXIS_TAPS;
    let push = frame_filter.inner_contrast * MOBILE_INNER_CONTRAST_GAIN * edge.weight * mobile_inner_luma(e - mean);
    encoded += vec3<f32>(push / (1.0 + abs(push) / MOBILE_INNER_CONTRAST_LIMIT));
    return vec4<f32>(decode_srgb(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0))), color.a);
}

// The line's window: `MOBILE_INNER_LINE_SIDE` taps a side, `MOBILE_INNER_LINE_REACH` steps from the centre to an edge,
// row by row from the north-west. A step is the frame's texel scale, so a 2x frame (SPAN) reads the texels of its
// 1x grid and draws its line as wide as a 1x frame's. Its near taps are the 3x3 about the centre.
const MOBILE_INNER_LINE_SIDE: u32 = 5u;
const MOBILE_INNER_LINE_TAPS: u32 = 25u;
const MOBILE_INNER_LINE_REACH: i32 = 2;
// The smoothing's binomial weights on each axis, 1 2 1 over their sum.
const MOBILE_INNER_SMOOTH_EDGE_WEIGHT: f32 = 0.25;
const MOBILE_INNER_SMOOTH_CENTRE_WEIGHT: f32 = 0.5;

// The line window's taps as they are read, one by one: their lumas smoothed by the binomial 3x3 about each of the
// centre's 3x3, row by row (`top`, `middle`, `bottom`, each west to east), so that no array is indexed by a
// computed tap; the near taps' lowest and highest luma; and whether any tap was on the rim.
struct MobileInnerLineSum {
    top: vec3<f32>,
    middle: vec3<f32>,
    bottom: vec3<f32>,
    near_low: f32,
    near_high: f32,
    rim: bool,
}

// The offset, in steps, of the line window's tap `tap` from its centre.
fn mobile_inner_line_offset(tap: u32) -> vec2<i32> {
    return vec2<i32>(i32(tap % MOBILE_INNER_LINE_SIDE) - MOBILE_INNER_LINE_REACH,
        i32(tap / MOBILE_INNER_LINE_SIDE) - MOBILE_INNER_LINE_REACH);
}

// Whether the tap `offset` steps from the centre is one of the near taps.
fn mobile_inner_line_is_near(offset: vec2<i32>) -> bool {
    return all(abs(offset) <= vec2<i32>(1));
}

// The texels of the line window's step: the frame's texel scale (0, an instance not filled, as 1).
fn mobile_inner_line_step(frame_filter: MobileFrameFilter) -> u32 {
    return max(frame_filter.texel_scale, 1u);
}

// A line window with no tap read yet.
fn mobile_inner_line_start() -> MobileInnerLineSum {
    return MobileInnerLineSum(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 1.0, 0.0, false);
}

// The binomial weights of a tap `offset` steps from the centre on one axis in the smoothed lumas at -1, 0 and 1.
fn mobile_inner_smooth_weights(offset: i32) -> vec3<f32> {
    let apart = vec3<i32>(offset + 1, offset, offset - 1);
    return select(vec3<f32>(0.0), select(vec3<f32>(MOBILE_INNER_SMOOTH_EDGE_WEIGHT),
        vec3<f32>(MOBILE_INNER_SMOOTH_CENTRE_WEIGHT), apart == vec3<i32>(0)), abs(apart) <= vec3<i32>(1));
}

// `sum` with the tap `tap` (straight, linear, before the sharpening) `offset` steps from the centre read in.
fn mobile_inner_line_add(sum_in: MobileInnerLineSum, offset: vec2<i32>, tap: vec4<f32>) -> MobileInnerLineSum {
    var sum = sum_in;
    if (mobile_edge_is_fringe(tap.a)) {
        sum.rim = true;
        return sum;
    }
    let luma = mobile_inner_luma(encode_srgb(tap.rgb));
    let across = mobile_inner_smooth_weights(offset.x) * luma;
    let down = mobile_inner_smooth_weights(offset.y);
    sum.top += down.x * across;
    sum.middle += down.y * across;
    sum.bottom += down.z * across;
    if (mobile_inner_line_is_near(offset)) {
        sum.near_low = min(sum.near_low, luma);
        sum.near_high = max(sum.near_high, luma);
    }
    return sum;
}

// Whether the near taps read into `sum` rule a line out before the others are read: one on the rim, or their lumas
// spread less than the low share of the threshold. A line lies beside a step of the texels' own luma, not of the
// smoothed one alone, so that a hard step is lined on its foot and not on the texel past it too.
fn mobile_inner_line_ruled_out(sum: MobileInnerLineSum, frame_filter: MobileFrameFilter) -> bool {
    return sum.rim || sum.near_high - sum.near_low < frame_filter.inner_threshold * MOBILE_INNER_LOW_SHARE;
}

// The smoothed luma `across` (each axis -1, 0 or 1) from the centre.
fn mobile_inner_smooth_at(sum: MobileInnerLineSum, across: vec2<i32>) -> f32 {
    let row = select(select(sum.middle, sum.top, across.y < 0), sum.bottom, across.y > 0);
    return row[across.x + 1];
}

// The interior line through the centre of the line window read into `sum`, by the frame's threshold: its weight, 0
// where there is none, 1 whole.
fn mobile_inner_line(sum: MobileInnerLineSum, frame_filter: MobileFrameFilter) -> f32 {
    if (mobile_inner_line_ruled_out(sum, frame_filter)) {
        return 0.0;
    }
    let a = sum.top.x;
    let b = sum.top.y;
    let c = sum.top.z;
    let d = sum.middle.x;
    let e = sum.middle.y;
    let k = sum.middle.z;
    let g = sum.bottom.x;
    let h = sum.bottom.y;
    let i = sum.bottom.z;
    let gradient = vec2<f32>(c - a + MOBILE_INNER_SOBEL_CENTRE_WEIGHT * (k - d) + i - g,
        g - a + MOBILE_INNER_SOBEL_CENTRE_WEIGHT * (h - b) + i - c) * MOBILE_INNER_SOBEL_NORM;
    let strength = length(gradient);
    let threshold = frame_filter.inner_threshold;
    let low = threshold * MOBILE_INNER_LOW_SHARE;
    if (strength < low) {
        return 0.0;
    }
    // Towards the lighter side: the nearest of the eight neighbours' directions to the gradient's.
    var across = vec2<i32>(sign(gradient));
    let size = abs(gradient);
    if (size.x > size.y * MOBILE_INNER_OCTANT_SLOPE) {
        across.y = 0;
    } else if (size.y > size.x * MOBILE_INNER_OCTANT_SLOPE) {
        across.x = 0;
    }
    // The line keeps to the foot of the dark side, where the luma starts to rise, so that it stays a step wide and
    // on the same side of the edge from texel to texel.
    let rise = mobile_inner_smooth_at(sum, across) - e;
    let fall = e - mobile_inner_smooth_at(sum, -across);
    if (rise < MOBILE_INNER_FOOT_RATIO * fall) {
        return 0.0;
    }
    // The step across the edge just before and just after the texel along it, towards the lighter side: the rows
    // (columns) beside the texel for an edge along an axis, the half-diagonals beside it for a diagonal one. Both
    // must carry the edge on, so that a fleck makes none.
    var steps: vec2<f32>;
    if (across.y == 0) {
        steps = vec2<f32>(c - a, i - g) * f32(across.x);
    } else if (across.x == 0) {
        steps = vec2<f32>(g - a, i - c) * f32(across.y);
    } else if (across.x == across.y) {
        steps = vec2<f32>(k - b, h - d) * f32(across.x);
    } else {
        steps = vec2<f32>(k - h, b - d) * f32(across.x);
    }
    let support = min(steps.x, steps.y);
    if (support < low) {
        return 0.0;
    }
    // Whole where the edge runs on whole on both sides, else ramping in from the low share of the threshold.
    return select(clamp((strength - low) / (threshold - low), 0.0, 1.0), 1.0, support >= threshold);
}

// `color` (straight, linear: after the sharpening) darkened by the interior line of weight `weight` through it, by
// the frame's filter: its own colour scaled, so the hue stays and no neighbour's colour comes in. The alpha stays.
fn mobile_inner_line_shade(color: vec4<f32>, weight: f32, frame_filter: MobileFrameFilter) -> vec4<f32> {
    let shade = 1.0 - frame_filter.inner_lines * weight * (1.0 - MOBILE_INNER_LINE_SHADE);
    return vec4<f32>(decode_srgb(encode_srgb(color.rgb) * shade), color.a);
}
