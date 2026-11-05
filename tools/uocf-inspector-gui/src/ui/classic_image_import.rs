//! Export and import of one Classic art or gump id, in the Art and Gumps views: the record as a PNG, an image
//! checked by the encoders' rules and shown as it will be stored before anything is written, then written to a copy
//! of the files (`uocf::classic::image_files`).

use std::path::{Path, PathBuf};

use eframe::egui;
use uocf::classic::image_files::{ClassicImageContainer, ClassicImageFiles, ClassicImageKind, ImageWrite, PreparedImage};
use uocf::enhanced::dds_writer::SizeChange;

use crate::app::UopInspectorApp;
use crate::ui::image_export::export_rgba_png;
use crate::ui::{source_choice, ClientSource, SourceAvailability};

/// Height of the stored-image preview; it scrolls beyond.
const PREVIEW_MAX_HEIGHT: f32 = 240.0;

/// Which container a view reads, to pick the matching files when both are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferredContainer {
    Mul,
    Uop,
}

/// The import in progress: the files found, the chosen image and what the encoders made of it.
#[derive(Default)]
pub struct ClassicImageImport {
    found_for: Option<(PathBuf, ClassicImageKind)>,
    found: Vec<ClassicImageFiles>,
    selected: usize,
    image_path: Option<PathBuf>,
    allow_resize: bool,
    /// For (files, id, image, size change allowed): the prepared record or the encoder's error.
    prepared: Option<(usize, u32, Result<PreparedImage, String>)>,
    preview: Option<egui::TextureHandle>,
}

impl ClassicImageImport {
    fn refresh_found(&mut self, dir: &Path, kind: ClassicImageKind, preferred: PreferredContainer) {
        let key = (dir.to_path_buf(), kind);
        if self.found_for.as_ref() == Some(&key) {
            return;
        }
        self.found = ClassicImageFiles::find(dir, kind);
        self.selected = self
            .found
            .iter()
            .position(|files| matches!((preferred, &files.container), (PreferredContainer::Mul, ClassicImageContainer::Mul { .. }) | (PreferredContainer::Uop, ClassicImageContainer::Uop(_))))
            .unwrap_or_default();
        self.found_for = Some(key);
        self.clear_image();
    }

    fn clear_image(&mut self) {
        self.image_path = None;
        self.prepared = None;
        self.preview = None;
    }

    fn prepare(&mut self, ctx: &egui::Context, id: u32) {
        let (Some(files), Some(path)) = (self.found.get(self.selected), &self.image_path) else { return };
        let size_change = if self.allow_resize { SizeChange::Allow } else { SizeChange::Refuse };
        let result = image::open(path)
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(|image| files.prepare(id, &image.into_rgba8(), size_change).map_err(|error| format!("{error:#}")));
        self.preview = result.as_ref().ok().and_then(|prepared| {
            let size = [prepared.width as usize, prepared.height as usize];
            let name = format!("classic_image_import_{}_{id}", files.kind.label());
            super::textures::load_rgba(ctx, name, size, &prepared.rgba, egui::TextureOptions::NEAREST).ok()
        });
        self.prepared = Some((self.selected, id, result));
    }
}

fn container_short_label(container: &ClassicImageContainer) -> String {
    let name = |path: &Path| path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    match container {
        ClassicImageContainer::Mul { index, data } => format!("{} + {}", name(index), name(data)),
        ClassicImageContainer::Uop(path) => name(path),
    }
}

/// The source button of a container: MUL or CC UOP.
fn container_source(container: &ClassicImageContainer) -> ClientSource {
    match container {
        ClassicImageContainer::Mul { .. } => ClientSource::Mul,
        ClassicImageContainer::Uop(_) => ClientSource::CcUop,
    }
}

/// The export/import panel of `kind` id `id`, for the Classic client folder of the settings.
pub fn ui_classic_image_panel(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, kind: ClassicImageKind, id: u32, preferred: PreferredContainer) {
    let Some(dir) = app.settings.cc_path.clone() else {
        return;
    };
    ui.collapsing(format!("Export / import {} (Classic files)", kind.label()), |ui| {
        let mut state = std::mem::take(&mut app.classic_image_import);
        state.refresh_found(&dir, kind, preferred);
        ui_panel(app, ctx, ui, &mut state, &dir, id);
        app.classic_image_import = state;
    });
}

fn ui_panel(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, state: &mut ClassicImageImport, dir: &Path, id: u32) {
    if state.found.is_empty() {
        ui.label(format!("{} holds no Classic files of this kind.", dir.display()));
        return;
    }
    let before = state.selected;
    // Only the Classic client's files are written here: no KR UOP or EC UOP.
    let kind = state.found[state.selected].kind.label();
    let position = |source: ClientSource| state.found.iter().position(|files| container_source(&files.container) == source);
    let clicked = source_choice(ui, Some(container_source(&state.found[state.selected].container)), |source| match source {
        ClientSource::Mul | ClientSource::CcUop => match position(source) {
            Some(index) => SourceAvailability::Found(format!(
                "{} in {}",
                container_short_label(&state.found[index].container),
                dir.display()
            )),
            None => SourceAvailability::Missing(format!(
                "No Classic {kind} {} in {}.",
                if source == ClientSource::Mul { "MUL files" } else { "UOP package" },
                dir.display()
            )),
        },
        ClientSource::KrUop | ClientSource::EcUop => SourceAvailability::NotApplicable,
    });
    if let Some(index) = clicked.and_then(position) {
        state.selected = index;
    }
    let files = state.found[state.selected].clone();
    if before != state.selected || state.prepared.as_ref().is_some_and(|(selected, prepared_id, _)| *selected != state.selected || *prepared_id != id) {
        state.prepared = None;
        state.preview = None;
        if state.image_path.is_some() {
            state.prepare(ctx, id);
        }
    }
    ui.label(files.kind.rules(id));

    ui.horizontal(|ui| {
        if ui.button("Export PNG...").clicked() {
            app.status_message = match files.read(id) {
                Ok(Some(image)) => match export_rgba_png(format!("{}_{id:#x}.png", files.kind.label()), image.width, image.height, &image.rgba) {
                    Ok(Some(path)) => format!("Exported {} {id:#x} to {path}.", files.kind.label()),
                    Ok(None) => String::new(),
                    Err(error) => format!("Export failed: {error}"),
                },
                Ok(None) => format!("{} {id:#x} holds no image in {}.", files.kind.label(), files.container.label()),
                Err(error) => format!("Export failed: {error:#}"),
            };
        }
        if ui.button("Import image...").clicked() {
            if let Some(path) = crate::dialog::file_dialog().add_filter("Image", &["png", "bmp", "tga", "jpg", "jpeg"]).pick_file() {
                state.image_path = Some(path);
                state.prepare(ctx, id);
            }
        }
        if ui.checkbox(&mut state.allow_resize, "Allow a size change").changed() && state.image_path.is_some() {
            state.prepare(ctx, id);
        }
    });

    let Some((_, _, result)) = &state.prepared else { return };
    if let Some(path) = &state.image_path {
        ui.monospace(path.display().to_string());
    }
    match result {
        Err(error) => {
            ui.colored_label(egui::Color32::RED, error);
        }
        Ok(prepared) => {
            ui.label(format!(
                "{} {}x{}, {} bytes; {} {id:#x}.",
                prepared.format.label(),
                prepared.width,
                prepared.height,
                prepared.uop_record.len(),
                if prepared.replaces { "replaces" } else { "adds" }
            ));
            for note in &prepared.notes {
                ui.colored_label(egui::Color32::YELLOW, note);
            }
            if let Some(preview) = &state.preview {
                ui.label("As stored:");
                egui::ScrollArea::both().id_salt("classic_image_import_preview").max_height(PREVIEW_MAX_HEIGHT).show(ui, |ui| {
                    ui.image(preview);
                });
            }
            if ui.button("Save copy...").clicked() {
                app.status_message = save_copy(&files, prepared, dir).unwrap_or_else(|error| format!("Save failed: {error:#}"));
            }
        }
    }
}

/// Writes the copy: a picked folder for the MUL pair (not the client folder), a picked path for a UOP.
fn save_copy(files: &ClassicImageFiles, prepared: &PreparedImage, dir: &Path) -> color_eyre::eyre::Result<String> {
    let output = match &files.container {
        ClassicImageContainer::Mul { .. } => {
            let Some(folder) = crate::dialog::pick_folder() else { return Ok(String::new()) };
            color_eyre::eyre::ensure!(!same_dir(&folder, dir), "pick another folder than the client's: the copy never replaces the source");
            files.container.in_dir(&folder)
        }
        ClassicImageContainer::Uop(source) => {
            let name = source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(path) = crate::dialog::file_dialog().add_filter("UOP package", &["uop"]).set_file_name(name).save_file() else {
                return Ok(String::new());
            };
            ClassicImageContainer::Uop(path)
        }
    };
    let write = files.write_copy(prepared, &output)?;
    let what = match write {
        ImageWrite::Added => "added",
        ImageWrite::Replaced => "replaced",
        ImageWrite::Unchanged => "unchanged (already this record)",
    };
    Ok(format!("{} {:#x} {what}; wrote {}.", files.kind.label(), prepared.id, output.label()))
}

fn same_dir(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_are_labelled_by_file_names() {
        let mul = ClassicImageContainer::Mul { index: PathBuf::from("/c/artidx.mul"), data: PathBuf::from("/c/art.mul") };
        assert_eq!(container_short_label(&mul), "artidx.mul + art.mul");
        assert_eq!(container_short_label(&ClassicImageContainer::Uop(PathBuf::from("/c/gumpartLegacyMUL.uop"))), "gumpartLegacyMUL.uop");
    }

    #[test]
    fn the_preferred_container_is_selected_when_both_are_found() {
        let dir = std::env::temp_dir().join(format!("uocf_inspector_classic_image_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["artidx.mul", "art.mul", "artLegacyMUL.uop"] {
            std::fs::write(dir.join(name), []).unwrap();
        }
        let mut state = ClassicImageImport::default();
        state.refresh_found(&dir, ClassicImageKind::Art, PreferredContainer::Uop);
        assert!(matches!(state.found[state.selected].container, ClassicImageContainer::Uop(_)));
        let mut state = ClassicImageImport::default();
        state.refresh_found(&dir, ClassicImageKind::Art, PreferredContainer::Mul);
        assert!(matches!(state.found[state.selected].container, ClassicImageContainer::Mul { .. }));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
