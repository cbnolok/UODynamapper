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
    hue_sampler, hue_texture,
    HEIGHT_SCALE, DEPTH_CLASS_BACKGROUND, DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF,
    PASS_MODE_OPAQUE, PASS_MODE_TRANSPARENT, DEPTH_CLASS_SURFACE_LIKE_FLOOR,
    SURFACE_LIKE_DEPTH_CLASS_OFFSET, STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON
}
#import "shaders/world/art/hue.wgsl"::apply_static_hue
#import "shaders/world/art/shading.wgsl"::apply_art_surface_shading

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
    out.depth = in.logical_depth;

    let layer = u32(in.uv_b.x);
    let inst = instances[in.instance_index];
    let atlas_extent = max(inst.uv_max - inst.uv_min, vec2<f32>(0.000001));
    var uv_in_tile = (in.uv - inst.uv_min) / atlas_extent;
    if (inst.texture_stretch > 0.0) {
        uv_in_tile = fract(in.world_pos.xz / inst.texture_stretch);
    }
    if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
        uv_in_tile = apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5));
    }
    let uv = inst.uv_min + uv_in_tile * atlas_extent;
    let color = textureSample(art_atlas, art_atlas_sampler, uv, i32(layer));
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
    shaded = vec4<f32>(apply_art_surface_shading(
        shaded.rgb,
        uv_in_tile,
        in.world_pos,
        inst.depth_class,
        scene.light_direction,
        effects,
        global_light,
        inst.local_light_rgba,
        true,
    ), shaded.a);
    if (effects.enable_grunge == 1u) {
        shaded = vec4<f32>(apply_visual_grunge(shaded.rgb, in.world_pos.xz, effects.grunge_strength, effects.post_process_profile), shaded.a);
    }

    shaded = vec4<f32>(apply_global_lighting_rgb(shaded.rgb, scene.global_lighting), shaded.a);

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
