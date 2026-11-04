//! `Edit` of the Multimap tab: replaces the loaded `Multimap.rle` with an image, and saves a copy through
//! `uocf::classic::multimap_rle`.

use std::path::Path;
use std::sync::Arc;

use color_eyre::eyre;
use eframe::egui;
use uocf::classic::multimap_rle::{self, MultimapRleImage};

use crate::app::{MultimapPreviewSelection, UopInspectorApp, ViewMode};
use crate::ui::editor_bar::{editor_bar, pick_output, report_saved, BarAction};
use crate::ui::tiledata_editor::same_file;
use crate::ui::unsaved_changes::TabEditor;

const MULTIMAP_FILE: &str = "Multimap.rle";
const RLE_FILTER: (&str, &[&str]) = ("Multimap RLE", &["rle"]);
const IMAGE_FILTER: (&str, &[&str]) = ("BMP or PNG image", &["png", "bmp"]);
const EDIT_HOVER: &str = "Replace the loaded Multimap.rle with a BMP or PNG image and save a copy. An image becomes black and white as the file stores it.";

#[derive(Default)]
pub struct MultimapEditor {
    pub enabled: bool,
    /// The image as it was before the first change since loading or saving; `Some` is the one unsaved state.
    opened: Option<Arc<MultimapRleImage>>,
    error: Option<String>,
}

impl MultimapEditor {
    pub(super) fn is_dirty(&self) -> bool {
        self.opened.is_some()
    }
}

/// Takes `image` as the loaded one, keeping the file as loaded for the discard.
fn replace(app: &mut UopInspectorApp, image: MultimapRleImage) {
    let Some(current) = app.cc_multimap.clone() else {
        return;
    };
    let opened = app.multimap_editor.opened.take().unwrap_or(current);
    app.multimap_editor.opened = (*opened != image).then_some(opened);
    app.cc_multimap = Some(Arc::new(image));
    app.multimap_texture = None;
    app.multimap_preview_selection = MultimapPreviewSelection::Loaded;
}

fn revert(app: &mut UopInspectorApp) {
    if let Some(opened) = app.multimap_editor.opened.take() {
        app.cc_multimap = Some(opened);
        app.multimap_texture = None;
    }
}

/// Writes the loaded image. The file it was read from is refused.
fn write_copy(app: &UopInspectorApp, output: &Path) -> eyre::Result<()> {
    let image = app.cc_multimap.as_ref().ok_or_else(|| eyre::eyre!("{MULTIMAP_FILE} is not loaded"))?;
    if app.cc_multimap_path.as_deref().is_some_and(|source| same_file(source, output)) {
        eyre::bail!("the loaded {MULTIMAP_FILE} is refused as the target: save a copy");
    }
    multimap_rle::save_rle(output, image)
}

pub(super) fn save(app: &mut UopInspectorApp) {
    let Some(output) = pick_output(MULTIMAP_FILE, RLE_FILTER) else {
        return;
    };
    match write_copy(app, &output) {
        Ok(()) => {
            app.multimap_editor.opened = None;
            app.multimap_editor.error = None;
            report_saved(app, &output);
        }
        Err(error) => app.multimap_editor.error = Some(format!("Save failed: {error:#}")),
    }
}

pub(super) const TAB_EDITOR: TabEditor = TabEditor {
    tab: ViewMode::Multimap,
    label: MULTIMAP_FILE,
    is_dirty: |app| app.multimap_editor.is_dirty(),
    save,
    discard: revert,
};

fn import_image(app: &mut UopInspectorApp) {
    let (filter, extensions) = IMAGE_FILTER;
    let Some(path) = crate::dialog::file_dialog().add_filter(filter, extensions).pick_file() else {
        return;
    };
    match multimap_rle::load_bitmap_or_png(&path) {
        Ok(image) => {
            replace(app, image);
            app.multimap_editor.error = None;
        }
        Err(error) => app.multimap_editor.error = Some(format!("{error:#}")),
    }
}

/// `Edit` and the rest of the bar.
pub(super) fn bar(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &mut app.multimap_editor;
    let dirty = state.is_dirty();
    let action = ui.horizontal_wrapped(|ui| editor_bar(ui, &mut state.enabled, EDIT_HOVER, dirty, "Save Multimap.rle...", &mut state.error)).inner;
    match action {
        Some(BarAction::Revert) => revert(app),
        Some(BarAction::Save) => save(app),
        None => {}
    }
    if !app.multimap_editor.enabled {
        return;
    }
    if ui.add_enabled(app.cc_multimap.is_some(), egui::Button::new("Import image...")).on_disabled_hover_text("Open a Multimap.rle first.").clicked() {
        import_image(app);
    }
    if let (Some(opened), Some(image)) = (&app.multimap_editor.opened, &app.cc_multimap) {
        if (opened.width, opened.height) != (image.width, image.height) {
            ui.weak(format!("{} x {} replaces a file of {} x {}.", image.width, image.height, opened.width, opened.height));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::{client_roots, output_path, CLASSIC_CLIENT_ROOTS_ENV};
    use uocf::classic::multimap_rle::{BLACK_PIXEL, WHITE_PIXEL};
    use uocf::utils::path::find_file_ignoring_case;

    const SIDE: u32 = 2;

    fn image(first: u8) -> MultimapRleImage {
        MultimapRleImage::new(SIDE, SIDE, vec![first, BLACK_PIXEL, BLACK_PIXEL, BLACK_PIXEL]).unwrap()
    }

    #[test]
    fn a_change_is_unsaved_and_discarding_takes_it_back() {
        let mut app = test_app();
        app.cc_multimap = Some(Arc::new(image(BLACK_PIXEL)));
        replace(&mut app, image(WHITE_PIXEL));
        assert!((TAB_EDITOR.is_dirty)(&app));
        assert_eq!(*app.cc_multimap.clone().unwrap(), image(WHITE_PIXEL));

        (TAB_EDITOR.discard)(&mut app);
        assert!(!(TAB_EDITOR.is_dirty)(&app));
        assert_eq!(*app.cc_multimap.clone().unwrap(), image(BLACK_PIXEL));
    }

    #[test]
    fn importing_the_image_as_loaded_is_no_change() {
        let mut app = test_app();
        app.cc_multimap = Some(Arc::new(image(BLACK_PIXEL)));
        replace(&mut app, image(WHITE_PIXEL));
        replace(&mut app, image(BLACK_PIXEL));
        assert!(!(TAB_EDITOR.is_dirty)(&app));
    }

    #[test]
    fn an_image_file_is_read_back_as_it_was_written() {
        let path = output_path("multimap-import.png");
        multimap_rle::save_bitmap_or_png(&path, &image(WHITE_PIXEL)).unwrap();
        assert_eq!(multimap_rle::load_bitmap_or_png(&path).unwrap(), image(WHITE_PIXEL));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn the_loaded_file_is_refused_as_the_target() {
        let source = output_path("refused-Multimap.rle");
        std::fs::write(&source, []).unwrap();
        let mut app = test_app();
        app.cc_multimap = Some(Arc::new(image(BLACK_PIXEL)));
        app.cc_multimap_path = Some(source.clone());
        assert!(write_copy(&app, &source).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), Vec::<u8>::new(), "nothing was written");
        std::fs::remove_file(&source).unwrap();
    }

    #[test]
    #[ignore = "needs real Classic clients: set UO_CLASSIC_CLIENT_ROOTS and TMPDIR, and run with --ignored"]
    fn a_shipped_multimap_saves_the_same_bytes_and_an_edit_comes_back_after_reopening() {
        for root in client_roots(CLASSIC_CLIENT_ROOTS_ENV) {
            let Some(source) = find_file_ignoring_case(&root, MULTIMAP_FILE) else {
                println!("{}: no {MULTIMAP_FILE}", root.display());
                continue;
            };
            let mut app = test_app();
            app.cc_multimap = Some(Arc::new(multimap_rle::load_rle(&source).unwrap()));
            app.cc_multimap_path = Some(source.clone());
            let output = output_path(MULTIMAP_FILE);
            write_copy(&app, &output).unwrap();
            assert!(std::fs::read(&output).unwrap() == std::fs::read(&source).unwrap(), "{}: differs after a save without changes", root.display());

            let loaded = app.cc_multimap.clone().unwrap();
            let mut pixels = loaded.pixels.clone();
            pixels[0] = if pixels[0] == BLACK_PIXEL { WHITE_PIXEL } else { BLACK_PIXEL };
            let edited = MultimapRleImage::new(loaded.width, loaded.height, pixels).unwrap();
            replace(&mut app, edited.clone());
            write_copy(&app, &output).unwrap();
            assert_eq!(multimap_rle::load_rle(&output).unwrap(), edited);
            std::fs::remove_file(&output).unwrap();
            println!("{}: same bytes; an edit comes back ({} x {})", root.display(), edited.width, edited.height);
        }
    }
}
