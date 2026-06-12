#![allow(deprecated)]

mod app;
mod core;
mod dialog;

use app::UopPopulatorApp;
use eframe::egui;

const APP_VIEWPORT_WIDTH: f32 = 1000.0;
const APP_VIEWPORT_HEIGHT: f32 = 700.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 400.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 300.0;

fn main() -> eframe::Result {
    dialog::normalize_linux_portal_env_before_threads();
    let _ = udd_logging::install_paris_logger();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([APP_VIEWPORT_WIDTH, APP_VIEWPORT_HEIGHT])
            .with_min_inner_size([APP_MIN_VIEWPORT_WIDTH, APP_MIN_VIEWPORT_HEIGHT]),
        ..Default::default()
    };

    eframe::run_native(
        "UOP Dictionary Populator",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(UopPopulatorApp::new(cc)))
        }),
    )
}
