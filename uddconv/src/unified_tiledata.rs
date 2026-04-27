//! Build-time and runtime support for `unified_tiledata.uddp`.
//!
//! Package layout:
//! - `metadata/land.bin`: dense table for `UnifiedLandTile` entries.
//! - `metadata/items.bin`: dense table for `UnifiedItemTile` entries.

use bytemuck::{Pod, Zeroable};
use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use log::info;

use crate::package_progress::build_and_write_package;
use uocf::classic::tiledata::TileData;
use uocf::enhanced::tile_database::ArtDefinition;
use uocf::udd::{
    xxh64_virtual_path, AddFileRequest, CompressionFlag as UddCompressionFlag, DataType,
    LookupMode, UddpBuilder, UddpReader,
};
use crate::source_paths::find_first_existing_file;

pub const UNIFIED_LAND_ENTRY_PATH: &str = "metadata/land.bin";
pub const UNIFIED_ITEM_ENTRY_PATH: &str = "metadata/items.bin";

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct UnifiedLandTile {
    pub tile_id: u32,
    pub texture_id: u16,
    pub tile_type: u8,
    pub _pad1: u8,
    pub flags: u64,
    pub radar_color: [u8; 4],
    pub name: [u8; 20],
}

impl UnifiedLandTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct UnifiedItemTile {
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
} // 48 + 4+2+2+2+2 (12) + 12 = 72 bytes. Aligned to 8.

impl UnifiedItemTile {
    pub fn name_ascii(&self) -> &str {
        let null_pos = self.name.iter().position(|&byte| byte == 0).unwrap_or(20);
        std::str::from_utf8(&self.name[..null_pos]).unwrap_or("")
    }
}

pub struct UnifiedTileDataPackage {
    #[allow(dead_code)]
    package: UddpReader,
    land_tiles: Vec<UnifiedLandTile>,
    item_tiles: Vec<UnifiedItemTile>,
}

impl UnifiedTileDataPackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UddpReader::open(fs::read(path.as_ref())?)
            .wrap_err_with(|| format!("load {}", path.as_ref().display()))?;
        Self::from_uddp_package(package)
    }

    pub fn from_uddp_package(package: UddpReader) -> eyre::Result<Self> {
        let land_bytes = read_path_entry(&package, UNIFIED_LAND_ENTRY_PATH)?;
        let item_bytes = read_path_entry(&package, UNIFIED_ITEM_ENTRY_PATH)?;

        Ok(Self {
            package,
            land_tiles: read_pod_vec(&land_bytes, UNIFIED_LAND_ENTRY_PATH)?,
            item_tiles: read_pod_vec(&item_bytes, UNIFIED_ITEM_ENTRY_PATH)?,
        })
    }

    pub fn land_tiles(&self) -> &[UnifiedLandTile] {
        &self.land_tiles
    }

    pub fn item_tiles(&self) -> &[UnifiedItemTile] {
        &self.item_tiles
    }

    pub fn land_tile(&self, tile_id: u32) -> Option<&UnifiedLandTile> {
        self.land_tiles.get(tile_id as usize)
    }

    pub fn item_tile(&self, tile_id: u32) -> Option<&UnifiedItemTile> {
        self.item_tiles.get(tile_id as usize)
    }
}

fn read_pod_vec<T: Pod>(bytes: &[u8], entry_path: &str) -> eyre::Result<Vec<T>> {
    let record_size = std::mem::size_of::<T>();
    if bytes.len() % record_size != 0 {
        eyre::bail!(
            "{} size {} is not a multiple of record size {}",
            entry_path,
            bytes.len(),
            record_size,
        );
    }

    Ok(bytes
        .chunks_exact(record_size)
        .map(bytemuck::pod_read_unaligned)
        .collect())
}

fn read_path_entry(package: &UddpReader, path: &str) -> eyre::Result<Vec<u8>> {
    package
        .read_file_by_path_hash(xxh64_virtual_path(path))
        .wrap_err_with(|| format!("unpack {path}"))
}

fn find_string_dictionary_path(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    find_first_existing_file(
        source_dirs,
        &["string_dictionary.uop", "string_Wdictionary.uop"],
    )
}

pub fn build_unified_tiledata_uddp(client_dir: &Path, out_file: &Path) -> eyre::Result<()> {
    build_unified_tiledata_uddp_from_sources(&[client_dir.to_path_buf()], out_file)
}

pub fn build_unified_tiledata_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
) -> eyre::Result<()> {
    let tiledata_path = find_first_existing_file(source_dirs, &["tiledata.mul"])
        .ok_or_else(|| eyre::eyre!("missing tiledata.mul"))?;
    let tileart_path = find_first_existing_file(source_dirs, &["tileart.uop"])
        .ok_or_else(|| eyre::eyre!("missing tileart.uop"))?;
    let stringdict_path = find_string_dictionary_path(source_dirs)
        .ok_or_else(|| eyre::eyre!("missing string_dictionary.uop or string_Wdictionary.uop"))?;

    println!("Using tiledata.mul: {}", tiledata_path.display());
    println!("Using tileart.uop: {}", tileart_path.display());
    println!("Using string dictionary: {}", stringdict_path.display());

    info!(
        "Converting Unified TileData from MUL/UOP sources to {}",
        out_file.display()
    );

    let cc_tiledata = TileData::load(tiledata_path.clone())?;
    let ec_art = ArtDefinition::load(&tileart_path, &stringdict_path)?;

    let mut unified_land = Vec::with_capacity(cc_tiledata.land_tiles().len());
    let pb =
        ProgressBar::new((cc_tiledata.land_tiles().len() + cc_tiledata.item_tiles().len()) as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} unifying tiledata ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    for tile in cc_tiledata.land_tiles() {
        pb.inc(1);
        // Land tiles in EC are not mapped via tileart.uop directly in the same way,
        // so for now we map their baseline properties.
        unified_land.push(UnifiedLandTile {
            tile_id: tile.tile_id as u32,
            texture_id: tile.texture_id,
            tile_type: 0,
            _pad1: 0,
            flags: map_cc_flags_to_unified(tile.flags.internal_flags),
            radar_color: [0, 0, 0, 0],
            name: tile.name,
        });
    }

    let mut unified_items = Vec::with_capacity(cc_tiledata.item_tiles().len());
    for tile in cc_tiledata.item_tiles() {
        pb.inc(1);
        let mut u_item = UnifiedItemTile {
            tile_id: tile.tile_id as u32,
            weight: tile.weight,
            quality: tile.quality,
            quantity: tile.quantity,
            hue_extra: tile.hue_extra,
            flags: map_cc_flags_to_unified(tile.flags.internal_flags),
            anim_id: tile.anim_id,
            stacking_offset: tile.stacking_offset,
            value: tile.value,
            height: tile.height_raw(),
            _pad1: 0,
            _pad2: 0,
            radar_color: [0, 0, 0, 0],
            name: tile.name,
            ec_texture_id: 0,
            ec_start_x: 0,
            ec_start_y: 0,
            ec_offset_x: 0,
            ec_offset_y: 0,
            cc_texture_id: 0,
            cc_start_x: 0,
            cc_start_y: 0,
            cc_offset_x: 0,
            cc_offset_y: 0,
        };

        if let Some(ec_data) = ec_art.definitions.get(&(tile.tile_id as u16)) {
            u_item.flags |= ec_data.flags.bits();

            // Unify radar color
            u_item.radar_color = [
                ec_data.radar_color.b,
                ec_data.radar_color.g,
                ec_data.radar_color.r,
                ec_data.radar_color.a,
            ];

            // Unify EC Texture
            if let Some(ec_tex) = &ec_data.ec_texture {
                u_item.ec_texture_id = ec_tex.texture_id;
                u_item.ec_start_x = ec_tex.start_x as i16;
                u_item.ec_start_y = ec_tex.start_y as i16;
                u_item.ec_offset_x = ec_tex.offset_x as i16;
                u_item.ec_offset_y = ec_tex.offset_y as i16;
            }

            // Unify CC Texture override
            if let Some(cc_tex) = &ec_data.cc_texture {
                u_item.cc_texture_id = cc_tex.texture_id;
                u_item.cc_start_x = cc_tex.start_x as i16;
                u_item.cc_start_y = cc_tex.start_y as i16;
                u_item.cc_offset_x = cc_tex.offset_x as i16;
                u_item.cc_offset_y = cc_tex.offset_y as i16;
            }
        }
        unified_items.push(u_item);
    }
    pb.finish_with_message("TileData unified");

    let land_bytes = bytemuck::cast_slice(&unified_land);
    let item_bytes = bytemuck::cast_slice(&unified_items);

    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(UNIFIED_LAND_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: land_bytes,
    })?;
    package.add_file(AddFileRequest {
        data_type: DataType::Metadata as u8,
        compression: UddCompressionFlag::ZstdNoDict,
        virtual_path: Some(UNIFIED_ITEM_ENTRY_PATH),
        path_hash64: None,
        id: None,
        data: item_bytes,
    })?;
    build_and_write_package(&mut package, out_file)?;

    Ok(())
}

/// Explicitly translates Classic Client 32-bit flags into the Enhanced Client 64-bit flag space.
/// While historically many of the lower 32-bits share the same integer values across clients,
/// explicit mapping ensures no assumptions are made and allows divergent flags to be mapped
/// correctly into a single dictionary.
fn map_cc_flags_to_unified(cc: u32) -> u64 {
    let mut ec = 0u64;

    if (cc & 0x00000001) != 0 {
        ec |= 0x1;
    } // Background
    if (cc & 0x00000002) != 0 {
        ec |= 0x2;
    } // Weapon
    if (cc & 0x00000004) != 0 {
        ec |= 0x4;
    } // Transparent
    if (cc & 0x00000008) != 0 {
        ec |= 0x8;
    } // Translucent
    if (cc & 0x00000010) != 0 {
        ec |= 0x10;
    } // Wall
    if (cc & 0x00000020) != 0 {
        ec |= 0x20;
    } // Damaging
    if (cc & 0x00000040) != 0 {
        ec |= 0x40;
    } // Impassable
    if (cc & 0x00000080) != 0 {
        ec |= 0x80;
    } // Wet
    if (cc & 0x00000100) != 0 {
        ec |= 0x100;
    } // CC: Unknown -> EC: Ignored
    if (cc & 0x00000200) != 0 {
        ec |= 0x200;
    } // Surface
    if (cc & 0x00000400) != 0 {
        ec |= 0x400;
    } // Bridge
    if (cc & 0x00000800) != 0 {
        ec |= 0x800;
    } // Generic / Stackable
    if (cc & 0x00001000) != 0 {
        ec |= 0x1000;
    } // Window
    if (cc & 0x00002000) != 0 {
        ec |= 0x2000;
    } // NoShoot
    if (cc & 0x00004000) != 0 {
        ec |= 0x4000;
    } // ArticleA
    if (cc & 0x00008000) != 0 {
        ec |= 0x8000;
    } // ArticleAn
    if (cc & 0x00010000) != 0 {
        ec |= 0x10000;
    } // Internal / Mongen
    if (cc & 0x00020000) != 0 {
        ec |= 0x20000;
    } // Foliage
    if (cc & 0x00040000) != 0 {
        ec |= 0x40000;
    } // PartialHue
    if (cc & 0x00080000) != 0 {
        ec |= 0x80000;
    } // CC: Unknown1 -> EC: UseNewArt
    if (cc & 0x00100000) != 0 {
        ec |= 0x100000;
    } // Map
    if (cc & 0x00200000) != 0 {
        ec |= 0x200000;
    } // Container
    if (cc & 0x00400000) != 0 {
        ec |= 0x400000;
    } // Wearable
    if (cc & 0x00800000) != 0 {
        ec |= 0x800000;
    } // LightSource
    if (cc & 0x01000000) != 0 {
        ec |= 0x1000000;
    } // Animated
    if (cc & 0x02000000) != 0 {
        ec |= 0x2000000;
    } // NoDiagonal / HoverOver
    if (cc & 0x04000000) != 0 {
        ec |= 0x4000000;
    } // CC: Unknown2 -> EC: ArtUsed
    if (cc & 0x08000000) != 0 {
        ec |= 0x8000000;
    } // Armor
    if (cc & 0x10000000) != 0 {
        ec |= 0x10000000;
    } // Roof
    if (cc & 0x20000000) != 0 {
        ec |= 0x20000000;
    } // Door
    if (cc & 0x40000000) != 0 {
        ec |= 0x40000000;
    } // StairBack
    if (cc & 0x80000000) != 0 {
        ec |= 0x80000000;
    } // StairRight

    ec
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_dir(test_name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("uddconv_{test_name}_{timestamp}"))
    }

    #[test]
    fn unified_tiledata_prefers_primary_string_dictionary_name() {
        let dir = temp_dir("string_dictionary_primary");
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(dir.join("string_dictionary.uop"), []).expect("write primary dict marker");
        fs::write(dir.join("string_Wdictionary.uop"), []).expect("write fallback dict marker");

        let found = find_string_dictionary_path(std::slice::from_ref(&dir)).expect("find dictionary file");

        assert_eq!(found, dir.join("string_dictionary.uop"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unified_tiledata_accepts_legacy_string_dictionary_name() {
        let dir = temp_dir("string_dictionary_legacy");
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(dir.join("string_Wdictionary.uop"), []).expect("write legacy dict marker");

        let found = find_string_dictionary_path(std::slice::from_ref(&dir)).expect("find legacy dictionary file");

        assert_eq!(found, dir.join("string_Wdictionary.uop"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unified_tiledata_package_roundtrip_preserves_metadata_records() {
        let mut land_tiles = vec![UnifiedLandTile::zeroed(); 8];
        land_tiles[7] = UnifiedLandTile {
            tile_id: 7,
            texture_id: 42,
            tile_type: 2,
            _pad1: 0,
            flags: 0x1234,
            radar_color: [1, 2, 3, 4],
            name: {
                let mut name = [0u8; 20];
                name[..4].copy_from_slice(b"sand");
                name
            },
        };
        let mut item_tiles = vec![UnifiedItemTile::zeroed(); 12];
        item_tiles[11] = UnifiedItemTile {
            tile_id: 11,
            weight: 1,
            quality: 2,
            quantity: 3,
            hue_extra: 4,
            flags: 0xABCD,
            anim_id: 5,
            stacking_offset: 6,
            value: 7,
            height: 8,
            _pad1: 0,
            _pad2: 0,
            radar_color: [9, 10, 11, 12],
            name: {
                let mut name = [0u8; 20];
                name[..5].copy_from_slice(b"chair");
                name
            },
            ec_texture_id: 100,
            ec_start_x: 101,
            ec_start_y: 102,
            ec_offset_x: 103,
            ec_offset_y: 104,
            cc_texture_id: 200,
            cc_start_x: 201,
            cc_start_y: 202,
            cc_offset_x: 203,
            cc_offset_y: 204,
        };

        let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: UddCompressionFlag::ZstdNoDict,
                virtual_path: Some(UNIFIED_LAND_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: bytemuck::cast_slice(&land_tiles),
            })
            .expect("add land metadata");
        package
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: UddCompressionFlag::ZstdNoDict,
                virtual_path: Some(UNIFIED_ITEM_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: bytemuck::cast_slice(&item_tiles),
            })
            .expect("add item metadata");

        let package = UnifiedTileDataPackage::from_uddp_package(
            UddpReader::open(package.build().expect("build package")).expect("open package"),
        )
        .expect("read unified tiledata package");

        assert_eq!(package.land_tiles().len(), 8);
        assert_eq!(package.item_tiles().len(), 12);
        assert_eq!(package.land_tile(7).expect("land tile").name_ascii(), "sand");
        let item = package.item_tile(11).expect("item tile");
        assert_eq!(item.name_ascii(), "chair");
        assert_eq!(item.ec_texture_id, 100);
        assert_eq!(item.cc_texture_id, 200);
    }
}
