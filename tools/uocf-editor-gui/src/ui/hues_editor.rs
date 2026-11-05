//! Editing controls of the Hues tab, for `hues.mul` and `Hues.uop` through `uocf::classic::hues` and
//! `uocf::enhanced::hues_mul_uop`.
//!
//! Both files are edited as `hues.mul` hues: 32 colours of 15 bits. A saved `hues.mul` keeps every byte the edits leave
//! alone. A saved `Hues.uop` is a copy of the loaded package where only the changed hues are written, by the chosen ramp:
//! a hue's 256-texel table holds more than its 32 colours in EA's packages, and an edited hue loses that.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre;
use eframe::egui;
use uocf::classic::hues::{HueEntry, HuesMul, HUE_NAME_LEN};
use uocf::enhanced::entry_edit::UopEntryEditor;
use uocf::enhanced::hues::{HUE_TABLE_ENTRIES, HUES_UOP_NAME};
use uocf::enhanced::hues_mul_uop::{
    build_hues_uop_entries, build_hues_uop_entries_on_base, expand_hue_color, hue_palette_row, hues_mul_from_uop_package,
    put_edited_hues, reduce_hue_color, write_hues_uop, HueRampMode,
};
use uocf::uop_container::package::UopPackage;
use uocf::utils::path::find_file_ignoring_case;

use crate::app::{HuesSource, UocfEditorApp, ViewMode};
use crate::logic::uop_cache::LoadedUop;
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction, MUL_FILTER, UOP_FILTER};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const HUES_MUL_NAME: &str = "hues.mul";
/// Bit 15 of a `hues.mul` colour, which is not part of the colour: an edit keeps it.
const COLOR_FLAG_BIT: u16 = 0x8000;
const TABLE_FIELD_HEX_DIGITS: usize = 4;
const VALUE_DRAG_SPEED: f64 = 0.1;
const TABLE_STRIP_HEIGHT: f32 = 24.0;
const NAME_FIELD_WIDTH: f32 = 180.0;
const RAMP_HOVER: &str = "How a saved Hues.uop fills the 256-texel table of an edited hue from its 32 colours. Interpolated ramps from each colour to the next, as EA's packages do; flat repeats each colour over 8 texels.";
const EC_PREVIEW_NOTE: &str = "The bitmap and the item below show the loaded package; this strip shows the edited hue.";

/// One of the two files, read when `Edit` is turned on.
struct HuesFile {
    source: PathBuf,
    /// The hues as in the file: a saved `Hues.uop` writes the hues that differ from them.
    loaded: HuesMul,
    current: HuesMul,
    /// The hues as they were before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<HuesMul>,
}

impl HuesFile {
    fn new(source: PathBuf, loaded: HuesMul) -> Self {
        Self { source, current: loaded.clone(), loaded, opened: None }
    }
}

#[derive(Default)]
pub struct HuesEditor {
    pub enabled: bool,
    classic: Option<HuesFile>,
    enhanced: Option<HuesFile>,
    ramp: HueRampMode,
    /// The conversion of `hues.mul` to `Hues.uop` writes on the loaded `Hues.uop`.
    keep_loaded_package: bool,
    error: Option<String>,
}

impl HuesEditor {
    fn file(&self, source: HuesSource) -> Option<&HuesFile> {
        match source {
            HuesSource::CcMul => self.classic.as_ref(),
            HuesSource::EcUop => self.enhanced.as_ref(),
        }
    }

    fn file_mut(&mut self, source: HuesSource) -> Option<&mut HuesFile> {
        match source {
            HuesSource::CcMul => self.classic.as_mut(),
            HuesSource::EcUop => self.enhanced.as_mut(),
        }
    }

    fn is_dirty(&self, source: HuesSource) -> bool {
        self.file(source).is_some_and(|file| file.opened.is_some())
    }
}

pub(super) const CLASSIC_TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Hues,
    label: "the hues.mul hues",
    is_dirty: |app| app.hues_editor.is_dirty(HuesSource::CcMul),
    save: |app| save(app, HuesSource::CcMul),
    discard: |app| revert(app, HuesSource::CcMul),
};

pub(super) const ENHANCED_TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Hues,
    label: "the Hues.uop hues",
    is_dirty: |app| app.hues_editor.is_dirty(HuesSource::EcUop),
    save: |app| save(app, HuesSource::EcUop),
    discard: |app| revert(app, HuesSource::EcUop),
};

fn loaded_hues_package(app: &UocfEditorApp) -> Option<Arc<LoadedUop>> {
    let named = |path: &Path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(HUES_UOP_NAME));
    app.uop_cache.loaded_uops.iter().find(|loaded| named(&loaded.path)).cloned()
}

fn classic_source(app: &UocfEditorApp) -> Option<PathBuf> {
    find_file_ignoring_case(&app.client_data.as_ref()?.path, HUES_MUL_NAME)
}

/// Reads the file of `source` when it is not open yet.
fn open(app: &mut UocfEditorApp, source: HuesSource) -> eyre::Result<()> {
    if app.hues_editor.file(source).is_some() {
        return Ok(());
    }
    match source {
        HuesSource::CcMul => {
            let path = classic_source(app).ok_or_else(|| eyre::eyre!("no {HUES_MUL_NAME} in the client folder"))?;
            app.hues_editor.classic = Some(HuesFile::new(path.clone(), HuesMul::load(&path)?));
        }
        HuesSource::EcUop => {
            let loaded = loaded_hues_package(app).ok_or_else(|| eyre::eyre!("{HUES_UOP_NAME} is not loaded"))?;
            let hues = hues_mul_from_uop_package(&loaded.package)?.hues;
            app.hues_editor.enhanced = Some(HuesFile::new(loaded.path.clone(), hues));
        }
    }
    Ok(())
}

/// Writes a copy of the package at `source` to `output`, with the hues of `current` that differ from `loaded`.
fn write_package_copy(source: &Path, loaded: &HuesMul, current: &HuesMul, ramp: HueRampMode, output: &Path) -> eyre::Result<()> {
    let edited: Vec<u32> = current.hues().zip(loaded.hues()).filter(|(now, then)| now != then).map(|(now, _)| now.id).collect();
    let mut editor = UopEntryEditor::load(source)?;
    put_edited_hues(&mut editor, current, &edited, ramp)?;
    editor.save_as(output)
}

/// Writes `hues` as a new `Hues.uop`: on `base` when given (its unedited hues keep their bytes, the others are
/// interpolated), else every hue by `ramp`.
fn write_converted_package(hues: &HuesMul, base: Option<&UopPackage>, ramp: HueRampMode, output: &Path) -> eyre::Result<()> {
    let entries = match base {
        Some(base) => build_hues_uop_entries_on_base(hues, base)?.0,
        None => build_hues_uop_entries(hues, ramp),
    };
    write_hues_uop(&entries, output, None)
}

fn save(app: &mut UocfEditorApp, source: HuesSource) {
    let ramp = app.hues_editor.ramp;
    let Some(file) = app.hues_editor.file(source) else {
        return;
    };
    let written = match source {
        HuesSource::CcMul => pick_output(HUES_MUL_NAME, MUL_FILTER).map(|output| {
            let result = if same_file(&file.source, &output) {
                Err(eyre::eyre!("the loaded {HUES_MUL_NAME} is refused as the target: save a copy"))
            } else {
                file.current.save(&output)
            };
            (output, result)
        }),
        HuesSource::EcUop => pick_output(HUES_UOP_NAME, UOP_FILTER).map(|output| {
            let result = write_package_copy(&file.source, &file.loaded, &file.current, ramp, &output);
            (output, result)
        }),
    };
    let Some((output, result)) = written else {
        return;
    };
    match result {
        Ok(()) => {
            if let Some(file) = app.hues_editor.file_mut(source) {
                file.opened = None;
            }
            app.hues_editor.error = None;
            report_saved(app, &output);
        }
        Err(error) => app.hues_editor.error = Some(format!("Save failed: {error:#}")),
    }
}

/// `hues.mul` as a `Hues.uop`, or `Hues.uop` as a `hues.mul`; the file on the tab stays as it is.
fn convert(app: &mut UocfEditorApp, source: HuesSource) {
    let Some(file) = app.hues_editor.file(source) else {
        return;
    };
    let hues = file.current.clone();
    let (ramp, keep) = (app.hues_editor.ramp, app.hues_editor.keep_loaded_package);
    let package = loaded_hues_package(app);
    let result = match source {
        HuesSource::CcMul => {
            let Some(output) = pick_output(HUES_UOP_NAME, UOP_FILTER) else {
                return;
            };
            let refused = package.as_ref().is_some_and(|package| same_file(&package.path, &output));
            let result = if refused {
                Err(eyre::eyre!("the loaded {HUES_UOP_NAME} is refused as the target: save a copy"))
            } else {
                let base = package.as_ref().filter(|_| keep).map(|package| &package.package);
                write_converted_package(&hues, base, ramp, &output)
            };
            (output, result)
        }
        HuesSource::EcUop => {
            let Some(output) = pick_output(HUES_MUL_NAME, MUL_FILTER) else {
                return;
            };
            let refused = classic_source(app).is_some_and(|path| same_file(&path, &output));
            let result = if refused {
                Err(eyre::eyre!("the loaded {HUES_MUL_NAME} is refused as the target: save a copy"))
            } else {
                hues.save(&output)
            };
            (output, result)
        }
    };
    match result {
        (output, Ok(())) => {
            app.hues_editor.error = None;
            report_saved(app, &output);
        }
        (_, Err(error)) => app.hues_editor.error = Some(format!("Conversion failed: {error:#}")),
    }
}

fn revert(app: &mut UocfEditorApp, source: HuesSource) {
    let Some(file) = app.hues_editor.file_mut(source) else {
        return;
    };
    let Some(opened) = file.opened.take() else {
        return;
    };
    file.current = opened;
    if source == HuesSource::CcMul {
        show_classic_hues(app);
    }
}

/// The hues the tabs draw with follow the edited `hues.mul`.
fn show_classic_hues(app: &mut UocfEditorApp) {
    let Some(file) = app.hues_editor.classic.as_ref() else {
        return;
    };
    let hues: Vec<HueEntry> = file.current.hues().copied().collect();
    if let Some(client) = app.client_data.as_mut() {
        client.hues = Some(Arc::new(hues));
    }
    app.texture_previews.clear();
}

/// Stores a hue's new contents, keeping the hues as opened for the discard.
fn store(app: &mut UocfEditorApp, source: HuesSource, entry: HueEntry) {
    let Some(file) = app.hues_editor.file_mut(source) else {
        return;
    };
    if file.opened.is_none() {
        file.opened = Some(file.current.clone());
    }
    if let Some(slot) = file.current.hue_mut(entry.id) {
        *slot = entry;
    }
    if source == HuesSource::CcMul {
        show_classic_hues(app);
    }
}

/// `Edit`, and while it is on the bar, the ramp and the conversion of the file on the tab.
pub(super) fn bar(app: &mut UocfEditorApp, ui: &mut egui::Ui, source: HuesSource) {
    let dirty = app.hues_editor.is_dirty(source);
    let (save_text, convert_text, edit_hover) = match source {
        HuesSource::CcMul => ("Save hues.mul...", "Convert to Hues.uop...", "Change the colours, table start and end and name of a hue, and save a copy of hues.mul."),
        HuesSource::EcUop => ("Save Hues.uop...", "Convert to hues.mul...", "Change the 32 colours and name of a hue, and save a copy of Hues.uop with only the changed hues."),
    };
    let editor = &mut app.hues_editor;
    let action = editor_bar(ui, &mut editor.enabled, edit_hover, dirty, save_text, &mut editor.error);
    if !app.hues_editor.enabled {
        return;
    }
    if let Err(error) = open(app, source) {
        app.hues_editor.error = Some(format!("{error:#}"));
        app.hues_editor.enabled = false;
        return;
    }
    match action {
        Some(BarAction::Revert) => revert(app, source),
        Some(BarAction::Save) => save(app, source),
        None => {}
    }
    ui.separator();
    let has_package = loaded_hues_package(app).is_some();
    let editor = &mut app.hues_editor;
    let on_loaded_package = source == HuesSource::CcMul && has_package && editor.keep_loaded_package;
    ui.add_enabled_ui(!on_loaded_package, |ui| {
        ui.label("Ramp:").on_hover_text(RAMP_HOVER);
        ui.selectable_value(&mut editor.ramp, HueRampMode::Linear, "Interpolated").on_hover_text(RAMP_HOVER);
        ui.selectable_value(&mut editor.ramp, HueRampMode::Flat, "Flat").on_hover_text(RAMP_HOVER);
    });
    if source == HuesSource::CcMul && has_package {
        ui.checkbox(&mut editor.keep_loaded_package, "On the loaded Hues.uop").on_hover_text(
            "The conversion keeps the bitmap, atlas row, swatch and name of every hue this hues.mul leaves unedited, byte for byte, and interpolates the others.",
        );
    }
    if ui.button(convert_text).on_hover_text("Writes a new file from the hues on this tab; they stay unsaved here if they were.").clicked() {
        convert(app, source);
    }
}

/// The name up to its first NUL, as text.
fn name_text(entry: &HueEntry) -> String {
    String::from_utf8_lossy(entry.name_bytes()).into_owned()
}

fn set_name(entry: &mut HueEntry, text: &str) {
    let bytes = text.as_bytes();
    let length = bytes.len().min(HUE_NAME_LEN);
    entry.name = [0; HUE_NAME_LEN];
    entry.name[..length].copy_from_slice(&bytes[..length]);
}

/// Sets colour `index`, keeping its bit 15.
fn set_color(entry: &mut HueEntry, index: usize, [red, green, blue]: [u8; 3]) {
    let flag = entry.color_table[index] & COLOR_FLAG_BIT;
    entry.color_table[index] = flag | reduce_hue_color([red, green, blue, u8::MAX]);
}

/// The 256 texels of the hue's table, as a saved `Hues.uop` would hold them.
fn table_strip(ui: &mut egui::Ui, entry: &HueEntry, ramp: HueRampMode) {
    let width = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, TABLE_STRIP_HEIGHT), egui::Sense::hover());
    let texel_width = rect.width() / HUE_TABLE_ENTRIES as f32;
    for (index, [red, green, blue, _]) in hue_palette_row(&entry.color_table, ramp).into_iter().enumerate() {
        let left = rect.left() + index as f32 * texel_width;
        let texel = egui::Rect::from_min_size(egui::pos2(left, rect.top()), egui::vec2(texel_width, rect.height()));
        ui.painter().rect_filled(texel, 0.0, egui::Color32::from_rgb(red, green, blue));
    }
}

/// The fields of hue `hue_id`, while editing.
pub(super) fn fields(app: &mut UocfEditorApp, ui: &mut egui::Ui, source: HuesSource, hue_id: u32) {
    if !app.hues_editor.enabled {
        return;
    }
    let ramp = app.hues_editor.ramp;
    let Some(mut entry) = app.hues_editor.file(source).and_then(|file| file.current.hue(hue_id)).copied() else {
        return;
    };
    let before = entry;
    ui.separator();
    ui.strong("Edit this hue");
    ui.horizontal_wrapped(|ui| {
        for index in 0..entry.color_table.len() {
            let [red, green, blue, _] = expand_hue_color(entry.color_table[index]);
            let mut color = [red, green, blue];
            let response = ui.color_edit_button_srgb(&mut color).on_hover_text(format!("Colour {index}"));
            if response.changed() {
                set_color(&mut entry, index, color);
            }
        }
    });
    table_strip(ui, &entry, ramp);
    if source == HuesSource::EcUop {
        ui.weak(EC_PREVIEW_NOTE);
    }
    ui.horizontal(|ui| {
        ui.label("Name:");
        let mut name = name_text(&entry);
        let field = egui::TextEdit::singleline(&mut name).desired_width(NAME_FIELD_WIDTH).char_limit(HUE_NAME_LEN);
        if ui.add(field).changed() {
            set_name(&mut entry, &name);
        }
        if source == HuesSource::CcMul {
            ui.label("Table start:");
            ui.add(egui::DragValue::new(&mut entry.table_start).hexadecimal(TABLE_FIELD_HEX_DIGITS, false, true).prefix("0x").speed(VALUE_DRAG_SPEED));
            ui.label("Table end:");
            ui.add(egui::DragValue::new(&mut entry.table_end).hexadecimal(TABLE_FIELD_HEX_DIGITS, false, true).prefix("0x").speed(VALUE_DRAG_SPEED));
        }
    });
    if entry != before {
        store(app, source, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::logic::ClientData;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV, EC_CLIENT_ROOTS_ENV};
    use uocf::classic::art::ArtMap;
    use uocf::classic::hues::{HUE_COLOR_COUNT, HUE_GROUP_SIZE};
    use uocf::classic::tiledata::TileData;
    use uocf::enhanced::hues::{EcHueTable, hue_bitmap_hash};

    const EDITED_HUE: u32 = 5;
    const OTHER_HUE: u32 = 6;
    const EDITED_COLOR: usize = 3;
    const EDITED_RGB: [u8; 3] = [0x80, 0x40, 0x20];
    /// `EDITED_RGB` in 15 bits.
    const EDITED_VALUE: u16 = (0x10 << 10) | (0x08 << 5) | 0x04;
    const EDITED_NAME: &str = "edited name";
    const GROUPS: usize = 2;
    /// The first texel after colour `EDITED_COLOR`, where the two ramps differ.
    const TEXEL_AFTER_EDITED_COLOR: usize = EDITED_COLOR * 8 + 1;

    fn edited(mut entry: HueEntry) -> HueEntry {
        set_color(&mut entry, EDITED_COLOR, EDITED_RGB);
        set_name(&mut entry, EDITED_NAME);
        entry
    }

    fn file_bytes() -> Vec<u8> {
        (0..GROUPS * HUE_GROUP_SIZE).map(|index| (index % 251) as u8).collect()
    }

    fn app_with_open_files() -> UocfEditorApp {
        let mut app = test_app();
        app.client_data = Some(ClientData {
            path: PathBuf::new(),
            art: Arc::new(ArtMap::load_standalone_uop(UopPackage::new_default())),
            tiledata: Arc::new(TileData::new_empty()),
            multis: None,
            _ec_multis: None,
            hues: None,
            animdata: None,
            anim_map: None,
        });
        let hues = HuesMul::parse(&file_bytes());
        app.hues_editor.classic = Some(HuesFile::new(PathBuf::new(), hues.clone()));
        app.hues_editor.enhanced = Some(HuesFile::new(PathBuf::new(), hues));
        app
    }

    fn current(app: &UocfEditorApp, source: HuesSource) -> &HuesMul {
        &app.hues_editor.file(source).unwrap().current
    }

    #[test]
    fn an_edit_keeps_the_flag_bit_and_the_other_bytes_of_the_hue() {
        let mut entry = HueEntry::empty(1);
        entry.color_table[EDITED_COLOR] = COLOR_FLAG_BIT;
        entry.name[HUE_NAME_LEN - 1] = b'x';
        let changed = edited(entry);
        assert_eq!(changed.color_table[EDITED_COLOR], COLOR_FLAG_BIT | EDITED_VALUE);
        assert_eq!(changed.name_bytes(), EDITED_NAME.as_bytes());
        assert_eq!(changed.name[HUE_NAME_LEN - 1], 0, "a new name replaces the whole field");
    }

    #[test]
    fn each_file_keeps_its_own_changes_and_discarding_takes_them_back() {
        let mut app = app_with_open_files();
        let entry = *current(&app, HuesSource::CcMul).hue(EDITED_HUE).unwrap();
        store(&mut app, HuesSource::CcMul, edited(entry));
        assert!((CLASSIC_TAB_EDITOR.is_dirty)(&app));
        assert!(!(ENHANCED_TAB_EDITOR.is_dirty)(&app));
        let shown = app.client_data.as_ref().unwrap().hues.clone().unwrap();
        assert_eq!(shown[EDITED_HUE as usize - 1], edited(entry), "the hues the tabs draw with follow the edit");
        assert_ne!(current(&app, HuesSource::CcMul).to_bytes(), file_bytes());

        store(&mut app, HuesSource::EcUop, edited(entry));
        assert!((ENHANCED_TAB_EDITOR.is_dirty)(&app));
        (ENHANCED_TAB_EDITOR.discard)(&mut app);
        assert!(!(ENHANCED_TAB_EDITOR.is_dirty)(&app));
        assert_eq!(current(&app, HuesSource::EcUop).to_bytes(), file_bytes());

        (CLASSIC_TAB_EDITOR.discard)(&mut app);
        assert!(!(CLASSIC_TAB_EDITOR.is_dirty)(&app));
        assert_eq!(current(&app, HuesSource::CcMul).to_bytes(), file_bytes());
        let shown = app.client_data.as_ref().unwrap().hues.clone().unwrap();
        assert_eq!(shown[EDITED_HUE as usize - 1], entry, "and come back with the discard");
    }

    /// The table row of hue `id` as a package stores it.
    fn table_of(package: &UopPackage, id: u32) -> EcHueTable {
        let bitmap = package.unpack_file_by_hash(hue_bitmap_hash(id as u16)).unwrap().unwrap();
        EcHueTable::from_bitmap(&bitmap).unwrap()
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_hues_mul_saves_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let Some(path) = find_file_ignoring_case(&root, HUES_MUL_NAME) else {
                println!("{}: no {HUES_MUL_NAME}", root.display());
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            let mut hues = HuesMul::load(&path).unwrap();
            let output = output_path(HUES_MUL_NAME);
            hues.save(&output).unwrap();
            assert!(std::fs::read(&output).unwrap() == bytes, "{}: a save without changes differs", root.display());

            let entry = edited(*hues.hue(EDITED_HUE).unwrap());
            *hues.hue_mut(EDITED_HUE).unwrap() = entry;
            hues.save(&output).unwrap();
            let reopened = HuesMul::load(&output).unwrap();
            assert_eq!(reopened.hue(EDITED_HUE), Some(&entry));
            assert_eq!(reopened.hue_count(), hues.hue_count());
            std::fs::remove_file(&output).unwrap();
            println!("{}: same bytes; the edit of a hue comes back ({} hues)", root.display(), hues.hue_count());
        }
    }

    #[test]
    #[ignore = "needs real Enhanced clients: set UO_EC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_hues_uop_saves_the_same_bytes_and_an_edit_in_either_ramp_comes_back_after_reopening() {
        for root in client_roots(EC_CLIENT_ROOTS_ENV) {
            let Some(path) = find_file_ignoring_case(&root, HUES_UOP_NAME) else {
                println!("{}: no {HUES_UOP_NAME}", root.display());
                continue;
            };
            let package = UopPackage::load(&path).unwrap();
            let loaded = hues_mul_from_uop_package(&package).unwrap().hues;
            let output = output_path(HUES_UOP_NAME);
            write_package_copy(&path, &loaded, &loaded, HueRampMode::Linear, &output).unwrap();
            assert!(std::fs::read(&output).unwrap() == std::fs::read(&path).unwrap(), "{}: a save without changes differs", root.display());

            let mut renamed = loaded.clone();
            set_name(renamed.hue_mut(EDITED_HUE).unwrap(), EDITED_NAME);
            write_package_copy(&path, &loaded, &renamed, HueRampMode::Flat, &output).unwrap();
            let saved = UopPackage::load(&output).unwrap();
            assert_eq!(hues_mul_from_uop_package(&saved).unwrap().hues.hue(EDITED_HUE).unwrap().name_bytes(), EDITED_NAME.as_bytes());
            assert_eq!(table_of(&saved, EDITED_HUE).entries, table_of(&package, EDITED_HUE).entries, "a new name leaves the table alone");

            for ramp in [HueRampMode::Linear, HueRampMode::Flat] {
                let mut current = loaded.clone();
                let entry = edited(*current.hue(EDITED_HUE).unwrap());
                *current.hue_mut(EDITED_HUE).unwrap() = entry;
                write_package_copy(&path, &loaded, &current, ramp, &output).unwrap();

                let saved = UopPackage::load(&output).unwrap();
                let reopened = hues_mul_from_uop_package(&saved).unwrap().hues;
                assert_eq!(reopened.hue(EDITED_HUE).unwrap().color_table[EDITED_COLOR] & !COLOR_FLAG_BIT, EDITED_VALUE);
                assert_eq!(reopened.hue(EDITED_HUE).unwrap().name_bytes(), EDITED_NAME.as_bytes());
                assert_eq!(reopened.hue(OTHER_HUE).unwrap().color_table, loaded.hue(OTHER_HUE).unwrap().color_table);
                assert_eq!(table_of(&saved, OTHER_HUE).entries, table_of(&package, OTHER_HUE).entries, "another hue keeps its table");
                assert_eq!(saved.iter_files().count(), package.iter_files().count(), "no entry is added");

                let table = table_of(&saved, EDITED_HUE);
                assert_eq!(table.entries, hue_palette_row(&entry.color_table, ramp));
                let [red, green, blue, _] = expand_hue_color(EDITED_VALUE);
                let after = table.entries[TEXEL_AFTER_EDITED_COLOR];
                assert_eq!(ramp == HueRampMode::Flat, after[..3] == [red, green, blue], "{ramp:?}");
            }
            std::fs::remove_file(&output).unwrap();
            println!("{}: same bytes; the edit comes back in both ramps ({} hues)", root.display(), loaded.hue_count());
        }
    }

    #[test]
    #[ignore = "needs real Classic and Enhanced clients: set both root variables and TMPDIR, and run with --ignored"]
    fn a_shipped_hues_mul_converts_to_hues_uop_and_back_the_same() {
        let package_path = client_roots(EC_CLIENT_ROOTS_ENV).iter().find_map(|root| find_file_ignoring_case(root, HUES_UOP_NAME)).unwrap();
        let base = UopPackage::load(&package_path).unwrap();
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let Some(path) = find_file_ignoring_case(&root, HUES_MUL_NAME) else {
                continue;
            };
            let bytes = std::fs::read(&path).unwrap();
            let hues = HuesMul::parse(&bytes);
            let output = output_path("converted-Hues.uop");
            for (ramp, on_base) in [(HueRampMode::Linear, None), (HueRampMode::Flat, None), (HueRampMode::Linear, Some(&base))] {
                write_converted_package(&hues, on_base, ramp, &output).unwrap();
                let converted = UopPackage::load(&output).unwrap();
                let back = hues_mul_from_uop_package(&converted).unwrap();
                assert!(back.hues.to_bytes() == bytes, "{} {ramp:?}: not the same hues.mul", root.display());
                assert_eq!(back.hues.hue(1).unwrap().color_table.len(), HUE_COLOR_COUNT);
            }
            std::fs::remove_file(&output).unwrap();
            println!("{}: back to the same hues.mul from both ramps and on the loaded package", root.display());
        }
    }
}
