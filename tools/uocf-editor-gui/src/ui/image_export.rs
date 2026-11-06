//! The image export every view shares: a menu with the file format (BMP, PNG or WebP) and the hue choice, the
//! image shown saved through a save dialog, and bulk exports into a folder ([`BulkExport`]), written off the UI thread.
//! The files are `uocf::image_export`'s: each source in its own colour format where the file format has one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use uocf::classic::hues::HueEntry;
pub use uocf::image_export::{sanitize_file_stem, ExportImage, ImageFileFormat};

/// The stem of a file named after nothing better.
const FALLBACK_STEM: &str = "image";
/// How often the bulk export window redraws its progress.
const BULK_PROGRESS_REPAINT: Duration = Duration::from_millis(100);
/// Failures a finished bulk export lists; the log has them all.
const LISTED_FAILURES: usize = 5;
const CLASSIC_HUE_FLAG: u16 = 0x8000;

/// The format and hue choice of the export menus, shared by every view.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageExportSettings {
    pub format: ImageFileFormat,
    /// Export the colours the view draws (hued), not the stored ones.
    pub hued: bool,
}

impl ImageExportSettings {
    /// Whether the export takes the view's hue: chosen, and the view hues.
    pub fn takes_hue(&self, menu: &ExportMenu) -> bool {
        self.hued && menu.hue.is_ok()
    }
}

/// A bulk entry of an export menu: the button and its tooltip.
pub struct BulkEntry {
    pub label: String,
    pub hover: String,
}

/// What a view's export menu offers.
pub struct ExportMenu {
    /// `Err(why)` when the view draws no hue (the hued choice greyed, `why` its tooltip).
    pub hue: Result<(), &'static str>,
    /// The image is a Classic animation frame (paletted BMP and PNG).
    pub frame: bool,
    pub bulk: Vec<BulkEntry>,
}

impl ExportMenu {
    pub fn unhued(why: &'static str) -> Self {
        Self { hue: Err(why), frame: false, bulk: Vec::new() }
    }
}

/// The view does not hue the image.
pub const NOT_HUED: &str = "This view draws the colours as stored.";

/// What the user chose in an export menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportClicked {
    Shown,
    Bulk(usize),
}

fn format_hover(format: ImageFileFormat, frame: bool) -> String {
    let text = match format {
        ImageFileFormat::Bmp => {
            "Classic art, gumps, land tiles and texmaps: 16-bit X1R5G5B5, the stored colours bit for bit (in art and \
             gumps, transparent is colour 0, black). Classic animation frames: 8-bit with the frame's palette. Enhanced \
             images: 32-bit with alpha."
        }
        ImageFileFormat::Png => {
            "Lossless RGBA, transparent where the image is; Classic channels widened as (c << 3) | (c >> 2), which \
             imports back to the same colours. Classic animation frames: 8-bit paletted, index 0 transparent."
        }
        ImageFileFormat::WebP => "Lossless RGBA: the same pixels as the PNG.",
    };
    if frame && format == ImageFileFormat::WebP {
        format!("{text}\nWebP has no palette mode in the image crate's encoder: the frame is written as RGBA.")
    } else {
        text.to_string()
    }
}

/// The export menu button. Returns the export chosen this frame; the menu stays open while the format or hue changes.
pub fn export_menu(ui: &mut egui::Ui, settings: &mut ImageExportSettings, menu: &ExportMenu) -> Option<ExportClicked> {
    let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let (_, inner) = egui::containers::menu::MenuButton::new("Export ⏷").config(config).ui(ui, |ui| {
        let mut clicked = None;
        ui.horizontal(|ui| {
            ui.label("Format:");
            for format in ImageFileFormat::ALL {
                let response = ui.add(egui::Button::selectable(settings.format == format, format.label()));
                if response.on_hover_text(format_hover(format, menu.frame)).clicked() {
                    settings.format = format;
                }
            }
        });
        let hue = ui.add_enabled(menu.hue.is_ok(), egui::Checkbox::new(&mut settings.hued, "As shown (hued)"));
        match menu.hue {
            Ok(()) => {
                hue.on_hover_text(
                    "Off: the colours as stored. On: the colours the view draws under its hue (a Classic hue only \
                     changes colours, so the 16-bit and palette formats stay).",
                );
            }
            Err(why) => {
                hue.on_disabled_hover_text(why);
            }
        }
        ui.separator();
        if ui.button(format!("Save image as {}...", settings.format.label())).clicked() {
            clicked = Some(ExportClicked::Shown);
        }
        for (index, entry) in menu.bulk.iter().enumerate() {
            if ui.button(&entry.label).on_hover_text(&entry.hover).clicked() {
                clicked = Some(ExportClicked::Bulk(index));
            }
        }
        if clicked.is_some() {
            ui.close();
        }
        clicked
    });
    inner.and_then(|inner| inner.inner)
}

/// The file stem of an entry or file named by its path (`build/worldart/00001234.dds`): every path separator and
/// dot an underscore (`build_worldart_00001234_dds`), so entries of one name in two folders, or with two
/// extensions, stay two files.
pub fn path_file_stem(path: &str) -> String {
    path.chars().map(|ch| if matches!(ch, '/' | '\\' | '.') { '_' } else { ch }).collect()
}

/// Saves `image` in `format` through a save dialog named `stem`; the path written, `None` when cancelled.
pub fn save_image(stem: &str, image: &ExportImage, format: ImageFileFormat) -> Result<Option<PathBuf>, String> {
    let extension = format.extension();
    let Some(mut path) = crate::dialog::file_dialog()
        .add_filter(format!("{} image", format.label()), &[extension])
        .set_file_name(format!("{}.{extension}", sanitize_file_stem(stem, FALLBACK_STEM)))
        .save_file()
    else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    image.write(&path, format).map_err(|error| format!("{error:#}"))?;
    Ok(Some(path))
}

/// The status line of [`save_image`]'s result.
pub fn save_status(what: &str, result: Result<Option<PathBuf>, String>) -> Option<String> {
    match result {
        Ok(Some(path)) => Some(format!("Exported {what} to {}.", path.display())),
        Ok(None) => None,
        Err(error) => Some(format!("Failed to export {what}: {error}")),
    }
}

/// [`save_image`] of straight-alpha RGBA, for the views whose images have no stored format of their own.
pub fn save_rgba(stem: &str, width: u32, height: u32, rgba: &[u8], format: ImageFileFormat) -> Result<Option<PathBuf>, String> {
    save_image(stem, &ExportImage::Rgba { width, height, rgba: rgba.to_vec() }, format)
}

/// The 15-bit colour a Classic hue makes of `color`, as the views draw it: by the red channel (bit 15 kept).
pub fn classic_hued(hue: &HueEntry, color: u16) -> u16 {
    (hue.apply_to_color16(color | CLASSIC_HUE_FLAG, false) & !CLASSIC_HUE_FLAG) | (color & CLASSIC_HUE_FLAG)
}

/// Hues straight-alpha RGBA as the art view does (transparent pixels kept).
pub fn hue_rgba(hue: &HueEntry, rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let color = u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]);
        let [alpha, red, green, blue] = hue.apply_to_color32(color, false).to_be_bytes();
        pixel.copy_from_slice(&[red, green, blue, alpha]);
    }
}

/// The files of one bulk item: `(stem, image)`, each written as `<stem>.<extension>`.
pub type BulkFiles = Vec<(String, ExportImage)>;

/// How a bulk export ended.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BulkOutcome {
    pub written: usize,
    /// Items without an image.
    pub empty: usize,
    pub failed: Vec<String>,
    pub cancelled: bool,
}

/// A bulk export into a folder, written on a worker thread, with its progress window.
pub struct BulkExport {
    title: String,
    folder: PathBuf,
    total: usize,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<BulkOutcome>>,
    outcome: Option<BulkOutcome>,
}

impl BulkExport {
    /// Starts writing the files `produce` makes of each of `items` into `folder`, in `format`.
    pub fn start<T: Send + 'static>(
        title: String,
        folder: PathBuf,
        format: ImageFileFormat,
        items: Vec<T>,
        produce: impl Fn(T) -> color_eyre::eyre::Result<BulkFiles> + Send + 'static,
    ) -> Self {
        let (done, cancel) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let total = items.len();
        let worker = {
            let (folder, done, cancel) = (folder.clone(), done.clone(), cancel.clone());
            std::thread::spawn(move || write_bulk(&folder, format, items, produce, &done, &cancel))
        };
        log::info!("{title}: {total} items into {}", folder.display());
        Self { title, folder, total, done, cancel, worker: Some(worker), outcome: None }
    }

    pub fn is_running(&self) -> bool {
        self.outcome.is_none()
    }

    /// The progress window; `false` once the user closes it after the end.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if self.worker.as_ref().is_some_and(|worker| worker.is_finished()) {
            let outcome = self.worker.take().expect("checked above").join().unwrap_or_else(|_| BulkOutcome {
                failed: vec!["the export thread panicked (see the log)".to_string()],
                ..BulkOutcome::default()
            });
            log::info!("{}: {outcome:?}", self.title);
            self.outcome = Some(outcome);
        }
        let mut keep = true;
        egui::Window::new(&self.title).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Into {}", self.folder.display()));
            let done = self.done.load(Ordering::Relaxed).min(self.total);
            let share = if self.total == 0 { 1.0 } else { done as f32 / self.total as f32 };
            ui.add(egui::ProgressBar::new(share).text(format!("{done} / {}", self.total)));
            match &self.outcome {
                None => {
                    ctx.request_repaint_after(BULK_PROGRESS_REPAINT);
                    let cancelling = self.cancel.load(Ordering::Relaxed);
                    if ui.add_enabled(!cancelling, egui::Button::new("Cancel")).clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                }
                Some(outcome) => {
                    let ending = if outcome.cancelled { "Cancelled" } else { "Done" };
                    ui.label(format!("{ending}: {} files written, {} without an image.", outcome.written, outcome.empty));
                    if !outcome.failed.is_empty() {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("{} failed (see the log):", outcome.failed.len()));
                        for failure in outcome.failed.iter().take(LISTED_FAILURES) {
                            ui.label(failure);
                        }
                    }
                    keep = !ui.button("Close").clicked();
                }
            }
        });
        keep
    }
}

/// The worker of [`BulkExport`]: every item's files into `folder`, until done or cancelled. The stems are
/// sanitized, so no file lands outside `folder`.
fn write_bulk<T>(
    folder: &Path,
    format: ImageFileFormat,
    items: Vec<T>,
    produce: impl Fn(T) -> color_eyre::eyre::Result<BulkFiles>,
    done: &AtomicUsize,
    cancel: &AtomicBool,
) -> BulkOutcome {
    let mut outcome = BulkOutcome::default();
    for item in items {
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            break;
        }
        match produce(item) {
            Ok(files) if files.is_empty() => outcome.empty += 1,
            Ok(files) => {
                for (stem, image) in files {
                    let name = format!("{}.{}", sanitize_file_stem(&stem, FALLBACK_STEM), format.extension());
                    match image.write(&folder.join(&name), format) {
                        Ok(()) => outcome.written += 1,
                        Err(error) => {
                            log::warn!("bulk export {name}: {error:#}");
                            outcome.failed.push(format!("{name}: {error:#}"));
                        }
                    }
                }
            }
            Err(error) => {
                log::warn!("bulk export: {error:#}");
                outcome.failed.push(format!("{error:#}"));
            }
        }
        done.fetch_add(1, Ordering::Relaxed);
    }
    outcome
}

/// Starts `export` on the app unless one runs, with a folder the user picks; the status line otherwise.
pub fn start_bulk(app: &mut crate::app::UocfEditorApp, start: impl FnOnce(PathBuf) -> BulkExport) {
    if app.bulk_export.as_ref().is_some_and(BulkExport::is_running) {
        app.status_message = "A bulk export is running: wait for it or cancel it first.".to_string();
        return;
    }
    if let Some(folder) = crate::dialog::pick_folder() {
        app.bulk_export = Some(start(folder));
    }
}

/// The bulk export window, while there is one.
pub fn show_bulk_export(app: &mut crate::app::UocfEditorApp, ctx: &egui::Context) {
    if let Some(export) = &mut app.bulk_export {
        if !export.show(ctx) {
            app.bulk_export = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: u32 = 4;
    /// The item that has no image.
    const EMPTY_ITEM: u32 = 2;

    fn one_pixel(id: u32) -> ExportImage {
        ExportImage::Rgba { width: 1, height: 1, rgba: vec![id as u8, 0, 0, u8::MAX] }
    }

    #[test]
    fn a_bulk_export_writes_each_file_into_the_folder_only() {
        let folder = crate::ui::test_support::output_path("bulk_export");
        std::fs::create_dir_all(&folder).unwrap();
        let (done, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
        let produce = |id: u32| {
            Ok(match id {
                EMPTY_ITEM => Vec::new(),
                // A stem with separators stays a file name in the folder.
                0 => vec![("../escape/../land_0x0000".to_string(), one_pixel(id))],
                _ => vec![(uocf::image_export::art_file_stem(id), one_pixel(id))],
            })
        };
        let outcome = write_bulk(&folder, ImageFileFormat::Png, (0..ITEMS).collect(), produce, &done, &cancel);
        assert_eq!(outcome, BulkOutcome { written: 3, empty: 1, failed: Vec::new(), cancelled: false });
        assert_eq!(done.load(Ordering::Relaxed), ITEMS as usize);
        let mut names: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["escapeland_0x0000.png", "land_0x0001.png", "land_0x0003.png"]);
        let decoded = image::open(folder.join("land_0x0003.png")).unwrap().into_rgba8();
        assert_eq!(decoded.into_raw(), [3, 0, 0, u8::MAX]);

        cancel.store(true, Ordering::Relaxed);
        let outcome = write_bulk(&folder, ImageFileFormat::Bmp, (0..ITEMS).collect(), produce, &done, &cancel);
        assert!(outcome.cancelled && outcome.written == 0);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_path_names_its_file_with_separators_and_dots_as_underscores() {
        assert_eq!(path_file_stem("build/worldart/00001234.dds"), "build_worldart_00001234_dds");
        assert_eq!(path_file_stem(r"data\effects\fizzle.tga"), "data_effects_fizzle_tga");
        assert_ne!(path_file_stem("fizzle.dds"), path_file_stem("fizzle.tga"), "two extensions are two files");
        assert_eq!(sanitize_file_stem(&path_file_stem("../up.dds"), FALLBACK_STEM), "___up_dds");
    }

    #[test]
    fn a_classic_hue_keeps_bit_15_and_follows_the_red_channel() {
        let hue = HueEntry { id: 1, color_table: std::array::from_fn(|index| index as u16), table_start: 0, table_end: 0, name: [0; 20] };
        let red = 7 << 10 | 3 << 5 | 9;
        assert_eq!(classic_hued(&hue, red), 7);
        assert_eq!(classic_hued(&hue, red | CLASSIC_HUE_FLAG), 7 | CLASSIC_HUE_FLAG);
    }
}
