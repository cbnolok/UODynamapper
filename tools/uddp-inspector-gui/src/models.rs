use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use udd_container::{Codec, FileKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Package,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreviewModeKind {
    Entry,
    Atlas,
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
    /*
    DirectPayload {
        offset: u64,
        size: u32,
    },
    */
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
