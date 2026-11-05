//! The Multimap tab: shows `Multimap.rle`, and builds a plain and an artistic one from a facet's radar image.
//!
//! The radar image comes from the client files: the Classic map and statics coloured by `radarcol.mul`
//! ([`uocf::classic::radar`], as the Maps tab draws them) from `mapN.mul` (MUL) or `mapNLegacyMUL.uop` (CC UOP), or
//! the Enhanced Client's own radar image `facetNN.dds` (EC UOP; the folder's `facetN.uop` maps carry no land
//! colours). There is no KR UOP source: the Enhanced files hold one radar image per facet.

use crate::app::{
    GeneratedMultimapPreview, MultimapConversionOutput, MultimapConverterState,
    MultimapPreviewSelection, MultimapRadarSource, MultimapWorkerResult, UopInspectorApp,
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
use super::{tab_source_choice, ClientSource, SourceAvailability};

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
const RADAR_SOURCES: [(MultimapRadarSource, ClientSource); 3] = [
    (MultimapRadarSource::Mul, ClientSource::Mul),
    (MultimapRadarSource::CcUop, ClientSource::CcUop),
    (MultimapRadarSource::EcUop, ClientSource::EcUop),
];
const NO_CC_FOLDER: &str = "No Classic Client folder is set (File > Search Paths).";
const NO_EC_FOLDER: &str = "No Enhanced Client folder is set (File > Search Paths).";
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
    ec_path: Option<PathBuf>,
    map_id: u32,
    present: [bool; RADAR_SOURCES.len()],
}

/// The Classic map container of a source; `None` for the Enhanced radar image.
fn classic_format(source: MultimapRadarSource) -> Option<ClassicMapFormat> {
    match source {
        MultimapRadarSource::Mul => Some(ClassicMapFormat::Mul),
        MultimapRadarSource::CcUop => Some(ClassicMapFormat::Uop),
        MultimapRadarSource::EcUop => None,
    }
}

/// `facetNN.dds` of map `map_id` in the Enhanced Client folder `ec_path`, if it is there.
fn enhanced_radar_path(ec_path: &Path, map_id: u32) -> Option<PathBuf> {
    let facet = u8::try_from(map_id).ok()?;
    find_file_ignoring_case(ec_path, &radar_dds_file_name(facet))
}

fn source_present(source: MultimapRadarSource, cc_path: Option<&Path>, ec_path: Option<&Path>, map_id: u32) -> bool {
    match classic_format(source) {
        Some(format) => cc_path.is_some_and(|cc_path| classic_map_found(cc_path, map_id, format)),
        None => ec_path.is_some_and(|ec_path| enhanced_radar_path(ec_path, map_id).is_some()),
    }
}

/// The tooltip of a source the folders hold.
fn source_found_text(source: MultimapRadarSource, map_id: u32) -> String {
    match (classic_format(source), u8::try_from(map_id)) {
        (Some(format), _) => map_found_text(map_id, format),
        (None, Ok(facet)) => format!("{} in the Enhanced Client folder.", radar_dds_file_name(facet)),
        (None, Err(_)) => format!("The Enhanced Client radar image of map {map_id}."),
    }
}

/// The tooltip of a source the folders do not hold.
fn source_missing_text(source: MultimapRadarSource, cc_path: Option<&Path>, ec_path: Option<&Path>, map_id: u32) -> String {
    match (classic_format(source), cc_path, ec_path) {
        (Some(_), None, _) => NO_CC_FOLDER.to_string(),
        (Some(format), Some(_), _) => map_not_found_text(map_id, format),
        (None, _, None) => NO_EC_FOLDER.to_string(),
        (None, _, Some(_)) => match u8::try_from(map_id) {
            Ok(facet) => format!("{} was not found in the Enhanced Client folder.", radar_dds_file_name(facet)),
            Err(_) => format!("The Enhanced Client has no radar image for map {map_id}."),
        },
    }
}

/// Which sources the folders hold for the map, looked for again only when the folders or the map changed. When
/// they changed and the chosen source is not there, the first source found is chosen.
fn refresh_found_sources(state: &mut MultimapConverterState, cc_path: Option<&Path>, ec_path: Option<&Path>) -> [bool; RADAR_SOURCES.len()] {
    if let Some(found) = &state.found {
        if found.cc_path.as_deref() == cc_path && found.ec_path.as_deref() == ec_path && found.map_id == state.map_id {
            return found.present;
        }
    }
    let present = RADAR_SOURCES.map(|(source, _)| source_present(source, cc_path, ec_path, state.map_id));
    let chosen = RADAR_SOURCES.iter().position(|&(source, _)| source == state.source);
    if !chosen.is_some_and(|index| present[index]) {
        if let Some(index) = present.iter().position(|&present| present) {
            state.source = RADAR_SOURCES[index].0;
        }
    }
    state.found = Some(FoundRadarSources {
        cc_path: cc_path.map(Path::to_path_buf),
        ec_path: ec_path.map(Path::to_path_buf),
        map_id: state.map_id,
        present,
    });
    present
}

pub fn ui_multimap(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
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

fn loaded_rle_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
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

fn converter_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
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

/// The map, the MUL / CC UOP / EC UOP buttons (a source not found greyed, with a tooltip) and the patches of a
/// Classic map.
fn radar_source_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let (cc_path, ec_path) = (app.settings.cc_path.clone(), app.settings.ec_path.clone());
    let state = &mut app.multimap_converter;
    ui.horizontal(|ui| {
        ui.label("Map");
        ui.add(egui::DragValue::new(&mut state.map_id).range(0..=u32::from(u8::MAX)));
    });
    let present = refresh_found_sources(state, cc_path.as_deref(), ec_path.as_deref());
    let map_id = state.map_id;
    let availability = |source: MultimapRadarSource| match RADAR_SOURCES.iter().position(|&(candidate, _)| candidate == source) {
        Some(index) if present[index] => SourceAvailability::Found(source_found_text(source, map_id)),
        _ => SourceAvailability::Missing(source_missing_text(source, cc_path.as_deref(), ec_path.as_deref(), map_id)),
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

    let folder = if classic { cc_path.map(|path| format!("CC {}", path.display())) } else { ec_path.map(|path| format!("EC {}", path.display())) };
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

fn preview_tabs(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
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

fn preview_actions(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
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

        if ui.button("Save PNG...").clicked() {
            if let Some((name, image)) = selected_image(app) {
                let rgba = image.to_rgba8();
                match super::image_export::export_rgba_png(
                    format!("{name}.png"),
                    image.width,
                    image.height,
                    &rgba,
                ) {
                    Ok(Some(path)) => app.status_message = format!("Saved {path}"),
                    Ok(None) => {}
                    Err(error) => app.status_message = format!("Failed to save PNG: {error}"),
                }
            }
        }
    });
}

fn draw_selected_preview(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
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

fn selected_details(app: &UopInspectorApp) -> Option<PreviewDetails> {
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

fn selected_image(app: &UopInspectorApp) -> Option<(String, MultimapRleImage)> {
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

fn build_generate_request(app: &UopInspectorApp) -> Result<GenerateRequest, String> {
    let state = &app.multimap_converter;
    let (cc_path, ec_path) = (app.settings.cc_path.as_deref(), app.settings.ec_path.as_deref());
    let input = match classic_format(state.source) {
        Some(format) => GenerateInput::ClassicRadar {
            cc_path: cc_path.ok_or_else(|| NO_CC_FOLDER.to_string())?.to_path_buf(),
            map_id: state.map_id,
            format,
            difs: ClassicDifOptions { map_difs: state.include_map_difs, static_difs: state.include_static_difs },
            include_verdata: state.include_verdata,
        },
        None => {
            let path = ec_path
                .and_then(|ec_path| enhanced_radar_path(ec_path, state.map_id))
                .ok_or_else(|| source_missing_text(state.source, cc_path, ec_path, state.map_id))?;
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

fn start_worker(app: &mut UopInspectorApp, request: GenerateRequest) {
    let (tx, rx) = mpsc::channel();
    app.multimap_worker_rx = Some(rx);
    app.multimap_worker_active = true;
    app.multimap_converter.status = "Generating...".to_string();
    thread::spawn(move || {
        let result = generate_multimaps(request).map_err(|error| error.to_string());
        let _ = tx.send(MultimapWorkerResult { result });
    });
}

fn poll_worker(app: &mut UopInspectorApp, ctx: &egui::Context) {
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
