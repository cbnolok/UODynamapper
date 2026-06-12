use crate::app::UopInspectorApp;
use crate::ui::{list_sort_controls, sorted_indices_by};
use eframe::egui;

pub fn ui_string_dictionary(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(dictionary) = app.uo_string_dictionary.clone() else {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select an Enhanced Client path containing string_dictionary.uop.");
            });
        });
        return;
    };
    let Some(rows) = app.string_dictionary_rows.clone() else {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Reload assets to rebuild the string dictionary row cache.");
            });
        });
        return;
    };

    egui::Panel::top("string_dictionary_tabs").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut app.view_mode,
                crate::app::ViewMode::StringDictionary,
                "Specialized",
            );
            if ui.button("Raw UOP").clicked() {
                if let Some(hash) = app.string_dictionary_raw_hash {
                    app.select_raw_uop_entry("string_dictionary.uop", hash);
                }
            }
        });
    });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("EC String Dictionary");
            ui.separator();
            ui.label(format!("{} strings", dictionary.len()));
        });

        egui::Grid::new("string_dictionary_header").striped(true).show(ui, |ui| {
            ui.label("unk1");
            ui.label(format!("0x{:016X}", dictionary.unk1()));
            ui.end_row();
            ui.label("unk2");
            ui.label(format!("0x{:08X}", dictionary.unk2()));
            ui.end_row();
        });

        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut app.search_query);
        });
        let sort = list_sort_controls(ui, "string_dictionary_rows", &["Index", "Offset", "String"], 0);
        ui.separator();

        let query = app.search_query.to_lowercase();
        let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| {
            match sort.option_index {
                1 => string_number(&left.offset).cmp(&string_number(&right.offset)),
                2 => left.value.to_ascii_lowercase().cmp(&right.value.to_ascii_lowercase()),
                _ => string_number(&left.index).cmp(&string_number(&right.index)),
            }
        });
        let matching_rows: Vec<usize> = sorted_indices
            .iter()
            .copied()
            .filter(|row_index| {
                query.is_empty() || rows[*row_index].search_text.contains(&query)
            })
            .collect();
        let row_count = matching_rows.len();

        egui::Grid::new("string_dictionary_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.label("Index");
                ui.label("Offset");
                ui.label("String");
                ui.end_row();
            });
        egui::ScrollArea::vertical()
            .id_salt("string_dictionary_rows")
            .show_rows(ui, 20.0, row_count, |ui, row_range| {
                egui::Grid::new("string_dictionary_visible_rows")
                    .striped(true)
                    .show(ui, |ui| {
                        for row_index in row_range {
                            let source_row_index = matching_rows[row_index];
                            let row = &rows[source_row_index];
                            ui.label(&row.index);
                            ui.label(&row.offset);
                            ui.monospace(&row.value);
                            ui.end_row();
                        }
                    });
            });
    });
}

fn string_number(value: &str) -> u64 {
    value.parse().unwrap_or(u64::MAX)
}
