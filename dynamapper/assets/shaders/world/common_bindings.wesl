// ============================================================================
// common_bindings.wesl — Shared uniform struct definitions across all shaders.
// ============================================================================

// Global scene state: camera, light, zoom level.
struct SceneUniform {
    camera_position: vec3<f32>,
    _pad_cam: f32,
    light_direction: vec3<f32>,
    global_lighting: f32,
    render_zoom: f32,
    adaptive_zoom_simplification: f32,
    ec_reflection_camera_distance: f32,
    // How many times wider than the KR client's 26-world-unit view the camera shows (client_filter.wesl).
    client_view_scale: f32,
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
    // Graphics settings: bit 0 enables CC transitions; bit 1 disables EC transitions.
    terrain_transition_flags: u32,
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
    enable_art_projected_shadows: u32,
    art_projected_shadow_strength: f32,
    art_projected_shadow_length: f32,
    art_projected_shadow_softness: f32,
    kr_land_material_contrast_strength: f32,
    static_light_decal_visibility: f32,
    enable_kr_transition_grime: u32,
    kr_transition_grime_strength: f32,
    enable_kr_color_wash: u32,
    kr_color_wash_strength: f32,
    kr_color_wash_scale: f32,
    // TEXTURE_FILTER_PIXEL_ART or TEXTURE_FILTER_CLIENT_BILINEAR (client_filter.wesl).
    texture_filter: u32,
    // The KR client's grunge on statics (art/client_grunge.wesl): on/off and strength.
    enable_client_grunge: u32,
    client_grunge_strength: f32,
    // Multiplies the land light decals and static local lights with light_decal_intensity: the intensity of the
    // source KR-like's and Enhanced's local lights take, 1 in Classic and UOD-Painterly.
    local_light_intensity: f32,
}

// Global lighting parameters shared across all shader types (land, art tiles, etc.).
struct GlobalLightingUniforms {
    enable_fog:      u32,
    enable_tonemap:  u32,
    enable_grading:  u32,
    enable_gloom:    u32,
    light_color: vec3<f32>,
    // TONE_CURVE_PROFILE or TONE_CURVE_IDENTITY (postprocess/tonemapping.wesl); the former _pad0_ slot.
    tone_curve: u32,
    atmosphere_tint: vec3<f32>,
    _pad1_: f32,
    exposure: f32,
    ambient_strength: f32,
    // land_diffuse_strength mirrors LandLightingUniforms.diffuse_strength, plumbed into
    // this shared struct so the art (flat-quad water) path can reproduce the terrain's
    // flat ambient+diffuse response without binding the land-only uniform. Occupies the
    // former _pad2_ vec2 slot (matched by offset to the Rust `_pad2_: Vec2`).
    land_diffuse_strength: f32,
    _pad2_: f32,
    // Split-tone colours, RGB only; each pads its 16-byte slot like the Rust `_pad3_` / `_pad4_`.
    grade_warm_color: vec3<f32>,
    _pad3_: f32,
    grade_cool_color: vec3<f32>,
    _pad4_: f32,
    grade_params: vec4<f32>,
    grade_extra: vec4<f32>,
    // The chroma knee after saturation (postprocess/color_grading.wesl): the knee (1 is off) and the slope above it;
    // padded like the Rust `_pad6_`.
    grade_chroma_knee: f32,
    grade_chroma_slope: f32,
    _pad6_: vec2<f32>,
    // [amount, falloff_height, shadow_bias]; padded like the Rust `_pad5_`.
    gloom_params: vec3<f32>,
    _pad5_: f32,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    gloom_color: vec4<f32>,
    fog_night_color: vec4<f32>,
    // The former trailing kr_tonemap_params (.x = KR tonemap pre-scale) is folded into exposure.
}
