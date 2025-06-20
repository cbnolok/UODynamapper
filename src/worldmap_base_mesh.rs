use bevy::{
    prelude::*,
    pbr::MaterialExtension,
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, ShaderRef, ShaderType},
    }
};
use crate::tile_cache::MAX_TILE_LAYERS;

// -- 1) Vertex attributes -----------------------------------------------------

/*
// Custom Vertex attribute for (layer<<16)|hue (again, only for custom render pipelines)
pub const ATTR_LAYER_HUE: MeshVertexAttribute = MeshVertexAttribute::new(
  "layer_hue",     // name
  10,                // shader location 3
  VertexFormat::Uint32,
);
*/

// Simple Pod‐safe vertex data.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertexAttrs {
  // Geometry:
  pub pos:   [f32; 3],
  pub norm:  [f32; 3],
  pub uv:    [f32; 2],

  // Custom data from here, use it only if we decide to use a custom rendering pipeling, but i
  //  really want to avoid it.
  //pub layer_hue: u32,       // layer<<16 | hue
}

// -- 2) Custom Material Definition --------------------------------------------

const TERRAIN_SHADER_PATH: &str = "shaders/worldmap/terrain_base.wgsl";

// -- Define uniforms to be used in the shader. Rust side.
// Uniform Buffer Size Limitations:
//    Most GPUs limit uniform buffers to 64KB (sometimes less!).
//    u32[2048] is 8192 bytes, twice is 16KB—OK, but you need to watch out if you want to add lots of fields.

#[repr(C, align(16))] // Most APIs demand 16-byte alignment per field.
#[derive(Copy, Clone, Debug, ShaderType, bytemuck::Zeroable)]
pub struct TerrainUniforms {
    pub light_dir: Vec3,
    _pad: f32,            // Always pad after vec3<f32>!
    pub layers: [u32; MAX_TILE_LAYERS as usize],
    pub hues: [u32; MAX_TILE_LAYERS as usize],
}

#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct TerrainMaterial {    
    #[texture(100, dimension="2d_array")]
    #[sampler(101)]
    pub tex_array: Handle<Image>,

    // ← This produces group(2), binding(2) as a 16-byte UBO
    #[uniform(102, min_binding_size=16)]
    pub uniforms: TerrainUniforms,
}

impl MaterialExtension for TerrainMaterial {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
    }
    fn fragment_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
    }
}

