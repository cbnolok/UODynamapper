//! Section 6: placeholder for the login server edit.

use eframe::egui;

use super::{muted_label, WARNING_COLOR};

pub fn ui_login(ui: &mut egui::Ui) {
    ui.colored_label(WARNING_COLOR, "Login server — coming next. Nothing here is written yet.");
    muted_label(
        ui,
        "This section will set the login server address of the client, as one more edit of the same patch and \
         manifest as the facet sizes and the seam.",
    );
    ui.add_enabled_ui(false, |ui| {
        ui.horizontal(|ui| {
            ui.label("Host:");
            ui.add(egui::TextEdit::singleline(&mut String::new()).hint_text("host name or IP"));
            ui.label("Port:");
            ui.add(egui::TextEdit::singleline(&mut String::new()).hint_text("port"));
        });
    });
}
