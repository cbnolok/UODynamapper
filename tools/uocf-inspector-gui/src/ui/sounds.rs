use eframe::egui;

use crate::app::{SoundPlayer, UopInspectorApp};
use crate::ui::{list_sort_controls, sorted_indices_by};

pub fn ui_sounds(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(sounds) = app.cc_sounds.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Select a Classic Client path containing soundidx.mul and sound.mul.");
        });
        return;
    };

    egui::SidePanel::left("sounds_list")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading("sound.mul");
            ui.horizontal(|ui| {
                ui.label("Slot");
                if ui.add(egui::DragValue::new(&mut app.selected_sound_slot).range(0..=u32::MAX)).changed() {
                    app.selected_sound_id = app.selected_sound_slot;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Id");
                ui.add(egui::DragValue::new(&mut app.selected_sound_id).range(0..=u32::MAX));
                if ui.button("Resolve").clicked() {
                    match sounds.read_id(app.selected_sound_id) {
                        Ok(Some(found)) => {
                            app.selected_sound_slot = found.sound.slot_id;
                            app.status_message = if found.translated {
                                format!(
                                    "Sound id {} resolved to slot {} via Sound.def.",
                                    found.requested_id, found.sound.slot_id
                                )
                            } else {
                                format!("Sound id {} uses direct slot.", found.requested_id)
                            };
                        }
                        Ok(None) => {
                            app.status_message = format!("No sound exists for id {}.", app.selected_sound_id);
                        }
                        Err(e) => {
                            app.status_message = format!("Failed to resolve sound id: {}", e);
                        }
                    }
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut app.sound_search_query)
                    .hint_text("Filter by slot or name"),
            );
            let sort = list_sort_controls(ui, "sound_entries", &["Slot", "Name", "Duration", "Bytes"], 0);
            ui.separator();

            if let Some(entries) = app.cc_sound_entries.clone() {
                let query = app.sound_search_query.to_ascii_lowercase();
                let sorted_indices = sorted_indices_by(&entries, sort.ordering(), |left, right| {
                    match sort.option_index {
                        1 => left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()),
                        2 => left
                            .duration_seconds
                            .partial_cmp(&right.duration_seconds)
                            .unwrap_or(std::cmp::Ordering::Equal),
                        3 => left.pcm_bytes.cmp(&right.pcm_bytes),
                        _ => left.slot_id.cmp(&right.slot_id),
                    }
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for index in sorted_indices {
                        let entry = &entries[index];
                        if !query.is_empty()
                            && !entry.slot_id.to_string().contains(&query)
                            && !entry.name.to_ascii_lowercase().contains(&query)
                        {
                            continue;
                        }

                        let label = if entry.name.is_empty() {
                            format!("{:04}  {:.2}s", entry.slot_id, entry.duration_seconds)
                        } else {
                            format!(
                                "{:04}  {}  {:.2}s  {} bytes",
                                entry.slot_id, entry.name, entry.duration_seconds, entry.pcm_bytes
                            )
                        };
                        if ui
                            .selectable_label(app.selected_sound_slot == entry.slot_id, label)
                            .clicked()
                        {
                            app.selected_sound_slot = entry.slot_id;
                            app.selected_sound_id = entry.slot_id;
                        }
                    }
                });
            }
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        let sound = match sounds.read_slot(app.selected_sound_slot) {
            Ok(Some(sound)) => sound,
            Ok(None) => {
                ui.label(format!("Sound slot {} is empty.", app.selected_sound_slot));
                return;
            }
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("Failed to read sound: {}", e));
                return;
            }
        };

        ui.heading(format!("Sound {}", sound.slot_id));
        ui.horizontal(|ui| {
            if ui.button("Play").clicked() {
                if app.sound_player.is_none() {
                    match SoundPlayer::new() {
                        Ok(player) => app.sound_player = Some(player),
                        Err(e) => {
                            app.status_message = format!("Failed to initialize audio output: {}", e);
                            return;
                        }
                    }
                }

                if let Some(player) = &mut app.sound_player {
                    match player.play_pcm(&sound.pcm_data) {
                        Ok(()) => {
                            app.status_message = format!("Playing sound {}.", sound.slot_id);
                        }
                        Err(e) => {
                            app.status_message = format!("Failed to play sound: {}", e);
                        }
                    }
                }
            }

            if ui.button("Stop").clicked() {
                if let Some(player) = &mut app.sound_player {
                    player.stop();
                }
            }

            if ui.button("Export WAV").clicked() {
                let default_name = if sound.name.is_empty() {
                    format!("sound_{:04}.wav", sound.slot_id)
                } else {
                    format!("sound_{:04}_{}.wav", sound.slot_id, sanitize_file_stem(&sound.name))
                };
                if let Some(path) = crate::dialog::file_dialog()
                    .add_filter("Wave Audio", &["wav"])
                    .set_file_name(default_name)
                    .save_file()
                {
                    match std::fs::write(&path, sound.wav_bytes()) {
                        Ok(()) => {
                            app.status_message = format!("Exported sound to {}.", path.display());
                        }
                        Err(e) => {
                            app.status_message = format!("Failed to export sound: {}", e);
                        }
                    }
                }
            }

            let playing = app.sound_player.as_ref().is_some_and(|player| player.is_playing());
            ui.label(if playing { "Playing" } else { "Idle" });
        });

        ui.separator();
        egui::Grid::new("sound_details").striped(true).show(ui, |ui| {
            ui.label("Slot");
            ui.label(sound.slot_id.to_string());
            ui.end_row();

            ui.label("Name");
            ui.label(if sound.name.is_empty() { "(unnamed)" } else { &sound.name });
            ui.end_row();

            ui.label("PCM bytes");
            ui.label(sound.pcm_data.len().to_string());
            ui.end_row();

            ui.label("Duration");
            ui.label(format!("{:.3}s", sound.duration_seconds()));
            ui.end_row();

            ui.label("Format");
            ui.label("PCM, 22050 Hz, mono, 16-bit");
            ui.end_row();
        });
    });
}

fn sanitize_file_stem(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.is_empty() {
        "sound".to_string()
    } else {
        out
    }
}
