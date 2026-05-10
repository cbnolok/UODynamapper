#import bevy_pbr::{
    forward_io::Vertex,
    mesh_functions,
    view_transformations,
}

struct GroundTileInstance {
    world_x: f32,
    world_z: f32,
    world_y: f32,
    layer: u32,
    depth_class: u32,
    base_world_y: f32,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    tile_x: f32,
    tile_y: f32,
    priority_z_units: f32,
    _pad1: u32,
    _pad2: vec2<u32>,
    color_rgba: vec4<f32>,
}

struct SpriteParams {
    render_mode: u32,
    alpha_cutoff: f32,
    pass_mode: u32,
    _pad: u32,
    map_width_tiles: f32,
    map_height_tiles: f32,
    _pad2: vec2<u32>,
}

struct GroundVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uv_b: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) logical_depth: f32,
}

struct GroundFragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@group(3) @binding(100) var art_atlas_sampler: sampler;
@group(3) @binding(101) var art_atlas: texture_2d_array<f32>;
@group(3) @binding(102) var<storage, read> instances: array<GroundTileInstance>;
@group(3) @binding(103) var<uniform> sprite_params: SpriteParams;

const HEIGHT_SCALE: f32 = 0.1;
const DEPTH_CLASS_BACKGROUND: u32 = 1u;
const DEPTH_CLASS_FOLIAGE: u32 = 2u;
const DEPTH_CLASS_ROOF: u32 = 3u;
const PASS_MODE_OPAQUE: u32 = 0u;
const PASS_MODE_TRANSPARENT: u32 = 1u;

fn clip_depth_to_frag_depth(clip_position: vec4<f32>) -> f32 {
    return clamp(clip_position.z / clip_position.w, 0.0, 1.0);
}

fn depth_class_logical_offset(depth_class: u32) -> f32 {
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

@vertex
fn vertex(vertex: Vertex) -> GroundVertexOutput {
    let inst = instances[mesh_functions::get_tag(vertex.instance_index)];

    let corner = vec2<f32>(vertex.position.x, vertex.position.z);
    let world_pos = vec3<f32>(
        inst.world_x + corner.x,
        inst.world_y,
        inst.world_z + corner.y,
    );
    var out: GroundVertexOutput;
    out.position = view_transformations::position_world_to_clip(world_pos);
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    out.uv_b = vec2<f32>(f32(inst.layer), 0.0);
    out.color = inst.color_rgba;
    out.logical_depth = logical_depth_from_projected_priority(inst);
    return out;
}

@fragment
fn fragment(in: GroundVertexOutput) -> GroundFragmentOutput {
    var out: GroundFragmentOutput;
    out.depth = in.logical_depth;

    let layer = u32(in.uv_b.x);
    let color = textureSample(art_atlas, art_atlas_sampler, in.uv, i32(layer));
    if (sprite_params.pass_mode == PASS_MODE_OPAQUE) {
        if color.a < 0.0001 || color.a < sprite_params.alpha_cutoff {
            discard;
        }
        out.depth = in.logical_depth;
    } else {
        if color.a >= sprite_params.alpha_cutoff {
            discard;
        }
        out.depth = in.position.z;
    }

    var shaded = color * in.color;
    if (sprite_params.pass_mode == PASS_MODE_TRANSPARENT) {
        shaded = vec4<f32>(shaded.rgb * shaded.a * 2.0, shaded.a);
    }
    out.color = shaded;
    return out;
}
