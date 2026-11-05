#![allow(dead_code)]

crate::eyre_imports!();
use bytemuck::{Pod, Zeroable};
use derive_new::new;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use crate::classic::verdata::{VerFileId, Verdata};

/* Struct to manage Flags for LandTile and ItemTile */

/// `NoHouse` (bit 32) of the 64-bit flags: no house may be placed on the tile. On land it agrees with the
/// LegacyTerrain `nohouse` flag on every client entry (see ENHANCED_CLIENT_FILES.md).
pub const TILE_FLAG_NO_HOUSE: u64 = 1 << 32;
/// `NoDraw` (bit 33): the tile is not drawn.
pub const TILE_FLAG_NO_DRAW: u64 = 1 << 33;
/// `Rotated` (bit 34): rotated tile art (the EC tileart `Rotated` flag).
pub const TILE_FLAG_ROTATED: u64 = 1 << 34;
/// `AlphaBlend` (bit 35).
pub const TILE_FLAG_ALPHA_BLEND: u64 = 1 << 35;
/// `NoShadow` (bit 36).
pub const TILE_FLAG_NO_SHADOW: u64 = 1 << 36;
/// `PixelBleed` (bit 37).
pub const TILE_FLAG_PIXEL_BLEED: u64 = 1 << 37;
/// `PlayAnimOnce` (bit 39).
pub const TILE_FLAG_PLAY_ANIM_ONCE: u64 = 1 << 39;
/// `MultiMovable` (bit 40).
pub const TILE_FLAG_MULTI_MOVABLE: u64 = 1 << 40;

/// The tile flags. Files from High Seas on store them as 64 bits; older files store 32 bits, read with the upper
/// half 0. The upper 32 bits follow the EC tileart `TaeFlag` layout: comparing a 7.0.x `tiledata.mul` with the EC
/// `tileart.uop` of the same item ids (36,611 records), every upper bit set in the Classic file is set in the tileart
/// record too (NoDraw 3, AlphaBlend 58, NoShadow 26, PixelBleed 11,482, PlayAnimOnce 15, MultiMovable 4 items);
/// land uses only `NoHouse` (94 tiles).
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable, Debug)]
pub struct Flags {
    pub internal_flags: u64,
}

#[allow(unused)]
impl Flags {
    #[inline(always)]
    fn has(&self, mask: u64) -> bool {
        0 != (self.internal_flags & mask)
    }

    fn value(&self) -> u64 {
        self.internal_flags
    }

    pub fn background(&self) -> bool {
        self.has(0x01)
    }
    pub fn weapon(&self) -> bool {
        self.has(0x02)
    }
    pub fn transparent(&self) -> bool {
        self.has(0x04)
    }
    pub fn translucent(&self) -> bool {
        self.has(0x08)
    }
    pub fn wall(&self) -> bool {
        self.has(0x10)
    }
    pub fn damaging(&self) -> bool {
        self.has(0x20)
    }
    pub fn impassable(&self) -> bool {
        self.has(0x40)
    }
    pub fn wet(&self) -> bool {
        self.has(0x80)
    }
    /*pub fn unknown(&self) -> bool {
        0 != ((self.internal_flags & 0x100) >> 8)
    }*/
    pub fn surface(&self) -> bool {
        self.has(0x200)
    }
    pub fn bridge(&self) -> bool {
        self.has(0x400)
    }
    pub fn generic(&self) -> bool {
        self.has(0x800)
    }
    pub fn stackable(&self) -> bool {
        self.generic()
    }
    pub fn window(&self) -> bool {
        self.has(0x1000)
    }
    pub fn noshoot(&self) -> bool {
        self.has(0x2000)
    }
    pub fn prefixa(&self) -> bool {
        self.has(0x4000)
    }
    pub fn prefixan(&self) -> bool {
        self.has(0x8000)
    }
    pub fn internal(&self) -> bool {
        self.has(0x10000)
    }
    pub fn foliage(&self) -> bool {
        self.has(0x20000)
    }
    pub fn partialhue(&self) -> bool {
        self.has(0x40000)
    }
    /*pub fn unknown1(&self) -> bool {
        0 != ((self.internal_flags & 0x80000) >> 19)
    }*/
    pub fn map(&self) -> bool {
        self.has(0x100000)
    }
    pub fn container(&self) -> bool {
        self.has(0x200000)
    }
    pub fn wearable(&self) -> bool {
        self.has(0x400000)
    }
    pub fn lightsource(&self) -> bool {
        self.has(0x800000)
    }
    pub fn animated(&self) -> bool {
        self.has(0x1000000)
    }
    pub fn nodiagonal(&self) -> bool {
        self.has(0x2000000)
    }
    /*pub fn unknown2(&self) -> bool {
        0 != ((self.internal_flags & 0x4000000) >> 26)
    }*/
    pub fn armor(&self) -> bool {
        self.has(0x8000000)
    }
    pub fn roof(&self) -> bool {
        self.has(0x10000000)
    }
    pub fn door(&self) -> bool {
        self.has(0x20000000)
    }
    pub fn stairback(&self) -> bool {
        self.has(0x40000000)
    }
    pub fn stairright(&self) -> bool {
        self.has(0x80000000)
    }
    pub fn nohouse(&self) -> bool {
        self.has(TILE_FLAG_NO_HOUSE)
    }
    pub fn nodraw(&self) -> bool {
        self.has(TILE_FLAG_NO_DRAW)
    }
    pub fn rotated(&self) -> bool {
        self.has(TILE_FLAG_ROTATED)
    }
    pub fn alphablend(&self) -> bool {
        self.has(TILE_FLAG_ALPHA_BLEND)
    }
    pub fn noshadow(&self) -> bool {
        self.has(TILE_FLAG_NO_SHADOW)
    }
    pub fn pixelbleed(&self) -> bool {
        self.has(TILE_FLAG_PIXEL_BLEED)
    }
    pub fn playanimonce(&self) -> bool {
        self.has(TILE_FLAG_PLAY_ANIM_ONCE)
    }
    pub fn multimovable(&self) -> bool {
        self.has(TILE_FLAG_MULTI_MOVABLE)
    }
}

impl Flags {
    /// Flags read from a file that stores 32 bits.
    fn from_32_bits(flags: u32) -> Self {
        Self { internal_flags: u64::from(flags) }
    }

    /// Flags read from a file that stores 64 bits.
    fn from_64_bits(flags: u64) -> Self {
        Self { internal_flags: flags }
    }
}
/* End of Flags struct */

/* Start of LandTile Struct */

#[derive(Clone, new)]
pub struct LandTile {
    /* Internal, utility properties */
    pub tile_id: i32,

    /* File properties */
    #[new(default)]
    pub flags: Flags,

    //pub unk1: i32,    // added with HS
    #[new(default)]
    pub texture_id: u16,

    #[new(default)]
    pub name: [u8; Self::NAME_LEN],
}

impl Default for LandTile {
    fn default() -> Self {
        Self {
            tile_id: Self::TILE_ID_UNUSED,
            flags: Flags::default(),
            texture_id: 0,
            name: [0; Self::NAME_LEN],
        }
    }
}

impl LandTile {
    const TILE_ID_UNUSED: i32 = -1;
    const TILES_PER_BLOCK: usize = 32;
    const BLOCK_QTY: usize = 512;

    const NAME_LEN: usize = 20;

    pub fn name_ascii(&self) -> &str {
        // Names are null-terminated ASCII strings. Find the null terminator
        // and convert the slice up to that point to a &str.
        let null_pos = self
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(Self::NAME_LEN);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }

    fn is_nodraw(&self) -> Option<bool> {
        match self.tile_id {
            Self::TILE_ID_UNUSED => None,
            _ => Some(self.tile_id == 2),
        }
    }
}
/* End of LandTile struct */

/* Start of ItemTile struct */

#[derive(Clone, new)]
pub struct ItemTile {
    // Some documentation was taken from UO Stratics, which may be outdated.

    /* Internal, utility properties */
    pub tile_id: i32,

    /* File properties */
    #[new(default)]
    pub flags: Flags,

    //pub unknown: u32, // Added with HS
    #[new(default)]
    pub weight: u8, // Stratics: 255 means not movable

    #[new(default)]
    pub quality: u8, // Stratics: If Wearable, this is a Layer. If Light Source, this is Light ID

    /// Bytes 2-3 after the weight and quality, kept for the writer (meaning unknown).
    #[new(default)]
    pub unknown0: u16,
    /// The byte after `unknown0`, kept for the writer (meaning unknown).
    #[new(default)]
    pub unknown1: u8,
    #[new(default)]
    pub quantity: u8, // Stratics: If Weapon, this is Weapon Struct. If Armor, Armor Struct

    #[new(default)]
    pub anim_id: u16, // Stratics: The Body ID the animatation. Add 50,000 and 60,000 respectivefully to get the two gump indicies assocaited with this tile

    /// The byte after the anim id, kept for the writer (meaning unknown).
    #[new(default)]
    pub unknown2: u8,
    #[new(default)]
    pub hue_extra: u8, // For colored light sources? or forms a u16 with unknown2 ?

    #[new(default)]
    pub stacking_offset: u8,

    #[new(default)]
    pub value: u8,

    #[new(default)]
    pub height: i8, // Stratics: If Conatainer, this is how much the container can hold

    #[new(default)]
    pub name: [u8; Self::NAME_LEN],
}

impl Default for ItemTile {
    fn default() -> Self {
        Self {
            tile_id: Self::TILE_ID_UNUSED,
            flags: Flags::default(),
            weight: 0,
            quality: 0,
            unknown0: 0,
            unknown1: 0,
            quantity: 0,
            anim_id: 0,
            unknown2: 0,
            hue_extra: 0,
            stacking_offset: 0,
            value: 0,
            height: 0,
            name: [0; Self::NAME_LEN],
        }
    }
}

impl ItemTile {
    const TILE_ID_UNUSED: i32 = -1;
    const TILES_PER_BLOCK: usize = 32;

    const NAME_LEN: usize = 20;

    fn height(&self) -> i8 {
        if self.flags.bridge() {
            self.height / 2
        } else {
            self.height
        }
    }
    pub fn height_raw(&self) -> i8 {
        self.height
    }

    fn gump_id_male(&self) -> u32 {
        self.anim_id as u32 + 50_000
    }
    fn gump_id_female(&self) -> u32 {
        self.anim_id as u32 + 60_000
    }

    pub fn name_ascii(&self) -> &str {
        // Names are null-terminated ASCII strings. Find the null terminator
        // and convert the slice up to that point to a &str.
        let null_pos = self
            .name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(Self::NAME_LEN);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }

    fn is_nodraw(&self) -> Option<bool> {
        let tid = self.tile_id;
        match tid {
            Self::TILE_ID_UNUSED => None,
            _ => Some(
                tid == 1
                    || tid == 8600
                    || tid == 8601
                    || tid == 8602
                    || tid == 8603
                    || tid == 8604
                    || tid == 8605
                    || tid == 8606
                    || tid == 8607
                    || tid == 8608
                    || tid == 8609
                    || tid == 8610
                    || tid == 8611
                    || tid == 8636,
            ),
        }
    }
}
/* End of ItemTile struct */

/* Enums for Tiledata file structure */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LandTileBinSize {
    Classic = 26,
    HS = 26 + 4, // From Stygian Abyss: High Seas and on
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemTileBinSize {
    Classic = 37,
    HS = 37 + 4, // From Stygian Abyss: High Seas and on
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemTileMaxIdxRev {
    // Inclusive: the value stores the last valid slot.
    // Slots starts from 0.

    // Classic
    Revision1 = 0x3FFF,
    // From Stygian Abyss and on
    Revision2 = 0x7FFF,
    Revision3 = 0xFFFF,
}

/// Bytes of the u32 header that opens each block of 32 tiles.
const TILE_BLOCK_HEADER_SIZE: u64 = 4;

/// A `tiledata.mul` layout: the record sizes (26/37 bytes with 32-bit flags, 30/41 with 64-bit flags) and the
/// static tile count. Both sections are whole blocks, 512 land blocks and `static count / 32` static blocks, so
/// the file length tells the layouts apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TiledataLayout {
    land: LandTileBinSize,
    item: ItemTileBinSize,
    max_item: ItemTileMaxIdxRev,
}

impl TiledataLayout {
    /// The layouts read. Measured on client files: UO Gold and Mondain's Legacy ship 32-bit flags with 16,384
    /// statics (1,036,288 bytes); 7.0.x from High Seas on ship 64-bit flags with 65,536 statics (3,188,736 bytes).
    /// Stygian Abyss before High Seas kept 32-bit flags with 32,768 statics (1,644,544 bytes). The other pairings
    /// of flag width and static count are read too: no two layouts have the same length.
    const KNOWN: [Self; 6] = [
        Self::new(LandTileBinSize::Classic, ItemTileBinSize::Classic, ItemTileMaxIdxRev::Revision1),
        Self::new(LandTileBinSize::Classic, ItemTileBinSize::Classic, ItemTileMaxIdxRev::Revision2),
        Self::new(LandTileBinSize::Classic, ItemTileBinSize::Classic, ItemTileMaxIdxRev::Revision3),
        Self::new(LandTileBinSize::HS, ItemTileBinSize::HS, ItemTileMaxIdxRev::Revision1),
        Self::new(LandTileBinSize::HS, ItemTileBinSize::HS, ItemTileMaxIdxRev::Revision2),
        Self::new(LandTileBinSize::HS, ItemTileBinSize::HS, ItemTileMaxIdxRev::Revision3),
    ];

    const fn new(land: LandTileBinSize, item: ItemTileBinSize, max_item: ItemTileMaxIdxRev) -> Self {
        Self { land, item, max_item }
    }

    const fn file_size(self) -> u64 {
        let tiles_per_block = LandTile::TILES_PER_BLOCK as u64;
        let land_block = TILE_BLOCK_HEADER_SIZE + self.land as u64 * tiles_per_block;
        let item_block = TILE_BLOCK_HEADER_SIZE + self.item as u64 * ItemTile::TILES_PER_BLOCK as u64;
        let item_blocks = (1 + self.max_item as u64) / ItemTile::TILES_PER_BLOCK as u64;
        land_block * LandTile::BLOCK_QTY as u64 + item_block * item_blocks
    }

    fn from_file_size(file_size: u64) -> eyre::Result<Self> {
        Self::KNOWN.into_iter().find(|layout| layout.file_size() == file_size).ok_or_else(|| {
            let known: Vec<String> = Self::KNOWN.iter().map(|layout| layout.file_size().to_string()).collect();
            eyre!("Malformed tiledata.mul? Size: {file_size} matches no known layout ({})", known.join(", "))
        })
    }
}
/* End of enums for Tiledata file structure */

/// Internal structures used for fast bulk parsing of the tiledata.mul file.
/// Design Choice: We use `#[repr(C, packed)]` to match the UO file format exactly on disk.
/// This allows us to use `bytemuck` to cast large byte sections into these structured blocks,
/// which is significantly faster than reading individual fields via `ReadBytesExt`.
///
/// Note: Endianness is handled during the conversion from Raw to final struct.
/// Since most modern systems are Little Endian (like UO data), this is often a zero-cost operation.

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawLandTileClassic {
    flags: u32,
    texture_id: u16,
    name: [u8; LandTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawLandTileHS {
    flags: u64,
    texture_id: u16,
    name: [u8; LandTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawItemTileClassic {
    flags: u32,
    weight: u8,
    quality: u8,
    unk0: u16,
    unk1: u8,
    quantity: u8,
    anim_id: u16,
    unk2: u8,
    hue_extra: u8,
    stacking_offset: u8,
    value: u8,
    height: i8,
    name: [u8; ItemTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawItemTileHS {
    flags: u64,
    weight: u8,
    quality: u8,
    unk0: u16,
    unk1: u8,
    quantity: u8,
    anim_id: u16,
    unk2: u8,
    hue_extra: u8,
    stacking_offset: u8,
    value: u8,
    height: i8,
    name: [u8; ItemTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawBlock<T> {
    header: u32,
    tiles: [T; 32],
}

fn block_headers<T: Pod>(blocks: &[RawBlock<T>]) -> Vec<u32> {
    blocks.iter().map(|block| block.header).collect()
}

/// Appends a block per header: the header, then its 32 tiles as records.
fn push_blocks<Tile, Raw: Pod>(
    out: &mut Vec<u8>,
    headers: &[u32],
    tiles: &[Tile],
    mut raw: impl FnMut(&Tile) -> eyre::Result<Raw>,
) -> eyre::Result<()> {
    for (header, block) in headers.iter().zip(tiles.chunks_exact(LandTile::TILES_PER_BLOCK)) {
        out.extend_from_slice(&header.to_le_bytes());
        for tile in block {
            out.extend_from_slice(bytemuck::bytes_of(&raw(tile)?));
        }
    }
    Ok(())
}

/* Start of Tiledata struct */

#[derive(Clone)]
pub struct TileData {
    land_tile_binary_size: LandTileBinSize,
    item_tile_binary_size: ItemTileBinSize,
    max_item_rev: ItemTileMaxIdxRev,
    land_data: Vec<LandTile>,
    item_data: Vec<ItemTile>,
    /// The u32 that opens each block of 32 tiles, kept for the writer (meaning unknown).
    land_block_headers: Vec<u32>,
    item_block_headers: Vec<u32>,
}
impl TileData {
    const LAND_TILE_MAX: usize = 0x4000;
    //const ITEM_TILE_MAX: usize = ItemTileMaxIdxRev::Revision3 as usize;

    /* Methods */

    #[inline(always)]
    fn push_tiles_from_blocks<RawTile, Tile, F>(
        out: &mut Vec<Tile>,
        blocks: &[RawBlock<RawTile>],
        mut make_tile: F,
    ) where
        RawTile: Pod,
        F: FnMut(i32, &RawTile) -> Tile,
    {
        const TILES_PER_BLOCK: usize = 32;
        out.reserve(blocks.len() * TILES_PER_BLOCK);
        let mut tile_id = out.len() as i32;
        for block in blocks {
            let tiles_ptr = std::ptr::addr_of!(block.tiles) as *const RawTile;
            for tile_idx in 0..TILES_PER_BLOCK {
                let raw = unsafe { std::ptr::read_unaligned(tiles_ptr.add(tile_idx)) };
                out.push(make_tile(tile_id, &raw));
                tile_id += 1;
            }
        }
    }

    pub fn land_tiles(&self) -> &[LandTile] {
        &self.land_data
    }

    pub fn item_tiles(&self) -> &[ItemTile] {
        &self.item_data
    }

    /// Whether the file stores 64-bit tile flags (High Seas on) rather than 32-bit ones.
    pub fn has_64_bit_flags(&self) -> bool {
        self.land_tile_binary_size == LandTileBinSize::HS
    }

    /// The bytes of a land and of a static tile record in the file.
    pub fn record_sizes(&self) -> (usize, usize) {
        (self.land_tile_binary_size as usize, self.item_tile_binary_size as usize)
    }

    pub fn new_empty() -> Self {
        Self {
            land_tile_binary_size: LandTileBinSize::Classic,
            item_tile_binary_size: ItemTileBinSize::Classic,
            max_item_rev: ItemTileMaxIdxRev::Revision1,
            land_data: Vec::new(),
            item_data: Vec::new(),
            land_block_headers: Vec::new(),
            item_block_headers: Vec::new(),
        }
    }

    pub fn land_tiles_mut(&mut self) -> &mut [LandTile] {
        &mut self.land_data
    }

    pub fn item_tiles_mut(&mut self) -> &mut [ItemTile] {
        &mut self.item_data
    }

    /// The `tiledata.mul` bytes, in the layout the file was read with: the block headers, then every land and item
    /// record with its unknown bytes. A file read with [`Self::load`] comes back byte for byte (with its verdata
    /// patches applied when it was read with them). A 32-bit layout refuses flags above bit 31.
    pub fn to_bytes(&self) -> eyre::Result<Vec<u8>> {
        let layout = TiledataLayout::new(self.land_tile_binary_size, self.item_tile_binary_size, self.max_item_rev);
        let land_blocks = LandTile::BLOCK_QTY;
        let item_count = 1 + self.max_item_rev as usize;
        let item_blocks = item_count / ItemTile::TILES_PER_BLOCK;
        eyre::ensure!(
            self.land_data.len() == land_blocks * LandTile::TILES_PER_BLOCK && self.land_block_headers.len() == land_blocks,
            "{} land tiles and {} block headers; the layout has {} and {land_blocks}",
            self.land_data.len(),
            self.land_block_headers.len(),
            land_blocks * LandTile::TILES_PER_BLOCK
        );
        eyre::ensure!(
            self.item_data.len() == item_count && self.item_block_headers.len() == item_blocks,
            "{} item tiles and {} block headers; the layout has {item_count} and {item_blocks}",
            self.item_data.len(),
            self.item_block_headers.len()
        );
        let mut out = Vec::with_capacity(layout.file_size() as usize);
        let narrow = |tile_kind: &str, tile_id: i32, flags: Flags| {
            u32::try_from(flags.internal_flags).map_err(|_| {
                eyre!("{tile_kind} tile {tile_id}: flags {:#x} do not fit the file's 32 bits", flags.internal_flags)
            })
        };
        if self.land_tile_binary_size == LandTileBinSize::Classic {
            push_blocks(&mut out, &self.land_block_headers, &self.land_data, |tile| {
                Ok(RawLandTileClassic {
                    flags: narrow("land", tile.tile_id, tile.flags)?,
                    texture_id: tile.texture_id,
                    name: tile.name,
                })
            })?;
        } else {
            push_blocks(&mut out, &self.land_block_headers, &self.land_data, |tile| {
                Ok(RawLandTileHS { flags: tile.flags.internal_flags, texture_id: tile.texture_id, name: tile.name })
            })?;
        }
        if self.item_tile_binary_size == ItemTileBinSize::Classic {
            push_blocks(&mut out, &self.item_block_headers, &self.item_data, |tile| {
                Ok(RawItemTileClassic {
                    flags: narrow("item", tile.tile_id, tile.flags)?,
                    weight: tile.weight,
                    quality: tile.quality,
                    unk0: tile.unknown0,
                    unk1: tile.unknown1,
                    quantity: tile.quantity,
                    anim_id: tile.anim_id,
                    unk2: tile.unknown2,
                    hue_extra: tile.hue_extra,
                    stacking_offset: tile.stacking_offset,
                    value: tile.value,
                    height: tile.height,
                    name: tile.name,
                })
            })?;
        } else {
            push_blocks(&mut out, &self.item_block_headers, &self.item_data, |tile| {
                Ok(RawItemTileHS {
                    flags: tile.flags.internal_flags,
                    weight: tile.weight,
                    quality: tile.quality,
                    unk0: tile.unknown0,
                    unk1: tile.unknown1,
                    quantity: tile.quantity,
                    anim_id: tile.anim_id,
                    unk2: tile.unknown2,
                    hue_extra: tile.hue_extra,
                    stacking_offset: tile.stacking_offset,
                    value: tile.value,
                    height: tile.height,
                    name: tile.name,
                })
            })?;
        }
        debug_assert_eq!(out.len() as u64, layout.file_size());
        Ok(out)
    }

    /// Writes [`to_bytes`](Self::to_bytes) to `path` through a temporary file in its folder: a failed save leaves
    /// `path` as it was.
    pub fn save(&self, path: &std::path::Path) -> eyre::Result<()> {
        let bytes = self.to_bytes()?;
        crate::utils::output_file::write_through_temp(path, |temp| {
            std::fs::write(temp, &bytes).wrap_err_with(|| format!("Write tiledata.mul at '{}'", temp.display()))
        })
    }

    pub fn load(file_path: PathBuf) -> eyre::Result<TileData> {
        Self::load_with_verdata(file_path, None)
    }

    pub fn load_with_verdata(
        file_path: PathBuf,
        verdata: Option<Arc<Verdata>>,
    ) -> eyre::Result<TileData> {
        let file_path = file_path
            .canonicalize()
            .wrap_err("Check tiledata.mul path")?;

        let mut file_handle = File::open(&file_path)
            .wrap_err_with(|| format!("Open tiledata.mul at '{}'", file_path.to_string_lossy()))?;
        let file_metadata = file_handle
            .metadata()
            .wrap_err("Get tiledata.mul metadata")?;

        let file_size = file_metadata.len();
        let layout = TiledataLayout::from_file_size(file_size)?;
        let mut tiledata = TileData {
            land_tile_binary_size: layout.land,
            item_tile_binary_size: layout.item,
            max_item_rev: layout.max_item,
            land_data: Vec::with_capacity(TileData::LAND_TILE_MAX),
            item_data: Vec::new(),
            land_block_headers: Vec::new(),
            item_block_headers: Vec::new(),
        };
        tiledata.item_data = Vec::with_capacity(1 + tiledata.max_item_rev as usize);

        log::info!(
            "uocf: Found Tiledata with size: {file_size}. \n\
        Detected LandTile size: {:?}, ItemTile size: {:?}, Max Item count: {:?} (0x{:X})",
            tiledata.land_tile_binary_size,
            tiledata.item_tile_binary_size,
            tiledata.max_item_rev,
            tiledata.max_item_rev as u32
        );

        let mut tiledata_file_bytes = {
            let mut buf = vec![0; file_size as usize];
            file_handle
                .read_exact(buf.as_mut())
                .wrap_err("Read tiledata.mul")?;
            buf
        };

        if let Some(verdata) = &verdata {
            apply_tiledata_verdata_patches(
                &mut tiledata_file_bytes,
                verdata,
                tiledata.land_tile_binary_size as usize,
                tiledata.item_tile_binary_size as usize,
                tiledata.max_item_rev as usize,
            )?;
        }

        // Read LandTiles
        // Optimization: We use bulk parsing to avoid thousands of individual I/O reads.
        // We first calculate the exact byte size of the land section and slice the buffer
        // to pass it to bytemuck. This prevents panics if the file has trailing bytes.
        let land_section_len = if tiledata.land_tile_binary_size == LandTileBinSize::Classic {
            LandTile::BLOCK_QTY * std::mem::size_of::<RawBlock<RawLandTileClassic>>()
        } else {
            LandTile::BLOCK_QTY * std::mem::size_of::<RawBlock<RawLandTileHS>>()
        };

        let land_bytes = &tiledata_file_bytes[..land_section_len];

        if tiledata.land_tile_binary_size == LandTileBinSize::Classic {
            let blocks: &[RawBlock<RawLandTileClassic>] = bytemuck::cast_slice(land_bytes);
            tiledata.land_block_headers = block_headers(blocks);
            Self::push_tiles_from_blocks(&mut tiledata.land_data, blocks, |tile_id, raw| {
                LandTile {
                    tile_id,
                    flags: Flags::from_32_bits(raw.flags),
                    texture_id: raw.texture_id,
                    name: raw.name,
                }
            });
        } else {
            let blocks: &[RawBlock<RawLandTileHS>] = bytemuck::cast_slice(land_bytes);
            tiledata.land_block_headers = block_headers(blocks);
            Self::push_tiles_from_blocks(&mut tiledata.land_data, blocks, |tile_id, raw| {
                LandTile {
                    tile_id,
                    flags: Flags::from_64_bits(raw.flags),
                    texture_id: raw.texture_id,
                    name: raw.name,
                }
            });
        }
        let i_tile = tiledata.land_data.len() as u32;
        log::info!("uocf: Loaded {i_tile} (0x{:x}) LandTiles.", i_tile);

        // Read ItemTiles
        // Optimization: Same as above, we slice the buffer for the item section specifically.
        let block_qty: usize = (1 + tiledata.max_item_rev as usize) / ItemTile::TILES_PER_BLOCK;
        let item_section_len = if tiledata.item_tile_binary_size == ItemTileBinSize::Classic {
            block_qty * std::mem::size_of::<RawBlock<RawItemTileClassic>>()
        } else {
            block_qty * std::mem::size_of::<RawBlock<RawItemTileHS>>()
        };

        // The item section starts immediately after the land section
        let item_bytes =
            &tiledata_file_bytes[land_section_len..land_section_len + item_section_len];

        if tiledata.item_tile_binary_size == ItemTileBinSize::Classic {
            let blocks: &[RawBlock<RawItemTileClassic>] = bytemuck::cast_slice(item_bytes);
            tiledata.item_block_headers = block_headers(blocks);
            Self::push_tiles_from_blocks(&mut tiledata.item_data, blocks, |tile_id, raw| {
                ItemTile {
                    tile_id,
                    flags: Flags::from_32_bits(raw.flags),
                    weight: raw.weight,
                    quality: raw.quality,
                    unknown0: raw.unk0,
                    unknown1: raw.unk1,
                    quantity: raw.quantity,
                    anim_id: raw.anim_id,
                    unknown2: raw.unk2,
                    hue_extra: raw.hue_extra,
                    stacking_offset: raw.stacking_offset,
                    value: raw.value,
                    height: raw.height,
                    name: raw.name,
                }
            });
        } else {
            let blocks: &[RawBlock<RawItemTileHS>] = bytemuck::cast_slice(item_bytes);
            tiledata.item_block_headers = block_headers(blocks);
            Self::push_tiles_from_blocks(&mut tiledata.item_data, blocks, |tile_id, raw| {
                ItemTile {
                    tile_id,
                    flags: Flags::from_64_bits(raw.flags),
                    weight: raw.weight,
                    quality: raw.quality,
                    unknown0: raw.unk0,
                    unknown1: raw.unk1,
                    quantity: raw.quantity,
                    anim_id: raw.anim_id,
                    unknown2: raw.unk2,
                    hue_extra: raw.hue_extra,
                    stacking_offset: raw.stacking_offset,
                    value: raw.value,
                    height: raw.height,
                    name: raw.name,
                }
            });
        }
        let i_tile = tiledata.item_data.len() as u32;
        log::info!("uocf: Loaded {i_tile} (0x{:x}) Item Tiles.", i_tile);

        Ok(tiledata)
    }
}

/* End of Tiledata struct */

fn apply_tiledata_verdata_patches(
    tiledata_file_bytes: &mut [u8],
    verdata: &Verdata,
    land_tile_size: usize,
    item_tile_size: usize,
    max_item_id: usize,
) -> eyre::Result<()> {
    let land_block_size = 4 + land_tile_size * LandTile::TILES_PER_BLOCK;
    let land_section_len = LandTile::BLOCK_QTY * land_block_size;
    let item_block_size = 4 + item_tile_size * ItemTile::TILES_PER_BLOCK;

    for (tile_id, entry) in verdata.entries_for(VerFileId::Tiledata) {
        if *tile_id < 0 {
            continue;
        }
        let tile_id = *tile_id as usize;
        if tile_id > max_item_id + TileData::LAND_TILE_MAX {
            continue;
        }
        let patch = verdata.read_patch_data(entry)?;

        let (offset, expected_len) = if tile_id < TileData::LAND_TILE_MAX {
            let block = tile_id / LandTile::TILES_PER_BLOCK;
            let tile_in_block = tile_id % LandTile::TILES_PER_BLOCK;
            (
                block * land_block_size + 4 + tile_in_block * land_tile_size,
                land_tile_size,
            )
        } else {
            let item_id = tile_id - TileData::LAND_TILE_MAX;
            let block = item_id / ItemTile::TILES_PER_BLOCK;
            let tile_in_block = item_id % ItemTile::TILES_PER_BLOCK;
            (
                land_section_len + block * item_block_size + 4 + tile_in_block * item_tile_size,
                item_tile_size,
            )
        };

        if patch.len() < expected_len || offset + expected_len > tiledata_file_bytes.len() {
            continue;
        }
        tiledata_file_bytes[offset..offset + expected_len]
            .copy_from_slice(&patch[..expected_len]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Land tile whose flags the tests set.
    const FLAGGED_LAND_TILE: usize = 3;
    /// Item tile whose flags the tests set.
    const FLAGGED_ITEM_TILE: usize = 5;
    /// Flags with a lower bit (impassable) and, in 64-bit files, upper bits.
    const LOW_FLAGS: u32 = 0x40;
    const HIGH_FLAGS: u64 = TILE_FLAG_NO_HOUSE | TILE_FLAG_PIXEL_BLEED;

    /// A tiledata file of `item_blocks` item blocks whose tile records are `land_size` and `item_size` bytes, with
    /// `flags` (stored in `flag_bytes` bytes) on the flagged land and item tile.
    fn tiledata_bytes(land_size: usize, item_size: usize, item_blocks: usize, flags: u64, flag_bytes: usize) -> Vec<u8> {
        const HEADER: usize = 4;
        let land_block = HEADER + land_size * LandTile::TILES_PER_BLOCK;
        let item_block = HEADER + item_size * ItemTile::TILES_PER_BLOCK;
        let land_section = LandTile::BLOCK_QTY * land_block;
        let mut bytes = vec![0u8; land_section + item_blocks * item_block];
        let flag_value = flags.to_le_bytes();
        let land_offset = HEADER + FLAGGED_LAND_TILE * land_size;
        bytes[land_offset..land_offset + flag_bytes].copy_from_slice(&flag_value[..flag_bytes]);
        let item_offset = land_section + HEADER + FLAGGED_ITEM_TILE * item_size;
        bytes[item_offset..item_offset + flag_bytes].copy_from_slice(&flag_value[..flag_bytes]);
        bytes
    }

    fn load(name: &str, bytes: &[u8]) -> TileData {
        let path = std::env::temp_dir().join(format!("uocf_tiledata_{name}_{}.mul", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let tiledata = TileData::load(path.clone()).unwrap();
        std::fs::remove_file(path).unwrap();
        tiledata
    }

    /// Every byte comes back, block headers and the unknown item bytes included, in both flag widths; a 32-bit
    /// file refuses flags above bit 31.
    #[test]
    fn every_layout_writes_back_byte_for_byte() {
        const HEADER_SEED: u8 = 0xA5;
        for (land, item, max_item, flag_bytes) in [
            (LandTileBinSize::Classic, ItemTileBinSize::Classic, ItemTileMaxIdxRev::Revision1, std::mem::size_of::<u32>()),
            (LandTileBinSize::HS, ItemTileBinSize::HS, ItemTileMaxIdxRev::Revision3, std::mem::size_of::<u64>()),
        ] {
            let item_blocks = (1 + max_item as usize) / ItemTile::TILES_PER_BLOCK;
            let mut bytes =
                tiledata_bytes(land as usize, item as usize, item_blocks, u64::from(LOW_FLAGS), flag_bytes);
            // Every byte nonzero and distinct by position, so a dropped or moved field shows.
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = HEADER_SEED ^ (index as u8);
            }
            let tiledata = load("rt", &bytes);
            assert_eq!(tiledata.to_bytes().unwrap(), bytes, "{land:?} {item:?}");
            if land == LandTileBinSize::Classic {
                let mut edited = tiledata;
                edited.item_tiles_mut()[FLAGGED_ITEM_TILE].flags.internal_flags = HIGH_FLAGS;
                assert!(edited.to_bytes().is_err());
            }
        }
    }

    #[test]
    fn high_seas_files_keep_the_upper_flag_bits() {
        let flags = HIGH_FLAGS | u64::from(LOW_FLAGS);
        let item_blocks = (1 + ItemTileMaxIdxRev::Revision3 as usize) / ItemTile::TILES_PER_BLOCK;
        let bytes = tiledata_bytes(
            LandTileBinSize::HS as usize,
            ItemTileBinSize::HS as usize,
            item_blocks,
            flags,
            std::mem::size_of::<u64>(),
        );
        let tiledata = load("hs", &bytes);
        for tile_flags in [tiledata.land_tiles()[FLAGGED_LAND_TILE].flags, tiledata.item_tiles()[FLAGGED_ITEM_TILE].flags] {
            assert_eq!(tile_flags.internal_flags, flags);
            assert!(tile_flags.nohouse() && tile_flags.pixelbleed() && tile_flags.impassable());
            assert!(!tile_flags.nodraw());
        }
    }

    /// UO Gold's `tiledata.mul` (32-bit flags, 16,384 statics) and a High Seas one (64-bit flags, 65,536 statics).
    const GOLD_TILEDATA_SIZE: u64 = 1_036_288;
    const HIGH_SEAS_TILEDATA_SIZE: u64 = 3_188_736;

    #[test]
    fn layouts_have_distinct_lengths() {
        let sizes: std::collections::BTreeSet<u64> =
            TiledataLayout::KNOWN.iter().map(|layout| layout.file_size()).collect();
        assert_eq!(sizes.len(), TiledataLayout::KNOWN.len());
        assert_eq!(TiledataLayout::KNOWN[0].file_size(), GOLD_TILEDATA_SIZE);
        assert_eq!(TiledataLayout::KNOWN[5].file_size(), HIGH_SEAS_TILEDATA_SIZE);
        assert!(TiledataLayout::from_file_size(GOLD_TILEDATA_SIZE + 1).is_err());
    }

    #[test]
    fn stygian_abyss_files_read_32_bit_flags_with_more_statics() {
        let item_blocks = (1 + ItemTileMaxIdxRev::Revision2 as usize) / ItemTile::TILES_PER_BLOCK;
        let bytes = tiledata_bytes(
            LandTileBinSize::Classic as usize,
            ItemTileBinSize::Classic as usize,
            item_blocks,
            u64::from(LOW_FLAGS),
            std::mem::size_of::<u32>(),
        );
        let tiledata = load("sa", &bytes);
        assert!(!tiledata.has_64_bit_flags());
        assert_eq!(tiledata.item_tiles().len(), 1 + ItemTileMaxIdxRev::Revision2 as usize);
        assert_eq!(tiledata.item_tiles()[FLAGGED_ITEM_TILE].flags.internal_flags, u64::from(LOW_FLAGS));
    }

    #[test]
    fn older_files_read_32_bit_flags() {
        let item_blocks = (1 + ItemTileMaxIdxRev::Revision1 as usize) / ItemTile::TILES_PER_BLOCK;
        let bytes = tiledata_bytes(
            LandTileBinSize::Classic as usize,
            ItemTileBinSize::Classic as usize,
            item_blocks,
            u64::from(LOW_FLAGS),
            std::mem::size_of::<u32>(),
        );
        let tiledata = load("classic", &bytes);
        let item = &tiledata.item_tiles()[FLAGGED_ITEM_TILE];
        assert_eq!(item.flags.internal_flags, u64::from(LOW_FLAGS));
        assert_eq!(tiledata.land_tiles()[FLAGGED_LAND_TILE].flags.internal_flags, u64::from(LOW_FLAGS));
        assert!(item.flags.impassable() && !item.flags.nohouse());
    }

    /// A save that cannot finish (here a 32-bit file holding an upper flag bit, then a full disk) leaves the file.
    #[test]
    fn a_failed_save_leaves_the_file_as_it_was() {
        const OLD_CONTENTS: &[u8] = b"old";
        let item_blocks = (1 + ItemTileMaxIdxRev::Revision1 as usize) / ItemTile::TILES_PER_BLOCK;
        let bytes = tiledata_bytes(
            LandTileBinSize::Classic as usize,
            ItemTileBinSize::Classic as usize,
            item_blocks,
            u64::from(LOW_FLAGS),
            std::mem::size_of::<u32>(),
        );
        let mut tiledata = load("failed_save", &bytes);
        let dir = std::env::temp_dir().join(format!("uocf_tiledata_failed_save_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("tiledata.mul");
        std::fs::write(&output, OLD_CONTENTS).unwrap();

        crate::utils::output_file::block_temp_file(&output);
        assert!(tiledata.save(&output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), OLD_CONTENTS);
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&output, OLD_CONTENTS).unwrap();

        tiledata.item_tiles_mut()[FLAGGED_ITEM_TILE].flags.internal_flags = HIGH_FLAGS;
        assert!(tiledata.save(&output).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), OLD_CONTENTS);

        tiledata.item_tiles_mut()[FLAGGED_ITEM_TILE].flags.internal_flags = u64::from(LOW_FLAGS);
        tiledata.save(&output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
