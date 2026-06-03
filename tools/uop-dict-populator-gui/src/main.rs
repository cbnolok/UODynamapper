mod app;
mod core;
mod dialog;

use app::UopPopulatorApp;
use eframe::egui;

fn main() -> eframe::Result {
    dialog::normalize_linux_portal_env_before_threads();
    let _ = udd_logging::install_paris_logger();
    
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([400.0, 300.0]),
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
