use crate::{/*fname,*/ impl_tracked_plugin, util_lib::tracked_plugin::*};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use super::scene::world::land::mesh_material::*;


#[derive(Resource, Clone, Copy)]

pub struct UniformState {
    pub tunables: TunablesUniform,
    pub lighting: LightingUniforms,

    // Track whether to push to GPU this frame
    pub dirty: bool,
}

impl Default for UniformState {
    fn default() -> Self {
        let (tun, light) = morning_preset(ShaderMode::KR);
        Self { tunables: tun, lighting: light, dirty: true }
    }
}

// ------------------------------ UI --------------------------------------

pub struct TerrainUiPlugin {
    pub registered_by: &'static str,
}
impl_tracked_plugin!(TerrainUiPlugin);
impl Plugin for TerrainUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<UniformState>()
            .add_systems(EguiPrimaryContextPass, terrain_ui_system)
            .add_systems(Update, push_uniforms_if_dirty);
    }
}


// ------------------------------ UI --------------------------------------

fn terrain_ui_system(mut egui_ctx: EguiContexts, mut u: ResMut<UniformState>) {
    let ctx = egui_ctx.ctx_mut().expect("No egui context?");
    egui::Window::new("Terrain Shader Controls")
        .default_pos([16.0, 16.0])
        .show(ctx, |ui| {
            // Mode and Normals
            ui.horizontal(|ui| {
                ui.label("Mode:");
                let mut mode = u.tunables.shading_mode;
                for (label, val) in [("Classic", 0u32), ("Enhanced", 1u32), ("KR", 2u32)] {
                    if ui.selectable_label(mode == val, label).clicked() {
                        mode = val;
                    }
                }
                if mode != u.tunables.shading_mode {
                    u.tunables.shading_mode = mode;
                    u.dirty = true;
                }
                ui.separator();
                ui.label("Normals:");
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

            // Toggles
            ui.collapsing("Toggles", |ui| {
                let mut changed = false;
                changed |= toggle_u32(ui, "Bent",     &mut u.tunables.enable_bent);
                changed |= toggle_u32(ui, "Fog",      &mut u.tunables.enable_fog);
                changed |= toggle_u32(ui, "Gloom",    &mut u.tunables.enable_gloom);
                changed |= toggle_u32(ui, "Tonemap",  &mut u.tunables.enable_tonemap);
                changed |= toggle_u32(ui, "Grading",  &mut u.tunables.enable_grading);
                if changed { u.dirty = true; }
            });

            // Intensities
            ui.collapsing("Intensities", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Ambient",      &mut u.tunables.ambient_strength, 0.0..=1.5);
                changed |= slider_s(ui, "Diffuse",      &mut u.tunables.diffuse_strength, 0.0..=2.0);
                changed |= slider_s(ui, "Specular",     &mut u.tunables.specular_strength, 0.0..=0.4);
                changed |= slider_s(ui, "Rim",          &mut u.tunables.rim_strength, 0.0..=0.5);
                changed |= slider_s(ui, "Fill (env)",   &mut u.tunables.fill_strength, 0.0..=1.0);
                ui.separator();
                changed |= slider_s(ui, "Sharpness Factor", &mut u.tunables.sharpness_factor, 0.5..=4.0);
                changed |= slider_s(ui, "Sharpness Mix",    &mut u.tunables.sharpness_mix,    0.0..=1.0);
                ui.separator();
                changed |= slider_s(ui, "Exposure", &mut u.lighting.exposure, 0.5..=2.0);
                if changed { u.dirty = true; }
            });

            // Lighting colors
            ui.collapsing("Lighting Colors", |ui| {
                let mut changed = false;
                changed |= color3(ui, "Light Color",   &mut u.lighting.light_color.into());
                changed |= color3(ui, "Ambient Color", &mut u.lighting.ambient_color.into());
                ui.separator();
                changed |= color4(ui, "Fill Sky (rgb + strength.a)",    &mut u.lighting.fill_sky_color.into());
                changed |= color4(ui, "Fill Ground (rgb + strength.a)", &mut u.lighting.fill_ground_color.into());
                ui.separator();
                changed |= color4(ui, "Rim (rgb + power.w)",            &mut u.lighting.rim_color.into());
                if changed { u.dirty = true; }
            });

            // Grading (Vibrant)
            ui.collapsing("KR Color Grading (Vibrant)", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Grade Strength",   &mut u.lighting.grade_params[0], 0.0..=2.0);
                changed |= slider_s(ui, "Headroom Reserve", &mut u.lighting.grade_params[1], 0.0..=0.5);
                changed |= slider_s(ui, "Fill Chroma Tint", &mut u.lighting.grade_params[2], 0.0..=1.0);

                // headroom on/off via local bool to avoid overlapping borrows
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
                changed |= slider_s(ui, "Vibrance",   &mut u.lighting.grade_extra[0], 0.0..=1.5);
                changed |= slider_s(ui, "Saturation", &mut u.lighting.grade_extra[1], 0.5..=2.0);
                changed |= slider_s(ui, "Contrast",   &mut u.lighting.grade_extra[2], 0.5..=2.0);
                changed |= slider_s(ui, "Split Tone Strength", &mut u.lighting.grade_extra[3], 0.0..=2.0);
                if changed { u.dirty = true; }
            });

            // Gloom
            ui.collapsing("Gloom (Moody Cool Darkening)", |ui| {
                let mut changed = false;
                changed |= slider_s(ui, "Amount",         &mut u.lighting.gloom_params[0], 0.0..=1.0);
                changed |= slider_s(ui, "Height Falloff", &mut u.lighting.gloom_params[1], 0.0..=0.05);
                changed |= slider_s(ui, "Shadow Bias",    &mut u.lighting.gloom_params[2], 0.0..=1.0);
                ui.label("Gloom ≠ Fog: It’s multiplicative, cool-toned, and biased to shadows.");
                if changed { u.dirty = true; }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Presets:");
                if ui.button("Morning").clicked()   { let (t,l)=morning_preset(mode_from_u(u.tunables.shading_mode));   u.tunables=t; u.lighting=l; u.dirty=true; }
                if ui.button("Afternoon").clicked() { let (t,l)=afternoon_preset(mode_from_u(u.tunables.shading_mode)); u.tunables=t; u.lighting=l; u.dirty=true; }
                if ui.button("Night").clicked()     { let (t,l)=night_preset(mode_from_u(u.tunables.shading_mode));     u.tunables=t; u.lighting=l; u.dirty=true; }
                if ui.button("Cave").clicked()      { let (t,l)=cave_preset(mode_from_u(u.tunables.shading_mode));      u.tunables=t; u.lighting=l; u.dirty=true; }
                ui.separator();
                if ui.button("Classic Mode").clicked()  { u.tunables.shading_mode = 0; u.dirty = true; }
                if ui.button("Enhanced Mode").clicked() { u.tunables.shading_mode = 1; u.dirty = true; }
                if ui.button("KR Mode").clicked()       { u.tunables.shading_mode = 2; u.dirty = true; }
            });
        });
}

// ------------------------- GPU Apply System ------------------------------
//
// Bevy 0.16: query MeshMaterial3d<LandCustomMaterial> (typed material handle component)
// and mutate the corresponding material assets. AsBindGroup re-uploads uniforms.

fn push_uniforms_if_dirty(
    mut mats: ResMut<Assets<LandCustomMaterial>>,
    q_mat_handles: Query<&MeshMaterial3d<LandCustomMaterial>>,
    mut u: ResMut<UniformState>,
) {
    if !u.dirty { return; }

    for mat_handle in q_mat_handles.iter() {
        if let Some(mat) = mats.get_mut(&mat_handle.0) {
            // Ensure your material extension struct exposes these fields:
            //   tunables_uniform: TunablesUniform
            //   lighting_uniform: LightingUniforms
            mat.extension.tunables_uniform = u.tunables;
            mat.extension.lighting_uniform = u.lighting;
        }
    }

    u.dirty = false;
}

// --------------------------- UI Helpers ----------------------------------

fn mode_from_u(v: u32) -> ShaderMode {
    match v {
        0 => ShaderMode::Classic2D,
        1 => ShaderMode::Enhanced2D,
        _ => ShaderMode::KR,
    }
}

fn toggle_u32(ui: &mut egui::Ui, label: &str, val: &mut u32) -> bool {
    let mut b = *val != 0;
    let changed = ui.checkbox(&mut b, label).changed();
    if changed {
        *val = if b { 1 } else { 0 };
    }
    changed
}

fn slider_s(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.add(egui::Slider::new(val, range).text(label)).changed()
}

// Correct egui APIs that take &mut [f32;N] directly, fixing the previous mismatch.
fn color3(ui: &mut egui::Ui, label: &str, rgb: &mut [f32; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.color_edit_button_rgb(rgb).changed() {
            changed = true;
        }
    });
    changed
}

fn color4(ui: &mut egui::Ui, label: &str, rgba: &mut [f32; 4]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.color_edit_button_rgba_unmultiplied(rgba).changed() {
            changed = true;
        }
    });
    changed
}
