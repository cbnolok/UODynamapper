// ============================================================================
// art::shading — Shared fake-lighting helpers for 2D art and ground-art passes.
// ============================================================================

#import "shaders/world/land/noise.wgsl"::noise_2d

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

fn apply_art_atmosphere_depth(
    rgb: vec3<f32>,
    alpha: f32,
    world_pos: vec3<f32>,
    camera_pos: vec3<f32>,
    atmosphere_tint: vec3<f32>,
    fog_color: vec4<f32>,
    fog_night_color: vec4<f32>,
    fog_params: vec4<f32>,
    enable_fog: u32,
    visual_profile: u32,
) -> vec3<f32> {
    if (enable_fog != 1u || visual_profile != 2u) {
        return rgb;
    }

    let dist_density = clamp(fog_params.x, 0.0, 1.0);
    let height_density = clamp(fog_params.y, 0.0, 1.0);
    if (dist_density <= 0.0001 && height_density <= 0.0001) {
        return rgb;
    }

    let dist_tiles = distance(world_pos.xz, camera_pos.xz);
    let distance_term = smoothstep(96.0, 520.0, dist_tiles) * dist_density * 4.5;
    let height_term = smoothstep(2.0, 16.0, max(world_pos.y, 0.0)) * height_density * 7.0;
    let alpha_weight = smoothstep(0.05, 0.65, alpha);
    let amount = clamp((distance_term + height_term) * clamp(fog_color.a, 0.0, 0.45) * 0.30 * alpha_weight, 0.0, 0.10);
    if (amount <= 0.0001) {
        return rgb;
    }

    let night_blend = clamp(fog_night_color.a, 0.0, 1.0);
    let fog_tint = mix(fog_color.rgb, fog_night_color.rgb, night_blend);
    let depth_tint = mix(atmosphere_tint, fog_tint, 0.20);
    let luma = art_luminance(rgb);
    let softened = mix(rgb, vec3<f32>(luma), amount * 0.18);
    return max(mix(softened, depth_tint, amount), vec3<f32>(0.0));
}

fn art_fake_normal(uv_in_tile: vec2<f32>, depth_class: u32, is_ground_art: bool) -> vec3<f32> {
    let center = uv_in_tile - vec2<f32>(0.5);
    var side_slope = clamp(center.x * 1.65, -0.9, 0.9);

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
        side_slope *= 0.45;
    } else if (depth_class == ART_DEPTH_CLASS_ROOF) {
        vertical += 0.34;
        face = 0.48;
        side_slope *= 0.30;
    } else {
        side_slope *= 0.35;
    }

    return normalize(vec3<f32>(side_slope, vertical, face));
}

fn apply_art_surface_shading(
    rgb: vec3<f32>,
    uv_in_tile: vec2<f32>,
    world_pos: vec3<f32>,
    depth_class: u32,
    light_direction: vec3<f32>,
    light_color: vec3<f32>,
    atmosphere_tint: vec3<f32>,
    enable_art_fake_normals: u32,
    art_shadow_strength: f32,
    art_highlight_strength: f32,
    art_depth_tint_strength: f32,
    art_contact_shadow_strength: f32,
    art_mottle_strength: f32,
    light_decal_intensity: f32,
    kr_art_temperature_strength: f32,
    local_light_rgba: vec4<f32>,
    is_ground_art: bool,
) -> vec3<f32> {
    let shadow_strength = clamp(art_shadow_strength, 0.0, 1.0);
    let highlight_strength = clamp(art_highlight_strength, 0.0, 1.0);
    let tint_strength = clamp(art_depth_tint_strength, 0.0, 1.0);
    let contact_strength = clamp(art_contact_shadow_strength, 0.0, 1.0);
    let mottle_strength = clamp(art_mottle_strength, 0.0, 1.5);
    let light_static = clamp(light_decal_intensity, 0.0, 2.0);

    let vertical_light = clamp(1.0 - uv_in_tile.y, 0.0, 1.0);
    let lower_occlusion = smoothstep(0.18, 1.0, uv_in_tile.y);
    let side_contact = 1.0 - smoothstep(0.0, 0.18, min(uv_in_tile.x, 1.0 - uv_in_tile.x));
    let contact_noise = 1.0 + (noise_2d(world_pos.xz * 0.25) - 0.5) * 0.16 * mottle_strength;

    var out_rgb = rgb;
    out_rgb *= 1.0 - shadow_strength * (0.35 * lower_occlusion + 0.15 * side_contact) * contact_noise;
    out_rgb += rgb * light_color * vertical_light * highlight_strength * (0.25 + 0.25 * light_static);
    out_rgb = mix(out_rgb, out_rgb * atmosphere_tint, tint_strength * lower_occlusion);

    if (enable_art_fake_normals == 1u) {
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
        var shadow_profile = 1.0;
        var local_light_profile = 1.0;
        var saturation_profile = 0.82;
        var plane_catch_profile = 1.0;
        var foot_contact_profile = 1.0;
        var edge_profile = 0.0;
        var side_falloff_profile = 0.0;
        let side_plane = smoothstep(0.18, 0.5, abs(uv_in_tile.x - 0.5));
        let dapple = 1.0 + (noise_2d(world_pos.xz * 1.7 + uv_in_tile * 17.0) - 0.5) * mottle_strength;
        if (depth_class == ART_DEPTH_CLASS_FOLIAGE) {
            depth_scale = 1.12;
            contact_scale = 1.28;
            shadow_profile = 1.16;
            local_light_profile = 0.78;
            saturation_profile = 0.92;
            plane_catch_profile = dapple;
            foot_contact_profile = 1.18;
            edge_profile = side_plane;
            side_falloff_profile = 0.75;
        } else if (depth_class == ART_DEPTH_CLASS_ROOF) {
            depth_scale = 0.92;
            contact_scale = 0.70;
            shadow_profile = 0.86;
            local_light_profile = 0.72;
            saturation_profile = 0.74;
            plane_catch_profile = 0.72;
            foot_contact_profile = 0.50;
            edge_profile = side_plane * 0.25;
            side_falloff_profile = 0.22;
        } else if (depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
            depth_scale = 0.78;
            contact_scale = 0.55;
            shadow_profile = 0.76;
            local_light_profile = 0.62;
            saturation_profile = 0.78;
            plane_catch_profile = 0.58;
            foot_contact_profile = 0.42;
            edge_profile = side_plane * 0.35;
        } else if (!is_ground_art) {
            shadow_profile = 1.04;
            plane_catch_profile = 1.0;
            edge_profile = side_plane * 0.08;
            side_falloff_profile = 0.52;
        } else {
            foot_contact_profile = 0.55;
            edge_profile = side_plane * 0.35;
        }

        let warm_key = light_color * (0.42 + 0.78 * wrap) * highlight_strength * depth_scale * plane_catch_profile;
        let cool_shadow = mix(vec3<f32>(1.0), atmosphere_tint, tint_strength * (0.45 + 0.35 * contact));
        let contact_shadow = ground_contact * contact_strength * contact_scale;
        let shadow_cut = 1.0 - (
            shadow_strength * (0.24 + 0.38 * (1.0 - lambert)) * contact_noise * shadow_profile
            + contact_shadow * (0.20 + 0.26 * (1.0 - lambert))
        );
        let edge_catch = edge_profile * (0.18 + 0.12 * top_catch) * plane_catch_profile;

        out_rgb = rgb * cool_shadow * shadow_cut;
        out_rgb += rgb * warm_key * (0.25 + 0.75 * top_catch);
        out_rgb += light_color * edge_catch * highlight_strength * 0.22;
        out_rgb += rgb * local_light_rgba.rgb * local_light_rgba.a * light_static * local_light_profile * (0.18 + 0.62 * top_catch);
        let foot_contact_noise = 1.0 + (noise_2d(world_pos.xz * 0.8 + uv_in_tile * 11.0) - 0.5) * 0.24 * mottle_strength;
        let foot_contact = smoothstep(0.68, 1.0, uv_in_tile.y) * foot_contact_noise * foot_contact_profile;
        out_rgb *= 1.0 - foot_contact * contact_strength * (0.10 + 0.10 * shadow_strength);
        let art_temperature_strength = clamp(kr_art_temperature_strength, 0.0, 1.5);
        let sun_mask = smoothstep(0.34, 0.92, wrap) * (0.45 + 0.55 * top_catch);
        let shade_mask = clamp((1.0 - smoothstep(0.22, 0.72, lambert)) * (0.35 + 0.65 * contact), 0.0, 1.0);
        let shaded_luma = art_luminance(out_rgb);
        out_rgb = mix(out_rgb, vec3<f32>(shaded_luma), shade_mask * shadow_strength * 0.14 * art_temperature_strength);
        out_rgb *= mix(vec3<f32>(1.0), vec3<f32>(0.84, 0.91, 1.08), shade_mask * 0.18 * art_temperature_strength);
        out_rgb *= mix(vec3<f32>(1.0), vec3<f32>(1.07, 1.02, 0.93), sun_mask * highlight_strength * 0.12 * art_temperature_strength);
        let light_opposed_side = select(uv_in_tile.x, 1.0 - uv_in_tile.x, L.x >= 0.0);
        let side_height_mask = smoothstep(0.12, 0.94, uv_in_tile.y) * (1.0 - top_catch * 0.55);
        let side_falloff = smoothstep(0.42, 0.96, light_opposed_side) * side_height_mask * side_falloff_profile;
        let side_falloff_strength = clamp(side_falloff * tint_strength * (0.055 + 0.055 * contact_strength) * art_temperature_strength, 0.0, 0.16);
        out_rgb = mix(out_rgb, out_rgb * vec3<f32>(0.90, 0.94, 1.04), side_falloff_strength);
        out_rgb = mix(out_rgb, art_saturate(out_rgb, saturation_profile), clamp(contact * shadow_strength * 0.35, 0.0, 1.0));
    }

    if (enable_art_fake_normals != 1u && local_light_rgba.a > 0.001) {
        let top_catch = pow(max(1.0 - uv_in_tile.y, 0.0), 1.7);
        out_rgb += rgb * local_light_rgba.rgb * local_light_rgba.a * light_static * (0.18 + 0.62 * top_catch);
    }

    if (mottle_strength > 0.0) {
        var mottle_profile = 0.10;
        if (depth_class == ART_DEPTH_CLASS_ROOF) {
            mottle_profile = 0.28;
        } else if (depth_class == ART_DEPTH_CLASS_FOLIAGE) {
            mottle_profile = 0.16;
        } else if (is_ground_art || depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
            mottle_profile = 0.08;
        }

        let broad_mottle = noise_2d(world_pos.xz * 0.55 + uv_in_tile * 2.0);
        let fine_mottle = noise_2d(world_pos.xz * 2.8 + uv_in_tile * 17.0);
        let mottle = ((broad_mottle - 0.5) + (fine_mottle - 0.5) * 0.35) * mottle_profile * mottle_strength;
        out_rgb *= 1.0 + mottle;
    }

    return max(out_rgb, vec3<f32>(0.0));
}
