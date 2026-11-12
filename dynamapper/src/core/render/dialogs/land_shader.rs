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
        build_save_presets, get_preset_slot, preset_scene_multiplier, reset_preset_to_factory,
        save_shader_settings, store_active_preset, FactoryShaderPresets, UniformState,
        CLASSIC_PRESET_GROUP, ENHANCED_PRESET_GROUP, KR_PRESET_GROUP, PAINTERLY_PRESET_GROUP,
    },
    core::render::{
        dialogs,
        scene::{
            camera::UiCameraResource,
            world::effects::client_night::{
                MAX_CLIENT_LIGHT_LEVEL, MAX_NIGHT_OVERLAY_STRENGTH, MIN_NIGHT_OVERLAY_STRENGTH,
                NIGHT_MAP_MEAN,
            },
        },
    },
    impl_tracked_plugin,
    prelude::*,
    util_lib::tracked_plugin::*,
};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

mod tips;

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

/// "Exposure" slider. The floor is the lowest exposure x "KR Tonemap Brightness" product the two former sliders
/// reached (0.5 x 0.02), so a cached KR setting folded into exposure stays on the slider.
const EXPOSURE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=2.0;

/// "Sun Azimuth" slider: a full compass turn in degrees, clockwise from north.
const SUN_AZIMUTH_RANGE_DEGREES: std::ops::RangeInclusive<f32> = 0.0..=360.0;
/// "Sun Elevation" slider: from the horizon to overhead, in degrees.
const SUN_ELEVATION_RANGE_DEGREES: std::ops::RangeInclusive<f32> = 0.0..=90.0;

/// Channels of a colour that may brighten as well as tint (light, rim, split-tone colours). egui's colour picker
/// clamps an edit to 0..1, so these colours are edited as numbers in this range instead.
const HDR_COLOR_RANGE: std::ops::RangeInclusive<f32> = 0.0..=2.0;
const HDR_COLOR_DRAG_SPEED: f64 = 0.005;
/// "Rim Power" slider. The shader never uses less than the floor (`max(0.1, rim_color.a)` in land/shading.wesl);
/// the presets use 2.5 to 3.
const RIM_POWER_RANGE: std::ops::RangeInclusive<f32> = 0.1..=8.0;
/// Gloom "Height Fade" slider in world units: land spans about -11.6..+11.5 (Z x 4/44), and the presets use
/// 0.005 to 0.01, so the slider is logarithmic to reach both ends.
const GLOOM_FADE_HEIGHT_RANGE: std::ops::RangeInclusive<f32> = 0.0..=12.0;
/// The fog pass draws no fog while both fog densities are below this (`fog_pass.wesl`), cloud noise included.
const FOG_DENSITY_MIN: f32 = 0.001;

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
    u.global_lighting = preset_scene_multiplier(preset);
    u.client_lighting = preset.client_lighting;
    u.client_shadows = preset.client_shadows;
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
                    if with_tip(ui.selectable_label(cur_period == period, label), tips::TIME_OF_DAY).clicked() {
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
            let gates = RenderStyleGates::of(&u);

            // Everything is visible at once, and each section is what a control acts on:
            // the whole frame, then land, then the two kinds of art side by side.
            let mut changed = whole_scene_controls(ui, &mut u, gates);
            ui.add_space(SECTION_SPACING);
            ui.separator();

            let width = ui.available_width();
            if width >= THREE_COLUMN_MIN_WIDTH {
                ui.columns(3, |columns| {
                    changed |= terrain_controls(&mut columns[0], &mut u, gates);
                    changed |= standard_art_controls(&mut columns[1], &mut u);
                    changed |= stylized_art_controls(&mut columns[2], &mut u);
                });
            } else if width >= TWO_COLUMN_MIN_WIDTH {
                ui.columns(2, |columns| {
                    changed |= terrain_controls(&mut columns[0], &mut u, gates);
                    changed |= standard_art_controls(&mut columns[1], &mut u);
                    columns[1].add_space(SECTION_SPACING);
                    changed |= stylized_art_controls(&mut columns[1], &mut u);
                });
            } else {
                changed |= terrain_controls(ui, &mut u, gates);
                ui.add_space(SECTION_SPACING);
                changed |= standard_art_controls(ui, &mut u);
                ui.add_space(SECTION_SPACING);
                changed |= stylized_art_controls(ui, &mut u);
            }

            if changed {
                u.dirty = true;
            }

            ui.add_space(SECTION_SPACING);
            ui.separator();
            with_tip(
                ui.checkbox(&mut advanced_open, "Show advanced raw shader knobs (opens a side panel on the right)"),
                tips::SHOW_ADVANCED,
            );

            if with_tip(ui.button("Reset Current Preset to Factory Defaults"), tips::RESET_PRESET).clicked() {
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
                if with_tip(ui.button("⟲ Undo to Last Saved"), tips::UNDO_TO_SAVED).clicked() {
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
            // Controls the active land shading never reads are greyed out, as in the main window; their tooltips
            // say why.
            let gates = RenderStyleGates::of(&u);
            let per_pixel = gates.is_per_pixel_land;
            let kr_per_pixel = gates.is_kr_per_pixel_land();

            // ------------------------- Effects -------------------------
            ui.collapsing("Effects", |ui| {
                let mut changed = false;
                ui.horizontal(|ui| {
                    with_tip(ui.label("Visual Profile:"), tips::VISUAL_PROFILE);
                    let mut profile = u.effects.post_process_profile;
                    for (label, val) in [("Neutral", 0u32), ("EC", 1u32), ("KR", 2u32)] {
                        if with_tip(ui.selectable_label(profile == val, label), tips::VISUAL_PROFILE).clicked() {
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
                if toggle_u32(ui, "Fog", &mut u.lighting.enable_fog, tips::FOG) {
                    changed = true;
                }
                changed |= toggle_u32(ui, "Grunge / Weathering", &mut u.effects.enable_grunge, tips::GRUNGE);
                changed |= slider_s(
                    ui,
                    "Grunge Strength",
                    &mut u.effects.grunge_strength,
                    0.0..=1.0,
                    tips::GRUNGE_STRENGTH,
                );
                ui.separator();
                ui.strong("Water surfaces (terrain and statics)");
                changed |= toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation, tips::WATER_ANIMATION);
                if changed {
                    u.dirty = true;
                }
            });
 
            ui.separator();

            // --------------------- Mode & Normals ---------------------
            ui.horizontal(|ui| {
                with_tip(ui.strong("Shading Mode:"), tips::SHADING_MODE);
                let mut mode = u.effects.shading_mode;
                for (label, val) in [("Classic", 0u32), ("Enhanced", 1u32), ("KR", 2u32)] {
                    if with_tip(ui.selectable_label(mode == val, label), tips::SHADING_MODE).clicked() {
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
                with_tip(ui.strong("Normals:"), tips::NORMALS);
                // Classic shades with geometric normals whatever this value is, so the selector is
                // disabled there; the stored value is kept for the fragment styles.
                if mode == 0 {
                    with_tip(ui.add_enabled(false, egui::Label::new("Geometric (fixed)")), tips::NORMALS);
                } else {
                    let mut nm = u.effects.normal_mode;
                    if with_tip(ui.selectable_label(nm == 0, "Geometric"), tips::NORMALS).clicked() {
                        nm = 0;
                    }
                    if with_tip(ui.selectable_label(nm == 1, "Bicubic"), tips::NORMALS).clicked() {
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

                // Tonemap, colour grading, bent normals (vertex shader) and blur (texture
                // sampling) apply in ANY shading mode (keep available always)
                changed |= toggle_u32(ui, "Tonemap", &mut u.lighting.enable_tonemap, tips::TONEMAP);
                changed |= toggle_u32(ui, "Color Grading", &mut u.lighting.enable_grading, tips::COLOR_GRADING);
                changed |= toggle_u32(ui, "Bent normals", &mut u.land_lighting.enable_bent, tips::BENT_NORMALS);
                changed |= toggle_u32(ui, "Blur", &mut u.effects.enable_blur, tips::BLUR);

                // Gloom is applied by the fragment shading modes only
                let is_classic = u.effects.shading_mode == 0;
                if !is_classic {
                    // Gloom: fragment-only semantic
                    let toggled =
                        toggle_en(ui, per_pixel, "Gloom (fragment)", &mut u.lighting.enable_gloom, tips::GLOOM);
                    if toggled {
                        if u.lighting.enable_gloom == 0 {
                            u.lighting.gloom_params[0] = 0.0;
                        } else if u.lighting.gloom_params[0] <= 0.0001 {
                            u.lighting.gloom_params[0] = 0.20;
                        }
                    }
                    changed |= toggled;
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

                // Global scene brightness, in the styles without the client night lighting (KR-like and Enhanced
                // dim through its Light Level instead, under Whole scene).
                if u.client_lighting.is_none() {
                    changed |= slider_s(
                        ui,
                        "Global Lighting (Scene Luminosity)",
                        &mut u.global_lighting,
                        0.0..=2.0,
                        tips::BRIGHTNESS,
                    );
                }

                // Ambient always shown
                changed |= slider_s(ui, "Ambient", &mut u.lighting.ambient_strength, 0.0..=1.5, tips::TERRAIN_AMBIENT);

                // Diffuse scales the sun light in every mode: Classic (vertex) multiplies its
                // precomputed vertex Lambert by it.
                let is_classic = u.effects.shading_mode == 0;
                changed |= slider_s(ui, "Diffuse", &mut u.land_lighting.diffuse_strength, 0.0..=2.0, tips::SUN_STRENGTH);

                // Exposure lives in lighting UBO
                changed |= slider_s(
                    ui,
                    "Exposure (Tonemap)",
                    &mut u.lighting.exposure,
                    EXPOSURE_RANGE,
                    tips::EXPOSURE,
                );

                ui.separator();

                // Blur parameters (texture sampling, every mode)
                changed |= slider_s(
                    ui,
                    "Blur Strength",
                    &mut u.effects.blur_strength,
                    0.0..=0.5,
                    tips::BLUR_STRENGTH,
                );
                changed |= slider_s(
                    ui,
                    "Blur Radius (screen pixels)",
                    &mut u.effects.blur_radius,
                    BLUR_RADIUS_MIN_PIXELS..=BLUR_RADIUS_MAX_PIXELS,
                    tips::BLUR_RADIUS,
                );

                ui.separator();

                if !is_classic {
                    changed |= slider_en(
                        ui,
                        per_pixel,
                        "Specular (fragment only)",
                        &mut u.land_lighting.specular_strength,
                        0.0..=0.4,
                        tips::SPECULAR,
                    );
                    changed |= slider_en(
                        ui,
                        per_pixel,
                        "Fill (env) (fragment only)",
                        &mut u.land_lighting.fill_strength,
                        0.0..=1.0,
                        tips::FILL,
                    );
                    ui.separator();
                    changed |= slider_en(
                        ui,
                        per_pixel,
                        "Sharpness Factor (fragment only)",
                        &mut u.land_lighting.sharpness_factor,
                        0.5..=4.0,
                        tips::SHARPNESS_FACTOR,
                    );
                    changed |= slider_en(
                        ui,
                        per_pixel,
                        "Sharpness Mix (fragment only)",
                        &mut u.land_lighting.sharpness_mix,
                        0.0..=1.0,
                        tips::SHARPNESS_MIX,
                    );
                    ui.separator();
                    changed |= slider_en(ui, per_pixel, "Rim (Enhanced and KR)", &mut u.land_lighting.rim_strength, 0.0..=0.5, tips::RIM);
                    changed |= slider_en(ui, kr_per_pixel, "Diffuse Wrap (KR only)", &mut u.land_lighting.diffuse_wrap, 0.0..=0.5, tips::DIFFUSE_WRAP);
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

                changed |= hdr_rgb(ui, "Light Color", &mut u.lighting.light_color, tips::LIGHT_COLOR);
                {
                    let mut v = u.lighting.atmosphere_tint;
                    if color3(ui, "Atmosphere Tint", &mut v, tips::ATMOSPHERE_TINT) {
                        u.lighting.atmosphere_tint = v;
                        changed = true;
                    }
                }

                ui.separator();

                ui.add_enabled_ui(per_pixel, |ui| {
                    let mut v = u.land_lighting.fill_sky_color;
                    if color4(ui, "Fill Sky (rgb + strength.a) (fragment only)", &mut v, tips::FILL_SKY) {
                        u.land_lighting.fill_sky_color = v;
                        changed = true;
                    }
                    let mut v = u.land_lighting.fill_ground_color;
                    if color4(
                        ui,
                        "Fill Ground (rgb + strength.a) (fragment only)",
                        &mut v,
                        tips::FILL_GROUND,
                    ) {
                        u.land_lighting.fill_ground_color = v;
                        changed = true;
                    }
                });

                ui.separator();

                // The rim colour is a brightening tint and its alpha a power above 1, both out of
                // the colour picker's 0..1, so they are edited as numbers and a slider.
                ui.add_enabled_ui(kr_per_pixel, |ui| {
                    let mut rgb = u.land_lighting.rim_color.truncate();
                    if hdr_rgb(ui, "Rim Color (KR only)", &mut rgb, tips::RIM_COLOR) {
                        u.land_lighting.rim_color = rgb.extend(u.land_lighting.rim_color.w);
                        changed = true;
                    }
                });
                changed |= slider_en(ui, per_pixel, "Rim Power (fragment only)", &mut u.land_lighting.rim_color.w, RIM_POWER_RANGE, tips::RIM_POWER);

                if changed {
                    u.dirty = true;
                }
            });

            // ----------------- KR like Color Grading (Vibrant) --------------
            // Grading runs after shading in every mode (land, art and ground art).
            ui.collapsing("Color Grading (Vibrant)", |ui| {
                let mut changed = false;
                {
                    changed |= slider_s(
                        ui,
                        "Grade Strength",
                        &mut u.lighting.grade_params[0],
                        0.0..=2.0,
                        tips::GRADE_STRENGTH,
                    );
                    changed |= slider_en(
                        ui,
                        kr_per_pixel,
                        "Headroom Reserve",
                        &mut u.lighting.grade_params[1],
                        0.0..=0.5,
                        tips::HEADROOM_RESERVE,
                    );
                    changed |= slider_en(
                        ui,
                        per_pixel,
                        "Fill Chroma Tint",
                        &mut u.lighting.grade_params[2],
                        0.0..=1.0,
                        tips::FILL_CHROMA_TINT,
                    );

                    // Headroom on/off via local bool to avoid overlapping borrows
                    {
                        let mut headroom_on = u.lighting.grade_params[3] >= 0.5;
                        let before = headroom_on;
                        with_tip(
                            ui.add_enabled(per_pixel, egui::Checkbox::new(&mut headroom_on, "Enable Headroom Limit")),
                            tips::HEADROOM_LIMIT,
                        );
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
                        tips::VIBRANCE,
                    );
                    changed |= slider_s(
                        ui,
                        "Saturation (global)",
                        &mut u.lighting.grade_extra[1],
                        0.5..=2.0,
                        tips::SATURATION,
                    );
                    changed |= slider_s(
                        ui,
                        "Chroma Knee",
                        &mut u.lighting.grade_chroma_knee,
                        0.0..=CHROMA_KNEE_OFF,
                        tips::CHROMA_KNEE,
                    );
                    changed |= slider_s(
                        ui,
                        "Chroma Knee Slope",
                        &mut u.lighting.grade_chroma_slope,
                        0.0..=1.0,
                        tips::CHROMA_KNEE_SLOPE,
                    );
                    changed |= slider_s(
                        ui,
                        "Contrast (global)",
                        &mut u.lighting.grade_extra[2],
                        0.5..=2.0,
                        tips::CONTRAST,
                    );
                    changed |= slider_s(
                        ui,
                        "Split Tone Strength",
                        &mut u.lighting.grade_extra[3],
                        0.0..=2.0,
                        tips::SPLIT_TONE,
                    );
                    changed |= hdr_rgb(ui, "Grade Warm (bright areas)", &mut u.lighting.grade_warm_color, tips::GRADE_WARM);
                    changed |= hdr_rgb(ui, "Grade Cool (dark areas)", &mut u.lighting.grade_cool_color, tips::GRADE_COOL);
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
                    changed |= slider_en(ui, per_pixel, "Amount", &mut u.lighting.gloom_params[0], 0.0..=1.0, tips::GLOOM_AMOUNT);
                    changed |= with_tip(
                        ui.add_enabled(
                            per_pixel,
                            egui::Slider::new(&mut u.lighting.gloom_params[1], GLOOM_FADE_HEIGHT_RANGE)
                                .logarithmic(true)
                                .text("Height Fade Height (world units)"),
                        ),
                        tips::GLOOM_FADE_HEIGHT,
                    )
                    .changed();
                    changed |= slider_en(ui, per_pixel, "Shadow Bias", &mut u.lighting.gloom_params[2], 0.0..=1.0, tips::GLOOM_SHADOW_BIAS);
                }
                if changed {
                    u.dirty = true;
                }

                // Gloom color + desaturation (KR-style enhancement)
                let mut changed_color = false;
                ui.add_enabled_ui(per_pixel, |ui| {
                    let mut v = u.lighting.gloom_color;
                    if color4(ui, "Gloom Tint (rgb + desat.a) (KR)", &mut v, tips::GLOOM_TINT) {
                        u.lighting.gloom_color = v;
                        changed_color = true;
                    }
                });
                if changed_color {
                    u.dirty = true;
                }
            });

            // ------------------------ Graphics ------------------------------
            ui.collapsing("Graphics Settings", |ui| {
                let mut changed = false;

                ui.strong("Whole scene");
                changed |= toggle_u32(ui, "Linear Filtering", &mut u.effects.enable_linear_filtering, tips::TEXTURE_SMOOTHNESS);
                changed |= texture_filter_row(ui, &mut u);
                changed |= slider_s(ui, "Sharpening Amount", &mut u.effects.sharpening_amount, 0.0..=2.0, tips::SHARPENING);
                changed |= toggle_u32(ui, "KR Color Wash", &mut u.effects.enable_kr_color_wash, tips::KR_COLOR_WASH);
                changed |= slider_s(ui, "KR Color Wash Strength", &mut u.effects.kr_color_wash_strength, 0.0..=1.5, tips::KR_COLOR_WASH_STRENGTH);
                changed |= slider_s(ui, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0, tips::KR_COLOR_WASH_SCALE);
                changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0, tips::LIGHT_STATIC_INTERACTION);

                ui.separator();
                ui.strong("Terrain");
                changed |= toggle_u32(ui, "Land Bump Maps", &mut u.effects.enable_normal_maps, tips::NORMAL_MAPS);
                changed |= slider_s(ui, "Land Bump Strength", &mut u.effects.land_normal_map_strength, 0.0..=1.0, tips::NORMAL_MAP_STRENGTH);
                // KR-like and Enhanced draw the lights into the night composite's light buffer, not as decals.
                let decals = u.client_lighting.is_none();
                changed |= slider_en(ui, decals, "Static Light Decals", &mut u.effects.static_light_decal_visibility, 0.0..=2.0, tips::STATIC_LIGHT_DECALS);
                changed |= slider_en(ui, kr_per_pixel, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5, tips::KR_LAND_TEMPERATURE);
                changed |= slider_en(ui, kr_per_pixel, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5, tips::KR_LAND_RELIEF_SHADOW);
                changed |= slider_en(ui, kr_per_pixel, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5, tips::KR_LAND_SHADOW_MOTTLE);
                changed |= slider_s(ui, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5, tips::KR_MATERIAL_CONTRAST);
                changed |= toggle_u32(ui, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime, tips::KR_TRANSITION_GRIME);
                changed |= slider_s(ui, "KR Transition Grime Strength", &mut u.effects.kr_transition_grime_strength, 0.0..=1.0, tips::KR_TRANSITION_GRIME_STRENGTH);

                ui.separator();
                ui.strong("Objects (art / statics)");
                changed |= toggle_u32(ui, "Art Fake Normals", &mut u.effects.enable_art_fake_normals, tips::ART_FAKE_NORMALS);
                changed |= slider_s(ui, "Art Shadow Shape", &mut u.effects.art_shadow_strength, 0.0..=1.0, tips::ART_SHADOW_SHAPE);
                changed |= slider_s(ui, "Art Contact Shadows", &mut u.effects.art_contact_shadow_strength, 0.0..=1.0, tips::ART_CONTACT_SHADOWS);
                changed |= slider_s(ui, "Art Mottle", &mut u.effects.art_mottle_strength, 0.0..=1.5, tips::ART_MOTTLE);
                changed |= slider_s(ui, "Art Light Catch", &mut u.effects.art_highlight_strength, 0.0..=1.0, tips::ART_LIGHT_CATCH);
                changed |= slider_s(ui, "Art Atmosphere Tint", &mut u.effects.art_depth_tint_strength, 0.0..=1.0, tips::ART_ATMOSPHERE_TINT);
                changed |= slider_s(ui, "KR Art Temperature", &mut u.effects.kr_art_temperature_strength, 0.0..=1.5, tips::KR_ART_TEMPERATURE);
                changed |= toggle_u32(ui, "Client Grunge", &mut u.effects.enable_client_grunge, tips::CLIENT_GRUNGE);
                changed |= slider_s(ui, "Client Grunge Strength", &mut u.effects.client_grunge_strength, 0.0..=1.0, tips::CLIENT_GRUNGE_STRENGTH);
                changed |= toggle_u32(ui, "Art Projected Shadows", &mut u.effects.enable_art_projected_shadows, tips::ART_PROJECTED_SHADOWS);
                changed |= slider_s(ui, "Art Projected Shadow Strength", &mut u.effects.art_projected_shadow_strength, 0.0..=1.0, tips::ART_PROJECTED_SHADOW_STRENGTH);
                changed |= slider_s(ui, "Art Projected Shadow Length", &mut u.effects.art_projected_shadow_length, 0.0..=1.5, tips::ART_PROJECTED_SHADOW_LENGTH);
                changed |= slider_s(ui, "Art Projected Shadow Softness", &mut u.effects.art_projected_shadow_softness, 0.05..=1.0, tips::ART_PROJECTED_SHADOW_SOFTNESS);

                if changed {
                    u.dirty = true;
                }
            });

            // ------------------------ Fog ------------------------------
            ui.collapsing("Fog Params", |ui| {
                let mut changed = false;
                // Tint + max mix (alpha)
                let mut fog = u.lighting.fog_color;
                if color4(ui, "Fog Color (alpha = max mix)", &mut fog, tips::FOG_COLOR) {
                    u.lighting.fog_color = fog;
                    changed = true;
                }
                // Densities and noise
                changed |= slider_s(
                    ui,
                    "Distance Density",
                    &mut u.lighting.fog_params[0],
                    0.0..=0.2,
                    tips::FOG_DISTANCE_DENSITY,
                );
                changed |= slider_s(
                    ui,
                    "Height Density",
                    &mut u.lighting.fog_params[1],
                    0.0..=0.2,
                    tips::FOG_HEIGHT_DENSITY,
                );
                // The cloud noise only shapes haze the densities make: alone it draws nothing.
                let has_haze = fog_has_haze(u.lighting.fog_params);
                changed |= slider_en(ui, has_haze, "Noise Scale", &mut u.lighting.fog_params[2], 0.0..=2.0, tips::FOG_NOISE_SCALE);
                changed |= slider_en(
                    ui,
                    has_haze,
                    "Noise Strength",
                    &mut u.lighting.fog_params[3],
                    0.0..=1.0,
                    tips::FOG_NOISE_STRENGTH,
                );
                if changed {
                    u.dirty = true;
                }

                // Night fog tint (KR-style enhancement)
                let mut changed_night = false;
                {
                    let mut v = u.lighting.fog_night_color;
                    if color4(ui, "Night Fog Tint (rgb + blend.a)", &mut v, tips::FOG_NIGHT_TINT) {
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

/// Which controls the active render style consumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RenderStyleGates {
    is_fragment: bool,
    is_kr_shading: bool,
    is_kr_profile: bool,
    /// A fragment style with smooth per-pixel land shading. Only it reads the fill, rim, specular, sharpness, gloom
    /// and headroom terms: Classic and the clients' per-vertex model (KR-like, Enhanced) read none of them.
    is_per_pixel_land: bool,
}

impl RenderStyleGates {
    fn of(u: &UniformState) -> Self {
        let is_fragment = u.effects.shading_mode != 0;
        Self {
            is_fragment,
            is_kr_shading: u.effects.shading_mode == 2,
            is_kr_profile: u.effects.post_process_profile == 2,
            is_per_pixel_land: is_fragment
                && u.land_lighting.terrain_shading_model == TERRAIN_SHADING_SMOOTH_PER_PIXEL,
        }
    }

    /// KR shading with per-pixel land shading: the only land shading that reads the diffuse wrap, the rim colour,
    /// the headroom reserve and the KR land temperature, relief shadow and shadow mottle.
    fn is_kr_per_pixel_land(self) -> bool {
        self.is_kr_shading && self.is_per_pixel_land
    }
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
        changed |= toggle_u32(ui, "Tonemap", &mut u.lighting.enable_tonemap, tips::TONEMAP);
        // "Color Grading" is the master on/off for the same vibrant grading
        // whose knobs live under Advanced → "Color Grading (Vibrant)". The
        // excess saturation the user sees comes from those defaults, not a
        // separate effect — the tooltip makes the relationship explicit.
        changed |= toggle_u32(ui, "Color Grading", &mut u.lighting.enable_grading, tips::COLOR_GRADING);
        changed |= toggle_u32(ui, "Texture Smoothness", &mut u.effects.enable_linear_filtering, tips::TEXTURE_SMOOTHNESS);
        changed |= toggle_u32(ui, "Fog", &mut u.lighting.enable_fog, tips::FOG);
    });
    changed |= texture_filter_row(ui, u);
    ui.horizontal_wrapped(|ui| {
        let enabled = u.lighting.enable_tonemap != 0;
        with_tip(ui.add_enabled(enabled, egui::Label::new("Tone Curve:")), tips::TONE_CURVE);
        for (label, curve, tip) in TONE_CURVE_OPTIONS {
            let selected = u.lighting.tone_curve == curve;
            if with_tip(ui.add_enabled(enabled, egui::Button::selectable(selected, label)), tip).clicked() && !selected {
                u.lighting.tone_curve = curve;
                changed = true;
            }
        }
    });
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        // KR-like and Enhanced dim as their clients do, through the light level and the night map; the other
        // styles through the Brightness multiplier.
        match u.client_lighting.as_mut() {
            Some(lighting) => changed |= client_night_controls(ui, lighting),
            None => changed |= slider_s(ui, "Brightness", &mut u.global_lighting, 0.0..=2.0, tips::BRIGHTNESS),
        }
        // KR's filmic tonemap had its own pre-scale here ("KR Tonemap Brightness"); it multiplied
        // exactly like Exposure, so it is folded into Exposure (Advanced -> Intensities).
        changed |= slider_s(ui, "Light/Static Interaction", &mut u.effects.light_decal_intensity, 0.0..=2.0, tips::LIGHT_STATIC_INTERACTION);

        let ui = &mut columns[1];
        changed |= toggle_slider(ui, true, "Grunge", &mut u.effects.enable_grunge, &mut u.effects.grunge_strength, 0.0..=1.0, tips::GRUNGE);
        changed |= toggle_slider(ui, gates.is_kr_profile, "KR Color Wash", &mut u.effects.enable_kr_color_wash, &mut u.effects.kr_color_wash_strength, 0.0..=1.5, tips::KR_COLOR_WASH);
        changed |= slider_en(ui, gates.is_kr_profile, "KR Color Wash Scale", &mut u.effects.kr_color_wash_scale, 8.0..=160.0, tips::KR_COLOR_WASH_SCALE);
    });
    changed
}

/// The client night lighting of KR-like and Enhanced: light level, night overlay strength and colour.
fn client_night_controls(ui: &mut egui::Ui, lighting: &mut ClientNightLighting) -> bool {
    let mut changed = ui
        .add(egui::Slider::new(&mut lighting.light_level, 0..=MAX_CLIENT_LIGHT_LEVEL).text("Light Level"))
        .on_hover_text(tips::CLIENT_LIGHT_LEVEL)
        .changed();
    changed |= slider_s(
        ui,
        "Night Overlay Strength",
        &mut lighting.overlay_strength,
        MIN_NIGHT_OVERLAY_STRENGTH..=MAX_NIGHT_OVERLAY_STRENGTH,
        tips::NIGHT_OVERLAY_STRENGTH,
    );
    changed |= night_overlay_color(ui, &mut lighting.overlay_color);
    changed
}

/// The night overlay colour as 8-bit values, as the night map stores them, with a button back to the client's.
fn night_overlay_color(ui: &mut egui::Ui, color: &mut Vec3) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let mut bytes = color.to_array().map(|channel| (channel * f32::from(u8::MAX)).round() as u8);
        if ui.color_edit_button_srgb(&mut bytes).on_hover_text(tips::NIGHT_OVERLAY_COLOR).changed() {
            *color = Vec3::from_array(bytes.map(|channel| f32::from(channel) / f32::from(u8::MAX)));
            changed = true;
        }
        ui.label("Night Overlay Colour").on_hover_text(tips::NIGHT_OVERLAY_COLOR);
        let is_client = *color == NIGHT_MAP_MEAN;
        if with_tip(ui.add_enabled(!is_client, egui::Button::new("Client")), tips::NIGHT_OVERLAY_CLIENT).clicked()
        {
            *color = NIGHT_MAP_MEAN;
            changed = true;
        }
    });
    changed
}

fn texture_filter_row(ui: &mut egui::Ui, u: &mut UniformState) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        with_tip(ui.label("Texture Filter:"), tips::TEXTURE_FILTER);
        for (label, filter, tip) in TEXTURE_FILTER_OPTIONS {
            let selected = u.effects.texture_filter == filter;
            if with_tip(ui.add(egui::Button::selectable(selected, label)), tip).clicked() && !selected {
                u.effects.texture_filter = filter;
                changed = true;
            }
        }
    });
    changed
}

fn terrain_controls(ui: &mut egui::Ui, u: &mut UniformState, gates: RenderStyleGates) -> bool {
    section_header(ui, "Terrain and water", "Land tiles. Water also covers static water surfaces.");
    let mut changed = toggle_u32(ui, "Water Animation", &mut u.effects.enable_water_animation, tips::WATER_ANIMATION);
    ui.horizontal_wrapped(|ui| {
        with_tip(ui.add_enabled(gates.is_fragment, egui::Label::new("Land Shading:")), tips::LAND_SHADING);
        for (label, model, tip) in TERRAIN_SHADING_OPTIONS {
            let selected = u.land_lighting.terrain_shading_model == model;
            if with_tip(ui.add_enabled(gates.is_fragment, egui::Button::selectable(selected, label)), tip).clicked() && !selected {
                u.land_lighting.terrain_shading_model = model;
                changed = true;
            }
        }
    });
    // The client per-vertex model shades land with its grey alone; these controls only reach the per-pixel model.
    let per_pixel = gates.is_per_pixel_land;
    // Every shading mode scales its sun light by this, Classic's vertex Lambert included; the client per-vertex model
    // uses it only on flat water.
    changed |= slider_s(ui, "Sun Strength", &mut u.land_lighting.diffuse_strength, 0.0..=2.0, tips::SUN_STRENGTH);
    changed |= slider_s(ui, "Sun Azimuth", &mut u.sun.azimuth_degrees, SUN_AZIMUTH_RANGE_DEGREES, tips::SUN_AZIMUTH);
    changed |= slider_s(ui, "Sun Elevation", &mut u.sun.elevation_degrees, SUN_ELEVATION_RANGE_DEGREES, tips::SUN_ELEVATION);
    changed |= slider_s(ui, "Terrain Ambient Light", &mut u.lighting.ambient_strength, 0.0..=1.5, tips::TERRAIN_AMBIENT);

    let mut land_relief = land_relief_control_value(u);
    if slider_en(ui, per_pixel, "Land Relief", &mut land_relief, 0.0..=1.0, tips::LAND_RELIEF) {
        apply_land_relief_control(u, land_relief);
        changed = true;
    }
    let mut soft_lighting = soft_lighting_control_value(u);
    if slider_en(ui, per_pixel, "Soft Lighting", &mut soft_lighting, 0.0..=1.0, tips::SOFT_LIGHTING) {
        apply_soft_lighting_control(u, soft_lighting);
        changed = true;
    }
    let mut highlights = highlights_control_value(u);
    if slider_en(ui, per_pixel, "Highlights", &mut highlights, 0.0..=1.0, tips::HIGHLIGHTS) {
        apply_highlights_control(u, highlights);
        changed = true;
    }

    let kr_per_pixel = gates.is_kr_per_pixel_land();
    changed |= slider_en(ui, kr_per_pixel, "KR Land Temperature", &mut u.effects.kr_land_temperature_strength, 0.0..=1.5, tips::KR_LAND_TEMPERATURE);
    changed |= slider_en(ui, kr_per_pixel, "KR Land Relief Shadow", &mut u.effects.kr_land_relief_shadow_strength, 0.0..=1.5, tips::KR_LAND_RELIEF_SHADOW);
    changed |= slider_en(ui, kr_per_pixel, "KR Land Shadow Mottle", &mut u.effects.kr_land_shadow_mottle_strength, 0.0..=1.5, tips::KR_LAND_SHADOW_MOTTLE);
    changed |= slider_en(ui, gates.is_kr_profile, "KR Material Contrast", &mut u.effects.kr_land_material_contrast_strength, 0.0..=1.5, tips::KR_MATERIAL_CONTRAST);
    changed |= toggle_slider(ui, gates.is_kr_profile, "KR Transition Grime", &mut u.effects.enable_kr_transition_grime, &mut u.effects.kr_transition_grime_strength, 0.0..=1.0, tips::KR_TRANSITION_GRIME);

    // Every land source and style takes a bump: an Enhanced or KR material's bump map where it has one (snow), else
    // the one derived from the texture (land/bump.wesl). The per-pixel lighting reads the bumped normal, Classic
    // shading and the client per-vertex model scale their vertex light by it, and Grunge's shading reads it too.
    changed |= toggle_slider(ui, true, "Bump Maps", &mut u.effects.enable_normal_maps, &mut u.effects.land_normal_map_strength, 0.0..=1.0, tips::NORMAL_MAPS);
    changed
}

fn standard_art_controls(ui: &mut egui::Ui, u: &mut UniformState) -> bool {
    section_header(ui, "Standard art", "Static art sprites.");
    let mut changed = client_shadows_toggle(ui, &mut u.client_shadows);
    // KR-like's client shadow replaces these sprite shadows.
    let sprite_shadows = u.client_shadows.is_none();
    let effects = &mut u.effects;
    changed |= slider_en(ui, sprite_shadows, "Shadow Shape", &mut effects.art_shadow_strength, 0.0..=1.0, tips::ART_SHADOW_SHAPE);
    changed |= slider_en(ui, sprite_shadows, "Contact Shadows", &mut effects.art_contact_shadow_strength, 0.0..=1.0, tips::ART_CONTACT_SHADOWS);
    changed |= slider_s(ui, "Mottle", &mut effects.art_mottle_strength, 0.0..=1.5, tips::ART_MOTTLE);
    changed |= slider_s(ui, "Light Catch", &mut effects.art_highlight_strength, 0.0..=1.0, tips::ART_LIGHT_CATCH);
    changed |= slider_s(ui, "Atmosphere Tint", &mut effects.art_depth_tint_strength, 0.0..=1.0, tips::ART_ATMOSPHERE_TINT);
    changed |= toggle_slider(ui, true, "Client Grunge", &mut effects.enable_client_grunge, &mut effects.client_grunge_strength, 0.0..=1.0, tips::CLIENT_GRUNGE);
    changed
}

/// KR-like's switch for the KR client's static shadows; nothing in the other styles.
fn client_shadows_toggle(ui: &mut egui::Ui, client_shadows: &mut Option<bool>) -> bool {
    let Some(enabled) = client_shadows.as_mut() else {
        return false;
    };
    ui.checkbox(enabled, "Client Shadows")
        .on_hover_text(tips::CLIENT_SHADOWS)
        .changed()
}

fn stylized_art_controls(ui: &mut egui::Ui, u: &mut UniformState) -> bool {
    section_header(ui, "KR-style art", "Static art sprites. Optional in every render style.");
    // KR-like's client shadow replaces the projected shadows.
    let sprite_shadows = u.client_shadows.is_none();
    let effects = &mut u.effects;
    let mut changed = slider_s(ui, "Temperature", &mut effects.kr_art_temperature_strength, 0.0..=1.5, tips::KR_ART_TEMPERATURE);
    changed |= toggle_slider(ui, sprite_shadows, "Projected Shadows", &mut effects.enable_art_projected_shadows, &mut effects.art_projected_shadow_strength, 0.0..=1.0, tips::ART_PROJECTED_SHADOWS);
    changed |= toggle_u32(ui, "Fake Normals", &mut effects.enable_art_fake_normals, tips::ART_FAKE_NORMALS);
    changed
}

/// Show `tip` when the pointer rests on the control, enabled or grayed out: egui shows a plain hover text only on
/// enabled widgets, and a grayed-out control is where "needs X" matters most.
fn with_tip(response: egui::Response, tip: &str) -> egui::Response {
    response.on_hover_text(tip).on_disabled_hover_text(tip)
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
    tip: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= with_tip(ui.add_enabled(enabled && *toggle != 0, egui::Slider::new(strength, range)), tip).changed();
        changed |= toggle_en(ui, enabled, label, toggle, tip);
    });
    changed
}

// Toggle a u32 field as a boolean (0/1). Returns true if the value changed.
fn toggle_u32(ui: &mut egui::Ui, label: &str, val: &mut u32, tip: &str) -> bool {
    let mut b = *val != 0;
    let changed = with_tip(ui.checkbox(&mut b, label), tip).changed();
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
    tip: &str,
) -> bool {
    with_tip(ui.add(egui::Slider::new(val, range).text(label)), tip).changed()
}

// Like `slider_s`, but grayed out (non-interactive) when `enabled` is false —
// used to show controls that the current render style does not consume.
fn slider_en(
    ui: &mut egui::Ui,
    enabled: bool,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    tip: &str,
) -> bool {
    with_tip(ui.add_enabled(enabled, egui::Slider::new(val, range).text(label)), tip).changed()
}

// Like `toggle_u32`, but grayed out when `enabled` is false.
fn toggle_en(ui: &mut egui::Ui, enabled: bool, label: &str, val: &mut u32, tip: &str) -> bool {
    let mut b = *val != 0;
    let changed = with_tip(ui.add_enabled(enabled, egui::Checkbox::new(&mut b, label)), tip).changed();
    if changed {
        *val = if b { 1 } else { 0 };
    }
    changed
}

// Edit a Vec3 as RGB (0..1). Returns true if changed.
fn color3(ui: &mut egui::Ui, label: &str, v: &mut Vec3, tip: &str) -> bool {
    let mut changed = false;
    let mut arr = v.to_array();
    ui.horizontal(|ui| {
        with_tip(ui.label(label), tip);
        if with_tip(ui.color_edit_button_rgb(&mut arr), tip).changed() {
            changed = true;
        }
    });
    if changed {
        *v = Vec3::from_array(arr);
    }
    changed
}

// Edit a Vec4 as RGBA (0..1). Returns true if changed.
fn color4(ui: &mut egui::Ui, label: &str, v: &mut Vec4, tip: &str) -> bool {
    let mut changed = false;
    let mut arr = v.to_array();
    ui.horizontal(|ui| {
        with_tip(ui.label(label), tip);
        if with_tip(ui.color_edit_button_rgba_unmultiplied(&mut arr), tip).changed() {
            changed = true;
        }
    });
    if changed {
        *v = Vec4::from_array(arr);
    }
    changed
}

/// Edit an RGB colour that may exceed 1 as three numbers in `HDR_COLOR_RANGE`, with a swatch of how it looks.
/// Returns true if changed.
fn hdr_rgb(ui: &mut egui::Ui, label: &str, v: &mut Vec3, tip: &str) -> bool {
    let mut changed = false;
    let mut arr = v.to_array();
    ui.horizontal(|ui| {
        for channel in &mut arr {
            let drag = egui::DragValue::new(channel).range(HDR_COLOR_RANGE).speed(HDR_COLOR_DRAG_SPEED);
            changed |= with_tip(ui.add(drag), tip).changed();
        }
        with_tip(
            egui::color_picker::show_color(ui, egui::Rgba::from_rgb(arr[0], arr[1], arr[2]), ui.spacing().interact_size),
            tip,
        );
        with_tip(ui.label(label), tip);
    });
    if changed {
        *v = Vec3::from_array(arr);
    }
    changed
}

/// Whether the fog pass draws any haze for these `fog_params` (distance and height density); the cloud noise
/// only shapes that haze.
fn fog_has_haze(fog_params: Vec4) -> bool {
    fog_params.x >= FOG_DENSITY_MIN || fog_params.y >= FOG_DENSITY_MIN
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
            client_lighting: None,
            client_shadows: None,
            sun: SunAngles::default(),
            dirty: false,
            saved_snapshot: None,
            active_preset: "classic.morning".to_string(),
        }
    }

    fn preset(global_lighting: f32) -> LandMaterialUniformsPresets {
        LandMaterialUniformsPresets {
            global_lighting: Some(global_lighting),
            client_lighting: None,
            client_shadows: None,
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
            global_lighting: Some(0.75),
            client_lighting: None,
            client_shadows: None,
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

        assert_eq!(presets.classic.morning.global_lighting, Some(0.42));
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
        assert_eq!(preset_for_style(&presets, kr_like, ShaderPresetPeriod::Night).global_lighting, Some(11.0));
        assert_eq!(preset_for_style(&presets, painterly, ShaderPresetPeriod::Night).global_lighting, Some(15.0));
    }

    #[test]
    fn client_lit_presets_load_their_lighting_and_a_neutral_scene_multiplier() {
        const NIGHT_LEVEL: u32 = 10;
        const STRENGTH: f32 = 0.5;
        let lighting =
            ClientNightLighting { light_level: NIGHT_LEVEL, overlay_strength: STRENGTH, overlay_color: NIGHT_MAP_MEAN };
        let mut presets = mode_presets();
        presets.kr.night = LandMaterialUniformsPresets {
            global_lighting: None,
            client_lighting: Some(lighting),
            ..preset(0.0)
        };
        let [classic, _, kr_like, _] = RENDER_STYLE_PRESETS;
        let mut u = uniform_state();
        u.global_lighting = 0.3;

        store_active_and_apply_render_preset(&mut u, &mut presets, kr_like, ShaderPresetPeriod::Night);
        assert_eq!(u.client_lighting, Some(lighting));
        assert_eq!(u.global_lighting, crate::configs::shader_presets::CLIENT_LIT_SCENE_MULTIPLIER);

        // Live edits go back to the client-lit slot without a global_lighting.
        u.client_lighting.as_mut().unwrap().light_level = MAX_CLIENT_LIGHT_LEVEL;
        store_active_and_apply_render_preset(&mut u, &mut presets, classic, ShaderPresetPeriod::Night);
        assert_eq!(presets.kr.night.client_lighting.map(|lighting| lighting.light_level), Some(MAX_CLIENT_LIGHT_LEVEL));
        assert_eq!(presets.kr.night.global_lighting, None);
        assert_eq!(u.client_lighting, None);
        assert_eq!(u.global_lighting, 3.0);
    }

    #[test]
    fn fog_noise_needs_a_fog_density() {
        let noise_only = Vec4::new(0.0, 0.0, 0.55, 0.16);
        assert!(!fog_has_haze(noise_only));
        assert!(fog_has_haze(Vec4::new(FOG_DENSITY_MIN, 0.0, 0.55, 0.16)));
        assert!(fog_has_haze(Vec4::new(0.0, FOG_DENSITY_MIN, 0.55, 0.16)));
    }

    #[test]
    fn bundled_gloom_fade_heights_and_rim_powers_are_on_their_sliders() {
        let factory = include_str!("../../../../assets/defaults/shader_presets.toml");
        let shipped = include_str!("../../../../assets/settings/shaders/land.toml");
        for text in [factory, shipped] {
            let presets: LandShaderModePresets =
                uocf::utils::config_toml::parse_strict("shader_presets.toml", text).expect("bundled");
            for group in [&presets.classic, &presets.enhanced, &presets.kr, &presets.painterly] {
                for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                    let fade = slot.lighting.gloom_params.y;
                    let power = slot.land_lighting.rim_color.w;
                    assert!(GLOOM_FADE_HEIGHT_RANGE.contains(&fade), "{fade}");
                    assert!(RIM_POWER_RANGE.contains(&power), "{power}");
                }
            }
        }
    }

    /// Only UOD-Painterly shades land per pixel by default, so only it enables the per-pixel controls; Enhanced
    /// switched to per pixel enables them but not the KR-only ones.
    #[test]
    fn per_pixel_land_controls_follow_the_style_and_its_land_shading() {
        let factory = include_str!("../../../../assets/defaults/shader_presets.toml");
        let presets: LandShaderModePresets =
            uocf::utils::config_toml::parse_strict("shader_presets.toml", factory).expect("bundled");
        let gates_of = |slot: &LandMaterialUniformsPresets, terrain_shading_model: u32| {
            let mut u = uniform_state();
            u.effects = slot.effects;
            u.land_lighting = slot.land_lighting;
            u.land_lighting.terrain_shading_model = terrain_shading_model;
            RenderStyleGates::of(&u)
        };
        let groups = [
            (&presets.classic, false, false),
            (&presets.enhanced, false, false),
            (&presets.kr, false, false),
            (&presets.painterly, true, true),
        ];
        for (group, per_pixel, kr_per_pixel) in groups {
            for slot in [&group.morning, &group.afternoon, &group.night, &group.cave] {
                let gates = gates_of(slot, slot.land_lighting.terrain_shading_model);
                assert_eq!(gates.is_per_pixel_land, per_pixel, "{gates:?}");
                assert_eq!(gates.is_kr_per_pixel_land(), kr_per_pixel, "{gates:?}");
            }
        }
        let enhanced = &presets.enhanced;
        for slot in [&enhanced.morning, &enhanced.afternoon, &enhanced.night, &enhanced.cave] {
            let gates = gates_of(slot, TERRAIN_SHADING_SMOOTH_PER_PIXEL);
            assert!(gates.is_per_pixel_land && !gates.is_kr_per_pixel_land(), "{gates:?}");
        }
    }

    /// The cached land.toml migration folds the retired KR tonemap gain only into these groups.
    #[test]
    fn kr_profile_preset_groups_are_the_kr_profile_render_styles() {
        const KR_PROFILE: u32 = 2;
        let groups: Vec<&str> = RENDER_STYLE_PRESETS
            .into_iter()
            .filter(|style| style.post_process_profile == KR_PROFILE)
            .map(|style| style.preset_group)
            .collect();
        assert_eq!(groups, crate::configs::shader_presets::KR_PROFILE_PRESET_GROUPS);
    }
}
