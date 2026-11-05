//! Effects tab: browser, editor and previewer of the client effects: `EffectDefinitionCollection.uop` (the
//! effects the 0xC7 packet plays), the effects of tileart records and the files of `EffectTexture.uop` (Chaos
//! `.ems` scripts, textures, NIF scenes). Chaos and NIF effects play live with particle-sim. The editing model
//! lives in `logic::effects_editor`, the files in `logic::effect_files`, NIF editing in `logic::nif_editor` (its
//! UI in `ui::effects_nif`), the preview in `logic::effect_preview`.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use eframe::egui;
use uocf::enhanced::effect_definition::{
    EffectDefinition, EffectEntryKey, EffectIssueSeverity, EffectValidationContext, PacketEffectPath, ParticleEffect,
    PropertyFlagMode,
    EFFECT_DEFINITION_COLLECTION_UOP_NAME, PARTICLE_ATTACH_PROJECTILE_ONLY, RANDOM_WEIGHT_TOTAL,
};
use uocf::enhanced::effect_texture::{effect_texture_hash, effect_texture_path, EFFECT_TEXTURE_UOP_NAME};
use uocf::enhanced::string_dictionary::UoStringDictionary;

use crate::app::{TileArtFileEntry, UopInspectorApp, ViewMode};
use crate::logic::effect_files::{
    canonical_script, check_script, decode_texture, latin1_decode, latin1_encode, read_import, EffectFiles, FileFilter,
    FileKind, FileRow, FileStatus, ScriptIssue, ScriptSeverity, IMPORTABLE_EXTENSIONS,
};
use crate::logic::effect_preview::{
    contact_sheet, export_times, fit_bounds, load_hue_atlas, preview_view, render_canvas, render_frames,
    write_png_sequence, CameraOrbit, LoopMode, PreviewInput, PreviewScene, PreviewSimulation, SimulationSettings, ViewSettings,
    WorldBounds, DEFAULT_EXPORT_FRAMES, DEFAULT_MOVE_SPEED, LIGHTING_NOTE, MAX_CAMERA_PITCH_DEGREES,
    MAX_CAMERA_YAW_DEGREES, MAX_EXPORT_FRAMES, MAX_LOD_LEVEL, MAX_MOVE_SPEED, MAX_PLAYBACK_SPEED, MAX_TIMELINE_S,
    MAX_ZOOM, MIN_CAMERA_PITCH_DEGREES, MIN_CAMERA_YAW_DEGREES, MIN_PLAYBACK_SPEED, MIN_TIMELINE_S, MIN_ZOOM,
    PREVIEW_RESOLUTION_PX,
};
use crate::logic::effect_gltf::{export_effect as export_gltf_effect, glb_file_name, write_glb, GltfSources, GLB_FILTER};
use crate::logic::effects_editor::{
    accepts_children, add_definition, add_entry, add_keyed_list, build_file_users, child_list_label, child_lists,
    definition_at, definition_at_mut, duplicate_definition, duplicate_entry, export_kdl, import_kdl, issue_selection,
    describe_packet_effect, move_definition, next_free_entry_id, open_collection, packet_routes_for_collection_id, parse_id, particle_name,
    remove_definition, remove_entry, remove_keyed_list, same_file, save_collection, set_entry_id, BrowseView,
    ChildList, CollectionSource, DefinitionFilter, DefinitionKind, EffectsDocument, EffectsEditKey, EffectsSelection,
    FileOverride, FileUser, NameIndex, PacketRoute, ParticleKindFilter, PathStep, PACKET_UNREACHABLE_NOTE,
};
use crate::ui::unsaved_changes::TabEditor;
use crate::ui::{arrow_delta, move_selection, UOP_PATCHER_NOTE};

const BROWSE_PANEL_WIDTH: f32 = 360.0;
const PREVIEW_PANEL_WIDTH: f32 = 420.0;
const VALIDATION_PANEL_HEIGHT: f32 = 150.0;
const ID_FILTER_WIDTH: f32 = 90.0;
const SCRIPT_EDITOR_ROWS: usize = 24;
/// Largest side of a texture thumbnail in the file details.
const THUMBNAIL_MAX_SIDE: f32 = 256.0;
/// Top-level definitions named in a list row before "...".
const SUMMARY_ITEMS: usize = 3;
const TILE_ID_HEX_DIGITS: usize = 4;
const MASK_HEX_DIGITS: usize = 8;
const VALUE_DRAG_SPEED: f64 = 0.1;
const FLOAT_DRAG_SPEED: f64 = 0.01;
/// Masks are typed, not dragged.
const NO_DRAG_SPEED: f64 = 0.0;
/// Camera turn per dragged point (secondary button) over the preview, degrees.
const ORBIT_DEGREES_PER_POINT: f32 = 0.5;
const SEED_DRAG_SPEED: f64 = 1.0;
const MS_PER_SECOND: f32 = 1000.0;
/// Decimals of the seconds shown on the timeline.
const TIMELINE_DECIMALS: usize = 2;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const NOTE_COLOR: egui::Color32 = egui::Color32::from_rgb(140, 170, 220);
const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 200, 110);
const ERROR_LINE_BACKGROUND: egui::Color32 = egui::Color32::from_rgba_premultiplied(110, 30, 30, 110);
const WARNING_LINE_BACKGROUND: egui::Color32 = egui::Color32::from_rgba_premultiplied(100, 80, 20, 90);
const ERROR_ICON: &str = "✖";
const WARNING_ICON: &str = "⚠";
const NOTE_ICON: &str = "ℹ";
const DIRTY_MARKER: &str = "*";
const PLAY_ICON: &str = "▶";
const PAUSE_ICON: &str = "⏸";
const RESTART_ICON: &str = "⏮";
/// Largest object hue the preview takes (object hues are 16-bit; the atlas holds 1..=`MAX_EC_HUES`).
const MAX_PREVIEW_HUE: u32 = u16::MAX as u32;
/// Gradient samples of a hue swatch.
const HUE_SWATCH_SAMPLES: usize = 16;
/// Size of a hue swatch, points.
const HUE_SWATCH_SIZE: egui::Vec2 = egui::vec2(96.0, 14.0);
/// Height of the hue picker list, points.
const HUE_PICKER_HEIGHT: f32 = 320.0;
const HUE_PICKER_ROW_HEIGHT: f32 = 18.0;
const HUE_NOTE: &str = "The hue of the object the effect plays on. Only NIF particles whose definition sets hued \
    take it (UOHuedParticleEffectNif: half speed, drawn offscreen through the Hues.uop atlas onto a camera-facing \
    quad); Chaos particles never do. Collection entries played by 0xC7 have no object and are never hued; the item \
    of a packet's tile effect or projectile takes the packet hue + 1, which reaches the hued particles of its \
    tileart effects. Animation frame effects take the animated object's hue. PropertyFlag definitions keyed by \
    the hue (property 7) play the case for this hue, else their default children.";

/// Share of the preview's height for the picture and its transport controls, until the user drags the line under them.
const PREVIEW_VIEW_DEFAULT_SHARE: f32 = 0.55;
const PREVIEW_VIEW_MIN_HEIGHT: f32 = 120.0;
const MIN_PREVIEW_IMAGE_SIDE: f32 = 48.0;
/// Rows under the picture: the transport, the time and particle count, and the warning that nothing plays.
const PREVIEW_CONTROL_ROWS: f32 = 3.0;
const FILES_LOAD_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FILES_LOAD_HOVER: &str = "Every file is unpacked once, to learn its kind and name. The tab works meanwhile; the \
    files, their previews and the checks that need them appear when it ends.";
const COLLECTION_SOURCE_NEEDED: &str = "Needs an EffectDefinitionCollection.uop as the source of the copy.";
const GUIDE_HINT: &str = "What each definition does, and how this tab works.";

const UOP_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);
const KDL_FILTER: (&str, &[&str]) = ("KDL text", &["kdl"]);
const EMS_FILTER: (&str, &[&str]) = ("Chaos script", &["ems"]);
const PNG_FILTER: (&str, &[&str]) = ("PNG image", &["png"]);
const IMPORT_FILTER_NAME: &str = "Effect file";
const DEFAULT_KDL_FILE_NAME: &str = "effects.kdl";
const DEFAULT_CONTACT_SHEET_FILE_NAME: &str = "effect_preview.png";
const HEADER_HASH_NOTE: &str = "EffectTexture.uop entries carry a signature header that cannot be recreated. The \
    client never checks it, so replaced and added entries get a fresh timestamp header instead.";

const UNDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y);

/// An `EffectTexture.uop` being read on a worker thread: opening one unpacks every file, which takes seconds.
struct FilesLoad {
    path: PathBuf,
    result: mpsc::Receiver<Result<EffectFiles, String>>,
}

/// State of the Effects tab.
#[derive(Default)]
pub struct EffectsPanelState {
    guide_open: bool,
    document: EffectsDocument,
    files: Option<Arc<EffectFiles>>,
    /// The `EffectTexture.uop` being read.
    files_load: Option<FilesLoad>,
    /// Increased whenever another collection is opened (the document revisions restart).
    document_generation: u64,
    /// Increased whenever another `EffectTexture.uop` is opened.
    files_generation: u64,
    /// The client directory the tab loaded on its own, so it does so once per directory.
    auto_loaded: Option<PathBuf>,
    view: BrowseView,
    selection: Option<EffectsSelection>,
    scroll_to_selection: bool,
    entry_filter: DefinitionFilter,
    tileart_filter: DefinitionFilter,
    file_filter: FileFilter,
    error: Option<String>,
    pending_open: Option<OpenRequest>,
    add_kind: Option<DefinitionKind>,
    new_entry_id: String,
    /// The packet field of [`Self::packet_lookup`].
    packet_lookup_path: PacketEffectPath,
    /// A 0xC7 packet effect value typed to find the entry it plays.
    packet_lookup: String,
    /// The id typed for the selected entry, for the entry index and id it was taken from.
    id_text: String,
    id_text_for: Option<(usize, u32)>,
    /// The particle name typed for the selected particle, for the selection and name index it was taken from.
    name_text: String,
    name_text_for: Option<(EffectsSelection, u32)>,
    name_index: Option<(usize, Arc<NameIndex>)>,
    entries_cache: Option<ListCache>,
    tileart_cache: Option<ListCache>,
    files_cache: Option<FilesCache>,
    users_cache: Option<UsersCache>,
    validation: Option<ValidationCache>,
    script_issue_cache: HashMap<u64, ScriptIssueCacheEntry>,
    selected_issue: Option<usize>,
    script_editor: Option<ScriptEditor>,
    thumbnails: HashMap<u64, Thumbnail>,
    nif_editor: super::effects_nif::NifEditorState,
    preview: PreviewPanel,
}

#[derive(Clone)]
enum OpenRequest {
    /// Both packages of a client directory.
    Client(PathBuf),
    Collection(PathBuf),
    Kdl(PathBuf),
    Files(PathBuf),
}

impl OpenRequest {
    fn replaces_collection(&self) -> bool {
        !matches!(self, Self::Files(_))
    }

    fn replaces_files(&self) -> bool {
        matches!(self, Self::Client(_) | Self::Files(_))
    }
}

/// Visible rows of the entry or tileart list with their labels.
struct ListCache {
    key: u64,
    rows: Arc<Vec<(u32, String)>>,
}

struct FilesCache {
    key: (u64, u64),
    rows: Arc<Vec<FileRow>>,
    filter: FileFilter,
    visible: Arc<Vec<usize>>,
}

struct UsersCache {
    key: u64,
    users: Arc<HashMap<u64, Vec<FileUser>>>,
}

struct ScriptIssueCacheEntry {
    payload: usize,
    existence: u64,
    issues: Arc<Vec<ScriptIssue>>,
}

/// One row of the validation panel.
struct PanelIssue {
    severity: ScriptSeverity,
    text: String,
    target: Option<EffectsSelection>,
}

struct ValidationCache {
    key: u64,
    issues: Arc<Vec<PanelIssue>>,
    checked: String,
}

/// The text of the selected Chaos script.
struct ScriptEditor {
    hash: u64,
    name: String,
    text: String,
    /// The payload the text was taken from or last stored as, to notice undo and redo.
    source: Option<Arc<[u8]>>,
    issues: Vec<ScriptIssue>,
    encode_error: Option<String>,
}

struct Thumbnail {
    payload: usize,
    texture: Result<egui::TextureHandle, String>,
}

/// What the preview shows.
#[derive(Debug, Clone, PartialEq)]
enum PreviewTarget {
    Entry(usize),
    Definition(usize, Vec<PathStep>),
    TileArt(u32, Vec<PathStep>),
    Script(u64),
}

#[derive(Debug, Clone, PartialEq)]
struct PreviewKey {
    target: PreviewTarget,
    document: (u64, u64),
    files: (u64, u64),
    strings: usize,
    tileart: (usize, u64),
    settings: SimulationSettings,
}

/// The lower part of the preview panel: the picture above it keeps its place.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PreviewSection {
    #[default]
    Settings,
    Parts,
    Export,
}

struct PreviewPanel {
    /// What the lower part of the panel shows.
    section: PreviewSection,
    settings: SimulationSettings,
    view: ViewSettings,
    moving: bool,
    move_speed: f32,
    duration_override: bool,
    duration_s: f32,
    playing: bool,
    repeat: bool,
    speed: f32,
    /// Playback clock, milliseconds.
    clock_ms: f32,
    key: Option<PreviewKey>,
    label: String,
    scene: Option<Arc<PreviewScene>>,
    simulation: Option<PreviewSimulation>,
    bounds: WorldBounds,
    /// Increased on every scene rebuild, for the render stamp.
    build: u64,
    texture: Option<egui::TextureHandle>,
    /// The scene build, the time, the view and whether the hue atlas was there.
    rendered: Option<(u64, u32, ViewSettings, bool)>,
    export_frames: usize,
    /// The hue atlas of a client directory, loaded when a hue is first set.
    hue_atlas: Option<(PathBuf, Result<Arc<particle_sim::raster::RasterTexture>, String>)>,
    /// Filter of the hue picker (number or name).
    hue_filter: String,
    /// glTF export: bake the particle simulation into per-frame meshes.
    gltf_bake: bool,
}

impl PreviewPanel {
    /// The hue atlas of `client_dir`, loading it once.
    fn hue_atlas(&mut self, client_dir: Option<&Path>) -> Option<&Result<Arc<particle_sim::raster::RasterTexture>, String>> {
        let dir = client_dir?;
        if self.hue_atlas.as_ref().is_none_or(|(loaded, _)| loaded != dir) {
            self.hue_atlas = Some((dir.to_path_buf(), load_hue_atlas(dir).map(Arc::new)));
        }
        self.hue_atlas.as_ref().map(|(_, atlas)| atlas)
    }

    /// The loaded hue atlas, if any.
    fn loaded_hue_atlas(&self) -> Option<Arc<particle_sim::raster::RasterTexture>> {
        self.hue_atlas.as_ref().and_then(|(_, atlas)| atlas.as_ref().ok()).cloned()
    }
}

impl Default for PreviewPanel {
    fn default() -> Self {
        Self {
            section: PreviewSection::default(),
            settings: SimulationSettings::default(),
            view: ViewSettings::default(),
            moving: false,
            move_speed: DEFAULT_MOVE_SPEED,
            duration_override: false,
            duration_s: 1.0,
            playing: true,
            repeat: true,
            speed: 1.0,
            clock_ms: 0.0,
            key: None,
            label: String::new(),
            scene: None,
            simulation: None,
            bounds: [particle_sim::Vec3::ZERO; 2],
            build: 0,
            texture: None,
            rendered: None,
            export_frames: DEFAULT_EXPORT_FRAMES,
            hue_atlas: None,
            hue_filter: String::new(),
            gltf_bake: false,
        }
    }
}

/// Owned lookups of the loaded client data.
#[derive(Clone)]
struct Lookups {
    strings: Option<Arc<UoStringDictionary>>,
    tileart: Option<Arc<Vec<TileArtFileEntry>>>,
    /// Edits of the records in the Tile Metadata tab, which keep the records' address.
    tileart_revision: u64,
    files: Option<Arc<EffectFiles>>,
    client_dir: Option<PathBuf>,
}

impl Lookups {
    /// Cache key of the tileart records.
    fn tileart_key(&self) -> (usize, u64) {
        (arc_address(&self.tileart), self.tileart_revision)
    }

    fn particle_name(&self, particle: &ParticleEffect) -> Option<String> {
        particle_name(particle, self.strings.as_deref())
    }

    fn tileart(&self, tile_id: u32) -> Option<&TileArtFileEntry> {
        let entries = self.tileart.as_ref()?;
        entries.binary_search_by_key(&tile_id, |file| file.entry.tile_id).ok().map(|index| &entries[index])
    }

    /// The tile name: the string dictionary index of the record, shifted like particle names.
    fn tile_name(&self, entry: &TileArtFileEntry) -> Option<String> {
        let index = entry.entry.string_dict_off.checked_sub(1)?;
        self.strings.as_ref()?.get_string(index as usize).map(str::to_owned)
    }

    /// The loaded client's two effect packages: never overwritten.
    fn client_paths(&self) -> Vec<PathBuf> {
        self.client_dir.iter().flat_map(|dir| [dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME), dir.join(EFFECT_TEXTURE_UOP_NAME)]).collect()
    }
}

fn lookups(app: &UopInspectorApp) -> Lookups {
    Lookups {
        strings: app.uo_string_dictionary.clone(),
        tileart: app.ec_tileart_entries.clone(),
        tileart_revision: app.tileart_editor.edits.revision(),
        files: app.effects_panel.files.clone(),
        client_dir: app.settings.ec_path.clone(),
    }
}

fn arc_address<T: ?Sized>(value: &Option<Arc<T>>) -> usize {
    value.as_ref().map_or(0, |value| Arc::as_ptr(value) as *const u8 as usize)
}

fn hash_of(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub fn ui_effects(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    auto_load(app);
    poll_files_load(app, ctx);
    handle_shortcuts(app, ctx);
    let lookups = lookups(app);

    egui::Panel::top("effects_toolbar").show(ui, |ui| toolbar(app, &lookups, ui));
    refresh_validation(app, &lookups);
    egui::Panel::bottom("effects_validation")
        .resizable(true)
        .default_size(VALIDATION_PANEL_HEIGHT)
        .show(ui, |ui| validation_panel(app, ui));
    egui::Panel::left("effects_browse")
        .resizable(true)
        .default_size(BROWSE_PANEL_WIDTH)
        .show(ui, |ui| browse_panel(app, &lookups, ui));
    egui::Panel::right("effects_preview")
        .resizable(true)
        .default_size(PREVIEW_PANEL_WIDTH)
        .show(ui, |ui| preview_panel(app, &lookups, ctx, ui));
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("effects_details_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| details_panel(app, &lookups, ctx, ui));
    });
    app.effects_panel.scroll_to_selection = false;
    pending_open_window(app, ctx);
    super::effects_guide::guide_window(ctx, &mut app.effects_panel.guide_open);
}

/// Opens the effects of the loaded client once, when nothing else is open.
fn auto_load(app: &mut UopInspectorApp) {
    let Some(dir) = app.settings.ec_path.clone() else {
        return;
    };
    let state = &app.effects_panel;
    if state.auto_loaded.as_ref() == Some(&dir)
        || state.document.is_dirty()
        || *state.document.source() != CollectionSource::None
        || state.files.is_some()
    {
        return;
    }
    app.effects_panel.auto_loaded = Some(dir.clone());
    if dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME).exists() || dir.join(EFFECT_TEXTURE_UOP_NAME).exists() {
        perform_open(app, OpenRequest::Client(dir));
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
    let state = &mut app.effects_panel;
    let changed = if undo { state.document.undo() } else { state.document.redo() };
    if changed {
        resolve_selection(state);
    }
}

fn resolve_selection(state: &mut EffectsPanelState) {
    let collection = state.document.collection();
    state.selection = state.selection.take().and_then(|selection| selection.resolve(collection));
}

fn select(state: &mut EffectsPanelState, selection: Option<EffectsSelection>, reveal: bool) {
    if state.selection != selection {
        state.document.break_coalescing();
    }
    if reveal {
        if let Some(selection) = &selection {
            state.view = match selection {
                EffectsSelection::Entry { .. } | EffectsSelection::Definition { .. } => BrowseView::Entries,
                EffectsSelection::TileArt { .. } => BrowseView::TileArt,
                EffectsSelection::File { .. } => BrowseView::Files,
            };
        }
        state.scroll_to_selection = true;
    }
    state.selection = selection;
}

fn request_open(app: &mut UopInspectorApp, request: OpenRequest) {
    let document = &app.effects_panel.document;
    let loses = (request.replaces_collection() && document.collection_dirty())
        || (request.replaces_files() && document.files_dirty());
    if loses {
        app.effects_panel.pending_open = Some(request);
    } else {
        perform_open(app, request);
    }
}

/// Starts reading an `EffectTexture.uop`, which replaces the open one and its file changes.
fn start_files_load(state: &mut EffectsPanelState, path: PathBuf, strings: Option<Arc<UoStringDictionary>>) {
    let (sender, result) = mpsc::channel();
    let worker_path = path.clone();
    std::thread::spawn(move || {
        // Nobody listens any more when another package was opened meanwhile.
        let _ = sender.send(EffectFiles::open(&worker_path, strings.as_deref()));
    });
    state.files_load = Some(FilesLoad { path, result });
    state.files = None;
    state.document.reset_files();
    state.files_generation += 1;
    state.thumbnails.clear();
    state.script_issue_cache.clear();
    state.script_editor = None;
    if matches!(state.selection, Some(EffectsSelection::File { .. })) {
        state.selection = None;
    }
}

/// Takes the `EffectTexture.uop` the worker has read, when it is done.
fn poll_files_load(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let state = &mut app.effects_panel;
    let Some(load) = &state.files_load else {
        return;
    };
    let result = match load.result.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(FILES_LOAD_POLL_INTERVAL);
            return;
        }
        Err(mpsc::TryRecvError::Disconnected) => Err("the reading stopped unexpectedly.".to_owned()),
    };
    let path = load.path.clone();
    state.files_load = None;
    match result {
        Ok(files) => {
            let count = files.rows(&BTreeMap::new()).len();
            state.files = Some(Arc::new(files));
            state.files_generation += 1;
            let message = format!("Effects: opened {count} files from {}.", path.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => state.error = Some(format!("Open failed: {}: {error}", path.display())),
    }
}

fn perform_open(app: &mut UopInspectorApp, request: OpenRequest) {
    let strings = app.uo_string_dictionary.clone();
    let client_collection = app.settings.ec_path.as_ref().map(|dir| dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME));
    let mut errors = Vec::new();
    let mut opened = Vec::new();
    let (collection_path, files_path, kdl_path) = match &request {
        OpenRequest::Client(dir) => {
            let collection = dir.join(EFFECT_DEFINITION_COLLECTION_UOP_NAME);
            let files = dir.join(EFFECT_TEXTURE_UOP_NAME);
            (collection.exists().then_some(collection), files.exists().then_some(files), None)
        }
        OpenRequest::Collection(path) => (Some(path.clone()), None, None),
        OpenRequest::Kdl(path) => (None, None, Some(path.clone())),
        OpenRequest::Files(path) => (None, Some(path.clone()), None),
    };
    let state = &mut app.effects_panel;
    if let Some(path) = collection_path {
        match open_collection(&path) {
            Ok(collection) => {
                let is_client = client_collection.as_deref().is_some_and(|client| same_file(client, &path));
                let source = if is_client { CollectionSource::ClientPackage(path.clone()) } else { CollectionSource::Package(path.clone()) };
                opened.push(format!("{} entries from {}", collection.entries.len(), path.display()));
                state.document.replace_collection(collection, source);
                state.document_generation += 1;
                state.selection = None;
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    if let Some(path) = kdl_path {
        match import_kdl(&path, strings.as_deref()) {
            Ok(collection) => {
                opened.push(format!("{} entries from {}", collection.entries.len(), path.display()));
                state.document.replace_collection(collection, CollectionSource::Kdl(path));
                state.document_generation += 1;
                state.selection = None;
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    if let Some(path) = files_path {
        start_files_load(state, path, strings.clone());
    }
    state.error = (!errors.is_empty()).then(|| format!("Open failed: {}", errors.join("; ")));
    if !opened.is_empty() {
        let message = format!("Effects: opened {}.", opened.join(", "));
        app.status_message = message.clone();
        app.log(message);
    }
}

fn pending_open_window(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(request) = app.effects_panel.pending_open.clone() else {
        return;
    };
    let (mut discard, mut cancel) = (false, false);
    egui::Window::new("Discard unsaved effect changes?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("The edited effects have unsaved changes that opening this discards.");
            ui.horizontal(|ui| {
                discard = ui.button("Discard and open").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if discard || cancel {
        app.effects_panel.pending_open = None;
    }
    if discard {
        perform_open(app, request);
    }
}

fn pick_file((name, extensions): (&str, &[&str])) -> Option<PathBuf> {
    crate::dialog::file_dialog().add_filter(name, extensions).pick_file()
}

fn save_file((name, extensions): (&str, &[&str]), default_name: &str) -> Option<PathBuf> {
    crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(default_name).save_file()
}

/// Paths a save must not write over: the loaded client files and the opened sources.
fn protected_paths(state: &EffectsPanelState, lookups: &Lookups) -> Vec<PathBuf> {
    let mut paths = lookups.client_paths();
    match state.document.source() {
        CollectionSource::ClientPackage(path) | CollectionSource::Package(path) | CollectionSource::Kdl(path) => {
            paths.push(path.clone());
        }
        CollectionSource::None => {}
    }
    if let Some(files) = &state.files {
        paths.push(files.source_path().to_path_buf());
    }
    paths
}

fn report_saved(app: &mut UopInspectorApp, message: String) {
    app.effects_panel.error = None;
    app.status_message = message.clone();
    app.log(message);
}

fn toolbar(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui) {
    let client_dir = lookups.client_dir.clone();
    ui.horizontal_wrapped(|ui| {
        if ui.button("Guide").on_hover_text(GUIDE_HINT).clicked() {
            app.effects_panel.guide_open = true;
        }
        ui.separator();
        ui.label("Open:");
        if ui
            .add_enabled(client_dir.is_some(), egui::Button::new("Client effects"))
            .on_hover_text("EffectDefinitionCollection.uop and EffectTexture.uop of the loaded client.")
            .on_disabled_hover_text("No Enhanced Client directory is set (Search Paths).")
            .clicked()
        {
            if let Some(dir) = client_dir.clone() {
                request_open(app, OpenRequest::Client(dir));
            }
        }
        if ui.button("Collection .uop...").clicked() {
            if let Some(path) = pick_file(UOP_FILTER) {
                request_open(app, OpenRequest::Collection(path));
            }
        }
        if ui.button("Import KDL...").clicked() {
            if let Some(path) = pick_file(KDL_FILTER) {
                request_open(app, OpenRequest::Kdl(path));
            }
        }
        if ui.button("EffectTexture .uop...").clicked() {
            if let Some(path) = pick_file(UOP_FILTER) {
                request_open(app, OpenRequest::Files(path));
            }
        }

        ui.separator();
        ui.label("Save:");
        let save_source = collection_save_source(app, lookups);
        if ui
            .add_enabled(save_source.is_some(), egui::Button::new("Collection .uop..."))
            .on_hover_text(format!(
                "Writes a copy of the source EffectDefinitionCollection.uop with exactly the edited entries. The \
                 loaded client files are refused as the target.\n{UOP_PATCHER_NOTE}"
            ))
            .on_disabled_hover_text(COLLECTION_SOURCE_NEEDED)
            .clicked()
        {
            if let (Some(source), Some(path)) = (save_source, save_file(UOP_FILTER, EFFECT_DEFINITION_COLLECTION_UOP_NAME)) {
                save_collection_to(app, lookups, &source, &path);
            }
        }
        let has_files = app.effects_panel.files.is_some();
        if ui
            .add_enabled(has_files, egui::Button::new("EffectTexture .uop..."))
            .on_hover_text(format!(
                "Writes a copy of the opened EffectTexture.uop with the file changes.\n{HEADER_HASH_NOTE}\n{UOP_PATCHER_NOTE}"
            ))
            .on_disabled_hover_text("No EffectTexture.uop is open.")
            .clicked()
        {
            if let Some(path) = save_file(UOP_FILTER, EFFECT_TEXTURE_UOP_NAME) {
                save_files_to(app, lookups, &path);
            }
        }
        if ui.button("Export KDL...").clicked() {
            if let Some(path) = save_file(KDL_FILTER, DEFAULT_KDL_FILE_NAME) {
                let state = &app.effects_panel;
                match export_kdl(state.document.collection(), &path, lookups.strings.as_deref()) {
                    Ok(()) => report_saved(app, format!("Exported the effect collection to {}.", path.display())),
                    Err(error) => app.effects_panel.error = Some(format!("Export failed: {error}")),
                }
            }
        }

        let document = &app.effects_panel.document;
        let (can_undo, can_redo, can_revert) = (document.can_undo(), document.can_redo(), document.differs_from_opened());
        let (collection_dirty, files_dirty) = (document.collection_dirty(), document.files_dirty());
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
            .on_hover_text("Back to the collection and files as opened (undoable).")
            .clicked()
        {
            let state = &mut app.effects_panel;
            state.document.revert_to_opened();
            resolve_selection(state);
        }
        ui.separator();
        if collection_dirty {
            ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} collection changed"));
        }
        if files_dirty {
            ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} files changed"));
        }
        ui.weak(source);
        if let Some(files) = &lookups.files {
            ui.weak(format!("files {}", files.source_path().display()));
        }
        if let Some(load) = &app.effects_panel.files_load {
            ui.spinner();
            ui.weak(format!("reading {}", load.path.display())).on_hover_text(FILES_LOAD_HOVER);
        }
        ui.label("ℹ").on_hover_text(HEADER_HASH_NOTE);
    });
    if let Some(error) = app.effects_panel.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ERROR_COLOR, error);
            if ui.small_button("Dismiss").clicked() {
                app.effects_panel.error = None;
            }
        });
    }
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Effects,
    label: "the effect collection and the file changes",
    is_dirty: |app| app.effects_panel.document.is_dirty(),
    save: save_unsaved_changes,
    discard: discard_unsaved_changes,
};

/// The package a saved collection is a copy of.
fn collection_save_source(app: &UopInspectorApp, lookups: &Lookups) -> Option<PathBuf> {
    let client_source = lookups.client_paths().into_iter().next().filter(|path| path.exists());
    app.effects_panel.document.source().package_path().map(Path::to_path_buf).or(client_source)
}

/// Asks where to write what is unsaved: the collection, then the file changes.
fn save_unsaved_changes(app: &mut UopInspectorApp) {
    let lookups = lookups(app);
    if app.effects_panel.document.collection_dirty() {
        match collection_save_source(app, &lookups) {
            Some(source) => {
                if let Some(path) = save_file(UOP_FILTER, EFFECT_DEFINITION_COLLECTION_UOP_NAME) {
                    save_collection_to(app, &lookups, &source, &path);
                }
            }
            None => app.effects_panel.error = Some(COLLECTION_SOURCE_NEEDED.to_owned()),
        }
    }
    if app.effects_panel.document.files_dirty() {
        if let Some(path) = save_file(UOP_FILTER, EFFECT_TEXTURE_UOP_NAME) {
            save_files_to(app, &lookups, &path);
        }
    }
}

fn discard_unsaved_changes(app: &mut UopInspectorApp) {
    let state = &mut app.effects_panel;
    state.document.revert_to_opened();
    state.document.mark_collection_saved();
    state.document.mark_files_saved();
    resolve_selection(state);
}

fn save_collection_to(app: &mut UopInspectorApp, lookups: &Lookups, source: &Path, output: &Path) {
    let state = &mut app.effects_panel;
    let protected = protected_paths(state, lookups);
    let protected: Vec<&Path> = protected.iter().map(PathBuf::as_path).collect();
    match save_collection(state.document.collection(), source, output, &protected) {
        Ok(changes) => {
            state.document.mark_collection_saved();
            report_saved(
                app,
                format!(
                    "Saved the effect collection to {} ({} replaced, {} added, {} removed).",
                    output.display(),
                    changes.replaced,
                    changes.added,
                    changes.removed
                ),
            );
        }
        Err(error) => state.error = Some(format!("Save failed: {error}")),
    }
}

fn save_files_to(app: &mut UopInspectorApp, lookups: &Lookups, output: &Path) {
    let state = &mut app.effects_panel;
    let Some(files) = state.files.clone() else {
        return;
    };
    if let Some((_, draft)) = state.document.nif_drafts().iter().next() {
        state.error = Some(format!(
            "Save refused: {} does not write ({}). Fix the lengths or undo the edit.",
            draft.name, draft.error
        ));
        return;
    }
    let protected = protected_paths(state, lookups);
    let protected: Vec<&Path> = protected.iter().map(PathBuf::as_path).collect();
    match files.save_as(state.document.files(), output, &protected) {
        Ok(report) => {
            state.document.mark_files_saved();
            report_saved(
                app,
                format!(
                    "Saved EffectTexture.uop to {} ({} replaced, {} added, {} removed).",
                    output.display(),
                    report.replaced,
                    report.added,
                    report.removed
                ),
            );
        }
        Err(error) => state.error = Some(format!("Save failed: {error}")),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Validation

fn script_existence_key(state: &EffectsPanelState) -> u64 {
    let present: Vec<(u64, bool)> =
        state.document.files().iter().map(|(&hash, change)| (hash, change.payload.is_some())).collect();
    hash_of(&(state.files_generation, present))
}

fn refresh_validation(app: &mut UopInspectorApp, lookups: &Lookups) {
    let state = &mut app.effects_panel;
    let key = hash_of(&(
        state.document_generation,
        state.document.revision(),
        state.files_generation,
        arc_address(&lookups.strings),
    ));
    if state.validation.as_ref().is_some_and(|cache| cache.key == key) {
        return;
    }
    let overrides = state.document.files().clone();
    let files = lookups.files.clone();
    let exists = |name: &str| files.as_ref().is_some_and(|files| files.exists(&overrides, name));
    let context = EffectValidationContext {
        strings: lookups.strings.as_deref(),
        particle_file_exists: files.is_some().then_some(&exists as &dyn Fn(&str) -> bool),
    };
    let collection = state.document.collection();
    let mut issues: Vec<PanelIssue> = collection
        .validate(&context)
        .into_iter()
        .map(|issue| PanelIssue {
            severity: match issue.severity() {
                EffectIssueSeverity::Error => ScriptSeverity::Error,
                EffectIssueSeverity::Warning => ScriptSeverity::Warning,
            },
            text: issue.to_string(),
            target: issue_selection(collection, &issue),
        })
        .collect();

    let mut checked = vec!["collection rules"];
    checked.push(if lookups.strings.is_some() { "particle names" } else { "particle names not checked (no string dictionary)" });
    if let Some(files) = &files {
        checked.push("particle files and Chaos scripts against EffectTexture.uop");
        let existence = script_existence_key(state);
        let scripts: Vec<FileRow> = files
            .rows(&overrides)
            .into_iter()
            .filter(|row| row.kind == FileKind::Chaos && row.status != FileStatus::Removed)
            .collect();
        for row in scripts {
            let Some(payload) = files.payload(&overrides, row.hash) else {
                continue;
            };
            let address = Arc::as_ptr(&payload) as *const u8 as usize;
            let cached = state
                .script_issue_cache
                .get(&row.hash)
                .filter(|entry| entry.payload == address && entry.existence == existence)
                .map(|entry| Arc::clone(&entry.issues));
            let script_issues = cached.unwrap_or_else(|| {
                let found = Arc::new(check_script(&payload, Some(&exists)));
                state.script_issue_cache.insert(
                    row.hash,
                    ScriptIssueCacheEntry { payload: address, existence, issues: Arc::clone(&found) },
                );
                found
            });
            for issue in script_issues.iter().filter(|issue| issue.severity != ScriptSeverity::Note) {
                issues.push(PanelIssue {
                    severity: issue.severity,
                    text: format!("{}: {issue}", row.label()),
                    target: Some(EffectsSelection::File { hash: row.hash }),
                });
            }
        }
        checked.push("edited NIF files (writing, links)");
        for (&hash, draft) in state.document.nif_drafts() {
            issues.push(PanelIssue {
                severity: ScriptSeverity::Error,
                text: format!("{}: does not write: {}", draft.name, draft.error),
                target: Some(EffectsSelection::File { hash }),
            });
        }
        for (&hash, change) in &overrides {
            let Some(payload) = change.payload.as_ref().filter(|_| crate::logic::effect_files::is_nif_name(&change.name)) else {
                continue;
            };
            let Ok(nif) = gamebryo_nif::NifFile::read(payload) else { continue };
            for issue in nif.link_issues() {
                let place = issue.block.map_or_else(|| "footer".to_owned(), |block| format!("block {block}"));
                issues.push(PanelIssue {
                    severity: ScriptSeverity::Warning,
                    text: format!(
                        "{}: {place} {} = {} points at {}, expected {}",
                        change.name,
                        issue.field,
                        issue.raw,
                        issue.found.unwrap_or("nothing"),
                        issue.expected
                    ),
                    target: Some(EffectsSelection::File { hash }),
                });
            }
        }
    } else {
        checked.push("particle files not checked (no EffectTexture.uop)");
    }
    issues.sort_by(|left, right| right.severity.cmp(&left.severity));
    let checked = format!("Checked: {}.", checked.join(", "));
    state.selected_issue = state.selected_issue.filter(|&index| index < issues.len());
    state.validation = Some(ValidationCache { key, issues: Arc::new(issues), checked });
}

fn severity_style(severity: ScriptSeverity) -> (&'static str, egui::Color32) {
    match severity {
        ScriptSeverity::Error => (ERROR_ICON, ERROR_COLOR),
        ScriptSeverity::Warning => (WARNING_ICON, WARNING_COLOR),
        ScriptSeverity::Note => (NOTE_ICON, NOTE_COLOR),
    }
}

fn validation_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &app.effects_panel;
    let Some(cache) = state.validation.as_ref() else {
        return;
    };
    let issues = Arc::clone(&cache.issues);
    let errors = issues.iter().filter(|issue| issue.severity == ScriptSeverity::Error).count();
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
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("effects_validation_scroll")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, issues.len().max(1), |ui, range| {
            if issues.is_empty() {
                ui.colored_label(OK_COLOR, "No issues.");
                return;
            }
            for index in range {
                let issue = &issues[index];
                ui.horizontal(|ui| {
                    let (icon, color) = severity_style(issue.severity);
                    ui.colored_label(color, icon);
                    if ui.selectable_label(selected_issue == Some(index), &issue.text).clicked() {
                        clicked = Some(index);
                    }
                });
            }
        });
    if let Some(index) = clicked {
        let state = &mut app.effects_panel;
        state.selected_issue = Some(index);
        let target = issues[index].target.clone();
        if target.is_some() {
            select(state, target, true);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Browse

fn definition_summary(definition: &EffectDefinition, lookups: &Lookups) -> String {
    match definition {
        EffectDefinition::Particle(particle) => match lookups.particle_name(particle) {
            Some(name) => name,
            None => format!("particle #{}", particle.name_index),
        },
        EffectDefinition::Sound(effect) => format!("sound {}", effect.sound_id),
        EffectDefinition::RoundRobin(effect) => format!("round robin ({})", effect.children.len()),
        EffectDefinition::PositionOffset(_) => "position offset".to_string(),
        EffectDefinition::Wait(effect) => format!("wait {}..{} ms", effect.min_delay_ms, effect.max_delay_ms),
        EffectDefinition::Lod(effect) => format!("LOD ({} levels)", effect.levels.len()),
        EffectDefinition::PropertyFlag(effect) => format!(
            "property flag {} ({} cases)",
            property_flag_mode_label(effect.mode),
            effect.cases.len()
        ),
        EffectDefinition::Earthquake(effect) => format!("earthquake {:.2} for {:.1} s", effect.amplitude, effect.duration),
        EffectDefinition::Random(effect) => format!("random ({} entries)", effect.entries.len()),
        EffectDefinition::UserPreference(effect) => format!("user preference {:#x}", effect.mask),
    }
}

fn list_summary(definitions: &[EffectDefinition], lookups: &Lookups) -> String {
    let mut parts: Vec<String> =
        definitions.iter().take(SUMMARY_ITEMS).map(|definition| definition_summary(definition, lookups)).collect();
    if definitions.len() > SUMMARY_ITEMS {
        parts.push("...".to_string());
    }
    if parts.is_empty() { "(empty)".to_string() } else { parts.join(" + ") }
}

fn property_flag_mode_label(mode: PropertyFlagMode) -> &'static str {
    match mode {
        PropertyFlagMode::FlagMask => "flag mask",
        PropertyFlagMode::Property => "property",
    }
}

fn entry_key_label(key: EffectEntryKey) -> String {
    match key {
        EffectEntryKey::Id(id) => id.to_string(),
        EffectEntryKey::Hash(hash) => format!("hash {hash:016x}"),
    }
}

/// The packet value of a list row: the fixed effect field's, else the explode effect field's.
fn packet_label(key: EffectEntryKey) -> String {
    let routes = key.id().map(packet_routes_for_collection_id).unwrap_or_default();
    let value = |wanted: PacketEffectPath| {
        routes.iter().find_map(|route| match route {
            PacketRoute::Field { path, value } if *path == wanted => Some(*value),
            _ => None,
        })
    };
    match (value(PacketEffectPath::FixedEffect), value(PacketEffectPath::MovingExplosion)) {
        (Some(value), _) => format!("packet {value}"),
        (None, Some(value)) => format!("explode {value}"),
        (None, None) => "no packet".to_string(),
    }
}

fn entry_rows(state: &mut EffectsPanelState, lookups: &Lookups) -> Arc<Vec<(u32, String)>> {
    let key = hash_of(&(
        state.document_generation,
        state.document.revision(),
        arc_address(&lookups.strings),
        format!("{:?}", state.entry_filter),
    ));
    if let Some(cache) = state.entries_cache.as_ref().filter(|cache| cache.key == key) {
        return Arc::clone(&cache.rows);
    }
    let collection = state.document.collection();
    let name_of = |particle: &ParticleEffect| lookups.particle_name(particle);
    let rows: Vec<(u32, String)> = state
        .entry_filter
        .visible_entries(collection, &name_of)
        .into_iter()
        .map(|index| {
            let entry = &collection.entries[index];
            let label = format!(
                "{:>6}  {:<13} {}",
                entry_key_label(entry.key),
                packet_label(entry.key),
                list_summary(&entry.definitions, lookups)
            );
            (index as u32, label)
        })
        .collect();
    let rows = Arc::new(rows);
    state.entries_cache = Some(ListCache { key, rows: Arc::clone(&rows) });
    rows
}

fn tileart_rows(state: &mut EffectsPanelState, lookups: &Lookups) -> Arc<Vec<(u32, String)>> {
    let key = hash_of(&(lookups.tileart_key(), arc_address(&lookups.strings), format!("{:?}", state.tileart_filter)));
    if let Some(cache) = state.tileart_cache.as_ref().filter(|cache| cache.key == key) {
        return Arc::clone(&cache.rows);
    }
    let name_of = |particle: &ParticleEffect| lookups.particle_name(particle);
    let rows: Vec<(u32, String)> = lookups
        .tileart
        .iter()
        .flat_map(|entries| entries.iter())
        .filter(|file| !file.entry.effects.is_empty())
        .filter_map(|file| {
            let tile_id = file.entry.tile_id;
            let name = lookups.tile_name(file).unwrap_or_default();
            let passes = state.tileart_filter.matches(&[tile_id], &[&name], &file.entry.effects, &name_of);
            passes.then(|| {
                let label = format!(
                    "0x{tile_id:0width$X} {name}  {}",
                    list_summary(&file.entry.effects, lookups),
                    width = TILE_ID_HEX_DIGITS
                );
                (tile_id, label)
            })
        })
        .collect();
    let rows = Arc::new(rows);
    state.tileart_cache = Some(ListCache { key, rows: Arc::clone(&rows) });
    rows
}

fn file_rows(state: &mut EffectsPanelState) -> (Arc<Vec<FileRow>>, Arc<Vec<usize>>) {
    let Some(files) = state.files.clone() else {
        return Default::default();
    };
    let key = (state.files_generation, state.document.revision());
    let rows = match state.files_cache.as_ref().filter(|cache| cache.key == key) {
        Some(cache) if cache.filter == state.file_filter => return (Arc::clone(&cache.rows), Arc::clone(&cache.visible)),
        Some(cache) => Arc::clone(&cache.rows),
        None => Arc::new(files.rows(state.document.files())),
    };
    let visible = Arc::new(state.file_filter.visible(&rows));
    state.files_cache =
        Some(FilesCache { key, rows: Arc::clone(&rows), filter: state.file_filter.clone(), visible: Arc::clone(&visible) });
    (rows, visible)
}

fn file_users(state: &mut EffectsPanelState, lookups: &Lookups) -> Arc<HashMap<u64, Vec<FileUser>>> {
    let key = hash_of(&(
        state.document_generation,
        state.document.revision(),
        state.files_generation,
        lookups.tileart_key(),
        arc_address(&lookups.strings),
    ));
    if let Some(cache) = state.users_cache.as_ref().filter(|cache| cache.key == key) {
        return Arc::clone(&cache.users);
    }
    let tileart: Vec<(u32, &[EffectDefinition])> = lookups
        .tileart
        .iter()
        .flat_map(|entries| entries.iter())
        .filter(|file| !file.entry.effects.is_empty())
        .map(|file| (file.entry.tile_id, file.entry.effects.as_slice()))
        .collect();
    let scripts = lookups.files.as_ref().map(|files| files.script_references(state.document.files())).unwrap_or_default();
    let name_of = |particle: &ParticleEffect| lookups.particle_name(particle);
    let users = Arc::new(build_file_users(state.document.collection(), &tileart, &name_of, &scripts));
    state.users_cache = Some(UsersCache { key, users: Arc::clone(&users) });
    users
}

fn definition_filter_ui(ui: &mut egui::Ui, filter: &mut DefinitionFilter, salt: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(egui::TextEdit::singleline(&mut filter.text).hint_text(hint));
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt((salt, "kind"))
            .selected_text(filter.kind.map_or("any kind", DefinitionKind::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut filter.kind, None, "any kind");
                for kind in DefinitionKind::ALL {
                    ui.selectable_value(&mut filter.kind, Some(kind), kind.label());
                }
            });
        egui::ComboBox::from_id_salt((salt, "particle"))
            .selected_text(particle_filter_label(filter.particle))
            .show_ui(ui, |ui| {
                for choice in [ParticleKindFilter::Any, ParticleKindFilter::Chaos, ParticleKindFilter::Nif] {
                    ui.selectable_value(&mut filter.particle, choice, particle_filter_label(choice));
                }
            });
        if ui.add_enabled(filter.is_active(), egui::Button::new("Clear")).clicked() {
            *filter = DefinitionFilter::default();
        }
    });
}

fn particle_filter_label(filter: ParticleKindFilter) -> &'static str {
    match filter {
        ParticleKindFilter::Any => "any particle",
        ParticleKindFilter::Chaos => "with Chaos particles",
        ParticleKindFilter::Nif => "with NIF particles",
    }
}

fn browse_panel(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui) {
    let state = &mut app.effects_panel;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.view, BrowseView::Entries, "Collection");
        ui.selectable_value(&mut state.view, BrowseView::TileArt, "TileArt");
        ui.selectable_value(&mut state.view, BrowseView::Files, "Files");
    });
    ui.separator();
    match state.view {
        BrowseView::Entries => entries_list(app, lookups, ui),
        BrowseView::TileArt => tileart_list(app, lookups, ui),
        BrowseView::Files => files_list(app, ui),
    }
}

/// A selectable list of rows, scrolled to the selected row on request. The arrow keys move the selection, unless a
/// text field has the keyboard, and keep the new row in view. Returns the row clicked or moved to.
fn row_list(
    ui: &mut egui::Ui,
    salt: &str,
    count: usize,
    selected_row: Option<usize>,
    scroll: bool,
    mut row: impl FnMut(&mut egui::Ui, usize) -> bool,
) -> Option<usize> {
    let row_height = ui.spacing().interact_size.y;
    let spacing = ui.spacing().item_spacing.y;
    let keyed = arrow_delta(ui, ui.ctx().text_edit_focused())
        .and_then(|delta| move_selection(&(0..count).collect::<Vec<_>>(), selected_row, delta))
        .filter(|&index| Some(index) != selected_row);
    let mut area = egui::ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    if scroll {
        if let Some(index) = selected_row {
            area = area.vertical_scroll_offset(index as f32 * (row_height + spacing));
        }
    }
    let mut clicked = None;
    area.show_rows(ui, row_height, count, |ui, range| {
        if let Some(index) = keyed {
            // The row may be outside the drawn range: its place follows from the first drawn row.
            let top = ui.max_rect().top() + (index as f32 - range.start as f32) * (row_height + spacing);
            let rect = egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), egui::vec2(ui.max_rect().width(), row_height));
            ui.scroll_to_rect(rect, None);
        }
        for index in range {
            if row(ui, index) {
                clicked = Some(index);
            }
        }
    });
    if keyed.is_some() {
        // The rows were drawn with the previous selection.
        ui.ctx().request_repaint();
    }
    clicked.or(keyed)
}

fn entries_list(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui) {
    let state = &mut app.effects_panel;
    definition_filter_ui(ui, &mut state.entry_filter, "effects_entry_filter", "id, packet, particle name, kind");
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.new_entry_id)
                .hint_text(next_free_entry_id(state.document.collection()).to_string())
                .desired_width(ID_FILTER_WIDTH),
        )
        .on_hover_text("Collection id of the new entry (decimal or 0x hex); empty = the next free id.");
        if ui.button("Add entry").clicked() {
            let id = parse_id(&state.new_entry_id);
            if id.is_none() && !state.new_entry_id.trim().is_empty() {
                state.error = Some(format!("Invalid id {:?}.", state.new_entry_id));
            } else {
                match state.document.edit_collection(None, |collection| add_entry(collection, id)) {
                    Ok(selection) => {
                        state.new_entry_id.clear();
                        select(state, Some(selection), true);
                    }
                    Err(error) => state.error = Some(error),
                }
            }
        }
    });
    packet_lookup_ui(state, ui);
    let rows = entry_rows(state, lookups);
    ui.label(format!("Showing {} of {} entries", rows.len(), state.document.collection().entries.len()));
    ui.separator();
    let selected_entry = state.selection.as_ref().and_then(EffectsSelection::entry);
    let selected_row = selected_entry.and_then(|entry| rows.iter().position(|(index, _)| *index as usize == entry));
    let clicked = row_list(ui, "effects_entries", rows.len(), selected_row, state.scroll_to_selection, |ui, row| {
        let (index, label) = &rows[row];
        let selected = selected_entry == Some(*index as usize);
        ui.add(egui::Button::selectable(selected, egui::RichText::new(label).monospace())).clicked()
    });
    if let Some(row) = clicked {
        select(state, Some(EffectsSelection::Entry { entry: rows[row].0 as usize }), false);
    }
}

/// Finds the collection entry a value of a 0xC7 packet field plays.
fn packet_lookup_ui(state: &mut EffectsPanelState, ui: &mut egui::Ui) {
    let mut open = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Packet effect");
        egui::ComboBox::from_id_salt("effects_packet_lookup_path")
            .selected_text(state.packet_lookup_path.label())
            .show_ui(ui, |ui| {
                for path in PacketEffectPath::ALL {
                    ui.selectable_value(&mut state.packet_lookup_path, path, path.label());
                }
            })
            .response
            .on_hover_text(
                "The 0xC7 field: the effect of a fixed effect (types 2/3), the effect of a moving effect (type 0) \
                 or its explode effect. 0x70 and 0xC0 leave both 0; type 1 (lightning) plays collection id 666.",
            );
        ui.add(egui::TextEdit::singleline(&mut state.packet_lookup).hint_text("value").desired_width(ID_FILTER_WIDTH))
            .on_hover_text(
                "Decimal or 0x hex. Fixed effects play collection id = value - 1 (9501: nothing, 0 and 9502: the \
                 classic tile effect); explode effects play value - 1 (0: the classic explosion); moving effects \
                 play no entry (9502 hides the projectile item's tileart effects).",
            );
        if state.packet_lookup.trim().is_empty() {
            return;
        }
        let Some(packet) = parse_id(&state.packet_lookup).and_then(|value| u16::try_from(value).ok()) else {
            ui.colored_label(ERROR_COLOR, "not a 16-bit value");
            return;
        };
        match describe_packet_effect(state.packet_lookup_path, packet) {
            Ok(id) => {
                let entry = state.document.collection().entries.iter().position(|entry| entry.key.id() == Some(id));
                match entry {
                    Some(entry) => {
                        if ui.link(format!("collection id {id}")).clicked() {
                            open = Some(entry);
                        }
                    }
                    None => {
                        ui.colored_label(WARNING_COLOR, format!("collection id {id}: no such entry"));
                    }
                }
            }
            Err(reason) => {
                ui.weak(reason);
            }
        }
    });
    if let Some(entry) = open {
        select(state, Some(EffectsSelection::Entry { entry }), true);
    }
}

fn tileart_list(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui) {
    let state = &mut app.effects_panel;
    if lookups.tileart.is_none() {
        ui.label("No tileart.uop is loaded (Enhanced Client search path).");
        return;
    }
    definition_filter_ui(ui, &mut state.tileart_filter, "effects_tileart_filter", "tile id, name, particle name, kind");
    let rows = tileart_rows(state, lookups);
    ui.label(format!("{} tileart records with effects shown", rows.len()));
    ui.separator();
    let selected_tile = match &state.selection {
        Some(EffectsSelection::TileArt { tile_id, .. }) => Some(*tile_id),
        _ => None,
    };
    let selected_row = selected_tile.and_then(|tile| rows.iter().position(|(id, _)| *id == tile));
    let clicked = row_list(ui, "effects_tileart", rows.len(), selected_row, state.scroll_to_selection, |ui, row| {
        let (tile_id, label) = &rows[row];
        ui.add(egui::Button::selectable(selected_tile == Some(*tile_id), egui::RichText::new(label).monospace())).clicked()
    });
    if let Some(row) = clicked {
        select(state, Some(EffectsSelection::TileArt { tile_id: rows[row].0, path: Vec::new() }), false);
    }
}

fn files_list(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &mut app.effects_panel;
    if let Some(load) = &state.files_load {
        ui.horizontal_wrapped(|ui| {
            ui.spinner();
            ui.label(format!("Reading {}...", load.path.display()));
        });
        return;
    }
    if state.files.is_none() {
        ui.label("No EffectTexture.uop is open.");
        return;
    }
    let filter = &mut state.file_filter;
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(egui::TextEdit::singleline(&mut filter.text).hint_text("name or hash"));
    });
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("effects_file_kind")
            .selected_text(filter.kind.map_or("any kind", FileKind::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut filter.kind, None, "any kind");
                for kind in FileKind::ALL {
                    ui.selectable_value(&mut filter.kind, Some(kind), kind.label());
                }
            });
        ui.checkbox(&mut filter.unnamed_only, "hash only");
        ui.checkbox(&mut filter.changed_only, "changed");
    });
    if ui
        .button("Import file...")
        .on_hover_text(format!(
            "Adds a {} file from disk, or replaces the file of the same name.\n{HEADER_HASH_NOTE}",
            IMPORTABLE_EXTENSIONS.join("/")
        ))
        .clicked()
    {
        if let Some(path) = pick_file((IMPORT_FILTER_NAME, &IMPORTABLE_EXTENSIONS)) {
            import_file(state, &path, None);
        }
    }
    let (rows, visible) = file_rows(state);
    ui.label(format!("Showing {} of {} files", visible.len(), rows.len()));
    ui.separator();
    let selected_hash = match &state.selection {
        Some(EffectsSelection::File { hash }) => Some(*hash),
        _ => None,
    };
    let selected_row = selected_hash.and_then(|hash| visible.iter().position(|&index| rows[index].hash == hash));
    let clicked = row_list(ui, "effects_files", visible.len(), selected_row, state.scroll_to_selection, |ui, row| {
        let file = &rows[visible[row]];
        let mut text = egui::RichText::new(format!("{:<32} {:<10} {}", file.label(), file.kind.label(), file.status.label())).monospace();
        if file.status == FileStatus::Removed {
            text = text.strikethrough();
        }
        ui.add(egui::Button::selectable(selected_hash == Some(file.hash), text)).clicked()
    });
    if let Some(row) = clicked {
        select(state, Some(EffectsSelection::File { hash: rows[visible[row]].hash }), false);
    }
}

/// Stores a file from disk under its own name, or under `as_name` (replacing a file).
fn import_file(state: &mut EffectsPanelState, path: &Path, as_name: Option<&str>) {
    match read_import(path) {
        Ok((name, payload)) => {
            let name = as_name.map_or(name, str::to_owned);
            let hash = effect_texture_hash(&name);
            state.document.edit(None, |data| data.set_file(hash, FileOverride { name, payload: Some(payload) }));
            select(state, Some(EffectsSelection::File { hash }), true);
        }
        Err(error) => state.error = Some(format!("Import failed: {error}")),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Details

fn details_panel(app: &mut UopInspectorApp, lookups: &Lookups, ctx: &egui::Context, ui: &mut egui::Ui) {
    let selection = app.effects_panel.selection.clone();
    match selection {
        None => {
            ui.label(
                "Select a collection entry, a tileart record or a file on the left. The 0xC7 packet plays collection \
                 id = packet field - 1 (see the packet paths of an entry); tileart effects play on the items of that \
                 graphic. Guide, in the toolbar, explains the definitions and this tab.",
            );
        }
        Some(EffectsSelection::Entry { entry }) => entry_details(app, lookups, ui, entry, None),
        Some(EffectsSelection::Definition { entry, path }) => entry_details(app, lookups, ui, entry, Some(path)),
        Some(EffectsSelection::TileArt { tile_id, path }) => tileart_details(app, lookups, ui, tile_id, &path),
        Some(EffectsSelection::File { hash }) => file_details(app, lookups, ctx, ui, hash),
    }
}

enum EntryAction {
    SetId(u32),
    Duplicate,
    Remove,
    AddTop(DefinitionKind),
}

fn entry_details(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui, entry: usize, path: Option<Vec<PathStep>>) {
    let state = &mut app.effects_panel;
    let Some(original) = state.document.collection().entries.get(entry).cloned() else {
        return;
    };
    let mut action = None;
    ui.heading(format!("Collection entry {}", entry_key_label(original.key)));
    egui::Grid::new("effects_entry_fields").num_columns(2).striped(true).show(ui, |ui| {
        ui.label("Collection id");
        ui.horizontal(|ui| match original.key {
            EffectEntryKey::Id(id) => {
                if state.id_text_for != Some((entry, id)) {
                    state.id_text = id.to_string();
                    state.id_text_for = Some((entry, id));
                }
                ui.add(egui::TextEdit::singleline(&mut state.id_text).desired_width(ID_FILTER_WIDTH));
                let typed = parse_id(&state.id_text);
                if ui.add_enabled(typed.is_some_and(|typed| typed != id), egui::Button::new("Set id")).clicked() {
                    action = typed.map(EntryAction::SetId);
                }
                if typed.is_none() {
                    ui.colored_label(ERROR_COLOR, "invalid id");
                }
            }
            EffectEntryKey::Hash(_) => {
                ui.label("none: the entry is stored under a hash that no scanned id produces");
            }
        });
        ui.end_row();
        ui.label("Name hash");
        ui.monospace(format!("{:016x}", original.key.hash()));
        ui.end_row();
        ui.label("Packet paths").on_hover_text(
            "Every packet path and value that plays this entry: fixed effects (0xC7 types 2/3) and explode effects \
             (type 0) play id = value - 1, except that fixed effects intercept 9501 (nothing) and 9502 (classic), so \
             ids 9500 and 9501 play only as explode effects. Played entries have no object: never hued.",
        );
        match original.key.id().map(packet_routes_for_collection_id) {
            Some(routes) if !routes.is_empty() => {
                ui.vertical(|ui| {
                    for route in routes {
                        ui.label(route.label());
                    }
                });
            }
            Some(_) => {
                ui.colored_label(WARNING_COLOR, PACKET_UNREACHABLE_NOTE);
            }
            None => {
                ui.label("none");
            }
        }
        ui.end_row();
    });
    ui.horizontal(|ui| {
        if ui.button("Duplicate entry").on_hover_text("A copy with the next free id, right after this one.").clicked() {
            action = Some(EntryAction::Duplicate);
        }
        if ui.button("Remove entry").clicked() {
            action = Some(EntryAction::Remove);
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Definitions");
        if ui.small_button("?").on_hover_text(GUIDE_HINT).clicked() {
            state.guide_open = true;
        }
        let kind = add_kind_combo(ui, &mut state.add_kind, "effects_add_top_kind");
        if ui.button("Add").on_hover_text("Adds a top-level definition: they all play together.").clicked() {
            action = Some(EntryAction::AddTop(kind));
        }
    });
    let selected_path = path.clone();
    let mut clicked = None;
    definition_tree(ui, &original.definitions, selected_path.as_deref(), lookups, &mut clicked, ("entry", entry));
    if let Some(path) = clicked {
        select(state, Some(EffectsSelection::Definition { entry, path }), false);
    }

    if let Some(action) = action {
        let result = match action {
            EntryAction::SetId(id) => state.document.edit_collection(None, |collection| set_entry_id(collection, entry, id)).map(|()| None),
            EntryAction::Duplicate => state.document.edit_collection(None, |collection| {
                let id = next_free_entry_id(collection);
                duplicate_entry(collection, entry, id).map(Some)
            }),
            EntryAction::Remove => Ok(state.document.edit_collection(None, |collection| remove_entry(collection, entry))),
            EntryAction::AddTop(kind) => Ok(state
                .document
                .edit_collection(None, |collection| add_definition(collection, entry, &[], ChildList::Top, kind))),
        };
        match result {
            Ok(Some(selection)) => select(state, Some(selection), true),
            Ok(None) => resolve_selection(state),
            Err(error) => state.error = Some(error),
        }
        return;
    }

    let Some(path) = path else {
        return;
    };
    let Some(definition) = definition_at(&original.definitions, &path).cloned() else {
        return;
    };
    ui.separator();
    let outcome = definition_editor(ui, state, lookups, &definition, &path, Some(entry), true);
    apply_definition_outcome(state, lookups, entry, &path, definition, outcome);
}

fn add_kind_combo(ui: &mut egui::Ui, choice: &mut Option<DefinitionKind>, salt: &str) -> DefinitionKind {
    let mut kind = choice.unwrap_or(DefinitionKind::Particle);
    egui::ComboBox::from_id_salt(salt).selected_text(kind.label()).show_ui(ui, |ui| {
        for candidate in DefinitionKind::ALL {
            ui.selectable_value(&mut kind, candidate, candidate.label()).on_hover_text(candidate.description());
        }
    });
    *choice = Some(kind);
    kind
}

fn definition_tree(
    ui: &mut egui::Ui,
    definitions: &[EffectDefinition],
    selected: Option<&[PathStep]>,
    lookups: &Lookups,
    clicked: &mut Option<Vec<PathStep>>,
    salt: (&str, usize),
) {
    if definitions.is_empty() {
        ui.weak("(no definitions: the entry plays nothing)");
    }
    for (index, definition) in definitions.iter().enumerate() {
        tree_node(ui, definition, vec![PathStep::top(index)], selected, lookups, clicked, salt);
    }
}

fn tree_node(
    ui: &mut egui::Ui,
    definition: &EffectDefinition,
    path: Vec<PathStep>,
    selected: Option<&[PathStep]>,
    lookups: &Lookups,
    clicked: &mut Option<Vec<PathStep>>,
    salt: (&str, usize),
) {
    let kind = DefinitionKind::of(definition);
    let label = format!("{}: {}", kind.label(), definition_summary(definition, lookups));
    let is_selected = selected == Some(path.as_slice());
    let lists = child_lists(definition);
    if lists.is_empty() {
        if ui.selectable_label(is_selected, label).on_hover_text(kind.description()).clicked() {
            *clicked = Some(path);
        }
        return;
    }
    let id = ui.make_persistent_id(("effects_tree", salt, &path));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            if ui.selectable_label(is_selected, &label).on_hover_text(kind.description()).clicked() {
                *clicked = Some(path.clone());
            }
        })
        .body(|ui| {
            for (list, children) in lists {
                let mut child_path = path.clone();
                child_path.push(PathStep { list, index: 0 });
                if list == ChildList::Children {
                    for (index, child) in children.iter().enumerate() {
                        child_path.last_mut().expect("pushed").index = index;
                        tree_node(ui, child, child_path.clone(), selected, lookups, clicked, salt);
                    }
                    continue;
                }
                ui.weak(child_list_label(definition, list));
                ui.indent((id, list), |ui| {
                    if children.is_empty() {
                        ui.weak("(empty)");
                    }
                    for (index, child) in children.iter().enumerate() {
                        child_path.last_mut().expect("pushed").index = index;
                        tree_node(ui, child, child_path.clone(), selected, lookups, clicked, salt);
                    }
                });
            }
        });
}

/// What the definition editor asks for.
#[derive(Default)]
struct DefinitionOutcome {
    /// The edited copy and the field that changed.
    edited: Option<(EffectDefinition, &'static str)>,
    action: Option<DefinitionAction>,
    select_file: Option<u64>,
}

enum DefinitionAction {
    Duplicate,
    Remove,
    Move(isize),
    AddChild(ChildList, DefinitionKind),
    AddKeyedList,
    RemoveKeyedList(ChildList),
    SetName(String),
}

/// The fields of a definition; `editable` false shows them read-only (tileart: the Tile Metadata tab edits those).
fn definition_editor(
    ui: &mut egui::Ui,
    state: &mut EffectsPanelState,
    lookups: &Lookups,
    original: &EffectDefinition,
    path: &[PathStep],
    entry: Option<usize>,
    editable: bool,
) -> DefinitionOutcome {
    let kind = DefinitionKind::of(original);
    let mut outcome = DefinitionOutcome::default();
    let mut definition = original.clone();
    let mut changed: Option<&'static str> = None;
    ui.heading(kind.label());
    ui.label(kind.description());
    if editable {
        ui.horizontal(|ui| {
            if ui.button("Duplicate").clicked() {
                outcome.action = Some(DefinitionAction::Duplicate);
            }
            if ui.button("Remove").clicked() {
                outcome.action = Some(DefinitionAction::Remove);
            }
            if ui.button("Move up").clicked() {
                outcome.action = Some(DefinitionAction::Move(-1));
            }
            if ui.button("Move down").clicked() {
                outcome.action = Some(DefinitionAction::Move(1));
            }
        });
    }
    ui.add_enabled_ui(editable, |ui| {
        egui::Grid::new(("effects_definition_fields", path)).num_columns(2).striped(true).show(ui, |ui| {
            changed = definition_fields(ui, state, lookups, &mut definition, path, entry, &mut outcome);
        });
        keyed_lists_editor(ui, &mut definition, &mut changed, &mut outcome);
        if editable && !child_lists(&definition).is_empty() {
            ui.horizontal(|ui| {
                ui.label("Add child to");
                let lists: Vec<ChildList> = child_lists(&definition).into_iter().map(|(list, _)| list).collect();
                let id = ui.make_persistent_id(("effects_child_list", path));
                let mut target = ui.data(|data| data.get_temp::<usize>(id)).unwrap_or(0).min(lists.len() - 1);
                egui::ComboBox::from_id_salt(id).selected_text(child_list_label(&definition, lists[target])).show_ui(ui, |ui| {
                    for (index, list) in lists.iter().enumerate() {
                        ui.selectable_value(&mut target, index, child_list_label(&definition, *list));
                    }
                });
                ui.data_mut(|data| data.insert_temp(id, target));
                let kind = add_kind_combo(ui, &mut state.add_kind, "effects_add_child_kind");
                if ui.button("Add").clicked() && accepts_children(&definition, lists[target]) {
                    outcome.action = Some(DefinitionAction::AddChild(lists[target], kind));
                }
            });
        }
    });
    if let Some(field) = changed {
        outcome.edited = Some((definition, field));
    }
    outcome
}

/// The plain fields of a definition; returns the changed field.
fn definition_fields(
    ui: &mut egui::Ui,
    state: &mut EffectsPanelState,
    lookups: &Lookups,
    definition: &mut EffectDefinition,
    path: &[PathStep],
    entry: Option<usize>,
    outcome: &mut DefinitionOutcome,
) -> Option<&'static str> {
    let mut changed = None;
    let mut field = |ui: &mut egui::Ui, label: &str, tooltip: &str, name: &'static str, add: &mut dyn FnMut(&mut egui::Ui) -> bool| {
        ui.label(label).on_hover_text(tooltip);
        if add(ui) {
            changed = Some(name);
        }
        ui.end_row();
    };
    match definition {
        EffectDefinition::Particle(particle) => {
            let selection = match entry {
                Some(entry) => EffectsSelection::Definition { entry, path: path.to_vec() },
                None => EffectsSelection::TileArt { tile_id: 0, path: path.to_vec() },
            };
            if state.name_text_for.as_ref() != Some(&(selection.clone(), particle.name_index)) {
                state.name_text = lookups.particle_name(particle).unwrap_or_default();
                state.name_text_for = Some((selection, particle.name_index));
            }
            ui.label("Name").on_hover_text(
                "The .nif or .ems file in EffectTexture.uop. Stored as a string dictionary index: only names already \
                 in the string dictionary can be set.",
            );
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.name_text).desired_width(ID_FILTER_WIDTH * 2.0));
                let current = lookups.particle_name(particle).unwrap_or_default();
                if ui.add_enabled(!state.name_text.eq_ignore_ascii_case(&current), egui::Button::new("Set name")).clicked() {
                    outcome.action = Some(DefinitionAction::SetName(state.name_text.clone()));
                }
                if let Some(name) = lookups.particle_name(particle) {
                    let hash = effect_texture_hash(&name);
                    let exists = lookups.files.as_ref().is_some_and(|files| files.exists(state.document.files(), &name));
                    if ui.add_enabled(exists, egui::Button::new("Open file")).on_disabled_hover_text("Not in EffectTexture.uop.").clicked() {
                        outcome.select_file = Some(hash);
                    }
                    if !exists && lookups.files.is_some() {
                        ui.colored_label(ERROR_COLOR, "missing file");
                    }
                }
            });
            ui.end_row();
            field(ui, "Name index", "Client string dictionary index of the file name.", "name_index", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.name_index).speed(VALUE_DRAG_SPEED)).changed()
            });
            field(ui, "Offset x", "Tiles east: added to the world x of the effect (confirmed; Nif Init 0x0041aa6a, Chaos Init 0x0041b119).", "offset_x", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.offset_x).speed(FLOAT_DRAG_SPEED)).changed()
            });
            field(ui, "Offset y", "Tiles south: subtracted from the world z of the effect (confirmed; Nif Init 0x0041aa6a, Chaos Init 0x0041b119).", "offset_y", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.offset_y).speed(FLOAT_DRAG_SPEED)).changed()
            });
            field(ui, "Camera offset", "Moves the effect along the camera down vector by -camera_offset * 0.12856: a positive value moves it up on screen (confirmed; Nif Init 0x0041aa6a, Chaos Init 0x0041b119).", "camera_offset", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.camera_offset).speed(FLOAT_DRAG_SPEED)).changed()
            });
            field(ui, "Scale", "Scene node scale (confirmed); the client uses its absolute value.", "scale", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.scale).speed(FLOAT_DRAG_SPEED)).changed()
            });
            let attach_tooltip = format!(
                "{PARTICLE_ATTACH_PROJECTILE_ONLY}: plays only while the item is shown in flight (thrown models). \
                 Client data: 2 on thrown models, 0 on arrows, 1 elsewhere; 0 and 1 behave alike."
            );
            field(ui, "Attach mode", &attach_tooltip, "attach_mode", &mut |ui| {
                ui.add(egui::DragValue::new(&mut particle.attach_mode).speed(VALUE_DRAG_SPEED)).changed()
            });
            field(
                ui,
                "Duration (s)",
                "A non-looping effect stops once elapsed >= duration; a looping one wraps the elapsed time when it is \
                 non-zero. Independent of the packet duration.",
                "duration",
                &mut |ui| ui.add(egui::DragValue::new(&mut particle.duration).speed(FLOAT_DRAG_SPEED)).changed(),
            );
            field(ui, "Looping", "Restarts the effect when it ends (confirmed).", "looping", &mut |ui| {
                ui.checkbox(&mut particle.looping, "").changed()
            });
            field(
                ui,
                "field_3c",
                "Must be 0: the client refuses a Nif effect when it is 1 and a Chaos effect when it is non-zero.",
                "field_3c",
                &mut |ui| ui.add(egui::DragValue::new(&mut particle.field_3c).speed(VALUE_DRAG_SPEED)).changed(),
            );
            field(ui, "Hued", "Uses UOHuedParticleEffectNif when the object hue is non-zero (confirmed).", "hued", &mut |ui| {
                ui.checkbox(&mut particle.hued, "").changed()
            });
        }
        EffectDefinition::Sound(sound) => {
            field(ui, "Sound id", "Sound played with distance attenuation (confirmed).", "sound_id", &mut |ui| {
                ui.add(egui::DragValue::new(&mut sound.sound_id).speed(VALUE_DRAG_SPEED)).changed()
            });
            field(ui, "value_08", "Copied but never read by the client; 1.0 in all client data (a gain?).", "value_08", &mut |ui| {
                ui.add(egui::DragValue::new(&mut sound.value_08).speed(FLOAT_DRAG_SPEED)).changed()
            });
        }
        EffectDefinition::PositionOffset(effect) => {
            for (direction, offset) in effect.offsets.iter_mut().enumerate() {
                let label = format!("Direction {direction}");
                field(ui, &label, "x, y offset of this direction: the client reads it only from a mount's animation sequence, to place the rider.", "offsets", &mut |ui| {
                    ui.horizontal(|ui| {
                        let x = ui.add(egui::DragValue::new(&mut offset.x).speed(VALUE_DRAG_SPEED)).changed();
                        let y = ui.add(egui::DragValue::new(&mut offset.y).speed(VALUE_DRAG_SPEED)).changed();
                        x || y
                    })
                    .inner
                });
            }
        }
        EffectDefinition::Wait(effect) => {
            field(ui, "Min delay (ms)", "Lower bound of the random delay before the next fire (client default 2000).", "min_delay_ms", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.min_delay_ms).speed(VALUE_DRAG_SPEED)).changed()
            });
            field(ui, "Max delay (ms)", "Upper bound of the random delay before the next fire (client default 4000).", "max_delay_ms", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.max_delay_ms).speed(VALUE_DRAG_SPEED)).changed()
            });
            field(ui, "Aligned", "Aligns the next fire time to a multiple of the min delay (confirmed).", "aligned", &mut |ui| {
                ui.checkbox(&mut effect.aligned, "").changed()
            });
        }
        EffectDefinition::PropertyFlag(effect) => {
            field(
                ui,
                "Mode",
                "Flag mask: the key is (object flags & mask) != 0, no match plays nothing. Property: the key is the \
                 object value selected by property, no match plays the default children.",
                "mode",
                &mut |ui| {
                    let before = effect.mode;
                    egui::ComboBox::from_id_salt("effects_property_flag_mode")
                        .selected_text(property_flag_mode_label(effect.mode))
                        .show_ui(ui, |ui| {
                            for mode in [PropertyFlagMode::FlagMask, PropertyFlagMode::Property] {
                                ui.selectable_value(&mut effect.mode, mode, property_flag_mode_label(mode));
                            }
                        });
                    effect.mode != before
                },
            );
            field(ui, "Mask", "Object flag mask of the flag mask mode.", "mask", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.mask).hexadecimal(MASK_HEX_DIGITS, false, true).prefix("0x").speed(NO_DRAG_SPEED))
                    .changed()
            });
            field(
                ui,
                "Property",
                "Object value of the property mode: 2 = vt+0x80 & 0xffff, 3 = vt+0xb4, 7 = object hue, 8 = vt+0x68, any \
                 other value = that tileart props1 key.",
                "property",
                &mut |ui| ui.add(egui::DragValue::new(&mut effect.property).speed(VALUE_DRAG_SPEED)).changed(),
            );
        }
        EffectDefinition::Earthquake(effect) => {
            field(ui, "Amplitude", "Camera target x shake: amplitude * sin(elapsed * frequency * 2pi).", "amplitude", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.amplitude).speed(FLOAT_DRAG_SPEED)).changed()
            });
            field(ui, "Duration (s)", "The effect is finished when the elapsed time exceeds it.", "duration", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.duration).speed(FLOAT_DRAG_SPEED)).changed()
            });
            field(ui, "Frequency (Hz)", "Shake frequency.", "frequency", &mut |ui| {
                ui.add(egui::DragValue::new(&mut effect.frequency).speed(FLOAT_DRAG_SPEED)).changed()
            });
        }
        EffectDefinition::UserPreference(effect) => {
            field(
                ui,
                "Mask",
                "The children are skipped when all of these user effect setting bits are set (bit 0 = footsteps \
                 disabled), or when it is 1 and object vt+0xa0 is negative.",
                "mask",
                &mut |ui| {
                    ui.add(egui::DragValue::new(&mut effect.mask).hexadecimal(MASK_HEX_DIGITS, false, true).prefix("0x").speed(NO_DRAG_SPEED))
                        .changed()
                },
            );
        }
        EffectDefinition::RoundRobin(_) | EffectDefinition::Lod(_) | EffectDefinition::Random(_) => {}
    }
    changed
}

/// The keys of LOD levels and property flag cases and the weights of random entries.
fn keyed_lists_editor(
    ui: &mut egui::Ui,
    definition: &mut EffectDefinition,
    changed: &mut Option<&'static str>,
    outcome: &mut DefinitionOutcome,
) {
    let (title, add_label, tooltip) = match definition {
        EffectDefinition::Lod(_) => ("Levels", "Add level", "Particle LOD setting key: UI Low/Medium/High = 1..3."),
        EffectDefinition::PropertyFlag(_) => ("Cases", "Add case", "The key looked up (see the mode)."),
        EffectDefinition::Random(_) => ("Entries", "Add entry", "Percent: entries are tried in order with r in [0, 100)."),
        _ => return,
    };
    ui.separator();
    ui.strong(title).on_hover_text(tooltip);
    let mut remove = None;
    egui::Grid::new("effects_keyed_lists").num_columns(3).show(ui, |ui| match definition {
        EffectDefinition::Lod(effect) => {
            for (index, level) in effect.levels.iter_mut().enumerate() {
                ui.label(format!("#{index}"));
                if ui.add(egui::DragValue::new(&mut level.key).speed(VALUE_DRAG_SPEED)).changed() {
                    *changed = Some("level key");
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(ChildList::Level(index));
                }
                ui.end_row();
            }
        }
        EffectDefinition::PropertyFlag(effect) => {
            for (index, case) in effect.cases.iter_mut().enumerate() {
                ui.label(format!("#{index}"));
                if ui.add(egui::DragValue::new(&mut case.key).speed(VALUE_DRAG_SPEED)).changed() {
                    *changed = Some("case key");
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(ChildList::Case(index));
                }
                ui.end_row();
            }
        }
        EffectDefinition::Random(effect) => {
            for (index, entry) in effect.entries.iter_mut().enumerate() {
                ui.label(format!("#{index}"));
                if ui.add(egui::DragValue::new(&mut entry.weight).speed(FLOAT_DRAG_SPEED).suffix(" %")).changed() {
                    *changed = Some("weight");
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(ChildList::RandomEntry(index));
                }
                ui.end_row();
            }
        }
        _ => {}
    });
    if let EffectDefinition::Random(effect) = definition {
        let sum: f32 = effect.entries.iter().map(|entry| entry.weight).sum();
        let color = if (sum - RANDOM_WEIGHT_TOTAL).abs() < f32::EPSILON { OK_COLOR } else { WARNING_COLOR };
        ui.colored_label(color, format!("Sum {sum} % of {RANDOM_WEIGHT_TOTAL} %"));
    }
    if ui.button(add_label).clicked() {
        outcome.action = Some(DefinitionAction::AddKeyedList);
    }
    if let Some(list) = remove {
        outcome.action = Some(DefinitionAction::RemoveKeyedList(list));
    }
}

fn apply_definition_outcome(
    state: &mut EffectsPanelState,
    lookups: &Lookups,
    entry: usize,
    path: &[PathStep],
    definition: EffectDefinition,
    outcome: DefinitionOutcome,
) {
    if let Some(hash) = outcome.select_file {
        select(state, Some(EffectsSelection::File { hash }), true);
        return;
    }
    if let Some((edited, field)) = outcome.edited {
        let key = Some(EffectsEditKey::Field { entry, path: path.to_vec(), field });
        state.document.edit_collection(key, |collection| replace_definition(collection, entry, path, edited));
    }
    let Some(action) = outcome.action else {
        return;
    };
    let selection = match action {
        DefinitionAction::Duplicate => state.document.edit_collection(None, |collection| duplicate_definition(collection, entry, path)),
        DefinitionAction::Remove => state.document.edit_collection(None, |collection| remove_definition(collection, entry, path)),
        DefinitionAction::Move(delta) => {
            state.document.edit_collection(None, |collection| move_definition(collection, entry, path, delta))
        }
        DefinitionAction::AddChild(list, kind) => {
            state.document.edit_collection(None, |collection| add_definition(collection, entry, path, list, kind))
        }
        DefinitionAction::AddKeyedList => {
            let mut edited = definition;
            add_keyed_list(&mut edited);
            state.document.edit_collection(None, |collection| replace_definition(collection, entry, path, edited));
            None
        }
        DefinitionAction::RemoveKeyedList(list) => {
            let mut edited = definition;
            remove_keyed_list(&mut edited, list);
            state.document.edit_collection(None, |collection| replace_definition(collection, entry, path, edited));
            None
        }
        DefinitionAction::SetName(name) => {
            let Some(index) = name_index(state, lookups).and_then(|index| index.name_index(&name)) else {
                state.error = Some(format!(
                    "{name:?} is not in the string dictionary: only existing names can be set (adding strings to \
                     string_dictionary.uop is not supported)."
                ));
                return;
            };
            let mut edited = definition;
            if let EffectDefinition::Particle(particle) = &mut edited {
                particle.name_index = index;
            }
            state.document.edit_collection(None, |collection| replace_definition(collection, entry, path, edited));
            None
        }
    };
    match selection {
        Some(selection) => select(state, Some(selection), false),
        None => resolve_selection(state),
    }
}

fn replace_definition(collection: &mut uocf::enhanced::effect_definition::EffectDefinitionCollection, entry: usize, path: &[PathStep], definition: EffectDefinition) {
    if let Some(target) = collection.entries.get_mut(entry).and_then(|entry| definition_at_mut(&mut entry.definitions, path)) {
        *target = definition;
    }
}

fn name_index(state: &mut EffectsPanelState, lookups: &Lookups) -> Option<Arc<NameIndex>> {
    let strings = lookups.strings.as_ref()?;
    let address = Arc::as_ptr(strings) as usize;
    if let Some((cached, index)) = &state.name_index {
        if *cached == address {
            return Some(Arc::clone(index));
        }
    }
    let index = Arc::new(NameIndex::new(strings));
    state.name_index = Some((address, Arc::clone(&index)));
    Some(index)
}

fn tileart_details(app: &mut UopInspectorApp, lookups: &Lookups, ui: &mut egui::Ui, tile_id: u32, path: &[PathStep]) {
    let state = &mut app.effects_panel;
    let Some(file) = lookups.tileart(tile_id) else {
        ui.label(format!("Tile 0x{tile_id:04X} is not in the loaded tileart.uop."));
        return;
    };
    let name = lookups.tile_name(file).unwrap_or_default();
    ui.heading(format!("Tile 0x{tile_id:0width$X} {name}", width = TILE_ID_HEX_DIGITS));
    ui.weak("Read only here: tileart effects are stored in tileart.uop, and the Tile Metadata tab edits them. They play on the items of this graphic.");
    ui.separator();
    let definitions = &file.entry.effects;
    let selected = (!path.is_empty()).then_some(path);
    let mut clicked = None;
    definition_tree(ui, definitions, selected, lookups, &mut clicked, ("tileart", tile_id as usize));
    if let Some(path) = clicked {
        select(state, Some(EffectsSelection::TileArt { tile_id, path }), false);
    }
    let Some(definition) = selected.and_then(|path| definition_at(definitions, path)) else {
        return;
    };
    ui.separator();
    let outcome = definition_editor(ui, state, lookups, definition, path, None, false);
    if let Some(hash) = outcome.select_file {
        select(state, Some(EffectsSelection::File { hash }), true);
    }
}

fn file_user_label(user: FileUser, state: &EffectsPanelState, lookups: &Lookups) -> String {
    match user {
        FileUser::Entry(index) => state
            .document
            .collection()
            .entries
            .get(index)
            .map_or_else(|| "entry ?".to_string(), |entry| format!("entry {}", entry_key_label(entry.key))),
        FileUser::TileArt(tile_id) => {
            let name = lookups.tileart(tile_id).and_then(|file| lookups.tile_name(file)).unwrap_or_default();
            format!("tile 0x{tile_id:0width$X} {name}", width = TILE_ID_HEX_DIGITS)
        }
        FileUser::Script(hash) => {
            let name = lookups.files.as_ref().and_then(|files| files.name(state.document.files(), hash));
            format!("script {}", name.unwrap_or_else(|| format!("{hash:016x}")))
        }
    }
}

fn file_user_selection(user: FileUser) -> EffectsSelection {
    match user {
        FileUser::Entry(entry) => EffectsSelection::Entry { entry },
        FileUser::TileArt(tile_id) => EffectsSelection::TileArt { tile_id, path: Vec::new() },
        FileUser::Script(hash) => EffectsSelection::File { hash },
    }
}

fn file_details(app: &mut UopInspectorApp, lookups: &Lookups, ctx: &egui::Context, ui: &mut egui::Ui, hash: u64) {
    let users = file_users(&mut app.effects_panel, lookups);
    let state = &mut app.effects_panel;
    let Some(files) = state.files.clone() else {
        return;
    };
    let (rows, _) = file_rows(state);
    let Some(row) = rows.iter().find(|row| row.hash == hash).cloned() else {
        ui.label(format!("No file {hash:016x} in EffectTexture.uop."));
        return;
    };
    ui.heading(row.label());
    egui::Grid::new("effects_file_fields").num_columns(2).striped(true).show(ui, |ui| {
        ui.label("Path");
        ui.monospace(row.name.as_deref().map_or_else(|| "(unresolved: only the hash is known)".to_string(), effect_texture_path));
        ui.end_row();
        ui.label("Name hash");
        ui.monospace(format!("{:016x}", row.hash));
        ui.end_row();
        ui.label("Kind");
        ui.label(row.kind.label());
        ui.end_row();
        ui.label("Size");
        ui.label(format!("{} bytes", row.size));
        ui.end_row();
        if row.status != FileStatus::Original {
            ui.label("Change");
            ui.colored_label(WARNING_COLOR, row.status.label());
            ui.end_row();
        }
    });

    let mut select_user = None;
    ui.collapsing(format!("Used by ({})", users.get(&hash).map_or(0, Vec::len)), |ui| {
        match users.get(&hash) {
            None => {
                ui.weak("No collection entry, tileart record or script names this file.");
            }
            Some(list) => {
                for &user in list {
                    if ui.link(file_user_label(user, state, lookups)).clicked() {
                        select_user = Some(user);
                    }
                }
            }
        }
    });
    if let Some(user) = select_user {
        select(state, Some(file_user_selection(user)), true);
        return;
    }

    let named = row.name.clone();
    let overridden = state.document.files().contains_key(&hash) || state.document.nif_drafts().contains_key(&hash);
    ui.horizontal_wrapped(|ui| {
        let replace = ui
            .add_enabled(named.is_some(), egui::Button::new("Replace from disk..."))
            .on_hover_text(HEADER_HASH_NOTE)
            .on_disabled_hover_text("The name of this entry is unknown: it cannot be written.");
        if replace.clicked() {
            if let (Some(name), Some(path)) = (named.as_deref(), pick_file((IMPORT_FILTER_NAME, &IMPORTABLE_EXTENSIONS))) {
                import_file(state, &path, Some(name));
            }
        }
        if ui.add_enabled(row.status != FileStatus::Removed, egui::Button::new("Export file...")).clicked() {
            let default_name = row.name.clone().unwrap_or_else(|| format!("{hash:016x}.bin"));
            if let (Some(payload), Some(path)) =
                (files.payload(state.document.files(), hash), crate::dialog::file_dialog().set_file_name(&default_name).save_file())
            {
                match std::fs::write(&path, &payload) {
                    Ok(()) => state.error = None,
                    Err(error) => state.error = Some(format!("Export failed: {}: {error}", path.display())),
                }
            }
        }
        let can_remove = named.is_some() && row.status != FileStatus::Removed;
        if ui.add_enabled(can_remove, egui::Button::new("Remove file")).clicked() {
            if let Some(name) = named.clone() {
                state.document.edit(None, |data| data.set_file(hash, FileOverride { name, payload: None }));
            }
        }
        if ui.add_enabled(overridden, egui::Button::new("Restore original")).on_hover_text("Drops the pending change of this file.").clicked() {
            state.document.edit(None, |data| data.restore_file(hash));
        }
    });
    ui.separator();
    if row.status == FileStatus::Removed {
        ui.weak("Removed: the saved package will not contain this file.");
        return;
    }
    let Some(payload) = files.payload(state.document.files(), hash) else {
        return;
    };
    match row.kind {
        FileKind::Chaos => script_editor(state, lookups, ui, hash, row.label(), &payload),
        FileKind::Tga | FileKind::Dds => texture_view(state, ctx, ui, hash, &payload),
        FileKind::Nif => {
            let outcome = super::effects_nif::nif_editor(
                ui,
                &mut state.nif_editor,
                &mut state.document,
                &files,
                state.files_generation,
                hash,
                row.name.as_deref(),
            );
            match outcome {
                Some(Ok(message)) => {
                    state.error = None;
                    app.status_message = message;
                }
                Some(Err(error)) => state.error = Some(error),
                None => {}
            }
        }
        FileKind::Psd | FileKind::Other | FileKind::Empty => {
            ui.weak("No viewer for this kind of file.");
        }
    }
}

fn payload_address(payload: &Arc<[u8]>) -> usize {
    Arc::as_ptr(payload) as *const u8 as usize
}

fn script_editor(state: &mut EffectsPanelState, lookups: &Lookups, ui: &mut egui::Ui, hash: u64, name: String, payload: &Arc<[u8]>) {
    let overrides = state.document.files().clone();
    let files = lookups.files.clone();
    let exists = move |file: &str| files.as_ref().is_some_and(|files| files.exists(&overrides, file));
    let reload = state
        .script_editor
        .as_ref()
        .is_none_or(|editor| editor.hash != hash || !editor.source.as_ref().is_some_and(|source| Arc::ptr_eq(source, payload)));
    if reload {
        state.script_editor = Some(ScriptEditor {
            hash,
            name: name.clone(),
            text: latin1_decode(payload),
            source: Some(Arc::clone(payload)),
            issues: check_script(payload, Some(&exists)),
            encode_error: None,
        });
    }
    let Some(editor) = state.script_editor.as_mut() else {
        return;
    };
    let mut new_text = None;
    ui.horizontal(|ui| {
        ui.strong("Chaos script");
        if ui.button("Canonical format").on_hover_text("Rewrites the text in the canonical layout (undoable).").clicked() {
            match canonical_script(&editor.text) {
                Ok(text) => new_text = Some(text),
                Err(error) => editor.encode_error = Some(format!("Cannot reformat: {error}")),
            }
        }
        if ui.button("Export .ems...").clicked() {
            if let Some(path) = save_file(EMS_FILTER, &editor.name) {
                if let Err(error) = std::fs::write(&path, payload.as_ref()) {
                    editor.encode_error = Some(format!("Export failed: {}: {error}", path.display()));
                }
            }
        }
    });
    let lines: HashMap<usize, ScriptSeverity> = editor.issues.iter().fold(HashMap::new(), |mut lines, issue| {
        if let Some(line) = issue.line {
            let entry = lines.entry(line).or_insert(issue.severity);
            *entry = (*entry).max(issue.severity);
        }
        lines
    });
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let color = ui.visuals().text_color();
        let mut job = egui::text::LayoutJob::default();
        for (index, line) in text.as_str().split_inclusive('\n').enumerate() {
            let background = match lines.get(&(index + 1)) {
                Some(ScriptSeverity::Error) => ERROR_LINE_BACKGROUND,
                Some(ScriptSeverity::Warning) => WARNING_LINE_BACKGROUND,
                Some(ScriptSeverity::Note) | None => egui::Color32::TRANSPARENT,
            };
            job.append(line, 0.0, egui::TextFormat { font_id: font.clone(), color, background, ..Default::default() });
        }
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let response = ui.add(
        egui::TextEdit::multiline(&mut editor.text)
            .code_editor()
            .desired_rows(SCRIPT_EDITOR_ROWS)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );
    if response.changed() {
        new_text = Some(editor.text.clone());
    }
    if let Some(error) = &editor.encode_error {
        ui.colored_label(ERROR_COLOR, error);
    }
    if editor.issues.is_empty() {
        ui.colored_label(OK_COLOR, "No issues.");
    }
    for issue in &editor.issues {
        let (icon, color) = severity_style(issue.severity);
        ui.horizontal(|ui| {
            ui.colored_label(color, icon);
            ui.label(issue.to_string());
        });
    }
    let references = crate::logic::effect_files::script_references(payload);
    let mut open = None;
    if !references.is_empty() {
        ui.separator();
        ui.strong("Files");
        for reference in references {
            let present = exists(&reference);
            ui.horizontal(|ui| {
                if ui.add_enabled(present, egui::Link::new(&reference)).clicked() {
                    open = Some(effect_texture_hash(&reference));
                }
                if !present {
                    ui.colored_label(ERROR_COLOR, "missing");
                }
            });
        }
    }

    if let Some(text) = new_text {
        match latin1_encode(&text) {
            Ok(bytes) => {
                let bytes: Arc<[u8]> = Arc::from(bytes);
                editor.text = text;
                editor.encode_error = None;
                editor.issues = check_script(&bytes, Some(&exists));
                editor.source = Some(Arc::clone(&bytes));
                let name = editor.name.clone();
                state.document.edit(Some(EffectsEditKey::FileText { hash }), |data| {
                    data.files.insert(hash, FileOverride { name, payload: Some(bytes) });
                });
            }
            Err(error) => editor.encode_error = Some(error),
        }
    }
    if let Some(hash) = open {
        select(state, Some(EffectsSelection::File { hash }), true);
    }
}

fn texture_view(state: &mut EffectsPanelState, ctx: &egui::Context, ui: &mut egui::Ui, hash: u64, payload: &Arc<[u8]>) {
    let address = state.document.files().get(&hash).map_or(0, |_| payload_address(payload));
    let stale = state.thumbnails.get(&hash).is_none_or(|thumbnail| thumbnail.payload != address);
    if stale {
        let texture = decode_texture(payload).and_then(|image| {
            let size = [image.width() as usize, image.height() as usize];
            let name = format!("effects_texture_{hash:016x}_{address}");
            super::textures::load_rgba(ctx, name, size, image.as_raw(), egui::TextureOptions::LINEAR)
        });
        state.thumbnails.insert(hash, Thumbnail { payload: address, texture });
    }
    match &state.thumbnails[&hash].texture {
        Ok(handle) => {
            let size = handle.size_vec2();
            ui.label(format!("{} x {}", size.x, size.y));
            let scale = (THUMBNAIL_MAX_SIDE / size.x.max(size.y)).min(1.0);
            ui.add(egui::Image::new(handle).fit_to_exact_size(size * scale));
        }
        Err(error) => {
            ui.colored_label(ERROR_COLOR, format!("Cannot decode: {error}"));
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Preview

fn preview_target(selection: &Option<EffectsSelection>, lookups: &Lookups, state: &EffectsPanelState) -> Option<PreviewTarget> {
    match selection.as_ref()? {
        EffectsSelection::Entry { entry } => Some(PreviewTarget::Entry(*entry)),
        EffectsSelection::Definition { entry, path } => Some(PreviewTarget::Definition(*entry, path.clone())),
        EffectsSelection::TileArt { tile_id, path } => Some(PreviewTarget::TileArt(*tile_id, path.clone())),
        EffectsSelection::File { hash } => {
            let files = lookups.files.as_ref()?;
            let payload = files.payload(state.document.files(), *hash)?;
            matches!(FileKind::detect(&payload, None), FileKind::Chaos | FileKind::Nif).then_some(PreviewTarget::Script(*hash))
        }
    }
}

/// Builds the scene of the target when anything it depends on changed.
fn refresh_preview(state: &mut EffectsPanelState, lookups: &Lookups) {
    let target = preview_target(&state.selection, lookups, state);
    let Some(target) = target else {
        state.preview.key = None;
        state.preview.scene = None;
        state.preview.simulation = None;
        state.preview.label = "Select an entry, a definition, a tileart record, a Chaos script or a NIF file.".to_string();
        return;
    };
    let preview = &mut state.preview;
    preview.settings.move_speed = if preview.moving { preview.move_speed } else { 0.0 };
    preview.settings.duration_override = preview.duration_override.then_some(preview.duration_s);
    let key = PreviewKey {
        target: target.clone(),
        document: (state.document_generation, state.document.revision()),
        files: (state.files_generation, state.document.revision()),
        strings: arc_address(&lookups.strings),
        tileart: lookups.tileart_key(),
        settings: preview.settings,
    };
    if preview.key.as_ref() == Some(&key) {
        return;
    }
    let overrides = state.document.files().clone();
    let files = lookups.files.clone();
    let payload = move |name: &str| files.as_ref().and_then(|files| files.payload(&overrides, effect_texture_hash(name)));
    let drafts = state.document.nif_drafts().clone();
    let nif_drafts = move |name: &str| drafts.get(&effect_texture_hash(name)).map(|draft| Arc::clone(&draft.nif));
    let input = PreviewInput { strings: lookups.strings.as_deref(), payload: &payload, nif_drafts: Some(&nif_drafts) };
    let collection = state.document.collection();
    let settings = preview.settings;
    let definitions = target_definitions(&target, &state.document, lookups);
    let (label, scene) = match &target {
        PreviewTarget::Entry(entry) => {
            let id = collection.entries.get(*entry).map_or_else(String::new, |entry| entry_key_label(entry.key));
            (format!("Entry {id}: all top-level definitions"), PreviewScene::build(&definitions, &settings, &input))
        }
        PreviewTarget::Definition(..) => ("Selected definition".to_string(), PreviewScene::build(&definitions, &settings, &input)),
        PreviewTarget::TileArt(tile_id, _) => {
            (format!("Tile 0x{tile_id:04X}"), PreviewScene::build_tileart(&definitions, &settings, &input))
        }
        PreviewTarget::Script(hash) => {
            let name = lookups.files.as_ref().and_then(|files| files.name(state.document.files(), *hash)).unwrap_or_default();
            (format!("File {name} (default particle fields)"), PreviewScene::build_script(&name, &settings, &input))
        }
    };
    let target_changed = preview.key.as_ref().is_none_or(|old| old.target != target);
    if target_changed {
        preview.clock_ms = 0.0;
        preview.view.pan = [0.0; 2];
    }
    preview.bounds = fit_bounds(&scene, &settings);
    preview.simulation = None;
    preview.scene = Some(Arc::new(scene));
    preview.label = label;
    preview.key = Some(key);
    preview.build += 1;
}

fn preview_panel(app: &mut UopInspectorApp, lookups: &Lookups, ctx: &egui::Context, ui: &mut egui::Ui) {
    let hue_names = app.ec_hues.clone();
    let state = &mut app.effects_panel;
    refresh_preview(state, lookups);
    let preview = &mut state.preview;
    if preview.settings.hue != particle_sim::hued::NO_HUE {
        preview.hue_atlas(lookups.client_dir.as_deref());
    }
    let hue_atlas = preview.loaded_hue_atlas();
    ui.heading("Preview");
    ui.label(&preview.label);
    let Some(scene) = preview.scene.clone() else {
        return;
    };
    let timeline_ms = preview.settings.timeline_ms() as f32;

    // Playback clock.
    if preview.playing {
        let elapsed_s = ui.input(|input| input.stable_dt) * preview.speed;
        (preview.clock_ms, preview.playing) = advance_clock(preview.clock_ms, elapsed_s, timeline_ms, preview.repeat);
        ctx.request_repaint();
    }
    advance_simulation(preview, &scene);
    if let Some(simulation) = preview.simulation.as_mut() {
        simulation.set_lighting(preview.view.lighting);
    }

    // Frame.
    let view = preview_view(preview.bounds, &preview.view, PREVIEW_RESOLUTION_PX, PREVIEW_RESOLUTION_PX);
    if let Some(simulation) = &preview.simulation {
        let stamp = (preview.build, simulation.time_ms(), preview.view, hue_atlas.is_some());
        if preview.rendered != Some(stamp) || preview.texture.is_none() {
            let canvas = render_canvas(&scene, simulation, &view, &preview.view, hue_atlas.as_deref());
            let size = [canvas.width, canvas.height];
            let rgba = canvas.to_rgba8();
            let max_side = ctx.input(|input| input.max_texture_side);
            match super::textures::rgba_image_problem(size, rgba.len(), max_side) {
                Some(problem) => state.error = Some(format!("Cannot show the preview frame: {problem}.")),
                None => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    match preview.texture.as_mut() {
                        Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                        None => preview.texture = Some(ctx.load_texture("effects_preview", image, egui::TextureOptions::LINEAR)),
                    }
                }
            }
            preview.rendered = Some(stamp);
        }
    }
    egui::Panel::top("effects_preview_view")
        .frame(egui::Frame::NONE)
        .resizable(true)
        .default_size(ui.available_height() * PREVIEW_VIEW_DEFAULT_SHARE)
        .min_size(PREVIEW_VIEW_MIN_HEIGHT)
        .show(ui, |ui| {
            if let Some(texture) = &preview.texture {
                let controls_height = PREVIEW_CONTROL_ROWS * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
                let room = (ui.available_height() - controls_height).max(MIN_PREVIEW_IMAGE_SIDE);
                let side = ui.available_width().min(room).min(PREVIEW_RESOLUTION_PX as f32);
                let image = egui::Image::new(texture).fit_to_exact_size(egui::vec2(side, side)).sense(egui::Sense::click_and_drag());
                let response = ui.vertical_centered(|ui| ui.add(image)).inner;
                let response = response.on_hover_text(
                    "Drag to pan, right-drag to turn the camera, Ctrl or Shift + wheel (or pinch) to zoom, double-click to reset \
                     the view.",
                );
                if response.dragged_by(egui::PointerButton::Primary) {
                    let scale = PREVIEW_RESOLUTION_PX as f32 / side;
                    let delta = response.drag_delta() * scale;
                    preview.view.pan[0] += delta.x;
                    preview.view.pan[1] += delta.y;
                }
                if response.dragged_by(egui::PointerButton::Secondary) {
                    let delta = response.drag_delta() * ORBIT_DEGREES_PER_POINT;
                    preview.view.orbit = preview.view.orbit.turned(-delta.x, delta.y);
                }
                if response.hovered() {
                    let zoom = ui.input(|input| input.zoom_delta());
                    if zoom != 1.0 {
                        preview.view.zoom = (preview.view.zoom * zoom).clamp(MIN_ZOOM, MAX_ZOOM);
                    }
                }
                if response.double_clicked() {
                    preview.view.zoom = 1.0;
                    preview.view.pan = [0.0; 2];
                    preview.view.orbit = CameraOrbit::default();
                }
            }

            // Transport.
            ui.horizontal(|ui| {
                if ui.button(if preview.playing { PAUSE_ICON } else { PLAY_ICON }).on_hover_text("Play / pause").clicked() {
                    preview.playing = !preview.playing;
                    if preview.playing && preview.clock_ms >= timeline_ms {
                        preview.clock_ms = 0.0;
                    }
                }
                if ui.button(RESTART_ICON).on_hover_text("Restart from 0").clicked() {
                    preview.clock_ms = 0.0;
                }
                if timeline_slider(ui, &mut preview.clock_ms, timeline_ms)
                    .on_hover_text("Scrub: re-simulates from 0 with the same seed.")
                    .changed()
                {
                    preview.playing = false;
                }
            });
            if let Some(simulation) = &preview.simulation {
                ui.weak(format!(
                    "t = {:.2} s, {} particles{}",
                    simulation.time_ms() as f32 / MS_PER_SECOND,
                    simulation.particle_count(),
                    if simulation.is_drained() { ", finished" } else { "" }
                ));
            }

            if !scene.has_playing_parts() {
                ui.colored_label(WARNING_COLOR, "Nothing previewable plays (see Parts).");
            }

            // A panel keeps the height of its content: without this it would stay as small as it ever was.
            ui.add_space(ui.available_height().max(0.0));
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut preview.section, PreviewSection::Settings, "Settings");
            ui.selectable_value(&mut preview.section, PreviewSection::Parts, format!("Parts ({})", scene.parts.len()));
            ui.selectable_value(&mut preview.section, PreviewSection::Export, "Export");
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("effects_preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match preview.section {
                PreviewSection::Settings => {
                    egui::Grid::new("effects_preview_settings").num_columns(2).show(ui, |ui| {
                        ui.label("Speed");
                        ui.add(egui::Slider::new(&mut preview.speed, MIN_PLAYBACK_SPEED..=MAX_PLAYBACK_SPEED).logarithmic(true).suffix("x"));
                        ui.end_row();
                        ui.label("Timeline");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut preview.settings.timeline_s).range(MIN_TIMELINE_S..=MAX_TIMELINE_S).speed(FLOAT_DRAG_SPEED).suffix(" s"));
                            ui.checkbox(&mut preview.repeat, "repeat");
                        });
                        ui.end_row();
                        ui.label("Loop").on_hover_text("The looping flag given to the particles.");
                        egui::ComboBox::from_id_salt("effects_preview_loop").selected_text(preview.settings.loop_mode.label()).show_ui(ui, |ui| {
                            for mode in LoopMode::ALL {
                                ui.selectable_value(&mut preview.settings.loop_mode, mode, mode.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Duration").on_hover_text("Replaces every particle's duration (seconds; for Chaos, the emission time of a looping effect, <= 0: forever).");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut preview.duration_override, "override");
                            ui.add_enabled(preview.duration_override, egui::DragValue::new(&mut preview.duration_s).speed(FLOAT_DRAG_SPEED).suffix(" s"));
                        });
                        ui.end_row();
                        ui.label("Emitter");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut preview.moving, "moving");
                            ui.add_enabled(preview.moving, egui::Slider::new(&mut preview.move_speed, 0.0..=MAX_MOVE_SPEED).suffix(" /s"));
                        });
                        ui.end_row();
                        ui.label("Seed");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut preview.settings.seed).speed(SEED_DRAG_SPEED));
                            if ui.small_button("next").clicked() {
                                preview.settings.seed = preview.settings.seed.wrapping_add(1);
                            }
                        });
                        ui.end_row();
                        ui.label("LOD setting").on_hover_text("The particle LOD setting LOD definitions play for (UI Low/Medium/High = 1..3).");
                        ui.add(egui::Slider::new(&mut preview.settings.lod_level, 0..=MAX_LOD_LEVEL));
                        ui.end_row();
                        ui.label("Hue").on_hover_text(HUE_NOTE);
                        hue_control(ui, preview, hue_atlas.as_deref(), hue_names.as_deref());
                        ui.end_row();
                        ui.label("Tileart").on_hover_text(
                            "Tileart particles with attach mode 2 (thrown models): UOObject::CreateComponents does not play them on \
                             the world object; they play only in flight.",
                        );
                        ui.checkbox(&mut preview.settings.play_projectile_only, "play attach mode 2 particles anyway");
                        ui.end_row();
                        ui.label("Background");
                        ui.color_edit_button_rgba_unmultiplied(&mut preview.view.background);
                        ui.end_row();
                        ui.label("Overlays");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut preview.view.show_bounds, "bounds");
                            ui.checkbox(&mut preview.view.show_emitter, "emitter");
                        });
                        ui.end_row();
                        ui.label("Show");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut preview.view.show_particles, "particles");
                            ui.checkbox(&mut preview.view.show_meshes, "meshes").on_hover_text("NIF geometry and mesh particles.");
                        });
                        ui.end_row();
                        ui.label("Camera").on_hover_text(
                            "Orbit about the view centre. The default is the client's view: effects are camera-aligned billboards, \
                             so the client sees an effect's local frame orthographically along +y with +z up (x right). Changing \
                             it re-simulates (NIF billboards face the camera).",
                        );
                        ui.horizontal(|ui| {
                            let orbit = &mut preview.view.orbit;
                            ui.add(
                                egui::DragValue::new(&mut orbit.yaw_degrees)
                                    .range(MIN_CAMERA_YAW_DEGREES..=MAX_CAMERA_YAW_DEGREES)
                                    .prefix("yaw ")
                                    .suffix(" deg"),
                            )
                            .on_hover_text("Degrees from the client's view direction (+y) toward +x, about +z.");
                            ui.add(
                                egui::DragValue::new(&mut orbit.pitch_degrees)
                                    .range(MIN_CAMERA_PITCH_DEGREES..=MAX_CAMERA_PITCH_DEGREES)
                                    .prefix("pitch ")
                                    .suffix(" deg"),
                            )
                            .on_hover_text("Degrees above the client's view (looking down along +y from +z).");
                            if ui.small_button("Client view").on_hover_text("Orthographic along local +y, +z up.").clicked() {
                                *orbit = CameraOrbit::default();
                            }
                        });
                        ui.end_row();
                        ui.label("Lighting").on_hover_text(LIGHTING_NOTE);
                        ui.horizontal(|ui| {
                            ui.color_edit_button_rgb(&mut preview.view.lighting.ambient).on_hover_text("Ambient light");
                            ui.label("ambient");
                            ui.color_edit_button_rgb(&mut preview.view.lighting.diffuse).on_hover_text("Diffuse light (no direction)");
                            ui.label("diffuse");
                            if ui.small_button("Client").on_hover_text("No light, as the client").clicked() {
                                preview.view.lighting = particle_sim::render::SceneLighting::default();
                            }
                        });
                        ui.end_row();
                        ui.label("Zoom");
                        ui.horizontal(|ui| {
                            ui.add(egui::Slider::new(&mut preview.view.zoom, MIN_ZOOM..=MAX_ZOOM).logarithmic(true));
                            if ui.small_button("Reset view").clicked() {
                                preview.view.zoom = 1.0;
                                preview.view.pan = [0.0; 2];
                            }
                        });
                        ui.end_row();
                    });

                    ui.weak(LIGHTING_NOTE);
                    if preview.settings.hue != particle_sim::hued::NO_HUE {
                        match &preview.hue_atlas {
                            Some((_, Err(error))) => ui.colored_label(WARNING_COLOR, format!("Hued parts are drawn unhued: {error}")),
                            None => ui.colored_label(WARNING_COLOR, "Hued parts are drawn unhued: no client directory for Hues.uop."),
                            Some((_, Ok(_))) => match &preview.simulation {
                                Some(simulation) if !simulation.has_hued_parts() => {
                                    ui.weak("No part plays hued (see the parts: Chaos, or hued not set).")
                                }
                                _ => ui.weak("Hued NIF parts: half speed, drawn through the hue atlas."),
                            },
                        };
                    }

                }
                PreviewSection::Parts => {
                    for (index, part) in scene.parts.iter().enumerate() {
                        let color = if part.status.plays() { OK_COLOR } else { WARNING_COLOR };
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(crate::logic::effect_preview::path_label(&part.path));
                            ui.colored_label(color, format!("{}: {}", part.kind.label(), part.status.describe()));
                        });
                        let warnings = part.status.warnings();
                        if !warnings.is_empty() {
                            egui::CollapsingHeader::new(format!("{} compile warning(s)", warnings.len()))
                                .id_salt(("effects_preview_part_warnings", index))
                                .show(ui, |ui| {
                                    for warning in warnings {
                                        ui.colored_label(WARNING_COLOR, warning);
                                    }
                                });
                        }
                    }
                    for note in &scene.notes {
                        ui.weak(note);
                    }
                    for problem in &scene.texture_problems {
                        ui.colored_label(WARNING_COLOR, problem);
                    }

                }
                PreviewSection::Export => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Export");
                        ui.add(egui::DragValue::new(&mut preview.export_frames).range(1..=MAX_EXPORT_FRAMES).suffix(" frames"));
                        let export_sequence = ui.button("PNG sequence...").clicked();
                        let export_sheet = ui.button("Contact sheet...").clicked();
                        if export_sequence || export_sheet {
                            let times = export_times(&preview.settings, preview.export_frames);
                            let frames = render_frames(&scene, &preview.settings, &view, &preview.view, hue_atlas.as_deref(), &times);
                            let result = if export_sequence {
                                crate::dialog::pick_folder()
                                    .map(|dir| write_png_sequence(&frames, &dir, "effect").map(|count| format!("Wrote {count} frames to {}.", dir.display())))
                            } else {
                                save_file(PNG_FILTER, DEFAULT_CONTACT_SHEET_FILE_NAME).map(|path| {
                                    contact_sheet(&frames)
                                        .save(&path)
                                        .map(|()| format!("Wrote the contact sheet {}.", path.display()))
                                        .map_err(|error| format!("failed to write {}: {error}", path.display()))
                                })
                            };
                            match result {
                                Some(Ok(message)) => {
                                    state.error = None;
                                    app.status_message = message;
                                }
                                Some(Err(error)) => state.error = Some(format!("Export failed: {error}")),
                                None => {}
                            }
                        }
                        ui.separator();
                        let target = preview.key.as_ref().map(|key| key.target.clone());
                        let gltf = ui
                            .add_enabled(target.is_some() && scene.has_playing_parts(), egui::Button::new("glTF..."))
                            .on_hover_text(format!("{}\n{EFFECT_GLTF_NOTE}", super::effects_nif::GLTF_EXPORT_NOTE))
                            .on_disabled_hover_text("Nothing previewable plays.");
                        ui.checkbox(&mut preview.gltf_bake, "bake particles").on_hover_text(super::effects_nif::GLTF_BAKE_NOTE);
                        if let (true, Some(target)) = (gltf.clicked(), target) {
                            let name = target_export_name(&target, &state.document, lookups);
                            if let Some(path) = save_file(GLB_FILTER, &glb_file_name(&name)) {
                                let definitions = target_definitions(&target, &state.document, lookups);
                                let overrides = state.document.files().clone();
                                let files = lookups.files.clone();
                                let payload =
                                    move |file: &str| files.as_ref().and_then(|files| files.payload(&overrides, effect_texture_hash(file)));
                                let drafts = state.document.nif_drafts().clone();
                                let nif_drafts = move |file: &str| drafts.get(&effect_texture_hash(file)).map(|draft| Arc::clone(&draft.nif));
                                let sources = GltfSources { payload: &payload, nif_drafts: &nif_drafts };
                                let options = super::effects_nif::gltf_options(preview.gltf_bake);
                                match write_glb(&path, &export_gltf_effect(&name, &scene, &definitions, &sources, &options)) {
                                    Ok(message) => {
                                        state.error = None;
                                        app.status_message = message;
                                    }
                                    Err(error) => state.error = Some(error),
                                }
                            }
                        }
                    });
                }
            });
    });
}

/// What the whole-effect glTF export adds.
const EFFECT_GLTF_NOTE: &str = "Exports the parts this preview plays, each at its offset and scale as the client \
     view places it (Chaos parts: their script and MESH .nif files). Position offsets and the preview's duration, \
     loop and movement overrides are not applied.";

/// The definition list the preview of `target` plays.
fn target_definitions(target: &PreviewTarget, document: &EffectsDocument, lookups: &Lookups) -> Vec<EffectDefinition> {
    let collection = document.collection();
    match target {
        PreviewTarget::Entry(entry) => collection.entries.get(*entry).map(|entry| entry.definitions.clone()).unwrap_or_default(),
        PreviewTarget::Definition(entry, path) => collection
            .entries
            .get(*entry)
            .and_then(|entry| definition_at(&entry.definitions, path))
            .cloned()
            .into_iter()
            .collect(),
        PreviewTarget::TileArt(tile_id, path) => {
            let all = lookups.tileart(*tile_id).map(|file| file.entry.effects.as_slice()).unwrap_or_default();
            if path.is_empty() {
                all.to_vec()
            } else {
                definition_at(all, path).cloned().into_iter().collect()
            }
        }
        // `PreviewScene::build_script` plays the file as one particle with default fields.
        PreviewTarget::Script(_) => vec![DefinitionKind::Particle.new_definition()],
    }
}

/// The name of a glTF export of `target`.
fn target_export_name(target: &PreviewTarget, document: &EffectsDocument, lookups: &Lookups) -> String {
    let collection = document.collection();
    let entry_label = |entry: usize| collection.entries.get(entry).map_or_else(String::new, |entry| entry_key_label(entry.key));
    match target {
        PreviewTarget::Entry(entry) => format!("effect_{}", entry_label(*entry)),
        PreviewTarget::Definition(entry, path) => {
            format!("effect_{}_{}", entry_label(*entry), crate::logic::effect_preview::path_label(path))
        }
        PreviewTarget::TileArt(tile_id, _) => format!("tileart_0x{tile_id:04X}"),
        PreviewTarget::Script(hash) => {
            lookups.files.as_ref().and_then(|files| files.name(document.files(), *hash)).unwrap_or_else(|| format!("{hash:016x}"))
        }
    }
}

/// The hue of the preview: a number, a swatch of its gradient and a picker of the client's hues.
fn hue_control(
    ui: &mut egui::Ui,
    preview: &mut PreviewPanel,
    atlas: Option<&particle_sim::raster::RasterTexture>,
    names: Option<&uocf::enhanced::hues::EcHuePackage>,
) {
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut preview.settings.hue).range(0..=MAX_PREVIEW_HUE))
            .on_hover_text("0: no hue.");
        hue_swatch(ui, atlas, preview.settings.hue);
        let name = u16::try_from(preview.settings.hue).ok().and_then(|hue| names?.hue_name(hue));
        if let Some(name) = name {
            ui.weak(name);
        }
        ui.menu_button("Pick...", |ui| {
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.text_edit_singleline(&mut preview.hue_filter);
            });
            let filter = preview.hue_filter.trim().to_ascii_lowercase();
            let hues: Vec<(u32, &str)> = (1..=u32::from(uocf::enhanced::hues::MAX_EC_HUES))
                .map(|hue| (hue, u16::try_from(hue).ok().and_then(|hue| names?.hue_name(hue)).unwrap_or("")))
                .filter(|(hue, name)| {
                    filter.is_empty() || hue.to_string().contains(&filter) || name.to_ascii_lowercase().contains(&filter)
                })
                .collect();
            egui::ScrollArea::vertical().max_height(HUE_PICKER_HEIGHT).show_rows(ui, HUE_PICKER_ROW_HEIGHT, hues.len(), |ui, rows| {
                for &(hue, name) in &hues[rows] {
                    ui.horizontal(|ui| {
                        hue_swatch(ui, atlas, hue);
                        if ui.selectable_label(preview.settings.hue == hue, format!("{hue} {name}")).clicked() {
                            preview.settings.hue = hue;
                            ui.close_kind(egui::UiKind::Menu);
                        }
                    });
                }
            });
        });
        if preview.settings.hue != particle_sim::hued::NO_HUE && ui.small_button("None").clicked() {
            preview.settings.hue = particle_sim::hued::NO_HUE;
        }
    });
}

/// The gradient of hue `hue` in the atlas, dark to bright (the entries the shader aims at); empty without the
/// atlas or a hue.
fn hue_swatch(ui: &mut egui::Ui, atlas: Option<&particle_sim::raster::RasterTexture>, hue: u32) {
    use particle_sim::hued::{hue_lookup, HueAtlasSampling, NO_HUE};
    let (rect, _) = ui.allocate_exact_size(HUE_SWATCH_SIZE, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, 0.0, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let Some(atlas) = atlas.filter(|_| hue != NO_HUE) else {
        return;
    };
    let width = rect.width() / HUE_SWATCH_SAMPLES as f32;
    for sample in 0..HUE_SWATCH_SAMPLES {
        let luminance = sample as f32 / (HUE_SWATCH_SAMPLES - 1) as f32;
        let [red, green, blue, alpha] = hue_lookup(atlas, hue, luminance, HueAtlasSampling::IntendedEntry);
        let color = egui::Rgba::from_rgba_unmultiplied(red, green, blue, alpha);
        let left = rect.left() + sample as f32 * width;
        let cell = egui::Rect::from_min_max(egui::pos2(left, rect.top()), egui::pos2(left + width, rect.bottom()));
        painter.rect_filled(cell, 0.0, color);
    }
}

/// The playback clock (milliseconds) `elapsed_s` seconds later, and whether it still plays: it wraps around a
/// repeating timeline and stops at the end of one that does not repeat.
fn advance_clock(clock_ms: f32, elapsed_s: f32, timeline_ms: f32, repeat: bool) -> (f32, bool) {
    let clock_ms = clock_ms + elapsed_s * MS_PER_SECOND;
    if clock_ms < timeline_ms {
        (clock_ms, true)
    } else if repeat && timeline_ms > 0.0 {
        (clock_ms % timeline_ms, true)
    } else {
        (timeline_ms, false)
    }
}

/// The scrubber of the playback clock (milliseconds), which changes the clock only when it is moved. It must not
/// clamp or round the clock it is given: the slider would then report a change on every frame the clock runs, and
/// the change stops the playback.
fn timeline_slider(ui: &mut egui::Ui, clock_ms: &mut f32, timeline_ms: f32) -> egui::Response {
    let mut seconds = *clock_ms / MS_PER_SECOND;
    let slider = egui::Slider::new(&mut seconds, 0.0..=timeline_ms / MS_PER_SECOND)
        .suffix(" s")
        .max_decimals(TIMELINE_DECIMALS)
        .clamping(egui::SliderClamping::Edits);
    let response = ui.add(slider);
    if response.changed() {
        *clock_ms = seconds * MS_PER_SECOND;
    }
    response
}

/// Brings the simulation to the playback clock: forward in steps, or re-simulated from 0 when going back or when
/// the camera turned.
fn advance_simulation(preview: &mut PreviewPanel, scene: &PreviewScene) {
    let target = preview.clock_ms.max(0.0) as u32;
    let orbit = preview.view.orbit;
    let rebuild = preview
        .simulation
        .as_ref()
        .is_none_or(|simulation| target < simulation.time_ms() || simulation.orbit() != orbit);
    if rebuild {
        preview.simulation = Some(PreviewSimulation::at(scene, &preview.settings, orbit, target));
        preview.rendered = None;
    } else if let Some(simulation) = preview.simulation.as_mut() {
        simulation.advance_to(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::effect_preview::tests::{dictionary, SPARK_SCRIPT};
    use crate::logic::effect_preview::PartStatus;
    use crate::logic::effects_editor::tests::scratch_dir;
    use crate::logic::nif_editor::tests::{test_nif, test_nif_bytes, FLOAT_DATA_BLOCK, SHAPE_DATA_BLOCK};
    use uocf::enhanced::effect_definition::{EffectCollectionEntry, EffectDefinitionCollection};
    use uocf::uop_container::file::CompressionFlag;
    use uocf::uop_container::package::UopPackage;

    const SCRIPT_NAME: &str = "spark.ems";
    const NIF_NAME: &str = "smoke.nif";
    /// A NIF that does not parse (not in the string dictionary: shown by hash).
    const BROKEN_NIF_NAME: &str = "broken.nif";
    const TEXTURE_NAME: &str = "spark.tga";
    /// String dictionary indices of the names in [`dictionary`], shifted by one like the client data.
    const SCRIPT_NAME_INDEX: u32 = 1;
    const NIF_NAME_INDEX: u32 = 2;
    const NIF_HEADER: &[u8] = b"Gamebryo File Format, Version 20.0.0.4\n";
    const TGA_UNCOMPRESSED_TRUE_COLOR: u8 = 2;
    const TGA_BITS_PER_PIXEL: u8 = 32;
    /// Eight alpha bits, top-left origin.
    const TGA_DESCRIPTOR: u8 = 0x28;
    const TGA_SIDE: u16 = 2;
    const TGA_PIXEL: [u8; 4] = [255, 255, 255, 255];
    const FRAMES_PER_STATE: usize = 2;
    const ENTRY_ID: u32 = 5000;

    fn tga() -> Vec<u8> {
        let mut bytes = vec![0, 0, TGA_UNCOMPRESSED_TRUE_COLOR, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&TGA_SIDE.to_le_bytes());
        bytes.extend_from_slice(&TGA_SIDE.to_le_bytes());
        bytes.extend_from_slice(&[TGA_BITS_PER_PIXEL, TGA_DESCRIPTOR]);
        for _ in 0..TGA_SIDE * TGA_SIDE {
            bytes.extend_from_slice(&TGA_PIXEL);
        }
        bytes
    }

    fn particle(name_index: u32) -> EffectDefinition {
        let EffectDefinition::Particle(mut particle) = DefinitionKind::Particle.new_definition() else { unreachable!() };
        particle.name_index = name_index;
        EffectDefinition::Particle(particle)
    }

    /// One definition of every kind, the selectors holding particles.
    fn collection() -> EffectDefinitionCollection {
        let mut definitions = vec![particle(SCRIPT_NAME_INDEX), particle(NIF_NAME_INDEX)];
        for kind in DefinitionKind::ALL {
            let mut definition = kind.new_definition();
            // Selectors get one keyed list (LOD level, case, random entry) to hold a child.
            add_keyed_list(&mut definition);
            definitions.push(definition);
        }
        let mut collection = EffectDefinitionCollection {
            entries: vec![EffectCollectionEntry { key: EffectEntryKey::Id(ENTRY_ID), definitions }],
        };
        let count = collection.entries[0].definitions.len();
        for index in 0..count {
            let path = [PathStep::top(index)];
            let lists = child_lists(definition_at(&collection.entries[0].definitions, &path).unwrap())
                .into_iter()
                .map(|(list, _)| list)
                .collect::<Vec<_>>();
            for list in lists {
                add_definition(&mut collection, 0, &path, list, DefinitionKind::Particle);
            }
        }
        collection
    }

    fn files_package(dir: &Path) -> PathBuf {
        let path = dir.join(EFFECT_TEXTURE_UOP_NAME);
        let mut package = UopPackage::new_default();
        let files: [(&str, Vec<u8>); 4] = [
            (SCRIPT_NAME, SPARK_SCRIPT.as_bytes().to_vec()),
            (NIF_NAME, test_nif_bytes()),
            (BROKEN_NIF_NAME, NIF_HEADER.to_vec()),
            (TEXTURE_NAME, tga()),
        ];
        for (name, payload) in files {
            package.add_file_from_memory(&payload, &effect_texture_path(name), CompressionFlag::None).unwrap();
        }
        package.finalize_and_save(&path).unwrap();
        path
    }

    const WORKER_POLL: Duration = Duration::from_millis(5);

    fn run_frames(app: &mut UopInspectorApp, ctx: &egui::Context) {
        for _ in 0..FRAMES_PER_STATE {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                crate::ui::draw_ui(app, &ctx, ui);
            });
        }
    }

    /// Polls until the worker is done reading the `EffectTexture.uop`.
    fn wait_for_files(app: &mut UopInspectorApp) {
        let ctx = egui::Context::default();
        while app.effects_panel.files_load.is_some() {
            poll_files_load(app, &ctx);
            std::thread::sleep(WORKER_POLL);
        }
    }

    /// An app on the Effects tab with the test collection and files.
    fn effects_app(dir: &Path) -> UopInspectorApp {
        let strings = dictionary();
        let files = EffectFiles::open(&files_package(dir), Some(&strings)).unwrap();
        let mut app = crate::app::tests::test_app();
        app.view_mode = crate::app::ViewMode::Effects;
        app.uo_string_dictionary = Some(Arc::new(strings));
        let state = &mut app.effects_panel;
        state.files = Some(Arc::new(files));
        state.document = EffectsDocument::new(collection(), CollectionSource::None);
        app
    }

    fn key_press(key: egui::Key) -> egui::RawInput {
        let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        egui::RawInput { events: vec![event], ..Default::default() }
    }

    #[test]
    fn the_clock_wraps_around_a_repeating_timeline_and_stops_at_the_end_of_another() {
        const TIMELINE_MS: f32 = 1000.0;
        assert_eq!(advance_clock(100.0, 0.25, TIMELINE_MS, true), (350.0, true));
        let (clock_ms, playing) = advance_clock(900.0, 0.25, TIMELINE_MS, true);
        assert!(playing && (clock_ms - 150.0).abs() < 1e-3, "{clock_ms}");
        assert_eq!(advance_clock(900.0, 0.25, TIMELINE_MS, false), (TIMELINE_MS, false));
    }

    /// The bug that stopped the playback after one frame: the slider rounded the running clock to its decimals
    /// and reported that as a change, which is a scrub.
    #[test]
    fn the_timeline_does_not_report_a_running_clock_as_a_scrub() {
        const TIMELINE_MS: f32 = 2000.0;
        const RUNNING_CLOCK_MS: f32 = 16.666;
        let mut clock_ms = RUNNING_CLOCK_MS;
        let mut changed = None;
        let _ = egui::Context::default().run_ui(egui::RawInput::default(), |ui| {
            changed = Some(timeline_slider(ui, &mut clock_ms, TIMELINE_MS).changed());
        });
        assert_eq!(changed, Some(false));
        assert_eq!(clock_ms, RUNNING_CLOCK_MS);
    }

    #[test]
    fn the_preview_plays_on_over_frames() {
        const FRAMES: usize = 30;
        let dir = scratch_dir("playback");
        let mut app = effects_app(&dir);
        app.effects_panel.selection = Some(EffectsSelection::Entry { entry: 0 });
        let ctx = egui::Context::default();
        for _ in 0..FRAMES {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let ctx = ui.ctx().clone();
                crate::ui::draw_ui(&mut app, &ctx, ui);
            });
        }
        let preview = &app.effects_panel.preview;
        assert!(preview.playing, "still playing");
        let frame_ms = egui::RawInput::default().predicted_dt * MS_PER_SECOND;
        assert!(preview.clock_ms > frame_ms * (FRAMES / 2) as f32, "the clock ran: {} ms", preview.clock_ms);
        assert!(ctx.has_requested_repaint(), "a playing preview asks for the next frame");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_arrow_keys_browse_the_file_list() {
        let dir = scratch_dir("arrow_keys");
        let mut app = effects_app(&dir);
        app.effects_panel.view = BrowseView::Files;
        let ctx = egui::Context::default();
        let press = |app: &mut UopInspectorApp, key| {
            let _ = ctx.run_ui(key_press(key), |ui| {
                let ctx = ui.ctx().clone();
                crate::ui::draw_ui(app, &ctx, ui);
            });
        };
        let (rows, visible) = file_rows(&mut app.effects_panel);
        let hash_at = |row: usize| rows[visible[row]].hash;
        let selected = |app: &UopInspectorApp| match app.effects_panel.selection {
            Some(EffectsSelection::File { hash }) => Some(hash),
            _ => None,
        };
        press(&mut app, egui::Key::ArrowDown);
        assert_eq!(selected(&app), Some(hash_at(0)), "the first row");
        press(&mut app, egui::Key::ArrowDown);
        assert_eq!(selected(&app), Some(hash_at(1)));
        press(&mut app, egui::Key::ArrowUp);
        assert_eq!(selected(&app), Some(hash_at(0)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_effect_files_are_read_off_the_ui_thread() {
        let dir = scratch_dir("files_load");
        let mut app = crate::app::tests::test_app();
        perform_open(&mut app, OpenRequest::Files(files_package(&dir)));
        let state = &app.effects_panel;
        assert!(state.files_load.is_some() && state.files.is_none());
        let generation = state.files_generation;

        wait_for_files(&mut app);
        let state = &app.effects_panel;
        assert!(state.files.is_some() && state.error.is_none());
        assert!(state.files_generation > generation, "what depends on the files is worked out again");
    }

    #[test]
    fn a_package_that_cannot_be_read_is_reported() {
        let dir = scratch_dir("files_load_failure");
        let mut app = crate::app::tests::test_app();
        let missing = dir.join("missing.uop");
        perform_open(&mut app, OpenRequest::Files(missing.clone()));
        wait_for_files(&mut app);
        let state = &app.effects_panel;
        assert!(state.files.is_none());
        let error = state.error.as_deref().expect("the failure is shown");
        assert!(error.starts_with("Open failed") && error.contains("missing.uop"), "{error}");
    }

    /// Draws the tab in every browse view and with every kind of selection: nothing may panic and the preview
    /// plays the selected entry.
    #[test]
    fn the_tab_draws_every_selection() {
        let dir = scratch_dir("ui");
        let strings = dictionary();
        let files = EffectFiles::open(&files_package(&dir), Some(&strings)).unwrap();
        let mut app = crate::app::tests::test_app();
        app.view_mode = crate::app::ViewMode::Effects;
        app.uo_string_dictionary = Some(Arc::new(strings));
        let collection = collection();
        let top_level = collection.entries[0].definitions.len();
        let state = &mut app.effects_panel;
        state.files = Some(Arc::new(files));
        state.document = EffectsDocument::new(collection, CollectionSource::None);

        let ctx = egui::Context::default();
        let mut selections = vec![None, Some(EffectsSelection::Entry { entry: 0 })];
        selections.extend((0..top_level).map(|index| Some(EffectsSelection::Definition { entry: 0, path: vec![PathStep::top(index)] })));
        selections.extend(
            [SCRIPT_NAME, NIF_NAME, BROKEN_NIF_NAME, TEXTURE_NAME]
                .map(|name| Some(EffectsSelection::File { hash: effect_texture_hash(name) })),
        );
        for view in [BrowseView::Entries, BrowseView::TileArt, BrowseView::Files] {
            for selection in &selections {
                app.effects_panel.view = view;
                app.effects_panel.selection = selection.clone();
                run_frames(&mut app, &ctx);
            }
        }

        app.effects_panel.selection = Some(EffectsSelection::Entry { entry: 0 });
        app.effects_panel.guide_open = true;
        for section in [PreviewSection::Parts, PreviewSection::Export, PreviewSection::Settings] {
            app.effects_panel.preview.section = section;
            run_frames(&mut app, &ctx);
        }
        let scene = app.effects_panel.preview.scene.clone().expect("the entry is previewed");
        assert!(scene.has_playing_parts());
        let playing: Vec<_> = scene.parts.iter().filter(|part| part.status.plays()).map(|part| &part.status).collect();
        assert!(
            playing.iter().any(|status| matches!(status, PartStatus::Chaos { .. }))
                && playing.iter().any(|status| matches!(status, PartStatus::Nif { .. })),
            "the entry mixes Chaos and NIF parts: {playing:?}"
        );
        assert!(app.effects_panel.preview.texture.is_some(), "a frame is shown");
        let validation = app.effects_panel.validation.as_ref().expect("validation ran");
        assert!(!validation.checked.is_empty());

        app.effects_panel.selection = Some(EffectsSelection::File { hash: effect_texture_hash(SCRIPT_NAME) });
        run_frames(&mut app, &ctx);
        let editor = app.effects_panel.script_editor.as_ref().expect("the script is open");
        assert!(editor.text.contains("particlesystem"));
        let thumbnail = app.effects_panel.thumbnails.get(&effect_texture_hash(TEXTURE_NAME)).expect("the texture was shown");
        assert!(thumbnail.texture.is_ok(), "the texture decodes");

        // The NIF file: editor (tree and block list, the key table and the shape data with absent fields) and
        // its preview with a turned camera and meshes only.
        let nif_hash = effect_texture_hash(NIF_NAME);
        app.effects_panel.selection = Some(EffectsSelection::File { hash: nif_hash });
        run_frames(&mut app, &ctx);
        assert_eq!(app.effects_panel.nif_editor.shown().map(|nif| nif.blocks.len()), Some(test_nif().blocks.len()));
        for (list, block) in [(false, FLOAT_DATA_BLOCK), (true, SHAPE_DATA_BLOCK)] {
            app.effects_panel.nif_editor.show_block(list, block);
            app.effects_panel.preview.view.orbit = CameraOrbit { yaw_degrees: 30.0, pitch_degrees: 20.0 };
            app.effects_panel.preview.view.show_particles = false;
            run_frames(&mut app, &ctx);
        }
        let scene = app.effects_panel.preview.scene.clone().expect("the NIF file is previewed");
        assert!(matches!(scene.parts[0].status, PartStatus::Nif { meshes: 1, .. }), "{:?}", scene.parts[0].status);
        assert_eq!(app.effects_panel.preview.simulation.as_ref().map(PreviewSimulation::orbit), Some(app.effects_panel.preview.view.orbit));
        app.effects_panel.selection = Some(EffectsSelection::File { hash: effect_texture_hash(BROKEN_NIF_NAME) });
        run_frames(&mut app, &ctx);
        assert!(app.effects_panel.nif_editor.shown().is_none(), "the broken file does not parse");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
