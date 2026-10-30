//! The NIF part of the Effects tab: the scene tree and block list of a `.nif` file of `EffectTexture.uop`, a
//! property grid over gamebryo-nif's reflection (scalars, enums and bit fields, colours, vectors, strings,
//! links, lists, key tables with curves), validation, import and export. The editing logic lives in
//! `logic::nif_editor`; every edit goes through the effects document (undo, preview refresh, save).

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use gamebryo_nif::reflect::{EnumKind, ListRef, StructRef};
use gamebryo_nif::{Field, LinkKind, NifEnum, NifFile, Value, NULL_LINK};

use crate::logic::effect_files::{read_import, EffectFiles};
use crate::logic::effects_editor::EffectsDocument;
use crate::logic::nif_editor::{
    block_label, current_nif, edit_nif_file, is_key_list, key_curves, link_candidates, ref_children, replace_nif_file,
    scene_roots, unreachable_blocks, write_nif, FieldStep, KeyCurve, NifEdit, PathEdit, ScalarValue,
};

const BLOCK_LIST_HEIGHT: f32 = 220.0;
const PROPERTY_GRID_HEIGHT: f32 = 480.0;
/// Elements of a list shown (vertex arrays can be long).
const MAX_LIST_ROWS: usize = 256;
/// Scene tree levels open by default.
const TREE_OPEN_DEPTH: usize = 2;
const INT_DRAG_SPEED: f64 = 0.1;
const FLOAT_DRAG_SPEED: f64 = 0.01;
/// Structs of at most this many floats are edited on one row (points, colours, quaternions).
const INLINE_FLOATS: usize = 4;
/// A 3x3 matrix: nine floats in rows of three.
const MATRIX_FLOATS: usize = 9;
const MATRIX_COLUMNS: usize = 3;
/// Width of a link combo box.
const LINK_COMBO_WIDTH: f32 = 220.0;
const CURVE_HEIGHT: f32 = 100.0;
const CURVE_STROKE_WIDTH: f32 = 1.5;
const CURVE_POINT_RADIUS: f32 = 2.5;
const CURVE_FRAME_COLOR: egui::Color32 = egui::Color32::from_gray(90);
/// Curve colours by component (r, g, b, a; a single value uses the first).
const CURVE_COLORS: [egui::Color32; 4] = [
    egui::Color32::from_rgb(230, 90, 90),
    egui::Color32::from_rgb(110, 200, 110),
    egui::Color32::from_rgb(100, 140, 240),
    egui::Color32::from_gray(200),
];
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const NIF_FILTER: (&str, &[&str]) = ("NIF scene", &["nif"]);
/// Base types offered by the block type filter (with every type of the file).
const BASE_TYPE_FILTERS: [&str; 6] =
    ["NiAVObject", "NiProperty", "NiTimeController", "NiInterpolator", "NiPSysModifier", "NiPSysEmitter"];
/// Compound types edited with a colour picker: (type, components).
const COLOR_TYPES: [(&str, usize); 2] = [("NiColor", 3), ("NiColorA", 4)];
const DUPLICATE_ICON: &str = "⧉";
const REMOVE_ICON: &str = "✖";
const GOTO_ICON: &str = "→";
const NONE_LINK_LABEL: &str = "None";

/// What the NIF editor shows.
#[derive(Default)]
pub struct NifEditorState {
    cache: Option<NifCache>,
    selected_block: usize,
    view: NifListView,
    /// A block type (or base type) the block list is limited to.
    type_filter: Option<&'static str>,
    /// Show the fields absent with the current values (conditions not met).
    show_absent: bool,
    /// The outcome of the last edit (count fields updated or not).
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum NifListView {
    #[default]
    Tree,
    Blocks,
}

/// The file shown, parsed, with what is derived from it.
struct NifCache {
    /// (file hash, files generation, document revision).
    key: (u64, u64, u64),
    nif: Result<Arc<NifFile>, String>,
    compile_warnings: Vec<String>,
    link_issues: Vec<String>,
    unreachable: Vec<usize>,
    /// Every block type of the file, sorted.
    types: Vec<&'static str>,
}

impl NifEditorState {
    /// The parsed file shown (tests).
    #[cfg(test)]
    pub fn shown(&self) -> Option<&Arc<NifFile>> {
        self.cache.as_ref().and_then(|cache| cache.nif.as_ref().ok())
    }

    /// Shows the block list (or the tree), absent fields and block `block` (tests).
    #[cfg(test)]
    pub fn show_block(&mut self, list: bool, block: usize) {
        self.view = if list { NifListView::Blocks } else { NifListView::Tree };
        self.show_absent = true;
        self.selected_block = block;
    }
}

/// What the property grid collected in one frame.
struct GridContext<'a> {
    nif: &'a NifFile,
    show_absent: bool,
    edits: Vec<PathEdit>,
    /// A block to select (a link followed).
    goto: Option<usize>,
}

impl GridContext<'_> {
    fn set(&mut self, path: &[FieldStep], value: ScalarValue) {
        self.edits.push((path.to_vec(), NifEdit::Set(value)));
    }
}

/// The NIF editor of file `hash`; returns a status message or an error for the panel.
pub fn nif_editor(
    ui: &mut egui::Ui,
    editor: &mut NifEditorState,
    document: &mut EffectsDocument,
    files: &EffectFiles,
    files_generation: u64,
    hash: u64,
    name: Option<&str>,
) -> Option<Result<String, String>> {
    let key = (hash, files_generation, document.revision());
    if editor.cache.as_ref().is_none_or(|cache| cache.key != key) {
        if editor.cache.as_ref().is_none_or(|cache| cache.key.0 != hash) {
            editor.selected_block = 0;
            editor.message = None;
        }
        editor.cache = Some(build_cache(key, document, files, name));
    }
    let cache = editor.cache.as_ref()?;
    let nif = match &cache.nif {
        Ok(nif) => Arc::clone(nif),
        Err(error) => {
            ui.colored_label(ERROR_COLOR, format!("Does not parse: {error}"));
            return import_export_buttons(ui, document, hash, name, None);
        }
    };
    let mut result = import_export_buttons(ui, document, hash, name, Some(&nif));
    ui.label(format!(
        "{}, {} blocks, {} root(s).",
        nif.header.header_string.to_string_lossy(),
        nif.blocks.len(),
        nif.roots.len()
    ));
    if let Some(draft) = document.nif_drafts().get(&hash) {
        ui.colored_label(
            ERROR_COLOR,
            format!(
                "Does not write: {}. The preview plays the edit; saving EffectTexture.uop is refused until an edit \
                 (or undo) makes the lengths agree.",
                draft.error
            ),
        );
    }
    for issue in &cache.link_issues {
        ui.colored_label(WARNING_COLOR, issue);
    }
    if !cache.compile_warnings.is_empty() {
        ui.collapsing(format!("Compile warnings ({})", cache.compile_warnings.len()), |ui| {
            for warning in &cache.compile_warnings {
                ui.colored_label(WARNING_COLOR, warning);
            }
        });
    }
    if let Some(message) = &editor.message {
        ui.weak(message);
    }
    ui.separator();

    // Block browser.
    ui.horizontal(|ui| {
        ui.selectable_value(&mut editor.view, NifListView::Tree, "Scene tree");
        ui.selectable_value(&mut editor.view, NifListView::Blocks, "Blocks");
        if editor.view == NifListView::Blocks {
            let label = editor.type_filter.unwrap_or("all types");
            egui::ComboBox::from_id_salt("effects_nif_type_filter").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut editor.type_filter, None, "all types");
                let present = BASE_TYPE_FILTERS.iter().filter(|base| nif.blocks.iter().any(|block| block.block_type().is_a(base)));
                for &type_name in present.chain(cache.types.iter()) {
                    ui.selectable_value(&mut editor.type_filter, Some(type_name), type_name);
                }
            });
        }
    });
    editor.selected_block = editor.selected_block.min(nif.blocks.len().saturating_sub(1));
    egui::ScrollArea::vertical().id_salt("effects_nif_blocks").max_height(BLOCK_LIST_HEIGHT).show(ui, |ui| {
        match editor.view {
            NifListView::Tree => {
                let mut shown = HashSet::new();
                for root in scene_roots(&nif) {
                    tree_node(ui, &nif, root, 0, &mut shown, &mut editor.selected_block);
                }
                if !cache.unreachable.is_empty() {
                    ui.collapsing(format!("Not in the tree ({})", cache.unreachable.len()), |ui| {
                        for &block in &cache.unreachable {
                            if ui.selectable_label(editor.selected_block == block, block_label(&nif, block)).clicked() {
                                editor.selected_block = block;
                            }
                        }
                    });
                }
            }
            NifListView::Blocks => {
                for (index, block) in nif.blocks.iter().enumerate() {
                    if editor.type_filter.is_some_and(|filter| !block.block_type().is_a(filter)) {
                        continue;
                    }
                    if ui.selectable_label(editor.selected_block == index, block_label(&nif, index)).clicked() {
                        editor.selected_block = index;
                    }
                }
            }
        }
    });
    ui.separator();

    // Property grid.
    let block = editor.selected_block;
    let Some(selected) = nif.blocks.get(block) else {
        return result;
    };
    ui.horizontal(|ui| {
        ui.strong(block_label(&nif, block));
        ui.checkbox(&mut editor.show_absent, "absent fields").on_hover_text(
            "Show the fields whose condition does not hold with the current values (not part of the file).",
        );
    });
    let mut grid = GridContext { nif: &nif, show_absent: editor.show_absent, edits: Vec::new(), goto: None };
    egui::ScrollArea::vertical().id_salt("effects_nif_fields").max_height(PROPERTY_GRID_HEIGHT).show(ui, |ui| {
        let id = ui.make_persistent_id(("effects_nif_block", hash, block));
        fields_ui(ui, &mut grid, &selected.fields(), &mut Vec::new(), id);
    });
    let GridContext { edits, goto, .. } = grid;
    if let Some(target) = goto {
        editor.selected_block = target;
    }
    if !edits.is_empty() {
        match edit_nif_file(document, files, hash, block, &edits) {
            Ok(outcomes) => {
                let notes: Vec<String> = outcomes
                    .iter()
                    .flat_map(|outcome| {
                        let updated = outcome.count_updated.as_ref().map(|(field, length)| format!("{field} set to {length}"));
                        let left = outcome
                            .length_not_updated
                            .as_ref()
                            .map(|expression| format!("length {expression} not updated: check it before saving"));
                        updated.into_iter().chain(left)
                    })
                    .collect();
                editor.message = (!notes.is_empty()).then(|| notes.join("; "));
            }
            Err(error) => result = Some(Err(format!("NIF edit refused: {error}"))),
        }
    }
    result
}

fn build_cache(key: (u64, u64, u64), document: &EffectsDocument, files: &EffectFiles, name: Option<&str>) -> NifCache {
    let nif = current_nif(document, files, key.0);
    let (mut compile_warnings, mut link_issues, mut unreachable, mut types) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    if let Ok(nif) = &nif {
        compile_warnings = particle_sim::nif::compile(nif, name.unwrap_or_default()).warnings;
        link_issues = nif
            .link_issues()
            .iter()
            .map(|issue| {
                let place = issue.block.map_or_else(|| "footer".to_owned(), |block| block_label(nif, block));
                let found = issue.found.unwrap_or("nothing (out of range)");
                format!("{place}: {} = {} points at {found}, expected {}", issue.field, issue.raw, issue.expected)
            })
            .collect();
        unreachable = unreachable_blocks(nif);
        types = nif.blocks.iter().map(|block| block.type_name()).collect();
        types.sort_unstable();
        types.dedup();
    }
    NifCache { key, nif, compile_warnings, link_issues, unreachable, types }
}

fn import_export_buttons(
    ui: &mut egui::Ui,
    document: &mut EffectsDocument,
    hash: u64,
    name: Option<&str>,
    nif: Option<&NifFile>,
) -> Option<Result<String, String>> {
    let mut result = None;
    ui.horizontal_wrapped(|ui| {
        let written = nif.filter(|_| !document.nif_drafts().contains_key(&hash)).map(write_nif);
        let export = ui
            .add_enabled(matches!(written, Some(Ok(_))), egui::Button::new("Export .nif..."))
            .on_disabled_hover_text("The file does not parse or does not write.");
        if export.clicked() {
            let default_name = name.map_or_else(|| format!("{hash:016x}.nif"), str::to_owned);
            if let (Some(Ok(bytes)), Some(path)) =
                (written, crate::dialog::file_dialog().set_file_name(&default_name).save_file())
            {
                result = Some(
                    std::fs::write(&path, bytes)
                        .map(|()| format!("Exported {}.", path.display()))
                        .map_err(|error| format!("Export failed: {}: {error}", path.display())),
                );
            }
        }
        let import = ui
            .add_enabled(name.is_some(), egui::Button::new("Import .nif..."))
            .on_hover_text("Replaces this file by a .nif from disk (it must parse).")
            .on_disabled_hover_text("The name of this entry is unknown: it cannot be written.");
        if import.clicked() {
            let picked = crate::dialog::file_dialog().add_filter(NIF_FILTER.0, NIF_FILTER.1).pick_file();
            if let (Some(name), Some(path)) = (name, picked) {
                result = Some(match read_import(&path) {
                    Ok((_, payload)) if crate::logic::effect_files::is_nif_name(&path.to_string_lossy()) => {
                        replace_nif_file(document, hash, name.to_owned(), payload);
                        Ok(format!("Imported {} as {name}.", path.display()))
                    }
                    Ok(_) => Err(format!("Import failed: {} is not a .nif file", path.display())),
                    Err(error) => Err(format!("Import failed: {error}")),
                });
            }
        }
    });
    result
}

fn tree_node(ui: &mut egui::Ui, nif: &NifFile, block: usize, depth: usize, shown: &mut HashSet<usize>, selected: &mut usize) {
    let first = shown.insert(block);
    let children = if first { ref_children(nif, block) } else { Vec::new() };
    let label = if first { block_label(nif, block) } else { format!("{} (shown above)", block_label(nif, block)) };
    if children.is_empty() {
        if ui.selectable_label(*selected == block, label).clicked() {
            *selected = block;
        }
        return;
    }
    let id = ui.make_persistent_id(("effects_nif_tree", block, depth));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < TREE_OPEN_DEPTH)
        .show_header(ui, |ui| {
            if ui.selectable_label(*selected == block, label).clicked() {
                *selected = block;
            }
        })
        .body(|ui| {
            for child in children {
                tree_node(ui, nif, child, depth + 1, shown, selected);
            }
        });
}

/// Hover text of a field: schema type, declaring type, length and condition.
fn field_hover(field: &Field<'_>) -> String {
    let info = field.info;
    let mut text = format!("{}: {} (in {})", info.name, info.nif_type, info.declared_in);
    if let Some(length) = info.length {
        text.push_str(&format!("\nlength: {length}"));
    }
    if let Some(condition) = info.condition {
        text.push_str(&format!("\nif {condition}{}", if field.present { "" } else { " (false now: absent)" }));
    }
    text
}

fn fields_ui(ui: &mut egui::Ui, cx: &mut GridContext<'_>, fields: &[Field<'_>], path: &mut Vec<FieldStep>, id: egui::Id) {
    for (index, field) in fields.iter().enumerate() {
        if !field.present && !cx.show_absent {
            continue;
        }
        path.push(FieldStep::Field(index));
        let hover = field_hover(field);
        ui.add_enabled_ui(field.present, |ui| {
            value_ui(ui, cx, field.name(), &hover, field.value, path, id.with(index));
        });
        path.pop();
    }
}

fn value_ui(
    ui: &mut egui::Ui,
    cx: &mut GridContext<'_>,
    label: &str,
    hover: &str,
    value: Value<'_>,
    path: &mut Vec<FieldStep>,
    id: egui::Id,
) {
    match value {
        Value::Struct(nested) => struct_ui(ui, cx, label, hover, nested, path, id),
        Value::List(list) => list_ui(ui, cx, label, hover, list, path, id),
        scalar => {
            ui.horizontal(|ui| {
                ui.label(label).on_hover_text(hover);
                scalar_ui(ui, cx, scalar, path, id);
            });
        }
    }
}

/// The floats of a struct whose present fields are all floats.
fn struct_floats(nested: &StructRef<'_>) -> Option<Vec<(usize, &'static str, f32)>> {
    let fields = nested.fields();
    fields
        .iter()
        .enumerate()
        .filter(|(_, field)| field.present)
        .map(|(index, field)| match field.value {
            Value::F32(&value) => Some((index, field.name(), value)),
            _ => None,
        })
        .collect()
}

fn struct_ui(
    ui: &mut egui::Ui,
    cx: &mut GridContext<'_>,
    label: &str,
    hover: &str,
    nested: StructRef<'_>,
    path: &mut Vec<FieldStep>,
    id: egui::Id,
) {
    let floats = struct_floats(&nested);
    let color = COLOR_TYPES.iter().find(|(name, _)| *name == nested.type_name()).map(|&(_, components)| components);
    match floats {
        Some(floats) if floats.len() <= INLINE_FLOATS || color.is_some() => {
            ui.horizontal(|ui| {
                ui.label(label).on_hover_text(format!("{hover}\n{}", nested.type_name()));
                inline_floats(ui, cx, &floats, color, path);
            });
        }
        Some(floats) if floats.len() == MATRIX_FLOATS => {
            ui.label(label).on_hover_text(format!("{hover}\n{}", nested.type_name()));
            ui.indent(id, |ui| {
                for row in floats.chunks(MATRIX_COLUMNS) {
                    ui.horizontal(|ui| inline_floats(ui, cx, row, None, path));
                }
            });
        }
        _ => {
            egui::CollapsingHeader::new(format!("{label} ({})", nested.type_name())).id_salt(id).show(ui, |ui| {
                fields_ui(ui, cx, &nested.fields(), path, id);
            });
        }
    }
}

/// Drag values of struct floats (index in the struct, name, value), with a colour picker for colours.
fn inline_floats(
    ui: &mut egui::Ui,
    cx: &mut GridContext<'_>,
    floats: &[(usize, &'static str, f32)],
    color: Option<usize>,
    path: &mut Vec<FieldStep>,
) {
    if let Some(components) = color.filter(|&components| components == floats.len()) {
        let mut rgba = [1.0f32; 4];
        for (slot, (_, _, value)) in rgba.iter_mut().zip(floats) {
            *slot = *value;
        }
        let changed = if components == COLOR_TYPES[0].1 {
            let mut rgb = [rgba[0], rgba[1], rgba[2]];
            let changed = ui.color_edit_button_rgb(&mut rgb).changed();
            rgba[..rgb.len()].copy_from_slice(&rgb);
            changed
        } else {
            ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed()
        };
        if changed {
            for ((index, _, _), value) in floats.iter().zip(rgba) {
                path.push(FieldStep::Field(*index));
                cx.set(path, ScalarValue::Float(value));
                path.pop();
            }
        }
    }
    for &(index, name, value) in floats {
        let mut edited = value;
        let response = ui.add(egui::DragValue::new(&mut edited).speed(FLOAT_DRAG_SPEED)).on_hover_text(name);
        if response.changed() {
            path.push(FieldStep::Field(index));
            cx.set(path, ScalarValue::Float(edited));
            path.pop();
        }
    }
}

fn int_ui<T: egui::emath::Numeric + Into<i128>>(ui: &mut egui::Ui, cx: &mut GridContext<'_>, path: &[FieldStep], value: T) {
    let mut edited = value;
    if ui.add(egui::DragValue::new(&mut edited).speed(INT_DRAG_SPEED)).changed() {
        cx.set(path, ScalarValue::Integer(edited.into()));
    }
}

fn scalar_ui(ui: &mut egui::Ui, cx: &mut GridContext<'_>, value: Value<'_>, path: &[FieldStep], id: egui::Id) {
    match value {
        Value::U8(&v) => int_ui(ui, cx, path, v),
        Value::I8(&v) => int_ui(ui, cx, path, v),
        Value::U16(&v) => int_ui(ui, cx, path, v),
        Value::I16(&v) => int_ui(ui, cx, path, v),
        Value::U32(&v) => int_ui(ui, cx, path, v),
        Value::I32(&v) => int_ui(ui, cx, path, v),
        Value::U64(&v) => int_ui(ui, cx, path, v),
        Value::F32(&v) => {
            let mut edited = v;
            if ui.add(egui::DragValue::new(&mut edited).speed(FLOAT_DRAG_SPEED)).changed() {
                cx.set(path, ScalarValue::Float(edited));
            }
        }
        Value::Bool(flag) => {
            let mut edited = flag.get();
            if ui.checkbox(&mut edited, "").changed() {
                cx.set(path, ScalarValue::Bool(edited));
            }
        }
        Value::String(text) => {
            let mut edited = text.to_string_lossy();
            if ui.text_edit_singleline(&mut edited).changed() {
                cx.set(path, ScalarValue::String(edited));
            }
        }
        Value::Enum(value) => enum_ui(ui, cx, value, path, id),
        Value::Link { raw, kind, target } => link_ui(ui, cx, raw, kind, target, path, id),
        Value::Struct(_) | Value::List(_) => {}
    }
}

fn enum_ui(ui: &mut egui::Ui, cx: &mut GridContext<'_>, value: &dyn NifEnum, path: &[FieldStep], id: egui::Id) {
    let raw = value.raw();
    match value.kind() {
        EnumKind::Enum(options) => {
            let shown = value.option_name().map_or_else(|| format!("{} (raw {raw})", value.type_name()), str::to_owned);
            egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
                for option in options {
                    if ui.selectable_label(option.value == raw, option.name).clicked() && option.value != raw {
                        cx.set(path, ScalarValue::EnumRaw(option.value));
                    }
                }
            });
        }
        EnumKind::Bitflags(options) => {
            ui.vertical(|ui| {
                ui.weak(format!("{} = {raw:#x}", value.type_name()));
                ui.horizontal_wrapped(|ui| {
                    for option in options.iter().filter(|option| option.value != 0) {
                        let mut on = raw & option.value == option.value;
                        if ui.checkbox(&mut on, option.name).changed() {
                            let edited = if on { raw | option.value } else { raw & !option.value };
                            cx.set(path, ScalarValue::EnumRaw(edited));
                        }
                    }
                });
            });
        }
        EnumKind::Bitfield(members) => {
            ui.vertical(|ui| {
                ui.weak(format!("{} = {raw:#x}", value.type_name()));
                for member in members {
                    let mask = if member.width >= u64::BITS { u64::MAX } else { (1u64 << member.width) - 1 };
                    let current = (raw >> member.pos) & mask;
                    let mut edited = current;
                    ui.horizontal(|ui| {
                        ui.label(member.name).on_hover_text(format!(
                            "{} bits {}..{}",
                            member.nif_type,
                            member.pos,
                            member.pos + member.width
                        ));
                        if member.width == 1 {
                            let mut on = current != 0;
                            if ui.checkbox(&mut on, "").changed() {
                                edited = u64::from(on);
                            }
                        } else {
                            ui.add(egui::DragValue::new(&mut edited).range(0..=mask).speed(INT_DRAG_SPEED));
                        }
                    });
                    if edited != current {
                        let cleared = raw & !(mask << member.pos);
                        cx.set(path, ScalarValue::EnumRaw(cleared | (edited & mask) << member.pos));
                    }
                }
            });
        }
    }
}

fn link_ui(
    ui: &mut egui::Ui,
    cx: &mut GridContext<'_>,
    raw: i32,
    kind: LinkKind,
    target: &'static str,
    path: &[FieldStep],
    id: egui::Id,
) {
    let nif = cx.nif;
    let shown = if raw == NULL_LINK { NONE_LINK_LABEL.to_owned() } else { block_label(nif, raw.max(0) as usize) };
    let kind_label = match kind {
        LinkKind::Ref => "Ref",
        LinkKind::Ptr => "Ptr",
    };
    egui::ComboBox::from_id_salt(id)
        .width(LINK_COMBO_WIDTH)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            if ui.selectable_label(raw == NULL_LINK, NONE_LINK_LABEL).clicked() && raw != NULL_LINK {
                cx.set(path, ScalarValue::Link(NULL_LINK));
            }
            for candidate in link_candidates(nif, target) {
                let Ok(value) = i32::try_from(candidate) else { continue };
                if ui.selectable_label(raw == value, block_label(nif, candidate)).clicked() && raw != value {
                    cx.set(path, ScalarValue::Link(value));
                }
            }
        })
        .response
        .on_hover_text(format!("{kind_label}<{target}>: the blocks of that type are offered"));
    if let Some(index) = usize::try_from(raw).ok().filter(|&index| index < nif.blocks.len()) {
        if ui.small_button(GOTO_ICON).on_hover_text("Select the linked block").clicked() {
            cx.goto = Some(index);
        }
    }
}

fn list_ui(
    ui: &mut egui::Ui,
    cx: &mut GridContext<'_>,
    label: &str,
    hover: &str,
    list: ListRef<'_>,
    path: &mut Vec<FieldStep>,
    id: egui::Id,
) {
    let length = list.len();
    let keys = is_key_list(&list);
    let header = egui::CollapsingHeader::new(format!("{label} [{length}]")).id_salt(id).default_open(keys).show(ui, |ui| {
        if keys {
            key_table(ui, cx, &list, path, id);
            let curves = key_curves(&list);
            if !curves.is_empty() {
                curve_plot(ui, &curves);
            }
        } else {
            for (index, item) in list.iter().enumerate().take(MAX_LIST_ROWS) {
                path.push(FieldStep::Item(index));
                let item_label = format!("[{index}]");
                match item {
                    Value::Struct(_) | Value::List(_) => {
                        value_ui(ui, cx, &item_label, hover, item, path, id.with(index));
                    }
                    scalar => {
                        ui.horizontal(|ui| {
                            ui.label(&item_label);
                            scalar_ui(ui, cx, scalar, path, id.with(index));
                        });
                    }
                }
                path.pop();
                item_buttons(ui, cx, path, index);
            }
            if length > MAX_LIST_ROWS {
                ui.weak(format!("... {} more not shown", length - MAX_LIST_ROWS));
            }
        }
        if ui.small_button("Add").on_hover_text("Appends a default element.").clicked() {
            cx.edits.push((path.clone(), NifEdit::Insert(length)));
        }
    });
    header.header_response.on_hover_text(hover);
}

/// Duplicate and remove buttons of list element `index` (`path` is the list).
fn item_buttons(ui: &mut egui::Ui, cx: &mut GridContext<'_>, path: &[FieldStep], index: usize) {
    ui.horizontal(|ui| {
        if ui.small_button(DUPLICATE_ICON).on_hover_text(format!("Duplicate [{index}]")).clicked() {
            cx.edits.push((path.to_vec(), NifEdit::Duplicate(index)));
        }
        if ui.small_button(REMOVE_ICON).on_hover_text(format!("Remove [{index}]")).clicked() {
            cx.edits.push((path.to_vec(), NifEdit::Remove(index)));
        }
    });
}

/// Keys as a table: one row per key, one column per present key field.
fn key_table(ui: &mut egui::Ui, cx: &mut GridContext<'_>, list: &ListRef<'_>, path: &mut Vec<FieldStep>, id: egui::Id) {
    let Some(Value::Struct(first)) = list.get(0) else { return };
    let columns: Vec<(usize, &'static str)> = first
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| field.present)
        .map(|(index, field)| (index, field.name()))
        .collect();
    egui::Grid::new(id.with("keys")).striped(true).show(ui, |ui| {
        ui.strong("#");
        for (_, name) in &columns {
            ui.strong(*name);
        }
        ui.end_row();
        for (row, item) in list.iter().enumerate().take(MAX_LIST_ROWS) {
            let Value::Struct(key) = item else { continue };
            let fields = key.fields();
            path.push(FieldStep::Item(row));
            ui.label(row.to_string());
            for &(column, _) in &columns {
                path.push(FieldStep::Field(column));
                match fields.get(column).map(|field| field.value) {
                    Some(Value::Struct(nested)) => match struct_floats(&nested) {
                        Some(floats) => {
                            ui.horizontal(|ui| {
                                let color = COLOR_TYPES
                                    .iter()
                                    .find(|(name, _)| *name == nested.type_name())
                                    .map(|&(_, components)| components);
                                inline_floats(ui, cx, &floats, color, path);
                            });
                        }
                        None => {
                            ui.weak(nested.type_name());
                        }
                    },
                    Some(value) => scalar_ui(ui, cx, value, path, id.with((row, column))),
                    None => {
                        ui.label("");
                    }
                }
                path.pop();
            }
            path.pop();
            item_buttons(ui, cx, path, row);
            ui.end_row();
        }
    });
    if list.len() > MAX_LIST_ROWS {
        ui.weak(format!("... {} more keys not shown", list.len() - MAX_LIST_ROWS));
    }
}

/// The key curves over time, each scaled to the common value range.
fn curve_plot(ui: &mut egui::Ui, curves: &[KeyCurve]) {
    let points = || curves.iter().flat_map(|curve| curve.points.iter());
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for point in points() {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    let width = ui.available_width();
    let (response, painter) = ui.allocate_painter(egui::vec2(width, CURVE_HEIGHT), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, CURVE_FRAME_COLOR), egui::StrokeKind::Inside);
    let span = |axis: usize| if max[axis] > min[axis] { max[axis] - min[axis] } else { 1.0 };
    let to_screen = |point: &[f32; 2]| {
        let x = rect.left() + (point[0] - min[0]) / span(0) * rect.width();
        let y = rect.bottom() - (point[1] - min[1]) / span(1) * rect.height();
        egui::pos2(x, y)
    };
    for (index, curve) in curves.iter().enumerate() {
        let color = CURVE_COLORS[index % CURVE_COLORS.len()];
        let line: Vec<egui::Pos2> = curve.points.iter().map(to_screen).collect();
        for point in &line {
            painter.circle_filled(*point, CURVE_POINT_RADIUS, color);
        }
        painter.add(egui::Shape::line(line, egui::Stroke::new(CURVE_STROKE_WIDTH, color)));
    }
    let labels: Vec<&str> = curves.iter().map(|curve| curve.label.as_str()).collect();
    response.on_hover_text(format!(
        "{}: time {:.3}..{:.3}, value {:.3}..{:.3} (linear between keys)",
        labels.join(", "),
        min[0],
        max[0],
        min[1],
        max[1]
    ));
}
