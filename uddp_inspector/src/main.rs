use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::collections::HashMap;
use std::path::PathBuf;
use std::io::{Cursor, Read};
use bytemuck::{Pod, Zeroable, pod_read_unaligned};
use byteorder::{LittleEndian, ReadBytesExt};
use uocf::udd::uddp::{
    UddpReader, FileKey, Codec,
    unpack_type, unpack_codec, unpack_offset40, reconstruct_stored_size, xxh64_virtual_path
};
use color_eyre::eyre;
use uddconv::tilemeta::{
    TileMetaItemTile, TileMetaLandTile, TILEMETA_ITEM_ENTRY_PATH, TILEMETA_LAND_ENTRY_PATH,
};

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Config {
    package_path: Option<PathBuf>,
    #[serde(default)]
    view_mode_virtual: bool,
    #[serde(default)]
    filter: String,
}

fn config_file_path() -> PathBuf {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest_dir).join("config.toml");
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join("config.toml");
        }
    }
    PathBuf::from("config.toml")
}

fn load_config() -> Config {
    let path = config_file_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        toml::from_str(&contents).unwrap_or_default()
    } else {
        Config::default()
    }
}

fn save_config(cfg: &Config) {
    let path = config_file_path();
    if let Ok(contents) = toml::to_string_pretty(cfg) {
        let _ = std::fs::write(path, contents);
    }
}

const DATA_TYPE_MAP: u8 = 3;
const DATA_TYPE_STATIC: u8 = 14;
const MAX_INLINE_PREVIEW_WIDTH: usize = 800;
const MAX_INLINE_PREVIEW_HEIGHT: usize = 600;

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
    atlas_pages: HashMap<u32, AtlasPageInfo>,
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
    preview_texture_size: Option<[usize; 2]>,
    atlas_texture: Option<egui::TextureHandle>,
    atlas_texture_size: Option<[usize; 2]>,
    atlas_text: Option<String>,
    preview_mode: PreviewModeKind,
    image_window_open: bool,
    image_window_mode: PreviewModeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewModeKind {
    Entry,
    Atlas,
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
    kind: String,
    summary: String,
    location: String,
    data: VirtualEntryData,
}

#[derive(Debug, Clone)]
enum VirtualEntryData {
    AtlasRect {
        page_index: u32,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        flags: u16,
    },
    TileMetaLand(TileMetaLandInfo),
    TileMetaItem(TileMetaItemInfo),
    MapBlock {
        source_entry_idx: usize,
    },
    StaticBlock {
        source_entry_idx: usize,
    },
}

#[derive(Debug, Clone)]
struct TileMetaLandInfo {
    texture_id: u16,
    tile_type: u8,
    flags: u64,
    radar_color: [u8; 4],
    name: String,
}

#[derive(Debug, Clone)]
struct TileMetaItemInfo {
    weight: u8,
    quality: u8,
    quantity: u8,
    hue_extra: u8,
    flags: u64,
    anim_id: u16,
    stacking_offset: u8,
    value: u8,
    height: i8,
    radar_color: [u8; 4],
    name: String,
    ec_texture_id: u32,
    ec_start_x: i16,
    ec_start_y: i16,
    ec_offset_x: i16,
    ec_offset_y: i16,
    cc_texture_id: u32,
    cc_start_x: i16,
    cc_start_y: i16,
    cc_offset_x: i16,
    cc_offset_y: i16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PackedMapTexel {
    tile_id: u16,
    packed_meta: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtlasPixelFormat {
    Rgba8888,
    Bc7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AtlasPageInfo {
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: AtlasPixelFormat,
}

#[derive(Debug, Clone)]
struct DecodedAtlasPage {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

fn normalize_linux_portal_env() {
    #[cfg(target_os = "linux")]
    {
        use std::env;

        let current = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        if !current.is_empty() {
            return;
        }

        let session = env::var("XDG_SESSION_DESKTOP").unwrap_or_default().to_ascii_lowercase();
        let desktop_session = env::var("DESKTOP_SESSION").unwrap_or_default().to_ascii_lowercase();
        let kde_full = env::var("KDE_FULL_SESSION").unwrap_or_default();

        let inferred = if kde_full.eq_ignore_ascii_case("true")
            || session.contains("kde")
            || session.contains("plasma")
            || desktop_session.contains("kde")
            || desktop_session.contains("plasma")
        {
            Some("KDE")
        } else if session.contains("gnome") || desktop_session.contains("gnome") {
            Some("GNOME")
        } else {
            None
        };

        if let Some(desktop) = inferred {
            env::set_var("XDG_CURRENT_DESKTOP", desktop);
        }
    }
}

fn open_package_dialog() -> Option<PathBuf> {
    normalize_linux_portal_env();
    rfd::FileDialog::new()
        .add_filter("UDDP Packages", &["uddp", "uddpi"])
        .pick_file()
}

fn save_file_dialog(default_name: &str) -> Option<PathBuf> {
    normalize_linux_portal_env();
    rfd::FileDialog::new().set_file_name(default_name).save_file()
}

fn atlas_page_paths(page_index: u32) -> Vec<String> {
    let mut paths = Vec::with_capacity(6);
    for ext in ["bc7", "rgba8888", "bin"] {
        paths.push(format!("pages/{page_index:05}.{ext}"));
    }
    for ext in ["bc7", "rgba8888", "bin"] {
        paths.push(format!("pages/{page_index}.{ext}"));
    }
    paths
}

fn parse_atlas_page_manifest(data: &[u8]) -> HashMap<u32, AtlasPageInfo> {
    if data.len() < 25 {
        return HashMap::new();
    }

    let mut cursor = Cursor::new(data);
    let mut magic = [0u8; 4];
    if cursor.read_exact(&mut magic).is_err() {
        return HashMap::new();
    }

    if !matches!(&magic, b"CAPG" | b"EAPG" | b"ELPG") {
        return HashMap::new();
    }

    let _version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_width = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let atlas_height = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let pixel_format = match cursor.read_u8().ok() {
        Some(0) => AtlasPixelFormat::Rgba8888,
        Some(1) => AtlasPixelFormat::Bc7,
        _ => return HashMap::new(),
    };
    let page_count = cursor.read_u32::<LittleEndian>().unwrap_or(0);

    let mut pages = HashMap::new();
    for _ in 0..page_count {
        let Ok(page_index) = cursor.read_u32::<LittleEndian>() else { break; };
        let Ok(_tile_count) = cursor.read_u32::<LittleEndian>() else { break; };
        let Ok(used_width) = cursor.read_u32::<LittleEndian>() else { break; };
        let Ok(used_height) = cursor.read_u32::<LittleEndian>() else { break; };
        pages.insert(
            page_index,
            AtlasPageInfo {
                atlas_width,
                atlas_height,
                used_width,
                used_height,
                pixel_format,
            },
        );
    }

    pages
}

fn decode_atlas_page(page_info: AtlasPageInfo, data: Vec<u8>) -> Option<DecodedAtlasPage> {
    match page_info.pixel_format {
        AtlasPixelFormat::Bc7 => {
            let extent = uddconv::bc7::ImageExtent::new(page_info.atlas_width, page_info.atlas_height).ok()?;
            let rgba = uddconv::bc7::decode_bc7_to_rgba8888(&data, extent).ok()?;
            Some(DecodedAtlasPage {
                rgba,
                width: page_info.atlas_width,
                height: page_info.atlas_height,
            })
        }
        AtlasPixelFormat::Rgba8888 => {
            let used_len = page_info.used_width as usize * page_info.used_height as usize * 4;
            let atlas_len = page_info.atlas_width as usize * page_info.atlas_height as usize * 4;
            if data.len() == used_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.used_width,
                    height: page_info.used_height,
                })
            } else if data.len() == atlas_len {
                Some(DecodedAtlasPage {
                    rgba: data,
                    width: page_info.atlas_width,
                    height: page_info.atlas_height,
                })
            } else {
                None
            }
        }
    }
}

fn slot_manifest_kind(magic: &[u8; 4]) -> Option<&'static str> {
    match magic {
        b"CASL" => Some("CC Art"),
        b"EASL" => Some("EC Art"),
        b"ELSL" => Some("EC Land"),
        _ => None,
    }
}

fn parse_virtual_entries_from_slot_manifest(data: &[u8]) -> Vec<VirtualEntry> {
    if data.len() < 24 {
        return Vec::new();
    }

    let mut cursor = Cursor::new(data);
    let mut magic = [0u8; 4];
    if cursor.read_exact(&mut magic).is_err() {
        return Vec::new();
    }

    let Some(kind) = slot_manifest_kind(&magic) else {
        return Vec::new();
    };

    let _version = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_w = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _atlas_h = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let _gutter = cursor.read_u32::<LittleEndian>().unwrap_or(0);
    let count = cursor.read_u32::<LittleEndian>().unwrap_or(0);

    let mut entries = Vec::new();
    for _ in 0..count {
        let Ok(id) = cursor.read_u32::<LittleEndian>() else { break; };
        let Ok(page_index) = cursor.read_u32::<LittleEndian>() else { break; };
        let Ok(_page_tile_idx) = cursor.read_u16::<LittleEndian>() else { break; };
        let Ok(flags) = cursor.read_u16::<LittleEndian>() else { break; };
        let Ok(x) = cursor.read_u16::<LittleEndian>() else { break; };
        let Ok(y) = cursor.read_u16::<LittleEndian>() else { break; };
        let Ok(width) = cursor.read_u16::<LittleEndian>() else { break; };
        let Ok(height) = cursor.read_u16::<LittleEndian>() else { break; };

        if (flags & 1) != 0 {
            entries.push(VirtualEntry {
                id,
                kind: kind.to_string(),
                summary: format!("{}x{} at {},{}", width, height, x, y),
                location: format!("page {}", page_index),
                data: VirtualEntryData::AtlasRect {
                    page_index,
                    x,
                    y,
                    width,
                    height,
                    flags,
                },
            });
        }
    }

    entries
}

impl InspectorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let _ = cc; // eframe storage not used; settings come from config.toml
        let cfg = load_config();

        let mut app = Self {
            package: None,
            entries: Vec::new(),
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: cfg.filter,
            package_path: None,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
        };

        if let Some(path) = cfg.package_path {
            if path.exists() {
                app.open_package(path);
            }
        }
        if cfg.view_mode_virtual && !app.virtual_entries.is_empty() {
            app.view_mode = ViewMode::Virtual;
        }

        app
    }

    fn clear_preview_state(&mut self) {
        self.preview_text = None;
        self.preview_texture = None;
        self.preview_texture_size = None;
        self.atlas_texture = None;
        self.atlas_texture_size = None;
        self.atlas_text = None;
        self.preview_mode = PreviewModeKind::Entry;
        self.image_window_mode = PreviewModeKind::Entry;
    }

    fn set_preview_image(
        &mut self,
        ctx: &egui::Context,
        texture_name: &str,
        size: [usize; 2],
        pixels: &[u8],
        label: String,
    ) {
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels);
        self.preview_texture = Some(ctx.load_texture(texture_name, color_image, Default::default()));
        self.preview_texture_size = Some(size);
        self.preview_text = Some(label);
    }

    fn set_atlas_image(
        &mut self,
        ctx: &egui::Context,
        texture_name: &str,
        size: [usize; 2],
        pixels: &[u8],
        label: String,
    ) {
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels);
        self.atlas_texture = Some(ctx.load_texture(texture_name, color_image, Default::default()));
        self.atlas_texture_size = Some(size);
        self.atlas_text = Some(label);
    }

    fn active_preview_image(&self) -> Option<(&egui::TextureHandle, [usize; 2], &str)> {
        match self.preview_mode {
            PreviewModeKind::Entry => self
                .preview_texture
                .as_ref()
                .zip(self.preview_texture_size)
                .map(|(texture, size)| {
                    (texture, size, self.preview_text.as_deref().unwrap_or("Image Preview"))
                }),
            PreviewModeKind::Atlas => self
                .atlas_texture
                .as_ref()
                .zip(self.atlas_texture_size)
                .map(|(texture, size)| {
                    (texture, size, self.atlas_text.as_deref().unwrap_or("Full Atlas"))
                }),
        }
    }

    fn image_for_window(&self) -> Option<(&egui::TextureHandle, [usize; 2], &str)> {
        match self.image_window_mode {
            PreviewModeKind::Entry => self
                .preview_texture
                .as_ref()
                .zip(self.preview_texture_size)
                .map(|(texture, size)| {
                    (texture, size, self.preview_text.as_deref().unwrap_or("Image Preview"))
                }),
            PreviewModeKind::Atlas => self
                .atlas_texture
                .as_ref()
                .zip(self.atlas_texture_size)
                .map(|(texture, size)| {
                    (texture, size, self.atlas_text.as_deref().unwrap_or("Full Atlas"))
                }),
        }
    }

    fn filtered_package_indices(&self) -> Vec<usize> {
        let filter_lower = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if self.filter.is_empty() {
                    return true;
                }
                let key_str = match entry.key {
                    FileKey::Id(id) => id.to_string(),
                    FileKey::PathHash(h) => format!("0x{:016X}", h),
                };
                let type_str = data_type_to_str(entry.data_type).to_lowercase();
                key_str.contains(&self.filter) || type_str.contains(&filter_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn filtered_virtual_indices(&self) -> Vec<usize> {
        let filter_lower = self.filter.to_lowercase();
        self.virtual_entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if self.filter.is_empty() {
                    return true;
                }
                entry.id.to_string().contains(&self.filter)
                    || entry.kind.to_lowercase().contains(&filter_lower)
                    || entry.summary.to_lowercase().contains(&filter_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn handle_keyboard_navigation(&mut self, ctx: &egui::Context) {
        if self.package.is_none() || ctx.wants_keyboard_input() {
            return;
        }

        let move_prev = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowLeft));
        let move_next = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowRight));

        if !move_prev && !move_next {
            return;
        }

        let filtered = if self.view_mode == ViewMode::Package {
            self.filtered_package_indices()
        } else {
            self.filtered_virtual_indices()
        };

        if filtered.is_empty() {
            return;
        }

        let current = if self.view_mode == ViewMode::Package {
            self.selected_idx
        } else {
            self.selected_virtual_idx
        };

        let current_pos = current
            .and_then(|selected| filtered.iter().position(|&idx| idx == selected))
            .unwrap_or(0);

        let target_pos = if move_prev {
            current_pos.saturating_sub(1)
        } else {
            (current_pos + 1).min(filtered.len().saturating_sub(1))
        };
        let target_idx = filtered[target_pos];

        if self.view_mode == ViewMode::Package {
            self.select_entry(ctx, target_idx);
        } else {
            self.select_virtual_entry(ctx, target_idx);
        }
    }

    fn read_file_by_path(&self, reader: &UddpReader, path: &str) -> Option<Vec<u8>> {
        let hash = xxh64_virtual_path(path);
        reader.read_file_by_path_hash(hash).ok()
    }

    fn read_entry_payload(&self, reader: &UddpReader, entry: &EntryInfo) -> Option<Vec<u8>> {
        match entry.key {
            FileKey::Id(id) => match reader.lookup_mode() {
                uocf::udd::uddp::LookupMode::DenseId => reader.read_file_by_dense_id(id),
                uocf::udd::uddp::LookupMode::SparseId => reader.read_file_by_sparse_id(id),
                _ => return None,
            }
            .ok(),
            FileKey::PathHash(h) => reader.read_file_by_path_hash(h).ok(),
        }
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
                self.atlas_pages.clear();
                self.detect_virtual_entries(&reader);

                self.package = Some(reader);
                self.package_path = Some(path);
                self.selected_idx = None;
                self.selected_virtual_idx = None;
                self.clear_preview_state();
                self.view_mode = ViewMode::Package;
            }
            Err(e) => {
                println!("Error opening package: {}", e);
            }
        }
    }

    fn detect_virtual_entries(&mut self, reader: &UddpReader) {
        self.virtual_entries.clear();
        self.atlas_pages.clear();

        if let Some(data) = self.read_file_by_path(reader, "metadata/pages.bin") {
            self.atlas_pages = parse_atlas_page_manifest(&data);
        }

        if let Some(data) = self.read_file_by_path(reader, "metadata/slots.bin") {
            let atlas_entries = parse_virtual_entries_from_slot_manifest(&data);
            if !atlas_entries.is_empty() {
                self.virtual_entries = atlas_entries;
                return;
            }
        }

        let tilemeta_entries = self.detect_tilemeta_virtual_entries(reader);
        if !tilemeta_entries.is_empty() {
            self.virtual_entries = tilemeta_entries;
            return;
        }

        self.virtual_entries = self.detect_block_virtual_entries();
    }

    fn detect_tilemeta_virtual_entries(&self, reader: &UddpReader) -> Vec<VirtualEntry> {
        let mut entries = Vec::new();

        if let Some(data) = self.read_file_by_path(reader, TILEMETA_LAND_ENTRY_PATH) {
            if let Some(land_tiles) = read_pod_records::<TileMetaLandTile>(&data) {
                entries.extend(land_tiles.into_iter().map(|tile| {
                    let name = tile.name_ascii().to_string();
                    VirtualEntry {
                        id: tile.tile_id,
                        kind: "TileMeta Land".to_string(),
                        summary: if name.is_empty() {
                            format!("texture {} type {}", tile.texture_id, tile.tile_type)
                        } else {
                            format!("{} | texture {}", name, tile.texture_id)
                        },
                        location: TILEMETA_LAND_ENTRY_PATH.to_string(),
                        data: VirtualEntryData::TileMetaLand(TileMetaLandInfo {
                            texture_id: tile.texture_id,
                            tile_type: tile.tile_type,
                            flags: tile.flags,
                            radar_color: tile.radar_color,
                            name,
                        }),
                    }
                }));
            }
        }

        if let Some(data) = self.read_file_by_path(reader, TILEMETA_ITEM_ENTRY_PATH) {
            if let Some(item_tiles) = read_pod_records::<TileMetaItemTile>(&data) {
                entries.extend(item_tiles.into_iter().map(|tile| {
                    let name = tile.name_ascii().to_string();
                    VirtualEntry {
                        id: tile.tile_id,
                        kind: "TileMeta Item".to_string(),
                        summary: if name.is_empty() {
                            format!("ec {} | cc {}", tile.ec_texture_id, tile.cc_texture_id)
                        } else {
                            format!("{} | ec {} | cc {}", name, tile.ec_texture_id, tile.cc_texture_id)
                        },
                        location: TILEMETA_ITEM_ENTRY_PATH.to_string(),
                        data: VirtualEntryData::TileMetaItem(TileMetaItemInfo {
                            weight: tile.weight,
                            quality: tile.quality,
                            quantity: tile.quantity,
                            hue_extra: tile.hue_extra,
                            flags: tile.flags,
                            anim_id: tile.anim_id,
                            stacking_offset: tile.stacking_offset,
                            value: tile.value,
                            height: tile.height,
                            radar_color: tile.radar_color,
                            name,
                            ec_texture_id: tile.ec_texture_id,
                            ec_start_x: tile.ec_start_x,
                            ec_start_y: tile.ec_start_y,
                            ec_offset_x: tile.ec_offset_x,
                            ec_offset_y: tile.ec_offset_y,
                            cc_texture_id: tile.cc_texture_id,
                            cc_start_x: tile.cc_start_x,
                            cc_start_y: tile.cc_start_y,
                            cc_offset_x: tile.cc_offset_x,
                            cc_offset_y: tile.cc_offset_y,
                        }),
                    }
                }));
            }
        }

        entries
    }

    fn detect_block_virtual_entries(&self) -> Vec<VirtualEntry> {
        let mut entries = Vec::new();

        for (idx, entry) in self.entries.iter().enumerate() {
            match entry.data_type {
                DATA_TYPE_MAP => {
                    let id = match entry.key {
                        FileKey::Id(id) => id,
                        FileKey::PathHash(_) => continue,
                    };
                    entries.push(VirtualEntry {
                        id,
                        kind: "Map Block".to_string(),
                        summary: format!("64 terrain cells | {} raw", format_size(entry.raw_size as u64)),
                        location: format!("entry {}", id),
                        data: VirtualEntryData::MapBlock { source_entry_idx: idx },
                    });
                }
                DATA_TYPE_STATIC => {
                    let id = match entry.key {
                        FileKey::Id(id) => id,
                        FileKey::PathHash(_) => continue,
                    };
                    entries.push(VirtualEntry {
                        id,
                        kind: "Static Block".to_string(),
                        summary: format!("{} raw", format_size(entry.raw_size as u64)),
                        location: format!("entry {}", id),
                        data: VirtualEntryData::StaticBlock { source_entry_idx: idx },
                    });
                }
                _ => {}
            }
        }

        entries
    }

    fn load_preview(&mut self, ctx: &egui::Context) {
        if let Some(reader) = &self.package {
            if self.view_mode == ViewMode::Package {
                if let Some(idx) = self.selected_idx {
                    let entry = &self.entries[idx];
                    let data = self.read_entry_payload(reader, entry);

                    if let Some(data) = data {
                        self.decode_package_data(ctx, data);
                    }
                }
            } else {
                if let Some(idx) = self.selected_virtual_idx {
                    let ventry = self.virtual_entries[idx].clone();
                    match &ventry.data {
                        VirtualEntryData::AtlasRect {
                            page_index,
                            x,
                            y,
                            width,
                            height,
                            ..
                        } => {
                            let mut page_data: Option<Vec<u8>> = None;
                            for path in atlas_page_paths(*page_index) {
                                if let Some(data) = self.read_file_by_path(reader, &path) {
                                    page_data = Some(data);
                                    break;
                                }
                            }

                            if let Some(data) = page_data {
                                let decoded_page = if let Some(page_info) = self.atlas_pages.get(page_index).copied() {
                                    decode_atlas_page(page_info, data.clone())
                                } else if data.starts_with(b"UDT1") {
                                    if let Ok(vram) = uddconv::bc7::VramTextureData::from_container_bytes(&data) {
                                        uddconv::bc7::decode_from_vram(&vram, uddconv::bc7::RawImageFormat::Rgba8888)
                                            .ok()
                                            .map(|rgba| DecodedAtlasPage {
                                                rgba,
                                                width: vram.extent().width(),
                                                height: vram.extent().height(),
                                            })
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
                                            uddconv::bc7::decode_bc7_to_rgba8888(&blocks, uddconv::bc7::ImageExtent::new(header.pixel_width, header.pixel_height).unwrap())
                                                .ok()
                                                .map(|rgba| DecodedAtlasPage {
                                                    rgba,
                                                    width: header.pixel_width,
                                                    height: header.pixel_height,
                                                })
                                        } else { None }
                                    } else { None }
                                } else if let Ok(image) = image::load_from_memory(&data) {
                                    Some(DecodedAtlasPage {
                                        width: image.width(),
                                        height: image.height(),
                                        rgba: image.to_rgba8().into_raw(),
                                    })
                                } else if data.len() == 2048 * 2048 * 4 {
                                    Some(DecodedAtlasPage {
                                        rgba: data,
                                        width: 2048,
                                        height: 2048,
                                    })
                                } else {
                                    None
                                };

                                if let Some(decoded_page) = decoded_page {
                                    let page_w = decoded_page.width as usize;
                                    let page_h = decoded_page.height as usize;
                                    let crop_bottom = *y as usize + *height as usize;
                                    let crop_right = *x as usize + *width as usize;

                                    if crop_right > page_w || crop_bottom > page_h {
                                        self.preview_texture = None;
                                        self.preview_text = Some(format!(
                                            "Atlas rect {},{} {}x{} exceeds decoded page {}x{} for {} ID {}.",
                                            x,
                                            y,
                                            width,
                                            height,
                                            decoded_page.width,
                                            decoded_page.height,
                                            ventry.kind,
                                            ventry.id
                                        ));
                                        return;
                                    }

                                    self.set_atlas_image(
                                        ctx,
                                        "atlas_preview_full",
                                        [decoded_page.width as usize, decoded_page.height as usize],
                                        &decoded_page.rgba,
                                        format!(
                                            "Atlas page {} for {} ID {} ({}x{})",
                                            page_index,
                                            ventry.kind,
                                            ventry.id,
                                            decoded_page.width,
                                            decoded_page.height
                                        ),
                                    );

                                    let mut cropped = Vec::with_capacity(*width as usize * *height as usize * 4);
                                    for py in 0..*height {
                                        let start = ((*y + py) as usize * page_w + *x as usize) * 4;
                                        let end = start + *width as usize * 4;
                                        if end <= decoded_page.rgba.len() {
                                            cropped.extend_from_slice(&decoded_page.rgba[start..end]);
                                        }
                                    }

                                    self.set_preview_image(
                                        ctx,
                                        "cropped_preview",
                                        [*width as usize, *height as usize],
                                        &cropped,
                                        format!("{} ID: {} ({}x{})", ventry.kind, ventry.id, width, height),
                                    );
                                } else {
                                    self.preview_texture = None;
                                    self.preview_texture_size = None;
                                    self.atlas_texture = None;
                                    self.atlas_texture_size = None;
                                    self.atlas_text = None;
                                    self.preview_text = Some(format!(
                                        "Failed to decode atlas page {} for {} ID {}.",
                                        page_index, ventry.kind, ventry.id
                                    ));
                                }
                            } else {
                                self.preview_texture = None;
                                self.preview_texture_size = None;
                                self.atlas_texture = None;
                                self.atlas_texture_size = None;
                                self.atlas_text = None;
                                self.preview_text = Some(format!(
                                    "Atlas page {} not found for {} ID {}. Tried {}.",
                                    page_index,
                                    ventry.kind,
                                    ventry.id,
                                    atlas_page_paths(*page_index).join(", ")
                                ));
                            }
                        }
                        VirtualEntryData::TileMetaLand(info) => {
                            self.preview_texture = None;
                            self.preview_text = Some(render_tilemeta_land_preview(ventry.id, info));
                        }
                        VirtualEntryData::TileMetaItem(info) => {
                            self.preview_texture = None;
                            self.preview_text = Some(render_tilemeta_item_preview(ventry.id, info));
                        }
                        VirtualEntryData::MapBlock { source_entry_idx } => {
                            self.preview_texture = None;
                            let preview = self
                                .entries
                                .get(*source_entry_idx)
                                .and_then(|entry| self.read_entry_payload(reader, entry))
                                .map(|data| render_map_block_preview(ventry.id, &data))
                                .unwrap_or_else(|| format!("Failed to read map block {}.", ventry.id));
                            self.preview_text = Some(preview);
                        }
                        VirtualEntryData::StaticBlock { source_entry_idx } => {
                            self.preview_texture = None;
                            let preview = self
                                .entries
                                .get(*source_entry_idx)
                                .and_then(|entry| self.read_entry_payload(reader, entry))
                                .map(|data| render_static_block_preview(ventry.id, &data))
                                .unwrap_or_else(|| format!("Failed to read static block {}.", ventry.id));
                            self.preview_text = Some(preview);
                        }
                    }
                }
            }
        }
    }

    fn decode_package_data(&mut self, ctx: &egui::Context, data: Vec<u8>) {
        self.atlas_texture = None;
        self.atlas_texture_size = None;
        self.atlas_text = None;

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
                        self.set_preview_image(ctx, "ktx2_preview", [width as usize, height as usize], &rgba, format!("KTX2 BC7 Texture: {}x{}", width, height));
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
                    self.set_preview_image(
                        ctx,
                        "vram_preview",
                        [extent.width() as usize, extent.height() as usize],
                        &rgba,
                        format!("VRAM Texture: {}x{} ({:?})", extent.width(), extent.height(), vram.format()),
                    );
                    return;
                }
            }
        }

        // 3. Try standard image formats
        if let Ok(image) = image::load_from_memory(&data) {
            let size = [image.width() as usize, image.height() as usize];
            let pixels = image.to_rgba8().into_raw();
            self.set_preview_image(ctx, "img_preview", size, &pixels, format!("Standard Image: {}x{} ({:?})", size[0], size[1], image.color()));
            return;
        }

        // 4. Try to guess raw RGBA8888 pages
        let total_pixels = data.len() / 4;
        if data.len() % 4 == 0 && total_pixels > 0 {
            let side = (total_pixels as f32).sqrt() as u32;
            if side * side == total_pixels as u32 && (side == 2048 || side == 1024 || side == 512 || side == 256 || side == 4096) {
                self.set_preview_image(ctx, "guessed_rgba", [side as usize, side as usize], &data, format!("Guessed Raw RGBA: {}x{}", side, side));
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

                if let Some(path) = save_file_dialog(&default_name) {
                    let _ = std::fs::write(path, data);
                }
            }
        }
    }
}

impl eframe::App for InspectorApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        save_config(&Config {
            package_path: self.package_path.clone(),
            view_mode_virtual: self.view_mode == ViewMode::Virtual,
            filter: self.filter.clone(),
        });
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keyboard_navigation(ctx);

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📁 Open UDDP...").clicked() {
                    if let Some(path) = open_package_dialog() {
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

        if self.image_window_open {
            let mut open = self.image_window_open;
            egui::Window::new("Texture Viewer")
                .open(&mut open)
                .resizable(true)
                .default_size([960.0, 720.0])
                .show(ctx, |ui| {
                    if let Some((texture, _size, label)) = self.image_for_window() {
                        ui.label(label);
                        ui.separator();
                        egui::ScrollArea::both().show(ui, |ui| {
                            ui.image(texture);
                        });
                    } else {
                        ui.label("No image available for the current selection.");
                    }
                });
            self.image_window_open = open;
        }
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

        if self.atlas_texture.is_some() {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.preview_mode, PreviewModeKind::Entry, "Entry View");
                ui.selectable_value(&mut self.preview_mode, PreviewModeKind::Atlas, "Full Atlas");
                if ui.button("Open Image Window").clicked() {
                    self.image_window_mode = self.preview_mode;
                    self.image_window_open = true;
                }
            });
            ui.add_space(5.0);
        } else if self.preview_texture.is_some() {
            ui.horizontal(|ui| {
                if ui.button("Open Image Window").clicked() {
                    self.image_window_mode = PreviewModeKind::Entry;
                    self.image_window_open = true;
                }
            });
            ui.add_space(5.0);
        }

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
                ui.label("Kind:"); ui.label(&ventry.kind); ui.end_row();
                ui.label("Summary:"); ui.label(&ventry.summary); ui.end_row();
                ui.label("Location:"); ui.label(&ventry.location); ui.end_row();
                match &ventry.data {
                    VirtualEntryData::AtlasRect { page_index, x, y, width, height, flags } => {
                        ui.label("Page Index:"); ui.label(page_index.to_string()); ui.end_row();
                        ui.label("Rect:"); ui.label(format!("{},{} - {}x{}", x, y, width, height)); ui.end_row();
                        ui.label("Flags:"); ui.label(format!("0x{:04X}", flags)); ui.end_row();
                    }
                    VirtualEntryData::TileMetaLand(info) => {
                        ui.label("Texture:"); ui.label(info.texture_id.to_string()); ui.end_row();
                        ui.label("Tile Type:"); ui.label(info.tile_type.to_string()); ui.end_row();
                        ui.label("Name:"); ui.label(if info.name.is_empty() { "<unnamed>" } else { &info.name }); ui.end_row();
                    }
                    VirtualEntryData::TileMetaItem(info) => {
                        ui.label("EC Texture:"); ui.label(info.ec_texture_id.to_string()); ui.end_row();
                        ui.label("CC Texture:"); ui.label(info.cc_texture_id.to_string()); ui.end_row();
                        ui.label("Name:"); ui.label(if info.name.is_empty() { "<unnamed>" } else { &info.name }); ui.end_row();
                    }
                    VirtualEntryData::MapBlock { .. } | VirtualEntryData::StaticBlock { .. } => {
                        ui.label("Block ID:"); ui.label(ventry.id.to_string()); ui.end_row();
                    }
                }
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);
        ui.heading("Content Preview");

        if let Some((texture, size, label)) = self.active_preview_image() {
            ui.label(label);
            if size[0] > MAX_INLINE_PREVIEW_WIDTH || size[1] > MAX_INLINE_PREVIEW_HEIGHT {
                ui.label(format!(
                    "Image is {}x{}, which is too large for the side panel. Open it in the separate window.",
                    size[0], size[1]
                ));
                if ui.button("Open Image Window").clicked() {
                    self.image_window_mode = self.preview_mode;
                    self.image_window_open = true;
                }
            } else {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(texture);
                });
            }
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
            let filtered_indices = self.filtered_package_indices();

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
            let filtered_indices = self.filtered_virtual_indices();

            TableBuilder::new(ui).striped(true).resizable(true).cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().at_least(80.0))  // ID
                .column(Column::auto().at_least(120.0)) // Kind
                .column(Column::remainder())            // Summary
                .column(Column::auto().at_least(90.0))  // Location
                .header(20.0, |mut header| {
                    header.col(|ui| { ui.strong("ID"); });
                    header.col(|ui| { ui.strong("Kind"); });
                    header.col(|ui| { ui.strong("Summary"); });
                    header.col(|ui| { ui.strong("Location"); });
                })
                .body(|body| {
                    body.rows(text_height, filtered_indices.len(), |mut row| {
                        let idx = filtered_indices[row.index()];
                        let id_str = self.virtual_entries[idx].id.to_string();
                        row.col(|ui| {
                            if ui.selectable_label(self.selected_virtual_idx == Some(idx), id_str).clicked() { self.select_virtual_entry(ctx, idx); }
                        });
                        row.col(|ui| { ui.label(&self.virtual_entries[idx].kind); });
                        row.col(|ui| { ui.label(&self.virtual_entries[idx].summary); });
                        row.col(|ui| { ui.label(&self.virtual_entries[idx].location); });
                    });
                });
        }
    }

    fn select_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_idx != Some(idx) {
            self.selected_idx = Some(idx);
            self.clear_preview_state();
            self.load_preview(ctx);
        }
    }

    fn select_virtual_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_virtual_idx != Some(idx) {
            self.selected_virtual_idx = Some(idx);
            self.clear_preview_state();
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

fn read_pod_records<T: Pod>(data: &[u8]) -> Option<Vec<T>> {
    let record_size = std::mem::size_of::<T>();
    if record_size == 0 || data.len() % record_size != 0 {
        return None;
    }

    Some(data.chunks_exact(record_size).map(pod_read_unaligned::<T>).collect())
}

fn render_tilemeta_land_preview(id: u32, info: &TileMetaLandInfo) -> String {
    format!(
        "TileMeta Land {}\nname: {}\ntexture_id: {}\ntile_type: {}\nflags: 0x{:016X}\nradar_color: rgba({}, {}, {}, {})",
        id,
        if info.name.is_empty() { "<unnamed>" } else { &info.name },
        info.texture_id,
        info.tile_type,
        info.flags,
        info.radar_color[0],
        info.radar_color[1],
        info.radar_color[2],
        info.radar_color[3]
    )
}

fn render_tilemeta_item_preview(id: u32, info: &TileMetaItemInfo) -> String {
    format!(
        "TileMeta Item {}\nname: {}\nflags: 0x{:016X}\nweight: {}\nquality: {}\nquantity: {}\nhue_extra: {}\nanim_id: {}\nstacking_offset: {}\nvalue: {}\nheight: {}\nradar_color: rgba({}, {}, {}, {})\nec_texture_id: {}\nec_start: {}, {}\nec_offset: {}, {}\ncc_texture_id: {}\ncc_start: {}, {}\ncc_offset: {}, {}",
        id,
        if info.name.is_empty() { "<unnamed>" } else { &info.name },
        info.flags,
        info.weight,
        info.quality,
        info.quantity,
        info.hue_extra,
        info.anim_id,
        info.stacking_offset,
        info.value,
        info.height,
        info.radar_color[0],
        info.radar_color[1],
        info.radar_color[2],
        info.radar_color[3],
        info.ec_texture_id,
        info.ec_start_x,
        info.ec_start_y,
        info.ec_offset_x,
        info.ec_offset_y,
        info.cc_texture_id,
        info.cc_start_x,
        info.cc_start_y,
        info.cc_offset_x,
        info.cc_offset_y,
    )
}

fn render_map_block_preview(block_id: u32, data: &[u8]) -> String {
    let Some(cells) = read_pod_records::<PackedMapTexel>(data) else {
        return format!(
            "Map block {} has invalid size: expected {} bytes, got {}.",
            block_id,
            64 * std::mem::size_of::<PackedMapTexel>(),
            data.len()
        );
    };

    if cells.len() != 64 {
        return format!("Map block {} contains {} cells, expected 64.", block_id, cells.len());
    }

    let mut text = format!("Map Block {}\n", block_id);
    for (index, cell) in cells.iter().enumerate() {
        let x = index % 8;
        let y = index / 8;
        let z = ((cell.packed_meta & 0x00FF) as i16 - 128) as i8;
        let mode = (cell.packed_meta >> 8) as u8;
        text.push_str(&format!("({x},{y}) tile={} z={} mode={}\n", cell.tile_id, z, mode));
    }
    text
}

fn render_static_block_preview(block_id: u32, data: &[u8]) -> String {
    let Some(tiles) = read_pod_records::<uocf::classic::statics::StaticTile>(data) else {
        return format!(
            "Static block {} has invalid size: {} bytes is not a multiple of {}.",
            block_id,
            data.len(),
            std::mem::size_of::<uocf::classic::statics::StaticTile>()
        );
    };

    let mut text = format!("Static Block {}\nentries: {}\n", block_id, tiles.len());
    for (index, tile) in tiles.iter().enumerate() {
        text.push_str(&format!(
            "#{index}: graphic={} offset=({}, {}) z={} hue={}\n",
            tile.graphic,
            tile.x_offset,
            tile.y_offset,
            tile.z,
            tile.hue,
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{
        atlas_page_paths, decode_atlas_page, parse_atlas_page_manifest,
        parse_virtual_entries_from_slot_manifest, read_pod_records, render_map_block_preview,
        slot_manifest_kind, AtlasPageInfo, AtlasPixelFormat, PackedMapTexel, VirtualEntryData,
    };
    use byteorder::{LittleEndian, WriteBytesExt};

    fn build_slot_manifest(magic: &[u8; 4], flags: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(42).unwrap();
        bytes.write_u32::<LittleEndian>(7).unwrap();
        bytes.write_u16::<LittleEndian>(3).unwrap();
        bytes.write_u16::<LittleEndian>(flags).unwrap();
        bytes.write_u16::<LittleEndian>(11).unwrap();
        bytes.write_u16::<LittleEndian>(22).unwrap();
        bytes.write_u16::<LittleEndian>(33).unwrap();
        bytes.write_u16::<LittleEndian>(44).unwrap();
        bytes
    }

    fn build_page_manifest(magic: &[u8; 4], pixel_format: u8, used_width: u32, used_height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(magic);
        bytes.write_u32::<LittleEndian>(2).unwrap();
        bytes.write_u32::<LittleEndian>(4096).unwrap();
        bytes.write_u32::<LittleEndian>(2048).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(pixel_format).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(7).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(used_width).unwrap();
        bytes.write_u32::<LittleEndian>(used_height).unwrap();
        bytes
    }

    #[test]
    fn slot_manifest_kind_recognizes_current_package_types() {
        assert_eq!(slot_manifest_kind(b"CASL"), Some("CC Art"));
        assert_eq!(slot_manifest_kind(b"EASL"), Some("EC Art"));
        assert_eq!(slot_manifest_kind(b"ELSL"), Some("EC Land"));
        assert_eq!(slot_manifest_kind(b"NOPE"), None);
    }

    #[test]
    fn parse_virtual_entries_accepts_ec_land_manifests() {
        let manifest = build_slot_manifest(b"ELSL", 1);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, 42);
        assert_eq!(entry.kind, "EC Land");
        match &entry.data {
            VirtualEntryData::AtlasRect { page_index, x, y, width, height, .. } => {
                assert_eq!(*page_index, 7);
                assert_eq!(*x, 11);
                assert_eq!(*y, 22);
                assert_eq!(*width, 33);
                assert_eq!(*height, 44);
            }
            other => panic!("expected atlas rect, got {other:?}"),
        }
    }

    #[test]
    fn parse_virtual_entries_skips_non_present_slots() {
        let manifest = build_slot_manifest(b"CASL", 0);
        let entries = parse_virtual_entries_from_slot_manifest(&manifest);

        assert!(entries.is_empty());
    }

    #[test]
    fn atlas_page_paths_try_zero_padded_names_first() {
        let paths = atlas_page_paths(7);

        assert_eq!(paths[0], "pages/00007.bc7");
        assert_eq!(paths[1], "pages/00007.rgba8888");
        assert_eq!(paths[2], "pages/00007.bin");
        assert_eq!(paths[3], "pages/7.bc7");
    }

    #[test]
    fn read_pod_records_reads_packed_map_cells() {
        let bytes = bytemuck::bytes_of(&PackedMapTexel { tile_id: 123, packed_meta: 0x8001 });
        let records = read_pod_records::<PackedMapTexel>(bytes).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tile_id, 123);
        assert_eq!(records[0].packed_meta, 0x8001);
    }

    #[test]
    fn render_map_block_preview_decodes_height_bias() {
        let cells = vec![PackedMapTexel { tile_id: 5, packed_meta: 128 }; 64];
        let preview = render_map_block_preview(9, bytemuck::cast_slice(&cells));

        assert!(preview.contains("Map Block 9"));
        assert!(preview.contains("(0,0) tile=5 z=0 mode=0"));
    }

    #[test]
    fn parse_atlas_page_manifest_reads_shared_dimensions_and_format() {
        let manifest = build_page_manifest(b"EAPG", 1, 3000, 1500);
        let pages = parse_atlas_page_manifest(&manifest);
        let info = pages.get(&7).unwrap();

        assert_eq!(info.atlas_width, 4096);
        assert_eq!(info.atlas_height, 2048);
        assert_eq!(info.used_width, 3000);
        assert_eq!(info.used_height, 1500);
        assert_eq!(info.pixel_format, AtlasPixelFormat::Bc7);
    }

    #[test]
    fn decode_atlas_page_uses_manifest_used_size_for_rgba() {
        let info = AtlasPageInfo {
            atlas_width: 4096,
            atlas_height: 2048,
            used_width: 3,
            used_height: 2,
            pixel_format: AtlasPixelFormat::Rgba8888,
        };
        let data = vec![255u8; 3 * 2 * 4];
        let decoded = decode_atlas_page(info, data).unwrap();

        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.rgba.len(), 24);
    }
}
