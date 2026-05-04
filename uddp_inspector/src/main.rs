use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::path::PathBuf;
use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};
use uocf::udd::uddp::{
    UddpReader, FileKey, Codec, 
    unpack_type, unpack_codec, unpack_offset40, reconstruct_stored_size, xxh64_virtual_path
};
use color_eyre::eyre;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("UDDP Package Inspector"),
        ..Default::default()
    };

    eframe::run_native(
        "uddp_inspector",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let app = InspectorApp::new(cc);
            Ok(Box::new(app))
        }),
    ).map_err(|e| eyre::eyre!("eframe error: {}", e))?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Package,
    Virtual,
}

struct InspectorApp {
    package: Option<UddpReader>,
    entries: Vec<EntryInfo>,
    selected_idx: Option<usize>,
    filter: String,
    package_path: Option<PathBuf>,
    
    // Virtual View state
    view_mode: ViewMode,
    virtual_entries: Vec<VirtualEntry>,
    selected_virtual_idx: Option<usize>,
    
    // Preview state
    preview_text: Option<String>,
    preview_texture: Option<egui::TextureHandle>,
}

struct EntryInfo {
    key: FileKey,
    raw_size: u32,
    stored_size: u32,
    data_type: u8,
    codec: Codec,
    offset: u64,
}

#[derive(Debug, Clone)]
struct VirtualEntry {
    id: u32,
    page_index: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    flags: u16,
    kind: String, // "CC Art", "EC Art", etc.
}

impl InspectorApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            package: None,
            entries: Vec::new(),
            selected_idx: None,
            filter: String::new(),
            package_path: None,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
        }
    }

    fn read_file_by_path(&self, reader: &UddpReader, path: &str) -> Option<Vec<u8>> {
        let hash = xxh64_virtual_path(path);
        reader.read_file_by_path_hash(hash).ok()
    }

    fn open_package(&mut self, path: PathBuf) {
        match UddpReader::load(&path) {
            Ok(reader) => {
                let records = reader.records();
                self.entries = records.into_iter().map(|r| {
                    EntryInfo {
                        key: r.key,
                        raw_size: r.locator.raw_size,
                        stored_size: reconstruct_stored_size(r.locator.raw_size, r.locator.meta32, r.locator.pos64),
                        data_type: unpack_type(r.locator.meta32),
                        codec: unpack_codec(r.locator.meta32),
                        offset: unpack_offset40(r.locator.pos64),
                    }
                }).collect();
                
                self.virtual_entries.clear();
                self.detect_virtual_entries(&reader);
                
                self.package = Some(reader);
                self.package_path = Some(path);
                self.selected_idx = None;
                self.selected_virtual_idx = None;
                self.preview_text = None;
                self.preview_texture = None;
                self.view_mode = ViewMode::Package;
            }
            Err(e) => {
                println!("Error opening package: {}", e);
            }
        }
    }

    fn detect_virtual_entries(&mut self, reader: &UddpReader) {
        let slot_manifest_path = "metadata/slots.bin";
        if let Some(data) = self.read_file_by_path(reader, slot_manifest_path) {
            if data.len() >= 24 {
                let mut cursor = Cursor::new(&data);
                let mut magic = [0u8; 4];
                let _ = cursor.read_exact(&mut magic);
                
                let kind = if &magic == b"CASL" { "CC Art" } else if &magic == b"EASL" { "EC Art" } else { "Unknown Slot" };
                
                if kind != "Unknown Slot" {
                    let _version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                    let _atlas_w = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                    let _atlas_h = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                    let count = cursor.read_u32::<LittleEndian>().unwrap_or(0);
                    
                    for _ in 0..count {
                        if let (Ok(id), Ok(page), Ok(_pindex), Ok(flags), Ok(x), Ok(y), Ok(w), Ok(h)) = (
                            cursor.read_u32::<LittleEndian>(),
                            cursor.read_u32::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                            cursor.read_u16::<LittleEndian>(),
                        ) {
                            if (flags & 1) != 0 { // Present flag
                                self.virtual_entries.push(VirtualEntry {
                                    id, page_index: page, x, y, width: w, height: h, flags, kind: kind.to_string()
                                });
                            }
                        } else { break; }
                    }
                }
            }
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.package {
            if self.view_mode == ViewMode::Package {
                if let Some(idx) = self.selected_idx {
                    let entry = &self.entries[idx];
                    let data = match entry.key {
                        FileKey::Id(id) => {
                            match reader.lookup_mode() {
                                uocf::udd::uddp::LookupMode::DenseId => reader.read_file_by_dense_id(id),
                                uocf::udd::uddp::LookupMode::SparseId => reader.read_file_by_sparse_id(id),
                                _ => unreachable!(),
                            }
                        }
                        FileKey::PathHash(h) => reader.read_file_by_path_hash(h),
                    }.ok();
                    
                    if let Some(data) = data {
                        self.decode_package_data(ctx, data);
                    }
                }
            } else {
                if let Some(idx) = self.selected_virtual_idx {
                    let ventry = self.virtual_entries[idx].clone();
                    // Virtual entry: load page, then crop
                    let mut page_data: Option<Vec<u8>> = None;
                    for ext in &["bc7", "rgba8888", "bin"] {
                        let path = format!("pages/{}.{}", ventry.page_index, ext);
                        if let Some(data) = self.read_file_by_path(reader, &path) {
                            page_data = Some(data);
                            break;
                        }
                    }
                    
                    if let Some(data) = page_data {
                        // Decode page to RGBA
                        let rgba: Option<Vec<u8>> = if data.starts_with(b"UDT1") {
                            if let Ok(vram) = uddconv::bc7::VramTextureData::from_container_bytes(&data) {
                                uddconv::bc7::decode_from_vram(&vram, uddconv::bc7::RawImageFormat::Rgba8888).ok()
                            } else { None }
                        } else if data.starts_with(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB]) {
                             if let Ok(reader) = ktx2::Reader::new(&data) {
                                let header = reader.header();
                                if header.format == Some(ktx2::Format::BC7_UNORM_BLOCK) {
                                    let level0 = reader.levels().next().unwrap();
                                    let mut blocks = level0.data.to_vec();
                                    if header.supercompression_scheme == Some(ktx2::SupercompressionScheme::Zstandard) {
                                        if let Ok(decompressed) = zstd::decode_all(std::io::Cursor::new(&blocks)) {
                                            blocks = decompressed;
                                        }
                                    }
                                    uddconv::bc7::decode_bc7_to_rgba8888(&blocks, uddconv::bc7::ImageExtent::new(header.pixel_width, header.pixel_height).unwrap()).ok()
                                } else { None }
                             } else { None }
                        } else {
                            // Try standard image
                            if let Ok(image) = image::load_from_memory(&data) {
                                Some(image.to_rgba8().into_raw())
                            } else {
                                if data.len() == 2048 * 2048 * 4 {
                                    Some(data)
                                } else { None }
                            }
                        };
                        
                        if let Some(rgba) = rgba {
                            let total_pixels = rgba.len() / 4;
                            let page_w = if total_pixels == 4096 * 2048 { 4096 } else { 2048 };
                            
                            let mut cropped = Vec::with_capacity(ventry.width as usize * ventry.height as usize * 4);
                            for py in 0..ventry.height {
                                let start = ((ventry.y + py) as usize * page_w + ventry.x as usize) * 4;
                                let end = start + ventry.width as usize * 4;
                                if end <= rgba.len() {
                                    cropped.extend_from_slice(&rgba[start..end]);
                                }
                            }
                            
                            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                                [ventry.width as usize, ventry.height as usize],
                                &cropped,
                            );
                            self.preview_texture = Some(ctx.load_texture("cropped_preview", color_image, Default::default()));
                            self.preview_text = Some(format!("{} ID: {} ({}x{})", ventry.kind, ventry.id, ventry.width, ventry.height));
                        }
                    }
                }
            }
        }
    }

    fn decode_package_data(&mut self, ctx: &egui::Context, data: Vec<u8>) {
        // 1. Try KTX2
        if data.starts_with(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB]) {
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
                    if let Ok(rgba) = uddconv::bc7::decode_bc7_to_rgba8888(&blocks, uddconv::bc7::ImageExtent::new(width, height).unwrap()) {
                        let color_image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
                        self.preview_texture = Some(ctx.load_texture("ktx2_preview", color_image, Default::default()));
                        self.preview_text = Some(format!("KTX2 BC7 Texture: {}x{}", width, height));
                        return;
                    }
                }
            }
        }

        // 2. Try VRAM Container (UDT1)
        if data.starts_with(b"UDT1") {
            if let Ok(vram) = uddconv::bc7::VramTextureData::from_container_bytes(&data) {
                let extent = vram.extent();
                if let Ok(rgba) = uddconv::bc7::decode_from_vram(&vram, uddconv::bc7::RawImageFormat::Rgba8888) {
                    let color_image = egui::ColorImage::from_rgba_unmultiplied([extent.width() as usize, extent.height() as usize], &rgba);
                    self.preview_texture = Some(ctx.load_texture("vram_preview", color_image, Default::default()));
                    self.preview_text = Some(format!("VRAM Texture: {}x{} ({:?})", extent.width(), extent.height(), vram.format()));
                    return;
                }
            }
        }

        // 3. Try standard image formats
        if let Ok(image) = image::load_from_memory(&data) {
            let size = [image.width() as usize, image.height() as usize];
            let pixels = image.to_rgba8().into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
            self.preview_texture = Some(ctx.load_texture("img_preview", color_image, Default::default()));
            self.preview_text = Some(format!("Standard Image: {}x{} ({:?})", size[0], size[1], image.color()));
            return;
        }

        // 4. Try to guess raw RGBA8888 pages
        let total_pixels = data.len() / 4;
        if data.len() % 4 == 0 && total_pixels > 0 {
            let side = (total_pixels as f32).sqrt() as u32;
            if side * side == total_pixels as u32 && (side == 2048 || side == 1024 || side == 512 || side == 256 || side == 4096) {
                let color_image = egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], &data);
                self.preview_texture = Some(ctx.load_texture("guessed_rgba", color_image, Default::default()));
                self.preview_text = Some(format!("Guessed Raw RGBA: {}x{}", side, side));
                return;
            }
        }

        // 5. Try string
        if let Ok(text) = String::from_utf8(data.clone()) {
            self.preview_text = Some(text);
        } else {
            // 6. Hex view fallback
            let mut hex = String::new();
            for (i, byte) in data.iter().take(1024).enumerate() {
                if i > 0 && i % 16 == 0 { hex.push('\n'); }
                hex.push_str(&format!("{:02X} ", byte));
            }
            if data.len() > 1024 { hex.push_str("\n..."); }
            self.preview_text = Some(hex);
        }
    }

    fn extract_payload(&self, idx: usize) {
        if let Some(reader) = &self.package {
            let entry = &self.entries[idx];
            let result = match entry.key {
                FileKey::Id(id) => {
                    match reader.lookup_mode() {
                        uocf::udd::uddp::LookupMode::DenseId => reader.read_file_by_dense_id(id),
                        uocf::udd::uddp::LookupMode::SparseId => reader.read_file_by_sparse_id(id),
                        _ => unreachable!(),
                    }
                }
                FileKey::PathHash(h) => reader.read_file_by_path_hash(h),
            };

            if let Ok(data) = result {
                let default_name = match self.entries[idx].key {
                    FileKey::Id(id) => format!("entry_{}.bin", id),
                    FileKey::PathHash(h) => format!("0x{:016X}.bin", h),
                };

                if let Some(path) = rfd::FileDialog::new().set_file_name(&default_name).save_file() {
                    let _ = std::fs::write(path, data);
                }
            }
        }
    }
}

impl eframe::App for InspectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Open UDDP...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("UDDP Packages", &["uddp", "uddpi"]).pick_file() {
                        self.open_package(path);
                    }
                }
                if let Some(path) = &self.package_path {
                    ui.label(egui::RichText::new(path.to_string_lossy()).strong());
                }
            });
        });

        egui::SidePanel::left("left_panel").resizable(true).default_width(300.0).show(ctx, |ui| {
            ui.heading("Package Info");
            if let Some(reader) = &self.package {
                let header = reader.header();
                egui::Grid::new("header_grid").show(ui, |ui| {
                    ui.label("Version:"); ui.label(format!("{}.{}", header.version_major, header.version_minor)); ui.end_row();
                    ui.label("Lookup Mode:"); ui.label(format!("{:?}", reader.lookup_mode())); ui.end_row();
                    ui.label("Files:"); ui.label(header.file_count.to_string()); ui.end_row();
                });
                
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                
                if !self.virtual_entries.is_empty() {
                    ui.heading("View Mode");
                    ui.horizontal(|ui| {
                        let mut virtual_mode = self.view_mode == ViewMode::Virtual;
                        if ui.checkbox(&mut virtual_mode, "Virtual View (per entry)").changed() {
                            self.view_mode = if virtual_mode { ViewMode::Virtual } else { ViewMode::Package };
                            self.selected_idx = None;
                            self.selected_virtual_idx = None;
                            self.preview_text = None;
                            self.preview_texture = None;
                        }
                    });
                    ui.add_space(10.0);
                }

                ui.label("Filter ID / Name:");
                ui.text_edit_singleline(&mut self.filter);
                
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                
                ui.heading("Dictionaries");
                let dicts = reader.dictionary_records();
                if dicts.is_empty() {
                    ui.label("No dictionaries.");
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        egui::Grid::new("dict_grid").show(ui, |ui| {
                            for (dtype, codec, size) in dicts {
                                ui.label(format!("T{}:", dtype));
                                ui.label(format!("{:?} ({})", codec, format_size(size as u64)));
                                ui.end_row();
                            }
                        });
                    });
                }
            } else {
                ui.label("No package loaded.");
            }
        });

        if let Some(idx) = if self.view_mode == ViewMode::Package { self.selected_idx } else { self.selected_virtual_idx } {
            egui::SidePanel::right("right_panel").resizable(true).default_width(450.0).show(ctx, |ui| {
                self.ui_details(ctx, ui, idx);
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.package.is_some() {
                self.render_table(ctx, ui);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a .uddp file to begin inspection.");
                });
            }
        });
    }
}

impl InspectorApp {
    fn ui_details(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui, idx: usize) {
        ui.horizontal(|ui| {
            ui.heading("Entry Details");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.view_mode == ViewMode::Package {
                    if ui.button("💾 Extract").clicked() { self.extract_payload(idx); }
                }
                if ui.button("✖").clicked() {
                    self.selected_idx = None;
                    self.selected_virtual_idx = None;
                }
            });
        });
        ui.add_space(5.0);
        
        if self.view_mode == ViewMode::Package {
            let entry = &self.entries[idx];
            ui.label(egui::RichText::new(format!("{:?}", entry.key)).strong());
            ui.separator();
            egui::Grid::new("detail_grid").show(ui, |ui| {
                ui.label("Data Type:"); ui.label(format!("{:?} ({})", data_type_to_str(entry.data_type), entry.data_type)); ui.end_row();
                ui.label("Codec:"); ui.label(format!("{:?}", entry.codec)); ui.end_row();
                ui.label("Raw Size:"); ui.label(format_size(entry.raw_size as u64)); ui.end_row();
                ui.label("Stored Size:"); ui.label(format_size(entry.stored_size as u64)); ui.end_row();
            });
        } else {
            let ventry = &self.virtual_entries[idx];
            ui.label(egui::RichText::new(format!("{} ID: {}", ventry.kind, ventry.id)).strong());
            ui.separator();
            egui::Grid::new("detail_grid_v").show(ui, |ui| {
                ui.label("Page Index:"); ui.label(ventry.page_index.to_string()); ui.end_row();
                ui.label("Rect:"); ui.label(format!("{},{} - {}x{}", ventry.x, ventry.y, ventry.width, ventry.height)); ui.end_row();
                ui.label("Flags:"); ui.label(format!("0x{:04X}", ventry.flags)); ui.end_row();
            });
        }
        
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);
        ui.heading("Content Preview");
        
        if let Some(texture) = &self.preview_texture {
            ui.label(self.preview_text.as_deref().unwrap_or("Image Preview"));
            egui::ScrollArea::both().show(ui, |ui| {
                ui.image(texture);
            });
        } else if let Some(text) = &self.preview_text {
            egui::ScrollArea::vertical().max_height(ui.available_height() - 20.0).show(ui, |ui| {
                let mut t = text.as_str();
                ui.add(egui::TextEdit::multiline(&mut t).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).interactive(false));
            });
        } else {
            ui.label("Loading preview...");
        }
    }

    fn render_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let text_height = egui::TextStyle::Body.resolve(ui.style()).size;
        
        if self.view_mode == ViewMode::Package {
            let filtered_indices: Vec<usize> = self.entries.iter().enumerate().filter(|(_, entry)| {
                if self.filter.is_empty() { return true; }
                let key_str = match entry.key {
                    FileKey::Id(id) => id.to_string(),
                    FileKey::PathHash(h) => format!("0x{:016X}", h),
                };
                let type_str = data_type_to_str(entry.data_type).to_lowercase();
                key_str.contains(&self.filter) || type_str.contains(&self.filter.to_lowercase())
            }).map(|(i, _)| i).collect();

            TableBuilder::new(ui).striped(true).resizable(true).cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(140.0)) // Key
                .column(Column::auto().at_least(80.0))  // Type
                .column(Column::auto().at_least(60.0))  // Codec
                .column(Column::auto().at_least(80.0))  // Raw
                .column(Column::remainder())            // Offset
                .header(20.0, |mut header| {
                    header.col(|ui| { ui.strong("Key / Hash"); });
                    header.col(|ui| { ui.strong("Type"); });
                    header.col(|ui| { ui.strong("Codec"); });
                    header.col(|ui| { ui.strong("Raw"); });
                    header.col(|ui| { ui.strong("Offset"); });
                })
                .body(|body| {
                    body.rows(text_height, filtered_indices.len(), |mut row| {
                        let idx = filtered_indices[row.index()];
                        row.col(|ui| {
                            let text = match self.entries[idx].key { FileKey::Id(id) => id.to_string(), FileKey::PathHash(h) => format!("0x{:016X}", h) };
                            if ui.selectable_label(self.selected_idx == Some(idx), text).clicked() { self.select_entry(ctx, idx); }
                        });
                        row.col(|ui| { ui.label(data_type_to_str(self.entries[idx].data_type)); });
                        row.col(|ui| { ui.label(format!("{:?}", self.entries[idx].codec)); });
                        row.col(|ui| { ui.label(format_size(self.entries[idx].raw_size as u64)); });
                        row.col(|ui| { ui.label(format!("0x{:08X}", self.entries[idx].offset)); });
                    });
                });
        } else {
            let filtered_indices: Vec<usize> = self.virtual_entries.iter().enumerate().filter(|(_, entry)| {
                if self.filter.is_empty() { return true; }
                entry.id.to_string().contains(&self.filter)
            }).map(|(i, _)| i).collect();

            TableBuilder::new(ui).striped(true).resizable(true).cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(80.0))  // ID
                .column(Column::auto().at_least(60.0))  // Page
                .column(Column::auto().at_least(60.0))  // X
                .column(Column::auto().at_least(60.0))  // Y
                .column(Column::auto().at_least(80.0))  // Dimensions
                .column(Column::remainder())            // Kind
                .header(20.0, |mut header| {
                    header.col(|ui| { ui.strong("Art ID"); });
                    header.col(|ui| { ui.strong("Page"); });
                    header.col(|ui| { ui.strong("X"); });
                    header.col(|ui| { ui.strong("Y"); });
                    header.col(|ui| { ui.strong("Size"); });
                    header.col(|ui| { ui.strong("Kind"); });
                })
                .body(|body| {
                    body.rows(text_height, filtered_indices.len(), |mut row| {
                        let idx = filtered_indices[row.index()];
                        let id_str = self.virtual_entries[idx].id.to_string();
                        row.col(|ui| {
                            if ui.selectable_label(self.selected_virtual_idx == Some(idx), id_str).clicked() { self.select_virtual_entry(ctx, idx); }
                        });
                        row.col(|ui| { ui.label(self.virtual_entries[idx].page_index.to_string()); });
                        row.col(|ui| { ui.label(self.virtual_entries[idx].x.to_string()); });
                        row.col(|ui| { ui.label(self.virtual_entries[idx].y.to_string()); });
                        row.col(|ui| { ui.label(format!("{}x{}", self.virtual_entries[idx].width, self.virtual_entries[idx].height)); });
                        row.col(|ui| { ui.label(&self.virtual_entries[idx].kind); });
                    });
                });
        }
    }

    fn select_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_idx != Some(idx) {
            self.selected_idx = Some(idx);
            self.preview_text = None;
            self.preview_texture = None;
            self.load_preview(ctx);
        }
    }

    fn select_virtual_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_virtual_idx != Some(idx) {
            self.selected_virtual_idx = Some(idx);
            self.preview_text = None;
            self.preview_texture = None;
            self.load_preview(ctx);
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{} B", bytes) }
    else if bytes < 1024 * 1024 { format!("{:.1} KiB", bytes as f32 / 1024.0) }
    else { format!("{:.1} MiB", bytes as f32 / (1024.0 * 1024.0)) }
}

fn data_type_to_str(t: u8) -> &'static str {
    match t {
        1 => "Art", 3 => "Map", 9 => "Texture", 11 => "Metadata", 14 => "Static",
        _ => "Other",
    }
}
