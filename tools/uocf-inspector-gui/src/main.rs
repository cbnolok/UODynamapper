mod app;
mod logic;
mod ui;

use app::UopInspectorApp;
use eframe::egui;

fn main() -> eframe::Result {
    let _ = udd_logging::install_paris_logger();
    
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "UOCF Inspector",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(UopInspectorApp::new(cc)))
        }),
    )
}
