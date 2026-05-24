// ============================================================================
// art::shading — Shared fake-lighting helpers for 2D art and ground-art passes.
// ============================================================================

#import "shaders/world/common_bindings.wgsl"::{LandEffectsUniform, GlobalLightingUniforms}
#import "shaders/world/land/noise.wgsl"::hash

const ART_DEPTH_CLASS_FOLIAGE: u32 = 2u;
const ART_DEPTH_CLASS_ROOF: u32 = 3u;
const ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR: u32 = 4u;

fn art_luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn art_saturate(c: vec3<f32>, saturation: f32) -> vec3<f32> {
    let luma = art_luminance(c);
    return mix(vec3<f32>(luma), c, saturation);
}

fn art_fake_normal(uv_in_tile: vec2<f32>, depth_class: u32, is_ground_art: bool) -> vec3<f32> {
    let center = uv_in_tile - vec2<f32>(0.5);
    let side_slope = clamp(center.x * 1.65, -0.9, 0.9);

    if (is_ground_art || depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
        let bevel_x = (smoothstep(0.0, 0.18, uv_in_tile.x) - smoothstep(0.82, 1.0, uv_in_tile.x)) * 0.32;
        let bevel_z = (smoothstep(0.0, 0.18, uv_in_tile.y) - smoothstep(0.82, 1.0, uv_in_tile.y)) * 0.32;
        return normalize(vec3<f32>(bevel_x, 1.0, bevel_z));
    }

    var vertical = 0.38 + (1.0 - uv_in_tile.y) * 0.72;
    var face = 0.82;
    if (depth_class == ART_DEPTH_CLASS_FOLIAGE) {
        vertical += 0.20;
        face = 0.62;
    } else if (depth_class == ART_DEPTH_CLASS_ROOF) {
        vertical += 0.34;
        face = 0.48;
    }

    return normalize(vec3<f32>(side_slope, vertical, face));
}

fn apply_art_surface_shading(
    rgb: vec3<f32>,
    uv_in_tile: vec2<f32>,
    world_pos: vec3<f32>,
    depth_class: u32,
    light_direction: vec3<f32>,
    effects: LandEffectsUniform,
    global_light: GlobalLightingUniforms,
    local_light_rgba: vec4<f32>,
    is_ground_art: bool,
) -> vec3<f32> {
    let shadow_strength = clamp(effects.art_shadow_strength, 0.0, 1.0);
    let highlight_strength = clamp(effects.art_highlight_strength, 0.0, 1.0);
    let tint_strength = clamp(effects.art_depth_tint_strength, 0.0, 1.0);
    let contact_strength = clamp(effects.art_contact_shadow_strength, 0.0, 1.0);
    let light_static = clamp(effects.light_decal_intensity, 0.0, 2.0);

    let vertical_light = clamp(1.0 - uv_in_tile.y, 0.0, 1.0);
    let lower_occlusion = smoothstep(0.18, 1.0, uv_in_tile.y);
    let side_contact = 1.0 - smoothstep(0.0, 0.18, min(uv_in_tile.x, 1.0 - uv_in_tile.x));
    let contact_noise = 0.85 + 0.15 * hash(floor(world_pos.xz * 0.25));

    var out_rgb = rgb;
    out_rgb *= 1.0 - shadow_strength * (0.35 * lower_occlusion + 0.15 * side_contact) * contact_noise;
    out_rgb += rgb * global_light.light_color * vertical_light * highlight_strength * (0.25 + 0.25 * light_static);
    out_rgb = mix(out_rgb, out_rgb * global_light.atmosphere_tint, tint_strength * lower_occlusion);

    if (effects.enable_art_fake_normals == 1u && effects.shading_mode == 2u) {
        let N = art_fake_normal(uv_in_tile, depth_class, is_ground_art);
        let L = normalize(light_direction);
        let lambert = max(dot(N, L), 0.0);
        let wrap = max(lambert * 0.75 + 0.25, 0.0);
        let top_catch = pow(max(1.0 - uv_in_tile.y, 0.0), 1.7);
        let contact = smoothstep(0.38, 1.0, uv_in_tile.y);
        let ground_contact = select(
            smoothstep(0.62, 1.0, uv_in_tile.y),
            1.0 - smoothstep(0.0, 0.28, min(min(uv_in_tile.x, 1.0 - uv_in_tile.x), min(uv_in_tile.y, 1.0 - uv_in_tile.y))),
            is_ground_art || depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR,
        );

        var depth_scale = 1.0;
        var contact_scale = 1.0;
        if (depth_class == ART_DEPTH_CLASS_FOLIAGE) {
            depth_scale = 1.12;
            contact_scale = 1.28;
        } else if (depth_class == ART_DEPTH_CLASS_ROOF) {
            depth_scale = 0.92;
            contact_scale = 0.70;
        } else if (depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
            depth_scale = 0.78;
            contact_scale = 0.55;
        }

        let warm_key = global_light.light_color * (0.42 + 0.78 * wrap) * highlight_strength * depth_scale;
        let cool_shadow = mix(vec3<f32>(1.0), global_light.atmosphere_tint, tint_strength * (0.45 + 0.35 * contact));
        let contact_shadow = ground_contact * contact_strength * contact_scale;
        let shadow_cut = 1.0 - (
            shadow_strength * (0.24 + 0.38 * (1.0 - lambert)) * contact_noise
            + contact_shadow * (0.20 + 0.26 * (1.0 - lambert))
        );
        let edge_catch = smoothstep(0.34, 0.5, abs(uv_in_tile.x - 0.5)) * (0.18 + 0.12 * top_catch);

        out_rgb = rgb * cool_shadow * shadow_cut;
        out_rgb += rgb * warm_key * (0.25 + 0.75 * top_catch);
        out_rgb += global_light.light_color * edge_catch * highlight_strength * 0.22;
        out_rgb += rgb * local_light_rgba.rgb * local_light_rgba.a * light_static * (0.18 + 0.62 * top_catch);
        out_rgb = mix(out_rgb, art_saturate(out_rgb, 0.82), clamp(contact * shadow_strength * 0.35, 0.0, 1.0));
    }

    return max(out_rgb, vec3<f32>(0.0));
}
