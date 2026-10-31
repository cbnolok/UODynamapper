//! The LegacyTerrain.uop view of the UOP browser: the definition and XML of the selected entry, and the editor of
//! every definition of the package (`logic/legacy_terrain_editor.rs`).

use std::path::Path;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use uocf::enhanced::legacy_terrain::{LegacyTerrainDocument, LegacyTerrainFlag, LEGACY_TERRAIN_UOP_NAME};
use uocf::uop_container::file::UopFile;
use uocf::uop_container::package::UopPackage;

use super::UOP_PATCHER_NOTE;
use crate::app::UopInspectorApp;
use crate::logic::legacy_terrain_editor::{LegacyTerrainEditor, LegacyTerrainField};

const ROW_HEIGHT: f32 = 22.0;
const TABLE_MAX_HEIGHT: f32 = 520.0;
const ID_COLUMN_WIDTH: f32 = 60.0;
const NAME_COLUMN_WIDTH: f32 = 180.0;
const TEX_MAP_COLUMN_WIDTH: f32 = 70.0;
const FLAG_COLUMN_WIDTH: f32 = 70.0;
const ACTIONS_COLUMN_WIDTH: f32 = 130.0;
/// Drag speed of the number fields (typing is the main input).
const DRAG_SPEED: f64 = 0.1;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);

/// An edit made in the table, applied once it is drawn.
enum RowAction {
    /// A field gained focus or started a drag: the next edit is a new undo step.
    BreakCoalescing,
    Name(u32, String),
    TexMap(u32, u32),
    Flag(u32, LegacyTerrainFlag, bool),
    Duplicate(u32),
    Remove(u32),
}

pub(super) fn is_legacy_terrain_package(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(LEGACY_TERRAIN_UOP_NAME))
}

/// The view of entry `file` of the LegacyTerrain package `package`, loaded from `source`.
pub(super) fn ui_legacy_terrain_view(
    app: &mut UopInspectorApp,
    ui: &mut egui::Ui,
    file: &UopFile,
    source: &Path,
    package: &UopPackage,
) {
    let mut editor = app.legacy_terrain_editor.take().filter(|editor| editor.source == source);
    match editor.as_mut() {
        None => {
            entry_definition(ui, file);
            ui.separator();
            if ui
                .button("Edit LegacyTerrain.uop")
                .on_hover_text("Edit every definition of the package and save a copy of it.")
                .clicked()
            {
                match LegacyTerrainEditor::open(source.to_path_buf(), package) {
                    Ok(opened) => editor = Some(opened),
                    Err(error) => app.log(format!("Failed to open the LegacyTerrain editor: {error}")),
                }
            }
        }
        Some(editing) => {
            let mut close = false;
            ui.horizontal_wrapped(|ui| close = toolbar(app, ui, editing));
            if close {
                editor = None;
            } else {
                editor_table(ui, editing);
            }
        }
    }
    app.legacy_terrain_editor = editor;
}

/// The typed definition of the entry and its XML.
fn entry_definition(ui: &mut egui::Ui, file: &UopFile) {
    let payload = match file.unpack() {
        Ok(payload) => payload,
        Err(error) => {
            ui.label(format!("Failed to unpack the entry: {error}"));
            return;
        }
    };
    ui.heading("LegacyTerrain definition");
    match LegacyTerrainDocument::parse(&payload) {
        Ok(document) => {
            let terrain = &document.terrain;
            let flags: Vec<&str> = terrain.flags.iter().map(|flag| flag.element_name()).collect();
            egui::Grid::new("legacy_terrain_definition").show(ui, |ui| {
                for (label, value) in [
                    ("id", terrain.id.to_string()),
                    ("name", terrain.name.clone()),
                    ("texMap", terrain.tex_map.to_string()),
                    ("flags", if flags.is_empty() { "none".to_owned() } else { flags.join(", ") }),
                ] {
                    ui.label(label);
                    ui.label(value);
                    ui.end_row();
                }
            });
        }
        Err(error) => {
            ui.colored_label(ERROR_COLOR, format!("Failed to parse the definition: {error:#}"));
        }
    }
    egui::CollapsingHeader::new("XML").default_open(true).show(ui, |ui| {
        let mut text = String::from_utf8_lossy(&payload);
        let mut text = text.to_mut().as_str();
        ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
    });
}

/// Undo, redo, save and close of the editor; true when the editor is closed.
fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui, editor: &mut LegacyTerrainEditor) -> bool {
    if ui.add_enabled(editor.edits.can_undo(), egui::Button::new("Undo")).clicked() {
        editor.undo();
    }
    if ui.add_enabled(editor.edits.can_redo(), egui::Button::new("Redo")).clicked() {
        editor.redo();
    }
    if ui
        .button("Save LegacyTerrain.uop...")
        .on_hover_text(format!(
            "Writes a copy of the loaded LegacyTerrain.uop with the changed definitions; every other entry keeps its \
             bytes. The loaded package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
        ))
        .clicked()
    {
        let (name, extensions) = UOP_FILTER;
        let output =
            crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(LEGACY_TERRAIN_UOP_NAME).save_file();
        if let Some(output) = output {
            match editor.save(&output) {
                Ok(changes) => {
                    editor.error = None;
                    let message = format!(
                        "Saved {}: {} replaced, {} added, {} removed.",
                        output.display(),
                        changes.replaced,
                        changes.added,
                        changes.removed
                    );
                    app.status_message = message.clone();
                    app.log(message);
                }
                Err(error) => editor.error = Some(format!("Save failed: {error}")),
            }
        }
    }
    let dirty = editor.edits.is_dirty();
    let close = ui.button(if dirty { "Discard and close" } else { "Close editor" }).clicked();
    if dirty {
        ui.colored_label(WARNING_COLOR, "* unsaved changes");
    }
    if let Some(error) = &editor.error {
        ui.colored_label(ERROR_COLOR, error);
    }
    close
}

/// Filter, add, and the definitions as editable rows with duplicate and delete per row.
fn editor_table(ui: &mut egui::Ui, editor: &mut LegacyTerrainEditor) {
    let mut actions = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(&mut editor.filter).on_hover_text("Id or name.");
        ui.separator();
        ui.label("New land tile id:");
        ui.add(egui::DragValue::new(&mut editor.new_id).speed(DRAG_SPEED))
            .on_hover_text("The id of the definition that Add or Duplicate creates.");
        if ui.button("Add").on_hover_text("Adds an empty definition with the new id.").clicked() {
            let new_id = editor.new_id;
            editor.error = editor.add(new_id, None).err();
        }
    });
    let ids = editor.visible_ids();
    ui.label(format!("{} of {} definitions", ids.len(), editor.documents.len()));
    let documents = &editor.documents;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        TableBuilder::new(ui)
            .striped(true)
            .max_scroll_height(TABLE_MAX_HEIGHT)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(ID_COLUMN_WIDTH))
            .column(Column::initial(NAME_COLUMN_WIDTH).resizable(true))
            .column(Column::exact(TEX_MAP_COLUMN_WIDTH))
            .columns(Column::auto().at_least(FLAG_COLUMN_WIDTH), LegacyTerrainFlag::ALL.len())
            .column(Column::auto().at_least(ACTIONS_COLUMN_WIDTH))
            .header(ROW_HEIGHT, |mut header| {
                for title in ["id", "name", "texMap"] {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
                for flag in LegacyTerrainFlag::ALL {
                    header.col(|ui| {
                        ui.strong(flag.element_name());
                    });
                }
                header.col(|_| {});
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, ids.len(), |mut row| {
                    let id = ids[row.index()];
                    let terrain = &documents[&id].terrain;
                    row.col(|ui| {
                        ui.label(id.to_string());
                    });
                    row.col(|ui| {
                        let mut name = terrain.name.clone();
                        let response = ui.text_edit_singleline(&mut name);
                        if response.gained_focus() {
                            actions.push(RowAction::BreakCoalescing);
                        }
                        if response.changed() {
                            actions.push(RowAction::Name(id, name));
                        }
                    });
                    row.col(|ui| {
                        let mut tex_map = terrain.tex_map;
                        let response = ui.add(egui::DragValue::new(&mut tex_map).speed(DRAG_SPEED));
                        if response.drag_started() || response.gained_focus() {
                            actions.push(RowAction::BreakCoalescing);
                        }
                        if response.changed() {
                            actions.push(RowAction::TexMap(id, tex_map));
                        }
                    });
                    for flag in LegacyTerrainFlag::ALL {
                        row.col(|ui| {
                            let mut on = terrain.has_flag(flag);
                            if ui.checkbox(&mut on, "").changed() {
                                actions.push(RowAction::Flag(id, flag, on));
                            }
                        });
                    }
                    row.col(|ui| {
                        if ui.small_button("Duplicate").on_hover_text("Copies it as the new land tile id.").clicked() {
                            actions.push(RowAction::Duplicate(id));
                        }
                        if ui.small_button("Delete").clicked() {
                            actions.push(RowAction::Remove(id));
                        }
                    });
                });
            });
    });
    for action in actions {
        match action {
            RowAction::BreakCoalescing => editor.edits.break_coalescing(),
            RowAction::Name(id, name) => editor.edit(id, LegacyTerrainField::Name, |terrain| terrain.name = name),
            RowAction::TexMap(id, tex_map) => {
                editor.edit(id, LegacyTerrainField::TexMap, |terrain| terrain.tex_map = tex_map)
            }
            RowAction::Flag(id, flag, on) => {
                editor.edit(id, LegacyTerrainField::Flag(flag), |terrain| terrain.set_flag(flag, on))
            }
            RowAction::Duplicate(id) => {
                let new_id = editor.new_id;
                editor.error = editor.add(new_id, Some(id)).err();
            }
            RowAction::Remove(id) => editor.remove(id),
        }
    }
}
