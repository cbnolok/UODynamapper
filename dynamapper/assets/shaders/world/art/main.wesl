import package::shaders::world::art::picking::{pick_clip, pick_pass, pick_color};
import bevy_pbr::render::{
    forward_io::Vertex,
    view_transformations,
    mesh_view_bindings::view,
    mesh_view_bindings::globals,
};
import package::shaders::world::effects::surface_effects::{apply_water_animation, water_flat_wrapped_lambert};
import package::shaders::postprocess::color_grading::{grade_color_vibrant};
import package::shaders::postprocess::global_lighting::{apply_global_lighting_rgb};
import package::shaders::postprocess::grunge::{apply_texture_visual_grunge, visual_grunge_uv};
import package::shaders::postprocess::kr_color_wash::apply_kr_color_wash;
import package::shaders::postprocess::tonemapping::{tonemap_with_curve};
import package::shaders::world::land::noise::noise_2d;
import package::shaders::world::pixel_art_filters::pixel_art_atlas_uv_iq;
import package::shaders::world::client_filter::{client_filtered_atlas_rect, TEXTURE_FILTER_CLIENT_BILINEAR};
import package::shaders::world::ec_item_quad::ec_item_quad_world;
import package::shaders::world::ec_item_surface::{apply_ec_item_surface, ec_item_surface_record};
import package::shaders::world::ec_liquid::{ec_liquid_texture_uv, ec_liquid_world_uv};

import package::shaders::world::art::art_bindings::{
    SpriteInstance, SpriteParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms,
    art_atlas_sampler, art_atlas, instances, sprite_params, scene, effects, global_light,
    hue_sampler, hue_texture, visual_grunge_texture, client_grunge_texture, client_grunge_sampler,
    item_surface_atlas, item_surface_lookup, item_surface_sampler,
    INV_SQRT_2, BILLBOARD_RIGHT_XZ, FLAT_QUAD_FORWARD_XZ, HEIGHT_SCALE, DEPTH_CLASS_REGULAR, DEPTH_CLASS_BACKGROUND,
    DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF, DEPTH_CLASS_SURFACE_LIKE_FLOOR, PASS_MODE_OPAQUE,
    PASS_MODE_TRANSPARENT, PASS_MODE_SHADOW, PASS_MODE_CLIENT_SHADOW, SURFACE_LIKE_DEPTH_CLASS_OFFSET
};
import package::shaders::world::art::client_shadow::{client_shadow_casts, client_shadow_world_offset,
    client_shadow_logical_offset, client_shadow_priority_depth};
import package::shaders::world::art::client_grunge::{client_grunges, client_grunge_uv, apply_client_grunge};
import package::shaders::world::art::hue::{apply_static_hue, apply_ec_sprite_hue, STATIC_HUE_FLAG_APPLY, STATIC_HUE_FLAG_EC_SPRITE};
import package::shaders::world::art::shading::apply_art_surface_shading;
import package::shaders::world::art::surface_blend::static_surface_coverage;
import package::shaders::world::ec_item_water::{ec_item_is_water, sample_ec_item_water};
import package::shaders::world::ec_liquid_camera::ec_liquid_camera_reflection_uv;
import package::shaders::world::art::depth::{static_priority_depth, horizontal_plane_frag_depth};
import package::shaders::world::projection::camera_relative_world_to_clip;

struct ArtVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) logical_depth: f32,
    @location(4) @interpolate(flat) depth_class: u32,
    @location(5) @interpolate(flat) is_wet: u32,
    @location(6) world_pos: vec3<f32>,
    @location(7) @interpolate(flat) instance_index: u32,
    @location(8) reflection_uv: vec2<f32>,
    // The KR client's grunge UV (client_grunge.wesl); used only while Client Grunge is on and the static is grunged.
    @location(9) client_grunge_uv: vec2<f32>,
}

struct ArtFragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

fn clip_depth_to_frag_depth(clip_position: vec4<f32>) -> f32 {
    return clamp(clip_position.z / clip_position.w, 0.0, 1.0);
}

fn depth_class_logical_offset(depth_class: u32) -> f32 {
    if (depth_class == DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
        return SURFACE_LIKE_DEPTH_CLASS_OFFSET;
    }

    if (depth_class == DEPTH_CLASS_BACKGROUND) {
        return -0.001;
    }

    if (depth_class == DEPTH_CLASS_ROOF) {
        return 0.002;
    }

    if (depth_class == DEPTH_CLASS_FOLIAGE) {
        return 2.0;
    }

    return 0.0;
}

fn logical_depth_from_projected_priority(inst: SpriteInstance) -> f32 {
    let logical_world_pos = vec3<f32>(
        inst.tile_x,
        inst.priority_z_units * HEIGHT_SCALE + depth_class_logical_offset(inst.depth_class),
        inst.tile_y,
    );
    return static_priority_depth(view_transformations::position_world_to_clip(logical_world_pos),
        view.clip_from_view, inst.sort_bias_ordinal);
}

// The client shadow's depth: its caster's priority point, moved with a wall's shift and half a tie-break step behind
// (client_shadow.wesl).
fn client_shadow_logical_depth(inst: SpriteInstance) -> f32 {
    let logical_world_pos = vec3<f32>(
        inst.tile_x,
        inst.priority_z_units * HEIGHT_SCALE + depth_class_logical_offset(inst.depth_class),
        inst.tile_y,
    ) + client_shadow_logical_offset(inst);
    return client_shadow_priority_depth(view_transformations::position_world_to_clip(logical_world_pos),
        view.clip_from_view, inst.sort_bias_ordinal);
}

// Every corner of a non-caster lands on this one clip point: zero-area triangles draw nothing.
const CLIENT_SHADOW_CULLED_POSITION: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 1.0);

@vertex
fn vertex(vertex: Vertex) -> ArtVertexOutput {
    let inst = instances[u32(vertex.uv_b.x + 0.5)];

    let corner = vec2<f32>(vertex.position.x, vertex.position.z);
    var world_pos: vec3<f32>;
    var local_world_pos: vec3<f32>;
    if (inst.flat_quad >= 2u) {
        // Tileart-owned EC material images share the sprite atlas, but cover
        // an axis-aligned world cell just like terrain-owned ground materials.
        let local = mix(inst.local_min, inst.local_max, corner);
        local_world_pos = vec3<f32>(local.x, 0.0, local.y);
        world_pos = vec3<f32>(inst.world_x + local.x, inst.world_y, inst.world_z + local.y);
    } else if (inst.flat_quad == 1u) {
        // CC surface-like statics (shoreline water) lie flat in the XZ plane instead of
        // standing up as a camera-facing card. Unlike the EC ground-quad path (plain square
        // texmap on an axis-aligned cell), the CC sprite already has the isometric diamond
        // baked into its pixels, so we lay it out on the diagonal screen-aligned basis
        // (BILLBOARD_RIGHT_XZ = on-screen horizontal, FLAT_QUAD_FORWARD_XZ = on-screen
        // vertical/depth). With the centered, pixel-sized bounds from
        // resolve_cc_surface_like_flat_quad_bounds, the sprite's diamond vertices land on the
        // cell corners so the water seams with the terrain beneath it.
        let local = mix(inst.local_min, inst.local_max, corner);
        local_world_pos = vec3<f32>(local.x * BILLBOARD_RIGHT_XZ.x + local.y * FLAT_QUAD_FORWARD_XZ.x, 0.0,
            local.x * BILLBOARD_RIGHT_XZ.y + local.y * FLAT_QUAD_FORWARD_XZ.y);
        world_pos = vec3<f32>(
            inst.world_x + local.x * BILLBOARD_RIGHT_XZ.x + local.y * FLAT_QUAD_FORWARD_XZ.x,
            inst.world_y,
            inst.world_z + local.x * BILLBOARD_RIGHT_XZ.y + local.y * FLAT_QUAD_FORWARD_XZ.y,
        );
    } else {
        let local = mix(inst.local_min, inst.local_max, vec2<f32>(corner.x, 1.0 - corner.y));
        local_world_pos = vec3<f32>(local.x * BILLBOARD_RIGHT_XZ.x, local.y, local.x * BILLBOARD_RIGHT_XZ.y);
        world_pos = vec3<f32>(
            inst.world_x + local.x * BILLBOARD_RIGHT_XZ.x,
            inst.world_y + local.y,
            inst.world_z + local.x * BILLBOARD_RIGHT_XZ.y,
        );
    }
    var projected_world_pos = world_pos;
    var projected_local_pos = local_world_pos;
    if (inst.flat_quad == 3u && view.clip_from_view[3][3] == 1.0
        && dot(view.world_from_view[2].xyz, vec3<f32>(0.577350269)) >= 0.999999) {
        let pixel_aspect = (view.clip_from_view[0][0] * view.viewport.z)
            / (view.clip_from_view[1][1] * view.viewport.w);
        projected_local_pos = ec_item_quad_world(vec3<f32>(0.0), corner,
            view.world_from_view[0].xyz, view.world_from_view[1].xyz, pixel_aspect);
        projected_world_pos = ec_item_quad_world(vec3<f32>(inst.world_x, inst.world_y, inst.world_z),
            corner, view.world_from_view[0].xyz, view.world_from_view[1].xyz, pixel_aspect);
    }
    if (sprite_params.pass_mode == PASS_MODE_SHADOW) {
        let caster_height = max(world_pos.y - inst.base_world_y, 0.0);
        let light_xz_len = max(length(scene.light_direction.xz), 0.001);
        let shadow_dir = -scene.light_direction.xz / light_xz_len;
        var profile_length = 1.0;
        if (inst.depth_class == DEPTH_CLASS_FOLIAGE) {
            profile_length = 1.18;
        } else if (inst.depth_class == DEPTH_CLASS_ROOF) {
            profile_length = 0.68;
        }
        let shadow_offset = shadow_dir * caster_height * effects.art_projected_shadow_length * profile_length;
        projected_world_pos = vec3<f32>(
            world_pos.x + shadow_offset.x,
            inst.base_world_y + 0.012,
            world_pos.z + shadow_offset.y,
        );
        projected_local_pos = vec3<f32>(local_world_pos.x + shadow_offset.x,
            projected_world_pos.y - inst.world_y, local_world_pos.z + shadow_offset.y);
    }
    var out: ArtVertexOutput;
    out.client_grunge_uv = vec2<f32>(0.0);
    if (effects.enable_client_grunge == 1u && client_grunges(inst)) {
        out.client_grunge_uv = client_grunge_uv(inst, projected_local_pos);
    }
    if (sprite_params.pass_mode == PASS_MODE_CLIENT_SHADOW) {
        projected_local_pos = client_shadow_world_offset(inst, corner);
    }
    out.position = camera_relative_world_to_clip(vec3<f32>(inst.world_x, inst.world_y, inst.world_z), projected_local_pos);
    if (sprite_params.pass_mode == PASS_MODE_CLIENT_SHADOW && !client_shadow_casts(inst)) {
        out.position = CLIENT_SHADOW_CULLED_POSITION;
    }
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    out.uv_b = vec2<f32>(f32(inst.layer), 0.0);
    out.color = inst.color_rgba;
    if (sprite_params.pass_mode == PASS_MODE_SHADOW) {
        out.logical_depth = clip_depth_to_frag_depth(out.position);
    } else if (sprite_params.pass_mode == PASS_MODE_CLIENT_SHADOW) {
        out.logical_depth = client_shadow_logical_depth(inst);
    } else {
        out.logical_depth = logical_depth_from_projected_priority(inst);
    }
    out.depth_class = inst.depth_class;
    out.is_wet = inst.is_wet_flags & 1u;
    out.world_pos = world_pos;
    out.instance_index = u32(vertex.uv_b.x + 0.5);
    out.reflection_uv = vec2<f32>(0.0);
    if (inst.flat_quad >= 2u && (inst.is_wet_flags & 1u) != 0u) {
        // Fixed-function reflection is generated at vertices. Ordinary sprites
        // and Classic flat art do not need this additional coordinate work.
        out.reflection_uv = ec_liquid_camera_reflection_uv(projected_world_pos, scene.ec_reflection_camera_distance);
    }
    @if(STATIC_PICK)
    out.position = pick_clip(out.position);
    return out;
}

fn shade_fragment(in: ArtVertexOutput, sample_index: u32) -> ArtFragmentOutput {
    var out: ArtFragmentOutput;
    out.depth = in.logical_depth;

    if sprite_params.render_mode == 1u {
        // Dot mode (radarcol)
        out.color = in.color;
        return out;
    }

    let layer = u32(in.uv_b.x);
    let inst = instances[in.instance_index];
    var uv = in.uv;
    let atlas_extent = max(inst.uv_max - inst.uv_min, vec2<f32>(0.000001));
    let uv_in_tile_for_shading = clamp((uv - inst.uv_min) / atlas_extent, vec2<f32>(0.0), vec2<f32>(1.0));
    let surface_footprint = max(fwidth(uv_in_tile_for_shading), vec2<f32>(0.00001));
    // The client filter's footprint: the sprite's atlas uv per screen pixel, taken before any branch.
    let sprite_uv_dx = dpdx(in.uv);
    let sprite_uv_dy = dpdy(in.uv);

    if (sprite_params.pass_mode == PASS_MODE_CLIENT_SHADOW) {
        // UOSpriteShaderShadow: black, with the caster texel's alpha. The pipeline writes only the target's alpha,
        // S = S (1 - a), and the client night composite darkens by clamp(S + 1 - s). The client filters the shadow
        // bilinear whatever the caster's filter (client_shadow.wesl), so its leaning edges stay smooth.
        let shadow_alpha = client_filtered_atlas_rect(art_atlas, i32(layer), uv, inst.uv_min, inst.uv_max, false,
            sprite_uv_dx, sprite_uv_dy, scene.client_view_scale).a;
        if (shadow_alpha <= 0.0) {
            discard;
        }
        out.color = vec4<f32>(0.0, 0.0, 0.0, shadow_alpha);
        return out;
    }

    if (sprite_params.pass_mode == PASS_MODE_SHADOW) {
        if (inst.flat_quad >= 2u) {
            discard;
        }
        if (effects.enable_art_projected_shadows == 0u || effects.art_projected_shadow_strength <= 0.0001 || sprite_params.render_mode == 1u) {
            discard;
        }
        var depth_profile = 0.0;
        if (in.depth_class == DEPTH_CLASS_FOLIAGE) {
            depth_profile = 1.0;
        } else if (in.depth_class == DEPTH_CLASS_ROOF) {
            depth_profile = 0.35;
        } else if (in.depth_class != DEPTH_CLASS_BACKGROUND && in.depth_class != DEPTH_CLASS_SURFACE_LIKE_FLOOR) {
            depth_profile = 0.85;
        }
        // Onset tuned for classic sprite world-heights (a 44px tile is only ~1
        // world unit tall); the old 0.45..3.2 window left short/typical statics
        // casting a near-zero shadow, so the strength knob looked inert.
        let caster_height = max(inst.local_max.y - inst.local_min.y, 0.0);
        let height_profile = smoothstep(0.2, 1.5, caster_height);
        var shadow_center = vec2<f32>(0.5, 0.56);
        var shadow_radius = vec2<f32>(0.54, 0.42);
        if (in.depth_class == DEPTH_CLASS_FOLIAGE) {
            shadow_center = vec2<f32>(0.5, 0.58);
            shadow_radius = vec2<f32>(0.62, 0.48);
        } else if (in.depth_class == DEPTH_CLASS_ROOF) {
            shadow_center = vec2<f32>(0.5, 0.52);
            shadow_radius = vec2<f32>(0.48, 0.34);
        }
        let centered = (uv_in_tile_for_shading - shadow_center) / shadow_radius;
        let ellipse = dot(centered, centered);
        let softness = clamp(effects.art_projected_shadow_softness, 0.05, 1.0);
        let mask = 1.0 - smoothstep(1.0 - softness * 0.48, 1.0, ellipse);
        let alpha_blur_radius = atlas_extent * (0.008 + 0.018 * softness);
        let alpha_center = clamp(uv, inst.uv_min, inst.uv_max);
        let alpha_x0 = clamp(uv + vec2<f32>(alpha_blur_radius.x, 0.0), inst.uv_min, inst.uv_max);
        let alpha_x1 = clamp(uv - vec2<f32>(alpha_blur_radius.x, 0.0), inst.uv_min, inst.uv_max);
        let alpha_y0 = clamp(uv + vec2<f32>(0.0, alpha_blur_radius.y), inst.uv_min, inst.uv_max);
        let alpha_y1 = clamp(uv - vec2<f32>(0.0, alpha_blur_radius.y), inst.uv_min, inst.uv_max);
        let alpha_d0 = clamp(uv + alpha_blur_radius, inst.uv_min, inst.uv_max);
        let alpha_d1 = clamp(uv - alpha_blur_radius, inst.uv_min, inst.uv_max);
        let alpha_d2 = clamp(uv + vec2<f32>(alpha_blur_radius.x, -alpha_blur_radius.y), inst.uv_min, inst.uv_max);
        let alpha_d3 = clamp(uv + vec2<f32>(-alpha_blur_radius.x, alpha_blur_radius.y), inst.uv_min, inst.uv_max);
        let blurred_alpha =
            textureSample(art_atlas, art_atlas_sampler, alpha_center, i32(layer)).a * 0.28 +
            (
                textureSample(art_atlas, art_atlas_sampler, alpha_x0, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_x1, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_y0, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_y1, i32(layer)).a
            ) * 0.11 +
            (
                textureSample(art_atlas, art_atlas_sampler, alpha_d0, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_d1, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_d2, i32(layer)).a +
                textureSample(art_atlas, art_atlas_sampler, alpha_d3, i32(layer)).a
            ) * 0.08;
        let silhouette = smoothstep(0.03, 0.44, blurred_alpha);
        var family_breakup = 1.0;
        if (in.depth_class == DEPTH_CLASS_FOLIAGE) {
            let leaf_noise = noise_2d(in.world_pos.xz * 1.35 + uv_in_tile_for_shading * 15.0);
            family_breakup = mix(0.58, 1.10, smoothstep(0.22, 0.92, leaf_noise));
        } else if (in.depth_class == DEPTH_CLASS_ROOF) {
            family_breakup = 0.82;
        }
        let alpha = mask * silhouette * height_profile * depth_profile * family_breakup * clamp(effects.art_projected_shadow_strength, 0.0, 1.0) * in.color.a;
        if (alpha <= 0.001) {
            discard;
        }
        out.color = vec4<f32>(0.025, 0.030, 0.036, alpha);
        return out;
    }

    // Flat CC surface-like water quads lie coplanar with the terrain mesh they overlay.
    // Every other sprite writes a per-instance CONSTANT logical depth
    // (logical_depth_from_projected_priority projects a single point at the cell origin,
    // inst.tile_x/inst.tile_y). That is correct for upright billboards — thin cards that must
    // sort in classic painter order as a unit — but wrong for a flat quad that spans a whole
    // tile in the ground plane: a single constant cannot track the terrain's per-fragment depth
    // gradient across the tile (the terrain's depth varies by ~2 world-units of x+z over one
    // cell, ~5.8e-4 in NDC, vs. the Background class' 0.001 world-Y logical offset, ~3e-7 NDC —
    // a ~2000x mismatch). A flat world-space surface therefore needs its own rasterized geometric
    // depth instead of an upright sprite's synthetic constant priority depth. The quad geometry
    // is already placed STATIC_ART_Y_BIAS (world-Y) above the terrain, so the two parallel planes
    // retain a small uniform separation without a depth-gradient mismatch. This also remains
    // valid for a future perspective camera, unlike ClassicUO's per-sprite synthetic depth.
    // Authored EC camera quads also use their rasterized plane depth, which is
    // constant across their camera-facing geometry rather than a horizontal cell.
    //
    // This policy prevents coplanar/intersection artifacts without changing authored alpha or
    // color coverage. Only reached in the opaque/transparent passes; billboards and the shadow
    // pass are unaffected.
    if (inst.flat_quad == 3u) {
        out.depth = in.position.z;
    } else if (inst.flat_quad != 0u) {
        out.depth = horizontal_plane_frag_depth(in.position, inst.world_y);
        // The pick target crops XY, so its pixel is not a screen position.
        @if(STATIC_PICK)
        out.depth = in.position.z;
    }

    var surface_header = vec4<u32>(0u);
    // A repeated surface wraps the client filter's taps, as the client's wrap address mode does.
    var surface_repeats = false;
    if (inst.flat_quad >= 2u) {
        surface_header = ec_item_surface_record(item_surface_lookup, inst.material_item_id);
        if (surface_header.x != 0u) {
            let repetition = bitcast<f32>(surface_header.w);
            let coordinates = select(vec2<f32>(uv_in_tile_for_shading.x, 1.0 - uv_in_tile_for_shading.y),
                ec_liquid_world_uv(in.world_pos.xz), surface_header.z == 1u);
            let transformed = ec_liquid_texture_uv(coordinates, repetition, vec2<f32>(0.0));
            uv = inst.uv_min + select(clamp(transformed, vec2<f32>(0.0), vec2<f32>(1.0)),
                fract(transformed), repetition != 1.0) * atlas_extent;
            surface_repeats = repetition != 1.0;
        } else if (inst.texture_stretch > 0.0) {
            uv = inst.uv_min + fract(in.world_pos.xz / inst.texture_stretch) * atlas_extent;
            surface_repeats = true;
        }
    }
    let use_ec_item_water = ec_item_is_water(surface_header);
    if (effects.enable_water_animation == 1u && in.is_wet == 1u && !use_ec_item_water) {
        let uv_in_tile = (uv - inst.uv_min) / atlas_extent;
        uv = inst.uv_min + apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5)) * atlas_extent;
    }
    let atlas_size = vec2<f32>(textureDimensions(art_atlas).xy);
    let sample_uv = pixel_art_atlas_uv_iq(uv, inst.uv_min, inst.uv_max, atlas_size);
    var color: vec4<f32>;
    if (use_ec_item_water) {
        let energy = global_light.ambient_strength
            + global_light.land_diffuse_strength * water_flat_wrapped_lambert(scene.light_direction);
        color = sample_ec_item_water(item_surface_atlas, item_surface_lookup,
            item_surface_sampler, surface_header, uv_in_tile_for_shading, in.world_pos.xz,
            in.reflection_uv, globals.time, effects.enable_water_animation == 1u,
            effects.enable_linear_filtering == 1u, energy);
    } else {
        if (effects.texture_filter == TEXTURE_FILTER_CLIENT_BILINEAR) {
            color = client_filtered_atlas_rect(art_atlas, i32(layer), uv, inst.uv_min, inst.uv_max,
                surface_repeats, sprite_uv_dx, sprite_uv_dy, scene.client_view_scale);
        } else {
            color = textureSample(art_atlas, art_atlas_sampler, sample_uv, i32(layer));
        }
        // Item surfaces (flat_quad 2 and 3) and their boundary coverage below come only from the Enhanced art
        // package, so they follow the art source, never the preset: every preset draws the authored coverage masks
        // filtered and fades the surface into the map, as the Enhanced client does.
        if (inst.flat_quad >= 2u) {
            color = apply_ec_item_surface(color, item_surface_atlas, item_surface_lookup,
                item_surface_sampler, surface_header, uv_in_tile_for_shading, in.world_pos.xz,
                effects.enable_linear_filtering == 1u, true);
        }
    }
    if (surface_header.x != 0u && !use_ec_item_water) {
        // The authored overlap margin allows a 1.5-pixel coverage fade at the
        // quad boundary without drawing a hard cutout over the liquid below.
        let edge = min(uv_in_tile_for_shading, vec2<f32>(1.0) - uv_in_tile_for_shading);
        color.a *= smoothstep(0.0, 1.5, min(edge.x / surface_footprint.x, edge.y / surface_footprint.y));
    }
    let blend_surface = (inst.is_wet_flags & 2u) != 0u && (effects.terrain_transition_flags & 2u) == 0u;
    if (blend_surface) {
        color.a *= static_surface_coverage(uv_in_tile_for_shading, (inst.is_wet_flags >> 2u) & 255u);
    }
    // Preserve fractional coverage and explicit material opacity in the blended
    // pass. Testing image alpha alone would make translucent legacy art opaque.
    let visible_alpha = color.a * in.color.a;
    let alpha_cutoff = select(sprite_params.alpha_cutoff, 0.9999,
        surface_header.x != 0u || blend_surface || in.color.a < 0.9999);
    var pass_mode = sprite_params.pass_mode;
    @if(STATIC_PICK)
    pass_mode = pick_pass();
    if (pass_mode == PASS_MODE_OPAQUE) {
        if visible_alpha < 0.0001 || visible_alpha < alpha_cutoff {
            discard;
        }
    } else {
        if visible_alpha < 0.0001 || visible_alpha >= alpha_cutoff {
            discard;
        }
    }

    @if(STATIC_PICK) {
        out.color = pick_color(in.instance_index, false, out.depth, sample_index);
        return out;
    }

    var shaded = color * in.color;
    var sprite_hue = vec4<u32>(0u);
    let ec_hue_flags = STATIC_HUE_FLAG_APPLY | STATIC_HUE_FLAG_EC_SPRITE;
    if (sprite_params.hue_enabled != 0u && inst.hue_id != 0u
        && (inst.hue_flags & ec_hue_flags) == ec_hue_flags) {
        sprite_hue = ec_item_surface_record(item_surface_lookup, inst.material_item_id);
    }
    if ((sprite_hue.y & 256u) != 0u) {
        shaded = apply_ec_sprite_hue(shaded, inst.hue_id, sprite_hue,
            (sample_uv - inst.uv_min) / atlas_extent, hue_texture, hue_sampler,
            item_surface_atlas, item_surface_lookup, effects.enable_linear_filtering == 1u);
    } else {
        shaded = apply_static_hue(
            shaded,
            inst.hue_id,
            inst.hue_flags,
            sprite_params.hue_enabled,
            hue_texture,
            hue_sampler,
        );
    }
    // The KR client grunges a static's hued colour (client_grunge.wesl).
    if (effects.enable_client_grunge == 1u && client_grunges(inst)) {
        shaded = vec4<f32>(apply_client_grunge(shaded.rgb, in.client_grunge_uv, effects.client_grunge_strength,
            client_grunge_texture, client_grunge_sampler), shaded.a);
    }
    // The existing fractional-edge color adjustment is not a material opacity
    // rule. Explicitly translucent art keeps straight RGB for standard blending.
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT && surface_header.x == 0u && !blend_surface
        && in.color.a >= 0.9999) {
        shaded = vec4<f32>(shaded.rgb * shaded.a * 2.0, shaded.a);
    }
    // CC surface-like water statics lie flat and must read as a continuous water
    // surface that seams with the terrain water beneath them. apply_art_surface_shading
    // models an upright object's self-occlusion (lower_occlusion / side_contact darkening,
    // top highlight) which is applied unconditionally regardless of the is_ground_art flag;
    // painted onto a flat water diamond it produces a bright-top/dark-edge shaded outline —
    // the "boxed shadow" artifact. Skip it for flat water, mirroring the EC ground path which
    // likewise bypasses this shading for its water material (ground.wesl `if (!use_ec_water_material)`).
    // Non-water flat statics and all billboards are unaffected.
    let is_cc_flat_water = inst.flat_quad == 1u && in.is_wet == 1u;
    if (is_cc_flat_water) {
        // Flat CC water is a level liquid surface that must read as the SAME even tone as
        // the terrain water it seams with (the terrain water shows through this quad's
        // transparent diamond corners, so any brightness mismatch appears as dark ticks
        // along the shoreline). Reproduce the terrain water's Classic response exactly:
        // albedo * (ambient + diffuse * wrapped_lambert), using the identical shared
        // wrapped-Lambert term and the same ambient / diffuse (diffuse arrives via the
        // shared uniform's land_diffuse_strength slot — see art_bindings / statics_draw).
        // No rim/spec/fill: a flat surface has no meaningful view/normal variation.
        let flat_water_lam = water_flat_wrapped_lambert(scene.light_direction);
        let flat_water_energy = global_light.ambient_strength + global_light.land_diffuse_strength * flat_water_lam;
        shaded = vec4<f32>(shaded.rgb * flat_water_energy, shaded.a);
    } else if (!use_ec_item_water) {
        shaded = vec4<f32>(apply_art_surface_shading(
            shaded.rgb,
            uv_in_tile_for_shading,
            in.world_pos,
            in.depth_class,
            scene.light_direction,
            global_light.light_color,
            global_light.atmosphere_tint,
            effects.enable_art_fake_normals,
            effects.art_shadow_strength,
            effects.art_highlight_strength,
            effects.art_depth_tint_strength,
            effects.art_contact_shadow_strength,
            effects.art_mottle_strength,
            effects.light_decal_intensity,
            effects.kr_art_temperature_strength,
            inst.local_light_rgba,
            inst.flat_quad != 0u,
        ), shaded.a);
    }
    if (effects.enable_grunge == 1u) {
        let visual_grunge = textureSample(visual_grunge_texture, art_atlas_sampler, visual_grunge_uv(in.world_pos.xz, effects.post_process_profile)).rgb;
        shaded = vec4<f32>(apply_texture_visual_grunge(shaded.rgb, in.world_pos.xz, effects.grunge_strength, effects.post_process_profile, visual_grunge), shaded.a);
    }

    let object_color_wash_shadow = clamp(
        smoothstep(0.42, 1.0, uv_in_tile_for_shading.y) * 0.40
        + select(0.0, 0.14, in.depth_class == DEPTH_CLASS_FOLIAGE)
        + select(0.0, 0.10, in.depth_class == DEPTH_CLASS_ROOF),
        0.0,
        1.0,
    );
    // A flat liquid has no per-sprite vertical shadow gradient or local-light
    // ownership. Feeding either into the KR wash paints the authored diamond
    // outline into the water and diverges from the terrain-owned surface.
    let color_wash_shadow = select(object_color_wash_shadow, 0.0, is_cc_flat_water || use_ec_item_water);
    let color_wash_local_light = select(inst.local_light_rgba.a, 0.0, is_cc_flat_water || use_ec_item_water);
    shaded = vec4<f32>(apply_kr_color_wash(
        shaded.rgb,
        in.world_pos.xz,
        color_wash_shadow,
        color_wash_local_light,
        effects.enable_kr_color_wash,
        effects.kr_color_wash_strength,
        effects.kr_color_wash_scale,
        effects.post_process_profile,
    ), shaded.a);

    shaded = vec4<f32>(apply_global_lighting_rgb(shaded.rgb, scene.global_lighting), shaded.a);
    // Fog is applied once to the fully composed frame by effects/fog_pass.wesl.

    var final_rgb = shaded.rgb;
    if (global_light.enable_grading == 1u) {
        final_rgb = grade_color_vibrant(final_rgb, global_light);
    }
    if (global_light.enable_tonemap == 1u) {
        final_rgb = tonemap_with_curve(max(final_rgb, vec3<f32>(0.0)), global_light.exposure, effects.post_process_profile, global_light.tone_curve);
    }

    out.color = vec4<f32>(max(final_rgb, vec3<f32>(0.0)), shaded.a);
    return out;
}

@fragment
fn fragment(in: ArtVertexOutput) -> ArtFragmentOutput { return shade_fragment(in, 0u); }

@if(STATIC_PICK)
@fragment
fn pick_fragment(in: ArtVertexOutput, @builtin(sample_index) sample_index: u32) -> ArtFragmentOutput {
    return shade_fragment(in, sample_index);
}
