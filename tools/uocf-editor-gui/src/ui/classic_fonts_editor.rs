//! Editing controls of the Fonts tab, for `fonts.mul` and `unifont*.mul` through `uocf::classic::fonts`.
//!
//! A font file is read for editing when it is first shown with `Edit` on, and every font file with changes is kept
//! until it is saved or the changes are discarded. Each of them is saved to a copy of its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre;
use eframe::egui;
use uocf::classic::fonts::{
    unicode_font_file_name, AsciiFont, AsciiFontGlyph, AsciiFontsFile, UnicodeFontData, UnicodeFontGlyph, ASCII_GLYPH_COUNT,
    ASCII_PRINTABLE_START,
};
use uocf::utils::path::find_file_ignoring_case;

use super::classic_fonts::{FontChoice, FOREGROUND};
use crate::app::{UocfEditorApp, ViewMode};
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction, MUL_FILTER};
use crate::ui::image_export::export_rgba_png;
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const ASCII_FILE_NAME: &str = "fonts.mul";
const FIRST_ASCII_CODE: u32 = ASCII_PRINTABLE_START as u32;
const LAST_ASCII_CODE: u32 = FIRST_ASCII_CODE + ASCII_GLYPH_COUNT as u32 - 1;
const LAST_CODE_POINT: u32 = u16::MAX as u32;
const CODE_POINT_HEX_DIGITS: usize = 4;
/// The width and height of a glyph added on an empty code point.
const NEW_GLYPH_SIDE: u8 = 8;
const DRAG_SPEED: f64 = 0.1;
const CELL_SIZE: f32 = 12.0;
const CELL_GAP: f32 = 1.0;
const EMPTY_CELL: egui::Color32 = egui::Color32::from_gray(45);
const DRAWN_CELL: egui::Color32 = egui::Color32::WHITE;
const GLYPH_PANEL_MAX_HEIGHT: f32 = 300.0;
const IMAGE_FILTER: (&str, &[&str]) = ("Image", &["png", "bmp", "tga"]);
const EDIT_HOVER: &str = "Change the glyphs of the font files and save copies. Click a glyph, then draw it, set its fields, or import an image.";
const DRAW_HELP: &str = "Left button draws, right button erases.";
const UNKNOWN_FIELD_HOVER: &str = "Its meaning is not known. It is kept as it is unless you change it.";
const SAMPLE_NOTE: &str = "The sample shows the loaded file, not the changes.";

/// A font file read for editing.
struct EditedFile<T> {
    source: PathBuf,
    data: T,
    /// The file as it was before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<T>,
}

impl<T: Clone> EditedFile<T> {
    fn new(source: PathBuf, data: T) -> Self {
        Self { source, data, opened: None }
    }

    fn change(&mut self, change: impl FnOnce(&mut T)) {
        if self.opened.is_none() {
            self.opened = Some(self.data.clone());
        }
        change(&mut self.data);
    }

    fn revert(&mut self) {
        if let Some(opened) = self.opened.take() {
            self.data = opened;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FontFile {
    Ascii,
    Unicode(usize),
}

impl FontFile {
    fn name(self) -> String {
        match self {
            Self::Ascii => ASCII_FILE_NAME.to_owned(),
            Self::Unicode(font) => unicode_font_file_name(font),
        }
    }
}

#[derive(Default)]
pub struct FontsEditor {
    pub(super) enabled: bool,
    /// The folder the files were read from.
    root: Option<PathBuf>,
    ascii: Option<EditedFile<AsciiFontsFile>>,
    unicode: BTreeMap<usize, EditedFile<UnicodeFontData>>,
    /// The chosen glyph: its character or code point.
    selected: u32,
    /// The colour an `fonts.mul` glyph is drawn with.
    paint: [u8; 3],
    /// Grows with every change, for the tab to redraw its glyphs.
    generation: u64,
    error: Option<String>,
}

impl FontsEditor {
    pub(super) fn ascii_font(&self, font: usize) -> Option<&AsciiFont> {
        self.ascii.as_ref()?.data.fonts.get(font)
    }

    pub(super) fn unicode_file(&self, font: usize) -> Option<&UnicodeFontData> {
        Some(&self.unicode.get(&font)?.data)
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn selected(&self) -> u32 {
        self.selected
    }

    pub(super) fn select(&mut self, code: u32) {
        self.selected = code;
    }

    fn is_dirty(&self) -> bool {
        self.next_unsaved().is_some()
    }

    fn next_unsaved(&self) -> Option<FontFile> {
        if self.ascii.as_ref().is_some_and(|file| file.opened.is_some()) {
            return Some(FontFile::Ascii);
        }
        self.unicode.iter().find(|(_, file)| file.opened.is_some()).map(|(&font, _)| FontFile::Unicode(font))
    }

    /// Lets go of the files without changes, for them to be read again.
    pub(super) fn forget_unchanged(&mut self) {
        self.ascii.take_if(|file| file.opened.is_none());
        self.unicode.retain(|_, file| file.opened.is_some());
    }

    fn revert(&mut self) {
        if let Some(file) = self.ascii.as_mut() {
            file.revert();
        }
        self.unicode.values_mut().for_each(EditedFile::revert);
        self.generation += 1;
    }

    /// Reads the file of `choice` when it is not open yet.
    fn open(&mut self, root: &Path, choice: FontChoice) -> eyre::Result<()> {
        if self.root.as_deref() != Some(root) && !self.is_dirty() {
            self.ascii = None;
            self.unicode.clear();
            self.root = Some(root.to_path_buf());
            self.generation += 1;
        }
        let find = |name: &str| find_file_ignoring_case(root, name).ok_or_else(|| eyre::eyre!("no {name} in {}", root.display()));
        match choice {
            FontChoice::Ascii(_) if self.ascii.is_none() => {
                let path = find(ASCII_FILE_NAME)?;
                let data = AsciiFontsFile::load(&path)?;
                self.ascii = Some(EditedFile::new(path, data));
                self.generation += 1;
            }
            FontChoice::Unicode(font) if !self.unicode.contains_key(&font) => {
                let path = find(&unicode_font_file_name(font))?;
                let data = UnicodeFontData::load(&path)?;
                self.unicode.insert(font, EditedFile::new(path, data));
                self.generation += 1;
            }
            _ => {}
        }
        Ok(())
    }

    /// Writes `file` to `output`, which must not be the file it was read from.
    fn write_copy(&self, file: FontFile, output: &Path) -> eyre::Result<()> {
        let refuse = |source: &Path| {
            if same_file(source, output) {
                eyre::bail!("the loaded {} is refused as the target: save a copy", file.name());
            }
            Ok(())
        };
        match file {
            FontFile::Ascii => {
                let edited = self.ascii.as_ref().ok_or_else(|| eyre::eyre!("{ASCII_FILE_NAME} is not open"))?;
                refuse(&edited.source)?;
                edited.data.save(output)
            }
            FontFile::Unicode(font) => {
                let edited = self.unicode.get(&font).ok_or_else(|| eyre::eyre!("{} is not open", file.name()))?;
                refuse(&edited.source)?;
                edited.data.save(output)
            }
        }
    }

    fn mark_saved(&mut self, file: FontFile) {
        match file {
            FontFile::Ascii => {
                if let Some(edited) = self.ascii.as_mut() {
                    edited.opened = None;
                }
            }
            FontFile::Unicode(font) => {
                if let Some(edited) = self.unicode.get_mut(&font) {
                    edited.opened = None;
                }
            }
        }
    }

    fn change_ascii(&mut self, font: usize, index: usize, header: u8, glyph: AsciiFontGlyph) {
        let Some(file) = self.ascii.as_mut() else {
            return;
        };
        file.change(|data| {
            if let Some(face) = data.fonts.get_mut(font) {
                face.header = header;
                if let Some(slot) = face.glyphs.get_mut(index) {
                    *slot = glyph;
                }
            }
        });
        self.generation += 1;
    }

    fn change_unicode(&mut self, font: usize, code: u16, glyph: Option<UnicodeFontGlyph>) {
        let Some(file) = self.unicode.get_mut(&font) else {
            return;
        };
        let mut result = Ok(());
        file.change(|data| result = data.set_glyph(code, glyph));
        if let Err(error) = result {
            self.error = Some(format!("{error:#}"));
        }
        self.generation += 1;
    }

    /// The fields and pixels of the chosen glyph of `choice`.
    pub(super) fn glyph_panel(&mut self, ui: &mut egui::Ui, choice: FontChoice) {
        egui::ScrollArea::both().id_salt("font_glyph_panel").max_height(GLYPH_PANEL_MAX_HEIGHT).show(ui, |ui| match choice {
            FontChoice::Ascii(font) => self.ascii_panel(ui, font),
            FontChoice::Unicode(font) => self.unicode_panel(ui, font),
        });
    }

    fn ascii_panel(&mut self, ui: &mut egui::Ui, font: usize) {
        let code = self.selected.clamp(FIRST_ASCII_CODE, LAST_ASCII_CODE);
        let index = (code - FIRST_ASCII_CODE) as usize;
        let Some(face) = self.ascii_font(font) else {
            return;
        };
        let (mut header, before_header) = (face.header, face.header);
        let Some(before) = face.glyphs.get(index).cloned() else {
            return;
        };
        let mut glyph = before.clone();
        let mut chosen = code;
        let (mut width, mut height) = (glyph.width, glyph.height);
        ui.horizontal_wrapped(|ui| {
            ui.label("Character:");
            ui.add(egui::DragValue::new(&mut chosen).range(FIRST_ASCII_CODE..=LAST_ASCII_CODE).speed(DRAG_SPEED));
            size_fields(ui, &mut width, &mut height);
            ui.label("Byte 3:").on_hover_text(UNKNOWN_FIELD_HOVER);
            ui.add(egui::DragValue::new(&mut glyph.unknown).speed(DRAG_SPEED));
            ui.label("Face header:").on_hover_text(UNKNOWN_FIELD_HOVER);
            ui.add(egui::DragValue::new(&mut header).speed(DRAG_SPEED));
        });
        glyph.resize(width, height);
        ui.horizontal(|ui| {
            ui.label("Colour:");
            ui.color_edit_button_srgb(&mut self.paint);
            if ui.button("Import image...").clicked() {
                match pick_image() {
                    Ok(Some((width, height, rgba))) => {
                        if let Err(error) = glyph.set_rgba8(width, height, &rgba) {
                            self.error = Some(format!("{error:#}"));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            if ui.button("Export PNG...").clicked() {
                let image = glyph.to_transparent_rgba8();
                self.export(format!("font_{font}_char_{code}.png"), image.width, image.height, &image.rgba);
            }
            ui.weak(DRAW_HELP);
        });
        let image = glyph.to_transparent_rgba8();
        let at = |x: usize, y: usize| {
            let start = (y * image.width as usize + x) * size_of::<u32>();
            let pixel = &image.rgba[start..start + size_of::<u32>()];
            if pixel[3] == 0 { EMPTY_CELL } else { egui::Color32::from_rgb(pixel[0], pixel[1], pixel[2]) }
        };
        if let Some((x, y, draw)) = pixel_grid(ui, image.width as usize, image.height as usize, at) {
            glyph.pixels[y * image.width as usize + x] = if draw { AsciiFontGlyph::pixel_of_rgb(self.paint) } else { uocf::classic::gump_write::TRANSPARENT };
        }
        self.selected = chosen;
        if glyph != before || header != before_header {
            self.change_ascii(font, index, header, glyph);
        }
    }

    fn unicode_panel(&mut self, ui: &mut egui::Ui, font: usize) {
        let code = self.selected.min(LAST_CODE_POINT) as u16;
        let Some(file) = self.unicode_file(font) else {
            return;
        };
        let current = file.glyph(code).cloned();
        let mut chosen = u32::from(code);
        ui.horizontal(|ui| {
            ui.label("Code point:");
            ui.add(egui::DragValue::new(&mut chosen).range(0..=LAST_CODE_POINT).hexadecimal(CODE_POINT_HEX_DIGITS, false, true).prefix("U+").speed(DRAG_SPEED));
        });
        self.selected = chosen;
        let Some(before) = current else {
            if ui.button("Add a glyph").on_hover_text("This code point has no glyph.").clicked() {
                self.change_unicode(font, code, Some(UnicodeFontGlyph::blank(NEW_GLYPH_SIDE, NEW_GLYPH_SIDE)));
            }
            return;
        };
        let mut glyph = before.clone();
        let (mut width, mut height) = (glyph.width, glyph.height);
        let mut remove = false;
        ui.horizontal_wrapped(|ui| {
            size_fields(ui, &mut width, &mut height);
            ui.label("Offset x:");
            ui.add(egui::DragValue::new(&mut glyph.offset_x).speed(DRAG_SPEED));
            ui.label("Offset y:");
            ui.add(egui::DragValue::new(&mut glyph.offset_y).speed(DRAG_SPEED));
            remove = ui.button("Remove glyph").clicked();
        });
        glyph.resize(width, height);
        ui.horizontal(|ui| {
            if ui.button("Import image...").clicked() {
                match pick_image() {
                    Ok(Some((width, height, rgba))) => {
                        if let Err(error) = glyph.set_rgba8(width, height, &rgba) {
                            self.error = Some(format!("{error:#}"));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            if ui.button("Export PNG...").clicked() {
                match glyph.to_rgba8(FOREGROUND, None) {
                    Ok(image) => self.export(format!("unifont_{font}_{code:04X}.png"), image.width, image.height, &image.rgba),
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
            ui.weak(DRAW_HELP);
        });
        let shown = glyph.clone();
        let at = |x: usize, y: usize| if shown.bit_at(x, y).unwrap_or(false) { DRAWN_CELL } else { EMPTY_CELL };
        if let Some((x, y, draw)) = pixel_grid(ui, glyph.width as usize, glyph.height as usize, at) {
            glyph.set_bit(x, y, draw);
        }
        if remove {
            self.change_unicode(font, code, None);
        } else if glyph != before {
            self.change_unicode(font, code, Some(glyph));
        }
    }

    fn export(&mut self, name: String, width: u32, height: u32, rgba: &[u8]) {
        if rgba.is_empty() {
            self.error = Some("This glyph has no pixels to export.".to_owned());
            return;
        }
        if let Err(error) = export_rgba_png(name, width, height, rgba) {
            self.error = Some(error);
        }
    }
}

/// The width and height fields of a glyph.
fn size_fields(ui: &mut egui::Ui, width: &mut u8, height: &mut u8) {
    ui.label("Width:");
    ui.add(egui::DragValue::new(width).speed(DRAG_SPEED));
    ui.label("Height:");
    ui.add(egui::DragValue::new(height).speed(DRAG_SPEED));
}

/// Asks for an image; its width, height and RGBA pixels, None when the user cancels.
fn pick_image() -> Result<Option<(u8, u8, Vec<u8>)>, String> {
    let (filter, extensions) = IMAGE_FILTER;
    let Some(path) = crate::dialog::file_dialog().add_filter(filter, extensions).pick_file() else {
        return Ok(None);
    };
    read_glyph_image(&path).map(Some)
}

fn read_glyph_image(path: &Path) -> Result<(u8, u8, Vec<u8>), String> {
    let image = image::open(path).map_err(|error| format!("{}: {error}", path.display()))?.to_rgba8();
    let (width, height) = image.dimensions();
    let size = u8::try_from(width).ok().zip(u8::try_from(height).ok());
    let (width, height) = size.ok_or_else(|| format!("{}: {width} x {height} is too large for a glyph ({} x {} at most)", path.display(), u8::MAX, u8::MAX))?;
    Ok((width, height, image.into_raw()))
}

/// Draws a glyph's cells. The cell under the pointer while a button is held or clicked: its x and y, and whether it is
/// the drawing button.
fn pixel_grid(ui: &mut egui::Ui, width: usize, height: usize, color_at: impl Fn(usize, usize) -> egui::Color32) -> Option<(usize, usize, bool)> {
    let size = egui::vec2(width as f32, height as f32) * CELL_SIZE;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    for y in 0..height {
        for x in 0..width {
            let corner = rect.min + egui::vec2(x as f32, y as f32) * CELL_SIZE;
            let cell = egui::Rect::from_min_size(corner, egui::Vec2::splat(CELL_SIZE - CELL_GAP));
            painter.rect_filled(cell, 0.0, color_at(x, y));
        }
    }
    let (primary, secondary) = ui.input(|input| (input.pointer.primary_down(), input.pointer.secondary_down()));
    let draw = primary || response.clicked();
    let erase = secondary || response.secondary_clicked();
    if !draw && !erase {
        return None;
    }
    let pointer = response.interact_pointer_pos().filter(|pointer| rect.contains(*pointer))?;
    let offset = (pointer - rect.min) / CELL_SIZE;
    Some((offset.x as usize, offset.y as usize, draw))
}

fn editor(app: &mut UocfEditorApp) -> &mut FontsEditor {
    &mut app.classic_file_views.fonts.editor
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::ClassicFonts,
    label: "the font files",
    is_dirty: |app| app.classic_file_views.fonts.editor.is_dirty(),
    save,
    discard: |app| editor(app).revert(),
};

/// Asks where to write each file with changes, one after the other, until none is left or the user cancels.
fn save(app: &mut UocfEditorApp) {
    while let Some(file) = editor(app).next_unsaved() {
        let Some(output) = pick_output(&file.name(), MUL_FILTER) else {
            return;
        };
        if let Err(error) = editor(app).write_copy(file, &output) {
            editor(app).error = Some(format!("Save failed: {error:#}"));
            return;
        }
        editor(app).mark_saved(file);
        editor(app).error = None;
        report_saved(app, &output);
    }
}

/// `Edit` and the rest of the bar, and the file of the chosen font read for it.
pub(super) fn bar(app: &mut UocfEditorApp, ui: &mut egui::Ui, choice: FontChoice) {
    let editor_state = editor(app);
    let dirty = editor_state.is_dirty();
    let action = editor_bar(ui, &mut editor_state.enabled, EDIT_HOVER, dirty, "Save font files...", &mut editor_state.error);
    match action {
        Some(BarAction::Revert) => editor(app).revert(),
        Some(BarAction::Save) => save(app),
        None => {}
    }
    if !editor(app).enabled {
        return;
    }
    let Some(root) = app.settings.cc_path.clone() else {
        return;
    };
    if let Err(error) = editor(app).open(&root, choice) {
        editor(app).error = Some(format!("{error:#}"));
        editor(app).enabled = false;
    }
    if matches!(choice, FontChoice::Unicode(_)) {
        ui.weak(SAMPLE_NOTE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV};
    use uocf::classic::fonts::UNICODE_FONT_FILES_MAX;

    const EDITED_INDEX: usize = 3;
    const EDITED_PIXEL: u16 = 0x1234;
    const EDITED_CODE: u16 = 0x0041;
    /// Hopefully past every shipped font's glyphs; the test looks for an absent one from here on.
    const FIRST_ADDED_CODE: u16 = 0xE000;

    fn small_ascii_file() -> AsciiFontsFile {
        let glyph = AsciiFontGlyph { width: 2, height: 1, unknown: 0, pixels: vec![0, 1] };
        AsciiFontsFile { fonts: vec![AsciiFont { header: 0, glyphs: vec![glyph; ASCII_GLYPH_COUNT] }], tail: vec![1, 2] }
    }

    fn with_open_files(app: &mut UocfEditorApp) {
        let state = editor(app);
        state.ascii = Some(EditedFile::new(PathBuf::new(), small_ascii_file()));
        state.unicode.insert(0, EditedFile::new(PathBuf::new(), UnicodeFontData::default()));
    }

    fn edited_ascii(glyph: &AsciiFontGlyph) -> AsciiFontGlyph {
        let mut edited = glyph.clone();
        edited.pixels[0] = EDITED_PIXEL;
        edited
    }

    #[test]
    fn a_change_in_either_file_is_unsaved_and_discarding_takes_every_change_back() {
        let mut app = test_app();
        with_open_files(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));

        let glyph = editor(&mut app).ascii_font(0).unwrap().glyphs[EDITED_INDEX].clone();
        let before = editor(&mut app).generation();
        editor(&mut app).change_ascii(0, EDITED_INDEX, 0, edited_ascii(&glyph));
        assert_ne!(editor(&mut app).generation(), before, "the tab redraws its glyphs");
        assert_eq!(editor(&mut app).next_unsaved(), Some(FontFile::Ascii));
        editor(&mut app).mark_saved(FontFile::Ascii);
        assert!(!(TAB_EDITOR.is_dirty)(&app));

        editor(&mut app).change_ascii(0, EDITED_INDEX, 0, glyph.clone());
        editor(&mut app).change_unicode(0, EDITED_CODE, Some(UnicodeFontGlyph::blank(NEW_GLYPH_SIDE, NEW_GLYPH_SIDE)));
        assert!((TAB_EDITOR.is_dirty)(&app));

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(editor(&mut app).unicode_file(0).unwrap().glyph(EDITED_CODE), None);
        assert_eq!(editor(&mut app).ascii_font(0).unwrap().glyphs[EDITED_INDEX], edited_ascii(&glyph), "as last saved");
    }

    #[test]
    fn a_read_again_keeps_only_the_files_with_changes() {
        let mut app = test_app();
        with_open_files(&mut app);
        editor(&mut app).change_unicode(0, EDITED_CODE, None);
        editor(&mut app).forget_unchanged();
        assert!(editor(&mut app).ascii.is_none());
        assert!(editor(&mut app).unicode_file(0).is_some());
    }

    #[test]
    fn a_wrong_bitmap_is_reported_and_not_stored() {
        let mut app = test_app();
        with_open_files(&mut app);
        let mut glyph = UnicodeFontGlyph::blank(NEW_GLYPH_SIDE, NEW_GLYPH_SIDE);
        glyph.bitmap.clear();
        editor(&mut app).change_unicode(0, EDITED_CODE, Some(glyph));
        assert!(editor(&mut app).error.is_some());
        assert_eq!(editor(&mut app).unicode_file(0).unwrap().glyph(EDITED_CODE), None);
    }

    fn check_ascii(root: &Path) {
        let mut state = FontsEditor::default();
        state.open(root, FontChoice::Ascii(0)).unwrap();
        let source = state.ascii.as_ref().unwrap().source.clone();
        let output = output_path(ASCII_FILE_NAME);
        state.write_copy(FontFile::Ascii, &output).unwrap();
        assert!(std::fs::read(&output).unwrap() == std::fs::read(&source).unwrap(), "{}: {ASCII_FILE_NAME} differs after a save without changes", root.display());

        for (face_index, face) in state.ascii.as_ref().unwrap().data.fonts.iter().enumerate() {
            for (index, glyph) in face.glyphs.iter().enumerate() {
                let image = glyph.to_transparent_rgba8();
                let mut imported = glyph.clone();
                imported.set_rgba8(glyph.width, glyph.height, &image.rgba).unwrap();
                assert_eq!(&imported, glyph, "{}: face {face_index} glyph {index} after an export and an import", root.display());
            }
        }

        let glyph = state.ascii_font(0).unwrap().glyphs.iter().position(|glyph| !glyph.pixels.is_empty()).unwrap();
        let edited = edited_ascii(&state.ascii_font(0).unwrap().glyphs[glyph]);
        state.change_ascii(0, glyph, 0, edited.clone());
        state.write_copy(FontFile::Ascii, &output).unwrap();
        let reopened = AsciiFontsFile::load(&output).unwrap();
        assert_eq!(reopened.fonts[0].glyphs[glyph], edited);
        std::fs::remove_file(&output).unwrap();
    }

    fn check_unicode(root: &Path, font: usize) -> bool {
        let name = unicode_font_file_name(font);
        if find_file_ignoring_case(root, &name).is_none() {
            return false;
        }
        let mut state = FontsEditor::default();
        state.open(root, FontChoice::Unicode(font)).unwrap();
        let source = state.unicode[&font].source.clone();
        let output = output_path(&name);
        state.write_copy(FontFile::Unicode(font), &output).unwrap();
        assert!(std::fs::read(&output).unwrap() == std::fs::read(&source).unwrap(), "{}: {name} differs after a save without changes", root.display());

        let file = state.unicode_file(font).unwrap();
        let code = (0..=u16::MAX).find(|&code| file.glyph(code).is_some_and(|glyph| !glyph.bitmap.is_empty())).unwrap();
        let mut changed = file.glyph(code).unwrap().clone();
        let bit = changed.bit_at(0, 0).unwrap();
        changed.set_bit(0, 0, !bit);
        changed.offset_x = changed.offset_x.wrapping_add(1);
        let added = (FIRST_ADDED_CODE..=u16::MAX).find(|&code| file.glyph(code).is_none()).unwrap();
        state.change_unicode(font, code, Some(changed.clone()));
        state.change_unicode(font, added, Some(UnicodeFontGlyph::blank(NEW_GLYPH_SIDE, NEW_GLYPH_SIDE)));
        state.write_copy(FontFile::Unicode(font), &output).unwrap();
        let reopened = UnicodeFontData::load(&output).unwrap();
        assert_eq!(reopened.glyph(code), Some(&changed));
        assert_eq!(reopened.glyph(added), Some(&UnicodeFontGlyph::blank(NEW_GLYPH_SIDE, NEW_GLYPH_SIDE)));
        std::fs::remove_file(&output).unwrap();
        true
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn shipped_font_files_save_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            check_ascii(&root);
            let unicode = (0..UNICODE_FONT_FILES_MAX).filter(|&font| check_unicode(&root, font)).count();
            println!("{}: {ASCII_FILE_NAME} and {unicode} unifont files save the same bytes; an edit comes back", root.display());
        }
    }

    #[test]
    fn a_too_large_image_is_refused() {
        let path = output_path("too-large-glyph.png");
        let side = u32::from(u8::MAX) + 1;
        image::RgbaImage::new(side, 1).save(&path).unwrap();
        assert!(read_glyph_image(&path).is_err());
        image::RgbaImage::new(1, 1).save(&path).unwrap();
        assert_eq!(read_glyph_image(&path).unwrap().0, 1);
        std::fs::remove_file(&path).unwrap();
    }
}
