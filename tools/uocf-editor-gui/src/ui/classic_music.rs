//! Classic music in the Audio tab: `Music/Digital/Config.txt` and the files it names, plus the other MP3 and MIDI
//! files of the `Music` folder.
//!
//! Lines of `Config.txt` can be changed and saved to a copy: [`MusicConfig`] writes every line it did not change
//! back as read. MIDI files are shown by what they declare (`uocf::audio::midi`). They cannot be played here: nothing
//! in the workspace synthesizes MIDI.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre;
use eframe::egui;
use uocf::audio::decode::decode;
use uocf::audio::midi::{analyze_midi, MidiAnalysis, MidiDivision};
use uocf::audio::{name_extension, probe_audio, AudioContent};
use uocf::classic::music::{
    find_dir_ignoring_case, MusicConfig, LOOP_FLAGS, MUSIC_CONFIG_FILE_NAME, MUSIC_DIGITAL_DIR_NAME, MUSIC_DIR_NAME,
};
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{UocfEditorApp, ViewMode};
use crate::ui::sounds::{play, save_file, source_selector, stop_and_status};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;
use crate::ui::{centered_row_offset, go_to_field};

const LIST_PANEL_WIDTH: f32 = 300.0;
const NEW_NAME_FIELD_WIDTH: f32 = 160.0;
/// Extensions of the files listed from the `Music` folder.
const MUSIC_FILE_EXTENSIONS: [&str; 3] = ["mp3", "mid", "midi"];
const MIDI_EXTENSIONS: [&str; 2] = ["mid", "midi"];
/// The flag a loop checkbox sets. The client also loops on the other of [`LOOP_FLAGS`], kept while the box stays on.
const LOOP_FLAG: &str = LOOP_FLAGS[0];
const MISSING_FILE_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Sounds,
    label: "the lines of the Classic music Config.txt",
    is_dirty: |app| app.classic_file_views.music.has_unsaved_changes(),
    save: save_config,
    discard: revert_config,
};

/// A row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MusicRow {
    /// A line of `Config.txt`.
    Line(u32),
    /// A file of the `Music` folder that no line names.
    File(PathBuf),
}

/// The selected row's file, read when it was selected.
struct SelectedTrack {
    row: MusicRow,
    path: Option<PathBuf>,
    bytes: Arc<[u8]>,
    content: AudioContent,
    midi: Option<Result<MidiAnalysis, String>>,
}

/// The state of the Classic music list.
#[derive(Default)]
pub struct ClassicMusicState {
    /// The client folder this was read from.
    client: Option<PathBuf>,
    config: Option<MusicConfig>,
    /// `Config.txt` as read or last saved: what the edits are compared to.
    saved_text: String,
    files: Vec<PathBuf>,
    error: Option<String>,
    filter: String,
    selected: Option<SelectedTrack>,
    name_edit: String,
    new_id: u32,
    new_name: String,
}

impl ClassicMusicState {
    /// Whether `Config.txt` holds edits that were not saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.config.as_ref().is_some_and(|config| config.to_text() != self.saved_text)
    }

    fn reload(&mut self, client: &Path) {
        *self = Self { client: Some(client.to_owned()), ..Self::default() };
        match MusicConfig::load(client) {
            Ok(Some(config)) => {
                self.saved_text = config.to_text();
                self.config = Some(config);
            }
            Ok(None) => {}
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        let named: Vec<PathBuf> = self.config.iter().flat_map(|config| config.iter().filter_map(|entry| config.file_path(entry))).collect();
        self.files = find_dir_ignoring_case(client, MUSIC_DIR_NAME).map_or_else(Vec::new, |music| music_files(&music, &named));
    }
}

/// The MP3 and MIDI files under `folder` that are not in `named`, sorted.
fn music_files(folder: &Path, named: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if has_extension(&path, &MUSIC_FILE_EXTENSIONS) && !named.contains(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.to_str().and_then(name_extension).is_some_and(|extension| extensions.contains(&extension.as_str()))
}

fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Reads the file of a row. The row stays selected without a file when its line names none that exists.
fn select(state: &mut ClassicMusicState, row: MusicRow) -> eyre::Result<()> {
    let path = match &row {
        MusicRow::Line(id) => state.config.as_ref().and_then(|config| config.get(*id).and_then(|entry| config.file_path(entry))),
        MusicRow::File(path) => Some(path.clone()),
    };
    if let MusicRow::Line(id) = &row {
        state.name_edit = state.config.as_ref().and_then(|config| config.get(*id)).map_or_else(String::new, |entry| entry.name.clone());
    }
    let bytes: Arc<[u8]> = match &path {
        Some(path) => Arc::from(std::fs::read(path)?),
        None => Arc::from(Vec::new()),
    };
    let midi = path.as_ref().filter(|path| has_extension(path, &MIDI_EXTENSIONS)).map(|_| analyze_midi(&bytes).map_err(|error| format!("{error:#}")));
    let content = probe_audio(&bytes);
    state.selected = Some(SelectedTrack { row, path, bytes, content, midi });
    Ok(())
}

pub fn ui_classic_music(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let Some(client) = app.settings.cc_path.clone() else { return };
    if app.classic_file_views.music.client.as_deref() != Some(client.as_path()) {
        app.classic_file_views.music.reload(&client);
    }

    egui::Panel::left("classic_music_list").resizable(true).default_size(LIST_PANEL_WIDTH).show(ui, |ui| {
        source_selector(app, ui);
        ui.heading("Music");
        let state = &mut app.classic_file_views.music;
        if let Some(error) = &state.error {
            ui.colored_label(MISSING_FILE_COLOR, error);
        }
        ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("Filter by id or name"));
        let go_to = go_to_field(ui, "classic_music_lines");
        ui.separator();
        let mut rows: Vec<(MusicRow, String)> = Vec::new();
        if let Some(config) = &state.config {
            rows.extend(config.iter().map(|entry| {
                let looping = if entry.loops() { "  loop" } else { "" };
                (MusicRow::Line(entry.id), format!("{:03}  {}{looping}", entry.id, entry.name))
            }));
        }
        rows.extend(state.files.iter().map(|path| (MusicRow::File(path.clone()), format!("file  {}", file_label(path)))));
        let query = state.filter.to_ascii_lowercase();
        rows.retain(|(_, label)| query.is_empty() || label.to_ascii_lowercase().contains(&query));
        let selected_row = state.selected.as_ref().map(|selected| selected.row.clone());
        let mut clicked = None;
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        let mut area = egui::ScrollArea::vertical();
        // Go to a line of Config.txt by its id; the files have none.
        let line_positions: Vec<(usize, u32)> = rows
            .iter()
            .enumerate()
            .filter_map(|(position, (row, _))| match row {
                MusicRow::Line(id) => Some((position, *id)),
                MusicRow::File(_) => None,
            })
            .collect();
        if let Some(line) = go_to.position_in(line_positions.iter().map(|&(_, id)| u64::from(id))) {
            let position = line_positions[line].0;
            clicked = Some(rows[position].0.clone());
            area = area.vertical_scroll_offset(centered_row_offset(ui, position, row_height));
        }
        area.show_rows(ui, row_height, rows.len(), |ui, range| {
            for (row, label) in &rows[range] {
                if ui.selectable_label(selected_row.as_ref() == Some(row), label).clicked() {
                    clicked = Some(row.clone());
                }
            }
        });
        if rows.is_empty() {
            ui.label(format!("No {MUSIC_CONFIG_FILE_NAME} and no MP3 or MIDI files in the {MUSIC_DIR_NAME} folder."));
        }
        if let Some(row) = clicked {
            if let Err(error) = select(state, row) {
                app.status_message = format!("Failed to read the music file: {error:#}");
            }
        }
    });

    egui::CentralPanel::default().show(ui, |ui| {
        ui_config_tools(app, ui);
        ui.separator();
        let Some(selected) = &app.classic_file_views.music.selected else {
            ui.label("Select a line or a file.");
            return;
        };
        let (row, path, bytes, content, midi) =
            (selected.row.clone(), selected.path.clone(), selected.bytes.clone(), selected.content.clone(), selected.midi.clone());
        if let MusicRow::Line(id) = row {
            ui_line_editor(app, ui, id);
            ui.separator();
        }
        let Some(path) = path else {
            ui.colored_label(MISSING_FILE_COLOR, "The file this line names does not exist.");
            return;
        };
        let name = file_label(&path);
        ui.heading(&name);
        ui.horizontal(|ui| {
            let decodable = content.format().is_decodable() && midi.is_none();
            let play_button = ui
                .add_enabled(decodable, egui::Button::new("Play"))
                .on_disabled_hover_text("MIDI files cannot be played here: nothing in the workspace synthesizes MIDI");
            if play_button.clicked() {
                play(app, decode(&bytes, &content), &name);
            }
            stop_and_status(app, ui);
            if ui.button("Export file").clicked() {
                let extension = path.to_str().and_then(name_extension).unwrap_or_default();
                save_file(app, name.clone(), ("Music file", extension.as_str()), || Ok(bytes.to_vec()));
            }
        });
        ui.separator();
        egui::Grid::new("classic_music_details").striped(true).show(ui, |ui| {
            let mut row = |key: &str, value: String| {
                ui.label(key);
                ui.label(value);
                ui.end_row();
            };
            row("File", path.display().to_string());
            row("Bytes", bytes.len().to_string());
            match &midi {
                Some(Ok(analysis)) => midi_rows(analysis, &mut row),
                Some(Err(error)) => row("MIDI", error.clone()),
                None => {
                    row("Format", content.format().label().to_owned());
                    row("Codec", content.detail());
                    if let Some(duration) = content.duration_seconds() {
                        row("Duration", format!("{duration:.3}s"));
                    }
                }
            }
        });
    });
}

fn midi_rows(analysis: &MidiAnalysis, row: &mut impl FnMut(&str, String)) {
    row("Format", format!("Standard MIDI file, format {}", analysis.format));
    row(
        "Division",
        match analysis.division {
            MidiDivision::TicksPerQuarter(ticks) => format!("{ticks} ticks per quarter note"),
            MidiDivision::Smpte { frames_per_second, ticks_per_frame } => format!("SMPTE {frames_per_second} fps, {ticks_per_frame} ticks per frame"),
        },
    );
    if let Some(duration) = analysis.duration_seconds {
        row("Duration", format!("{duration:.3}s"));
    }
    row("First tempo", format!("{:.1} bpm", analysis.first_tempo_bpm));
    row("Notes", analysis.notes().to_string());
    for (index, track) in analysis.tracks.iter().enumerate() {
        let programs: Vec<String> = track.programs.iter().map(u8::to_string).collect();
        row(
            &format!("Track {index}"),
            format!(
                "{}: {} events, {} notes, programs [{}]",
                track.name.as_deref().unwrap_or("(unnamed)"),
                track.events,
                track.notes,
                programs.join(", ")
            ),
        );
    }
}

/// Asks where to write `Config.txt`, and writes it. It stays unsaved when the user cancels or the write fails, and the
/// loaded file is refused.
fn save_config(app: &mut UocfEditorApp) {
    let state = &mut app.classic_file_views.music;
    let Some(config) = &state.config else {
        return;
    };
    let Some(path) = crate::dialog::file_dialog().add_filter("Text", &["txt"]).set_file_name(MUSIC_CONFIG_FILE_NAME).save_file() else {
        return;
    };
    let loaded = find_file_ignoring_case(&config.directory, MUSIC_CONFIG_FILE_NAME);
    app.status_message = if loaded.is_some_and(|loaded| same_file(&loaded, &path)) {
        format!("The loaded {MUSIC_CONFIG_FILE_NAME} is refused as the target: save a copy.")
    } else {
        match config.save(&path) {
            Ok(()) => {
                state.saved_text = config.to_text();
                format!("Saved {}.", path.display())
            }
            Err(error) => format!("Failed to save: {error:#}"),
        }
    };
}

fn revert_config(app: &mut UocfEditorApp) {
    let state = &mut app.classic_file_views.music;
    if let Some(client) = state.client.clone() {
        state.reload(&client);
    }
}

/// Save, revert and adding a line, for the whole file.
fn ui_config_tools(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let state = &mut app.classic_file_views.music;
    if state.config.is_none() {
        ui.label(format!("This client has no {MUSIC_DIR_NAME}/{MUSIC_DIGITAL_DIR_NAME}/{MUSIC_CONFIG_FILE_NAME}."));
        return;
    }
    let dirty = state.has_unsaved_changes();
    let mut status = None;
    let (mut save_clicked, mut revert_clicked) = (false, false);
    ui.horizontal(|ui| {
        ui.strong(MUSIC_CONFIG_FILE_NAME);
        if dirty {
            ui.label("(unsaved changes)");
        }
        if ui.add_enabled(dirty, egui::Button::new("Save as...")).on_hover_text("Write the file to a path you pick. Lines you did not change are written back as read. The loaded file itself is refused.").clicked() {
            save_clicked = true;
        }
        if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
            revert_clicked = true;
        }
        ui.separator();
        ui.label("Add id");
        ui.add(egui::DragValue::new(&mut state.new_id));
        ui.add(egui::TextEdit::singleline(&mut state.new_name).hint_text("file name").desired_width(NEW_NAME_FIELD_WIDTH));
        if ui.add_enabled(!state.new_name.is_empty(), egui::Button::new("Add line")).clicked() {
            if let Some(config) = &mut state.config {
                match config.set(state.new_id, &state.new_name, None) {
                    Ok(()) => state.new_name.clear(),
                    Err(error) => status = Some(format!("{error:#}")),
                }
            }
        }
    });
    if let Some(status) = status {
        app.status_message = status;
    }
    if save_clicked {
        save_config(app);
    }
    if revert_clicked {
        revert_config(app);
    }
}

/// The name and the loop flag of one line.
fn ui_line_editor(app: &mut UocfEditorApp, ui: &mut egui::Ui, id: u32) {
    let state = &mut app.classic_file_views.music;
    let Some(entry) = state.config.as_ref().and_then(|config| config.get(id)).cloned() else {
        ui.label(format!("Music id {id} has no line."));
        return;
    };
    let mut status = None;
    let mut reselect = false;
    ui.horizontal(|ui| {
        ui.strong(format!("Music id {id}"));
        ui.label("Name");
        ui.text_edit_singleline(&mut state.name_edit);
        if ui.add_enabled(state.name_edit != entry.name, egui::Button::new("Apply")).clicked() {
            if let Some(config) = &mut state.config {
                match config.set(id, &state.name_edit, entry.flag.as_deref()) {
                    Ok(()) => reselect = true,
                    Err(error) => status = Some(format!("{error:#}")),
                }
            }
        }
        let mut loops = entry.loops();
        if ui.checkbox(&mut loops, "Loop").on_hover_text("The client loops on the flag `loop` or `lop`").changed() {
            if let Some(config) = &mut state.config {
                let flag = loops.then_some(LOOP_FLAG);
                if let Err(error) = config.set(id, &entry.name, flag) {
                    status = Some(format!("{error:#}"));
                }
            }
        }
        if ui.button("Remove line").clicked() {
            if let Some(config) = &mut state.config {
                config.remove(id);
            }
            state.selected = None;
        }
    });
    if reselect {
        if let Err(error) = select(state, MusicRow::Line(id)) {
            status = Some(format!("Failed to read the music file: {error:#}"));
        }
    }
    if let Some(status) = status {
        app.status_message = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONFIG: &str = "# a comment\r\n0 britain1.mp3,loop\r\n9 britainpos,lop\r\n12  vesper\r\n";

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_editor_classic_music_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Music/Digital")).unwrap();
        dir
    }

    fn state_with_config(name: &str) -> (PathBuf, ClassicMusicState) {
        let dir = scratch_dir(name);
        std::fs::write(dir.join("Music/Digital/Config.txt"), SAMPLE_CONFIG).unwrap();
        std::fs::write(dir.join("Music/Digital/britain1.mp3"), b"not audio").unwrap();
        std::fs::write(dir.join("Music/Digital/other.mp3"), b"not audio").unwrap();
        std::fs::write(dir.join("Music/moonglow.mid"), b"not midi").unwrap();
        let mut state = ClassicMusicState::default();
        state.reload(&dir);
        (dir, state)
    }

    #[test]
    fn an_unchanged_config_saves_the_same_bytes() {
        let (dir, state) = state_with_config("unchanged");
        assert!(!state.has_unsaved_changes());
        let output = dir.join("saved.txt");
        state.config.as_ref().unwrap().save(&output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), SAMPLE_CONFIG.as_bytes());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_edit_changes_only_its_line_and_counts_as_unsaved() {
        let (dir, mut state) = state_with_config("edit");
        state.config.as_mut().unwrap().set(9, "britainpos", None).unwrap();
        assert!(state.has_unsaved_changes());
        assert_eq!(state.config.as_ref().unwrap().to_text(), "# a comment\r\n0 britain1.mp3,loop\r\n9 britainpos\r\n12  vesper\r\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn discarding_takes_the_config_back_to_the_file() {
        let (dir, state) = state_with_config("discard");
        let mut app = crate::app::tests::test_app();
        app.classic_file_views.music = state;
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        app.classic_file_views.music.config.as_mut().unwrap().set(9, "britainpos", None).unwrap();
        assert!((TAB_EDITOR.is_dirty)(&app));

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(app.classic_file_views.music.config.as_ref().unwrap().to_text(), SAMPLE_CONFIG);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_file_list_holds_only_what_no_line_names() {
        let (dir, state) = state_with_config("files");
        let names: Vec<String> = state.files.iter().map(|path| file_label(path)).collect();
        assert_eq!(names, ["other.mp3", "moonglow.mid"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_line_without_a_file_selects_with_no_path() {
        let (dir, mut state) = state_with_config("missing");
        select(&mut state, MusicRow::Line(12)).unwrap();
        let selected = state.selected.as_ref().unwrap();
        assert_eq!(selected.path, None);
        assert_eq!(state.name_edit, "vesper");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
