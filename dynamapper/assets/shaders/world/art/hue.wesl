const STATIC_HUE_FLAG_APPLY: u32 = 1u;
const HUE_STRIP_WIDTH: f32 = 256.0;
const HUE_ATLAS_HEIGHT: u32 = 1024u;

fn hue_strip_origin(hue_id: u32) -> vec2<u32> {
    if (hue_id < HUE_ATLAS_HEIGHT) {
        return vec2<u32>(0u, hue_id);
    }

    let adjusted = hue_id - HUE_ATLAS_HEIGHT;
    let column = 1u + (adjusted / HUE_ATLAS_HEIGHT);
    let row = adjusted % HUE_ATLAS_HEIGHT;
    return vec2<u32>(column * u32(HUE_STRIP_WIDTH), row);
}

fn apply_static_hue(
    color: vec4<f32>,
    hue_id: u32,
    hue_flags: u32,
    hue_enabled: u32,
    hues: texture_2d<f32>,
    hues_sampler: sampler,
) -> vec4<f32> {
    if (hue_enabled == 0u || hue_id == 0u || (hue_flags & STATIC_HUE_FLAG_APPLY) == 0u) {
        return color;
    }

    let dims = textureDimensions(hues);
    let origin = hue_strip_origin(hue_id);
    if (origin.x + u32(HUE_STRIP_WIDTH) > dims.x || origin.y >= dims.y) {
        return color;
    }

    let luma = clamp(dot(color.rgb, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 1.0);
    let sample_pos = vec2<f32>(
        f32(origin.x) + luma * (HUE_STRIP_WIDTH - 1.0) + 0.5,
        f32(origin.y) + 0.5,
    );
    let hue_uv = sample_pos / vec2<f32>(dims);
    let hue_color = textureSample(hues, hues_sampler, hue_uv);

    return vec4<f32>(hue_color.rgb, color.a * hue_color.a);
}
