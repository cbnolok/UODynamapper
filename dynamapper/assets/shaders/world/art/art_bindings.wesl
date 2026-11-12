// ============================================================================
// art::bindings — Uniform struct definitions, GPU resource bindings, and
//                compile-time constants shared by all art shader modules.
// ============================================================================

import package::shaders::world::common_bindings::{SceneUniform, GlobalLightingUniforms, LandEffectsUniform};
import package::shaders::world::uo_coords::UO_Z_TO_WORLD_SCALE;

struct SpriteInstance {
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
    // Tiledata flags: bit 0 = is_wet (animated water UV distortion).
    // Bit 1 = static/map surface blend; bits 2..9 = NW,N,NE,W,E,SW,S,SE neighbors.
    // Bit 10 = the KR client grunges the static (STATIC_FLAG_CLIENT_GRUNGE, client_grunge.wesl).
    is_wet_flags: u32,
    hue_id: u32,
    hue_flags: u32,
    // Flat-quad flag: 1 = lay flat in the XZ plane (CC surface-like water statics),
    // 2 = axis-aligned EC surface, 3 = authored EC camera quad, 0 = billboard.
    // Branched on in main.wesl vertex.
    flat_quad: u32,
    texture_stretch: f32,
    material_item_id: u32,
    // The KR client's static shadow (client_shadow.wesl): CLIENT_SHADOW_FLAG_* bits, and the caster's height in UO
    // z units above the ground or surface below it.
    client_shadow_flags: u32,
    client_shadow_height: f32,
    // The KR client's base point on the ground, world x and z, for the client grunge (client_grunge.wesl).
    client_grunge_anchor: vec2<f32>,
    // The static local lights at the quad's corners (unit quad corner (x, z) at index x + 2 z), RGBA8 packed for
    // unpack4x8unorm; the vertex shader picks its corner's and the rasterizer interpolates them.
    local_light_corners: array<u32, 4>,
    color_rgba: vec4<f32>,
}

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
    // Tiledata flags: bit 0 = is_wet (animated water UV distortion).
    is_wet_flags: u32,
    texture_stretch: f32,
    hue_id: u32,
    hue_flags: u32,
    _pad_hue: vec2<u32>,
    // The static local lights at the quad's corners (unit quad corner (x, z) at index x + 2 z), RGBA8 packed for
    // unpack4x8unorm; the vertex shader picks its corner's and the rasterizer interpolates them.
    local_light_corners: array<u32, 4>,
    color_rgba: vec4<f32>,
}

struct SpriteParams {
    render_mode: u32,
    alpha_cutoff: f32,
    pass_mode: u32,
    hue_enabled: u32,
    map_width_tiles: f32,
    map_height_tiles: f32,
    _pad_sp: vec2<u32>,
}

@group(3) @binding(100) var art_atlas_sampler: sampler;
@group(3) @binding(101) var art_atlas: texture_2d_array<f32>;
@group(3) @binding(102) var<storage, read> instances: array<SpriteInstance>;
@group(3) @binding(103) var<uniform> sprite_params: SpriteParams;
@group(3) @binding(105) var<uniform> scene: SceneUniform;
@group(3) @binding(106) var<uniform> effects: LandEffectsUniform;
@group(3) @binding(107) var<uniform> global_light: GlobalLightingUniforms;
@group(3) @binding(108) var hue_sampler: sampler;
@group(3) @binding(109) var hue_texture: texture_2d<f32>;
@group(3) @binding(110) var visual_grunge_texture: texture_2d<f32>;
@group(3) @binding(111) var item_surface_atlas: texture_2d_array<f32>;
@group(3) @binding(112) var item_surface_lookup: texture_2d<u32>;
@group(3) @binding(113) var item_surface_sampler: sampler;
@group(3) @binding(114) var client_grunge_texture: texture_2d<f32>;
@group(3) @binding(115) var client_grunge_sampler: sampler;

// Constants
const INV_SQRT_2: f32 = 0.70710678118;
const BILLBOARD_RIGHT_XZ: vec2<f32> = vec2<f32>(INV_SQRT_2, -INV_SQRT_2);
// In-plane ground direction perpendicular to BILLBOARD_RIGHT_XZ, pointing down-screen
// (increasing camera depth). Together with BILLBOARD_RIGHT_XZ it forms the diagonal basis that
// lays a CC surface-like water static flat as an on-screen diamond matching the projected
// terrain cell, undoing the isometric diamond baked into the sprite's pixels (see main.wesl).
const FLAT_QUAD_FORWARD_XZ: vec2<f32> = vec2<f32>(INV_SQRT_2, INV_SQRT_2);
const HEIGHT_SCALE: f32 = UO_Z_TO_WORLD_SCALE;
const DEPTH_CLASS_REGULAR: u32 = 0u;
const DEPTH_CLASS_BACKGROUND: u32 = 1u;
const DEPTH_CLASS_FOLIAGE: u32 = 2u;
const DEPTH_CLASS_ROOF: u32 = 3u;
const DEPTH_CLASS_SURFACE_LIKE_FLOOR: u32 = 4u;
const PASS_MODE_OPAQUE: u32 = 0u;
const PASS_MODE_TRANSPARENT: u32 = 1u;
const PASS_MODE_SHADOW: u32 = 2u;
// The KR client's static shadow into the shadow buffer S, the main target's alpha (client_shadow.wesl).
const PASS_MODE_CLIENT_SHADOW: u32 = 3u;
const SURFACE_LIKE_DEPTH_CLASS_OFFSET: f32 = -4.0;
