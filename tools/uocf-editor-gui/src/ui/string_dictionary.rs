use crate::app::{collect_string_dictionary_rows, UocfEditorApp, ViewMode};
use crate::logic::entry_text_editor::save_entry_copy;
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction, UOP_FILTER};
use crate::ui::unsaved_changes::TabEditor;
use crate::ui::{centered_row_offset, go_to_field, list_sort_controls, sorted_indices_by};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uocf::enhanced::entry_edit::EntryTarget;
use uocf::enhanced::string_dictionary::UoStringDictionary;

const PACKAGE_NAME: &str = "string_dictionary.uop";
const ROW_HEIGHT: f32 = 20.0;

#[derive(Default)]
pub struct StringDictionaryEditor {
    pub enabled: bool,
    /// The dictionary as it was before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<Arc<UoStringDictionary>>,
    selected: Option<usize>,
    /// The selected string being typed; stored when the field loses focus.
    text: String,
    new_string: String,
    error: Option<String>,
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::StringDictionary,
    label: "the string_dictionary.uop strings",
    is_dirty: |app| app.string_dictionary_editor.opened.is_some(),
    save,
    discard: revert,
};

/// The loaded package the dictionary came from.
fn source_package(app: &UocfEditorApp) -> Option<PathBuf> {
    let named = |path: &Path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(PACKAGE_NAME));
    app.uop_cache.loaded_uops.iter().map(|loaded| loaded.path.clone()).find(|path| named(path))
}

/// Writes a copy of the package at `source` to `output`, with the dictionary entry `hash` holding `dictionary`.
fn write_copy(dictionary: &UoStringDictionary, source: &Path, hash: u64, output: &Path) -> color_eyre::eyre::Result<()> {
    let payload = dictionary.to_payload_bytes()?;
    save_entry_copy(source, &EntryTarget::Hash(hash), &payload, output)?;
    Ok(())
}

fn save(app: &mut UocfEditorApp) {
    let (Some(dictionary), Some(hash), Some(source)) = (app.uo_string_dictionary.clone(), app.string_dictionary_raw_hash, source_package(app)) else {
        return;
    };
    let Some(output) = pick_output(PACKAGE_NAME, UOP_FILTER) else {
        return;
    };
    match write_copy(&dictionary, &source, hash, &output) {
        Ok(()) => {
            app.string_dictionary_editor.opened = None;
            app.string_dictionary_editor.error = None;
            report_saved(app, &output);
        }
        Err(error) => app.string_dictionary_editor.error = Some(format!("Save failed: {error:#}")),
    }
}

fn revert(app: &mut UocfEditorApp) {
    let Some(opened) = app.string_dictionary_editor.opened.take() else {
        return;
    };
    app.string_dictionary_rows = Some(Arc::new(collect_string_dictionary_rows(&opened)));
    app.uo_string_dictionary = Some(opened);
    app.string_dictionary_editor.selected = None;
}

/// Runs `edit` on the dictionary, keeping it as opened for the discard.
fn edit_dictionary(app: &mut UocfEditorApp, edit: impl FnOnce(&mut UoStringDictionary)) {
    let Some(dictionary) = app.uo_string_dictionary.as_mut() else {
        return;
    };
    app.string_dictionary_editor.opened.get_or_insert_with(|| Arc::clone(dictionary));
    edit(Arc::make_mut(dictionary));
}

/// Stores `text` as the string `index`, when it differs.
fn set_string(app: &mut UocfEditorApp, index: usize, text: String) {
    let current = app.uo_string_dictionary.as_ref().and_then(|dictionary| dictionary.get_string(index).map(str::to_owned));
    if current.is_none_or(|current| current == text) {
        return;
    }
    let search_text = format!("{} {} {}", index, index + 1, text.to_lowercase());
    if let Some(row) = app.string_dictionary_rows.as_mut().and_then(|rows| Arc::make_mut(rows).get_mut(index)) {
        row.value.clone_from(&text);
        row.search_text = search_text;
    }
    edit_dictionary(app, |dictionary| {
        dictionary.set_string(index, text);
    });
}

/// Appends `text`, and selects it.
fn add_string(app: &mut UocfEditorApp, text: String) {
    edit_dictionary(app, |dictionary| {
        dictionary.push_string(text);
    });
    if let Some(dictionary) = app.uo_string_dictionary.as_deref() {
        app.string_dictionary_rows = Some(Arc::new(collect_string_dictionary_rows(dictionary)));
        let last = dictionary.len().checked_sub(1);
        select_string(app, last);
    }
}

fn select_string(app: &mut UocfEditorApp, index: Option<usize>) {
    let text = index.and_then(|index| app.uo_string_dictionary.as_ref()?.get_string(index).map(str::to_owned));
    app.string_dictionary_editor.selected = index;
    app.string_dictionary_editor.text = text.unwrap_or_default();
}

/// The bar, and while editing the field of the selected string and the one adding a string.
fn editing_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let editor = &mut app.string_dictionary_editor;
    let dirty = editor.opened.is_some();
    let hover = "Change a string or add one, and save a copy of the package. Other files name a string by its index, so strings keep their places and none can be removed.";
    let action = ui.horizontal(|ui| editor_bar(ui, &mut editor.enabled, hover, dirty, "Save string_dictionary.uop...", &mut editor.error)).inner;
    match action {
        Some(BarAction::Revert) => revert(app),
        Some(BarAction::Save) => save(app),
        None => {}
    }
    if !app.string_dictionary_editor.enabled {
        return;
    }
    let selected = app.string_dictionary_editor.selected;
    let mut stored = None;
    let mut added = None;
    let editor = &mut app.string_dictionary_editor;
    ui.horizontal(|ui| match selected {
        Some(index) => {
            ui.label(format!("String {index}:"));
            let width = ui.available_width();
            let field = egui::TextEdit::singleline(&mut editor.text).desired_width(width);
            if ui.add(field).lost_focus() {
                stored = Some((index, editor.text.clone()));
            }
        }
        None => {
            ui.weak("Click an index in the list to change its string.");
        }
    });
    ui.horizontal(|ui| {
        ui.label("New string:");
        ui.text_edit_singleline(&mut editor.new_string);
        if ui.add_enabled(!editor.new_string.is_empty(), egui::Button::new("Add")).on_hover_text("Appends it after the last string.").clicked() {
            added = Some(std::mem::take(&mut editor.new_string));
        }
    });
    if let Some((index, text)) = stored {
        set_string(app, index, text);
    }
    if let Some(text) = added {
        add_string(app, text);
    }
}

pub fn ui_string_dictionary(app: &mut UocfEditorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let has_dictionary = app.uo_string_dictionary.is_some();
    if !has_dictionary {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Select an Enhanced Client path containing string_dictionary.uop.");
            });
        });
        return;
    }
    if app.string_dictionary_rows.is_none() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Reload assets to rebuild the string dictionary row cache.");
            });
        });
        return;
    }

    egui::Panel::top("string_dictionary_tabs").show(ui, |ui| {
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

    egui::CentralPanel::default().show(ui, |ui| {
        editing_controls(app, ui);
        ui.separator();
        // After the edits above: a copy held across them would make the next edit copy the whole table.
        let (Some(dictionary), Some(rows)) = (app.uo_string_dictionary.clone(), app.string_dictionary_rows.clone()) else {
            return;
        };
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
            ui.text_edit_singleline(&mut app.searches.string_dictionary);
        });
        let go_to = go_to_field(ui, "string_dictionary_rows");
        let sort = list_sort_controls(ui, "string_dictionary_rows", &["Index", "Offset", "String"], 0);
        ui.separator();

        let query = app.searches.string_dictionary.to_lowercase();
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
        let editing = app.string_dictionary_editor.enabled;
        let mut clicked = None;
        let mut area = egui::ScrollArea::vertical().id_salt("string_dictionary_rows");
        // Go to a string by its index: the row is selected only while editing, as a click would.
        if let Some(position) = go_to.position_in(matching_rows.iter().map(|&row_index| string_number(&rows[row_index].index))) {
            if editing {
                clicked = Some(matching_rows[position]);
            }
            area = area.vertical_scroll_offset(centered_row_offset(ui, position, ROW_HEIGHT));
        }
        area.show_rows(ui, ROW_HEIGHT, row_count, |ui, row_range| {
            egui::Grid::new("string_dictionary_visible_rows")
                .striped(true)
                .show(ui, |ui| {
                    for row_index in row_range {
                        let source_row_index = matching_rows[row_index];
                        let row = &rows[source_row_index];
                        if editing {
                            let selected = app.string_dictionary_editor.selected == Some(source_row_index);
                            if ui.selectable_label(selected, &row.index).clicked() {
                                clicked = Some(source_row_index);
                            }
                        } else {
                            ui.label(&row.index);
                        }
                        ui.label(&row.offset);
                        ui.monospace(&row.value);
                        ui.end_row();
                    }
                });
        });
        if clicked.is_some() {
            select_string(app, clicked);
        }
    });
}

fn string_number(value: &str) -> u64 {
    value.parse().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::{client_roots, output_path, EC_CLIENT_ROOTS_ENV};
    use uocf::uop_container::package::UopPackage;
    use uocf::utils::path::find_file_ignoring_case;

    const CHANGED: &str = "changed";
    const ADDED: &str = "added";

    fn payload() -> Vec<u8> {
        let mut payload = vec![0u8; 8];
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for text in ["alpha", "beta"] {
            payload.extend_from_slice(&(text.len() as u16).to_le_bytes());
            payload.extend_from_slice(text.as_bytes());
        }
        payload
    }

    fn app_with_dictionary() -> UocfEditorApp {
        let dictionary = UoStringDictionary::from_payload_bytes(&payload(), "test").unwrap();
        let mut app = test_app();
        app.string_dictionary_rows = Some(Arc::new(collect_string_dictionary_rows(&dictionary)));
        app.uo_string_dictionary = Some(Arc::new(dictionary));
        app
    }

    fn text_of(app: &UocfEditorApp, index: usize) -> &str {
        app.uo_string_dictionary.as_ref().unwrap().get_string(index).unwrap()
    }

    #[test]
    fn a_string_left_as_it_was_is_no_change() {
        let mut app = app_with_dictionary();
        set_string(&mut app, 0, "alpha".to_owned());
        assert!(!(TAB_EDITOR.is_dirty)(&app));
    }

    #[test]
    fn a_changed_and_an_added_string_show_and_discarding_takes_them_back() {
        let mut app = app_with_dictionary();
        set_string(&mut app, 0, CHANGED.to_owned());
        add_string(&mut app, ADDED.to_owned());
        assert!((TAB_EDITOR.is_dirty)(&app));
        assert_eq!(text_of(&app, 0), CHANGED);
        assert_eq!(text_of(&app, 2), ADDED);
        let rows = app.string_dictionary_rows.clone().unwrap();
        assert_eq!((rows.len(), rows[0].value.as_str(), rows[0].search_text.contains(CHANGED)), (3, CHANGED, true));
        assert_eq!(app.string_dictionary_editor.selected, Some(2));

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(app.uo_string_dictionary.as_ref().unwrap().to_payload_bytes().unwrap(), payload());
        assert_eq!(app.string_dictionary_rows.as_ref().unwrap().len(), 2);
    }

    #[test]
    #[ignore = "needs real Enhanced clients: set UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_dictionary_saves_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let Some(source) = find_file_ignoring_case(&root, PACKAGE_NAME) else {
                println!("{}: no package", root.display());
                continue;
            };
            let package = UopPackage::load(&source).unwrap();
            let hash = package.get_file_by_hash(0).map_or_else(|| package.iter_files().find(|file| file.has_size()).unwrap().filename_hash(), |file| file.filename_hash());
            let mut dictionary = UoStringDictionary::load(&source).unwrap();
            let output = output_path(PACKAGE_NAME);

            write_copy(&dictionary, &source, hash, &output).unwrap();
            assert!(std::fs::read(&output).unwrap() == std::fs::read(&source).unwrap(), "{}: a save without changes differs", root.display());

            let count = dictionary.len();
            assert!(dictionary.set_string(0, CHANGED.to_owned()));
            let added = dictionary.push_string(ADDED.to_owned());
            write_copy(&dictionary, &source, hash, &output).unwrap();
            let reopened = UoStringDictionary::load(&output).unwrap();
            assert_eq!(reopened.get_string(0), Some(CHANGED));
            assert_eq!(reopened.get_string(added), Some(ADDED));
            assert_eq!(reopened.len(), count + 1);
            std::fs::remove_file(&output).unwrap();
            println!("{}: same bytes; the edits of {count} strings come back", root.display());
        }
    }
}
