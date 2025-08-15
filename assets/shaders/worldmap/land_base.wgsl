// Bevy I/O and helpers
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions, pbr_functions, pbr_fragment,
    view_transformations,
}

// ==============================
// Hot-reload debug flags
// ==============================
const DEBUG_EDGE_BLEND: u32 = 0u; // 0 = off, 1 = show edge blend factor in color
const DEBUG_NORMALS:    u32 = 0u; // 0 = off, 1 = show the final normal as RGB
const DEBUG_HEIGHT:     u32 = 0u; // Show tile height as grayscale
const DEBUG_SLOPE:      u32 = 0u; // Show slope steepness as grayscale

// Test-tunable passthrough (lets you override uniforms from constants for tests)
const USE_TEST_TUNABLES: u32 = 1u;

// Lighting mode: 0 = Gouraud (vertex) lighting, 1 = per-fragment lighting
const TEST_TUNABLES_USE_VERTEX_LIGHTING: u32 = 0u;

const TEST_TUNABLES_SHARPNESS_FACTOR:     f32 = 0.0;
const TEST_TUNABLES_SHARPNESS_MIX_FACTOR: f32 = 0.0;

// ====== TOGGLES: GRAPHICAL ENHANCEMENTS ======
const ENABLE_FRAGMENT_LIGHTING: u32 = 0u;

// 0 = geometric (hard) normals, 1 = bicubic (smooth) normals
const ENABLE_SMOOTH_NORMALS: u32 = 0u;

// Fake "shadowing" by darkening terrain based on height differences
const ENABLE_AMBIENT_OCCLUSION: u32 = 0u;

// Blend textures based on slope steepness
const ENABLE_SLOPE_BASED_TEXTURING: u32 = 0u;

// Add specular highlights for wet/snowy/glossy terrain
const ENABLE_SPECULAR: u32 = 0u;

// ==============================
// Chunk / grid layout constants
// ==============================
const CHUNK_TILE_NUM_1D: u32 = 8u;                          // 8x8 tiles form the "core" chunk
const HEIGHT_GRID_NUM_1D: u32 = CHUNK_TILE_NUM_1D + 1u;     // 9x9 heights to include neighbors
const HEIGHT_GRID_TOTAL:  u32 = HEIGHT_GRID_NUM_1D * HEIGHT_GRID_NUM_1D;

const TEX_SIZE_SMALL: u32 = 0u;
const TEX_SIZE_BIG:   u32 = 1u;

// ==============================
// Material / uniform interfaces
// ==============================
struct TileUniform {
    tile_height:   f32, // ALREADY SCALED to world units on the CPU side
    texture_size:  u32, // 0 = small texture atlas, 1 = big texture atlas
    texture_layer: u32, // array layer to sample
    texture_hue:   u32,
};

struct LandUniform {
    light_dir:    vec3<f32>,
    _pad0:        f32,
    chunk_origin: vec2<f32>, // chunk world origin in tile units (x,z)
    _pad1:        vec2<f32>,
    tiles:        array<TileUniform, HEIGHT_GRID_TOTAL>, // 9x9 entries
};

struct SceneUniform {
    camera_position: vec3<f32>,
    light_direction: vec3<f32>, // IMPORTANT: this MUST be normalized on the CPU
    _pad: f32,
}

struct TunablesUniform {
    use_vertex_lighting:   u32,
    sharpness_factor:      f32,
    sharpness_mix_factor:  f32,
    _pad:                  f32,
};

@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small:   texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big:     texture_2d_array<f32>;
@group(2) @binding(103) var<uniform> land:    LandUniform;
@group(2) @binding(104) var<uniform> scene:   SceneUniform;
@group(2) @binding(105) var<uniform> tunables: TunablesUniform;

// ==============================
// Utility: 9x9 grid height fetch
// ==============================
fn tile_height_at_9x9(ix: i32, iz: i32) -> f32 {
    let cx = clamp(ix, 0, i32(HEIGHT_GRID_NUM_1D) - 1);
    let cz = clamp(iz, 0, i32(HEIGHT_GRID_NUM_1D) - 1);
    let idx = cz * i32(HEIGHT_GRID_NUM_1D) + cx;
    return land.tiles[u32(idx)].tile_height;
}

// ==============================
// Cubic interpolation helpers
// ==============================
// We keep both the cubic value and the analytic derivative (w.r.t. t).
// This avoids numerical finite differences (eps) and is both faster and exact.
//
// NOTE on terminology: "w.r.t x" / "w.r.t z" means "with respect to x/z" in the
// calculus sense (partial derivatives). Here, x and z are the terrain's horizontal
// axes in tile units; t is a 1D interpolation parameter in [0,1] used inside cubic blends.
//
// The cubic interpolation polynomial is the same shape used previously. We
// compute both the interpolated value and its derivative at t. Those derivatives
// let us obtain analytic partials dH/dx and dH/dz for normals (see below).
fn cubic_interp_value_and_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> vec2<f32> {
    // Coefficients for the cubic polynomial (Catmull-Rom-like basis).
    // value(t) = ((a * t + b) * t + c) * t + d
    // d/dt     = 3*a*t^2 + 2*b*t + c
    //
    // WHY DERIVATIVES: Our terrain is a height field H(x,z). To light it
    // smoothly, we need its surface normal, which depends on the *slopes*
    // (partial derivatives) dH/dx and dH/dz. By deriving the cubic formula,
    // we get exact partials analytically, avoiding noisy finite differences.
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b =       p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0           + 0.5 * p2;
    let d =           p1;

    let value = ((a * t + b) * t + c) * t + d;
    let deriv = (3.0 * a * t * t) + (2.0 * b * t) + c; // derivative w.r.t. t
    return vec2<f32>(value, deriv);
}

// Convenience wrappers when only value or derivative is required.
fn cubic_interp_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return cubic_interp_value_and_derivative(p0, p1, p2, p3, t).x;
}
fn cubic_interp_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return cubic_interp_value_and_derivative(p0, p1, p2, p3, t).y;
}

// ==============================
// Bicubic normal from heights (analytic derivatives)
// ==============================
// We treat the 9x9 grid as samples of a height field H(x,z) at integer tile coords.
// For a point inside a tile, we compute bicubic interpolation to estimate H and
// compute its partial derivatives dH/dx and dH/dz analytically using the cubic
// derivative formulas above.
//
// Algorithm (high-level):
//  1) For each of the 4 z-rows, compute cubic interpolation along x: row_val[i] and
//     row_dval_dx[i] (derivative w.r.t x for that row).
//  2) Height H = cubic_interp(row_val0..3, frac_z).
//     dH/dx  = cubic_interp(row_dval_dx0..3, frac_z).  (derivatives along x are interpolated in z)
//  3) For derivative w.r.t z: for each of the 4 x-columns compute cubic_interp along z
//     and their derivatives (dcol_dz). Then dH/dz = cubic_interp(dcol_dz0..3, frac_x).
//
// This produces exact bicubic partials and avoids any finite difference epsilon.
fn get_bicubic_normal(world_x: f32, world_z: f32) -> vec3<f32> {
    let local_x = world_x - land.chunk_origin.x; // local tile coords within chunk
    let local_z = world_z - land.chunk_origin.y;

    // The integer tile "cell" where the point lies (0..7), plus fractional (0..1).
    let base_x = floor(local_x);
    let base_z = floor(local_z);
    let frac_x = local_x - base_x;
    let frac_z = local_z - base_z;

    // Integer indices in the 9x9 grid near our point.
    let ix = i32(base_x);
    let iz = i32(base_z);

    // Fetch 4x4 neighborhood; indexing consistent with earlier code:
    let h00 = tile_height_at_9x9(ix - 1, iz - 1);
    let h10 = tile_height_at_9x9(ix + 0, iz - 1);
    let h20 = tile_height_at_9x9(ix + 1, iz - 1);
    let h30 = tile_height_at_9x9(ix + 2, iz - 1);

    let h01 = tile_height_at_9x9(ix - 1, iz + 0);
    let h11 = tile_height_at_9x9(ix + 0, iz + 0);
    let h21 = tile_height_at_9x9(ix + 1, iz + 0);
    let h31 = tile_height_at_9x9(ix + 2, iz + 0);

    let h02 = tile_height_at_9x9(ix - 1, iz + 1);
    let h12 = tile_height_at_9x9(ix + 0, iz + 1);
    let h22 = tile_height_at_9x9(ix + 1, iz + 1);
    let h32 = tile_height_at_9x9(ix + 2, iz + 1);

    let h03 = tile_height_at_9x9(ix - 1, iz + 2);
    let h13 = tile_height_at_9x9(ix + 0, iz + 2);
    let h23 = tile_height_at_9x9(ix + 1, iz + 2);
    let h33 = tile_height_at_9x9(ix + 2, iz + 2);

    // --- Interpolate along x for each z-row: get value and derivative w.r.t x for each row ---
    // rowN = cubic_interp_value(row entries, frac_x)
    // drowN_dx = cubic_interp_derivative(row entries, frac_x)
    let row0_vd = cubic_interp_value_and_derivative(h00, h10, h20, h30, frac_x);
    let row1_vd = cubic_interp_value_and_derivative(h01, h11, h21, h31, frac_x);
    let row2_vd = cubic_interp_value_and_derivative(h02, h12, h22, h32, frac_x);
    let row3_vd = cubic_interp_value_and_derivative(h03, h13, h23, h33, frac_x);

    // Values along rows (these are functions of x evaluated at frac_x)
    let row0 = row0_vd.x;
    let row1 = row1_vd.x;
    let row2 = row2_vd.x;
    let row3 = row3_vd.x;

    // Derivatives of those rows w.r.t x (i.e., partial of bicubic's row interpolation)
    let drow0_dx = row0_vd.y;
    let drow1_dx = row1_vd.y;
    let drow2_dx = row2_vd.y;
    let drow3_dx = row3_vd.y;

    // Final height H (bicubic): interpolate the row values along z
    let height_here = cubic_interp_value(row0, row1, row2, row3, frac_z);

    // dH/dx = cubic_interp(drow0_dx..drow3_dx, frac_z)
    let dHdx = cubic_interp_value(drow0_dx, drow1_dx, drow2_dx, drow3_dx, frac_z);

    // --- For dH/dz, compute cubic interpolation along z for each x-column and get derivative ---
    // column values (interpolating along z)
    let col0_vd = cubic_interp_value_and_derivative(h00, h01, h02, h03, frac_z);
    let col1_vd = cubic_interp_value_and_derivative(h10, h11, h12, h13, frac_z);
    let col2_vd = cubic_interp_value_and_derivative(h20, h21, h22, h23, frac_z);
    let col3_vd = cubic_interp_value_and_derivative(h30, h31, h32, h33, frac_z);

    let col0 = col0_vd.x;
    let col1 = col1_vd.x;
    let col2 = col2_vd.x;
    let col3 = col3_vd.x;

    // Derivative of columns w.r.t z
    let dcol0_dz = col0_vd.y;
    let dcol1_dz = col1_vd.y;
    let dcol2_dz = col2_vd.y;
    let dcol3_dz = col3_vd.y;

    // dH/dz = cubic_interp(dcol0_dz .. dcol3_dz, frac_x)
    let dHdz = cubic_interp_value(dcol0_dz, dcol1_dz, dcol2_dz, dcol3_dz, frac_x);

    // Convert to normal: height field is y = H(x,z) so normal ~ (-dH/dx, 1, -dH/dz)
    return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// ==============================
// Edge-blend helper
// ==============================
fn chunk_edge_blend_factor(local_x: f32, local_z: f32) -> f32 {
    let tile_x   = floor(local_x);
    let tile_z   = floor(local_z);
    let dist_x   = min(tile_x, f32(CHUNK_TILE_NUM_1D - 1u) - tile_x);
    let dist_z   = min(tile_z, f32(CHUNK_TILE_NUM_1D - 1u) - tile_z);
    let min_dist = min(dist_x, dist_z);
    // Blend stronger within 2 tiles of edge
    return 1.0 - smoothstep(0.0, 2.0, min_dist);
}

// ==============================
// Vertex stage
// ==============================
@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    let grid_w: u32 = HEIGHT_GRID_NUM_1D;

    // Compute grid coords inside the 9x9 array.
    let grid_x: u32 = vertex_index % grid_w;
    let grid_z: u32 = vertex_index / grid_w;

    // --- 1) Displace the Y coordinate using the already-scaled height ---
    var displaced_local_pos = in.position;
    displaced_local_pos.y = land.tiles[vertex_index].tile_height;

    // --- 2) Compute a geometric (hard) normal from neighboring heights (central differences) ---
    // FIX: previous bug used down_z for left/right indexing; must use current grid_z for left/right.
    let left_x  = select(grid_x - 1u, 0u, grid_x == 0u);
    let right_x = min(grid_x + 1u, grid_w - 1u);
    let down_z  = select(grid_z - 1u, 0u, grid_z == 0u);
    let up_z    = min(grid_z + 1u, grid_w - 1u);

    // Correct row used for left/right: use grid_z (current row)
    let h_left  = land.tiles[grid_z * grid_w + left_x ].tile_height;
    let h_right = land.tiles[grid_z * grid_w + right_x].tile_height;
    let h_down  = land.tiles[down_z * grid_w + grid_x ].tile_height;
    let h_up    = land.tiles[up_z   * grid_w + grid_x ].tile_height;

    let dHdx_geo = 0.5 * (h_right - h_left);
    let dHdz_geo = 0.5 * (h_up    - h_down);
    let geometric_normal_local = normalize(vec3<f32>(-dHdx_geo, 1.0, -dHdz_geo));

    // --- 3) Standard Bevy transforms to world space ---
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let geometric_normal_world = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(displaced_local_pos, 1.0));
    out.position       = view_transformations::position_world_to_clip(out.world_position.xyz);
    out.instance_index = in.instance_index;

    // Pass UVs for texture sampling
    out.uv = in.uv;

    // Decide shading mode (uniform or test override)
    var use_vertex_lighting: u32 = select(tunables.use_vertex_lighting,
                                          TEST_TUNABLES_USE_VERTEX_LIGHTING,
                                          USE_TEST_TUNABLES == 1u);

    if (use_vertex_lighting == 1u) {
        // === Gouraud (per-vertex) lighting path ===

        // Optionally compute a smooth bicubic normal (local-space) at the world location.
        var normal_local_for_lighting = geometric_normal_local;
        if (ENABLE_SMOOTH_NORMALS == 1u) {
            // get_bicubic_normal expects world-space x,z in tile units; it returns a LOCAL-space normal.
            normal_local_for_lighting = get_bicubic_normal(out.world_position.x, out.world_position.z);
        }

        // Convert to world-space once and reuse.
        let normal_world_for_lighting = mesh_functions::mesh_normal_local_to_world(normal_local_for_lighting, in.instance_index);

        // Blend toward geometric normal near the chunk edges to hide seams.
        let local_x = out.world_position.x - land.chunk_origin.x;
        let local_z = out.world_position.z - land.chunk_origin.y;
        let blend   = chunk_edge_blend_factor(local_x, local_z);
        let blended_normal_world = normalize(mix(normal_world_for_lighting, geometric_normal_world, blend));

        // Lambert (diffuse) term: how aligned the normal is with the (normalized) light direction.
        // IMPORTANT: scene.light_direction must be normalized on CPU so dot(N, L) ∈ [0,1].
        // Why normalize? The dot product of two unit vectors equals cos(theta), which is
        // physically meaningful diffuse. A non-unit L scales brightness incorrectly.
        let N = blended_normal_world;
        let L = scene.light_direction; // already normalized externally
        let lambert = max(dot(N, L), 0.0);

        // Store per-vertex lighting in uv_b.x for the fragment to reuse.
        out.uv_b = vec2<f32>(lambert, 0.0);
        // Store the world-space normal (for fragment debug or optional usage).
        out.world_normal = blended_normal_world;

    } else {
        // === Per-fragment lighting path ===
        // We pass down the geometric normal in world-space; the fragment will compute smooth normals if needed.
        out.world_normal = geometric_normal_world;
        out.uv_b = vec2<f32>(0.0, 0.0);
    }

    return out;
}

// ==============================
// Fragment stage
// ==============================
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var out: vec4<f32>;

    // -----------------------
    // 1) Per-tile data lookup
    // -----------------------
    // Convert world position to local chunk space (tile units).
    let local_x = in.world_position.x - land.chunk_origin.x;
    let local_z = in.world_position.z - land.chunk_origin.y;

    // Integer tile (0..7) the fragment lies in, used for texture lookup.
    let tile_x_u = clamp(u32(floor(local_x)), 0u, CHUNK_TILE_NUM_1D - 1u);
    let tile_z_u = clamp(u32(floor(local_z)), 0u, CHUNK_TILE_NUM_1D - 1u);

    // IMPORTANT: since the uniform is a 9x9 array, we index rows with stride 9.
    let tile_index_9 = tile_z_u * HEIGHT_GRID_NUM_1D + tile_x_u;

    let tile_height = land.tiles[tile_index_9].tile_height;

    let texture_size  = land.tiles[tile_index_9].texture_size;
    let texture_layer = land.tiles[tile_index_9].texture_layer;

    // Fractional UV inside tile (0..1 each axis).
    let uv_in_tile = vec2<f32>(fract(local_x), fract(local_z));

    // -----------------------
    // 2) Lighting
    // -----------------------
    var base_color: vec3<f32>;
    var base_alpha: f32;
    var lighting_factor: f32;
    var final_normal_world: vec3<f32>;

    // Decide lighting mode (same logic as vertex, plus compile-time override).
    var use_vertex_lighting: u32 = select(tunables.use_vertex_lighting,
                                          TEST_TUNABLES_USE_VERTEX_LIGHTING,
                                          USE_TEST_TUNABLES == 1u);

    // Sample the texture
    var sampled_rgba: vec4<f32>;
    if (texture_size == TEX_SIZE_BIG) {
        sampled_rgba = textureSample(texarray_big, texarray_sampler, uv_in_tile, i32(texture_layer));
    } else {
        sampled_rgba = textureSample(texarray_small, texarray_sampler, uv_in_tile, i32(texture_layer));
    }
    base_color = sampled_rgba.rgb;
    base_alpha = sampled_rgba.a;

    if (use_vertex_lighting == 1u) {
        // Gouraud: vertex shader already computed lambert and blended normals.
        lighting_factor = in.uv_b.x;
        final_normal_world = in.world_normal; // stored by vertex stage
    } else {
        // Per-fragment: compute (optionally smooth) normal here and do Lambert.
        // Note: in.world_normal is in world-space (geometric normal).
        var world_normal_for_lighting = in.world_normal; // world-space geometric normal

        if (ENABLE_SMOOTH_NORMALS == 1u) {
            // get_bicubic_normal returns a LOCAL-space normal; convert to world-space once
            let smooth_local = get_bicubic_normal(in.world_position.x, in.world_position.z);
            let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
            let blend = chunk_edge_blend_factor(local_x, local_z);
            // Blend between the smooth (bicubic) world normal and the geometric world normal
            world_normal_for_lighting = normalize(mix(smooth_world, in.world_normal, blend));
        }
        final_normal_world = normalize(world_normal_for_lighting);

        // Lambertian diffuse. Assumes scene.light_direction is normalized on CPU.
        // WHY NORMALIZE (recap): dot(N, L) with both unit-length equals cos(theta),
        // giving physically meaningful diffuse energy. Non-unit L would scale
        // the result, producing overly bright or dark shading.
        let L = scene.light_direction;
        lighting_factor = max(dot(final_normal_world, L), 0.0);
    }

    // Optional ambient occlusion (very cheap, coarse). If enabled, consider averaging
    // multiple neighbors to avoid directional bias; kept minimal here.
    if (ENABLE_AMBIENT_OCCLUSION == 1u) {
        let neighbor_height_diff = abs(tile_height - tile_height_at_9x9(i32(tile_x_u) + 1, i32(tile_z_u)));
        let occlusion = clamp(1.0 - neighbor_height_diff * 0.1, 0.5, 1.0);
        lighting_factor *= occlusion;
    }

    // Optional slope-based texture blending: keep conservative defaults so peaks aren't pure gray.
    if (ENABLE_SLOPE_BASED_TEXTURING == 1u) {
        let slope = 1.0 - final_normal_world.y; // 0 = flat, 1 = vertical
        let rock_tint = vec3<f32>(0.6, 0.6, 0.6);
        let snow_tint = vec3<f32>(0.95, 0.95, 1.0);
        let altitude = tile_height; // already in world units
        let rock_strength = clamp(slope * 1.25, 0.0, 1.0);
        let snow_base = clamp((altitude - 50.0) / 20.0, 0.0, 1.0); // tweak 50/20 to taste
        let snow_strength = snow_base * (1.0 - slope);
        base_color = mix(base_color, rock_tint, rock_strength);
        base_color = mix(base_color, snow_tint, snow_strength);
    }

    // Optional specular highlights
    if (ENABLE_SPECULAR == 1u) {
        let view_dir = normalize(scene.camera_position - in.world_position.xyz);
        let reflect_dir = reflect(-scene.light_direction, final_normal_world); // L is normalized
        let spec = pow(max(dot(view_dir, reflect_dir), 0.0), 16.0);
        base_color += spec * 0.2;
    }

    // -----------------------
    // 3) Sharpness mix on diffuse term (applied ONCE)
    // -----------------------
    let sharpness_factor     = select(tunables.sharpness_factor,     TEST_TUNABLES_SHARPNESS_FACTOR,     USE_TEST_TUNABLES == 1u);
    let sharpness_mix_factor = select(tunables.sharpness_mix_factor, TEST_TUNABLES_SHARPNESS_MIX_FACTOR, USE_TEST_TUNABLES == 1u);

    // Do NOT multiply base_color by lighting here (avoids double-lighting).
    // Shape the diffuse lobe to taste:
    let lambert_raw    = lighting_factor;
    let lambert_sharp  = pow(max(lambert_raw, 1e-4), sharpness_factor);
    let lambert_shaped = mix(lambert_raw, lambert_sharp, sharpness_mix_factor);

    // Tiny hemispherical ambient to prevent crushed shadows on steep slopes:
    // Up-facing surfaces get a bit more ambient than sideways/down-facing.
    let upness = clamp(final_normal_world.y, 0.0, 1.0);
    let ambient_up   = 0.55;
    let ambient_down = 0.25;
    let hemi_ambient = mix(ambient_down, ambient_up, upness);

    // Final brightness: ambient + shaped diffuse
    let brightness = clamp(hemi_ambient + lambert_shaped * 0.75, 0.0, 1.5);

    // -----------------------
    // 4) Debug modes
    // -----------------------
    if (DEBUG_HEIGHT == 1u) {
        let h = tile_height / 255.0;
        out = vec4<f32>(h, h, h, 1.0);
        return out;
    }
    if (DEBUG_SLOPE == 1u) {
        let slope = 1.0 - final_normal_world.y;
        out = vec4<f32>(slope, slope, slope, 1.0);
        return out;
    }
    if (DEBUG_NORMALS == 1u) {
        let normal_rgb = final_normal_world * 0.5 + vec3<f32>(0.5, 0.5, 0.5);
        return vec4<f32>(normal_rgb, 1.0);
    }
    if (DEBUG_EDGE_BLEND == 1u) {
        let edge_blend = chunk_edge_blend_factor(local_x, local_z);
        let debug_color = mix(vec3<f32>(0.1, 1.0, 0.1), vec3<f32>(1.0, 0.1, 1.0), edge_blend);
        return vec4<f32>(debug_color, 1.0);
    }

    // -----------------------
    // 5) Final color (apply lighting ONCE here)
    // -----------------------
    let lit_rgb = base_color * brightness;
    return vec4<f32>(lit_rgb, base_alpha);
}
