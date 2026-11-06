use crate::app::{
    AnimationFrameUopEntry, AnimationFrameUopScanResult, ArtSource, UocfEditorApp, ANIMATIONFRAME_UOP_NAMES,
};
use crate::ui::animation_frames::{self, ActionFrames, FramesViewState};
use crate::ui::animation_hue::{self, AnimationHueState};
use crate::ui::animation_list::{self, ActionKey, ActionSource, AnimationListState};
use crate::ui::{
    enhanced_files_availability, folder_files_availability, source_choice, ClientSource,
    CLASSIC_FOLDER,
};
use color_eyre::eyre;
use eframe::egui;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use uocf::animation_sequence::{AnimationSequence, DIRECTION_COUNT};
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::michelangelo_uop_codec::{
    export_anim_blocks_from_mul, MichelangeloPatch, MichelangeloPatchEntry,
};
use uocf::classic::vd_codec::VdFile;
use uocf::enhanced::mobile_body::{MobileBodyDefinition, FALLBACK_FRAMES_BODY, MOBILE_DEFINITION_BASE};
use uocf::uop_container::hash::hash_file_name_single;

/// State of the animations view kept across frames.
#[derive(Default)]
pub(crate) struct AnimationsView {
    pub(super) list: AnimationListState,
    pub(super) frames: FramesViewState,
    pub(super) hue: AnimationHueState,
    /// The body type and the loaded package count of the last `AnimationSequence` lookup, so a missing entry is
    /// looked up once.
    sequence_lookup: Option<(u32, usize)>,
    /// The source the controls showed last, so a switch from any tab resets what belongs to the old source.
    shown_source: Option<ArtSource>,
}

const ANIMATION_CONTROLS_PANEL_WIDTH: f32 = 280.0;
const ANIMATION_ACTIONS_PANEL_WIDTH: f32 = 260.0;
/// Client default of the `flying_animations` setting, used to resolve sequence actions.
const FLYING_ANIMATIONS_ENABLED: bool = true;
/// The anim file index of a source without MUL files: anim.mul, or the first AnimationFrame package.
const DEFAULT_ANIM_FILE_INDEX: u8 = 0;
const ANIM_MUL_FILES: &str = "anim*.mul and anim*.idx";
const ANIMATION_FRAME_UOP_FILES: &str = "AnimationFrame*.uop";

pub fn ui_animations(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::left("anim_controls")
        .resizable(true)
        .default_size(ANIMATION_CONTROLS_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading("Animation Controls");
            ui.separator();

            let mut body_id = app.selected_anim_id;
            ui.horizontal(|ui| {
                ui.label("Body ID:");
                if ui.add(egui::DragValue::new(&mut body_id)).changed() {
                    app.selected_anim_id = body_id;
                    app.selected_animationframe_file_hash = None;
                    app.current_frame_idx = 0;
                }
            });

            // The EC client draws a server body through its tileart.uop creature placeholder.
            let sequence_id = selected_sequence_id(app);
            let frames_body_id = selected_frames_body_id(app);
            if sequence_id != body_id || frames_body_id != body_id {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Redirected to: sequence {sequence_id}, frames {frames_body_id}"),
                );
            }
            if let Some(forced_hue) = selected_body_definition(app).and_then(|definition| definition.forced_hue()) {
                ui.colored_label(egui::Color32::YELLOW, format!("Drawn with hue {forced_hue:#06x}"));
            }

            ui.horizontal(|ui| {
                ui.label("Anim File Index:");
                ui.add(egui::DragValue::new(&mut app.selected_anim_file_idx).range(0..=5));
            });

            ui.separator();
            ui.heading("Source");
            animation_source_selector(app, ui);
            let shown_source = app.animations_view.shown_source.replace(app.selected_legacy_source);
            if shown_source.is_some_and(|shown| !same_animation_files(shown, app.selected_legacy_source)) {
                app.selected_animationframe_file_hash = None;
                app.current_frame_idx = 0;
                app.selected_anim_file_idx = first_anim_file_index(app, app.selected_legacy_source);
            }

            if app.selected_legacy_source == ArtSource::Mul {
                ui.separator();
                ui.heading("Classic MUL Layout");
                ui.horizontal(|ui| {
                    ui.label("Action:");
                    ui.add(egui::DragValue::new(&mut app.selected_action_id));
                });
                ui.horizontal(|ui| {
                    ui.label("Direction:");
                    for d in 0..5 {
                        ui.selectable_value(&mut app.selected_direction, d, d.to_string());
                    }
                });
            }

            let listed_action = app.selected_action_id;
            if let Some((_, seq)) = &app.selected_anim_sequence {
                ui.separator();
                ui.heading("Logical Sequence");

                egui::Grid::new("anim_seq_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Action:");
                        egui::ComboBox::from_id_salt("action_combo")
                            .selected_text(format!("Action {}", app.selected_action_id))
                            .show_ui(ui, |ui| {
                                let action_ids = seq.actions.keys().filter_map(|&id| u16::try_from(id).ok());
                                for id in action_ids {
                                    ui.selectable_value(
                                        &mut app.selected_action_id,
                                        id,
                                        format!("Action {}", id),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("Direction:");
                        ui.horizontal(|ui| {
                            for d in 0..DIRECTION_COUNT as u8 {
                                ui.selectable_value(&mut app.selected_direction, d, d.to_string());
                            }
                        });
                        ui.end_row();
                    });
            }
            // An action picked here is the server's: the body is then drawn through its placeholder.
            if app.selected_action_id != listed_action {
                app.selected_animationframe_file_hash = None;
            }

            show_animation_navigation(app, ctx, ui);
        });

    egui::Panel::right("anim_actions")
        .resizable(true)
        .default_size(ANIMATION_ACTIONS_PANEL_WIDTH)
        .show(ui, |ui| {
            show_animation_action_column(app, ui);
        });

    egui::CentralPanel::default().show(ui, |ui| {
        if app.client_data.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label("Load client data first");
            });
            return;
        }
        let body_type = selected_sequence_id(app);

        // Try to load AnimationSequence if not already loaded for this body type
        let lookup = (body_type, app.uop_cache.loaded_uops.len());
        if app.animations_view.sequence_lookup != Some(lookup) {
            app.animations_view.sequence_lookup = Some(lookup);
            let mut found_seq = None;
            for loaded in &app.uop_cache.loaded_uops {
                if loaded.path.file_name().unwrap_or_default().to_string_lossy().contains("AnimationSequence") {
                    let hash = hash_file_name_single(&AnimationSequence::entry_path(body_type));
                    // The package is loaded lazily: its entries are read from the file on demand.
                    match loaded.package.unpack_file_by_hash(hash).map(|data| data.map(|data| AnimationSequence::parse(&data))) {
                        Ok(Some(Ok(seq))) => {
                            found_seq = Some((body_type, seq));
                            break;
                        }
                        Ok(Some(Err(error))) => log::warn!("AnimationSequence {body_type}: {error:#}"),
                        Err(error) => log::warn!("AnimationSequence {body_type}: {error}"),
                        Ok(None) => {}
                    }
                }
            }
            app.selected_anim_sequence = found_seq;
        }

        let ActionTarget { key, sequence } = match action_target(app) {
            Ok(target) => target,
            Err(message) => {
                ui.label(message);
                return;
            }
        };
        let decode_key = action_decode_key(app, key, sequence);
        let frames = animation_frames::decoded_frames(app, decode_key, |app| decode_action(app, key, sequence));
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Body {} · action {} · {}", key.body, key.action, key.provenance()));
            if key.body != app.selected_anim_id {
                ui.weak(format!("(drawn for body {})", app.selected_anim_id));
            }
            if key.action != app.selected_action_id {
                ui.weak(format!("(action {} is an alias of {})", app.selected_action_id, key.action));
            }
            if let Some(sequence) = sequence {
                if app.selected_anim_sequence.as_ref().is_some_and(|(id, _)| *id == sequence) {
                    ui.weak(format!("· AnimationSequence {sequence}"));
                } else {
                    ui.weak(format!("· no AnimationSequence {sequence}"));
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(&frames.split);
            if frames.unplaced > 0 {
                ui.colored_label(egui::Color32::YELLOW, format!("{} frames fall in no direction", frames.unplaced));
            }
            if frames.failed > 0 {
                ui.colored_label(egui::Color32::YELLOW, format!("{} frames failed to decode (see the log)", frames.failed));
            }
        });
        show_action_file_controls(app, ui, key);
        let is_ec = is_ec_source(app.selected_legacy_source);
        animation_hue::hue_picker(app, ui, is_ec);
        if let Some(error) = &frames.error {
            ui.label(format!("Error loading animation: {error}"));
            return;
        }
        ui.separator();
        let name = format!("{}_body_{}_action_{}", animation_source_label(app.selected_legacy_source), key.body, key.action);
        let hue = animation_hue::frame_hue(app, is_ec);
        animation_frames::show_player(app, ctx, ui, &frames, &hue, &name);
        ui.separator();
        animation_frames::show_grid(app, ctx, ui, &frames, &hue);
    });
}

fn show_animation_action_column(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    ui.heading("Animation Actions");
    ui.separator();

    if app.selected_legacy_source != ArtSource::Any {
        let export_context = app.client_data.as_ref().map(|_| selected_frames_body_id(app));

        ui.heading("Patch Export");
        let can_export = export_context.is_some();
        ui.vertical(|ui| {
            if ui
                .add_enabled(can_export, egui::Button::new("Export VD"))
                .clicked()
            {
                if let Some(body_id) = export_context {
                    let default_name =
                        format!("anim_{}_{}.vd", app.selected_anim_file_idx, body_id);
                    if let Some(path) = crate::dialog::file_dialog()
                        .set_file_name(default_name)
                        .save_file()
                    {
                        match export_selected_animation_patch(
                            app,
                            body_id,
                            AnimationPatchExportFormat::Vd,
                            &path,
                        ) {
                            Ok(entry_count) => {
                                app.status_message = format!(
                                    "Exported {entry_count} animation patch entry to {}.",
                                    path.display()
                                );
                            }
                            Err(e) => {
                                app.status_message =
                                    format!("Failed to export animation .vd: {e}");
                            }
                        }
                    }
                }
            }

            if ui
                .add_enabled(can_export, egui::Button::new("Export Michelangelo UOP"))
                .clicked()
            {
                if let Some(body_id) = export_context {
                    let default_name =
                        format!("anim_{}_{}.uop", app.selected_anim_file_idx, body_id);
                    if let Some(path) = crate::dialog::file_dialog()
                        .set_file_name(default_name)
                        .save_file()
                    {
                        match export_selected_animation_patch(
                            app,
                            body_id,
                            AnimationPatchExportFormat::MichelangeloUop,
                            &path,
                        ) {
                            Ok(entry_count) => {
                                app.status_message = format!(
                                    "Exported {entry_count} animation patch entry to {}.",
                                    path.display()
                                );
                            }
                            Err(e) => {
                                app.status_message =
                                    format!("Failed to export animation .uop: {e}");
                            }
                        }
                    }
                }
            }
        });
    } else {
        ui.label("Select an animation source before exporting patches.");
    }

    ui.separator();
    ui.heading("Playback");
    ui.horizontal(|ui| {
        if ui
            .button(if app.is_playing { "⏸ Stop" } else { "▶ Play" })
            .clicked()
        {
            app.is_playing = !app.is_playing;
        }
        if ui.button("⏮").clicked() {
            app.current_frame_idx = 0;
        }
        if ui.button("⬅").clicked() {
            app.current_frame_idx = app.current_frame_idx.saturating_sub(1);
        }
        if ui.button("➡").clicked() {
            app.current_frame_idx += 1;
        }
    });

    ui.add(egui::Slider::new(&mut app.playback_speed, 0.1..=5.0).text("Speed"));
    ui.checkbox(&mut app.loop_animation, "Loop Animation");
}

fn show_animation_navigation(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.separator();
    ui.heading("Animation Tree");
    match app.selected_legacy_source {
        ArtSource::Mul | ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
            // An Enhanced body typed in the field is drawn through its placeholder, so only a listed entry names one.
            let match_unhashed = !is_ec_source(app.selected_legacy_source);
            animation_list::show_action_list(app, ctx, ui, match_unhashed);
        }
        ArtSource::Any => {
            ui.label("Select an animation source to browse.");
        }
    }
}

/// The entry the central panel draws, and the body whose `AnimationSequence` entry splits its frames (Enhanced).
struct ActionTarget {
    key: ActionKey,
    sequence: Option<u32>,
}

/// The listed entry the selection names; else the typed body and action: Classic as typed, Enhanced through the
/// placeholder of the body (its frames body, else body 0 like the client) and the sequence's action alias.
fn action_target(app: &mut UocfEditorApp) -> Result<ActionTarget, String> {
    let source = app.selected_legacy_source;
    let is_ec = is_ec_source(source);
    let keys = animation_list::action_keys(app, source).ok_or("Scanning AnimationFrame UOP metadata...")?;
    let sequence = is_ec.then(|| selected_sequence_id(app));
    if let Some(key) = animation_list::find_selected(&keys, animation_list::selection_of(app), false) {
        return Ok(ActionTarget { key, sequence });
    }
    let action = match &app.selected_anim_sequence {
        Some((id, sequence_entry)) if is_ec && Some(*id) == sequence => sequence_entry
            .resolve_action(u32::from(app.selected_action_id), FLYING_ANIMATIONS_ENABLED)
            .and_then(|action| u16::try_from(action).ok())
            .unwrap_or(app.selected_action_id),
        _ => app.selected_action_id,
    };
    let group = app.selected_anim_file_idx;
    let find = |body: u32| {
        keys.iter().filter(|key| key.body == body && key.action == action).min_by_key(|key| key.group() != group).copied()
    };
    let body = selected_frames_body_id(app);
    let key = find(body).or_else(|| {
        (is_ec && selected_body_definition(app).is_some() && body != FALLBACK_FRAMES_BODY)
            .then(|| find(FALLBACK_FRAMES_BODY))
            .flatten()
    });
    let files = match source {
        ArtSource::Mul => animation_list::mul_file_name(group),
        _ => "AnimationFrame UOP packages".to_string(),
    };
    key.map(|key| ActionTarget { key, sequence }).ok_or_else(|| format!("Body {body} has no action {action} in {files}."))
}

/// Hash of everything the frames of `key` are decoded from.
fn action_decode_key(app: &UocfEditorApp, key: ActionKey, sequence: Option<u32>) -> u64 {
    let sequence_loaded = app.selected_anim_sequence.as_ref().is_some_and(|(id, _)| Some(*id) == sequence);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (app.selected_legacy_source as u8, key, sequence, sequence_loaded, app.uop_cache.loaded_uops.len()).hash(&mut hasher);
    hasher.finish()
}

/// The frames of every stored direction of `key`'s action.
fn decode_action(app: &mut UocfEditorApp, key: ActionKey, sequence: Option<u32>) -> ActionFrames {
    match key.source {
        ActionSource::Mul { file_index, .. } => {
            let Some(anim_map) = app.client_data.as_ref().and_then(|client| client.anim_map.clone()) else {
                return ActionFrames::from_error("Classic animation MUL sources are not loaded".to_string());
            };
            let Ok(body) = u16::try_from(key.body) else {
                return ActionFrames::from_error(format!("Body {} is outside the classic MUL animation layout", key.body));
            };
            let split = format!("one {} entry per direction", animation_list::mul_file_name(file_index));
            ActionFrames::classic(
                |direction| {
                    let index = uocf::classic::anim::classic_animation_source_index_from_identity(file_index, body, key.action, direction)?;
                    anim_map.has_anim(file_index, index).then(|| anim_map.decode_animation_index(file_index, index))
                },
                split,
            )
        }
        ActionSource::Uop { package_index, file_hash, .. } => {
            let payload = listed_uop_entry(app, package_index, file_hash)
                .ok_or_else(|| eyre::eyre!("AnimationFrame entry {file_hash:#018x} is no longer listed"))
                .and_then(|entry| animationframe_payload(app, &entry));
            let payload = match payload {
                Ok(payload) => payload,
                Err(error) => return ActionFrames::from_error(format!("{error:#}")),
            };
            if !is_ec_source(app.selected_legacy_source) {
                let split = "one AnimationFrame entry holding every direction".to_string();
                return ActionFrames::classic(|direction| Some(AnimationFrameCc::decode_direction(&payload, direction)), split);
            }
            let sequence = app.selected_anim_sequence.as_ref().filter(|(id, _)| Some(*id) == sequence).map(|(_, sequence)| sequence);
            ActionFrames::enhanced(&payload, sequence, key.action).unwrap_or_else(|error| ActionFrames::from_error(format!("{error:#}")))
        }
    }
}

/// The scanned AnimationFrame UOP entry of a listed action.
fn listed_uop_entry(app: &mut UocfEditorApp, package_index: usize, file_hash: u64) -> Option<AnimationFrameUopEntry> {
    let source = app.selected_legacy_source;
    let entries = animationframe_uop_entries(app, source)?;
    entries.iter().find(|entry| entry.package_index == package_index && entry.file_hash == file_hash).cloned()
}

/// Export and replace of the frames of `key`'s action.
fn show_action_file_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui, key: ActionKey) {
    match key.source {
        ActionSource::Mul { file_index, .. } => {
            if let Ok(body_id) = u16::try_from(key.body) {
                show_classic_frames_controls(app, ui, &ClassicFramesTarget::Mul { file_index, body_id, action: key.action });
            }
        }
        ActionSource::Uop { package_index, file_hash, .. } => {
            let Some(entry) = listed_uop_entry(app, package_index, file_hash) else {
                return;
            };
            if is_ec_source(app.selected_legacy_source) {
                show_animationframe_replace_controls(app, ui, &entry, key.body, key.action);
            } else {
                show_classic_frames_controls(app, ui, &ClassicFramesTarget::Uop { entry, body_id: key.body, action: key.action });
            }
        }
    }
}

/// Name of the sequence package searched next to an AnimationFrame package.
const ANIMATION_SEQUENCE_UOP_NAME: &str = "AnimationSequence.uop";
const UOP_FILE_FILTER: (&str, &[&str]) = ("UOP package", &["uop"]);

/// "Replace frames": export the entry as PNGs plus `animationframe.kdl`, or encode such a folder back into a copy
/// of the package (and of `AnimationSequence.uop` when the frames per direction change).
fn show_animationframe_replace_controls(
    app: &mut UocfEditorApp,
    ui: &mut egui::Ui,
    entry: &AnimationFrameUopEntry,
    body_id: u32,
    action: u16,
) {
    ui.horizontal(|ui| {
        if ui
            .button("Export frames...")
            .on_hover_text(
                "Writes the frames as RGBA PNGs plus animationframe.kdl (anchors, frame numbers, palette) into a new \
                 subfolder.",
            )
            .clicked()
        {
            if let Some(dir) = crate::dialog::pick_folder() {
                app.status_message = match export_animationframe_folder(app, entry, body_id, action, &dir) {
                    Ok(message) => message,
                    Err(error) => format!("Frame export failed: {error:#}"),
                };
            }
        }
        if ui
            .button("Replace frames...")
            .on_hover_text(format!(
                "Encodes a folder written by Export frames (edited) and writes a copy of the package with this entry \
                 replaced; the opened package is refused as the target. When the frames per direction change, a copy \
                 of the {ANIMATION_SEQUENCE_UOP_NAME} next to the package is written too.\n{}",
                crate::ui::UOP_PATCHER_NOTE
            ))
            .clicked()
        {
            if let Some(dir) = crate::dialog::pick_folder() {
                app.status_message = match replace_animationframe_entry(app, entry, body_id, action, &dir) {
                    Ok(message) => message,
                    Err(error) => format!("Frame replace failed: {error:#}"),
                };
            }
        }
    });
}

fn animationframe_package_path(app: &UocfEditorApp, entry: &AnimationFrameUopEntry) -> eyre::Result<PathBuf> {
    app.uop_cache
        .loaded_uops
        .get(entry.package_index)
        .map(|loaded| loaded.path.clone())
        .ok_or_else(|| eyre::eyre!("AnimationFrame package {} is no longer loaded", entry.package_index))
}

/// The sequence package next to `package`, and the action's frames per direction there (sequence id = body).
fn animationframe_sequence(package: &Path, body_id: u32, action: u16) -> eyre::Result<Option<(PathBuf, u32)>> {
    let Some(path) = package
        .parent()
        .and_then(|dir| uocf::utils::path::find_file_ignoring_case(dir, ANIMATION_SEQUENCE_UOP_NAME))
    else {
        return Ok(None);
    };
    let sequences = uocf::uop_container::package::UopPackage::load(&path)?;
    let dialect = uocf::enhanced::client_dialect::ClientDialect::of_package_path(&path);
    let count =
        uocf::enhanced::animationframe::package::sequence_frame_count(&sequences, body_id, u32::from(action), dialect)?;
    Ok(count.map(|count| (path, count)))
}

fn export_animationframe_folder(
    app: &UocfEditorApp,
    entry: &AnimationFrameUopEntry,
    body_id: u32,
    action: u16,
    dir: &Path,
) -> eyre::Result<String> {
    let payload = animationframe_payload(app, entry)?;
    let package = animationframe_package_path(app, entry)?;
    let frames_per_direction = animationframe_sequence(&package, body_id, action)?.map(|(_, count)| count);
    let output = dir.join(format!("body{body_id:05}_action{action:02}"));
    let sidecar = uocf::enhanced::animationframe::folder::export_folder(
        &payload,
        body_id,
        action,
        frames_per_direction,
        true,
        &output,
    )?;
    Ok(format!("Exported {} frames to {}", sidecar.frames.len(), output.display()))
}

fn replace_animationframe_entry(
    app: &UocfEditorApp,
    entry: &AnimationFrameUopEntry,
    body_id: u32,
    action: u16,
    dir: &Path,
) -> eyre::Result<String> {
    use uocf::animation_sequence::STORED_DIRECTION_COUNT;
    use uocf::enhanced::animationframe::package::{write_entry, write_sequence_frame_count};

    let imported = uocf::enhanced::animationframe::folder::import_folder(dir)?;
    let sidecar = &imported.sidecar;
    eyre::ensure!(
        (sidecar.body, sidecar.action) == (body_id, action),
        "the folder holds body {} action {}, not body {body_id} action {action}",
        sidecar.body,
        sidecar.action
    );
    let package = animationframe_package_path(app, entry)?;
    let frames = sidecar.frames.len();
    let per_direction = sidecar.frames_per_direction.or_else(|| {
        let count = u32::try_from(frames).ok()?;
        (count % STORED_DIRECTION_COUNT == 0).then_some(count / STORED_DIRECTION_COUNT)
    });
    let sequence_change = match animationframe_sequence(&package, body_id, action)? {
        Some((path, current)) if per_direction != Some(current) => {
            let new = per_direction.ok_or_else(|| {
                eyre::eyre!(
                    "{frames} frames are not a multiple of {STORED_DIRECTION_COUNT} directions: set \
                     frames-per-direction= in the sidecar"
                )
            })?;
            Some((path, current, new))
        }
        _ => None,
    };

    let file_name = package.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(output) = crate::dialog::file_dialog()
        .add_filter(UOP_FILE_FILTER.0, UOP_FILE_FILTER.1)
        .set_file_name(&file_name)
        .save_file()
    else {
        return Ok("Frame replace cancelled".to_string());
    };
    let mut sequence_message = String::new();
    if let Some((sequence_path, current, new)) = sequence_change {
        let Some(sequence_output) = crate::dialog::file_dialog()
            .set_title(format!("{ANIMATION_SEQUENCE_UOP_NAME}: {current} -> {new} frames per direction"))
            .add_filter(UOP_FILE_FILTER.0, UOP_FILE_FILTER.1)
            .set_file_name(ANIMATION_SEQUENCE_UOP_NAME)
            .save_file()
        else {
            return Ok("Frame replace cancelled: the frames per direction change and need a sequence copy".to_string());
        };
        let dialect = uocf::enhanced::client_dialect::ClientDialect::of_package_path(&sequence_path);
        write_sequence_frame_count(&sequence_path, &sequence_output, body_id, u32::from(action), new, dialect)?;
        sequence_message = format!("; {current} -> {new} frames per direction in {}", sequence_output.display());
    }
    let change = write_entry(&package, &output, body_id, action, &imported.payload)?;
    let report = imported.report;
    Ok(format!(
        "Wrote {} ({frames} frames, {} colours{}, entry {change:?}){sequence_message}",
        output.display(),
        report.distinct_colours,
        if report.quantized { ", quantized" } else { "" }
    ))
}

/// The Classic frames an export or replace works on: an `animationlegacyframe` entry (all five directions of an
/// action) of a CC AnimationFrame*.uop, or the five direction entries of an action in `anim*.mul`.
enum ClassicFramesTarget {
    Uop { entry: AnimationFrameUopEntry, body_id: u32, action: u16 },
    Mul { file_index: u8, body_id: u16, action: u16 },
}

impl ClassicFramesTarget {
    fn body_action(&self) -> (u32, u16) {
        match self {
            Self::Uop { body_id, action, .. } => (*body_id, *action),
            Self::Mul { body_id, action, .. } => (u32::from(*body_id), *action),
        }
    }
}

const MUL_INDEX_FILE_FILTER: (&str, &[&str]) = ("Animation index", &["idx"]);
const MUL_DATA_FILE_FILTER: (&str, &[&str]) = ("Animation data", &["mul"]);

/// "Export frames" / "Replace frames" for Classic frames (`classicanimation.kdl` folders).
fn show_classic_frames_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui, target: &ClassicFramesTarget) {
    let what = match target {
        ClassicFramesTarget::Uop { .. } => "a copy of the package with this entry replaced",
        ClassicFramesTarget::Mul { .. } => {
            "copies of anim*.idx and anim*.mul with the action's direction entries appended (verdata.mul is not read)"
        }
    };
    ui.horizontal(|ui| {
        if ui
            .button("Export frames...")
            .on_hover_text(
                "Writes the action's frames (all directions) as RGBA PNGs plus classicanimation.kdl (anchors, frame \
                 numbers, palettes) into a new subfolder.",
            )
            .clicked()
        {
            if let Some(dir) = crate::dialog::pick_folder() {
                app.status_message = match export_classic_frames_folder(app, target, &dir) {
                    Ok(message) => message,
                    Err(error) => format!("Frame export failed: {error:#}"),
                };
            }
        }
        let note = match target {
            ClassicFramesTarget::Uop { .. } => format!("\n{}", crate::ui::UOP_PATCHER_NOTE),
            ClassicFramesTarget::Mul { .. } => String::new(),
        };
        if ui
            .button("Replace frames...")
            .on_hover_text(format!(
                "Encodes a folder written by Export frames (edited; alpha 0 or 255 only) and writes {what}; the opened \
                 files are refused as targets. A UOP entry whose frames per direction change also needs a copy of \
                 the {ANIMATION_SEQUENCE_UOP_NAME} next to the package.{note}"
            ))
            .clicked()
        {
            if let Some(dir) = crate::dialog::pick_folder() {
                app.status_message = match replace_classic_frames(app, target, &dir) {
                    Ok(message) => message,
                    Err(error) => format!("Frame replace failed: {error:#}"),
                };
            }
        }
    });
}

/// The `anim*.idx` / `anim*.mul` pair of MUL file `file_index` (0 is anim.mul) in the Classic client folder.
fn classic_mul_pair(app: &UocfEditorApp, file_index: u8) -> eyre::Result<(PathBuf, PathBuf)> {
    use uocf::classic::anim_frames::folder::FIRST_MUL_FILE;
    let client = app.client_data.as_ref().ok_or_else(|| eyre::eyre!("no Classic client is loaded"))?;
    let stem = match file_index {
        0 => "anim".to_string(),
        index => format!("anim{}", u32::from(index) + u32::from(FIRST_MUL_FILE)),
    };
    let find = |extension: &str| {
        let name = format!("{stem}.{extension}");
        uocf::utils::path::find_file_ignoring_case(&client.path, &name)
            .ok_or_else(|| eyre::eyre!("{name} is not in {}", client.path.display()))
    };
    Ok((find(MUL_INDEX_FILE_FILTER.1[0])?, find(MUL_DATA_FILE_FILTER.1[0])?))
}

/// The MUL index entries of the five directions of `body_id` / `action`.
fn classic_mul_indices(file_index: u8, body_id: u16, action: u16) -> eyre::Result<Vec<(u8, u32)>> {
    use uocf::animation_sequence::STORED_DIRECTION_COUNT;
    (0..STORED_DIRECTION_COUNT as u8)
        .map(|direction| {
            uocf::classic::anim::classic_animation_source_index_from_identity(file_index, body_id, action, direction)
                .map(|index| (direction, index))
                .ok_or_else(|| eyre::eyre!("body {body_id} has no action {action} in the layout of this file"))
        })
        .collect()
}

fn export_classic_frames_folder(app: &UocfEditorApp, target: &ClassicFramesTarget, dir: &Path) -> eyre::Result<String> {
    use uocf::classic::anim_frames::folder::{export_mul_folder, export_uop_folder, FIRST_MUL_FILE};
    let (body_id, action) = target.body_action();
    let output = dir.join(format!("body{body_id:05}_action{action:02}"));
    let sidecar = match target {
        ClassicFramesTarget::Uop { entry, .. } => {
            let payload = animationframe_payload(app, entry)?;
            let package = animationframe_package_path(app, entry)?;
            let frames_per_direction = animationframe_sequence(&package, body_id, action)?.map(|(_, count)| count);
            export_uop_folder(&payload, body_id, action, frames_per_direction, true, &output)?
        }
        ClassicFramesTarget::Mul { file_index, body_id, action } => {
            let (idx, mul) = classic_mul_pair(app, *file_index)?;
            let mut entries = Vec::new();
            for (direction, index) in classic_mul_indices(*file_index, *body_id, *action)? {
                if let Some(payload) = uocf::classic::anim_frames::mul::read_mul_entry(&idx, &mul, index)? {
                    entries.push((direction, payload));
                }
            }
            eyre::ensure!(!entries.is_empty(), "{} has no body {body_id} action {action}", idx.display());
            export_mul_folder(&entries, file_index + FIRST_MUL_FILE, u32::from(*body_id), *action, true, &output)?
        }
    };
    let frames: usize = sidecar.groups.iter().map(|group| group.frames.len()).sum();
    Ok(format!("Exported {frames} frames to {}", output.display()))
}

fn replace_classic_frames(app: &UocfEditorApp, target: &ClassicFramesTarget, dir: &Path) -> eyre::Result<String> {
    use uocf::classic::anim_frames::folder::{import_folder, FrameSource, FIRST_MUL_FILE};

    let imported = import_folder(dir)?;
    let sidecar = &imported.sidecar;
    let (body_id, action) = target.body_action();
    eyre::ensure!(
        (sidecar.body, sidecar.action) == (body_id, action),
        "the folder holds body {} action {}, not body {body_id} action {action}",
        sidecar.body,
        sidecar.action
    );
    let distinct: usize = imported.groups.iter().map(|group| group.report.distinct_colours).max().unwrap_or_default();
    let quantized = if imported.groups.iter().any(|group| group.report.quantized) { ", quantized" } else { "" };
    match target {
        ClassicFramesTarget::Uop { entry, .. } => {
            use uocf::animation_sequence::STORED_DIRECTION_COUNT;
            use uocf::classic::anim_frames::uop::{write_uop_entry, ClassicUopEntry};
            use uocf::enhanced::animationframe::package::write_sequence_frame_count;

            eyre::ensure!(sidecar.source == FrameSource::Uop, "the folder holds anim*.mul frames, not a UOP entry");
            let base = animationframe_payload(app, entry)?;
            let payload = imported.uop_payload(Some(&base))?;
            let frames = ClassicUopEntry::parse(&payload)?.frames.len();
            let package = animationframe_package_path(app, entry)?;
            let per_direction = sidecar.frames_per_direction.or_else(|| {
                let count = u32::try_from(frames).ok()?;
                (count % STORED_DIRECTION_COUNT == 0).then_some(count / STORED_DIRECTION_COUNT)
            });
            let sequence_change = match animationframe_sequence(&package, body_id, action)? {
                Some((path, current)) if per_direction != Some(current) => {
                    let new = per_direction.ok_or_else(|| {
                        eyre::eyre!(
                            "{frames} frames are not a multiple of {STORED_DIRECTION_COUNT} directions: set \
                             frames-per-direction= in the sidecar"
                        )
                    })?;
                    Some((path, current, new))
                }
                _ => None,
            };
            let file_name = package.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(output) = crate::dialog::file_dialog()
                .add_filter(UOP_FILE_FILTER.0, UOP_FILE_FILTER.1)
                .set_file_name(&file_name)
                .save_file()
            else {
                return Ok("Frame replace cancelled".to_string());
            };
            let mut sequence_message = String::new();
            if let Some((sequence_path, current, new)) = sequence_change {
                let Some(sequence_output) = crate::dialog::file_dialog()
                    .set_title(format!("{ANIMATION_SEQUENCE_UOP_NAME}: {current} -> {new} frames per direction"))
                    .add_filter(UOP_FILE_FILTER.0, UOP_FILE_FILTER.1)
                    .set_file_name(ANIMATION_SEQUENCE_UOP_NAME)
                    .save_file()
                else {
                    return Ok("Frame replace cancelled: the frames per direction change and need a sequence copy".to_string());
                };
                let dialect = uocf::enhanced::client_dialect::ClientDialect::of_package_path(&sequence_path);
                write_sequence_frame_count(&sequence_path, &sequence_output, body_id, u32::from(action), new, dialect)?;
                sequence_message = format!("; {current} -> {new} frames per direction in {}", sequence_output.display());
            }
            let change = write_uop_entry(&package, &output, body_id, action, &payload)?;
            Ok(format!(
                "Wrote {} ({frames} frames, {distinct} colours{quantized}, entry {change:?}){sequence_message}",
                output.display()
            ))
        }
        ClassicFramesTarget::Mul { file_index, body_id, action } => {
            use uocf::classic::anim_frames::mul::write_mul_entries;

            let file = file_index + FIRST_MUL_FILE;
            eyre::ensure!(
                sidecar.source == FrameSource::Mul { file },
                "the folder does not hold frames of anim file {file} (file=1 is anim.mul)"
            );
            let indices = classic_mul_indices(*file_index, *body_id, *action)?;
            let entries = imported
                .mul_payloads()?
                .into_iter()
                .map(|(direction, payload)| {
                    let index = indices.iter().find(|(d, _)| *d == direction).map(|(_, index)| *index);
                    index.map(|index| (index, payload)).ok_or_else(|| eyre::eyre!("direction {direction} is past the five"))
                })
                .collect::<eyre::Result<Vec<_>>>()?;
            let (idx, mul) = classic_mul_pair(app, *file_index)?;
            let name = |path: &Path| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(idx_output) = crate::dialog::file_dialog()
                .add_filter(MUL_INDEX_FILE_FILTER.0, MUL_INDEX_FILE_FILTER.1)
                .set_file_name(name(&idx))
                .save_file()
            else {
                return Ok("Frame replace cancelled".to_string());
            };
            let Some(mul_output) = crate::dialog::file_dialog()
                .add_filter(MUL_DATA_FILE_FILTER.0, MUL_DATA_FILE_FILTER.1)
                .set_directory(idx_output.parent().unwrap_or(Path::new("")))
                .set_file_name(name(&mul))
                .save_file()
            else {
                return Ok("Frame replace cancelled".to_string());
            };
            let changes = write_mul_entries(&idx, &mul, &idx_output, &mul_output, &entries)?;
            Ok(format!(
                "Wrote {} and {} ({} directions, {distinct} colours at most{quantized}, entries {changes:?})",
                idx_output.display(),
                mul_output.display(),
                changes.len()
            ))
        }
    }
}

fn animationframe_payload(
    app: &UocfEditorApp,
    entry: &AnimationFrameUopEntry,
) -> eyre::Result<Vec<u8>> {
    let loaded = app
        .uop_cache
        .loaded_uops
        .get(entry.package_index)
        .ok_or_else(|| {
            eyre::eyre!(
                "AnimationFrame package {} is no longer loaded",
                entry.package_index
            )
        })?;
    if loaded
        .package
        .get_file_by_hash(entry.file_hash)
        .is_none()
    {
        return Err(eyre::eyre!(
            "AnimationFrame payload 0x{:016x} not found in {}",
            entry.file_hash,
            loaded.path.display()
        ));
    }
    loaded
        .package
        .unpack_file_by_hash(entry.file_hash)
        .map_err(|error| eyre::eyre!("Failed to unpack AnimationFrame payload: {error}"))?
        .ok_or_else(|| {
            eyre::eyre!(
                "AnimationFrame payload 0x{:016x} not found in {}",
                entry.file_hash,
                loaded.path.display()
            )
        })
}

pub(super) fn animationframe_uop_entries(
    app: &mut UocfEditorApp,
    source: ArtSource,
) -> Option<std::sync::Arc<Vec<AnimationFrameUopEntry>>> {
    let key = (source as u8, app.uop_cache.loaded_uops.len());
    if let Some(entries) = app.animationframe_uop_entries.get(&key) {
        return Some(entries.clone());
    }

    if let Some(rx) = app.animationframe_uop_worker_rx.take() {
        match rx.try_recv() {
            Ok(result) => {
                app.animationframe_uop_worker_key = None;
                for (hash, path) in result.new_dic_entries {
                    app.dictionary.set(hash, path);
                }
                app.animationframe_uop_entries
                    .insert(result.key, std::sync::Arc::new(result.entries));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                app.animationframe_uop_worker_rx = Some(rx);
                return None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.animationframe_uop_worker_key = None;
            }
        }
    }

    if let Some(entries) = app.animationframe_uop_entries.get(&key) {
        return Some(entries.clone());
    }
    if app.animationframe_uop_worker_key == Some(key) {
        return None;
    }

    let cc_path = app.settings.cc_path.clone();
    let ec_path = app.shown_enhanced_folder().map(Path::to_path_buf);
    let is_ec = matches!(source, ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr);
    let is_cc = source == ArtSource::CcUop;
    let sequence_package = if is_ec {
        app.uop_cache.loaded_uops.iter().find_map(|loaded| {
            loaded
                .path
                .file_name()?
                .to_str()?
                .eq_ignore_ascii_case("animationsequence.uop")
                .then(|| loaded.package.clone())
        })
    } else {
        None
    };
    // Pre-build sequence hash→body_id from the dictionary so the scan thread can skip
    // hash generation for already-known body entries.
    let dict_seq_map: std::collections::HashMap<u64, u32> = if is_ec {
        app.dictionary
            .iter_named()
            .filter_map(|(hash, name)| {
                if name.starts_with("data/animationsequence/")
                    || name.starts_with("build/animationsequence/")
                {
                    let stem = name.rsplit('/').next()?;
                    let body_id = stem.strip_suffix(".bin")?.parse::<u32>().ok()?;
                    Some((hash, body_id))
                } else {
                    None
                }
            })
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    // Pre-build CC frame hash→(body_id, action_id) from the dictionary.
    let dict_cc_frame_map: std::collections::HashMap<u64, (u32, u16)> = if is_cc {
        app.dictionary
            .iter_named()
            .filter_map(|(hash, name)| {
                if !name.starts_with("build/animationlegacyframe/") {
                    return None;
                }
                // path: build/animationlegacyframe/{body:06}/{action:02}.bin
                let mut parts = name.splitn(4, '/');
                let _ = parts.next(); // "build"
                let _ = parts.next(); // "animationlegacyframe"
                let body_str = parts.next()?;
                let file_str = parts.next()?;
                let body_id = body_str.parse::<u32>().ok()?;
                let action_id = file_str.strip_suffix(".bin")?.parse::<u16>().ok()?;
                Some((hash, (body_id, action_id)))
            })
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    let packages = app
        .uop_cache
        .loaded_uops
        .iter()
        .enumerate()
        .filter_map(|(package_index, loaded)| {
            animationframe_package_group_id(&loaded.path)?;
            if !animationframe_package_matches_source(
                source,
                &loaded.path,
                cc_path.as_deref(),
                ec_path.as_deref(),
            ) {
                return None;
            }
            Some((package_index, loaded.path.clone(), loaded.package.clone()))
        })
        .collect::<Vec<_>>();
    if packages.is_empty() {
        let entries = std::sync::Arc::new(Vec::new());
        app.animationframe_uop_entries.insert(key, entries.clone());
        return Some(entries);
    }

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (entries, new_dic_entries) = scan_animationframe_uop_packages(
            packages,
            source,
            sequence_package,
            dict_seq_map,
            dict_cc_frame_map,
        );
        let _ = tx.send(AnimationFrameUopScanResult { key, entries, new_dic_entries });
    });
    app.animationframe_uop_worker_rx = Some(rx);
    app.animationframe_uop_worker_key = Some(key);
    None
}

fn animationframe_package_matches_source(
    source: ArtSource,
    path: &Path,
    cc_path: Option<&Path>,
    ec_path: Option<&Path>,
) -> bool {
    match source {
        ArtSource::CcUop => cc_path.is_none_or(|base| path.starts_with(base)),
        ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
            ec_path.is_none_or(|base| path.starts_with(base))
        }
        ArtSource::Mul | ArtSource::Any => false,
    }
}

fn scan_animationframe_uop_packages(
    packages: Vec<(usize, PathBuf, uocf::uop_container::package::UopPackage)>,
    source: ArtSource,
    sequence_package: Option<uocf::uop_container::package::UopPackage>,
    dict_seq_map: std::collections::HashMap<u64, u32>,
    dict_cc_frame_map: std::collections::HashMap<u64, (u32, u16)>,
) -> (Vec<AnimationFrameUopEntry>, Vec<(u64, String)>) {
    if source == ArtSource::CcUop {
        return scan_cc_animationframe_uop_packages(packages, dict_cc_frame_map);
    }
    if matches!(source, ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr) {
        return scan_ec_animationframe_uop_packages(packages, sequence_package, dict_seq_map);
    }

    (Vec::new(), Vec::new())
}

fn scan_cc_animationframe_uop_packages(
    packages: Vec<(usize, PathBuf, uocf::uop_container::package::UopPackage)>,
    dict_frame_map: std::collections::HashMap<u64, (u32, u16)>,
) -> (Vec<AnimationFrameUopEntry>, Vec<(u64, String)>) {
    use uocf::enhanced::animationframe::MAX_BODY_ID;
    use uocf::uop_container::hash::hash_file_name_simd_batch_strs_into;

    const MAX_ACTION_ID_CC: u16 = 100;
    const CC_DIRECTIONS: u8 = 5;

    // Build brute-force reverse map only for body_ids not already in the dict.
    let known_bodies: std::collections::HashSet<u32> = dict_frame_map.values().map(|&(b, _)| b).collect();
    let unknown_bodies: Vec<u32> = (0..MAX_BODY_ID).filter(|b| !known_bodies.contains(b)).collect();

    let mut combined_map: std::collections::HashMap<u64, (u32, u16)> =
        std::collections::HashMap::with_capacity(dict_frame_map.len() + unknown_bodies.len() * MAX_ACTION_ID_CC as usize);
    combined_map.extend(dict_frame_map.iter().map(|(&h, &p)| (h, p)));

    if !unknown_bodies.is_empty() {
        let total = unknown_bodies.len() * MAX_ACTION_ID_CC as usize;
        let paths: Vec<String> = unknown_bodies
            .iter()
            .flat_map(|&body_id| {
                (0..MAX_ACTION_ID_CC).map(move |action_id| AnimationFrameCc::animationframe_path(body_id, action_id))
            })
            .collect();
        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let mut hashes = vec![0u64; total];
        hash_file_name_simd_batch_strs_into(&path_refs, &mut hashes);
        for (i, hash) in hashes.into_iter().enumerate() {
            let body_id = unknown_bodies[i / MAX_ACTION_ID_CC as usize];
            let action_id = (i % MAX_ACTION_ID_CC as usize) as u16;
            combined_map.insert(hash, (body_id, action_id));
        }
    }

    let mut entries = Vec::new();
    let mut new_dic_entries: Vec<(u64, String)> = Vec::new();

    for (package_index, path, package) in &packages {
        let Some(group_id) = animationframe_package_group_id(path) else {
            continue;
        };
        for (file_hash, file) in package.files_by_hash() {
            if !file.has_size() {
                continue;
            }
            let Some(&(body_id, action_id)) = combined_map.get(file_hash) else {
                continue;
            };
            if !dict_frame_map.contains_key(file_hash) {
                new_dic_entries.push((
                    *file_hash,
                    AnimationFrameCc::animationframe_path(body_id, action_id),
                ));
            }
            for direction in 0..CC_DIRECTIONS {
                entries.push(AnimationFrameUopEntry {
                    package_index: *package_index,
                    file_hash: *file_hash,
                    body_id,
                    action_id: Some(action_id),
                    direction: Some(direction),
                    group_id: Some(group_id),
                    source_index: action_id as u32,
                });
            }
        }
    }

    entries.sort_by_key(|entry| {
        (
            entry.body_id,
            entry.action_id.unwrap_or(0),
            entry.direction.unwrap_or(0),
            entry.group_id.unwrap_or(0),
            entry.source_index,
        )
    });
    (entries, new_dic_entries)
}

fn scan_ec_animationframe_uop_packages(
    packages: Vec<(usize, PathBuf, uocf::uop_container::package::UopPackage)>,
    sequence_package: Option<uocf::uop_container::package::UopPackage>,
    dict_seq_map: std::collections::HashMap<u64, u32>,
) -> (Vec<AnimationFrameUopEntry>, Vec<(u64, String)>) {
    let (primary_map, mut new_dic_entries) = sequence_package
        .map(|pkg| build_ec_sequence_hash_map(&pkg, dict_seq_map))
        .unwrap_or_default();

    let mut entries = Vec::new();
    let mut unresolved: Vec<(u64, usize, u8)> = Vec::new();

    for (package_index, path, package) in &packages {
        let Some(group_id) = animationframe_package_group_id(path) else {
            continue;
        };
        for (file_hash, file) in package.files_by_hash() {
            if !file.has_size() {
                continue;
            }
            if let Some(&(body_id, action_id)) = primary_map.get(file_hash) {
                entries.push(AnimationFrameUopEntry {
                    package_index: *package_index,
                    file_hash: *file_hash,
                    body_id,
                    action_id: Some(action_id),
                    direction: None,
                    group_id: Some(group_id),
                    source_index: action_id as u32,
                });
            } else {
                unresolved.push((*file_hash, *package_index, group_id));
            }
        }
    }

    if !unresolved.is_empty() {
        let fallback_map = build_ec_brute_force_hash_map();
        for (file_hash, package_index, group_id) in unresolved {
            let Some(&(body_id, action_id)) = fallback_map.get(&file_hash) else {
                continue;
            };
            entries.push(AnimationFrameUopEntry {
                package_index,
                file_hash,
                body_id,
                action_id: Some(action_id),
                direction: None,
                group_id: Some(group_id),
                source_index: action_id as u32,
            });
            new_dic_entries.push((
                file_hash,
                uocf::enhanced::animationframe::AnimationFrame::animationframe_path(body_id, action_id),
            ));
        }
    }

    entries.sort_by_key(|entry| {
        (
            entry.body_id,
            entry.action_id.unwrap_or(0),
            entry.group_id.unwrap_or(0),
        )
    });
    (entries, new_dic_entries)
}

fn build_ec_sequence_hash_map(
    sequence_package: &uocf::uop_container::package::UopPackage,
    dict_seq_map: std::collections::HashMap<u64, u32>,
) -> (std::collections::HashMap<u64, (u32, u16)>, Vec<(u64, String)>) {
    use std::collections::HashMap;
    use uocf::enhanced::animationframe::{AnimationFrame, MAX_BODY_ID};
    use uocf::uop_container::hash::hash_file_name_simd_batch_strs_into;

    // Collect hashes of sequence files that are not yet in the dictionary.
    // For those already known, use the dict directly — no hashing needed.
    let mut seq_hash_to_body: HashMap<u64, u32> = HashMap::with_capacity(dict_seq_map.len() + MAX_BODY_ID as usize);
    seq_hash_to_body.extend(dict_seq_map.iter().map(|(&h, &b)| (h, b)));

    // Determine which body_ids are not yet covered by the dict so we can
    // batch-hash only those candidate paths.
    let known_bodies: std::collections::HashSet<u32> = dict_seq_map.values().copied().collect();
    let unknown_bodies: Vec<u32> = (0..MAX_BODY_ID)
        .filter(|b| !known_bodies.contains(b))
        .collect();

    if !unknown_bodies.is_empty() {
        let seq_paths: Vec<String> = unknown_bodies.iter().copied().map(AnimationSequence::entry_path).collect();
        let seq_path_refs: Vec<&str> = seq_paths.iter().map(|s| s.as_str()).collect();
        let mut seq_hashes = vec![0u64; seq_path_refs.len()];
        hash_file_name_simd_batch_strs_into(&seq_path_refs, &mut seq_hashes);

        for (&body_id, hash) in unknown_bodies.iter().zip(seq_hashes) {
            seq_hash_to_body.insert(hash, body_id);
        }
    }

    // Iterate the package's actual files — only those that exist — and resolve each to a body_id.
    // Collect new dic entries for sequence files not already in the dict.
    let mut frame_pairs: Vec<(u32, u16)> = Vec::new();
    let mut new_dic_entries: Vec<(u64, String)> = Vec::new();
    for (file_hash, file) in sequence_package.files_by_hash() {
        if !file.has_size() {
            continue;
        }
        let Some(&body_id) = seq_hash_to_body.get(file_hash) else {
            continue;
        };
        let Ok(Some(data)) = sequence_package.unpack_file_by_hash(*file_hash) else {
            continue;
        };
        let Ok(sequence) = AnimationSequence::parse(&data) else {
            continue;
        };
        if !dict_seq_map.contains_key(file_hash) {
            new_dic_entries.push((*file_hash, AnimationSequence::entry_path(body_id)));
        }
        // Candidate frame entries: the plain actions under the sequence's own body (frames are
        // keyed by frame body; the stored id is an equipment wear-check id, not a frame key).
        for (&action_id, action) in &sequence.actions {
            if let (None, Ok(action_id)) = (action.alias, u16::try_from(action_id)) {
                frame_pairs.push((body_id, action_id));
            }
        }
    }

    // Batch-hash all AnimationFrame paths derived from the sequence data.
    // These are also new dic entries (AnimationFrame hashes).
    let frame_paths: Vec<String> = frame_pairs
        .iter()
        .map(|&(body_id, action_id)| AnimationFrame::animationframe_path(body_id, action_id))
        .collect();
    let frame_path_refs: Vec<&str> = frame_paths.iter().map(|s| s.as_str()).collect();
    let mut frame_hashes = vec![0u64; frame_paths.len()];
    hash_file_name_simd_batch_strs_into(&frame_path_refs, &mut frame_hashes);

    let mut map = HashMap::with_capacity(frame_pairs.len());
    for (i, &(body_id, action_id)) in frame_pairs.iter().enumerate() {
        let hash = frame_hashes[i];
        map.insert(hash, (body_id, action_id));
        new_dic_entries.push((hash, frame_paths[i].clone()));
    }
    (map, new_dic_entries)
}

fn build_ec_brute_force_hash_map() -> std::collections::HashMap<u64, (u32, u16)> {
    use uocf::enhanced::animationframe::{AnimationFrame, MAX_BODY_ID, MAX_ACTION_ID_FALLBACK};
    use uocf::uop_container::hash::hash_file_name_simd_batch_strs_into;

    let total = MAX_BODY_ID as usize * MAX_ACTION_ID_FALLBACK as usize;
    let paths: Vec<String> = (0..MAX_BODY_ID)
        .flat_map(|body_id| {
            (0..MAX_ACTION_ID_FALLBACK)
                .map(move |action_id| AnimationFrame::animationframe_path(body_id, action_id))
        })
        .collect();
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut hashes = vec![0u64; total];
    hash_file_name_simd_batch_strs_into(&path_refs, &mut hashes);

    let mut map = std::collections::HashMap::with_capacity(total);
    for (i, hash) in hashes.into_iter().enumerate() {
        let body_id = (i / MAX_ACTION_ID_FALLBACK as usize) as u32;
        let action_id = (i % MAX_ACTION_ID_FALLBACK as usize) as u16;
        map.insert(hash, (body_id, action_id));
    }
    map
}

fn animationframe_package_group_id(path: &Path) -> Option<u8> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let index = file_name
        .strip_prefix("animationframe")?
        .strip_suffix(".uop")?
        .parse::<u8>()
        .ok()?;
    (1..=6).contains(&index).then_some(index - 1)
}

/// Creature placeholder `10000000 + body` of the selected server body, when `tileart.uop` is
/// loaded.
fn selected_body_definition(app: &UocfEditorApp) -> Option<MobileBodyDefinition> {
    // An entry picked in the list is stored under its frames body: no placeholder applies.
    if app.selected_animationframe_file_hash.is_some() {
        return None;
    }
    let entries = app.ec_tileart_entries.as_ref()?;
    let tile_id = MOBILE_DEFINITION_BASE.checked_add(app.selected_anim_id)?;
    let index = entries.binary_search_by_key(&tile_id, |file| file.entry.tile_id).ok()?;
    MobileBodyDefinition::from_tileart_entry(&entries[index].entry)
}

pub(super) fn is_ec_source(source: ArtSource) -> bool {
    matches!(source, ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr)
}

/// Whether two sources read the same animation files: every Enhanced Client source reads the AnimationFrame packages
/// of the Enhanced folder.
fn same_animation_files(a: ArtSource, b: ArtSource) -> bool {
    a == b || (is_ec_source(a) && is_ec_source(b))
}

/// A valid anim file index for `source`: the first loaded MUL file for MUL, the default index otherwise.
fn first_anim_file_index(app: &UocfEditorApp, source: ArtSource) -> u8 {
    let anim_map = app.client_data.as_ref().and_then(|client| client.anim_map.as_ref());
    match (source, anim_map) {
        (ArtSource::Mul, Some(anim_map)) => (0..uocf::classic::anim::MAX_ANIM_FILES)
            .find(|&file_index| anim_map.source_index_count(file_index).is_some())
            .unwrap_or(DEFAULT_ANIM_FILE_INDEX),
        _ => DEFAULT_ANIM_FILE_INDEX,
    }
}

/// Whether an AnimationFrame package of `folder` is loaded.
fn has_animationframe_packages(app: &UocfEditorApp, folder: Option<&Path>) -> bool {
    folder.is_some_and(|folder| {
        app.uop_cache
            .loaded_uops
            .iter()
            .any(|loaded| animationframe_package_group_id(&loaded.path).is_some() && loaded.path.starts_with(folder))
    })
}

/// The source buttons. Every Enhanced-format source of a folder reads its AnimationFrame packages, so the button of the
/// client shown is pressed; a click on it keeps its art source in step: Texture.uop for KR UOP.
fn animation_source_selector(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let cc_path = app.settings.cc_path.clone();
    let selected = match app.selected_legacy_source {
        ArtSource::Mul => Some(ClientSource::Mul),
        ArtSource::CcUop => Some(ClientSource::CcUop),
        ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => app.shown_enhanced,
        ArtSource::Any => None,
    };
    let has_mul = app.client_data.as_ref().is_some_and(|client| client.anim_map.is_some());
    let has_cc_uop = has_animationframe_packages(app, cc_path.as_deref());
    let has_ec_uop = has_animationframe_packages(app, app.shown_enhanced_folder());
    let clicked = source_choice(ui, selected, |source| match source {
        ClientSource::Mul => folder_files_availability(cc_path.as_deref(), CLASSIC_FOLDER, ANIM_MUL_FILES, has_mul),
        ClientSource::CcUop => {
            folder_files_availability(cc_path.as_deref(), CLASSIC_FOLDER, ANIMATION_FRAME_UOP_FILES, has_cc_uop)
        }
        ClientSource::KrUop | ClientSource::EcUop => {
            enhanced_files_availability(app, source, ANIMATION_FRAME_UOP_FILES, &ANIMATIONFRAME_UOP_NAMES, has_ec_uop)
        }
    });
    if let Some(source) = clicked {
        app.show_enhanced_client(source);
        app.selected_legacy_source = match source {
            ClientSource::Mul => ArtSource::Mul,
            ClientSource::CcUop => ArtSource::CcUop,
            ClientSource::KrUop => ArtSource::EcUopKr,
            ClientSource::EcUop => ArtSource::EcUop,
        };
    }
}

/// `AnimationSequence.uop` entry of the selected server body: its placeholder body type, else
/// the body itself.
fn selected_sequence_id(app: &UocfEditorApp) -> u32 {
    selected_body_definition(app).map_or(app.selected_anim_id, |definition| definition.sequence_id)
}

/// Body under which the frames of the selected server body are stored: for EC sources the
/// placeholder appearance (body 0 is tried when it has no frames), else the selected id.
fn selected_frames_body_id(app: &UocfEditorApp) -> u32 {
    if !is_ec_source(app.selected_legacy_source) {
        return app.selected_anim_id;
    }
    selected_body_definition(app).map_or(app.selected_anim_id, |definition| definition.requested_frames_body())
}

fn selected_mul_source_index(app: &UocfEditorApp, body_id: u32) -> eyre::Result<u32> {
    if body_id > u16::MAX as u32 {
        eyre::bail!("Body {} is outside the classic MUL animation layout", body_id);
    }

    uocf::classic::anim::classic_animation_source_index_from_identity(
        app.selected_anim_file_idx,
        body_id as u16,
        app.selected_action_id,
        app.selected_direction,
    )
    .ok_or_else(|| {
        eyre::eyre!(
            "Body {} action {} direction {} is outside classic MUL source {} layout",
            body_id,
            app.selected_action_id,
            app.selected_direction,
            app.selected_anim_file_idx
        )
    })
}

fn animation_source_label(source: ArtSource) -> &'static str {
    match source {
        ArtSource::Mul => "mul",
        ArtSource::CcUop => "cc_uop",
        ArtSource::EcUop => "ec_uop",
        ArtSource::EcUopLegacy => "ec_uop_legacy",
        ArtSource::EcUopKr => "ec_uop_kr",
        ArtSource::Any => "any",
    }
}

#[derive(Clone, Copy)]
enum AnimationPatchExportFormat {
    Vd,
    MichelangeloUop,
}

fn export_selected_animation_patch(
    app: &UocfEditorApp,
    body_id: u32,
    format: AnimationPatchExportFormat,
    output: &Path,
) -> eyre::Result<usize> {
    let patch = selected_animation_patch(app, body_id)?;

    match format {
        AnimationPatchExportFormat::Vd => {
            let entry = patch
                .entries
                .into_iter()
                .next()
                .ok_or_else(|| eyre::eyre!("animation patch export produced no entries"))?;
            VdFile::for_anim(entry.index, entry.extra, entry.data)?.save(output)?;
            Ok(1)
        }
        AnimationPatchExportFormat::MichelangeloUop => {
            let entry_count = patch.entries.len();
            patch.save(output)?;
            Ok(entry_count)
        }
    }
}

fn selected_animation_patch(app: &UocfEditorApp, body_id: u32) -> eyre::Result<MichelangeloPatch> {
    match app.selected_legacy_source {
        ArtSource::Mul => {
            let client = app
                .client_data
                .as_ref()
                .ok_or_else(|| eyre::eyre!("Classic client path is not loaded"))?;
            let (idx_path, mul_path) = anim_pair_paths(&client.path, app.selected_anim_file_idx);
            let source_index = selected_mul_source_index(app, body_id)?;
            export_anim_blocks_from_mul(idx_path, &mul_path, &[source_index as i32], 0, 0)
        }
        ArtSource::CcUop => {
            let payload = selected_cc_animationframe_payload(app, body_id)?;
            Ok(single_payload_patch(body_id as i32, 0, payload))
        }
        ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
            let payload = selected_ec_animationframe_payload(app, body_id)?;
            Ok(single_payload_patch(body_id as i32, 0, payload))
        }
        ArtSource::Any => Err(eyre::eyre!("Select a concrete animation source before exporting")),
    }
}

fn selected_cc_animationframe_payload(app: &UocfEditorApp, body_id: u32) -> eyre::Result<Vec<u8>> {
    let mut group_ids = vec![app.selected_anim_file_idx.min(5)];
    for group_id in 0..=5 {
        if !group_ids.contains(&group_id) {
            group_ids.push(group_id);
        }
    }

    let mut last_error = None;
    for group_id in group_ids {
        let internal_path = AnimationFrameCc::animationframe_path(body_id, u16::from(group_id));
        match animationframe_payload_from_loaded_uops(
            app,
            "AnimationFrame",
            &internal_path,
        ) {
            Ok(payload) => return Ok(payload),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| eyre::eyre!("CC AnimationFrame entry not found")))
}

fn selected_ec_animationframe_payload(app: &UocfEditorApp, body_id: u32) -> eyre::Result<Vec<u8>> {
    let internal_path = format!("data/animationframe/{:06}.bin", body_id);
    animationframe_payload_from_loaded_uops(app, "AnimationFrame", &internal_path)
}

fn animationframe_payload_from_loaded_uops(
    app: &UocfEditorApp,
    package_name_part: &str,
    internal_path: &str,
) -> eyre::Result<Vec<u8>> {
    let hash = hash_file_name_single(internal_path);
    for loaded in &app.uop_cache.loaded_uops {
        if loaded
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .contains(package_name_part)
        {
            if let Some(file) = loaded.package.get_file_by_hash(hash) {
                return file
                    .unpack()
                    .map_err(|error| eyre::eyre!("Failed to unpack {internal_path}: {error}"));
            }
        }
    }

    Err(eyre::eyre!(
        "AnimationFrame entry '{}' (0x{:016x}) not found in loaded UOPs",
        internal_path,
        hash
    ))
}

fn single_payload_patch(index: i32, extra: i32, payload: Vec<u8>) -> MichelangeloPatch {
    MichelangeloPatch {
        entries: vec![MichelangeloPatchEntry::anim(index, extra, payload)],
    }
}

fn anim_pair_paths(client_path: &Path, file_idx: u8) -> (PathBuf, PathBuf) {
    let suffix = if file_idx == 0 {
        String::new()
    } else {
        (file_idx + 1).to_string()
    };
    (
        client_path.join(format!("anim{suffix}.idx")),
        client_path.join(format!("anim{suffix}.mul")),
    )
}
