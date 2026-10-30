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

/// Entries of one hue table; the table is indexed by the red byte of a hueable colour.
pub const HUE_TABLE_ENTRIES: usize = 256;
/// `hue%04d.bmp` must be a 32 bpp bitmap of this size (`UOHues::LoadHueBitmap`).
pub const HUE_BITMAP_WIDTH: u32 = HUE_TABLE_ENTRIES as u32;
pub const HUE_BITMAP_HEIGHT: u32 = 2;
/// The table is the top image row (the second stored row of the bottom-up bitmap).
pub const HUE_TABLE_IMAGE_ROW: usize = 0;
/// Classic "partial hue" bit. `UOHues::GetHueTable` masks it away: in the EC, partial hueing is the
/// per-colour "keep colour" flag of the animation palette.
pub const HUE_PARTIAL_FLAG: u16 = 0x8000;
/// Hues with this bit and without `HUE_PARTIAL_FLAG` have no table (except `SHADOW_HUE`).
pub const HUE_NO_TABLE_FLAG: u16 = 0x4000;
/// Hue number bits kept when `HUE_PARTIAL_FLAG` is set.
pub const HUE_NUMBER_MASK: u16 = 0x3fff;
/// Translucent dark silhouette hue: every colour becomes black with alpha from its luminance.
pub const SHADOW_HUE: u16 = 0x4001;
/// Table key `GetHueTable` returns for `SHADOW_HUE`.
pub const SHADOW_HUE_TABLE_KEY: u16 = 0x9c3;
/// Hue the client draws ghost bodies with.
pub const GHOST_HUE: u16 = 0x3e3;
/// Shadow alpha = `t * SHADOW_ALPHA_RANGE + SHADOW_ALPHA_MIN` for the inverted, normalized luminance
/// `t` in 0..1, so 60..190.
pub const SHADOW_ALPHA_MIN: f32 = 60.0;
pub const SHADOW_ALPHA_RANGE: f32 = 130.0;
/// Luminance weights of the shadow hue, `L = 0.34 * b0 + 0.59 * b1 + 0.07 * b2`.
pub const SHADOW_LUMINANCE_WEIGHTS: [f32; 3] = [0.34, 0.59, 0.07];

const RGBA_CHANNELS: usize = 4;
const RGB_CHANNELS: usize = 3;
const ALPHA_CHANNEL: usize = 3;
const MAX_CHANNEL: f32 = u8::MAX as f32;

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

/// Which colour table a server hue uses, following `UOHues::GetHueTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcHueLookup {
    /// No hue: the colours are drawn as stored.
    None,
    /// `SHADOW_HUE`: black with alpha from the colour luminance.
    Shadow,
    /// The table of `hue{N:04}.bmp` (client table key `N - 1`).
    Bitmap(u16),
}

impl EcHueLookup {
    /// Maps a server hue: 0 -> none; `SHADOW_HUE` -> shadow; `HUE_PARTIAL_FLAG` set ->
    /// `hue & HUE_NUMBER_MASK`; no flag bits -> the hue itself; only `HUE_NO_TABLE_FLAG` -> none.
    pub fn from_hue(hue: u16) -> Self {
        if hue == 0 {
            return Self::None;
        }
        if hue == SHADOW_HUE {
            return Self::Shadow;
        }
        let number = if hue & HUE_PARTIAL_FLAG != 0 {
            hue & HUE_NUMBER_MASK
        } else if hue & HUE_NO_TABLE_FLAG == 0 {
            hue
        } else {
            return Self::None;
        };
        if number == 0 || number > MAX_EC_HUES {
            Self::None
        } else {
            Self::Bitmap(number)
        }
    }

    /// Client table key (`GetHueTable` result), `None` when the hue has no table.
    pub fn table_key(self) -> Option<u16> {
        match self {
            Self::None => None,
            Self::Shadow => Some(SHADOW_HUE_TABLE_KEY),
            Self::Bitmap(number) => Some(number - 1),
        }
    }
}

/// One hue table: RGBA entries indexed by the red byte of a hueable colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcHueTable {
    pub entries: [[u8; RGBA_CHANNELS]; HUE_TABLE_ENTRIES],
}

impl EcHueTable {
    /// Reads the table from a `hue%04d.bmp` payload: 32 bpp, 256 x 2, top image row.
    pub fn from_bitmap(bytes: &[u8]) -> eyre::Result<Self> {
        let (width, height, rgba) = decode_hue_image_to_rgba(bytes)?;
        if width != HUE_BITMAP_WIDTH || height != HUE_BITMAP_HEIGHT {
            eyre::bail!(
                "hue bitmap is {width}x{height}, expected {HUE_BITMAP_WIDTH}x{HUE_BITMAP_HEIGHT}"
            );
        }
        let row_bytes = HUE_TABLE_ENTRIES * RGBA_CHANNELS;
        let row = &rgba[HUE_TABLE_IMAGE_ROW * row_bytes..(HUE_TABLE_IMAGE_ROW + 1) * row_bytes];
        let mut entries = [[0u8; RGBA_CHANNELS]; HUE_TABLE_ENTRIES];
        for (entry, pixel) in entries.iter_mut().zip(row.chunks_exact(RGBA_CHANNELS)) {
            entry.copy_from_slice(pixel);
        }
        Ok(Self { entries })
    }
}

/// Hue tables of `Hues.uop`, keyed by bitmap number (server hue).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EcHueTables {
    pub tables: BTreeMap<u16, EcHueTable>,
}

/// The colour transform of one server hue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcHueTableRef<'a> {
    None,
    Shadow,
    Table(&'a EcHueTable),
}

impl EcHueTables {
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let package = UopPackage::load_with_mode(path, LoadMode::Lazy).wrap_err("failed to load hues.uop")?;
        Self::from_package(&package)
    }

    pub fn from_package(package: &UopPackage) -> eyre::Result<Self> {
        let mut tables = BTreeMap::new();
        for hue_id in 1..=MAX_EC_HUES {
            let Some(bytes) = package
                .unpack_file_by_hash(hue_bitmap_hash(hue_id))
                .wrap_err_with(|| format!("failed to unpack {}", hue_bitmap_path(hue_id)))?
            else {
                continue;
            };
            let table = EcHueTable::from_bitmap(&bytes)
                .wrap_err_with(|| format!("failed to read {}", hue_bitmap_path(hue_id)))?;
            tables.insert(hue_id, table);
        }
        Ok(Self { tables })
    }

    /// Colour transform of a server hue (`UOHues::GetHueTable`); a hue whose bitmap is missing has
    /// no table.
    pub fn hue_table(&self, hue: u16) -> EcHueTableRef<'_> {
        match EcHueLookup::from_hue(hue) {
            EcHueLookup::None => EcHueTableRef::None,
            EcHueLookup::Shadow => EcHueTableRef::Shadow,
            EcHueLookup::Bitmap(number) => self
                .tables
                .get(&number)
                .map_or(EcHueTableRef::None, EcHueTableRef::Table),
        }
    }

    /// Hues straight-alpha RGBA pixels in place. `hue_mask` has one byte per pixel; a non-zero
    /// byte marks a hueable pixel.
    pub fn apply(&self, hue: u16, rgba: &mut [u8], hue_mask: &[u8]) {
        self.hue_table(hue).apply(rgba, hue_mask);
    }
}

impl EcHueTableRef<'_> {
    /// Hues straight-alpha RGBA pixels in place.
    ///
    /// A table replaces the colour of each hueable pixel with `table[red]` and keeps its edge
    /// alpha (scaled by the table alpha, 255 in every client table). The shadow hue turns every
    /// coded pixel black with alpha `t * 130 + 60` from the inverted luminance; the client's exact
    /// normalization of `t` was read only in outline, so this is `1 - L / 255`.
    pub fn apply(self, rgba: &mut [u8], hue_mask: &[u8]) {
        match self {
            Self::None => {}
            Self::Table(table) => {
                for (pixel, &mask) in rgba.chunks_exact_mut(RGBA_CHANNELS).zip(hue_mask) {
                    if mask == 0 || pixel[ALPHA_CHANNEL] == 0 {
                        continue;
                    }
                    let entry = table.entries[pixel[0] as usize];
                    pixel[..RGB_CHANNELS].copy_from_slice(&entry[..RGB_CHANNELS]);
                    pixel[ALPHA_CHANNEL] = scale_channel(pixel[ALPHA_CHANNEL], entry[ALPHA_CHANNEL]);
                }
            }
            Self::Shadow => {
                for pixel in rgba.chunks_exact_mut(RGBA_CHANNELS) {
                    if pixel[ALPHA_CHANNEL] == 0 {
                        continue;
                    }
                    let alpha = shadow_alpha([pixel[0], pixel[1], pixel[2]]);
                    pixel[..RGB_CHANNELS].fill(0);
                    pixel[ALPHA_CHANNEL] = scale_channel(pixel[ALPHA_CHANNEL], alpha);
                }
            }
        }
    }
}

/// Alpha of an opaque colour under `SHADOW_HUE`.
pub fn shadow_alpha(rgb: [u8; 3]) -> u8 {
    let luminance: f32 = rgb
        .iter()
        .zip(SHADOW_LUMINANCE_WEIGHTS)
        .map(|(&channel, weight)| channel as f32 * weight)
        .sum();
    let inverted = 1.0 - (luminance / MAX_CHANNEL).clamp(0.0, 1.0);
    (inverted * SHADOW_ALPHA_RANGE + SHADOW_ALPHA_MIN).round() as u8
}

fn scale_channel(value: u8, scale: u8) -> u8 {
    ((u16::from(value) * u16::from(scale) + u16::from(u8::MAX) / 2) / u16::from(u8::MAX)) as u8
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

    fn hue_bitmap(first_row: impl Fn(usize) -> [u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 54];
        bytes[0..2].copy_from_slice(b"BM");
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(HUE_BITMAP_WIDTH as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(HUE_BITMAP_HEIGHT as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&32u16.to_le_bytes());
        // Bottom-up: the first stored row is the bottom image row, filled with junk.
        bytes.extend(std::iter::repeat_n(0xee, HUE_TABLE_ENTRIES * 4));
        for index in 0..HUE_TABLE_ENTRIES {
            let [r, g, b, a] = first_row(index);
            bytes.extend_from_slice(&[b, g, r, a]);
        }
        bytes
    }

    #[test]
    fn hue_lookup_follows_get_hue_table() {
        assert_eq!(EcHueLookup::from_hue(0), EcHueLookup::None);
        assert_eq!(EcHueLookup::from_hue(1), EcHueLookup::Bitmap(1));
        assert_eq!(EcHueLookup::from_hue(1).table_key(), Some(0));
        assert_eq!(EcHueLookup::from_hue(GHOST_HUE), EcHueLookup::Bitmap(GHOST_HUE));
        assert_eq!(EcHueLookup::from_hue(0x8000 | 0x21), EcHueLookup::Bitmap(0x21));
        assert_eq!(EcHueLookup::from_hue(0xc000 | 0x21), EcHueLookup::Bitmap(0x21));
        assert_eq!(EcHueLookup::from_hue(0x8000), EcHueLookup::None);
        assert_eq!(EcHueLookup::from_hue(0x4000 | 0x21), EcHueLookup::None);
        assert_eq!(EcHueLookup::from_hue(SHADOW_HUE), EcHueLookup::Shadow);
        assert_eq!(EcHueLookup::from_hue(SHADOW_HUE).table_key(), Some(SHADOW_HUE_TABLE_KEY));
        assert_eq!(EcHueLookup::from_hue(MAX_EC_HUES + 1), EcHueLookup::None);
    }

    #[test]
    fn hue_table_reads_the_top_image_row_as_rgba() {
        let bitmap = hue_bitmap(|index| [index as u8, 10, 20, 255]);
        let table = EcHueTable::from_bitmap(&bitmap).unwrap();

        assert_eq!(table.entries[0], [0, 10, 20, 255]);
        assert_eq!(table.entries[200], [200, 10, 20, 255]);
        assert!(EcHueTable::from_bitmap(&bitmap[..bitmap.len() - 4]).is_err());
    }

    #[test]
    fn hue_tables_hue_only_masked_pixels_and_keep_edge_alpha() {
        let table = EcHueTable::from_bitmap(&hue_bitmap(|index| [0, index as u8, 0, 255])).unwrap();
        let tables = EcHueTables {
            tables: BTreeMap::from([(GHOST_HUE, table)]),
        };
        let mut rgba = vec![
            90, 90, 90, 255, // hueable solid
            120, 120, 120, 127, // hueable edge
            200, 30, 30, 255, // keeps its colour
            0, 0, 0, 0, // transparent
        ];
        tables.apply(GHOST_HUE, &mut rgba, &[1, 1, 0, 1]);

        assert_eq!(rgba, vec![0, 90, 0, 255, 0, 120, 0, 127, 200, 30, 30, 255, 0, 0, 0, 0]);
        assert_eq!(tables.hue_table(0), EcHueTableRef::None);
        assert_eq!(tables.hue_table(1), EcHueTableRef::None);
        assert!(matches!(tables.hue_table(0x8000 | GHOST_HUE), EcHueTableRef::Table(_)));
    }

    #[test]
    fn shadow_hue_turns_coded_pixels_into_translucent_black() {
        assert_eq!(shadow_alpha([0, 0, 0]), (SHADOW_ALPHA_MIN + SHADOW_ALPHA_RANGE) as u8);
        assert_eq!(shadow_alpha([255, 255, 255]), SHADOW_ALPHA_MIN as u8);

        let mut rgba = vec![255, 255, 255, 255, 0, 0, 0, 0];
        EcHueTables::default().apply(SHADOW_HUE, &mut rgba, &[0, 0]);
        assert_eq!(rgba, vec![0, 0, 0, SHADOW_ALPHA_MIN as u8, 0, 0, 0, 0]);
    }
}
