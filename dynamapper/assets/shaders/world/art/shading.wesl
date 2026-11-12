// ============================================================================
// art::shading — Shared fake-lighting helpers for 2D art and ground-art passes.
// ============================================================================

import package::shaders::world::land::noise::noise_2d;

const ART_DEPTH_CLASS_BACKGROUND: u32 = 1u;
const ART_DEPTH_CLASS_FOLIAGE: u32 = 2u;
const ART_DEPTH_CLASS_ROOF: u32 = 3u;
const ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR: u32 = 4u;

// Vertical-edge shading profiles for plain upright billboards (Regular/Background depth
// classes — walls, bridges, docks, furniture; anything that isn't foliage/roof/floor/ground).
// These sprites are flat rectangular cards with no real beveled vertical edges, so any shading
// keyed to horizontal-UV distance from the sprite's left/right margins paints a spurious
// bright/dark "frame" around the sprite silhouette: seen in-game as a highlighted "vertical
// margin" on plaster/stucco walls (from the additive edge_catch term this drives) and a
// "boxed shadow" on Background bridge/dock tiles (from the one-sided side_falloff this drives).
// Because both terms are smoothstep ramps keyed to the UV rectangle's edges, they produce a
// visible border at essentially any nonzero strength, so they are removed structurally here
// rather than tuned down. The Lambertian directional shading (side_slope normal -> shadow_cut
// and warm_key top highlight) is retained, so upright billboards still get sensible 3D-ish
// lighting — just without the edge frame. Erred conservative (edge polish fully removed);
// re-tune these upward if in-game visual review wants a subtle margin cue back.
const ART_UPRIGHT_BILLBOARD_EDGE_PROFILE: f32 = 0.0;
const ART_UPRIGHT_BILLBOARD_SIDE_FALLOFF_PROFILE: f32 = 0.0;

// How much of a static local light an art pixel catches: a base share everywhere plus a top catch, which on an
// upright sprite ramps from its foot (0) to its top (1) with this exponent.
const ART_LOCAL_LIGHT_BASE_CATCH: f32 = 0.18;
const ART_LOCAL_LIGHT_TOP_CATCH: f32 = 0.62;
const ART_LOCAL_LIGHT_TOP_CATCH_EXPONENT: f32 = 1.7;
// A flat static (a floor, a flat roof, the ground art) has no top to catch more of the light: it takes the ramp's mean
// over the sprite height, the integral of (1 - v)^exponent, evenly. Ramping it per sprite drew every tile of a lit
// floor or roof brighter at its far edge.
const ART_LOCAL_LIGHT_FLAT_TOP_CATCH: f32 = 1.0 / (1.0 + ART_LOCAL_LIGHT_TOP_CATCH_EXPONENT);
// SpriteInstance is_wet_flags bit: the upright card shows flat art, a floor or a flat roof. Keep in sync with
// STATIC_FLAG_FLAT_ART in statics_collect.rs.
const STATIC_FLAG_FLAT_ART: u32 = 1u << 11u;

// The static local light at a unit quad corner (x, z), from the instance's four packed corners
// (statics_collect.rs `static_local_light_corners`); the vertex shaders pass it on for the rasterizer to interpolate.
fn art_local_light_at_corner(corners: array<u32, 4>, corner: vec2<f32>) -> vec4<f32> {
    return unpack4x8unorm(corners[u32(corner.x + 0.5) + 2u * u32(corner.y + 0.5)]);
}

// The share of a static local light an art pixel catches (the ART_LOCAL_LIGHT_* constants above).
fn art_local_light_catch(uv_in_tile: vec2<f32>, flat_art: bool) -> f32 {
    let top_catch = select(pow(max(1.0 - uv_in_tile.y, 0.0), ART_LOCAL_LIGHT_TOP_CATCH_EXPONENT),
        ART_LOCAL_LIGHT_FLAT_TOP_CATCH, flat_art);
    return ART_LOCAL_LIGHT_BASE_CATCH + ART_LOCAL_LIGHT_TOP_CATCH * top_catch;
}

fn art_luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn art_saturate(c: vec3<f32>, saturation: f32) -> vec3<f32> {
    let luma = art_luminance(c);
    return mix(vec3<f32>(luma), c, saturation);
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
    local_light_intensity: f32,
    kr_art_temperature_strength: f32,
    local_light_rgba: vec4<f32>,
    is_ground_art: bool,
    // The art is flat (a flat quad, or an upright card showing a floor or flat roof): it catches the local lights
    // evenly.
    flat_art: bool,
) -> vec3<f32> {
    let shadow_strength = clamp(art_shadow_strength, 0.0, 1.0);
    let highlight_strength = clamp(art_highlight_strength, 0.0, 1.0);
    let tint_strength = clamp(art_depth_tint_strength, 0.0, 1.0);
    let contact_strength = clamp(art_contact_shadow_strength, 0.0, 1.0);
    let mottle_strength = clamp(art_mottle_strength, 0.0, 1.5);
    let light_static = clamp(light_decal_intensity, 0.0, 2.0) * local_light_intensity;

    let vertical_light = clamp(1.0 - uv_in_tile.y, 0.0, 1.0);
    let lower_occlusion = smoothstep(0.18, 1.0, uv_in_tile.y);
    let side_contact = 1.0 - smoothstep(0.0, 0.18, min(uv_in_tile.x, 1.0 - uv_in_tile.x));
    let contact_noise = 1.0 + (noise_2d(world_pos.xz * 0.25) - 0.5) * 0.16 * mottle_strength;

    var out_rgb = rgb;
    out_rgb *= 1.0 - shadow_strength * (0.35 * lower_occlusion + 0.15 * side_contact) * contact_noise;
    // Global light catch belongs to its own strength control. Local-light
    // intensity must not brighten every sprite when the collector is empty.
    // 0.5 preserves the existing response at neutral interaction intensity 1.
    out_rgb += rgb * light_color * vertical_light * highlight_strength * 0.5;
    // Atmosphere tint uses `lower_occlusion` (a per-V-coordinate gradient) which is
    // only meaningful on an upright billboard. On flat floor/ground/background tiles
    // it would darken/tint by an arbitrary vertical gradient, so skip it there.
    let is_floor_like =
        is_ground_art
        || depth_class == ART_DEPTH_CLASS_SURFACE_LIKE_FLOOR
        || depth_class == ART_DEPTH_CLASS_BACKGROUND;
    let depth_tint_weight = select(tint_strength * lower_occlusion, 0.0, is_floor_like);
    out_rgb = mix(out_rgb, out_rgb * atmosphere_tint, depth_tint_weight);

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
            // SurfaceLikeFloor tiles that are NOT wet still stand up as ordinary upright
            // billboards (e.g. a bridge deck: SURFACE-flagged but not WET, so it never enters
            // the flat-quad path and is_ground_art is false here) — same spurious edge-frame
            // artifact as the plain upright-billboard branch below, so zero it the same way.
            // A genuinely flat SurfaceLikeFloor instance (is_ground_art true) keeps the
            // original edge_profile; in practice real CC water never reaches this branch (it
            // classifies Background, checked earlier in resolve_static_depth_class), so this
            // case is untested but left as designed rather than assumed away.
            edge_profile = select(side_plane * 0.35, ART_UPRIGHT_BILLBOARD_EDGE_PROFILE, !is_ground_art);
        } else if (!is_ground_art) {
            // Regular/Background upright billboards (walls, bridges, docks). Keep the
            // Lambertian directional shading (shadow_profile/plane_catch_profile feed
            // shadow_cut + warm_key) but zero the vertical-edge terms that otherwise frame
            // the flat sprite with a spurious highlighted margin / boxed shadow — see the
            // ART_UPRIGHT_BILLBOARD_* constants above.
            shadow_profile = 1.04;
            plane_catch_profile = 1.0;
            edge_profile = ART_UPRIGHT_BILLBOARD_EDGE_PROFILE;
            side_falloff_profile = ART_UPRIGHT_BILLBOARD_SIDE_FALLOFF_PROFILE;
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
        out_rgb += rgb * local_light_rgba.rgb * local_light_rgba.a * light_static * local_light_profile
            * art_local_light_catch(uv_in_tile, flat_art);
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
        out_rgb += rgb * local_light_rgba.rgb * local_light_rgba.a * light_static
            * art_local_light_catch(uv_in_tile, flat_art);
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
