// Land shader live UI (Bevy 0.16 + egui)
// - Controls LandEffectsUniform and LandLightingUniform as used by your WGSL
// - Writes to material assets so Bevy re-uploads uniforms automatically
// - Shading modes:
//      0 = Classic 2D (vertex/Gouraud; faithful to original)
//      1 = Enhanced 2D (fragment; subtle improvements, still faithful)
//      2 = KR-like     (fragment; painterly, vibrant, rim + gloom)
// - Render styles pick a preset group: Classic, Enhanced, KR-like (the faithful KR client
//   look) and UOD-Painterly (the painterly KR look). KR-like and UOD-Painterly both use
//   shading mode 2 and the KR content; only their preset values differ.

use super::super::scene::world::land::mesh_material::*;
use crate::core::controls::input_actions::ActionToggleShaderSettings;
use crate::{
    configs::shader_presets::{
        build_save_presets, get_preset_slot, reset_preset_to_factory, save_shader_settings,
        store_active_preset, FactoryShaderPresets, UniformState, CLASSIC_PRESET_GROUP,
        ENHANCED_PRESET_GROUP, KR_PRESET_GROUP, PAINTERLY_PRESET_GROUP,
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

/// One Basic-Controls "Render Style": the preset group it loads, its land shading mode and
/// post-process visual profile, and its tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RenderStylePreset {
    label: &'static str,
    preset_group: &'static str,
    shading_mode: u32,
    post_process_profile: u32,
    tooltip: &'static str,
}

/// Basic-Controls "Render Style" presets.
/// Each style couples the land shading mode with the matching post-process visual
/// profile so a single pick activates the whole look — notably, KR's grime / color-wash
/// effects are gated behind `post_process_profile == KR` and would otherwise stay inert.
/// The Advanced "Visual Profile" selector can still override the profile afterward.
/// KR-like and UOD-Painterly share the KR mode, profile and content; their preset groups differ.
const RENDER_STYLE_PRESETS: [RenderStylePreset; 4] = [
    // vertex/Gouraud land shading + Neutral profile
    RenderStylePreset {
        label: "Classic",
        preset_group: CLASSIC_PRESET_GROUP,
        shading_mode: 0,
        post_process_profile: 0,
        tooltip: "Vertex shading. Aims for original fidelity.",
    },
    // fragment land shading + EC profile
    RenderStylePreset {
        label: "Enhanced",
        preset_group: ENHANCED_PRESET_GROUP,
        shading_mode: 1,
        post_process_profile: 1,
        tooltip: "Fragment shading. Subtle improvements.",
    },
    // fragment land shading + KR profile, client terms only
    RenderStylePreset {
        label: "KR-like",
        preset_group: KR_PRESET_GROUP,
        shading_mode: 2,
        post_process_profile: 2,
        tooltip: "KR content drawn the way the KR client draws it.",
    },
    // fragment land shading + KR profile, every effect
    RenderStylePreset {
        label: "UOD-Painterly",
        preset_group: PAINTERLY_PRESET_GROUP,
        shading_mode: 2,
        post_process_profile: 2,
        tooltip: "KR content, fragment shading. Vibrant and painterly.",
    },
];

/// The render style whose preset group the active preset key names. A key from no known
/// group falls back to the first style with the live shading mode.
fn active_render_style(u: &UniformState) -> RenderStylePreset {
    let group = u.active_preset.split_once('.').map_or(u.active_preset.as_str(), |(group, _)| group);
    RENDER_STYLE_PRESETS
        .into_iter()
        .find(|style| style.preset_group == group)
        .or_else(|| RENDER_STYLE_PRESETS.into_iter().find(|style| style.shading_mode == u.effects.shading_mode))
        .unwrap_or(RENDER_STYLE_PRESETS[0])
}

/// Whole-scene "Tone Curve" choices under the Tonemap toggle: `(label, tone_curve, tooltip)`.
const TONE_CURVE_OPTIONS: [(&str, u32, &str); 2] = [
    ("Profile", TONE_CURVE_PROFILE, "The visual profile's curve (Reinhard, EC or KR filmic), exposure inside it."),
    ("Identity", TONE_CURVE_IDENTITY, "No curve, as the clients draw: exposure is plain brightness."),
];

/// Terrain "Land Shading" choices for the fragment styles: `(label, terrain_shading_model, tooltip)`.
const TERRAIN_SHADING_OPTIONS: [(&str, u32, &str); 2] = [
    ("Smooth per-pixel", TERRAIN_SHADING_SMOOTH_PER_PIXEL, "Lighting per pixel from a smooth normal, with every land lighting control."),
    ("Client per-vertex", TERRAIN_SHADING_CLIENT_PER_VERTEX, "As the KR and Enhanced clients: a half-Lambert grey per cell corner, blended over two triangles."),
];

/// Whole-scene "Texture Filter" choices for land and sprites: `(label, texture_filter, tooltip)`.
const TEXTURE_FILTER_OPTIONS: [(&str, u32, &str); 2] = [
    ("Pixel art", TEXTURE_FILTER_PIXEL_ART, "Nearest texels; Texture Smoothness softens only their seams."),
    ("Client bilinear", TEXTURE_FILTER_CLIENT_BILINEAR, "As the KR client: plain bilinear on land and sprites. Mip-like averaging only when zoomed out past its 18.4-tile view."),
];

/// "Sun Azimuth" slider: a full compass turn in degrees, clockwise from north.
const SUN_AZIMUTH_RANGE_DEGREES: std::ops::RangeInclusive<f32> = 0.0..=360.0;
/// "Sun Elevation" slider: from the horizon to overhead, in degrees.
const SUN_ELEVATION_RANGE_DEGREES: std::ops::RangeInclusive<f32> = 0.0..=90.0;

const PRESET_PERIOD_BUTTONS: [(&str, ShaderPresetPeriod); 4] = [
    ("Morning", ShaderPresetPeriod::Morning),
    ("Afternoon", ShaderPresetPeriod::Afternoon),
    ("Night", ShaderPresetPeriod::Night),
    ("Cave", ShaderPresetPeriod::Cave),
];

// The terrain column and both art columns need this content width to sit side by side.
const SHADER_WINDOW_DEFAULT_POS: [f32; 2] = [16.0, 80.0];
const SHADER_WINDOW_DEFAULT_WIDTH: f32 = 960.0;
// Tall enough to open with every section and the buttons below them visible, without scrolling.
const SHADER_WINDOW_DEFAULT_HEIGHT: f32 = 580.0;
const THREE_COLUMN_MIN_WIDTH: f32 = 900.0;
const TWO_COLUMN_MIN_WIDTH: f32 = 600.0;
const SECTION_SPACING: f32 = 8.0;

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

    fn key(self, preset_group: &str) -> String {
        let period = match self {
            Self::Morning => "morning",
            Self::Afternoon => "afternoon",
            Self::Night => "night",
            Self::Cave => "cave",
        };
        format!("{preset_group}.{period}")
    }
}

fn preset_for_style(
    presets: &LandShaderModePresets,
    style: RenderStylePreset,
    period: ShaderPresetPeriod,
) -> &LandMaterialUniformsPresets {
    // Every RENDER_STYLE_PRESETS group is a field of LandShaderModePresets (unit-tested).
    get_preset_slot(presets, &period.key(style.preset_group)).unwrap_or(&presets.classic.morning)
}

fn apply_render_preset(
    u: &mut UniformState,
    preset: &LandMaterialUniformsPresets,
    style: RenderStylePreset,
    period: ShaderPresetPeriod,
) {
    let (mode, profile) = (style.shading_mode, style.post_process_profile);
    // Animation is an interactive playback preference, not part of the visual
    // time/style preset. Keep the user's current choice when changing either.
    let enable_water_animation = u.effects.enable_water_animation;
    let terrain_transition_flags = u.effects.terrain_transition_flags;
    u.effects = preset.effects;
    u.effects.shading_mode = mode;
    u.effects.post_process_profile = profile;
    u.effects.enable_water_animation = enable_water_animation;
    u.effects.terrain_transition_flags = terrain_transition_flags;
    u.lighting = preset.lighting;
    u.land_lighting = preset.land_lighting;
    u.global_lighting = preset.global_lighting;
    u.sun = preset.sun;
    u.active_preset = period.key(style.preset_group);
    u.dirty = true;
}

fn store_active_and_apply_render_preset(
    u: &mut UniformState,
    presets: &mut LandShaderModePresets,
    style: RenderStylePreset,
    period: ShaderPresetPeriod,
) {
    store_active_preset(presets, u);
    let preset = preset_for_style(presets, style, period);
    apply_render_preset(u, preset, style, period);
}

/// Default screen position of the detached "Advanced Raw Shader Knobs" window.
/// Placed to the right of the main Land Shader Controls window (whose default
/// pos is `SHADER_WINDOW_DEFAULT_POS`) so the long list of low-level sliders opens as a side
/// panel instead of stretching the main window past the screen height.
const ADVANCED_WINDOW_DEFAULT_POS: [f32; 2] = [
    SHADER_WINDOW_DEFAULT_POS[0] + SHADER_WINDOW_DEFAULT_WIDTH + ADVANCED_WINDOW_GAP,
    SHADER_WINDOW_DEFAULT_POS[1],
];
const ADVANCED_WINDOW_GAP: f32 = 12.0;

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
    let main_window = egui::Window::new(title)
        .default_pos(SHADER_WINDOW_DEFAULT_POS)
        .default_size([SHADER_WINDOW_DEFAULT_WIDTH, SHADER_WINDOW_DEFAULT_HEIGHT])
        .open(&mut window_open)
        .resizable(true)
        .vscroll(true)
        .show(ctx, |ui| {
            // Render Style and time of day pick the preset, so they share the top row.
            ui.horizontal_wrapped(|ui| {
                // Render Style couples the land shading mode with the matching
                // post-process visual profile (see RENDER_STYLE_PRESETS) so one pick
                // activates the whole look, including the KR-gated effects.
                ui.strong("Render Style:");
                let cur_style = active_render_style(&u);
                let cur_period = ShaderPresetPeriod::from_active_key(&u.active_preset);
                for style in RENDER_STYLE_PRESETS {
                    if ui.selectable_label(cur_style == style, style.label).on_hover_text(style.tooltip).clicked() {
                        store_active_and_apply_render_preset(
                            &mut u,
                            &mut shader_presets,
                            style,
                            cur_period,
                        );
                    }
                }

                ui.separator();

                // ------------------------ Presets -------------------------
                ui.strong("Time of Day:");
                for (label, period) in PRESET_PERIOD_BUTTONS {
                    if ui.selectable_label(cur_period == period, label).clicked() {
                        store_active_and_apply_render_preset(
                            &mut u,
                            &mut shader_presets,
                            cur_style,
                            period,
                        );
                    }
                }
            });
            ui.add_space(SECTION_SPACING);

            // Render-style gating: controls the current style never consumes are
            // shown grayed out. Fragment features are inactive in Classic (vertex);
            // KR-land shading effects need KR shading; KR post effects need the KR
            // profile. Art (Objects) shading is independent of render style, so those
            // controls stay enabled in every style.
            let gates = RenderStyleGates {
                is_fragment: u.effects.shading_mode != 0,
                is_kr_shading: u.effects.shading_mode == 2,
                is_kr_profile: u.effects.post_process_profile == 2,
            };

            // Everything is visible at once, and each section is what a control acts on:
            // the whole frame, then land, then the two kinds of art side by side.
            let mut changed = whole_scene_controls(ui, &mut u, gates);
            ui.add_space(SECTION_SPACING);
            ui.separator();

            let width = ui.available_width();
            if width >= THREE_COLUMN_MIN_WIDTH {
                ui.columns(3, |columns| {
                    changed |= terrain_controls(&mut columns[0], &mut u, gates);
                    changed |= standard_art_controls(&mut columns[1], &mut u.effects);
                    changed |= stylized_art_controls(&mut columns[2], &mut u.effects);
                });
            } else if width >= TWO_COLUMN_MIN_WIDTH {
                ui.columns(2, |columns| {
                    changed |= terrain_controls(&mut columns[0], &mut u, gates);
                    changed |= standard_art_controls(&mut columns[1], &mut u.effects);
                    columns[1].add_space(SECTION_SPACING);
                    changed |= stylized_art_controls(&mut columns[1], &mut u.effects);
                });
            } else {
                changed |= terrain_controls(ui, &mut u, gates);
                ui.add_space(SECTION_SPACING);
                changed |= standard_art_controls(ui, &mut u.effects);
                ui.add_space(SECTION_SPACING);
                changed |= stylized_art_controls(ui, &mut u.effects);
            }

            if changed {
                u.dirty = true;
            }

            ui.add_space(SECTION_SPACING);
            ui.separator();
            ui.checkbox(&mut advanced_open, "Show advanced raw shader knobs (opens a side panel on the right)");

            if ui.button("Reset Current Preset to Factory Defaults").clicked() {
                let style = active_render_style(&u);
                let period = ShaderPresetPeriod::from_active_key(&u.active_preset);
                let key = period.key(style.preset_group);
                reset_preset_to_factory(
                    &mut shader_presets,
                    &factory_shader_presets.0,
                    &key,
                );
                let preset = preset_for_style(&shader_presets, style, period);
                apply_render_preset(&mut u, preset, style, period);
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
    let advanced_position = main_window.as_ref().map(|window|
        [window.response.rect.right() + ADVANCED_WINDOW_GAP, window.response.rect.top()]
    ).unwrap_or(ADVANCED_WINDOW_DEFAULT_POS);
    egui::Window::new("Advanced Raw Shader Knobs")
        .default_pos(advanced_position)
        // Set the area's initial width too: egui otherwise constrains its
        // larger default area before measuring this window, moving it left.
        .default_width(360.0)
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
                ui.separator();
                ui.strong("Whole scene");
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
                ui.separator();
                ui.strong("Water surfaces (terrain and statics)");
                changed |= toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation);
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
            });
            ui.horizontal(|ui| {
                let mode = u.effects.shading_mode;
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

                ui.strong("Whole scene");
                changed |= toggle_u32(ui, "Linear Filtering", &mut u.effects.enable_linear_filtering);
                changed |= texture_filter_row(ui, &mut u);
                changed |= slider_s(ui, "Sharpening Amount", &mut u.effects.sharpening_amount, 0.0..=2.0);
                changed |= toggle_u32(ui, "KR Color Wash", &mut u.effects.enable_kr_color_wash);
                changed |= slider_s(ui, "KR Color Wash Strength", &mut u.effects.kr_color_wash_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0);
                changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0);

                ui.separator();
                ui.strong("Terrain");
                changed |= toggle_u32(ui, "Texture Normal Maps", &mut u.effects.enable_normal_maps);
                changed |= slider_s(ui, "Land Normal Maps", &mut u.effects.land_normal_map_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Static Light Decals", &mut u.effects.static_light_decal_visibility, 0.0..=2.0);
                changed |= slider_s(ui, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5);
                changed |= slider_s(ui, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5);
                changed |= toggle_u32(ui, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime);
                changed |= slider_s(ui, "KR Transition Grime Strength", &mut u.effects.kr_transition_grime_strength, 0.0..=1.0);

                ui.separator();
                ui.strong("Objects (art / statics)");
                changed |= toggle_u32(ui, "Art Fake Normals", &mut u.effects.enable_art_fake_normals);
                changed |= slider_s(ui, "Art Shadow Shape", &mut u.effects.art_shadow_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Contact Shadows", &mut u.effects.art_contact_shadow_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Mottle", &mut u.effects.art_mottle_strength, 0.0..=1.5);
                changed |= slider_s(ui, "Art Light Catch", &mut u.effects.art_highlight_strength, 0.0..=1.0);
                changed |= slider_s(ui, "Art Atmosphere Tint", &mut u.effects.art_depth_tint_strength, 0.0..=1.0);
                changed |= slider_s(ui, "KR Art Temperature", &mut u.effects.kr_art_temperature_strength, 0.0..=1.5);
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
    settings: Res<Settings>,
) {
    let transitions = settings.graphics.terrain_transition_flags();
    if u.effects.terrain_transition_flags != transitions {
        u.effects.terrain_transition_flags = transitions;
        u.dirty = true;
    }
    if !u.dirty {
        return;
    }

    for (_handle, mat) in mats.iter_mut() {
        // Overwrite the embedded uniforms used by the material extension.
        mat.extension.effects_uniform = u.effects;
        mat.extension.global_lighting_uniform = u.lighting;
        mat.extension.land_lighting_uniform = u.land_lighting;
        mat.extension.scene_uniform.light_direction = u.sun.light_direction();

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
/// Which controls the active render style consumes.
#[derive(Clone, Copy)]
struct RenderStyleGates {
    is_fragment: bool,
    is_kr_shading: bool,
    is_kr_profile: bool,
}

/// Titled section of the basic controls, with a small caption stating which part of the
/// render it affects.
fn section_header(ui: &mut egui::Ui, title: &str, scope: &str) {
    ui.strong(title);
    ui.small(scope);
}

fn whole_scene_controls(ui: &mut egui::Ui, u: &mut UniformState, gates: RenderStyleGates) -> bool {
    section_header(ui, "Whole scene", "Affects both land and art / the full frame.");
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
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
        changed |= toggle_tip(
            ui,
            "Texture Smoothness",
            &mut u.effects.enable_linear_filtering,
            "Softens texel seams in the pixel-art filter. Liquids and item surfaces follow it under either filter.",
        );
        changed |= toggle_u32(ui, "Fog", &mut u.lighting.enable_fog);
    });
    changed |= texture_filter_row(ui, u);
    ui.horizontal_wrapped(|ui| {
        let enabled = u.lighting.enable_tonemap != 0;
        ui.add_enabled(enabled, egui::Label::new("Tone Curve:"));
        for (label, curve, tip) in TONE_CURVE_OPTIONS {
            let selected = u.lighting.tone_curve == curve;
            if ui.add_enabled(enabled, egui::Button::selectable(selected, label)).on_hover_text(tip).clicked() && !selected {
                u.lighting.tone_curve = curve;
                changed = true;
            }
        }
    });
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        changed |= slider_s(ui, "Brightness", &mut u.global_lighting, 0.0..=2.0);
        // KR's filmic tonemap pre-scales the image darker than EC/Classic;
        // this exposes that pre-scale (higher = brighter). Only the KR profile
        // uses it, so it is grayed out in the other styles.
        changed |= slider_en(ui, gates.is_kr_profile, "KR Tonemap Brightness", &mut u.lighting.kr_tonemap_params.x, 0.02..=1.0);
        changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0);

        let ui = &mut columns[1];
        changed |= toggle_slider(ui, true, "Grunge", &mut u.effects.enable_grunge, &mut u.effects.grunge_strength, 0.0..=1.0);
        changed |= toggle_slider(ui, gates.is_kr_profile, "KR Color Wash", &mut u.effects.enable_kr_color_wash, &mut u.effects.kr_color_wash_strength, 0.0..=1.5);
        changed |= slider_en(ui, gates.is_kr_profile, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0);
    });
    changed
}

fn texture_filter_row(ui: &mut egui::Ui, u: &mut UniformState) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Texture Filter:");
        for (label, filter, tip) in TEXTURE_FILTER_OPTIONS {
            let selected = u.effects.texture_filter == filter;
            if ui.add(egui::Button::selectable(selected, label)).on_hover_text(tip).clicked() && !selected {
                u.effects.texture_filter = filter;
                changed = true;
            }
        }
    });
    changed
}

fn terrain_controls(ui: &mut egui::Ui, u: &mut UniformState, gates: RenderStyleGates) -> bool {
    section_header(ui, "Terrain and water", "Land tiles. Water also covers static water surfaces.");
    let mut changed = toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation);
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled(gates.is_fragment, egui::Label::new("Land Shading:"));
        for (label, model, tip) in TERRAIN_SHADING_OPTIONS {
            let selected = u.land_lighting.terrain_shading_model == model;
            if ui.add_enabled(gates.is_fragment, egui::Button::selectable(selected, label)).on_hover_text(tip).clicked() && !selected {
                u.land_lighting.terrain_shading_model = model;
                changed = true;
            }
        }
    });
    // The client per-vertex model shades land with its grey alone; these controls only reach the per-pixel model.
    let per_pixel = gates.is_fragment && u.land_lighting.terrain_shading_model == TERRAIN_SHADING_SMOOTH_PER_PIXEL;
    changed |= slider_en(ui, gates.is_fragment, "Sun Strength", &mut u.land_lighting.diffuse_strength, 0.0..=2.0);
    changed |= slider_tip(ui, "Sun Azimuth", &mut u.sun.azimuth_degrees, SUN_AZIMUTH_RANGE_DEGREES,
        "Compass bearing of the sun in degrees, clockwise from north (180 = south). Lights land and art and turns the sprite shadows.");
    changed |= slider_tip(ui, "Sun Elevation", &mut u.sun.elevation_degrees, SUN_ELEVATION_RANGE_DEGREES,
        "Height of the sun above the horizon in degrees (90 = overhead).");
    changed |= slider_s(ui, "Terrain Ambient Light", &mut u.lighting.ambient_strength, 0.0..=1.5);

    let mut land_relief = land_relief_control_value(u);
    if slider_en(ui, per_pixel, "Land Relief", &mut land_relief, 0.0..=1.0) {
        apply_land_relief_control(u, land_relief);
        changed = true;
    }
    let mut soft_lighting = soft_lighting_control_value(u);
    if slider_en(ui, per_pixel, "Soft Lighting", &mut soft_lighting, 0.0..=1.0) {
        apply_soft_lighting_control(u, soft_lighting);
        changed = true;
    }
    let mut highlights = highlights_control_value(u);
    if slider_en(ui, per_pixel, "Highlights", &mut highlights, 0.0..=1.0) {
        apply_highlights_control(u, highlights);
        changed = true;
    }

    let kr_per_pixel = gates.is_kr_shading && per_pixel;
    changed |= slider_en(ui, kr_per_pixel, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5);
    changed |= slider_en(ui, kr_per_pixel, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5);
    changed |= slider_en(ui, kr_per_pixel, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5);
    changed |= slider_en(ui, gates.is_kr_profile, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5);
    changed |= toggle_slider(ui, gates.is_kr_profile, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime, &mut u.effects.kr_transition_grime_strength, 0.0..=1.0);

    // Normal maps only apply to EC-atlas land textures (`texture_size == 2`
    // in the shader); on a CC art source they are a structural no-op — the
    // label states this since there is no global "source is EC" signal here.
    changed |= toggle_slider(ui, gates.is_fragment, "Normal Maps (EC land only)", &mut u.effects.enable_normal_maps, &mut u.effects.land_normal_map_strength, 0.0..=1.0);
    changed
}

fn standard_art_controls(ui: &mut egui::Ui, effects: &mut LandEffectsUniform) -> bool {
    section_header(ui, "Standard art", "Static art sprites.");
    let mut changed = slider_s(ui, "Shadow Shape", &mut effects.art_shadow_strength, 0.0..=1.0);
    changed |= slider_s(ui, "Contact Shadows", &mut effects.art_contact_shadow_strength, 0.0..=1.0);
    changed |= slider_s(ui, "Mottle", &mut effects.art_mottle_strength, 0.0..=1.5);
    changed |= slider_s(ui, "Light Catch", &mut effects.art_highlight_strength, 0.0..=1.0);
    changed |= slider_s(ui, "Atmosphere Tint", &mut effects.art_depth_tint_strength, 0.0..=1.0);
    changed
}

fn stylized_art_controls(ui: &mut egui::Ui, effects: &mut LandEffectsUniform) -> bool {
    section_header(ui, "KR-style art", "Static art sprites. Optional in every render style.");
    let mut changed = slider_s(ui, "Temperature", &mut effects.kr_art_temperature_strength, 0.0..=1.5);
    changed |= toggle_slider(ui, true, "Projected Shadows", &mut effects.enable_art_projected_shadows, &mut effects.art_projected_shadow_strength, 0.0..=1.0);
    changed |= toggle_u32(ui, "Fake Normals", &mut effects.enable_art_fake_normals);
    changed
}

/// One row for an effect with a strength: the slider, then the checkbox that switches the effect.
/// The slider lines up with plain slider rows and grays out while the effect is off.
fn toggle_slider(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    toggle: &mut u32,
    strength: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.add_enabled(enabled && *toggle != 0, egui::Slider::new(strength, range)).changed();
        changed |= toggle_en(ui, enabled, label, toggle);
    });
    changed
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

// Like `slider_s`, but shows an explanatory tooltip on hover.
fn slider_tip(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    tip: &str,
) -> bool {
    ui.add(egui::Slider::new(val, range).text(label)).on_hover_text(tip).changed()
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
            sun: SunAngles::default(),
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
            sun: SunAngles::default(),
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
    fn render_presets_preserve_source_controls_and_enforce_the_selected_style() {
        let mut u = uniform_state();
        u.effects.enable_water_animation = 0;
        u.effects.terrain_transition_flags = 3;
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
            sun: SunAngles { azimuth_degrees: 180.0, elevation_degrees: 45.0 },
        };

        apply_render_preset(
            &mut u,
            &preset,
            RENDER_STYLE_PRESETS[0],
            ShaderPresetPeriod::Morning,
        );

        assert_eq!(u.effects.shading_mode, 0);
        assert_eq!(u.effects.post_process_profile, 0);
        assert_eq!(u.effects.enable_water_animation, 0);
        assert_eq!(u.effects.terrain_transition_flags, 3);
        assert_eq!(u.active_preset, "classic.morning");
        assert_eq!(u.global_lighting, 0.75);
        assert_eq!(u.lighting.exposure, 1.25);
        assert_eq!(u.land_lighting.diffuse_strength, 0.8);
        assert_eq!(u.sun, preset.sun);
        assert!(u.dirty);
    }

    #[test]
    fn render_style_changes_keep_the_active_time_period() {
        let period = ShaderPresetPeriod::from_active_key("kr.cave");

        assert_eq!(period, ShaderPresetPeriod::Cave);
        assert_eq!(period.key(CLASSIC_PRESET_GROUP), "classic.cave");
        assert_eq!(period.key(ENHANCED_PRESET_GROUP), "enhanced.cave");
        assert_eq!(period.key(PAINTERLY_PRESET_GROUP), "painterly.cave");
    }

    fn mode_presets() -> LandShaderModePresets {
        LandShaderModePresets {
            classic: mode(1.0),
            enhanced: mode(5.0),
            kr: mode(9.0),
            painterly: mode(13.0),
            default_preset: "classic.morning".to_string(),
        }
    }

    #[test]
    fn changing_presets_captures_live_values_before_loading_the_target() {
        let mut u = uniform_state();
        let mut presets = mode_presets();
        u.global_lighting = 0.42;

        store_active_and_apply_render_preset(
            &mut u,
            &mut presets,
            RENDER_STYLE_PRESETS[1],
            ShaderPresetPeriod::Afternoon,
        );

        assert_eq!(presets.classic.morning.global_lighting, 0.42);
        assert_eq!(u.active_preset, "enhanced.afternoon");
        assert_eq!(u.global_lighting, 6.0);
    }

    #[test]
    fn kr_like_and_painterly_share_the_kr_paths_but_load_their_own_presets() {
        let presets = mode_presets();
        let mut seen_groups = Vec::new();
        for style in RENDER_STYLE_PRESETS {
            let key = ShaderPresetPeriod::Night.key(style.preset_group);
            assert!(get_preset_slot(&presets, &key).is_some(), "{key}");
            seen_groups.push(style.preset_group);

            let mut u = uniform_state();
            apply_render_preset(&mut u, preset_for_style(&presets, style, ShaderPresetPeriod::Night), style, ShaderPresetPeriod::Night);
            assert_eq!(active_render_style(&u), style);
        }
        seen_groups.dedup();
        assert_eq!(seen_groups.len(), RENDER_STYLE_PRESETS.len());

        let [_, _, kr_like, painterly] = RENDER_STYLE_PRESETS;
        assert_eq!((kr_like.shading_mode, kr_like.post_process_profile), (2, 2));
        assert_eq!((painterly.shading_mode, painterly.post_process_profile), (2, 2));
        assert_eq!(preset_for_style(&presets, kr_like, ShaderPresetPeriod::Night).global_lighting, 11.0);
        assert_eq!(preset_for_style(&presets, painterly, ShaderPresetPeriod::Night).global_lighting, 15.0);
    }
}
