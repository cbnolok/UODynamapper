#![allow(dead_code)]

crate::eyre_imports!();
use derive_new::new;
use std::fs::File;
use std::io::prelude::*;
use std::path::PathBuf;
use bytemuck::{Pod, Zeroable};

/* Struct to manage Flags for LandTile and ItemTile */

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Flags {
    pub internal_flags: u32,
}

#[allow(unused)]
impl Flags {
    #[inline(always)]
    fn has(&self, mask: u32) -> bool {
        0 != (self.internal_flags & mask)
    }

    fn value(&self) -> u32 {
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
}
/* End of Flags struct */

/* Start of LandTile Struct */

#[derive(Clone, Debug, new)]
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
        let null_pos = self.name.iter().position(|&c| c == 0).unwrap_or(Self::NAME_LEN);
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

#[derive(Clone, Debug, new)]
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

    //pub unknown0: u16,
    //pub unknown1: u8,
    #[new(default)]
    pub quantity: u8, // Stratics: If Weapon, this is Weapon Struct. If Armor, Armor Struct

    #[new(default)]
    pub anim_id: u16, // Stratics: The Body ID the animatation. Add 50,000 and 60,000 respectivefully to get the two gump indicies assocaited with this tile

    //pub unknown2: u8,
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
            quantity: 0,
            anim_id: 0,
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
        let null_pos = self.name.iter().position(|&c| c == 0).unwrap_or(Self::NAME_LEN);
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
    flags: Flags,
    texture_id: u16,
    name: [u8; LandTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawLandTileHS {
    flags: Flags,
    unk: i32,
    texture_id: u16,
    name: [u8; LandTile::NAME_LEN],
}

#[repr(C, packed)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawItemTileClassic {
    flags: Flags,
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
    flags: Flags,
    unk_hs: i32,
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

/* Start of Tiledata struct */

pub struct TileData {
    land_tile_binary_size: LandTileBinSize,
    item_tile_binary_size: ItemTileBinSize,
    max_item_rev: ItemTileMaxIdxRev,
    land_data: Vec<LandTile>,
    item_data: Vec<ItemTile>,
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

    pub fn load(file_path: PathBuf) -> eyre::Result<TileData> {
        let file_path = file_path
            .canonicalize()
            .wrap_err("Check tiledata.mul path")?;

        let mut file_handle = File::open(&file_path)
            .wrap_err_with(|| format!("Open tiledata.mul at '{}'", file_path.to_string_lossy()))?;
        let file_metadata = file_handle
            .metadata()
            .wrap_err("Get tiledata.mul metadata")?;

        const FILE_SIZE_REV1: u64 = {
            const LAND_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (LandTileBinSize::Classic as u64 * LandTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 = LandTile::BLOCK_QTY as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            const ITEM_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (ItemTileBinSize::Classic as u64 * ItemTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 =
                    (1 + ItemTileMaxIdxRev::Revision1 as u64) / ItemTile::TILES_PER_BLOCK as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            LAND_SECTION_SIZE + ITEM_SECTION_SIZE
        };

        const FILE_SIZE_REV2: u64 = {
            const LAND_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (LandTileBinSize::HS as u64 * LandTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 = LandTile::BLOCK_QTY as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            const ITEM_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (ItemTileBinSize::HS as u64 * ItemTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 =
                    (1 + ItemTileMaxIdxRev::Revision2 as u64) / ItemTile::TILES_PER_BLOCK as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            LAND_SECTION_SIZE + ITEM_SECTION_SIZE
        };

        const FILE_SIZE_REV3: u64 = {
            const LAND_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (LandTileBinSize::HS as u64 * LandTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 = LandTile::BLOCK_QTY as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            const ITEM_SECTION_SIZE: u64 = {
                const BLOCK_SIZE: u64 = 4 /* u32 header */ + (ItemTileBinSize::HS as u64 * ItemTile::TILES_PER_BLOCK as u64);
                const BLOCK_QTY: u64 =
                    (1 + ItemTileMaxIdxRev::Revision3 as u64) / ItemTile::TILES_PER_BLOCK as u64;
                BLOCK_SIZE * BLOCK_QTY
            };
            LAND_SECTION_SIZE + ITEM_SECTION_SIZE
        };

        let file_size = file_metadata.len();
        if file_size < FILE_SIZE_REV1 {
            return Err(eyre!(
                "Tiledata.mul too short: it doesn't have room for land tile data.".to_owned()
            ));
        }

        let mut tiledata = TileData {
            land_tile_binary_size: LandTileBinSize::Classic,
            item_tile_binary_size: ItemTileBinSize::Classic,
            max_item_rev: ItemTileMaxIdxRev::Revision1,
            land_data: Vec::with_capacity(TileData::LAND_TILE_MAX),
            item_data: Vec::new(),
        };

        if file_size == FILE_SIZE_REV1 {
            tiledata = TileData {
                land_tile_binary_size: LandTileBinSize::Classic,
                item_tile_binary_size: ItemTileBinSize::Classic,
                max_item_rev: ItemTileMaxIdxRev::Revision1,
                ..tiledata
            };
        } else if file_size == FILE_SIZE_REV2 {
            tiledata = TileData {
                land_tile_binary_size: LandTileBinSize::HS,
                item_tile_binary_size: ItemTileBinSize::HS,
                max_item_rev: ItemTileMaxIdxRev::Revision2,
                ..tiledata
            };
        } else if file_size == FILE_SIZE_REV3 {
            tiledata = TileData {
                land_tile_binary_size: LandTileBinSize::HS,
                item_tile_binary_size: ItemTileBinSize::HS,
                max_item_rev: ItemTileMaxIdxRev::Revision3,
                ..tiledata
            };
        } else {
            return Err(eyre!(
                format!("Malformed tiledata.mul? Size: {file_size}").to_owned()
            ));
        }
        tiledata.item_data = Vec::with_capacity(1 + tiledata.max_item_rev as usize);

        log::info!(
        "Found Tiledata with size: {file_size}. \n\
        Detected LandTile size: {:?}, ItemTile size: {:?}, Max Item count: {:?} (0x{:X})",
            tiledata.land_tile_binary_size,
            tiledata.item_tile_binary_size,
            tiledata.max_item_rev,
            tiledata.max_item_rev as u32
        );

        let tiledata_file_bytes = {
            let mut buf = vec![0; file_size as usize];
            file_handle
                .read_exact(buf.as_mut())
                .wrap_err("Read tiledata.mul")?;
            buf
        };


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
            Self::push_tiles_from_blocks(&mut tiledata.land_data, blocks, |tile_id, raw| {
                LandTile {
                    tile_id,
                    flags: raw.flags,
                    texture_id: raw.texture_id,
                    name: raw.name,
                }
            });
        } else {
            let blocks: &[RawBlock<RawLandTileHS>] = bytemuck::cast_slice(land_bytes);
            Self::push_tiles_from_blocks(&mut tiledata.land_data, blocks, |tile_id, raw| {
                LandTile {
                    tile_id,
                    flags: raw.flags,
                    texture_id: raw.texture_id,
                    name: raw.name,
                }
            });
        }
        let i_tile = tiledata.land_data.len() as u32;
        log::info!("Loaded {i_tile} (0x{:x}) LandTiles.", i_tile);

        // Read ItemTiles
        // Optimization: Same as above, we slice the buffer for the item section specifically.
        let block_qty: usize = (1 + tiledata.max_item_rev as usize) / ItemTile::TILES_PER_BLOCK;
        let item_section_len = if tiledata.item_tile_binary_size == ItemTileBinSize::Classic {
            block_qty * std::mem::size_of::<RawBlock<RawItemTileClassic>>()
        } else {
            block_qty * std::mem::size_of::<RawBlock<RawItemTileHS>>()
        };

        // The item section starts immediately after the land section
        let item_bytes = &tiledata_file_bytes[land_section_len..land_section_len + item_section_len];

        if tiledata.item_tile_binary_size == ItemTileBinSize::Classic {
            let blocks: &[RawBlock<RawItemTileClassic>] = bytemuck::cast_slice(item_bytes);
            Self::push_tiles_from_blocks(&mut tiledata.item_data, blocks, |tile_id, raw| {
                ItemTile {
                    tile_id,
                    flags: raw.flags,
                    weight: raw.weight,
                    quality: raw.quality,
                    quantity: raw.quantity,
                    anim_id: raw.anim_id,
                    hue_extra: raw.hue_extra,
                    stacking_offset: raw.stacking_offset,
                    value: raw.value,
                    height: raw.height,
                    name: raw.name,
                }
            });
        } else {
            let blocks: &[RawBlock<RawItemTileHS>] = bytemuck::cast_slice(item_bytes);
            Self::push_tiles_from_blocks(&mut tiledata.item_data, blocks, |tile_id, raw| {
                ItemTile {
                    tile_id,
                    flags: raw.flags,
                    weight: raw.weight,
                    quality: raw.quality,
                    quantity: raw.quantity,
                    anim_id: raw.anim_id,
                    hue_extra: raw.hue_extra,
                    stacking_offset: raw.stacking_offset,
                    value: raw.value,
                    height: raw.height,
                    name: raw.name,
                }
            });
        }
        let i_tile = tiledata.item_data.len() as u32;
        log::info!("Loaded {i_tile} (0x{:x}) Item Tiles.", i_tile);

        Ok(tiledata)
    }
}

/* End of Tiledata struct */
