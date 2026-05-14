#import bevy_pbr::{
    forward_io::Vertex,
    view_transformations,
    mesh_view_bindings::globals,
}
#import "shaders/worldmap/water.wgsl"::water_distort_uv

struct SpriteInstance {
    world_x: f32,
    world_z: f32,
    world_y: f32,
    layer: u32,
    depth_class: u32,
    base_world_y: f32,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    local_min: vec2<f32>,
    local_max: vec2<f32>,
    tile_x: f32,
    tile_y: f32,
    priority_z_units: f32,
    sort_bias_ordinal: u32,
    // Tiledata flags: bit 0 = is_wet (animated water UV distortion).
    is_wet_flags: u32,
    _pad_inst: u32,
    color_rgba: vec4<f32>,
}

struct SpriteParams {
    render_mode: u32,
    alpha_cutoff: f32,
    pass_mode: u32,
    _pad: u32,
    map_width_tiles: f32,
    map_height_tiles: f32,
    _pad_sp: vec2<u32>,
}

struct ArtVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) logical_depth: f32,
    @location(4) @interpolate(flat) depth_class: u32,
    @location(5) @interpolate(flat) is_wet: u32,
}

struct ArtFragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@group(3) @binding(100) var art_atlas_sampler: sampler;
@group(3) @binding(101) var art_atlas: texture_2d_array<f32>;
@group(3) @binding(102) var<storage, read> instances: array<SpriteInstance>;
@group(3) @binding(103) var<uniform> sprite_params: SpriteParams;

const INV_SQRT_2: f32 = 0.70710678118;
const BILLBOARD_RIGHT_XZ: vec2<f32> = vec2<f32>(INV_SQRT_2, -INV_SQRT_2);
const HEIGHT_SCALE: f32 = 0.1;
const DEPTH_CLASS_REGULAR: u32 = 0u;
const DEPTH_CLASS_BACKGROUND: u32 = 1u;
const DEPTH_CLASS_FOLIAGE: u32 = 2u;
const DEPTH_CLASS_ROOF: u32 = 3u;
const DEPTH_CLASS_SURFACE_LIKE_FLOOR: u32 = 4u;
const PASS_MODE_OPAQUE: u32 = 0u;
const PASS_MODE_TRANSPARENT: u32 = 1u;
const SURFACE_LIKE_DEPTH_CLASS_OFFSET: f32 = -4.0;
const STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON: f32 = 0.000001;

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
    // Pass the wet flag as a flat (non-interpolated) attribute to the fragment shader.
    out.is_wet = inst.is_wet_flags & 1u;
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
    // Apply animated water UV distortion if this sprite tile is marked IsWet.
    // The sin/cos breathing effect mirrors ClassicUO's reference implementation.
    var uv = in.uv;
    if (in.is_wet == 1u) {
        uv = water_distort_uv(uv);
    }
    let color = textureSample(art_atlas, art_atlas_sampler, uv, i32(layer));
    if (sprite_params.pass_mode == PASS_MODE_OPAQUE) {
        if color.a < 0.0001 || color.a < sprite_params.alpha_cutoff {
            discard;
        }
        out.depth = in.logical_depth;
    } else {
        if color.a >= sprite_params.alpha_cutoff {
            discard;
        }
        out.depth = in.logical_depth;
    }

    var shaded = color * in.color;
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT) {
        shaded = vec4<f32>(shaded.rgb * shaded.a * 2.0, shaded.a);
    }
    out.color = shaded;
    return out;
}
