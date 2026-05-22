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

#import "shaders/world/art/art_bindings.wgsl"::{
    SpriteInstance, SpriteParams, SceneUniform, LandEffectsUniform, GlobalLightingUniforms,
    art_atlas_sampler, art_atlas, instances, sprite_params, scene, effects, global_light,
    INV_SQRT_2, BILLBOARD_RIGHT_XZ, HEIGHT_SCALE, DEPTH_CLASS_REGULAR, DEPTH_CLASS_BACKGROUND,
    DEPTH_CLASS_FOLIAGE, DEPTH_CLASS_ROOF, DEPTH_CLASS_SURFACE_LIKE_FLOOR, PASS_MODE_OPAQUE,
    PASS_MODE_TRANSPARENT, SURFACE_LIKE_DEPTH_CLASS_OFFSET, STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON
}
#import "shaders/world/land/noise.wgsl"::hash

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
    return clip_depth_to_frag_depth(view_transformations::position_world_to_clip(logical_world_pos));
}

fn apply_sort_bias_to_frag_depth(depth: f32, sort_bias_ordinal: u32) -> f32 {
    return clamp(depth - f32(sort_bias_ordinal) * STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON, 0.0, 1.0);
}

fn apply_art_surface_shading(rgb: vec3<f32>, uv_in_tile: vec2<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let shadow_strength = clamp(effects.art_shadow_strength, 0.0, 1.0);
    let highlight_strength = clamp(effects.art_highlight_strength, 0.0, 1.0);
    let tint_strength = clamp(effects.art_depth_tint_strength, 0.0, 1.0);
    let light_static = clamp(effects.light_decal_intensity, 0.0, 2.0);

    let vertical_light = clamp(1.0 - uv_in_tile.y, 0.0, 1.0);
    let lower_occlusion = smoothstep(0.18, 1.0, uv_in_tile.y);
    let side_contact = 1.0 - smoothstep(0.0, 0.18, min(uv_in_tile.x, 1.0 - uv_in_tile.x));
    let contact_noise = 0.85 + 0.15 * hash(floor(world_pos.xz * 0.25));

    var out_rgb = rgb;
    out_rgb *= 1.0 - shadow_strength * (0.35 * lower_occlusion + 0.15 * side_contact) * contact_noise;
    out_rgb += rgb * global_light.light_color * vertical_light * highlight_strength * (0.25 + 0.25 * light_static);
    out_rgb = mix(out_rgb, out_rgb * global_light.atmosphere_tint, tint_strength * lower_occlusion);
    return max(out_rgb, vec3<f32>(0.0));
}

@vertex
fn vertex(vertex: Vertex) -> ArtVertexOutput {
    let inst = instances[u32(vertex.uv_b.x + 0.5)];

    let corner = vec2<f32>(vertex.position.x, vertex.position.z);
    let local = mix(inst.local_min, inst.local_max, vec2<f32>(corner.x, 1.0 - corner.y));
    let world_pos = vec3<f32>(
        inst.world_x + local.x * BILLBOARD_RIGHT_XZ.x,
        inst.world_y + local.y,
        inst.world_z + local.x * BILLBOARD_RIGHT_XZ.y,
    );
    var out: ArtVertexOutput;
    out.position = view_transformations::position_world_to_clip(world_pos);
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    out.uv_b = vec2<f32>(f32(inst.layer), 0.0);
    out.color = inst.color_rgba;
    out.logical_depth = apply_sort_bias_to_frag_depth(
        logical_depth_from_projected_priority(inst),
        inst.sort_bias_ordinal,
    );
    out.depth_class = inst.depth_class;
    out.is_wet = inst.is_wet_flags & 1u;
    out.world_pos = world_pos;
    out.instance_index = u32(vertex.uv_b.x + 0.5);
    return out;
}

@fragment
fn fragment(in: ArtVertexOutput) -> ArtFragmentOutput {
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
    if (effects.enable_water_animation == 1u && in.is_wet == 1u) {
        let uv_in_tile = (uv - inst.uv_min) / atlas_extent;
        uv = inst.uv_min + apply_water_animation(uv_in_tile, vec2<f32>(0.5, 0.5)) * atlas_extent;
    }
    let uv_in_tile_for_shading = clamp((uv - inst.uv_min) / atlas_extent, vec2<f32>(0.0), vec2<f32>(1.0));
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
    shaded = vec4<f32>(apply_art_surface_shading(shaded.rgb, uv_in_tile_for_shading, in.world_pos), shaded.a);
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
