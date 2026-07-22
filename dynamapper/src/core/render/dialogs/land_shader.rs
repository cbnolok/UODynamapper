// Land shader live UI (Bevy 0.16 + egui)
// - Controls LandEffectsUniform and LandLightingUniform as used by your WGSL
// - Writes to material assets so Bevy re-uploads uniforms automatically
// - Shading modes:
//      0 = Classic 2D (vertex/Gouraud; faithful to original)
//      1 = Enhanced 2D (fragment; subtle improvements, still faithful)
//      2 = KR-like     (fragment; painterly, vibrant, rim + gloom)

use super::super::scene::world::land::mesh_material::*;
use crate::core::controls::input_actions::ActionToggleShaderSettings;
use crate::{
    configs::shader_presets::{
        build_save_presets, reset_preset_to_factory, save_shader_settings,
        store_active_preset, FactoryShaderPresets, UniformState,
    },
    core::render::{dialogs, scene::camera::UiCameraResource},
    impl_tracked_plugin,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

/// Resource that tracks whether the land shader panel is visible.
/// Toggled by the keybinding; also closed when egui's own close button is used.
#[derive(Resource, Default)]
pub struct LandShaderUiState {
    pub open: bool,
    pub advanced_open: bool,
}

// Plugin that draws the UI and applies changes to materials.
pub struct LandUiPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(LandUiPlugin);

impl Plugin for LandUiPlugin {
    fn build(&self, app: &mut App) {
        // Draw UI in the egui pass
        app.init_resource::<LandShaderUiState>()
            .add_observer(sys_shader_ui_toggle)
            .add_systems(EguiPrimaryContextPass, land_ui_system)
            // Push "dirty" values into GPU materials
            .add_systems(Update, push_uniforms_if_dirty);
    }
}

fn sys_shader_ui_toggle(
    _trigger: On<ActionToggleShaderSettings>,
    mut state: ResMut<LandShaderUiState>,
) {
    state.open = !state.open;
}

/// Basic-Controls "Render Style" presets: `(label, shading_mode, post_process_profile)`.
/// Each style couples the land shading mode with the matching post-process visual
/// profile so a single pick activates the whole look — notably, KR's grime / color-wash
/// effects are gated behind `post_process_profile == KR` and would otherwise stay inert.
/// The Advanced "Visual Profile" selector can still override the profile afterward.
const RENDER_STYLE_PRESETS: [(&str, u32, u32); 3] = [
    ("Classic", 0, 0),  // vertex/Gouraud land shading + Neutral profile
    ("Enhanced", 1, 1), // fragment land shading + EC profile
    ("KR", 2, 2),       // fragment land shading + KR profile
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShaderPresetPeriod {
    Morning,
    Afternoon,
    Night,
    Cave,
}

impl ShaderPresetPeriod {
    fn from_active_key(key: &str) -> Self {
        match key.split_once('.').map(|(_, period)| period) {
            Some("afternoon") => Self::Afternoon,
            Some("night") => Self::Night,
            Some("cave") => Self::Cave,
            _ => Self::Morning,
        }
    }

    fn key(self, mode: u32) -> String {
        let mode = match mode {
            0 => "classic",
            1 => "enhanced",
            _ => "kr",
        };
        let period = match self {
            Self::Morning => "morning",
            Self::Afternoon => "afternoon",
            Self::Night => "night",
            Self::Cave => "cave",
        };
        format!("{mode}.{period}")
    }
}

fn preset_for_style(
    presets: &LandShaderModePresets,
    mode: u32,
    period: ShaderPresetPeriod,
) -> &LandMaterialUniformsPresets {
    let style = match mode {
        0 => &presets.classic,
        1 => &presets.enhanced,
        _ => &presets.kr,
    };
    match period {
        ShaderPresetPeriod::Morning => &style.morning,
        ShaderPresetPeriod::Afternoon => &style.afternoon,
        ShaderPresetPeriod::Night => &style.night,
        ShaderPresetPeriod::Cave => &style.cave,
    }
}

fn apply_render_preset(
    u: &mut UniformState,
    preset: &LandMaterialUniformsPresets,
    mode: u32,
    profile: u32,
    period: ShaderPresetPeriod,
) {
    // Animation is an interactive playback preference, not part of the visual
    // time/style preset. Keep the user's current choice when changing either.
    let enable_water_animation = u.effects.enable_water_animation;
    u.effects = preset.effects;
    u.effects.shading_mode = mode;
    u.effects.post_process_profile = profile;
    u.effects.enable_water_animation = enable_water_animation;
    u.lighting = preset.lighting;
    u.land_lighting = preset.land_lighting;
    u.global_lighting = preset.global_lighting;
    u.active_preset = period.key(mode);
    u.dirty = true;
}

fn store_active_and_apply_render_preset(
    u: &mut UniformState,
    presets: &mut LandShaderModePresets,
    mode: u32,
    profile: u32,
    period: ShaderPresetPeriod,
) {
    store_active_preset(presets, u);
    let preset = preset_for_style(presets, mode, period);
    apply_render_preset(u, preset, mode, profile, period);
}

/// Default screen position of the detached "Advanced Raw Shader Knobs" window.
/// Placed to the right of the main Land Shader Controls window (whose default
/// pos is `[16, 80]`) so the long list of low-level sliders opens as a side
/// panel instead of stretching the main window past the screen height.
const ADVANCED_WINDOW_DEFAULT_POS: [f32; 2] = [560.0, 80.0];

// ============================== UI SYSTEM ===============================
// Renders a window with controls for mode, toggles, intensities, colors,
// grading, gloom, and presets. Updates UniformState + sets "dirty" when changed.

pub fn land_ui_system(
    mut egui_contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    mut u: ResMut<UniformState>,
    mut shader_presets: ResMut<LandShaderModePresets>,
    factory_shader_presets: Res<FactoryShaderPresets>,
    settings: ResMut<Settings>,
    mut ui_state: ResMut<LandShaderUiState>,
    mut was_open: Local<bool>,
) {
    // Early return if not open. We allow one extra pass if was_open was true 
    // to ensure the "Detect close transition" logic at the end of this function runs.
    if !ui_state.open && !*was_open {
        return;
    }
    // Try to get the egui context - if it fails, skip rendering this frame
    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut egui_contexts, &egui_ui_camera) else {
        return;
    };

    let title = format!(
        "Land Shader Controls [{:?}]",
        settings.keybindings.shader_settings
    );
    let mut window_open = ui_state.open;
    let mut advanced_open = ui_state.advanced_open;

    // `open_mut()` lets the egui title-bar close button sync back to our resource.
    egui::Window::new(title)
        .default_pos([16.0, 80.0])
        .open(&mut window_open)
        .resizable(true)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label("Modes: 0=Classic (vertex), 1=Enhanced (fragment), 2=KR-like (fragment).");
            ui.label(
                "Classic aims for original fidelity. Enhanced is subtle. KR is vibrant/painterly.",
            );
            ui.add_space(6.0);

            ui.collapsing("Basic Controls", |ui| {
                let mut changed = false;

                // Render Style couples the land shading mode with the matching
                // post-process visual profile (see RENDER_STYLE_PRESETS) so one pick
                // activates the whole look, including the KR-gated effects.
                ui.horizontal(|ui| {
                    ui.strong("Render Style:");
                    let cur_mode = u.effects.shading_mode;
                    for (label, mode, profile) in RENDER_STYLE_PRESETS {
                        if ui.selectable_label(cur_mode == mode, label).clicked() {
                            let period = ShaderPresetPeriod::from_active_key(&u.active_preset);
                            store_active_and_apply_render_preset(
                                &mut u,
                                &mut shader_presets,
                                mode,
                                profile,
                                period,
                            );
                            changed = true;
                        }
                    }
                });

                // Render-style gating: controls the current style never consumes are
                // shown grayed out. Fragment features are inactive in Classic (vertex);
                // KR-land shading effects need KR shading; KR post effects need the KR
                // profile. Art (Objects) shading is independent of render style, so those
                // controls stay enabled in every style.
                let is_classic = u.effects.shading_mode == 0;
                let is_fragment = !is_classic;
                let is_kr_shading = u.effects.shading_mode == 2;
                let is_kr_profile = u.effects.post_process_profile == 2;

                // Terrain / Objects / Everything are split into collapsible
                // sub-dropdowns so the panel stays short enough to fit on screen;
                // each group's body opens with a caption of what it affects.
                basic_scope_group(ui, "Terrain (land)", "Affects land/ground tiles only.", |ui| {
                    changed |= slider_en(ui, is_fragment, "Sun Strength", &mut u.land_lighting.diffuse_strength, 0.0..=2.0);
                    changed |= slider_s(ui, "Terrain Ambient Light", &mut u.lighting.ambient_strength, 0.0..=1.5);

                    let mut land_relief = land_relief_control_value(&u);
                    if slider_en(ui, is_fragment, "Land Relief", &mut land_relief, 0.0..=1.0) {
                        apply_land_relief_control(&mut u, land_relief);
                        changed = true;
                    }
                    let mut soft_lighting = soft_lighting_control_value(&u);
                    if slider_en(ui, is_fragment, "Soft Lighting", &mut soft_lighting, 0.0..=1.0) {
                        apply_soft_lighting_control(&mut u, soft_lighting);
                        changed = true;
                    }
                    let mut highlights = highlights_control_value(&u);
                    if slider_en(ui, is_fragment, "Highlights", &mut highlights, 0.0..=1.0) {
                        apply_highlights_control(&mut u, highlights);
                        changed = true;
                    }

                    changed |= slider_en(ui, is_kr_shading, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5);
                    changed |= slider_en(ui, is_kr_shading, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5);
                    changed |= slider_en(ui, is_kr_shading, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5);
                    changed |= slider_en(ui, is_kr_profile, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5);
                    changed |= toggle_en(ui, is_kr_profile, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime);
                    changed |= slider_en(ui, is_kr_profile, "KR Transition Grime", &mut u.effects.kr_transition_grime_strength, 0.0..=1.0);

                    // Normal maps only apply to EC-atlas land textures (`texture_size == 2`
                    // in the shader); on a CC art source they are a structural no-op — the
                    // label states this since there is no global "source is EC" signal here.
                    changed |= toggle_en(ui, is_fragment, "Normal Maps (EC land only)", &mut u.effects.enable_normal_maps);
                    changed |= slider_en(ui, is_fragment, "Land Normal Maps (EC land only)", &mut u.effects.land_normal_map_strength, 0.0..=1.0);
                });

                basic_scope_group(ui, "Objects (art / statics)", "Affects static art sprites only.", |ui| {
                    changed |= slider_s(ui, "Art Shadow Shape", &mut u.effects.art_shadow_strength, 0.0..=1.0);
                    changed |= slider_s(ui, "Art Contact Shadows", &mut u.effects.art_contact_shadow_strength, 0.0..=1.0);
                    changed |= slider_s(ui, "Art Mottle", &mut u.effects.art_mottle_strength, 0.0..=1.5);
                    changed |= slider_s(ui, "Art Light Catch", &mut u.effects.art_highlight_strength, 0.0..=1.0);
                    changed |= slider_s(ui, "Art Atmosphere Tint", &mut u.effects.art_depth_tint_strength, 0.0..=1.0);
                    changed |= slider_s(ui, "KR Art Temperature", &mut u.effects.kr_art_temperature_strength, 0.0..=1.5);
                    ui.horizontal(|ui| {
                        changed |= toggle_u32(ui, "Art Projected Shadows", &mut u.effects.enable_art_projected_shadows);
                        changed |= toggle_u32(ui, "Art Fake Normals", &mut u.effects.enable_art_fake_normals);
                    });
                    changed |= slider_s(ui, "Art Projected Shadows", &mut u.effects.art_projected_shadow_strength, 0.0..=1.0);
                });

                basic_scope_group(ui, "Everything (whole scene)", "Affects both land and art / the full frame.", |ui| {
                    changed |= slider_s(ui, "Brightness", &mut u.global_lighting, 0.0..=2.0);
                    // KR's filmic tonemap pre-scales the image darker than EC/Classic;
                    // this exposes that pre-scale (higher = brighter). Only the KR profile
                    // uses it, so it is grayed out in the other styles.
                    changed |= slider_en(ui, is_kr_profile, "KR Tonemap Brightness", &mut u.lighting.kr_tonemap_params.x, 0.02..=1.0);
                    ui.horizontal(|ui| {
                        ui.label("Output:");
                        changed |= toggle_u32(ui, "Tonemap", &mut u.lighting.enable_tonemap);
                        // "Color Grading" is the master on/off for the same vibrant grading
                        // whose knobs live under Advanced → "Color Grading (Vibrant)". The
                        // excess saturation the user sees comes from those defaults, not a
                        // separate effect — the tooltip makes the relationship explicit.
                        changed |= toggle_tip(
                            ui,
                            "Color Grading",
                            &mut u.lighting.enable_grading,
                            "Master on/off for the vibrant color grading tuned under \
                             Advanced -> Color Grading (Vibrant). If colors look \
                             oversaturated, lower Saturation / Vibrance there.",
                        );
                        changed |= toggle_u32(ui, "Texture Smoothness", &mut u.effects.enable_linear_filtering);
                    });
                    ui.horizontal(|ui| {
                        changed |= toggle_u32(ui, "Grunge", &mut u.effects.enable_grunge);
                        changed |= toggle_u32(ui, "Fog", &mut u.lighting.enable_fog);
                        changed |= toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation);
                        changed |= toggle_en(ui, is_kr_profile, "KR Color Wash", &mut u.effects.enable_kr_color_wash);
                    });
                    changed |= slider_s(ui, "Grunge Strength", &mut u.effects.grunge_strength, 0.0..=1.0);
                    changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0);
                    changed |= slider_en(ui, is_kr_profile, "KR Color Wash", &mut u.effects.kr_color_wash_strength, 0.0..=1.5);
                    changed |= slider_en(ui, is_kr_profile, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0);
                });

                if changed {
                    u.dirty = true;
                }
            });

            ui.checkbox(&mut advanced_open, "Show advanced raw shader knobs (opens a side panel on the right)");

            // ------------------------ Presets -------------------------
            ui.horizontal(|ui| {
                ui.strong("Presets:");
                let mode = u.effects.shading_mode.min(2);
                let profile = mode;

                if ui.button("Morning").clicked() {
                    let period = ShaderPresetPeriod::Morning;
                    store_active_and_apply_render_preset(
                        &mut u,
                        &mut shader_presets,
                        mode,
                        profile,
                        period,
                    );
                }
                if ui.button("Afternoon").clicked() {
                    let period = ShaderPresetPeriod::Afternoon;
                    store_active_and_apply_render_preset(
                        &mut u,
                        &mut shader_presets,
                        mode,
                        profile,
                        period,
                    );
                }
                if ui.button("Night").clicked() {
                    let period = ShaderPresetPeriod::Night;
                    store_active_and_apply_render_preset(
                        &mut u,
                        &mut shader_presets,
                        mode,
                        profile,
                        period,
                    );
                }
                if ui.button("Cave").clicked() {
                    let period = ShaderPresetPeriod::Cave;
                    store_active_and_apply_render_preset(
                        &mut u,
                        &mut shader_presets,
                        mode,
                        profile,
                        period,
                    );
                }
            });

            if ui.button("Reset Current Preset to Factory Defaults").clicked() {
                let mode = u.effects.shading_mode.min(2);
                let period = ShaderPresetPeriod::from_active_key(&u.active_preset);
                let key = period.key(mode);
                reset_preset_to_factory(
                    &mut shader_presets,
                    &factory_shader_presets.0,
                    &key,
                );
                let preset = preset_for_style(&shader_presets, mode, period);
                apply_render_preset(&mut u, preset, mode, mode, period);
            }

            // Undo button: restore the last saved values.
            ui.horizontal(|ui| {
                if ui.button("⟲ Undo to Last Saved").clicked() {
                    if let Some(snap) = u.saved_snapshot {
                        u.restore(&snap);
                    }
                }
                // Show active preset label
                ui.label(format!("Active: {}", u.active_preset));
            });
        });

    // Advanced raw knobs open in their own window to the right of the main one,
    // so the long list of low-level sliders never stretches the main window past
    // the screen height. `.open()` hides this window when the checkbox is off.
    egui::Window::new("Advanced Raw Shader Knobs")
        .default_pos(ADVANCED_WINDOW_DEFAULT_POS)
        .open(&mut advanced_open)
        .resizable(true)
        .vscroll(true)
        .show(ctx, |ui| {

            // ------------------------- Effects -------------------------
            ui.collapsing("Effects", |ui| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Visual Profile:");
                    let mut profile = u.effects.post_process_profile;
                    for (label, val) in [("Neutral", 0u32), ("EC", 1u32), ("KR", 2u32)] {
                        if ui.selectable_label(profile == val, label).clicked() {
                            profile = val;
                        }
                    }
                    if profile != u.effects.post_process_profile {
                        u.effects.post_process_profile = profile;
                        changed = true;
                    }
                });
                if toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation) {
                    changed = true;
                }
                if toggle_u32(ui, "Fog", &mut u.lighting.enable_fog) {
                    changed = true;
                }
                changed |= toggle_u32(ui, "Grunge / Weathering", &mut u.effects.enable_grunge);
                changed |= slider_s(
                    ui,
                    "Grunge Strength",
                    &mut u.effects.grunge_strength,
                    0.0..=1.0,
                );
                if changed {
                    u.dirty = true;
                }
            });
 
            ui.separator();

            // --------------------- Mode & Normals ---------------------
            ui.horizontal(|ui| {
                ui.strong("Shading Mode:");
                let mut mode = u.effects.shading_mode;
                for (label, val) in [("Classic", 0u32), ("Enhanced", 1u32), ("KR", 2u32)] {
                    if ui.selectable_label(mode == val, label).clicked() {
                        mode = val;
                    }
                }
                if mode != u.effects.shading_mode {
                    u.effects.shading_mode = mode;
                    u.dirty = true; // push to GPU next frame
                }

                ui.separator();

                ui.strong("Normals:");
                // Classic always uses geometric normals — disable selector.
                if mode == 0 {
                    ui.add_enabled(false, egui::Label::new("Geometric (fixed)"));
                    if u.effects.normal_mode != 0 {
                        u.effects.normal_mode = 0;
                        u.dirty = true;
                    }
                } else {
                    let mut nm = u.effects.normal_mode;
                    if ui.selectable_label(nm == 0, "Geometric").clicked() {
                        nm = 0;
                    }
                    if ui.selectable_label(nm == 1, "Bicubic").clicked() {
                        nm = 1;
                    }
                    if nm != u.effects.normal_mode {
                        u.effects.normal_mode = nm;
                        u.dirty = true;
                    }
                }
            });

            ui.separator();

            // ------------------------- Toggles -------------------------
            // Show only toggles that are relevant to the selected shading mode.
            ui.collapsing("Toggles", |ui| {
                let mut changed = false;

                // Tonemap apply in ANY shading mode (keep available always)
                changed |= toggle_u32(ui, "Tonemap", &mut u.lighting.enable_tonemap);

                // Color grading & fragment-only features only when in fragment modes
                let is_classic = u.effects.shading_mode == 0;
                if !is_classic {
                    changed |= toggle_u32(ui, "Color Grading (fragment)", &mut u.lighting.enable_grading);
                    changed |= toggle_u32(ui, "Bent normals (fragment)", &mut u.land_lighting.enable_bent);

                    // Gloom: fragment-only semantic
                    let toggled =
                        toggle_u32(ui, "Gloom (fragment)", &mut u.lighting.enable_gloom);
                    if toggled {
                        if u.lighting.enable_gloom == 0 {
                            u.lighting.gloom_params[0] = 0.0;
                        } else if u.lighting.gloom_params[0] <= 0.0001 {
                            u.lighting.gloom_params[0] = 0.20;
                        }
                    }
                    changed |= toggled;

                    // Blur is fragment-only
                    changed |= toggle_u32(ui, "Blur (fragment)", &mut u.effects.enable_blur);
                } else {
                    // For classic path we can optionally display the state but disabled,
                    // but to keep the UI clean we simply hide fragment-only toggles here.
                }

                if changed {
                    u.dirty = true;
                }
            });

            // ------------------------ Intensities ----------------------
            // Global Lighting is a new, always-available knob that multiplies final shading.
            ui.collapsing("Intensities", |ui| {
                let mut changed = false;

                // Global scene brightness, always shown
                changed |= slider_s(
                    ui,
                    "Global Lighting (Scene Luminosity)",
                    &mut u.global_lighting,
                    0.0..=2.0,
                );

                // Ambient always shown
                changed |= slider_s(ui, "Ambient", &mut u.lighting.ambient_strength, 0.0..=1.5);

                // Diffuse is meaningful only for fragment modes. For Classic (vertex)
                // the shader uses the precomputed vertex Lambert (old behavior) and
                // we intentionally hide the diffuse control to avoid confusion.
                let is_classic = u.effects.shading_mode == 0;
                if !is_classic {
                    changed |= slider_s(ui, "Diffuse", &mut u.land_lighting.diffuse_strength, 0.0..=2.0);
                } else {
                    // Show a small label to explain why Diffuse is hidden
                    ui.label("Diffuse slider hidden in Classic mode (vertex shading).");
                }

                // Exposure lives in lighting UBO
                changed |= slider_s(
                    ui,
                    "Exposure (Tonemap)",
                    &mut u.lighting.exposure,
                    0.5..=2.0,
                );

                ui.separator();

                if !is_classic {
                    changed |= slider_s(
                        ui,
                        "Specular (fragment only)",
                        &mut u.land_lighting.specular_strength,
                        0.0..=0.4,
                    );
                    changed |= slider_s(
                        ui,
                        "Fill (env) (fragment only)",
                        &mut u.land_lighting.fill_strength,
                        0.0..=1.0,
                    );
                    ui.separator();
                    changed |= slider_s(
                        ui,
                        "Sharpness Factor (fragment only)",
                        &mut u.land_lighting.sharpness_factor,
                        0.5..=4.0,
                    );
                    changed |= slider_s(
                        ui,
                        "Sharpness Mix (fragment only)",
                        &mut u.land_lighting.sharpness_mix,
                        0.0..=1.0,
                    );
                    ui.separator();
                    // Blur parameters (fragment-only)
                    changed |= slider_s(
                        ui,
                        "Blur Strength (fragment only)",
                        &mut u.effects.blur_strength,
                        0.0..=0.5,
                    );
                    changed |= slider_s(
                        ui,
                        "Blur Radius (screen pixels) (fragment only)",
                        &mut u.effects.blur_radius,
                        0.5..=8.0,
                    );

                    changed |= slider_s(ui, "Rim (KR only)", &mut u.land_lighting.rim_strength, 0.0..=0.5);
                    changed |= slider_s(ui, "Diffuse Wrap (KR only)", &mut u.land_lighting.diffuse_wrap, 0.0..=0.5);
                } else {
                    // In Classic mode show a helper note.
                    ui.label("Fragment-only intensities hidden in Classic (vertex) mode.");
                }

                if changed {
                    u.dirty = true;
                }
            });

            // ---------------------- Lighting Colors --------------------
            ui.collapsing("Lighting Colors", |ui| {
                let mut changed = false;

                {
                    let mut v = u.lighting.light_color;
                    if color3(ui, "Light Color", &mut v) {
                        u.lighting.light_color = v;
                        changed = true;
                    }
                }
                {
                    let mut v = u.lighting.atmosphere_tint;
                    if color3(ui, "Atmosphere Tint", &mut v) {
                        u.lighting.atmosphere_tint = v;
                        changed = true;
                    }
                }

                ui.separator();

                {
                    let mut v = u.land_lighting.fill_sky_color;
                    if color4(ui, "Fill Sky (rgb + strength.a) (fragment only)", &mut v) {
                        u.land_lighting.fill_sky_color = v;
                        changed = true;
                    }
                }
                {
                    let mut v = u.land_lighting.fill_ground_color;
                    if color4(
                        ui,
                        "Fill Ground (rgb + strength.a) (fragment only)",
                        &mut v,
                    ) {
                        u.land_lighting.fill_ground_color = v;
                        changed = true;
                    }
                }

                ui.separator();

                {
                    let mut v = u.land_lighting.rim_color;
                    if color4(ui, "Rim (rgb + power.w) (fragment only)", &mut v) {
                        u.land_lighting.rim_color = v;
                        changed = true;
                    }
                }

                if changed {
                    u.dirty = true;
                }
            });

            // ----------------- KR like Color Grading (Vibrant) --------------
            ui.collapsing("Color Grading (Vibrant) (fragment only)", |ui| {
                let mut changed = false;
                if u.effects.shading_mode == 0 {
                    ui.label("Color grading is fragment-only and hidden in Classic mode.");
                } else {
                    changed |= slider_s(
                        ui,
                        "Grade Strength",
                        &mut u.lighting.grade_params[0],
                        0.0..=2.0,
                    );
                    changed |= slider_s(
                        ui,
                        "Headroom Reserve",
                        &mut u.lighting.grade_params[1],
                        0.0..=0.5,
                    );
                    changed |= slider_s(
                        ui,
                        "Fill Chroma Tint",
                        &mut u.lighting.grade_params[2],
                        0.0..=1.0,
                    );

                    // Headroom on/off via local bool to avoid overlapping borrows
                    {
                        let mut headroom_on = u.lighting.grade_params[3] >= 0.5;
                        let before = headroom_on;
                        ui.checkbox(&mut headroom_on, "Enable Headroom Limit");
                        if headroom_on != before {
                            u.lighting.grade_params[3] = if headroom_on { 1.0 } else { 0.0 };
                            changed = true;
                        }
                    }

                    ui.separator();
                    changed |= slider_s(
                        ui,
                        "Vibrance (selective sat)",
                        &mut u.lighting.grade_extra[0],
                        0.0..=1.5,
                    );
                    changed |= slider_s(
                        ui,
                        "Saturation (global)",
                        &mut u.lighting.grade_extra[1],
                        0.5..=2.0,
                    );
                    changed |= slider_s(
                        ui,
                        "Contrast (global)",
                        &mut u.lighting.grade_extra[2],
                        0.5..=2.0,
                    );
                    changed |= slider_s(
                        ui,
                        "Split Tone Strength",
                        &mut u.lighting.grade_extra[3],
                        0.0..=2.0,
                    );
                }

                if changed {
                    u.dirty = true;
                }
            });

            // ------------------------ Gloom ----------------------------
            ui.collapsing("Gloom (Moody Cool Darkening) (fragment only)", |ui| {
                let mut changed = false;
                if u.effects.shading_mode == 0 {
                    ui.label("Gloom is fragment-only and hidden in Classic mode.");
                } else {
                    changed |= slider_s(ui, "Amount", &mut u.lighting.gloom_params[0], 0.0..=1.0);
                    changed |= slider_s(
                        ui,
                        "Height Fade Height (world units)",
                        &mut u.lighting.gloom_params[1],
                        0.0..=200.0,
                    );
                    changed |= slider_s(ui, "Shadow Bias", &mut u.lighting.gloom_params[2], 0.0..=1.0);

                    ui.add_space(4.0);
                    ui.label("Fog Height Bias (continuous): -1 = valley/ground fog, 0 = neutral, +1 = high-alt haze");
                    changed |= slider_s(
                        ui,
                        "Fog Height Bias (-1..+1)",
                        &mut u.lighting.gloom_params[3],
                        -1.0..=1.0,
                    );
                }
                if changed {
                    u.dirty = true;
                }

                // Gloom color + desaturation (KR-style enhancement)
                let mut changed_color = false;
                {
                    let mut v = u.lighting.gloom_color;
                    if color4(ui, "Gloom Tint (rgb + desat.a) (KR)", &mut v) {
                        u.lighting.gloom_color = v;
                        changed_color = true;
                    }
                }
                if changed_color {
                    u.dirty = true;
                }
            });

            // ------------------------ Graphics ------------------------------
            ui.collapsing("Graphics Settings", |ui| {
                let mut changed = false;

                changed |= toggle_u32(ui, "Linear Filtering", &mut u.effects.enable_linear_filtering);
                changed |= slider_s(ui, "Sharpening Amount", &mut u.effects.sharpening_amount, 0.0..=2.0);
                changed |= toggle_u32(ui, "Texture Normal Maps", &mut u.effects.enable_normal_maps);
                changed |= toggle_u32(ui, "Art Fake Normals", &mut u.effects.enable_art_fake_normals);
                changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0);
                changed |= slider_s(ui, "Static Light Decals", &mut u.effects.static_light_decal_visibility, 0.0..=2.0);
                changed |= slider_s(ui, "Art Shadow Shape", &mut u.effects.art_shadow_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Contact Shadows", &mut u.effects.art_contact_shadow_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Mottle", &mut u.effects.art_mottle_strength, 0.0..=1.5);
                changed |= slider_s(ui, "Art Light Catch", &mut u.effects.art_highlight_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Atmosphere Tint", &mut u.effects.art_depth_tint_strength, 0.0..=1.0);
                changed |= slider_s(ui, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5);
                changed |= toggle_u32(ui, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime);
                changed |= slider_s(ui, "KR Transition Grime Strength", &mut u.effects.kr_transition_grime_strength, 0.0..=1.0);
                changed |= toggle_u32(ui, "KR Color Wash", &mut u.effects.enable_kr_color_wash);
                changed |= slider_s(ui, "KR Color Wash Strength", &mut u.effects.kr_color_wash_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0);
                changed |= slider_s(ui, "KR Art Temperature", &mut u.effects.kr_art_temperature_strength, 0.0..=1.5);
                changed |= slider_s(ui, "Land Normal Maps", &mut u.effects.land_normal_map_strength, 0.0..=1.0);
                changed |= toggle_u32(ui, "Art Projected Shadows", &mut u.effects.enable_art_projected_shadows);
                changed |= slider_s(ui, "Art Projected Shadow Strength", &mut u.effects.art_projected_shadow_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Projected Shadow Length", &mut u.effects.art_projected_shadow_length, 0.0..=1.5);
                changed |= slider_s(ui, "Art Projected Shadow Softness", &mut u.effects.art_projected_shadow_softness, 0.05..=1.0);

                if changed {
                    u.dirty = true;
                }
            });

            // ------------------------ Fog ------------------------------
            ui.collapsing("Fog Params", |ui| {
                let mut changed = false;
                // Tint + max mix (alpha)
                let mut fog = u.lighting.fog_color;
                if color4(ui, "Fog Color (alpha = max mix)", &mut fog) {
                    u.lighting.fog_color = fog;
                    changed = true;
                }
                // Densities and noise
                changed |= slider_s(
                    ui,
                    "Distance Density",
                    &mut u.lighting.fog_params[0],
                    0.0..=0.2,
                );
                changed |= slider_s(
                    ui,
                    "Height Density",
                    &mut u.lighting.fog_params[1],
                    0.0..=0.2,
                );
                changed |= slider_s(ui, "Noise Scale", &mut u.lighting.fog_params[2], 0.0..=2.0);
                changed |= slider_s(
                    ui,
                    "Noise Strength",
                    &mut u.lighting.fog_params[3],
                    0.0..=1.0,
                );
                if changed {
                    u.dirty = true;
                }

                // Night fog tint (KR-style enhancement)
                let mut changed_night = false;
                {
                    let mut v = u.lighting.fog_night_color;
                    if color4(ui, "Night Fog Tint (rgb + blend.a)", &mut v) {
                        u.lighting.fog_night_color = v;
                        changed_night = true;
                    }
                }
                if changed_night {
                    u.dirty = true;
                }
            });

        });

    ui_state.open = window_open;
    ui_state.advanced_open = advanced_open;

    // Detect close transition: save shader settings to land.toml,
    // but only if values actually changed from the last saved snapshot.
    if *was_open && !ui_state.open {
        /*
        let values_changed = match u.saved_snapshot {
            Some(snap) => {
                // Compare current live values against the last-saved snapshot.
                // If any field differs, we need to persist.
                u.effects != snap.effects
                    || u.lighting != snap.lighting
                    || u.land_lighting != snap.land_lighting
                    || (u.global_lighting - snap.global_lighting).abs() > f32::EPSILON
            }
            // No previous save — treat as changed so we persist the initial state.
            None => true,
        };
        if values_changed {
        */
        let save_presets = build_save_presets(&u, &shader_presets);
        save_shader_settings(&save_presets);
        u.saved_snapshot = Some(u.snapshot());
        //}
    }
    *was_open = ui_state.open;
}

// push_uniforms_if_dirty updates ALL LandCustomMaterial assets.
// That guarantees that materials not referenced this frame still get the new values
// (fixes "stale lighting when moving" problem).
fn push_uniforms_if_dirty(
    mut mats: ResMut<Assets<LandCustomMeshMaterial>>,
    _q_mat_handles: Query<&MeshMaterial3d<LandCustomMeshMaterial>>, // kept for parity; unused
    mut u: ResMut<UniformState>,
) {
    if !u.dirty {
        return;
    }

    for (_handle, mat) in mats.iter_mut() {
        // Overwrite the embedded uniforms used by the material extension.
        mat.extension.effects_uniform = u.effects;
        mat.extension.global_lighting_uniform = u.lighting;
        mat.extension.land_lighting_uniform = u.land_lighting;

        // NEW: write global lighting into the land uniform so shader sees it
        // NOTE: adjust the path if your extension uses a different name for the land UBO.
        mat.extension.scene_uniform.global_lighting = u.global_lighting;
    }

    u.dirty = false;
}

// ============================ UI HELPERS =================================
// These helpers return "changed" (bool) so callers can set u.dirty |= changed,
// avoiding overlapping &mut borrows inside the helper.

// Toggle a u32 field as a boolean (0/1). Returns true if the value changed.
/// Collapsible scope group inside Basic Controls: a titled sub-dropdown whose
/// body opens with a small caption stating which part of the render it affects.
/// Splitting the many controls into per-scope dropdowns keeps the panel from
/// growing taller than the screen. Defaults to open so nothing is hidden on
/// first show; the user collapses the groups they don't need.
fn basic_scope_group(
    ui: &mut egui::Ui,
    title: &str,
    scope: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(title)
        .default_open(true)
        .show(ui, |ui| {
            ui.small(scope);
            add_contents(ui);
        });
}

fn toggle_u32(ui: &mut egui::Ui, label: &str, val: &mut u32) -> bool {
    let mut b = *val != 0;
    let changed = ui.checkbox(&mut b, label).changed();
    if changed {
        *val = if b { 1 } else { 0 };
    }
    changed
}

// Slider for f32 values in the given range. Returns true if changed.
fn slider_s(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add(egui::Slider::new(val, range).text(label)).changed()
}

// Like `slider_s`, but grayed out (non-interactive) when `enabled` is false —
// used to show controls that the current render style does not consume.
fn slider_en(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add_enabled(enabled, egui::Slider::new(val, range).text(label))
        .changed()
}

// Like `toggle_u32`, but shows an explanatory tooltip on hover.
fn toggle_tip(ui: &mut egui::Ui, label: &str, val: &mut u32, tip: &str) -> bool {
    let mut b = *val != 0;
    let changed = ui.checkbox(&mut b, label).on_hover_text(tip).changed();
    if changed {
        *val = if b { 1 } else { 0 };
    }
    changed
}

// Like `toggle_u32`, but grayed out when `enabled` is false.
fn toggle_en(ui: &mut egui::Ui, enabled: bool, label: &str, val: &mut u32) -> bool {
    let mut b = *val != 0;
    let changed = ui
        .add_enabled(enabled, egui::Checkbox::new(&mut b, label))
        .changed();
    if changed {
        *val = if b { 1 } else { 0 };
    }
    changed
}

// Edit a Vec3 as RGB (0..1). Returns true if changed.
fn color3(ui: &mut egui::Ui, label: &str, v: &mut Vec3) -> bool {
    let mut changed = false;
    let mut arr = v.to_array();
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.color_edit_button_rgb(&mut arr).changed() {
            changed = true;
        }
    });
    if changed {
        *v = Vec3::from_array(arr);
    }
    changed
}

// Edit a Vec4 as RGBA (0..1). Returns true if changed.
fn color4(ui: &mut egui::Ui, label: &str, v: &mut Vec4) -> bool {
    let mut changed = false;
    let mut arr = v.to_array();
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.color_edit_button_rgba_unmultiplied(&mut arr).changed() {
            changed = true;
        }
    });
    if changed {
        *v = Vec4::from_array(arr);
    }
    changed
}

fn land_relief_control_value(u: &UniformState) -> f32 {
    if u.effects.normal_mode == 0 {
        0.0
    } else {
        (u.land_lighting.sharpness_mix / 0.55).clamp(0.0, 1.0)
    }
}

fn apply_land_relief_control(u: &mut UniformState, land_relief: f32) {
    let land_relief = land_relief.clamp(0.0, 1.0);
    u.effects.normal_mode = if land_relief > 0.15 { 1 } else { 0 };
    u.land_lighting.enable_bent = if land_relief > 0.55 { 1 } else { 0 };
    u.land_lighting.sharpness_mix = (land_relief * 0.55).clamp(0.0, 0.55);
    u.land_lighting.sharpness_factor = (1.0 + land_relief * 1.5).clamp(1.0, 3.0);
}

fn soft_lighting_control_value(u: &UniformState) -> f32 {
    u.land_lighting.fill_strength.clamp(0.0, 1.0)
}

fn apply_soft_lighting_control(u: &mut UniformState, soft_lighting: f32) {
    let soft_lighting = soft_lighting.clamp(0.0, 1.0);
    u.land_lighting.fill_strength = soft_lighting;
    u.land_lighting.diffuse_wrap = (soft_lighting * 0.35).clamp(0.0, 0.5);
}

fn highlights_control_value(u: &UniformState) -> f32 {
    let from_rim = u.land_lighting.rim_strength / 0.45;
    let from_specular = u.land_lighting.specular_strength / 0.25;
    ((from_rim + from_specular) * 0.5).clamp(0.0, 1.0)
}

fn apply_highlights_control(u: &mut UniformState, highlights: f32) {
    let highlights = highlights.clamp(0.0, 1.0);
    u.land_lighting.rim_strength = highlights * 0.45;
    u.land_lighting.specular_strength = highlights * 0.25;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_state() -> UniformState {
        UniformState {
            effects: LandEffectsUniform::default(),
            lighting: GlobalLightingUniforms::default(),
            land_lighting: LandLightingUniforms::default(),
            global_lighting: 1.0,
            dirty: false,
            saved_snapshot: None,
            active_preset: "classic.morning".to_string(),
        }
    }

    fn preset(global_lighting: f32) -> LandMaterialUniformsPresets {
        LandMaterialUniformsPresets {
            global_lighting,
            effects: LandEffectsUniform::default(),
            lighting: GlobalLightingUniforms::default(),
            land_lighting: LandLightingUniforms::default(),
        }
    }

    fn mode(base: f32) -> LandRenderStylePresetsPerMode {
        LandRenderStylePresetsPerMode {
            morning: preset(base),
            afternoon: preset(base + 1.0),
            night: preset(base + 2.0),
            cave: preset(base + 3.0),
        }
    }

    #[test]
    fn basic_composite_controls_roundtrip_their_slider_values() {
        let mut u = uniform_state();

        apply_land_relief_control(&mut u, 0.8);
        assert_eq!(land_relief_control_value(&u), 0.8);

        apply_soft_lighting_control(&mut u, 0.7);
        assert_eq!(soft_lighting_control_value(&u), 0.7);

        apply_highlights_control(&mut u, 0.6);
        assert!((highlights_control_value(&u) - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn render_presets_preserve_animation_and_enforce_the_selected_style() {
        let mut u = uniform_state();
        u.effects.enable_water_animation = 0;
        let preset = LandMaterialUniformsPresets {
            global_lighting: 0.75,
            effects: LandEffectsUniform {
                shading_mode: 2,
                post_process_profile: 2,
                enable_water_animation: 1,
                ..Default::default()
            },
            lighting: GlobalLightingUniforms {
                exposure: 1.25,
                ..Default::default()
            },
            land_lighting: LandLightingUniforms {
                diffuse_strength: 0.8,
                ..Default::default()
            },
        };

        apply_render_preset(
            &mut u,
            &preset,
            0,
            0,
            ShaderPresetPeriod::Morning,
        );

        assert_eq!(u.effects.shading_mode, 0);
        assert_eq!(u.effects.post_process_profile, 0);
        assert_eq!(u.effects.enable_water_animation, 0);
        assert_eq!(u.active_preset, "classic.morning");
        assert_eq!(u.global_lighting, 0.75);
        assert_eq!(u.lighting.exposure, 1.25);
        assert_eq!(u.land_lighting.diffuse_strength, 0.8);
        assert!(u.dirty);
    }

    #[test]
    fn render_style_changes_keep_the_active_time_period() {
        let period = ShaderPresetPeriod::from_active_key("kr.cave");

        assert_eq!(period, ShaderPresetPeriod::Cave);
        assert_eq!(period.key(0), "classic.cave");
        assert_eq!(period.key(1), "enhanced.cave");
    }

    #[test]
    fn changing_presets_captures_live_values_before_loading_the_target() {
        let mut u = uniform_state();
        let mut presets = LandShaderModePresets {
            classic: mode(1.0),
            enhanced: mode(5.0),
            kr: mode(9.0),
            default_preset: "classic.morning".to_string(),
        };
        u.global_lighting = 0.42;

        store_active_and_apply_render_preset(
            &mut u,
            &mut presets,
            1,
            1,
            ShaderPresetPeriod::Afternoon,
        );

        assert_eq!(presets.classic.morning.global_lighting, 0.42);
        assert_eq!(u.active_preset, "enhanced.afternoon");
        assert_eq!(u.global_lighting, 6.0);
    }
}
