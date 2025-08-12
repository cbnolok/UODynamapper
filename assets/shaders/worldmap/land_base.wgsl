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

const USE_TEST_TUNABLES: u32 = 1;
const TEST_TUNABLES_USE_VERTEX_LIGHTING: u32 = 1;
const TEST_TUNABLES_SHARPNESS_FACTOR: f32 = 1.0;
const TEST_TUNABLES_SHARPNESS_MIX_FACTOR: f32 = 1.0;

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
    light_dir: vec3<f32>,        // 12 bytes
    _pad: f32,                   // 4 bytes padding to align to 16 bytes
    chunk_origin: vec2<f32>,     // 8 bytes
    _pad2: vec2<f32>,            // 8 bytes padding to align the array
    tiles: array<TileUniform, CHUNK_TILE_NUM_TOTAL>,  // tile info
};

struct TileUniform {
    tile_height: u32,
    texture_size: u32,
    texture_layer: u32,
    texture_hue: u32,
};

// === User tunable parameters ===
struct TunablesUniform {
    // Like a bool: 0 = per-pixel bicubic lighting, 1 = per-vertex Gouraud lighting
    use_vertex_lighting: u32,
    // Sharpness of normal smoothing: 0.0 = blocky normals (flat shading), 1.0 = full bicubic smooth normals
    sharpness_factor: f32,
    // How much of the sharpened color is mixed to the original color.
    sharpness_mix_factor: f32,
    _pad: f32,
}

@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small: texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big: texture_2d_array<f32>;
@group(2) @binding(103) var<uniform> land: LandUniforms;
@group(2) @binding(104) var<uniform> tunables: TunablesUniform;


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

// Bicubic interpolation derivative
fn cubic_interp_deriv(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let a = -0.5*p0 + 1.5*p1 - 1.5*p2 + 0.5*p3;
    let b = p0 - 2.5*p1 + 2.0*p2 - 0.5*p3;
    let c = -0.5*p0 + 0.5*p2;
    return 3.0*a*t*t + 2.0*b*t + c;
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

    // Compute partial derivatives (slopes) for normal.
    // dx is computed analytically from the derivatives of the interpolation functions.
    let d_col0_dx = cubic_interp_deriv(h00, h10, h20, h30, fx);
    let d_col1_dx = cubic_interp_deriv(h01, h11, h21, h31, fx);
    let d_col2_dx = cubic_interp_deriv(h02, h12, h22, h32, fx);
    let d_col3_dx = cubic_interp_deriv(h03, h13, h23, h33, fx);
    let dx = cubic_interp(d_col0_dx, d_col1_dx, d_col2_dx, d_col3_dx, fz);

    // dz is computed numerically from the final height interpolation.
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
) -> VertexOutput {
    var out: VertexOutput;

    // Get instance transform for this mesh
    let mesh_world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var world_from_local = mesh_world_from_local;

    // World space normal from mesh geometry
    var world_normal_geom = mesh_functions::mesh_normal_local_to_world(in.normal, in.instance_index);
    // World space position
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(in.position, 1.0));
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);
    out.instance_index = in.instance_index;

    // Pass UV for texture sampling
    out.uv = in.uv;

    // Decide lighting path
    let use_vertex_lighting: u32 = select(tunables.use_vertex_lighting, TEST_TUNABLES_USE_VERTEX_LIGHTING, USE_TEST_TUNABLES == 1);
    if (use_vertex_lighting == 0u) {
        // === Gouraud (vertex) lighting mode ===

        // Compute bicubic normal from the heightmap
        // get_bicubic_normal returns tangent-space normal, so we must transform it to world space
        let bicubic_normal = get_bicubic_normal(out.uv.x, out.uv.y);
        let bicubic_normal_world = normalize(
            mesh_functions::mesh_normal_local_to_world(bicubic_normal, in.instance_index)
        );

        // At chunk boundaries, bicubic normals are incorrect due to missing neighbor data.
        // We blend them with the mesh geometry normals to hide the seams.
        let local_x = out.uv.x - land.chunk_origin.x;
        let local_z = out.uv.y - land.chunk_origin.y;
        let tx = floor(local_x);
        let ty = floor(local_z);
        let dist_x = min(tx, 7.0 - tx);
        let dist_y = min(ty, 7.0 - ty);
        let dist_from_edge = min(dist_x, dist_y);
        let blend_factor = 1.0 - smoothstep(0.0, 2.0, dist_from_edge);

        // Combine geometry normal and bicubic normal, blending at the edges.
        let combined_normal_world = normalize(mix(bicubic_normal_world, world_normal_geom, blend_factor));

        // Lambert term for Gouraud shading
        let lambert = max(dot(combined_normal_world, normalize(land.light_dir)), 0.0);

        // Pass Lambert value in uv_b.x so fragment shader can use it
        out.uv_b = vec2<f32>(lambert, 0.0);

        // Store the combined normal for possible debugging or additional effects
        out.world_normal = combined_normal_world;

    } else {
        // === Per-pixel (fragment) lighting mode ===
        // We just pass the mesh geometry normal here; fragment shader will sample heightmap normal
        out.world_normal = world_normal_geom;

        // uv_b.x unused in this path, set to 0
        out.uv_b = vec2<f32>(0.0, 0.0);
    }

    return out;
}

@fragment
fn fragment(
    in: VertexOutput
) -> @location(0) vec4<f32> {
    var out: FragmentOutput;

    // === Per-tile data fetch ===
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

    // Compute local UV within the tile
    let uv_tile = vec2<f32>(fract(local_x), fract(local_z));

    // Sample the correct texture array based on texture size (big/small)
    var tex_color: vec4<f32>;
    if (texture_size == TEX_SIZE_BIG) {
        tex_color = textureSampleLevel(texarray_big, texarray_sampler, uv_tile, texture_layer, 0.0);
    } else {
        tex_color = textureSampleLevel(texarray_small, texarray_sampler, uv_tile, texture_layer, 0.0);
    }

    // === Lighting calculation ===
    var lighting: f32;
    let use_vertex_lighting: u32 = select(tunables.use_vertex_lighting, TEST_TUNABLES_USE_VERTEX_LIGHTING, USE_TEST_TUNABLES == 1);
    if (use_vertex_lighting == 1u) {
        // Vertex lighting (Gouraud)
        lighting = in.uv_b.x;
    } else {
        // Per-pixel lighting using bicubic normal.
        // At chunk boundaries, bicubic normals are incorrect due to missing neighbor data.
        // We blend them with the mesh geometry normals to hide the seams.
        var normal_local_bicubic = get_bicubic_normal(in.uv.x, in.uv.y);
        normal_local_bicubic = normalize(normal_local_bicubic);
        let normal_world_bicubic = mesh_functions::mesh_normal_local_to_world(normal_local_bicubic, in.instance_index);

        let dist_x = min(f32(tx), 7.0 - f32(tx));
        let dist_y = min(f32(ty), 7.0 - f32(ty));
        let dist_from_edge = min(dist_x, dist_y);
        let blend_factor = 1.0 - smoothstep(0.0, 2.0, dist_from_edge);

        let final_normal_world = normalize(mix(normal_world_bicubic, in.world_normal, blend_factor));
        lighting = max(dot(final_normal_world, normalize(land.light_dir)), 0.0);
    }


    // === Sharpness blend ===
    let sharpness_factor = select(tunables.sharpness_factor, TEST_TUNABLES_SHARPNESS_FACTOR, USE_TEST_TUNABLES == 1);
    let sharpness_mix_factor = select(tunables.sharpness_mix_factor, TEST_TUNABLES_SHARPNESS_MIX_FACTOR, USE_TEST_TUNABLES == 1);
    if ((sharpness_factor != 0.0) && (sharpness_mix_factor != 0.0)) {
        let color_soft = lighting;
        let color_sharp = pow(lighting, sharpness_factor);
        lighting = mix(color_soft, color_sharp, sharpness_mix_factor);
    }

    // === Ambient light ===
    // Ambient Occlusion constant
    let ao = 0.4;

    // Calculate brightness with mix of ambient occlusion and Lambert (Gouraud) or bicubic normal.
    let brightness = lighting * 0.6 + ao * 0.4;

    // Compute final color with lighting applied
    let lit = tex_color.rgb * brightness;

    out.color = vec4<f32>(lit, tex_color.a);
    return out.color;
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

