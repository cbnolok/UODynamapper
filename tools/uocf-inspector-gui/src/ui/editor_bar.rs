//! The controls every file editor has: `Edit`, and while editing `Revert to loaded`, the save button, the unsaved marker
//! and the last error; and the file dialog of a save.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::app::UopInspectorApp;

const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const SAVE_HOVER: &str = "Writes a copy. The loaded file itself is refused as the target.";

pub(super) const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);

/// What the user asked of the bar this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BarAction {
    Revert,
    Save,
}

/// `Edit`, and while it is on the rest of the bar, on the current row.
pub(super) fn editor_bar(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    edit_hover: &str,
    dirty: bool,
    save_text: &str,
    error: &mut Option<String>,
) -> Option<BarAction> {
    let mut action = None;
    ui.toggle_value(enabled, "Edit").on_hover_text(edit_hover);
    if !*enabled {
        return None;
    }
    if ui.add_enabled(dirty, egui::Button::new("Revert to loaded")).clicked() {
        action = Some(BarAction::Revert);
    }
    if ui.button(save_text).on_hover_text(SAVE_HOVER).clicked() {
        action = Some(BarAction::Save);
    }
    if dirty {
        ui.colored_label(WARNING_COLOR, "* unsaved changes");
    }
    if let Some(message) = error.clone() {
        ui.colored_label(ERROR_COLOR, message);
        if ui.small_button("Dismiss").clicked() {
            *error = None;
        }
    }
    action
}

/// Asks where to write `file_name`; None when the user cancels.
pub(super) fn pick_output(file_name: &str, (filter, extensions): (&str, &[&str])) -> Option<PathBuf> {
    crate::dialog::file_dialog().add_filter(filter, extensions).set_file_name(file_name).save_file()
}

/// Says that `output` was written.
pub(super) fn report_saved(app: &mut UopInspectorApp, output: &Path) {
    let message = format!("Saved {}.", output.display());
    app.status_message = message.clone();
    app.log(message);
}
