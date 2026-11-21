//! The `AnimationSequence.uop` entry of the shown body, below the frames of the Animations tab: what each action
//! plays (frames, fps, cycle, alias, skipped frames, effects, offsets) beside the frames its frame file stores, the
//! action groups, the timing of a rider on a mount, a whole-client scan, and checked edits of the entry saved into a
//! new `AnimationSequence.uop`.
//!
//! The entry is read from the `AnimationSequence.uop` of the selected source's folder (the Classic folder for MUL
//! and CC UOP, the Enhanced or KR folder shown for the others), in that folder's dialect.

use crate::app::{ArtSource, UocfEditorApp};
use crate::logic::uop_cache::LoadedUop;
use crate::ui::animation_frames::PlaybackTiming;
use crate::ui::animation_list::ActionKey;
use crate::ui::animations::{
    animationframe_uop_entries, body_definition, is_ec_source, selected_body_definition, selected_frames_body_id,
    selected_sequence_id, ANIMATION_SEQUENCE_UOP_NAME, FLYING_ANIMATIONS_ENABLED, UOP_FILE_FILTER,
};
use color_eyre::eyre;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;
use uocf::animation_sequence::scan::{ClientSequences, ScanReport, MOUNT_REFERENCE_BODIES};
use uocf::animation_sequence::summary::{group_name, movement_name, ride_timings, ActionSummary, AliasState, StoredFrames};
use uocf::animation_sequence::{
    AnimationSequence, GROUP_MOUNTED, STORED_DIRECTION_COUNT, UNSET_BYTE_OFFSET, UNSET_SHORT_OFFSET,
};
use uocf::enhanced::animationframe::package::{read_sequence, write_sequence};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::mobile_body::FALLBACK_FRAMES_BODY;

/// Height of the panel at first.
pub(super) const SEQUENCE_PANEL_HEIGHT: f32 = 300.0;
const EDIT_PANEL_WIDTH: f32 = 320.0;
const ROW_HEIGHT: f32 = 18.0;
/// The mount of the rider / mount check at first: the brown horse.
const DEFAULT_MOUNT_BODY: u32 = 200;
/// Bounds of the edit fields.
const MAX_EDIT_FPS: f32 = 120.0;
const MAX_EDIT_FRAME_COUNT: u32 = 255;
const FPS_DRAG_SPEED: f64 = 0.1;
/// How often a running background job is polled.
const JOB_POLL_INTERVAL: Duration = Duration::from_millis(200);
const SCAN_REPORT_NAME: &str = "animation_sequence_scan.md";
const MARKDOWN_FILE_FILTER: (&str, &[&str]) = ("Markdown report", &["md"]);
const TSV_EXTENSION: &str = "tsv";
const MISMATCH_COLOUR: egui::Color32 = egui::Color32::YELLOW;
const BROKEN_ALIAS_COLOUR: egui::Color32 = egui::Color32::ORANGE;
const MISSING_COLOUR: egui::Color32 = egui::Color32::LIGHT_RED;
const FRAME_COUNT_WARNING: &str = "The frame file's numbering depends on the frame count: direction d holds frames \
                                   (d - 3) * frame count + 1 onwards. Without frames re-imported to match, the client \
                                   shows the wrong frames.";

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum SequenceTab {
    #[default]
    Actions,
    Groups,
    Ride,
    Scan,
}

/// The frames stored per action of one frame body, read in the background.
struct StoredFramesJob {
    key: u64,
    frames_body: u32,
    receiver: Option<Receiver<BTreeMap<u16, Result<StoredFrames, String>>>>,
    frames: BTreeMap<u16, Result<StoredFrames, String>>,
}

/// A whole-client scan writing its report.
struct ScanJob {
    folder: PathBuf,
    report: PathBuf,
    receiver: Option<Receiver<Result<Vec<(String, usize)>, String>>>,
    result: Option<Result<Vec<(String, usize)>, String>>,
}

/// State of the sequence panel kept across frames.
#[derive(Default)]
pub(crate) struct SequencePanelState {
    tab: SequenceTab,
    /// (body type, loaded package count, package) of the last lookup, so a missing entry is looked up once.
    lookup: Option<(u32, usize, Option<PathBuf>)>,
    /// The package `app.selected_anim_sequence` was read from, with its dialect.
    package: Option<(PathBuf, ClientDialect)>,
    /// The shown entry as stored; `app.selected_anim_sequence` holds the edited copy.
    original: Option<AnimationSequence>,
    /// Edited entries by (package, sequence id), kept while other bodies are shown.
    edits: BTreeMap<(PathBuf, u32), AnimationSequence>,
    /// Bumped by the edits that change how the frames split (frame count, alias): part of the decode key.
    pub(super) frames_generation: u64,
    stored: Option<StoredFramesJob>,
    mount_body: Option<u32>,
    /// The skipped frames field, with the action it was filled for.
    skip_text: Option<(u32, String)>,
    scan: Option<ScanJob>,
}

/// The folder whose `AnimationSequence.uop` the source reads: the Classic folder for MUL and CC UOP, else the
/// Enhanced or KR folder shown.
fn source_folder(app: &UocfEditorApp) -> Option<PathBuf> {
    match app.selected_legacy_source {
        ArtSource::Mul | ArtSource::CcUop => app.settings.cc_path.clone(),
        ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => app.shown_enhanced_folder().map(Path::to_path_buf),
        ArtSource::Any => None,
    }
}

/// The loaded `AnimationSequence.uop` of the source's folder.
fn source_sequence_package(app: &UocfEditorApp) -> Option<Arc<LoadedUop>> {
    let folder = source_folder(app)?;
    app.uop_cache
        .loaded_uops
        .iter()
        .find(|loaded| {
            loaded.path.starts_with(&folder)
                && loaded.path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(ANIMATION_SEQUENCE_UOP_NAME))
        })
        .cloned()
}

/// Reads the sequence entry of the selected body from the source's package into `app.selected_anim_sequence`
/// (its edited copy when there is one), once per body, package set and source.
pub(super) fn load_selected_sequence(app: &mut UocfEditorApp) {
    let body_type = selected_sequence_id(app);
    let loaded = source_sequence_package(app);
    let lookup = (body_type, app.uop_cache.loaded_uops.len(), loaded.as_ref().map(|loaded| loaded.path.clone()));
    let state = &mut app.animations_view.sequence;
    if state.lookup.as_ref() == Some(&lookup) {
        return;
    }
    state.lookup = Some(lookup);
    state.package = None;
    state.original = None;
    state.skip_text = None;
    app.selected_anim_sequence = None;
    let Some(loaded) = loaded else { return };
    let dialect = ClientDialect::of_package_path(&loaded.path);
    state.package = Some((loaded.path.clone(), dialect));
    match read_sequence(&loaded.package, body_type, dialect) {
        Ok(Some(sequence)) => {
            let shown = state.edits.get(&(loaded.path.clone(), body_type)).cloned().unwrap_or_else(|| sequence.clone());
            state.original = Some(sequence);
            app.selected_anim_sequence = Some((body_type, shown));
        }
        Ok(None) => {}
        Err(error) => log::warn!("AnimationSequence {body_type}: {error:#}"),
    }
}

/// How the client plays `key` (a UOP source with the body's sequence loaded): the requested action's fps
/// (`GetActionFps`: the resolved action's) and skipped frames. The requested action is the selected one when it
/// resolves to `key`'s, else `key`'s.
pub(super) fn playback_timing(app: &UocfEditorApp, key: ActionKey) -> Option<PlaybackTiming> {
    if !matches!(app.selected_legacy_source, ArtSource::CcUop) && !is_ec_source(app.selected_legacy_source) {
        return None;
    }
    let (id, sequence) = app.selected_anim_sequence.as_ref()?;
    if *id != selected_sequence_id(app) {
        return None;
    }
    let selected = u32::from(app.selected_action_id);
    let shown = u32::from(key.action);
    let requested =
        if sequence.resolve_action(selected, FLYING_ANIMATIONS_ENABLED) == Some(shown) { selected } else { shown };
    Some(PlaybackTiming {
        fps: sequence.action_fps(requested, FLYING_ANIMATIONS_ENABLED),
        skipped: sequence.actions.get(&requested).map(|record| record.skipped_frames.clone()).unwrap_or_default(),
    })
}

/// Records an edit of the shown entry: kept in `edits` until it equals the stored entry again.
fn record_edit(app: &mut UocfEditorApp, frames_change: bool) {
    let state = &mut app.animations_view.sequence;
    let (Some((path, _)), Some((id, sequence))) = (&state.package, &app.selected_anim_sequence) else { return };
    let key = (path.clone(), *id);
    if state.original.as_ref() == Some(sequence) {
        state.edits.remove(&key);
    } else {
        state.edits.insert(key, sequence.clone());
    }
    if frames_change {
        state.frames_generation += 1;
    }
}

fn is_edited(app: &UocfEditorApp) -> bool {
    let state = &app.animations_view.sequence;
    matches!((&state.original, &app.selected_anim_sequence), (Some(original), Some((_, shown))) if original != shown)
}

/// The panel: tabs, the shown entry and its save controls, then the tab.
pub(super) fn show_sequence_panel(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.strong("Sequence");
        let tab = &mut app.animations_view.sequence.tab;
        ui.selectable_value(tab, SequenceTab::Actions, "Actions");
        ui.selectable_value(tab, SequenceTab::Groups, "Groups");
        ui.selectable_value(tab, SequenceTab::Ride, "Rider / mount");
        ui.selectable_value(tab, SequenceTab::Scan, "Client scan");
        ui.separator();
        let body_type = selected_sequence_id(app);
        match (&app.animations_view.sequence.package, &app.selected_anim_sequence) {
            (Some((path, dialect)), Some((id, _))) => {
                // The folder names the client; the dialect is only the layout (a Classic folder reads as Enhanced).
                ui.label(format!("Entry {id} of {} · {} layout", package_folder_name(path), dialect.label()))
                    .on_hover_text(path.display().to_string());
            }
            (Some((path, _)), None) => {
                ui.colored_label(MISSING_COLOUR, format!("No sequence entry {body_type}"))
                    .on_hover_text(path.display().to_string());
            }
            (None, _) => {
                ui.weak(format!("No {ANIMATION_SEQUENCE_UOP_NAME} loaded for this source"));
            }
        }
        show_save_controls(app, ui);
    });
    ui.separator();
    match app.animations_view.sequence.tab {
        SequenceTab::Actions => show_actions_tab(app, ui),
        SequenceTab::Groups => show_groups_tab(app, ui),
        SequenceTab::Ride => show_ride_tab(app, ui),
        SequenceTab::Scan => show_scan_tab(app, ui),
    }
    // The panel keeps its height whatever the tab shows, so the tab row stays under the pointer.
    ui.take_available_space();
}

fn show_save_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let edited = is_edited(app);
    let pending = app.animations_view.sequence.edits.len();
    if edited {
        ui.colored_label(MISMATCH_COLOUR, "edited");
    }
    if pending > usize::from(edited) {
        ui.weak(format!("({pending} edited entries)")).on_hover_text("Edited entries of other bodies are kept until reverted.");
    }
    if ui.add_enabled(edited, egui::Button::new("Revert")).clicked() {
        let state = &mut app.animations_view.sequence;
        if let (Some((path, _)), Some((id, shown)), Some(original)) =
            (&state.package, &mut app.selected_anim_sequence, &state.original)
        {
            state.edits.remove(&(path.clone(), *id));
            *shown = original.clone();
            state.frames_generation += 1;
            state.skip_text = None;
        }
    }
    let can_save = app.selected_anim_sequence.is_some() && app.animations_view.sequence.package.is_some();
    if ui
        .add_enabled(can_save, egui::Button::new("Save sequence..."))
        .on_hover_text(format!(
            "Writes a copy of the {ANIMATION_SEQUENCE_UOP_NAME} with this entry as shown, in the source's format; the \
             source package is refused as the target.\n{}",
            crate::ui::UOP_PATCHER_NOTE
        ))
        .clicked()
    {
        app.status_message = match save_sequence(app) {
            Ok(message) => message,
            Err(error) => format!("Sequence save failed: {error:#}"),
        };
    }
}

/// Writes the shown entry into a new `AnimationSequence.uop` picked by the user.
fn save_sequence(app: &UocfEditorApp) -> eyre::Result<String> {
    let (source, dialect) = app.animations_view.sequence.package.clone().ok_or_else(|| eyre::eyre!("no sequence package"))?;
    let (id, sequence) = app.selected_anim_sequence.as_ref().ok_or_else(|| eyre::eyre!("no sequence entry"))?;
    let Some(output) = crate::dialog::file_dialog()
        .set_title(format!("{ANIMATION_SEQUENCE_UOP_NAME} with entry {id}"))
        .add_filter(UOP_FILE_FILTER.0, UOP_FILE_FILTER.1)
        .set_file_name(ANIMATION_SEQUENCE_UOP_NAME)
        .save_file()
    else {
        return Ok("Sequence save cancelled".to_string());
    };
    let change = write_sequence(&source, &output, *id, sequence, dialect)?;
    Ok(format!("Wrote {} with sequence entry {id} ({change:?}, {})", output.display(), dialect.label()))
}

/// The frame body whose frames the selected body draws from the source's packages: the placeholder's frames body,
/// or `FALLBACK_FRAMES_BODY` when that has none (Enhanced sources).
fn stored_frames_body(app: &UocfEditorApp, entries: &[crate::app::AnimationFrameUopEntry]) -> u32 {
    let requested = selected_frames_body_id(app);
    let has_frames = |body: u32| entries.iter().any(|entry| entry.body_id == body);
    let fallback = is_ec_source(app.selected_legacy_source) && selected_body_definition(app).is_some();
    if !has_frames(requested) && fallback && has_frames(FALLBACK_FRAMES_BODY) {
        FALLBACK_FRAMES_BODY
    } else {
        requested
    }
}

/// Starts or polls the read of the frames stored per action of the selected body; `None` while scanning or for a
/// MUL source.
fn update_stored_frames(app: &mut UocfEditorApp, ctx: &egui::Context) -> Option<u32> {
    let source = app.selected_legacy_source;
    if !matches!(source, ArtSource::CcUop) && !is_ec_source(source) {
        app.animations_view.sequence.stored = None;
        return None;
    }
    let entries = animationframe_uop_entries(app, source)?;
    let frames_body = stored_frames_body(app, &entries);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (source as u8, frames_body, Arc::as_ptr(&entries) as usize, app.uop_cache.loaded_uops.len()).hash(&mut hasher);
    let key = hasher.finish();
    let state = &mut app.animations_view.sequence;
    if state.stored.as_ref().is_none_or(|job| job.key != key) {
        let mut jobs: BTreeMap<u16, (Arc<LoadedUop>, u64)> = BTreeMap::new();
        for entry in entries.iter().filter(|entry| entry.body_id == frames_body) {
            let (Some(action), Some(loaded)) = (entry.action_id, app.uop_cache.loaded_uops.get(entry.package_index)) else {
                continue;
            };
            jobs.entry(action).or_insert_with(|| (loaded.clone(), entry.file_hash));
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let frames = jobs
                .into_iter()
                .map(|(action, (loaded, hash))| {
                    let read = loaded
                        .package
                        .unpack_file_by_hash(hash)
                        .map_err(|error| error.to_string())
                        .and_then(|payload| payload.ok_or_else(|| "no payload".to_string()))
                        .and_then(|payload| StoredFrames::read(&payload).map_err(|error| format!("{error:#}")));
                    (action, read)
                })
                .collect();
            let _ = sender.send(frames);
        });
        state.stored = Some(StoredFramesJob { key, frames_body, receiver: Some(receiver), frames: BTreeMap::new() });
    }
    let job = state.stored.as_mut()?;
    if let Some(receiver) = &job.receiver {
        match receiver.try_recv() {
            Ok(frames) => {
                job.frames = frames;
                job.receiver = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(JOB_POLL_INTERVAL),
            Err(TryRecvError::Disconnected) => job.receiver = None,
        }
    }
    Some(job.frames_body)
}

/// The name of the folder holding the package `path` (its client's folder).
fn package_folder_name(path: &Path) -> String {
    path.parent().and_then(Path::file_name).map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn optional_text(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| "-".to_string(), |value| value.to_string())
}

fn frames_text(frames: &BTreeSet<u32>) -> String {
    if frames.is_empty() {
        return "-".to_string();
    }
    frames.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

fn offsets_text<T: PartialEq + std::fmt::Display + Copy>(offsets: &[(T, T)], unset: T) -> String {
    let value = |value: T| if value == unset { "-".to_string() } else { value.to_string() };
    offsets.iter().map(|&(x, y)| format!("({},{})", value(x), value(y))).collect::<Vec<_>>().join(" ")
}

fn cycle_text(summary: &ActionSummary) -> String {
    summary.cycle_seconds().map_or_else(|| "-".to_string(), |seconds| format!("{seconds:.3}"))
}

/// The action table, and the editor of the selected action beside it.
fn show_actions_tab(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let frames_body = update_stored_frames(app, ui.ctx());
    if app.selected_anim_sequence.is_none() {
        ui.label("No sequence entry: the actions play at the client's defaults (16 fps).");
        return;
    }
    egui::Panel::right("sequence_action_editor").resizable(true).default_size(EDIT_PANEL_WIDTH).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("sequence_action_editor_scroll").show(ui, |ui| show_action_editor(app, ui));
    });
    egui::CentralPanel::default().show(ui, |ui| show_action_table(app, ui, frames_body));
}

fn show_action_table(app: &mut UocfEditorApp, ui: &mut egui::Ui, frames_body: Option<u32>) {
    let Some((_, sequence)) = &app.selected_anim_sequence else { return };
    let stored = app.animations_view.sequence.stored.as_ref();
    let reading = stored.is_some_and(|job| job.receiver.is_some());
    ui.horizontal_wrapped(|ui| {
        match frames_body {
            Some(body) if reading => {
                ui.spinner();
                ui.weak(format!("reading the frames stored for body {body}..."));
            }
            Some(body) => {
                ui.weak(format!("Stored: frames per direction in the frame files of body {body} ({STORED_DIRECTION_COUNT} directions)."));
            }
            None if app.selected_legacy_source == ArtSource::Mul => {
                ui.weak("Stored frames are read for UOP sources; Classic plays MUL bodies at 80 ms per frame, not at the sequence fps.");
            }
            None => {
                ui.weak("Scanning the AnimationFrame packages...");
            }
        }
    });
    let summaries = sequence.action_summaries();
    let selected_action = u32::from(app.selected_action_id);
    let mut clicked = None;
    let headers = ["Action", "Frames", "Stored", "FPS", "Cycle s", "Alias", "Plays", "Skipped", "Effects", "Offsets"];
    let mut builder = TableBuilder::new(ui)
        .id_salt("sequence_actions")
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for _ in 1..headers.len() {
        builder = builder.column(Column::auto());
    }
    builder
        .column(Column::remainder())
        .header(ROW_HEIGHT, |mut header| {
            for name in headers {
                header.col(|ui| {
                    ui.strong(name);
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, summaries.len(), |mut row| {
                let summary = summaries[row.index()];
                let record = &sequence.actions[&summary.action];
                row.set_selected(summary.action == selected_action);
                row.col(|ui| {
                    ui.label(summary.action.to_string());
                });
                row.col(|ui| {
                    ui.label(summary.frame_count.to_string());
                });
                row.col(|ui| {
                    let stored_frames = summary
                        .resolved
                        .and_then(|action| u16::try_from(action).ok())
                        .and_then(|action| stored.and_then(|job| job.frames.get(&action)));
                    match (stored_frames, stored.is_some() && !reading) {
                        (Some(Ok(frames)), _) => {
                            let text = frames.per_direction_text();
                            if frames.matches(summary.frame_count) {
                                ui.label(text);
                            } else {
                                ui.colored_label(MISMATCH_COLOUR, text).on_hover_text(format!(
                                    "The frame file holds {} frames numbered up to {}; the sequence asks for {} per \
                                     direction ({}).",
                                    frames.count,
                                    frames.highest_number,
                                    summary.frame_count,
                                    STORED_DIRECTION_COUNT * summary.frame_count
                                ));
                            }
                        }
                        (Some(Err(error)), _) => {
                            ui.colored_label(MISSING_COLOUR, "error").on_hover_text(error);
                        }
                        (None, true) if summary.alias == AliasState::Plain => {
                            ui.colored_label(MISSING_COLOUR, "no file").on_hover_text("A plain action without a frame file: the client draws nothing.");
                        }
                        (None, true) => {
                            ui.weak("no file");
                        }
                        (None, false) => {
                            ui.weak("-");
                        }
                    }
                });
                row.col(|ui| {
                    ui.label(summary.fps.to_string());
                });
                row.col(|ui| {
                    ui.label(cycle_text(&summary)).on_hover_text("Shown frames / fps: skipped frames take no time.");
                });
                row.col(|ui| match summary.alias {
                    AliasState::Plain => {
                        ui.weak("-");
                    }
                    AliasState::Alias(target) => {
                        ui.label(target.to_string());
                    }
                    AliasState::AliasOfAlias(target) | AliasState::MissingTarget(target) => {
                        let why = if matches!(summary.alias, AliasState::AliasOfAlias(_)) { "is an alias itself" } else { "is not in the sequence" };
                        ui.colored_label(BROKEN_ALIAS_COLOUR, format!("{target} !")).on_hover_text(format!(
                            "Action {target} {why}: the client resolves one alias level only and plays the lowest plain \
                             action ({}) instead.",
                            optional_text(summary.resolved)
                        ));
                    }
                });
                row.col(|ui| {
                    ui.label(optional_text(summary.resolved));
                });
                row.col(|ui| {
                    ui.label(frames_text(&record.skipped_frames));
                });
                row.col(|ui| {
                    if summary.effects == 0 {
                        ui.weak("-");
                    } else {
                        let effects: Vec<String> = record
                            .frame_effects
                            .iter()
                            .flat_map(|(frame, effects)| effects.iter().map(move |effect| format!("frame {frame}: {effect:?}")))
                            .collect();
                        ui.label(summary.effects.to_string()).on_hover_text(effects.join("\n"));
                    }
                });
                row.col(|ui| {
                    if summary.offsets_set || summary.extra_offsets_set {
                        ui.label("set").on_hover_text(format!(
                            "Rider offsets: {}\nExtra offsets: {}",
                            offsets_text(&record.offsets, UNSET_BYTE_OFFSET),
                            offsets_text(&record.extra_offsets, UNSET_SHORT_OFFSET)
                        ));
                    } else {
                        ui.weak("-");
                    }
                });
                if row.response().clicked() {
                    clicked = Some(summary.action);
                }
            });
        });
    if let Some(action) = clicked.and_then(|action| u16::try_from(action).ok()) {
        app.selected_action_id = action;
        app.selected_animationframe_file_hash = None;
        app.current_frame_idx = 0;
    }
}

/// Parses "0,3" (0-based frames); empty or "none" for none.
fn parse_frames(text: &str) -> Result<BTreeSet<u32>, String> {
    let text = text.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") || text == "-" {
        return Ok(BTreeSet::new());
    }
    text.split(',').map(|frame| frame.trim().parse().map_err(|_| format!("not a frame number: {frame}"))).collect()
}

/// The fields of the selected action, each applied when changed.
fn show_action_editor(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let action = u32::from(app.selected_action_id);
    let dialect = app.animations_view.sequence.package.as_ref().map(|(_, dialect)| *dialect);
    let original = app.animations_view.sequence.original.as_ref().and_then(|original| original.actions.get(&action)).cloned();
    let Some((_, sequence)) = &mut app.selected_anim_sequence else { return };
    ui.heading(format!("Action {action}"));
    let Some(summary) = sequence.action_summary(action) else {
        ui.label(format!("The sequence has no action {action}: the client plays it at 16 fps without frames."));
        return;
    };
    let record = sequence.actions[&action].clone();
    // Two short lines, so a longer value never wraps a line and moves the fields under the pointer.
    ui.label(format!("Plays {} · {} frames at {} fps", optional_text(summary.resolved), summary.frame_count, summary.fps));
    ui.label(format!("Cycle {} s", cycle_text(&summary)));
    let mut result: Option<Result<bool, String>> = None;
    egui::Grid::new("sequence_action_fields").num_columns(2).show(ui, |ui| {
        ui.label("FPS:");
        let mut fps = record.fps;
        if ui.add(egui::DragValue::new(&mut fps).range(0.0..=MAX_EDIT_FPS).speed(FPS_DRAG_SPEED)).changed() {
            result = Some(sequence.set_fps(action, fps).map(|_| false).map_err(|error| format!("{error:#}")));
        }
        ui.end_row();

        ui.label("Frame count:");
        if record.alias.is_some() {
            ui.weak("alias: no frames of its own");
        } else {
            let mut frame_count = record.frame_count;
            if ui.add(egui::DragValue::new(&mut frame_count).range(0..=MAX_EDIT_FRAME_COUNT)).changed() {
                result = Some(sequence.set_frame_count(action, frame_count).map(|_| true).map_err(|error| format!("{error:#}")));
            }
        }
        ui.end_row();

        ui.label("Alias of:");
        let mut alias = record.alias;
        egui::ComboBox::from_id_salt("sequence_alias")
            .selected_text(alias.map_or_else(|| "none (own frames)".to_string(), |target| format!("action {target}")))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut alias, None, "none (own frames)");
                for &target in sequence.actions.keys().filter(|&&target| target != action) {
                    ui.selectable_value(&mut alias, Some(target), format!("action {target}"));
                }
            });
        if alias != record.alias {
            result = Some(sequence.set_alias(action, alias).map(|_| true).map_err(|error| format!("{error:#}")));
        }
        ui.end_row();
    });
    if original.as_ref().is_some_and(|original| original.frame_count != sequence.actions[&action].frame_count) {
        ui.colored_label(MISMATCH_COLOUR, FRAME_COUNT_WARNING);
    }

    ui.horizontal(|ui| {
        let state = &mut app.animations_view.sequence;
        if state.skip_text.as_ref().is_none_or(|(shown, _)| *shown != action) {
            state.skip_text = Some((action, frames_text(&record.skipped_frames)));
        }
        let Some((_, text)) = &mut state.skip_text else { return };
        let has_skipped = dialect != Some(ClientDialect::KingdomReborn);
        ui.label("Skipped frames:");
        ui.add_enabled(has_skipped, egui::TextEdit::singleline(text).desired_width(80.0))
            .on_hover_text("0-based frames within a direction, comma separated; the player passes over them.")
            .on_disabled_hover_text("The Kingdom Reborn layout stores no skipped frames.");
        if ui.add_enabled(has_skipped, egui::Button::new("Apply")).clicked() {
            result = Some(match parse_frames(text) {
                Ok(frames) => sequence.set_skipped_frames(action, frames).map(|_| false).map_err(|error| format!("{error:#}")),
                Err(error) => Err(error),
            });
        }
    });

    if !record.frame_effects.is_empty() {
        egui::CollapsingHeader::new(format!("Frame effects ({})", summary.effects)).id_salt("sequence_action_effects").show(ui, |ui| {
            for (frame, effects) in &record.frame_effects {
                for effect in effects {
                    ui.label(format!("frame {frame}: {effect:?}"));
                }
            }
        });
    }
    egui::CollapsingHeader::new("Offsets").id_salt("sequence_action_offsets").show(ui, |ui| {
        ui.label(format!("Rider: {}", offsets_text(&record.offsets, UNSET_BYTE_OFFSET)));
        ui.label(format!("Extra: {}", offsets_text(&record.extra_offsets, UNSET_SHORT_OFFSET)));
        ui.weak("Per direction 0..7; - is the sequence's value.");
    });

    match result {
        Some(Ok(frames_change)) => record_edit(app, frames_change),
        Some(Err(error)) => app.status_message = format!("Sequence edit refused: {error}"),
        None => {}
    }
}

/// The parent chain of `group`, the group first (cycles cut).
fn group_chain(sequence: &AnimationSequence, group: i8) -> Vec<i8> {
    let mut chain = vec![group];
    while let Some(parent) = sequence.groups.get(chain.last().unwrap()).and_then(|lookup| lookup.parent) {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
    }
    chain
}

/// Every group with its moves (editable), parent chain, transitions and typed actions.
fn show_groups_tab(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let Some((_, sequence)) = &mut app.selected_anim_sequence else {
        ui.label("No sequence entry.");
        return;
    };
    let mut edit = None;
    egui::ScrollArea::vertical().id_salt("sequence_groups").show(ui, |ui| {
        let keys: Vec<i8> = sequence.groups.keys().copied().collect();
        for key in keys {
            let group = sequence.groups[&key].clone();
            let chain: Vec<String> = group_chain(sequence, key).iter().map(i8::to_string).collect();
            let title = format!(
                "Group {key} ({}) · parent {} · default action {}",
                group_name(key),
                optional_text(group.parent),
                group.default_action
            );
            egui::CollapsingHeader::new(title).id_salt(("sequence_group", key)).default_open(true).show(ui, |ui| {
                ui.weak(format!("Lookups search {}; the default action of the last one is the fallback.", chain.join(" -> ")));
                egui::Grid::new(("sequence_group_moves", key)).num_columns(3).striped(true).show(ui, |ui| {
                    ui.strong("Movement");
                    ui.strong("Action");
                    ui.strong("Alternate");
                    ui.end_row();
                    for (&movement, moves) in &group.moves {
                        ui.label(movement_name(movement));
                        let mut changed = *moves;
                        ui.add(egui::DragValue::new(&mut changed.action));
                        ui.add(egui::DragValue::new(&mut changed.alternate));
                        if changed != *moves {
                            edit = Some((key, movement, changed));
                        }
                        ui.end_row();
                    }
                });
                if !group.transitions.is_empty() {
                    let transitions: Vec<String> =
                        group.transitions.iter().map(|(transition, action)| format!("{transition} -> {action}")).collect();
                    ui.label(format!("Transitions: {}", transitions.join(", ")));
                }
                if !group.typed.is_empty() {
                    egui::CollapsingHeader::new(format!("Typed actions ({})", group.typed.len()))
                        .id_salt(("sequence_group_typed", key))
                        .show(ui, |ui| {
                            egui::Grid::new(("sequence_group_typed_grid", key)).num_columns(2).striped(true).show(ui, |ui| {
                                for ((kind, sub_type), actions) in &group.typed {
                                    ui.label(format!("{kind}/{sub_type}"));
                                    let actions: Vec<String> = actions.iter().map(u32::to_string).collect();
                                    ui.label(if actions.is_empty() { "-".to_string() } else { actions.join(" ") });
                                    ui.end_row();
                                }
                            });
                        });
                }
            });
        }
    });
    if let Some((group, movement, moves)) = edit {
        match sequence.set_move_actions(group, movement, moves) {
            Ok(_) => record_edit(app, false),
            Err(error) => app.status_message = format!("Sequence edit refused: {error:#}"),
        }
    }
}

/// The sequence entry `body` animates with, from the shown package: its placeholder's body type, else the body;
/// its edited copy when there is one.
fn package_sequence(app: &UocfEditorApp, body: u32) -> Option<(u32, AnimationSequence)> {
    let sequence_id = body_definition(app, body).map_or(body, |definition| definition.sequence_id);
    let (path, dialect) = app.animations_view.sequence.package.as_ref()?;
    if let Some(edited) = app.animations_view.sequence.edits.get(&(path.clone(), sequence_id)) {
        return Some((sequence_id, edited.clone()));
    }
    let loaded = app.uop_cache.loaded_uops.iter().find(|loaded| &loaded.path == path)?;
    read_sequence(&loaded.package, sequence_id, *dialect).ok().flatten().map(|sequence| (sequence_id, sequence))
}

/// The shown body as a rider on a picked mount: their move actions per movement side by side.
fn show_ride_tab(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let Some((rider_id, rider)) = app.selected_anim_sequence.clone() else {
        ui.label("No sequence entry.");
        return;
    };
    let mut mount_body = app.animations_view.sequence.mount_body.unwrap_or(DEFAULT_MOUNT_BODY);
    ui.horizontal_wrapped(|ui| {
        ui.label("Mount body:");
        ui.add(egui::DragValue::new(&mut mount_body));
        for body in MOUNT_REFERENCE_BODIES {
            ui.selectable_value(&mut mount_body, body, body.to_string());
        }
    });
    app.animations_view.sequence.mount_body = Some(mount_body);
    let Some((mount_id, mount)) = package_sequence(app, mount_body) else {
        ui.colored_label(MISSING_COLOUR, format!("Body {mount_body} has no sequence entry."));
        return;
    };
    if !rider.groups.contains_key(&GROUP_MOUNTED) {
        ui.colored_label(MISMATCH_COLOUR, format!("Sequence {rider_id} has no mounted group (2): it does not ride."));
    }
    ui.weak(format!(
        "Rider: sequence {rider_id}, group 2 (mounted). Mount: sequence {mount_id}, group {} ({}): group 2 when its \
         sequence has one, else 1. A cycle is shown frames / fps.",
        mount.mount_group(),
        group_name(mount.mount_group())
    ));
    let side = |ui: &mut egui::Ui, summary: Option<ActionSummary>| match summary {
        Some(summary) => {
            ui.label(summary.action.to_string());
            ui.label(summary.shown_frames().to_string());
            ui.label(summary.fps.to_string());
            ui.label(cycle_text(&summary));
        }
        None => {
            for _ in 0..4 {
                ui.weak("-");
            }
        }
    };
    egui::Grid::new("sequence_ride").num_columns(10).striped(true).show(ui, |ui| {
        for header in ["Movement", "Rider action", "Frames", "FPS", "Cycle s", "Mount action", "Frames", "FPS", "Cycle s", "Difference"] {
            ui.strong(header);
        }
        ui.end_row();
        for timing in ride_timings(&rider, &mount) {
            ui.label(movement_name(timing.movement));
            side(ui, timing.rider);
            side(ui, timing.mount);
            let cycles = (timing.rider.and_then(|rider| rider.cycle_seconds()), timing.mount.and_then(|mount| mount.cycle_seconds()));
            match cycles {
                (Some(rider), Some(mount)) if timing.cycles_differ() => {
                    ui.colored_label(MISMATCH_COLOUR, format!("{:+.3} s", rider - mount))
                        .on_hover_text("The rider's cycle differs from the mount's: their frames drift apart.");
                }
                (Some(_), Some(_)) => {
                    ui.label("same");
                }
                _ => {
                    ui.weak("-");
                }
            }
            ui.end_row();
        }
    });
}

/// A scan of the source's whole client folder, writing a markdown report and a TSV beside it.
fn show_scan_tab(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let folder = source_folder(app);
    if let Some(job) = &mut app.animations_view.sequence.scan {
        if let Some(receiver) = &job.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    job.result = Some(result);
                    job.receiver = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint_after(JOB_POLL_INTERVAL),
                Err(TryRecvError::Disconnected) => {
                    job.result = Some(Err("the scan stopped".to_string()));
                    job.receiver = None;
                }
            }
        }
    }
    let running = app.animations_view.sequence.scan.as_ref().is_some_and(|job| job.receiver.is_some());
    ui.horizontal_wrapped(|ui| {
        let clicked = ui
            .add_enabled(folder.is_some() && !running, egui::Button::new("Scan client folder..."))
            .on_hover_text(
                "Checks the folder's AnimationSequence.uop against its AnimationFrame*.uop: frame counts against the \
                 frames stored, aliases of aliases, plain actions without a frame file, frame bodies without a \
                 sequence, and rider / mount cycle mismatches. Writes a markdown report and a TSV beside it.",
            )
            .clicked();
        match &folder {
            Some(folder) => ui.weak(folder.display().to_string()),
            None => ui.weak("Select a source with a client folder."),
        };
        if let (true, Some(folder)) = (clicked, &folder) {
            start_scan(app, folder);
        }
    });
    let Some(job) = &app.animations_view.sequence.scan else { return };
    match &job.result {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Scanning {}...", job.folder.display()));
            });
        }
        Some(Err(error)) => {
            ui.colored_label(MISSING_COLOUR, format!("Scan failed: {error}"));
        }
        Some(Ok(summary)) => {
            ui.label(format!("{}: report {}", job.folder.display(), job.report.display()));
            egui::Grid::new("sequence_scan_summary").num_columns(2).striped(true).show(ui, |ui| {
                for (label, count) in summary {
                    ui.label(count.to_string());
                    ui.label(label);
                    ui.end_row();
                }
            });
        }
    }
}

/// Asks for the report path and scans `folder` in the background.
fn start_scan(app: &mut UocfEditorApp, folder: &Path) {
    let Some(report) = crate::dialog::file_dialog()
        .set_title("AnimationSequence scan report")
        .add_filter(MARKDOWN_FILE_FILTER.0, MARKDOWN_FILE_FILTER.1)
        .set_file_name(SCAN_REPORT_NAME)
        .save_file()
    else {
        return;
    };
    if report.starts_with(folder) {
        app.status_message = format!("The scan report must be outside the client folder {}", folder.display());
        return;
    }
    let tsv = report.with_extension(TSV_EXTENSION);
    if tsv == report {
        app.status_message = format!("The scan report must not be a .{TSV_EXTENSION} file");
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let (scan_folder, scan_report) = (folder.to_path_buf(), report.clone());
    std::thread::spawn(move || {
        let result = (|| -> eyre::Result<Vec<(String, usize)>> {
            let scan = ScanReport::new(&ClientSequences::load(&scan_folder)?);
            std::fs::write(&scan_report, scan.to_markdown())?;
            std::fs::write(&tsv, scan.to_tsv())?;
            Ok(scan.summary())
        })();
        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
    });
    app.animations_view.sequence.scan =
        Some(ScanJob { folder: folder.to_path_buf(), report, receiver: Some(receiver), result: None });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_frames_parse_from_a_comma_list() {
        assert_eq!(parse_frames(" 0, 3 "), Ok(BTreeSet::from([0, 3])));
        assert_eq!(parse_frames("none"), Ok(BTreeSet::new()));
        assert_eq!(parse_frames(""), Ok(BTreeSet::new()));
        assert!(parse_frames("1,x").is_err());
    }

    #[test]
    fn an_entry_names_the_client_folder_of_its_package() {
        let classic = Path::new("/clients/Ultima Online Classic/AnimationSequence.uop");
        assert_eq!(package_folder_name(classic), "Ultima Online Classic");
        assert_eq!(package_folder_name(Path::new("AnimationSequence.uop")), "AnimationSequence.uop");
    }
}
