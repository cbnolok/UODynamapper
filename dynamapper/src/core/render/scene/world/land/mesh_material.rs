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
    pub texture_size: u32, // 0: small, 1: big
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
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TunablesUniform {
    // modes/toggles
    pub shading_mode: u32, // 0 classic, 1 enhanced, 2 KR
    pub normal_mode: u32,  // 0 geometric, 1 bicubic
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
    // padding to keep 16B alignment (WGSL expects 4-f32 slots)
    pub _pad_c1: f32,
    pub _pad_c2: f32,
    pub _pad_c3: f32,
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightingUniforms {
    // vec3 + pad
    pub light_color: Vec3,
    pub _pad0: f32,

    pub ambient_color: Vec3,
    pub _pad1: f32,

    pub exposure: f32,
    pub gamma: f32, // keep 2.2 or 1.0; unused in shader
    pub _pad2: Vec2,

    pub fill_sky_color: Vec4,
    pub fill_ground_color: Vec4,
    pub rim_color: Vec4,

    pub grade_warm_color: Vec4,
    pub grade_cool_color: Vec4,
    pub grade_params: Vec4, // [grade_strength, headroom_reserve, hemi_chroma_tint, headroom_on]
    pub grade_extra: Vec4,  // [vibrance, saturation, contrast, split_strength]

    pub gloom_params: Vec4, // [amount, height_falloff, shadow_bias, _]

    // Fog
    pub fog_color: Vec4,
    pub fog_params: Vec4,
    //   fog_color = [r,g,b, max_mix]
    //   fog_params = [distance_density, height_density, noise_scale, noise_strength]
}

#[derive(Clone, Copy, Debug)]
pub enum ShaderMode {
    Classic2D = 0,
    Enhanced2D = 1,
    KR = 2,
}

fn common_tunables(mode: ShaderMode) -> TunablesUniform {
    // Defaults suitable for KR; presets will tweak per ambient.
    let (ambient, diffuse, specular, rim, fill) = match mode {
        ShaderMode::Classic2D => (0.18, 1.00, 0.00, 0.00, 0.00),
        ShaderMode::Enhanced2D => (0.18, 1.05, 0.03, 0.06, 0.28),
        ShaderMode::KR => (0.18, 1.10, 0.05, 0.16, 0.34),
    };
    TunablesUniform {
        shading_mode: mode as u32,
        normal_mode: 1, // bicubic by default; set 0 for exact old normals
        enable_bent: 1,
        enable_fog: 0,

        enable_gloom: if matches!(mode, ShaderMode::KR) { 1 } else { 0 },
        enable_tonemap: 1,
        enable_grading: 1,
        enable_blur: 0,

        ambient_strength: ambient,
        diffuse_strength: diffuse,
        specular_strength: specular,
        rim_strength: rim,

        fill_strength: fill,
        sharpness_factor: match mode {
            ShaderMode::Classic2D => 1.0,
            ShaderMode::Enhanced2D => 1.5,
            ShaderMode::KR => 2.0,
        },
        sharpness_mix: match mode {
            ShaderMode::Classic2D => 0.0,
            ShaderMode::Enhanced2D => 0.25,
            ShaderMode::KR => 0.55,
        },

        blur_strength: 0.15, // subtle by default

        // (slot C) tiny UV radius is enough for softening
        blur_radius: 0.0025,
        _pad_c1: 0.0,
        _pad_c2: 0.0,
        _pad_c3: 0.0,
    }
}

fn lighting_base() -> LightingUniforms {
    LightingUniforms {
        light_color: [1.06, 0.99, 0.92].into(),
        _pad0: 0.0,
        ambient_color: [0.18, 0.22, 0.29].into(),
        _pad1: 0.0,

        exposure: 1.08,
        gamma: 2.2,
        _pad2: [0.0, 0.0].into(),

        fill_sky_color: [0.48, 0.68, 1.00, 0.75].into(),
        fill_ground_color: [0.30, 0.20, 0.12, 0.35].into(),
        rim_color: [0.80, 0.90, 1.05, 2.6].into(), // rgb, power in .w

        // warm/cool tints (rgb), alpha unused (vibrance/contrast move to grade_extra)
        grade_warm_color: [1.08, 0.98, 0.90, 0.0].into(),
        grade_cool_color: [0.82, 0.95, 1.05, 0.0].into(),

        // grade_strength, headroom_reserve, hemi_chroma_tint, headroom_on
        grade_params: [0.95, 0.15, 0.38, 1.0].into(),

        // vibrance, saturation, contrast, split_strength
        grade_extra: [0.55, 1.25, 1.20, 1.00].into(),

        // amount, height_falloff, shadow_bias, _
        gloom_params: [0.20, 0.008, 0.45, 0.0].into(),

        // Visible, subtle bluish fog by default (UI can override)
        fog_color: [0.75, 0.85, 0.95, 0.5].into(),
        // distance_density, height_density, noise_scale, noise_strength
        fog_params: [0.02, 0.00, 0.00, 0.00].into(),
    }
}

// Helpers to tweak lighting per ambient quickly.
fn set_lighting(
    mut l: LightingUniforms,
    light_color: Vec3,
    ambient_color: Vec3,
    fill_sky: Vec4,
    fill_ground: Vec4,
    rim: Vec4,
    grade_params: Vec4,
    grade_extra: Vec4,
    gloom: Vec4,
    exposure: f32,
) -> LightingUniforms {
    l.light_color = light_color;
    l.ambient_color = ambient_color;
    l.fill_sky_color = fill_sky;
    l.fill_ground_color = fill_ground;
    l.rim_color = rim;
    l.grade_params = grade_params;
    l.grade_extra = grade_extra;
    l.gloom_params = gloom;
    l.exposure = exposure;
    l
}

// --------------------------- PRESETS ----------------------------------------

pub fn morning_preset(mode: ShaderMode) -> (TunablesUniform, LightingUniforms) {
    let mut t = common_tunables(mode);
    let mut l = lighting_base();

    // Morning: soft warm sun, cool ambient; moderate gloom; vibrant but gentle.
    l = set_lighting(
        l,
        [1.05, 0.99, 0.92].into(),
        [0.17, 0.22, 0.29].into(),
        [0.50, 0.70, 1.00, 0.70].into(),
        [0.28, 0.19, 0.12, 0.35].into(),
        [0.70, 0.85, 1.00, 2.5].into(),
        [0.90, 0.15, 0.35, 1.0].into(), // grade_strength, headroom, hemi_chroma, on
        [0.45, 1.20, 1.15, 0.90].into(), // vibrance, saturation, contrast, split
        [0.20, 0.008, 0.45, 0.0].into(), // gloom
        1.05,
    );
    if matches!(mode, ShaderMode::Classic2D) {
        t.enable_grading = 0;
        t.enable_gloom = 0;
        t.fill_strength = 0.0;
    }
    (t, l)
}

pub fn afternoon_preset(mode: ShaderMode) -> (TunablesUniform, LightingUniforms) {
    let mut t = common_tunables(mode);
    let mut l = lighting_base();

    // Afternoon: brighter sun, rich colors; lighter gloom.
    l = set_lighting(
        l,
        [1.08, 0.99, 0.90].into(),
        [0.18, 0.22, 0.28].into(),
        [0.48, 0.68, 1.00, 0.75].into(),
        [0.30, 0.20, 0.12, 0.35].into(),
        [0.95, 0.95, 0.85, 2.7].into(),
        [1.00, 0.15, 0.40, 1.0].into(),
        [0.60, 1.25, 1.22, 1.00].into(),
        [0.15, 0.007, 0.35, 0.0].into(),
        1.10,
    );
    if matches!(mode, ShaderMode::Classic2D) {
        t.enable_grading = 0;
        t.enable_gloom = 0;
        t.fill_strength = 0.0;
    }
    (t, l)
}

pub fn night_preset(mode: ShaderMode) -> (TunablesUniform, LightingUniforms) {
    let mut t = common_tunables(mode);
    let mut l = lighting_base();

    // Night: cool light, low ambient, stronger rim silhouettes, stronger gloom.
    t.ambient_strength = 0.10;
    t.diffuse_strength = 0.70;
    t.specular_strength = 0.03;
    t.rim_strength = if matches!(mode, ShaderMode::Classic2D) {
        0.0
    } else {
        0.22
    };
    t.fill_strength = 0.28;

    l = set_lighting(
        l,
        [0.80, 0.88, 1.05].into(),
        [0.12, 0.16, 0.24].into(),
        [0.40, 0.60, 1.00, 0.70].into(),
        [0.20, 0.16, 0.14, 0.30].into(),
        [0.65, 0.85, 1.10, 3.0].into(),
        [1.10, 0.18, 0.45, 1.0].into(),
        [0.75, 1.20, 1.30, 0.80].into(),
        [0.40, 0.010, 0.65, 0.0].into(),
        1.20,
    );
    if matches!(mode, ShaderMode::Classic2D) {
        t.enable_grading = 0;
        t.enable_gloom = 0;
        t.fill_strength = 0.0;
    }
    (t, l)
}

pub fn cave_preset(mode: ShaderMode) -> (TunablesUniform, LightingUniforms) {
    let mut t = common_tunables(mode);
    let mut l = lighting_base();

    // Cave: very low ambient, heavier gloom, cool tones, reduced exposure.
    t.ambient_strength = 0.06;
    t.diffuse_strength = 0.85;
    t.specular_strength = 0.02;
    t.rim_strength = if matches!(mode, ShaderMode::Classic2D) {
        0.0
    } else {
        0.14
    };
    t.fill_strength = 0.22;

    l = set_lighting(
        l,
        [0.90, 0.95, 1.05].into(),
        [0.10, 0.14, 0.20].into(),
        [0.38, 0.60, 1.00, 0.65].into(),
        [0.18, 0.14, 0.12, 0.28].into(),
        [0.55, 0.75, 1.05, 2.8].into(),
        [1.05, 0.20, 0.38, 1.0].into(),
        [0.65, 1.15, 1.28, 0.80].into(),
        [0.65, 0.012, 0.75, 0.0].into(),
        0.95,
    );
    if matches!(mode, ShaderMode::Classic2D) {
        t.enable_grading = 0;
        t.enable_gloom = 0;
        t.fill_strength = 0.0;
    }
    (t, l)
}
