use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::app::{LocalizedStringDisplayRow, LocalizedStringsSource, UocfEditorApp, ViewMode};
use crate::ui::unsaved_changes::TabEditor;
use crate::ui::{
    arrow_delta, centered_row_offset, enhanced_files_availability, folder_files_availability,
    go_to_field, list_sort_controls, move_selection, sorted_indices_by, source_choice, ClientSource,
    SourceAvailability, CLASSIC_FOLDER, UOP_PATCHER_NOTE,
};
use eframe::egui;
use uocf::enhanced::localized_strings::{language_of_hash, write_tables, LocalizedStringTable};

const CLASSIC_CLILOC_LIST_PANEL_WIDTH: f32 = 320.0;
/// The Classic string files, when none is loaded.
const CLILOC_FILES: &str = "Cliloc.*";
const LOCALIZED_STRINGS_FILE: &str = "LocalizedStrings.uop";
const LOCALIZED_STRINGS_FILES_PANEL_WIDTH: f32 = 340.0;
const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const KDL_FILTER: (&str, &[&str]) = ("KDL text", &["kdl"]);
/// Rows of the string editor's text box.
const EDITOR_TEXT_ROWS: usize = 4;
/// Height of a row of the LocalizedStrings table.
const LOCALIZED_STRINGS_ROW_HEIGHT: f32 = 20.0;

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Clilocs,
    label: "the LocalizedStrings.uop tables",
    is_dirty: |app| app.localized_strings_editor.edits != app.localized_strings_editor.saved_edits,
    save: |app| save_with_dialog(&mut app.localized_strings_editor),
    discard: |app| {
        let editor = &mut app.localized_strings_editor;
        editor.edits.clear();
        editor.saved_edits.clear();
    },
};

/// Edits of the opened LocalizedStrings.uop: edited tables are kept here, and saved as a copy of the package.
#[derive(Default)]
pub struct LocalizedStringsEditor {
    /// The package the tables come from; edits are saved as a copy of it.
    pub source: Option<PathBuf>,
    /// Edited tables, by entry hash.
    edits: BTreeMap<u64, LocalizedStringTable>,
    /// The edited tables at the last save.
    saved_edits: BTreeMap<u64, LocalizedStringTable>,
    /// The id typed in the editor, and its draft text.
    id_text: String,
    draft: String,
    status: Option<String>,
}

impl LocalizedStringsEditor {
    /// An editor for the package at `source`, with no edits.
    pub fn for_package(source: PathBuf) -> Self {
        Self { source: Some(source), ..Self::default() }
    }

    fn table<'a>(&'a self, hash: u64, original: &'a LocalizedStringTable) -> &'a LocalizedStringTable {
        self.edits.get(&hash).unwrap_or(original)
    }

    fn select(&mut self, hash: u64, original: &LocalizedStringTable, id: u32) {
        self.id_text = id.to_string();
        self.draft = self.table(hash, original).get(id).unwrap_or_default().to_string();
    }
}

pub fn ui_clilocs(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let has_cliloc = app.cliloc.is_some();
    let has_localized = app.localized_strings.is_some();
    let cliloc_files = app
        .selected_cliloc_file_idx
        .and_then(|index| app.cliloc_files.get(index))
        .map_or_else(|| CLILOC_FILES.to_string(), |entry| entry.label.clone());

    if app.localized_strings_source == LocalizedStringsSource::Cliloc && !has_cliloc && has_localized {
        app.localized_strings_source = LocalizedStringsSource::LocalizedStringsUop;
    } else if app.localized_strings_source == LocalizedStringsSource::LocalizedStringsUop
        && !has_localized
        && has_cliloc
    {
        app.localized_strings_source = LocalizedStringsSource::Cliloc;
    }

    // The Classic Cliloc.* files are not UOP packages: MUL. The Classic client has no strings in a UOP package: no CC
    // UOP. LocalizedStrings.uop is KR UOP or EC UOP by the folder it is read from.
    egui::Panel::top("cliloc_source_tabs").show(ui, |ui| {
        let selected = match app.localized_strings_source {
            LocalizedStringsSource::Cliloc => Some(ClientSource::Mul),
            LocalizedStringsSource::LocalizedStringsUop => app.shown_enhanced,
        };
        let clicked = source_choice(ui, selected, |source| match source {
            ClientSource::Mul => folder_files_availability(app.settings.cc_path.as_deref(), CLASSIC_FOLDER, &cliloc_files, has_cliloc),
            ClientSource::CcUop => SourceAvailability::NotApplicable,
            ClientSource::KrUop | ClientSource::EcUop => {
                enhanced_files_availability(app, source, LOCALIZED_STRINGS_FILE, &[LOCALIZED_STRINGS_FILE], has_localized)
            }
        });
        match clicked {
            Some(ClientSource::Mul) => app.localized_strings_source = LocalizedStringsSource::Cliloc,
            Some(source) => {
                app.localized_strings_source = LocalizedStringsSource::LocalizedStringsUop;
                app.show_enhanced_client(source);
            }
            None => {}
        }
    });

    match app.localized_strings_source {
        LocalizedStringsSource::Cliloc => ui_classic_cliloc(app, ctx, ui),
        LocalizedStringsSource::LocalizedStringsUop => ui_localized_strings(app, ctx, ui),
    }
}

fn ui_classic_cliloc(app: &mut UocfEditorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(cliloc) = app.cliloc.clone() else {
        return;
    };
    let entries = &cliloc.entries;

    egui::Panel::left("classic_cliloc_list")
        .resizable(true)
        .default_size(CLASSIC_CLILOC_LIST_PANEL_WIDTH)
        .show(ui, |ui| {
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

    egui::CentralPanel::default().show(ui, |ui| {
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

fn cliloc_translation_selector(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
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

fn ui_localized_strings(app: &mut UocfEditorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(package) = app.localized_strings.clone() else {
        return;
    };
    let Some(row_files) = app.localized_string_rows.clone() else {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Reload assets to rebuild the localized strings row cache.");
            });
        });
        return;
    };
    if app.selected_localized_file_hash.is_none() {
        app.selected_localized_file_hash = package.files.first().map(|file| file.filename_hash);
    }

    egui::Panel::left("localized_strings_files")
        .resizable(true)
        .default_size(LOCALIZED_STRINGS_FILES_PANEL_WIDTH)
        .show(ui, |ui| {
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
                let edited = if app.localized_strings_editor.edits.contains_key(&file.filename_hash) { " *" } else { "" };
                let language = language_of_hash(file.filename_hash).map_or("", |language| language.code);
                let label = format!(
                    "{:016X} {language} ({} strings){edited}",
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

    egui::Panel::top("localized_strings_raw_tabs").show(ui, |ui| {
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

    egui::CentralPanel::default().show(ui, |ui| {
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

        localized_strings_editor(app, ui, file_hash, &file.strings);
        ui.separator();
        search_box(app, ui);
        ui.separator();
        if let Some(id) = localized_strings_table(app, ui, &file.strings.entries, &row_file.rows) {
            app.localized_strings_editor.select(file_hash, &file.strings, id);
        }
    });
}

fn search_box(app: &mut UocfEditorApp, ui: &mut egui::Ui) -> bool {
    let mut has_focus = false;
    ui.horizontal(|ui| {
        ui.label("Search:");
        has_focus = ui.text_edit_singleline(&mut app.searches.clilocs).has_focus();
    });
    has_focus
}

fn cliloc_list(
    app: &mut UocfEditorApp,
    ui: &mut egui::Ui,
    entries: &[uocf::classic::cliloc::ClilocEntry],
    search_has_focus: bool,
) {
    let go_to = go_to_field(ui, "classic_cliloc_entries");
    let query = app.searches.clilocs.trim();
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
    let keyboard_moved = if let Some(delta) = arrow_delta(ui, search_has_focus || go_to.has_focus) {
        if let Some(number) =
            move_selection(&visible_numbers, Some(app.selected_cliloc_number), delta)
        {
            app.selected_cliloc_number = number;
        }
        true
    } else {
        false
    };

    let mut area = egui::ScrollArea::vertical();
    if let Some(position) = go_to.position_in(visible_numbers.iter().map(|&number| u64::try_from(number).unwrap_or(u64::MAX))) {
        app.selected_cliloc_number = visible_numbers[position];
        area = area.vertical_scroll_offset(centered_row_offset(ui, position, row_height));
    }
    area.show_rows(ui, row_height, visible_indices.len(), |ui, row_range| {
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
    app: &mut UocfEditorApp,
    ui: &mut egui::Ui,
    entries: &[uocf::enhanced::localized_strings::LocalizedStringEntry],
    rows: &[LocalizedStringDisplayRow],
) -> Option<u32> {
    let mut clicked = None;
    let go_to = go_to_field(ui, "localized_strings_rows");
    let query = app.searches.clilocs.to_lowercase();
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
    let mut area = egui::ScrollArea::vertical().id_salt("localized_strings_rows");
    let row_ids = matching_rows.iter().map(|&row_index| u64::try_from(string_number(&rows[row_index].id)).unwrap_or(u64::MAX));
    if let Some(position) = go_to.position_in(row_ids) {
        clicked = entries.get(rows[matching_rows[position]].entry_index).map(|entry| entry.id);
        area = area.vertical_scroll_offset(centered_row_offset(ui, position, LOCALIZED_STRINGS_ROW_HEIGHT));
    }
    area.show_rows(ui, LOCALIZED_STRINGS_ROW_HEIGHT, row_count, |ui, row_range| {
        egui::Grid::new("localized_strings_visible_rows")
            .striped(true)
                .show(ui, |ui| {
                    for row_index in row_range {
                    let source_row_index = matching_rows[row_index];
                    let row = &rows[source_row_index];
                    let text = localized_text(entries, row);
                    if ui.selectable_label(app.localized_strings_editor.id_text == row.id, &row.id).clicked() {
                        clicked = entries.get(row.entry_index).map(|entry| entry.id);
                    }
                    ui.label(&row.unk);
                    ui.label(text);
                    ui.end_row();
                }
            });
    });
    clicked
}

/// The string editor of the selected table: set or remove a string, save the edited tables as a copy of the
/// package, export or import the table as KDL. The table below shows the package as loaded.
fn localized_strings_editor(app: &mut UocfEditorApp, ui: &mut egui::Ui, hash: u64, original: &LocalizedStringTable) {
    let editor = &mut app.localized_strings_editor;
    ui.horizontal(|ui| {
        ui.label("String id:");
        let response = ui.add(egui::TextEdit::singleline(&mut editor.id_text).desired_width(90.0));
        if response.changed() {
            if let Ok(id) = editor.id_text.trim().parse::<u32>() {
                editor.draft = editor.table(hash, original).get(id).unwrap_or_default().to_string();
            }
        }
        let id = editor.id_text.trim().parse::<u32>().ok();
        let exists = id.is_some_and(|id| editor.table(hash, original).get(id).is_some());
        if ui.add_enabled(id.is_some(), egui::Button::new(if exists { "Set" } else { "Add" })).clicked() {
            let id = id.expect("enabled with an id");
            let mut table = editor.table(hash, original).clone();
            table.set(id, editor.draft.clone());
            editor.edits.insert(hash, table);
            editor.status = Some(format!("String {id} set (not saved)."));
        }
        if ui.add_enabled(exists, egui::Button::new("Remove")).clicked() {
            let id = id.expect("enabled with an id");
            let mut table = editor.table(hash, original).clone();
            table.remove(id);
            editor.edits.insert(hash, table);
            editor.status = Some(format!("String {id} removed (not saved)."));
        }
        if ui.add_enabled(editor.edits.contains_key(&hash), egui::Button::new("Revert table")).clicked() {
            editor.edits.remove(&hash);
            editor.status = Some("Table edits reverted.".to_string());
        }
    });
    ui.add(egui::TextEdit::multiline(&mut editor.draft).desired_rows(EDITOR_TEXT_ROWS).desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        let can_save = editor.source.is_some() && !editor.edits.is_empty();
        if ui
            .add_enabled(can_save, egui::Button::new("Save package..."))
            .on_hover_text(format!(
                "Writes a copy of the opened LocalizedStrings.uop with the edited tables replaced; the other entries keep \
                 their bytes. The opened package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
            ))
            .clicked()
        {
            save_with_dialog(editor);
        }
        let stem = language_of_hash(hash).map_or_else(|| format!("{hash:016X}"), |language| language.code.to_string());
        if ui.button("Export table KDL...").clicked() {
            if let Some(path) = crate::dialog::file_dialog().add_filter(KDL_FILTER.0, KDL_FILTER.1).set_file_name(format!("{stem}.kdl")).save_file() {
                let table = editor.table(hash, original);
                editor.status = Some(match std::fs::write(&path, table.to_kdl_string()) {
                    Ok(()) => format!("Wrote {} ({} strings).", path.display(), table.len()),
                    Err(error) => format!("Failed to write {}: {error}", path.display()),
                });
            }
        }
        if ui.button("Import table KDL...").on_hover_text("Replaces the whole selected table (not saved).").clicked() {
            if let Some(path) = crate::dialog::file_dialog().add_filter(KDL_FILTER.0, KDL_FILTER.1).pick_file() {
                editor.status = Some(match import_kdl(&path) {
                    Ok(table) => {
                        let message = format!("Read {} ({} strings; not saved).", path.display(), table.len());
                        editor.edits.insert(hash, table);
                        message
                    }
                    Err(error) => format!("{error:#}"),
                });
            }
        }
        if !editor.edits.is_empty() {
            ui.label(format!("{} edited tables", editor.edits.len()));
        }
    });
    if let Some(status) = &editor.status {
        ui.small(status);
    }
}

fn import_kdl(path: &Path) -> color_eyre::eyre::Result<LocalizedStringTable> {
    let text = std::fs::read_to_string(path)?;
    LocalizedStringTable::from_kdl_str(&path.display().to_string(), &text)
}

fn save_with_dialog(editor: &mut LocalizedStringsEditor) {
    let output = crate::dialog::file_dialog()
        .add_filter(UOP_FILTER.0, UOP_FILTER.1)
        .set_file_name("LocalizedStrings.uop")
        .save_file();
    if let Some(path) = output {
        let status = save(editor, &path);
        editor.status = Some(status);
    }
}

fn save(editor: &mut LocalizedStringsEditor, output: &Path) -> String {
    let Some(source) = &editor.source else {
        return "No LocalizedStrings.uop is open.".to_string();
    };
    let tables: Vec<(u64, &LocalizedStringTable)> = editor.edits.iter().map(|(hash, table)| (*hash, table)).collect();
    match write_tables(source, output, &tables) {
        Ok(write) => {
            editor.saved_edits = editor.edits.clone();
            format!(
                "Wrote {}: {} tables replaced, {} added, {} unchanged.",
                output.display(),
                write.replaced,
                write.added,
                write.unchanged
            )
        }
        Err(error) => format!("Save failed: {error:#}"),
    }
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
