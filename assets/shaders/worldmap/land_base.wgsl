// land_fixed.wgsl
// Full shader: runtime shading mode, bicubic normals, optional bent normals,
// hemisphere fill, rim, specular, multiplicative KR-style clouds/fog,
// painterly grading + Reinhard tonemapping.
// Designed for Bevy/WGPU (wgsl), with runtime tunables and hot-reload constants.
// ============================================================================

// Bevy PBR imports
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    view_transformations,
}

// ============================================================================
// HOT-RELOAD / COMPILE-TIME QUICK OVERRIDES
// Set these to 1 to override runtime uniforms quickly during development.
// Useful for hot-reloading shader tweaks without changing CPU code.
const HOT_OVERRIDE_USE_UNIFORMS:    u32 = 1u; // 1 = use tunables from CPU; 0 = use HOT_* constants below
const HOT_SHADING_MODE:             u32 = 2u; // 0 = Classic(Gouraud), 1 = Fragment, 2 = Fragment (KR-style default)
const HOT_NORMAL_MODE:              u32 = 1u; // 0 = geometric, 1 = bicubic
const HOT_ENABLE_BENT:              u32 = 1u;
const HOT_ENABLE_TONEMAP:           u32 = 1u;
const HOT_ENABLE_GRADING:           u32 = 1u;
const HOT_ENABLE_FOG:               u32 = 1u;

// Override intensity defaults for quick testing (used only when HOT_OVERRIDE_USE_UNIFORMS == 0)
const HOT_AMBIENT:      f32 = 0.40;
const HOT_DIFFUSE:      f32 = 1.00;
const HOT_SPECULAR:     f32 = 0.12;
const HOT_RIM:          f32 = 0.25;
const HOT_FILL:         f32 = 0.45;
const HOT_EXPOSURE:     f32 = 1.0;

// TODO: harmonize the override settings names. also, normalize is NOT a constant function, we can't use it like this!
// Main light
const OVERRIDE_LIGHT_DIR: vec3<f32>   = normalize(vec3<f32>(-0.5, -1.0, -0.3));
const OVERRIDE_LIGHT_COLOR: vec3<f32> = vec3<f32>(1.0, 0.95, 0.85);

// Ambient/fill
const OVERRIDE_AMBIENT_COLOR: vec3<f32> = vec3<f32>(0.25, 0.28, 0.32);
const OVERRIDE_FILL_DIR: vec3<f32>      = normalize(vec3<f32>(0.3, 1.0, 0.2));
const OVERRIDE_FILL_STRENGTH: f32       = 0.35;

// Tone mapping
const OVERRIDE_EXPOSURE: f32 = 1.0;
const OVERRIDE_GAMMA: f32    = 2.2;

// ============================================================================
// BINDINGS / UNIFORMS
// Keep these indices in sync with your Rust Material's AsBindGroup ordering.
// ============================================================================

struct TileUniform {
    tile_height:   f32, // scaled to world units on CPU
    texture_size:  u32,
    texture_layer: u32,
    texture_hue:   u32,
};

struct LandUniform {
    light_dir:    vec3<f32>, // legacy slot; prefer scene.light_direction
    _pad0:        f32,
    chunk_origin: vec2<f32>, // world origin of chunk in tile units (x, z)
    _pad1:        vec2<f32>,
    tiles:        array<TileUniform, 169>, // 13x13 height/info grid
};

struct SceneUniform {
    camera_position: vec3<f32>,
    light_direction: vec3<f32>, // MUST be normalized on CPU for best perf and correctness
    _pad: f32,
};

// Tunables: runtime-uniform controls for lighting & effects (change from Rust)
struct TunablesUniform {
    // Modes
    shading_mode:    u32, // 0 = Gouraud (classic), 1 = Per-fragment
    normal_mode:     u32, // 0 = geometric, 1 = bicubic
    enable_bent:     u32, // tilt normals toward sky in concavities
    enable_tonemap:  u32,
    enable_grading:  u32,
    enable_fog:      u32,

    // Intensities (grouped to keep alignment)
    ambient_strength: f32,
    diffuse_strength: f32,
    specular_strength:f32,
    rim_strength:     f32,

    fill_strength:    f32,
    exposure:         f32,
    sharpness_factor: f32,
    sharpness_mix:    f32,

    _pad: f32,
};

struct VisualUniform {
    fog_color: vec4<f32>,    // rgb=color, a=opacity
    fog_params: vec4<f32>,   // x=strength, y=scale, z=speed_x, w=speed_y
    fill_sky_color: vec4<f32>,   // rgb + a = strength
    fill_ground_color: vec4<f32>,// rgb + a = strength
    rim_color: vec4<f32>,        // rgb + a = rim_power
    grade_warm_color: vec4<f32>, // rgb + a unused
    grade_cool_color: vec4<f32>, // rgb + a unused
    grade_params: vec4<f32>,     // x = grade_strength
    // time for animated clouds (seconds)
    time_seconds: f32,
    _pad: vec3<f32>,
};

// TODO: some of those were extracted from the tunablesuniform, we have to remove the duplicate ones from the
//  origin struct. Actually, maybe we can divide the structs by naming them based on separation of concerns and code
//  re-usability. Some of the settings are relative to lighting and color correction (to be applied to every game object), others
//  are relevant only for map chunks, others are relative only to the scene (like the fog, which is an overlay on top of everything else)
struct LightingUniforms {
    light_dir: vec3<f32>,      // Main directional light direction
    light_color: vec3<f32>,    // Main light color (linear HDR, not gamma)
    ambient_color: vec3<f32>,  // Ambient term (fill light substitute)
    fill_dir: vec3<f32>,       // Secondary fill light direction
    fill_strength: f32,        // How strong the fill is
    exposure: f32,             // Exposure bias for tonemapping
    gamma: f32,                // Gamma correction
};

@group(2) @binding(100) var texarray_sampler: sampler;
@group(2) @binding(101) var texarray_small:   texture_2d_array<f32>;
@group(2) @binding(102) var texarray_big:     texture_2d_array<f32>;
@group(2) @binding(103) var<uniform> land:    LandUniform;
@group(2) @binding(104) var<uniform> scene:   SceneUniform;
@group(2) @binding(105) var<uniform> tunables: TunablesUniform;
@group(2) @binding(106) var<uniform> visual:  VisualUniform;
@group(2) @binding(107) var<uniform> lighting: LightingUniforms;


// ============================================================================
// CONSTANTS & GRID HELPERS
// ============================================================================
const CHUNK_TILE_NUM_1D: u32 = 8u;
const DATA_GRID_BORDER: i32 = 2;     // margin radius around 8x8 core -> 13x13
const DATA_GRID_SIDE: i32 = 13;
const MESH_GRID_SIDE: u32 = 9u;      // vertex nodes: 9x9

// Safe fetch (maps relative ix/iz -> 0..12)
fn tile_index_clamped(ix: i32, iz: i32) -> u32 {
    let gx = clamp(ix + DATA_GRID_BORDER, 0, DATA_GRID_SIDE - 1);
    let gz = clamp(iz + DATA_GRID_BORDER, 0, DATA_GRID_SIDE - 1);
    return u32(gz * DATA_GRID_SIDE + gx);
}

fn tile_at_13x13(ix: i32, iz: i32) -> TileUniform {
    return land.tiles[tile_index_clamped(ix, iz)];
}
fn tile_height_at_13x13(ix: i32, iz: i32) -> f32 {
    return tile_at_13x13(ix, iz).tile_height;
}

// Edge-blend factor: 0 in interior, ramps toward 1 near chunk boundary to soften seams.
fn chunk_edge_blend_factor(local_x: f32, local_z: f32) -> f32 {
    let tx = floor(local_x);
    let tz = floor(local_z);
    let dx = min(tx, f32(CHUNK_TILE_NUM_1D - 1u) - tx);
    let dz = min(tz, f32(CHUNK_TILE_NUM_1D - 1u) - tz);
    let min_dist = min(dx, dz);
    // blend starts at distance 2 tiles
    return 1.0 - smoothstep(0.0, 2.0, min_dist);
}

// Cheap 2D value-noise for animated clouds/fog mask (fast, low-quality but good for stylistic use).
fn hash(p: vec2<f32>) -> f32 {
    let p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    let p3_shifted = p3 + dot(p3, p3.yzx + vec3<f32>(19.19));
    return fract((p3_shifted.x + p3_shifted.y) * p3_shifted.z);
}
fn noise_2d(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i + vec2<f32>(0.0, 0.0));
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// ============================================================================
// CUBIC INTERP (value + analytic derivative)
// We compute both value and d/dt to generate accurate dH/dx, dH/dz for bicubic normals.
// t is a local fractional coordinate in [0,1] along one axis (x or z).
// ============================================================================
fn cubic_interp_value_and_derivative(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> vec2<f32> {
    let a = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
    let b =       p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c = -0.5 * p0           + 0.5 * p2;
    let d =           p1;
    let value = ((a * t + b) * t + c) * t + d;
    let deriv = (3.0 * a * t * t) + (2.0 * b * t) + c;
    return vec2<f32>(value, deriv);
}
fn cubic_value(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return cubic_interp_value_and_derivative(p0,p1,p2,p3,t).x;
}
fn cubic_deriv(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    return cubic_interp_value_and_derivative(p0,p1,p2,p3,t).y;
}

// ============================================================================
// NORMALS
// Bicubic analytic normal from 4x4 patch of heights stored in the 13x13 grid.
// ============================================================================

fn get_bicubic_normal(world_pos: vec3<f32>) -> vec3<f32> {
    // local coords in tile units relative to chunk origin (x,z)
    let local_x = world_pos.x - land.chunk_origin.x;
    let local_z = world_pos.z - land.chunk_origin.y;

    let base_x = floor(local_x);
    let base_z = floor(local_z);
    let frac_x = local_x - base_x;
    let frac_z = local_z - base_z;

    let ix = i32(base_x);
    let iz = i32(base_z);

    // fetch 4x4 patch
    let h00 = tile_height_at_13x13(ix - 1, iz - 1);
    let h10 = tile_height_at_13x13(ix + 0, iz - 1);
    let h20 = tile_height_at_13x13(ix + 1, iz - 1);
    let h30 = tile_height_at_13x13(ix + 2, iz - 1);

    let h01 = tile_height_at_13x13(ix - 1, iz + 0);
    let h11 = tile_height_at_13x13(ix + 0, iz + 0);
    let h21 = tile_height_at_13x13(ix + 1, iz + 0);
    let h31 = tile_height_at_13x13(ix + 2, iz + 0);

    let h02 = tile_height_at_13x13(ix - 1, iz + 1);
    let h12 = tile_height_at_13x13(ix + 0, iz + 1);
    let h22 = tile_height_at_13x13(ix + 1, iz + 1);
    let h32 = tile_height_at_13x13(ix + 2, iz + 1);

    let h03 = tile_height_at_13x13(ix - 1, iz + 2);
    let h13 = tile_height_at_13x13(ix + 0, iz + 2);
    let h23 = tile_height_at_13x13(ix + 1, iz + 2);
    let h33 = tile_height_at_13x13(ix + 2, iz + 2);

    // Interpolate rows (x) -> values and d/dx
    let row0 = cubic_interp_value_and_derivative(h00, h10, h20, h30, frac_x);
    let row1 = cubic_interp_value_and_derivative(h01, h11, h21, h31, frac_x);
    let row2 = cubic_interp_value_and_derivative(h02, h12, h22, h32, frac_x);
    let row3 = cubic_interp_value_and_derivative(h03, h13, h23, h33, frac_x);

    // dH/dx = cubic interp of the row derivatives (interpolated along z)
    let dHdx = cubic_value(row0.y, row1.y, row2.y, row3.y, frac_z);

    // dH/dz: interpolate columns
    let col0 = cubic_interp_value_and_derivative(h00, h01, h02, h03, frac_z);
    let col1 = cubic_interp_value_and_derivative(h10, h11, h12, h13, frac_z);
    let col2 = cubic_interp_value_and_derivative(h20, h21, h22, h23, frac_z);
    let col3 = cubic_interp_value_and_derivative(h30, h31, h32, h33, frac_z);
    let dHdz = cubic_value(col0.y, col1.y, col2.y, col3.y, frac_x);

    // Heightfield normal
    return normalize(vec3<f32>(-dHdx, 1.0, -dHdz));
}

// Cheap bent-normal approximation (very conservative)
fn get_bent_normal(world_pos: vec3<f32>, base_normal_world: vec3<f32>) -> vec3<f32> {
    // quick center cell & 4-neighbors occlusion heuristic
    let local_x = world_pos.x - land.chunk_origin.x;
    let local_z = world_pos.z - land.chunk_origin.y;
    let cx = i32(floor(local_x));
    let cz = i32(floor(local_z));

    let hc = tile_height_at_13x13(cx, cz);
    let hl = tile_height_at_13x13(cx - 1, cz);
    let hr = tile_height_at_13x13(cx + 1, cz);
    let hd = tile_height_at_13x13(cx, cz - 1);
    let hu = tile_height_at_13x13(cx, cz + 1);

    let pos_slopes = max(0.0, hl - hc) + max(0.0, hr - hc) + max(0.0, hd - hc) + max(0.0, hu - hc);
    let occl = clamp(pos_slopes * 0.25, 0.0, 1.0);

    let mix_factor = occl * 0.5; // conservative bend toward up
    return normalize(mix(base_normal_world, vec3<f32>(0.0, 1.0, 0.0), mix_factor));
}

// ============================================================================
// LIGHTING HELPERS
// All helpers assume N,L,V are in world-space; L is expected normalized by CPU.
// ============================================================================

// Lambert (expects unit N & L). We do NOT normalize L here if HOT_OVERRIDE_USE_UNIFORMS==1
fn get_lambert(N: vec3<f32>, L: vec3<f32>) -> f32 {
    // Defensive normalization of N only (cheap). L assumed normalized on CPU for perf.
    return max(dot(normalize(N), L), 0.0);
}

// Blinn-Phong specular intensity. Return scalar.
fn get_specular(N: vec3<f32>, L: vec3<f32>, V: vec3<f32>, shininess: f32) -> f32 {
    let H = normalize(L + V);
    return pow(max(dot(normalize(N), H), 0.0), shininess);
}

// Rim term (grazing)
fn get_rim(N: vec3<f32>, V: vec3<f32>, power: f32) -> f32 {
    let rim_dot = 1.0 - max(dot(normalize(N), normalize(V)), 0.0);
    return pow(rim_dot, power);
}

// Hemisphere fill: returns (ambient_scalar, upness) as vec2
fn get_hemisphere_fill(N: vec3<f32>) -> vec2<f32> {
    let upness = clamp(dot(normalize(N), vec3<f32>(0.0,1.0,0.0)) * 0.5 + 0.5, 0.0, 1.0);
    // ambient strength will be applied by tunables.fill_strength * returned.x
    let sky_strength = visual.fill_sky_color.a;
    let ground_strength = visual.fill_ground_color.a;
    let ambient_strength = mix(ground_strength, sky_strength, upness);
    return vec2<f32>(ambient_strength, upness);
}

// Reinhard tonemap with exposure control: color * exposure -> color/(1+color)
fn tonemap_reinhard_with_exposure(c: vec3<f32>, exposure: f32) -> vec3<f32> {
    let e = max(exposure, 1e-6);
    let mapped = (c * e) / (vec3<f32>(1.0) + (c * e));
    return mapped;
}

// Painterly grading: warm midtones + cool shadows. Strength in visual.grade_params.x
fn grade_color(color: vec3<f32>) -> vec3<f32> {
    let luma = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let warm_mix = smoothstep(0.0, 1.0, luma);
    let cool_mix = 1.0 - warm_mix;
    let strength = visual.grade_params.x;
    let tint = visual.grade_warm_color.rgb * warm_mix + visual.grade_cool_color.rgb * cool_mix;
    // modest influence to avoid overpowering
    return mix(color, color + tint * 0.25 * strength, strength);
}

// ============================================================================
// VERTEX SHADER
// - Displace vertex Y using pre-packed heights
// - Compute geometric normal and (if in Gouraud) compute per-vertex lighting
// ============================================================================
@vertex
fn vertex(in: Vertex, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    // Resolve runtime vs hot overrides
    var shading_mode: u32 = tunables.shading_mode;
    var normal_mode:  u32 = tunables.normal_mode;
    var enable_bent:  u32 = tunables.enable_bent;
    var enable_tonemap: u32 = tunables.enable_tonemap;
    var enable_grading: u32 = tunables.enable_grading;
    var enable_fog:   u32 = tunables.enable_fog;

    if (HOT_OVERRIDE_USE_UNIFORMS == 0u) {
        shading_mode = HOT_SHADING_MODE;
        normal_mode  = HOT_NORMAL_MODE;
        enable_bent  = HOT_ENABLE_BENT;
        enable_tonemap = HOT_ENABLE_TONEMAP;
        enable_grading = HOT_ENABLE_GRADING;
        enable_fog = HOT_ENABLE_FOG;
    }

    // Compute mesh grid coords (node coords 0..8)
    let grid_x: u32 = vertex_index % MESH_GRID_SIDE;
    let grid_z: u32 = vertex_index / MESH_GRID_SIDE;

    // Map node coords into 13x13 data index (add border)
    let arr_x = i32(grid_x) + DATA_GRID_BORDER;
    let arr_z = i32(grid_z) + DATA_GRID_BORDER;
    let data_idx = u32(arr_z) * u32(DATA_GRID_SIDE) + u32(arr_x);

    // Displace Y using prepacked height (tile node)
    var displaced_local_pos = in.position;
    displaced_local_pos.y = land.tiles[data_idx].tile_height;

    // Geometric normal via central differences on node grid:
    let h_left  = tile_height_at_13x13(i32(grid_x) - 1, i32(grid_z));
    let h_right = tile_height_at_13x13(i32(grid_x) + 1, i32(grid_z));
    let h_down  = tile_height_at_13x13(i32(grid_x), i32(grid_z) - 1);
    let h_up    = tile_height_at_13x13(i32(grid_x), i32(grid_z) + 1);
    let dHdx_geo = 0.5 * (h_right - h_left);
    let dHdz_geo = 0.5 * (h_up    - h_down);
    let geometric_normal_local = normalize(vec3<f32>(-dHdx_geo, 1.0, -dHdz_geo));

    // Transform to world space and clip
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(displaced_local_pos, 1.0));
    out.position = view_transformations::position_world_to_clip(out.world_position.xyz);

    // Pass basic attributes
    out.uv = in.uv;
    out.instance_index = in.instance_index;
    out.world_normal = mesh_functions::mesh_normal_local_to_world(geometric_normal_local, in.instance_index);

    // Clear uv_b (we use uv_b.x to carry per-vertex lambert when Gouraud)
    out.uv_b = vec2<f32>(0.0, 0.0);

    // Gouraud: compute per-vertex lighting (geometric normal only -> stable classic look)
    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && shading_mode == 0u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_SHADING_MODE == 0u) ) {
        // Use geometric local normal (not bicubic) for classic look
        let normal_local = geometric_normal_local;
        let normal_world_for_lighting = mesh_functions::mesh_normal_local_to_world(normal_local, in.instance_index);

        // Simple lambert (assume scene.light_direction is normalized on CPU)
        let L = scene.light_direction; // no normalize for perf; CPU must supply unit-length
        let lam = get_lambert(normal_world_for_lighting, L);

        // Simple ambient scalar for classic: use either uniform or hot-constant
        var ambient_s = tunables.ambient_strength;
        if (HOT_OVERRIDE_USE_UNIFORMS == 0u) { ambient_s = HOT_AMBIENT; }
        // Compose brightness (follow earlier classic mix)
        // diffuse strength multiplied in fragment; here we just pass lambert
        out.uv_b.x = lam;
    }

    return out;
}

// ============================================================================
// FRAGMENT SHADER
// - sample albedo, compute chosen normal (geometric/bicubic + bent),
// - compose lighting (diffuse + ambient + rim + spec) with tunable intensities,
// - apply multiplicative animated cloud/fog (KR style), grading + tonemap.
// ============================================================================
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Resolve runtime vs hot-overrides (fragment side)
    var shading_mode: u32 = tunables.shading_mode;
    var normal_mode:  u32 = tunables.normal_mode;
    var enable_bent:  u32 = tunables.enable_bent;
    var enable_tonemap: u32 = tunables.enable_tonemap;
    var enable_grading: u32 = tunables.enable_grading;
    var enable_fog:   u32 = tunables.enable_fog;

    var ambient_strength: f32 = tunables.ambient_strength;
    var diffuse_strength: f32 = tunables.diffuse_strength;
    var specular_strength: f32 = tunables.specular_strength;
    var rim_strength: f32 = tunables.rim_strength;
    var fill_strength: f32 = tunables.fill_strength;
    var exposure: f32 = tunables.exposure;
    var sharpness_factor: f32 = tunables.sharpness_factor;
    var sharpness_mix: f32 = tunables.sharpness_mix;

    if (HOT_OVERRIDE_USE_UNIFORMS == 0u) {
        shading_mode = HOT_SHADING_MODE;
        normal_mode  = HOT_NORMAL_MODE;
        enable_bent  = HOT_ENABLE_BENT;
        enable_tonemap = HOT_ENABLE_TONEMAP;
        enable_grading = HOT_ENABLE_GRADING;
        enable_fog = HOT_ENABLE_FOG;
        ambient_strength = HOT_AMBIENT;
        diffuse_strength = HOT_DIFFUSE;
        specular_strength = HOT_SPECULAR;
        rim_strength = HOT_RIM;
        fill_strength = HOT_FILL;
        exposure = HOT_EXPOSURE;
        // sharpen params default to 0 if not passed by CPU
    }

    // --- 1) Lookup tile & albedo ---
    // Note: horizontal plane is X,Z in world space. Use in.world_position.x and .z.
    let local_x = in.world_position.x - land.chunk_origin.x;
    let local_z = in.world_position.z - land.chunk_origin.y;

    let tile_x_i = i32(floor(local_x));
    let tile_z_i = i32(floor(local_z));

    // uv within tile (0..1)
    let uv_in_tile = vec2<f32>(fract(local_x), fract(local_z));
    let tile = tile_at_13x13(tile_x_i, tile_z_i);

    var base_rgba: vec4<f32>;
    if (tile.texture_size == 1u) {
        base_rgba = textureSample(texarray_big, texarray_sampler, uv_in_tile, i32(tile.texture_layer));
    } else {
        base_rgba = textureSample(texarray_small, texarray_sampler, uv_in_tile, i32(tile.texture_layer));
    }

    let base_albedo = base_rgba.rgb;
    let base_alpha  = base_rgba.a;

    // --- 2) Select normals (world-space)
    var final_normal_world = in.world_normal; // geometric world normal from vertex stage

    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && normal_mode == 1u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_NORMAL_MODE == 1u) ) {
        // bicubic normal computed in local-space (from heights) -> transform to world
        let smooth_local = get_bicubic_normal(in.world_position.xyz);
        let smooth_world = mesh_functions::mesh_normal_local_to_world(smooth_local, in.instance_index);
        // edge blend to hide seams
        let blend_edge = chunk_edge_blend_factor(local_x, local_z);
        final_normal_world = normalize(mix(smooth_world, in.world_normal, blend_edge));
    } else {
        // ensure normalized
        final_normal_world = normalize(in.world_normal);
    }

    // optional bent normal (biases normal toward sky in concavities)
    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && tunables.enable_bent == 1u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_ENABLE_BENT == 1u) ) {
        final_normal_world = get_bent_normal(in.world_position.xyz, final_normal_world);
    }

    // --- 3) Lighting composition ---
    // Use L as provided by CPU. We assume CPU normalizes scene.light_direction for perf.
    let L = scene.light_direction; // DO NOT normalize here for perf; CPU must supply unit vector.
    let V = normalize(scene.camera_position - in.world_position.xyz);

    var hdr_rgb = vec3<f32>(0.0, 0.0, 0.0);

    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && shading_mode == 0u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_SHADING_MODE == 0u) ) {
        // Gouraud classic: use precomputed lambert (interpolated) + uniform ambient
        let lam_v = in.uv_b.x; // per-vertex Lambert [0..1]
        // compose brightness: diffuse_strength*lambert + ambient_strength
        let brightness = lam_v * diffuse_strength + ambient_strength;
        hdr_rgb = base_albedo * brightness;
        // No rim/specular in classic Gouraud path (they look wrong when interpolated)
    } else {
        // Per-fragment path
        let lam = get_lambert(final_normal_world, L);

        // shape diffuse with tunable sharpness
        let lam_sharp = pow(max(lam, 1e-4), max(0.0001, sharpness_factor));
        let lam_shaped = mix(lam, lam_sharp, sharpness_mix);
        let diffuse_term = diffuse_strength * lam_shaped;

        // hemisphere fill (color tint & scalar)
        var hemi_scalar: f32 = 0.0;
        var hemi_upness: f32 = 0.0;
        if (fill_strength > 0.0) {
            let hemi = get_hemisphere_fill(final_normal_world);
            hemi_scalar = hemi.x * fill_strength; // tune by global fill_strength
            hemi_upness = hemi.y;
        }

        // combine base: base_color * (diffuse + ambient)
        // ambient_strength is a uniform scalar preserving classic behavior
        let ambient_term = ambient_strength;
        hdr_rgb = base_albedo * (diffuse_term + ambient_term * 0.5 + hemi_scalar * 0.5);

        // rim (view-dependent)
        if (rim_strength > 0.001) {
            let rim_power = max(0.1, visual.rim_color.a);
            let rim_val = get_rim(final_normal_world, V, rim_power);
            // apply rim color and scale conservatively
            hdr_rgb += visual.rim_color.rgb * rim_val * rim_strength * 0.5;
        }

        // subtle specular
        if (specular_strength > 0.0001) {
            let spec_val = get_specular(final_normal_world, L, V, 32.0);
            hdr_rgb += vec3<f32>(1.0, 1.0, 1.0) * spec_val * specular_strength;
        }
    }

    // --- 4) KR-style multiplicative clouds/fog (animated)
    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && tunables.enable_fog == 1u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_ENABLE_FOG == 1u) ) {
        // fog_params: x=strength, y=scale, z=speed_x, w=speed_y
        let world_uv = in.world_position.xz * visual.fog_params.y;
        let time_off = visual.fog_params.zw * visual.time_seconds;
        let cloud_uv = world_uv + time_off;
        // bias noise into 0..1
        let mask = noise_2d(cloud_uv * 0.25) * 0.5 + 0.5;
        let fog_strength = clamp(visual.fog_params.x, 0.0, 1.0);
        let fog_opacity = visual.fog_color.a;
        let fog_mask = clamp(mask * fog_opacity * fog_strength, 0.0, 1.0);
        // multiplicative darkening (subtle)
        let tint = visual.fog_color.rgb * (fog_mask * 0.25);
        hdr_rgb = hdr_rgb * (1.0 - fog_mask * 0.6) + hdr_rgb * tint;
    }

    // --- 5) Grading + Tonemap (HDR workflow)
    var post = hdr_rgb;
    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && enable_grading == 1u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_ENABLE_GRADING == 1u) ) {
        post = grade_color(post);
    }

    var final_rgb = post;
    if ( (HOT_OVERRIDE_USE_UNIFORMS == 1u && enable_tonemap == 1u) ||
         (HOT_OVERRIDE_USE_UNIFORMS == 0u && HOT_ENABLE_TONEMAP == 1u) ) {
        final_rgb = tonemap_reinhard_with_exposure(post, exposure);
    }

    final_rgb = max(final_rgb, vec3<f32>(0.0)); // avoid negative values

    return vec4<f32>(final_rgb, base_alpha);
}
