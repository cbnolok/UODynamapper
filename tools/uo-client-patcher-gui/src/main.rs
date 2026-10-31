//! UO Client Patcher: opens a client executable, edits its facet sizes and seam, previews the exact byte changes and
//! writes a patched copy with its manifest.

use color_eyre::eyre;
use eframe::egui;
use gui_shared::{run_native_with_setup, NativeWindow};

mod app;
mod logic;
mod panel_logger;
mod ui;

use app::{PatcherApp, SETTINGS_KEY};

const APP_ID: &str = "uo_client_patcher_gui";
const APP_VIEWPORT_WIDTH: f32 = 1100.0;
const APP_VIEWPORT_HEIGHT: f32 = 800.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 900.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 600.0;
/// Loads a system font for the symbols of the rows (✔ ⚠ ✖) and the dashes.
const INSTALL_UNICODE_FONT_FALLBACK: bool = true;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    run_native_with_setup(
        APP_ID,
        NativeWindow {
            title: "UO Client Patcher",
            width: APP_VIEWPORT_WIDTH,
            height: APP_VIEWPORT_HEIGHT,
            min_width: APP_MIN_VIEWPORT_WIDTH,
            min_height: APP_MIN_VIEWPORT_HEIGHT,
        },
        INSTALL_UNICODE_FONT_FALLBACK,
        |cc| Ok(Box::new(PatcherApp::new(cc))),
    )
    .map_err(|error| eyre::eyre!("eframe error: {error}"))
}

impl eframe::App for PatcherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_window(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }
}
