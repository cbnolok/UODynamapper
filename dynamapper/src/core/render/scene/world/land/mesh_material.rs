use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};
use serde::Deserialize;

// ------------- Land material/shader data -------------
pub type LandCustomMaterial = ExtendedMaterial<StandardMaterial, LandMaterialExtension>;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct LandMaterialExtension {
    #[sampler(100)]
    //pub tex_sampler: Sampler,
    #[texture(101, dimension = "2d_array")]
    pub texarray_small: Handle<Image>,
    #[texture(102, dimension = "2d_array")]
    pub texarray_big: Handle<Image>,
    #[uniform(103, min_binding_size = 16)]
    pub land_uniform: LandUniform,
    #[uniform(104, min_binding_size = 16)]
    pub scene_uniform: SceneUniform,
    #[uniform(105, min_binding_size = 16)]
    pub effects_uniform: LandEffectsUniform,
    #[uniform(106, min_binding_size = 16)]
    pub lighting_uniform: LandLightingUniforms,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
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

/// Each chunk mesh gets a shader material generated per-chunk, with this struct as its extension.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TileUniform {
    pub tile_height: f32,
    pub texture_size: u32, // 0: small, 1: big
    pub texture_layer: u32,
    pub texture_hue: u32,
    // Ensure to have 16 bytes alignment (WGSL std140 layout), add padding if needed.
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LandUniform {
    pub chunk_origin: Vec2,
    pub _pad2: Vec2,
    pub tiles: [TileUniform; 169], // 13x13 grid for seamless normals
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SceneUniform {
    pub camera_position: Vec3,
    pub time_seconds: f32,
    pub light_direction: Vec3,
    pub global_lighting: f32,
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, Deserialize, Default)]
pub struct LandEffectsUniform {
    // TODO: keep here only non-lighting data. Move the others to LandLightingUniforms, then update the shader and terrain_shader_ui.rs.

    // modes/toggles
    pub shading_mode: u32,
    pub normal_mode: u32,
    pub enable_bent: u32,
    pub enable_fog: u32,
    pub enable_gloom: u32,
    pub enable_tonemap: u32,
    pub enable_grading: u32,
    // optional pre-shade blur of base albedo at fragment level
    pub enable_blur: u32,

    // intensities
    pub ambient_strength: f32,
    pub diffuse_strength: f32,
    pub specular_strength: f32,
    pub rim_strength: f32,
    pub fill_strength: f32,
    pub sharpness_factor: f32,
    pub sharpness_mix: f32,

    // mix factor (0..1) with blurred albedo
    pub blur_strength: f32,

    // Intensities (slot C, 16B)
    // blur radius in UV units (very small numbers like 0.001..0.005)
    pub blur_radius: f32,
    #[serde(default)]
    pub _pad_c1: f32,
    #[serde(default)]
    pub _pad_c2: f32,
    #[serde(default)]
    pub _pad_c3: f32,
}


#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, Deserialize, Default)]
pub struct LandLightingUniforms {
    // vec3 + pad
    pub light_color: Vec3,
    #[serde(default)]
    pub _pad0: f32,
    pub ambient_color: Vec3,
    #[serde(default)]
    pub _pad1: f32,
    pub exposure: f32,
    pub gamma: f32,
    #[serde(default)]
    pub _pad2: Vec2,
    pub fill_sky_color: Vec4,
    pub fill_ground_color: Vec4,
    pub rim_color: Vec4,
    pub grade_warm_color: Vec4,
    pub grade_cool_color: Vec4,
    // TODO: explain via a comment the fields of the Vec fields that are NOT RGBA colors.
    pub grade_params: Vec4,
    pub grade_extra: Vec4,
    pub gloom_params: Vec4,
    pub fog_color: Vec4,
    pub fog_params: Vec4,
    //   fog_color = [r,g,b, max_mix]
    //   fog_params = [distance_density, height_density, noise_scale, noise_strength]
}



#[derive(Clone, Copy, Debug)]
pub enum LandShaderMode {
    Classic2D = 0,
    Enhanced2D = 1,
    KR = 2,
}

#[derive(Resource, Debug, Deserialize)]
pub struct LandShaderModePresets {
    pub classic: LandRenderStylePresetsPerMode,
    pub enhanced: LandRenderStylePresetsPerMode,
    pub kr: LandRenderStylePresetsPerMode,
}

#[derive(Debug, Deserialize)]
pub struct LandRenderStylePresetsPerMode {
    pub morning: LandMaterialUniformsPresets,
    pub afternoon: LandMaterialUniformsPresets,
    pub night: LandMaterialUniformsPresets,
    pub cave: LandMaterialUniformsPresets,
}
#[derive(Debug, Deserialize)]
pub struct LandMaterialUniformsPresets {
    pub effects: LandEffectsUniform,
    pub lighting: LandLightingUniforms,
}
