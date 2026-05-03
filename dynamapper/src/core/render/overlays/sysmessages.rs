use crate::ingame_sysmessage_logger::{self, InGameLog};
use crate::{
    core::render::{dialogs, scene::camera::UiCameraResource},
    prelude::*,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use std::time::Duration;

pub struct SystemMessagesPlugin {
    pub registered_by: &'static str,
}
crate::impl_tracked_plugin!(SystemMessagesPlugin);

impl Plugin for SystemMessagesPlugin {
    fn build(&self, app: &mut App) {
        crate::util_lib::tracked_plugin::log_plugin_build(self);
        app.add_systems(
            EguiPrimaryContextPass,
            sys_render_sysmessages.run_if(in_state(AppState::InGame)),
        );
    }
}

const FONT_SIZE: f32 = 16.0;
const LOG_MAX_AGE_SEC: u64 = 8;
const MAX_VISIBLE_MESSAGES: usize = 5;
const LOG_CLEANUP_INTERVAL_SEC: f32 = 0.5;

fn sys_render_sysmessages(
    mut contexts: EguiContexts,
    egui_ui_camera: Res<UiCameraResource>,
    settings: Res<crate::configs::settings::Settings>,
    time: Res<Time>,
    mut last_cleanup_acc: Local<f32>,
    mut cached_logs: Local<Vec<InGameLog>>,
    mut cached_log_count: Local<usize>,
) {
    let Some(ctx) = dialogs::get_egui_context_ready_mut(&mut contexts, &egui_ui_camera) else {
        return;
    };

    *last_cleanup_acc += time.delta_secs();
    let mut refreshed_logs = false;
    if *last_cleanup_acc >= LOG_CLEANUP_INTERVAL_SEC {
        ingame_sysmessage_logger::clear_expired(Duration::from_secs(LOG_MAX_AGE_SEC));
        *last_cleanup_acc = 0.0;
        refreshed_logs = true;
    }

    let current_log_count = ingame_sysmessage_logger::len();
    if refreshed_logs || *cached_log_count != current_log_count {
        *cached_logs = ingame_sysmessage_logger::get_logs();
        *cached_log_count = current_log_count;
    }

    if cached_logs.is_empty() {
        return;
    }

    // 3. Render at bottom-left
    egui::Area::new(egui::Id::new("in_game_logger_area"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(20.0, -20.0))
        .show(ctx, |ui| {
            // Semi-transparent background for the whole log area
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(100))
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.set_max_width(400.0);

                    let mut scroll_area = egui::ScrollArea::vertical()
                        .max_height(120.0) // Roughly 5 lines
                        .auto_shrink([false, true])
                        .stick_to_bottom(true);

                    // If we exceed 5 messages, show the scrollbar
                    use egui::scroll_area::ScrollBarVisibility;
                    if cached_logs.len() <= MAX_VISIBLE_MESSAGES {
                        scroll_area =
                            scroll_area.scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden);
                    } else {
                        scroll_area = scroll_area
                            .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded);
                    }

                    let scale = settings.app.window.sysmessages_scale;
                    scroll_area.show(ui, |ui| {
                        ui.vertical(|ui| {
                            for log in cached_logs.iter() {
                                render_log_line(ui, log, scale);
                            }
                        });
                    });
                });
        });
}
fn render_log_line(ui: &mut egui::Ui, log: &InGameLog, scale: f32) {
    let now = std::time::Instant::now();
    let age = now.duration_since(log.timestamp).as_secs_f32();

    // Fade out in the last 1 second as requested
    let alpha = if age > (LOG_MAX_AGE_SEC as f32 - 1.0) {
        ((LOG_MAX_AGE_SEC as f32 - age) / 1.0).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let srgba = log.color.to_srgba();
    let color = egui::Color32::from_rgba_unmultiplied(
        (srgba.red * 255.0) as u8,
        (srgba.green * 255.0) as u8,
        (srgba.blue * 255.0) as u8,
        (alpha * 255.0) as u8,
    );

    let text = format!("{} {}", log.symbol, log.message);
    ui.label(
        egui::RichText::new(text)
            .color(color)
            .size(FONT_SIZE * scale),
    );
}
