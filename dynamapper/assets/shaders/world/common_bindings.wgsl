// ============================================================================
// common_bindings.wgsl — Shared uniform struct definitions across all shaders.
// ============================================================================

const USE_VOLUMETRIC_NOISE: u32 = 1u;

// Global scene state: camera, light, zoom level.
struct SceneUniform {
    camera_position: vec3<f32>,
    _pad_cam: f32,
    light_direction: vec3<f32>,
    global_lighting: f32,
    render_zoom: f32,
    adaptive_zoom_simplification: f32,
    _pad0_: vec2<f32>,
}

// Visual feature toggles and intensity parameters.
struct LandEffectsUniform {
    shading_mode:   u32,
    normal_mode:    u32,
    enable_blur:    u32,
    enable_linear_filtering: u32,
    art_shadow_strength:     f32,
    sharpening_amount:       f32,
    blur_strength:           f32,
    blur_radius:             f32,
    enable_water_animation: u32,
    post_process_profile: u32,
    enable_grunge: u32,
    _pad_eff0: u32,
    grunge_strength: f32,
    light_decal_intensity: f32,
    art_highlight_strength: f32,
    art_depth_tint_strength: f32,
    enable_normal_maps: u32,
    enable_art_fake_normals: u32,
    art_contact_shadow_strength: f32,
    art_mottle_strength: f32,
    kr_land_temperature_strength: f32,
    kr_land_relief_shadow_strength: f32,
    kr_land_shadow_mottle_strength: f32,
    kr_art_temperature_strength: f32,
    land_normal_map_strength: f32,
}

// Global lighting parameters shared across all shader types (land, art tiles, etc.).
struct GlobalLightingUniforms {
    enable_fog:      u32,
    enable_tonemap:  u32,
    enable_grading:  u32,
    enable_gloom:    u32,
    light_color: vec3<f32>,
    _pad0_: f32,
    atmosphere_tint: vec3<f32>,
    _pad1_: f32,
    exposure: f32,
    ambient_strength: f32,
    _pad2_: vec2<f32>,
    grade_warm_color: vec4<f32>,
    grade_cool_color: vec4<f32>,
    grade_params: vec4<f32>,
    grade_extra: vec4<f32>,
    gloom_params: vec4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    gloom_color: vec4<f32>,
    fog_night_color: vec4<f32>,
}
