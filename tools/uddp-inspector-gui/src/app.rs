use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use udd_assets::{AtlasCacheOptions, MobileAnimCcPackage, MobileAnimEcPackage};
use udd_assets::tilemeta::{
    TileMetaItemTile, TileMetaLandTile, TILEMETA_ITEM_ENTRY_PATH, TILEMETA_LAND_ENTRY_PATH,
};
use udd_container::{
    reconstruct_stored_size,
    unpack_codec,
    unpack_offset40,
    unpack_type,
    xxh64_virtual_path, // Codec,
    FileKey,
    UddpReader,
    UddpReaderOptions,
};

use crate::logic::decoding::*;
use crate::logic::discovery::*;
use crate::logic::rendering::*;
use crate::models::*;
use crate::utils::*;

pub const MAX_INLINE_PREVIEW_WIDTH: usize = 800;
pub const MAX_INLINE_PREVIEW_HEIGHT: usize = 600;
pub const DATA_TYPE_MAP: u8 = 3;
pub const DATA_TYPE_STATIC: u8 = 14;
const MISSING_TEXTURE_ID: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAnimTreeOrder {
    BodyType,
    BodyId,
}

fn terrain_definition_path(material_id: u32) -> String {
    format!("build/terraindefinition/{material_id:08}.bin")
}

pub struct InspectorApp {
    pub package: Option<UddpReader>,
    pub entries: Vec<EntryInfo>,
    pub atlas_pages: HashMap<u32, AtlasPageInfo>,
    pub selected_idx: Option<usize>,
    pub filter: String,
    pub hide_empty_entries: bool,
    pub package_path: Option<PathBuf>,
    pub mobile_anim_cc_package: Option<Arc<MobileAnimCcPackage>>,
    pub mobile_anim_ec_package: Option<Arc<MobileAnimEcPackage>>,
    pub selected_mobile_anim_index: usize,
    pub selected_mobile_anim_frame_index: usize,
    pub mobile_anim_is_playing: bool,
    pub mobile_anim_last_frame_time: f64,
    pub mobile_anim_playback_speed: f32,
    pub mobile_anim_loop: bool,
    pub mobile_anim_frame_reset_pending: bool,
    pub mobile_anim_tree_order: MobileAnimTreeOrder,
    pub mobile_anim_tree_collapse_revision: u64,

    // Virtual View state
    pub view_mode: ViewMode,
    pub virtual_entries: Vec<VirtualEntry>,
    pub virtual_material_entries: Vec<VirtualEntry>,
    pub virtual_entry_mode: VirtualEntryMode,
    pub selected_virtual_idx: Option<usize>,

    // Preview state
    pub preview_text: Option<String>,
    pub preview_texture: Option<egui::TextureHandle>,
    pub preview_texture_size: Option<[usize; 2]>,
    pub atlas_texture: Option<egui::TextureHandle>,
    pub atlas_texture_size: Option<[usize; 2]>,
    pub atlas_text: Option<String>,
    pub decoded_atlas_page_index: Option<u32>,
    pub decoded_atlas_page: Option<DecodedAtlasPage>,
    pub preview_mode: PreviewModeKind,
    pub image_window_open: bool,
    pub image_window_mode: PreviewModeKind,
    pub texture_zoom: f32,
    pub focus_filter: bool,
    pub scroll_to_selected: bool,
}

impl InspectorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            package: None,
            entries: Vec::new(),
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: true,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            decoded_atlas_page_index: None,
            decoded_atlas_page: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
            texture_zoom: 1.0,
            focus_filter: false,
            scroll_to_selected: false,
        }
    }

    pub fn clear_preview_state(&mut self) {
        self.preview_text = None;
        self.preview_texture = None;
        self.preview_texture_size = None;
        self.atlas_texture = None;
        self.atlas_texture_size = None;
        self.atlas_text = None;
        self.decoded_atlas_page_index = None;
        self.decoded_atlas_page = None;
        self.preview_mode = PreviewModeKind::Entry;
        self.image_window_mode = PreviewModeKind::Entry;
        self.texture_zoom = 1.0;
    }

    pub fn set_preview_image(
        &mut self,
        ctx: &egui::Context,
        texture_name: &str,
        size: [usize; 2],
        pixels: &[u8],
        label: String,
    ) {
        let expected_len = size[0].saturating_mul(size[1]).saturating_mul(4);
        if size[0] == 0 || size[1] == 0 || pixels.len() != expected_len {
            self.preview_texture = None;
            self.preview_texture_size = None;
            self.preview_text = Some(format!(
                "{label}\nUnable to preview image: invalid RGBA buffer for {}x{} ({} bytes).",
                size[0],
                size[1],
                pixels.len()
            ));
            return;
        }

        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels);
        self.preview_texture =
            Some(ctx.load_texture(texture_name, color_image, Default::default()));
        self.preview_texture_size = Some(size);
        self.preview_text = Some(label);
    }

    /*
    pub fn set_atlas_image(
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
    */

    pub fn active_preview_image(&self) -> Option<(&egui::TextureHandle, [usize; 2], &str)> {
        match self.preview_mode {
            PreviewModeKind::Entry => self
                .preview_texture
                .as_ref()
                .zip(self.preview_texture_size)
                .map(|(texture, size)| {
                    (
                        texture,
                        size,
                        self.preview_text.as_deref().unwrap_or("Image Preview"),
                    )
                }),
            PreviewModeKind::Atlas => self
                .atlas_texture
                .as_ref()
                .zip(self.atlas_texture_size)
                .map(|(texture, size)| {
                    (
                        texture,
                        size,
                        self.atlas_text.as_deref().unwrap_or("Full Atlas"),
                    )
                }),
        }
    }

    fn clear_entry_preview_state(&mut self) {
        self.preview_text = None;
        self.preview_texture = None;
        self.preview_texture_size = None;
        self.preview_mode = PreviewModeKind::Entry;
        self.image_window_mode = PreviewModeKind::Entry;
        self.texture_zoom = 1.0;
    }

    fn virtual_atlas_page_index(&self, idx: usize) -> Option<u32> {
        let entry = self.active_virtual_entries().get(idx)?;
        if let VirtualEntryData::AtlasRect { page_index, .. } = entry.data {
            Some(page_index)
        } else {
            None
        }
    }

    pub fn image_for_window(&self) -> Option<(&egui::TextureHandle, [usize; 2], &str)> {
        match self.image_window_mode {
            PreviewModeKind::Entry => self
                .preview_texture
                .as_ref()
                .zip(self.preview_texture_size)
                .map(|(texture, size)| {
                    (
                        texture,
                        size,
                        self.preview_text.as_deref().unwrap_or("Image Preview"),
                    )
                }),
            PreviewModeKind::Atlas => self
                .atlas_texture
                .as_ref()
                .zip(self.atlas_texture_size)
                .map(|(texture, size)| {
                    (
                        texture,
                        size,
                        self.atlas_text.as_deref().unwrap_or("Full Atlas"),
                    )
                }),
        }
    }

    pub fn filtered_package_indices(&self) -> Vec<usize> {
        let filter_lower = self.filter.to_lowercase();
        let filter_clean = self.filter.trim().trim_start_matches("0x");
        let filter_hex = u64::from_str_radix(filter_clean, 16).ok();
        let filter_dec = self.filter.parse::<u32>().ok();

        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if self.hide_empty_entries && Self::package_entry_is_empty(entry) {
                    return false;
                }

                if self.filter.is_empty() {
                    return true;
                }

                // Match ID exactly
                if let (Some(dec), FileKey::Id(id)) = (filter_dec, entry.key) {
                    if dec == id {
                        return true;
                    }
                }

                // Match Hash exactly
                if let (Some(hex), FileKey::PathHash(h)) = (filter_hex, entry.key) {
                    if hex == h {
                        return true;
                    }
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

    pub fn filtered_virtual_indices(&self) -> Vec<usize> {
        let filter_lower = self.filter.to_lowercase();
        let filter_dec = self.filter.parse::<u32>().ok();

        self.active_virtual_entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                if self.hide_empty_entries && self.virtual_entry_is_empty(entry) {
                    return false;
                }

                if self.filter.is_empty() {
                    return true;
                }

                // Match ID exactly
                if let Some(dec) = filter_dec {
                    if dec == entry.id {
                        return true;
                    }
                }

                entry.id.to_string().contains(&self.filter)
                    || entry.kind.to_lowercase().contains(&filter_lower)
                    || entry.summary.to_lowercase().contains(&filter_lower)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn package_entry_is_empty(entry: &EntryInfo) -> bool {
        entry.raw_size == 0
    }

    fn virtual_entry_is_empty(&self, entry: &VirtualEntry) -> bool {
        match &entry.data {
            VirtualEntryData::AtlasRect {
                page_index,
                width,
                height,
                ..
            } => *page_index == u32::MAX || *width == 0 || *height == 0,
            VirtualEntryData::MapBlock { source_entry_idx }
            | VirtualEntryData::StaticBlock { source_entry_idx } => self
                .entries
                .get(*source_entry_idx)
                .map(Self::package_entry_is_empty)
                .unwrap_or(true),
            _ => false,
        }
    }

    pub fn active_virtual_entries(&self) -> &[VirtualEntry] {
        if self.virtual_entry_mode == VirtualEntryMode::Material
            && !self.virtual_material_entries.is_empty()
        {
            &self.virtual_material_entries
        } else {
            &self.virtual_entries
        }
    }

    pub fn handle_keyboard_navigation(&mut self, ctx: &egui::Context) {
        if self.package.is_none() || ctx.wants_keyboard_input() {
            return;
        }

        let move_prev =
            ctx.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowLeft));
        let move_next = ctx
            .input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowRight));

        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.focus_filter = true;
        }

        let f3_next = ctx.input(|i| i.key_pressed(egui::Key::F3) && !i.modifiers.shift);
        let f3_prev = ctx.input(|i| i.key_pressed(egui::Key::F3) && i.modifiers.shift);

        if !move_prev && !move_next && !f3_next && !f3_prev {
            return;
        }

        let move_prev = move_prev || f3_prev;
        let _move_next = move_next || f3_next;

        let filtered = match self.view_mode {
            ViewMode::Package => self.filtered_package_indices(),
            ViewMode::Virtual => self.filtered_virtual_indices(),
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => self.filtered_mobile_anim_indices(),
        };

        if filtered.is_empty() {
            return;
        }

        let current = match self.view_mode {
            ViewMode::Package => self.selected_idx,
            ViewMode::Virtual => self.selected_virtual_idx,
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => Some(self.selected_mobile_anim_index),
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

        match self.view_mode {
            ViewMode::Package => self.select_entry(ctx, target_idx),
            ViewMode::Virtual => self.select_virtual_entry(ctx, target_idx),
            ViewMode::MobileAnimCc | ViewMode::MobileAnimEc => {
                self.select_mobile_animation_entry(ctx, target_idx);
            }
        }
    }

    pub fn read_file_by_path(&self, reader: &UddpReader, path: &str) -> Option<Vec<u8>> {
        let hash = xxh64_virtual_path(path);
        reader.read_file_by_path_hash(hash).ok()
    }

    pub fn read_entry_payload(&self, reader: &UddpReader, entry: &EntryInfo) -> Option<Vec<u8>> {
        match entry.key {
            FileKey::Id(id) => match reader.lookup_mode() {
                udd_container::LookupMode::DenseId => reader.read_file_by_dense_id(id),
                udd_container::LookupMode::SparseId => reader.read_file_by_sparse_id(id),
                _ => return None,
            }
            .ok(),
            FileKey::PathHash(h) => reader.read_file_by_path_hash(h).ok(),
        }
    }

    pub fn open_package(&mut self, path: PathBuf) {
        let reader_options = UddpReaderOptions::enable_decoded_entry_cache();
        match UddpReader::load_with_options(&path, reader_options) {
            Ok(reader) => {
                let records = reader.records();
                self.entries = records
                    .into_iter()
                    .map(|r| EntryInfo {
                        key: r.key,
                        raw_size: r.locator.raw_size,
                        stored_size: reconstruct_stored_size(
                            r.locator.raw_size,
                            r.locator.meta32,
                            r.locator.pos64,
                        ),
                        data_type: unpack_type(r.locator.meta32),
                        codec: unpack_codec(r.locator.meta32),
                        offset: unpack_offset40(r.locator.pos64),
                    })
                    .collect();

                self.virtual_entries.clear();
                self.virtual_material_entries.clear();
                self.virtual_entry_mode = VirtualEntryMode::Entry;
                self.atlas_pages.clear();
                self.mobile_anim_cc_package = MobileAnimCcPackage::from_uddp_package_with_options(
                    reader.clone(),
                    AtlasCacheOptions::disabled(),
                )
                .ok()
                .map(Arc::new);
                self.mobile_anim_ec_package = MobileAnimEcPackage::from_uddp_package_with_options(
                    reader.clone(),
                    AtlasCacheOptions::disabled(),
                )
                .ok()
                .map(Arc::new);
                self.selected_mobile_anim_index = 0;
                self.selected_mobile_anim_frame_index = 0;
                self.mobile_anim_is_playing = false;
                self.mobile_anim_last_frame_time = 0.0;
                self.mobile_anim_frame_reset_pending = true;
                self.mobile_anim_tree_order = MobileAnimTreeOrder::BodyType;
                self.mobile_anim_tree_collapse_revision = 0;
                self.detect_virtual_entries(&reader);

                self.package = Some(reader);
                self.package_path = Some(path);
                self.selected_idx = None;
                self.selected_virtual_idx = None;
                self.clear_preview_state();
                self.view_mode = if self.mobile_anim_cc_package.is_some() {
                    ViewMode::MobileAnimCc
                } else if self.mobile_anim_ec_package.is_some() {
                    ViewMode::MobileAnimEc
                } else {
                    ViewMode::Package
                };
            }
            Err(e) => {
                println!("Error opening package: {}", e);
            }
        }
    }

    pub fn detect_virtual_entries(&mut self, reader: &UddpReader) {
        self.virtual_entries.clear();
        self.virtual_material_entries.clear();
        self.atlas_pages.clear();

        if let Some(data) = self.read_file_by_path(reader, "metadata/pages.bin") {
            self.atlas_pages = parse_atlas_page_manifest(&data);
        }

        if let Some(data) = self.read_file_by_path(reader, "metadata/slots.bin") {
            let atlas_entries = parse_virtual_entries_from_slot_manifest(&data);
            if !atlas_entries.is_empty() {
                if let Ok(package) = udd_assets::TexLandEcPackage::from_uddp_package(reader.clone()) {
                    self.virtual_material_entries = self.detect_tex_land_ec_material_entries(&package);
                }
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

    pub fn detect_tex_land_ec_material_entries(
        &self,
        package: &udd_assets::TexLandEcPackage,
    ) -> Vec<VirtualEntry> {
        let mut material_ids = package
            .terrain_provenance()
            .iter()
            .map(|record| record.material_id)
            .collect::<Vec<_>>();
        material_ids.sort_unstable();
        material_ids.dedup();

        material_ids
            .into_iter()
            .map(|material_id| {
                let records = package
                    .terrain_provenance()
                    .iter()
                    .filter(|record| record.material_id == material_id)
                    .collect::<Vec<_>>();
                let material_name_id = records
                    .first()
                    .map(|record| record.material_name_id)
                    .unwrap_or(0);
                let alias_count = package
                    .terrain_provenance()
                    .iter()
                    .filter(|record| record.material_id == material_id)
                    .filter_map(|record| (record.alias_slot_id != 0).then_some(record.alias_slot_id))
                    .collect::<HashSet<_>>()
                    .len();
                let preview = self.detect_tex_land_ec_material_preview(
                    package,
                    material_id,
                    records.as_slice(),
                );
                let primary_texture_id = records
                    .iter()
                    .find_map(|record| {
                        (record.primary_texture_id != MISSING_TEXTURE_ID)
                            .then_some(record.primary_texture_id)
                    });
                let primary_texture = primary_texture_id
                    .map(|texture_id| texture_id.to_string())
                    .unwrap_or_else(|| "missing".to_string());
                let selected_texture_count = records
                    .iter()
                    .filter_map(|record| {
                        (record.selected_texture_id != MISSING_TEXTURE_ID)
                            .then_some(record.selected_texture_id)
                    })
                    .collect::<HashSet<_>>()
                    .len();
                let path = terrain_definition_path(material_id);
                let path_hash = uocf::uop_container::hash::hash_file_name_single(&path);
                let summary = format!(
                    "primary {} | aliases {} | textures {}",
                    primary_texture,
                    alias_count,
                    selected_texture_count
                );
                VirtualEntry {
                    id: material_id,
                    _data_type: 11,
                    kind: "EC Land Material".to_string(),
                    summary,
                    location: path.clone(),
                    data: VirtualEntryData::EcLandMaterial(EcLandMaterialInfo {
                        material_id,
                        material_name_id,
                        terrain_definition_path: path,
                        terrain_definition_hash64: path_hash,
                        alias_count,
                        selected_texture_count,
                        primary_texture_id,
                        preview,
                    }),
                }
            })
            .collect()
    }

    fn detect_tex_land_ec_material_preview(
        &self,
        package: &udd_assets::TexLandEcPackage,
        material_id: u32,
        records: &[&udd_assets::tex_land_ec::TexLandEcTerrainProvenanceRecord],
    ) -> Option<EcLandMaterialPreview> {
        let query_tile_id = records
            .iter()
            .filter_map(|record| {
                (record.alias_slot_id != 0
                    && record.alias_slot_id != udd_assets::tex_land_ec::MISSING_SLOT_ID)
                    .then_some(record.alias_slot_id)
            })
            .min()
            .unwrap_or(material_id);
        let slot_id = package.resolve_effective_runtime_slot_id(query_tile_id)?;
        let slot = package.present_slot(slot_id)?;

        Some(EcLandMaterialPreview {
            slot_id,
            page_index: slot.page_index,
            x: slot.x,
            y: slot.y,
            width: slot.width,
            height: slot.height,
        })
    }

    fn material_preview_page_data(
        &self,
        reader: &UddpReader,
        preview: Option<&EcLandMaterialPreview>,
    ) -> Option<Vec<u8>> {
        let preview = preview?;
        for path in atlas_page_paths(preview.page_index) {
            if let Some(data) = self.read_file_by_path(reader, &path) {
                return Some(data);
            }
        }
        None
    }

    fn decode_virtual_atlas_page(
        &self,
        page_index: u32,
        page_bytes: Vec<u8>,
    ) -> Option<DecodedAtlasPage> {
        if let Some(page_info) = self.atlas_pages.get(&page_index).copied() {
            decode_atlas_page(page_info, page_bytes)
        } else if page_bytes.starts_with(b"UDT1") {
            if let Ok(vram) = udd_conv::bc7::VramTextureData::from_container_bytes(&page_bytes) {
                udd_conv::bc7::decode_from_vram(
                    &vram,
                    udd_conv::bc7::RawImageFormat::Rgba8888,
                )
                .ok()
                .map(|rgba| DecodedAtlasPage {
                    rgba,
                    width: vram.extent().width(),
                    height: vram.extent().height(),
                })
            } else {
                None
            }
        } else if page_bytes.starts_with(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB]) {
            if let Ok(k_reader) = ktx2::Reader::new(&page_bytes) {
                let k_header = k_reader.header();
                if k_header.format == Some(ktx2::Format::BC7_UNORM_BLOCK) {
                    let level0 = k_reader.levels().next().unwrap();
                    let mut blocks = level0.data.to_vec();
                    if k_header.supercompression_scheme
                        == Some(ktx2::SupercompressionScheme::Zstandard)
                    {
                        if let Ok(decompressed) = zstd::decode_all(std::io::Cursor::new(&blocks)) {
                            blocks = decompressed;
                        }
                    }
                    udd_conv::bc7::decode_bc7_to_rgba8888(
                        &blocks,
                        udd_conv::bc7::ImageExtent::new(
                            k_header.pixel_width,
                            k_header.pixel_height,
                        )
                        .ok()?,
                    )
                    .ok()
                    .map(|rgba| DecodedAtlasPage {
                        rgba,
                        width: k_header.pixel_width,
                        height: k_header.pixel_height,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn detect_tilemeta_virtual_entries(&self, reader: &UddpReader) -> Vec<VirtualEntry> {
        let mut entries = Vec::new();

        if let Some(data) = self.read_file_by_path(reader, TILEMETA_LAND_ENTRY_PATH) {
            if let Some(land_tiles) = read_pod_records::<TileMetaLandTile>(&data) {
                entries.extend(land_tiles.into_iter().map(|tile| {
                    let name = tile.name_ascii().to_string();
                    VirtualEntry {
                        id: tile.tile_id,
                        _data_type: 11, // Metadata
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
                        _data_type: 11, // Metadata
                        kind: "TileMeta Item".to_string(),
                        summary: if name.is_empty() {
                            format!("ec {} | cc {}", tile.ec_texture_id, tile.cc_texture_id)
                        } else {
                            format!(
                                "{} | ec {} | cc {}",
                                name, tile.ec_texture_id, tile.cc_texture_id
                            )
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
                            ec_offset_x: 0,
                            ec_offset_y: 0,
                            cc_texture_id: tile.cc_texture_id,
                            cc_start_x: tile.cc_start_x,
                            cc_start_y: tile.cc_start_y,
                            cc_offset_x: 0,
                            cc_offset_y: 0,
                        }),
                    }
                }));
            }
        }

        entries
    }

    pub fn detect_block_virtual_entries(&self) -> Vec<VirtualEntry> {
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
                        _data_type: DATA_TYPE_MAP,
                        kind: "Map Block".to_string(),
                        summary: format!(
                            "64 terrain cells | {} raw",
                            format_size(entry.raw_size as u64)
                        ),
                        location: format!("entry {}", id),
                        data: VirtualEntryData::MapBlock {
                            source_entry_idx: idx,
                        },
                    });
                }
                DATA_TYPE_STATIC => {
                    let id = match entry.key {
                        FileKey::Id(id) => id,
                        FileKey::PathHash(_) => continue,
                    };
                    entries.push(VirtualEntry {
                        id,
                        _data_type: DATA_TYPE_STATIC,
                        kind: "Static Block".to_string(),
                        summary: format!("{} raw", format_size(entry.raw_size as u64)),
                        location: format!("entry {}", id),
                        data: VirtualEntryData::StaticBlock {
                            source_entry_idx: idx,
                        },
                    });
                }
                _ => {}
            }
        }

        entries
    }

    pub fn load_preview(&mut self, ctx: &egui::Context) {
        let u_reader = match &self.package {
            Some(r) => r,
            None => return,
        };

        if self.view_mode == ViewMode::Package {
            let result = if let Some(idx) = self.selected_idx {
                let entry = self.entries[idx].clone();
                self.read_entry_payload(u_reader, &entry)
                    .map(|data| (entry, data))
            } else {
                None
            };

            if let Some((entry, data)) = result {
                self.decode_package_data(ctx, &entry, data);
            }
        } else {
            // Virtual View
            let result: Option<(VirtualEntry, Option<Vec<u8>>, Option<Vec<u8>>)> =
                if let Some(idx) = self.selected_virtual_idx {
                    let ventry = self.active_virtual_entries()[idx].clone();
                    match ventry.data.clone() {
                        VirtualEntryData::AtlasRect { page_index, .. } => {
                            if self.decoded_atlas_page_index == Some(page_index)
                                && self.decoded_atlas_page.is_some()
                            {
                                Some((ventry, None, None))
                            } else {
                                let mut page_data = None;
                                for path in atlas_page_paths(page_index) {
                                    if let Some(data) = self.read_file_by_path(u_reader, &path) {
                                        page_data = Some(data);
                                        break;
                                    }
                                }
                                page_data.map(|data| (ventry, Some(data), None))
                            }
                        }
                        VirtualEntryData::EcLandMaterial(info) => {
                            Some((
                                ventry,
                                self.material_preview_page_data(u_reader, info.preview.as_ref()),
                                None,
                            ))
                        }
                        /*
                        VirtualEntryData::DirectPayload { offset, size } => {
                            let mut data = vec![0u8; *size as usize];
                            if u_reader.read_entry(*offset, &mut data).is_ok() {
                                Some((ventry, None, Some(data)))
                            } else {
                                None
                            }
                        }
                        */
                        _ => Some((ventry, None, None)),
                    }
                } else {
                    None
                };

            if let Some(res) = result {
                let (ventry, page_data_opt, _direct_data_opt): (
                    VirtualEntry,
                    Option<Vec<u8>>,
                    Option<Vec<u8>>,
                ) = res;
                match ventry.data {
                    VirtualEntryData::AtlasRect {
                        page_index,
                        x,
                        y,
                        width,
                        height,
                        ..
                    } => {
                        if self.decoded_atlas_page_index != Some(page_index) {
                            self.decoded_atlas_page_index = None;
                            self.decoded_atlas_page = page_data_opt.and_then(|page_bytes| {
                                self.decode_virtual_atlas_page(page_index, page_bytes)
                            });
                            if self.decoded_atlas_page.is_some() {
                                self.decoded_atlas_page_index = Some(page_index);
                            }
                            self.atlas_texture = None;
                            self.atlas_texture_size = None;
                            self.atlas_text = None;
                        }

                        if let Some(page) = self.decoded_atlas_page.as_ref() {
                            let rect = [x as usize, y as usize, width as usize, height as usize];
                            let cropped = crop_rgba(&page.rgba, page.width as usize, rect);
                            let atlas_image = if self.atlas_texture.is_none() {
                                let atlas_size = [page.width as usize, page.height as usize];
                                let expected_len =
                                    atlas_size[0].saturating_mul(atlas_size[1]).saturating_mul(4);
                                if atlas_size[0] != 0
                                    && atlas_size[1] != 0
                                    && page.rgba.len() == expected_len
                                {
                                    Some(egui::ColorImage::from_rgba_unmultiplied(
                                        atlas_size,
                                        &page.rgba,
                                    ))
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            let atlas_size = [page.width as usize, page.height as usize];

                            self.set_preview_image(
                                ctx,
                                "virtual_tile",
                                [width as usize, height as usize],
                                &cropped,
                                format!("Virtual Tile: {} ({}x{})", ventry.id, width, height),
                            );
                            if let Some(atlas_image) = atlas_image {
                                self.atlas_texture = Some(ctx.load_texture(
                                    "full_atlas",
                                    atlas_image,
                                    egui::TextureOptions::default(),
                                ));
                                self.atlas_texture_size = Some(atlas_size);
                                self.atlas_text = Some(format!("Full Atlas Page {}", page_index));
                            }
                        }
                    }
                    VirtualEntryData::EcLandMaterial(info) => {
                        if let (Some(preview), Some(page_bytes)) = (info.preview.as_ref(), page_data_opt) {
                            let decoded_page = self
                                .atlas_pages
                                .get(&preview.page_index)
                                .copied()
                                .and_then(|page_info| decode_atlas_page(page_info, page_bytes));
                            if let Some(page) = decoded_page {
                                let rect = [
                                    preview.x as usize,
                                    preview.y as usize,
                                    preview.width as usize,
                                    preview.height as usize,
                                ];
                                let cropped = crop_rgba(&page.rgba, page.width as usize, rect);
                                self.set_preview_image(
                                    ctx,
                                    "ec_land_material",
                                    [preview.width as usize, preview.height as usize],
                                    &cropped,
                                    format!(
                                        "Material: {} via slot {} ({}x{})",
                                        info.material_id,
                                        preview.slot_id,
                                        preview.width,
                                        preview.height
                                    ),
                                );
                            }
                        } else {
                            self.preview_texture = None;
                            self.preview_text = Some(format!(
                                "Material {} has no resolved preview slot.",
                                info.material_id
                            ));
                        }
                    }
                    /*
                    VirtualEntryData::DirectPayload { .. } => {
                        if let Some(payload_bytes) = direct_data_opt {
                            let payload_bytes: Vec<u8> = payload_bytes;
                            let entry = EntryInfo {
                                key: FileKey::Id(ventry.id),
                                offset: 0, // Not used by decoder
                                raw_size: payload_bytes.len() as u32,
                                stored_size: payload_bytes.len() as u32,
                                codec: Codec::None,
                                _data_type: ventry.data_type,
                            };
                            self.decode_package_data(ctx, &entry, payload_bytes);
                        }
                    }
                    */
                    VirtualEntryData::TileMetaLand(info) => {
                        self.preview_texture = None;
                        self.preview_text = Some(render_tilemeta_land_preview(ventry.id, &info));
                    }
                    VirtualEntryData::TileMetaItem(info) => {
                        self.preview_texture = None;
                        self.preview_text = Some(render_tilemeta_item_preview(ventry.id, &info));
                    }
                    VirtualEntryData::MapBlock { source_entry_idx } => {
                        self.preview_texture = None;
                        let preview = self
                            .entries
                            .get(source_entry_idx)
                            .and_then(|entry| self.read_entry_payload(u_reader, entry))
                            .map(|data| render_map_block_preview(ventry.id, &data))
                            .unwrap_or_else(|| format!("Failed to read map block {}.", ventry.id));
                        self.preview_text = Some(preview);
                    }
                    VirtualEntryData::StaticBlock { source_entry_idx } => {
                        self.preview_texture = None;
                        let preview = self
                            .entries
                            .get(source_entry_idx)
                            .and_then(|entry| self.read_entry_payload(u_reader, entry))
                            .map(|data| render_static_block_preview(ventry.id, &data))
                            .unwrap_or_else(|| {
                                format!("Failed to read static block {}.", ventry.id)
                            });
                        self.preview_text = Some(preview);
                    }
                }
            }
        }
    }

    pub fn decode_package_data(&mut self, ctx: &egui::Context, entry: &EntryInfo, data: Vec<u8>) {
        self.atlas_texture = None;
        self.atlas_texture_size = None;
        self.atlas_text = None;

        // 0. Try UO Art/Texture decoders if it's a known UO data type and not KTX2/UDT1
        let is_ktx2 = data.starts_with(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB]);
        let is_udt1 = data.starts_with(b"UDT1");

        if (entry.data_type == 1 || entry.data_type == 9) && !is_ktx2 && !is_udt1 {
            if entry.data_type == 9 {
                // Land texture: 64x64 or 128x128 BGRA5551
                let size = if data.len() == 64 * 64 * 2 {
                    [64, 64]
                } else if data.len() == 128 * 128 * 2 {
                    [128, 128]
                } else {
                    [0, 0]
                };
                if size[0] > 0 {
                    let mut rgba = vec![0u8; size[0] * size[1] * 4];
                    let pixels_u16: &[u16] = bytemuck::cast_slice(&data);
                    uocf::utils::color::bulk_convert_bgra5551_to_rgba8888(pixels_u16, &mut rgba);
                    self.set_preview_image(
                        ctx,
                        "uo_land_tex",
                        size,
                        &rgba,
                        format!("UO Land Texture: {}x{}", size[0], size[1]),
                    );
                    return;
                }
            } else if entry.data_type == 1 {
                // UO Art
                if let FileKey::Id(id) = entry.key {
                    if id < 0x4000 {
                        // Land diamond
                        let mut rgba = [0u8; 44 * 44 * 4];
                        if uocf::classic::art::decode_land_tile_from_raw(&data, &mut rgba).is_ok() {
                            self.set_preview_image(
                                ctx,
                                "uo_art_land",
                                [44, 44],
                                &rgba,
                                format!("UO Land Art ID {}", id),
                            );
                            return;
                        }
                    } else if let Ok((w, h, rgba)) =
                        uocf::classic::art::decode_static_tile_from_raw(&data)
                    {
                        self.set_preview_image(
                            ctx,
                            "uo_art_static",
                            [w as usize, h as usize],
                            &rgba,
                            format!("UO Static Art ID {}", id),
                        );
                        return;
                    }
                }
            }
        }

        // 1. Try KTX2
        if data.starts_with(&[0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB]) {
            if let Ok(k_reader) = ktx2::Reader::new(&data) {
                let header = k_reader.header();
                let width = header.pixel_width;
                let height = header.pixel_height;
                if header.format == Some(ktx2::Format::BC7_UNORM_BLOCK) {
                    let level0 = k_reader.levels().next().unwrap();
                    let mut blocks = level0.data.to_vec();
                    if header.supercompression_scheme
                        == Some(ktx2::SupercompressionScheme::Zstandard)
                    {
                        if let Ok(decompressed) = zstd::decode_all(std::io::Cursor::new(&blocks)) {
                            blocks = decompressed;
                        }
                    }
                    if let Ok(rgba) = udd_conv::bc7::decode_bc7_to_rgba8888(
                        &blocks,
                        udd_conv::bc7::ImageExtent::new(width, height).unwrap(),
                    ) {
                        self.set_preview_image(
                            ctx,
                            "ktx2_preview",
                            [width as usize, height as usize],
                            &rgba,
                            format!("KTX2 BC7 Texture: {}x{}", width, height),
                        );
                        return;
                    }
                }
            }
        }

        // 2. Try VRAM Container (UDT1)
        if data.starts_with(b"UDT1") {
            if let Ok(vram) = udd_conv::bc7::VramTextureData::from_container_bytes(&data) {
                let extent = vram.extent();
                if let Ok(rgba) =
                    udd_conv::bc7::decode_from_vram(&vram, udd_conv::bc7::RawImageFormat::Rgba8888)
                {
                    self.set_preview_image(
                        ctx,
                        "vram_preview",
                        [extent.width() as usize, extent.height() as usize],
                        &rgba,
                        format!(
                            "VRAM Texture: {}x{} ({:?})",
                            extent.width(),
                            extent.height(),
                            vram.format()
                        ),
                    );
                    return;
                }
            }
        }

        // 3. Try standard image formats
        if let Ok(image) = image::load_from_memory(&data) {
            let size = [image.width() as usize, image.height() as usize];
            let pixels = image.to_rgba8().into_raw();
            self.set_preview_image(
                ctx,
                "img_preview",
                size,
                &pixels,
                format!(
                    "Standard Image: {}x{} ({:?})",
                    size[0],
                    size[1],
                    image.color()
                ),
            );
            return;
        }

        // 4. Try to guess raw RGBA8888 pages
        let total_pixels = data.len() / 4;
        if data.len() % 4 == 0 && total_pixels > 0 {
            let side = (total_pixels as f32).sqrt() as u32;
            if side * side == total_pixels as u32
                && (side == 2048 || side == 1024 || side == 512 || side == 256 || side == 4096)
            {
                self.set_preview_image(
                    ctx,
                    "guessed_rgba",
                    [side as usize, side as usize],
                    &data,
                    format!("Guessed Raw RGBA: {}x{}", side, side),
                );
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
                if i > 0 && i % 16 == 0 {
                    hex.push('\n');
                }
                hex.push_str(&format!("{:02X} ", byte));
            }
            if data.len() > 1024 {
                hex.push_str("\n...");
            }
            self.preview_text = Some(hex);
        }
    }

    pub fn extract_payload(&self, idx: usize) {
        if let Some(reader) = &self.package {
            let entry = &self.entries[idx];
            let result = match entry.key {
                FileKey::Id(id) => match reader.lookup_mode() {
                    udd_container::LookupMode::DenseId => reader.read_file_by_dense_id(id),
                    udd_container::LookupMode::SparseId => reader.read_file_by_sparse_id(id),
                    _ => unreachable!(),
                },
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

    pub fn select_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_idx != Some(idx) {
            self.selected_idx = Some(idx);
            self.scroll_to_selected = true;
            self.clear_preview_state();
            self.load_preview(ctx);
        }
    }

    pub fn select_virtual_entry(&mut self, ctx: &egui::Context, idx: usize) {
        if self.selected_virtual_idx != Some(idx) {
            let previous_page = self.selected_virtual_idx.and_then(|idx| {
                self.virtual_atlas_page_index(idx)
            });
            let next_page = self.virtual_atlas_page_index(idx);
            self.selected_virtual_idx = Some(idx);
            self.scroll_to_selected = true;
            if previous_page.is_some() && previous_page == next_page {
                self.clear_entry_preview_state();
            } else {
                self.clear_preview_state();
            }
            self.load_preview(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};
    use udd_container::{
        AddFileRequest,
        CompressionFlag,
        DataType,
        FileKey,
        LookupMode,
        UddpBuilder,
    };

    fn empty_test_app(view_mode: ViewMode) -> InspectorApp {
        InspectorApp {
            package: None,
            entries: Vec::new(),
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: false,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode,
            virtual_entries: Vec::new(),
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            decoded_atlas_page_index: None,
            decoded_atlas_page: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
            texture_zoom: 1.0,
            focus_filter: false,
            scroll_to_selected: false,
        }
    }

    fn add_metadata_file(builder: &mut UddpBuilder, virtual_path: &str, data: &[u8]) {
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(virtual_path),
                path_hash64: None,
                id: None,
                data,
            })
            .expect("add metadata file");
    }

    fn ec_land_page_manifest_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ELPG");
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(0).unwrap();
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes
    }

    fn ec_land_slot_manifest_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ELSL");
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(64).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u8(0).unwrap();
        bytes.write_u32::<LittleEndian>(16_409).unwrap();
        for art_id in 0..=16_408u32 {
            bytes.write_u32::<LittleEndian>(art_id).unwrap();
            if art_id == 77 || art_id == 100 || art_id == 16_408 {
                bytes.write_u32::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes
                    .write_u16::<LittleEndian>(
                        udd_assets::tex_land_ec::SLOT_FLAG_PRESENT
                            | udd_assets::tex_land_ec::SLOT_FLAG_LAND,
                    )
                    .unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(44).unwrap();
                bytes.write_u16::<LittleEndian>(44).unwrap();
            } else {
                bytes
                    .write_u32::<LittleEndian>(udd_assets::tex_land_ec::MISSING_PAGE_INDEX)
                    .unwrap();
                bytes
                    .write_u16::<LittleEndian>(udd_assets::tex_land_ec::MISSING_PAGE_TILE_INDEX)
                    .unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
                bytes.write_u16::<LittleEndian>(0).unwrap();
            }
        }
        bytes
    }

    fn ec_land_terrain_provenance_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"ELTP");
        bytes.write_u32::<LittleEndian>(3).unwrap();
        bytes.write_u32::<LittleEndian>(2).unwrap();
        for (texture_id, canonical_slot_id, layer_index) in [
            (1_000_003, 16_408, udd_assets::tex_land_ec::MISSING_TERRAIN_LAYER_INDEX),
            (2_000_520, 100, 0),
        ] {
            bytes.write_u32::<LittleEndian>(52).unwrap();
            bytes.write_i32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(77).unwrap();
            bytes.write_u64::<LittleEndian>(0).unwrap();
            bytes.write_u32::<LittleEndian>(texture_id).unwrap();
            bytes.write_u32::<LittleEndian>(canonical_slot_id).unwrap();
            bytes.write_u32::<LittleEndian>(layer_index).unwrap();
            bytes.write_f32::<LittleEndian>(5.0).unwrap();
            bytes.write_u32::<LittleEndian>(2_000_520).unwrap();
            bytes.write_u32::<LittleEndian>(0).unwrap();
            bytes
                .write_u8(udd_assets::tex_land_ec::TERRAIN_PRIMARY_REASON_NON_SUPPORT_PREFERRED_REPETITION)
                .unwrap();
            bytes
                .write_u16::<LittleEndian>(
                    udd_assets::tex_land_ec::TERRAIN_PRIMARY_FLAG_SELECTED_PREFERRED_REPETITION,
                )
                .unwrap();
        }
        bytes
    }

    fn ec_land_test_package() -> udd_assets::TexLandEcPackage {
        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        add_metadata_file(
            &mut builder,
            udd_assets::tex_land_ec::UDDP_PAGE_MANIFEST_ENTRY_VPATH,
            &ec_land_page_manifest_bytes(),
        );
        add_metadata_file(
            &mut builder,
            udd_assets::tex_land_ec::UDDP_SLOT_MANIFEST_ENTRY_VPATH,
            &ec_land_slot_manifest_bytes(),
        );
        add_metadata_file(
            &mut builder,
            udd_assets::tex_land_ec::UDDP_TERRAIN_PROVENANCE_ENTRY_VPATH,
            &ec_land_terrain_provenance_bytes(),
        );
        let bytes = builder.build().expect("build test package");
        udd_assets::TexLandEcPackage::from_uddp_package(
            UddpReader::open(bytes).expect("open test package"),
        )
        .expect("load EC land package")
    }

    #[test]
    fn test_clear_preview_state() {
        let mut app = InspectorApp {
            package: None,
            entries: Vec::new(),
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: false,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: Some("hello".to_string()),
            preview_texture: None,
            preview_texture_size: Some([10, 10]),
            atlas_texture: None,
            atlas_texture_size: Some([20, 20]),
            atlas_text: Some("world".to_string()),
            decoded_atlas_page_index: Some(7),
            decoded_atlas_page: Some(DecodedAtlasPage {
                rgba: vec![255, 0, 0, 255],
                width: 1,
                height: 1,
            }),
            preview_mode: PreviewModeKind::Atlas,
            image_window_open: true,
            image_window_mode: PreviewModeKind::Atlas,
            texture_zoom: 2.5,
            focus_filter: false,
            scroll_to_selected: false,
        };

        app.clear_preview_state();

        assert_eq!(app.preview_text, None);
        assert_eq!(app.preview_texture_size, None);
        assert_eq!(app.atlas_texture_size, None);
        assert_eq!(app.atlas_text, None);
        assert_eq!(app.decoded_atlas_page_index, None);
        assert!(app.decoded_atlas_page.is_none());
        assert_eq!(app.preview_mode, PreviewModeKind::Entry);
        assert_eq!(app.image_window_mode, PreviewModeKind::Entry);
        assert_eq!(app.texture_zoom, 1.0);
    }

    #[test]
    fn test_filtered_package_indices() {
        let mut app = InspectorApp {
            package: None,
            entries: vec![
                EntryInfo {
                    key: FileKey::Id(42),
                    raw_size: 100,
                    stored_size: 100,
                    data_type: 1, // Art
                    codec: udd_container::Codec::None,
                    offset: 0,
                },
                EntryInfo {
                    key: FileKey::PathHash(0xABCDEF1234567890),
                    raw_size: 200,
                    stored_size: 200,
                    data_type: 9, // Texture (maps to "Texture")
                    codec: udd_container::Codec::None,
                    offset: 0,
                },
                EntryInfo {
                    key: FileKey::Id(999),
                    raw_size: 300,
                    stored_size: 300,
                    data_type: 3, // Map (maps to "Map")
                    codec: udd_container::Codec::None,
                    offset: 0,
                },
                EntryInfo {
                    key: FileKey::Id(5555),
                    raw_size: 0,
                    stored_size: 0,
                    data_type: 1,
                    codec: udd_container::Codec::None,
                    offset: 0,
                },
            ],
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: false,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            decoded_atlas_page_index: None,
            decoded_atlas_page: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
            texture_zoom: 1.0,
            focus_filter: false,
            scroll_to_selected: false,
        };

        // Empty filter matches all
        assert_eq!(app.filtered_package_indices(), vec![0, 1, 2]);

        app.hide_empty_entries = false;
        assert_eq!(app.filtered_package_indices(), vec![0, 1, 2, 3]);
        app.hide_empty_entries = true;

        // Filter by Decimal ID
        app.filter = "42".to_string();
        assert_eq!(app.filtered_package_indices(), vec![0]);

        // Filter by Hex Hash
        app.filter = "0xABCDEF1234567890".to_string();
        assert_eq!(app.filtered_package_indices(), vec![1]);

        // Filter by Hex Hash without 0x prefix
        app.filter = "ABCDEF1234567890".to_string();
        assert_eq!(app.filtered_package_indices(), vec![1]);

        // Filter by data type name substring
        app.filter = "text".to_string();
        assert_eq!(app.filtered_package_indices(), vec![1]);

        app.filter = "art".to_string();
        assert_eq!(app.filtered_package_indices(), vec![0]);

        app.filter = "5555".to_string();
        assert_eq!(app.filtered_package_indices(), Vec::<usize>::new());
        app.hide_empty_entries = false;
        assert_eq!(app.filtered_package_indices(), vec![3]);
    }

    #[test]
    fn test_filtered_virtual_indices() {
        let mut app = InspectorApp {
            package: None,
            entries: Vec::new(),
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: false,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode: ViewMode::Virtual,
            virtual_entries: vec![
                VirtualEntry {
                    id: 100,
                    _data_type: 1,
                    kind: "CC Art".to_string(),
                    summary: "16x16 at 0,0".to_string(),
                    location: "page 0".to_string(),
                    data: VirtualEntryData::AtlasRect {
                        page_index: 0,
                        x: 0,
                        y: 0,
                        width: 16,
                        height: 16,
                        flags: 1,
                    },
                },
                VirtualEntry {
                    id: 200,
                    _data_type: 11,
                    kind: "TileMeta Land".to_string(),
                    summary: "Grass Tile".to_string(),
                    location: "metadata/land.bin".to_string(),
                    data: VirtualEntryData::TileMetaLand(TileMetaLandInfo {
                        texture_id: 5,
                        tile_type: 1,
                        flags: 0,
                        radar_color: [0, 255, 0, 255],
                        name: "Grass".to_string(),
                    }),
                },
                VirtualEntry {
                    id: 300,
                    _data_type: 9,
                    kind: "EC Land".to_string(),
                    summary: "empty".to_string(),
                    location: "missing page".to_string(),
                    data: VirtualEntryData::AtlasRect {
                        page_index: u32::MAX,
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                        flags: 1,
                    },
                },
            ],
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            decoded_atlas_page_index: None,
            decoded_atlas_page: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
            texture_zoom: 1.0,
            focus_filter: false,
            scroll_to_selected: false,
        };

        // Empty filter matches all
        assert_eq!(app.filtered_virtual_indices(), vec![0, 1]);

        app.hide_empty_entries = false;
        assert_eq!(app.filtered_virtual_indices(), vec![0, 1, 2]);
        app.hide_empty_entries = true;

        // Filter by Decimal ID
        app.filter = "100".to_string();
        assert_eq!(app.filtered_virtual_indices(), vec![0]);

        // Filter by kind substring
        app.filter = "meta".to_string();
        assert_eq!(app.filtered_virtual_indices(), vec![1]);

        // Filter by summary substring
        app.filter = "grass".to_string();
        assert_eq!(app.filtered_virtual_indices(), vec![1]);

        app.filter = "300".to_string();
        assert_eq!(app.filtered_virtual_indices(), Vec::<usize>::new());
        app.hide_empty_entries = false;
        assert_eq!(app.filtered_virtual_indices(), vec![2]);
    }

    #[test]
    fn terrain_definition_path_uses_material_id_and_hashes_as_uop_path() {
        let path = terrain_definition_path(20_000_061);

        assert_eq!(path, "build/terraindefinition/20000061.bin");
        assert_eq!(
            uocf::uop_container::hash::hash_file_name_single(&path),
            uocf::uop_container::hash::hash_file_name_single(
                "build/terraindefinition/20000061.bin"
            )
        );
    }

    #[test]
    fn material_virtual_entries_preview_resolved_primary_texture_slot() {
        let app = empty_test_app(ViewMode::Virtual);
        let package = ec_land_test_package();

        let entries = app.detect_tex_land_ec_material_entries(&package);
        let material_entry = entries
            .iter()
            .find(|entry| entry.id == 52)
            .expect("material 52 entry");
        let VirtualEntryData::EcLandMaterial(info) = &material_entry.data else {
            panic!("expected material entry");
        };
        let preview = info.preview.as_ref().expect("material preview");

        assert_eq!(preview.slot_id, 100);
        assert_eq!(info.primary_texture_id, Some(2_000_520));
    }

    #[test]
    fn test_detect_block_virtual_entries() {
        let app = InspectorApp {
            package: None,
            entries: vec![
                EntryInfo {
                    key: FileKey::Id(5),
                    raw_size: 1024,
                    stored_size: 1024,
                    data_type: DATA_TYPE_MAP,
                    codec: udd_container::Codec::None,
                    offset: 100,
                },
                EntryInfo {
                    key: FileKey::Id(10),
                    raw_size: 512,
                    stored_size: 512,
                    data_type: DATA_TYPE_STATIC,
                    codec: udd_container::Codec::None,
                    offset: 200,
                },
                EntryInfo {
                    key: FileKey::PathHash(1234), // Should be skipped (needs Id)
                    raw_size: 512,
                    stored_size: 512,
                    data_type: DATA_TYPE_STATIC,
                    codec: udd_container::Codec::None,
                    offset: 300,
                },
            ],
            atlas_pages: HashMap::new(),
            selected_idx: None,
            filter: String::new(),
            hide_empty_entries: true,
            package_path: None,
            mobile_anim_cc_package: None,
            mobile_anim_ec_package: None,
            selected_mobile_anim_index: 0,
            selected_mobile_anim_frame_index: 0,
            mobile_anim_is_playing: false,
            mobile_anim_last_frame_time: 0.0,
            mobile_anim_playback_speed: 1.0,
            mobile_anim_loop: true,
            mobile_anim_frame_reset_pending: false,
            mobile_anim_tree_order: MobileAnimTreeOrder::BodyType,
            mobile_anim_tree_collapse_revision: 0,
            view_mode: ViewMode::Package,
            virtual_entries: Vec::new(),
            virtual_material_entries: Vec::new(),
            virtual_entry_mode: VirtualEntryMode::Entry,
            selected_virtual_idx: None,
            preview_text: None,
            preview_texture: None,
            preview_texture_size: None,
            atlas_texture: None,
            atlas_texture_size: None,
            atlas_text: None,
            decoded_atlas_page_index: None,
            decoded_atlas_page: None,
            preview_mode: PreviewModeKind::Entry,
            image_window_open: false,
            image_window_mode: PreviewModeKind::Entry,
            texture_zoom: 1.0,
            focus_filter: false,
            scroll_to_selected: false,
        };

        let virtuals = app.detect_block_virtual_entries();
        assert_eq!(virtuals.len(), 2);

        assert_eq!(virtuals[0].id, 5);
        assert_eq!(virtuals[0].kind, "Map Block");
        if let VirtualEntryData::MapBlock { source_entry_idx } = virtuals[0].data {
            assert_eq!(source_entry_idx, 0);
        } else {
            panic!("Expected MapBlock data");
        }

        assert_eq!(virtuals[1].id, 10);
        assert_eq!(virtuals[1].kind, "Static Block");
        if let VirtualEntryData::StaticBlock { source_entry_idx } = virtuals[1].data {
            assert_eq!(source_entry_idx, 1);
        } else {
            panic!("Expected StaticBlock data");
        }
    }
}
