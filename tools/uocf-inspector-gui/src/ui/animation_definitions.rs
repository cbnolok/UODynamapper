//! Animation definitions tab: viewer and editor of `AnimationDefinition.uop`, the animated items (one entry per
//! item id: frame sets of tile ids with a frame interval, a successor and per-frame effects). The editing model lives
//! in `logic::animation_definition_editor`; field meanings in `uocf::enhanced::animation_definition`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use uocf::classic::art::static_art_id_for_source;
use uocf::enhanced::animation_definition::{
    AnimationDefinition, AnimationDefinitionIssue, AnimationIssueSeverity, ANIMATION_DEFINITION_UOP_NAME,
};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{concrete_art_tile_source, UopInspectorApp, ViewMode};
use crate::logic::animation_definition_editor::{
    frame_effects_kdl, parse_single_kdl, AnimationDefinitionEditor, AnimationDefinitionField,
};
use crate::ui::{multis, UOP_PATCHER_NOTE};

const LIST_PANEL_WIDTH: f32 = 260.0;
const VALIDATION_PANEL_HEIGHT: f32 = 150.0;
/// Largest side of a frame thumbnail; larger art is scaled down.
const THUMBNAIL_MAX_SIDE: f32 = 44.0;
const TILE_ID_HEX_DIGITS: usize = 4;
/// Drag speed of the id fields (typing is the main input).
const ID_DRAG_SPEED: f64 = 0.1;
/// Drag speed of the frame interval, in seconds.
const INTERVAL_DRAG_SPEED: f64 = 0.001;
const VALUE_DRAG_SPEED: f64 = 0.01;
/// Rows of the KDL text editors.
const KDL_EDITOR_ROWS: usize = 14;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 200, 110);
const ERROR_ICON: &str = "✖";
const WARNING_ICON: &str = "⚠";
const DIRTY_MARKER: &str = "*";
const NO_NEXT_LABEL: &str = "none (stops)";

const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const KDL_FILTER: (&str, &[&str]) = ("KDL text", &["kdl"]);

const UNDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);

/// State of the Animation definitions tab.
#[derive(Default)]
pub struct AnimationDefinitionsPanelState {
    editor: Option<AnimationDefinitionEditor>,
    /// Increased whenever another package is opened (the edit revisions restart).
    generation: u64,
    /// The client package opened by itself once, so a failed open is not retried every frame.
    auto_opened: Option<PathBuf>,
    filter: String,
    selected: Option<u32>,
    /// Item id typed for an added or duplicated definition.
    new_id: u32,
    error: Option<String>,
    pending_open: Option<PathBuf>,
    effects_draft: Option<EffectsDraft>,
    entry_draft: Option<EntryDraft>,
    validation: Option<ValidationCache>,
}

/// The effects of one frame, edited as KDL text.
struct EffectsDraft {
    id: u32,
    set: u32,
    frame: usize,
    text: String,
    error: Option<String>,
}

/// One whole entry, edited as KDL text.
struct EntryDraft {
    id: u32,
    text: String,
    error: Option<String>,
}

struct ValidationCache {
    key: (u64, u64, usize),
    issues: Arc<Vec<AnimationDefinitionIssue>>,
}

/// An edit collected while drawing, applied afterwards.
enum Action {
    Edit(u32, AnimationDefinitionField, Box<dyn FnOnce(&mut AnimationDefinition)>),
    BreakCoalescing,
    AddSet(u32),
    RemoveSet(u32, u32),
    AddFrame(u32, u32),
    RemoveFrame(u32, u32, usize),
    MoveFrame(u32, u32, usize, bool),
    EditEffects(u32, u32, usize),
    JumpToArt(u32),
}

pub fn ui_animation_definitions(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    auto_open_client(app);
    handle_shortcuts(app, ctx);
    egui::Panel::top("animation_definitions_toolbar").show(ui, |ui| toolbar(app, ui));

    if app.animation_definitions_panel.editor.is_none() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No AnimationDefinition.uop open. Set the Enhanced Client or Kingdom Reborn folder (Search Paths), \
                     or open a package.",
                );
            });
        });
        pending_open_window(app, ctx);
        return;
    }

    refresh_validation(app);
    egui::Panel::bottom("animation_definitions_validation")
        .resizable(true)
        .default_size(VALIDATION_PANEL_HEIGHT)
        .show(ui, |ui| validation_panel(app, ui));
    egui::Panel::left("animation_definitions_list")
        .resizable(true)
        .default_size(LIST_PANEL_WIDTH)
        .show(ui, |ui| list_panel(app, ui));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("animation_definitions_details_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| details_panel(app, ctx, ui));
    });
    effects_window(app, ctx);
    entry_window(app, ctx);
    pending_open_window(app, ctx);
}

/// `AnimationDefinition.uop` of the Enhanced Client folder of the search paths.
fn client_package(app: &UopInspectorApp) -> Option<PathBuf> {
    find_file_ignoring_case(app.settings.ec_path.as_deref()?, ANIMATION_DEFINITION_UOP_NAME)
}

fn strings(app: &UopInspectorApp) -> Option<Arc<UoStringDictionary>> {
    app.uo_string_dictionary.clone()
}

fn auto_open_client(app: &mut UopInspectorApp) {
    let Some(path) = client_package(app) else {
        return;
    };
    let state = &app.animation_definitions_panel;
    if state.editor.is_some() || state.auto_opened.as_ref() == Some(&path) {
        return;
    }
    app.animation_definitions_panel.auto_opened = Some(path.clone());
    perform_open(app, path);
}

fn handle_shortcuts(app: &mut UopInspectorApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let undo = ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT));
    let redo = ctx.input_mut(|input| input.consume_shortcut(&REDO_SHORTCUT));
    if let Some(editor) = app.animation_definitions_panel.editor.as_mut() {
        if undo {
            editor.undo();
        } else if redo {
            editor.redo();
        }
    }
}

fn request_open(app: &mut UopInspectorApp, path: PathBuf) {
    if app.animation_definitions_panel.editor.as_ref().is_some_and(|editor| editor.edits.is_dirty()) {
        app.animation_definitions_panel.pending_open = Some(path);
    } else {
        perform_open(app, path);
    }
}

fn perform_open(app: &mut UopInspectorApp, path: PathBuf) {
    let dialect = ClientDialect::of_package_path(&path);
    match AnimationDefinitionEditor::open(path.clone(), dialect) {
        Ok(editor) => {
            let message = format!(
                "Opened {} animation definitions ({}) from {}.",
                editor.definitions.len(),
                dialect.label(),
                path.display()
            );
            let state = &mut app.animation_definitions_panel;
            state.selected = state.selected.filter(|id| editor.definitions.contains_key(id));
            state.editor = Some(editor);
            state.generation += 1;
            state.error = None;
            state.effects_draft = None;
            state.entry_draft = None;
            state.validation = None;
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.animation_definitions_panel.error = Some(format!("Open failed: {error}")),
    }
}

fn pending_open_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(path) = app.animation_definitions_panel.pending_open.clone() else {
        return;
    };
    let (mut discard, mut cancel) = (false, false);
    egui::Window::new("Discard unsaved animation definition changes?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("The animation definitions have unsaved changes. Opening another package discards them.");
            ui.horizontal(|ui| {
                discard = ui.button("Discard and open").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if discard || cancel {
        app.animation_definitions_panel.pending_open = None;
    }
    if discard {
        perform_open(app, path);
    }
}

fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let client = client_package(app);
    let strings = strings(app);
    ui.horizontal_wrapped(|ui| {
        ui.label("Open:");
        if ui
            .add_enabled(client.is_some(), egui::Button::new("Client AnimationDefinition.uop"))
            .on_hover_text("AnimationDefinition.uop of the Enhanced Client folder (Kingdom Reborn next to UOKR.exe).")
            .on_disabled_hover_text("The Enhanced Client folder of the search paths has no AnimationDefinition.uop.")
            .clicked()
        {
            if let Some(path) = client.clone() {
                request_open(app, path);
            }
        }
        if ui.button("Package...").clicked() {
            if let Some(path) = crate::dialog::file_dialog().add_filter(UOP_FILTER.0, UOP_FILTER.1).pick_file() {
                request_open(app, path);
            }
        }

        let has_editor = app.animation_definitions_panel.editor.is_some();
        ui.separator();
        ui.label("Import:");
        let import_files = ui
            .add_enabled(has_editor, egui::Button::new("KDL files..."))
            .on_hover_text("Replaces or adds the entries of KDL files (undoable).")
            .clicked();
        let import_folder = ui
            .add_enabled(has_editor, egui::Button::new("KDL folder..."))
            .on_hover_text("Replaces or adds the entries of every .kdl file of a folder (undoable).")
            .clicked();
        let paths = if import_files {
            crate::dialog::file_dialog().add_filter(KDL_FILTER.0, KDL_FILTER.1).pick_files()
        } else if import_folder {
            crate::dialog::pick_folder().map(|folder| vec![folder])
        } else {
            None
        };
        if let Some(paths) = paths {
            import(app, &paths, strings.as_deref());
        }

        ui.separator();
        ui.label("Save:");
        if ui
            .add_enabled(has_editor, egui::Button::new("Package..."))
            .on_hover_text(format!(
                "Writes a copy of the opened AnimationDefinition.uop with only the changed entries replaced, added or \
                 removed. The opened package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
            ))
            .clicked()
        {
            let file_name = ANIMATION_DEFINITION_UOP_NAME;
            if let Some(path) = crate::dialog::file_dialog().add_filter(UOP_FILTER.0, UOP_FILTER.1).set_file_name(file_name).save_file() {
                save(app, &path, strings.as_deref());
            }
        }
        let selected = app.animation_definitions_panel.selected;
        if ui
            .add_enabled(has_editor && selected.is_some(), egui::Button::new("Export selected KDL..."))
            .clicked()
        {
            export_selected(app, strings.as_deref());
        }
        if ui
            .add_enabled(has_editor, egui::Button::new("Export all KDL..."))
            .on_hover_text("One <item id>.kdl file per entry, into a folder.")
            .clicked()
        {
            if let Some(dir) = crate::dialog::pick_folder() {
                export_all(app, &dir, strings.as_deref());
            }
        }

        let state = &mut app.animation_definitions_panel;
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        ui.separator();
        if ui.add_enabled(editor.edits.can_undo(), egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
            editor.undo();
        }
        if ui.add_enabled(editor.edits.can_redo(), egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
            editor.redo();
        }
        let can_revert = !editor.edits.changes_since_opened(&editor.definitions).is_empty();
        if ui
            .add_enabled(can_revert, egui::Button::new("Revert to loaded"))
            .on_hover_text("Back to the definitions as opened (undoable).")
            .clicked()
        {
            editor.edits.revert_to_opened(&mut editor.definitions);
        }
        ui.separator();
        if editor.edits.is_dirty() {
            ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
        }
        ui.weak(format!("{} ({})", editor.source.display(), editor.dialect.label()));
    });
    if let Some(error) = app.animation_definitions_panel.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ERROR_COLOR, error);
            if ui.small_button("Dismiss").clicked() {
                app.animation_definitions_panel.error = None;
            }
        });
    }
}

fn import(app: &mut UopInspectorApp, paths: &[PathBuf], strings: Option<&UoStringDictionary>) {
    let state = &mut app.animation_definitions_panel;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    match editor.import_kdl(paths, strings) {
        Ok(import) => {
            state.error = None;
            state.selected = import.added.first().or(import.replaced.first()).copied().or(state.selected);
            let message = format!(
                "Imported {} KDL files: {} definitions replaced, {} added.",
                import.files.len(),
                import.replaced.len(),
                import.added.len()
            );
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => state.error = Some(format!("Import failed: {error}")),
    }
}

fn save(app: &mut UopInspectorApp, path: &Path, strings: Option<&UoStringDictionary>) {
    let state = &mut app.animation_definitions_panel;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    match editor.save(path, strings) {
        Ok(changes) => {
            state.error = None;
            let message = format!(
                "Saved {}: {} entries replaced, {} added, {} removed.",
                path.display(),
                changes.replaced,
                changes.added,
                changes.removed
            );
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => state.error = Some(format!("Save failed: {error}")),
    }
}

fn export_selected(app: &mut UopInspectorApp, strings: Option<&UoStringDictionary>) {
    let state = &mut app.animation_definitions_panel;
    let (Some(editor), Some(id)) = (state.editor.as_ref(), state.selected) else {
        return;
    };
    let file_name = uocf::enhanced::animation_definition::animation_definition_kdl_file_name(id);
    let Some(path) = crate::dialog::file_dialog().add_filter(KDL_FILTER.0, KDL_FILTER.1).set_file_name(&file_name).save_file() else {
        return;
    };
    match editor.export_kdl(id, &path, strings) {
        Ok(()) => {
            state.error = None;
            let message = format!("Exported item {id} to {}.", path.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => state.error = Some(format!("Export failed: {error}")),
    }
}

fn export_all(app: &mut UopInspectorApp, dir: &Path, strings: Option<&UoStringDictionary>) {
    let state = &mut app.animation_definitions_panel;
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    match editor.export_kdl_all(dir, strings) {
        Ok(count) => {
            state.error = None;
            let message = format!("Exported {count} KDL files to {}.", dir.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => state.error = Some(format!("Export failed: {error}")),
    }
}

fn refresh_validation(app: &mut UopInspectorApp) {
    let strings = strings(app);
    let strings_address = strings.as_ref().map_or(0, |strings| Arc::as_ptr(strings) as usize);
    let state = &mut app.animation_definitions_panel;
    let Some(editor) = state.editor.as_ref() else {
        return;
    };
    let key = (state.generation, editor.edits.revision(), strings_address);
    if state.validation.as_ref().is_some_and(|cache| cache.key == key) {
        return;
    }
    let issues = Arc::new(editor.issues(strings.as_deref()));
    state.validation = Some(ValidationCache { key, issues });
}

fn severity_style(severity: AnimationIssueSeverity) -> (&'static str, egui::Color32) {
    match severity {
        AnimationIssueSeverity::Error => (ERROR_ICON, ERROR_COLOR),
        AnimationIssueSeverity::Warning => (WARNING_ICON, WARNING_COLOR),
    }
}

fn validation_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &mut app.animation_definitions_panel;
    let Some(cache) = state.validation.as_ref() else {
        return;
    };
    let issues = Arc::clone(&cache.issues);
    let errors = issues.iter().filter(|issue| issue.severity() == AnimationIssueSeverity::Error).count();
    let warnings = issues.len() - errors;
    ui.horizontal_wrapped(|ui| {
        ui.heading("Validation");
        ui.colored_label(if errors > 0 { ERROR_COLOR } else { OK_COLOR }, format!("{errors} errors"));
        ui.colored_label(if warnings > 0 { WARNING_COLOR } else { OK_COLOR }, format!("{warnings} warnings"));
        ui.weak("Errors block saving. Particle files are not checked here.");
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("animation_definitions_validation_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if issues.is_empty() {
                ui.colored_label(OK_COLOR, "No issues.");
            }
            for issue in issues.iter() {
                ui.horizontal(|ui| {
                    let (icon, color) = severity_style(issue.severity());
                    ui.colored_label(color, icon);
                    if ui.selectable_label(state.selected == issue.item_id, issue.to_string()).clicked() {
                        state.selected = issue.item_id;
                    }
                });
            }
        });
}

fn list_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &mut app.animation_definitions_panel;
    let Some(editor) = state.editor.as_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.text_edit_singleline(&mut state.filter).on_hover_text("Item id or stored id, decimal or hex.");
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Item id:");
        ui.add(hex_id(&mut state.new_id));
        let result = if ui.button("Add").on_hover_text("A looping set of one frame showing the item.").clicked() {
            Some(editor.add_new(state.new_id))
        } else if ui.add_enabled(state.selected.is_some(), egui::Button::new("Duplicate selected")).clicked() {
            state.selected.map(|selected| editor.duplicate(selected, state.new_id))
        } else {
            None
        };
        match result {
            Some(Ok(())) => {
                state.selected = Some(state.new_id);
                state.error = None;
            }
            Some(Err(error)) => state.error = Some(error),
            None => {}
        }
        if let Some(selected) = state.selected.filter(|id| editor.definitions.contains_key(id)) {
            if ui.button("Remove selected").clicked() {
                editor.remove(selected);
            }
        }
    });
    ui.separator();
    let worst = |id: u32| {
        state.validation.as_ref().and_then(|cache| {
            cache.issues.iter().filter(|issue| issue.item_id == Some(id)).map(AnimationDefinitionIssue::severity).max()
        })
    };
    let visible = editor.visible_ids(&state.filter);
    ui.weak(format!("{} of {} definitions", visible.len(), editor.definitions.len()));
    let mut clicked = None;
    egui::ScrollArea::vertical().id_salt("animation_definitions_list_scroll").auto_shrink([false, false]).show(ui, |ui| {
        for id in visible {
            let definition = &editor.definitions[&id];
            ui.horizontal(|ui| {
                match worst(id) {
                    Some(severity) => {
                        let (icon, color) = severity_style(severity);
                        ui.colored_label(color, icon);
                    }
                    None => {
                        ui.label(" ");
                    }
                }
                let mut label = format!("{id} (0x{id:04X}): {} sets", definition.sets.len());
                if definition.stored_id != id {
                    label.push_str(&format!(", stored as {}", definition.stored_id));
                }
                if ui.selectable_label(state.selected == Some(id), label).clicked() {
                    clicked = Some(id);
                }
            });
        }
    });
    if let Some(id) = clicked {
        editor.edits.break_coalescing();
        state.selected = Some(id);
    }
}

/// A drag value of `original` built by `widget`; a change becomes an edit of `field` applied by `apply`.
fn drag<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    original: T,
    widget: impl FnOnce(&mut T) -> egui::DragValue<'_>,
    actions: &mut Vec<Action>,
    id: u32,
    field: AnimationDefinitionField,
    apply: impl FnOnce(&mut AnimationDefinition, T) + 'static,
) {
    let mut edited = original;
    let response = ui.add(widget(&mut edited));
    if response.drag_started() || response.gained_focus() {
        actions.push(Action::BreakCoalescing);
    }
    if response.changed() && edited != original {
        actions.push(Action::Edit(id, field, Box::new(move |definition| apply(definition, edited))));
    }
}

fn hex_id(value: &mut u32) -> egui::DragValue<'_> {
    egui::DragValue::new(value).speed(ID_DRAG_SPEED).hexadecimal(TILE_ID_HEX_DIGITS, false, true).prefix("0x")
}

fn value_drag(value: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(value).speed(VALUE_DRAG_SPEED).range(0.0..=f32::MAX)
}

fn interval_drag(value: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(value).speed(INTERVAL_DRAG_SPEED).range(0.0..=f32::MAX)
}

/// A set key picker; `none` adds the "no successor" choice.
fn set_picker(ui: &mut egui::Ui, salt: impl std::hash::Hash + std::fmt::Debug, keys: &[u32], current: Option<u32>, none: bool) -> Option<Option<u32>> {
    let label = |key: Option<u32>| key.map_or(NO_NEXT_LABEL.to_string(), |key| format!("set {key}"));
    let mut picked = None;
    egui::ComboBox::from_id_salt(salt).selected_text(label(current)).show_ui(ui, |ui| {
        if none && ui.selectable_label(current.is_none(), NO_NEXT_LABEL).clicked() {
            picked = Some(None);
        }
        for &key in keys {
            if ui.selectable_label(current == Some(key), label(Some(key))).clicked() {
                picked = Some(Some(key));
            }
        }
    });
    picked.filter(|picked| *picked != current)
}

fn details_panel(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let state = &app.animation_definitions_panel;
    let Some((id, definition)) = state
        .selected
        .and_then(|id| state.editor.as_ref()?.definitions.get(&id).map(|definition| (id, definition.clone())))
    else {
        ui.label("Select a definition in the list.");
        return;
    };
    let tile_ids: Vec<u32> = definition.sets.values().flat_map(|set| set.frames.iter().map(|frame| frame.tile_id)).collect();
    let thumbnails: Vec<_> = tile_ids.iter().map(|&tile_id| multis::static_tile_thumbnail(app, ctx, tile_id)).collect();
    let mut thumbnails = thumbnails.into_iter();
    let keys: Vec<u32> = definition.sets.keys().copied().collect();
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.heading(format!("Item {id} (0x{id:04X})"));
        if ui.button("Edit as KDL...").on_hover_text("The whole entry as KDL text.").clicked() {
            let strings = strings(app);
            let text = app
                .animation_definitions_panel
                .editor
                .as_ref()
                .and_then(|editor| editor.kdl_text(id, strings.as_deref()))
                .unwrap_or_default();
            app.animation_definitions_panel.entry_draft = Some(EntryDraft { id, text, error: None });
        }
    });
    egui::Grid::new(("animation_definition_fields", id)).num_columns(2).show(ui, |ui| {
        ui.label("Stored id").on_hover_text("The id the entry stores (an item may reuse another item's frames under its own id).");
        drag(ui, definition.stored_id, hex_id, &mut actions, id, AnimationDefinitionField::StoredId, |definition, value| {
            definition.stored_id = value
        });
        ui.end_row();
        ui.label("Start set").on_hover_text("The set played first; the client rejects an entry whose start set is missing.");
        if let Some(Some(key)) = set_picker(ui, ("animation_start", id), &keys, Some(definition.start_set), false) {
            actions.push(Action::Edit(id, AnimationDefinitionField::StartSet, Box::new(move |definition| definition.start_set = key)));
        }
        ui.end_row();
        ui.label("Second set").on_hover_text("The client rejects an entry whose second set is missing.");
        if let Some(Some(key)) = set_picker(ui, ("animation_second", id), &keys, Some(definition.second_set), false) {
            actions.push(Action::Edit(id, AnimationDefinitionField::SecondSet, Box::new(move |definition| definition.second_set = key)));
        }
        ui.end_row();
        ui.label("Value").on_hover_text("Meaning not traced; the client rejects negative values.");
        drag(ui, definition.value, value_drag, &mut actions, id, AnimationDefinitionField::Value, |definition, value| {
            definition.value = value
        });
        ui.end_row();
    });

    ui.separator();
    for (&key, set) in &definition.sets {
        let title = format!("Set {key}: {} frames, {} s per frame", set.frames.len(), set.frame_interval);
        egui::CollapsingHeader::new(title).id_salt(("animation_set", id, key)).default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Interval (s):");
                drag(ui, set.frame_interval, interval_drag, &mut actions, id, AnimationDefinitionField::Interval(key), move |definition, value| {
                    if let Some(set) = definition.sets.get_mut(&key) {
                        set.frame_interval = value;
                    }
                });
                ui.label("Next:");
                if let Some(next) = set_picker(ui, ("animation_next", id, key), &keys, set.next, true) {
                    actions.push(Action::Edit(id, AnimationDefinitionField::Next(key), Box::new(move |definition| {
                        if let Some(set) = definition.sets.get_mut(&key) {
                            set.next = next;
                        }
                    })));
                }
                if ui.button("Add frame").clicked() {
                    actions.push(Action::AddFrame(id, key));
                }
                if ui.button("Remove set").clicked() {
                    actions.push(Action::RemoveSet(id, key));
                }
            });
            egui::Grid::new(("animation_frames", id, key)).num_columns(5).striped(true).show(ui, |ui| {
                for (index, frame) in set.frames.iter().enumerate() {
                    match thumbnails.next().flatten() {
                        Some(handle) => {
                            let size = handle.size_vec2();
                            let scale = (THUMBNAIL_MAX_SIDE / size.x.max(size.y)).min(1.0);
                            let image = egui::Image::new(&handle).fit_to_exact_size(size * scale).sense(egui::Sense::click());
                            if ui.add(image).on_hover_text("Show in the Art tab.").clicked() {
                                actions.push(Action::JumpToArt(frame.tile_id));
                            }
                        }
                        None => {
                            ui.weak("no art");
                        }
                    }
                    drag(ui, frame.tile_id, hex_id, &mut actions, id, AnimationDefinitionField::FrameTile(key, index), move |definition, value| {
                        if let Some(frame) = definition.sets.get_mut(&key).and_then(|set| set.frames.get_mut(index)) {
                            frame.tile_id = value;
                        }
                    });
                    let effects = match frame.effects.len() {
                        0 => "no effects".to_string(),
                        1 => "1 effect".to_string(),
                        count => format!("{count} effects"),
                    };
                    if ui.button(effects).on_hover_text("Edit the frame's effects as KDL text.").clicked() {
                        actions.push(Action::EditEffects(id, key, index));
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(index > 0, egui::Button::new("⏶")).on_hover_text("Move up").clicked() {
                            actions.push(Action::MoveFrame(id, key, index, true));
                        }
                        if ui.add_enabled(index + 1 < set.frames.len(), egui::Button::new("⏷")).on_hover_text("Move down").clicked() {
                            actions.push(Action::MoveFrame(id, key, index, false));
                        }
                    });
                    if ui.button("Remove").clicked() {
                        actions.push(Action::RemoveFrame(id, key, index));
                    }
                    ui.end_row();
                }
            });
        });
    }
    if ui.button("Add set").clicked() {
        actions.push(Action::AddSet(id));
    }
    apply_actions(app, actions);
}

fn apply_actions(app: &mut UopInspectorApp, actions: Vec<Action>) {
    let strings = strings(app);
    for action in actions {
        let state = &mut app.animation_definitions_panel;
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        match action {
            Action::Edit(id, field, edit) => editor.edit(id, field, edit),
            Action::BreakCoalescing => editor.edits.break_coalescing(),
            Action::AddSet(id) => {
                editor.add_set(id);
            }
            Action::RemoveSet(id, key) => editor.remove_set(id, key),
            Action::AddFrame(id, key) => editor.add_frame(id, key),
            Action::RemoveFrame(id, key, index) => editor.remove_frame(id, key, index),
            Action::MoveFrame(id, key, index, up) => editor.move_frame(id, key, index, up),
            Action::EditEffects(id, set, frame) => {
                let text = editor
                    .definitions
                    .get(&id)
                    .and_then(|definition| definition.sets.get(&set)?.frames.get(frame))
                    .map(|frame| frame_effects_kdl(frame, strings.as_deref()))
                    .unwrap_or_default();
                state.effects_draft = Some(EffectsDraft { id, set, frame, text, error: None });
            }
            Action::JumpToArt(tile_id) => {
                let source = concrete_art_tile_source(app.selected_legacy_source);
                app.selected_tex_art_cc_id = Some(static_art_id_for_source(tile_id, source));
                app.view_mode = ViewMode::TexArtCc;
            }
        }
    }
}

fn kdl_editor(ui: &mut egui::Ui, text: &mut String) {
    ui.add(egui::TextEdit::multiline(text).code_editor().desired_rows(KDL_EDITOR_ROWS).desired_width(f32::INFINITY));
}

fn effects_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let strings = strings(app);
    let state = &mut app.animation_definitions_panel;
    let Some(draft) = state.effects_draft.as_mut() else {
        return;
    };
    let (mut apply, mut close) = (false, false);
    egui::Window::new("Frame effects").collapsible(false).show(ctx, |ui| {
        ui.label(format!("Item {} (0x{:04X}), set {}, frame {}", draft.id, draft.id, draft.set, draft.frame));
        ui.weak("EffectDefinitionCollection KDL grammar: particle, sound, wait, random, ... One node per effect.");
        kdl_editor(ui, &mut draft.text);
        if let Some(error) = &draft.error {
            ui.colored_label(ERROR_COLOR, error);
        }
        ui.horizontal(|ui| {
            apply = ui.button("Apply").clicked();
            close = ui.button("Close").clicked();
        });
    });
    if apply {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        match editor.set_frame_effects_kdl(draft.id, draft.set, draft.frame, &draft.text, strings.as_deref()) {
            Ok(()) => close = true,
            Err(error) => draft.error = Some(error),
        }
    }
    if close {
        state.effects_draft = None;
    }
}

fn entry_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let strings = strings(app);
    let state = &mut app.animation_definitions_panel;
    let Some(draft) = state.entry_draft.as_mut() else {
        return;
    };
    let (mut apply, mut close) = (false, false);
    egui::Window::new("Animation definition KDL").show(ctx, |ui| {
        ui.label(format!("Item {} (0x{:04X}); Apply replaces the entry (undoable).", draft.id, draft.id));
        kdl_editor(ui, &mut draft.text);
        if let Some(error) = &draft.error {
            ui.colored_label(ERROR_COLOR, error);
        }
        ui.horizontal(|ui| {
            apply = ui.button("Apply").clicked();
            close = ui.button("Close").clicked();
        });
    });
    if apply {
        let Some(editor) = state.editor.as_mut() else {
            return;
        };
        match parse_single_kdl(&draft.text, strings.as_deref()) {
            Ok((parsed_id, _)) if parsed_id != draft.id => {
                draft.error = Some(format!("the text is item {parsed_id}, not {}; import it as a KDL file instead", draft.id));
            }
            Ok((_, definition)) => {
                editor.edits.break_coalescing();
                editor.edit(draft.id, AnimationDefinitionField::Structure, |slot| *slot = definition);
                editor.edits.break_coalescing();
                close = true;
            }
            Err(error) => draft.error = Some(error),
        }
    }
    if close {
        state.entry_draft = None;
    }
}
