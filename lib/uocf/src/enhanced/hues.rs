use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};

use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::{LoadMode, UopPackage};

pub const HUES_UOP_NAME: &str = "hues.uop";
pub const HUES_ATLAS_PATH: &str = "build/hues/hues.dds";
pub const HUENAMES_PATH: &str = "data/definitions/hues/huenames.csv";
pub const FIXED_PALETTE_HASH: u64 = 0xFA5C_6A1B_C0D8_B01B;
pub const FIXED_PALETTE_NAME: &str = "0xFA5C6A1BC0D8B01B_.dds";
pub const MAX_EC_HUES: u16 = 3000;
pub const HUE_STRIP_WIDTH: u32 = 256;
pub const HUES_ATLAS_HEIGHT: u32 = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcHueBitmapEntry {
    pub hue_id: u16,
    pub path: String,
    pub filename_hash: u64,
    pub byte_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcHuePackage {
    pub atlas_hash: Option<u64>,
    pub atlas_byte_len: Option<u32>,
    pub huenames_hash: Option<u64>,
    pub huenames_byte_len: Option<u32>,
    pub fixed_palette_hash: Option<u64>,
    pub fixed_palette_byte_len: Option<u32>,
    pub names: BTreeMap<u16, String>,
    pub bitmaps: Vec<EcHueBitmapEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcHueAtlasCoord {
    pub column: u32,
    pub row: u32,
    pub x: u32,
    pub y: u32,
}

impl EcHuePackage {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let mut package = UopPackage::load_with_mode(path, LoadMode::Lazy).wrap_err("failed to load hues.uop")?;
        let huenames_hash = huenames_hash();
        if package.get_file_by_hash(huenames_hash).is_some() {
            package
                .ensure_file_data_loaded_by_hash(huenames_hash)
                .wrap_err("failed to load huenames.csv payload")?;
        }
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let atlas_hash = hash_file_name_single(HUES_ATLAS_PATH);
        let huenames_hash = hash_file_name_single(HUENAMES_PATH);

        let atlas_file = package.get_file_by_hash(atlas_hash);
        let huenames_file = package.get_file_by_hash(huenames_hash);
        let fixed_palette_file = package.get_file_by_hash(FIXED_PALETTE_HASH);

        let names = if let Some(file) = huenames_file {
            let bytes = file.unpack().wrap_err("failed to unpack huenames.csv")?;
            parse_huenames_csv(&bytes)
        } else {
            BTreeMap::new()
        };

        let mut bitmaps = Vec::new();
        for hue_id in 1..=MAX_EC_HUES {
            let path = hue_bitmap_path(hue_id);
            let filename_hash = hash_file_name_single(&path);
            if let Some(file) = package.get_file_by_hash(filename_hash) {
                bitmaps.push(EcHueBitmapEntry {
                    hue_id,
                    path,
                    filename_hash,
                    byte_len: file.decompressed_size(),
                });
            }
        }

        Ok(Self {
            atlas_hash: atlas_file.map(|file| file.filename_hash()),
            atlas_byte_len: atlas_file.map(|file| file.decompressed_size()),
            huenames_hash: huenames_file.map(|file| file.filename_hash()),
            huenames_byte_len: huenames_file.map(|file| file.decompressed_size()),
            fixed_palette_hash: fixed_palette_file.map(|file| file.filename_hash()),
            fixed_palette_byte_len: fixed_palette_file.map(|file| file.decompressed_size()),
            names,
            bitmaps,
        })
    }

    pub fn hue_name(&self, hue_id: u16) -> Option<&str> {
        self.names.get(&hue_id).map(String::as_str)
    }

    pub fn bitmap_for_hue(&self, hue_id: u16) -> Option<&EcHueBitmapEntry> {
        self.bitmaps.iter().find(|entry| entry.hue_id == hue_id)
    }
}

pub fn hue_bitmap_path(hue_id: u16) -> String {
    format!("data/definitions/hues/hue{hue_id:04}.bmp")
}

pub fn hue_bitmap_hash(hue_id: u16) -> u64 {
    hash_file_name_single(&hue_bitmap_path(hue_id))
}

pub fn hues_atlas_hash() -> u64 {
    hash_file_name_single(HUES_ATLAS_PATH)
}

pub fn huenames_hash() -> u64 {
    hash_file_name_single(HUENAMES_PATH)
}

pub fn atlas_coord_for_hue(hue_id: u16) -> Option<EcHueAtlasCoord> {
    if hue_id == 0 || hue_id > MAX_EC_HUES {
        return None;
    }

    let index = hue_id as u32;
    let column = if index < HUES_ATLAS_HEIGHT {
        0
    } else {
        1 + ((index - HUES_ATLAS_HEIGHT) / HUES_ATLAS_HEIGHT)
    };
    let row = if column == 0 {
        index
    } else {
        (index - HUES_ATLAS_HEIGHT) % HUES_ATLAS_HEIGHT
    };

    Some(EcHueAtlasCoord {
        column,
        row,
        x: column * HUE_STRIP_WIDTH,
        y: row,
    })
}

pub fn parse_huenames_csv(bytes: &[u8]) -> BTreeMap<u16, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut names = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        let Some((id_text, name)) = line.split_once(',') else {
            continue;
        };
        let Ok(hue_id) = id_text.trim().parse::<u16>() else {
            continue;
        };
        if hue_id == 0 || hue_id > MAX_EC_HUES {
            continue;
        }

        names.insert(hue_id, name.to_string());
    }
    names
}

pub fn decode_hue_image_to_rgba(bytes: &[u8]) -> eyre::Result<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(bytes).wrap_err("failed to decode hue image")?;
    let rgba = image.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_paths_and_hashes_match_expected_names() {
        assert_eq!(hue_bitmap_path(1), "data/definitions/hues/hue0001.bmp");
        assert_eq!(hue_bitmap_path(3000), "data/definitions/hues/hue3000.bmp");
        assert_eq!(hues_atlas_hash(), hash_file_name_single(HUES_ATLAS_PATH));
        assert_eq!(huenames_hash(), hash_file_name_single(HUENAMES_PATH));
        assert_eq!(FIXED_PALETTE_HASH, 0xFA5C_6A1B_C0D8_B01B);
        assert_eq!(hue_bitmap_hash(42), hash_file_name_single("data/definitions/hues/hue0042.bmp"));
    }

    #[test]
    fn atlas_coordinates_follow_converter_layout() {
        assert_eq!(atlas_coord_for_hue(0), None);
        assert_eq!(
            atlas_coord_for_hue(1),
            Some(EcHueAtlasCoord {
                column: 0,
                row: 1,
                x: 0,
                y: 1,
            })
        );
        assert_eq!(
            atlas_coord_for_hue(1023),
            Some(EcHueAtlasCoord {
                column: 0,
                row: 1023,
                x: 0,
                y: 1023,
            })
        );
        assert_eq!(
            atlas_coord_for_hue(1024),
            Some(EcHueAtlasCoord {
                column: 1,
                row: 0,
                x: 256,
                y: 0,
            })
        );
        assert_eq!(
            atlas_coord_for_hue(2048),
            Some(EcHueAtlasCoord {
                column: 2,
                row: 0,
                x: 512,
                y: 0,
            })
        );
        assert_eq!(atlas_coord_for_hue(3001), None);
    }

    #[test]
    fn huenames_csv_parser_accepts_converter_style_rows() {
        let names = parse_huenames_csv(b"1,red\r\n2,\r\nbad row\r\nx,nope\r\n3001,too far\r\n3,blue,with comma\r\n");

        assert_eq!(names.get(&1).map(String::as_str), Some("red"));
        assert_eq!(names.get(&2).map(String::as_str), Some(""));
        assert_eq!(names.get(&3).map(String::as_str), Some("blue,with comma"));
        assert!(!names.contains_key(&3001));
        assert_eq!(names.len(), 3);
    }
}
