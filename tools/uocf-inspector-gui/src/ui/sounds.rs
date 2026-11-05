use std::sync::Arc;

use color_eyre::eyre;
use eframe::egui;
use uocf::audio::decode::{decode, decode_classic_pcm, DecodedAudio};
use uocf::audio::validate::FormatPolicy;
use uocf::audio::{AudioContent, AudioFormat};
use uocf::classic::sound::{SOUND_HEADER_BYTES, SOUND_MUL_FILE_NAME, SOUND_UOP_FILE_NAME};
use uocf::classic::sound_edit::{sound_header, ClassicSoundEditor};
use uocf::enhanced::audio::{AudioKind, AudioPackage, AUDIO_ENTRY_ROOT, AUDIO_UOP_FILE_NAME};
use uocf::enhanced::audio_edit::{AudioImportOptions, AudioPackageEditor, AudioPut, AudioTarget, NewAudio};

use crate::app::{SoundPlayer, SoundSource, UopInspectorApp};
use crate::ui::{
    classic_music, enhanced_files_availability, enhanced_folder_source, folder_files_availability, go_to_field,
    list_sort_controls, sorted_indices_by, source_choice, ClientSource, SourceAvailability, CLASSIC_FOLDER,
};

const SOUNDS_LIST_PANEL_WIDTH: f32 = 280.0;
const CLASSIC_MUSIC_LABEL: &str = "Classic music";
const CLASSIC_MUSIC_HOVER: &str = "The music of the Classic client folder: Music/Digital/Config.txt and its files.";
/// The replace options, kept in the egui memory.
const CONVERT_OPTION_ID: &str = "sounds_replace_convert";
const ALLOW_UNVERIFIED_OPTION_ID: &str = "sounds_replace_allow_unverified";
const SLOT_NAME_OPTION_ID: &str = "sounds_slot_name";
const NEW_ENTRY_OPTION_ID: &str = "sounds_new_entry";
const SLOT_NAME_WIDTH: f32 = 180.0;
const AUDIO_KINDS: [(AudioKind, &str); 3] = [(AudioKind::Sound, "Sound"), (AudioKind::LegacySound, "Legacy sound"), (AudioKind::Music, "Music")];
const NEW_ENTRY_TEXT_HINT: &str = "data/audio/...";
const TABLE_PATH_HOVER: &str = "The path of a new table row, as in the table: Sounds\\... for a sound, Music/... for music. An id that has a row takes it from there; leave it empty.";
const ALLOW_UNVERIFIED_HOVER: &str = "Accept, with warnings, audio the client's decoder plays wrongly or in a way nothing verified (MPEG-2.5, PCM data that is not whole sample frames): for a test client only. A file the decoder refuses or misreads, or of another kind than the entry's extension, is refused all the same";

pub fn ui_sounds(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let sources = available_sources(app);
    let Some(&first) = sources.first() else {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Select a Classic Client path (sound.mul or soundLegacyMUL.uop) or an Enhanced Client path (Audio.uop).");
        });
        return;
    };
    if !sources.contains(&app.sound_source) {
        app.sound_source = first;
    }
    match app.sound_source {
        SoundSource::Classic => ui_classic_sounds(app, ctx, ui),
        SoundSource::Enhanced => ui_enhanced_audio(app, ctx, ui),
        SoundSource::ClassicMusic => classic_music::ui_classic_music(app, ui),
    }
}

/// What the loaded clients have to show.
fn available_sources(app: &UopInspectorApp) -> Vec<SoundSource> {
    let mut sources = Vec::new();
    if app.cc_sounds.is_some() {
        sources.push(SoundSource::Classic);
    }
    if app.ec_audio.is_some() {
        sources.push(SoundSource::Enhanced);
    }
    if app.settings.cc_path.is_some() {
        sources.push(SoundSource::ClassicMusic);
    }
    sources
}

/// The source of the loaded Classic sounds: the Classic client loads them from one file, sound.mul when it is there.
fn classic_sounds_source(app: &UopInspectorApp) -> Option<ClientSource> {
    let sounds = app.cc_sounds.as_ref()?;
    Some(if sounds.source_label() == SOUND_MUL_FILE_NAME { ClientSource::Mul } else { ClientSource::CcUop })
}

/// The audio sources, then the Classic music view: it is another view of the Classic folder, not a source of sounds.
pub(super) fn source_selector(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let classic = classic_sounds_source(app);
    let cc_path = app.settings.cc_path.clone();
    let ec_path = app.settings.ec_path.clone();
    let enhanced = ec_path.as_deref().map(|folder| enhanced_folder_source(&ctx, folder));
    let selected = match app.sound_source {
        SoundSource::Classic => classic,
        SoundSource::Enhanced => enhanced,
        SoundSource::ClassicMusic => None,
    };
    let has_audio = app.ec_audio.is_some();
    let classic_file = |source: ClientSource, file: &str| match (&cc_path, classic) {
        (Some(folder), Some(ClientSource::Mul)) if source == ClientSource::CcUop => SourceAvailability::Missing(format!(
            "No {file} loaded from {}: the Classic sounds load from one file, and {SOUND_MUL_FILE_NAME} comes first.",
            folder.display(),
        )),
        _ => folder_files_availability(cc_path.as_deref(), CLASSIC_FOLDER, file, classic == Some(source)),
    };
    ui.horizontal_wrapped(|ui| {
        let clicked = source_choice(ui, selected, |source| match source {
            ClientSource::Mul => classic_file(source, SOUND_MUL_FILE_NAME),
            ClientSource::CcUop => classic_file(source, SOUND_UOP_FILE_NAME),
            ClientSource::KrUop | ClientSource::EcUop => {
                enhanced_files_availability(&ctx, ec_path.as_deref(), source, AUDIO_UOP_FILE_NAME, has_audio)
            }
        });
        match clicked {
            Some(ClientSource::Mul | ClientSource::CcUop) => app.sound_source = SoundSource::Classic,
            Some(ClientSource::KrUop | ClientSource::EcUop) => app.sound_source = SoundSource::Enhanced,
            None => {}
        }
        ui.separator();
        // Its label never wraps: in a narrow panel the button moves to the next row whole.
        let music = egui::Button::selectable(app.sound_source == SoundSource::ClassicMusic, CLASSIC_MUSIC_LABEL)
            .wrap_mode(egui::TextWrapMode::Extend);
        if ui
            .add_enabled(cc_path.is_some(), music)
            .on_hover_text(CLASSIC_MUSIC_HOVER)
            .on_disabled_hover_text(format!("{CLASSIC_MUSIC_HOVER} No {CLASSIC_FOLDER} folder is set."))
            .clicked()
        {
            app.sound_source = SoundSource::ClassicMusic;
        }
    });
}

/// Starts playback, opening the audio output on first use.
pub(super) fn play(app: &mut UopInspectorApp, decoded: eyre::Result<DecodedAudio>, label: &str) {
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

pub(super) fn stop_and_status(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    if ui.button("Stop").clicked() {
        if let Some(player) = &mut app.sound_player {
            player.stop();
        }
    }
    let playing = app.sound_player.as_ref().is_some_and(|player| player.is_playing());
    ui.label(if playing { "Playing" } else { "Idle" });
}

pub(super) fn save_file(app: &mut UopInspectorApp, default_name: String, filter: (&str, &str), bytes: impl FnOnce() -> eyre::Result<Vec<u8>>) {
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

fn save_classic_copies(editor: &mut ClassicSoundEditor, output: &std::path::Path) -> eyre::Result<String> {
    let written = editor.save_to(output)?;
    Ok(written.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", "))
}

/// Writes copies of the Classic sound files into `output`, with `slot` set from `file`, under `name` when given.
fn write_classic_sound(client: &std::path::Path, slot: u32, file: &[u8], name: Option<&str>, convert: bool, output: &std::path::Path) -> eyre::Result<String> {
    let mut editor = ClassicSoundEditor::load(client)?;
    let put = editor.put_sound(slot, file, name, convert)?;
    let written = save_classic_copies(&mut editor, output)?;
    Ok(format!("{:?} slot {slot} '{}'{}; wrote {written}.", put.write, put.name, if put.converted { " (converted)" } else { "" }))
}

/// Writes copies of the Classic sound files into `output`, with `slot` named `name` and its sound kept.
fn rename_classic_sound(client: &std::path::Path, slot: u32, name: &str, output: &std::path::Path) -> eyre::Result<String> {
    let mut editor = ClassicSoundEditor::load(client)?;
    let bytes = editor.slot_bytes(slot)?.ok_or_else(|| eyre::eyre!("slot {slot} is empty"))?;
    let (header, pcm) = bytes.split_at_checked(SOUND_HEADER_BYTES).ok_or_else(|| eyre::eyre!("slot {slot} has no header"))?;
    let renamed = [sound_header(Some(name), Some(header))?.as_slice(), pcm].concat();
    let write = editor.put_slot_bytes(slot, &renamed)?;
    let written = save_classic_copies(&mut editor, output)?;
    Ok(format!("{write:?} slot {slot} '{name}'; wrote {written}."))
}

/// Sets `slot` of copies of the Classic sound files from a picked WAV, into a picked folder.
fn replace_classic_slot(app: &mut UopInspectorApp, slot: u32, name: Option<&str>, convert: bool) {
    let Some(client) = app.settings.cc_path.clone() else {
        app.status_message = "No Classic Client path.".to_owned();
        return;
    };
    let Some(input) = pick_input(convert) else { return };
    let Some(output) = crate::dialog::pick_folder() else { return };
    let result = std::fs::read(&input).map_err(Into::into).and_then(|file| write_classic_sound(&client, slot, &file, name, convert, &output));
    app.status_message = result.unwrap_or_else(|e| format!("Failed to set slot {slot}: {e:#}"));
}

fn rename_classic_slot(app: &mut UopInspectorApp, slot: u32, name: &str) {
    let Some(client) = app.settings.cc_path.clone() else {
        app.status_message = "No Classic Client path.".to_owned();
        return;
    };
    let Some(output) = crate::dialog::pick_folder() else { return };
    app.status_message = rename_classic_sound(&client, slot, name, &output).unwrap_or_else(|e| format!("Failed to rename slot {slot}: {e:#}"));
}

/// The controls that set a Classic slot from a file, and its name; an empty slot has no sound to keep and needs a name.
fn classic_slot_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui, slot: u32, empty: bool) {
    ui.horizontal(|ui| {
        let convert = convert_checkbox(ui, "Decode any WAV or MP3 at 22050 Hz and downmix it to mono (no resampling)");
        let id = egui::Id::new(SLOT_NAME_OPTION_ID);
        let mut name = ui.data_mut(|data| data.get_temp::<String>(id)).unwrap_or_default();
        ui.label("Name");
        let hint = if empty { "needed" } else { "kept when empty" };
        ui.add(egui::TextEdit::singleline(&mut name).hint_text(hint).desired_width(SLOT_NAME_WIDTH));
        ui.data_mut(|data| data.insert_temp(id, name.clone()));
        let new_name = (!name.is_empty()).then_some(name.as_str());
        let text = if empty { "Add sound..." } else { "Replace slot..." };
        let button = ui.add_enabled(!empty || new_name.is_some(), egui::Button::new(text)).on_hover_text(
            "Write copies of the sound files, with this slot set from a 16-bit PCM mono 22050 Hz WAV, into another folder",
        );
        if button.clicked() {
            replace_classic_slot(app, slot, new_name, convert);
        }
        if empty {
            return;
        }
        let button = ui
            .add_enabled(new_name.is_some(), egui::Button::new("Rename..."))
            .on_hover_text("Write copies of the sound files, with only this slot's name changed, into another folder");
        if button.clicked() {
            rename_classic_slot(app, slot, &name);
        }
    });
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
    let target = AudioTarget::Name(name.to_owned());
    let result = std::fs::read(&input).map_err(Into::into).and_then(|file| {
        write_audio_copy(source, &output, |editor| editor.replace(&target, &file, options))
    });
    app.status_message = result.unwrap_or_else(|e| format!("Failed to replace {name}: {e:#}"));
}

/// Applies `put` to the `source` Audio.uop and writes the copy to `output`.
fn write_audio_copy(source: &std::path::Path, output: &std::path::Path, put: impl FnOnce(&mut AudioPackageEditor) -> eyre::Result<AudioPut>) -> eyre::Result<String> {
    let mut editor = AudioPackageEditor::load(source)?;
    let put = put(&mut editor)?;
    editor.save_as(output)?;
    let warnings = if put.warnings.is_empty() { String::new() } else { format!(" Warnings: {}.", put.warnings.join("; ")) };
    let rows = if put.tables_with_new_rows.is_empty() { String::new() } else { format!(", with a new row in {}", put.tables_with_new_rows.join(" and ")) };
    Ok(format!(
        "{:?} {} ({:?}{}){rows} in {}.{warnings}",
        put.write,
        put.name,
        put.compression,
        if put.converted { ", converted" } else { "" },
        output.display()
    ))
}

/// What the section that adds an entry to the Audio.uop holds, kept in the egui memory.
#[derive(Clone)]
struct NewEntryFields {
    by_id: bool,
    kind: AudioKind,
    id: u32,
    /// The entry name, or the table path of a new row.
    text: String,
    loops: bool,
}

impl Default for NewEntryFields {
    fn default() -> Self {
        Self { by_id: false, kind: AudioKind::Sound, id: 0, text: String::new(), loops: false }
    }
}

impl NewEntryFields {
    fn new_audio(&self) -> NewAudio {
        if !self.by_id {
            return NewAudio::Name(self.text.clone());
        }
        let path = (!self.text.is_empty()).then(|| self.text.clone());
        NewAudio::Id { kind: self.kind, id: self.id, path, loops: (self.kind == AudioKind::Music && self.loops).then_some(true) }
    }
}

/// Writes a copy of the Audio.uop with a new entry from a picked file, to a picked path.
fn add_ec_entry(app: &mut UopInspectorApp, source: &std::path::Path, new: &NewAudio, options: AudioImportOptions) {
    let Some(input) = pick_input(true) else { return };
    let Some(output) = crate::dialog::file_dialog()
        .add_filter("UOP package", &["uop"])
        .set_file_name(uocf::enhanced::audio::AUDIO_UOP_FILE_NAME)
        .save_file()
    else {
        return;
    };
    let result = std::fs::read(&input).map_err(Into::into).and_then(|file| {
        write_audio_copy(source, &output, |editor| editor.add(new, &file, options))
    });
    app.status_message = result.unwrap_or_else(|e| format!("Failed to add the entry: {e:#}"));
}

/// The options of a replace or an add.
fn import_options(ui: &mut egui::Ui) -> AudioImportOptions {
    let convert = convert_checkbox(ui, "Convert a file a WAV entry cannot take to 16-bit PCM WAV (no MP3 encoding, no resampling)");
    let allow = option_checkbox(ui, ALLOW_UNVERIFIED_OPTION_ID, "Allow unverified format", ALLOW_UNVERIFIED_HOVER);
    AudioImportOptions { policy: if allow { FormatPolicy::AllowUnverified } else { FormatPolicy::VerifiedOnly }, convert }
}

fn new_entry_section(app: &mut UopInspectorApp, ui: &mut egui::Ui, source: &std::path::Path) {
    egui::CollapsingHeader::new("Add an entry").id_salt("ec_audio_add").show(ui, |ui| {
        let id = egui::Id::new(NEW_ENTRY_OPTION_ID);
        let mut fields = ui.data_mut(|data| data.get_temp::<NewEntryFields>(id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut fields.by_id, false, "By name");
            ui.selectable_value(&mut fields.by_id, true, "By table id");
        });
        if fields.by_id {
            ui.horizontal(|ui| {
                let shown = AUDIO_KINDS.iter().find(|(kind, _)| *kind == fields.kind).map_or("", |(_, label)| label);
                egui::ComboBox::from_id_salt("ec_audio_add_kind").selected_text(shown).show_ui(ui, |ui| {
                    for (kind, label) in AUDIO_KINDS {
                        ui.selectable_value(&mut fields.kind, kind, label);
                    }
                });
                ui.add(egui::DragValue::new(&mut fields.id));
            });
            ui.label("Table path").on_hover_text(TABLE_PATH_HOVER);
            ui.text_edit_singleline(&mut fields.text);
            if fields.kind == AudioKind::Music {
                ui.checkbox(&mut fields.loops, "Loops");
            }
        } else {
            ui.label("Entry name");
            ui.add(egui::TextEdit::singleline(&mut fields.text).hint_text(NEW_ENTRY_TEXT_HINT));
        }
        let options = import_options(ui);
        let ready = fields.by_id || !fields.text.is_empty();
        if ui.add_enabled(ready, egui::Button::new("Add...")).on_hover_text("Write a copy of the Audio.uop with this entry added").clicked() {
            add_ec_entry(app, source, &fields.new_audio(), options);
        }
        ui.data_mut(|data| data.insert_temp(id, fields));
    });
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
            let go_to = go_to_field(ui, "sound_entries");
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
                let visible_indices: Vec<usize> = sorted_indices
                    .into_iter()
                    .filter(|&index| {
                        let entry = &entries[index];
                        query.is_empty()
                            || entry.slot_id.to_string().contains(&query)
                            || entry.name.to_ascii_lowercase().contains(&query)
                    })
                    .collect();
                let went_to = go_to.position_in(visible_indices.iter().map(|&index| u64::from(entries[index].slot_id)));
                if let Some(position) = went_to {
                    app.selected_sound_slot = entries[visible_indices[position]].slot_id;
                    app.selected_sound_id = app.selected_sound_slot;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for index in visible_indices {
                        let entry = &entries[index];
                        let label = if entry.name.is_empty() {
                            format!("{:04}  {:.2}s", entry.slot_id, entry.duration_seconds)
                        } else {
                            format!(
                                "{:04}  {}  {:.2}s  {} bytes",
                                entry.slot_id, entry.name, entry.duration_seconds, entry.pcm_bytes
                            )
                        };
                        let selected = app.selected_sound_slot == entry.slot_id;
                        let response = ui.selectable_label(selected, label);
                        if went_to.is_some() && selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
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
                classic_slot_controls(app, ui, app.selected_sound_slot, true);
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
        classic_slot_controls(app, ui, sound.slot_id, false);

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
            new_entry_section(app, ui, audio.path());
            ui.add(egui::TextEdit::singleline(&mut app.sound_search_query).hint_text("Filter by id or name"));
            let go_to = go_to_field(ui, "ec_audio_entries");
            let sort = list_sort_controls(ui, "ec_audio_entries", &["Id", "Name"], 0);
            ui.separator();
            let Some(entries) = app.ec_audio_entries.clone() else { return };
            let rows: Vec<(String, Option<u32>)> = entries.iter().map(|entry| (ec_entry_label(app, entry), entry.id)).collect();
            let query = app.sound_search_query.to_ascii_lowercase();
            let sorted_indices = sorted_indices_by(&rows, sort.ordering(), |left, right| match sort.option_index {
                1 => left.0.cmp(&right.0),
                _ => (left.1.is_none(), left.1).cmp(&(right.1.is_none(), right.1)),
            });
            let visible_indices: Vec<usize> = sorted_indices
                .into_iter()
                .filter(|&index| query.is_empty() || rows[index].0.to_ascii_lowercase().contains(&query))
                .collect();
            // Entries without an id are not a Go-to target.
            let numbered: Vec<usize> = visible_indices.iter().copied().filter(|&index| rows[index].1.is_some()).collect();
            let went_to = go_to.position_in(numbered.iter().filter_map(|&index| rows[index].1).map(u64::from));
            let mut clicked = went_to.map(|position| entries[numbered[position]].hash);
            egui::ScrollArea::vertical().show(ui, |ui| {
                for index in visible_indices {
                    let label = &rows[index].0;
                    let hash = entries[index].hash;
                    let response = ui.selectable_label(app.selected_ec_audio_hash == Some(hash), label);
                    if went_to.is_some() && clicked == Some(hash) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
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
            let options = import_options(ui);
            let button = ui
                .add_enabled(entry.name.is_some(), egui::Button::new("Replace..."))
                .on_hover_text("Write a copy of the Audio.uop with this entry replaced (keeping its compression)")
                .on_disabled_hover_text("Only named entries can be replaced");
            if button.clicked() {
                replace_ec_entry(app, audio.path(), &name, options);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV, EC_CLIENT_ROOTS_ENV};
    use uocf::audio::wav::encode_pcm_wav;
    use uocf::classic::sound::{SoundMap, CHANNELS, MAX_SOUND_ID, NOMINAL_SAMPLE_RATE};
    use uocf::uop_container::hash::hash_file_name_single;

    const NEW_NAME: &str = "uocf_inspector.wav";
    const NEW_ENTRY: &str = "data/audio/uocf_inspector_added.wav";
    const PCM_BYTES: usize = 2048;
    const ID: u32 = 7;
    const TABLE_PATH: &str = "Music/Test.mp3";

    fn test_wav() -> Vec<u8> {
        encode_pcm_wav(NOMINAL_SAMPLE_RATE, CHANNELS, &[0; PCM_BYTES])
    }

    fn folder_name(prefix: &str, root: &std::path::Path) -> String {
        format!("{prefix}-{}", root.file_name().unwrap().to_string_lossy().replace(' ', "-"))
    }

    #[test]
    fn the_new_entry_fields_name_the_entry_or_the_table_row() {
        let by_name = NewEntryFields { text: NEW_ENTRY.to_owned(), loops: true, ..NewEntryFields::default() };
        assert_eq!(by_name.new_audio(), NewAudio::Name(NEW_ENTRY.to_owned()));

        let row = NewEntryFields { by_id: true, kind: AudioKind::Music, id: ID, text: TABLE_PATH.to_owned(), loops: true };
        assert_eq!(row.new_audio(), NewAudio::Id { kind: AudioKind::Music, id: ID, path: Some(TABLE_PATH.to_owned()), loops: Some(true) });

        let has_row = NewEntryFields { by_id: true, kind: AudioKind::Sound, id: ID, text: String::new(), loops: true };
        assert_eq!(has_row.new_audio(), NewAudio::Id { kind: AudioKind::Sound, id: ID, path: None, loops: None }, "only music loops");
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn shipped_classic_sounds_save_the_same_bytes_and_a_rename_and_a_new_slot_come_back() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let same = output_path(&folder_name("sound-same", &root));
            let mut editor = ClassicSoundEditor::load(&root).unwrap();
            let names: Vec<String> = editor.file_names().into_iter().map(str::to_owned).collect();
            save_classic_copies(&mut editor, &same).unwrap();
            for name in &names {
                assert!(std::fs::read(root.join(name)).unwrap() == std::fs::read(same.join(name)).unwrap(), "{}: {name} differs after a save without changes", root.display());
            }

            let original = SoundMap::load(&root).unwrap();
            let slots = 0..=MAX_SOUND_ID;
            let first = slots.clone().find(|&slot| original.read_slot(slot).unwrap().is_some()).unwrap();
            let empty = slots.clone().find(|&slot| original.read_slot(slot).unwrap().is_none()).unwrap();
            let renamed_dir = output_path(&folder_name("sound-renamed", &root));
            rename_classic_sound(&root, first, NEW_NAME, &renamed_dir).unwrap();
            let renamed = SoundMap::load(&renamed_dir).unwrap().read_slot(first).unwrap().unwrap();
            assert_eq!(renamed.name, NEW_NAME);
            assert!(renamed.pcm_data == original.read_slot(first).unwrap().unwrap().pcm_data, "{}: the sound changed", root.display());

            let added_dir = output_path(&folder_name("sound-added", &root));
            assert!(write_classic_sound(&root, empty, &test_wav(), None, false, &added_dir).is_err(), "a new slot needs a name");
            write_classic_sound(&root, empty, &test_wav(), Some(NEW_NAME), false, &added_dir).unwrap();
            let added = SoundMap::load(&added_dir).unwrap().read_slot(empty).unwrap().unwrap();
            assert_eq!((added.name.as_str(), added.pcm_data.len()), (NEW_NAME, PCM_BYTES));
            println!("{}: {names:?} save the same bytes; slot {first} renamed and slot {empty} added come back", root.display());
            for dir in [same, renamed_dir, added_dir] {
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }

    #[test]
    #[ignore = "needs real clients: set UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_audio_package_saves_the_same_bytes_and_an_added_entry_comes_back() {
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let audio = AudioPackage::load(&root).unwrap().unwrap();
            let same = output_path(&folder_name("audio-same", &root));
            AudioPackageEditor::load(audio.path()).unwrap().save_as(&same).unwrap();
            let identical = std::fs::read(&same).unwrap() == std::fs::read(audio.path()).unwrap();
            assert!(identical, "{}: differs after a save without changes", root.display());
            std::fs::remove_file(&same).unwrap();

            let output = output_path(&folder_name("audio-added", &root));
            let options = AudioImportOptions::default();
            let new = NewAudio::Name(NEW_ENTRY.to_owned());
            println!("{}", write_audio_copy(audio.path(), &output, |editor| editor.add(&new, &test_wav(), options)).unwrap());
            let reopened = AudioPackage::load_file(&output).unwrap();
            let entry = reopened.entry_by_hash(hash_file_name_single(NEW_ENTRY)).unwrap().unwrap();
            assert!(*entry.payload == *test_wav(), "{}: the entry differs after reopening", root.display());
            println!("{}: same bytes; an added entry comes back", root.display());
            std::fs::remove_file(&output).unwrap();
        }
    }
}
