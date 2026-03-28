// ============================================================================
// land::bindings — Uniform struct definitions, GPU resource bindings, and
//                  compile-time constants shared by all land shader modules.
// ============================================================================


// ============================================================================
// Compile-time DEV config (unified DEV_* prefix)
// ============================================================================

const USE_VOLUMETRIC_NOISE: u32 = 1u; // 0=flat fog, 1=domain-warped billow modulation

// ============================================================================
// Struct definitions
// ============================================================================

// Per-tile metadata read from the paged atlas texture.
// R16 channel = texture layer index; G16 = packed [height_biased:low8 | tex_size:high8].
struct TileUniform {
  tile_height:   f32,
  texture_size:  u32, // 0=small atlas, 1=big atlas
  texture_layer: u32,
  texture_hue:   u32,
};

// Parameters for the LRU paged tile metadata atlas.
// Maps logical world pages (up to 256) to physical GPU array layers.
struct AtlasParams {
  page_texels: vec2<u32>,
  tiles_per_page: vec2<u32>,
  max_layers: u32,
  world_pages_x: u32,
  _pad: vec2<u32>,
  page_to_layer: array<vec4<u32>, 64>, // Maps 256 logic pages → physical layer (4 per vec4)
};

// Global scene state: camera, light, zoom level.
struct SceneUniform {
  camera_position: vec3<f32>,
  _pad_cam: f32,
  light_direction: vec3<f32>, // expected normalized by CPU
  // global scene light scaler (pre-tonemap). Default 1.0 from CPU/UI.
  global_lighting: f32,
  // Camera orthographic scale factor.
  render_zoom: f32,
  // 0..1 simplification amount used for zoomed-out checkerboard shading.
  adaptive_zoom_simplification: f32,
  _pad0_: vec2<f32>,
};

// Visual feature toggles and intensity parameters.
// Modes / toggles go in the first two vec4 slots for std140 alignment.
struct LandEffectsUniform {
  // Modes & texture blur toggle (vec4 slot 0)
  shading_mode:   u32, // 0=Classic (vertex), 1=Enhanced (frag), 2=KR (frag)
  normal_mode:    u32, // 0=geometric, 1=bicubic
  enable_blur:    u32, // optional pre-shade blur of base albedo
  enable_linear_filtering: u32,

  // Graphics / texture reconstruction (vec4 slot 1)
  reconstruction_mode:     u32,
  sharpening_amount:       f32,
  blur_strength:           f32,
  blur_radius:             f32,
};

// Global lighting parameters shared across all shader types (land, art tiles, etc.).
// Art tiles are 2D sprites — no 3D mesh — so they don't need view/normal–dependent
// lighting, but they share grading, fog, gloom, tonemapping, and base light colors.
struct GlobalLightingUniforms {
    // Toggles (vec4 slot 0)
    enable_fog:      u32,
    enable_tonemap:  u32,
    enable_grading:  u32,
    enable_gloom:    u32,

    // Colors
    light_color: vec3<f32>,
    _pad0_: f32,
    ambient_color: vec3<f32>,
    _pad1_: f32,

    // Exposure / gamma / ambient
    exposure: f32,
    gamma: f32,
    ambient_strength: f32,
    _pad2_: f32,

    // Grading
    grade_warm_color: vec4<f32>,
    grade_cool_color: vec4<f32>,
    // grade_params: [strength, headroom_reserve, chroma_tint, headroom_on]
    grade_params: vec4<f32>,
    // grade_extra: [vibrance, saturation, contrast, split_strength]
    grade_extra: vec4<f32>,

    // Gloom
    // gloom_params: [amount, falloff_height, shadow_bias, fog_height_bias]
    gloom_params: vec4<f32>,

    // Fog
    // fog_color: [r, g, b, max_mix]
    fog_color: vec4<f32>,
    // fog_params: [distance_density, height_density, noise_scale, noise_strength]
    fog_params: vec4<f32>,
};

// Land-specific lighting: parameters that require a 3D mesh with surface
// normals and a view vector (not applicable to flat art tiles).
struct LandLightingUniforms {
    // Toggle
    enable_bent:    u32,
    _pad0_:         u32,
    _pad1_:         u32,
    _pad2_:         u32,

    // Intensities
    diffuse_strength:  f32,
    specular_strength: f32,
    rim_strength:      f32,
    fill_strength:     f32,

    sharpness_factor:  f32,
    sharpness_mix:     f32,
    _pad3_:            f32,
    _pad4_:            f32,

    // Hemisphere fill & rim colors (need N / V)
    fill_sky_color: vec4<f32>,
    fill_ground_color: vec4<f32>,
    rim_color: vec4<f32>,
};

// ============================================================================
// GPU resource bindings (group 3, matching Rust #[uniform(10X)] indices)
// ============================================================================

@group(3) @binding(100) var tex_small_sampler: sampler;
@group(3) @binding(101) var tex_small: texture_2d_array<f32>;
@group(3) @binding(102) var tex_big:   texture_2d_array<f32>;
@group(3) @binding(103) var tile_meta_atlas: texture_2d_array<u32>;
@group(3) @binding(104) var<uniform> ATLAS:        AtlasParams;
@group(3) @binding(105) var<uniform> scene:        SceneUniform;
@group(3) @binding(106) var<uniform> effects:      LandEffectsUniform;
@group(3) @binding(107) var<uniform> global_light:  GlobalLightingUniforms;
@group(3) @binding(108) var<uniform> land_light:   LandLightingUniforms;

// ============================================================================
// Grid / chunk constants
// ============================================================================

const CHUNK_TILE_NUM_DIM: u32 = 8u;
const DATA_GRID_BORDER:   i32 = 2;
const DATA_GRID_SIDE:     i32 = 13;  // DATA_GRID_BORDER + CHUNK_TILE_NUM_DIM + DATA_GRID_BORDER
const MESH_GRID_SIDE:     u32 = 9u;
