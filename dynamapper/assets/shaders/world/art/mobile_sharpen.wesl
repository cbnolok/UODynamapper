// ============================================================================
// art::mobile_sharpen — Contrast-adaptive sharpening of the mobiles' frames: AMD FidelityFX CAS (ffx_cas.h, its
// filter without scaling, the soft minimum and maximum of the whole window), by the style's strength
// (`effects.mobile_sharpening`, 0 off). The window is the 3x3 texels of the frame around the fragment's, on the
// frame's own texel grid, so the sharpening stays put as the view zooms; main.wesl mobile_sharpened takes each tap
// as the fragment took its colour (the nearest texel, or the client's filter in KR-like), inside the frame's
// rectangle only, so no tap reads the frame's neighbours in the atlas. It covers the 1x and 2x frames and the
// in-betweens alike. The colours are sharpened on their sRGB-encoded values, as the clients' 8-bit art; the alpha
// stays.
// ============================================================================

import package::shaders::world::srgb::{encode_srgb, decode_srgb};

// The window, row by row from its top-left texel: 3 x 3 taps, the fragment's own in the middle.
const MOBILE_SHARPEN_WINDOW_SIDE: u32 = 3u;
const MOBILE_SHARPEN_WINDOW_TAPS: u32 = 9u;
const MOBILE_SHARPEN_CENTRE_TAP: u32 = 4u;
// The cross around the centre (CAS's b, d, f, h) and the corners (a, c, g, i).
const MOBILE_SHARPEN_NORTH_TAP: u32 = 1u;
const MOBILE_SHARPEN_WEST_TAP: u32 = 3u;
const MOBILE_SHARPEN_EAST_TAP: u32 = 5u;
const MOBILE_SHARPEN_SOUTH_TAP: u32 = 7u;
const MOBILE_SHARPEN_NORTH_WEST_TAP: u32 = 0u;
const MOBILE_SHARPEN_NORTH_EAST_TAP: u32 = 2u;
const MOBILE_SHARPEN_SOUTH_WEST_TAP: u32 = 6u;
const MOBILE_SHARPEN_SOUTH_EAST_TAP: u32 = 8u;
const MOBILE_SHARPEN_CROSS_TAPS: f32 = 4.0;
// The lobe at strength 1: CAS's strongest, its sharpness 1, -1 / mix(8, 5, 1). Strength 0.625 is CAS's weakest
// (sharpness 0, -1 / 8); below it the lobe goes on down to 0, which leaves the colour as it is.
const MOBILE_SHARPEN_MAX_LOBE: f32 = 0.2;
// CAS's soft minimum and maximum each add the cross's and the whole window's, so they reach 2 on a white window.
const MOBILE_SHARPEN_SOFT_LIMIT: f32 = 2.0;
// Keeps a black window's soft maximum from dividing by 0: its amplitude is 0.
const MOBILE_SHARPEN_MIN_SOFT_MAX: f32 = 1.0e-5;

// Whether the style sharpens the mobiles: off, the fragment skips the window's taps (a uniform branch).
fn mobile_sharpening_on(strength: f32) -> bool {
    return strength > 0.0;
}

// The offset, in texels, of the window's tap `tap` from the fragment's.
fn mobile_sharpen_tap_offset(tap: u32) -> vec2<f32> {
    let centre = f32(MOBILE_SHARPEN_CENTRE_TAP % MOBILE_SHARPEN_WINDOW_SIDE);
    return vec2<f32>(f32(tap % MOBILE_SHARPEN_WINDOW_SIDE), f32(tap / MOBILE_SHARPEN_WINDOW_SIDE)) - vec2<f32>(centre);
}

// The window's centre sharpened by `strength` (0..1). The taps are straight alpha and linear. A tap lends its own
// colour by its alpha and the centre's in the rest, so the transparent texels around a silhouette, or a frame's
// outside, neither darken nor brighten its edge. A transparent centre stays as it is, and the alpha always does.
fn mobile_sharpen(window: array<vec4<f32>, MOBILE_SHARPEN_WINDOW_TAPS>, strength: f32) -> vec4<f32> {
    var taps = window;
    let centre = taps[MOBILE_SHARPEN_CENTRE_TAP];
    if (centre.a <= 0.0) {
        return centre;
    }
    let e = encode_srgb(centre.rgb);
    var t: array<vec3<f32>, MOBILE_SHARPEN_WINDOW_TAPS>;
    for (var tap = 0u; tap < MOBILE_SHARPEN_WINDOW_TAPS; tap++) {
        t[tap] = mix(e, encode_srgb(taps[tap].rgb), clamp(taps[tap].a, 0.0, 1.0));
    }
    let b = t[MOBILE_SHARPEN_NORTH_TAP];
    let d = t[MOBILE_SHARPEN_WEST_TAP];
    let f = t[MOBILE_SHARPEN_EAST_TAP];
    let h = t[MOBILE_SHARPEN_SOUTH_TAP];
    let a = t[MOBILE_SHARPEN_NORTH_WEST_TAP];
    let c = t[MOBILE_SHARPEN_NORTH_EAST_TAP];
    let g = t[MOBILE_SHARPEN_SOUTH_WEST_TAP];
    let i = t[MOBILE_SHARPEN_SOUTH_EAST_TAP];
    // Soft minimum and maximum: the cross's plus the whole window's.
    let cross_min = min(min(min(b, d), min(f, h)), e);
    let cross_max = max(max(max(b, d), max(f, h)), e);
    let soft_min = cross_min + min(cross_min, min(min(a, c), min(g, i)));
    let soft_max = cross_max + max(cross_max, max(max(a, c), max(g, i)));
    // The distance to the nearer limit over the maximum: small where the window already spans much of the range,
    // so hard edges are sharpened less than soft detail and nothing clips.
    let amplitude = sqrt(clamp(min(soft_min, vec3<f32>(MOBILE_SHARPEN_SOFT_LIMIT) - soft_max)
        / max(soft_max, vec3<f32>(MOBILE_SHARPEN_MIN_SOFT_MAX)), vec3<f32>(0.0), vec3<f32>(1.0)));
    let lobe = -amplitude * (clamp(strength, 0.0, 1.0) * MOBILE_SHARPEN_MAX_LOBE);
    let sharpened = (lobe * (b + d + f + h) + e) / (vec3<f32>(1.0) + MOBILE_SHARPEN_CROSS_TAPS * lobe);
    return vec4<f32>(decode_srgb(clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0))), centre.a);
}
