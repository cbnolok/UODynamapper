//! Build-time and runtime support for `cc_tiledata.uddp`.
//!
//! Package layout:
//! - `metadata/land.bin`: dense table for land tiledata entries.
//! - `metadata/items.bin`: dense table for item tiledata entries.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use color_eyre::eyre::{self, WrapErr};

use uocf::{
    classic::tiledata::{ItemTile, LandTile, TileData},
    udd::{
        xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
        LookupMode, UddpBuilder, UddpReader,
    },
};

const LAND_MAGIC: [u8; 4] = *b"CTDL";
const ITEM_MAGIC: [u8; 4] = *b"CTDI";
const CCTILEDATA_VERSION: u32 = 1;
const LAND_ENTRY_PATH: &str = "metadata/land.bin";
const ITEM_ENTRY_PATH: &str = "metadata/items.bin";
const NAME_LEN: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcTileDataBuildSummary {
    pub land_tile_count: u32,
    pub item_tile_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcLandTileRecord {
    pub tile_id: u32,
    pub flags: u32,
    pub texture_id: u16,
    pub name: [u8; NAME_LEN],
}

impl CcLandTileRecord {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(NAME_LEN);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcItemTileRecord {
    pub tile_id: u32,
    pub flags: u32,
    pub weight: u8,
    pub quality: u8,
    pub quantity: u8,
    pub anim_id: u16,
    pub hue_extra: u8,
    pub stacking_offset: u8,
    pub value: u8,
    pub height: i8,
    pub name: [u8; NAME_LEN],
}

impl CcItemTileRecord {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(NAME_LEN);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

pub struct CcTileDataPackage {
    package: UddpReader,
    land_tiles: Vec<CcLandTileRecord>,
    item_tiles: Vec<CcItemTileRecord>,
}

impl CcTileDataPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::open(fs::read(path.as_ref())?)
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let land_tiles = parse_land_tiles(&read_path_entry(&package, LAND_ENTRY_PATH)?)?;
        let item_tiles = parse_item_tiles(&read_path_entry(&package, ITEM_ENTRY_PATH)?)?;

        Ok(Self {
            package,
            land_tiles,
            item_tiles,
        })
    }

    pub fn package(&self) -> &UddpReader {
        &self.package
    }

    pub fn land_tiles(&self) -> &[CcLandTileRecord] {
        &self.land_tiles
    }

    pub fn item_tiles(&self) -> &[CcItemTileRecord] {
        &self.item_tiles
    }

    pub fn land_tile(&self, tile_id: u32) -> Option<&CcLandTileRecord> {
        self.land_tiles.get(tile_id as usize)
    }

    pub fn item_tile(&self, tile_id: u32) -> Option<&CcItemTileRecord> {
        self.item_tiles.get(tile_id as usize)
    }
}

pub fn convert_tiledata_mul_to_cc_tiledata_uddp(
    client_dir: &Path,
    out_file: &Path,
) -> eyre::Result<CcTileDataBuildSummary> {
    let tiledata_path = client_dir.join("tiledata.mul");
    if !tiledata_path.is_file() {
        eyre::bail!("missing required file: {}", tiledata_path.display());
    }

    let tiledata = TileData::load(tiledata_path.clone())
        .wrap_err_with(|| format!("load {}", tiledata_path.display()))?;
    let land_bytes = serialize_land_tiles(tiledata.land_tiles())?;
    let item_bytes = serialize_item_tiles(tiledata.item_tiles())?;

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(LAND_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &land_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(ITEM_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: &item_bytes,
    })?;
    fs::write(out_file, package.build()?)
        .wrap_err_with(|| format!("save {}", out_file.display()))?;

    Ok(CcTileDataBuildSummary {
        land_tile_count: tiledata.land_tiles().len() as u32,
        item_tile_count: tiledata.item_tiles().len() as u32,
    })
}

fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("unpack {path}"))
}

fn serialize_land_tiles(tiles: &[LandTile]) -> eyre::Result<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::with_capacity(12 + tiles.len() * 30);
    bytes.extend_from_slice(&LAND_MAGIC);
    bytes.write_u32::<LittleEndian>(CCTILEDATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(tiles.len() as u32)?;
    for tile in tiles {
        bytes.write_u32::<LittleEndian>(tile.tile_id as u32)?;
        bytes.write_u32::<LittleEndian>(tile.flags.internal_flags)?;
        bytes.write_u16::<LittleEndian>(tile.texture_id)?;
        bytes.extend_from_slice(&tile.name);
    }
    Ok(bytes)
}

fn serialize_item_tiles(tiles: &[ItemTile]) -> eyre::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(12 + tiles.len() * 37);
    bytes.extend_from_slice(&ITEM_MAGIC);
    bytes.write_u32::<LittleEndian>(CCTILEDATA_VERSION)?;
    bytes.write_u32::<LittleEndian>(tiles.len() as u32)?;
    for tile in tiles {
        bytes.write_u32::<LittleEndian>(tile.tile_id as u32)?;
        bytes.write_u32::<LittleEndian>(tile.flags.internal_flags)?;
        bytes.write_u8(tile.weight)?;
        bytes.write_u8(tile.quality)?;
        bytes.write_u8(tile.quantity)?;
        bytes.write_u16::<LittleEndian>(tile.anim_id)?;
        bytes.write_u8(tile.hue_extra)?;
        bytes.write_u8(tile.stacking_offset)?;
        bytes.write_u8(tile.value)?;
        bytes.write_i8(tile.height_raw())?;
        bytes.extend_from_slice(&tile.name);
    }
    Ok(bytes)
}

fn parse_land_tiles(bytes: &[u8]) -> eyre::Result<Vec<CcLandTileRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != LAND_MAGIC {
        eyre::bail!("invalid cc_tiledata land magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CCTILEDATA_VERSION {
        eyre::bail!("unsupported cc_tiledata land version {version}");
    }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut tiles: Vec<CcLandTileRecord> = Vec::with_capacity(count);
    for _ in 0..count {
        let tile_id = cursor.read_u32::<LittleEndian>()?;
        let flags = cursor.read_u32::<LittleEndian>()?;
        let texture_id = cursor.read_u16::<LittleEndian>()?;
        let mut name = [0u8; NAME_LEN];
        cursor.read_exact(&mut name)?;
        tiles.push(CcLandTileRecord {
            tile_id,
            flags,
            texture_id,
            name,
        });
    }
    Ok(tiles)
}

fn parse_item_tiles(bytes: &[u8]) -> eyre::Result<Vec<CcItemTileRecord>> {
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;
    if magic != ITEM_MAGIC {
        eyre::bail!("invalid cc_tiledata item magic");
    }
    let version = cursor.read_u32::<LittleEndian>()?;
    if version != CCTILEDATA_VERSION {
        eyre::bail!("unsupported cc_tiledata item version {version}");
    }
    let count = cursor.read_u32::<LittleEndian>()? as usize;
    let mut tiles: Vec<CcItemTileRecord> = Vec::with_capacity(count);
    for _ in 0..count {
        let tile_id = cursor.read_u32::<LittleEndian>()?;
        let flags = cursor.read_u32::<LittleEndian>()?;
        let weight = cursor.read_u8()?;
        let quality = cursor.read_u8()?;
        let quantity = cursor.read_u8()?;
        let anim_id = cursor.read_u16::<LittleEndian>()?;
        let hue_extra = cursor.read_u8()?;
        let stacking_offset = cursor.read_u8()?;
        let value = cursor.read_u8()?;
        let height = cursor.read_i8()?;
        let mut name = [0u8; NAME_LEN];
        cursor.read_exact(&mut name)?;
        tiles.push(CcItemTileRecord {
            tile_id,
            flags,
            weight,
            quality,
            quantity,
            anim_id,
            hue_extra,
            stacking_offset,
            value,
            height,
            name,
        });
    }
    Ok(tiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn land_roundtrip_keeps_names_and_flags() {
        let tiles = vec![CcLandTileRecord {
            tile_id: 7,
            flags: 0x1234,
            texture_id: 42,
            name: {
                let mut name = [0u8; NAME_LEN];
                name[..4].copy_from_slice(b"sand");
                name
            },
        }];

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&LAND_MAGIC);
        bytes.write_u32::<LittleEndian>(CCTILEDATA_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(tiles[0].tile_id).unwrap();
        bytes.write_u32::<LittleEndian>(tiles[0].flags).unwrap();
        bytes.write_u16::<LittleEndian>(tiles[0].texture_id).unwrap();
        bytes.extend_from_slice(&tiles[0].name);

        let parsed: Vec<CcLandTileRecord> = parse_land_tiles(&bytes).unwrap();
        assert_eq!(parsed, tiles);
        assert_eq!(parsed[0].name_ascii(), "sand");
    }

    #[test]
    fn item_roundtrip_keeps_core_fields() {
        let mut name = [0u8; NAME_LEN];
        name[..5].copy_from_slice(b"chair");

        let record = CcItemTileRecord {
            tile_id: 12,
            flags: 0xABCD,
            weight: 1,
            quality: 2,
            quantity: 3,
            anim_id: 4,
            hue_extra: 5,
            stacking_offset: 6,
            value: 7,
            height: 8,
            name,
        };

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&ITEM_MAGIC);
        bytes.write_u32::<LittleEndian>(CCTILEDATA_VERSION).unwrap();
        bytes.write_u32::<LittleEndian>(1).unwrap();
        bytes.write_u32::<LittleEndian>(record.tile_id).unwrap();
        bytes.write_u32::<LittleEndian>(record.flags).unwrap();
        bytes.write_u8(record.weight).unwrap();
        bytes.write_u8(record.quality).unwrap();
        bytes.write_u8(record.quantity).unwrap();
        bytes.write_u16::<LittleEndian>(record.anim_id).unwrap();
        bytes.write_u8(record.hue_extra).unwrap();
        bytes.write_u8(record.stacking_offset).unwrap();
        bytes.write_u8(record.value).unwrap();
        bytes.write_i8(record.height).unwrap();
        bytes.extend_from_slice(&record.name);

        let parsed: Vec<CcItemTileRecord> = parse_item_tiles(&bytes).unwrap();
        assert_eq!(parsed, vec![record]);
        assert_eq!(parsed[0].name_ascii(), "chair");
    }
}
