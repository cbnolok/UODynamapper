use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use udd_container::{Codec, FileKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Package,
    Virtual,
    MobileAnimCc,
    MobileAnimEc,
    /// An `effects_ec.uddp` package.
    Effects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewModeKind {
    Entry,
    Atlas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualEntryMode {
    Entry,
    Material,
}

#[derive(Debug, Clone, Copy)]
pub struct EntryInfo {
    pub key: FileKey,
    pub raw_size: u32,
    pub stored_size: u32,
    pub data_type: u8,
    pub codec: Codec,
    pub offset: u64,
}

#[derive(Debug, Clone)]
pub struct VirtualEntry {
    pub id: u32,
    pub _data_type: u8,
    pub kind: String,
    pub summary: String,
    pub location: String,
    pub data: VirtualEntryData,
}

#[derive(Debug, Clone)]
pub enum VirtualEntryData {
    AtlasRect {
        page_index: u32,
        page_tile_idx: u16,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        flags: u16,
        upscale_factor: u16,
        upscale_algorithm: u16,
    },
    EcLandMaterial(EcLandMaterialInfo),
    TileMetaLand(TileMetaLandInfo),
    TileMetaItem(TileMetaItemInfo),
    Gump(GumpInfo),
    WorldLight(WorldLightInfo),
    MapBlock {
        source_entry_idx: usize,
    },
    StaticBlock {
        source_entry_idx: usize,
    },
    /*
    DirectPayload {
        offset: u64,
        size: u32,
    },
    */
}

#[derive(Debug, Clone)]
pub struct EcLandMaterialInfo {
    pub material_id: u32,
    pub material_name_id: i32,
    pub terrain_definition_path: String,
    pub terrain_definition_hash64: u64,
    pub alias_count: usize,
    pub selected_texture_count: usize,
    pub primary_texture_id: Option<u32>,
    pub selected_textures: Vec<EcLandMaterialTextureInfo>,
    pub override_textures: Vec<EcLandMaterialOverrideTextureInfo>,
    pub preview: Option<EcLandMaterialPreview>,
    /// The material's terrain layer class (from its shader name).
    pub layer_class: Option<uocf::enhanced::terrain_definition::TerrainLayerClass>,
}

#[derive(Debug, Clone)]
pub struct EcLandMaterialTextureInfo {
    pub layer_index: u32,
    pub texture_id: u32,
    pub runtime_slot_id: Option<u32>,
    pub repetition: f32,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct EcLandMaterialOverrideTextureInfo {
    pub role: String,
    pub layer_index: Option<u32>,
    pub texture_id: u32,
    pub runtime_slot_id: Option<u32>,
    pub repetition: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct EcLandMaterialPreview {
    pub slot_id: u32,
    pub page_index: u32,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct TileMetaLandInfo {
    pub texture_id: u16,
    pub tile_type: u8,
    pub flags: u64,
    pub radar_color: [u8; 4],
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TileMetaItemInfo {
    pub weight: u8,
    pub quality: u8,
    pub quantity: u8,
    pub hue_extra: u8,
    pub flags: u64,
    pub anim_id: u16,
    pub stacking_offset: u8,
    pub value: u8,
    pub height: i8,
    pub radar_color: [u8; 4],
    pub name: String,
    pub ec_texture_id: u32,
    pub ec_start_x: i16,
    pub ec_start_y: i16,
    pub ec_offset_x: i16,
    pub ec_offset_y: i16,
    pub cc_texture_id: u32,
    pub cc_start_x: i16,
    pub cc_start_y: i16,
    pub cc_offset_x: i16,
    pub cc_offset_y: i16,
    /// The item's tileart texture refs, in package order.
    pub texture_refs: Vec<TileMetaTextureRefInfo>,
    /// The role-aware main-texture choice.
    pub main_texture: Option<TileMetaMainTextureInfo>,
    /// Image packages found beside the tilemeta, which the refs' image holders were looked up in.
    pub image_packages: Vec<&'static str>,
}

/// One tileart texture ref of a `tilemeta` item.
#[derive(Debug, Clone)]
pub struct TileMetaTextureRefInfo {
    pub block_index: u8,
    pub item_index: u8,
    pub texture_id: u32,
    pub stable_role: &'static str,
    pub physical_package: String,
    pub logical_family: String,
    pub primary_selected: bool,
    pub auxiliary: bool,
    pub stretch: f32,
    /// This ref is the main-texture choice.
    pub is_main: bool,
    /// The package beside the tilemeta that holds the image.
    pub image_holder: String,
}

/// The role-aware main-texture choice of a `tilemeta` item.
#[derive(Debug, Clone)]
pub struct TileMetaMainTextureInfo {
    pub texture_id: u32,
    pub stable_role: &'static str,
    pub reason: &'static str,
    pub low_confidence: bool,
}

#[derive(Debug, Clone)]
pub struct GumpInfo {
    pub source: GumpSourceInfo,
}

#[derive(Debug, Clone)]
pub enum GumpSourceInfo {
    SingleFile {
        source_entry_idx: Option<usize>,
        raw_size: u32,
    },
    AtlasSlot {
        page_index: u32,
        page_gump_index: u16,
        /// The stored texels on the page, in physical texels: the gump trimmed to its drawn texels.
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        upscale_factor: u16,
        /// Where the stored texels start in the gump's canvas, and the canvas, in logical texels.
        offset_x: u16,
        offset_y: u16,
        canvas_width: u16,
        canvas_height: u16,
    },
}

#[derive(Debug, Clone)]
pub struct WorldLightInfo {
    pub width: u16,
    pub height: u16,
    pub flags: u16,
    pub path: String,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct PackedMapTexel {
    pub tile_id: u16,
    pub packed_meta: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtlasPixelFormat {
    Rgba8888,
    Bc7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasPageInfo {
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub used_width: u32,
    pub used_height: u32,
    pub pixel_format: AtlasPixelFormat,
}

#[derive(Debug, Clone)]
pub struct DecodedAtlasPage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
