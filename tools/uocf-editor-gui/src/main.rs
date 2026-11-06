mod app;
mod dialog;
mod logic;
mod ui;

use app::UocfEditorApp;
use uot_gui::{run_native_with_setup, NativeWindow};

const APP_VIEWPORT_WIDTH: f32 = 1200.0;
const APP_VIEWPORT_HEIGHT: f32 = 800.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 400.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 300.0;

fn main() -> eframe::Result {
    let _ = uot_logging::install_paris_logger();

    run_native_with_setup(
        "UOCF Editor",
        NativeWindow {
            title: "UOCF Editor",
            width: APP_VIEWPORT_WIDTH,
            height: APP_VIEWPORT_HEIGHT,
            min_width: APP_MIN_VIEWPORT_WIDTH,
            min_height: APP_MIN_VIEWPORT_HEIGHT,
        },
        true,
        |cc| Ok(Box::new(UocfEditorApp::new(cc))),
    )
}
