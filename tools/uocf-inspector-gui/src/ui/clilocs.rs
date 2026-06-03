use crate::app::{LocalizedStringsSource, UopInspectorApp};
use eframe::egui;
use uocf::enhanced::localized_strings::LocalizedStringEntry;

pub fn ui_clilocs(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let has_cliloc = app.cliloc.is_some();
    let has_localized = app.localized_strings.is_some();

    if app.localized_strings_source == LocalizedStringsSource::Cliloc && !has_cliloc && has_localized {
        app.localized_strings_source = LocalizedStringsSource::LocalizedStringsUop;
    } else if app.localized_strings_source == LocalizedStringsSource::LocalizedStringsUop
        && !has_localized
        && has_cliloc
    {
        app.localized_strings_source = LocalizedStringsSource::Cliloc;
    }

    egui::TopBottomPanel::top("cliloc_source_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if has_cliloc {
                ui.selectable_value(
                    &mut app.localized_strings_source,
                    LocalizedStringsSource::Cliloc,
                    "Cliloc.enu",
                );
            }
            if has_localized {
                ui.selectable_value(
                    &mut app.localized_strings_source,
                    LocalizedStringsSource::LocalizedStringsUop,
                    "localizedstrings.uop",
                );
            }
        });
    });

    match app.localized_strings_source {
        LocalizedStringsSource::Cliloc => ui_classic_cliloc(app, ctx),
        LocalizedStringsSource::LocalizedStringsUop => ui_localized_strings(app, ctx),
    }
}

fn ui_classic_cliloc(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(cliloc) = app.cliloc.clone() else {
        return;
    };
    let entries = &cliloc.entries;

    egui::SidePanel::left("classic_cliloc_list")
        .resizable(true)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.heading(format!("Cliloc.enu ({})", entries.len()));
            ui.separator();
            search_box(app, ui);
            ui.separator();
            cliloc_list(app, ui, entries);
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading(format!("CliLoc {}", app.selected_cliloc_number));
        ui.separator();
        if let Some(entry) = entries.iter().find(|entry| entry.number == app.selected_cliloc_number) {
            ui.label(format!("Flag: 0x{:02X}", entry.flag));
            ui.separator();
            ui.label(&entry.text);
        } else {
            ui.label("Select a cliloc entry from the left panel.");
        }
    });
}

fn ui_localized_strings(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(package) = app.localized_strings.clone() else {
        return;
    };
    if app.selected_localized_file_hash.is_none() {
        app.selected_localized_file_hash = package.files.first().map(|file| file.filename_hash);
    }

    egui::SidePanel::left("localized_strings_files")
        .resizable(true)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.heading(format!("localizedstrings.uop ({})", package.files.len()));
            ui.separator();
            for file in &package.files {
                let label = format!(
                    "{:016X} ({} strings)",
                    file.filename_hash,
                    file.strings.len()
                );
                if ui
                    .selectable_label(app.selected_localized_file_hash == Some(file.filename_hash), label)
                    .clicked()
                {
                    app.selected_localized_file_hash = Some(file.filename_hash);
                }
            }
        });

    egui::TopBottomPanel::top("localized_strings_raw_tabs").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut app.localized_strings_source,
                LocalizedStringsSource::LocalizedStringsUop,
                "Specialized",
            );
            if ui.button("Raw selected").clicked() {
                if let Some(hash) = app.selected_localized_file_hash {
                    app.select_raw_localized_strings_file(hash);
                }
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        let Some(file_hash) = app.selected_localized_file_hash else {
            ui.label("Select a localized strings file.");
            return;
        };
        let Some(file) = package.files.iter().find(|file| file.filename_hash == file_hash) else {
            ui.label("Selected localized strings file is missing.");
            return;
        };

        ui.horizontal(|ui| {
            ui.heading(format!("0x{:016X}", file.filename_hash));
            ui.separator();
            ui.label(format!("{} bytes", file.byte_len));
            ui.label(format!("{} strings", file.strings.len()));
        });
        ui.separator();

        search_box(app, ui);
        ui.separator();
        localized_strings_table(app, ui, &file.strings.entries);
    });
}

fn search_box(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("Search:");
        ui.text_edit_singleline(&mut app.search_query);
    });
}

fn cliloc_list(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    entries: &[uocf::classic::cliloc::ClilocEntry],
) {
    let query = app.search_query.trim();
    let row_height = ui.spacing().interact_size.y;

    if query.is_empty() {
        egui::ScrollArea::vertical().show_rows(ui, row_height, entries.len(), |ui, row_range| {
            for row in row_range {
                let entry = &entries[row];
                let label = format!("{}: {}", entry.number, entry.text);
                if ui
                    .selectable_label(app.selected_cliloc_number == entry.number, label)
                    .clicked()
                {
                    app.selected_cliloc_number = entry.number;
                }
            }
        });
        return;
    }

    let query = query.to_lowercase();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for entry in entries {
            if !cliloc_entry_matches_query(entry, &query) {
                continue;
            }
            let label = format!("{}: {}", entry.number, entry.text);
            if ui
                .selectable_label(app.selected_cliloc_number == entry.number, label)
                .clicked()
            {
                app.selected_cliloc_number = entry.number;
            }
        }
    });
}

fn cliloc_entry_matches_query(entry: &uocf::classic::cliloc::ClilocEntry, query: &str) -> bool {
    entry.number.to_string().contains(query) || entry.text.to_lowercase().contains(query)
}

fn localized_strings_table(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    entries: &[LocalizedStringEntry],
) {
    let query = app.search_query.to_lowercase();
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("localized_strings_grid").striped(true).show(ui, |ui| {
            ui.label("ID");
            ui.label("unk");
            ui.label("String value");
            ui.end_row();
            for entry in entries {
                if !query.is_empty()
                    && !entry.id.to_string().contains(&query)
                    && !entry.text.to_lowercase().contains(&query)
                {
                    continue;
                }
                ui.label(entry.id.to_string());
                ui.label(format!("0x{:02X}", entry.unk));
                ui.label(&entry.text);
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cliloc_entry_query_matches_number_or_text_case_insensitively() {
        let entry = uocf::classic::cliloc::ClilocEntry {
            number: 3001234,
            flag: 0,
            text: "Bank Balance".to_string(),
        };

        assert!(cliloc_entry_matches_query(&entry, "1234"));
        assert!(cliloc_entry_matches_query(&entry, "balance"));
        assert!(cliloc_entry_matches_query(&entry, "bank"));
        assert!(!cliloc_entry_matches_query(&entry, "vendor"));
    }
}
