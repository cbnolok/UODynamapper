//! The native images of an Enhanced Client `Interface.uop`: the icons and textures its own interface draws, named by
//! its UI XML and not by a gump id, beside the Classic gumps of `data/interface/default/textures/gumpart` that the
//! Gumps list shows. An entry the dictionary does not name is kept when its content is an image.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use eframe::egui;
use uocf::enhanced::dds_writer::DdsDescription;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::enhanced::tga_writer::TgaConvention;

use crate::app::{guess_uop_image_format_from_payload, UopInspectorApp};
use crate::logic::dictionary::Dictionary;
use crate::logic::uop_cache::LoadedUop;
use crate::ui::image_export::{export_rgba_png, sanitize_file_stem};
use crate::ui::{arrow_delta, move_selection, uop_browser, UOP_PATCHER_NOTE};

const INTERFACE_UOP_NAME: &str = "interface.uop";
/// The folder of the Classic gumps, listed by id in the Gumps list.
const GUMPART_FOLDER: &str = "data/interface/default/textures/gumpart";
/// Left out of the folder labels: every named entry starts with it.
const INTERFACE_ROOT: &str = "data/interface/";
const IMAGE_EXTENSIONS: [&str; 2] = ["dds", "tga"];
const UNNAMED_FOLDER: &str = "(not in the dictionary)";
const LIST_PANEL_WIDTH: f32 = 360.0;
const THUMBNAIL_SIDE: f32 = 32.0;
/// Largest side of a thumbnail's texture: larger images are scaled down before they are kept.
const THUMBNAIL_TEXTURE_SIDE: u32 = 64;
const PREVIEW_ZOOMS: [(f32, &str); 4] = [(1.0, "1x"), (2.0, "2x"), (4.0, "4x"), (8.0, "8x")];
const EXPORT_FALLBACK_NAME: &str = "interface_texture";
const EMPTY_IMAGE_NOTE: &str = "This entry is an empty image: a TGA header of 0 x 0 pixels, as shipped.";

/// An image entry of the package.
#[derive(Clone, Debug, PartialEq, Eq)]
struct InterfaceTexture {
    hash: u64,
    /// `None` when the dictionary does not name the entry.
    name: Option<String>,
    folder: String,
}

impl InterfaceTexture {
    fn full_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("{:016X}", self.hash))
    }

    fn file_name(&self) -> String {
        let name = self.full_name();
        name.rsplit_once('/').map_or_else(|| name.clone(), |(_, file)| file.to_owned())
    }
}

/// The state of the list.
#[derive(Default)]
pub struct EcInterfaceTexturesState {
    /// The package and the dictionary size the list was built from.
    source: Option<(std::path::PathBuf, usize)>,
    entries: Arc<Vec<InterfaceTexture>>,
    folder_counts: Vec<(String, usize)>,
    folder: Option<String>,
    filter: String,
    selected: Option<u64>,
    zoom_index: usize,
    /// `None` for an entry that does not decode.
    thumbnails: HashMap<u64, Option<egui::TextureHandle>>,
}

/// The folder of an image entry by its name, `None` for what this list does not show.
fn image_folder(name: &str) -> Option<&str> {
    let (folder, _) = name.rsplit_once('/')?;
    let extension = uocf::audio::name_extension(name)?;
    (IMAGE_EXTENSIONS.contains(&extension.as_str()) && folder != GUMPART_FOLDER).then_some(folder)
}

fn folder_label(folder: &str) -> &str {
    folder.strip_prefix(INTERFACE_ROOT).unwrap_or(folder)
}

fn collect_textures(loaded: &LoadedUop, dictionary: &Dictionary) -> Vec<InterfaceTexture> {
    let mut entries: Vec<InterfaceTexture> = loaded
        .package
        .iter_files()
        .filter_map(|file| {
            let hash = file.filename_hash();
            match dictionary.resolve(hash) {
                Some(name) => image_folder(name).map(|folder| InterfaceTexture { hash, name: Some(name.to_owned()), folder: folder.to_owned() }),
                None => {
                    let payload = loaded.package.unpack_file_by_hash(hash).ok().flatten()?;
                    guess_uop_image_format_from_payload(&payload)?;
                    Some(InterfaceTexture { hash, name: None, folder: UNNAMED_FOLDER.to_owned() })
                }
            }
        })
        .collect();
    entries.sort_by(|left, right| {
        (left.name.is_none(), &left.folder, &left.name, left.hash).cmp(&(right.name.is_none(), &right.folder, &right.name, right.hash))
    });
    entries
}

fn count_folders(entries: &[InterfaceTexture]) -> Vec<(String, usize)> {
    let mut counts = BTreeMap::new();
    for entry in entries {
        *counts.entry(entry.folder.as_str()).or_insert(0) += 1;
    }
    let mut folders: Vec<(String, usize)> = counts.into_iter().map(|(folder, count)| (folder.to_owned(), count)).collect();
    // By name, the images without one last.
    folders.sort_by_key(|(folder, _)| folder == UNNAMED_FOLDER);
    folders
}

/// The indices of the entries of `folder` (all when `None`) whose full name holds `filter`, ignoring case.
fn visible_indices(entries: &[InterfaceTexture], folder: Option<&str>, filter: &str) -> Vec<usize> {
    let filter = filter.trim().to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| folder.is_none_or(|folder| entry.folder == folder))
        .filter(|(_, entry)| filter.is_empty() || entry.full_name().to_ascii_lowercase().contains(&filter))
        .map(|(index, _)| index)
        .collect()
}

/// A small copy of an entry's image for its row.
fn decode_thumbnail(payload: &[u8], name: &str) -> Option<egui::ColorImage> {
    let (_, format) = guess_uop_image_format_from_payload(payload)
        .or_else(|| (uocf::audio::name_extension(name).as_deref() == Some("tga")).then_some(("tga", ECImageFormat::TGA)))?;
    let file = TextureFile {
        metadata: RawTextureItem::absent(),
        is_ec: true,
        format,
        props: None,
        raw_data: payload.into(),
        image_data_offset: 0,
    };
    let image = file.decode_to_rgba().ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    let largest = width.max(height);
    let image = if largest > THUMBNAIL_TEXTURE_SIDE {
        let scaled = |side: u32| (side * THUMBNAIL_TEXTURE_SIDE / largest).max(1);
        image::imageops::thumbnail(&image, scaled(width), scaled(height))
    } else {
        image
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}

fn thumbnail(app: &mut UopInspectorApp, ctx: &egui::Context, loaded: &LoadedUop, entry: &InterfaceTexture) -> Option<egui::TextureHandle> {
    if let Some(known) = app.ec_interface_textures.thumbnails.get(&entry.hash) {
        return known.clone();
    }
    let image = loaded.package.unpack_file_by_hash(entry.hash).ok().flatten().and_then(|payload| decode_thumbnail(&payload, &entry.full_name()));
    // An image that cannot be a texture has no thumbnail; the details say why.
    let handle = image.and_then(|image| {
        crate::ui::textures::load_color_image(ctx, format!("interface_thumbnail_{:016x}", entry.hash), image, egui::TextureOptions::NEAREST).ok()
    });
    app.ec_interface_textures.thumbnails.insert(entry.hash, handle.clone());
    handle
}

/// Whether an entry is a TGA of no pixels, which the client packages ship.
fn is_empty_image(payload: &[u8]) -> bool {
    TgaConvention::of(payload).is_ok_and(|convention| convention.width() == 0 || convention.height() == 0)
}

/// The stored format of an image entry.
fn stored_format(payload: &[u8]) -> String {
    if let Ok(description) = DdsDescription::parse(payload) {
        return format!("DDS {}, {} mip levels", description.format.label(), description.mip_count);
    }
    TgaConvention::of(payload).map_or_else(|_| "unknown".to_owned(), |convention| convention.label())
}

pub fn ui_interface_textures(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let found = app.uop_cache.loaded_uops.iter().position(|loaded| {
        loaded.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case(INTERFACE_UOP_NAME))
    });
    let Some(uop_idx) = found else {
        ui.label("Select an Enhanced Client or Kingdom Reborn path: these images are in Interface.uop.");
        return;
    };
    let loaded = Arc::clone(&app.uop_cache.loaded_uops[uop_idx]);
    let source = (loaded.path.clone(), app.dictionary.count());
    if app.ec_interface_textures.source.as_ref() != Some(&source) {
        let entries = collect_textures(&loaded, &app.dictionary);
        app.ec_interface_textures = EcInterfaceTexturesState {
            source: Some(source),
            folder_counts: count_folders(&entries),
            entries: Arc::new(entries),
            ..EcInterfaceTexturesState::default()
        };
    }

    let entries = Arc::clone(&app.ec_interface_textures.entries);
    egui::Panel::left("ec_interface_textures_list").resizable(true).default_size(LIST_PANEL_WIDTH).show(ui, |ui| {
        ui_list(app, ctx, ui, &loaded, &entries);
    });
    egui::CentralPanel::default().show(ui, |ui| {
        let selected = app.ec_interface_textures.selected.and_then(|hash| entries.iter().find(|entry| entry.hash == hash));
        match selected {
            Some(entry) => ui_details(app, ctx, ui, uop_idx, &loaded, entry),
            None => {
                ui.label("Select an image.");
            }
        }
    });
}

fn ui_list(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, loaded: &LoadedUop, entries: &[InterfaceTexture]) {
    let state = &mut app.ec_interface_textures;
    let selected_label = state.folder.as_deref().map_or_else(|| format!("All ({})", entries.len()), |folder| folder_label(folder).to_owned());
    egui::ComboBox::from_id_salt("ec_interface_textures_folder").width(ui.available_width()).selected_text(selected_label).show_ui(ui, |ui| {
        ui.selectable_value(&mut state.folder, None, format!("All ({})", entries.len()));
        for (folder, count) in &state.folder_counts {
            ui.selectable_value(&mut state.folder, Some(folder.clone()), format!("{} ({count})", folder_label(folder)));
        }
    });
    let filter_focus = ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("Filter by name")).has_focus();
    let visible = visible_indices(entries, state.folder.as_deref(), &state.filter);
    ui.weak(format!("{} images", visible.len()));
    ui.separator();

    let row_height = THUMBNAIL_SIDE + ui.spacing().item_spacing.y;
    let mut area = egui::ScrollArea::vertical().id_salt("ec_interface_textures_rows").auto_shrink([false, false]);
    if let Some(delta) = arrow_delta(ui, filter_focus) {
        let hashes: Vec<u64> = visible.iter().map(|&index| entries[index].hash).collect();
        state.selected = move_selection(&hashes, state.selected, delta);
        if let Some(position) = state.selected.and_then(|hash| hashes.iter().position(|&other| other == hash)) {
            area = area.vertical_scroll_offset((position as f32 * row_height - ui.available_height() / 2.0).max(0.0));
        }
    }
    area.show_rows(ui, THUMBNAIL_SIDE, visible.len(), |ui, range| {
        for &index in &visible[range] {
            let entry = &entries[index];
            let handle = thumbnail(app, ctx, loaded, entry);
            ui.horizontal(|ui| {
                let (slot, _) = ui.allocate_exact_size(egui::Vec2::splat(THUMBNAIL_SIDE), egui::Sense::hover());
                if let Some(handle) = &handle {
                    let size = handle.size_vec2();
                    let drawn = size * (THUMBNAIL_SIDE / size.x.max(size.y)).min(1.0);
                    egui::Image::new(handle).paint_at(ui, egui::Rect::from_center_size(slot.center(), drawn));
                }
                let selected = app.ec_interface_textures.selected == Some(entry.hash);
                if ui.selectable_label(selected, entry.file_name()).on_hover_text(entry.full_name()).clicked() {
                    app.ec_interface_textures.selected = Some(entry.hash);
                }
            });
        }
    });
}

fn ui_details(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui, uop_idx: usize, loaded: &LoadedUop, entry: &InterfaceTexture) {
    let (hash, name) = (entry.hash, entry.full_name());
    let Some(payload) = app.get_uop_entry_payload(uop_idx, hash) else {
        ui.label(format!("Failed to read {name}."));
        return;
    };
    let texture = app.get_uop_texture(ctx, hash, &payload, &name);
    ui.heading(entry.file_name());
    ui.horizontal_wrapped(|ui| {
        let zoom_index = &mut app.ec_interface_textures.zoom_index;
        for (index, (_, label)) in PREVIEW_ZOOMS.iter().enumerate() {
            ui.selectable_value(zoom_index, index, *label);
        }
        ui.separator();
        if ui.add_enabled(texture.is_some(), egui::Button::new("Export PNG...")).clicked() {
            export_png(app, hash, &entry.file_name());
        }
        if ui.button("Open raw entry").clicked() {
            app.select_raw_uop_entry_at_path(&loaded.path, hash);
        }
        if ui.button("Replace file...").on_hover_text(format!("{}\n{UOP_PATCHER_NOTE}", uop_browser::REPLACE_FILE_HOVER)).clicked() {
            uop_browser::replace_entry_file(app, &loaded.path, &loaded.package, hash, &name);
        }
    });
    egui::Grid::new("ec_interface_texture_details").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        match &entry.name {
            Some(name) => ui.monospace(name),
            None => ui.weak(UNNAMED_FOLDER),
        };
        ui.end_row();
        ui.label("Hash");
        ui.monospace(format!("{hash:016X}"));
        ui.end_row();
        ui.label("Stored as");
        ui.label(stored_format(&payload));
        ui.end_row();
        ui.label("Bytes");
        ui.label(payload.len().to_string());
        ui.end_row();
        if let Some(texture) = &texture {
            let [width, height] = texture.size();
            ui.label("Size");
            ui.label(format!("{width} x {height}"));
            ui.end_row();
        }
    });
    ui.separator();
    let Some(texture) = texture else {
        ui.label(if is_empty_image(&payload) { EMPTY_IMAGE_NOTE } else { "This entry does not decode as an image." });
        return;
    };
    let (zoom, _) = PREVIEW_ZOOMS[app.ec_interface_textures.zoom_index];
    egui::ScrollArea::both().id_salt("ec_interface_texture_preview").auto_shrink([false, false]).show(ui, |ui| {
        ui.add(egui::Image::new(&texture).fit_to_exact_size(texture.size_vec2() * zoom).texture_options(egui::TextureOptions::NEAREST));
    });
}

fn export_png(app: &mut UopInspectorApp, hash: u64, file_name: &str) {
    let Some(preview) = app.current_image_preview().filter(|preview| preview.key == hash) else {
        return;
    };
    let (width, height, rgba) = (preview.width, preview.height, Arc::clone(&preview.rgba));
    let stem = file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem);
    let default_name = format!("{}.png", sanitize_file_stem(stem, EXPORT_FALLBACK_NAME));
    match export_rgba_png(default_name, width, height, &rgba) {
        Ok(Some(path)) => app.status_message = format!("Exported {path}."),
        Ok(None) => {}
        Err(error) => app.status_message = format!("Failed to export: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(folder: &str, file: &str) -> InterfaceTexture {
        InterfaceTexture { hash: 0, name: Some(format!("{folder}/{file}")), folder: folder.to_owned() }
    }

    #[test]
    fn the_classic_gumps_and_other_kinds_are_left_out() {
        const ACTIONS: &str = "data/interface/default/icons/actions";
        assert_eq!(image_folder(&format!("{ACTIONS}/a.dds")), Some(ACTIONS));
        assert_eq!(image_folder(&format!("{ACTIONS}/a.TGA")), Some(ACTIONS));
        assert_eq!(image_folder(&format!("{GUMPART_FOLDER}/00009504.tga")), None);
        assert_eq!(image_folder("data/interface/default/source/window.xml"), None);
        assert_eq!(image_folder("no_folder.dds"), None);
    }

    #[test]
    fn folders_are_counted_in_order() {
        let entries = [named("b", "1.dds"), named("a", "2.dds"), named("b", "3.tga")];
        assert_eq!(count_folders(&entries), [("a".to_owned(), 1), ("b".to_owned(), 2)]);
    }

    #[test]
    fn the_folder_of_unnamed_images_comes_last() {
        let entries = [named(UNNAMED_FOLDER, "1.dds"), named("zzz", "2.dds")];
        assert_eq!(count_folders(&entries), [("zzz".to_owned(), 1), (UNNAMED_FOLDER.to_owned(), 1)]);
    }

    #[test]
    fn the_folder_and_the_filter_narrow_the_list() {
        let entries = [named("a", "Sword.dds"), named("a", "shield.dds"), named("b", "sword2.tga")];
        assert_eq!(visible_indices(&entries, None, ""), [0, 1, 2]);
        assert_eq!(visible_indices(&entries, Some("a"), ""), [0, 1]);
        assert_eq!(visible_indices(&entries, None, " SWORD "), [0, 2]);
        assert_eq!(visible_indices(&entries, Some("b"), "shield"), [] as [usize; 0]);
    }

    #[test]
    fn a_large_image_gets_a_small_thumbnail_and_garbage_gets_none() {
        const SIDE: u32 = 200;
        let image = image::RgbaImage::from_pixel(SIDE, SIDE / 2, image::Rgba([1, 2, 3, 255]));
        let tga = uocf::enhanced::tga_writer::TgaConvention::of(&minimal_tga()).unwrap().encode_rgba(&image).unwrap();
        let thumbnail = decode_thumbnail(&tga, "a.tga").unwrap();
        assert_eq!(thumbnail.size, [THUMBNAIL_TEXTURE_SIDE as usize, THUMBNAIL_TEXTURE_SIDE as usize / 2]);
        assert!(decode_thumbnail(b"not an image", "a.bin").is_none());
    }

    #[test]
    #[ignore = "needs real clients: set UO_EC_CLIENT_ROOTS to folders with an Interface.uop and run with --ignored"]
    fn every_listed_image_of_a_shipped_interface_decodes() {
        const CLIENT_ROOTS_ENV: &str = "UO_EC_CLIENT_ROOTS";
        const DICTIONARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../_shared_assets/Dictionary.dic");
        const TGA_HEADER_BYTES: usize = 18;
        let roots = std::env::var_os(CLIENT_ROOTS_ENV).unwrap_or_else(|| panic!("set {CLIENT_ROOTS_ENV}"));
        let mut dictionary = Dictionary::new();
        dictionary.load_dic(std::path::Path::new(DICTIONARY)).unwrap();
        for root in std::env::split_paths(&roots) {
            let Some(path) = uocf::utils::path::find_file_ignoring_case(&root, "Interface.uop") else {
                println!("{}: no Interface.uop", root.display());
                continue;
            };
            let package = uocf::uop_container::package::UopPackage::load(&path).unwrap();
            let loaded = LoadedUop { path, package };
            let entries = collect_textures(&loaded, &dictionary);
            let mut replaceable = 0;
            let mut empty = 0;
            // A TGA read accepts a payload with a zero at the width or height offset, whatever the payload is, so
            // an empty result only counts as a real empty image when the entry is a bare header.
            let failed: Vec<String> = entries
                .iter()
                .filter(|entry| {
                    let payload = loaded.package.unpack_file_by_hash(entry.hash).unwrap().unwrap();
                    let key = uocf::enhanced::entry_edit::package_key(&loaded.path);
                    replaceable += usize::from(uocf::enhanced::entry_image::entry_image_format(&key, entry.hash, &payload).is_some());
                    let Some(thumbnail) = decode_thumbnail(&payload, &entry.full_name()) else {
                        return true;
                    };
                    let no_pixels = thumbnail.size[0] == 0 || thumbnail.size[1] == 0;
                    empty += usize::from(no_pixels);
                    let well_formed = DdsDescription::parse(&payload).is_ok() || TgaConvention::of(&payload).is_ok();
                    !well_formed || (no_pixels && payload.len() != TGA_HEADER_BYTES)
                })
                .map(InterfaceTexture::full_name)
                .collect();
            println!(
                "{}: {} images, {} folders, {replaceable} convertible, {empty} empty, {} failed: {:?}",
                root.display(),
                entries.len(),
                count_folders(&entries).len(),
                failed.len(),
                &failed[..failed.len().min(20)]
            );
            assert!(!entries.is_empty());
        }
    }

    /// An uncompressed 32-bit TGA of one pixel.
    fn minimal_tga() -> Vec<u8> {
        const HEADER: [u8; 18] = [0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8];
        let mut bytes = HEADER.to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 255]);
        bytes
    }
}
