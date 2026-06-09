use crate::app::{LocalizedStringDisplayRow, LocalizedStringsSource, UopInspectorApp};
use crate::ui::{arrow_delta, list_sort_controls, move_selection, sorted_indices_by};
use eframe::egui;

const CLASSIC_CLILOC_LIST_PANEL_WIDTH: f32 = 320.0;
const LOCALIZED_STRINGS_FILES_PANEL_WIDTH: f32 = 340.0;

pub fn ui_clilocs(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let has_cliloc = app.cliloc.is_some();
    let has_localized = app.localized_strings.is_some();
    let cliloc_tab_label = app
        .selected_cliloc_file_idx
        .and_then(|index| app.cliloc_files.get(index))
        .map(|entry| entry.label.as_str())
        .unwrap_or("Cliloc");

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
                    cliloc_tab_label,
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
        .default_width(CLASSIC_CLILOC_LIST_PANEL_WIDTH)
        .show(ctx, |ui| {
            let heading = app
                .selected_cliloc_file_idx
                .and_then(|index| app.cliloc_files.get(index))
                .map(|entry| entry.label.as_str())
                .unwrap_or("Cliloc");
            ui.heading(format!("{heading} ({})", entries.len()));
            cliloc_translation_selector(app, ui);
            ui.separator();
            let search_has_focus = search_box(app, ui);
            ui.separator();
            cliloc_list(app, ui, entries, search_has_focus);
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

fn cliloc_translation_selector(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    if app.cliloc_files.len() <= 1 {
        if let Some(index) = app.selected_cliloc_file_idx {
            if let Some(entry) = app.cliloc_files.get(index) {
                ui.small(entry.path.display().to_string());
            }
        }
        return;
    }

    let selected_label = app
        .selected_cliloc_file_idx
        .and_then(|index| app.cliloc_files.get(index))
        .map(|entry| entry.label.clone())
        .unwrap_or_else(|| "Select cliloc".to_string());
    let choices = app
        .cliloc_files
        .iter()
        .enumerate()
        .map(|(index, entry)| (index, entry.label.clone(), entry.path.display().to_string()))
        .collect::<Vec<_>>();
    let mut selected_index = app.selected_cliloc_file_idx;

    egui::ComboBox::from_label("Translation")
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            for (index, label, path) in &choices {
                let response = ui.selectable_value(&mut selected_index, Some(*index), label);
                response.on_hover_text(path);
            }
        });

    if selected_index != app.selected_cliloc_file_idx {
        if let Some(index) = selected_index {
            app.select_cliloc_file(index);
        }
    }
}

fn ui_localized_strings(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(package) = app.localized_strings.clone() else {
        return;
    };
    let Some(row_files) = app.localized_string_rows.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Reload assets to rebuild the localized strings row cache.");
            });
        });
        return;
    };
    if app.selected_localized_file_hash.is_none() {
        app.selected_localized_file_hash = package.files.first().map(|file| file.filename_hash);
    }

    egui::SidePanel::left("localized_strings_files")
        .resizable(true)
        .default_width(LOCALIZED_STRINGS_FILES_PANEL_WIDTH)
        .show(ctx, |ui| {
            ui.heading(format!("localizedstrings.uop ({})", package.files.len()));
            ui.separator();
            let sort = list_sort_controls(ui, "localized_strings_files", &["Hash", "Strings"], 0);
            let file_indices = sorted_indices_by(&package.files, sort.ordering(), |left, right| {
                match sort.option_index {
                    1 => left.strings.len().cmp(&right.strings.len()),
                    _ => left.filename_hash.cmp(&right.filename_hash),
                }
            });
            for index in file_indices {
                let file = &package.files[index];
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
        let Some(row_file) = row_files.iter().find(|file| file.filename_hash == file_hash) else {
            ui.label("Selected localized strings row cache is missing.");
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
        localized_strings_table(app, ui, &file.strings.entries, &row_file.rows);
    });
}

fn search_box(app: &mut UopInspectorApp, ui: &mut egui::Ui) -> bool {
    let mut has_focus = false;
    ui.horizontal(|ui| {
        ui.label("Search:");
        has_focus = ui.text_edit_singleline(&mut app.search_query).has_focus();
    });
    has_focus
}

fn cliloc_list(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    entries: &[uocf::classic::cliloc::ClilocEntry],
    search_has_focus: bool,
) {
    let query = app.search_query.trim();
    let row_height = ui.spacing().interact_size.y;
    let sort = list_sort_controls(ui, "classic_cliloc_entries", &["Number", "Text"], 0);
    let sorted_indices = sorted_indices_by(entries, sort.ordering(), |left, right| {
        match sort.option_index {
            1 => left.text.to_ascii_lowercase().cmp(&right.text.to_ascii_lowercase()),
            _ => left.number.cmp(&right.number),
        }
    });
    let visible_indices = visible_cliloc_indices(entries, &sorted_indices, query);
    let visible_numbers = visible_indices
        .iter()
        .map(|index| entries[*index].number)
        .collect::<Vec<_>>();
    let keyboard_moved = if let Some(delta) = arrow_delta(ui, search_has_focus) {
        if let Some(number) =
            move_selection(&visible_numbers, Some(app.selected_cliloc_number), delta)
        {
            app.selected_cliloc_number = number;
        }
        true
    } else {
        false
    };

    egui::ScrollArea::vertical().show_rows(ui, row_height, visible_indices.len(), |ui, row_range| {
        for row in row_range {
            let entry = &entries[visible_indices[row]];
                let label = format!("{}: {}", entry.number, entry.text);
                let selected = app.selected_cliloc_number == entry.number;
                let response = ui.selectable_label(selected, label);
                if keyboard_moved && selected {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
                if response.clicked() {
                    app.selected_cliloc_number = entry.number;
                }
            }
        });
}

fn visible_cliloc_indices(
    entries: &[uocf::classic::cliloc::ClilocEntry],
    sorted_indices: &[usize],
    query: &str,
) -> Vec<usize> {
    if query.is_empty() {
        return sorted_indices.to_vec();
    }

    let query = query.to_lowercase();
    sorted_indices
        .iter()
        .copied()
        .filter(|index| cliloc_entry_matches_query(&entries[*index], &query))
        .collect()
}

fn cliloc_entry_matches_query(entry: &uocf::classic::cliloc::ClilocEntry, query: &str) -> bool {
    entry.number.to_string().contains(query) || entry.text.to_lowercase().contains(query)
}

fn localized_strings_table(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    entries: &[uocf::enhanced::localized_strings::LocalizedStringEntry],
    rows: &[LocalizedStringDisplayRow],
) {
    let query = app.search_query.to_lowercase();
    let sort = list_sort_controls(ui, "localized_strings_rows", &["ID", "String", "unk"], 0);
    let sorted_indices = sorted_indices_by(rows, sort.ordering(), |left, right| {
        match sort.option_index {
            1 => localized_text(entries, left)
                .to_ascii_lowercase()
                .cmp(&localized_text(entries, right).to_ascii_lowercase()),
            2 => left.unk.cmp(&right.unk),
            _ => string_number(&left.id).cmp(&string_number(&right.id)),
        }
    });
    let matching_rows: Vec<usize> = sorted_indices
        .iter()
        .copied()
        .filter(|row_index| query.is_empty() || rows[*row_index].search_text.contains(&query))
        .collect();
    let row_count = matching_rows.len();

    egui::Grid::new("localized_strings_header").striped(true).show(ui, |ui| {
        ui.label("ID");
        ui.label("unk");
        ui.label("String value");
        ui.end_row();
    });
    egui::ScrollArea::vertical()
        .id_salt("localized_strings_rows")
        .show_rows(ui, 20.0, row_count, |ui, row_range| {
            egui::Grid::new("localized_strings_visible_rows")
                .striped(true)
                    .show(ui, |ui| {
                        for row_index in row_range {
                        let source_row_index = matching_rows[row_index];
                        let row = &rows[source_row_index];
                        let text = localized_text(entries, row);
                        ui.label(&row.id);
                        ui.label(&row.unk);
                        ui.label(text);
                        ui.end_row();
                    }
                });
        });
}

fn localized_text<'a>(
    entries: &'a [uocf::enhanced::localized_strings::LocalizedStringEntry],
    row: &LocalizedStringDisplayRow,
) -> &'a str {
    entries
        .get(row.entry_index)
        .map(|entry| entry.text.as_str())
        .unwrap_or("")
}

fn string_number(value: &str) -> i32 {
    value.parse().unwrap_or(i32::MAX)
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

    #[test]
    fn visible_cliloc_indices_respects_search_query() {
        let entries = vec![
            uocf::classic::cliloc::ClilocEntry {
                number: 100,
                flag: 0,
                text: "Vendor".to_string(),
            },
            uocf::classic::cliloc::ClilocEntry {
                number: 200,
                flag: 0,
                text: "Bank Balance".to_string(),
            },
            uocf::classic::cliloc::ClilocEntry {
                number: 201,
                flag: 0,
                text: "Stable".to_string(),
            },
        ];
        let sorted_indices = vec![0, 1, 2];

        assert_eq!(visible_cliloc_indices(&entries, &sorted_indices, ""), vec![0, 1, 2]);
        assert_eq!(visible_cliloc_indices(&entries, &sorted_indices, "bank"), vec![1]);
        assert_eq!(visible_cliloc_indices(&entries, &sorted_indices, "20"), vec![1, 2]);
    }
}
