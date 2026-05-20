use crate::app::UopInspectorApp;
use eframe::egui;

pub fn ui_string_dictionary(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(dictionary) = app.uo_string_dictionary.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select an Enhanced Client path containing string_dictionary.uop.");
            });
        });
        return;
    };

    egui::TopBottomPanel::top("string_dictionary_tabs").show(ctx, |ui| {
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

    egui::CentralPanel::default().show(ctx, |ui| {
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
        ui.separator();

        let query = app.search_query.to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("string_dictionary_grid")
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Index");
                    ui.label("Offset");
                    ui.label("String");
                    ui.end_row();

                    for (index, value) in dictionary.iter() {
                        let offset = index + 1;
                        if !query.is_empty()
                            && !index.to_string().contains(&query)
                            && !offset.to_string().contains(&query)
                            && !value.to_lowercase().contains(&query)
                        {
                            continue;
                        }

                        ui.label(index.to_string());
                        ui.label(offset.to_string());
                        ui.monospace(value);
                        ui.end_row();
                    }
                });
        });
    });
}
