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
    if let Some(decoded) = decode_ec_hue_bmp32_to_rgba(bytes) {
        return Ok(decoded);
    }

    let image = image::load_from_memory(bytes).wrap_err("failed to decode hue image")?;
    let rgba = image.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn decode_ec_hue_bmp32_to_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return None;
    }

    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes = bytes.get(offset..offset + 2)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes = bytes.get(offset..offset + 4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let read_i32 = |offset: usize| -> Option<i32> {
        let bytes = bytes.get(offset..offset + 4)?;
        Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };

    let data_offset = read_u32(10)? as usize;
    let dib_header_size = read_u32(14)?;
    if dib_header_size < 40 {
        return None;
    }
    let width = read_i32(18)?;
    let height = read_i32(22)?;
    let planes = read_u16(26)?;
    let bits_per_pixel = read_u16(28)?;
    let compression = read_u32(30)?;

    if width <= 0 || height == 0 || planes != 1 || bits_per_pixel != 32 || compression != 0 {
        return None;
    }

    let width = width as u32;
    let height_abs = height.unsigned_abs();
    let row_bytes = width as usize * 4;
    let data_bytes = row_bytes.checked_mul(height_abs as usize)?;
    if bytes.len() < data_offset.checked_add(data_bytes)? {
        return None;
    }

    let mut rgba = vec![0u8; data_bytes];
    for y in 0..height_abs as usize {
        let src_y = if height > 0 {
            height_abs as usize - 1 - y
        } else {
            y
        };
        let src_row = data_offset + src_y * row_bytes;
        let dst_row = y * row_bytes;
        for x in 0..width as usize {
            let src = src_row + x * 4;
            let dst = dst_row + x * 4;
            rgba[dst] = bytes[src + 2];
            rgba[dst + 1] = bytes[src + 1];
            rgba[dst + 2] = bytes[src];
            rgba[dst + 3] = bytes[src + 3];
        }
    }

    Some((width, height_abs, rgba))
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

    #[test]
    fn ec_hue_bmp_decoder_ignores_client_size_field() {
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&13u32.to_le_bytes());
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&2i32.to_le_bytes());
        bytes[22..26].copy_from_slice(&2i32.to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&32u16.to_le_bytes());
        bytes[30..34].copy_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[
            1, 2, 3, 4,
            5, 6, 7, 8,
            9, 10, 11, 12,
            13, 14, 15, 16,
        ]);

        let (width, height, rgba) = decode_hue_image_to_rgba(&bytes).expect("decode EC hue BMP");

        assert_eq!((width, height), (2, 2));
        assert_eq!(
            rgba,
            vec![
                11, 10, 9, 12,
                15, 14, 13, 16,
                3, 2, 1, 4,
                7, 6, 5, 8,
            ]
        );
    }
}
