use crate::app::{
    GeneratedMultimapPreview, MultimapConversionOutput, MultimapConversionSource,
    MultimapConverterState, MultimapPreviewSelection, MultimapWorkerResult, UopInspectorApp,
};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use uocf::classic::multimap_render::{
    render_multimap, DecodedRgba, MultimapRenderOptions, MultimapStyle,
};
use uocf::classic::multimap_rle::{self, MultimapRleImage, BLACK_PIXEL};

const GENERATED_PLAIN_KEY: u64 = 0x6F00000000000001;
const GENERATED_ARTISTIC_KEY: u64 = 0x6F00000000000002;
const MULTIMAP_TOOLS_PANEL_WIDTH: f32 = 340.0;

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
    Ktx2(PathBuf),
    ClientRadar {
        cc_path: PathBuf,
        ec_path: Option<PathBuf>,
        tilemeta_path: Option<PathBuf>,
        map_id: u32,
        map_source_preference: crate::app::MultimapMapSourcePreference,
        use_ec_radarcol: bool,
        include_verdata: bool,
        include_map_difs: bool,
        include_static_difs: bool,
    },
}

pub fn ui_multimap(app: &mut UopInspectorApp, ctx: &egui::Context) {
    poll_worker(app, ctx);

    egui::SidePanel::left("multimap_tools")
        .resizable(true)
        .default_width(MULTIMAP_TOOLS_PANEL_WIDTH)
        .show(ctx, |ui| {
            ui.heading("Multimap");
            ui.separator();
            loaded_rle_controls(app, ui);
            ui.separator();
            converter_controls(app, ui);
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        preview_tabs(app, ui);
        ui.separator();
        preview_actions(app, ui);
        ui.separator();
        draw_selected_preview(app, ctx, ui);
    });
}

fn loaded_rle_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button("Open RLE...").clicked() {
            if let Some(path) = crate::dialog::file_dialog()
                .add_filter("Multimap RLE", &["rle"])
                .pick_file()
            {
                match multimap_rle::load_rle(&path) {
                    Ok(image) => {
                        app.cc_multimap = Some(std::sync::Arc::new(image));
                        app.cc_multimap_path = Some(path.clone());
                        app.multimap_texture = None;
                        app.multimap_preview_selection = MultimapPreviewSelection::Loaded;
                        app.status_message = format!("Loaded {}", path.display());
                    }
                    Err(error) => {
                        app.status_message = format!("Failed to load RLE: {error}");
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
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut app.multimap_converter.source,
            MultimapConversionSource::Ktx2,
            "KTX2",
        );
        ui.selectable_value(
            &mut app.multimap_converter.source,
            MultimapConversionSource::ClientRadar,
            "Map Data",
        );
    });

    match app.multimap_converter.source {
        MultimapConversionSource::Ktx2 => ktx2_source_controls(&mut app.multimap_converter, ui),
        MultimapConversionSource::ClientRadar => client_radar_controls(app, ui),
    }

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

fn ktx2_source_controls(state: &mut MultimapConverterState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label("File");
        ui.text_edit_singleline(&mut state.ktx2_path);
        if ui.button("...").clicked() {
            if let Some(path) = crate::dialog::file_dialog()
                .add_filter("KTX2", &["ktx2"])
                .pick_file()
            {
                state.ktx2_path = path.display().to_string();
            }
        }
    });
}

fn client_radar_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let state = &mut app.multimap_converter;
    ui.horizontal(|ui| {
        ui.label("Map");
        ui.add(egui::DragValue::new(&mut state.map_id).range(0..=255));
    });
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut state.map_source_preference,
            crate::app::MultimapMapSourcePreference::Mul,
            "MUL",
        );
        ui.selectable_value(
            &mut state.map_source_preference,
            crate::app::MultimapMapSourcePreference::Uop,
            "UOP",
        );
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.include_verdata, "verdata");
        ui.checkbox(&mut state.include_map_difs, "map difs");
        ui.checkbox(&mut state.include_static_difs, "static difs");
    });
    ui.checkbox(&mut state.use_ec_radarcol, "EC radar colors");

    ui.horizontal(|ui| {
        ui.label("tilemeta");
        ui.text_edit_singleline(&mut state.tilemeta_path);
        if ui.button("...").clicked() {
            if let Some(path) = crate::dialog::file_dialog()
                .add_filter("UDDP", &["uddp"])
                .pick_file()
            {
                state.tilemeta_path = path.display().to_string();
            }
        }
    });

    if let Some(path) = &app.settings.cc_path {
        ui.monospace(format!("CC {}", path.display()));
    }
    if let Some(path) = &app.settings.ec_path {
        ui.monospace(format!("EC {}", path.display()));
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
            if let Some((name, image)) = selected_image(app) {
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
        ui.monospace(format!("black {}", details.black_pixels));
        ui.monospace(format!("white {}", details.white_pixels));
    });
    if let Some(source) = details.source {
        ui.monospace(source);
    }
    if let Some(crop) = details.crop {
        ui.monospace(format!("crop {}x{}+{},{}", crop.width, crop.height, crop.x, crop.y));
    }

    ui.add(
        egui::Slider::new(&mut app.multimap_zoom, 0.05..=2.0)
            .logarithmic(true)
            .text("Zoom"),
    );
    let size = egui::vec2(
        details.width as f32 * app.multimap_zoom,
        details.height as f32 * app.multimap_zoom,
    );
    egui::ScrollArea::both().show(ui, |ui| {
        ui.add(egui::Image::new(&handle).fit_to_exact_size(size));
    });
}

struct PreviewDetails {
    label: String,
    width: u32,
    height: u32,
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
    let input = match state.source {
        MultimapConversionSource::Ktx2 => {
            let path = path_from_text(&state.ktx2_path)
                .ok_or_else(|| "Select a KTX2 source file.".to_string())?;
            GenerateInput::Ktx2(path)
        }
        MultimapConversionSource::ClientRadar => {
            let cc_path = app
                .settings
                .cc_path
                .clone()
                .ok_or_else(|| "Set a Classic Client path first.".to_string())?;
            let tilemeta_path = path_from_text(&state.tilemeta_path);
            GenerateInput::ClientRadar {
                cc_path,
                ec_path: app.settings.ec_path.clone(),
                tilemeta_path,
                map_id: state.map_id,
                map_source_preference: state.map_source_preference,
                use_ec_radarcol: state.use_ec_radarcol,
                include_verdata: state.include_verdata,
                include_map_difs: state.include_map_difs,
                include_static_difs: state.include_static_difs,
            }
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

fn path_from_text(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
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

fn load_source_rgba(request: &GenerateRequest) -> color_eyre::eyre::Result<(DecodedRgba, String)> {
    match &request.input {
        GenerateInput::Ktx2(path) => {
            let (width, height, rgba) =
                udd_image_codecs::ktx2::decode_ktx2_bc7_zstd_to_rgba8888(path)?;
            Ok((
                DecodedRgba::new(width, height, rgba)?,
                path.display().to_string(),
            ))
        }
        GenerateInput::ClientRadar {
            cc_path,
            ec_path,
            tilemeta_path,
            map_id,
            map_source_preference,
            use_ec_radarcol,
            include_verdata,
            include_map_difs,
            include_static_difs,
        } => {
            let tilemeta_path = resolve_tilemeta_path(
                cc_path,
                ec_path.as_deref(),
                tilemeta_path.as_deref(),
                *map_id,
                *use_ec_radarcol,
                *include_verdata,
                *include_map_difs,
                *include_static_difs,
            )?;
            let source_dirs = [cc_path.clone()];
            let patch_options = udd_conv::classic_patches::ClassicPatchOptions {
                verdata: *include_verdata,
                map_difs: *include_map_difs,
                static_difs: *include_static_difs,
            };
            let radar_result = udd_conv::cc_radar::build_facet_radar_rgba_with_options(
                &source_dirs,
                &tilemeta_path.path,
                *map_id,
                map_source_preference.to_udd_conv(),
                &patch_options,
            );
            if tilemeta_path.delete_after_use {
                let _ = std::fs::remove_file(&tilemeta_path.path);
            }
            let (width, height, rgba) = radar_result?;
            Ok((
                DecodedRgba::new(width, height, rgba)?,
                format!("map{} radar", map_id),
            ))
        }
    }
}

struct ResolvedTilemetaPath {
    path: PathBuf,
    delete_after_use: bool,
}

fn resolve_tilemeta_path(
    cc_path: &Path,
    ec_path: Option<&Path>,
    explicit_tilemeta_path: Option<&Path>,
    map_id: u32,
    use_ec_radarcol: bool,
    include_verdata: bool,
    include_map_difs: bool,
    include_static_difs: bool,
) -> color_eyre::eyre::Result<ResolvedTilemetaPath> {
    if let Some(path) = explicit_tilemeta_path {
        return Ok(ResolvedTilemetaPath {
            path: path.to_path_buf(),
            delete_after_use: false,
        });
    }

    let path = std::env::temp_dir().join(format!(
        "uocf_inspector_multimap_tilemeta_{}_{}.uddp",
        std::process::id(),
        map_id
    ));
    let patch_options = udd_conv::classic_patches::ClassicPatchOptions {
        verdata: include_verdata,
        map_difs: include_map_difs,
        static_difs: include_static_difs,
    };
    let options = udd_conv::tilemeta::TileMetaBuildOptions {
        use_ec_radarcol,
        classic_patches: patch_options,
        ..Default::default()
    };

    if let Some(ec_path) = ec_path {
        udd_conv::tilemeta::build_tilemeta_uddp_from_split_sources(
            cc_path,
            ec_path,
            &path,
            &options,
        )?;
    } else {
        udd_conv::tilemeta::build_tilemeta_uddp_from_sources(
            &[cc_path.to_path_buf()],
            &path,
            &options,
        )?;
    }

    Ok(ResolvedTilemetaPath {
        path,
        delete_after_use: true,
    })
}
