use eframe::egui;
use egui_extras::{Column, TableBuilder};
use udd_container::FileKey;
use crate::app::InspectorApp;
use crate::models::ViewMode;
use crate::utils::{data_type_to_str, format_size};

const TABLE_HEADER_HEIGHT: f32 = 20.0;

const PACKAGE_TABLE_KEY_MIN_WIDTH: f32 = 140.0;
const PACKAGE_TABLE_TYPE_MIN_WIDTH: f32 = 80.0;
const PACKAGE_TABLE_CODEC_MIN_WIDTH: f32 = 60.0;
const PACKAGE_TABLE_RAW_MIN_WIDTH: f32 = 80.0;

const VIRTUAL_TABLE_ID_MIN_WIDTH: f32 = 80.0;
const VIRTUAL_TABLE_KIND_MIN_WIDTH: f32 = 120.0;
const VIRTUAL_TABLE_LOCATION_MIN_WIDTH: f32 = 90.0;

impl InspectorApp {
    pub fn render_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        match self.view_mode {
            ViewMode::Package => self.render_package_table(ctx, ui),
            ViewMode::Virtual => self.render_virtual_table(ctx, ui),
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => {}
        }
    }

    fn render_package_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
        let filtered_indices = self.filtered_package_indices();

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(PACKAGE_TABLE_KEY_MIN_WIDTH)) // Key
            .column(Column::auto().at_least(PACKAGE_TABLE_TYPE_MIN_WIDTH)) // Type
            .column(Column::auto().at_least(PACKAGE_TABLE_CODEC_MIN_WIDTH)) // Codec
            .column(Column::auto().at_least(PACKAGE_TABLE_RAW_MIN_WIDTH)) // Raw
            .column(Column::remainder()) // Offset
            .header(TABLE_HEADER_HEIGHT, |mut header| {
                header.col(|ui| {
                    ui.strong("Key / Hash");
                });
                header.col(|ui| {
                    ui.strong("Type");
                });
                header.col(|ui| {
                    ui.strong("Codec");
                });
                header.col(|ui| {
                    ui.strong("Raw");
                });
                header.col(|ui| {
                    ui.strong("Offset");
                });
            })
            .body(|body| {
                body.rows(text_height, filtered_indices.len(), |mut row| {
                    let idx = filtered_indices[row.index()];
                    row.col(|ui| {
                        let text = match self.entries[idx].key {
                            FileKey::Id(id) => id.to_string(),
                            FileKey::PathHash(h) => format!("0x{:016X}", h),
                        };
                        let is_selected = self.selected_idx == Some(idx);
                        let resp = ui.selectable_label(is_selected, text);
                        if is_selected && self.scroll_to_selected {
                            resp.scroll_to_me(None);
                            self.scroll_to_selected = false;
                        }
                        if resp.clicked() {
                            self.select_entry(ctx, idx);
                        }
                    });
                    row.col(|ui| {
                        ui.label(data_type_to_str(self.entries[idx].data_type));
                    });
                    row.col(|ui| {
                        ui.label(format!("{:?}", self.entries[idx].codec));
                    });
                    row.col(|ui| {
                        ui.label(format_size(self.entries[idx].raw_size as u64));
                    });
                    row.col(|ui| {
                        ui.label(format!("0x{:08X}", self.entries[idx].offset));
                    });
                });
            });
    }

    fn render_virtual_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
        let filtered_indices = self.filtered_virtual_indices();

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(VIRTUAL_TABLE_ID_MIN_WIDTH)) // ID
            .column(Column::auto().at_least(VIRTUAL_TABLE_KIND_MIN_WIDTH)) // Kind
            .column(Column::remainder()) // Summary
            .column(Column::auto().at_least(VIRTUAL_TABLE_LOCATION_MIN_WIDTH)) // Location
            .header(TABLE_HEADER_HEIGHT, |mut header| {
                header.col(|ui| {
                    ui.strong("ID");
                });
                header.col(|ui| {
                    ui.strong("Kind");
                });
                header.col(|ui| {
                    ui.strong("Summary");
                });
                header.col(|ui| {
                    ui.strong("Location");
                });
            })
            .body(|body| {
                body.rows(text_height, filtered_indices.len(), |mut row| {
                    let idx = filtered_indices[row.index()];
                    let entry = &self.active_virtual_entries()[idx];
                    let id_str = entry.id.to_string();
                    let kind = entry.kind.clone();
                    let summary = entry.summary.clone();
                    let location = entry.location.clone();
                    row.col(|ui| {
                        let is_selected = self.selected_virtual_idx == Some(idx);
                        let resp = ui.selectable_label(is_selected, id_str);
                        if is_selected && self.scroll_to_selected {
                            resp.scroll_to_me(None);
                            self.scroll_to_selected = false;
                        }
                        if resp.clicked() {
                            self.select_virtual_entry(ctx, idx);
                        }
                    });
                    row.col(|ui| {
                        ui.label(kind);
                    });
                    row.col(|ui| {
                        ui.label(summary);
                    });
                    row.col(|ui| {
                        ui.label(location);
                    });
                });
            });
    }
}
