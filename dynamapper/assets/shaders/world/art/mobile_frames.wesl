// ============================================================================
// art::mobile_frames — A mobile's two frames on one card, crossfaded (render/mobiles/in_between.rs), for the mobiles'
// pipelines (MOBILE_FRAMES). The card spans both frames with their anchors on one point, its uv in the first frame's
// coordinates; the next frame lies `next_uv_offset` from them, on its own page `next_layer`. Each frame shows inside
// its own rectangle only, so a tap never reads a frame's atlas neighbours, and the card holds what a picture of the
// two frames side by side on a transparent card would. The two taps blend premultiplied on their sRGB-encoded values,
// as the clients' 8-bit blending would mix the frames; a blend of the linear values brightens the mix of a dark and a
// light texel.
// ============================================================================

import package::shaders::world::art::art_bindings::FrameCrossfade;
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

// One tap of the card at `uv` (the first frame's coordinates, on page `layer`): both frames' taps, blended.
fn mobile_frames_tap(atlas: texture_2d_array<f32>, atlas_sampler: sampler, layer: i32, uv: vec2<f32>,
    crossfade: FrameCrossfade) -> vec4<f32> {
    return mobile_frames_mix(
        mobile_frame_tap(atlas, atlas_sampler, layer, uv, crossfade.frame_uv_min, crossfade.frame_uv_max),
        mobile_frame_tap(atlas, atlas_sampler, i32(crossfade.next_layer), uv + crossfade.next_uv_offset,
            crossfade.next_uv_min, crossfade.next_uv_max),
        crossfade.next_weight);
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

// The client's filter on both frames of the card, blended. `uv`, the card's span `uv_min`..`uv_max` and the
// derivatives are atlas-normalized, in the first frame's coordinates on page `layer` (client_filtered_atlas_rect).
fn mobile_frames_client_filtered(atlas: texture_2d_array<f32>, layer: i32, uv: vec2<f32>, uv_min: vec2<f32>,
    uv_max: vec2<f32>, crossfade: FrameCrossfade, uv_dx: vec2<f32>, uv_dy: vec2<f32>, client_view_scale: f32)
    -> vec4<f32> {
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
    return mobile_frames_mix(first, next, crossfade.next_weight);
}
