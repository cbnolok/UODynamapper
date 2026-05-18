use std::path::Path;
use color_eyre::eyre::{self, WrapErr};
use bytemuck::{Pod, Zeroable};
use udd_container::UddpReader;
use crate::common::{read_path_entry, read_pod_vec};

pub const TILEMETA_LAND_ENTRY_PATH: &str = "metadata/land.bin";
pub const TILEMETA_ITEM_ENTRY_PATH: &str = "metadata/items.bin";
pub const TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH: &str = "metadata/item_texture_refs_index.bin";
pub const TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH: &str = "metadata/item_texture_refs.bin";

pub const TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY: u8 = 1 << 0;
pub const TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED: u8 = 1 << 1;

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaLandTile {
    pub tile_id: u32,
    pub texture_id: u16,
    pub tile_type: u8,
    pub _pad1: u8,
    pub flags: u64,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],
}

impl TileMetaLandTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TileMetaItemTile {
    pub tile_id: u32,
    pub weight: u8,
    pub quality: u8,
    pub quantity: u8,
    pub hue_extra: u8,
    pub flags: u64,
    pub anim_id: u16,
    pub stacking_offset: u8,
    pub value: u8,
    pub height: i8,
    pub _pad1: u8,
    pub _pad2: u16,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum TileMetaItemVisualKind {
    #[default]
    RegularArt = 0,
    SurfaceLike = 1,
}

impl TileMetaItemTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }

    pub fn visual_kind(&self) -> TileMetaItemVisualKind {
        match self._pad1 {
            1 => TileMetaItemVisualKind::SurfaceLike,
            _ => TileMetaItemVisualKind::RegularArt,
        }
    }

    pub fn is_surface_like(&self) -> bool {
        self.visual_kind() == TileMetaItemVisualKind::SurfaceLike
    }

    pub fn set_visual_kind(&mut self, kind: TileMetaItemVisualKind) {
        self._pad1 = kind as u8;
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRefSpan {
    pub start: u32,
    pub len: u16,
    pub _pad: u16,
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Default, Pod, Zeroable)]
pub struct TileMetaItemTextureRef {
    pub texture_id: u32,
    pub texture_type: u8,
    pub block_index: u8,
    pub item_index: u8,
    pub flags: u8,
    pub texture_stretch: f32,
    pub unk4: u8,
    pub _pad0: [u8; 3],
    pub unk6: u32,
    pub unk7: u32,
}

impl TileMetaItemTextureRef {
    pub fn is_auxiliary(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_AUXILIARY != 0
    }

    pub fn is_primary_selected(&self) -> bool {
        self.flags & TILEMETA_ITEM_TEXTURE_FLAG_PRIMARY_SELECTED != 0
    }

    pub fn is_world_art(&self) -> bool {
        self.texture_type == 1
    }
}

pub struct TileMetaPackage {
    #[allow(dead_code)]
    package: UddpReader,
    land_tiles: Vec<TileMetaLandTile>,
    item_tiles: Vec<TileMetaItemTile>,
    item_texture_ref_spans: Vec<TileMetaItemTextureRefSpan>,
    item_texture_refs: Vec<TileMetaItemTextureRef>,
}

impl TileMetaPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load(path.as_ref())
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn load_in_memory(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::load_in_memory(path.as_ref())
            .wrap_err_with(|| format!("load_in_memory {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let land_bytes = read_path_entry(&package, TILEMETA_LAND_ENTRY_PATH)?;
        let item_bytes = read_path_entry(&package, TILEMETA_ITEM_ENTRY_PATH)?;
        let item_texture_ref_spans = read_optional_pod_vec(
            &package,
            TILEMETA_ITEM_TEXTURE_REF_INDEX_ENTRY_PATH,
        )?;
        let item_texture_refs = read_optional_pod_vec(
            &package,
            TILEMETA_ITEM_TEXTURE_REF_ENTRY_PATH,
        )?;

        Ok(Self {
            package,
            land_tiles: read_pod_vec(&land_bytes, TILEMETA_LAND_ENTRY_PATH)?,
            item_tiles: read_pod_vec(&item_bytes, TILEMETA_ITEM_ENTRY_PATH)?,
            item_texture_ref_spans,
            item_texture_refs,
        })
    }

    pub fn land_tiles(&self) -> &[TileMetaLandTile] {
        &self.land_tiles
    }

    pub fn item_tiles(&self) -> &[TileMetaItemTile] {
        &self.item_tiles
    }

    pub fn land_tile(&self, tile_id: u32) -> Option<&TileMetaLandTile> {
        self.land_tiles.get(tile_id as usize)
    }

    pub fn item_tile(&self, tile_id: u32) -> Option<&TileMetaItemTile> {
        self.item_tiles.get(tile_id as usize)
    }

    pub fn item_texture_refs(&self, tile_id: u32) -> &[TileMetaItemTextureRef] {
        let Some(span) = self.item_texture_ref_spans.get(tile_id as usize) else {
            return &self.item_texture_refs[0..0];
        };

        let start = span.start as usize;
        let end = start.saturating_add(span.len as usize);
        if start > self.item_texture_refs.len() || end > self.item_texture_refs.len() {
            return &self.item_texture_refs[0..0];
        }

        &self.item_texture_refs[start..end]
    }

    pub fn main_ec_texture_ref(&self, tile_id: u32) -> Option<&TileMetaItemTextureRef> {
        let texture_refs = self.item_texture_refs(tile_id);

        texture_refs
            .iter()
            .find(|texture_ref| {
                !texture_ref.is_auxiliary()
                    && texture_ref.is_world_art()
                    && texture_ref.texture_id == tile_id
            })
            .or_else(|| {
                texture_refs.iter().find(|texture_ref| {
                    !texture_ref.is_auxiliary()
                        && texture_ref.is_primary_selected()
                        && texture_ref.is_world_art()
                })
            })
            .or_else(|| {
                texture_refs
                    .iter()
                    .find(|texture_ref| !texture_ref.is_auxiliary() && texture_ref.is_world_art())
            })
            .or_else(|| {
                texture_refs.iter().find(|texture_ref| {
                    !texture_ref.is_auxiliary() && texture_ref.is_primary_selected()
                })
            })
            .or_else(|| texture_refs.iter().find(|texture_ref| !texture_ref.is_auxiliary()))
    }

    pub fn main_ec_texture_id(&self, tile_id: u32) -> Option<u32> {
        self.main_ec_texture_ref(tile_id)
            .map(|texture_ref| texture_ref.texture_id)
            .or_else(|| {
                self.item_tile(tile_id)
                    .and_then(|item| (item.ec_texture_id != 0).then_some(item.ec_texture_id))
            })
    }
}

fn read_optional_pod_vec<T: Pod>(package: &UddpReader, path: &str) -> eyre::Result<Vec<T>> {
    match read_path_entry(package, path) {
        Ok(bytes) => read_pod_vec(&bytes, path),
        Err(_) => Ok(Vec::new()),
    }
}
