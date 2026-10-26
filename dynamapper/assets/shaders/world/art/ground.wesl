import bevy_pbr::render::{
    forward_io::Vertex,
    view_transformations,
    mesh_view_bindings::globals,
};
import package::shaders::world::effects::surface_effects::{apply_water_animation, water_flat_wrapped_lambert};
import package::shaders::world::ec_liquid::{sample_ec_liquid, ec_liquid_reflection_uv};
import package::shaders::world::ec_item_liquid::apply_ec_item_liquid;
import package::shaders::postprocess::color_grading::{grade_color_vibrant};
import package::shaders::postprocess::global_lighting::{apply_global_lighting_rgb};
import package::shaders::postprocess::grunge::{apply_texture_visual_grunge, visual_grunge_uv};
import package::shaders::postprocess::kr_color_wash::apply_kr_color_wash;
import package::shaders::postprocess::tonemapping::{tonemap_ec_kr_profile};
import package::shaders::world::pixel_art_filters::{pixel_art_atlas_uv_iq};

import package::shaders::world::art::art_ground_bindings::{
    GroundTileInstance, SpriteParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms,
    art_atlas_sampler, art_atlas, instances, sprite_params, scene, effects, global_light,
    hue_sampler, hue_texture, land_page_atlas, land_page_lookup, visual_grunge_texture,
    item_liquid_atlas, item_liquid_lookup,
    HEIGHT_SCALE, DEPTH_CLASS_BACKGROUND, DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF,
    PASS_MODE_OPAQUE, PASS_MODE_TRANSPARENT, DEPTH_CLASS_SURFACE_LIKE_FLOOR,
    SURFACE_LIKE_DEPTH_CLASS_OFFSET, STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON,
    GROUND_FLAG_EC_WATER_MATERIAL
};
import package::shaders::world::art::hue::apply_static_hue;
import package::shaders::world::art::shading::apply_art_surface_shading;

struct GroundVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) logical_depth: f32,
    @location(4) @interpolate(flat) is_wet: u32,
    @location(5) world_pos: vec3<f32>,
    @location(6) @interpolate(flat) instance_index: u32,
}

struct GroundFragmentOutput {
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

fn logical_depth_from_projected_priority(inst: GroundTileInstance) -> f32 {
    let logical_world_pos = vec3<f32>(
        inst.tile_x,
        inst.priority_z_units * HEIGHT_SCALE + depth_class_logical_offset(inst.depth_class),
        inst.tile_y,
    );
    return clip_depth_to_frag_depth(view_transformations::position_world_to_clip(logical_world_pos));
}

fn apply_sort_bias_to_frag_depth(depth: f32, sort_bias_ordinal: u32) -> f32 {
    return clamp(depth - f32(sort_bias_ordinal) * STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON, 0.0, 1.0);
}

@vertex
fn vertex(vertex: Vertex) -> GroundVertexOutput {
    let inst = instances[u32(vertex.uv_b.x + 0.5)];

    let corner = vec2<f32>(vertex.position.x, vertex.position.z);
    let local = mix(inst.local_min, inst.local_max, corner);
    let world_pos = vec3<f32>(
        inst.world_x + local.x,
        inst.world_y,
        inst.world_z + local.y,
    );
    var out: GroundVertexOutput;
    out.position = view_transformations::position_world_to_clip(world_pos);
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    out.uv_b = vec2<f32>(f32(inst.layer), 0.0);
    out.color = inst.color_rgba;
    out.logical_depth = apply_sort_bias_to_frag_depth(
        logical_depth_from_projected_priority(inst),
        inst.sort_bias_ordinal,
    );
    out.is_wet = inst.is_wet_flags & 1u;
    out.world_pos = world_pos;
    out.instance_index = u32(vertex.uv_b.x + 0.5);
    return out;
}

@fragment
fn fragment(in: GroundVertexOutput) -> GroundFragmentOutput {
    var out: GroundFragmentOutput;
    // This is a horizontal world quad, so depth must vary across its surface.
    // A sprite's constant priority depth intersects the terrain plane and leaves
    // triangular gaps along otherwise coplanar EC water tiles.
    out.depth = in.position.z;

    let layer = u32(in.uv_b.x);
    let inst = instances[in.instance_index];
    let atlas_extent = max(inst.uv_max - inst.uv_min, vec2<f32>(0.000001));
    var uv_in_tile = (in.uv - inst.uv_min) / atlas_extent;
    let use_ec_water_material = (inst.material_flags & GROUND_FLAG_EC_WATER_MATERIAL) != 0u;
    var color: vec4<f32>;
    if (use_ec_water_material) {
        let reflection_uv = ec_liquid_reflection_uv(
            view_transformations::position_world_to_view(in.world_pos),
            view_transformations::direction_world_to_view(vec3<f32>(0.0, 1.0, 0.0)),
        );
        color = sample_ec_liquid(land_page_atlas, land_page_lookup, art_atlas_sampler,
            inst.material_payload, in.world_pos.xz, reflection_uv, globals.time,
            effects.enable_water_animation == 1u, effects.enable_linear_filtering == 1u);
        let energy = global_light.ambient_strength
            + global_light.land_diffuse_strength * water_flat_wrapped_lambert(scene.light_direction);
        color = apply_ec_item_liquid(vec4<f32>(color.rgb * energy, color.a),
            item_liquid_atlas, item_liquid_lookup, art_atlas_sampler, inst.material_item_id,
            uv_in_tile, in.world_pos.xz, globals.time,
            effects.enable_water_animation == 1u, effects.enable_linear_filtering == 1u);
    } else {
        if (inst.texture_stretch > 0.0) {
            uv_in_tile = fract(in.world_pos.xz / inst.texture_stretch);
        }
        if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
            uv_in_tile = apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5));
        }
        let uv = inst.uv_min + uv_in_tile * atlas_extent;
        let atlas_size = vec2<f32>(textureDimensions(art_atlas).xy);
        let sample_uv = pixel_art_atlas_uv_iq(uv, inst.uv_min, inst.uv_max, atlas_size);
        color = textureSample(art_atlas, art_atlas_sampler, sample_uv, i32(layer));
    }
    // Blend authored partial-alpha coverage instead of making half-opaque masks
    // solid. Other art keeps its existing sprite split.
    let alpha_cutoff = select(sprite_params.alpha_cutoff, 0.9999, use_ec_water_material);
    if (sprite_params.pass_mode == PASS_MODE_OPAQUE) {
        if color.a < 0.0001 || color.a < alpha_cutoff {
            discard;
        }
    } else {
        if color.a < 0.0001 || color.a >= alpha_cutoff {
            discard;
        }
    }

    var shaded = color * in.color;
    shaded = apply_static_hue(
        shaded,
        inst.hue_id,
        inst.hue_flags,
        sprite_params.hue_enabled,
        hue_texture,
        hue_sampler,
    );
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT && !use_ec_water_material) {
        shaded = vec4<f32>(shaded.rgb * shaded.a * 2.0, shaded.a);
    }
    if (!use_ec_water_material) {
        shaded = vec4<f32>(apply_art_surface_shading(
            shaded.rgb,
            uv_in_tile,
            in.world_pos,
            inst.depth_class,
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
            true,
        ), shaded.a);
    }
    if (effects.enable_grunge == 1u) {
        let visual_grunge = textureSample(visual_grunge_texture, art_atlas_sampler, visual_grunge_uv(in.world_pos.xz, effects.post_process_profile)).rgb;
        shaded = vec4<f32>(apply_texture_visual_grunge(shaded.rgb, in.world_pos.xz, effects.grunge_strength, effects.post_process_profile, visual_grunge), shaded.a);
    }
    let color_wash_shadow = select(clamp(
        smoothstep(0.48, 1.0, uv_in_tile.y) * 0.28
        + select(0.0, 0.10, inst.depth_class == DEPTH_CLASS_FOLIAGE)
        + select(0.0, 0.08, inst.depth_class == DEPTH_CLASS_ROOF),
        0.0,
        1.0,
    ), 0.0, use_ec_water_material);
    shaded = vec4<f32>(apply_kr_color_wash(
        shaded.rgb,
        in.world_pos.xz,
        color_wash_shadow,
        select(inst.local_light_rgba.a, 0.0, use_ec_water_material),
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
        final_rgb = tonemap_ec_kr_profile(max(final_rgb, vec3<f32>(0.0)), global_light.exposure, effects.post_process_profile, global_light.kr_tonemap_params.x);
    }

    out.color = vec4<f32>(max(final_rgb, vec3<f32>(0.0)), shaded.a);
    return out;
}
