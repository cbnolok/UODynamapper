use std::sync::Arc;

use color_eyre::eyre;
use eframe::egui;
use uocf::audio::decode::{decode, decode_classic_pcm, DecodedAudio};
use uocf::audio::validate::FormatPolicy;
use uocf::audio::{AudioContent, AudioFormat};
use uocf::classic::sound_edit::ClassicSoundEditor;
use uocf::enhanced::audio::{AudioKind, AudioPackage, AUDIO_ENTRY_ROOT};
use uocf::enhanced::audio_edit::{AudioImportOptions, AudioPackageEditor, AudioTarget};

use crate::app::{SoundPlayer, SoundSource, UopInspectorApp};
use crate::ui::{list_sort_controls, sorted_indices_by};

const SOUNDS_LIST_PANEL_WIDTH: f32 = 280.0;
/// The replace options, kept in the egui memory.
const CONVERT_OPTION_ID: &str = "sounds_replace_convert";
const ALLOW_UNVERIFIED_OPTION_ID: &str = "sounds_replace_allow_unverified";

pub fn ui_sounds(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    if app.cc_sounds.is_none() && app.ec_audio.is_none() {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Select a Classic Client path (sound.mul or soundLegacyMUL.uop) or an Enhanced Client path (Audio.uop).");
        });
        return;
    }
    match (app.cc_sounds.is_some(), app.ec_audio.is_some()) {
        (false, true) => app.sound_source = SoundSource::Enhanced,
        (true, false) => app.sound_source = SoundSource::Classic,
        _ => {}
    }
    match app.sound_source {
        SoundSource::Classic => ui_classic_sounds(app, ctx, ui),
        SoundSource::Enhanced => ui_enhanced_audio(app, ctx, ui),
    }
}

/// The Classic/Enhanced switch, when both are loaded.
fn source_selector(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    if app.cc_sounds.is_some() && app.ec_audio.is_some() {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut app.sound_source, SoundSource::Classic, "Classic");
            ui.selectable_value(&mut app.sound_source, SoundSource::Enhanced, "Enhanced");
        });
    }
}

/// Starts playback, opening the audio output on first use.
fn play(app: &mut UopInspectorApp, decoded: eyre::Result<DecodedAudio>, label: &str) {
    let decoded = match decoded {
        Ok(decoded) => decoded,
        Err(e) => {
            app.status_message = format!("Failed to decode {label}: {e:#}");
            return;
        }
    };
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
        app.status_message = match player.play(&decoded) {
            Ok(()) => format!("Playing {label} ({:.3}s).", decoded.duration_seconds()),
            Err(e) => format!("Failed to play {label}: {}", e),
        };
    }
}

fn stop_and_status(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    if ui.button("Stop").clicked() {
        if let Some(player) = &mut app.sound_player {
            player.stop();
        }
    }
    let playing = app.sound_player.as_ref().is_some_and(|player| player.is_playing());
    ui.label(if playing { "Playing" } else { "Idle" });
}

fn save_file(app: &mut UopInspectorApp, default_name: String, filter: (&str, &str), bytes: impl FnOnce() -> eyre::Result<Vec<u8>>) {
    let Some(path) = crate::dialog::file_dialog().add_filter(filter.0, &[filter.1]).set_file_name(default_name).save_file() else {
        return;
    };
    app.status_message = match bytes().and_then(|bytes| std::fs::write(&path, bytes).map_err(Into::into)) {
        Ok(()) => format!("Exported to {}.", path.display()),
        Err(e) => format!("Failed to export: {e:#}"),
    };
}

/// A checkbox whose state lives in the egui memory under `id`.
fn option_checkbox(ui: &mut egui::Ui, id: &str, label: &str, hover: &str) -> bool {
    let id = egui::Id::new(id);
    let mut value = ui.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));
    ui.checkbox(&mut value, label).on_hover_text(hover);
    ui.data_mut(|data| data.insert_temp(id, value));
    value
}

fn convert_checkbox(ui: &mut egui::Ui, hover: &str) -> bool {
    option_checkbox(ui, CONVERT_OPTION_ID, "Convert", hover)
}

/// Picks the replacement file (WAV, or MP3 when `mp3`).
fn pick_input(mp3: bool) -> Option<std::path::PathBuf> {
    let dialog = crate::dialog::file_dialog().add_filter("Wave Audio", &["wav"]);
    let dialog = if mp3 { dialog.add_filter("MPEG Layer III", &["mp3"]) } else { dialog };
    dialog.pick_file()
}

/// Writes copies of the Classic sound files with `slot` replaced by a picked WAV, into a picked folder.
fn replace_classic_slot(app: &mut UopInspectorApp, slot: u32, convert: bool) {
    let Some(client) = app.settings.cc_path.clone() else {
        app.status_message = "No Classic Client path.".to_owned();
        return;
    };
    let Some(input) = pick_input(convert) else { return };
    let Some(output) = crate::dialog::pick_folder() else { return };
    let result = (|| -> eyre::Result<String> {
        let file = std::fs::read(&input)?;
        let mut editor = ClassicSoundEditor::load(&client)?;
        let put = editor.put_sound(slot, &file, None, convert)?;
        let written = editor.save_to(&output)?;
        let names: Vec<String> = written.iter().map(|path| path.display().to_string()).collect();
        Ok(format!("{:?} slot {slot} '{}'{}; wrote {}.", put.write, put.name, if put.converted { " (converted)" } else { "" }, names.join(", ")))
    })();
    app.status_message = result.unwrap_or_else(|e| format!("Failed to replace slot {slot}: {e:#}"));
}

/// Writes a copy of the Audio.uop with the entry `name` replaced by a picked file, to a picked path.
fn replace_ec_entry(app: &mut UopInspectorApp, source: &std::path::Path, name: &str, options: AudioImportOptions) {
    let Some(input) = pick_input(true) else { return };
    let Some(output) = crate::dialog::file_dialog()
        .add_filter("UOP package", &["uop"])
        .set_file_name(uocf::enhanced::audio::AUDIO_UOP_FILE_NAME)
        .save_file()
    else {
        return;
    };
    let result = (|| -> eyre::Result<String> {
        let file = std::fs::read(&input)?;
        let mut editor = AudioPackageEditor::load(source)?;
        let put = editor.replace(&AudioTarget::Name(name.to_owned()), &file, options)?;
        editor.save_as(&output)?;
        let warnings = if put.warnings.is_empty() { String::new() } else { format!(" Warnings: {}.", put.warnings.join("; ")) };
        Ok(format!(
            "{:?} {name} ({:?}{}) in {}.{warnings}",
            put.write,
            put.compression,
            if put.converted { ", converted" } else { "" },
            output.display()
        ))
    })();
    app.status_message = result.unwrap_or_else(|e| format!("Failed to replace {name}: {e:#}"));
}

fn ui_classic_sounds(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(sounds) = app.cc_sounds.clone() else {
        return;
    };

    egui::Panel::left("sounds_list")
        .resizable(true)
        .default_size(SOUNDS_LIST_PANEL_WIDTH)
        .show(ui, |ui| {
            source_selector(app, ui);
            ui.heading(sounds.source_label());
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

    egui::CentralPanel::default().show(ui, |ui| {
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
        let content = AudioContent::ClassicPcm { pcm_len: sound.pcm_data.len() };

        ui.heading(format!("Sound {}", sound.slot_id));
        ui.horizontal(|ui| {
            if ui.button("Play").clicked() {
                play(app, Ok(decode_classic_pcm(&sound.pcm_data)), &format!("sound {}", sound.slot_id));
            }
            stop_and_status(app, ui);
            if ui.button("Export WAV").clicked() {
                let default_name = if sound.name.is_empty() {
                    format!("sound_{:04}.wav", sound.slot_id)
                } else {
                    format!("sound_{:04}_{}.wav", sound.slot_id, sanitize_file_stem(&sound.name))
                };
                save_file(app, default_name, ("Wave Audio", "wav"), || Ok(sound.wav_bytes()));
            }
        });
        ui.horizontal(|ui| {
            let convert = convert_checkbox(ui, "Decode any WAV or MP3 at 22050 Hz and downmix it to mono (no resampling)");
            let button = ui.button("Replace slot...").on_hover_text(
                "Write copies of the sound files, with this slot set from a 16-bit PCM mono 22050 Hz WAV, into another folder",
            );
            if button.clicked() {
                replace_classic_slot(app, sound.slot_id, convert);
            }
        });

        ui.separator();
        egui::Grid::new("sound_details").striped(true).show(ui, |ui| {
            ui.label("Slot");
            ui.label(sound.slot_id.to_string());
            ui.end_row();

            ui.label("Name");
            ui.label(if sound.name.is_empty() { "(unnamed)" } else { &sound.name });
            ui.end_row();

            ui.label("Source");
            ui.label(format!("{:?}", sound.source));
            ui.end_row();

            ui.label("PCM bytes");
            ui.label(sound.pcm_data.len().to_string());
            ui.end_row();

            ui.label("Duration");
            ui.label(format!("{:.3}s", sound.duration_seconds()));
            ui.end_row();

            ui.label("Format");
            ui.label(content.detail());
            ui.end_row();

            ui.label("Header bytes 32..40");
            ui.label(uocf::audio::hex_prefix(&sound.header[uocf::classic::sound::SOUND_NAME_BYTES..]));
            ui.end_row();
        });
    });
}

/// The label of a list entry: the id and kind, then the name (the tables', the dictionary's, or the hash).
fn ec_entry_label(app: &UopInspectorApp, entry: &crate::app::EcAudioListEntry) -> String {
    let name = entry
        .table_name
        .as_deref()
        .or_else(|| app.dictionary.resolve(entry.hash))
        .map(|name| name.strip_prefix(AUDIO_ENTRY_ROOT).unwrap_or(name).to_owned())
        .unwrap_or_else(|| format!("{:016X}", entry.hash));
    match (entry.kind, entry.id) {
        (Some(AudioKind::Sound), Some(id)) => format!("{id:04} sound  {name}"),
        (Some(AudioKind::LegacySound), Some(id)) => format!("{id:04} legacy  {name}"),
        (Some(AudioKind::Music), Some(id)) => format!("{id:04} music  {name}"),
        _ => name,
    }
}

fn select_ec_entry(app: &mut UopInspectorApp, audio: &AudioPackage, hash: u64) {
    app.selected_ec_audio_hash = Some(hash);
    app.ec_audio_selected = match audio.entry_by_hash(hash) {
        Ok(Some(mut entry)) => {
            if entry.name.is_none() {
                entry.name = app.dictionary.resolve(hash).map(str::to_owned);
            }
            Some(Arc::new(entry))
        }
        Ok(None) => None,
        Err(e) => {
            app.status_message = format!("Failed to read entry {hash:016X}: {e:#}");
            None
        }
    };
}

fn ui_enhanced_audio(app: &mut UopInspectorApp, _ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(audio) = app.ec_audio.clone() else {
        return;
    };

    egui::Panel::left("ec_audio_list")
        .resizable(true)
        .default_size(SOUNDS_LIST_PANEL_WIDTH)
        .show(ui, |ui| {
            source_selector(app, ui);
            ui.heading("Audio.uop");
            ui.add(egui::TextEdit::singleline(&mut app.sound_search_query).hint_text("Filter by id or name"));
            let sort = list_sort_controls(ui, "ec_audio_entries", &["Id", "Name"], 0);
            ui.separator();
            let Some(entries) = app.ec_audio_entries.clone() else { return };
            let rows: Vec<(String, Option<u32>)> = entries.iter().map(|entry| (ec_entry_label(app, entry), entry.id)).collect();
            let query = app.sound_search_query.to_ascii_lowercase();
            let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| match sort.option_index {
                1 => left.0.cmp(&right.0),
                _ => (left.1.is_none(), left.1).cmp(&(right.1.is_none(), right.1)),
            });
            let mut clicked = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for index in sorted_indices {
                    let label = &rows[index].0;
                    if !query.is_empty() && !label.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    let hash = entries[index].hash;
                    if ui.selectable_label(app.selected_ec_audio_hash == Some(hash), label).clicked() {
                        clicked = Some(hash);
                    }
                }
            });
            if let Some(hash) = clicked {
                select_ec_entry(app, &audio, hash);
            }
        });

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(entry) = app.ec_audio_selected.clone() else {
            ui.label("Select an entry.");
            return;
        };
        let name = entry.name.clone().unwrap_or_else(|| format!("{:016X}", entry.hash));
        let file_name = name.rsplit('/').next().unwrap_or(&name).to_owned();
        ui.heading(&file_name);
        ui.horizontal(|ui| {
            let decodable = entry.content.format().is_decodable();
            if ui.add_enabled(decodable, egui::Button::new("Play")).clicked() {
                play(app, decode(&entry.payload, &entry.content), &file_name);
            }
            stop_and_status(app, ui);
            let extension = entry.content.format().extension().unwrap_or("bin");
            if ui.button("Export file").clicked() {
                let default_name = if file_name.contains('.') { file_name.clone() } else { format!("{file_name}.{extension}") };
                save_file(app, default_name, (entry.content.format().label(), extension), || Ok(entry.payload.to_vec()));
            }
            if ui.add_enabled(decodable, egui::Button::new("Export decoded WAV")).clicked() {
                let stem = file_name.rsplit_once('.').map_or(file_name.as_str(), |(stem, _)| stem);
                save_file(app, format!("{}.wav", sanitize_file_stem(stem)), ("Wave Audio", "wav"), || {
                    Ok(decode(&entry.payload, &entry.content)?.to_wav())
                });
            }
        });
        ui.horizontal(|ui| {
            let convert = convert_checkbox(ui, "Convert a file a WAV entry cannot take to 16-bit PCM WAV (no MP3 encoding, no resampling)");
            let allow = option_checkbox(
                ui,
                ALLOW_UNVERIFIED_OPTION_ID,
                "Allow unverified format",
                "Accept a format the shipped Audio.uop does not show the client playing, with warnings: for a test client only. A file of another kind than the entry's extension is refused all the same",
            );
            let policy = if allow { FormatPolicy::AllowUnverified } else { FormatPolicy::VerifiedOnly };
            let button = ui
                .add_enabled(entry.name.is_some(), egui::Button::new("Replace..."))
                .on_hover_text("Write a copy of the Audio.uop with this entry replaced (keeping its compression)")
                .on_disabled_hover_text("Only named entries can be replaced");
            if button.clicked() {
                replace_ec_entry(app, audio.path(), &name, AudioImportOptions { policy, convert });
            }
        });

        ui.separator();
        egui::Grid::new("ec_audio_details").striped(true).show(ui, |ui| {
            let mut row = |key: &str, value: String| {
                ui.label(key);
                ui.label(value);
                ui.end_row();
            };
            row("Name", name.clone());
            row("Hash", format!("{:016X}", entry.hash));
            row("Compression", format!("{:?}", entry.compression));
            row("Stored bytes", entry.stored.len().to_string());
            row("Bytes", entry.payload.len().to_string());
            row("Format", entry.content.format().label().to_owned());
            row("Codec", entry.content.detail());
            if let Some(duration) = entry.content.duration_seconds() {
                row("Duration", format!("{duration:.3}s"));
            }
            match &entry.content {
                AudioContent::Mpeg(analysis) => {
                    let tags: Vec<String> = analysis.id3v2.iter().map(|tag| tag.label()).collect();
                    row("ID3v2", if tags.is_empty() { "none".to_owned() } else { tags.join(", ") });
                    row("ID3v1", analysis.tail.id3v1.to_string());
                    let info = analysis.info_header.as_ref();
                    row("Info header", info.map_or("none".to_owned(), |info| info.kind.label().to_owned()));
                    if let Some(encoder) = info.and_then(|info| info.encoder.clone()) {
                        row("Encoder", encoder);
                    }
                    row("Frames", analysis.frames.to_string());
                }
                AudioContent::Wav(analysis) => {
                    let chunks: Vec<String> = analysis.chunks.iter().map(|chunk| chunk.id_label()).collect();
                    row("Chunks", chunks.join(" "));
                    if let Some(format) = analysis.format {
                        row("fmt size", format.chunk_size.to_string());
                    }
                }
                _ => {}
            }
        });
        if entry.content.format() == AudioFormat::Text {
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.monospace(String::from_utf8_lossy(&entry.payload));
            });
        }
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
