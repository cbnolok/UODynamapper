

// ----------------------------------------------------------------------------
// assets/shaders/terrain.wgsl
//  • View uniform as uniform buffer (group 0)
//  • Mesh transform as storage buffer (group 1) ← MATCHES Bevy 0.13
//  • Material uniforms + texture array (group 2)
//  • Gouraud (per‐vertex) diffuse lighting
// ----------------------------------------------------------------------------

// Bevy hardcoded structs in:
//  bevy/crates/bevy_pbr/src/render/forward_io.wgsl
//  main/crates/bevy_pbr/src/render/pbr.wgsl
// Bevy (like most engines) only stores what you actually insert into the mesh, so if i do not
//  insert the vertex attribute 'color', the vertex shader input Vertex struct won't have it.
// WGSL (and GPU in general) does not allow querying if a vertex attribute exists at runtime.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions, pbr_functions, pbr_fragment,
    view_transformations,
}

const MAX_TILE_LAYERS: u32 = 2048; 
const TILE_PX: u32         = 44;

// — Group 0: camera/view
struct ViewUniform {
    view_proj: mat4x4<f32>,  // 64 bytes
};
@group(0) @binding(0)
var<uniform> View: ViewUniform;

// — Group 1: mesh (model) transform
struct MeshUniform {
    model: mat4x4<f32>,      // 64 bytes
};
@group(1) @binding(0)
// StorageAccess(LOAD) → storage buffer
var<storage, read> Mesh: MeshUniform;

// — Group 2: terrain material
@group(2) @binding(0) var atlas: texture_2d_array<f32>;
@group(2) @binding(1) var atlas_sampler: sampler;

// ADD HUE AND LAYER INSTEAD OF USING VERTEX ATTRIBUTES ??
struct TerrainUniforms {
    light_dir: vec3<f32>,    // 12 bytes
    _pad: f32,               // pad to 16‐byte alignment
};
@group(2) @binding(2)
var<uniform> terrain: TerrainUniforms;


@vertex
fn vertex(
    in: Vertex,
//    @builtin(front_facing) is_front: bool,
) -> VertexOutput {
    // We don't actually need the vertex shader, just pass around things and keep it for now.
    var out: VertexOutput;

    let mesh_world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var world_from_local = mesh_world_from_local;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        in.normal, in.instance_index
    );

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0));
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);
    out.instance_index = in.instance_index;
    
    /*
    out.uv_b = in.uv_b;
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        in.tangent,
        in.instance_index
    );
    out.color = in.color;
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        in.instance_index, mesh_world_from_local[3]);
    */
    
    // model & viewproj
    //let world_pos = Mesh.model * vec4<f32>(in.position, 1.0);
    //out.clip_pos  = View.view_proj * out.world_position; // world_pos;
    
    // pass UV & texture layer (index) in the texture array
    out.uv        = in.uv;
    //out.layer     = in.layer_hue >> 16u;
    out.layer_hue = in.layer_hue;

    // compute Gouraud lighting
    let world_norm = normalize((Mesh.model * vec4<f32>(in.normal, 0.0)).xyz);
    out.light      = max(dot(world_norm, normalize(terrain.light_dir)), 0.0);   // Lambert
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;

    // Sample your texture (skip or replace if untextured):
    let tex_color = textureSample(atlas, atlas_sampler, in.uv, in.layer);

    // Small ambient factor
    let ao = 0.2;
    let lambert = in.light;
    let brightness = lambert * 0.6 + ao * 0.4;

    // Final lit color (Gouraud: modulate by interpolated lighting)
    let lit = tex_color.rgb * brightness;

    out.color = vec4<f32>(lit, tex_color.a);
    return out;
}

/*
@fragment
//fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;
    out.color = vec4<f32>(1.0, 0.0, 0.0, 1.0); // bright red.

    // Test 1: does the code reach here?
    //return out;

    /*
    // Test 2
    // you’ll see each chunk colored by its layer index. This tells us the layer_hue attribute is flowing through.
    //let layer_f = f32(in.layer) / f32(MAX_TILE_LAYERS);
    //return vec4(layer_f, 1.0 - layer_f, 0.0, 1.0);
    */

    // generate a PbrInput struct from the StandardMaterial bindings
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // we can optionally modify the input before lighting and alpha_discard is applied
    //pbr_input.material.base_color.b = pbr_input.material.base_color.r;

    // alpha discard
    //pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: VertexOutput;
    // apply lighting
    out.color = apply_pbr_lighting(pbr_input);

    // we can optionally modify the lit color before post-processing is applied
    //out.color = vec4<f32>(vec4<u32>(out.color * f32(my_extended_material.quantize_steps))) / f32(my_extended_material.quantize_steps);

    // apply in-shader post processing (fog, alpha-premultiply, and also tonemapping, debanding if the camera is non-hdr)
    // note this does not include fullscreen postprocessing effects like bloom.
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);

    // we can optionally modify the final result here
    //out.color = out.color * 2.0;


    /*
    // IF using an atlas:
    // UVs need to be offset and scaled for the rectangle of the atlas corresponding to in.layer.
    // in.uv is 0→1 for a single tile
    let tiles_per_row = 16u;
    let tile_size = 1.0 / f32(tiles_per_row);
    let tile_x = f32(in.layer % tiles_per_row);
    let tile_y = f32(in.layer / tiles_per_row);
    let uv_scaled = in.uv * tile_size;
    let uv_final  = uv_scaled + vec2(tile_x, tile_y) * tile_size;
    // sample from the 2D array: 4‐arg textureSample
    let tex_color = textureSample(atlas, atlas_sampler, uv_final, in.layer);
    */
    let tex_color = textureSample(atlas, atlas_sampler, in.uv, in.layer);

    // Gouraud: modulate by interpolated light.    
    // Pure diffuse = albedo * max(dot(normal, light),0) often leaves large black areas.
    // Even in stylized terrain you usually add a small ambient term:
    let ao = 0.2;  // ambient factor
    let lambert = in.light;
    //let brightness = lambert * 0.8 + ao * 0.2;
    let brightness = lambert * 0.6 + 0.4;
    let lit = tex_color.rgb * brightness;
    
    // No ambient lighting.
    //let lit = tex_color.rgb * in.light;    
    
    out.color = vec4<f32>(lit, tex_color.a);
    return out;
}
*/