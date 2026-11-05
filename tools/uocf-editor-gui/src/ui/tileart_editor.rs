//! Editing controls of the tileart.uop view of the Tile Metadata tab: the toolbar (edit mode, undo, duplicate,
//! delete, save) and the record panel (fixed fields, flags, image windows, radar colour, properties, sitting, stack
//! aliases, appearance filters, texture blocks, effects), which shows the record whether or not it can be edited.
//! The controls collect [`TileArtEdit`]s while the table draws from its copy of the records; [`apply_edits`] then
//! applies them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use uocf::enhanced::effect_definition::{effect_definitions_from_kdl, effect_definitions_to_kdl, EffectDefinition};
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::{
    TaeAppearanceFilter, TaeBodyAppearance, TaeFlag, TaeImgOffset, TaeProp, TaeSittingAnimation, TaeStackAlias, TaeTexture,
    TaeTextureImage, TileArtEntry, TILEART_KR_RECORD_VERSION, TILEART_SHADER_BLOCK_COUNT, TILEART_SHADER_BLOCK_ENHANCED,
    TILEART_SHADER_BLOCK_PRESENT, TILEART_SITTING_WORD_COUNT, TILEART_UOP_NAME,
};

use crate::app::{tileart_property_name, TileArtFileEntry, UocfEditorApp, ViewMode};
use crate::logic::tileart_editor::{save_tileart, EffectsDraft, TileArtEditor, TileArtField, TileArtStore};
use crate::ui::unsaved_changes::TabEditor;
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
/// Room for one flag's checkbox and name: a group has as many flag columns as fit.
const FLAG_CELL_WIDTH: f32 = 140.0;
/// A record's groups go in as many columns of at least this width as fit.
const GROUP_MIN_WIDTH: f32 = 330.0;
/// The texture blocks and effects take this many group widths: their tables have many columns.
const WIDE_GROUP_SPAN: f32 = 2.0;
const WIDE_GROUP_MIN_WIDTH: f32 = WIDE_GROUP_SPAN * GROUP_MIN_WIDTH;
/// How much of a field's colour stays while the record is read only: its values must stay readable.
const READ_ONLY_ALPHA: f32 = 0.85;
const RADAR_SWATCH_SIDE: f32 = 16.0;
const FLOAT_SHOWN_DECIMALS: usize = 3;
/// What the shader blocks of a record draw, by position.
const TEXTURE_BLOCK_NAMES: [&str; TILEART_SHADER_BLOCK_COUNT] = ["WorldArt", "TileArtLegacy", "TileArtEnhanced", "Light texture"];
/// The stretch of a new texture item: the client switches the sampler to wrap on any other value.
const NEW_ITEM_STRETCH: f32 = 1.0;
const STRING_INDEX_HOVER: &str = "The position of the text in string_dictionary.uop, plus 1. 0 names nothing.";
const SHADER_IMPLEMENTATION_HOVER: &str = "The shader implementation index the client passes to the shader factory. 0 in all client records.";
const SOURCE_KIND_HOVER: &str = "0 loads the texture, 2 loads it and forces wrap addressing, and any other value makes the client skip it.";
const SAMPLER_HOVER: &str = "Not read by the client, which binds an item to the shader map at its position.";
const VALUE_LISTS_HOVER: &str = "The client keeps up to 4 values.";
/// Column titles of the sub-record tables, each with its hover text ("" for none).
const PROPERTY_COLUMNS: [(&str, &str); 3] = [("Key", ""), ("Name", ""), ("Value", "")];
const STACK_ALIAS_COLUMNS: [(&str, &str); 2] = [("Amount", ""), ("Amount id", "")];
const BODY_APPEARANCE_COLUMNS: [(&str, &str); 2] = [("Body", ""), ("Appearance", "")];
const GENDER_APPEARANCE_COLUMNS: [(&str, &str); 2] = [("Gender", ""), ("Appearance", "")];
/// Title of the column of the rows' Delete buttons.
const DELETE_COLUMN: &str = "Delete";
const TEXTURE_ITEM_COLUMNS: [(&str, &str); 7] = [
    ("Item", ""),
    ("Name index", STRING_INDEX_HOVER),
    ("Path", ""),
    ("Source", SOURCE_KIND_HOVER),
    ("Stretch", ""),
    ("Sampler", SAMPLER_HOVER),
    ("UV set", ""),
];
const EFFECTS_TEXT_ROWS: usize = 6;
const EFFECTS_APPLY_HOVER: &str = "Reads the text into the record (undoable). Text that is not applied is not saved.";
/// The appearance filters a record can gain.
const NEW_APPEARANCE_FILTERS: [(&str, fn() -> TaeAppearanceFilter); 3] = [
    ("Add by body", || TaeAppearanceFilter::ByOwnerBodyType(Vec::new())),
    ("Add by gender", || TaeAppearanceFilter::ByOwnerGender { gender: 0, appearance: 0 }),
    ("Add animation by body", || TaeAppearanceFilter::AnimationByOwnerBodyType(Vec::new())),
];
/// Sitting words after the direction mask, with their labels.
const SITTING_SIGNED_WORDS: [&str; TILEART_SITTING_WORD_COUNT - 1] =
    ["seat_offset_x", "seat_offset_y", "window_shift", "field_10", "field_14"];

/// The six stored values of an image window, in the order UOETE lays them out.
const WINDOW_SLOTS: [(&str, fn(&mut TaeImgOffset) -> &mut i32); 6] = [
    ("x start", |window| &mut window.x_start),
    ("y start", |window| &mut window.y_start),
    ("x end", |window| &mut window.x_end),
    ("y end", |window| &mut window.y_end),
    ("x offset", |window| &mut window.x_off),
    ("y offset", |window| &mut window.y_off),
];

/// An edit collected while drawing, applied by [`apply_edits`]; it returns the entry to select.
pub(super) type TileArtEdit = Box<dyn FnOnce(&mut TileArtEditor, &mut TileArtStore) -> Option<u64>>;

/// The editing toolbar row.
pub(super) fn toolbar(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui, edits: &mut Vec<TileArtEdit>) {
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

/// Draws a record's groups in as many equal columns as fit, the fields readable but locked unless `editable`.
pub(super) fn record_columns(ui: &mut egui::Ui, editable: bool, add_groups: impl FnOnce(&mut [egui::Ui])) {
    columns_at_least(ui, editable, GROUP_MIN_WIDTH, add_groups);
}

/// [`record_columns`] for groups that need `min_width` (tables of many columns).
fn columns_at_least(ui: &mut egui::Ui, editable: bool, min_width: f32, add_groups: impl FnOnce(&mut [egui::Ui])) {
    ui.scope(|ui| {
        ui.visuals_mut().disabled_alpha = READ_ONLY_ALPHA;
        ui.add_enabled_ui(editable, |ui| {
            let count = ((ui.available_width() / min_width) as usize).max(1);
            ui.columns(count, add_groups);
        });
    });
}

/// A titled group of fields, in the column that is the shortest so far.
pub(super) fn group<R>(columns: &mut [egui::Ui], title: &str, add_fields: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let column = columns
        .iter_mut()
        .min_by(|left, right| left.min_rect().bottom().total_cmp(&right.min_rect().bottom()))
        .expect("a record has at least one column");
    egui::Frame::group(column.style())
        .show(column, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.strong(title);
                add_fields(ui)
            })
            .inner
        })
        .inner
}

/// The fields of the selected record. `name` is the text of its name index.
pub(super) fn record_panel(
    ui: &mut egui::Ui,
    file: &TileArtFileEntry,
    name: Option<&str>,
    strings: Option<&UoStringDictionary>,
    effects_draft: &mut Option<EffectsDraft>,
    editable: bool,
    edits: &mut Vec<TileArtEdit>,
) {
    let hash = file.filename_hash;
    let entry = &file.entry;
    let mut fields = Fields { hash, editable, edits };
    record_columns(ui, editable, |columns| {
        group(columns, "Record", |ui| {
            egui::Grid::new(("tileart_record", hash)).num_columns(2).striped(true).show(ui, |ui| {
                ui.label("Filename hash");
                ui.monospace(format!("0x{hash:016X}"));
                ui.end_row();
                ui.label("Version");
                ui.label(entry.version.to_string());
                ui.end_row();
                ui.label("Name");
                ui.label(name.unwrap_or("-"));
                ui.end_row();
                fields.number(ui, "Name index", "name_index", entry.string_dict_off, |entry, value| entry.string_dict_off = value);
                ui.label("Stored tile id");
                ui.horizontal(|ui| {
                    fields.inline_number(ui, "tile_id", entry.tile_id, |entry, value| entry.tile_id = value);
                    ui.weak(format!("0x{:04X}", entry.tile_id));
                });
                ui.end_row();
                fields.number(ui, "Old id", "old_id", entry.old_id, |entry, value| entry.old_id = value);
                fields.number(ui, "Body type", "body_type", entry.body_type, |entry, value| entry.body_type = value);
                fields.number(ui, "Facing", "facing", entry.facing, |entry, value| entry.facing = value);
                fields.number(ui, "field_60", "field_60", entry.field_60, |entry, value| entry.field_60 = value);
                ui.label("House join bits");
                let mut bits = entry.house_join_bits;
                let drag = egui::DragValue::new(&mut bits).speed(VALUE_DRAG_SPEED).hexadecimal(HOUSE_JOIN_HEX_DIGITS, false, true);
                if ui.add(drag).changed() {
                    fields.push("house_join_bits", move |entry| entry.house_join_bits = bits);
                }
                ui.end_row();
            });
        });
        group(columns, "Art and light", |ui| {
            egui::Grid::new(("tileart_art_light", hash)).num_columns(2).striped(true).show(ui, |ui| {
                fields.float(ui, "World art scale", "world_art_scale", entry.world_art_scale, |entry, value| entry.world_art_scale = value);
                fields.float(ui, "Legacy art scale", "legacy_art_scale", entry.legacy_art_scale, |entry, value| entry.legacy_art_scale = value);
                fields.float(ui, "Rotation degrees", "rotation_degrees", entry.rotation_degrees, |entry, value| entry.rotation_degrees = value);
                fields.float(ui, "Light scale", "light_scale", entry.light_scale, |entry, value| entry.light_scale = value);
                fields.float(ui, "Light offset x", "light_offset_x", entry.light_offset_x, |entry, value| entry.light_offset_x = value);
                fields.float(ui, "Light offset y", "light_offset_y", entry.light_offset_y, |entry, value| entry.light_offset_y = value);
                fields.number(ui, "light_field_70", "light_field_70", entry.light_field_70, |entry, value| entry.light_field_70 = value);
                ui.label("Radar RGBA");
                ui.horizontal(|ui| {
                    let colour = &entry.radarcol;
                    fields.inline_number(ui, "radar_r", colour.r, |entry, value| entry.radarcol.r = value);
                    fields.inline_number(ui, "radar_g", colour.g, |entry, value| entry.radarcol.g = value);
                    fields.inline_number(ui, "radar_b", colour.b, |entry, value| entry.radarcol.b = value);
                    fields.inline_number(ui, "radar_a", colour.a, |entry, value| entry.radarcol.a = value);
                    let (swatch, _) = ui.allocate_exact_size(egui::Vec2::splat(RADAR_SWATCH_SIDE), egui::Sense::hover());
                    ui.painter().rect_filled(swatch, 0.0, egui::Color32::from_rgb(colour.r, colour.g, colour.b));
                });
                ui.end_row();
            });
            fields.toggle(ui, "Has world art", "has_world_art", entry.has_world_art, |entry, value| entry.has_world_art = value);
            fields.toggle(ui, "Enhanced tileart", "uses_enhanced_tileart", entry.uses_enhanced_tileart, |entry, value| entry.uses_enhanced_tileart = value);
            fields.toggle(ui, "Alternate animation", "alternate_animation", entry.alternate_animation, |entry, value| entry.alternate_animation = value);
        });
        group(columns, "EC image", |ui| fields.window(ui, "ec_window", &entry.ec_img_offset, |entry| &mut entry.ec_img_offset));
        group(columns, "CC image", |ui| fields.window(ui, "cc_window", &entry.cc_img_offset, |entry| &mut entry.cc_img_offset));
        for (label, field, flags, flags2) in [("Flags 1", "flags1", entry.flags1, false), ("Flags 2", "flags2", entry.flags2, true)] {
            group(columns, label, |ui| fields.flags(ui, field, flags, flags2));
        }
        for (label, field, props, second) in
            [("Properties 1", "props1", &entry.prop_vector1, false), ("Properties 2", "props2", &entry.prop_vector2, true)]
        {
            group(columns, label, |ui| fields.props(ui, field, props, second));
        }
        group(columns, "Sitting", |ui| fields.sitting(ui, entry.sitting.as_ref()));
        group(columns, "Stack aliases", |ui| fields.stack_aliases(ui, &entry.stack_alias_vector));
        group(columns, "Appearance filters", |ui| fields.appearance(ui, &entry.appearance_vector));
    });
    columns_at_least(ui, editable, WIDE_GROUP_MIN_WIDTH, |columns| {
        group(columns, "Texture blocks", |ui| fields.texture_blocks(ui, entry, strings));
        group(columns, "Effects", |ui| fields.effects(ui, &entry.effects, strings, effects_draft));
    });
}

/// Pushes the edits of one record.
struct Fields<'a> {
    hash: u64,
    /// Whether rows can be added and deleted: those buttons are left out of a read-only record.
    editable: bool,
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
        set: impl FnOnce(&mut TileArtEntry, T) + 'static,
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
        self.inline_float(ui, field, current, set);
        ui.end_row();
    }

    fn inline_float(&mut self, ui: &mut egui::Ui, field: TileArtField, current: f32, set: impl FnOnce(&mut TileArtEntry, f32) + 'static) {
        let mut value = current;
        let response = ui.add(egui::DragValue::new(&mut value).speed(FLOAT_DRAG_SPEED).min_decimals(FLOAT_SHOWN_DECIMALS));
        self.session(&response);
        if response.changed() {
            self.push(field, move |entry| set(entry, value));
        }
    }

    fn toggle(&mut self, ui: &mut egui::Ui, label: &str, field: TileArtField, current: bool, set: fn(&mut TileArtEntry, bool)) {
        let mut value = current;
        if ui.checkbox(&mut value, label).changed() {
            self.push(field, move |entry| set(entry, value));
        }
    }

    /// An image window: its six stored values, each its own field, and the sizes they give, as UOETE shows them.
    fn window(&mut self, ui: &mut egui::Ui, field: TileArtField, current: &TaeImgOffset, window_of: fn(&mut TileArtEntry) -> &mut TaeImgOffset) {
        let mut copy = current.clone();
        egui::Grid::new((field, self.hash)).num_columns(4).striped(true).show(ui, |ui| {
            for (index, (label, slot)) in WINDOW_SLOTS.into_iter().enumerate() {
                let mut value = *slot(&mut copy);
                ui.label(label);
                let response = ui.add(egui::DragValue::new(&mut value).speed(VALUE_DRAG_SPEED));
                self.session(&response);
                if response.changed() {
                    self.push(field, move |entry| *slot(window_of(entry)) = value);
                }
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
            let (width, height) = (current.x_end.wrapping_sub(current.x_start), current.y_end.wrapping_sub(current.y_start));
            for (label, value) in [
                ("Width", width),
                ("Height", height),
                ("Width + x offset", width.wrapping_add(current.x_off)),
                ("Height + y offset", height.wrapping_add(current.y_off)),
            ] {
                ui.label(label);
                ui.label(value.to_string());
                if label.starts_with("Height") {
                    ui.end_row();
                }
            }
        });
    }

    /// A 64-bit flag field: the hex value (applied when the field loses focus) and a checkbox per named bit.
    fn flags(&mut self, ui: &mut egui::Ui, field: TileArtField, current: TaeFlag, flags2: bool) {
        let set = move |entry: &mut TileArtEntry, value: TaeFlag| {
            if flags2 {
                entry.flags2 = value;
            } else {
                entry.flags1 = value;
            }
        };
        let buffer_id = ui.make_persistent_id((field, self.hash, "hex"));
        let mut text = ui
            .data_mut(|data| data.get_temp::<String>(buffer_id))
            .unwrap_or_else(|| format!("{:0FLAG_HEX_DIGITS$X}", current.bits()));
        let response = ui.horizontal(|ui| {
            ui.label("Val:");
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
        let flag_columns = ((ui.available_width() / FLAG_CELL_WIDTH) as usize).max(1);
        egui::Grid::new((field, self.hash, "bits")).num_columns(flag_columns).show(ui, |ui| {
            let single_bits = TaeFlag::all().iter_names().filter(|(_, flag)| flag.bits().is_power_of_two());
            for (column, (name, flag)) in single_bits.enumerate() {
                let mut set_bit = current.contains(flag);
                if ui.checkbox(&mut set_bit, name).changed() {
                    let mut value = current;
                    value.set(flag, set_bit);
                    self.push(field, move |entry| set(entry, value));
                }
                if (column + 1) % flag_columns == 0 {
                    ui.end_row();
                }
            }
        });
    }

    /// A property list: key and value per row, add and delete.
    fn props(&mut self, ui: &mut egui::Ui, field: TileArtField, props: &[TaeProp], second: bool) {
        egui::Grid::new((field, self.hash, "rows")).striped(true).show(ui, |ui| {
            if !props.is_empty() {
                header_row(ui, &PROPERTY_COLUMNS, self.editable);
            }
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
                if self.editable && ui.small_button("Delete").clicked() {
                    self.push(field, move |entry| {
                        prop_list(entry, second).remove(index);
                    });
                }
                ui.end_row();
            }
        });
        if props.is_empty() {
            ui.weak("none");
        }
        if self.editable && ui.small_button("Add property").clicked() {
            self.push(field, move |entry| prop_list(entry, second).push(TaeProp::default()));
        }
    }

    /// The sitting block: present or not, and its words.
    fn sitting(&mut self, ui: &mut egui::Ui, sitting: Option<&TaeSittingAnimation>) {
        let field = "sitting";
        let mut present = sitting.is_some();
        if ui.checkbox(&mut present, "Sitting block").changed() {
            self.push(field, move |entry| entry.sitting = present.then(TaeSittingAnimation::default));
        }
        let Some(sitting) = sitting else {
            return;
        };
        egui::Grid::new((field, self.hash)).num_columns(2).striped(true).show(ui, |ui| {
            ui.label("direction_mask");
            let mut mask = sitting.direction_mask;
            if ui.add(egui::DragValue::new(&mut mask).speed(VALUE_DRAG_SPEED)).changed() {
                self.push(field, move |entry| {
                    if let Some(sitting) = entry.sitting.as_mut() {
                        sitting.direction_mask = mask;
                    }
                });
            }
            ui.end_row();
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
                ui.end_row();
            }
        });
    }

    /// The stack aliases: amount and amount id per row, add and delete.
    fn stack_aliases(&mut self, ui: &mut egui::Ui, aliases: &[TaeStackAlias]) {
        let field = "stack_aliases";
        egui::Grid::new((field, self.hash)).striped(true).show(ui, |ui| {
            if !aliases.is_empty() {
                header_row(ui, &STACK_ALIAS_COLUMNS, self.editable);
            }
            for (index, alias) in aliases.iter().enumerate() {
                let (mut amount, mut amount_id) = (alias.amount, alias.amount_id);
                let amount_changed = ui.add(egui::DragValue::new(&mut amount).speed(VALUE_DRAG_SPEED)).changed();
                let amount_id_changed = ui.add(egui::DragValue::new(&mut amount_id).speed(VALUE_DRAG_SPEED)).changed();
                if amount_changed || amount_id_changed {
                    self.push(field, move |entry| {
                        if let Some(alias) = entry.stack_alias_vector.get_mut(index) {
                            *alias = TaeStackAlias { amount, amount_id };
                        }
                    });
                }
                if self.editable && ui.small_button("Delete").clicked() {
                    self.push(field, move |entry| {
                        entry.stack_alias_vector.remove(index);
                    });
                }
                ui.end_row();
            }
        });
        if aliases.is_empty() {
            ui.weak("none");
        }
        if self.editable && ui.small_button("Add stack alias").clicked() {
            self.push(field, |entry| entry.stack_alias_vector.push(TaeStackAlias::default()));
        }
    }

    fn appearance(&mut self, ui: &mut egui::Ui, filters: &[TaeAppearanceFilter]) {
        let field = "appearance";
        for (index, filter) in filters.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{index}: {}", appearance_filter_name(filter)));
                if self.editable && ui.small_button("Delete").clicked() {
                    self.push(field, move |entry| {
                        entry.appearance_vector.remove(index);
                    });
                }
            });
            match filter {
                TaeAppearanceFilter::ByOwnerGender { gender, appearance } => {
                    egui::Grid::new(("tileart_gender_appearance", self.hash, index)).striped(true).show(ui, |ui| {
                        header_row(ui, &GENDER_APPEARANCE_COLUMNS, false);
                        self.inline_number(ui, field, *gender, move |entry, value| {
                            if let Some(TaeAppearanceFilter::ByOwnerGender { gender, .. }) = entry.appearance_vector.get_mut(index) {
                                *gender = value;
                            }
                        });
                        self.inline_number(ui, field, *appearance, move |entry, value| {
                            if let Some(TaeAppearanceFilter::ByOwnerGender { appearance, .. }) = entry.appearance_vector.get_mut(index) {
                                *appearance = value;
                            }
                        });
                        ui.end_row();
                    });
                }
                TaeAppearanceFilter::ByOwnerBodyType(mappings) | TaeAppearanceFilter::AnimationByOwnerBodyType(mappings) => {
                    self.body_appearances(ui, index, mappings);
                }
            }
        }
        if filters.is_empty() {
            ui.weak("none");
        }
        if self.editable {
            ui.horizontal_wrapped(|ui| {
                for (label, new_filter) in NEW_APPEARANCE_FILTERS {
                    if ui.small_button(label).clicked() {
                        self.push(field, move |entry| entry.appearance_vector.push(new_filter()));
                    }
                }
            });
        }
    }

    /// The record's effects as KDL text, one node per effect, read back as a whole by `Apply`.
    fn effects(
        &mut self,
        ui: &mut egui::Ui,
        effects: &[EffectDefinition],
        strings: Option<&UoStringDictionary>,
        draft: &mut Option<EffectsDraft>,
    ) {
        let source = effect_definitions_to_kdl(effects, strings);
        let hash = self.hash;
        if !draft.as_ref().is_some_and(|draft| draft.hash == hash && draft.source == source) {
            *draft = None;
        }
        let draft = draft.get_or_insert_with(|| EffectsDraft { hash, text: source.clone(), source, error: None });
        ui.horizontal_wrapped(|ui| {
            ui.weak("One KDL node per effect.");
            crate::ui::help_mark(ui, crate::ui::effects_guide::EFFECT_KDL_HELP);
        });
        ui.add(egui::TextEdit::multiline(&mut draft.text).code_editor().desired_rows(EFFECTS_TEXT_ROWS).desired_width(f32::INFINITY));
        if let Some(error) = &draft.error {
            ui.colored_label(ERROR_COLOR, error);
        }
        if !self.editable {
            return;
        }
        ui.horizontal(|ui| {
            let changed = draft.text != draft.source;
            if ui.add_enabled(changed, egui::Button::new("Apply")).on_hover_text(EFFECTS_APPLY_HOVER).clicked() {
                match effect_definitions_from_kdl(&draft.text, strings) {
                    Ok(read) if read == effects => {
                        draft.text.clone_from(&draft.source);
                        draft.error = None;
                    }
                    Ok(read) => {
                        draft.error = None;
                        self.edits.push(Box::new(|editor, _| {
                            editor.edits.break_coalescing();
                            None
                        }));
                        self.push("effects", move |entry| entry.effects = read);
                    }
                    Err(error) => draft.error = Some(format!("{error:#}")),
                }
            }
            if ui.add_enabled(changed, egui::Button::new("Reset text")).clicked() {
                draft.text.clone_from(&draft.source);
                draft.error = None;
            }
        });
    }

    fn body_appearances(&mut self, ui: &mut egui::Ui, filter: usize, mappings: &[TaeBodyAppearance]) {
        let field = "appearance";
        egui::Grid::new(("tileart_body_appearances", self.hash, filter)).striped(true).show(ui, |ui| {
            if !mappings.is_empty() {
                header_row(ui, &BODY_APPEARANCE_COLUMNS, self.editable);
            }
            for (row, mapping) in mappings.iter().enumerate() {
                self.inline_number(ui, field, mapping.body_type, move |entry, value| {
                    if let Some(mapping) = body_appearances(entry, filter).and_then(|mappings| mappings.get_mut(row)) {
                        mapping.body_type = value;
                    }
                });
                self.inline_number(ui, field, mapping.appearance, move |entry, value| {
                    if let Some(mapping) = body_appearances(entry, filter).and_then(|mappings| mappings.get_mut(row)) {
                        mapping.appearance = value;
                    }
                });
                if self.editable && ui.small_button("Delete").clicked() {
                    self.push(field, move |entry| {
                        if let Some(mappings) = body_appearances(entry, filter) {
                            mappings.remove(row);
                        }
                    });
                }
                ui.end_row();
            }
        });
        if self.editable && ui.small_button("Add body").clicked() {
            self.push(field, move |entry| {
                if let Some(mappings) = body_appearances(entry, filter) {
                    mappings.push(TaeBodyAppearance::default());
                }
            });
        }
    }

    fn texture_blocks(&mut self, ui: &mut egui::Ui, entry: &TileArtEntry, strings: Option<&UoStringDictionary>) {
        let field = "texture_blocks";
        for (index, block) in entry.texture_vector.iter().enumerate() {
            if entry.version == TILEART_KR_RECORD_VERSION && index == TILEART_SHADER_BLOCK_ENHANCED {
                continue;
            }
            let mut present = block.has_texture == TILEART_SHADER_BLOCK_PRESENT;
            let name = TEXTURE_BLOCK_NAMES.get(index).copied().unwrap_or_default();
            if ui.checkbox(&mut present, format!("Block {index}: {name}")).changed() {
                self.push(field, move |entry| {
                    if let Some(block) = entry.texture_vector.get_mut(index) {
                        *block = present_block(present);
                    }
                });
            }
            if present {
                self.texture_block(ui, index, block, strings);
            }
        }
    }

    fn texture_block(&mut self, ui: &mut egui::Ui, index: usize, block: &TaeTexture, strings: Option<&UoStringDictionary>) {
        let field = "texture_blocks";
        let text_of = |offset: u32| strings.and_then(|strings| strings.get_string(offset.checked_sub(1)? as usize));
        ui.horizontal_wrapped(|ui| {
            ui.label("Shader name").on_hover_text(STRING_INDEX_HOVER);
            self.inline_number(ui, field, block.type_string_off, move |entry, value| {
                if let Some(block) = entry.texture_vector.get_mut(index) {
                    block.type_string_off = value;
                }
            });
            ui.weak(text_of(block.type_string_off).unwrap_or("-"));
            ui.label("Implementation").on_hover_text(SHADER_IMPLEMENTATION_HOVER);
            self.inline_number(ui, field, block.field_04, move |entry, value| {
                if let Some(block) = entry.texture_vector.get_mut(index) {
                    block.field_04 = value;
                }
            });
        });
        egui::ScrollArea::horizontal().id_salt(("tileart_texture_items_scroll", self.hash, index)).show(ui, |ui| {
            egui::Grid::new(("tileart_texture_items", self.hash, index)).striped(true).show(ui, |ui| {
                if !block.texture_items.is_empty() {
                    header_row(ui, &TEXTURE_ITEM_COLUMNS, self.editable);
                }
                for (item_index, item) in block.texture_items.iter().enumerate() {
                    ui.label(item_index.to_string());
                    self.inline_number(ui, field, item.name_string_off, move |entry, value| {
                        if let Some(item) = texture_item(entry, index, item_index) {
                            item.name_string_off = value;
                        }
                    });
                    match text_of(item.name_string_off) {
                        Some(path) => {
                            ui.label(path_file_name(path)).on_hover_text(path);
                        }
                        None => {
                            ui.weak("-");
                        }
                    }
                    self.inline_number(ui, field, item.source_kind, move |entry, value| {
                        if let Some(item) = texture_item(entry, index, item_index) {
                            item.source_kind = value;
                        }
                    });
                    self.inline_float(ui, field, item.texture_stretch, move |entry, value| {
                        if let Some(item) = texture_item(entry, index, item_index) {
                            item.texture_stretch = value;
                        }
                    });
                    self.inline_number(ui, field, item.sampler_index, move |entry, value| {
                        if let Some(item) = texture_item(entry, index, item_index) {
                            item.sampler_index = value;
                        }
                    });
                    self.inline_number(ui, field, item.uv_set, move |entry, value| {
                        if let Some(item) = texture_item(entry, index, item_index) {
                            item.uv_set = value;
                        }
                    });
                    if self.editable && ui.small_button("Delete").clicked() {
                        self.push(field, move |entry| {
                            if let Some(block) = entry.texture_vector.get_mut(index) {
                                block.texture_items.remove(item_index);
                            }
                        });
                    }
                    ui.end_row();
                }
            });
        });
        if self.editable && ui.small_button("Add item").clicked() {
            self.push(field, move |entry| {
                if let Some(block) = entry.texture_vector.get_mut(index) {
                    block.texture_items.push(new_texture_item(block.texture_items.len()));
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Technique").on_hover_text(VALUE_LISTS_HOVER);
            for (position, value) in block.technique.iter().enumerate() {
                self.inline_number(ui, field, *value, move |entry, value| {
                    if let Some(slot) = entry.texture_vector.get_mut(index).and_then(|block| block.technique.get_mut(position)) {
                        *slot = value;
                    }
                });
            }
            if self.editable && ui.small_button("+").clicked() {
                self.push(field, move |entry| {
                    if let Some(block) = entry.texture_vector.get_mut(index) {
                        block.technique.push(0);
                    }
                });
            }
            if self.editable && !block.technique.is_empty() && ui.small_button("-").clicked() {
                self.push(field, move |entry| {
                    if let Some(block) = entry.texture_vector.get_mut(index) {
                        block.technique.pop();
                    }
                });
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Tweaks").on_hover_text(VALUE_LISTS_HOVER);
            for (position, value) in block.tweaks.iter().enumerate() {
                self.inline_float(ui, field, *value, move |entry, value| {
                    if let Some(slot) = entry.texture_vector.get_mut(index).and_then(|block| block.tweaks.get_mut(position)) {
                        *slot = value;
                    }
                });
            }
            if self.editable && ui.small_button("+").clicked() {
                self.push(field, move |entry| {
                    if let Some(block) = entry.texture_vector.get_mut(index) {
                        block.tweaks.push(0.0);
                    }
                });
            }
            if self.editable && !block.tweaks.is_empty() && ui.small_button("-").clicked() {
                self.push(field, move |entry| {
                    if let Some(block) = entry.texture_vector.get_mut(index) {
                        block.tweaks.pop();
                    }
                });
            }
        });
        ui.separator();
    }
}

/// The header row of a sub-record table: its column titles, each with its hover text when it has one.
/// The titles of `columns`, then of the rows' Delete buttons when they have them.
fn header_row(ui: &mut egui::Ui, columns: &[(&str, &str)], deletable: bool) {
    for (title, hover) in columns {
        let label = ui.strong(*title);
        if !hover.is_empty() {
            label.on_hover_text(*hover);
        }
    }
    if deletable {
        ui.strong(DELETE_COLUMN);
    }
    ui.end_row();
}

/// The last part of a texture path (`Build\TileArtLegacy\00001234.tga`): the full path is the hover text.
fn path_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn appearance_filter_name(filter: &TaeAppearanceFilter) -> &'static str {
    match filter {
        TaeAppearanceFilter::ByOwnerBodyType(_) => "appearance by owner body type",
        TaeAppearanceFilter::ByOwnerGender { .. } => "appearance by owner gender",
        TaeAppearanceFilter::AnimationByOwnerBodyType(_) => "animation appearance by owner body type",
    }
}

fn body_appearances(entry: &mut TileArtEntry, filter: usize) -> Option<&mut Vec<TaeBodyAppearance>> {
    match entry.appearance_vector.get_mut(filter)? {
        TaeAppearanceFilter::ByOwnerBodyType(mappings) | TaeAppearanceFilter::AnimationByOwnerBodyType(mappings) => Some(mappings),
        TaeAppearanceFilter::ByOwnerGender { .. } => None,
    }
}

fn texture_item(entry: &mut TileArtEntry, block: usize, item: usize) -> Option<&mut TaeTextureImage> {
    entry.texture_vector.get_mut(block)?.texture_items.get_mut(item)
}

/// A block as the checkbox leaves it: without data either way, and marked present when asked.
fn present_block(present: bool) -> TaeTexture {
    if present {
        TaeTexture { has_texture: TILEART_SHADER_BLOCK_PRESENT, ..TaeTexture::default() }
    } else {
        TaeTexture::default()
    }
}

/// A new item at `position`: it names no texture until its name is set.
fn new_texture_item(position: usize) -> TaeTextureImage {
    TaeTextureImage { texture_stretch: NEW_ITEM_STRETCH, sampler_index: position as u32, ..TaeTextureImage::default() }
}

fn prop_list(entry: &mut TileArtEntry, second: bool) -> &mut Vec<TaeProp> {
    if second {
        &mut entry.prop_vector2
    } else {
        &mut entry.prop_vector1
    }
}

/// Applies the collected edits to the loaded records and moves the selection.
pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::TileMetadata,
    label: "the tileart.uop records",
    is_dirty: |app| app.tileart_editor.edits.is_dirty(),
    save: |app| {
        if let (Some(source), Some(output)) = (app.ec_tileart_path.clone(), save_path()) {
            save(app, &source, &output);
        }
    },
    discard: |app| {
        let (Some(entries), Some(rows)) = (app.ec_tileart_entries.as_mut(), app.ec_tileart_rows.as_mut()) else {
            return;
        };
        let mut store = TileArtStore {
            entries: Arc::make_mut(entries),
            rows: Arc::make_mut(rows),
            dictionary: app.uo_string_dictionary.as_deref(),
        };
        app.tileart_editor.edits.revert_to_opened(&mut store);
        app.tileart_editor.edits.mark_saved();
    },
};

pub(super) fn apply_edits(app: &mut UocfEditorApp, edits: Vec<TileArtEdit>) {
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

/// Asks where to write the copy, starting outside the client folder (in the home folder).
fn save_path() -> Option<PathBuf> {
    let (name, extensions) = UOP_FILTER;
    let mut dialog = crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(TILEART_UOP_NAME);
    if let Some(home) = std::env::home_dir() {
        dialog = dialog.set_directory(home);
    }
    dialog.save_file()
}

fn save(app: &mut UocfEditorApp, source: &Path, output: &Path) {
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
