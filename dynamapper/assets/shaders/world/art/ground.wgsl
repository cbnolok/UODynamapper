#import bevy_pbr::{
    forward_io::Vertex,
    view_transformations,
    mesh_view_bindings::globals,
}
#import "shaders/world/effects/surface_effects.wgsl"::apply_water_animation
#import "shaders/postprocess/color_grading.wgsl"::{grade_color_vibrant}
#import "shaders/postprocess/global_lighting.wgsl"::{apply_global_lighting_rgb}
#import "shaders/postprocess/grunge.wgsl"::{apply_visual_grunge}
#import "shaders/postprocess/tonemapping.wgsl"::{tonemap_ec_kr_profile}

#import "shaders/world/art/art_ground_bindings.wgsl"::{
    GroundTileInstance, SpriteParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms,
    art_atlas_sampler, art_atlas, instances, sprite_params, scene, effects, global_light,
    hue_sampler, hue_texture, land_page_atlas, land_page_lookup,
    HEIGHT_SCALE, DEPTH_CLASS_BACKGROUND, DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF,
    PASS_MODE_OPAQUE, PASS_MODE_TRANSPARENT, DEPTH_CLASS_SURFACE_LIKE_FLOOR,
    SURFACE_LIKE_DEPTH_CLASS_OFFSET, STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON,
    GROUND_FLAG_EC_WATER_MATERIAL
}
#import "shaders/world/art/hue.wgsl"::apply_static_hue
#import "shaders/world/art/shading.wgsl"::{apply_art_surface_shading, apply_art_atmosphere_depth}

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

const LAND_PAGE_LOOKUP_TILE_CAPACITY: u32 = 16384u;
const LAND_PAGE_LOOKUP_ROLE_BASE: u32 = 0u;
const LAND_PAGE_LOOKUP_ROLE_NORMAL: u32 = 3u;

struct LandLookupSlot {
    present: bool,
    texture_layer: u32,
    texture_origin: vec2<u32>,
    texture_extent: vec2<u32>,
    texture_stretch: f32,
};

fn clip_depth_to_frag_depth(clip_position: vec4<f32>) -> f32 {
    return clamp(clip_position.z / clip_position.w, 0.0, 1.0);
}

fn ec_lookup_uv(payload: u32, role: u32) -> vec2<i32> {
    let lookup_dims = textureDimensions(land_page_lookup);
    let lookup_index = role * LAND_PAGE_LOOKUP_TILE_CAPACITY + payload;
    return vec2<i32>(
        i32(lookup_index % lookup_dims.x),
        i32(lookup_index / lookup_dims.x),
    );
}

fn read_ec_lookup_slot(payload: u32, role: u32) -> LandLookupSlot {
    let slot = textureLoad(land_page_lookup, ec_lookup_uv(payload, role), 0);
    let packed_wh = slot.w;
    let w = packed_wh & 0xFFFFu;
    let h = packed_wh >> 16u;
    let page_index = slot.x & 0xFFFFu;
    let stretch_q8 = slot.x >> 16u;
    return LandLookupSlot(
        w != 0u || h != 0u,
        page_index,
        vec2<u32>(slot.y, slot.z),
        vec2<u32>(w, h),
        f32(stretch_q8) / 256.0,
    );
}

fn ec_slot_world_uv(world_xz: vec2<f32>, slot: LandLookupSlot) -> vec2<f32> {
    let tile_w = max(f32(slot.texture_extent.x), 1.0);
    let stretch = select(tile_w / 44.0, slot.texture_stretch, slot.texture_stretch > 0.0);
    return fract(world_xz / stretch);
}

fn sample_ec_lookup_slot_rgba(uv: vec2<f32>, slot: LandLookupSlot) -> vec4<f32> {
    let layer: i32 = i32(slot.texture_layer);
    let tile_dims = max(vec2<f32>(slot.texture_extent), vec2<f32>(1.0));
    let use_linear = effects.enable_linear_filtering == 1u;

    if (use_linear) {
        let atlas_dims = vec2<f32>(textureDimensions(land_page_atlas));
        let local_px = clamp(uv * tile_dims, vec2<f32>(0.5), tile_dims - vec2<f32>(0.5));
        let atlas_uv = (vec2<f32>(slot.texture_origin) + local_px) / atlas_dims;
        return textureSample(land_page_atlas, art_atlas_sampler, atlas_uv, layer);
    }

    let local_iuv = clamp(vec2<i32>(uv * tile_dims), vec2<i32>(0), vec2<i32>(slot.texture_extent) - 1);
    let atlas_iuv = vec2<i32>(slot.texture_origin) + local_iuv;
    return textureLoad(land_page_atlas, atlas_iuv, layer, 0);
}

fn ec_liquid_perturbed_base_uv(world_xz: vec2<f32>, base_uv: vec2<f32>, payload: u32) -> vec2<f32> {
    let normal = read_ec_lookup_slot(payload, LAND_PAGE_LOOKUP_ROLE_NORMAL);
    if (!normal.present) {
        return base_uv;
    }

    let moving_world_xz = vec2<f32>(world_xz.x, world_xz.y - globals.time * 1.0);
    let normal_uv = ec_slot_world_uv(moving_world_xz, normal);
    let normal_sample = sample_ec_lookup_slot_rgba(normal_uv, normal);
    let perturbation = 0.30 * (normal_sample.rg - vec2<f32>(0.5)) * 2.0;
    return fract(base_uv + perturbation);
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
    out.depth = in.logical_depth;

    let layer = u32(in.uv_b.x);
    let inst = instances[in.instance_index];
    let atlas_extent = max(inst.uv_max - inst.uv_min, vec2<f32>(0.000001));
    var uv_in_tile = (in.uv - inst.uv_min) / atlas_extent;
    let use_ec_water_material = (inst.material_flags & GROUND_FLAG_EC_WATER_MATERIAL) != 0u;
    var color: vec4<f32>;
    if (use_ec_water_material) {
        let base_slot = read_ec_lookup_slot(inst.material_payload, LAND_PAGE_LOOKUP_ROLE_BASE);
        if (!base_slot.present) {
            discard;
        }
        uv_in_tile = ec_slot_world_uv(in.world_pos.xz, base_slot);
        if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
            uv_in_tile = ec_liquid_perturbed_base_uv(in.world_pos.xz, uv_in_tile, inst.material_payload);
        }
        color = sample_ec_lookup_slot_rgba(uv_in_tile, base_slot);
    } else {
        if (inst.texture_stretch > 0.0) {
            uv_in_tile = fract(in.world_pos.xz / inst.texture_stretch);
        }
        if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
            uv_in_tile = apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5));
        }
        let uv = inst.uv_min + uv_in_tile * atlas_extent;
        color = textureSample(art_atlas, art_atlas_sampler, uv, i32(layer));
    }
    if (sprite_params.pass_mode == PASS_MODE_OPAQUE) {
        if color.a < 0.0001 || color.a < sprite_params.alpha_cutoff {
            discard;
        }
    } else {
        if color.a >= sprite_params.alpha_cutoff {
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
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT) {
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
        shaded = vec4<f32>(apply_visual_grunge(shaded.rgb, in.world_pos.xz, effects.grunge_strength, effects.post_process_profile), shaded.a);
    }

    shaded = vec4<f32>(apply_global_lighting_rgb(shaded.rgb, scene.global_lighting), shaded.a);
    shaded = vec4<f32>(apply_art_atmosphere_depth(
        shaded.rgb,
        shaded.a,
        in.world_pos,
        scene.camera_position,
        global_light.atmosphere_tint,
        global_light.fog_color,
        global_light.fog_night_color,
        global_light.fog_params,
        global_light.enable_fog,
        effects.post_process_profile,
    ), shaded.a);

    var final_rgb = shaded.rgb;
    if (global_light.enable_grading == 1u) {
        final_rgb = grade_color_vibrant(final_rgb, global_light);
    }
    if (global_light.enable_tonemap == 1u) {
        final_rgb = tonemap_ec_kr_profile(max(final_rgb, vec3<f32>(0.0)), global_light.exposure, effects.post_process_profile);
    }

    out.color = vec4<f32>(max(final_rgb, vec3<f32>(0.0)), shaded.a);
    return out;
}
