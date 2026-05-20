// ============================================================================
// land::bindings — Uniform struct definitions, GPU resource bindings, and
//                  compile-time constants shared by all land shader modules.
// ============================================================================

#import "shaders/world/common_bindings.wgsl"::{SceneUniform, GlobalLightingUniforms, LandEffectsUniform, USE_VOLUMETRIC_NOISE}

// Virtual metadata record extracted from the tile_meta_atlas.
struct TileUniform {
  tile_height:   f32,
  texture_size:  u32, // 0=cc-small array, 1=cc-big array, 2=ec page atlas, 3=missing, 4=cc page atlas
  texture_layer: u32,
  texture_hue:   u32,
  texture_origin: vec2<u32>,
  texture_extent: vec2<u32>,
  // 1 when the tile has the IsWet tiledata flag set (animated water effect).
  is_wet: u32,
  _pad_tu: u32,
};

// Parameters for the LRU paged tile metadata atlas.
struct AtlasParams {
  page_texels: vec2<u32>,
  tiles_per_page: vec2<u32>,
  max_layers: u32,
  world_pages_x: u32,
  _pad: vec2<u32>,
  page_to_layer: array<vec4<u32>, 64>, // Maps 256 logic pages → physical layer (4 per vec4)
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
    diffuse_wrap:      f32,
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
@group(3) @binding(109) var land_page_atlas: texture_2d_array<f32>;
@group(3) @binding(110) var land_page_lookup: texture_2d<u32>;
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
