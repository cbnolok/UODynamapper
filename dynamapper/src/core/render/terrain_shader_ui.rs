// Terrain shader live UI (Bevy 0.16 + egui)
// - Controls TunablesUniform and LightingUniforms as used by your WGSL
// - Writes to material assets so Bevy re-uploads uniforms automatically
// - Shading modes:
//      0 = Classic 2D (vertex/Gouraud; faithful to original)
//      1 = Enhanced 2D (fragment; subtle improvements, still faithful)
//      2 = KR-like     (fragment; painterly, vibrant, rim + gloom)
//

use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};

use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

// Your material types and uniform structs (exported by your mesh_material module):
// - LandCustomMaterial: ExtendedMaterial<StandardMaterial, LandMaterialExtension>
// - TunablesUniform, LightingUniforms
// - ShaderMode, morning_preset, afternoon_preset, night_preset, cave_preset
use super::scene::world::land::mesh_material::*;

// Holds current UI-edited values and a dirty flag.
// Bevy detects asset changes and re-uploads uniforms automatically.
#[derive(Resource, Clone, Copy)]
pub struct UniformState {
    pub tunables: TunablesUniform,  // modes/toggles + intensities
    pub lighting: LightingUniforms, // light/fill/rim + grading + gloom + exposure
    pub dirty: bool,                // when true, push to GPU materials this frame
}

impl Default for UniformState {
    fn default() -> Self {
        let (tun, light) = morning_preset(ShaderMode::KR);
        Self {
            tunables: tun,
            lighting: light,
            dirty: true,
        }
    }
}

// Plugin that draws the UI and applies changes to materials.
pub struct TerrainUiPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainUiPlugin);

impl Plugin for TerrainUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<UniformState>()
            // Draw UI in the egui pass
            .add_systems(EguiPrimaryContextPass, terrain_ui_system)
            // Push "dirty" values into GPU materials
            .add_systems(Update, push_uniforms_if_dirty);
    }
}

// ============================== UI SYSTEM ===============================
// Renders a window with controls for mode, toggles, intensities, colors,
// grading, gloom, and presets. Updates UniformState + sets "dirty" when changed.

fn terrain_ui_system(mut egui_ctx: EguiContexts, mut u: ResMut<UniformState>) {
    let ctx = egui_ctx.ctx_mut().expect("No egui context?");
    egui::Window::new("Terrain Shader Controls")
        .default_pos([16.0, 16.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.label("Modes: 0=Classic (vertex), 1=Enhanced (fragment), 2=KR-like (fragment).");
            ui.label("Classic aims for original fidelity. Enhanced is subtle. KR is vibrant/painterly.");
            ui.add_space(6.0);

            // --------------------- Mode & Normals ---------------------
            ui.horizontal(|ui| {
                ui.strong("Mode:");
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
                if ui.selectable_label(nm == 0, "Geometric").clicked() { nm = 0; }
                if ui.selectable_label(nm == 1, "Bicubic").clicked()   { nm = 1; }
                if nm != u.tunables.normal_mode {
                    u.tunables.normal_mode = nm;
                    u.dirty = true;
                }
            });

            ui.separator();

            // ------------------------- Toggles -------------------------
            // Fragment-only features show "(non-classic only)" so it’s clear they won’t affect mode 0.
            // - Bent (non-classic only): biases normals toward "up" in concavities (fake GI)
            // - Fog:  multiplicative tint by animated mask (applies in any mode)
            // - Gloom (non-classic only): cool, shadow-biased multiplicative darkening (mood)
            // - Tonemap: exposure + curve (applies in any mode)
            // - Grading (non-classic only): vibrance/saturation/contrast/split-toning
            ui.collapsing("Toggles", |ui| {
                let mut changed = false;

                changed |= toggle_u32(ui, "Bent (non-classic only)",     &mut u.tunables.enable_bent);
                changed |= toggle_u32(ui, "Fog",      &mut u.tunables.enable_fog);
                // Gloom toggle: hard-off sets amount=0; turning back on restores a sensible default
                {
                    //let before = u.tunables.enable_gloom;
                    let toggled = toggle_u32(ui, "Gloom (non-classic only)", &mut u.tunables.enable_gloom);
                    if toggled {
                        if u.tunables.enable_gloom == 0 {
                            // amount
                            u.lighting.gloom_params[0] = 0.0;
                        } else if u.lighting.gloom_params[0] <= 0.0001 {
                            u.lighting.gloom_params[0] = 0.20;
                        }
                    }
                    changed |= toggled;
                }
                 changed |= toggle_u32(ui, "Tonemap",  &mut u.tunables.enable_tonemap);
                changed |= toggle_u32(ui, "Grading (non-classic only)",  &mut u.tunables.enable_grading);
                // New: Blur toggle (fragment-only)
                changed |= toggle_u32(ui, "Blur (non-classic only)",     &mut u.tunables.enable_blur);

                if changed { u.dirty = true; }
            });

            // ------------------------ Intensities ----------------------
            // Theory quick guide:
            // - Ambient: base brightness in shadows (scalar on albedo)
            // - Diffuse: lambert(sun) shaped light on lit faces (scalar on albedo)
            // - Specular (non-classic only): small neutral highlight (additive)
            // - Rim (non-classic only): silhouette lift (shadow-biased; headroom-limited)
            // - Fill (non-classic only): sky/ground env; luma adds brightness, chroma adds subtle tint
            // - Sharpness (non-classic only): lambert^factor; Mix blends classic and sharpened
            ui.collapsing("Intensities", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Ambient",                   &mut u.tunables.ambient_strength, 0.0..=1.5);
                changed |= slider_s(ui, "Diffuse",                   &mut u.tunables.diffuse_strength, 0.0..=2.0);
                changed |= slider_s(ui, "Specular (non-classic only)", &mut u.tunables.specular_strength, 0.0..=0.4);
                changed |= slider_s(ui, "Rim (non-classic only)",      &mut u.tunables.rim_strength, 0.0..=0.5);
                changed |= slider_s(ui, "Fill (env) (non-classic only)", &mut u.tunables.fill_strength, 0.0..=1.0);
                ui.separator();
                changed |= slider_s(ui, "Sharpness Factor (non-classic only)", &mut u.tunables.sharpness_factor, 0.5..=4.0);
                changed |= slider_s(ui, "Sharpness Mix (non-classic only)",    &mut u.tunables.sharpness_mix,    0.0..=1.0);
                ui.separator();
                // New: Subtle pre-shade blur of base albedo
                // Strength is a mix factor; radius is in UV units (very small numbers)
                changed |= slider_s(ui, "Blur Strength (non-classic only)", &mut u.tunables.blur_strength, 0.0..=0.5);
                changed |= slider_s(ui, "Blur Radius (UV) (non-classic only)", &mut u.tunables.blur_radius, 0.0005..=0.01);
ui.separator();

                changed |= slider_s(ui, "Exposure (Tonemap)",         &mut u.lighting.exposure, 0.5..=2.0);
                if changed { u.dirty = true; }
            });

            // ---------------------- Lighting Colors --------------------
            // Stored in the lighting UBO so you can retint per environment:
            // - light_color: key light (sun) color (generally slightly warm)
            // - ambient_color: base cool ambient (used also by fog/gloom tint)
            // - fill_sky_color (non-classic only): rgb + strength in .a
            // - fill_ground_color (non-classic only): rgb + strength in .a
            // - rim_color (non-classic only): rgb + rim "power" in .w (2..4 = thin edge)
            ui.collapsing("Lighting Colors", |ui| {
                let mut changed = false;

                // Light color (Vec3 UI -> [f32;3] uniform)
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
                    if color4(ui, "Fill Sky (rgb + strength.a) (non-classic only)", &mut v) {
                        u.lighting.fill_sky_color = v;
                        changed = true;
                    }
                }
                {
                    let mut v = u.lighting.fill_ground_color.clone();
                    if color4(ui, "Fill Ground (rgb + strength.a) (non-classic only)", &mut v) {
                        u.lighting.fill_ground_color = v;
                        changed = true;
                    }
                }

                ui.separator();

                {
                    let mut v = u.lighting.rim_color.clone();
                    if color4(ui, "Rim (rgb + power.w) (non-classic only)", &mut v) {
                        u.lighting.rim_color = v;
                        changed = true;
                    }
                }

                if changed { u.dirty = true; }
            });

            // ----------------- KR Color Grading (Vibrant) --------------
            // (non-classic only)
            // - grade_params:
            //   [0] grade_strength: overall grading amount
            //   [1] headroom_reserve: keep brightness room for rim/spec (prevents bleaching)
            //   [2] hemi_chroma_tint: how much sky/ground color to add as chroma
            //   [3] headroom_on: 0/1 runtime toggle for headroom limiting
            // - grade_extra (vibrant engine):
            //   [0] vibrance: boosts low-saturation regions more than high-sat
            //   [1] saturation: global saturation factor
            //   [2] contrast: global contrast factor
            //   [3] split_strength: cool lows + warm highs split-toning
            ui.collapsing("KR Color Grading (Vibrant) (non-classic only)", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Grade Strength",      &mut u.lighting.grade_params[0], 0.0..=2.0);
                changed |= slider_s(ui, "Headroom Reserve",    &mut u.lighting.grade_params[1], 0.0..=0.5);
                changed |= slider_s(ui, "Fill Chroma Tint",    &mut u.lighting.grade_params[2], 0.0..=1.0);

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
                changed |= slider_s(ui, "Vibrance (selective sat)", &mut u.lighting.grade_extra[0], 0.0..=1.5);
                changed |= slider_s(ui, "Saturation (global)",      &mut u.lighting.grade_extra[1], 0.5..=2.0);
                changed |= slider_s(ui, "Contrast (global)",        &mut u.lighting.grade_extra[2], 0.5..=2.0);
                changed |= slider_s(ui, "Split Tone Strength",      &mut u.lighting.grade_extra[3], 0.0..=2.0);
                if changed { u.dirty = true; }
            });

            // ------------------------ Gloom ----------------------------
            // (non-classic only)
            // Adds moody, cool darkening biased to shadows (distinct from fog).
            // - gloom_params:
            //   [0] amount: overall strength (0..1)
            //   [1] height_falloff: exp falloff with world height (0..0.05 typical)
            //   [2] shadow_bias: bias toward shadowed faces (0..1)
            ui.collapsing("Gloom (Moody Cool Darkening) (non-classic only)", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Amount",         &mut u.lighting.gloom_params[0], 0.0..=1.0);
                changed |= slider_s(ui, "Height Fade Height (world units) (non-classic only)", &mut u.lighting.gloom_params[1], 0.0..=200.0);
                changed |= slider_s(ui, "Shadow Bias",    &mut u.lighting.gloom_params[2], 0.0..=1.0);
                ui.label("Tip: Gloom is multiplicative and keeps hues intact; use with KR mode for mood.");
                if changed { u.dirty = true; }
            });

                        // ------------------------ Fog ------------------------------
            // Distance + height exponential fog with optional noise modulation.
            // Note: fog is in SceneUniform, not LightingUniforms.
            ui.collapsing("Fog Params", |ui| {
                let mut changed = false;
                // Tint + max mix (alpha)
                let mut fog = u.lighting.fog_color;
                if color4(ui, "Fog Color (alpha = max mix)", &mut fog) { u.lighting.fog_color = fog; changed = true; }
                // Densities and noise
                changed |= slider_s(ui, "Distance Density", &mut u.lighting.fog_params[0], 0.0..=0.2);
                changed |= slider_s(ui, "Height Density",   &mut u.lighting.fog_params[1], 0.0..=0.2);
                changed |= slider_s(ui, "Noise Scale",      &mut u.lighting.fog_params[2], 0.0..=2.0);
                changed |= slider_s(ui, "Noise Strength",   &mut u.lighting.fog_params[3], 0.0..=1.0);
                if changed { u.dirty = true; }
            });

            ui.separator();

            // ------------------------ Presets -------------------------
            // Apply environment presets using the current shading mode.
            ui.horizontal(|ui| {
                ui.strong("Presets:");
                if ui.button("Morning").clicked() {
                    let (t, l) = morning_preset(mode_from_u(u.tunables.shading_mode));
                    u.tunables = t; u.lighting = l; u.dirty = true;
                }
                if ui.button("Afternoon").clicked() {
                    let (t, l) = afternoon_preset(mode_from_u(u.tunables.shading_mode));
                    u.tunables = t; u.lighting = l; u.dirty = true;
                }
                if ui.button("Night").clicked() {
                    let (t, l) = night_preset(mode_from_u(u.tunables.shading_mode));
                    u.tunables = t; u.lighting = l; u.dirty = true;
                }
                if ui.button("Cave").clicked() {
                    let (t, l) = cave_preset(mode_from_u(u.tunables.shading_mode));
                    u.tunables = t; u.lighting = l; u.dirty = true;
                }
            });
        });
}

fn push_uniforms_if_dirty(
    mut mats: ResMut<Assets<LandCustomMaterial>>,
    q_mat_handles: Query<&MeshMaterial3d<LandCustomMaterial>>,
    mut u: ResMut<UniformState>,
) {
    if !u.dirty {
        return;
    }

    // Update only materials actually used by current chunk entities.
    for mat_handle in q_mat_handles.iter() {
        if let Some(mat) = mats.get_mut(&mat_handle.0) {
            mat.extension.tunables_uniform = u.tunables;
            mat.extension.lighting_uniform = u.lighting;
            mat.extension.lighting_uniform.fog_color = u.lighting.fog_color;
            mat.extension.lighting_uniform.fog_params = u.lighting.fog_params;
        }
    }

    u.dirty = false;
}

// ============================ UI HELPERS =================================
// These helpers return "changed" (bool) so callers can set u.dirty |= changed,
// avoiding overlapping &mut borrows inside the helper.

// Converts numeric mode to your enum for preset builders.
fn mode_from_u(v: u32) -> ShaderMode {
    match v {
        0 => ShaderMode::Classic2D,
        1 => ShaderMode::Enhanced2D,
        _ => ShaderMode::KR,
    }
}

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

/*
// Edit a Vec2 in-place (generic 2-parameter control). Returns true if changed.
fn color2(ui: &mut egui::Ui, label: &str, rg: &mut Vec2) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        // Two simple sliders (0..1) for clarity; swap to another UI if needed.
        let c1 = ui.add(egui::Slider::new(&mut rg.x, 0.0..=1.0).text("x")).changed();
        let c2 = ui.add(egui::Slider::new(&mut rg.y, 0.0..=1.0).text("y")).changed();
        changed = c1 || c2;
    });
    changed
}
*/

// Edit a Vec3 as RGB (0..1). Returns true if changed.
fn color3(ui: &mut egui::Ui, label: &str, v: &mut Vec3) -> bool {
    let mut changed = false;
    // egui wants &mut [f32;3]; convert temporarily and write back if changed.
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
