#import bevy_pbr::{
    forward_io::Vertex,
    view_transformations,
    mesh_view_bindings::globals,
}
#import "shaders/worldmap/surface_effects.wgsl"::apply_water_animation

#import "shaders/worldmap/art/art_ground_bindings.wgsl"::{
    GroundTileInstance, SpriteParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms,
    art_atlas_sampler, art_atlas, instances, sprite_params, scene, effects, global_light,
    HEIGHT_SCALE, DEPTH_CLASS_BACKGROUND, DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF,
    PASS_MODE_OPAQUE, PASS_MODE_TRANSPARENT, DEPTH_CLASS_SURFACE_LIKE_FLOOR,
    SURFACE_LIKE_DEPTH_CLASS_OFFSET, STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON
}
#import "shaders/worldmap/land/noise.wgsl"::hash

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
    var uv = in.uv;
    if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
        let inst = instances[in.instance_index];
        let uv_in_tile = (uv - inst.uv_min) / (inst.uv_max - inst.uv_min);
        uv = inst.uv_min + apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5)) * (inst.uv_max - inst.uv_min);
    }
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
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT) {
        shaded = vec4<f32>(shaded.rgb * shaded.a * 2.0, shaded.a);
    }

    // Apply scene-wide lighting
    shaded = vec4<f32>(shaded.rgb * scene.global_lighting, shaded.a);

    out.color = vec4<f32>(shaded.rgb, shaded.a);
    return out;
}
