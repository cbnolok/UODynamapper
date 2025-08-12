use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};
use super::TILE_NUM_PER_CHUNK_TOTAL;

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
    pub land_uniform: LandUniforms,
    #[uniform(104, min_binding_size = 16)]
    pub tunables_uniform: TunablesUniform,
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
    pub tile_height: u32,
    pub texture_size: u32,  // 0: small, 1: big
    pub texture_layer: u32,
    pub texture_hue: u32,
    // Ensure to have 16 bytes alignment (WGSL std140 layout), add padding if needed.
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LandUniforms {
    pub light_dir: Vec3,
    pub _pad: u32,
    pub chunk_origin: Vec2,
    pub _pad2: Vec2,
    pub tiles: [TileUniform; TILE_NUM_PER_CHUNK_TOTAL],
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TunablesUniform {
    // Like a bool: 0 = per-pixel bicubic lighting, 1 = per-vertex Gouraud lighting
    pub use_vertex_lighting: u32,
    // Sharpness of normal smoothing: 0.0 = blocky normals (flat shading), 1.0 = full bicubic smooth normals
    pub sharpness_factor: f32,
    // How much of the sharpened color is mixed to the original color.
    pub sharpness_mix_factor: f32,
    _pad: f32,

}

