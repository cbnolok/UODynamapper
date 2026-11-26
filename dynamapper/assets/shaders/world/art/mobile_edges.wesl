// ============================================================================
// art::mobile_edges — The edge treatments of a mobile's frames, each off at 0 in the frame's filter
// (`MobileFrameFilter`, render/mobiles/sharpening.rs, from runtime_specs/mobiles/Sharpening.kdl):
// - the alpha contrast pulls each frame's partial alpha towards 0 or 1 about a midpoint, one frame at a time, before
//   two frames crossfade, so the Enhanced frames' anti-aliased rim shows fewer half-transparent texels;
// - the contour darkens the rim and the outermost opaque texels towards a colour, as the Classic frames' outline,
//   from the cross of taps the sharpening's window holds;
// - the outline lays a line of that colour one texel wide outside the silhouette, under its rim, on the card the
//   frame's filter pads by a texel each side.
// The colours mix on their sRGB-encoded values, as the clients' 8-bit art.
// ============================================================================

import package::shaders::world::art::art_bindings::MobileFrameFilter;
import package::shaders::world::srgb::{encode_srgb, decode_srgb};

// The alpha above which a texel counts as opaque, not the rim: BC7 keeps the Classic frames' opaque texels at 254.
const MOBILE_EDGE_OPAQUE_ALPHA: f32 = 0.98;
// The narrowest span the contrast maps onto 0..1 about its midpoint: at contrast 1 the alpha is all but cut there.
const MOBILE_EDGE_MIN_CONTRAST_SPAN: f32 = 1.0e-3;

// Whether the frame's alpha is contrasted.
fn mobile_alpha_contrast_on(frame_filter: MobileFrameFilter) -> bool {
    return frame_filter.alpha_contrast > 0.0;
}

// A frame's tap with its partial alpha pulled from the midpoint: (a - m) / (1 - contrast) + m, clamped, which keeps
// 0, 1 and the midpoint and leaves the alpha as it is at contrast 0. The colour is straight and stays.
fn mobile_alpha_contrast(tap: vec4<f32>, frame_filter: MobileFrameFilter) -> vec4<f32> {
    if (!mobile_alpha_contrast_on(frame_filter)) {
        return tap;
    }
    let span = max(1.0 - frame_filter.alpha_contrast, MOBILE_EDGE_MIN_CONTRAST_SPAN);
    let alpha = clamp((tap.a - frame_filter.alpha_midpoint) / span + frame_filter.alpha_midpoint, 0.0, 1.0);
    return vec4<f32>(tap.rgb, alpha);
}

// Whether a texel of alpha `alpha` is the rim (partly transparent), not the opaque body.
fn mobile_edge_is_fringe(alpha: f32) -> bool {
    return alpha < MOBILE_EDGE_OPAQUE_ALPHA;
}

// Whether the frame's contour or outline is drawn.
fn mobile_contour_on(frame_filter: MobileFrameFilter) -> bool {
    return frame_filter.contour > 0.0;
}
fn mobile_outline_on(frame_filter: MobileFrameFilter) -> bool {
    return frame_filter.outline > 0.0;
}

// The contour's colour, sRGB-encoded.
fn mobile_contour_encoded(frame_filter: MobileFrameFilter) -> vec3<f32> {
    return unpack4x8unorm(frame_filter.contour_color).rgb;
}

// `color` (straight, linear) darkened towards the contour colour by the frame's contour strength times how far the
// texel is from the opaque body: one less the lowest alpha of the texel and its cross `cross_alpha` (north, west,
// east, south; a tap outside the card counts transparent). An opaque texel inside the body keeps its colour, one
// beside a transparent texel takes the full strength, as does the rim.
fn mobile_contour(color: vec4<f32>, cross_alpha: vec4<f32>, frame_filter: MobileFrameFilter) -> vec4<f32> {
    let lowest = min(min(min(cross_alpha.x, cross_alpha.y), min(cross_alpha.z, cross_alpha.w)), color.a);
    let weight = frame_filter.contour * clamp(1.0 - lowest, 0.0, 1.0);
    let darkened = mix(encode_srgb(color.rgb), mobile_contour_encoded(frame_filter), weight);
    return vec4<f32>(decode_srgb(darkened), color.a);
}

// `color` (straight, linear) over the outline: the contour colour at the outline's opacity times the highest alpha
// of the cross `cross_alpha`, under the texel, so a transparent texel beside the silhouette takes the line and the
// rim lies over it.
fn mobile_outline(color: vec4<f32>, cross_alpha: vec4<f32>, frame_filter: MobileFrameFilter) -> vec4<f32> {
    let nearest = max(max(cross_alpha.x, cross_alpha.y), max(cross_alpha.z, cross_alpha.w));
    let line = frame_filter.outline * clamp(nearest, 0.0, 1.0) * (1.0 - color.a);
    let alpha = color.a + line;
    if (alpha <= 0.0) {
        return color;
    }
    let encoded = (encode_srgb(color.rgb) * color.a + mobile_contour_encoded(frame_filter) * line) / alpha;
    return vec4<f32>(decode_srgb(encoded), alpha);
}
