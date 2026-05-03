#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    view_transformations,
}

struct SpriteInstance {
    world_x: f32,
    world_z: f32,
    world_y: f32,
    layer: u32,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    pixel_size: vec2<f32>,
    _pad: vec2<f32>,
    color_rgba: vec4<f32>,
}

struct SpriteParams {
    render_mode: u32,
    alpha_cutoff: f32,
    _pad: vec2<f32>,
}

@group(3) @binding(100) var art_atlas_sampler: sampler;
@group(3) @binding(101) var art_atlas: texture_2d_array<f32>;
@group(3) @binding(102) var<storage, read> instances: array<SpriteInstance>;
@group(3) @binding(103) var<uniform> sprite_params: SpriteParams;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let inst = instances[vertex.instance_index];
    
    // We can use vertex.position.x and .z for corner offsets (0.0 to 1.0)
    let corner = vec2<f32>(vertex.position.x, vertex.position.z);
    
    // In Bevy's 3D view, x and z are horizontal, y is up.
    let world_pos = vec3<f32>(
        inst.world_x + corner.x * inst.pixel_size.x,
        inst.world_y,
        inst.world_z + corner.y * inst.pixel_size.y,
    );
    
    var out: VertexOutput;
    out.position = view_transformations::position_world_to_clip(world_pos);
    out.uv = mix(inst.uv_min, inst.uv_max, corner);
    // Pack layer into uv_b.x
    out.uv_b = vec2<f32>(f32(inst.layer), 0.0);
    out.color = inst.color_rgba;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if sprite_params.render_mode == 1u {
        // Dot mode (radarcol)
        return in.color;
    }
    
    let layer = u32(in.uv_b.x);
    let color = textureSample(art_atlas, art_atlas_sampler, in.uv, i32(layer));
    if color.a < sprite_params.alpha_cutoff {
        discard;
    }
    
    return color * in.color;
}
