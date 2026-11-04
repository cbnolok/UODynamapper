//! Editing controls of the MultiCollection.uop view of the Multis tab: the toolbar (edit mode, undo, add and delete
//! multis, save) and the selected component editor. The controls collect [`MultiEdit`]s while the view draws from
//! its copy of the collection; [`apply_edits`] then applies them to the loaded collection.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use uocf::enhanced::multis::{MultiItem, MultiItemPart, MULTI_COLLECTION_UOP_NAME};

use crate::app::{UopInspectorApp, ViewMode};
use crate::logic::multi_editor::{
    first_free_multi_id, format_string_ids, new_part, parse_string_ids, save_multi_collection, MultiEditor,
    MultiPartField,
};
use crate::ui::unsaved_changes::TabEditor;
use crate::ui::UOP_PATCHER_NOTE;

const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const UNDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
/// Drag speed of the number fields (typing is the main input).
const VALUE_DRAG_SPEED: f64 = 0.1;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const DIRTY_MARKER: &str = "*";
const STRING_IDS_FIELD_WIDTH: f32 = 160.0;

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Multis,
    label: "the MultiCollection.uop multis",
    is_dirty: |app| app.multi_editor.edits.is_dirty(),
    save: |app| {
        if let (Some(source), Some(output)) = (app.multi_collection_path.clone(), save_path()) {
            save(app, &source, &output);
        }
    },
    discard: |app| {
        if let Some(collection) = app.multi_collection.as_mut() {
            app.multi_editor.edits.revert_to_opened(Arc::make_mut(collection));
            app.multi_editor.edits.mark_saved();
        }
    },
};

/// What to select after an edit.
pub(super) enum Selection {
    Multi(u32),
    Part(usize),
    NoPart,
}

/// An edit collected while drawing, applied by [`apply_edits`].
pub(super) type MultiEdit = Box<dyn FnOnce(&mut MultiEditor, &mut uocf::enhanced::multis::MultiCollection) -> Option<Selection>>;

/// The editing toolbar row; `selected` is the multi selected in the list.
pub(super) fn toolbar(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, edits: &mut Vec<MultiEdit>) {
    let selected = app.selected_multi_id;
    let selected_exists = app.multi_collection.as_ref().is_some_and(|collection| collection.get(selected).is_some());
    let source = app.multi_collection_path.clone();
    ui.toggle_value(&mut app.multi_editor.enabled, "Edit")
        .on_hover_text("Edit the multis: components, new and deleted multis; save a copy of the package.");
    if !app.multi_editor.enabled {
        return;
    }
    if !ctx.egui_wants_keyboard_input() {
        if ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT)) {
            edits.push(Box::new(|editor, collection| editor.edits.undo(collection).and_then(selection_after_undo)));
        } else if ctx.input_mut(|input| input.consume_shortcut(&REDO_SHORTCUT)) {
            edits.push(Box::new(|editor, collection| editor.edits.redo(collection).and_then(selection_after_undo)));
        }
    }
    let editor = &mut app.multi_editor;
    ui.separator();
    if ui.add_enabled(editor.edits.can_undo(), egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
        edits.push(Box::new(|editor, collection| editor.edits.undo(collection).and_then(selection_after_undo)));
    }
    if ui.add_enabled(editor.edits.can_redo(), egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
        edits.push(Box::new(|editor, collection| editor.edits.redo(collection).and_then(selection_after_undo)));
    }
    if ui.button("Revert to loaded").on_hover_text("Back to the multis as loaded (undoable).").clicked() {
        edits.push(Box::new(|editor, collection| {
            editor.edits.revert_to_opened(collection);
            None
        }));
    }
    ui.separator();
    ui.label("New id:");
    ui.add(egui::DragValue::new(&mut editor.new_multi_id).speed(VALUE_DRAG_SPEED));
    if ui.small_button("First free").clicked() {
        if let Some(id) = app.multi_collection.as_deref().and_then(first_free_multi_id) {
            app.multi_editor.new_multi_id = id;
        }
    }
    let new_id = app.multi_editor.new_multi_id;
    if ui.button("Add empty").clicked() {
        edits.push(Box::new(move |editor, collection| add_multi(editor, collection, new_id, None)));
    }
    if ui.add_enabled(selected_exists, egui::Button::new(format!("Duplicate {selected}"))).clicked() {
        edits.push(Box::new(move |editor, collection| add_multi(editor, collection, new_id, Some(selected))));
    }
    if ui.add_enabled(selected_exists, egui::Button::new(format!("Delete {selected}"))).clicked() {
        edits.push(Box::new(move |editor, collection| {
            editor.remove_multi(collection, selected);
            None
        }));
    }
    ui.separator();
    if ui
        .add_enabled(source.is_some(), egui::Button::new("Save MultiCollection.uop..."))
        .on_hover_text(format!(
            "Writes a copy of the loaded MultiCollection.uop with these multis; housing.bin and the unchanged \
             multis keep their bytes. The loaded package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
        ))
        .clicked()
    {
        if let (Some(source), Some(output)) = (source, save_path()) {
            save(app, &source, &output);
        }
    }
    let editor = &app.multi_editor;
    if editor.edits.is_dirty() {
        ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
    }
    if let Some(error) = editor.error.clone() {
        ui.colored_label(ERROR_COLOR, error);
        if ui.small_button("Dismiss").clicked() {
            app.multi_editor.error = None;
        }
    }
}

/// The editor of the selected component of `item`, with add, duplicate and delete buttons.
pub(super) fn component_panel(
    ui: &mut egui::Ui,
    item: &MultiItem,
    selected_part: Option<usize>,
    edits: &mut Vec<MultiEdit>,
) {
    let id = item.id;
    ui.horizontal(|ui| {
        ui.strong(format!("Multi {id}: {} components", item.parts.len()));
        ui.separator();
        if ui.button("Add component").clicked() {
            let part = selected_part.and_then(|index| item.parts.get(index)).cloned().unwrap_or_else(new_part);
            edits.push(Box::new(move |editor, collection| editor.add_part(collection, id, part).map(Selection::Part)));
        }
        if let Some(index) = selected_part.filter(|index| *index < item.parts.len()) {
            if ui.button("Duplicate component").clicked() {
                edits.push(Box::new(move |editor, collection| {
                    editor.duplicate_part(collection, id, index).map(Selection::Part)
                }));
            }
            if ui.button("Delete component").clicked() {
                edits.push(Box::new(move |editor, collection| {
                    editor.remove_part(collection, id, index);
                    Some(Selection::NoPart)
                }));
            }
        }
    });
    let Some((index, part)) = selected_part.and_then(|index| item.parts.get(index).map(|part| (index, part))) else {
        ui.label("Select a component in the list to edit it.");
        return;
    };
    egui::Grid::new(("multi_component_editor", id, index)).num_columns(2).show(ui, |ui| {
        ui.label(format!("Component #{index}"));
        ui.horizontal(|ui| {
            part_value(ui, edits, id, index, MultiPartField::ItemId, "Tile", part.item_id, |part, value| part.item_id = value);
            part_value(ui, edits, id, index, MultiPartField::X, "X", part.x, |part, value| part.x = value);
            part_value(ui, edits, id, index, MultiPartField::Y, "Y", part.y, |part, value| part.y = value);
            part_value(ui, edits, id, index, MultiPartField::Z, "Z", part.z, |part, value| part.z = value);
            part_value(ui, edits, id, index, MultiPartField::Flag2, "flag2", part.flag2, |part, value| part.flag2 = value);
            let mut not_drawn = part.not_drawn;
            if ui.checkbox(&mut not_drawn, "Not drawn").on_hover_text("The client never draws this part.").changed() {
                edits.push(Box::new(move |editor, collection| {
                    editor.edit_part(collection, id, index, MultiPartField::NotDrawn, |part| part.not_drawn = not_drawn);
                    None
                }));
            }
        });
        ui.end_row();
        ui.label("String ids");
        string_ids_field(ui, edits, id, index, part);
        ui.end_row();
    });
}

fn part_value<T: egui::emath::Numeric + PartialEq + 'static>(
    ui: &mut egui::Ui,
    edits: &mut Vec<MultiEdit>,
    id: u32,
    index: usize,
    field: MultiPartField,
    label: &str,
    current: T,
    set: fn(&mut MultiItemPart, T),
) {
    ui.label(label);
    let mut value = current;
    let response = ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED));
    if starts_an_edit(&response) {
        edits.push(Box::new(|editor, _| {
            editor.edits.break_coalescing();
            None
        }));
    }
    if response.changed() && value != current {
        edits.push(Box::new(move |editor, collection| {
            editor.edit_part(collection, id, index, field, |part| set(part, value));
            None
        }));
    }
}

/// A new drag or typing session of a field: a new undo step even when the same field was edited last.
pub(super) fn starts_an_edit(response: &egui::Response) -> bool {
    response.drag_started() || response.gained_focus()
}

/// The string ids as text; applied when the field loses focus.
fn string_ids_field(ui: &mut egui::Ui, edits: &mut Vec<MultiEdit>, id: u32, index: usize, part: &MultiItemPart) {
    let buffer_id = ui.make_persistent_id(("multi_part_string_ids", id, index));
    let mut text = ui
        .data_mut(|data| data.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| format_string_ids(&part.string_ids));
    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(STRING_IDS_FIELD_WIDTH))
        .on_hover_text("string_dictionary.uop indices of the part tags, separated by commas.");
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(buffer_id, text));
        return;
    }
    ui.data_mut(|data| data.remove::<String>(buffer_id));
    if !response.lost_focus() {
        return;
    }
    match parse_string_ids(&text) {
        Ok(ids) if ids != part.string_ids => edits.push(Box::new(move |editor, collection| {
            editor.edit_part(collection, id, index, MultiPartField::StringIds, |part| part.string_ids = ids);
            None
        })),
        Ok(_) => {}
        Err(error) => edits.push(Box::new(move |editor, _| {
            editor.error = Some(error);
            None
        })),
    }
}

/// Applies the collected edits to the loaded collection and moves the selection.
pub(super) fn apply_edits(app: &mut UopInspectorApp, ctx: &egui::Context, edits: Vec<MultiEdit>, part_id: egui::Id) {
    if edits.is_empty() {
        return;
    }
    let Some(collection) = app.multi_collection.as_mut() else {
        return;
    };
    let collection = Arc::make_mut(collection);
    let mut selections = Vec::new();
    for edit in edits {
        selections.extend(edit(&mut app.multi_editor, collection));
    }
    for selection in selections {
        match selection {
            Selection::Multi(id) => {
                app.selected_multi_id = id;
                app.selected_multi_uop_hash =
                    app.multi_collection.as_ref().and_then(|collection| collection.get(id)).map(|item| item.filename_hash);
            }
            Selection::Part(index) => {
                ctx.data_mut(|data| data.insert_temp(part_id, index));
            }
            Selection::NoPart => {
                ctx.data_mut(|data| data.remove::<usize>(part_id));
            }
        }
    }
}

fn add_multi(
    editor: &mut MultiEditor,
    collection: &mut uocf::enhanced::multis::MultiCollection,
    id: u32,
    template: Option<u32>,
) -> Option<Selection> {
    match editor.add_multi(collection, id, template) {
        Ok(()) => Some(Selection::Multi(id)),
        Err(error) => {
            editor.error = Some(error);
            None
        }
    }
}

/// After an undo or redo, the multi it changed when it was a single one.
fn selection_after_undo(ids: Vec<u32>) -> Option<Selection> {
    match ids.as_slice() {
        [id] => Some(Selection::Multi(*id)),
        _ => None,
    }
}

fn save_path() -> Option<PathBuf> {
    let (name, extensions) = UOP_FILTER;
    crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(MULTI_COLLECTION_UOP_NAME).save_file()
}

fn save(app: &mut UopInspectorApp, source: &std::path::Path, output: &std::path::Path) {
    let Some(collection) = app.multi_collection.as_deref() else {
        return;
    };
    match save_multi_collection(collection, source, output) {
        Ok(changes) => {
            app.multi_editor.edits.mark_saved();
            app.multi_editor.error = None;
            let message = format!(
                "Saved {} ({} multis replaced, {} added, {} removed).",
                output.display(),
                changes.replaced,
                changes.added,
                changes.removed
            );
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.multi_editor.error = Some(format!("Save failed: {error}")),
    }
}
