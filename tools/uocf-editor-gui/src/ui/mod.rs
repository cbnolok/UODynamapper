use eframe::egui;
use crate::app::UocfEditorApp;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use uocf::client_config::login_servers::KINGDOM_REBORN_EXE;
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::utils::path::find_file_ignoring_case;

pub mod animation_definitions;
pub mod animations;
mod animation_frames;
mod animation_hue;
mod animation_list;
pub mod animated_items;
pub mod animdata;
pub mod animdata_editor;
pub mod art_viewer;
pub mod uop_browser;
mod legacy_terrain;
pub(crate) mod terrain_definition_editor;
pub mod multis;
mod multi_editor;
mod tileart_editor;
pub mod tiledata_editor;
mod tile_metadata;
mod tile_preview;
pub mod housing;
pub mod effects;
mod effects_guide;
pub mod effects_nif;
pub mod multimap;
pub mod multimap_editor;
pub mod hues;
pub mod hues_editor;
pub mod image_export;
pub mod gumps;
mod ec_paperdoll;
pub mod clilocs;
pub mod terrain_definition;
pub mod string_dictionary;
mod editor_bar;
pub mod sounds;
pub mod upscale_preview;
pub mod classic_maps;
mod classic_maps_convert;
pub mod classic_multi_editor;
pub mod classic_music;
pub mod ec_interface_textures;
pub mod classic_fonts;
pub mod classic_fonts_editor;
pub mod classic_defs;
pub mod classic_defs_editor;
pub mod classic_image_import;
pub mod textures;
pub(crate) mod unsaved_changes;

/// What the tests of the editors share.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    pub(crate) const EC_CLIENT_ROOTS_ENV: &str = "UO_EC_CLIENT_ROOTS";
    pub(crate) const KR_CLIENT_DIR_ENV: &str = "UO_KR_CLIENT_DIR";
    pub(crate) const CLASSIC_CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";

    /// The client folders the variable lists.
    pub(crate) fn client_roots(variable: &str) -> Vec<PathBuf> {
        let roots = std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable}"));
        std::env::split_paths(&roots).collect()
    }

    /// Where a test writes a file `name`: an absolute path under `TMPDIR`, which the runs on real clients point away
    /// from them.
    pub(crate) fn output_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("uocf-editor-{}-{name}", std::process::id()));
        assert!(path.is_absolute(), "{}", path.display());
        path
    }
}

/// The state of the Classic file views.
#[derive(Default)]
pub struct ClassicFileViews {
    pub maps: classic_maps::ClassicMapsState,
    pub music: classic_music::ClassicMusicState,
    pub fonts: classic_fonts::ClassicFontsState,
    pub defs: classic_defs::ClassicDefsState,
}

const HELP_MARK: &str = "(?)";

/// Hint shown wherever a `.uop` package is saved.
pub(crate) const UOP_PATCHER_NOTE: &str = "The official patcher re-downloads edited entries and deletes added ones: \
                                           keep edited packages in a client copy the patcher never runs on.";

/// A "(?)" next to something, whose tooltip explains it.
pub(crate) fn help_mark(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.weak(HELP_MARK).on_hover_cursor(egui::CursorIcon::Help).on_hover_text(text)
}

/// The Search Paths row under a folder's: what the folder is read as, a warning in the warning colour.
fn folder_status_row(ui: &mut egui::Ui, status: Option<(String, bool)>) {
    let Some((text, warning)) = status else {
        return;
    };
    ui.label("");
    if warning {
        ui.colored_label(ui.visuals().warn_fg_color, text);
    } else {
        ui.weak(text);
    }
    ui.end_row();
}

/// Whether the window `id` can be drawn in this frame. eframe draws a minimized window outside the event loop, where it
/// cannot create a window: asking for a new immediate viewport there panics. The window opens once the main one is back.
pub(crate) fn can_show_immediate_viewport(ctx: &egui::Context, id: egui::ViewportId) -> bool {
    ctx.input(|input| input.viewport().minimized != Some(true) || input.raw.viewports.contains_key(&id))
}

pub(super) fn arrow_delta(ui: &egui::Ui, disabled: bool) -> Option<isize> {
    if disabled {
        return None;
    }

    if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
        Some(1)
    } else if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
        Some(-1)
    } else {
        None
    }
}

pub(super) fn move_selection<T: Copy + Eq>(
    visible: &[T],
    selected: Option<T>,
    delta: isize,
) -> Option<T> {
    if visible.is_empty() {
        return selected;
    }

    let current = selected
        .and_then(|selected| visible.iter().position(|value| *value == selected));
    let next = match (current, delta) {
        (Some(index), delta) if delta < 0 => index.saturating_sub(1),
        (Some(index), delta) if delta > 0 => (index + 1).min(visible.len() - 1),
        (Some(index), _) => index,
        (None, delta) if delta < 0 => visible.len() - 1,
        (None, _) => 0,
    };

    Some(visible[next])
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ListSortState {
    pub option_index: usize,
    pub ordering: ListSortOrdering,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ListSortOrdering {
    None,
    Ascending,
    Descending,
}

impl ListSortState {
    pub fn ordering(self) -> Option<bool> {
        match self.ordering {
            ListSortOrdering::None => None,
            ListSortOrdering::Ascending => Some(false),
            ListSortOrdering::Descending => Some(true),
        }
    }
}

pub(super) fn list_sort_controls(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    options: &[&'static str],
    default_option_index: usize,
) -> ListSortState {
    let id = ui.make_persistent_id(("uocf_list_sort", id_salt));
    let mut state = ui
        .data_mut(|data| data.get_temp::<ListSortState>(id))
        .unwrap_or(ListSortState {
            option_index: default_option_index.min(options.len().saturating_sub(1)),
            ordering: ListSortOrdering::None,
        });
    if state.option_index >= options.len() {
        state.option_index = default_option_index.min(options.len().saturating_sub(1));
    }

    ui.horizontal(|ui| {
        ui.label("Sort:");
        egui::ComboBox::from_id_salt((id, "option"))
            .selected_text(options.get(state.option_index).copied().unwrap_or("Source"))
            .show_ui(ui, |ui| {
                for (index, option) in options.iter().enumerate() {
                    ui.selectable_value(&mut state.option_index, index, *option);
                }
            });
        egui::ComboBox::from_id_salt((id, "ordering"))
            .selected_text(match state.ordering {
                ListSortOrdering::None => "No ordering",
                ListSortOrdering::Ascending => "Ascending",
                ListSortOrdering::Descending => "Descending",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.ordering, ListSortOrdering::None, "No ordering");
                ui.selectable_value(&mut state.ordering, ListSortOrdering::Ascending, "Ascending");
                ui.selectable_value(&mut state.ordering, ListSortOrdering::Descending, "Descending");
            });
    });

    ui.data_mut(|data| data.insert_temp(id, state));
    state
}

pub(super) fn sorted_indices_by<T>(
    items: &[T],
    ordering: Option<bool>,
    compare: impl Fn(&T, &T) -> Ordering,
) -> Vec<usize> {
    let Some(descending) = ordering else {
        return (0..items.len()).collect();
    };

    let mut indices: Vec<usize> = (0..items.len()).collect();
    indices.sort_by(|left, right| {
        let ordering = compare(&items[*left], &items[*right]);
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| left.cmp(right))
    });
    indices
}

const GO_TO_HINT: &str = "id or 0x id";
const GO_TO_FIELD_WIDTH: f32 = 90.0;
const HEX_ID_PREFIXES: [&str; 2] = ["0x", "0X"];
const HEX_RADIX: u32 = 16;

/// What a "Go to" field reports in a frame.
pub(crate) struct GoTo {
    /// The id typed, in the frame Enter is pressed or "Go" is clicked.
    pub target: Option<u64>,
    /// Whether the field has the keyboard: the arrow keys are then its own, not the list's.
    pub has_focus: bool,
}

impl GoTo {
    /// The position in `ids`, the list's ids in the order shown, of the entry to jump to: the typed id, or the nearest.
    pub(crate) fn position_in(&self, ids: impl IntoIterator<Item = u64>) -> Option<usize> {
        self.target.and_then(|target| nearest_id_position(ids, target))
    }
}

/// A "Go to" field for a list of entries by id, typed in decimal or as 0x hex. Its text lives in egui's memory under
/// `id_salt`, so a list adopts it without a field of its own. The list selects `GoTo::position_in` and scrolls to it:
/// `centered_row_offset` for a list drawn with `show_rows`, else `Response::scroll_to_me`.
pub(crate) fn go_to_field(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug) -> GoTo {
    let id = ui.make_persistent_id(("uocf_go_to", id_salt));
    let mut text = ui.data_mut(|data| data.get_temp::<String>(id)).unwrap_or_default();
    let mut go_to = GoTo { target: None, has_focus: false };
    ui.horizontal(|ui| {
        ui.label("Go to:");
        let response = ui.add(egui::TextEdit::singleline(&mut text).hint_text(GO_TO_HINT).desired_width(GO_TO_FIELD_WIDTH));
        go_to.has_focus = response.has_focus();
        let entered = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let parsed = parse_entry_id(&text);
        let clicked = ui.add_enabled(parsed.is_some(), egui::Button::new("Go")).clicked();
        if entered || clicked {
            go_to.target = parsed;
        }
        if parsed.is_none() && !text.trim().is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, "not an id");
        }
    });
    ui.data_mut(|data| data.insert_temp(id, text));
    go_to
}

/// An entry id typed in decimal or as 0x hex.
pub(crate) fn parse_entry_id(text: &str) -> Option<u64> {
    let text = text.trim();
    match HEX_ID_PREFIXES.iter().find_map(|prefix| text.strip_prefix(prefix)) {
        Some(hex) => u64::from_str_radix(hex, HEX_RADIX).ok(),
        None => text.parse().ok(),
    }
}

/// The position in `ids` (any order) of `target`, else of the id nearest to it, the lower one on a tie.
pub(crate) fn nearest_id_position(ids: impl IntoIterator<Item = u64>, target: u64) -> Option<usize> {
    ids.into_iter().enumerate().min_by_key(|&(_, id)| (id.abs_diff(target), id)).map(|(position, _)| position)
}

/// The scroll offset of a `show_rows` list of `row_height` rows that puts row `position` in the middle of the view.
/// `show_rows` lays out only the rows in view, so `scroll_to_me` can't reach the others.
pub(crate) fn centered_row_offset(ui: &egui::Ui, position: usize, row_height: f32) -> f32 {
    let stride = row_height + ui.spacing().item_spacing.y;
    (position as f32 * stride - (ui.available_height() - stride) / 2.0).max(0.0)
}

/// The zoom factor of no zoom.
pub(crate) const NO_ZOOM: f32 = 1.0;

/// This frame's zoom factor for a view drawn in `rect`: Shift+wheel, Ctrl+wheel or a pinch while the pointer is over
/// it, else `NO_ZOOM`. The plain wheel scrolls instead (see `gui_shared::install_zoom_and_scroll_input_options`).
pub(crate) fn hovered_zoom_factor(ui: &egui::Ui, rect: egui::Rect) -> f32 {
    if ui.rect_contains_pointer(rect) { ui.input(|input| input.zoom_delta()) } else { NO_ZOOM }
}

/// Where a tab's data comes from, as every source selector names it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ClientSource {
    /// The Classic client's files outside UOP packages (`.mul`, and loose files such as `Cliloc.*`).
    Mul,
    /// The Classic client's UOP packages.
    CcUop,
    /// Kingdom Reborn data in UOP packages: a Kingdom Reborn client's, or the KR art the Enhanced Client carries.
    KrUop,
    /// The Enhanced Client's UOP packages.
    EcUop,
}

impl ClientSource {
    /// Every source, in the order the selectors show them.
    pub(crate) const ALL: [Self; 4] = [Self::Mul, Self::CcUop, Self::KrUop, Self::EcUop];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Mul => "MUL",
            Self::CcUop => "CC UOP",
            Self::KrUop => "KR UOP",
            Self::EcUop => "EC UOP",
        }
    }
}

/// Whether a tab can show a source, with the tooltip of its button.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum SourceAvailability {
    /// Loaded: the tooltip names what is read and from where.
    Found(String),
    /// Not loaded: the button is greyed, and the tooltip says what was looked for and where.
    Missing(String),
    /// Never applies to the tab: no button.
    NotApplicable,
}

/// The client folder the settings name for the Classic files.
pub(crate) const CLASSIC_FOLDER: &str = "Classic client";
/// The client folder the settings name for the Enhanced Client files.
pub(crate) const ENHANCED_FOLDER: &str = "Enhanced Client";
/// The client folder the settings name for the Kingdom Reborn files.
pub(crate) const KINGDOM_REBORN_FOLDER: &str = "Kingdom Reborn client";
/// The two sources read from an Enhanced-format folder, in selector order.
pub(crate) const ENHANCED_SOURCES: [ClientSource; 2] = [ClientSource::KrUop, ClientSource::EcUop];

/// `files` of a client folder: found when `loaded`, else missing from `folder`, or no folder set.
pub(crate) fn folder_files_availability(folder: Option<&Path>, folder_kind: &str, files: &str, loaded: bool) -> SourceAvailability {
    match folder {
        None => SourceAvailability::Missing(format!("{files}: no {folder_kind} folder is set.")),
        Some(folder) if loaded => SourceAvailability::Found(format!("{files} from {}", folder.display())),
        Some(folder) => SourceAvailability::Missing(format!("No {files} loaded from {} (see the log).", folder.display())),
    }
}

/// The folders the two Enhanced-format sources read, from the settings: KR UOP the Kingdom Reborn client folder, EC
/// UOP the Enhanced Client folder. An Enhanced Client folder that holds a Kingdom Reborn client (`UOKR.exe`) is read
/// as KR UOP while no Kingdom Reborn folder is set, as before that setting existed, and not at all once one is.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct EnhancedFolders {
    /// The folder of KR UOP.
    pub kr: Option<PathBuf>,
    /// The folder of EC UOP.
    pub ec: Option<PathBuf>,
    /// The Enhanced Client folder of the settings, when it holds a Kingdom Reborn client.
    pub enhanced_kr_client: Option<PathBuf>,
    /// The Kingdom Reborn folder of the settings holds no `UOKR.exe`.
    pub kr_without_exe: bool,
}

impl EnhancedFolders {
    /// The folders of the settings' Kingdom Reborn folder `kr` and Enhanced Client folder `ec`. Lists them, to tell
    /// their clients apart.
    pub(crate) fn from_settings(kr: Option<&Path>, ec: Option<&Path>) -> Self {
        let holds_kr = |folder: &Path| ClientDialect::of_client_dir(folder).is_kingdom_reborn();
        let enhanced_kr_client = ec.filter(|folder| holds_kr(folder)).map(Path::to_path_buf);
        Self {
            kr: kr.map(Path::to_path_buf).or_else(|| enhanced_kr_client.clone()),
            ec: ec.filter(|_| enhanced_kr_client.is_none()).map(Path::to_path_buf),
            kr_without_exe: kr.is_some_and(|folder| !holds_kr(folder)),
            enhanced_kr_client,
        }
    }

    /// The folder of `source`, KR UOP or EC UOP; `None` for the Classic sources.
    pub(crate) fn folder(&self, source: ClientSource) -> Option<&Path> {
        match source {
            ClientSource::KrUop => self.kr.as_deref(),
            ClientSource::EcUop => self.ec.as_deref(),
            ClientSource::Mul | ClientSource::CcUop => None,
        }
    }

    /// The Enhanced-format source a reload loads: `preferred` when its folder is set, else EC UOP, else KR UOP.
    pub(crate) fn shown_source(&self, preferred: Option<ClientSource>) -> Option<ClientSource> {
        preferred.into_iter().chain([ClientSource::EcUop, ClientSource::KrUop]).find(|&source| self.folder(source).is_some())
    }

    /// KR UOP reads the Enhanced Client folder, which holds a Kingdom Reborn client.
    fn kr_is_enhanced_folder(&self) -> bool {
        self.kr.is_some() && self.kr == self.enhanced_kr_client
    }

    /// The folder of `source` for a tooltip: its path and which setting it is.
    pub(crate) fn folder_text(&self, source: ClientSource, folder: &Path) -> String {
        match source {
            ClientSource::KrUop if self.kr_is_enhanced_folder() => format!(
                "{} (the {ENHANCED_FOLDER} folder, which holds a Kingdom Reborn client: {KINGDOM_REBORN_EXE})",
                folder.display(),
            ),
            ClientSource::KrUop => format!("{} (the {KINGDOM_REBORN_FOLDER} folder)", folder.display()),
            _ => format!("{} (the {ENHANCED_FOLDER} folder)", folder.display()),
        }
    }

    /// Why `source` has no folder.
    pub(crate) fn no_folder_text(&self, source: ClientSource, files: &str) -> String {
        match (source, &self.enhanced_kr_client) {
            (ClientSource::EcUop, Some(folder)) => {
                format!("No Enhanced Client: {} holds a Kingdom Reborn client ({KINGDOM_REBORN_EXE}).", folder.display())
            }
            (ClientSource::KrUop, _) => format!("{files}: no {KINGDOM_REBORN_FOLDER} folder is set."),
            _ => format!("{files}: no {ENHANCED_FOLDER} folder is set."),
        }
    }

    /// The status line of the settings' Kingdom Reborn folder, and whether it is a warning.
    pub(crate) fn kr_status(&self) -> Option<(String, bool)> {
        match &self.kr {
            Some(_) if self.kr_is_enhanced_folder() => None,
            Some(_) if self.kr_without_exe => Some((
                format!("Read as KR UOP, but it holds no {KINGDOM_REBORN_EXE}: is it a Kingdom Reborn client?"),
                true,
            )),
            Some(_) => Some(("Read as KR UOP.".to_string(), false)),
            None => None,
        }
    }

    /// The status line of the settings' Enhanced Client folder, and whether it is a warning.
    pub(crate) fn ec_status(&self) -> Option<(String, bool)> {
        match (&self.ec, &self.enhanced_kr_client) {
            (Some(_), _) => Some(("Read as EC UOP.".to_string(), false)),
            (None, Some(_)) if self.kr_is_enhanced_folder() => {
                Some((format!("It holds a Kingdom Reborn client ({KINGDOM_REBORN_EXE}): read as KR UOP."), false))
            }
            (None, Some(_)) => Some((
                format!(
                    "It holds a Kingdom Reborn client ({KINGDOM_REBORN_EXE}), and the {KINGDOM_REBORN_FOLDER} folder is \
                     set: not read."
                ),
                true,
            )),
            (None, None) => None,
        }
    }
}

/// `files` of an Enhanced-format folder, shown as `source` (KR UOP or EC UOP). The tabs show one such client at a time
/// ([`UocfEditorApp::show_enhanced_client`]): its files are found when `loaded`. The other client's are found when its
/// folder holds one of `file_names`; choosing them reloads every tab from that folder, so unsaved changes hold them
/// back.
pub(crate) fn enhanced_files_availability(
    app: &UocfEditorApp,
    source: ClientSource,
    files: &str,
    file_names: &[&str],
    loaded: bool,
) -> SourceAvailability {
    let folders = &app.enhanced_folders;
    let Some(folder) = folders.folder(source) else {
        return SourceAvailability::Missing(folders.no_folder_text(source, files));
    };
    let from = folders.folder_text(source, folder);
    if app.shown_enhanced == Some(source) {
        return if loaded {
            SourceAvailability::Found(format!("{files} from {from}"))
        } else {
            SourceAvailability::Missing(format!("No {files} loaded from {from} (see the log)."))
        };
    }
    if !file_names.iter().any(|name| find_file_ignoring_case(folder, name).is_some()) {
        return SourceAvailability::Missing(format!("No {files} in {from}."));
    }
    match unsaved_changes::first_dirty_editor(app) {
        Some(label) => SourceAvailability::Missing(format!(
            "Save or discard the unsaved changes to {label} first: showing {} reloads every tab.",
            source.label(),
        )),
        None => SourceAvailability::Found(format!("{files} from {from}. Choosing it reloads every tab from that folder.")),
    }
}

/// One row of toggle buttons, one per source in [`ClientSource::ALL`] order, `selected` (if any) pressed. A missing
/// source stays greyed, with its tooltip; a source that does not apply is left out. Returns the source clicked this
/// frame when it differs from `selected`.
pub(crate) fn source_choice(
    ui: &mut egui::Ui,
    selected: Option<ClientSource>,
    availability: impl Fn(ClientSource) -> SourceAvailability,
) -> Option<ClientSource> {
    source_buttons(ui, selected, availability).0
}

/// [`source_choice`] over a tab's own sources, each paired in `sources` with the [`ClientSource`] its button stands
/// for; a client source no pair names is left out. Returns the tab source clicked this frame.
pub(crate) fn tab_source_choice<S: Copy + PartialEq>(
    ui: &mut egui::Ui,
    sources: &[(S, ClientSource)],
    selected: S,
    availability: impl Fn(S) -> SourceAvailability,
) -> Option<S> {
    let tab_source = |client: ClientSource| sources.iter().find(|&&(_, candidate)| candidate == client).map(|&(source, _)| source);
    let selected = sources.iter().find(|&&(source, _)| source == selected).map(|&(_, client)| client);
    source_choice(ui, selected, |client| tab_source(client).map_or(SourceAvailability::NotApplicable, &availability)).and_then(tab_source)
}

/// The buttons of [`source_choice`], with the response of each.
fn source_buttons(
    ui: &mut egui::Ui,
    selected: Option<ClientSource>,
    availability: impl Fn(ClientSource) -> SourceAvailability,
) -> (Option<ClientSource>, Vec<(ClientSource, egui::Response)>) {
    ui.horizontal(|ui| {
        let mut clicked = None;
        let mut buttons = Vec::new();
        for source in ClientSource::ALL {
            let (enabled, tooltip) = match availability(source) {
                SourceAvailability::Found(tooltip) => (true, tooltip),
                SourceAvailability::Missing(tooltip) => (false, tooltip),
                SourceAvailability::NotApplicable => continue,
            };
            let pressed = selected == Some(source);
            let response = ui.add_enabled(enabled, egui::Button::selectable(pressed, source.label()));
            let response = if enabled { response.on_hover_text(tooltip) } else { response.on_disabled_hover_text(tooltip) };
            if response.clicked() && !pressed {
                clicked = Some(source);
            }
            buttons.push((source, response));
        }
        (clicked, buttons)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_indices_by_preserves_source_order_for_equal_keys() {
        let values = ["b", "a", "b"];
        let indices = sorted_indices_by(&values, Some(false), |left, right| left.cmp(right));
        assert_eq!(indices, vec![1, 0, 2]);
    }

    #[test]
    fn sorted_indices_by_reverses_requested_order() {
        let values = [2, 1, 3];
        let indices = sorted_indices_by(&values, Some(true), |left, right| left.cmp(right));
        assert_eq!(indices, vec![2, 0, 1]);
    }

    #[test]
    fn a_new_window_waits_for_a_minimized_main_window() {
        let id = egui::ViewportId::from_hash_of("window");
        let can_show = |minimized: Option<bool>, window_exists: bool| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().minimized = minimized;
            if window_exists {
                input.viewports.insert(id, egui::ViewportInfo::default());
            }
            let mut result = None;
            let _ = egui::Context::default().run_ui(input, |ui| result = Some(can_show_immediate_viewport(ui.ctx(), id)));
            result.unwrap()
        };
        assert!(can_show(None, false), "a backend that does not tell");
        assert!(can_show(Some(false), false));
        assert!(!can_show(Some(true), false), "eframe would panic");
        assert!(can_show(Some(true), true), "a window that exists goes on being drawn");
    }

    /// Runs a frame of a source selector with `events`; returns the click and each button with its place and whether
    /// it is enabled.
    fn source_selector_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        selected: Option<ClientSource>,
        availability: &[SourceAvailability; 4],
    ) -> (Option<ClientSource>, Vec<(ClientSource, egui::Rect, bool)>) {
        let mut result = (None, Vec::new());
        let _ = ctx.run_ui(selector_frame_input(events), |ui| {
            let (clicked, buttons) = source_buttons(ui, selected, |source| {
                let index = ClientSource::ALL.iter().position(|&candidate| candidate == source).unwrap();
                availability[index].clone()
            });
            result = (clicked, buttons.into_iter().map(|(source, response)| (source, response.rect, response.enabled())).collect());
        });
        result
    }

    fn selector_frame_input(events: Vec<egui::Event>) -> egui::RawInput {
        const SCREEN_SIZE: egui::Vec2 = egui::vec2(600.0, 100.0);
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN_SIZE)), events, ..Default::default() }
    }

    fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    #[test]
    fn the_source_selector_keeps_its_order_and_labels_and_greys_missing_sources() {
        assert_eq!(ClientSource::ALL.map(ClientSource::label), ["MUL", "CC UOP", "KR UOP", "EC UOP"]);
        let availability = [
            SourceAvailability::Found("art.mul".to_string()),
            SourceAvailability::Missing("no artLegacyMUL.uop".to_string()),
            SourceAvailability::NotApplicable,
            SourceAvailability::Found("LegacyTexture.uop".to_string()),
        ];
        let ctx = egui::Context::default();
        let selected = Some(ClientSource::Mul);
        let (clicked, buttons) = source_selector_frame(&ctx, Vec::new(), selected, &availability);
        assert_eq!(clicked, None);
        let shown: Vec<_> = buttons.iter().map(|&(source, _, enabled)| (source, enabled)).collect();
        assert_eq!(shown, [(ClientSource::Mul, true), (ClientSource::CcUop, false), (ClientSource::EcUop, true)], "KR UOP does not apply");
        assert!(buttons.windows(2).all(|pair| pair[0].1.max.x <= pair[1].1.min.x), "one row, left to right");

        let place = |wanted: ClientSource| buttons.iter().find(|(source, ..)| *source == wanted).unwrap().1.center();
        let click = |source| source_selector_frame(&ctx, click_at(place(source)), selected, &availability).0;
        assert_eq!(click(ClientSource::CcUop), None, "a greyed source is not selectable");
        assert_eq!(click(ClientSource::Mul), None, "the selected source is no change");
        assert_eq!(click(ClientSource::EcUop), Some(ClientSource::EcUop));

        let mut chosen = None;
        let _ = ctx.run_ui(selector_frame_input(click_at(place(ClientSource::EcUop))), |ui| {
            let index = |source| ClientSource::ALL.iter().position(|&candidate| candidate == source).unwrap();
            chosen = source_choice(ui, selected, |source| availability[index(source)].clone());
        });
        assert_eq!(chosen, Some(ClientSource::EcUop), "the public selector returns the click");
    }

    /// A folder under `TMPDIR` holding the empty `files`, removed when dropped.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(name: &str, files: &[&str]) -> Self {
            let path = test_support::output_path(name);
            std::fs::create_dir_all(&path).unwrap();
            for file in files {
                std::fs::write(path.join(file), []).unwrap();
            }
            Self(path)
        }

        fn path(&self) -> Option<&Path> {
            Some(&self.0)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn each_enhanced_source_has_its_own_folder() {
        let kr = TempFolder::new("kr", &[KINGDOM_REBORN_EXE]);
        let ec = TempFolder::new("ec", &[]);
        let ec_holding_kr = TempFolder::new("ec-holding-kr", &["uokr.EXE"]);
        let kr_without_exe = TempFolder::new("kr-without-exe", &[]);
        let (kr_uop, ec_uop) = (ClientSource::KrUop, ClientSource::EcUop);

        let both = EnhancedFolders::from_settings(kr.path(), ec.path());
        assert_eq!((both.folder(kr_uop), both.folder(ec_uop)), (kr.path(), ec.path()));
        assert_eq!(both.folder(ClientSource::Mul), None);
        assert_eq!(both.shown_source(None), Some(ec_uop), "the Enhanced Client first");
        assert_eq!(both.shown_source(Some(kr_uop)), Some(kr_uop));
        assert_eq!(both.kr_status(), Some(("Read as KR UOP.".to_string(), false)));
        assert_eq!(both.ec_status(), Some(("Read as EC UOP.".to_string(), false)));

        let enhanced_only = EnhancedFolders::from_settings(None, ec.path());
        assert_eq!(enhanced_only.shown_source(Some(kr_uop)), Some(ec_uop), "no Kingdom Reborn folder to show");
        assert_eq!(enhanced_only.kr_status(), None);
        assert!(enhanced_only.no_folder_text(kr_uop, "hues.uop").contains("no Kingdom Reborn client folder is set"));

        // A Kingdom Reborn client in the Enhanced folder is still KR UOP, and the tooltips say so.
        let enhanced_kr = EnhancedFolders::from_settings(None, ec_holding_kr.path());
        assert_eq!((enhanced_kr.folder(kr_uop), enhanced_kr.folder(ec_uop)), (ec_holding_kr.path(), None));
        assert_eq!(enhanced_kr.shown_source(None), Some(kr_uop));
        assert_eq!(enhanced_kr.kr_status(), None);
        let (status, warning) = enhanced_kr.ec_status().unwrap();
        assert!(status.contains("read as KR UOP") && !warning, "{status}");
        assert!(enhanced_kr.folder_text(kr_uop, &ec_holding_kr.0).contains("Enhanced Client folder, which holds a Kingdom Reborn client"));
        assert!(enhanced_kr.no_folder_text(ec_uop, "hues.uop").starts_with("No Enhanced Client:"));

        // With a Kingdom Reborn folder set too, the Enhanced folder's Kingdom Reborn client is not read.
        let both_kr = EnhancedFolders::from_settings(kr.path(), ec_holding_kr.path());
        assert_eq!((both_kr.folder(kr_uop), both_kr.folder(ec_uop)), (kr.path(), None));
        let (status, warning) = both_kr.ec_status().unwrap();
        assert!(status.contains("not read") && warning, "{status}");

        let (status, warning) = EnhancedFolders::from_settings(kr_without_exe.path(), None).kr_status().unwrap();
        assert!(status.contains("holds no UOKR.exe") && warning, "{status}");
        assert_eq!(EnhancedFolders::from_settings(None, None).shown_source(Some(ec_uop)), None);
    }

    #[test]
    fn enhanced_sources_name_their_folder_and_wait_for_unsaved_changes() {
        const FILES: &str = "hues.uop";
        let kr = TempFolder::new("kr-hues", &[KINGDOM_REBORN_EXE, "Hues.uop"]);
        let ec = TempFolder::new("ec-hues", &[]);
        let mut app = crate::app::tests::test_app();
        app.enhanced_folders = EnhancedFolders::from_settings(kr.path(), ec.path());
        app.shown_enhanced = Some(ClientSource::EcUop);
        let availability = |app: &UocfEditorApp, source, loaded| enhanced_files_availability(app, source, FILES, &[FILES], loaded);
        let text = |availability: SourceAvailability| match availability {
            SourceAvailability::Found(text) => (true, text),
            SourceAvailability::Missing(text) => (false, text),
            SourceAvailability::NotApplicable => panic!("an Enhanced source applies"),
        };

        let (found, shown) = text(availability(&app, ClientSource::EcUop, true));
        assert!(found && shown.contains(&ec.0.display().to_string()) && shown.contains("Enhanced Client folder"), "{shown}");
        let (found, failed) = text(availability(&app, ClientSource::EcUop, false));
        assert!(!found && failed.starts_with("No hues.uop loaded from"), "{failed}");

        let (found, other) = text(availability(&app, ClientSource::KrUop, false));
        assert!(found && other.contains("Kingdom Reborn client folder") && other.contains("reloads every tab"), "{other}");
        app.shown_enhanced = Some(ClientSource::KrUop);
        let (found, absent) = text(availability(&app, ClientSource::EcUop, true));
        assert!(!found && absent.starts_with("No hues.uop in"), "{absent}");
        app.shown_enhanced = Some(ClientSource::EcUop);

        let edit = app.entry_text_edits.open(Path::new("/packages/GameData.uop"), 1, b"text\n").unwrap();
        edit.buffer.push('!');
        let (found, held) = text(availability(&app, ClientSource::KrUop, false));
        assert!(!found && held.starts_with("Save or discard the unsaved changes"), "{held}");
        let (found, _) = text(availability(&app, ClientSource::EcUop, true));
        assert!(found, "the client shown is not reloaded");
    }

    #[test]
    fn sorted_indices_by_can_leave_source_order_unchanged() {
        let values = [2, 1, 3];
        let indices = sorted_indices_by(&values, None, |left, right| left.cmp(right));
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn go_to_ids_are_decimal_or_0x_hex() {
        assert_eq!(parse_entry_id("9504"), Some(9504));
        assert_eq!(parse_entry_id(" 0x2520 "), Some(9504));
        assert_eq!(parse_entry_id("0X2520"), Some(9504));
        assert_eq!(parse_entry_id("2520h"), None);
        assert_eq!(parse_entry_id("0x"), None);
        assert_eq!(parse_entry_id(""), None);
        assert_eq!(parse_entry_id("-1"), None);
    }

    #[test]
    fn go_to_picks_the_id_or_the_nearest_in_shown_order() {
        let shown = [30_u64, 10, 20, 40];
        assert_eq!(nearest_id_position(shown, 20), Some(2));
        assert_eq!(nearest_id_position(shown, 22), Some(2));
        assert_eq!(nearest_id_position(shown, 25), Some(2), "a tie goes to the lower id");
        assert_eq!(nearest_id_position(shown, 0), Some(1));
        assert_eq!(nearest_id_position(shown, u64::MAX), Some(3));
        assert_eq!(nearest_id_position([], 5), None);
        let go_to = GoTo { target: Some(39), has_focus: false };
        assert_eq!(go_to.position_in(shown), Some(3));
        assert_eq!(GoTo { target: None, has_focus: false }.position_in(shown), None);
    }
}

pub fn draw_ui(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    app.report_gpu_errors();
    unsaved_changes::hold_app_close(app, ctx);
    unsaved_changes::hold_tab_switch(app);
    egui::Panel::top("top_panel").show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open UOP...").clicked() {
                    app.open_uop();
                    ui.close_kind(egui::UiKind::Menu);
                }
                if ui.button("Search Paths...").clicked() {
                    app.show_search_paths = true;
                    ui.close_kind(egui::UiKind::Menu);
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            
            ui.separator();
            if ui.button("Upscale Preview").clicked() {
                app.show_upscale_preview = true;
            }

            ui.separator();
            
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Home, "Home");
                if uop_browser::has_visible_uop_explorer_package(app) {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::UopExplorer, "UOP Explorer");
                }
                if app.client_data.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TexArtCc, "Art");
                }
                if app.client_data.as_ref().and_then(|client| client.multis.as_ref()).is_some()
                    || app.multi_collection.is_some()
                {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multis, "Multis");
                }
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Housing, "Housing");
                let animated_items = matches!(app.view_mode, crate::app::ViewMode::AnimationDefinitions | crate::app::ViewMode::AnimData);
                if ui.selectable_label(animated_items, "Animated items").clicked() && !animated_items {
                    app.view_mode = crate::app::ViewMode::AnimationDefinitions;
                }
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Effects, "Effects");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multimap, "Multimap");
                if app.client_data.is_some() || app.ec_hues.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Hues, "Hues");
                }
                if app.cliloc.is_some() || app.localized_strings.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Clilocs, "CliLocs");
                }
                if app.cc_tiledata.is_some() || app.ec_tileart_entries.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TileMetadata, "Tile Metadata");
                }
                if app.cc_gumps.is_some()
                    || app.uop_cache.loaded_uops.iter().any(|loaded| {
                        loaded
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.eq_ignore_ascii_case("interface.uop"))
                            .unwrap_or(false)
                    })
                {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Gumps, "Gumps");
                }
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Animations, "Animations");
                if app.terrain_def_package.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TerrainDefinition, "Terrain Def");
                }
                if app.uo_string_dictionary.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::StringDictionary, "String Dict");
                }
                if app.cc_sounds.is_some() || app.ec_audio.is_some() || app.settings.cc_path.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Sounds, "Audio");
                }
                if app.settings.cc_path.is_some() {
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::ClassicMaps, "Maps");
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::ClassicFonts, "Fonts");
                    ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::ClassicDefs, "Def files");
                }
            });
        });
    });

    unsaved_changes::hold_tab_switch(app);
    egui::Panel::bottom("status_bar").show(ui, |ui| {
        ui.horizontal(|ui| {
            unsaved_changes::save_button(app, ui);
            ui.separator();
            ui.label(&app.status_message);
        });
    });
    unsaved_changes::leave_tab_window(app, ctx);
    unsaved_changes::close_app_window(app, ctx);

    let mut show_search_paths = app.show_search_paths;
    if show_search_paths {
        egui::Window::new("Search Paths")
            .open(&mut show_search_paths)
            .show(ctx, |ui| {
                egui::Grid::new("paths_grid").show(ui, |ui| {
                    ui.label("CC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.cc_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::pick_folder() {
                                app.settings.cc_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("KR Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.kr_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::pick_folder() {
                                app.settings.kr_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();
                    folder_status_row(ui, app.enhanced_folders.kr_status());

                    ui.label("EC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.ec_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::pick_folder() {
                                app.settings.ec_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();
                    folder_status_row(ui, app.enhanced_folders.ec_status());

                    ui.label("Dictionary (.dic):");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.dict_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = crate::dialog::file_dialog()
                                .add_filter("DIC Dictionary", &["dic"])
                                .pick_file()
                            {
                                app.settings.dict_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();
                });
            });
        app.show_search_paths = show_search_paths;
    }
    image_export::show_bulk_export(app, ctx);

    if app.view_mode == crate::app::ViewMode::UopExplorer
        && app.selected_uop_idx.is_none()
        && !uop_browser::has_visible_uop_explorer_package(app)
    {
        unsaved_changes::leave_a_tab_that_is_gone(app, crate::app::ViewMode::Home);
    }

    match app.view_mode {
        crate::app::ViewMode::Home => {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.heading("Logs");
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in &app.logs {
                            ui.label(log);
                        }
                    });
            });
        }
        crate::app::ViewMode::UopExplorer => {
            uop_browser::ui_uop_browser(app, ctx, ui);
        }
        crate::app::ViewMode::TexArtCc | crate::app::ViewMode::CcTileData => {
            art_viewer::ui_art_viewer(app, ctx, ui);
        }
        crate::app::ViewMode::TileMetadata => {
            tile_metadata::ui_tile_metadata(app, ctx, ui);
        }
        crate::app::ViewMode::Animations => {
            animations::ui_animations(app, ctx, ui);
        }
        crate::app::ViewMode::Gumps => {
            gumps::ui_gumps(app, ctx, ui);
        }
        crate::app::ViewMode::AnimData => {
            animdata::ui_animdata(app, ctx, ui);
        }
        crate::app::ViewMode::Multis => {
            multis::ui_multis(app, ctx, ui);
        }
        crate::app::ViewMode::Housing => {
            housing::ui_housing(app, ctx, ui);
        }
        crate::app::ViewMode::AnimationDefinitions => {
            animation_definitions::ui_animation_definitions(app, ctx, ui);
        }
        crate::app::ViewMode::Effects => {
            effects::ui_effects(app, ctx, ui);
        }
        crate::app::ViewMode::Multimap => {
            multimap::ui_multimap(app, ctx, ui);
        }
        crate::app::ViewMode::Hues => {
            hues::ui_hues(app, ctx, ui);
        }
        crate::app::ViewMode::Clilocs => {
            clilocs::ui_clilocs(app, ctx, ui);
        }
        crate::app::ViewMode::TerrainDefinition => {
            terrain_definition::ui_terrain_definition(app, ctx, ui);
        }
        crate::app::ViewMode::StringDictionary => {
            string_dictionary::ui_string_dictionary(app, ctx, ui);
        }
        crate::app::ViewMode::Sounds => {
            sounds::ui_sounds(app, ctx, ui);
        }
        crate::app::ViewMode::ClassicMaps => {
            classic_maps::ui_classic_maps(app, ctx, ui);
        }
        crate::app::ViewMode::ClassicFonts => {
            classic_fonts::ui_classic_fonts(app, ctx, ui);
        }
        crate::app::ViewMode::ClassicDefs => {
            classic_defs::ui_classic_defs(app, ctx, ui);
        }
    }

    upscale_preview::ui_upscale_preview_window(app, ctx, ui);
    classic_maps::ui_classic_maps_window(app, ctx);
}
