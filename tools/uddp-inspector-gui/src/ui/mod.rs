pub mod details;
pub mod mobile_anim;
pub mod table;

use eframe::egui;

pub fn metadata_grid(id: &'static str) -> egui::Grid {
    egui::Grid::new(id)
        .num_columns(2)
        .striped(true)
        .spacing([18.0, 6.0])
        .min_col_width(96.0)
}

pub fn metadata_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text).weak());
}
