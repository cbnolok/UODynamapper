//! Editing controls of the Classic view of the Multis tab, for `multi.mul` and its index through
//! `uocf::classic::multi_write`.
//!
//! While `Edit` is on, the tab shows the file being edited, as it is on disk: `verdata.mul`, which the view otherwise
//! lays over it, is not part of it.

use std::path::{Path, PathBuf};

use color_eyre::eyre;
use eframe::egui;
use uocf::classic::multi::MultiPart;
use uocf::classic::multi_write::{MultiMulFile, MultiPartRecord, HIGH_SEAS_PART_BYTES, MULTI_DATA_FILE, MULTI_INDEX_FILES};
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{UopInspectorApp, ViewMode};
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction, MUL_FILTER};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const INDEX_EXTENSION: &str = "idx";
const VALUE_DRAG_SPEED: f64 = 0.1;
const ITEM_HEX_DIGITS: usize = 4;
const FLAGS_HEX_DIGITS: usize = 8;
const EDIT_HOVER: &str = "Change the parts of a multi, or add a multi, and save a copy of multi.mul and its index. This shows multi.mul as it is on disk, without verdata.mul.";
const HIGH_SEAS_FIELD_HOVER: &str = "The last field of a 16-byte part. Its meaning is not known. It is kept as it is unless you change it.";
const FLAGS_HOVER: &str = "Bit 0 is set on the parts the client draws.";

#[derive(Default)]
pub struct ClassicMultiEditor {
    pub enabled: bool,
    /// The file, with the paths it was read from: the data and the index.
    file: Option<(MultiMulFile, PathBuf, PathBuf)>,
    /// The file as it was before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<MultiMulFile>,
    /// Grows with every change, for the tab to read the parts again.
    generation: u64,
    error: Option<String>,
}

impl ClassicMultiEditor {
    fn is_dirty(&self) -> bool {
        self.opened.is_some()
    }

    fn data(&self) -> Option<&MultiMulFile> {
        self.file.as_ref().map(|(file, _, _)| file)
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// The parts of multi `id`, when the file is open.
    pub(super) fn parts(&self, id: u32) -> Option<Vec<MultiPart>> {
        let parts = self.records(id)?;
        Some(parts.into_iter().map(|record| record.part).collect())
    }

    /// The number of multis, when the file is open.
    pub(super) fn multi_count(&self) -> Option<u32> {
        self.data().map(|file| file.len() as u32)
    }

    fn records(&self, id: u32) -> Option<Vec<MultiPartRecord>> {
        Some(self.data()?.parts(id as usize).ok().flatten().unwrap_or_default())
    }

    fn open(&mut self, root: &Path) -> eyre::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let data = find_file_ignoring_case(root, MULTI_DATA_FILE).ok_or_else(|| eyre::eyre!("no {MULTI_DATA_FILE} in {}", root.display()))?;
        let index = MULTI_INDEX_FILES.iter().find_map(|name| find_file_ignoring_case(root, name)).ok_or_else(|| eyre::eyre!("no multi index in {}", root.display()))?;
        self.file = Some((MultiMulFile::load(root)?, data, index));
        self.generation += 1;
        Ok(())
    }

    fn revert(&mut self) {
        let Some(opened) = self.opened.take() else {
            return;
        };
        if let Some((file, _, _)) = self.file.as_mut() {
            *file = opened;
        }
        self.generation += 1;
    }

    /// Stores the parts of multi `id`, keeping the file as opened for the discard.
    fn set_parts(&mut self, id: u32, parts: &[MultiPartRecord]) {
        let Some((file, _, _)) = self.file.as_mut() else {
            return;
        };
        if self.opened.is_none() {
            self.opened = Some(file.clone());
        }
        if let Err(error) = file.set_parts(id as usize, parts) {
            self.error = Some(format!("{error:#}"));
        }
        self.generation += 1;
    }
}

/// Where the index of a data file written to `output` goes: the name of the source's index for a file with the
/// usual name, else beside it under its own name.
fn index_target(output: &Path, source_index: &Path) -> PathBuf {
    let usual = output.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(MULTI_DATA_FILE));
    match source_index.file_name().filter(|_| usual) {
        Some(name) => output.with_file_name(name),
        None => output.with_extension(INDEX_EXTENSION),
    }
}

/// Writes the file and its index beside `output`. The files it was read from are refused.
fn write_copy(editor: &ClassicMultiEditor, output: &Path) -> eyre::Result<PathBuf> {
    let (file, data, index) = editor.file.as_ref().ok_or_else(|| eyre::eyre!("{MULTI_DATA_FILE} is not open"))?;
    let output_index = index_target(output, index);
    if same_file(data, output) || same_file(index, &output_index) {
        eyre::bail!("the loaded {MULTI_DATA_FILE} or its index is refused as the target: save a copy");
    }
    file.save(&output_index, output)?;
    Ok(output_index)
}

fn editor(app: &mut UopInspectorApp) -> &mut ClassicMultiEditor {
    &mut app.classic_multi_editor
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Multis,
    label: "multi.mul",
    is_dirty: |app| app.classic_multi_editor.is_dirty(),
    save,
    discard: |app| {
        editor(app).revert();
    },
};

fn save(app: &mut UopInspectorApp) {
    let Some(output) = pick_output(MULTI_DATA_FILE, MUL_FILTER) else {
        return;
    };
    match write_copy(editor(app), &output) {
        Ok(_) => {
            editor(app).opened = None;
            editor(app).error = None;
            report_saved(app, &output);
        }
        Err(error) => editor(app).error = Some(format!("Save failed: {error:#}")),
    }
}

/// `Edit` and the rest of the bar, and the file read for it.
pub(super) fn bar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let root = app.client_data.as_ref().map(|client| client.path.clone());
    let state = editor(app);
    let dirty = state.is_dirty();
    let action = editor_bar(ui, &mut state.enabled, EDIT_HOVER, dirty, "Save multi.mul...", &mut state.error);
    match action {
        Some(BarAction::Revert) => editor(app).revert(),
        Some(BarAction::Save) => save(app),
        None => {}
    }
    if !editor(app).enabled {
        return;
    }
    let Some(root) = root else {
        return;
    };
    if let Err(error) = editor(app).open(&root) {
        editor(app).error = Some(format!("{error:#}"));
        editor(app).enabled = false;
    }
}

/// A new part: the chosen one's values when there is one, else a drawn part of item 0.
fn new_part(parts: &[MultiPartRecord], selected: Option<usize>) -> MultiPartRecord {
    selected.and_then(|index| parts.get(index)).copied().unwrap_or(MultiPartRecord {
        part: MultiPart { item_id: 0, x: 0, y: 0, z: 0, flags: super::multis::CLASSIC_MULTI_PART_VISIBLE_FLAG },
        high_seas_field: 0,
    })
}

/// The buttons and fields of the parts of multi `id`. `selected` is the part chosen in the tab's list; the part to choose
/// after a change is returned, with `Some(None)` for none.
pub(super) fn part_panel(app: &mut UopInspectorApp, ui: &mut egui::Ui, id: u32, selected: Option<usize>) -> Option<Option<usize>> {
    let state = editor(app);
    let parts = state.records(id)?;
    let selected = selected.filter(|&index| index < parts.len());
    let mut new_selection = None;
    let mut changed = parts.clone();
    ui.horizontal(|ui| {
        ui.strong(format!("Multi {id}: {} parts", parts.len()));
        ui.separator();
        if ui.button("Add part").clicked() {
            changed.push(new_part(&parts, selected));
            new_selection = Some(Some(changed.len() - 1));
        }
        if let Some(index) = selected {
            if ui.button("Delete part").clicked() {
                changed.remove(index);
                new_selection = Some(None);
            }
        }
        let count = state.multi_count().unwrap_or_default();
        if ui.button("Add multi").on_hover_text(format!("Adds multi {count} at the end, with one part.")).clicked() {
            state.set_parts(count, &[new_part(&[], None)]);
        }
    });
    if let Some(index) = selected {
        if let Some(record) = changed.get_mut(index) {
            let high_seas = state.data().is_some_and(|file| file.part_bytes == HIGH_SEAS_PART_BYTES);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Part #{index}"));
                ui.label("Tile:");
                ui.add(egui::DragValue::new(&mut record.part.item_id).hexadecimal(ITEM_HEX_DIGITS, false, true).prefix("0x").speed(VALUE_DRAG_SPEED));
                for (name, value) in [("X:", &mut record.part.x), ("Y:", &mut record.part.y), ("Z:", &mut record.part.z)] {
                    ui.label(name);
                    ui.add(egui::DragValue::new(value).speed(VALUE_DRAG_SPEED));
                }
                ui.label("Flags:").on_hover_text(FLAGS_HOVER);
                ui.add(egui::DragValue::new(&mut record.part.flags).hexadecimal(FLAGS_HEX_DIGITS, false, true).prefix("0x").speed(VALUE_DRAG_SPEED));
                if high_seas {
                    ui.label("Last field:").on_hover_text(HIGH_SEAS_FIELD_HOVER);
                    ui.add(egui::DragValue::new(&mut record.high_seas_field).speed(VALUE_DRAG_SPEED));
                }
            });
        }
    } else {
        ui.weak("Select a part in the list to edit it.");
    }
    if changed != parts {
        state.set_parts(id, &changed);
    }
    new_selection
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV};
    use uocf::classic::multi_write::LEGACY_PART_BYTES;

    const MULTI: u32 = 0;
    const EDITED_ITEM: u16 = 0x1234;
    const EDITED_Z: i16 = 11;

    fn small_file() -> MultiMulFile {
        let part = [0u8; HIGH_SEAS_PART_BYTES];
        let mut index = Vec::new();
        for value in [0i32, HIGH_SEAS_PART_BYTES as i32, 0] {
            index.extend_from_slice(&value.to_le_bytes());
        }
        MultiMulFile::from_bytes(&index, &part).unwrap()
    }

    fn edited_parts(mut parts: Vec<MultiPartRecord>) -> Vec<MultiPartRecord> {
        parts[0].part.item_id = EDITED_ITEM;
        parts[0].part.z = EDITED_Z;
        parts
    }

    #[test]
    fn a_change_is_unsaved_and_discarding_takes_it_back() {
        let mut app = test_app();
        editor(&mut app).file = Some((small_file(), PathBuf::new(), PathBuf::new()));
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        let parts = editor(&mut app).records(MULTI).unwrap();
        editor(&mut app).set_parts(MULTI, &edited_parts(parts));
        assert!((TAB_EDITOR.is_dirty)(&app));
        assert_eq!(editor(&mut app).parts(MULTI).unwrap()[0].item_id, EDITED_ITEM);

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(editor(&mut app).parts(MULTI).unwrap()[0].item_id, 0);
    }

    #[test]
    fn the_index_goes_beside_the_data_under_the_right_name() {
        let index = Path::new("/client/multiidx.mul");
        assert_eq!(index_target(Path::new("/copy/multi.mul"), index), Path::new("/copy/multiidx.mul"));
        assert_eq!(index_target(Path::new("/copy/MULTI.MUL"), Path::new("/client/multi.idx")), Path::new("/copy/multi.idx"));
        assert_eq!(index_target(Path::new("/copy/edited.mul"), index), Path::new("/copy/edited.idx"));
    }

    #[test]
    fn the_loaded_files_are_refused_as_the_target() {
        let data = output_path("refused-multi.mul");
        let index = output_path("refused-multi.idx");
        std::fs::write(&data, []).unwrap();
        std::fs::write(&index, []).unwrap();
        let mut state = ClassicMultiEditor::default();
        state.file = Some((small_file(), data.clone(), index.clone()));
        assert!(write_copy(&state, &data).is_err());
        assert_eq!(std::fs::read(&data).unwrap(), Vec::<u8>::new(), "nothing was written");
        std::fs::remove_file(&data).unwrap();
        std::fs::remove_file(&index).unwrap();
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_multi_mul_saves_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            if find_file_ignoring_case(&root, MULTI_DATA_FILE).is_none() {
                println!("{}: no {MULTI_DATA_FILE}", root.display());
                continue;
            }
            let mut state = ClassicMultiEditor::default();
            state.open(&root).unwrap();
            let (_, data, index) = state.file.as_ref().unwrap();
            let (source_data, source_index) = (std::fs::read(data).unwrap(), std::fs::read(index).unwrap());
            let output = output_path(MULTI_DATA_FILE);
            let output_index = write_copy(&state, &output).unwrap();
            assert!(std::fs::read(&output).unwrap() == source_data, "{}: the data differs after a save without changes", root.display());
            assert!(std::fs::read(&output_index).unwrap() == source_index, "{}: the index differs after a save without changes", root.display());

            let id = (0..state.multi_count().unwrap()).find(|&id| !state.records(id).unwrap().is_empty()).unwrap();
            let edited = edited_parts(state.records(id).unwrap());
            state.set_parts(id, &edited);
            let added = state.multi_count().unwrap();
            state.set_parts(added, &[new_part(&[], None)]);
            write_copy(&state, &output).unwrap();
            let reopened = MultiMulFile::from_bytes(&std::fs::read(&output_index).unwrap(), &std::fs::read(&output).unwrap()).unwrap();
            assert_eq!(reopened.parts(id as usize).unwrap().unwrap(), edited);
            assert_eq!(reopened.parts(added as usize).unwrap().unwrap(), vec![new_part(&[], None)]);
            let part_bytes = state.data().unwrap().part_bytes;
            assert!(part_bytes == LEGACY_PART_BYTES || part_bytes == HIGH_SEAS_PART_BYTES);
            std::fs::remove_file(&output).unwrap();
            std::fs::remove_file(&output_index).unwrap();
            println!("{}: same bytes; an edit and a new multi come back ({} multis, {part_bytes}-byte parts)", root.display(), added);
        }
    }
}
