// Terrain shader live UI (Bevy 0.16 + egui)
// - Controls TunablesUniform and LightingUniforms as used by your WGSL
// - Writes to material assets so Bevy re-uploads uniforms automatically
// - Shading modes:
//      0 = Classic 2D (vertex/Gouraud; faithful to original)
//      1 = Enhanced 2D (fragment; subtle improvements, still faithful)
//      2 = KR-like     (fragment; painterly, vibrant, rim + gloom)
//

use crate::{
    external_data::shader_presets::UniformState, impl_tracked_plugin, // prelude::*,
    util_lib::tracked_plugin::*,
};

use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

// Your material types and uniform structs (exported by your mesh_material module):
// - LandCustomMaterial: ExtendedMaterial<StandardMaterial, LandMaterialExtension>
// - TunablesUniform, LightingUniforms
// - ShaderMode, morning_preset, afternoon_preset, night_preset, cave_preset
use super::scene::world::land::mesh_material::*;

// Plugin that draws the UI and applies changes to materials.
pub struct TerrainUiPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainUiPlugin);

impl Plugin for TerrainUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            // Draw UI in the egui pass
            .add_systems(EguiPrimaryContextPass, terrain_ui_system)
            // Push "dirty" values into GPU materials
            .add_systems(Update, push_uniforms_if_dirty);
    }
}

// ============================== UI SYSTEM ===============================
// Renders a window with controls for mode, toggles, intensities, colors,
// grading, gloom, and presets. Updates UniformState + sets "dirty" when changed.

fn terrain_ui_system(
    mut egui_ctx: EguiContexts,
    mut u: ResMut<UniformState>,
    shader_presets: Res<LandShaderModePresets>,
) {
    let ctx = egui_ctx.ctx_mut().expect("No egui context?");
    egui::Window::new("Terrain Shader Controls")
        .default_pos([16.0, 80.0])
        .default_open(false)
        .resizable(true)
        .show(ctx, |ui| {
            ui.label("Modes: 0=Classic (vertex), 1=Enhanced (fragment), 2=KR-like (fragment).");
            ui.label(
                "Classic aims for original fidelity. Enhanced is subtle. KR is vibrant/painterly.",
            );
            ui.add_space(6.0);

            // --------------------- Mode & Normals ---------------------
            ui.horizontal(|ui| {
                ui.strong("Shading Mode:");
                let mut mode = u.tunables.shading_mode;
                for (label, val) in [("Classic", 0u32), ("Enhanced", 1u32), ("KR", 2u32)] {
                    if ui.selectable_label(mode == val, label).clicked() {
                        mode = val;
                    }
                }
                if mode != u.tunables.shading_mode {
                    u.tunables.shading_mode = mode;
                    u.dirty = true; // push to GPU next frame
                }

                ui.separator();

                ui.strong("Normals:");
                let mut nm = u.tunables.normal_mode;
                if ui.selectable_label(nm == 0, "Geometric").clicked() {
                    nm = 0;
                }
                if ui.selectable_label(nm == 1, "Bicubic").clicked() {
                    nm = 1;
                }
                if nm != u.tunables.normal_mode {
                    u.tunables.normal_mode = nm;
                    u.dirty = true;
                }
            });

            ui.separator();

            // ------------------------- Toggles -------------------------
            // Show only toggles that are relevant to the selected shading mode.
            ui.collapsing("Toggles", |ui| {
                let mut changed = false;

                // Fog and Tonemap apply in ANY shading mode (keep available always)
                changed |= toggle_u32(ui, "Fog", &mut u.tunables.enable_fog);
                changed |= toggle_u32(ui, "Tonemap", &mut u.tunables.enable_tonemap);

                // Color grading & fragment-only features only when in fragment modes
                let is_classic = u.tunables.shading_mode == 0;
                if !is_classic {
                    changed |= toggle_u32(ui, "Color Grading (fragment)", &mut u.tunables.enable_grading);
                    changed |= toggle_u32(ui, "Bent normals (fragment)", &mut u.tunables.enable_bent);

                    // Gloom: fragment-only semantic
                    let toggled =
                        toggle_u32(ui, "Gloom (fragment)", &mut u.tunables.enable_gloom);
                    if toggled {
                        if u.tunables.enable_gloom == 0 {
                            u.lighting.gloom_params[0] = 0.0;
                        } else if u.lighting.gloom_params[0] <= 0.0001 {
                            u.lighting.gloom_params[0] = 0.20;
                        }
                    }
                    changed |= toggled;

                    // Blur is fragment-only
                    changed |= toggle_u32(ui, "Blur (fragment)", &mut u.tunables.enable_blur);
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
                changed |= slider_s(ui, "Ambient", &mut u.tunables.ambient_strength, 0.0..=1.5);

                // Diffuse is meaningful only for fragment modes. For Classic (vertex)
                // the shader uses the precomputed vertex Lambert (old behavior) and
                // we intentionally hide the diffuse control to avoid confusion.
                let is_classic = u.tunables.shading_mode == 0;
                if !is_classic {
                    changed |= slider_s(ui, "Diffuse", &mut u.tunables.diffuse_strength, 0.0..=2.0);
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
                        &mut u.tunables.specular_strength,
                        0.0..=0.4,
                    );
                    changed |= slider_s(
                        ui,
                        "Fill (env) (fragment only)",
                        &mut u.tunables.fill_strength,
                        0.0..=1.0,
                    );
                    ui.separator();
                    changed |= slider_s(
                        ui,
                        "Sharpness Factor (fragment only)",
                        &mut u.tunables.sharpness_factor,
                        0.5..=4.0,
                    );
                    changed |= slider_s(
                        ui,
                        "Sharpness Mix (fragment only)",
                        &mut u.tunables.sharpness_mix,
                        0.0..=1.0,
                    );
                    ui.separator();
                    // Blur parameters (fragment-only)
                    changed |= slider_s(
                        ui,
                        "Blur Strength (fragment only)",
                        &mut u.tunables.blur_strength,
                        0.0..=0.5,
                    );
                    changed |= slider_s(
                        ui,
                        "Blur Radius (screen pixels) (fragment only)",
                        &mut u.tunables.blur_radius,
                        0.5..=8.0,
                    );

                    changed |= slider_s(ui, "Rim (KR only)", &mut u.tunables.rim_strength, 0.0..=0.5);
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
                    let mut v = u.lighting.light_color.clone();
                    if color3(ui, "Light Color", &mut v) {
                        u.lighting.light_color = v;
                        changed = true;
                    }
                }
                {
                    let mut v = u.lighting.ambient_color.clone();
                    if color3(ui, "Ambient Color", &mut v) {
                        u.lighting.ambient_color = v;
                        changed = true;
                    }
                }

                ui.separator();

                {
                    let mut v = u.lighting.fill_sky_color.clone();
                    if color4(ui, "Fill Sky (rgb + strength.a) (fragment only)", &mut v) {
                        u.lighting.fill_sky_color = v;
                        changed = true;
                    }
                }
                {
                    let mut v = u.lighting.fill_ground_color.clone();
                    if color4(
                        ui,
                        "Fill Ground (rgb + strength.a) (fragment only)",
                        &mut v,
                    ) {
                        u.lighting.fill_ground_color = v;
                        changed = true;
                    }
                }

                ui.separator();

                {
                    let mut v = u.lighting.rim_color.clone();
                    if color4(ui, "Rim (rgb + power.w) (fragment only)", &mut v) {
                        u.lighting.rim_color = v;
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
                if u.tunables.shading_mode == 0 {
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
                if u.tunables.shading_mode == 0 {
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
            });

            ui.separator();

            // ------------------------ Presets -------------------------
            ui.horizontal(|ui| {
                ui.strong("Presets:");
                if ui.button("Morning").clicked() {
                    let preset = match u.tunables.shading_mode {
                        0 => &shader_presets.classic.morning,
                        1 => &shader_presets.enhanced.morning,
                        _ => &shader_presets.kr.morning,
                    };
                    u.tunables = preset.tunables;
                    u.lighting = preset.lighting;
                    u.global_lighting = 1.0;
                    u.dirty = true;
                }
                if ui.button("Afternoon").clicked() {
                    let preset = match u.tunables.shading_mode {
                        0 => &shader_presets.classic.afternoon,
                        1 => &shader_presets.enhanced.afternoon,
                        _ => &shader_presets.kr.afternoon,
                    };
                    u.tunables = preset.tunables;
                    u.lighting = preset.lighting;
                    u.global_lighting = 1.0;
                    u.dirty = true;
                }
                if ui.button("Night").clicked() {
                    let preset = match u.tunables.shading_mode {
                        0 => &shader_presets.classic.night,
                        1 => &shader_presets.enhanced.night,
                        _ => &shader_presets.kr.night,
                    };
                    u.tunables = preset.tunables;
                    u.lighting = preset.lighting;
                    u.global_lighting = 1.0;
                    u.dirty = true;
                }
                if ui.button("Cave").clicked() {
                    let preset = match u.tunables.shading_mode {
                        0 => &shader_presets.classic.cave,
                        1 => &shader_presets.enhanced.cave,
                        _ => &shader_presets.kr.cave,
                    };
                    u.tunables = preset.tunables;
                    u.lighting = preset.lighting;
                    u.global_lighting = 1.0;
                    u.dirty = true;
                }
            });
        });
}

// push_uniforms_if_dirty updates ALL LandCustomMaterial assets.
// That guarantees that materials not referenced this frame still get the new values
// (fixes "stale lighting when moving" problem).
fn push_uniforms_if_dirty(
    mut mats: ResMut<Assets<LandCustomMaterial>>,
    _q_mat_handles: Query<&MeshMaterial3d<LandCustomMaterial>>, // kept for parity; unused
    mut u: ResMut<UniformState>,
) {
    if !u.dirty {
        return;
    }

    for (_handle, mat) in mats.iter_mut() {
        // Overwrite the embedded uniforms used by the material extension.
        mat.extension.tunables_uniform = u.tunables;
        mat.extension.lighting_uniform = u.lighting;

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
