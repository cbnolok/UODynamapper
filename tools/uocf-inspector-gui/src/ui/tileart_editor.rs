//! Editing controls of the tileart.uop view of the Tile Metadata tab: the toolbar (edit mode, undo, duplicate,
//! delete, save) and the record editor (fixed fields, flags, image windows, radar colour, properties, sitting). The
//! shader blocks, stack aliases, appearance filters and effects are kept as loaded. The controls collect
//! [`TileArtEdit`]s while the table draws from its copy of the records; [`apply_edits`] then applies them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use uocf::enhanced::tileart::{
    TaeFlag, TaeProp, TaeSittingAnimation, TileArtEntry, TILEART_SITTING_WORD_COUNT, TILEART_UOP_NAME,
};

use crate::app::{tileart_property_name, TileArtFileEntry, UopInspectorApp};
use crate::logic::tileart_editor::{save_tileart, TileArtEditor, TileArtField, TileArtStore};
use crate::ui::UOP_PATCHER_NOTE;

const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const UNDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);
/// Drag speed of the number fields (typing is the main input).
const VALUE_DRAG_SPEED: f64 = 0.1;
/// Drag speed of the scale and offset fields.
const FLOAT_DRAG_SPEED: f64 = 0.01;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const DIRTY_MARKER: &str = "*";
const FLAG_HEX_DIGITS: usize = 16;
const HOUSE_JOIN_HEX_DIGITS: usize = 4;
const FLAG_FIELD_WIDTH: f32 = 150.0;
const FLAG_COLUMNS: usize = 4;
/// Sitting words after the direction mask, with their labels.
const SITTING_SIGNED_WORDS: [&str; TILEART_SITTING_WORD_COUNT - 1] =
    ["seat_offset_x", "seat_offset_y", "window_shift", "field_10", "field_14"];

/// An edit collected while drawing, applied by [`apply_edits`]; it returns the entry to select.
pub(super) type TileArtEdit = Box<dyn FnOnce(&mut TileArtEditor, &mut TileArtStore) -> Option<u64>>;

/// The editing toolbar row.
pub(super) fn toolbar(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, edits: &mut Vec<TileArtEdit>) {
    ui.toggle_value(&mut app.tileart_editor.enabled, "Edit")
        .on_hover_text("Edit the records; duplicate a record as a new tile id; save a copy of tileart.uop.");
    if !app.tileart_editor.enabled {
        return;
    }
    let undo = |editor: &mut TileArtEditor, store: &mut TileArtStore| single_key(editor.edits.undo(store));
    let redo = |editor: &mut TileArtEditor, store: &mut TileArtStore| single_key(editor.edits.redo(store));
    if !ctx.egui_wants_keyboard_input() {
        if ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT)) {
            edits.push(Box::new(undo));
        } else if ctx.input_mut(|input| input.consume_shortcut(&REDO_SHORTCUT)) {
            edits.push(Box::new(redo));
        }
    }
    let selected = app.selected_tileart_hash;
    let editor = &mut app.tileart_editor;
    ui.separator();
    if ui.add_enabled(editor.edits.can_undo(), egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
        edits.push(Box::new(undo));
    }
    if ui.add_enabled(editor.edits.can_redo(), egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
        edits.push(Box::new(redo));
    }
    if ui.button("Revert to loaded").on_hover_text("Back to the records as loaded (undoable).").clicked() {
        edits.push(Box::new(|editor, store| {
            editor.edits.revert_to_opened(store);
            None
        }));
    }
    ui.separator();
    ui.label("New tile id:");
    ui.add(egui::DragValue::new(&mut editor.new_tile_id).speed(VALUE_DRAG_SPEED));
    let new_tile_id = editor.new_tile_id;
    if let Some(hash) = selected {
        if ui
            .button("Duplicate selected")
            .on_hover_text(
                "Adds a copy of the selected record as entry build/tileart/%08d.bin of the new id, with that id \
                 stored. The art, name and texture paths stay the copied ones.",
            )
            .clicked()
        {
            edits.push(Box::new(move |editor, store| match editor.duplicate(store, hash, new_tile_id) {
                Ok(hash) => Some(hash),
                Err(error) => {
                    editor.error = Some(error);
                    None
                }
            }));
        }
        if ui.button("Delete selected").clicked() {
            edits.push(Box::new(move |editor, store| {
                editor.remove(store, hash);
                None
            }));
        }
    }
    ui.separator();
    let source = app.ec_tileart_path.clone();
    if ui
        .add_enabled(source.is_some(), egui::Button::new("Save tileart.uop..."))
        .on_hover_text(format!(
            "Writes a copy of the loaded tileart.uop with the changed, added and deleted records; the others keep \
             their bytes. The loaded package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
        ))
        .clicked()
    {
        if let (Some(source), Some(output)) = (source, save_path()) {
            save(app, &source, &output);
        }
    }
    if app.tileart_editor.edits.is_dirty() {
        ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
    }
    if let Some(error) = app.tileart_editor.error.clone() {
        ui.colored_label(ERROR_COLOR, error);
        if ui.small_button("Dismiss").clicked() {
            app.tileart_editor.error = None;
        }
    }
}

/// The editor of the selected record.
pub(super) fn record_panel(ui: &mut egui::Ui, file: &TileArtFileEntry, edits: &mut Vec<TileArtEdit>) {
    let hash = file.filename_hash;
    let entry = &file.entry;
    let mut fields = Fields { hash, edits };
    egui::CollapsingHeader::new("Edit record").default_open(true).show(ui, |ui| {
        egui::Grid::new(("tileart_edit_core", hash)).num_columns(2).striped(true).show(ui, |ui| {
            fields.number(ui, "Name index", "name_index", entry.string_dict_off, |entry, value| entry.string_dict_off = value);
            fields.number(ui, "Stored tile id", "tile_id", entry.tile_id, |entry, value| entry.tile_id = value);
            fields.number(ui, "Old id", "old_id", entry.old_id, |entry, value| entry.old_id = value);
            fields.number(ui, "Body type", "body_type", entry.body_type, |entry, value| entry.body_type = value);
            fields.number(ui, "Facing", "facing", entry.facing, |entry, value| entry.facing = value);
            fields.float(ui, "World art scale", "world_art_scale", entry.world_art_scale, |entry, value| entry.world_art_scale = value);
            fields.float(ui, "Legacy art scale", "legacy_art_scale", entry.legacy_art_scale, |entry, value| entry.legacy_art_scale = value);
            fields.float(ui, "Rotation degrees", "rotation_degrees", entry.rotation_degrees, |entry, value| entry.rotation_degrees = value);
            fields.float(ui, "Light scale", "light_scale", entry.light_scale, |entry, value| entry.light_scale = value);
            fields.float(ui, "Light offset x", "light_offset_x", entry.light_offset_x, |entry, value| entry.light_offset_x = value);
            fields.float(ui, "Light offset y", "light_offset_y", entry.light_offset_y, |entry, value| entry.light_offset_y = value);
            fields.number(ui, "field_60", "field_60", entry.field_60, |entry, value| entry.field_60 = value);
            fields.number(ui, "light_field_70", "light_field_70", entry.light_field_70, |entry, value| entry.light_field_70 = value);
            ui.label("House join bits");
            let mut bits = entry.house_join_bits;
            let drag = egui::DragValue::new(&mut bits).speed(VALUE_DRAG_SPEED).hexadecimal(HOUSE_JOIN_HEX_DIGITS, false, true);
            if ui.add(drag).changed() {
                fields.push("house_join_bits", move |entry| entry.house_join_bits = bits);
            }
            ui.end_row();
            ui.label("Switches");
            ui.horizontal(|ui| {
                fields.toggle(ui, "Has world art", "has_world_art", entry.has_world_art, |entry, value| entry.has_world_art = value);
                fields.toggle(ui, "Enhanced tileart", "uses_enhanced_tileart", entry.uses_enhanced_tileart, |entry, value| entry.uses_enhanced_tileart = value);
                fields.toggle(ui, "Alternate animation", "alternate_animation", entry.alternate_animation, |entry, value| entry.alternate_animation = value);
            });
            ui.end_row();
            ui.label("Radar RGBA");
            ui.horizontal(|ui| {
                let colour = &entry.radarcol;
                fields.inline_number(ui, "radar_r", colour.r, |entry, value| entry.radarcol.r = value);
                fields.inline_number(ui, "radar_g", colour.g, |entry, value| entry.radarcol.g = value);
                fields.inline_number(ui, "radar_b", colour.b, |entry, value| entry.radarcol.b = value);
                fields.inline_number(ui, "radar_a", colour.a, |entry, value| entry.radarcol.a = value);
            });
            ui.end_row();
            for (label, field, window, select) in [
                ("EC window", "ec_window", &entry.ec_img_offset, true),
                ("CC window", "cc_window", &entry.cc_img_offset, false),
            ] {
                ui.label(label).on_hover_text("x start, y start, x end, y end, x offset, y offset");
                ui.horizontal(|ui| {
                    let values = [window.x_start, window.y_start, window.x_end, window.y_end, window.x_off, window.y_off];
                    for (index, current) in values.into_iter().enumerate() {
                        let mut value = current;
                        if ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED)).changed() {
                            fields.push(field, move |entry| {
                                let window = if select { &mut entry.ec_img_offset } else { &mut entry.cc_img_offset };
                                let slots = [
                                    &mut window.x_start,
                                    &mut window.y_start,
                                    &mut window.x_end,
                                    &mut window.y_end,
                                    &mut window.x_off,
                                    &mut window.y_off,
                                ];
                                *slots.into_iter().nth(index).expect("six window values") = value;
                            });
                        }
                    }
                });
                ui.end_row();
            }
        });

        for (label, field, flags, flags2) in [("Flags1", "flags1", entry.flags1, false), ("Flags2", "flags2", entry.flags2, true)] {
            fields.flags(ui, label, field, flags, flags2);
        }
        for (label, field, props, second) in
            [("Properties 1", "props1", &entry.prop_vector1, false), ("Properties 2", "props2", &entry.prop_vector2, true)]
        {
            fields.props(ui, label, field, props, second);
        }
        fields.sitting(ui, entry.sitting.as_ref());
    });
}

/// Pushes the edits of one record.
struct Fields<'a> {
    hash: u64,
    edits: &'a mut Vec<TileArtEdit>,
}

impl Fields<'_> {
    /// Starts a new undo step when `response` starts a drag or typing session.
    fn session(&mut self, response: &egui::Response) {
        if super::multi_editor::starts_an_edit(response) {
            self.edits.push(Box::new(|editor, _| {
                editor.edits.break_coalescing();
                None
            }));
        }
    }

    fn push(&mut self, field: TileArtField, edit: impl FnOnce(&mut TileArtEntry) + 'static) {
        let hash = self.hash;
        self.edits.push(Box::new(move |editor, store| {
            editor.edit(store, hash, field, edit);
            None
        }));
    }

    fn number<T: egui::emath::Numeric + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        field: TileArtField,
        current: T,
        set: fn(&mut TileArtEntry, T),
    ) {
        ui.label(label);
        self.inline_number(ui, field, current, set);
        ui.end_row();
    }

    fn inline_number<T: egui::emath::Numeric + 'static>(
        &mut self,
        ui: &mut egui::Ui,
        field: TileArtField,
        current: T,
        set: fn(&mut TileArtEntry, T),
    ) {
        let mut value = current;
        let response = ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED));
        self.session(&response);
        if response.changed() {
            self.push(field, move |entry| set(entry, value));
        }
    }

    fn float(&mut self, ui: &mut egui::Ui, label: &str, field: TileArtField, current: f32, set: fn(&mut TileArtEntry, f32)) {
        ui.label(label);
        let mut value = current;
        let response = ui.add(egui::DragValue::new(&mut value).speed(FLOAT_DRAG_SPEED));
        self.session(&response);
        if response.changed() {
            self.push(field, move |entry| set(entry, value));
        }
        ui.end_row();
    }

    fn toggle(&mut self, ui: &mut egui::Ui, label: &str, field: TileArtField, current: bool, set: fn(&mut TileArtEntry, bool)) {
        let mut value = current;
        if ui.checkbox(&mut value, label).changed() {
            self.push(field, move |entry| set(entry, value));
        }
    }

    /// A 64-bit flag field: the hex value (applied when the field loses focus) and a checkbox per named bit.
    fn flags(&mut self, ui: &mut egui::Ui, label: &str, field: TileArtField, current: TaeFlag, flags2: bool) {
        let set = move |entry: &mut TileArtEntry, value: TaeFlag| {
            if flags2 {
                entry.flags2 = value;
            } else {
                entry.flags1 = value;
            }
        };
        egui::CollapsingHeader::new(format!("{label} 0x{:0FLAG_HEX_DIGITS$X}", current.bits()))
            .id_salt((field, self.hash))
            .show(ui, |ui| {
                let buffer_id = ui.make_persistent_id((field, self.hash, "hex"));
                let mut text = ui
                    .data_mut(|data| data.get_temp::<String>(buffer_id))
                    .unwrap_or_else(|| format!("{:0FLAG_HEX_DIGITS$X}", current.bits()));
                let response = ui.horizontal(|ui| {
                    ui.label("Hex");
                    ui.add(egui::TextEdit::singleline(&mut text).desired_width(FLAG_FIELD_WIDTH))
                }).inner;
                if response.has_focus() {
                    ui.data_mut(|data| data.insert_temp(buffer_id, text));
                } else {
                    ui.data_mut(|data| data.remove::<String>(buffer_id));
                    if response.lost_focus() {
                        match u64::from_str_radix(text.trim().trim_start_matches("0x"), 16) {
                            Ok(bits) => self.push(field, move |entry| set(entry, TaeFlag::from_bits_retain(bits))),
                            Err(_) => {
                                let error = format!("{text:?} is not a hexadecimal flag value");
                                self.edits.push(Box::new(move |editor, _| {
                                    editor.error = Some(error);
                                    None
                                }));
                            }
                        }
                    }
                }
                egui::Grid::new((field, self.hash, "bits")).num_columns(FLAG_COLUMNS).show(ui, |ui| {
                    let single_bits = TaeFlag::all().iter_names().filter(|(_, flag)| flag.bits().is_power_of_two());
                    for (column, (name, flag)) in single_bits.enumerate() {
                        let mut set_bit = current.contains(flag);
                        if ui.checkbox(&mut set_bit, name).changed() {
                            let mut value = current;
                            value.set(flag, set_bit);
                            self.push(field, move |entry| set(entry, value));
                        }
                        if (column + 1) % FLAG_COLUMNS == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
    }

    /// A property list: key and value per row, add and delete.
    fn props(&mut self, ui: &mut egui::Ui, label: &str, field: TileArtField, props: &[TaeProp], second: bool) {
        egui::CollapsingHeader::new(format!("{label} ({})", props.len())).id_salt((field, self.hash)).show(ui, |ui| {
            egui::Grid::new((field, self.hash, "rows")).striped(true).show(ui, |ui| {
                for (index, prop) in props.iter().enumerate() {
                    let (mut key, mut value) = (prop.id, prop.val);
                    let key_changed = ui.add(egui::DragValue::new(&mut key).speed(VALUE_DRAG_SPEED)).changed();
                    ui.label(tileart_property_name(key));
                    let value_changed = ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED)).changed();
                    if key_changed || value_changed {
                        self.push(field, move |entry| {
                            if let Some(prop) = prop_list(entry, second).get_mut(index) {
                                *prop = TaeProp { id: key, val: value };
                            }
                        });
                    }
                    if ui.small_button("Delete").clicked() {
                        self.push(field, move |entry| {
                            prop_list(entry, second).remove(index);
                        });
                    }
                    ui.end_row();
                }
            });
            if ui.small_button("Add property").clicked() {
                self.push(field, move |entry| prop_list(entry, second).push(TaeProp::default()));
            }
        });
    }

    /// The sitting block: present or not, and its words.
    fn sitting(&mut self, ui: &mut egui::Ui, sitting: Option<&TaeSittingAnimation>) {
        let field = "sitting";
        egui::CollapsingHeader::new("Sitting").id_salt((field, self.hash)).show(ui, |ui| {
            let mut present = sitting.is_some();
            if ui.checkbox(&mut present, "Sitting block").changed() {
                self.push(field, move |entry| entry.sitting = present.then(TaeSittingAnimation::default));
            }
            let Some(sitting) = sitting else {
                return;
            };
            ui.horizontal(|ui| {
                ui.label("direction_mask");
                let mut mask = sitting.direction_mask;
                if ui.add(egui::DragValue::new(&mut mask).speed(VALUE_DRAG_SPEED)).changed() {
                    self.push(field, move |entry| {
                        if let Some(sitting) = entry.sitting.as_mut() {
                            sitting.direction_mask = mask;
                        }
                    });
                }
                let values =
                    [sitting.seat_offset_x, sitting.seat_offset_y, sitting.window_shift, sitting.field_10, sitting.field_14];
                for (index, (label, current)) in SITTING_SIGNED_WORDS.into_iter().zip(values).enumerate() {
                    ui.label(label);
                    let mut value = current;
                    if ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED)).changed() {
                        self.push(field, move |entry| {
                            if let Some(sitting) = entry.sitting.as_mut() {
                                let words = [
                                    &mut sitting.seat_offset_x,
                                    &mut sitting.seat_offset_y,
                                    &mut sitting.window_shift,
                                    &mut sitting.field_10,
                                    &mut sitting.field_14,
                                ];
                                *words.into_iter().nth(index).expect("five signed words") = value;
                            }
                        });
                    }
                }
            });
        });
    }
}

fn prop_list(entry: &mut TileArtEntry, second: bool) -> &mut Vec<TaeProp> {
    if second {
        &mut entry.prop_vector2
    } else {
        &mut entry.prop_vector1
    }
}

/// Applies the collected edits to the loaded records and moves the selection.
pub(super) fn apply_edits(app: &mut UopInspectorApp, edits: Vec<TileArtEdit>) {
    if edits.is_empty() {
        return;
    }
    let (Some(entries), Some(rows)) = (app.ec_tileart_entries.as_mut(), app.ec_tileart_rows.as_mut()) else {
        return;
    };
    let mut store = TileArtStore {
        entries: Arc::make_mut(entries),
        rows: Arc::make_mut(rows),
        dictionary: app.uo_string_dictionary.as_deref(),
    };
    for edit in edits {
        if let Some(hash) = edit(&mut app.tileart_editor, &mut store) {
            app.selected_tileart_hash = Some(hash);
        }
    }
}

/// After an undo or redo, the entry it changed.
fn single_key(keys: Option<Vec<u64>>) -> Option<u64> {
    match keys.as_deref() {
        Some([hash]) => Some(*hash),
        _ => None,
    }
}

fn save_path() -> Option<PathBuf> {
    let (name, extensions) = UOP_FILTER;
    crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(TILEART_UOP_NAME).save_file()
}

fn save(app: &mut UopInspectorApp, source: &Path, output: &Path) {
    let Some(entries) = app.ec_tileart_entries.clone() else {
        return;
    };
    let result = save_tileart(&app.tileart_editor.edits, &entries, source, output);
    match result {
        Ok(changes) => {
            app.tileart_editor.edits.mark_saved();
            app.tileart_editor.error = None;
            let message = format!(
                "Saved {} ({} records replaced, {} added, {} removed).",
                output.display(),
                changes.replaced,
                changes.added,
                changes.removed
            );
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.tileart_editor.error = Some(format!("Save failed: {error}")),
    }
}
