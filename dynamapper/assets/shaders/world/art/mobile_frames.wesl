// ============================================================================
// art::mobile_frames — A mobile's two frames on one card, crossfaded (render/mobiles/in_between.rs), for the mobiles'
// pipelines (MOBILE_FRAMES). The card spans both frames with their anchors on one point, its uv in the first frame's
// coordinates; the next frame lies `next_uv_offset` from them, on its own page `next_layer`. Each frame shows inside
// its own rectangle only, so a tap never reads a frame's atlas neighbours, and the card holds what a picture of the
// two frames side by side on a transparent card would. The two taps blend premultiplied on their sRGB-encoded values,
// as the clients' 8-bit blending would mix the frames; a blend of the linear values brightens the mix of a dark and a
// light texel. Each frame's alpha takes the frame filter's contrast before the blend (art/mobile_edges.wesl), so the
// crossfade between two contrasted frames stays as soft as between two stored ones. A card padded for an outline reads
// its one frame through the same rectangle test (`mobile_frame_alone_tap`), and the outline's taps of the frames' alpha
// are gathered (`mobile_frames_cross_alpha`, `mobile_frames_bilinear_alpha`).
// ============================================================================

import package::shaders::world::art::art_bindings::{FrameCrossfade, MobileFrameFilter};
import package::shaders::world::art::mobile_edges::{mobile_alpha_contrast, mobile_alpha_contrast_alphas};
import package::shaders::world::client_filter::{client_mip_box, client_premultiplied, client_rect_texel,
    client_straight, CLIENT_FILTER_BOX_TAP_WEIGHT};
import package::shaders::world::srgb::{encode_srgb, decode_srgb};

// Whether the card shows both frames blended; with a weight of 0 it is the first frame's own.
fn mobile_frames_blend(crossfade: FrameCrossfade) -> bool {
    return crossfade.next_weight > 0.0;
}

// The two frames' colours, straight alpha and linear, mixed by the next one's `weight`: colour and alpha
// premultiplied and blended on the sRGB-encoded values, then straight and linear again. A transparent texel lends
// no colour.
fn mobile_frames_mix(first: vec4<f32>, next: vec4<f32>, weight: f32) -> vec4<f32> {
    let alpha = mix(first.a, next.a, weight);
    if (alpha <= 0.0) {
        return vec4<f32>(0.0);
    }
    let premultiplied = mix(encode_srgb(first.rgb) * first.a, encode_srgb(next.rgb) * next.a, weight);
    return vec4<f32>(decode_srgb(premultiplied / alpha), alpha);
}

// One tap of a frame at `uv`, transparent outside its rectangle `uv_min`..`uv_max`. The atlas samples nearest, so
// the texel read is the frame's wherever `uv` lies inside.
fn mobile_frame_tap(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, uv: vec2<f32>,
    uv_min: vec2<f32>, uv_max: vec2<f32>) -> vec4<f32> {
    if (any(uv < uv_min) || any(uv >= uv_max)) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(atlas, atlas_sampler, uv, layer, 0.0);
}

// One tap of the card at `uv` (the first frame's coordinates, on page `layer`): both frames' taps, each with the
// filter's alpha contrast, blended.
fn mobile_frames_tap(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, uv: vec2<f32>,
    crossfade: FrameCrossfade, frame_filter: MobileFrameFilter) -> vec4<f32> {
    return mobile_frames_mix(
        mobile_alpha_contrast(mobile_frame_tap(atlas, atlas_sampler, layer, uv, crossfade.frame_uv_min,
            crossfade.frame_uv_max), frame_filter),
        mobile_alpha_contrast(mobile_frame_tap(atlas, atlas_sampler, i32(crossfade.next_layer),
            uv + crossfade.next_uv_offset, crossfade.next_uv_min, crossfade.next_uv_max), frame_filter),
        crossfade.next_weight);
}

// One tap of a card that shows its first frame alone, transparent outside the frame's rectangle: the card padded
// beyond the frame for an outline reads nothing of the frame's atlas neighbours.
fn mobile_frame_alone_tap(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, uv: vec2<f32>,
    crossfade: FrameCrossfade, frame_filter: MobileFrameFilter) -> vec4<f32> {
    return mobile_alpha_contrast(mobile_frame_tap(atlas, atlas_sampler, layer, uv, crossfade.frame_uv_min,
        crossfade.frame_uv_max), frame_filter);
}

// A texel of a frame's page, transparent outside the frame's texels `frame_min`..`frame_max`.
fn mobile_frame_texel(atlas: texture_2d_array<f32>, layer: i32, texel: vec2<i32>, frame_min: vec2<i32>,
    frame_max: vec2<i32>) -> vec4<f32> {
    let inside = all(texel >= frame_min) && all(texel < frame_max);
    let loaded = textureLoad(atlas, clamp(texel, frame_min, frame_max - vec2<i32>(1)), layer, 0);
    return select(vec4<f32>(0.0), loaded, inside);
}

// One bilinear tap of a frame on the card, premultiplied, as client_bilinear_rect on the card's rectangle (`origin`,
// `extent` texels on the frame's page), the card's texels outside the frame transparent.
fn mobile_frame_bilinear(atlas: texture_2d_array<f32>, layer: i32, origin: vec2<i32>, extent: vec2<i32>,
    frame_min: vec2<i32>, frame_max: vec2<i32>, px: vec2<f32>) -> vec4<f32> {
    let corner = px - vec2<f32>(0.5);
    let first = vec2<i32>(floor(corner));
    let weight = corner - floor(corner);
    let t0 = origin + client_rect_texel(first, extent, false);
    let t1 = origin + client_rect_texel(first + vec2<i32>(1), extent, false);
    let c00 = client_premultiplied(mobile_frame_texel(atlas, layer, t0, frame_min, frame_max));
    let c10 = client_premultiplied(mobile_frame_texel(atlas, layer, vec2<i32>(t1.x, t0.y), frame_min, frame_max));
    let c01 = client_premultiplied(mobile_frame_texel(atlas, layer, vec2<i32>(t0.x, t1.y), frame_min, frame_max));
    let c11 = client_premultiplied(mobile_frame_texel(atlas, layer, t1, frame_min, frame_max));
    return mix(mix(c00, c10, weight.x), mix(c01, c11, weight.x), weight.y);
}

// The client's filter (client_filtered_rect) on a frame of the card: one bilinear tap, or the emulated mip box of
// four, blended premultiplied and returned straight.
fn mobile_frame_client_filtered(atlas: texture_2d_array<f32>, layer: i32, origin: vec2<i32>, extent: vec2<i32>,
    frame_min: vec2<i32>, frame_max: vec2<i32>, px: vec2<f32>, dx_texels: vec2<f32>, dy_texels: vec2<f32>,
    client_view_scale: f32) -> vec4<f32> {
    let box = client_mip_box(dx_texels, dy_texels, client_view_scale);
    if (!box.enabled) {
        return client_straight(mobile_frame_bilinear(atlas, layer, origin, extent, frame_min, frame_max, px));
    }
    return client_straight((
        mobile_frame_bilinear(atlas, layer, origin, extent, frame_min, frame_max, px - box.offset_x - box.offset_y)
        + mobile_frame_bilinear(atlas, layer, origin, extent, frame_min, frame_max, px + box.offset_x - box.offset_y)
        + mobile_frame_bilinear(atlas, layer, origin, extent, frame_min, frame_max, px - box.offset_x + box.offset_y)
        + mobile_frame_bilinear(atlas, layer, origin, extent, frame_min, frame_max, px + box.offset_x + box.offset_y))
        * CLIENT_FILTER_BOX_TAP_WEIGHT);
}

// The client's filter on both frames of the card, each with the filter's alpha contrast, blended. `uv`, the card's
// span `uv_min`..`uv_max` and the derivatives are atlas-normalized, in the first frame's coordinates on page `layer`
// (client_filtered_atlas_rect).
fn mobile_frames_client_filtered(atlas: texture_2d_array<f32>, layer: i32, uv: vec2<f32>, uv_min: vec2<f32>,
    uv_max: vec2<f32>, crossfade: FrameCrossfade, uv_dx: vec2<f32>, uv_dy: vec2<f32>, client_view_scale: f32,
    frame_filter: MobileFrameFilter) -> vec4<f32> {
    let atlas_size = vec2<f32>(textureDimensions(atlas).xy);
    let origin = vec2<i32>(round(uv_min * atlas_size));
    let extent = max(vec2<i32>(round((uv_max - uv_min) * atlas_size)), vec2<i32>(1));
    let next_origin = origin + vec2<i32>(round(crossfade.next_uv_offset * atlas_size));
    let px = (uv - uv_min) * atlas_size;
    let dx_texels = uv_dx * atlas_size;
    let dy_texels = uv_dy * atlas_size;
    let first = mobile_frame_client_filtered(atlas, layer, origin, extent,
        vec2<i32>(round(crossfade.frame_uv_min * atlas_size)), vec2<i32>(round(crossfade.frame_uv_max * atlas_size)),
        px, dx_texels, dy_texels, client_view_scale);
    let next = mobile_frame_client_filtered(atlas, i32(crossfade.next_layer), next_origin, extent,
        vec2<i32>(round(crossfade.next_uv_min * atlas_size)), vec2<i32>(round(crossfade.next_uv_max * atlas_size)),
        px, dx_texels, dy_texels, client_view_scale);
    return mobile_frames_mix(mobile_alpha_contrast(first, frame_filter), mobile_alpha_contrast(next, frame_filter),
        crossfade.next_weight);
}

// The client's filter on a card that shows its first frame alone, the card's texels outside the frame transparent
// (a card padded for an outline), with the filter's alpha contrast. The arguments as mobile_frames_client_filtered's.
fn mobile_frame_alone_client_filtered(atlas: texture_2d_array<f32>, layer: i32, uv: vec2<f32>, uv_min: vec2<f32>,
    uv_max: vec2<f32>, crossfade: FrameCrossfade, uv_dx: vec2<f32>, uv_dy: vec2<f32>, client_view_scale: f32,
    frame_filter: MobileFrameFilter) -> vec4<f32> {
    let atlas_size = vec2<f32>(textureDimensions(atlas).xy);
    let origin = vec2<i32>(round(uv_min * atlas_size));
    let extent = max(vec2<i32>(round((uv_max - uv_min) * atlas_size)), vec2<i32>(1));
    return mobile_alpha_contrast(mobile_frame_client_filtered(atlas, layer, origin, extent,
        vec2<i32>(round(crossfade.frame_uv_min * atlas_size)), vec2<i32>(round(crossfade.frame_uv_max * atlas_size)),
        (uv - uv_min) * atlas_size, uv_dx * atlas_size, uv_dy * atlas_size, client_view_scale), frame_filter);
}

// The channel a gather of the frames' alpha reads.
const MOBILE_FRAME_ALPHA_COMPONENT: u32 = 3u;

// The alphas of the 2 x 2 texels of page `layer` around the texel corner `corner`, each 0 outside the frame's texels
// `frame_min`..`frame_max`, in the order of a gather: x the lower left texel (corner - (1, 0)), y the lower right
// (corner), z the upper right (corner - (0, 1)), w the upper left (corner - (1, 1)). One gather at the corner reads
// those four texels whatever the sampler's filter.
fn mobile_frame_gathered_alpha(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, corner: vec2<i32>,
    atlas_size: vec2<f32>, frame_min: vec2<i32>, frame_max: vec2<i32>) -> vec4<f32> {
    let gathered = textureGather(MOBILE_FRAME_ALPHA_COMPONENT, atlas, atlas_sampler, vec2<f32>(corner) / atlas_size,
        layer);
    let low = corner - vec2<i32>(1);
    let low_inside = (low >= frame_min) & (low < frame_max);
    let high_inside = (corner >= frame_min) & (corner < frame_max);
    let inside = vec4<bool>(low_inside.x && high_inside.y, high_inside.x && high_inside.y,
        high_inside.x && low_inside.y, low_inside.x && low_inside.y);
    return select(vec4<f32>(0.0), gathered, inside);
}

// The alphas of the texels north, west, east and south of `texel` on page `layer`, each 0 outside the frame's texels
// `frame_min`..`frame_max`: two gathers, at the texel's upper left and lower right corners.
fn mobile_frame_cross_alpha(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, texel: vec2<i32>,
    atlas_size: vec2<f32>, frame_min: vec2<i32>, frame_max: vec2<i32>) -> vec4<f32> {
    let upper_left = mobile_frame_gathered_alpha(atlas, atlas_sampler, layer, texel, atlas_size, frame_min, frame_max);
    let lower_right = mobile_frame_gathered_alpha(atlas, atlas_sampler, layer, texel + vec2<i32>(1), atlas_size,
        frame_min, frame_max);
    return vec4<f32>(upper_left.z, upper_left.x, lower_right.z, lower_right.x);
}

// The alphas of the nearest taps north, west, east and south of the card's texel `texel` (on page `layer`, in the first
// frame's coordinates), as mobile_frames_tap and mobile_frame_alone_tap take them on a card padded for an outline: each
// frame's with the filter's alpha contrast, transparent outside its rectangle, the two blended while they crossfade.
fn mobile_frames_cross_alpha(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, texel: vec2<i32>,
    crossfade: FrameCrossfade, frame_filter: MobileFrameFilter) -> vec4<f32> {
    let atlas_size = vec2<f32>(textureDimensions(atlas).xy);
    let first = mobile_alpha_contrast_alphas(mobile_frame_cross_alpha(atlas, atlas_sampler, layer, texel, atlas_size,
        vec2<i32>(round(crossfade.frame_uv_min * atlas_size)), vec2<i32>(round(crossfade.frame_uv_max * atlas_size))),
        frame_filter);
    if (!mobile_frames_blend(crossfade)) {
        return first;
    }
    let next = mobile_alpha_contrast_alphas(mobile_frame_cross_alpha(atlas, atlas_sampler, i32(crossfade.next_layer),
        texel + vec2<i32>(round(crossfade.next_uv_offset * atlas_size)), atlas_size,
        vec2<i32>(round(crossfade.next_uv_min * atlas_size)), vec2<i32>(round(crossfade.next_uv_max * atlas_size))),
        frame_filter);
    return mix(first, next, crossfade.next_weight);
}

// The alpha of one bilinear tap of a frame on the card, as mobile_frame_bilinear's: the card's rectangle starts at
// texel `origin` of page `layer`, `px` is in texels from it, the texels outside the frame transparent. One gather
// reads the tap's four texels; the card's clamp to its own texels is left out, as an outlined card's edge texels lie
// outside the frame and read transparent either way.
fn mobile_frame_bilinear_alpha(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, origin: vec2<i32>,
    frame_min: vec2<i32>, frame_max: vec2<i32>, px: vec2<f32>, atlas_size: vec2<f32>) -> f32 {
    let corner = px - vec2<f32>(0.5);
    let weight = corner - floor(corner);
    let texels = mobile_frame_gathered_alpha(atlas, atlas_sampler, layer,
        origin + vec2<i32>(floor(corner)) + vec2<i32>(1), atlas_size, frame_min, frame_max);
    return mix(mix(texels.w, texels.z, weight.x), mix(texels.x, texels.y, weight.x), weight.y);
}

// The alpha of the client's filter at `uv` on a card padded for an outline, one bilinear tap (inside the client's view,
// where its emulated mip box is empty), as mobile_frames_client_filtered and mobile_frame_alone_client_filtered take
// it: each frame's with the filter's alpha contrast, the two blended while they crossfade. `uv` and the card's
// `uv_min` are atlas-normalized, in the first frame's coordinates on page `layer`.
fn mobile_frames_bilinear_alpha(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, uv: vec2<f32>,
    uv_min: vec2<f32>, crossfade: FrameCrossfade, frame_filter: MobileFrameFilter) -> f32 {
    let atlas_size = vec2<f32>(textureDimensions(atlas).xy);
    let origin = vec2<i32>(round(uv_min * atlas_size));
    let px = (uv - uv_min) * atlas_size;
    let first = mobile_alpha_contrast(vec4<f32>(0.0, 0.0, 0.0, mobile_frame_bilinear_alpha(atlas, atlas_sampler, layer,
        origin, vec2<i32>(round(crossfade.frame_uv_min * atlas_size)),
        vec2<i32>(round(crossfade.frame_uv_max * atlas_size)), px, atlas_size)), frame_filter).a;
    if (!mobile_frames_blend(crossfade)) {
        return first;
    }
    let next = mobile_alpha_contrast(vec4<f32>(0.0, 0.0, 0.0, mobile_frame_bilinear_alpha(atlas, atlas_sampler,
        i32(crossfade.next_layer), origin + vec2<i32>(round(crossfade.next_uv_offset * atlas_size)),
        vec2<i32>(round(crossfade.next_uv_min * atlas_size)), vec2<i32>(round(crossfade.next_uv_max * atlas_size)), px,
        atlas_size)), frame_filter).a;
    return mix(first, next, crossfade.next_weight);
}
