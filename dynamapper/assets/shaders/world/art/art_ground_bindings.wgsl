// ============================================================================
// art::ground_bindings — Ground-art specific bindings with the correct instance
//                       storage type for the ground shader.
// ============================================================================

#import "shaders/world/common_bindings.wgsl"::{SceneUniform, GlobalLightingUniforms, LandEffectsUniform}

struct GroundTileInstance {
    world_x: f32,
    world_z: f32,
    world_y: f32,
    layer: u32,
    depth_class: u32,
    base_world_y: f32,
    uv_min: vec2<f32>,
    uv_max: vec2<f32>,
    local_min: vec2<f32>,
    local_max: vec2<f32>,
    tile_x: f32,
    tile_y: f32,
    priority_z_units: f32,
    sort_bias_ordinal: u32,
    is_wet_flags: u32,
    texture_stretch: f32,
    color_rgba: vec4<f32>,
}

struct SpriteParams {
    render_mode: u32,
    alpha_cutoff: f32,
    pass_mode: u32,
    _pad: u32,
    map_width_tiles: f32,
    map_height_tiles: f32,
    _pad_sp: vec2<u32>,
}

@group(3) @binding(100) var art_atlas_sampler: sampler;
@group(3) @binding(101) var art_atlas: texture_2d_array<f32>;
@group(3) @binding(102) var<storage, read> instances: array<GroundTileInstance>;
@group(3) @binding(103) var<uniform> sprite_params: SpriteParams;
@group(3) @binding(105) var<uniform> scene: SceneUniform;
@group(3) @binding(106) var<uniform> effects: LandEffectsUniform;
@group(3) @binding(107) var<uniform> global_light: GlobalLightingUniforms;

const HEIGHT_SCALE: f32 = 0.1;
const DEPTH_CLASS_BACKGROUND: u32 = 1u;
const DEPTH_CLASS_FOLIAGE: u32 = 2u;
const DEPTH_CLASS_ROOF: u32 = 3u;
const DEPTH_CLASS_SURFACE_LIKE_FLOOR: u32 = 4u;
const PASS_MODE_OPAQUE: u32 = 0u;
const PASS_MODE_TRANSPARENT: u32 = 1u;
const SURFACE_LIKE_DEPTH_CLASS_OFFSET: f32 = -4.0;
const STATIC_DEPTH_TIE_BREAK_FRAG_EPSILON: f32 = 0.000001;
