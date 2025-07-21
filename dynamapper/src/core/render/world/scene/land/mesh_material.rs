use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};

// ------------- Land material/shader data -------------
// Uniform buffer -> just a fancy name for a struct that is passed to the shader, has
//  global scope and is passed per draw call (so for each chunk mesh).
// Uniform Buffer Size Limitations:
//    Most GPUs limit uniform buffers to 64KB (sometimes less!).
//    u32[2048] is 8192 bytes, twice is 16KB—OK, but you need to watch out if you want to add lots of fields.

// Uniform buffer layouts:
//  Most APIs demand 16-byte alignment per field.
//  For a field to be valid in a uniform buffer, each element of an array must be treated as a “vec4” (i.e., 16 bytes each), not simply a u32 (or f32)!
//  It’s a GPU shader hardware limitation—and applies to both WGSL and to Bevy encase/Buffer.

// In order to have 16-bytes (not bit!) alignment, we can use some packing helpers.
// UVec4 (from glam crate, used by Bevy) is a struct holding four unsigned 32-bit integers (u32 values), used as a “vector of four elements”:

/// Each chunk mesh gets a shader material generated per-chunk, with this struct as its extension.
///
/// See comments above LandUniforms for why uniforms are aligned the way they are.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, ShaderType, bytemuck::Zeroable)]
pub struct LandUniforms {
    pub light_dir: Vec3,
    _pad: f32,
    pub chunk_origin: Vec2,
    _pad2: Vec2,
    pub layers: [UVec4; (super::TILE_NUM_PER_CHUNK_TOTAL as usize + 3) / 4],
    pub hues: [UVec4; (super::TILE_NUM_PER_CHUNK_TOTAL as usize + 3) / 4],
}

pub type LandCustomMaterial = ExtendedMaterial<StandardMaterial, LandMaterialExtension>;

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct LandMaterialExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub tex_array: Handle<Image>,
    #[uniform(102, min_binding_size = 16)]
    pub uniforms: LandUniforms,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/worldmap/land_base.wgsl".into()
    }
}

