pub mod details;
pub mod mobile_anim;
pub mod table;

use eframe::egui;

const METADATA_GRID_COL_SPACING: f32 = 18.0;
const METADATA_GRID_ROW_SPACING: f32 = 6.0;
const METADATA_GRID_MIN_COL_WIDTH: f32 = 96.0;

pub fn metadata_grid(id: &'static str) -> egui::Grid {
    egui::Grid::new(id)
        .num_columns(2)
        .striped(true)
        .spacing([METADATA_GRID_COL_SPACING, METADATA_GRID_ROW_SPACING])
        .min_col_width(METADATA_GRID_MIN_COL_WIDTH)
}

pub fn metadata_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text).weak());
}
