use crate::app::{ArtSource, UopInspectorApp};
use crate::ui::image_export::{export_rgba_png, sanitize_file_stem};
use color_eyre::eyre;
use eframe::egui;
use std::path::{Path, PathBuf};
use uocf::classic::anim::AnimMap;
use uocf::classic::animationframe_cc::AnimationFrameCc;
use uocf::classic::michelangelo_uop_codec::{
    export_anim_blocks_from_mul, MichelangeloPatch, MichelangeloPatchEntry,
};
use uocf::classic::vd_codec::VdFile;
use uocf::uop_container::hash::hash_file_name_single;

pub fn ui_animations(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::SidePanel::left("anim_controls")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading("Animation Controls");
            ui.separator();

            let mut body_id = app.selected_anim_id;
            ui.horizontal(|ui| {
                ui.label("Body ID:");
                if ui.add(egui::DragValue::new(&mut body_id)).changed() {
                    app.selected_anim_id = body_id;
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
            ui.horizontal(|ui| {
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::Mul, "MUL");
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::CcUop, "CC UOP");
                ui.radio_value(&mut app.selected_legacy_source, ArtSource::EcUop, "EC UOP");
            });

            if app.selected_legacy_source != ArtSource::Any {
                let export_context = app.client_data.as_ref().map(|client| {
                    let body_id = client
                        .anim_defs
                        .as_ref()
                        .map(|defs| defs.resolve(app.selected_anim_id))
                        .unwrap_or(app.selected_anim_id);
                    body_id
                });

                ui.separator();
                ui.heading("Patch Export");
                ui.horizontal(|ui| {
                    let can_export = export_context.is_some();
                    if ui
                        .add_enabled(can_export, egui::Button::new("Export VD"))
                        .clicked()
                    {
                        if let Some(body_id) = export_context {
                            let default_name = format!(
                                "anim_{}_{}.vd",
                                app.selected_anim_file_idx, body_id
                            );
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
                            let default_name = format!(
                                "anim_{}_{}.uop",
                                app.selected_anim_file_idx, body_id
                            );
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
            }

            ui.separator();
            ui.heading("Playback");
            ui.horizontal(|ui| {
                if ui
                    .button(if app.is_playing {
                        "⏸ Stop"
                    } else {
                        "▶ Play"
                    })
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

    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(client) = &app.client_data {
            let body_id = if let Some(defs) = &client.anim_defs {
                defs.resolve(app.selected_anim_id)
            } else {
                app.selected_anim_id
            };

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
                ArtSource::Mul => AnimMap::load(client.path.clone()).and_then(|anim_map| {
                    anim_map.decode_animation(app.selected_anim_file_idx, body_id)
                }),
                ArtSource::CcUop => {
                    let mut found_frames = Err(eyre::eyre!("Animation not found in UOPs"));
                    for loaded in &app.uop_cache.loaded_uops {
                        if loaded
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .contains("AnimationFrame")
                        {
                            for grp_id in 0..5 {
                                let path = format!(
                                    "build/animationlegacyframe/{:06}/{:02}.bin",
                                    body_id, grp_id
                                );
                                let hash = uocf::uop_container::hash::hash_file_name_single(&path);
                                if let Some(file) = loaded.package.get_file_by_hash(hash) {
                                    if let Ok(data) = file.unpack() {
                                        if let Ok(anim) = AnimationFrameCc::parse(&data) {
                                            found_frames = Ok(anim.frames);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if found_frames.is_ok() {
                            break;
                        }
                    }
                    found_frames
                }
                ArtSource::EcUop => {
                    let mut found_frames = Err(eyre::eyre!("Animation not found in EC UOPs"));
                    for loaded in &app.uop_cache.loaded_uops {
                        if loaded
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .contains("AnimationFrame")
                        {
                            let path = format!("data/animationframe/{:06}.bin", body_id);
                            let hash = uocf::uop_container::hash::hash_file_name_single(&path);
                            if let Some(file) = loaded.package.get_file_by_hash(hash) {
                                if let Ok(data) = file.unpack() {
                                    if let Ok(anim) =
                                        uocf::enhanced::animationframe::AnimationFrame::load(&data)
                                    {
                                        let mut decoded_frames =
                                            Vec::with_capacity(anim.frames.len());
                                        for entry in &anim.frames {
                                            if let Ok(decoded) = anim.decode_frame(entry) {
                                                decoded_frames.push(
                                                    uocf::classic::anim::AnimFrame {
                                                        width: decoded.width,
                                                        height: decoded.height,
                                                        center_x: decoded.center_x,
                                                        center_y: decoded.center_y,
                                                        data: decoded.data,
                                                    },
                                                );
                                            }
                                        }
                                        found_frames = Ok(decoded_frames);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    found_frames
                }
                _ => Err(eyre::eyre!("Source not supported yet")),
            };

            match frames_res {
                Ok(all_frames) => {
                    let logical_frames = if let Some(seq) = &app.selected_anim_sequence {
                        seq.actions
                            .get(&app.selected_action_id)
                            .and_then(|a| a.directions.get(app.selected_direction as usize))
                            .map(|d| &d.frame_indices)
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

                            let key = if app.selected_anim_sequence.is_some() {
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

fn animation_source_label(source: ArtSource) -> &'static str {
    match source {
        ArtSource::Mul => "mul",
        ArtSource::CcUop => "cc_uop",
        ArtSource::EcUop => "ec_uop",
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
            export_anim_blocks_from_mul(idx_path, &mul_path, &[body_id as i32], 0, 0)
        }
        ArtSource::CcUop => {
            let payload = selected_cc_animationframe_payload(app, body_id)?;
            Ok(single_payload_patch(body_id as i32, 0, payload))
        }
        ArtSource::EcUop => {
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
        let internal_path = format!(
            "build/animationlegacyframe/{:06}/{:02}.bin",
            body_id, group_id
        );
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
