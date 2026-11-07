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

    /// Small visible static-light set for KR-style terrain light catch.
    #[uniform(111, visibility(fragment))]
    pub static_light_uniform: LandStaticLightUniform,

    /// Shared generated grunge/noise texture for EC/KR weathering overlays.
    #[texture(112, visibility(fragment))]
    pub visual_grunge_texture: Handle<Image>,

    /// Compact Classic `art.mul` land diamonds used only by flat wet terrain.
    #[texture(115, dimension = "2d_array", visibility(fragment))]
    pub classic_land_art_atlas: Handle<Image>,

    /// Maps a Classic land graphic id to `[layer, width, height, upscale]` in
    /// `classic_land_art_atlas`; layer 0 means absent.
    #[texture(116, sample_type = "u_int", visibility(fragment))]
    pub classic_land_art_lookup: Handle<Image>,

    /// First/last sorted EC material payloads within the original 49-pass limit.
    #[texture(117, sample_type = "u_int", visibility(fragment))]
    pub ec_terrain_bases: Handle<Image>,

    /// Exact sorted ownership layers and normalized masks for resident EC chunks.
    #[texture(118, dimension = "2d_array", sample_type = "u_int", visibility(fragment))]
    pub ec_terrain_stacks: Handle<Image>,
}

impl MaterialExtension for LandMaterialExtension {
    fn vertex_shader() -> ShaderRef {
        "shaders/world/land/main.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/world/land/main.wesl".into()
    }

    /*
    fn deferred_vertex_shader() -> ShaderRef {
        "shaders/world/land/main.wesl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/world/land/main.wesl".into()
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
    pub ec_reflection_camera_distance: f32,
    pub _pad: f32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
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
    /// Fake per-art vertical/occlusion shading strength.
    pub art_shadow_strength: f32,
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
    pub post_process_profile: u32,
    /// Procedural world-space grunge/weathering multiplier.
    pub enable_grunge: u32,
    /// Source-wide controls from Graphics settings, not a style/time preset.
    /// Bit 0 enables CC transitions; bit 1 disables EC transitions.
    #[serde(skip)]
    pub terrain_transition_flags: u32,

    // --- Atmosphere controls (vec4 slot 3) ---
    /// Strength of procedural grunge/weathering on terrain and art.
    pub grunge_strength: f32,
    /// Reserved for future additive light decals.
    pub light_decal_intensity: f32,
    /// Fake per-art light catch strength.
    pub art_highlight_strength: f32,
    /// Fake atmosphere/depth tint on art sprites and ground statics.
    pub art_depth_tint_strength: f32,

    // --- Reserved future controls (vec4 slot 4) ---
    /// Reserved for future texture normal-map atlas sampling.
    pub enable_normal_maps: u32,
    /// 1 = use KR-style fake normal lighting on 2D art sprites/ground art.
    pub enable_art_fake_normals: u32,
    /// KR-style contact darkening for static art bases, walls, foliage, and ground art.
    pub art_contact_shadow_strength: f32,
    /// Strength of procedural variation applied to art shading and roof repetition.
    pub art_mottle_strength: f32,

    // --- KR style controls (vec4 slot 5) ---
    /// 0 disables KR land warm/cool split; 1 preserves the preset strength.
    pub kr_land_temperature_strength: f32,
    /// 0 disables KR land relief/crease darkening; 1 preserves the preset strength.
    pub kr_land_relief_shadow_strength: f32,
    /// 0 disables KR land shadow mottling; 1 preserves the preset strength.
    pub kr_land_shadow_mottle_strength: f32,
    /// 0 disables KR art warm/cool split; 1 preserves the preset strength.
    pub kr_art_temperature_strength: f32,

    // --- Texture normal / art projected shadow controls (vec4 slot 6) ---
    /// Strength for EC land role-3 texture normal contribution.
    pub land_normal_map_strength: f32,
    /// 1 = render KR-style projected bounds shadows for tall art sprites.
    pub enable_art_projected_shadows: u32,
    /// Opacity multiplier for projected art shadows.
    pub art_projected_shadow_strength: f32,
    /// Directional projection length in world tiles per sprite height unit.
    pub art_projected_shadow_length: f32,

    // --- Art projected shadow softness (vec4 slot 7) ---
    /// Edge softness for bounds-projected art shadows.
    pub art_projected_shadow_softness: f32,
    /// 0 disables KR material-family micro contrast; 1 preserves preset strength.
    pub kr_land_material_contrast_strength: f32,
    /// Visible static-light mask opacity independent from local light response.
    pub static_light_decal_visibility: f32,
    /// 1 = add KR-style edge grime where textured land materials meet.
    pub enable_kr_transition_grime: u32,

    // --- KR transition grime controls (vec4 slot 8) ---
    /// Strength of KR material-boundary grime/dirt accumulation.
    pub kr_transition_grime_strength: f32,
    /// 1 = add low-frequency KR-style uneven color wash over land and art.
    pub enable_kr_color_wash: u32,
    /// Strength of KR low-frequency painted tint/illumination variation.
    pub kr_color_wash_strength: f32,
    /// World-tile wavelength for the broad KR color wash.
    pub kr_color_wash_scale: f32,
}

/// Global lighting parameters shared across all shader types (land, art tiles, etc.).
/// Art tiles are 2D sprites with a predefined bounding box — no 3D mesh — so they
/// don't need view/normal–dependent lighting, but they share grading, fog, gloom,
/// tonemapping, and base light/ambient colors with the land shader.
#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct GlobalLightingUniforms {
    // --- Toggles (vec4 slot 0) ---
    pub enable_fog: u32,
    pub enable_tonemap: u32,
    pub enable_grading: u32,
    pub enable_gloom: u32,

    // --- Colors ---
    pub light_color: Vec3,
    /// The tone stage `enable_tonemap` switches on: `TONE_CURVE_PROFILE` or `TONE_CURVE_IDENTITY`.
    /// Takes the former `_pad0_` slot after `light_color`.
    pub tone_curve: u32,
    pub atmosphere_tint: Vec3,
    #[serde(skip)]
    pub _pad1_: f32,

    // --- Exposure / ambient ---
    pub exposure: f32,
    pub ambient_strength: f32,
    #[serde(skip)]
    pub _pad2_: Vec2,

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

    // --- KR tonemap ---
    //   kr_tonemap_params.x = pre-scale applied to the incoming color for the KR
    //   post-process profile, before the filmic curve in `tonemap_ec_kr_profile`.
    //   Exposed as a UI knob so KR brightness is tunable instead of hardcoded (the
    //   old fixed value crushed KR ~11x darker than EC). y/z/w reserved.
    //   A full vec4 slot is used (not a trailing scalar + vec3) to match the other
    //   *_params blocks — naga-oil mis-composes this struct with a trailing vec3.
    //   Appended as a new trailing slot. Hand-written preset files must set it; a cached
    //   land.toml from an older build gets it from the bundled presets.
    pub kr_tonemap_params: Vec4,
}

/// Default KR tonemap pre-scale. The old hardcoded value was ≈ 0.09
/// (middle_gray(0.18) / luminance(2.0)), which crushed KR ~11x darker than EC.
/// Defaulted near EC's own pre-scale (~1.0) so KR presets are legible out of the
/// box; the "KR Tonemap Brightness" knob can still lower it for a moodier look.
pub const DEFAULT_KR_TONEMAP_GAIN: f32 = 0.9;

/// `tone_curve`: the visual profile's curve (Reinhard, EC filmic or KR filmic), with `exposure` inside it.
pub const TONE_CURVE_PROFILE: u32 = 0;
/// `tone_curve`: no curve. `exposure` scales the color as plain brightness, clamped to [0, 1] like the clients'
/// 8-bit target. Matches `TONE_CURVE_IDENTITY` in `postprocess/tonemapping.wesl`.
pub const TONE_CURVE_IDENTITY: u32 = 1;

/// `terrain_shading_model`: the fragment shading paths (`shading_mode` 1 and 2) with a per-pixel normal.
pub const TERRAIN_SHADING_SMOOTH_PER_PIXEL: u32 = 0;
/// `terrain_shading_model`: the KR and Enhanced clients' terrain shade. One half-Lambert grey per cell corner from
/// the client's vertex normal, interpolated over the client's two triangles per cell, times the material colour.
/// Matches `TERRAIN_SHADING_CLIENT_PER_VERTEX` in `world/land/land_bindings.wesl`.
pub const TERRAIN_SHADING_CLIENT_PER_VERTEX: u32 = 1;

/// Land-specific lighting: parameters that require a 3D mesh with surface normals
/// and a view vector, so they only apply to the terrain (not to flat art tiles).
#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct LandLightingUniforms {
    // --- Toggle ---
    pub enable_bent: u32,
    /// How the fragment paths shade land: `TERRAIN_SHADING_SMOOTH_PER_PIXEL` or
    /// `TERRAIN_SHADING_CLIENT_PER_VERTEX`. Classic (`shading_mode` 0) ignores it. Takes the former `_pad0_` slot.
    pub terrain_shading_model: u32,
    #[serde(skip)]
    pub _pad1_: u32,
    #[serde(skip)]
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
    #[serde(skip)]
    pub _pad4_: f32,

    // --- Hemisphere fill & rim colors (need N / V) ---
    pub fill_sky_color: Vec4,
    pub fill_ground_color: Vec4,
    pub rim_color: Vec4,
}

pub const LAND_STATIC_LIGHT_MAX: usize = 16;

#[repr(C, align(16))]
#[derive(Clone, Copy, PartialEq, ShaderType)]
#[allow(dead_code)]
pub struct LandStaticLightUniform {
    /// xyz = world position, w = radius in world tiles.
    pub lights: [Vec4; LAND_STATIC_LIGHT_MAX],
    /// rgb = light response color, a reserved.
    pub colors: [Vec4; LAND_STATIC_LIGHT_MAX],
    /// x = active light count, remaining lanes reserved.
    pub params: UVec4,
}

impl Default for LandStaticLightUniform {
    fn default() -> Self {
        Self {
            lights: [Vec4::ZERO; LAND_STATIC_LIGHT_MAX],
            colors: [Vec4::ZERO; LAND_STATIC_LIGHT_MAX],
            params: UVec4::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum LandShaderMode {
    Classic2D = 0,
    Enhanced2D = 1,
    KR = 2,
}

#[derive(Resource, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LandShaderModePresets {
    pub classic: LandRenderStylePresetsPerMode,
    pub enhanced: LandRenderStylePresetsPerMode,
    /// The faithful KR client look (KR content, shading_mode 2, post_process_profile 2).
    pub kr: LandRenderStylePresetsPerMode,
    /// Dynamapper's painterly look on the same KR content and shader paths as `kr`; only the values differ.
    pub painterly: LandRenderStylePresetsPerMode,
    pub default_preset: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LandRenderStylePresetsPerMode {
    pub morning: LandMaterialUniformsPresets,
    pub afternoon: LandMaterialUniformsPresets,
    pub night: LandMaterialUniformsPresets,
    pub cave: LandMaterialUniformsPresets,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LandMaterialUniformsPresets {
    pub global_lighting: f32,
    pub effects: LandEffectsUniform,
    pub lighting: GlobalLightingUniforms,
    pub land_lighting: LandLightingUniforms,
    pub sun: SunAngles,
}

/// A preset's sun: one static light direction for land, art and the sprite shadows, as compass angles.
///
/// The world axes are x = UO x (east), z = UO y (south) and y up, so azimuth 0 is north (-z) and 90 east (+x).
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SunAngles {
    /// Compass bearing of the sun in degrees, clockwise from north: 90 east, 180 south, 270 west.
    pub azimuth_degrees: f32,
    /// Height of the sun above the horizon in degrees: 0 on the horizon, 90 overhead.
    pub elevation_degrees: f32,
}

impl SunAngles {
    /// The unit vector towards the sun: the shaders' `scene.light_direction`.
    ///
    /// The trigonometry runs in f64, so the default sun gives `BAKED_GLOBAL_LIGHT.normalize()` bit for bit.
    pub fn light_direction(self) -> Vec3 {
        let azimuth = f64::from(self.azimuth_degrees).to_radians();
        let elevation = f64::from(self.elevation_degrees).to_radians();
        let horizontal = elevation.cos();
        bevy::math::DVec3::new(horizontal * azimuth.sin(), elevation.sin(), -horizontal * azimuth.cos()).as_vec3()
    }
}

impl Default for SunAngles {
    /// `BAKED_GLOBAL_LIGHT`, the light of every preset before the sun was a setting.
    fn default() -> Self {
        Self {
            azimuth_degrees: crate::core::constants::BAKED_SUN_AZIMUTH_DEGREES,
            elevation_degrees: crate::core::constants::BAKED_SUN_ELEVATION_DEGREES,
        }
    }
}
