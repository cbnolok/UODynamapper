//! The shaders compiling, bottom right, while a batch of them shows (`render::shader_compiles`): the count done of the
//! batch, a bar, and the compiles running longest by name, with their time once slow. A single slow compile left has
//! no count to progress by, so its bar sweeps instead.

use crate::render::shader_compiles::{shader_compile_view, CompileView, SLOW_COMPILE};
use crate::render::view::camera::UiCameraResource;
use crate::ui::dialogs;
use crate::prelude::*;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

pub struct ShaderCompilesOverlayPlugin {
    pub registered_by: &'static str,
}
crate::impl_tracked_plugin!(ShaderCompilesOverlayPlugin);

impl Plugin for ShaderCompilesOverlayPlugin {
    fn build(&self, app: &mut App) {
        crate::util_lib::tracked_plugin::log_plugin_build(self);
        // In every state: the world's pipelines compile while the scene sets up.
        app.add_systems(EguiPrimaryContextPass, sys_render_shader_compiles);
    }
}

const MARGIN: f32 = 20.0;
const WIDTH: f32 = 340.0;
const PADDING: f32 = 8.0;
const CORNER_RADIUS: f32 = 4.0;
const BACKGROUND_ALPHA: u8 = 170;
const BAR_HEIGHT: f32 = 8.0;
const TITLE_SIZE: f32 = 15.0;
const NAME_SIZE: f32 = 12.0;
const NAME_COLOR: egui::Color32 = egui::Color32::from_gray(200);
const FAILED_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 110, 90);
/// The sweeping bar's moving part, as a share of the bar, and the seconds of one sweep.
const SWEEP_SHARE: f32 = 0.3;
const SWEEP_SECS: f64 = 1.5;

fn sys_render_shader_compiles(mut contexts: EguiContexts, egui_ui_camera: Res<UiCameraResource>) {
    let Some(view) = shader_compile_view() else { return };
    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut contexts, &egui_ui_camera) else { return };
    egui::Area::new(egui::Id::new("shader_compiles_overlay"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-MARGIN, -MARGIN))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(BACKGROUND_ALPHA))
                .corner_radius(CORNER_RADIUS)
                .inner_margin(PADDING)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    show_compiles(ui, &view);
                });
        });
}

fn show_compiles(ui: &mut egui::Ui, view: &CompileView) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("Compiling shaders {}/{}", view.done, view.total))
            .size(TITLE_SIZE).color(egui::Color32::WHITE));
        if view.failed > 0 {
            ui.label(egui::RichText::new(format!("{} failed", view.failed)).size(TITLE_SIZE).color(FAILED_COLOR));
        }
    });
    let single_slow = view.total - view.done == 1 && view.listed.first().is_some_and(|listed| listed.elapsed >= SLOW_COMPILE);
    if single_slow {
        sweeping_bar(ui);
    } else {
        let progress = if view.total == 0 { 1.0 } else { view.done as f32 / view.total as f32 };
        ui.add(egui::ProgressBar::new(progress).desired_width(WIDTH).desired_height(BAR_HEIGHT));
    }
    for listed in &view.listed {
        let text = if listed.elapsed >= SLOW_COMPILE {
            format!("{} \u{2014} {:.0} s", listed.name, listed.elapsed.as_secs_f64().floor())
        } else {
            listed.name.clone()
        };
        ui.add(egui::Label::new(egui::RichText::new(text).size(NAME_SIZE).color(NAME_COLOR)).truncate());
    }
    if view.more > 0 {
        ui.label(egui::RichText::new(format!("and {} more", view.more)).size(NAME_SIZE).color(NAME_COLOR));
    }
}

/// A bar whose filled part sweeps across, for a compile with no count to progress by.
fn sweeping_bar(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, BAR_HEIGHT), egui::Sense::hover());
    let visuals = ui.visuals();
    let radius = BAR_HEIGHT / 2.0;
    ui.painter().rect_filled(rect, radius, visuals.extreme_bg_color);
    let phase = (ui.input(|input| input.time) / SWEEP_SECS).fract() as f32;
    let start = rect.left() + (phase * (1.0 + SWEEP_SHARE) - SWEEP_SHARE) * rect.width();
    let end = start + SWEEP_SHARE * rect.width();
    let part = egui::Rect::from_x_y_ranges(start.max(rect.left())..=end.min(rect.right()), rect.y_range());
    ui.painter().rect_filled(part, radius, visuals.selection.bg_fill);
    ui.ctx().request_repaint();
}
