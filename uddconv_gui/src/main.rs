use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use color_eyre::eyre;
use uddconv::{
    cc_art::{CcArtAtlasOptions, convert_art_mul_to_cc_art_uddp_from_sources, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    ec_art::{EcArtAtlasOptions, convert_ec_art_uop_to_ec_art_uddp_from_sources},
    ec_land::{EcLandAtlasOptions, convert_ec_land_uop_to_ec_land_uddp_from_sources},
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources},
    cc_map::convert_map_mul_to_uddp_from_sources,
    cc_statics::convert_statics_mul_to_uddp_from_sources,
    cc_radar::{build_facet_radar_dds, RadarFormat, RadarBuildOptions},
    source_paths::gather_source_dirs,
};
use uddconv_cli::{
    package_info::get_package_info_string,
    extract::extract_package,
    tool_cli::{diff_paths, DiffKind},
};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([900.0, 700.0])
            .with_title("UODynamapper Asset Converter"),
        ..Default::default()
    };

    eframe::run_native(
        "uddconv_gui",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let app = UddConvApp::new(cc);
            Ok(Box::new(app))
        }),
    ).map_err(|e| eyre::eyre!("eframe error: {}", e))?;

    Ok(())
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct AppSettings {
    cc_dir: Option<PathBuf>,
    ec_dir: Option<PathBuf>,
    input_uddp_dir: PathBuf,
    output_uddp_dir: PathBuf,
    link_uddp_dirs: bool,
    bc7_cc_art: bool,
    bc7_ec_art: bool,
    bc7_ec_land: bool,
    radar_format: RadarFormat,
    radar_zstd: i32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cc_dir: None,
            ec_dir: None,
            input_uddp_dir: PathBuf::from("packages"),
            output_uddp_dir: PathBuf::from("packages"),
            link_uddp_dirs: true,
            bc7_cc_art: false,
            bc7_ec_art: false,
            bc7_ec_land: false,
            radar_format: RadarFormat::Bc7,
            radar_zstd: 3,
        }
    }
}

struct LogMessage {
    text: String,
    level: LogLevel,
}

#[derive(PartialEq, Clone, Copy)]
enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

struct UddConvApp {
    settings: AppSettings,
    logs: Arc<Mutex<Vec<LogMessage>>>,
    is_converting: Arc<Mutex<bool>>,
    current_tab: Tab,

    // Tool state
    tool_file_1: Option<PathBuf>,
    tool_file_2: Option<PathBuf>,
    preview_path: Option<PathBuf>,
    preview_texture: Option<egui::TextureHandle>,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Sources,
    Assets,
    World,
    Tools,
}

impl UddConvApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            AppSettings::default()
        };

        Self {
            settings,
            logs: Arc::new(Mutex::new(vec![LogMessage {
                text: "Application started. Please configure your source directories.".to_string(),
                level: LogLevel::Info,
            }])),
            is_converting: Arc::new(Mutex::new(false)),
            current_tab: Tab::Sources,
            tool_file_1: None,
            tool_file_2: None,
            preview_path: None,
            preview_texture: None,
        }
    }

    fn get_output_path(&self, filename: &str) -> PathBuf {
        if !self.settings.output_uddp_dir.exists() {
            let _ = std::fs::create_dir_all(&self.settings.output_uddp_dir);
        }
        self.settings.output_uddp_dir.join(filename)
    }

    fn get_input_uddp_path(&self, filename: &str) -> PathBuf {
        self.settings.input_uddp_dir.join(filename)
    }
}

impl eframe::App for UddConvApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());

        self.ui_preview_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.heading("UODynamapper Asset Converter");
                ui.add_space(15.0);

                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.current_tab, Tab::Sources, "📁 Sources");
                    ui.selectable_value(&mut self.current_tab, Tab::Assets, "🎨 Assets");
                    ui.selectable_value(&mut self.current_tab, Tab::World, "🌍 World");
                    ui.selectable_value(&mut self.current_tab, Tab::Tools, "🛠 Tools");
                });

                ui.add_space(5.0);
                ui.separator();
                ui.add_space(15.0);

                egui::ScrollArea::vertical()
                    .id_salt("main_scroll")
                    .show(ui, |ui| {
                        match self.current_tab {
                            Tab::Sources => self.ui_sources(ui),
                            Tab::Assets => self.ui_assets(ui),
                            Tab::World => self.ui_world(ui),
                            Tab::Tools => self.ui_tools(ui),
                        }
                    });

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.heading("Logs");
                    if ui.button("Clear").clicked() {
                        if let Ok(mut logs) = self.logs.lock() {
                            logs.clear();
                        }
                    }
                    if *self.is_converting.lock().unwrap() {
                        ui.spinner();
                        ui.label("Processing...");
                    }
                });

                ui.add_space(5.0);
                let text_edit_id = ui.make_persistent_id("log_view");
                egui::ScrollArea::vertical()
                    .id_salt(text_edit_id)
                    .auto_shrink([false, false])
                    .max_height(250.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if let Ok(logs) = self.logs.lock() {
                            for log in logs.iter() {
                                let color = match log.level {
                                    LogLevel::Info => egui::Color32::from_gray(200),
                                    LogLevel::Success => egui::Color32::from_rgb(100, 255, 100),
                                    LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                                    LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                                };
                                ui.colored_label(color, &log.text);
                            }
                        }
                    });
            });
        });

        if *self.is_converting.lock().unwrap() {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }
}

impl UddConvApp {
    fn ui_sources(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Source Directories");
                ui.label("Configure where your Ultima Online client files are located.");
                ui.add_space(10.0);

                egui::Grid::new("source_grid")
                    .num_columns(3)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Classic Client (CC):");
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.settings.cc_dir = Some(path);
                            }
                        }
                        if let Some(path) = &self.settings.cc_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(egui::Color32::LIGHT_RED, "Not selected (required for CC Art/Map)");
                        }
                        ui.end_row();

                        ui.label("Enhanced Client (EC):");
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.settings.ec_dir = Some(path);
                            }
                        }
                        if let Some(path) = &self.settings.ec_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(egui::Color32::LIGHT_RED, "Not selected (required for EC Art/Land)");
                        }
                        ui.end_row();

                        ui.label("Input UDDP Folder:");
                        ui.horizontal(|ui| {
                            if ui.button("Select...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.settings.input_uddp_dir = path.clone();
                                    if self.settings.link_uddp_dirs {
                                        self.settings.output_uddp_dir = path;
                                    }
                                }
                            }
                            ui.checkbox(&mut self.settings.link_uddp_dirs, "Link to Output");
                        });
                        ui.label(self.settings.input_uddp_dir.to_string_lossy());
                        ui.end_row();

                        ui.label("Output UDDP Folder:");
                        ui.add_enabled_ui(!self.settings.link_uddp_dirs, |ui| {
                            if ui.button("Select...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.settings.output_uddp_dir = path;
                                }
                            }
                        });
                        ui.label(self.settings.output_uddp_dir.to_string_lossy());
                        ui.end_row();
                    });
            });

            ui.add_space(15.0);
            ui.label("Atlas size and gutter are automatically managed for optimal compatibility.");
        });
    }

    fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Asset Packing");
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(15.0);

            let is_busy = *self.is_converting.lock().unwrap();

            ui.add_enabled_ui(!is_busy, |ui| {
                ui.spacing_mut().item_spacing.y = 15.0;

                if draw_asset_row(ui, "Pack CC Art", "Classic items and land textures (art.mul)", Some(&mut self.settings.bc7_cc_art)) {
                    self.convert_cc_art();
                }
                if draw_asset_row(ui, "Pack EC Art", "Enhanced Client static items (worldart)", Some(&mut self.settings.bc7_ec_art)) {
                    self.convert_ec_art();
                }
                if draw_asset_row(ui, "Pack EC Land", "Enhanced Client high-res terrain textures", Some(&mut self.settings.bc7_ec_land)) {
                    self.convert_ec_land();
                }
                if draw_asset_row(ui, "Pack Tilemeta", "Unified metadata and radar color data", None) {
                    self.convert_tilemeta();
                }
            });
        });
    }

    fn ui_world(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("World Data Packing");
            ui.label("Convert map and statics mul files into optimized block packages.");
            ui.add_space(15.0);

            let is_busy = *self.is_converting.lock().unwrap();

            for map_id in 0..=5 {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.set_min_width(80.0);
                        ui.label(egui::RichText::new(format!("Map {}", map_id)).strong());

                        ui.add_enabled_ui(!is_busy, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;

                                if ui.button("Map").clicked() {
                                    self.convert_map(map_id);
                                }
                                if ui.button("Statics").clicked() {
                                    self.convert_statics(map_id);
                                }
                                if ui.button("RadarMap").clicked() {
                                    self.convert_radar(map_id);
                                }

                                let radar_path = self.get_output_path(&format!("facet0{}.{}", map_id, self.settings.radar_format.extension()));
                                if radar_path.exists() {
                                    if ui.button("👁 View").clicked() {
                                        self.show_preview(&radar_path);
                                    }
                                }

                                egui::ComboBox::from_id_salt(format!("radar_fmt_{}", map_id))
                                    .selected_text(format!("{:?}", self.settings.radar_format))
                                    .width(80.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.settings.radar_format, RadarFormat::Rgba8, "RGBA8");
                                        ui.selectable_value(&mut self.settings.radar_format, RadarFormat::Bc7, "BC7");
                                    });

                                ui.separator();

                                if ui.button(egui::RichText::new("ALL").color(egui::Color32::from_rgb(100, 200, 255))).clicked() {
                                    self.convert_all(map_id);
                                }
                            });
                        });
                    });
                });
                ui.add_space(5.0);
            }
        });
    }

    fn ui_tools(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Package Tools");
            ui.label("Inspect, extract and compare UDDP packages.");
            ui.add_space(15.0);

            let is_busy = *self.is_converting.lock().unwrap();

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Inspect & Extract");
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    if ui.button("📁 Select Package...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("UDDP Packages", &["uddp", "uddpi"])
                            .pick_file() {
                            self.tool_file_1 = Some(path);
                        }
                    }
                    if let Some(path) = &self.tool_file_1 {
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        if ui.button("❌").clicked() { self.tool_file_1 = None; }
                    }
                });

                ui.add_space(5.0);
                ui.add_enabled_ui(!is_busy && self.tool_file_1.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("ℹ️ Get Info").clicked() {
                            self.tool_info();
                        }
                        if ui.button("📦 Extract Contents").clicked() {
                            self.tool_extract();
                        }
                    });
                });
            });

            ui.add_space(15.0);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Compare Packages (Diff)");
                ui.add_space(5.0);

                egui::Grid::new("diff_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Package A:");
                    ui.horizontal(|ui| {
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                self.tool_file_1 = Some(path);
                            }
                        }
                        if let Some(path) = &self.tool_file_1 {
                            ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        }
                    });
                    ui.end_row();

                    ui.label("Package B:");
                    ui.horizontal(|ui| {
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                self.tool_file_2 = Some(path);
                            }
                        }
                        if let Some(path) = &self.tool_file_2 {
                            ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                        }
                    });
                    ui.end_row();
                });

                ui.add_space(10.0);
                ui.add_enabled_ui(!is_busy && self.tool_file_1.is_some() && self.tool_file_2.is_some(), |ui| {
                    if ui.button("🔍 Run Diff").clicked() {
                        self.tool_diff();
                    }
                });
            });
        });
    }

    // --- Task Helpers ---

    fn spawn_task<F>(&self, name: String, task: F)
    where F: FnOnce() -> eyre::Result<String> + Send + 'static
    {
        let is_converting = self.is_converting.clone();
        let logs = self.logs.clone();

        *is_converting.lock().unwrap() = true;

        std::thread::spawn(move || {
            {
                let mut logs = logs.lock().unwrap();
                logs.push(LogMessage {
                    text: format!("[RUN] Starting: {}", name),
                    level: LogLevel::Info,
                });
            }

            let result = task();

            {
                let mut logs = logs.lock().unwrap();
                match result {
                    Ok(msg) => {
                        logs.push(LogMessage {
                            text: format!("[DONE] {}: {}", name, msg),
                            level: LogLevel::Success,
                        });
                    }
                    Err(e) => {
                        logs.push(LogMessage {
                            text: format!("[ERR] {}: {}", name, e),
                            level: LogLevel::Error,
                        });
                    }
                }
            }
            *is_converting.lock().unwrap() = false;
        });
    }

    fn convert_cc_art(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("cc_art.uddp");
        self.spawn_task("CC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            let summary = convert_art_mul_to_cc_art_uddp_from_sources(
                &sources, &output,
                &CcArtAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    use_bc7: settings.bc7_cc_art
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    fn convert_ec_art(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("ec_art.uddp");
        self.spawn_task("EC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            let summary = convert_ec_art_uop_to_ec_art_uddp_from_sources(
                &sources, &output,
                &EcArtAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    crop_transparent_bounds: false,
                    use_bc7: settings.bc7_ec_art
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    fn convert_ec_land(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("ec_land.uddp");
        self.spawn_task("EC Land Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            let summary = convert_ec_land_uop_to_ec_land_uddp_from_sources(
                &sources, &output,
                &EcLandAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    use_bc7: settings.bc7_ec_land
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    fn convert_tilemeta(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tilemeta.uddp");
        self.spawn_task("Tilemeta Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            build_tilemeta_uddp_from_sources(&sources, &output, &TileMetaBuildOptions { adjust_cropped_ec_art: false, use_ec_radarcol: false })?;
            Ok(format!("Wrote tilemeta.uddp to {}", output.display()))
        });
    }

    fn convert_map(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("map{}.uddp", map_id));
        self.spawn_task(format!("Map {} Packing", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            let summary = convert_map_mul_to_uddp_from_sources(&sources, &output, map_id)?;
            Ok(format!("Wrote {} blocks to {}", summary.block_count, output.display()))
        });
    }

    fn convert_statics(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("statics{}.uddp", map_id));
        self.spawn_task(format!("Statics {} Packing", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            let summary = convert_statics_mul_to_uddp_from_sources(&sources, &output, map_id)?;
            Ok(format!("Wrote {} blocks to {}", summary.block_count, output.display()))
        });
    }

    fn convert_radar(&self, map_id: u32) {
        let settings = self.settings.clone();
        let tilemeta_path = self.get_input_uddp_path("tilemeta.uddp");
        let output = self.get_output_path(&format!("facet0{}.{}", map_id, settings.radar_format.extension()));
        self.spawn_task(format!("RadarMap {} Generation", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if !tilemeta_path.exists() {
                eyre::bail!("tilemeta.uddp not found in input UDDP directory. Pack Tilemeta first!");
            }
            if settings.radar_format == RadarFormat::Bc7Ktx2 {
                uddconv_ktx2::build_facet_radar_ktx2(
                    &sources,
                    &tilemeta_path,
                    &output,
                    map_id,
                    settings.radar_zstd,
                )?;
            } else {
                build_facet_radar_dds(&sources, &tilemeta_path, &output, map_id, &RadarBuildOptions {
                    format: settings.radar_format,
                    zstd_level: settings.radar_zstd,
                })?;
            }
            Ok(format!("Wrote radar texture to {}", output.display()))
        });
    }

    fn show_preview(&mut self, path: &std::path::Path) {
        self.preview_path = Some(path.to_path_buf());
        self.preview_texture = None;
    }

    fn ui_preview_window(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.preview_path.clone() {
            let mut open = true;
            egui::Window::new(format!("Preview: {}", path.file_name().unwrap().to_string_lossy()))
                .open(&mut open)
                .resizable(true)
                .default_size([800.0, 800.0])
                .show(ctx, |ui| {
                    if self.preview_texture.is_none() {
                        if let Ok(data) = std::fs::read(&path) {
                            if path.extension().map_or(false, |ext| ext == "ktx2") {
                                if let Ok(reader) = ktx2::Reader::new(&data) {
                                    let header = reader.header();
                                    let width = header.pixel_width;
                                    let height = header.pixel_height;
                                    if header.format == Some(ktx2::Format::BC7_UNORM_BLOCK) {
                                        let level0 = reader.levels().next().unwrap();
                                        let mut blocks = level0.data.to_vec();
                                        if header.supercompression_scheme == Some(ktx2::SupercompressionScheme::Zstandard) {
                                            if let Ok(decompressed) = zstd::decode_all(std::io::Cursor::new(&blocks)) {
                                                blocks = decompressed;
                                            }
                                        }
                                        let extent = uddconv::bc7::ImageExtent::new(width, height).unwrap();
                                        if let Ok(rgba) = uddconv::bc7::decode_bc7_to_rgba8888(&blocks, extent) {
                                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                                [width as usize, height as usize],
                                                &rgba,
                                            );
                                            self.preview_texture = Some(ctx.load_texture("preview", color_image, Default::default()));
                                        }
                                    }
                                }
                            } else {
                                if let Ok(image) = image::load_from_memory(&data) {
                                    let size = [image.width() as usize, image.height() as usize];
                                    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.to_rgba8().as_flat_samples().as_slice());
                                    self.preview_texture = Some(ctx.load_texture("preview", color_image, Default::default()));
                                }
                            }
                        }
                    }

                    if let Some(texture) = &self.preview_texture {
                        egui::ScrollArea::both().show(ui, |ui| {
                            ui.image(texture);
                        });
                    }
                });
            if !open {
                self.preview_path = None;
                self.preview_texture = None;
            }
        }
    }

    fn convert_all(&self, map_id: u32) {
        let settings = self.settings.clone();
        let tilemeta_path = self.get_input_uddp_path("tilemeta.uddp");

        self.spawn_task(format!("Full Map {} Batch", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());

            // 1. Map
            let map_output = settings.output_uddp_dir.join(format!("map{}.uddp", map_id));
            convert_map_mul_to_uddp_from_sources(&sources, &map_output, map_id)?;

            // 2. Statics
            let statics_output = settings.output_uddp_dir.join(format!("statics{}.uddp", map_id));
            convert_statics_mul_to_uddp_from_sources(&sources, &statics_output, map_id)?;

            // 3. Radar
            let radar_output = settings.output_uddp_dir.join(format!("facet0{}.{}", map_id, settings.radar_format.extension()));
            if tilemeta_path.exists() {
                if settings.radar_format == RadarFormat::Bc7Ktx2 {
                    uddconv_ktx2::build_facet_radar_ktx2(
                        &sources,
                        &tilemeta_path,
                        &radar_output,
                        map_id,
                        settings.radar_zstd,
                    )?;
                } else {
                    build_facet_radar_dds(&sources, &tilemeta_path, &radar_output, map_id, &RadarBuildOptions {
                        format: settings.radar_format,
                        zstd_level: settings.radar_zstd,
                    })?;
                }
            } else {
                return Ok(format!("Map {} and Statics {} complete, but RadarMap skipped (tilemeta.uddp missing in input dir).", map_id, map_id));
            }

            Ok(format!("Map, Statics, and RadarMap for Map {} successfully generated.", map_id))
        });
    }

    // --- Tool Task Helpers ---

    fn tool_info(&self) {
        let file = self.tool_file_1.clone().unwrap();
        self.spawn_task(format!("Info: {}", file.file_name().unwrap().to_string_lossy()), move || {
            get_package_info_string(&file)
        });
    }

    fn tool_extract(&self) {
        let file = self.tool_file_1.clone().unwrap();
        self.spawn_task(format!("Extract: {}", file.file_name().unwrap().to_string_lossy()), move || {
            extract_package(&file, None)?;
            Ok(format!("Extracted to sidecar folder next to {}", file.display()))
        });
    }

    fn tool_diff(&self) {
        let left = self.tool_file_1.clone().unwrap();
        let right = self.tool_file_2.clone().unwrap();
        self.spawn_task("Diff Packages".to_string(), move || {
            // Note: diff_paths prints to stdout, so this won't show in the GUI logs
            // unless we refactor it too. For now we just call it.
            diff_paths(&left, &right, DiffKind::Auto)?;
            Ok("Diff complete. Check console for output.".to_string())
        });
    }
}

fn draw_asset_row(ui: &mut egui::Ui, title: &str, desc: &str, bc7: Option<&mut bool>) -> bool {
    let row_width = ui.available_width().min(700.0);
    let btn_size = egui::vec2(160.0, 55.0);
    let mut clicked = false;

    ui.group(|ui| {
        ui.set_width(row_width);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(btn_size.x);
                if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new(title).strong())).clicked() {
                    clicked = true;
                }
                if let Some(bc7_val) = bc7 {
                    ui.checkbox(bc7_val, "BC7 Compression");
                }
            });

            ui.add_space(25.0);

            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).strong().size(18.0));
                ui.add_space(4.0);
                ui.label(desc);
            });
        });
    });

    clicked
}
