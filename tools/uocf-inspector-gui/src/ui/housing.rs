//! Housing tab: viewer and editor of `housing.bin`, the house customization pieces of `MultiCollection.uop`.
//! The editing model lives in `logic::housing_editor`; field meanings in `uocf::enhanced::housing`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use uocf::classic::art::{static_art_id_for_source, ArtMap};
use uocf::classic::cliloc::Cliloc;
use uocf::enhanced::housing::{
    housing_entitlement, housing_feature_name, housing_slot_label, housing_tile_type_name, HousingConfidence,
    HousingEntitlement, HousingIssue, HousingIssueSeverity, HousingPiece, HousingPieces, HousingSlotList,
    HousingValidationContext, HOUSING_CLIENT_EXPANSION_FILTER, HOUSING_CLIENT_TILE_TYPES, HOUSING_FEATURES,
};
use uocf::enhanced::localized_strings::LocalizedStringsPackage;
use uocf::enhanced::multis::MultiCollection;

use crate::app::{concrete_art_tile_source, ArtSource, TileArtFileEntry, UopInspectorApp, ViewMode};
use crate::logic::housing_editor::{
    add_piece, add_slot, add_tile_type, categories_of, delete_piece, duplicate_piece, entry_category_tooltip,
    export_kdl, find_target, import_kdl, issue_target, kept_entry, move_piece, open_client_package,
    open_housing_bin, parse_id,
    remove_slot, remove_tile_type, save_housing_bin, save_multi_collection, HousingDocument, HousingDocumentSource,
    HousingEditKey, HousingField, HousingIssueTarget, HousingPieceFilter, HousingSelection,
};
use crate::ui::{multis, UOP_PATCHER_NOTE};

const TREE_PANEL_WIDTH: f32 = 380.0;
const VALIDATION_PANEL_HEIGHT: f32 = 170.0;
const TILE_FILTER_WIDTH: f32 = 90.0;
/// Largest side of a slot thumbnail; larger art is scaled down.
const THUMBNAIL_MAX_SIDE: f32 = 44.0;
const MULTI_PREVIEW_SIZE: egui::Vec2 = egui::vec2(440.0, 300.0);
const TILE_ID_HEX_DIGITS: usize = 4;
const FEATURE_MASK_HEX_DIGITS: usize = 8;
/// Drag speed of the id and number fields (typing is the main input).
const VALUE_DRAG_SPEED: f64 = 0.1;
/// The feature mask field is typed only: dragging a bit mask is meaningless.
const NO_DRAG_SPEED: f64 = 0.0;
/// Lua publishes categories and groups 1-based.
const LUA_INDEX_BASE: u32 = 1;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 200, 110);
const HIGHLIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 220, 64);
const ERROR_ICON: &str = "✖";
const WARNING_ICON: &str = "⚠";
const DIRTY_MARKER: &str = "*";

const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const HOUSING_BIN_FILTER: (&str, &[&str]) = ("housing.bin", &["bin"]);
const KDL_FILTER: (&str, &[&str]) = ("KDL text", &["kdl"]);
const DEFAULT_MULTI_COLLECTION_FILE_NAME: &str = uocf::enhanced::multis::MULTI_COLLECTION_UOP_NAME;
const DEFAULT_HOUSING_BIN_FILE_NAME: &str = "housing.bin";
const DEFAULT_KDL_FILE_NAME: &str = "housing.kdl";

const UNDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);

/// State of the Housing tab.
#[derive(Default)]
pub struct HousingPanelState {
    document: Option<HousingDocument>,
    /// The client pieces the document was opened from, to follow asset reloads while it is unchanged.
    client_pieces: Option<Arc<HousingPieces>>,
    /// Increased whenever another document is opened (the document revisions restart).
    document_generation: u64,
    selection: Option<HousingSelection>,
    /// Slot rows to highlight in the piece details (from a validation issue).
    highlighted_slot: Option<(HousingSlotList, u32)>,
    selected_issue: Option<usize>,
    filter: HousingPieceFilter,
    tile_filter_text: String,
    error: Option<String>,
    pending_open: Option<OpenRequest>,
    preview_multi: Option<u32>,
    /// Increased to rebuild the tree open state around the selection.
    reveal_revision: u64,
    scroll_to_selection: bool,
    strings: Option<StringsCache>,
    strings_generation: u64,
    visible: Option<VisibleCache>,
    validation: Option<ValidationCache>,
}

#[derive(Clone)]
enum OpenRequest {
    Client,
    HousingBin(PathBuf),
    Kdl(PathBuf),
}

/// Cliloc texts by number, from the loaded cliloc file and localized strings.
struct StringsCache {
    cliloc: Option<Arc<Cliloc>>,
    localized: Option<Arc<LocalizedStringsPackage>>,
    localized_file: Option<u64>,
    texts: Arc<HashMap<u32, String>>,
}

struct VisibleCache {
    key: (u64, u64, u64),
    filter: HousingPieceFilter,
    visible: Arc<Vec<Vec<usize>>>,
}

#[derive(PartialEq)]
struct ValidationKey {
    document: (u64, u64),
    strings_generation: u64,
    tileart: usize,
    /// Edits of the records in the Tile Metadata tab, which keep the records' address.
    tileart_revision: u64,
    art: usize,
    art_source: ArtSource,
    multis: usize,
    /// Edits of the multis in the Multis tab, which keep the collection's address.
    multis_revision: u64,
}

struct ValidationCache {
    key: ValidationKey,
    issues: Arc<Vec<HousingIssue>>,
    checked: String,
}

/// Tile id existence: tileart records when tileart.uop is loaded, else art in the Multis tab's art source.
#[derive(Clone)]
enum TileLookup {
    TileArt(Arc<Vec<TileArtFileEntry>>),
    Art(Arc<ArtMap>, ArtSource),
}

impl TileLookup {
    fn exists(&self, tile_id: u32) -> bool {
        match self {
            Self::TileArt(entries) => entries.binary_search_by_key(&tile_id, |file| file.entry.tile_id).is_ok(),
            Self::Art(art, source) => art.has_id_from_source(static_art_id_for_source(tile_id, *source), *source),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::TileArt(_) => "tile ids against tileart.uop",
            Self::Art(..) => "tile ids against the art of the Multis tab source (no tileart.uop loaded)",
        }
    }
}

/// Owned lookups of the loaded client data.
#[derive(Clone)]
struct Lookups {
    tiles: Option<TileLookup>,
    multis: Option<Arc<MultiCollection>>,
    texts: Arc<HashMap<u32, String>>,
}

impl Lookups {
    fn text(&self, cliloc: u32) -> Option<&str> {
        self.texts.get(&cliloc).map(String::as_str)
    }
}

fn lookups(app: &UopInspectorApp) -> Lookups {
    let tiles = app.ec_tileart_entries.clone().map(TileLookup::TileArt).or_else(|| {
        let client = app.client_data.as_ref()?;
        Some(TileLookup::Art(Arc::clone(&client.art), concrete_art_tile_source(app.selected_legacy_source)))
    });
    let texts = app
        .housing_panel
        .strings
        .as_ref()
        .map(|strings| Arc::clone(&strings.texts))
        .unwrap_or_default();
    Lookups { tiles, multis: app.multi_collection.clone(), texts }
}

fn same_arc<T>(left: &Option<Arc<T>>, right: &Option<Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn arc_address<T>(value: &Option<Arc<T>>) -> usize {
    value.as_ref().map_or(0, |value| Arc::as_ptr(value) as usize)
}

pub fn ui_housing(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    follow_client_pieces(app);
    refresh_strings(app);
    handle_shortcuts(app, ctx);

    egui::Panel::top("housing_toolbar").show(ui, |ui| toolbar(app, ui));

    if app.housing_panel.document.is_none() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No housing.bin loaded. Load a client with a MultiCollection.uop (Search Paths), or open a \
                     loose housing.bin or a KDL file.",
                );
            });
        });
        pending_open_window(app, ctx);
        return;
    }

    refresh_validation(app);
    egui::Panel::bottom("housing_validation")
        .resizable(true)
        .default_size(VALIDATION_PANEL_HEIGHT)
        .show(ui, |ui| validation_panel(app, ui));
    egui::Panel::left("housing_tree")
        .resizable(true)
        .default_size(TREE_PANEL_WIDTH)
        .show(ui, |ui| tree_panel(app, ui));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("housing_details_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| details_panel(app, ctx, ui));
    });
    app.housing_panel.scroll_to_selection = false;
    pending_open_window(app, ctx);
}

/// Opens the client pieces when nothing is open, and follows a reload of them while the open client document
/// is unchanged.
fn follow_client_pieces(app: &mut UopInspectorApp) {
    let Some(client) = app.housing_pieces.clone() else {
        return;
    };
    let state = &app.housing_panel;
    let follow = match &state.document {
        None => state.client_pieces.is_none(),
        Some(document) => {
            matches!(document.source(), HousingDocumentSource::ClientPackage(_))
                && !document.differs_from_opened()
                && !state.client_pieces.as_ref().is_some_and(|opened| Arc::ptr_eq(opened, &client))
        }
    };
    if follow {
        if let Err(error) = open_from_client(app) {
            app.housing_panel.error = Some(error);
        }
    }
}

fn refresh_strings(app: &mut UopInspectorApp) {
    let cliloc = app.cliloc.clone();
    let localized = app.localized_strings.clone();
    let localized_file = app.selected_localized_file_hash;
    let state = &mut app.housing_panel;
    if state.strings.as_ref().is_some_and(|strings| {
        same_arc(&strings.cliloc, &cliloc)
            && same_arc(&strings.localized, &localized)
            && strings.localized_file == localized_file
    }) {
        return;
    }

    let mut texts = HashMap::new();
    if let Some(package) = &localized {
        let files = package
            .files
            .iter()
            .filter(|file| localized_file.is_none_or(|hash| file.filename_hash == hash));
        for entry in files.flat_map(|file| &file.strings.entries) {
            texts.entry(entry.id).or_insert_with(|| entry.text.clone());
        }
    }
    if let Some(cliloc) = &cliloc {
        for entry in &cliloc.entries {
            if let Ok(number) = u32::try_from(entry.number) {
                texts.insert(number, entry.text.clone());
            }
        }
    }
    state.strings = Some(StringsCache { cliloc, localized, localized_file, texts: Arc::new(texts) });
    state.strings_generation += 1;
}

fn strings_label(app: &UopInspectorApp) -> &'static str {
    match (app.cliloc.is_some(), app.localized_strings.is_some()) {
        (true, true) => "cliloc and localized strings",
        (true, false) => "cliloc",
        (false, true) => "localized strings",
        (false, false) => "no clilocs loaded",
    }
}

fn handle_shortcuts(app: &mut UopInspectorApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let undo = ctx.input_mut(|input| input.consume_shortcut(&UNDO_SHORTCUT));
    let redo = ctx.input_mut(|input| input.consume_shortcut(&REDO_SHORTCUT));
    if undo {
        undo_redo(app, true);
    } else if redo {
        undo_redo(app, false);
    }
}

fn undo_redo(app: &mut UopInspectorApp, undo: bool) {
    let state = &mut app.housing_panel;
    let Some(document) = state.document.as_mut() else {
        return;
    };
    let changed = if undo { document.undo() } else { document.redo() };
    if changed {
        let pieces = document.pieces();
        state.selection = state.selection.and_then(|selection| selection.resolve(pieces));
    }
}

fn set_document(app: &mut UopInspectorApp, document: HousingDocument, client_pieces: Option<Arc<HousingPieces>>) {
    let message = format!("Opened housing.bin from {}.", document.source().label());
    let state = &mut app.housing_panel;
    state.document = Some(document);
    state.client_pieces = client_pieces;
    state.document_generation += 1;
    state.selection = None;
    state.highlighted_slot = None;
    state.selected_issue = None;
    state.preview_multi = None;
    state.error = None;
    state.visible = None;
    state.validation = None;
    app.status_message = message;
}

fn open_from_client(app: &mut UopInspectorApp) -> Result<(), String> {
    let (Some(pieces), Some(path)) = (app.housing_pieces.clone(), app.multi_collection_path.clone()) else {
        return Err("no client MultiCollection.uop with a housing.bin is loaded".to_string());
    };
    let document = HousingDocument::new((*pieces).clone(), HousingDocumentSource::ClientPackage(path));
    set_document(app, document, Some(pieces));
    Ok(())
}

/// Reads housing.bin again from the loaded client's MultiCollection.uop (picks up changes on disk).
fn reopen_client_package(app: &mut UopInspectorApp) -> Result<(), String> {
    let Some(path) = app.multi_collection_path.clone() else {
        return Err("no client MultiCollection.uop is loaded".to_string());
    };
    let document = open_client_package(&path)?;
    let client_pieces = app.housing_pieces.clone();
    set_document(app, document, client_pieces);
    Ok(())
}

fn request_open(app: &mut UopInspectorApp, request: OpenRequest) {
    if app.housing_panel.document.as_ref().is_some_and(HousingDocument::is_dirty) {
        app.housing_panel.pending_open = Some(request);
    } else {
        perform_open(app, request);
    }
}

fn perform_open(app: &mut UopInspectorApp, request: OpenRequest) {
    let result = match request {
        OpenRequest::Client => reopen_client_package(app),
        OpenRequest::HousingBin(path) => open_housing_bin(&path).map(|document| set_document(app, document, None)),
        OpenRequest::Kdl(path) => import_kdl(&path).map(|document| set_document(app, document, None)),
    };
    if let Err(error) = result {
        app.housing_panel.error = Some(format!("Open failed: {error}"));
    }
}

fn pending_open_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(request) = app.housing_panel.pending_open.clone() else {
        return;
    };
    let (mut discard, mut cancel) = (false, false);
    egui::Window::new("Discard unsaved housing changes?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("The edited housing.bin has unsaved changes. Opening another one discards them.");
            ui.horizontal(|ui| {
                discard = ui.button("Discard and open").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if discard || cancel {
        app.housing_panel.pending_open = None;
    }
    if discard {
        perform_open(app, request);
    }
}

fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let client_available = app.housing_pieces.is_some() && app.multi_collection_path.is_some();
    let save_source = app.multi_collection_path.clone();
    ui.horizontal_wrapped(|ui| {
        ui.label("Open:");
        if ui
            .add_enabled(client_available, egui::Button::new("Client MultiCollection.uop"))
            .on_hover_text("housing.bin of the loaded client's MultiCollection.uop.")
            .on_disabled_hover_text("No client MultiCollection.uop with a housing.bin is loaded.")
            .clicked()
        {
            request_open(app, OpenRequest::Client);
        }
        if ui.button("housing.bin...").clicked() {
            if let Some(path) = pick_file(HOUSING_BIN_FILTER) {
                request_open(app, OpenRequest::HousingBin(path));
            }
        }
        if ui.button("Import KDL...").clicked() {
            if let Some(path) = pick_file(KDL_FILTER) {
                request_open(app, OpenRequest::Kdl(path));
            }
        }

        let has_document = app.housing_panel.document.is_some();
        ui.separator();
        ui.label("Save:");
        if ui
            .add_enabled(has_document && save_source.is_some(), egui::Button::new("MultiCollection.uop..."))
            .on_hover_text(format!(
                "Writes a copy of the loaded client's MultiCollection.uop with only housing.bin replaced. The \
                 loaded package itself is refused as the target.\n{UOP_PATCHER_NOTE}"
            ))
            .on_disabled_hover_text("Needs a loaded client MultiCollection.uop as the source of the copy.")
            .clicked()
        {
            if let (Some(source), Some(path)) =
                (save_source.as_deref(), save_file(UOP_FILTER, DEFAULT_MULTI_COLLECTION_FILE_NAME))
            {
                save(app, |pieces| save_multi_collection(pieces, source, &path), &path);
            }
        }
        if ui.add_enabled(has_document, egui::Button::new("housing.bin...")).clicked() {
            if let Some(path) = save_file(HOUSING_BIN_FILTER, DEFAULT_HOUSING_BIN_FILE_NAME) {
                save(app, |pieces| save_housing_bin(pieces, &path), &path);
            }
        }
        if ui.add_enabled(has_document, egui::Button::new("Export KDL...")).clicked() {
            if let Some(path) = save_file(KDL_FILTER, DEFAULT_KDL_FILE_NAME) {
                save(app, |pieces| export_kdl(pieces, &path), &path);
            }
        }

        let Some(document) = app.housing_panel.document.as_ref() else {
            return;
        };
        let (can_undo, can_redo, can_revert) =
            (document.can_undo(), document.can_redo(), document.differs_from_opened());
        let dirty = document.is_dirty();
        let source = document.source().label();
        ui.separator();
        if ui.add_enabled(can_undo, egui::Button::new("Undo")).on_hover_text("Ctrl+Z").clicked() {
            undo_redo(app, true);
        }
        if ui.add_enabled(can_redo, egui::Button::new("Redo")).on_hover_text("Ctrl+Y").clicked() {
            undo_redo(app, false);
        }
        if ui
            .add_enabled(can_revert, egui::Button::new("Revert to loaded"))
            .on_hover_text("Back to the pieces as opened (undoable).")
            .clicked()
        {
            let state = &mut app.housing_panel;
            if let Some(document) = state.document.as_mut() {
                document.revert_to_opened();
                state.selection = state.selection.and_then(|selection| selection.resolve(document.pieces()));
            }
        }
        ui.separator();
        if dirty {
            ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
        }
        ui.weak(source);
    });
    if let Some(error) = app.housing_panel.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ERROR_COLOR, error);
            if ui.small_button("Dismiss").clicked() {
                app.housing_panel.error = None;
            }
        });
    }
}

fn pick_file((name, extensions): (&str, &[&str])) -> Option<PathBuf> {
    crate::dialog::file_dialog().add_filter(name, extensions).pick_file()
}

fn save_file((name, extensions): (&str, &[&str]), default_name: &str) -> Option<PathBuf> {
    crate::dialog::file_dialog()
        .add_filter(name, extensions)
        .set_file_name(default_name)
        .save_file()
}

fn save(app: &mut UopInspectorApp, write: impl FnOnce(&HousingPieces) -> Result<(), String>, path: &std::path::Path) {
    let Some(document) = app.housing_panel.document.as_mut() else {
        return;
    };
    match write(document.pieces()) {
        Ok(()) => {
            document.mark_saved();
            app.housing_panel.error = None;
            let message = format!("Saved the housing pieces to {}.", path.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.housing_panel.error = Some(format!("Save failed: {error}")),
    }
}

fn select(state: &mut HousingPanelState, selection: Option<HousingSelection>, reveal: bool) {
    if state.selection != selection {
        state.highlighted_slot = None;
        state.preview_multi = None;
        if let Some(document) = state.document.as_mut() {
            document.break_coalescing();
        }
    }
    state.selection = selection;
    if reveal {
        state.reveal_revision += 1;
        state.scroll_to_selection = true;
    }
}

fn visible_pieces(state: &mut HousingPanelState) -> Arc<Vec<Vec<usize>>> {
    let Some(document) = state.document.as_ref() else {
        return Arc::default();
    };
    let key = (state.document_generation, document.revision(), state.strings_generation);
    if let Some(cache) = state.visible.as_ref().filter(|cache| cache.key == key && cache.filter == state.filter) {
        return Arc::clone(&cache.visible);
    }
    let texts = state.strings.as_ref().map(|strings| Arc::clone(&strings.texts)).unwrap_or_default();
    let visible = Arc::new(state.filter.visible_pieces(document.pieces(), &|cliloc| texts.get(&cliloc).cloned()));
    state.visible = Some(VisibleCache { key, filter: state.filter.clone(), visible: Arc::clone(&visible) });
    visible
}

fn refresh_validation(app: &mut UopInspectorApp) {
    let lookups = lookups(app);
    let state = &app.housing_panel;
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let key = ValidationKey {
        document: (state.document_generation, document.revision()),
        strings_generation: state.strings_generation,
        tileart: arc_address(&app.ec_tileart_entries),
        tileart_revision: app.tileart_editor.edits.revision(),
        art: app.client_data.as_ref().map_or(0, |client| Arc::as_ptr(&client.art) as usize),
        art_source: app.selected_legacy_source,
        multis: arc_address(&app.multi_collection),
        multis_revision: app.multi_editor.edits.revision(),
    };
    if state.validation.as_ref().is_some_and(|cache| cache.key == key) {
        return;
    }

    let tile_exists = |id| lookups.tiles.as_ref().is_some_and(|tiles| tiles.exists(id));
    let multi_exists = |id| lookups.multis.as_ref().is_some_and(|multis| multis.get(id).is_some());
    let cliloc_exists = |id| lookups.texts.contains_key(&id);
    let has_strings = !lookups.texts.is_empty();
    let context = HousingValidationContext {
        tile_exists: lookups.tiles.is_some().then_some(&tile_exists as &dyn Fn(u32) -> bool),
        multi_exists: lookups.multis.is_some().then_some(&multi_exists as &dyn Fn(u32) -> bool),
        cliloc_exists: has_strings.then_some(&cliloc_exists as &dyn Fn(u32) -> bool),
    };
    let issues = document.pieces().validate(&context);
    let mut checked = vec!["client rules"];
    checked.push(lookups.tiles.as_ref().map_or("tile ids not checked (no tileart or art loaded)", TileLookup::label));
    checked.push(if lookups.multis.is_some() {
        "multi ids against MultiCollection.uop"
    } else {
        "multi ids not checked (no MultiCollection.uop loaded)"
    });
    checked.push(if has_strings { "tooltips against the loaded clilocs" } else { "tooltips not checked (no clilocs)" });
    let checked = format!("Checked: {}.", checked.join(", "));
    let state = &mut app.housing_panel;
    state.selected_issue = state.selected_issue.filter(|&index| index < issues.len());
    state.validation = Some(ValidationCache { key, issues: Arc::new(issues), checked });
}

fn validation_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &app.housing_panel;
    let Some(cache) = state.validation.as_ref() else {
        return;
    };
    let issues = Arc::clone(&cache.issues);
    let errors = issues.iter().filter(|issue| issue.severity() == HousingIssueSeverity::Error).count();
    let warnings = issues.len() - errors;
    ui.horizontal_wrapped(|ui| {
        ui.heading("Validation");
        ui.colored_label(if errors > 0 { ERROR_COLOR } else { OK_COLOR }, format!("{errors} errors"));
        ui.colored_label(if warnings > 0 { WARNING_COLOR } else { OK_COLOR }, format!("{warnings} warnings"));
        ui.weak(&cache.checked);
    });
    ui.separator();
    let selected_issue = state.selected_issue;
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("housing_validation_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if issues.is_empty() {
                ui.colored_label(OK_COLOR, "No issues.");
            }
            for (index, issue) in issues.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (icon, color) = match issue.severity() {
                        HousingIssueSeverity::Error => (ERROR_ICON, ERROR_COLOR),
                        HousingIssueSeverity::Warning => (WARNING_ICON, WARNING_COLOR),
                    };
                    ui.colored_label(color, icon);
                    if ui.selectable_label(selected_issue == Some(index), issue.to_string()).clicked() {
                        clicked = Some(index);
                    }
                });
            }
        });

    let Some(index) = clicked else {
        return;
    };
    let target = issue_target(&issues[index]);
    let state = &mut app.housing_panel;
    state.selected_issue = Some(index);
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let selection = find_target(document.pieces(), target);
    select(state, selection, true);
    state.highlighted_slot = match target {
        HousingIssueTarget::Piece { slot, .. } => slot,
        HousingIssueTarget::TileType(_) | HousingIssueTarget::Category { .. } => None,
    };
}

fn tree_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.heading("Pieces");
    let state = &mut app.housing_panel;
    ui.horizontal(|ui| {
        ui.label("Feature:");
        egui::ComboBox::from_id_salt("housing_feature_filter")
            .selected_text(feature_filter_label(state.filter.feature_bit))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.filter.feature_bit, None, "Any");
                for (bit, name) in HOUSING_FEATURES {
                    ui.selectable_value(&mut state.filter.feature_bit, Some(bit), format!("{name} ({bit:#X})"));
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Tile id:");
        ui.add(
            egui::TextEdit::singleline(&mut state.tile_filter_text)
                .hint_text("0x0009")
                .desired_width(TILE_FILTER_WIDTH),
        )
        .on_hover_text("Pieces using this tile id in a solid or window slot (decimal or 0x hex).");
        state.filter.tile_id = parse_id(&state.tile_filter_text);
        if state.filter.tile_id.is_none() && !state.tile_filter_text.trim().is_empty() {
            ui.colored_label(ERROR_COLOR, "invalid id");
        }
    });
    ui.horizontal(|ui| {
        ui.label("Text:");
        ui.add(egui::TextEdit::singleline(&mut state.filter.text).hint_text("cliloc text or number"))
            .on_hover_text("Matches the tooltip cliloc number or the category name text.");
    });
    let filter_active = state.filter.is_active();
    ui.horizontal(|ui| {
        if ui.add_enabled(filter_active, egui::Button::new("Clear filters")).clicked() {
            state.filter = HousingPieceFilter::default();
            state.tile_filter_text.clear();
        }
        if ui.button("Add tile type").clicked() {
            if let Some(document) = state.document.as_mut() {
                let selection = document.edit(None, add_tile_type);
                select(state, Some(selection), true);
            }
        }
    });
    ui.separator();

    let filter_active = state.filter.is_active();
    let visible = visible_pieces(state);
    let state = &app.housing_panel;
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let pieces = document.pieces();
    let texts = state.strings.as_ref().map(|strings| Arc::clone(&strings.texts)).unwrap_or_default();
    let shown: usize = visible.iter().map(Vec::len).sum();
    ui.label(format!("Showing {shown} of {} pieces", pieces.pieces().count()));
    ui.weak(format!("Texts: {}", strings_label(app)));

    let tree = TreeContext {
        pieces,
        texts: &texts,
        selection: state.selection,
        reveal_revision: state.reveal_revision,
        filter_active,
        scroll_to_selection: state.scroll_to_selection,
    };
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("housing_tree_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (entry_index, visible_pieces) in visible.iter().enumerate() {
                if filter_active && visible_pieces.is_empty() {
                    continue;
                }
                tree.tile_type_node(ui, entry_index, visible_pieces, &mut clicked);
            }
        });
    if clicked.is_some() {
        select(&mut app.housing_panel, clicked, false);
    }
}

struct TreeContext<'a> {
    pieces: &'a HousingPieces,
    texts: &'a HashMap<u32, String>,
    selection: Option<HousingSelection>,
    reveal_revision: u64,
    filter_active: bool,
    scroll_to_selection: bool,
}

impl TreeContext<'_> {
    fn selectable(&self, ui: &mut egui::Ui, selection: HousingSelection, text: impl Into<egui::WidgetText>) -> egui::Response {
        let selected = self.selection == Some(selection);
        let response = ui.selectable_label(selected, text);
        if selected && self.scroll_to_selection {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        response
    }

    fn default_open(&self, contains_selection: bool) -> bool {
        self.filter_active || (self.reveal_revision > 0 && contains_selection)
    }

    fn tile_type_node(
        &self,
        ui: &mut egui::Ui,
        entry_index: usize,
        visible: &[usize],
        clicked: &mut Option<HousingSelection>,
    ) {
        let entry = &self.pieces.tile_types[entry_index];
        let categories = categories_of(entry, visible);
        let mut header = format!(
            "{} {}: {} pieces, {} categories",
            entry.tile_type,
            tile_type_name(entry.tile_type),
            entry.pieces.len(),
            categories_of(entry, &(0..entry.pieces.len()).collect::<Vec<_>>()).len()
        );
        if kept_entry(self.pieces, entry.tile_type) != Some(entry_index) {
            header.push_str(" [ignored: repeated tile type]");
        }
        let id = ui.make_persistent_id(("housing_tile_type", entry_index, self.reveal_revision, self.filter_active));
        let contains_selection = self.selection.is_some_and(|selection| selection.entry() == entry_index);
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, self.default_open(contains_selection))
            .show_header(ui, |ui| {
                let selection = HousingSelection::TileType { entry: entry_index };
                if self.selectable(ui, selection, egui::RichText::new(header).strong()).clicked() {
                    *clicked = Some(selection);
                }
            })
            .body(|ui| {
                for (category, indices) in categories {
                    self.category_node(ui, entry_index, category, &indices, clicked);
                }
            });
    }

    fn category_node(
        &self,
        ui: &mut egui::Ui,
        entry_index: usize,
        category: u32,
        indices: &[usize],
        clicked: &mut Option<HousingSelection>,
    ) {
        let entry = &self.pieces.tile_types[entry_index];
        let (tooltip, entitlement) = category_info(self.pieces, entry_index, category);
        let name = tooltip
            .map(|cliloc| self.texts.get(&cliloc).cloned().unwrap_or_else(|| format!("cliloc {cliloc}")))
            .unwrap_or_else(|| "(no tooltip)".to_string());
        let expansion = entitlement.map(entitlement_short_label).unwrap_or_default();
        let header = format!("Category {category}: {name} [{expansion}] ({} pieces)", indices.len());
        let hover = format!(
            "Tooltip cliloc: {}\nEntitlement: {}",
            tooltip.map_or_else(|| "none".to_string(), |cliloc| cliloc.to_string()),
            entitlement.map_or_else(String::new, |entitlement| entitlement_long_label(entitlement, self.texts))
        );
        let id = ui.make_persistent_id((
            "housing_category",
            entry_index,
            category,
            self.reveal_revision,
            self.filter_active,
        ));
        let contains_selection = match self.selection {
            Some(HousingSelection::Category { entry, category: selected }) => entry == entry_index && selected == category,
            Some(HousingSelection::Piece { entry, piece }) => {
                entry == entry_index && entry_piece(entry, piece, self.pieces).is_some_and(|piece| piece.category == category)
            }
            _ => false,
        };
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, self.default_open(contains_selection))
            .show_header(ui, |ui| {
                let selection = HousingSelection::Category { entry: entry_index, category };
                if self.selectable(ui, selection, header).on_hover_text(hover).clicked() {
                    *clicked = Some(selection);
                }
            })
            .body(|ui| {
                for &index in indices {
                    let piece = &entry.pieces[index];
                    let selection = HousingSelection::Piece { entry: entry_index, piece: index };
                    let text = format!("Group {} (#{index}): {}", piece.group, slot_counts(piece));
                    if self.selectable(ui, selection, text).clicked() {
                        *clicked = Some(selection);
                    }
                }
            });
    }
}

fn entry_piece(entry: usize, piece: usize, pieces: &HousingPieces) -> Option<&HousingPiece> {
    pieces.tile_types.get(entry)?.pieces.get(piece)
}

/// Category name cliloc and entitlement: the client's values (`category_tooltip` / `category_entitlement`) for
/// the tile type entry it keeps, the same rules within the entry for an ignored repeated entry.
fn category_info(pieces: &HousingPieces, entry_index: usize, category: u32) -> (Option<u32>, Option<HousingEntitlement>) {
    let entry = &pieces.tile_types[entry_index];
    if kept_entry(pieces, entry.tile_type) == Some(entry_index) {
        return (
            pieces.category_tooltip(entry.tile_type, category),
            pieces.category_entitlement(entry.tile_type, category),
        );
    }
    let last_mask = entry.pieces.iter().rev().find(|piece| piece.category == category).map(|piece| piece.feature_mask);
    (entry_category_tooltip(entry, category), last_mask.map(housing_entitlement))
}

fn tile_type_name(tile_type: u32) -> &'static str {
    housing_tile_type_name(tile_type).unwrap_or("unknown")
}

fn feature_filter_label(bit: Option<u32>) -> String {
    match bit {
        None => "Any".to_string(),
        Some(bit) => housing_feature_name(bit).map_or_else(|| format!("{bit:#X}"), str::to_string),
    }
}

/// Names of the known bits of a mask, plus the unknown rest in hex.
fn feature_mask_names(mask: u32) -> String {
    let mut names: Vec<String> = HOUSING_FEATURES
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| name.to_string())
        .collect();
    let known = HOUSING_FEATURES.iter().fold(0, |known, (bit, _)| known | bit);
    if mask & !known != 0 {
        names.push(format!("unknown bits {:#X}", mask & !known));
    }
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

fn entitlement_short_label(entitlement: HousingEntitlement) -> String {
    entitlement
        .expansion
        .map_or_else(|| feature_mask_names(entitlement.feature_mask), str::to_string)
}

fn entitlement_long_label(entitlement: HousingEntitlement, texts: &HashMap<u32, String>) -> String {
    let text = texts.get(&entitlement.cliloc).map(String::as_str).unwrap_or("(text not loaded)");
    format!(
        "mask {:#X} ({}), entitlement cliloc {}: {text}",
        entitlement.feature_mask,
        entitlement_short_label(entitlement),
        entitlement.cliloc
    )
}

fn slot_counts(piece: &HousingPiece) -> String {
    HousingSlotList::ALL
        .into_iter()
        .filter(|&list| !piece.slots(list).is_empty())
        .map(|list| format!("{} {}", piece.slots(list).len(), list.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn confidence_name(confidence: HousingConfidence) -> &'static str {
    match confidence {
        HousingConfidence::Low => "low",
        HousingConfidence::Medium => "medium",
        HousingConfidence::High => "high",
    }
}

fn cliloc_text_label(ui: &mut egui::Ui, lookups: &Lookups, cliloc: u32) {
    if cliloc == 0 {
        ui.weak("none");
    } else if let Some(text) = lookups.text(cliloc) {
        ui.label(format!("\"{text}\""));
    } else if lookups.texts.is_empty() {
        ui.weak("(clilocs not loaded)");
    } else {
        ui.colored_label(WARNING_COLOR, "no string for this cliloc");
    }
}

fn thumbnail(ui: &mut egui::Ui, handle: &egui::TextureHandle) -> egui::Response {
    let size = handle.size_vec2();
    let scale = (THUMBNAIL_MAX_SIDE / size.x.max(size.y)).min(1.0);
    ui.add(egui::Image::new(handle).fit_to_exact_size(size * scale).sense(egui::Sense::click()))
}

fn jump_to_art(app: &mut UopInspectorApp, tile_id: u32) {
    let source = concrete_art_tile_source(app.selected_legacy_source);
    app.selected_tex_art_cc_id = Some(static_art_id_for_source(tile_id, source));
    app.view_mode = ViewMode::TexArtCc;
}

fn details_panel(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let state = &mut app.housing_panel;
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let selection = state.selection.and_then(|selection| selection.resolve(document.pieces()));
    state.selection = selection;
    match selection {
        None => {
            ui.label("Select a tile type, category or piece in the tree.");
        }
        Some(HousingSelection::TileType { entry }) => tile_type_details(app, ui, entry),
        Some(HousingSelection::Category { entry, category }) => category_details(app, ctx, ui, entry, category),
        Some(HousingSelection::Piece { entry, piece }) => piece_details(app, ctx, ui, entry, piece),
    }
}

fn tile_type_details(app: &mut UopInspectorApp, ui: &mut egui::Ui, entry_index: usize) {
    let state = &mut app.housing_panel;
    let Some(document) = state.document.as_mut() else {
        return;
    };
    let pieces = document.pieces();
    let entry = &pieces.tile_types[entry_index];
    let kept = kept_entry(pieces, entry.tile_type) == Some(entry_index);
    let category_count = categories_of(entry, &(0..entry.pieces.len()).collect::<Vec<_>>()).len();
    let piece_count = entry.pieces.len();
    let mut tile_type = entry.tile_type;

    ui.heading(format!("Tile type {} ({})", tile_type, tile_type_name(tile_type)));
    let mut changed = false;
    egui::Grid::new("housing_tile_type_grid").striped(true).show(ui, |ui| {
        ui.label("Tile type");
        ui.horizontal(|ui| {
            changed = ui.add(egui::DragValue::new(&mut tile_type).speed(VALUE_DRAG_SPEED)).changed();
            ui.label(tile_type_name(tile_type));
        });
        ui.end_row();
        ui.label("Pieces");
        ui.label(piece_count.to_string());
        ui.end_row();
        ui.label("Categories");
        ui.label(category_count.to_string());
        ui.end_row();
        ui.label("Entry");
        ui.label(format!("#{entry_index} in file order"));
        ui.end_row();
    });
    if !HOUSING_CLIENT_TILE_TYPES.contains(&tile_type) {
        ui.colored_label(
            WARNING_COLOR,
            format!(
                "Outside the client tile types {}..={}: the client ignores these pieces.",
                HOUSING_CLIENT_TILE_TYPES.start(),
                HOUSING_CLIENT_TILE_TYPES.end()
            ),
        );
    }
    if !kept {
        ui.colored_label(ERROR_COLOR, "A later entry has the same tile type: the client ignores this one.");
    }

    let mut action = None;
    ui.horizontal(|ui| {
        if ui.button("Add piece (new category)").clicked() {
            action = Some(TileTypeAction::AddPiece);
        }
        if ui.button("Remove tile type").on_hover_text("Removes the tile type with all its pieces (undoable).").clicked() {
            action = Some(TileTypeAction::Remove);
        }
    });

    if changed {
        let key = Some(HousingEditKey { entry: entry_index, piece: None, field: HousingField::TileType });
        document.edit(key, |pieces| pieces.tile_types[entry_index].tile_type = tile_type);
    }
    let selection = match action {
        None => return,
        Some(TileTypeAction::AddPiece) => document.edit(None, |pieces| add_piece(pieces, entry_index, None)),
        Some(TileTypeAction::Remove) => document.edit(None, |pieces| remove_tile_type(pieces, entry_index)),
    };
    select(state, selection, true);
}

enum TileTypeAction {
    AddPiece,
    Remove,
}

fn category_details(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, entry_index: usize, category: u32) {
    let lookups = lookups(app);
    let state = &app.housing_panel;
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let pieces = document.pieces();
    let entry = &pieces.tile_types[entry_index];
    let tile_type = entry.tile_type;
    let (tooltip, entitlement) = category_info(pieces, entry_index, category);
    let groups: Vec<(usize, u32, u32, String)> = entry
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, piece)| piece.category == category)
        .map(|(index, piece)| (index, piece.group, piece.feature_mask, slot_counts(piece)))
        .collect();
    // The client's preview tile of a category: its first nonzero solid id.
    let preview_tile = entry
        .pieces
        .iter()
        .filter(|piece| piece.category == category)
        .flat_map(|piece| &piece.solid)
        .map(|slot| slot.id)
        .find(|&id| id != 0);
    let preview_texture = preview_tile.and_then(|tile| multis::static_tile_thumbnail(app, ctx, tile));

    ui.heading(format!("{} category {category}", tile_type_name(tile_type)));
    egui::Grid::new("housing_category_grid").striped(true).show(ui, |ui| {
        ui.label("Tooltip");
        ui.horizontal(|ui| {
            ui.label(tooltip.map_or_else(|| "none".to_string(), |cliloc| cliloc.to_string()));
            cliloc_text_label(ui, &lookups, tooltip.unwrap_or(0));
        });
        ui.end_row();
        if let Some(entitlement) = entitlement {
            ui.label("Feature mask").on_hover_text("The mask of the category's last piece, as the client uses it.");
            ui.label(format!("{:#X} ({})", entitlement.feature_mask, feature_mask_names(entitlement.feature_mask)));
            ui.end_row();
            ui.label("Expansion");
            ui.label(entitlement.expansion.unwrap_or("(not a single known bit)"));
            ui.end_row();
            ui.label("Entitlement cliloc");
            ui.horizontal(|ui| {
                ui.label(entitlement.cliloc.to_string());
                cliloc_text_label(ui, &lookups, entitlement.cliloc);
            });
            ui.end_row();
        }
        ui.label("Preview tile");
        ui.horizontal(|ui| {
            match preview_tile {
                Some(tile) => ui.monospace(format!("{tile:#06X}")),
                None => ui.weak("none"),
            };
            if let Some(texture) = &preview_texture {
                thumbnail(ui, texture);
            }
        });
        ui.end_row();
        ui.label("Lua row");
        ui.monospace(format!("Type[{tile_type}].Category[{}]", category + LUA_INDEX_BASE));
        ui.end_row();
    });

    ui.separator();
    ui.strong(format!("Groups ({})", groups.len()));
    let mut clicked = None;
    egui::Grid::new("housing_category_groups").striped(true).show(ui, |ui| {
        for header in ["Group", "Piece", "Feature mask", "Slots"] {
            ui.strong(header);
        }
        ui.end_row();
        for (index, group, mask, counts) in &groups {
            if ui.selectable_label(false, format!("Group {group}")).clicked() {
                clicked = Some(HousingSelection::Piece { entry: entry_index, piece: *index });
            }
            ui.label(format!("#{index}"));
            ui.monospace(format!("{mask:#X}"));
            ui.label(counts);
            ui.end_row();
        }
    });
    let add_group = ui.button("Add group").on_hover_text("A new piece of this category with the next group number.").clicked();

    let state = &mut app.housing_panel;
    if add_group {
        if let Some(document) = state.document.as_mut() {
            clicked = document.edit(None, |pieces| add_piece(pieces, entry_index, Some(category)));
        }
    }
    if clicked.is_some() {
        select(state, clicked, true);
    }
}

enum PieceAction {
    Duplicate,
    Delete,
    Move(isize),
    AddGroup,
}

/// Slot row data fetched before drawing (thumbnails need the app mutably).
struct SlotRowView {
    thumbnail: Option<egui::TextureHandle>,
}

fn piece_details(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, entry_index: usize, piece_index: usize) {
    let lookups = lookups(app);
    let state = &app.housing_panel;
    let Some(document) = state.document.as_ref() else {
        return;
    };
    let pieces = document.pieces();
    let entry = &pieces.tile_types[entry_index];
    let tile_type = entry.tile_type;
    let piece_count = entry.pieces.len();
    let original = entry.pieces[piece_index].clone();
    let (category_tooltip, entitlement) = category_info(pieces, entry_index, original.category);
    let highlighted_slot = state.highlighted_slot;
    let first_multi = original.multis.first().map(|slot| slot.id);
    let preview_multi = state.preview_multi.or(first_multi);
    let can_jump_to_art = app.client_data.is_some();
    let rows: Vec<Vec<SlotRowView>> = HousingSlotList::ALL
        .into_iter()
        .map(|list| {
            original
                .slots(list)
                .iter()
                .map(|slot| SlotRowView {
                    thumbnail: list
                        .holds_tile_ids()
                        .then(|| multis::static_tile_thumbnail(app, ctx, slot.id))
                        .flatten(),
                })
                .collect()
        })
        .collect();

    let mut piece = original.clone();
    let mut edit_key = None;
    let mut action = None;
    let mut jump_art = None;
    let mut show_multi = None;
    let mut new_preview = None;
    let field_key = |field| Some(HousingEditKey { entry: entry_index, piece: Some(piece_index), field });

    ui.heading(format!(
        "{} category {} group {}",
        tile_type_name(tile_type),
        original.category,
        original.group
    ));
    ui.weak(format!("Piece #{piece_index} of {piece_count} in tile type entry #{entry_index}"));
    ui.horizontal(|ui| {
        if ui.button("Duplicate").clicked() {
            action = Some(PieceAction::Duplicate);
        }
        if ui.button("Delete").clicked() {
            action = Some(PieceAction::Delete);
        }
        if ui.add_enabled(piece_index > 0, egui::Button::new("Move up")).clicked() {
            action = Some(PieceAction::Move(-1));
        }
        if ui.add_enabled(piece_index + 1 < piece_count, egui::Button::new("Move down")).clicked() {
            action = Some(PieceAction::Move(1));
        }
        if ui
            .button("Add group")
            .on_hover_text("A new piece of this category with the next group number.")
            .clicked()
        {
            action = Some(PieceAction::AddGroup);
        }
    });
    ui.separator();

    egui::Grid::new("housing_piece_fields").striped(true).num_columns(2).show(ui, |ui| {
        ui.label("Category").on_hover_text("Row of the customization window (Lua: index + 1).");
        if ui.add(egui::DragValue::new(&mut piece.category).speed(VALUE_DRAG_SPEED)).changed() {
            edit_key = field_key(HousingField::Category);
        }
        ui.end_row();

        ui.label("Group").on_hover_text("Sub-row of the category (Lua: index + 1).");
        if ui.add(egui::DragValue::new(&mut piece.group).speed(VALUE_DRAG_SPEED)).changed() {
            edit_key = field_key(HousingField::Group);
        }
        ui.end_row();

        ui.label("Feature mask");
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let mask = egui::DragValue::new(&mut piece.feature_mask)
                    .hexadecimal(FEATURE_MASK_HEX_DIGITS, false, true)
                    .prefix("0x")
                    .speed(NO_DRAG_SPEED);
                if ui.add(mask).changed() {
                    edit_key = field_key(HousingField::FeatureMask);
                }
                ui.label(feature_mask_names(piece.feature_mask));
            });
            ui.horizontal_wrapped(|ui| {
                for (bit, name) in HOUSING_FEATURES {
                    let mut set = piece.feature_mask & bit != 0;
                    if ui.checkbox(&mut set, name).on_hover_text(format!("{bit:#X}")).changed() {
                        piece.feature_mask ^= bit;
                        edit_key = field_key(HousingField::FeatureMask);
                    }
                }
            });
            if piece.feature_mask & HOUSING_CLIENT_EXPANSION_FILTER == 0 {
                ui.colored_label(WARNING_COLOR, "No bit inside the client expansion filter: the client skips the piece.");
            } else if piece.feature_mask & !HOUSING_CLIENT_EXPANSION_FILTER != 0 {
                ui.colored_label(WARNING_COLOR, "Bits outside the client expansion filter.");
            }
        });
        ui.end_row();

        ui.label("Tooltip cliloc").on_hover_text("Category name; the client uses the first nonzero tooltip of the category.");
        ui.horizontal(|ui| {
            if ui.add(egui::DragValue::new(&mut piece.tooltip_cliloc).speed(VALUE_DRAG_SPEED)).changed() {
                edit_key = field_key(HousingField::Tooltip);
            }
            cliloc_text_label(ui, &lookups, piece.tooltip_cliloc);
        });
        ui.end_row();

        ui.label("Category name");
        ui.horizontal(|ui| {
            ui.label(category_tooltip.map_or_else(|| "none".to_string(), |cliloc| cliloc.to_string()));
            cliloc_text_label(ui, &lookups, category_tooltip.unwrap_or(0));
        });
        ui.end_row();

        if let Some(entitlement) = entitlement {
            ui.label("Category entitlement")
                .on_hover_text("From the mask of the category's last piece, as the client computes it.");
            ui.vertical(|ui| {
                ui.label(format!(
                    "mask {:#X}: {}",
                    entitlement.feature_mask,
                    entitlement_short_label(entitlement)
                ));
                ui.horizontal(|ui| {
                    ui.label(format!("cliloc {}", entitlement.cliloc));
                    cliloc_text_label(ui, &lookups, entitlement.cliloc);
                });
            });
            ui.end_row();
        }
    });

    let mut add_list = None;
    let mut remove_row = None;
    for (list_index, list) in HousingSlotList::ALL.into_iter().enumerate() {
        let expected = list.expected_tile_type().is_none_or(|expected| expected == tile_type);
        if !expected && piece.slots(list).is_empty() {
            continue;
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(format!("{} list ({} slots)", list.name(), piece.slots(list).len()));
            if !expected {
                ui.colored_label(WARNING_COLOR, format!("only used by {}", tile_type_name(list.expected_tile_type().unwrap_or(0))));
            }
            if ui.small_button("Add slot").clicked() {
                add_list = Some(list);
            }
        });
        let slot_count = piece.slots(list).len();
        if slot_count == 0 {
            continue;
        }
        egui::Grid::new(("housing_slots", list.name())).striped(true).show(ui, |ui| {
            let id_header = if list.holds_tile_ids() { "Tile id" } else { "Multi id" };
            for header in ["Slot", "Label", id_header, "Preview", "Status", ""] {
                ui.strong(header);
            }
            ui.end_row();
            for row in 0..slot_count {
                let slot = &mut piece.slots_mut(list)[row];
                let highlighted = highlighted_slot == Some((list, slot.slot));
                ui.horizontal(|ui| {
                    if highlighted {
                        ui.colored_label(HIGHLIGHT_COLOR, "▶");
                    }
                    if ui.add(egui::DragValue::new(&mut slot.slot).speed(VALUE_DRAG_SPEED)).changed() {
                        edit_key = field_key(HousingField::Slot { list, row });
                    }
                });
                match housing_slot_label(tile_type, list, slot.slot) {
                    Some(label) => {
                        ui.label(format!("{} [{}]", label.label, confidence_name(label.confidence)))
                            .on_hover_text(format!("Slot meaning, confidence {}.", confidence_name(label.confidence)));
                    }
                    None => {
                        ui.weak("-");
                    }
                }
                let id = egui::DragValue::new(&mut slot.id).speed(VALUE_DRAG_SPEED);
                let id = if list.holds_tile_ids() { id.hexadecimal(TILE_ID_HEX_DIGITS, false, true).prefix("0x") } else { id };
                if ui.add(id).changed() {
                    edit_key = field_key(HousingField::SlotId { list, row });
                }
                let slot_id = slot.id;
                if list.holds_tile_ids() {
                    match rows[list_index].get(row).and_then(|view| view.thumbnail.as_ref()) {
                        Some(texture) => {
                            if thumbnail(ui, texture).on_hover_text("Show in the Art tab").clicked() && can_jump_to_art {
                                jump_art = Some(slot_id);
                            }
                        }
                        None => {
                            ui.weak("no art");
                        }
                    }
                    match &lookups.tiles {
                        None => ui.weak("not checked"),
                        Some(tiles) if tiles.exists(slot_id) => ui.colored_label(OK_COLOR, "ok"),
                        Some(_) => ui.colored_label(ERROR_COLOR, "missing tile"),
                    };
                } else {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(preview_multi == Some(slot_id), "Preview").clicked() {
                            new_preview = Some(slot_id);
                        }
                        if ui.small_button("Multis tab").clicked() {
                            show_multi = Some(slot_id);
                        }
                    });
                    match &lookups.multis {
                        None => ui.weak("not checked"),
                        Some(multis) if multis.get(slot_id).is_some() => ui.colored_label(OK_COLOR, "ok"),
                        Some(_) => ui.colored_label(ERROR_COLOR, "missing multi"),
                    };
                }
                ui.horizontal(|ui| {
                    if list.holds_tile_ids()
                        && ui.add_enabled(can_jump_to_art, egui::Button::new("Art").small()).clicked()
                    {
                        jump_art = Some(slot_id);
                    }
                    if ui.small_button("Remove").clicked() {
                        remove_row = Some((list, row));
                    }
                });
                ui.end_row();
            }
        });

        if list == HousingSlotList::Multi {
            if let Some(multi_id) = new_preview.or(preview_multi).filter(|id| piece.multis.iter().any(|slot| slot.id == *id)) {
                ui.label(format!("Multi {multi_id} (drawn parts):"));
                ui.push_id(("housing_multi_preview", multi_id), |ui| {
                    ui.allocate_ui(MULTI_PREVIEW_SIZE, |ui| {
                        ui.set_min_size(MULTI_PREVIEW_SIZE);
                        if !multis::draw_collection_multi_preview(app, ctx, ui, multi_id) {
                            ui.weak("Not in the loaded MultiCollection.uop.");
                        }
                    });
                });
            }
        }
    }

    if let Some(list) = add_list {
        add_slot(&mut piece, list);
        edit_key = None;
    }
    if let Some((list, row)) = remove_row {
        remove_slot(&mut piece, list, row);
        edit_key = None;
    }

    let state = &mut app.housing_panel;
    if new_preview.is_some() {
        state.preview_multi = new_preview;
    }
    let Some(document) = state.document.as_mut() else {
        return;
    };
    if piece != original {
        if edit_key.is_none() {
            document.break_coalescing();
        }
        document.edit(edit_key, |pieces| pieces.tile_types[entry_index].pieces[piece_index] = piece);
    }
    if let Some(action) = action {
        let selection = document.edit(None, |pieces| match action {
            PieceAction::Duplicate => duplicate_piece(pieces, entry_index, piece_index),
            PieceAction::Delete => delete_piece(pieces, entry_index, piece_index),
            PieceAction::Move(delta) => move_piece(pieces, entry_index, piece_index, delta),
            PieceAction::AddGroup => add_piece(pieces, entry_index, Some(original.category)),
        });
        select(state, selection, true);
    }
    if let Some(tile_id) = jump_art {
        jump_to_art(app, tile_id);
    } else if let Some(multi_id) = show_multi {
        multis::show_multi_in_multis_tab(app, ctx, multi_id);
    }
}
