// -------------------------------------------------
// Land (map) tile.
// Apply the correct texture from the texture array.
// Apply Goraud shading.
// -------------------------------------------------

// The shader is invoked per-mesh instance (i.e. "draw call").

// Vertex attributes + Bevy hardcoded structs in:
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

const TILE_PX: u32         = 44;
const MAX_TILE_LAYERS: u32 = 2048;
//const MAX_TILE_LAYERS_VEC4: u32 = (MAX_TILE_LAYERS + 3) / 4;

const CHUNK_TILE_NUM_1D: u32 = 8;
const CHUNK_TILE_NUM_TOTAL: u32 = CHUNK_TILE_NUM_1D * CHUNK_TILE_NUM_1D;
//const CHUNK_TILE_NUM_TOTAL_VEC4: u32 = (CHUNK_TILE_NUM_TOTAL + 3) / 4;

const TEX_SIZE_SMALL: u32 = 0;
const TEX_SIZE_BIG: u32 = 1;

// Uniform buffers:
// All uniforms (including textures, uniform buffers, etc.) are set per-mesh—in Bevy,
//  this means per entity with a material/mesh combination.
// When your chunk/mesh is rendered, your material's uniforms are set for that mesh only.
//  Each chunk/mesh can have its own uniform buffer, with info for just that chunk.
// https://www.w3.org/TR/WGSL/#alignment-and-size

/*
// Bevy's default vertex shader utilities (from mesh_functions) handle the mesh/instance transform,
//  including instancing and batching quirks. They abstract over Mesh.model, View.view_proj, and ensure the correct coordinate space.

// Bevy default rendering pipeline default uniforms:

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
var<storage, read> Mesh: MeshUniform;
*/

// — Group 2: land material, custom uniforms.
struct LandUniforms {
    light_dir: vec3<f32>,    // 12 bytes
    _pad:   f32,             // pad to 16‐byte alignment by adding 4 bytes (f32)
    chunk_origin: vec2<f32>,
    _pad2: vec2<f32>,
    tiles: array<TileUniform, CHUNK_TILE_NUM_TOTAL>,
};
struct TileUniform {
    tile_height: u32,
    texture_size: u32,
    texture_layer: u32,
    texture_hue: u32,
};

@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small: texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big: texture_2d_array<f32>;
@group(2) @binding(103)
var<uniform> land: LandUniforms;


@vertex
fn vertex(
    in: Vertex,
//    @builtin(front_facing) is_front: bool,
) -> VertexOutput {
    var out: VertexOutput;

    let mesh_world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var world_from_local = mesh_world_from_local;

    out.world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0));
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);
    out.instance_index = in.instance_index;

    // pass UV
    out.uv = in.uv;

    // Compute Gouraud lighting
    //let world_norm = normalize((Mesh.model * vec4<f32>(in.normal, 0.0)).xyz);
    let world_norm = normalize(out.world_normal); // already in world space

    // Use the unused uv_1 attr to pass this data to the fragment shader
    out.uv_b = vec2<f32>(
        max(dot(world_norm, normalize(land.light_dir)), 0.0), // Lambert
        0.0
    );
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;

    // Get the layer of the texture in the texture array from our uniform buffer.
    let local_x = in.world_position.x - land.chunk_origin.x;
    let local_z = in.world_position.z - land.chunk_origin.y;
    let tx: u32 = clamp(u32(floor(local_x)), 0u, CHUNK_TILE_NUM_1D-1u);
    let ty: u32 = clamp(u32(floor(local_z)), 0u, CHUNK_TILE_NUM_1D-1u);
    let tile_index: u32 = ty * CHUNK_TILE_NUM_1D + tx;

    let texture_size: u32  = land.tiles[tile_index].texture_size;
    let texture_layer: u32 = land.tiles[tile_index].texture_layer;
    //let texture_hue: u32   = land.tiles[tile_index].texture_hue;

    // Sample the land tile texture.
    //  We could use textureSample or textureSampleLevel, the latter lets us choose the mip level
    //  (we want to force 0, even if it wouldn't be necessary because the texture array we create has only 1 level).
    var tex_color: vec4<f32>; // RGBA8888
    if (texture_size == TEX_SIZE_BIG) {
        tex_color = textureSampleLevel(texarray_big,   texarray_sampler, in.uv, u32(texture_layer), 0.0);
    } else {
        tex_color = textureSampleLevel(texarray_small, texarray_sampler, in.uv, u32(texture_layer), 0.0);
    }

    // Ambient light factor.
    let ao = 0.4;
    let lambert = in.uv_b.x;  // Lambert calculated in the vertex shader.
    let brightness = lambert * 0.6 + ao * 0.4;
    //let brightness = lambert * 0.7 + ao * 0.3;

    // Debug:
    //out.color = vec4<f32>(f32(layer) / f32(MAX_TILE_LAYERS), 0.0, 0.0, 1.0); // debug
    //out.color = vec4<f32>(in.world_position.y * 0.05, 0.5, 0.0, 1.0); // debug
    //let lit = out.color.rgb * brightness;
    //out.color = vec4<f32>(lit, out.color.a);
    //return out;

    /*
    // Filters:

    // Desaturation
    let avg = (lit.r + lit.g + lit.b) / 3.0;
    let saturation = 0.66; // tweak for taste
    let uo_lit = mix(vec3<f32>(avg, avg, avg), lit, saturation);
    out.color = vec4<f32>(uo_lit, tex_color.a);

    // Fog
    let fog = 0.02 * in.world_position.z; // fake simple fog, tweak for taste
    out.color = mix(out.color, vec4<f32>(0.6,0.7,1.0,1.0), clamp(fog,0.0,0.5));
    */

    // For debugging the texture quality issue, avoid Gouraud lighting.
    //out.color = vec4<f32>(tex_color.rgb, tex_color.a);
    //return out;

    // Final lit color (Gouraud: modulate by interpolated lighting).
    let lit = tex_color.rgb * brightness;
    out.color = vec4<f32>(lit, tex_color.a);
    return out;
}
