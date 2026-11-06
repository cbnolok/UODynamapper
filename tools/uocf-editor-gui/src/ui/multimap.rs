//! The Multimap tab: shows `Multimap.rle`, and builds a plain and an artistic one from a facet's radar image.
//!
//! The radar image comes from the client files: the Classic map and statics coloured by `radarcol.mul`
//! ([`uocf::classic::radar`], as the Maps tab draws them) from `mapN.mul` (MUL) or `mapNLegacyMUL.uop` (CC UOP), or
//! an Enhanced-format client's own radar image `facetNN.dds`: the Kingdom Reborn folder's (KR UOP) or the Enhanced
//! Client folder's (EC UOP). The folders' `facetN.uop` maps carry no land colours.

use crate::app::{
    GeneratedMultimapPreview, MultimapConversionOutput, MultimapConverterState,
    MultimapPreviewSelection, MultimapRadarSource, MultimapWorkerResult, UocfEditorApp,
};
use color_eyre::eyre::{self, WrapErr};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use uocf::classic::facet_sources::{ClassicDifOptions, ClassicFacetSources, ClassicMapFormat};
use uocf::classic::multimap_render::{
    render_multimap, DecodedRgba, MultimapRenderOptions, MultimapStyle,
};
use uocf::classic::multimap_rle::{self, MultimapRleImage, BLACK_PIXEL};
use uocf::classic::radar::{load_client_radarcol, render_radar, RadarColours, RADAR_RGBA_CHANNELS};
use uocf::classic::tiledata::TileData;
use uocf::classic::verdata::Verdata;
use uocf::enhanced::facet_radar::{radar_dds_file_name, FacetRadarDds};
use uocf::map_edition::MapEdition;
use uocf::utils::path::find_file_ignoring_case;

use super::classic_maps::{classic_map_found, map_found_text, map_not_found_text};
use super::{tab_source_choice, ClientSource, EnhancedFolders, SourceAvailability};

const GENERATED_PLAIN_KEY: u64 = 0x6F00000000000001;
const GENERATED_ARTISTIC_KEY: u64 = 0x6F00000000000002;
const MULTIMAP_TOOLS_PANEL_WIDTH: f32 = 340.0;
const MULTIMAP_ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.05..=2.0;
const TILEDATA_FILE: &str = "tiledata.mul";
/// The radar is read at one pixel per tile.
const FULL_SCALE: u32 = 1;
/// The full-size mip of `facetNN.dds`.
const FULL_SIZE_LEVEL: u32 = 0;
const RGB_CHANNELS: usize = 3;
const OPAQUE: u8 = u8::MAX;
/// The radar sources with the sources their buttons stand for, in the order every tab shows them.
const RADAR_SOURCES: [(MultimapRadarSource, ClientSource); 4] = [
    (MultimapRadarSource::Mul, ClientSource::Mul),
    (MultimapRadarSource::CcUop, ClientSource::CcUop),
    (MultimapRadarSource::KrUop, ClientSource::KrUop),
    (MultimapRadarSource::EcUop, ClientSource::EcUop),
];
const NO_CC_FOLDER: &str = "No Classic Client folder is set (File > Search Paths).";
const PATCHES_CLASSIC_ONLY: &str = "The patches apply to the Classic map files; the Enhanced Client's radar image is read as shipped.";

struct GenerateRequest {
    input: GenerateInput,
    source_x: u32,
    source_y: u32,
    source_width: Option<u32>,
    source_height: Option<u32>,
    output_width: u32,
    output_height: u32,
    edge_threshold: u16,
    line_radius: u32,
}

enum GenerateInput {
    /// The Classic map and statics of `map_id`, coloured by `radarcol.mul`.
    ClassicRadar {
        cc_path: PathBuf,
        map_id: u32,
        format: ClassicMapFormat,
        difs: ClassicDifOptions,
        include_verdata: bool,
    },
    /// The Enhanced Client's radar image.
    EnhancedRadar(PathBuf),
}

/// Which of [`RADAR_SOURCES`] the client folders hold for a map, with the folders and map they were looked for in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundRadarSources {
    cc_path: Option<PathBuf>,
    enhanced: EnhancedFolders,
    map_id: u32,
    present: [bool; RADAR_SOURCES.len()],
}

/// The Classic map container of a source; `None` for an Enhanced-format radar image.
fn classic_format(source: MultimapRadarSource) -> Option<ClassicMapFormat> {
    match source {
        MultimapRadarSource::Mul => Some(ClassicMapFormat::Mul),
        MultimapRadarSource::CcUop => Some(ClassicMapFormat::Uop),
        MultimapRadarSource::KrUop | MultimapRadarSource::EcUop => None,
    }
}

/// The source button `source` stands for.
fn client_source(source: MultimapRadarSource) -> ClientSource {
    RADAR_SOURCES.iter().find(|&&(candidate, _)| candidate == source).map_or(ClientSource::EcUop, |&(_, client)| client)
}

/// `facetNN.dds` of map `map_id` in the Enhanced-format client folder `folder`, if it is there.
fn enhanced_radar_path(folder: &Path, map_id: u32) -> Option<PathBuf> {
    let facet = u8::try_from(map_id).ok()?;
    find_file_ignoring_case(folder, &radar_dds_file_name(facet))
}

/// The radar image of map `map_id` an Enhanced-format source names, for a tooltip.
fn enhanced_radar_name(map_id: u32) -> String {
    match u8::try_from(map_id) {
        Ok(facet) => radar_dds_file_name(facet),
        Err(_) => format!("The radar image of map {map_id}"),
    }
}

fn source_present(source: MultimapRadarSource, cc_path: Option<&Path>, enhanced: &EnhancedFolders, map_id: u32) -> bool {
    match classic_format(source) {
        Some(format) => cc_path.is_some_and(|cc_path| classic_map_found(cc_path, map_id, format)),
        None => enhanced.folder(client_source(source)).is_some_and(|folder| enhanced_radar_path(folder, map_id).is_some()),
    }
}

/// The tooltip of a source the folders hold.
fn source_found_text(source: MultimapRadarSource, enhanced: &EnhancedFolders, map_id: u32) -> String {
    let client = client_source(source);
    match (classic_format(source), enhanced.folder(client)) {
        (Some(format), _) => map_found_text(map_id, format),
        (None, Some(folder)) => format!("{} from {}.", enhanced_radar_name(map_id), enhanced.folder_text(client, folder)),
        (None, None) => enhanced.no_folder_text(client, &enhanced_radar_name(map_id)),
    }
}

/// The tooltip of a source the folders do not hold.
fn source_missing_text(source: MultimapRadarSource, cc_path: Option<&Path>, enhanced: &EnhancedFolders, map_id: u32) -> String {
    let client = client_source(source);
    match (classic_format(source), cc_path, enhanced.folder(client)) {
        (Some(_), None, _) => NO_CC_FOLDER.to_string(),
        (Some(format), Some(_), _) => map_not_found_text(map_id, format),
        (None, _, None) => enhanced.no_folder_text(client, &enhanced_radar_name(map_id)),
        (None, _, Some(folder)) => {
            format!("{} was not found in {}.", enhanced_radar_name(map_id), enhanced.folder_text(client, folder))
        }
    }
}

/// Which sources the folders hold for the map, looked for again only when the folders or the map changed. When
/// they changed and the chosen source is not there, the first source found is chosen.
fn refresh_found_sources(state: &mut MultimapConverterState, cc_path: Option<&Path>, enhanced: &EnhancedFolders) -> [bool; RADAR_SOURCES.len()] {
    if let Some(found) = &state.found {
        if found.cc_path.as_deref() == cc_path && found.enhanced == *enhanced && found.map_id == state.map_id {
            return found.present;
        }
    }
    let present = RADAR_SOURCES.map(|(source, _)| source_present(source, cc_path, enhanced, state.map_id));
    let chosen = RADAR_SOURCES.iter().position(|&(source, _)| source == state.source);
    if !chosen.is_some_and(|index| present[index]) {
        if let Some(index) = present.iter().position(|&present| present) {
            state.source = RADAR_SOURCES[index].0;
        }
    }
    state.found = Some(FoundRadarSources {
        cc_path: cc_path.map(Path::to_path_buf),
        enhanced: enhanced.clone(),
        map_id: state.map_id,
        present,
    });
    present
}

pub fn ui_multimap(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    poll_worker(app, ctx);

    egui::Panel::left("multimap_tools")
        .resizable(true)
        .default_size(MULTIMAP_TOOLS_PANEL_WIDTH)
        .show(ui, |ui| {
            ui.heading("Multimap");
            ui.separator();
            super::multimap_editor::bar(app, ui);
            ui.separator();
            loaded_rle_controls(app, ui);
            ui.separator();
            converter_controls(app, ui);
        });

    egui::CentralPanel::default().show(ui, |ui| {
        preview_tabs(app, ui);
        ui.separator();
        preview_actions(app, ui);
        ui.separator();
        draw_selected_preview(app, ctx, ui);
    });
}

fn loaded_rle_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let dirty = app.multimap_editor.is_dirty();
    ui.horizontal(|ui| {
        if ui.add_enabled(!dirty, egui::Button::new("Open RLE...")).on_disabled_hover_text("Save or revert the changes first.").clicked() {
            if let Some(path) = crate::dialog::file_dialog()
                .add_filter("Multimap RLE", &["rle"])
                .pick_file()
            {
                match multimap_rle::load_rle(&path) {
                    Ok(image) => {
                        app.log_loaded(&[&path], format_args!("{}x{} pixels", image.width, image.height));
                        app.cc_multimap = Some(std::sync::Arc::new(image));
                        app.cc_multimap_path = Some(path.clone());
                        app.multimap_texture = None;
                        app.multimap_preview_selection = MultimapPreviewSelection::Loaded;
                        app.status_message = format!("Loaded {}", path.display());
                    }
                    Err(error) => {
                        app.status_message = format!("Failed to load RLE: {error}");
                        app.log_load_failure(&[&path], format_args!("{error:#}"));
                    }
                }
            }
        }
    });

    if let Some(path) = &app.cc_multimap_path {
        ui.monospace(path.display().to_string());
    } else if app.cc_multimap.is_some() {
        ui.monospace("Classic Client multimap.rle");
    }
}

fn converter_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let active = app.multimap_worker_active;
    radar_source_controls(app, ui);

    ui.separator();
    crop_controls(&mut app.multimap_converter, ui);
    ui.separator();

    if ui
        .add_enabled(!active, egui::Button::new("Generate Plain + Artistic"))
        .clicked()
    {
        match build_generate_request(app) {
            Ok(request) => start_worker(app, request),
            Err(error) => {
                app.multimap_converter.status = error;
            }
        }
    }

    if active {
        ui.spinner();
    }
    if !app.multimap_converter.status.is_empty() {
        ui.label(&app.multimap_converter.status);
    }
}

/// The map, the MUL / CC UOP / KR UOP / EC UOP buttons (a source not found greyed, with a tooltip) and the patches of a
/// Classic map. The radar image of KR UOP and EC UOP is read from its own folder, whichever client the other tabs show.
fn radar_source_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let (cc_path, enhanced) = (app.settings.cc_path.clone(), app.enhanced_folders.clone());
    let state = &mut app.multimap_converter;
    ui.horizontal(|ui| {
        ui.label("Map");
        ui.add(egui::DragValue::new(&mut state.map_id).range(0..=u32::from(u8::MAX)));
    });
    let present = refresh_found_sources(state, cc_path.as_deref(), &enhanced);
    let map_id = state.map_id;
    let availability = |source: MultimapRadarSource| match RADAR_SOURCES.iter().position(|&(candidate, _)| candidate == source) {
        Some(index) if present[index] => SourceAvailability::Found(source_found_text(source, &enhanced, map_id)),
        _ => SourceAvailability::Missing(source_missing_text(source, cc_path.as_deref(), &enhanced, map_id)),
    };
    if let Some(source) = tab_source_choice(ui, &RADAR_SOURCES, state.source, availability) {
        state.source = source;
    }
    let classic = classic_format(state.source).is_some();
    ui.horizontal(|ui| {
        ui.add_enabled(classic, egui::Checkbox::new(&mut state.include_verdata, "verdata")).on_disabled_hover_text(PATCHES_CLASSIC_ONLY);
        ui.add_enabled(classic, egui::Checkbox::new(&mut state.include_map_difs, "map difs")).on_disabled_hover_text(PATCHES_CLASSIC_ONLY);
        ui.add_enabled(classic, egui::Checkbox::new(&mut state.include_static_difs, "static difs")).on_disabled_hover_text(PATCHES_CLASSIC_ONLY);
    });

    let folder = match classic_format(state.source) {
        Some(_) => cc_path.map(|path| format!("CC {}", path.display())),
        None => {
            let client = client_source(state.source);
            enhanced.folder(client).map(|path| format!("{} {}", client.label(), path.display()))
        }
    };
    if let Some(folder) = folder {
        ui.monospace(folder);
    }
}

fn crop_controls(state: &mut MultimapConverterState, ui: &mut egui::Ui) {
    egui::Grid::new("multimap_converter_grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Source X");
            ui.add(egui::DragValue::new(&mut state.source_x));
            ui.end_row();

            ui.label("Source Y");
            ui.add(egui::DragValue::new(&mut state.source_y));
            ui.end_row();

            ui.label("Source W");
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.source_width_enabled, "");
                ui.add_enabled(
                    state.source_width_enabled,
                    egui::DragValue::new(&mut state.source_width).range(1..=u32::MAX),
                );
            });
            ui.end_row();

            ui.label("Source H");
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.source_height_enabled, "");
                ui.add_enabled(
                    state.source_height_enabled,
                    egui::DragValue::new(&mut state.source_height).range(1..=u32::MAX),
                );
            });
            ui.end_row();

            ui.label("Output W");
            ui.add(egui::DragValue::new(&mut state.output_width).range(1..=u32::MAX));
            ui.end_row();

            ui.label("Output H");
            ui.add(egui::DragValue::new(&mut state.output_height).range(1..=u32::MAX));
            ui.end_row();

            ui.label("Edge");
            ui.add(egui::Slider::new(&mut state.edge_threshold, 1..=255));
            ui.end_row();

            ui.label("Ink");
            ui.add(egui::Slider::new(&mut state.line_radius, 0..=4));
            ui.end_row();
        });
}

fn preview_tabs(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let loaded = app.cc_multimap.is_some();
    let plain = app.generated_multimap_plain.is_some();
    let artistic = app.generated_multimap_artistic.is_some();
    if !preview_available(app.multimap_preview_selection, loaded, plain, artistic) {
        app.multimap_preview_selection = if artistic {
            MultimapPreviewSelection::Artistic
        } else if plain {
            MultimapPreviewSelection::Plain
        } else {
            MultimapPreviewSelection::Loaded
        };
    }

    ui.horizontal(|ui| {
        if loaded {
            ui.selectable_value(
                &mut app.multimap_preview_selection,
                MultimapPreviewSelection::Loaded,
                "Loaded RLE",
            );
        }
        if plain {
            ui.selectable_value(
                &mut app.multimap_preview_selection,
                MultimapPreviewSelection::Plain,
                "Plain",
            );
        }
        if artistic {
            ui.selectable_value(
                &mut app.multimap_preview_selection,
                MultimapPreviewSelection::Artistic,
                "Artistic",
            );
        }
    });
}

fn preview_actions(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("Save RLE...").clicked() {
            if app.multimap_preview_selection == MultimapPreviewSelection::Loaded {
                super::multimap_editor::save(app);
            } else if let Some((name, image)) = selected_image(app) {
                if let Some(path) = crate::dialog::file_dialog()
                    .add_filter("Multimap RLE", &["rle"])
                    .set_file_name(format!("{name}.rle"))
                    .save_file()
                {
                    match multimap_rle::save_rle(&path, &image) {
                        Ok(()) => app.status_message = format!("Saved {}", path.display()),
                        Err(error) => app.status_message = format!("Failed to save RLE: {error}"),
                    }
                }
            }
        }

        let menu = super::image_export::ExportMenu::unhued(super::image_export::NOT_HUED);
        if super::image_export::export_menu(ui, &mut app.image_export, &menu) == Some(super::image_export::ExportClicked::Shown) {
            if let Some((name, image)) = selected_image(app) {
                let rgba = image.to_rgba8();
                let saved = super::image_export::save_rgba(&name, image.width, image.height, &rgba, app.image_export.format);
                if let Some(status) = super::image_export::save_status(&name, saved) {
                    app.status_message = status;
                }
            }
        }
    });
}

fn draw_selected_preview(app: &mut UocfEditorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(details) = selected_details(app) else {
        ui.centered_and_justified(|ui| {
            ui.label("No multimap image selected.");
        });
        return;
    };

    let handle = match app.multimap_preview_selection {
        MultimapPreviewSelection::Loaded => app.get_multimap_texture(ctx),
        selection => app.get_generated_multimap_texture(ctx, selection),
    };
    let Some(handle) = handle else {
        ui.centered_and_justified(|ui| {
            ui.label("Failed to build preview texture.");
        });
        return;
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(details.label);
        ui.monospace(format!("{} x {}", details.width, details.height));
        ui.monospace(format!("pixels {}", details.total_pixels));
        ui.monospace(format!("black {}", details.black_pixels));
        ui.monospace(format!("white {}", details.white_pixels));
    });
    if let Some(source) = details.source {
        ui.monospace(source);
    }
    if let Some(crop) = details.crop {
        ui.monospace(format!("crop {}x{}+{},{}", crop.width, crop.height, crop.x, crop.y));
    }

    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut app.multimap_zoom, MULTIMAP_ZOOM_RANGE)
                .logarithmic(true)
                .text("Zoom"),
        );
        ui.weak("Shift or Ctrl + wheel zooms; the wheel scrolls.");
    });
    let size = egui::vec2(
        details.width as f32 * app.multimap_zoom,
        details.height as f32 * app.multimap_zoom,
    );
    let view = egui::ScrollArea::both().show(ui, |ui| {
        ui.add(egui::Image::new(&handle).fit_to_exact_size(size));
    });
    // Shift or Ctrl + wheel, or a pinch, zoom the image under the pointer; the wheel alone scrolls it.
    let zoom = super::hovered_zoom_factor(ui, view.inner_rect);
    if zoom != super::NO_ZOOM {
        app.multimap_zoom =
            (app.multimap_zoom * zoom).clamp(*MULTIMAP_ZOOM_RANGE.start(), *MULTIMAP_ZOOM_RANGE.end());
    }
}

struct PreviewDetails {
    label: String,
    width: u32,
    height: u32,
    total_pixels: usize,
    black_pixels: usize,
    white_pixels: usize,
    source: Option<String>,
    crop: Option<uocf::classic::multimap_render::SourceRect>,
}

fn selected_details(app: &UocfEditorApp) -> Option<PreviewDetails> {
    match app.multimap_preview_selection {
        MultimapPreviewSelection::Loaded => {
            let image = app.cc_multimap.as_ref()?;
            Some(details_for_image(
                "Loaded RLE",
                image,
                app.cc_multimap_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                None,
            ))
        }
        MultimapPreviewSelection::Plain => {
            let preview = app.generated_multimap_plain.as_ref()?;
            Some(details_for_generated(preview))
        }
        MultimapPreviewSelection::Artistic => {
            let preview = app.generated_multimap_artistic.as_ref()?;
            Some(details_for_generated(preview))
        }
    }
}

fn details_for_generated(preview: &GeneratedMultimapPreview) -> PreviewDetails {
    details_for_image(
        &preview.label,
        &preview.image,
        Some(format!(
            "{} ({} x {})",
            preview.source_label, preview.source_width, preview.source_height
        )),
        Some(preview.crop),
    )
}

fn details_for_image(
    label: &str,
    image: &MultimapRleImage,
    source: Option<String>,
    crop: Option<uocf::classic::multimap_render::SourceRect>,
) -> PreviewDetails {
    let black_pixels = image
        .pixels
        .iter()
        .filter(|&&pixel| pixel == BLACK_PIXEL)
        .count();
    let total_pixels = image.pixels.len();
    PreviewDetails {
        label: label.to_string(),
        width: image.width,
        height: image.height,
        total_pixels,
        black_pixels,
        white_pixels: total_pixels.saturating_sub(black_pixels),
        source,
        crop,
    }
}

fn selected_image(app: &UocfEditorApp) -> Option<(String, MultimapRleImage)> {
    match app.multimap_preview_selection {
        MultimapPreviewSelection::Loaded => {
            let image = app.cc_multimap.as_ref()?;
            Some(("multimap".to_string(), image.as_ref().clone()))
        }
        MultimapPreviewSelection::Plain => {
            let preview = app.generated_multimap_plain.as_ref()?;
            Some(("multimap_plain".to_string(), preview.image.clone()))
        }
        MultimapPreviewSelection::Artistic => {
            let preview = app.generated_multimap_artistic.as_ref()?;
            Some(("multimap_artistic".to_string(), preview.image.clone()))
        }
    }
}

fn preview_available(
    selection: MultimapPreviewSelection,
    loaded: bool,
    plain: bool,
    artistic: bool,
) -> bool {
    match selection {
        MultimapPreviewSelection::Loaded => loaded,
        MultimapPreviewSelection::Plain => plain,
        MultimapPreviewSelection::Artistic => artistic,
    }
}

fn build_generate_request(app: &UocfEditorApp) -> Result<GenerateRequest, String> {
    let state = &app.multimap_converter;
    let (cc_path, enhanced) = (app.settings.cc_path.as_deref(), &app.enhanced_folders);
    let input = match classic_format(state.source) {
        Some(format) => GenerateInput::ClassicRadar {
            cc_path: cc_path.ok_or_else(|| NO_CC_FOLDER.to_string())?.to_path_buf(),
            map_id: state.map_id,
            format,
            difs: ClassicDifOptions { map_difs: state.include_map_difs, static_difs: state.include_static_difs },
            include_verdata: state.include_verdata,
        },
        None => {
            let path = enhanced
                .folder(client_source(state.source))
                .and_then(|folder| enhanced_radar_path(folder, state.map_id))
                .ok_or_else(|| source_missing_text(state.source, cc_path, enhanced, state.map_id))?;
            GenerateInput::EnhancedRadar(path)
        }
    };

    Ok(GenerateRequest {
        input,
        source_x: state.source_x,
        source_y: state.source_y,
        source_width: state.source_width_enabled.then_some(state.source_width),
        source_height: state.source_height_enabled.then_some(state.source_height),
        output_width: state.output_width,
        output_height: state.output_height,
        edge_threshold: state.edge_threshold,
        line_radius: state.line_radius,
    })
}

fn start_worker(app: &mut UocfEditorApp, request: GenerateRequest) {
    let (tx, rx) = mpsc::channel();
    app.multimap_worker_rx = Some(rx);
    app.multimap_worker_active = true;
    app.multimap_converter.status = "Generating...".to_string();
    thread::spawn(move || {
        let result = generate_multimaps(request).map_err(|error| error.to_string());
        let _ = tx.send(MultimapWorkerResult { result });
    });
}

fn poll_worker(app: &mut UocfEditorApp, ctx: &egui::Context) {
    let message = app.multimap_worker_rx.as_ref().and_then(|rx| match rx.try_recv() {
        Ok(message) => Some(Ok(message)),
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(Err("Multimap worker disconnected.".to_string())),
    });

    let Some(message) = message else {
        if app.multimap_worker_active {
            ctx.request_repaint();
        }
        return;
    };

    app.multimap_worker_rx = None;
    app.multimap_worker_active = false;

    match message {
        Ok(worker) => match worker.result {
            Ok(output) => {
                app.generated_multimap_plain = Some(output.plain);
                app.generated_multimap_artistic = Some(output.artistic);
                app.texture_previews.remove(&GENERATED_PLAIN_KEY);
                app.texture_previews.remove(&GENERATED_ARTISTIC_KEY);
                app.multimap_preview_selection = MultimapPreviewSelection::Artistic;
                app.multimap_converter.status = "Generated plain and artistic maps.".to_string();
                app.status_message = app.multimap_converter.status.clone();
            }
            Err(error) => {
                app.multimap_converter.status = format!("Generation failed: {error}");
                app.status_message = app.multimap_converter.status.clone();
            }
        },
        Err(error) => {
            app.multimap_converter.status = error;
            app.status_message = app.multimap_converter.status.clone();
        }
    }
}

fn generate_multimaps(request: GenerateRequest) -> color_eyre::eyre::Result<MultimapConversionOutput> {
    let (decoded, source_label) = load_source_rgba(&request)?;
    let base = MultimapRenderOptions {
        source_x: request.source_x,
        source_y: request.source_y,
        source_width: request.source_width,
        source_height: request.source_height,
        output_width: request.output_width,
        output_height: request.output_height,
        edge_threshold: request.edge_threshold,
        line_radius: request.line_radius,
        style: MultimapStyle::Edge,
    };

    let plain = render_multimap(&decoded, base)?;
    let artistic = render_multimap(
        &decoded,
        MultimapRenderOptions {
            style: MultimapStyle::Classic,
            ..base
        },
    )?;

    Ok(MultimapConversionOutput {
        plain: GeneratedMultimapPreview {
            label: "Plain multimap".to_string(),
            source_label: source_label.clone(),
            source_width: decoded.width,
            source_height: decoded.height,
            crop: plain.crop,
            image: plain.image,
        },
        artistic: GeneratedMultimapPreview {
            label: "Artistic multimap".to_string(),
            source_label,
            source_width: decoded.width,
            source_height: decoded.height,
            crop: artistic.crop,
            image: artistic.image,
        },
    })
}

fn load_source_rgba(request: &GenerateRequest) -> eyre::Result<(DecodedRgba, String)> {
    match &request.input {
        GenerateInput::ClassicRadar { cc_path, map_id, format, difs, include_verdata } => {
            classic_radar_rgba(cc_path, *map_id, *format, *difs, *include_verdata)
        }
        GenerateInput::EnhancedRadar(path) => enhanced_radar_rgba(path),
    }
}

/// The radar of Classic map `map_id` in `cc_path`, as the Maps tab draws it, at one pixel per tile.
fn classic_radar_rgba(
    cc_path: &Path,
    map_id: u32,
    format: ClassicMapFormat,
    difs: ClassicDifOptions,
    include_verdata: bool,
) -> eyre::Result<(DecodedRgba, String)> {
    let dirs = [cc_path.to_path_buf()];
    let sources = ClassicFacetSources::resolve(&dirs, map_id, &[format], difs, MapEdition::Standard)?;
    let verdata = if include_verdata { Verdata::load_from_dirs(&dirs)? } else { None };
    let mut plane = sources.load_map_plane(None, verdata.clone())?;
    let size = plane.size_cells();
    let statics = sources.load_statics_reader(size.width, size.height, verdata.clone())?.load_all()?;
    let tiledata = find_file_ignoring_case(cc_path, TILEDATA_FILE)
        .map(|path| TileData::load_with_verdata(path, verdata))
        .transpose()?;
    let colours = RadarColours::with_tiledata(load_client_radarcol(cc_path)?, tiledata.as_ref());
    let radar = render_radar(&mut plane, &statics, &colours, FULL_SCALE)?;
    let label = format!("map{map_id} radar from {}", sources.map.path.display());
    Ok((DecodedRgba::new(radar.width, radar.height, radar.rgba)?, label))
}

/// The Enhanced Client's radar image `path` (`facetNN.dds`), its full-size mip.
fn enhanced_radar_rgba(path: &Path) -> eyre::Result<(DecodedRgba, String)> {
    let dds = FacetRadarDds::load(path)?;
    let rgb = dds.decode_level_rgb8(FULL_SIZE_LEVEL);
    let mut rgba = Vec::with_capacity(rgb.len() / RGB_CHANNELS * RADAR_RGBA_CHANNELS);
    for pixel in rgb.chunks_exact(RGB_CHANNELS) {
        rgba.extend_from_slice(pixel);
        rgba.push(OPAQUE);
    }
    let decoded = DecodedRgba::new(dds.width(), dds.height(), rgba).wrap_err_with(|| path.display().to_string())?;
    Ok((decoded, path.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::output_path;
    use uocf::client_config::login_servers::KINGDOM_REBORN_EXE;

    /// Facets whose radar image only one of the folders holds.
    const KR_ONLY_FACET: u8 = 0;
    const EC_ONLY_FACET: u8 = 5;

    #[test]
    fn each_enhanced_radar_source_reads_its_own_folder() {
        let (kr, ec) = (output_path("multimap-kr"), output_path("multimap-ec"));
        std::fs::create_dir_all(&kr).unwrap();
        std::fs::create_dir_all(&ec).unwrap();
        std::fs::write(kr.join(KINGDOM_REBORN_EXE), []).unwrap();
        std::fs::write(kr.join(radar_dds_file_name(KR_ONLY_FACET)), []).unwrap();
        std::fs::write(ec.join(radar_dds_file_name(EC_ONLY_FACET)), []).unwrap();
        let (kr_map, ec_map) = (u32::from(KR_ONLY_FACET), u32::from(EC_ONLY_FACET));
        let enhanced = EnhancedFolders::from_settings(Some(&kr), Some(&ec));
        let present = |source, map_id| source_present(source, None, &enhanced, map_id);
        assert!(present(MultimapRadarSource::KrUop, kr_map) && !present(MultimapRadarSource::EcUop, kr_map));
        assert!(!present(MultimapRadarSource::KrUop, ec_map) && present(MultimapRadarSource::EcUop, ec_map));

        let found = source_found_text(MultimapRadarSource::KrUop, &enhanced, kr_map);
        assert!(found.contains(&kr.display().to_string()) && found.contains("Kingdom Reborn client folder"), "{found}");
        let missing = source_missing_text(MultimapRadarSource::EcUop, None, &enhanced, kr_map);
        assert!(missing.starts_with("facet00.dds was not found in") && missing.contains(&ec.display().to_string()), "{missing}");
        let no_kr = EnhancedFolders::from_settings(None, Some(&ec));
        let missing = source_missing_text(MultimapRadarSource::KrUop, None, &no_kr, kr_map);
        assert!(missing.contains("no Kingdom Reborn client folder is set"), "{missing}");

        let mut state = MultimapConverterState { map_id: kr_map, ..Default::default() };
        assert_eq!(refresh_found_sources(&mut state, None, &enhanced), [false, false, true, false]);
        assert_eq!(state.source, MultimapRadarSource::KrUop, "the first source found is chosen");

        for folder in [kr, ec] {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}
