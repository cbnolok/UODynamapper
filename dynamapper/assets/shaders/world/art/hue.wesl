import package::shaders::world::texture_data::texture_data_rgb;
import package::shaders::world::ec_item_surface::ec_item_surface_record;

const STATIC_HUE_FLAG_APPLY: u32 = 1u;
const STATIC_HUE_FLAG_EC_SPRITE: u32 = 2u;
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

fn ec_sprite_mask_load(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                       header: vec4<u32>, pixel: vec2<i32>) -> f32 {
    let extent = vec2<u32>(header.w & 65535u, header.w >> 16u);
    let local = vec2<u32>(clamp(pixel, vec2<i32>(0), vec2<i32>(extent) - vec2<i32>(1)));
    let block = local / 256u;
    let source = ec_item_surface_record(lookup, header.x + block.y * header.z + block.x);
    return textureLoad(atlas, vec2<i32>(source.yz + local % 256u), i32(source.x), 0).a;
}

fn ec_sprite_mask(atlas: texture_2d_array<f32>, lookup: texture_2d<u32>,
                  header: vec4<u32>, uv: vec2<f32>, linear: bool) -> f32 {
    let extent = vec2<f32>(f32(header.w & 65535u), f32(header.w >> 16u));
    let pixel = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * extent;
    if (!linear) { return ec_sprite_mask_load(atlas, lookup, header, vec2<i32>(floor(pixel))); }
    // Explicit bounded taps preserve native alpha across packed-block seams,
    // including masks/crops larger than one auxiliary layer.
    let first = vec2<i32>(floor(pixel - vec2<f32>(0.5)));
    let weight = fract(pixel - vec2<f32>(0.5));
    let north = mix(ec_sprite_mask_load(atlas, lookup, header, first),
        ec_sprite_mask_load(atlas, lookup, header, first + vec2<i32>(1, 0)), weight.x);
    let south = mix(ec_sprite_mask_load(atlas, lookup, header, first + vec2<i32>(0, 1)),
        ec_sprite_mask_load(atlas, lookup, header, first + vec2<i32>(1)), weight.x);
    return mix(north, south, weight.y);
}

fn apply_ec_sprite_hue(color: vec4<f32>, hue_id: u32, header: vec4<u32>, uv: vec2<f32>,
    hues: texture_2d<f32>, hues_sampler: sampler, atlas: texture_2d_array<f32>,
    lookup: texture_2d<u32>, linear: bool) -> vec4<f32> {
    let dims = textureDimensions(hues);
    let origin = hue_strip_origin(hue_id);
    if (origin.x + u32(HUE_STRIP_WIDTH) > dims.x || origin.y >= dims.y) { return color; }
    var coverage = 1.0;
    if ((header.y & 512u) != 0u) {
        if (header.x == 0u) { return color; } // Declared source unavailable: retain authored art.
        coverage = ec_sprite_mask(atlas, lookup, header, uv, linear);
    }
    if (coverage == 0.0) { return color; }
    // UOSprite.psh uses encoded red, despite its "luminance" comment.
    let sample_pos = vec2<f32>(f32(origin.x) + texture_data_rgb(color.rgb).r * 255.0 + 0.5,
        f32(origin.y) + 0.5);
    let hue = textureSampleLevel(hues, hues_sampler, sample_pos / vec2<f32>(dims), 0.0);
    return mix(color, vec4<f32>(hue.rgb, color.a * hue.a), coverage);
}
