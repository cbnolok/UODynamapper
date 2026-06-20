use crate::app::{
    AnimationFrameUopEntry, AnimationFrameUopScanResult, ArtSource, UopInspectorApp,
};
use crate::ui::image_export::{export_rgba_png, sanitize_file_stem};
use crate::ui::{arrow_delta, list_sort_controls, move_selection};
use color_eyre::eyre;
use eframe::egui;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Duration;
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::michelangelo_uop_codec::{
    export_anim_blocks_from_mul, MichelangeloPatch, MichelangeloPatchEntry,
};
use uocf::classic::vd_codec::VdFile;
use uocf::uop_container::hash::hash_file_name_single;


#[derive(Clone, Copy)]
struct MulAnimationTreeEntry {
    source_index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MulAnimationEntryKey {
    body_id: u16,
    action_id: u16,
    direction: u8,
    source_index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AnimationFrameEntryKey {
    package_index: usize,
    file_hash: u64,
    body_id: u32,
    action_id: Option<u16>,
    direction: Option<u8>,
    group_id: Option<u8>,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum AnimationTreeOrder {
    BodyId,
    SourceIndex,
}

static ANIMATION_TREE_ORDER: AtomicU8 = AtomicU8::new(0);
static ANIMATION_TREE_COLLAPSE_REVISION: AtomicU64 = AtomicU64::new(0);

const ANIMATION_CONTROLS_PANEL_WIDTH: f32 = 280.0;
const ANIMATION_ACTIONS_PANEL_WIDTH: f32 = 260.0;

pub fn ui_animations(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    egui::Panel::left("anim_controls")
        .resizable(true)
        .default_size(ANIMATION_CONTROLS_PANEL_WIDTH)
        .show_inside(ui, |ui| {
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

            // Resolve redirect if available
            if let Some(client) = &app.client_data {
                if let Some(defs) = &client.anim_defs {
                    let resolved = defs.resolve(body_id);
                    if resolved != body_id {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            format!("Redirected to: {}", resolved),
                        );
                    }
                }
            }

            ui.horizontal(|ui| {
                ui.label("Anim File Index:");
                ui.add(egui::DragValue::new(&mut app.selected_anim_file_idx).range(0..=5));
            });

            ui.separator();
            ui.heading("Source");
            let previous_source = app.selected_legacy_source;
            ui.horizontal(|ui| {
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::Mul, "MUL");
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP");
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::EcUop, "EC UOP");
            });
            if app.selected_legacy_source != previous_source {
                app.selected_animationframe_file_hash = None;
                app.current_frame_idx = 0;
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

            show_animation_navigation(app, ctx, ui);

            if let Some(seq) = &app.selected_anim_sequence {
                ui.separator();
                ui.heading("Logical Sequence");

                egui::Grid::new("anim_seq_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Action:");
                        egui::ComboBox::from_id_salt("action_combo")
                            .selected_text(format!("Action {}", app.selected_action_id))
                            .show_ui(ui, |ui| {
                                let mut action_ids: Vec<_> = seq.actions.keys().collect();
                                action_ids.sort();
                                for &id in action_ids {
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
                            for d in 0..5 {
                                ui.selectable_value(&mut app.selected_direction, d, d.to_string());
                            }
                        });
                        ui.end_row();
                    });
            }
        });

    egui::Panel::right("anim_actions")
        .resizable(true)
        .default_size(ANIMATION_ACTIONS_PANEL_WIDTH)
        .show_inside(ui, |ui| {
            show_animation_action_column(app, ui);
        });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        if app.client_data.is_some() {
            let body_id = app
                .client_data
                .as_ref()
                .and_then(|client| client.anim_defs.as_ref())
                .map(|defs| defs.resolve(app.selected_anim_id))
                .unwrap_or(app.selected_anim_id);

            // Try to load AnimationSequence if not already loaded for this Body ID
            if app.selected_anim_sequence.as_ref().map(|s| s.body_id) != Some(body_id) {
                let mut found_seq = None;
                for loaded in &app.uop_cache.loaded_uops {
                    if loaded
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .contains("AnimationSequence")
                    {
                        let path = format!("data/animationsequence/{:06}.bin", body_id);
                        let hash = uocf::uop_container::hash::hash_file_name_single(&path);
                        if let Some(file) = loaded.package.get_file_by_hash(hash) {
                            if let Ok(data) = file.unpack() {
                                if let Ok(seq) = uocf::animation_sequence::AnimationSequence::parse(
                                    &data, body_id,
                                ) {
                                    found_seq = Some(seq);
                                    break;
                                }
                            }
                        }
                    }
                }
                app.selected_anim_sequence = found_seq;
            }

            let frames_res: eyre::Result<Vec<uocf::classic::anim::AnimFrame>> = match app
                .selected_legacy_source
            {
                ArtSource::Mul => app
                    .client_data
                    .as_ref()
                    .and_then(|client| client.anim_map.as_ref())
                    .ok_or_else(|| eyre::eyre!("Classic animation MUL sources are not loaded"))
                    .and_then(|anim_map| {
                        let source_index = selected_mul_source_index(app, body_id)?;
                        anim_map.decode_animation_index(app.selected_anim_file_idx, source_index)
                    }),
                ArtSource::CcUop => {
                    let entry = selected_cc_animationframe_entry(app, body_id);
                    entry.and_then(|entry| {
                        let data = animationframe_payload(app, &entry)?;
                        AnimationFrameCc::decode_direction(&data, app.selected_direction)
                    })
                }
                ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
                    let entry = selected_ec_animationframe_entry(app, body_id);
                    entry.and_then(|entry| {
                        let data = animationframe_payload(app, &entry)?;
                        decode_amo_animationframe_payload(&data)
                    })
                }
                _ => Err(eyre::eyre!("Source not supported yet")),
            };

            match frames_res {
                Ok(all_frames) => {
                    let logical_frames = if matches!(
                        app.selected_legacy_source,
                        ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr
                    ) && app.selected_animationframe_file_hash.is_none()
                    {
                        if let Some(seq) = &app.selected_anim_sequence {
                            seq.actions
                                .get(&app.selected_action_id)
                                .and_then(|a| a.directions.get(app.selected_direction as usize))
                                .map(|d| &d.frame_indices)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let frames = if let Some(indices) = logical_frames {
                        indices
                            .iter()
                            .filter_map(|&idx| all_frames.get(idx as usize))
                            .cloned()
                            .collect::<Vec<_>>()
                    } else {
                        all_frames
                    };

                    if frames.is_empty() {
                        ui.label("Animation has no frames (or empty logical sequence)");
                    } else {
                        if app.is_playing {
                            let time = ctx.input(|i| i.time);
                            let frame_delay = 0.1 / app.playback_speed as f64;
                            if time - app.last_frame_time > frame_delay {
                                app.current_frame_idx += 1;
                                if app.current_frame_idx >= frames.len() {
                                    if app.loop_animation {
                                        app.current_frame_idx = 0;
                                    } else {
                                        app.current_frame_idx = frames.len() - 1;
                                        app.is_playing = false;
                                    }
                                }
                                app.last_frame_time = time;
                                ctx.request_repaint();
                            }
                        }

                        let frame_idx = app.current_frame_idx.min(frames.len() - 1);
                        let frame = &frames[frame_idx];

                        ui.horizontal(|ui| {
                            ui.label(format!("Frame {} / {}", frame_idx + 1, frames.len()));
                            if logical_frames.is_some() {
                                ui.separator();
                                ui.label(format!(
                                    "Action: {}, Direction: {}",
                                    app.selected_action_id, app.selected_direction
                                ));
                            }
                        });
                        ui.label(format!(
                            "Size: {}x{}, Center: {},{}",
                            frame.width, frame.height, frame.center_x, frame.center_y
                        ));

                        if frame.width > 0 && frame.height > 0 {
                            if ui.button("Export PNG").clicked() {
                                let source = animation_source_label(app.selected_legacy_source);
                                let name = sanitize_file_stem(
                                    &format!(
                                        "{source}_body_{}_action_{}_dir_{}_frame_{}",
                                        body_id,
                                        app.selected_action_id,
                                        app.selected_direction,
                                        frame_idx
                                    ),
                                    "animation_frame",
                                );
                                match export_rgba_png(
                                    format!("{name}.png"),
                                    frame.width as u32,
                                    frame.height as u32,
                                    &frame.data,
                                ) {
                                    Ok(Some(path)) => {
                                        app.status_message =
                                            format!("Exported animation frame to {path}.");
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        app.status_message =
                                            format!("Failed to export animation frame: {e}");
                                    }
                                }
                            }

                            let key = if app.selected_legacy_source == ArtSource::Mul {
                                let source_index =
                                    selected_mul_source_index(app, body_id).unwrap_or(0);
                                0xAC000000
                                    | (app.selected_anim_file_idx as u64) << 48
                                    | (source_index as u64) << 16
                                    | frame_idx as u64
                            } else if logical_frames.is_some() {
                                0xAB000000
                                    | (app.selected_legacy_source as u64) << 48
                                    | (body_id as u64) << 32
                                    | (app.selected_action_id as u64) << 16
                                    | (app.selected_direction as u64) << 8
                                    | frame_idx as u64
                            } else {
                                0xAA000000
                                    | (app.selected_legacy_source as u64) << 32
                                    | (body_id as u64) << 16
                                    | frame_idx as u64
                            };

                            let handle = app
                                .texture_previews
                                .entry(key)
                                .or_insert_with(|| {
                                    let image = egui::ColorImage::from_rgba_unmultiplied(
                                        [frame.width as usize, frame.height as usize],
                                        &frame.data[..],
                                    );
                                    ctx.load_texture(
                                        format!(
                                            "anim_{}_{}_{}",
                                            body_id, app.selected_action_id, frame_idx
                                        ),
                                        image,
                                        Default::default(),
                                    )
                                })
                                .clone();
                            app.register_current_image_preview(
                                key,
                                format!(
                                    "animation {} action {} frame {}",
                                    body_id, app.selected_action_id, frame_idx
                                ),
                                frame.width as u32,
                                frame.height as u32,
                                &frame.data,
                            );

                            ui.image(&handle);
                        } else {
                            ui.label("(Empty frame)");
                        }
                    }
                }
                Err(e) => {
                    ui.label(format!("Error loading animation: {}", e));
                }
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Load client data first");
            });
        }
    });
}

fn show_animation_action_column(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.heading("Animation Actions");
    ui.separator();

    if app.selected_legacy_source != ArtSource::Any {
        let export_context = app.client_data.as_ref().map(|client| {
            client
                .anim_defs
                .as_ref()
                .map(|defs| defs.resolve(app.selected_anim_id))
                .unwrap_or(app.selected_anim_id)
        });

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

fn show_animation_navigation(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    ui.separator();
    ui.heading("Animation Tree");
    let mut tree_order = load_animation_tree_order();
    ui.horizontal(|ui| {
        if app.selected_legacy_source == ArtSource::Mul {
            egui::ComboBox::from_id_salt("uocf_animation_tree_order")
                .selected_text(match tree_order {
                    AnimationTreeOrder::BodyId => "Body ID",
                    AnimationTreeOrder::SourceIndex => "Source Index",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut tree_order,
                        AnimationTreeOrder::BodyId,
                        "Body ID",
                    );
                    ui.selectable_value(
                        &mut tree_order,
                        AnimationTreeOrder::SourceIndex,
                        "Source Index",
                    );
                });
            store_animation_tree_order(tree_order);
        }
        if ui.button("Collapse All").clicked() {
            ANIMATION_TREE_COLLAPSE_REVISION.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(
            app.selected_legacy_source,
            ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr
        ) && ui.button("Rescan Animations").clicked()
        {
            let key = (app.selected_legacy_source as u8, app.uop_cache.loaded_uops.len());
            app.animationframe_uop_entries.remove(&key);
            app.animationframe_uop_frame_counts.clear();
            app.animationframe_uop_worker_rx = None;
            app.animationframe_uop_worker_key = None;
        }
    });
    let sort = list_sort_controls(ui, "animation_tree_entries", &["ID/Index"], 0);
    let collapse_revision = ANIMATION_TREE_COLLAPSE_REVISION.load(Ordering::Relaxed);

    match app.selected_legacy_source {
        ArtSource::Mul => {
            show_mul_animation_tree(
                app,
                ctx,
                ui,
                tree_order,
                sort.ordering(),
                collapse_revision,
                ui.available_height(),
            );
        }
        ArtSource::CcUop | ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => {
            let has_sequence = app.selected_anim_sequence.is_some();
            let tree_height = if has_sequence {
                ui.available_height() * 0.6
            } else {
                ui.available_height()
            };
            show_uop_animationframe_tree(app, ctx, ui, sort.ordering(), collapse_revision, tree_height);
            if has_sequence {
                ui.separator();
                ui.heading("Animation Sequence");
                show_sequence_animation_tree(app, ctx, ui, sort.ordering(), collapse_revision, ui.available_height());
            }
        }
        ArtSource::Any => {
            ui.label("Select an animation source to browse.");
        }
    }
}

fn load_animation_tree_order() -> AnimationTreeOrder {
    match ANIMATION_TREE_ORDER.load(Ordering::Relaxed) {
        1 => AnimationTreeOrder::SourceIndex,
        _ => AnimationTreeOrder::BodyId,
    }
}

fn store_animation_tree_order(order: AnimationTreeOrder) {
    ANIMATION_TREE_ORDER.store(
        match order {
            AnimationTreeOrder::BodyId => 0,
            AnimationTreeOrder::SourceIndex => 1,
        },
        Ordering::Relaxed,
    );
}

fn show_mul_animation_tree(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    tree_order: AnimationTreeOrder,
    ordering: Option<bool>,
    collapse_revision: u64,
    max_height: f32,
) {
    let Some(anim_map) = app
        .client_data
        .as_ref()
        .and_then(|client| client.anim_map.clone())
    else {
        ui.label("Classic animation MUL sources are not loaded.");
        return;
    };

    let file_index = app.selected_anim_file_idx;
    let Some(source_index_count) = anim_map.source_index_count(file_index) else {
        ui.label(format!("Animation source {} is not loaded.", file_index));
        return;
    };

    ui.label("Filter:");
    let filter_response = ui.text_edit_singleline(&mut app.search_query);
    let query = app.search_query.to_ascii_lowercase();

    let mut tree = BTreeMap::<u16, BTreeMap<u16, BTreeMap<u8, MulAnimationTreeEntry>>>::new();
    let mut source_tree = BTreeMap::<u32, (u16, u16, u8, MulAnimationTreeEntry)>::new();
    for source_index in 0..source_index_count {
        let source_index = source_index as u32;
        if !anim_map.has_anim(file_index, source_index) {
            continue;
        }
        let identity = uocf::classic::anim::classic_animation_identity_from_source_index(
            file_index,
            source_index,
        );
        if identity.flags != 0 {
            continue;
        }
        let search_label = format!(
            "body {} action {} direction {} index {}",
            identity.body_id, identity.action_id, identity.direction, source_index
        );
        if !query.is_empty() && !search_label.contains(&query) {
            continue;
        }
        tree.entry(identity.body_id)
            .or_default()
            .entry(identity.action_id)
            .or_default()
            .insert(identity.direction, MulAnimationTreeEntry { source_index });
        source_tree.insert(
            source_index,
            (
                identity.body_id,
                identity.action_id,
                identity.direction,
                MulAnimationTreeEntry { source_index },
            ),
        );
    }

    let visible_keys = mul_animation_entry_keys(tree_order, ordering, &tree, &source_tree);
    let selected_key = selected_mul_animation_entry_key(app, &visible_keys);
    let keyboard_moved = if let Some(delta) = arrow_delta(ui, filter_response.has_focus()) {
        if let Some(key) = move_selection(&visible_keys, selected_key, delta) {
            select_mul_animation_entry_key(app, ctx, key);
        }
        true
    } else {
        false
    };

    let scroll_width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("mul_animation_tree")
        .auto_shrink([false, false])
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(scroll_width);
            let allow_default_open = collapse_revision == 0;
            let default_open = allow_default_open && !query.is_empty();
            match tree_order {
                AnimationTreeOrder::BodyId => {
                    if tree.is_empty() {
                        ui.label("No animations match the current source/filter.");
                    }
                    let mut bodies = tree.into_iter().collect::<Vec<_>>();
                    if ordering == Some(true) {
                        bodies.reverse();
                    }
                    for (body_id, actions) in bodies {
                        egui::CollapsingHeader::new(format!("Body {}", body_id))
                            .id_salt((
                                "uocf_anim_body",
                                collapse_revision,
                                file_index,
                                body_id,
                            ))
                            .default_open(
                                default_open
                                    || (allow_default_open
                                        && app.selected_anim_id == u32::from(body_id)),
                            )
                            .show(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        app.selected_anim_id == u32::from(body_id),
                                        "Select body",
                                    )
                                    .clicked()
                                {
                                    app.selected_anim_id = u32::from(body_id);
                                    app.current_frame_idx = 0;
                                    app.last_frame_time = ctx.input(|input| input.time);
                                }
                                for (action_id, directions) in actions {
                                    egui::CollapsingHeader::new(format!("Action {}", action_id))
                                        .id_salt((
                                            "uocf_anim_action",
                                            collapse_revision,
                                            file_index,
                                            body_id,
                                            action_id,
                                        ))
                                        .default_open(
                                            default_open
                                                || (allow_default_open
                                                    && app.selected_anim_id == u32::from(body_id)
                                                    && app.selected_action_id == action_id),
                                        )
                                        .show(ui, |ui| {
                                            for (direction, entry) in directions {
                                                show_mul_direction_tree(
                                                    app,
                                                    ctx,
                                                    ui,
                                                    &anim_map,
                                                    file_index,
                                                    body_id,
                                                    action_id,
                                                    direction,
                                                    entry,
                                                    default_open,
                                                    collapse_revision,
                                                    keyboard_moved,
                                                );
                                            }
                                        });
                                }
                            });
                    }
                }
                AnimationTreeOrder::SourceIndex => {
                    if source_tree.is_empty() {
                        ui.label("No animations match the current source/filter.");
                    }
                    let mut sources = source_tree.into_iter().collect::<Vec<_>>();
                    if ordering == Some(true) {
                        sources.reverse();
                    }
                    for (source_index, (body_id, action_id, direction, entry)) in sources {
                        egui::CollapsingHeader::new(format!(
                            "Index {}: Body {} / Action {} / Direction {}",
                            source_index, body_id, action_id, direction
                        ))
                        .id_salt((
                            "uocf_anim_source",
                            collapse_revision,
                            file_index,
                            source_index,
                        ))
                        .default_open(
                            default_open
                                || (allow_default_open
                                    && app.selected_anim_id == u32::from(body_id)
                                    && app.selected_action_id == action_id
                                    && app.selected_direction == direction),
                        )
                        .show(ui, |ui| {
                            show_mul_frame_list(
                                app, ctx, ui, &anim_map, file_index, body_id, action_id,
                                direction, entry.source_index, keyboard_moved,
                            );
                        });
                    }
                }
            }
        });
}

fn show_mul_direction_tree(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    anim_map: &uocf::classic::anim::AnimMap,
    file_index: u8,
    body_id: u16,
    action_id: u16,
    direction: u8,
    entry: MulAnimationTreeEntry,
    default_open: bool,
    collapse_revision: u64,
    keyboard_moved: bool,
) {
    egui::CollapsingHeader::new(format!(
        "Direction {} (idx {})",
        direction, entry.source_index
    ))
    .id_salt((
        "uocf_anim_direction",
        collapse_revision,
        file_index,
        body_id,
        action_id,
        direction,
    ))
    .default_open(
        default_open
            || (collapse_revision == 0
                && app.selected_anim_id == u32::from(body_id)
                && app.selected_action_id == action_id
                && app.selected_direction == direction),
    )
    .show(ui, |ui| {
        show_mul_frame_list(
            app,
            ctx,
            ui,
            anim_map,
            file_index,
            body_id,
            action_id,
            direction,
            entry.source_index,
            keyboard_moved,
        );
    });
}

fn mul_animation_entry_keys(
    tree_order: AnimationTreeOrder,
    ordering: Option<bool>,
    tree: &BTreeMap<u16, BTreeMap<u16, BTreeMap<u8, MulAnimationTreeEntry>>>,
    source_tree: &BTreeMap<u32, (u16, u16, u8, MulAnimationTreeEntry)>,
) -> Vec<MulAnimationEntryKey> {
    let mut keys = Vec::new();
    match tree_order {
        AnimationTreeOrder::BodyId => {
            for (body_id, actions) in tree {
                for (action_id, directions) in actions {
                    for (direction, entry) in directions {
                        keys.push(MulAnimationEntryKey {
                            body_id: *body_id,
                            action_id: *action_id,
                            direction: *direction,
                            source_index: entry.source_index,
                        });
                    }
                }
            }
        }
        AnimationTreeOrder::SourceIndex => {
            for (source_index, (body_id, action_id, direction, _entry)) in source_tree {
                keys.push(MulAnimationEntryKey {
                    body_id: *body_id,
                    action_id: *action_id,
                    direction: *direction,
                    source_index: *source_index,
                });
            }
        }
    }
    if ordering == Some(true) {
        keys.reverse();
    }
    keys
}

fn selected_mul_animation_entry_key(
    app: &UopInspectorApp,
    visible_keys: &[MulAnimationEntryKey],
) -> Option<MulAnimationEntryKey> {
    visible_keys
        .iter()
        .copied()
        .find(|key| {
            app.selected_anim_id == u32::from(key.body_id)
                && app.selected_action_id == key.action_id
                && app.selected_direction == key.direction
        })
}

fn select_mul_animation_entry_key(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    key: MulAnimationEntryKey,
) {
    app.selected_anim_id = u32::from(key.body_id);
    app.selected_action_id = key.action_id;
    app.selected_direction = key.direction;
    app.current_frame_idx = 0;
    app.last_frame_time = ctx.input(|input| input.time);
}

fn show_mul_frame_list(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    anim_map: &uocf::classic::anim::AnimMap,
    file_index: u8,
    body_id: u16,
    action_id: u16,
    direction: u8,
    source_index: u32,
    keyboard_moved: bool,
) {
    match anim_map.decode_animation_index_metadata(file_index, source_index) {
        Ok(frames) => {
            if frames.is_empty() {
                ui.label("No frames.");
            }
            for frame_index in 0..frames.len() {
                let selected = app.selected_anim_id == u32::from(body_id)
                    && app.selected_action_id == action_id
                    && app.selected_direction == direction
                    && app.current_frame_idx == frame_index;
                let response = ui.selectable_label(selected, format!("Frame {}", frame_index));
                if keyboard_moved && selected {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
                if response.clicked() {
                    app.selected_anim_id = u32::from(body_id);
                    app.selected_action_id = action_id;
                    app.selected_direction = direction;
                    app.current_frame_idx = frame_index;
                    app.last_frame_time = ctx.input(|input| input.time);
                }
            }
        }
        Err(error) => {
            ui.label(format!("Unable to read frames: {error}"));
        }
    }
}

fn show_uop_animationframe_tree(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    ordering: Option<bool>,
    collapse_revision: u64,
    max_height: f32,
) {
    ui.label("Filter:");
    let filter_response = ui.text_edit_singleline(&mut app.search_query);
    let query = app.search_query.to_ascii_lowercase();
    let Some(entries) =
        collect_uop_animationframe_tree_entries(app, app.selected_legacy_source, &query)
    else {
        ui.label("Scanning AnimationFrame UOP metadata...");
        ctx.request_repaint_after(Duration::from_millis(100));
        return;
    };

    let mut bodies = entries.into_iter().collect::<Vec<_>>();
    if ordering == Some(true) {
        bodies.reverse();
    }
    let visible_keys = animationframe_entry_keys(&bodies, app.selected_legacy_source);
    let selected_key = selected_animationframe_entry_key(app, &visible_keys);
    let keyboard_moved = if let Some(delta) = arrow_delta(ui, filter_response.has_focus()) {
        if let Some(key) = move_selection(&visible_keys, selected_key, delta) {
            select_animationframe_entry_key(app, ctx, key);
        }
        true
    } else {
        false
    };

    let scroll_width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt(match app.selected_legacy_source {
            ArtSource::CcUop => "cc_animationframe_tree",
            ArtSource::EcUop | ArtSource::EcUopLegacy | ArtSource::EcUopKr => "ec_animationframe_tree",
            _ => "animationframe_tree",
        })
        .auto_shrink([false, false])
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(scroll_width);
            if bodies.is_empty() {
                ui.label("No AnimationFrame entries match the current source/filter.");
                return;
            }

            for (body_id, body_entries) in bodies {
                let selected_body = app.selected_anim_id == body_id;
                egui::CollapsingHeader::new(format!("Body {}", body_id))
                    .id_salt((
                        "uocf_animationframe_body",
                        collapse_revision,
                        app.selected_legacy_source as u8,
                        body_id,
                    ))
                    .default_open(collapse_revision == 0 && selected_body)
                    .show(ui, |ui| {
                        if ui.selectable_label(selected_body, "Select body").clicked() {
                            app.selected_anim_id = body_id;
                            app.selected_animationframe_file_hash = None;
                            app.current_frame_idx = 0;
                            app.last_frame_time = ctx.input(|input| input.time);
                        }

                        if app.selected_legacy_source == ArtSource::CcUop {
                            let mut actions =
                                BTreeMap::<u16, BTreeMap<u8, Vec<AnimationFrameUopEntry>>>::new();
                            for entry in body_entries {
                                if let (Some(action_id), Some(direction)) =
                                    (entry.action_id, entry.direction)
                                {
                                    actions
                                        .entry(action_id)
                                        .or_default()
                                        .entry(direction)
                                        .or_default()
                                        .push(entry);
                                }
                            }
                            for (action_id, directions) in actions {
                                egui::CollapsingHeader::new(format!("Action {}", action_id))
                                    .id_salt((
                                        "uocf_animationframe_action",
                                        collapse_revision,
                                        body_id,
                                        action_id,
                                    ))
                                    .default_open(
                                        collapse_revision == 0
                                            && app.selected_anim_id == body_id
                                            && app.selected_action_id == action_id,
                                    )
                                    .show(ui, |ui| {
                                        for (direction, entries) in directions {
                                            egui::CollapsingHeader::new(format!(
                                                "Direction {}",
                                                direction
                                            ))
                                                .id_salt((
                                                    "uocf_animationframe_direction",
                                                    collapse_revision,
                                                    body_id,
                                                    action_id,
                                                    direction,
                                                ))
                                                .default_open(
                                                    collapse_revision == 0
                                                        && app.selected_anim_id == body_id
                                                        && app.selected_action_id == action_id
                                                        && app.selected_direction == direction,
                                                )
                                                .show(ui, |ui| {
                                                    for entry in entries {
                                                        show_uop_animationframe_entry_frames(
                                                            app, ctx, ui, &entry, body_id, action_id,
                                                            direction, keyboard_moved,
                                                        );
                                                    }
                                                });
                                        }
                                    });
                            }
                        } else {
                            let mut actions = BTreeMap::<u16, Vec<AnimationFrameUopEntry>>::new();
                            let mut direct_entries = Vec::new();
                            for entry in body_entries {
                                if let Some(action_id) = entry.action_id {
                                    actions.entry(action_id).or_default().push(entry);
                                } else {
                                    direct_entries.push(entry);
                                }
                            }
                            for (action_id, entries) in actions {
                                egui::CollapsingHeader::new(format!("Action {}", action_id))
                                    .id_salt((
                                        "uocf_ec_animationframe_action",
                                        collapse_revision,
                                        body_id,
                                        action_id,
                                    ))
                                    .default_open(
                                        collapse_revision == 0
                                            && app.selected_anim_id == body_id
                                            && app.selected_action_id == action_id,
                                    )
                                    .show(ui, |ui| {
                                        for entry in entries {
                                            show_ec_uop_animationframe_entry_frames(
                                                app, ctx, ui, &entry, body_id, keyboard_moved,
                                            );
                                        }
                                    });
                            }
                            for entry in direct_entries {
                                show_ec_uop_animationframe_entry_frames(
                                    app,
                                    ctx,
                                    ui,
                                    &entry,
                                    body_id,
                                    keyboard_moved,
                                );
                            }
                        }
                    });
            }
        });
}

fn collect_uop_animationframe_tree_entries(
    app: &mut UopInspectorApp,
    source: ArtSource,
    query: &str,
) -> Option<BTreeMap<u32, Vec<AnimationFrameUopEntry>>> {
    let mut entries = BTreeMap::<u32, Vec<AnimationFrameUopEntry>>::new();
    for entry in animationframe_uop_entries(app, source)?.iter().cloned() {
        let body_id = entry.body_id;
        if !query.is_empty() && !format!("body {body_id}").contains(query) {
            continue;
        }

        entries.entry(body_id).or_default().push(entry);
    }
    Some(entries)
}

fn animationframe_entry_keys(
    bodies: &[(u32, Vec<AnimationFrameUopEntry>)],
    source: ArtSource,
) -> Vec<AnimationFrameEntryKey> {
    let mut keys = Vec::new();
    for (body_id, body_entries) in bodies {
        if source == ArtSource::CcUop {
            let mut actions =
                BTreeMap::<u16, BTreeMap<u8, Vec<&AnimationFrameUopEntry>>>::new();
            for entry in body_entries {
                if let (Some(action_id), Some(direction)) = (entry.action_id, entry.direction) {
                    actions
                        .entry(action_id)
                        .or_default()
                        .entry(direction)
                        .or_default()
                        .push(entry);
                }
            }
            for (_action_id, directions) in actions {
                for (_direction, entries) in directions {
                    keys.extend(
                        entries
                            .into_iter()
                            .map(|entry| animationframe_entry_key(entry, *body_id)),
                    );
                }
            }
        } else {
            let mut actions = BTreeMap::<u16, Vec<&AnimationFrameUopEntry>>::new();
            let mut direct_entries = Vec::new();
            for entry in body_entries {
                if let Some(action_id) = entry.action_id {
                    actions.entry(action_id).or_default().push(entry);
                } else {
                    direct_entries.push(entry);
                }
            }
            for (_action_id, entries) in actions {
                keys.extend(
                    entries
                        .into_iter()
                        .map(|entry| animationframe_entry_key(entry, *body_id)),
                );
            }
            keys.extend(
                direct_entries
                    .into_iter()
                    .map(|entry| animationframe_entry_key(entry, *body_id)),
            );
        }
    }
    keys
}

fn animationframe_entry_key(entry: &AnimationFrameUopEntry, body_id: u32) -> AnimationFrameEntryKey {
    AnimationFrameEntryKey {
        package_index: entry.package_index,
        file_hash: entry.file_hash,
        body_id,
        action_id: entry.action_id,
        direction: entry.direction,
        group_id: entry.group_id,
    }
}

fn selected_animationframe_entry_key(
    app: &UopInspectorApp,
    visible_keys: &[AnimationFrameEntryKey],
) -> Option<AnimationFrameEntryKey> {
    visible_keys
        .iter()
        .copied()
        .find(|key| animationframe_entry_key_is_selected(app, *key))
}

fn animationframe_entry_key_is_selected(app: &UopInspectorApp, key: AnimationFrameEntryKey) -> bool {
    app.selected_anim_id == key.body_id
        && app.selected_animationframe_file_hash == Some(key.file_hash)
        && key
            .action_id
            .is_none_or(|action_id| app.selected_action_id == action_id)
        && key
            .direction
            .is_none_or(|direction| app.selected_direction == direction)
        && key
            .group_id
            .is_none_or(|group_id| app.selected_anim_file_idx == group_id)
}

fn select_animationframe_entry_key(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    key: AnimationFrameEntryKey,
) {
    app.selected_anim_id = key.body_id;
    if let Some(action_id) = key.action_id {
        app.selected_action_id = action_id;
    }
    if let Some(direction) = key.direction {
        app.selected_direction = direction;
    }
    app.selected_animationframe_file_hash = Some(key.file_hash);
    if let Some(group_id) = key.group_id {
        app.selected_anim_file_idx = group_id;
    }
    app.current_frame_idx = 0;
    app.last_frame_time = ctx.input(|input| input.time);
}

fn show_uop_animationframe_entry_frames(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &AnimationFrameUopEntry,
    body_id: u32,
    action_id: u16,
    direction: u8,
    keyboard_moved: bool,
) {
    let frame_count = if entry.frame_count == 0 {
        cc_direction_frame_count(app, entry).unwrap_or(0)
    } else {
        entry.frame_count
    };
    let label = match entry.group_id {
        Some(group_id) => format!(
            "Group {group_id:02} / idx {}: {} frames",
            entry.source_index,
            if frame_count == 0 {
                "unknown".to_string()
            } else {
                frame_count.to_string()
            }
        ),
        None => format!(
            "AnimationFrame: {} frames",
            if frame_count == 0 {
                "unknown".to_string()
            } else {
                frame_count.to_string()
            }
        ),
    };

    egui::CollapsingHeader::new(label)
        .id_salt((
            "uocf_animationframe_entry",
            entry.package_index,
            entry.file_hash,
            body_id,
            action_id,
            direction,
        ))
        .default_open(
            app.selected_anim_id == body_id
                && app.selected_action_id == action_id
                && app.selected_direction == direction
                && app.selected_animationframe_file_hash.is_none_or(|hash| {
                    hash == entry.file_hash
                })
                && entry.group_id.is_none_or(|group_id| {
                    app.selected_anim_file_idx == group_id
                }),
        )
        .show(ui, |ui| {
            let selected_entry = app.selected_anim_id == body_id
                && app.selected_action_id == action_id
                && app.selected_direction == direction
                && app.selected_animationframe_file_hash == Some(entry.file_hash);
            let response = ui.selectable_label(
                selected_entry,
                "Select direction",
            );
            if keyboard_moved && selected_entry {
                response.scroll_to_me(Some(egui::Align::Center));
            }
            if response.clicked() {
                select_animationframe_entry_key(
                    app,
                    ctx,
                    animationframe_entry_key(entry, body_id),
                );
            }

            if frame_count == 0 {
                ui.label("Frame count unavailable until payload is decoded.");
            }
            for frame_index in 0..frame_count {
                let selected = app.selected_anim_id == body_id
                    && app.selected_action_id == action_id
                    && app.selected_direction == direction
                    && app.current_frame_idx == frame_index
                    && app.selected_animationframe_file_hash.is_none_or(|hash| {
                        hash == entry.file_hash
                    })
                    && entry.group_id.is_none_or(|group_id| {
                        app.selected_anim_file_idx == group_id
                    });
                if ui
                    .selectable_label(selected, format!("Frame {}", frame_index))
                    .clicked()
                {
                    app.selected_anim_id = body_id;
                    app.selected_action_id = action_id;
                    app.selected_direction = direction;
                    app.selected_animationframe_file_hash = Some(entry.file_hash);
                    if let Some(group_id) = entry.group_id {
                        app.selected_anim_file_idx = group_id;
                    }
                    app.current_frame_idx = frame_index;
                    app.last_frame_time = ctx.input(|input| input.time);
                }
            }
        });
}

fn show_ec_uop_animationframe_entry_frames(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    entry: &AnimationFrameUopEntry,
    body_id: u32,
    keyboard_moved: bool,
) {
    let selected_entry = app.selected_anim_id == body_id
        && app.selected_animationframe_file_hash == Some(entry.file_hash);
    let frame_count = if entry.frame_count == 0 {
        ec_animationframe_frame_count(app, entry).unwrap_or(0)
    } else {
        entry.frame_count
    };
    let frame_count_label = if frame_count == 0 {
        "unknown".to_string()
    } else {
        frame_count.to_string()
    };
    let label = match entry.group_id {
        Some(group_id) => {
            if let Some(block_index) = entry.block_index {
                format!(
                    "Group {group_id:02} / block {} file {}: {} frames",
                    block_index, entry.source_index, frame_count_label
                )
            } else {
                format!(
                    "Group {group_id:02} / 0x{:016X}: {} frames",
                    entry.file_hash, frame_count_label
                )
            }
        }
        None => format!(
            "AnimationFrame 0x{:016X}: {} frames",
            entry.file_hash, frame_count_label
        ),
    };

    egui::CollapsingHeader::new(label)
        .id_salt((
            "uocf_ec_animationframe_entry",
            entry.package_index,
            entry.file_hash,
            body_id,
        ))
        .default_open(selected_entry)
        .show(ui, |ui| {
            let response = ui.selectable_label(selected_entry, "Select block");
            if keyboard_moved && selected_entry {
                response.scroll_to_me(Some(egui::Align::Center));
            }
            if response.clicked() {
                select_animationframe_entry_key(
                    app,
                    ctx,
                    animationframe_entry_key(entry, body_id),
                );
            }

            if frame_count == 0 {
                ui.label("Frame count unavailable until payload is decoded.");
            }
            for frame_index in 0..frame_count {
                let selected = selected_entry && app.current_frame_idx == frame_index;
                if ui
                    .selectable_label(selected, format!("Frame {}", frame_index))
                    .clicked()
                {
                    app.selected_anim_id = body_id;
                    if let Some(action_id) = entry.action_id {
                        app.selected_action_id = action_id;
                    }
                    app.selected_animationframe_file_hash = Some(entry.file_hash);
                    if let Some(group_id) = entry.group_id {
                        app.selected_anim_file_idx = group_id;
                    }
                    app.current_frame_idx = frame_index;
                    app.last_frame_time = ctx.input(|input| input.time);
                }
            }
        });
}

fn selected_cc_animationframe_entry(
    app: &mut UopInspectorApp,
    body_id: u32,
) -> eyre::Result<AnimationFrameUopEntry> {
    let entries = animationframe_uop_entries(app, ArtSource::CcUop)
        .ok_or_else(|| eyre::eyre!("Scanning CC AnimationFrame UOP metadata"))?;
    entries
        .iter()
        .find(|entry| {
            app.selected_animationframe_file_hash == Some(entry.file_hash)
                && entry.body_id == body_id
                && entry.action_id == Some(app.selected_action_id)
                && entry.direction == Some(app.selected_direction)
        })
        .or_else(|| {
            entries.iter().find(|entry| {
                app.selected_animationframe_file_hash == Some(entry.file_hash)
                    && entry.body_id == body_id
            })
        })
        .or_else(|| {
            entries.iter().find(|entry| {
                entry.body_id == body_id
                    && entry.action_id == Some(app.selected_action_id)
                    && entry.direction == Some(app.selected_direction)
                    && entry.group_id == Some(app.selected_anim_file_idx)
            })
        })
        .or_else(|| {
            entries.iter().find(|entry| {
                entry.body_id == body_id
                    && entry.action_id == Some(app.selected_action_id)
                    && entry.direction == Some(app.selected_direction)
            })
        })
        .cloned()
        .ok_or_else(|| {
            eyre::eyre!(
                "Animation body {} action {} direction {} not found in loaded CC AnimationFrame UOPs",
                body_id,
                app.selected_action_id,
                app.selected_direction
            )
        })
}

fn selected_ec_animationframe_entry(
    app: &mut UopInspectorApp,
    body_id: u32,
) -> eyre::Result<AnimationFrameUopEntry> {
    let source = app.selected_legacy_source;
    let entries = animationframe_uop_entries(app, source)
        .ok_or_else(|| eyre::eyre!("Scanning EC AnimationFrame UOP metadata"))?;
    entries
        .iter()
        .find(|entry| {
            app.selected_animationframe_file_hash == Some(entry.file_hash)
                && entry.body_id == body_id
        })
        .or_else(|| {
            entries.iter().find(|entry| {
                entry.body_id == body_id && entry.group_id == Some(app.selected_anim_file_idx)
            })
        })
        .or_else(|| entries.iter().find(|entry| entry.body_id == body_id))
        .cloned()
        .ok_or_else(|| {
            eyre::eyre!(
                "Animation body {} not found in loaded EC AnimationFrame UOPs",
                body_id
            )
        })
}

fn animationframe_payload(
    app: &UopInspectorApp,
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

fn decode_amo_animationframe_payload(data: &[u8]) -> eyre::Result<Vec<uocf::classic::anim::AnimFrame>> {
    let animation = uocf::enhanced::animationframe::AnimationFrame::load(data)?;
    let mut decoded_frames = Vec::with_capacity(animation.frames.len());
    for entry in &animation.frames {
        if let Ok(decoded) = animation.decode_frame(entry) {
            decoded_frames.push(uocf::classic::anim::AnimFrame {
                width: decoded.width,
                height: decoded.height,
                center_x: decoded.center_x,
                center_y: decoded.center_y,
                data: decoded.data,
            });
        }
    }
    Ok(decoded_frames)
}

fn cc_direction_frame_count(app: &mut UopInspectorApp, entry: &AnimationFrameUopEntry) -> Option<usize> {
    let direction = entry.direction.unwrap_or(app.selected_direction);
    let key = (
        app.uop_cache.loaded_uops.len(),
        entry.package_index,
        entry.file_hash,
        direction,
    );
    if let Some(count) = app.animationframe_uop_frame_counts.get(&key) {
        return Some(*count);
    }

    let payload = animationframe_payload(app, entry).ok()?;
    let metadata = AnimationFrameCc::direction_metadata(
        &payload,
        direction,
    ).ok()?;
    let count = metadata.len();
    app.animationframe_uop_frame_counts.insert(key, count);
    Some(count)
}

fn ec_animationframe_frame_count(app: &mut UopInspectorApp, entry: &AnimationFrameUopEntry) -> Option<usize> {
    let key = (
        app.uop_cache.loaded_uops.len(),
        entry.package_index,
        entry.file_hash,
        u8::MAX,
    );
    if let Some(count) = app.animationframe_uop_frame_counts.get(&key) {
        return Some(*count);
    }

    let payload = animationframe_payload(app, entry).ok()?;
    let metadata = uocf::enhanced::animationframe::AnimationFrame::load_metadata(&payload).ok()?;
    let count = metadata.frames_count as usize;
    app.animationframe_uop_frame_counts.insert(key, count);
    Some(count)
}

fn animationframe_uop_entries(
    app: &mut UopInspectorApp,
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
    let ec_path = app.settings.ec_path.clone();
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
                    block_index: None,
                    source_index: action_id as u32,
                    frame_count: 0,
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
                    block_index: None,
                    source_index: action_id as u32,
                    frame_count: 0,
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
                block_index: None,
                source_index: action_id as u32,
                frame_count: 0,
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
    use uocf::animation_sequence::{AnimationSequence, ec_sequence_path_6digit, ec_sequence_path_8digit};
    use uocf::enhanced::animationframe::{AnimationFrame, MAX_BODY_ID};
    use uocf::uop_container::hash::hash_file_name_simd_batch_strs_into;

    // Collect hashes of sequence files that are not yet in the dictionary.
    // For those already known, use the dict directly — no hashing needed.
    let mut seq_hash_to_body: HashMap<u64, u32> = HashMap::with_capacity(dict_seq_map.len() + MAX_BODY_ID as usize * 2);
    seq_hash_to_body.extend(dict_seq_map.iter().map(|(&h, &b)| (h, b)));

    // Determine which body_ids are not yet covered by the dict so we can
    // batch-hash only those candidate paths.
    let known_bodies: std::collections::HashSet<u32> = dict_seq_map.values().copied().collect();
    let unknown_bodies: Vec<u32> = (0..MAX_BODY_ID)
        .filter(|b| !known_bodies.contains(b))
        .collect();

    if !unknown_bodies.is_empty() {
        let seq_paths_6: Vec<String> = unknown_bodies.iter().copied().map(ec_sequence_path_6digit).collect();
        let seq_paths_8: Vec<String> = unknown_bodies.iter().copied().map(ec_sequence_path_8digit).collect();
        let seq_path_refs: Vec<&str> = seq_paths_6.iter().chain(seq_paths_8.iter()).map(|s| s.as_str()).collect();
        let mut seq_hashes = vec![0u64; seq_path_refs.len()];
        hash_file_name_simd_batch_strs_into(&seq_path_refs, &mut seq_hashes);

        for (i, hash) in seq_hashes.into_iter().enumerate() {
            let body_id = unknown_bodies[i % unknown_bodies.len()];
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
        let Some(count) = AnimationSequence::ec_action_count(&data) else {
            continue;
        };
        if !dict_seq_map.contains_key(file_hash) {
            // Determine which path format this hash corresponds to and record it.
            let path_6 = ec_sequence_path_6digit(body_id);
            let path_8 = ec_sequence_path_8digit(body_id);
            use uocf::uop_container::hash::hash_file_name_single;
            let canonical_path = if hash_file_name_single(&path_6) == *file_hash {
                path_6
            } else {
                path_8
            };
            new_dic_entries.push((*file_hash, canonical_path));
        }
        for action_id in 0..count as u16 {
            frame_pairs.push((body_id, action_id));
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

fn show_sequence_animation_tree(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    ordering: Option<bool>,
    collapse_revision: u64,
    max_height: f32,
) {
    let Some(seq) = app.selected_anim_sequence.clone() else {
        ui.label("No AnimationSequence entry is loaded for the selected body.");
        return;
    };

    let scroll_width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("sequence_animation_tree")
        .auto_shrink([false, false])
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(scroll_width);
            let mut action_ids: Vec<_> = seq.actions.keys().copied().collect();
            action_ids.sort_unstable();
            if ordering == Some(true) {
                action_ids.reverse();
            }
            for action_id in action_ids {
                let Some(action) = seq.actions.get(&action_id) else {
                    continue;
                };
                egui::CollapsingHeader::new(format!("Action {}", action.action_id))
                    .id_salt((
                        "uocf_anim_sequence_action",
                        collapse_revision,
                        seq.body_id,
                        action.action_id,
                    ))
                    .default_open(
                        collapse_revision == 0
                            && app.selected_action_id == action.action_id,
                    )
                    .show(ui, |ui| {
                        for (direction, direction_data) in action.directions.iter().enumerate() {
                            let direction = direction as u8;
                            egui::CollapsingHeader::new(format!("Direction {}", direction))
                                .id_salt((
                                    "uocf_anim_sequence_direction",
                                    collapse_revision,
                                    seq.body_id,
                                    action.action_id,
                                    direction,
                                ))
                                .default_open(
                                    collapse_revision == 0
                                        && app.selected_action_id == action.action_id
                                        && app.selected_direction == direction,
                                )
                                .show(ui, |ui| {
                                    if direction_data.frame_indices.is_empty() {
                                        ui.label("No frames.");
                                    }
                                    for (frame_index, source_frame) in
                                        direction_data.frame_indices.iter().enumerate()
                                    {
                                        let selected =
                                            app.selected_action_id == action.action_id
                                                && app.selected_direction == direction
                                                && app.current_frame_idx == frame_index;
                                        if ui
                                            .selectable_label(
                                                selected,
                                                format!(
                                                    "Frame {} -> source {}",
                                                    frame_index, source_frame
                                                ),
                                            )
                                            .clicked()
                                        {
                                            app.selected_action_id = action.action_id;
                                            app.selected_direction = direction;
                                            app.current_frame_idx = frame_index;
                                            app.last_frame_time = ctx.input(|input| input.time);
                                        }
                                    }
                                });
                        }
                    });
            }
        });
}

fn selected_mul_source_index(app: &UopInspectorApp, body_id: u32) -> eyre::Result<u32> {
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
    app: &UopInspectorApp,
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

fn selected_animation_patch(app: &UopInspectorApp, body_id: u32) -> eyre::Result<MichelangeloPatch> {
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

fn selected_cc_animationframe_payload(app: &UopInspectorApp, body_id: u32) -> eyre::Result<Vec<u8>> {
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

fn selected_ec_animationframe_payload(app: &UopInspectorApp, body_id: u32) -> eyre::Result<Vec<u8>> {
    let internal_path = format!("data/animationframe/{:06}.bin", body_id);
    animationframe_payload_from_loaded_uops(app, "AnimationFrame", &internal_path)
}

fn animationframe_payload_from_loaded_uops(
    app: &UopInspectorApp,
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
