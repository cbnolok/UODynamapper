use bevy::{
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderRef, ShaderType},
};

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
    pub tunables_uniform: TunablesUniform,
    #[uniform(106, min_binding_size = 16)]
    pub lighting_uniform: LightingUniforms,
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
    pub texture_size: u32,  // 0: small, 1: big
    pub texture_layer: u32,
    pub texture_hue: u32,
    // Ensure to have 16 bytes alignment (WGSL std140 layout), add padding if needed.
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LandUniform {
    pub light_dir: Vec3,
    pub _pad: u32,
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
    pub _pad1: f32,
    // Fog
    pub fog_color: Vec4,
    pub fog_params: Vec4, // strength, scale, speed_x, speed_y
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TunablesUniform {
    // Modes
    pub shading_mode: u32,
    pub normal_mode: u32,
    pub enable_bent: u32,
    pub enable_tonemap: u32,

    pub enable_grading: u32,
    pub enable_fog: u32,
    pub _pad1: Vec2,

    // Intensities
    pub ambient_strength: f32,
    pub diffuse_strength: f32,
    pub specular_strength:f32,
    pub rim_strength:     f32,

    pub sharpness_factor: f32,
    pub sharpness_mix:    f32,
    pub _pad2: Vec2,
}



#[repr(C)]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingUniforms {
    pub light_dir: Vec3,
    _pad1: f32,
    pub light_color: Vec3,
    _pad2: f32,
    pub ambient_color: Vec3,
    _pad3: f32,
    pub fill_dir: Vec3,
    pub fill_strength: f32,
    pub exposure: f32,
    pub gamma: f32,
    pub _pad4: Vec2, // align to 16 bytes

    // from VisualUniform
    pub fill_sky_color: Vec4, // .rgb = color, .a = strength
    pub fill_ground_color: Vec4,
    pub rim_color: Vec4, // .rgb = color, .a = power
    pub grade_warm_color: Vec4,
    pub grade_cool_color: Vec4,
    pub grade_params: Vec4, // strength, ...
}

impl Default for LightingUniforms {
    fn default() -> Self {
        Self {
            light_dir: Vec3::new(-0.5, -1.0, -0.3).normalize(),
            _pad1: 0.0,
            light_color: [1.0, 0.95, 0.85].into(),
            _pad2: 0.0,
            ambient_color: [0.25, 0.28, 0.32].into(),
            _pad3: 0.0,
            fill_dir: Vec3::new(0.3, 1.0, 0.2).normalize(),
            fill_strength: 0.35,
            exposure: 1.0,
            gamma: 2.2,
            _pad4: [0.0, 0.0].into(),

            fill_sky_color: Vec4::new(0.5, 0.6, 0.8, 0.2),
            fill_ground_color: Vec4::new(0.4, 0.3, 0.2, 0.1),
            rim_color: Vec4::new(1.0, 1.0, 0.8, 4.0),
            grade_warm_color: Vec4::new(1.0, 0.9, 0.8, 1.0),
            grade_cool_color: Vec4::new(0.8, 0.9, 1.0, 1.0),
            grade_params: Vec4::new(0.1, 0.0, 0.0, 0.0),
        }
    }
}

