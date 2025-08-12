#![allow(dead_code)]

crate::eyre_imports!();
use byteorder::{LittleEndian, ReadBytesExt};
use derive_new::new;
use std::fs::File;
use std::io::{prelude::*, Cursor};
use std::path::PathBuf;

/* Struct to manage Flags for LandTile and ItemTile */

#[derive(Clone, Debug, Default)]
pub struct Flags {
    internal_flags: u32,
}

#[allow(unused)]
impl Flags {
    fn value(&self) -> u32 {
        self.internal_flags
    }

    pub fn background(&self) -> bool {
        0 != (self.internal_flags & 0x01)
    }
    pub fn weapon(&self) -> bool {
        0 != ((self.internal_flags & 0x02) >> 1)
    }
    pub fn transparent(&self) -> bool {
        0 != ((self.internal_flags & 0x04) >> 2)
    }
    pub fn translucent(&self) -> bool {
        0 != ((self.internal_flags & 0x08) >> 3)
    }
    pub fn wall(&self) -> bool {
        0 != ((self.internal_flags & 0x10) >> 4)
    }
    pub fn damaging(&self) -> bool {
        0 != ((self.internal_flags & 0x20) >> 5)
    }
    pub fn impassable(&self) -> bool {
        0 != ((self.internal_flags & 0x40) >> 6)
    }
    pub fn wet(&self) -> bool {
        0 != ((self.internal_flags & 0x80) >> 7)
    }
    /*pub fn unknown(&self) -> bool {
        0 != ((self.internal_flags & 0x100) >> 8)
    }*/
    pub fn surface(&self) -> bool {
        0 != ((self.internal_flags & 0x200) >> 9)
    }
    pub fn bridge(&self) -> bool {
        0 != ((self.internal_flags & 0x400) >> 10)
    }
    pub fn generic(&self) -> bool {
        0 != ((self.internal_flags & 0x800) >> 11)
    }
    pub fn stackable(&self) -> bool {
        self.generic()
    }
    pub fn window(&self) -> bool {
        0 != ((self.internal_flags & 0x1000) >> 12)
    }
    pub fn noshoot(&self) -> bool {
        0 != ((self.internal_flags & 0x2000) >> 13)
    }
    pub fn prefixa(&self) -> bool {
        0 != ((self.internal_flags & 0x4000) >> 14)
    }
    pub fn prefixan(&self) -> bool {
        0 != ((self.internal_flags & 0x8000) >> 15)
    }
    pub fn internal(&self) -> bool {
        0 != ((self.internal_flags & 0x10000) >> 16)
    }
    pub fn foliage(&self) -> bool {
        0 != ((self.internal_flags & 0x20000) >> 17)
    }
    pub fn partialhue(&self) -> bool {
        0 != ((self.internal_flags & 0x40000) >> 18)
    }
    /*pub fn unknown1(&self) -> bool {
        0 != ((self.internal_flags & 0x80000) >> 19)
    }*/
    pub fn map(&self) -> bool {
        0 != ((self.internal_flags & 0x100000) >> 20)
    }
    pub fn container(&self) -> bool {
        0 != ((self.internal_flags & 0x200000) >> 21)
    }
    pub fn wearable(&self) -> bool {
        0 != ((self.internal_flags & 0x400000) >> 22)
    }
    pub fn lightsource(&self) -> bool {
        0 != ((self.internal_flags & 0x800000) >> 23)
    }
    pub fn animated(&self) -> bool {
        0 != ((self.internal_flags & 0x1000000) >> 24)
    }
    pub fn nodiagonal(&self) -> bool {
        0 != ((self.internal_flags & 0x2000000) >> 25)
    }
    /*pub fn unknown2(&self) -> bool {
        0 != ((self.internal_flags & 0x4000000) >> 26)
    }*/
    pub fn armor(&self) -> bool {
        0 != ((self.internal_flags & 0x8000000) >> 27)
    }
    pub fn roof(&self) -> bool {
        0 != ((self.internal_flags & 0x10000000) >> 28)
    }
    pub fn door(&self) -> bool {
        0 != ((self.internal_flags & 0x20000000) >> 29)
    }
    pub fn stairback(&self) -> bool {
        0 != ((self.internal_flags & 0x40000000) >> 30)
    }
    pub fn stairright(&self) -> bool {
        0 != ((self.internal_flags & 0x80000000) >> 31)
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
    height: i8, // Stratics: If Conatainer, this is how much the container can hold

    #[new(default)]
    name: [u8; Self::NAME_LEN],
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
    fn height_raw(&self) -> i8 {
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
            land_data: vec![LandTile::default(); TileData::LAND_TILE_MAX],
            item_data: vec![],
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
        tiledata.item_data = vec![ItemTile::default(); 1 + tiledata.max_item_rev as usize];

        println!(
            "Found Tiledata with size: {file_size}. \n\
        Detected LandTile size: {:?}, ItemTile size: {:?}, Max Item count: {:?} (0x{:X})",
            tiledata.land_tile_binary_size,
            tiledata.item_tile_binary_size,
            tiledata.max_item_rev,
            tiledata.max_item_rev.clone() as u32
        );

        let mut tiledata_file_rdr = {
            let mut buf = vec![0; file_size as usize];
            file_handle
                .read_exact(buf.as_mut())
                .wrap_err("Read tiledata.mul")?;
            Cursor::new(buf)
        };

        let mut err_buf;

        // Read LandTiles
        let mut i_tile: u32 = 0;
        for _i_land_block in 0..LandTile::BLOCK_QTY {
            err_buf = format!(
                "Reading tiledata info for item tile {i_tile} (0x{:x}): reading ",
                i_tile
            );

            let _header = tiledata_file_rdr
                .read_u32::<LittleEndian>()
                .wrap_err(err_buf.clone() + "header")?;

            for _i_tile_in_block in 0..LandTile::TILES_PER_BLOCK {
                let land_tile = &mut tiledata.land_data[i_tile as usize];
                land_tile.tile_id = i_tile as i32;

                land_tile.flags.internal_flags = tiledata_file_rdr
                    .read_u32::<LittleEndian>()
                    .wrap_err(err_buf.clone() + "flags")?;

                if tiledata.land_tile_binary_size == LandTileBinSize::HS {
                    let _unk = tiledata_file_rdr
                        .read_i32::<LittleEndian>()
                        .wrap_err(err_buf.clone() + "unk field")?;
                }

                land_tile.texture_id = tiledata_file_rdr
                    .read_u16::<LittleEndian>()
                    .wrap_err(err_buf.clone() + "texture id")?;

                tiledata_file_rdr
                    .read_exact(&mut land_tile.name)
                    .wrap_err(err_buf.clone() + "name")?;

                i_tile = i_tile.saturating_add(1);
            }
        }
        println!("Loaded {i_tile} (0x{:x}) LandTiles.", i_tile);

        // Read ItemTiles
        i_tile = 0_u32;
        let block_qty: usize = (1 + tiledata.max_item_rev as usize) / ItemTile::TILES_PER_BLOCK;
        for _i_item_block in 0..block_qty as u32 {
            err_buf = format!(
                "Reading tiledata info for item tile {i_tile} (0x{:x}): reading ",
                i_tile
            );

            let _header = tiledata_file_rdr
                .read_u32::<LittleEndian>()
                .wrap_err(err_buf.clone() + "header")?;

            for _i_tile_in_block in 0..ItemTile::TILES_PER_BLOCK {
                let item_tile = &mut tiledata.item_data[i_tile as usize];
                item_tile.tile_id = i_tile as i32;

                item_tile.flags.internal_flags = tiledata_file_rdr
                    .read_u32::<LittleEndian>()
                    .wrap_err(err_buf.clone() + "flags")?;

                if tiledata.item_tile_binary_size == ItemTileBinSize::HS {
                    let _unk = tiledata_file_rdr
                        .read_i32::<LittleEndian>()
                        .wrap_err(err_buf.clone() + "unk field HS")?;
                }

                item_tile.weight = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "weight")?;

                item_tile.quality = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "quality")?;

                let _unk0 = tiledata_file_rdr
                    .read_u16::<LittleEndian>()
                    .wrap_err(err_buf.clone() + "unk field 0")?;

                let _unk1 = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "unk field 1")?;

                item_tile.quantity = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "weight")?;

                item_tile.anim_id = tiledata_file_rdr
                    .read_u16::<LittleEndian>()
                    .wrap_err(err_buf.clone() + "anim id")?;

                let _unk2 = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "unk field 2")?;

                item_tile.hue_extra = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "hue extra")?;

                item_tile.stacking_offset = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "stacking offset")?;

                item_tile.value = tiledata_file_rdr
                    .read_u8()
                    .wrap_err(err_buf.clone() + "value")?;

                item_tile.height = tiledata_file_rdr
                    .read_i8()
                    .wrap_err(err_buf.clone() + "height")?;

                tiledata_file_rdr
                    .read_exact(&mut item_tile.name)
                    .wrap_err(err_buf.clone() + "name")?;

                i_tile = i_tile.saturating_add(1);
            }
        }
        println!("Loaded {i_tile} (0x{:x}) Item Tiles.", i_tile);

        assert_eq!(
            tiledata_file_rdr.get_ref().len() as u64,
            tiledata_file_rdr.position()
        ); // Consumed the whole file

        Ok(tiledata)
    }
}

/* End of Tiledata struct */
