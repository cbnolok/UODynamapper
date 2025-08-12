// -------------------------------------------------
// Land (map) tile.
// Apply the correct texture from the texture array.
// Apply Gouraud (per-vertex) or bicubic (per-fragment) lighting, switchable.
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

const CHUNK_TILE_NUM_1D: u32 = 8;
const CHUNK_TILE_NUM_TOTAL: u32 = CHUNK_TILE_NUM_1D * CHUNK_TILE_NUM_1D;

const TEX_SIZE_SMALL: u32 = 0;
const TEX_SIZE_BIG: u32 = 1;

// Uniform buffers:
// All uniforms (including textures, uniform buffers, etc.) are set per-mesh—in Bevy,
//  this means per entity with a material/mesh combination.
// When your chunk/mesh is rendered, your material's uniforms are set for that mesh only.
//  Each chunk/mesh can have its own uniform buffer, with info for just that chunk.

struct LandUniforms {
    light_dir: vec3<f32>,        // 12 bytes
    _pad: f32,                   // 4 bytes padding to align to 16 bytes
    chunk_origin: vec2<f32>,     // 8 bytes
    _pad2: vec2<f32>,            // 8 bytes padding to align the array
    tiles: array<TileUniform, CHUNK_TILE_NUM_TOTAL>,  // tile info
    use_vertex_lighting: u32,    // 0 = per-pixel bicubic lighting, 1 = per-vertex Gouraud lighting
    _pad3: vec3<u32>,            // padding for 16-byte alignment
};

struct TileUniform {
    tile_height: u32,
    texture_size: u32,
    texture_layer: u32,
    texture_hue: u32,
};

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
@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small: texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big: texture_2d_array<f32>;
@group(2) @binding(103) var<uniform> land: LandUniforms;

// Helper: get tile height at coords (clamped inside chunk)
fn tile_height_at(tx: i32, ty: i32) -> f32 {
    let clamped_tx = clamp(tx, 0, i32(CHUNK_TILE_NUM_1D) - 1);
    let clamped_ty = clamp(ty, 0, i32(CHUNK_TILE_NUM_1D) - 1);
    let index = clamped_ty * i32(CHUNK_TILE_NUM_1D) + clamped_tx;
    return f32(land.tiles[u32(index)].tile_height);
}

// Bicubic interpolation helper function for 1D
fn cubic_interp(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a = -0.5*p0 + 1.5*p1 - 1.5*p2 + 0.5*p3;
    let b = p0 - 2.5*p1 + 2.0*p2 - 0.5*p3;
    let c = -0.5*p0 + 0.5*p2;
    let d = p1;
    return a*t*t*t + b*t*t + c*t + d;
}

// Bicubic normal calculation from tile heights for smooth lighting
fn get_bicubic_normal(x: f32, z: f32) -> vec3<f32> {
    // Position relative to chunk origin
    let local_x = x - land.chunk_origin.x;
    let local_z = z - land.chunk_origin.y;

    // Clamp inside chunk tiles
    let tx = clamp(i32(floor(local_x)), 0, i32(CHUNK_TILE_NUM_1D) - 1);
    let ty = clamp(i32(floor(local_z)), 0, i32(CHUNK_TILE_NUM_1D) - 1);

    // Normalized fractional part inside tile
    let fx = local_x - floor(local_x);
    let fz = local_z - floor(local_z);

    // Gather heights of the 16 surrounding tiles for bicubic (4x4)
    // Clamp coords for each neighbor
    let h00 = tile_height_at(tx - 1, ty - 1);
    let h10 = tile_height_at(tx    , ty - 1);
    let h20 = tile_height_at(tx + 1, ty - 1);
    let h30 = tile_height_at(tx + 2, ty - 1);

    let h01 = tile_height_at(tx - 1, ty    );
    let h11 = tile_height_at(tx    , ty    );
    let h21 = tile_height_at(tx + 1, ty    );
    let h31 = tile_height_at(tx + 2, ty    );

    let h02 = tile_height_at(tx - 1, ty + 1);
    let h12 = tile_height_at(tx    , ty + 1);
    let h22 = tile_height_at(tx + 1, ty + 1);
    let h32 = tile_height_at(tx + 2, ty + 1);

    let h03 = tile_height_at(tx - 1, ty + 2);
    let h13 = tile_height_at(tx    , ty + 2);
    let h23 = tile_height_at(tx + 1, ty + 2);
    let h33 = tile_height_at(tx + 2, ty + 2);

    // Interpolate heights in x for each row
    let col0 = cubic_interp(h00, h10, h20, h30, fx);
    let col1 = cubic_interp(h01, h11, h21, h31, fx);
    let col2 = cubic_interp(h02, h12, h22, h32, fx);
    let col3 = cubic_interp(h03, h13, h23, h33, fx);

    // Interpolate result in z
    let height = cubic_interp(col0, col1, col2, col3, fz);

    // Compute partial derivatives (slopes) numerically for normal
    // Derivative wrt x
    let dx = (
        cubic_interp(h10, h20, h30, h30, fx + 0.01) -
        cubic_interp(h10, h20, h30, h30, fx - 0.01)
    ) / 0.02;
    // Derivative wrt z
    let dz = (
        cubic_interp(col0, col1, col2, col3, fz + 0.01) -
        cubic_interp(col0, col1, col2, col3, fz - 0.01)
    ) / 0.02;

    // Construct normal vector, note Y up
    let normal = normalize(vec3<f32>(-dx, 1.0, -dz));
    return normal;
}

@vertex
fn vertex(
    in: Vertex,
//    @builtin(front_facing) is_front: bool,
) -> VertexOutput {
    var out: VertexOutput;

    // Transform mesh position and normal to world space
    let mesh_world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var world_from_local = mesh_world_from_local;

    out.world_normal = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0));
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);
    out.instance_index = in.instance_index;

    // Pass UV through
    out.uv = in.uv;

    // Compute Gouraud lighting (per-vertex Lambert)
    let world_norm = normalize(out.world_normal); // already in world space
    let lambert = max(dot(world_norm, normalize(land.light_dir)), 0.0);

    // Pass lighting value to fragment shader in uv_b.x (uv_b.y unused)
    out.uv_b = vec2<f32>(lambert, 0.0);

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;

    // Calculate local tile coordinates from world position minus chunk origin
    let local_x = in.world_position.x - land.chunk_origin.x;
    let local_z = in.world_position.z - land.chunk_origin.y;

    // Clamp tile coords to chunk tile bounds
    let tx: u32 = clamp(u32(floor(local_x)), 0u, CHUNK_TILE_NUM_1D - 1u);
    let ty: u32 = clamp(u32(floor(local_z)), 0u, CHUNK_TILE_NUM_1D - 1u);
    let tile_index: u32 = ty * CHUNK_TILE_NUM_1D + tx;

    // Fetch per-tile data
    let tile_height: u32 = land.tiles[tile_index].tile_height;
    let texture_size: u32 = land.tiles[tile_index].texture_size;
    let texture_layer: u32 = land.tiles[tile_index].texture_layer;
    //let texture_hue: u32 = land.tiles[tile_index].texture_hue; // unused

    // Compute local UV within the tile by fract(local_x, local_z)
    let uv_tile = vec2<f32>(fract(local_x), fract(local_z));

    // Sample the correct texture array based on texture size (big/small)
    var tex_color: vec4<f32>;
    if (texture_size == TEX_SIZE_BIG) {
        tex_color = textureSampleLevel(texarray_big, texarray_sampler, uv_tile, texture_layer, 0.0);
    } else {
        tex_color = textureSampleLevel(texarray_small, texarray_sampler, uv_tile, texture_layer, 0.0);
    }

    // Ambient Occlusion constant
    let ao = 0.4;

    // Calculate lighting factor (Lambert) from vertex shader or fallback
    // use_vertex_lighting = 1: use in.uv_b.x computed in vertex shader (Lambert)
    // use_vertex_lighting = 0: compute Lambert here per-pixel for smoother shading
    let lambert: f32 = select(
        max(dot(normalize(in.world_normal), normalize(land.light_dir)), 0.0),
        in.uv_b.x,
        land.use_vertex_lighting == 1u
    );

    // Calculate brightness with mix of ambient occlusion and Lambert
    let brightness = lambert * 0.6 + ao * 0.4;

    // Compute final color with lighting applied
    let lit = tex_color.rgb * brightness;

    // Optional: apply sharpness modifier if needed (from uniform)
    //let sharpness = f32(land.sharpness);
    //lit = pow(lit, vec3<f32>(sharpness));

    out.color = vec4<f32>(lit, tex_color.a);
    return out;
}


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

