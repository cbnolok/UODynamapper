use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use serde::{self, Deserialize, Serialize};

// ------------- Land material/shader data -------------
pub type LandCustomMeshMaterial = ExtendedMaterial<StandardMaterial, LandMaterialExtension>;

/// The primary Bevy Material extension for terrain rendering.
/// This struct defines the BindGroup layout for the land shader, mapping Rust fields
/// to WGSL @binding indices.
#[derive(AsBindGroup, Asset, TypePath, Clone)]
pub struct LandMaterialExtension {
    /// 2D Array containing all 64x64 Classic Client land textures.
    #[texture(101, dimension = "2d_array", visibility(vertex, fragment))]
    #[sampler(100, visibility(vertex, fragment))]
    pub texarray_small: Handle<Image>,

    /// 2D Array containing all 128x128 Classic Client land textures.
    #[texture(102, dimension = "2d_array", visibility(vertex, fragment))]
    pub texarray_big: Handle<Image>,

    /// 2D Array containing the currently selected land atlas pages.
    #[texture(109, dimension = "2d_array", visibility(vertex, fragment))]
    pub land_page_atlas: Handle<Image>,

    /// Lookup texture for mapping land texture ids to physical atlas coordinates.
    #[texture(110, sample_type = "u_int", visibility(vertex, fragment))]
    pub land_page_lookup: Handle<Image>,

    /// The paged metadata atlas. Each texel (4 bytes) represents one world tile.
    /// Format: Rg16Uint (R=GraphicID/Layer, G=Packed Height/Flags).
    #[texture(
        103,
        dimension = "2d_array",
        sample_type = "u_int",
        visibility(vertex, fragment)
    )]
    pub tile_meta_atlas: Handle<Image>,

    /// Configuration for the metadata atlas paging system.
    /// Maps logical 2048x2048 world pages to physical GPU array layers.
    #[uniform(104, visibility(vertex, fragment))]
    pub atlas_params: crate::core::render::scene::world::land::tile_atlas::AtlasParams,

    /// Global camera and lighting direction state.
    #[uniform(105, visibility(vertex, fragment))]
    pub scene_uniform: SceneUniform,

    /// Feature toggles and intensity parameters (blur, filtering, water animation).
    #[uniform(106, visibility(vertex, fragment))]
    pub effects_uniform: LandEffectsUniform,

    /// Shared lighting parameters (fog, grading, tonemap) used across all shaders.
    #[uniform(107, visibility(vertex, fragment))]
    pub global_lighting_uniform: GlobalLightingUniforms,

    /// Terrain-specific lighting (bent normals, rim, specular, fill).
    #[uniform(108, visibility(vertex, fragment))]
    pub land_lighting_uniform: LandLightingUniforms,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/worldmap/land/main.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/worldmap/land/main.wgsl".into()
    }

    /*
    fn deferred_vertex_shader() -> ShaderRef {
        "shaders/worldmap/land/main.wgsl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/worldmap/land/main.wgsl".into()
    }
    */

    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

// Uniform buffer -> just a fancy name for a struct that is passed to the shader, has
//  global scope and is passed per draw call (so for each chunk mesh).
// Uniform Buffer Size Limitations:
//    Most GPUs limit uniform buffers to 64KB (sometimes less!).
//    u32[2048] is 8192 bytes, twice is 16KB—OK, but you need to watch out if you want to add lots of fields.

// Uniform buffer layouts:
//  Most APIs demand 16-byte (not bit!) alignment per field.
//  For a field to be valid in a uniform buffer, each element of an array must be treated as a “vec4” (i.e., 16 bytes each), not simply a u32 (or f32)!
//  It’s a GPU shader hardware limitation—and applies to both WGSL and to Bevy encase/Buffer.

// In order to have 16-bytes (not bit!) alignment, we can use some packing helpers.
// UVec4 (from glam crate, used by Bevy) is a struct holding four unsigned 32-bit integers (u32 values), used as a “vector of four elements”:

#[repr(C, align(16))]
#[derive(Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable, Default)]
#[allow(dead_code)] // ShaderType derive generates internal `check` functions that appear unused
pub struct SceneUniform {
    pub camera_position: Vec3,
    pub _pad_cam: f32,
    pub light_direction: Vec3,
    pub global_lighting: f32,
    pub render_zoom: f32,
    pub adaptive_zoom_simplification: f32,
    pub _pad: Vec2,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[allow(dead_code)] // ShaderType derive generates internal `check` functions that appear unused
pub struct LandEffectsUniform {
    // Non-lighting rendering controls only.  Lighting toggles and intensities
    // live in GlobalLightingUniforms / LandLightingUniforms so they can be
    // shared with future art-tile shaders (2D sprites that don't have a 3D
    // mesh but still need global lighting, grading, fog, etc.).

    // --- Modes & texture blur toggle (vec4 slot 0) ---
    pub shading_mode: u32,
    pub normal_mode: u32,
    /// Optional pre-shade blur of base albedo at fragment level.
    pub enable_blur: u32,
    pub enable_linear_filtering: u32,

    // --- Graphics controls (vec4 slot 1) ---
    /// Reserved for layout compatibility; shader upscaling/reconstruction is disabled.
    pub reconstruction_mode: u32,
    pub sharpening_amount: f32,
    /// Mix factor (0..1) with blurred albedo.
    pub blur_strength: f32,
    /// Blur radius in screen pixels (small values like 0.5..8.0).
    pub blur_radius: f32,

    // --- Animated water (vec4 slot 2) ---
    /// 1 = apply sin/cos UV distortion to IsWet tiles; 0 = disable.
    /// Automatically suppressed at high zoom (> 10) in the shader.
    pub enable_water_animation: u32,
    /// 0 = neutral/current, 1 = Enhanced Client, 2 = Kingdom Reborn.
    #[serde(default)]
    pub post_process_profile: u32,
    /// Procedural world-space grunge/weathering multiplier.
    #[serde(default)]
    pub enable_grunge: u32,
    #[serde(default)]
    pub _pad_eff0: u32,

    // --- Atmosphere controls (vec4 slot 3) ---
    /// Strength of procedural grunge/weathering on terrain and art.
    #[serde(default)]
    pub grunge_strength: f32,
    /// Reserved for future additive light decals.
    #[serde(default)]
    pub light_decal_intensity: f32,
    /// Reserved for future texture normal-map atlas sampling.
    #[serde(default)]
    pub enable_normal_maps: u32,
    #[serde(default)]
    pub _pad_eff1: u32,
}

/// Global lighting parameters shared across all shader types (land, art tiles, etc.).
/// Art tiles are 2D sprites with a predefined bounding box — no 3D mesh — so they
/// don't need view/normal–dependent lighting, but they share grading, fog, gloom,
/// tonemapping, and base light/ambient colors with the land shader.
#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[allow(dead_code)]
pub struct GlobalLightingUniforms {
    // --- Toggles (vec4 slot 0) ---
    pub enable_fog: u32,
    pub enable_tonemap: u32,
    pub enable_grading: u32,
    pub enable_gloom: u32,

    // --- Colors ---
    pub light_color: Vec3,
    #[serde(default)]
    pub _pad0_: f32,
    pub ambient_color: Vec3,
    #[serde(default)]
    pub _pad1_: f32,

    // --- Exposure / gamma ---
    pub exposure: f32,
    pub gamma: f32,
    pub ambient_strength: f32,
    #[serde(default)]
    pub _pad2_: f32,

    // --- Grading ---
    pub grade_warm_color: Vec4,
    pub grade_cool_color: Vec4,
    //   grade_params  = [strength, headroom_reserve, chroma_tint, headroom_on]
    pub grade_params: Vec4,
    //   grade_extra   = [vibrance, saturation, contrast, split_tone_strength]
    pub grade_extra: Vec4,

    // --- Gloom ---
    //   gloom_params  = [amount, falloff_height, shadow_bias, fog_height_bias]
    pub gloom_params: Vec4,

    // --- Fog ---
    //   fog_color     = [r, g, b, max_mix]
    pub fog_color: Vec4,
    //   fog_params    = [distance_density, height_density, noise_scale, noise_strength]
    pub fog_params: Vec4,

    // --- KR-style enhancements ---
    //   gloom_color   = [r, g, b, desaturation_amount]
    //   RGB: dedicated gloom tint (replaces auto-derive from ambient when non-zero).
    //   A: how much to desaturate in gloomy areas (0.0 = none, 1.0 = full grayscale).
    pub gloom_color: Vec4,
    //   fog_night_color = [r, g, b, blend_factor]
    //   Blends fog_color toward this darker tint. At blend=0: fog_color as-is.
    //   At blend=1: fully replaces fog_color with this (e.g. deep blue-black night fog).
    pub fog_night_color: Vec4,
}

/// Land-specific lighting: parameters that require a 3D mesh with surface normals
/// and a view vector, so they only apply to the terrain (not to flat art tiles).
#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[allow(dead_code)]
pub struct LandLightingUniforms {
    // --- Toggle ---
    pub enable_bent: u32,
    #[serde(default)]
    pub _pad0_: u32,
    #[serde(default)]
    pub _pad1_: u32,
    #[serde(default)]
    pub _pad2_: u32,

    // --- Intensities ---
    pub diffuse_strength: f32,
    pub specular_strength: f32,
    pub rim_strength: f32,
    pub fill_strength: f32,

    pub sharpness_factor: f32,
    pub sharpness_mix: f32,
    /// Half-Lambert wrap factor: 0.0 = standard Lambert, 0.3–0.5 = KR-style soft shadow terminator.
    pub diffuse_wrap: f32,
    #[serde(default)]
    pub _pad4_: f32,

    // --- Hemisphere fill & rim colors (need N / V) ---
    pub fill_sky_color: Vec4,
    pub fill_ground_color: Vec4,
    pub rim_color: Vec4,
}

#[derive(Clone, Copy, Debug)]
pub enum LandShaderMode {
    Classic2D = 0,
    Enhanced2D = 1,
    KR = 2,
}

#[derive(Resource, Deserialize, Serialize)]
pub struct LandShaderModePresets {
    pub classic: LandRenderStylePresetsPerMode,
    pub enhanced: LandRenderStylePresetsPerMode,
    pub kr: LandRenderStylePresetsPerMode,
    pub default_preset: String,
}

#[derive(Deserialize, Serialize)]
pub struct LandRenderStylePresetsPerMode {
    pub morning: LandMaterialUniformsPresets,
    pub afternoon: LandMaterialUniformsPresets,
    pub night: LandMaterialUniformsPresets,
    pub cave: LandMaterialUniformsPresets,
}
#[derive(Deserialize, Serialize)]
pub struct LandMaterialUniformsPresets {
    pub global_lighting: f32,
    pub effects: LandEffectsUniform,
    pub lighting: GlobalLightingUniforms,
    pub land_lighting: LandLightingUniforms,
}
