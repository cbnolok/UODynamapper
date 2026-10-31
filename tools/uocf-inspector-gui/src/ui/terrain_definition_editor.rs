//! The TerrainDefinition.uop view of the UOP browser: the selected entry, and the editor of every definition of the
//! package (`logic/terrain_definition_editor.rs`).

use std::path::Path;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use uocf::enhanced::terrain_definition::{
    TerrainDefinitionEntry, TerrainDefinitionIssue, TERRAIN_DEFINITION_UOP_NAME, TERRAIN_FLAG_NAMES,
};
use uocf::uop_container::package::UopPackage;

use super::UOP_PATCHER_NOTE;
use crate::app::UopInspectorApp;
use crate::logic::terrain_definition_editor::{TerrainDefinitionEditor, TerrainDefinitionField};

const ROW_HEIGHT: f32 = 22.0;
const TABLE_MAX_HEIGHT: f32 = 320.0;
const ID_COLUMN_WIDTH: f32 = 50.0;
const NAME_COLUMN_WIDTH: f32 = 200.0;
const VALUE_COLUMN_WIDTH: f32 = 90.0;
const SHADER_COLUMN_WIDTH: f32 = 190.0;
const ACTIONS_COLUMN_WIDTH: f32 = 170.0;
/// Drag speed of the integer fields (typing is the main input).
const DRAG_SPEED: f64 = 0.1;
/// Drag speed of the float fields.
const FLOAT_DRAG_SPEED: f64 = 0.01;
/// Issues listed above the table; the rest are counted.
const MAX_LISTED_ISSUES: usize = 8;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);

/// An edit made in the table or the detail grids, applied once they are drawn.
enum Action {
    /// A field gained focus or started a drag: the next edit is a new undo step.
    BreakCoalescing,
    Select(u32),
    Edit(u32, TerrainDefinitionField, Box<dyn FnOnce(&mut TerrainDefinitionEntry)>),
    Shader(u32, u32),
    AddAlias(u32),
    RemoveLastAlias(u32),
    Duplicate(u32),
    Remove(u32),
}

pub(super) fn is_terrain_definition_package(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(TERRAIN_DEFINITION_UOP_NAME))
}

/// The editor of the TerrainDefinition package `package`, loaded from `source`, or the button opening it.
pub(super) fn ui_terrain_definition_editor(app: &mut UopInspectorApp, ui: &mut egui::Ui, source: &Path, package: &UopPackage) {
    let mut editor = app.terrain_definition_editor.take().filter(|editor| editor.source == source);
    match editor.as_mut() {
        None => {
            if ui
                .button("Edit TerrainDefinition.uop")
                .on_hover_text("Edit every definition of the package and save a copy of it.")
                .clicked()
            {
                let dictionary = app.uo_string_dictionary.as_deref();
                match TerrainDefinitionEditor::open(source.to_path_buf(), package, dictionary) {
                    Ok(opened) => editor = Some(opened),
                    Err(error) => app.log(format!("Failed to open the TerrainDefinition editor: {error}")),
                }
            }
        }
        Some(editing) => {
            let mut close = false;
            ui.horizontal_wrapped(|ui| close = toolbar(app, ui, editing));
            if close {
                editor = None;
            } else {
                issues(ui, editing);
                editor_table(ui, editing);
            }
        }
    }
    app.terrain_definition_editor = editor;
}

/// Whether the editor of `source` is open (the entry views are then hidden).
pub(super) fn is_editing(app: &UopInspectorApp, source: &Path) -> bool {
    app.terrain_definition_editor.as_ref().is_some_and(|editor| editor.source == source)
}

/// Undo, redo, save and close of the editor; true when the editor is closed.
fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui, editor: &mut TerrainDefinitionEditor) -> bool {
    ui.label(format!("{:?} records", editor.dialect));
    if ui.add_enabled(editor.edits.can_undo(), egui::Button::new("Undo")).clicked() {
        editor.undo();
    }
    if ui.add_enabled(editor.edits.can_redo(), egui::Button::new("Redo")).clicked() {
        editor.redo();
    }
    if ui
        .button("Save TerrainDefinition.uop...")
        .on_hover_text(format!(
            "Writes a copy of the loaded TerrainDefinition.uop with the changed definitions, in the package's record \
             layout; every other entry keeps its bytes. The loaded package itself is refused as the target, and so is \
             a definition that would crash the client.\n{UOP_PATCHER_NOTE}"
        ))
        .clicked()
    {
        let (name, extensions) = UOP_FILTER;
        let output =
            crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(TERRAIN_DEFINITION_UOP_NAME).save_file();
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

/// The client checks of the edited definitions.
fn issues(ui: &mut egui::Ui, editor: &TerrainDefinitionEditor) {
    let issues = editor.issues();
    for issue in issues.iter().take(MAX_LISTED_ISSUES) {
        let color = if issue.crashes_client() { ERROR_COLOR } else { WARNING_COLOR };
        ui.colored_label(color, issue.to_string());
    }
    if issues.len() > MAX_LISTED_ISSUES {
        ui.colored_label(WARNING_COLOR, format!("... and {} more issues", issues.len() - MAX_LISTED_ISSUES));
    }
}

/// A drag field; pushes the coalescing break and the edit it makes.
fn drag<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: T,
    speed: f64,
    actions: &mut Vec<Action>,
    id: u32,
    field: TerrainDefinitionField,
    apply: impl FnOnce(&mut TerrainDefinitionEntry, T) + 'static,
) {
    let mut edited = value;
    let response = ui.add(egui::DragValue::new(&mut edited).speed(speed));
    if response.drag_started() || response.gained_focus() {
        actions.push(Action::BreakCoalescing);
    }
    if response.changed() {
        actions.push(Action::Edit(id, field, Box::new(move |definition| apply(definition, edited))));
    }
}

/// Filter, duplicate, the definitions as editable rows, and the aliases, layers and shader values of the selected
/// one.
fn editor_table(ui: &mut egui::Ui, editor: &mut TerrainDefinitionEditor) {
    let mut actions = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(&mut editor.filter).on_hover_text("Id or name.");
        ui.separator();
        ui.label("New material id:");
        ui.add(egui::DragValue::new(&mut editor.new_id).speed(DRAG_SPEED))
            .on_hover_text("The id of the definition that Duplicate creates.");
    });
    let ids = editor.visible_ids();
    ui.label(format!("{} of {} definitions", ids.len(), editor.definitions.len()));
    let definitions = &editor.definitions;
    let shaders = &editor.shaders;
    let selected = editor.selected;
    egui::ScrollArea::horizontal().id_salt("terrain_definition_table").show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("terrain_definition_rows")
            .striped(true)
            .max_scroll_height(TABLE_MAX_HEIGHT)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(ID_COLUMN_WIDTH))
            .column(Column::initial(NAME_COLUMN_WIDTH).resizable(true))
            .columns(Column::exact(VALUE_COLUMN_WIDTH), 2)
            .column(Column::initial(SHADER_COLUMN_WIDTH).resizable(true))
            .column(Column::auto().at_least(ACTIONS_COLUMN_WIDTH))
            .header(ROW_HEIGHT, |mut header| {
                for title in ["id", "name", "coverage exp.", "aliases", "shader", ""] {
                    header.col(|ui| {
                        ui.strong(title);
                    });
                }
            })
            .body(|body| {
                body.rows(ROW_HEIGHT, ids.len(), |mut row| {
                    let id = ids[row.index()];
                    let definition = &definitions[&id];
                    row.col(|ui| {
                        if ui.selectable_label(selected == Some(id), id.to_string()).clicked() {
                            actions.push(Action::Select(id));
                        }
                    });
                    row.col(|ui| {
                        ui.label(definition.name.as_deref().unwrap_or("?"))
                            .on_hover_text(format!("string {}", definition.name_index));
                    });
                    row.col(|ui| {
                        drag(ui, definition.coverage_exponent(), FLOAT_DRAG_SPEED, &mut actions, id,
                            TerrainDefinitionField::CoverageExponent, |definition, value| definition.set_coverage_exponent(value));
                    });
                    row.col(|ui| {
                        ui.label(definition.aliases.len().to_string());
                    });
                    row.col(|ui| match &definition.texture {
                        Some(texture) => {
                            let current = shaders.get(&texture.shader_name_index).cloned().unwrap_or_default();
                            egui::ComboBox::from_id_salt(("terrain_shader", id)).selected_text(current).show_ui(ui, |ui| {
                                for (&index, name) in shaders {
                                    if ui.selectable_label(index == texture.shader_name_index, name).clicked()
                                        && index != texture.shader_name_index
                                    {
                                        actions.push(Action::Shader(id, index));
                                    }
                                }
                            });
                        }
                        None => {
                            ui.colored_label(WARNING_COLOR, "no shader block");
                        }
                    });
                    row.col(|ui| {
                        if ui.small_button("Edit").on_hover_text("Shows its aliases, layers and shader values.").clicked() {
                            actions.push(Action::Select(id));
                        }
                        if ui.small_button("Duplicate").on_hover_text("Copies it as the new material id.").clicked() {
                            actions.push(Action::Duplicate(id));
                        }
                        if ui.small_button("Delete").clicked() {
                            actions.push(Action::Remove(id));
                        }
                    });
                });
            });
    });
    if let Some(definition) = selected.and_then(|id| definitions.get(&id)) {
        ui.separator();
        definition_details(ui, editor, definition, &mut actions);
    }
    for action in actions {
        match action {
            Action::BreakCoalescing => editor.edits.break_coalescing(),
            Action::Select(id) => editor.selected = Some(id),
            Action::Edit(id, field, edit) => editor.edit(id, field, edit),
            Action::Shader(id, index) => editor.set_shader(id, index),
            Action::AddAlias(id) => editor.add_alias(id),
            Action::RemoveLastAlias(id) => editor.remove_last_alias(id),
            Action::Duplicate(id) => {
                let new_id = editor.new_id;
                editor.error = editor.duplicate(id, new_id).err();
            }
            Action::Remove(id) => editor.remove(id),
        }
    }
}

/// The header floats, aliases with their flags, texture layers, techniques and tweaks of `definition`.
fn definition_details(
    ui: &mut egui::Ui,
    editor: &TerrainDefinitionEditor,
    definition: &TerrainDefinitionEntry,
    actions: &mut Vec<Action>,
) {
    let id = definition.id;
    ui.heading(format!("Material {id}: {}", definition.name.as_deref().unwrap_or("?")));
    ui.horizontal_wrapped(|ui| {
        ui.label("field_34");
        drag(ui, definition.field_34, FLOAT_DRAG_SPEED, actions, id, TerrainDefinitionField::Field34,
            |definition, value| definition.field_34 = value);
        ui.label("field_3c");
        drag(ui, definition.field_3c, FLOAT_DRAG_SPEED, actions, id, TerrainDefinitionField::Field3c,
            |definition, value| definition.field_3c = value);
    })
    .response
    .on_hover_text("No client reader; 0 in every shipped record.");

    ui.horizontal(|ui| {
        ui.strong("Aliases");
        if ui.small_button("Add").on_hover_text("Appends the next subtype, without flags.").clicked() {
            actions.push(Action::AddAlias(id));
        }
        if ui.small_button("Remove last").on_hover_text("The subtype-0 alias stays.").clicked() {
            actions.push(Action::RemoveLastAlias(id));
        }
    });
    egui::Grid::new(("terrain_aliases", id)).striped(true).show(ui, |ui| {
        ui.strong("subtype");
        ui.strong("legacy land");
        for flag in &TERRAIN_FLAG_NAMES {
            let hint = if flag.read_by_client { "read by the client" } else { "not read by the client" };
            ui.strong(flag.name).on_hover_text(hint);
        }
        ui.strong("raw flags");
        ui.end_row();
        for (position, alias) in definition.aliases.iter().enumerate() {
            ui.label(alias.subtype.to_string());
            drag(ui, alias.legacy_land_id, DRAG_SPEED, actions, id, TerrainDefinitionField::AliasLegacyLand(position),
                move |definition, value| definition.aliases[position].legacy_land_id = value);
            for flag in &TERRAIN_FLAG_NAMES {
                let mask = flag.mask_in(editor.dialect);
                let mut on = alias.tile_flags & mask != 0;
                if ui.checkbox(&mut on, "").changed() {
                    actions.push(Action::BreakCoalescing);
                    actions.push(Action::Edit(id, TerrainDefinitionField::AliasFlags(position), Box::new(move |definition| {
                        let flags = &mut definition.aliases[position].tile_flags;
                        *flags = if on { *flags | mask } else { *flags & !mask };
                    })));
                }
            }
            ui.monospace(format!("{:#x}", alias.tile_flags));
            ui.end_row();
        }
    });

    let Some(texture) = &definition.texture else {
        return;
    };
    ui.strong("Texture layers");
    egui::Grid::new(("terrain_layers", id)).striped(true).show(ui, |ui| {
        for title in ["#", "path", "stretch", "sampler"] {
            ui.strong(title);
        }
        ui.end_row();
        for (position, layer) in texture.layers.iter().enumerate() {
            ui.label(position.to_string());
            ui.label(layer.path.as_deref().unwrap_or("?")).on_hover_text(format!("string {}", layer.path_index));
            drag(ui, layer.texture_repetition, FLOAT_DRAG_SPEED, actions, id, TerrainDefinitionField::LayerStretch(position),
                move |definition, value| {
                    if let Some(texture) = definition.texture.as_mut() {
                        texture.layers[position].texture_repetition = value;
                    }
                });
            drag(ui, layer.sampler_index, DRAG_SPEED, actions, id, TerrainDefinitionField::LayerSampler(position),
                move |definition, value| {
                    if let Some(texture) = definition.texture.as_mut() {
                        texture.layers[position].sampler_index = value;
                    }
                });
            ui.end_row();
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.strong("Techniques");
        for (position, value) in texture.techniques().iter().enumerate() {
            drag(ui, *value, DRAG_SPEED, actions, id, TerrainDefinitionField::Technique(position),
                move |definition, value| {
                    if let Some(texture) = definition.texture.as_mut() {
                        texture.techniques_mut()[position] = value;
                    }
                });
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.strong("Tweaks");
        for (position, value) in texture.tweaks().iter().enumerate() {
            drag(ui, *value, FLOAT_DRAG_SPEED, actions, id, TerrainDefinitionField::Tweak(position),
                move |definition, value| {
                    if let Some(texture) = definition.texture.as_mut() {
                        texture.tweaks_mut()[position] = value;
                    }
                });
        }
    });
    if definition.validate().iter().any(TerrainDefinitionIssue::crashes_client) {
        ui.colored_label(ERROR_COLOR, "This definition would crash the client.");
    }
}
