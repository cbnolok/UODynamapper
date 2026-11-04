//! Editing controls of the animdata.mul view of the Animated items tab: the stored fields of a record, and a save that
//! writes a copy through `AnimData::to_bytes`. The chunk headers, the trailing bytes and the frames past a record's
//! count are kept as they were. Field meanings follow `uocf::classic::animdata`.

use std::sync::Arc;

use eframe::egui;
use uocf::classic::animdata::{AnimData, AnimDataEntry, ANIMDATA_FRAME_COUNT};
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{UopInspectorApp, ViewMode};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const ANIMDATA_FILE_NAME: &str = "animdata.mul";
const MUL_FILTER: (&str, &[&str]) = ("MUL file", &["mul"]);
/// Drag speed of the number fields (typing is the main input).
const VALUE_DRAG_SPEED: f64 = 0.1;
const UNKNOWN_HEX_DIGITS: usize = 2;
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 90, 90);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);
const DIRTY_MARKER: &str = "*";

/// The state of the editor.
#[derive(Default)]
pub struct AnimDataEditor {
    pub enabled: bool,
    /// The records as they were before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<Arc<AnimData>>,
    error: Option<String>,
}

impl AnimDataEditor {
    pub fn has_unsaved_changes(&self) -> bool {
        self.opened.is_some()
    }
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::AnimData,
    label: "the animdata.mul records",
    is_dirty: |app| app.animdata_editor.has_unsaved_changes(),
    save,
    discard: revert,
};

/// The editing toolbar row. While editing, the record field reaches the records with no frames, which the list leaves out.
pub(super) fn toolbar(app: &mut UopInspectorApp, ui: &mut egui::Ui, record_count: usize) {
    ui.toggle_value(&mut app.animdata_editor.enabled, "Edit").on_hover_text("Edit the selected record; save a copy of animdata.mul.");
    if !app.animdata_editor.enabled {
        return;
    }
    ui.label("Record:");
    let last_record = u32::try_from(record_count.saturating_sub(1)).unwrap_or(u32::MAX);
    let record = egui::DragValue::new(&mut app.selected_animdata_id).range(0..=last_record).speed(VALUE_DRAG_SPEED);
    if ui.add(record).on_hover_text("Any record, with frames or not.").changed() {
        app.current_frame_idx = 0;
        app.is_playing = false;
    }
    ui.separator();
    if ui.add_enabled(app.animdata_editor.has_unsaved_changes(), egui::Button::new("Revert to loaded")).clicked() {
        revert(app);
    }
    if ui.button("Save animdata.mul...").on_hover_text("Writes a copy. The loaded file itself is refused as the target.").clicked() {
        save(app);
    }
    if app.animdata_editor.has_unsaved_changes() {
        ui.colored_label(WARNING_COLOR, format!("{DIRTY_MARKER} unsaved changes"));
    }
    if let Some(error) = app.animdata_editor.error.clone() {
        ui.colored_label(ERROR_COLOR, error);
        if ui.small_button("Dismiss").clicked() {
            app.animdata_editor.error = None;
        }
    }
}

fn save(app: &mut UopInspectorApp) {
    let Some(client) = app.client_data.as_ref() else {
        return;
    };
    let Some(animdata) = client.animdata.clone() else {
        return;
    };
    let source = find_file_ignoring_case(&client.path, ANIMDATA_FILE_NAME);
    let (name, extensions) = MUL_FILTER;
    let Some(output) = crate::dialog::file_dialog().add_filter(name, extensions).set_file_name(ANIMDATA_FILE_NAME).save_file() else {
        return;
    };
    if source.is_some_and(|source| same_file(&source, &output)) {
        app.animdata_editor.error = Some("The loaded animdata.mul is refused as the target: save a copy.".to_owned());
        return;
    }
    match animdata.save(&output) {
        Ok(()) => {
            app.animdata_editor.opened = None;
            app.animdata_editor.error = None;
            let message = format!("Saved {}.", output.display());
            app.status_message = message.clone();
            app.log(message);
        }
        Err(error) => app.animdata_editor.error = Some(format!("Save failed: {error:#}")),
    }
}

fn revert(app: &mut UopInspectorApp) {
    if let Some(opened) = app.animdata_editor.opened.take() {
        if let Some(client) = app.client_data.as_mut() {
            client.animdata = Some(opened);
        }
    }
}

/// A record's byte fields, as rows of the grid around them; returns whether one changed.
pub(super) fn record_fields(ui: &mut egui::Ui, entry: &mut AnimDataEntry, editing: bool) -> bool {
    let mut changed = false;
    ui.label("Unknown");
    let unknown = egui::DragValue::new(&mut entry.unknown).hexadecimal(UNKNOWN_HEX_DIGITS, false, true).prefix("0x").speed(VALUE_DRAG_SPEED);
    changed |= ui.add_enabled(editing, unknown).changed();
    ui.end_row();
    // A count above the stored frames is left alone until the user changes it.
    let frame_count = egui::DragValue::new(&mut entry.frame_count)
        .range(0..=ANIMDATA_FRAME_COUNT)
        .clamp_existing_to_range(false)
        .speed(VALUE_DRAG_SPEED);
    ui.label("Frame count");
    changed |= ui.add_enabled(editing, frame_count).changed();
    ui.end_row();
    ui.label("Frame interval");
    changed |= ui.add_enabled(editing, egui::DragValue::new(&mut entry.frame_interval).speed(VALUE_DRAG_SPEED)).changed();
    ui.end_row();
    ui.label("Frame start");
    changed |= ui.add_enabled(editing, egui::DragValue::new(&mut entry.frame_start).speed(VALUE_DRAG_SPEED)).changed();
    ui.end_row();
    changed
}

/// A frame's offset from the record's id; returns whether it changed.
pub(super) fn frame_offset_field(ui: &mut egui::Ui, offset: &mut i8, editing: bool) -> bool {
    if !editing {
        ui.label(offset.to_string());
        return false;
    }
    ui.add(egui::DragValue::new(offset).speed(VALUE_DRAG_SPEED)).changed()
}

/// Stores a record's new contents, keeping the records as opened for the discard.
pub(super) fn store(app: &mut UopInspectorApp, entry: AnimDataEntry) {
    let Some(animdata) = app.client_data.as_mut().and_then(|client| client.animdata.as_mut()) else {
        return;
    };
    app.animdata_editor.opened.get_or_insert_with(|| Arc::clone(animdata));
    if let Some(record) = Arc::make_mut(animdata).entries.get_mut(entry.id as usize) {
        *record = entry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::logic::ClientData;
    use uocf::classic::animdata::{ANIMDATA_CHUNK_SIZE, ANIMDATA_RECORD_SIZE};
    use uocf::classic::art::ArtMap;
    use uocf::classic::tiledata::TileData;
    use uocf::uop_container::package::UopPackage;

    const CHUNK_HEADER_BYTES: usize = 4;
    /// Past the frames of a record: the unknown byte, then the frame count.
    const FRAME_COUNT_OFFSET: usize = ANIMDATA_FRAME_COUNT + 1;
    const EDITED_RECORD: u32 = 3;
    const EDITED_FRAME_COUNT: u8 = 2;
    const TRAILING_BYTES: &[u8] = b"tail";

    /// Two zeroed chunks with their headers set and bytes past the last chunk.
    fn file_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 2 * ANIMDATA_CHUNK_SIZE];
        bytes[0] = 1;
        bytes[ANIMDATA_CHUNK_SIZE] = 2;
        bytes.extend_from_slice(TRAILING_BYTES);
        bytes
    }

    fn app_with_animdata() -> UopInspectorApp {
        let mut app = test_app();
        app.client_data = Some(ClientData {
            path: std::path::PathBuf::new(),
            art: Arc::new(ArtMap::load_standalone_uop(UopPackage::new_default())),
            tiledata: Arc::new(TileData::new_empty()),
            multis: None,
            _ec_multis: None,
            hues: None,
            animdata: Some(Arc::new(AnimData::from_bytes(&file_bytes()).unwrap())),
            anim_map: None,
        });
        app
    }

    fn animdata(app: &UopInspectorApp) -> Arc<AnimData> {
        app.client_data.as_ref().unwrap().animdata.clone().unwrap()
    }

    fn edit_record(app: &mut UopInspectorApp) {
        let mut entry = animdata(app).get(EDITED_RECORD).unwrap().clone();
        entry.frame_count = EDITED_FRAME_COUNT;
        store(app, entry);
    }

    #[test]
    fn an_edit_changes_only_its_byte_and_discarding_takes_it_back() {
        let mut app = app_with_animdata();
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        edit_record(&mut app);
        assert!((TAB_EDITOR.is_dirty)(&app));
        assert_eq!(animdata(&app).get(EDITED_RECORD).unwrap().frame_count, EDITED_FRAME_COUNT);

        let mut expected = file_bytes();
        expected[CHUNK_HEADER_BYTES + EDITED_RECORD as usize * ANIMDATA_RECORD_SIZE + FRAME_COUNT_OFFSET] = EDITED_FRAME_COUNT;
        assert_eq!(animdata(&app).to_bytes().unwrap(), expected);

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(animdata(&app).to_bytes().unwrap(), file_bytes());
    }

    #[test]
    fn a_second_edit_keeps_the_records_as_opened() {
        let mut app = app_with_animdata();
        edit_record(&mut app);
        let mut entry = animdata(&app).get(EDITED_RECORD).unwrap().clone();
        entry.frame_interval = EDITED_FRAME_COUNT;
        store(&mut app, entry);
        (TAB_EDITOR.discard)(&mut app);
        assert_eq!(animdata(&app).to_bytes().unwrap(), file_bytes());
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and run with --ignored"]
    fn a_shipped_animdata_saves_the_same_bytes_and_an_edit_changes_only_its_field() {
        const CLIENT_ROOTS_ENV: &str = "UO_CLASSIC_CLIENT_ROOTS";
        let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
        for root in std::env::split_paths(&roots) {
            let Some(path) = find_file_ignoring_case(&root, ANIMDATA_FILE_NAME) else {
                println!("{}: no {ANIMDATA_FILE_NAME}", root.display());
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            let mut animdata = AnimData::load(path).unwrap();
            assert_eq!(animdata.to_bytes().unwrap(), bytes, "{}", root.display());

            let most_frames = animdata.entries.iter().map(|entry| entry.frame_count).max().unwrap();
            let entry = animdata.entries.iter_mut().find(|entry| entry.is_active()).unwrap();
            entry.frame_interval = entry.frame_interval.wrapping_add(1);
            let edited = animdata.to_bytes().unwrap();
            assert_eq!(edited.len(), bytes.len());
            let differing = edited.iter().zip(&bytes).filter(|(left, right)| left != right).count();
            assert_eq!(differing, 1, "{}", root.display());
            println!("{}: same bytes; one edit changes one byte; at most {most_frames} frames", root.display());
        }
    }
}
