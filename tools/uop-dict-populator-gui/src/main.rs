mod app;
mod core;
mod dialog;

use app::UopPopulatorApp;
use gui_shared::{run_native_with_setup, NativeWindow};

const APP_VIEWPORT_WIDTH: f32 = 1000.0;
const APP_VIEWPORT_HEIGHT: f32 = 700.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 400.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 300.0;

fn main() -> eframe::Result {
    let _ = udd_logging::install_paris_logger();

    run_native_with_setup(
        "UOP Dictionary Populator",
        NativeWindow {
            title: "UOP Dictionary Populator",
            width: APP_VIEWPORT_WIDTH,
            height: APP_VIEWPORT_HEIGHT,
            min_width: APP_MIN_VIEWPORT_WIDTH,
            min_height: APP_MIN_VIEWPORT_HEIGHT,
        },
        false,
        |cc| Ok(Box::new(UopPopulatorApp::new(cc))),
    )
}
