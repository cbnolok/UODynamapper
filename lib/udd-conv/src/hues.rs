use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context, WrapErr};
use log::info;

use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;
use udd_assets::hues::{
    encode_hues_csv, HueSlotRecord, HUE_FLAG_PRESENT, HUES_METADATA_ENTRY_PATH,
    HUES_TEXTURE_ENTRY_PATH, HUES_TEXTURE_HEIGHT, HUES_TEXTURE_WIDTH, HUE_STRIP_WIDTH, MAX_HUE_ID,
};
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::classic::hues::{load_hues, HueEntry};
use uocf::enhanced::hues::{
    atlas_coord_for_hue, decode_hue_image_to_rgba, EcHuePackage, HUES_UOP_NAME,
};
use uocf::uop_container::package::{LoadMode, UopPackage};

const BLUR_RADIUS: usize = 6;
const BLUR_ALPHA_TABLE: [u32; 17] = [14, 10, 8, 6, 5, 5, 4, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2];

pub struct HuesOptions {
    pub compression: CompressionFlag,
}

impl Default for HuesOptions {
    fn default() -> Self {
        Self {
            compression: CompressionFlag::ZstdNoDict,
        }
    }
}

pub fn convert_hues_mul_to_hues_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &HuesOptions,
) -> eyre::Result<()> {
    let hues_path = find_first_existing_file(source_dirs, &["hues.mul"])
        .ok_or_else(|| eyre::eyre!("missing hues.mul"))?;
    println!("Using CC hues source file: {}", hues_path.display());
    convert_hues_mul_to_hues_uddp(&hues_path, out_file, options)
}

pub fn convert_hues_to_hues_uddp_from_sources(
    source_dirs: &[PathBuf],
    out_file: &Path,
    options: &HuesOptions,
) -> eyre::Result<()> {
    if let Some(hues_path) = find_first_existing_file(source_dirs, &["hues.mul"]) {
        println!("Using CC hues source file: {}", hues_path.display());
        return convert_hues_mul_to_hues_uddp(&hues_path, out_file, options);
    }

    let hues_path = find_first_existing_file(source_dirs, &[HUES_UOP_NAME])
        .ok_or_else(|| eyre::eyre!("missing hues.mul or hues.uop"))?;
    println!("Using EC hues source file: {}", hues_path.display());
    convert_hues_uop_to_hues_uddp(&hues_path, out_file, options)
}

pub fn convert_hues_mul_to_hues_uddp(
    hues_mul_path: &Path,
    out_file: &Path,
    options: &HuesOptions,
) -> eyre::Result<()> {
    info!("Converting hues.mul to {}", out_file.display());
    let hues = load_hues(hues_mul_path).wrap_err("failed to load hues.mul")?;
    let (records, texture_bytes) = build_hues_texture_and_records(&hues)?;
    write_hues_uddp(&records, &texture_bytes, out_file, options)
}

pub fn convert_hues_uop_to_hues_uddp(
    hues_uop_path: &Path,
    out_file: &Path,
    options: &HuesOptions,
) -> eyre::Result<()> {
    info!("Converting hues.uop to {}", out_file.display());
    let mut package =
        UopPackage::load_with_mode(hues_uop_path, LoadMode::Lazy).wrap_err("failed to load hues.uop")?;
    let ec_hues = EcHuePackage::from_package(&package).wrap_err("failed to index hues.uop")?;
    let (records, texture_bytes) = build_ec_hues_texture_and_records(&mut package, &ec_hues)?;
    write_hues_uddp(&records, &texture_bytes, out_file, options)
}

fn write_hues_uddp(
    records: &[HueSlotRecord],
    texture_bytes: &[u8],
    out_file: &Path,
    options: &HuesOptions,
) -> eyre::Result<()> {
    let csv_bytes = encode_hues_csv(records)?;
    let mut package = UddpBuilder::new(LookupMode::VirtualPathHash);
    package
        .add_file(AddFileRequest {
            data_type: DataType::Metadata as u8,
            compression: options.compression,
            width: 0,
            height: 0,
            virtual_path: Some(HUES_METADATA_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &csv_bytes,
        })
        .context("add metadata/hues.csv")?;
    package
        .add_file(AddFileRequest {
            data_type: DataType::Texture as u8,
            compression: options.compression,
            width: HUES_TEXTURE_WIDTH,
            height: HUES_TEXTURE_HEIGHT,
            virtual_path: Some(HUES_TEXTURE_ENTRY_PATH),
            path_hash64: None,
            id: None,
            data: &texture_bytes,
        })
        .context("add textures/hues.rgba8888")?;

    build_and_write_package(&mut package, out_file)?;
    Ok(())
}

fn build_ec_hues_texture_and_records(
    package: &mut UopPackage,
    ec_hues: &EcHuePackage,
) -> eyre::Result<(Vec<HueSlotRecord>, Vec<u8>)> {
    let mut texture_bytes = vec![0u8; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4];
    let mut records = Vec::new();

    for entry in &ec_hues.bitmaps {
        let coord = atlas_coord_for_hue(entry.hue_id)
            .ok_or_else(|| eyre::eyre!("failed to compute atlas coord for hue {}", entry.hue_id))?;
        let bytes = package
            .unpack_file_by_hash_cached(entry.filename_hash)
            .wrap_err_with(|| format!("failed to unpack EC hue {}", entry.hue_id))?
            .ok_or_else(|| eyre::eyre!("missing EC hue bitmap {}", entry.hue_id))?;
        let (width, height, pixels) = decode_hue_image_to_rgba(&bytes)
            .wrap_err_with(|| format!("failed to decode EC hue bitmap {}", entry.hue_id))?;
        let row = build_palette_row_from_rgba_image(width, height, &pixels)?;
        blit_palette_row(&mut texture_bytes, coord.x, coord.y, &row)?;

        records.push(HueSlotRecord {
            hue_id: entry.hue_id,
            name: ec_hues.hue_name(entry.hue_id).unwrap_or("").to_string(),
            table_start: 0,
            table_end: (HUE_STRIP_WIDTH - 1) as u16,
            texture_column: coord.column,
            texture_row: coord.row,
            palette_width_pixels: HUE_STRIP_WIDTH,
            flags: HUE_FLAG_PRESENT,
        });
    }

    records.sort_by_key(|record| record.hue_id);
    Ok((records, texture_bytes))
}

fn build_hues_texture_and_records(hues: &[HueEntry]) -> eyre::Result<(Vec<HueSlotRecord>, Vec<u8>)> {
    let mut texture_bytes = vec![0u8; HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4];
    let mut records = Vec::new();

    for hue in hues.iter().filter(|hue| hue.id >= 1 && hue.id <= u32::from(MAX_HUE_ID)) {
        let coord = uocf::enhanced::hues::atlas_coord_for_hue(hue.id as u16)
            .ok_or_else(|| eyre::eyre!("failed to compute atlas coord for hue {}", hue.id))?;
        let mut row = build_palette_row_rgba(&hue.color_table);
        blur_palette_row_in_place(&mut row, BLUR_RADIUS);
        blit_palette_row(&mut texture_bytes, coord.x, coord.y, &row)?;

        records.push(HueSlotRecord {
            hue_id: hue.id as u16,
            name: decode_hue_name(&hue.name),
            table_start: hue.table_start,
            table_end: hue.table_end,
            texture_column: coord.column,
            texture_row: coord.row,
            palette_width_pixels: HUE_STRIP_WIDTH,
            flags: HUE_FLAG_PRESENT,
        });
    }

    records.sort_by_key(|record| record.hue_id);
    Ok((records, texture_bytes))
}

fn decode_hue_name(name: &[u8; 20]) -> String {
    String::from_utf8_lossy(name)
        .trim_matches('\0')
        .trim_end()
        .to_string()
}

fn build_palette_row_rgba(color_table: &[u16; 32]) -> Vec<u8> {
    let mut out = vec![0u8; HUE_STRIP_WIDTH as usize * 4];
    for (index, color16) in color_table.iter().copied().enumerate() {
        let rgba = argb1555_to_rgba8888_exact(color16);
        let start = index * 8 * 4;
        for repeat in 0..8 {
            let offset = start + repeat * 4;
            out[offset..offset + 4].copy_from_slice(&rgba);
        }
    }
    out
}

fn build_palette_row_from_rgba_image(
    width: u32,
    height: u32,
    pixels: &[u8],
) -> eyre::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        eyre::bail!("EC hue bitmap has empty dimensions");
    }
    let expected_len = width as usize * height as usize * 4;
    if pixels.len() != expected_len {
        eyre::bail!(
            "EC hue bitmap has {} bytes, expected {} for {}x{} RGBA",
            pixels.len(),
            expected_len,
            width,
            height
        );
    }

    let mut row = vec![0u8; HUE_STRIP_WIDTH as usize * 4];
    let sample_y = height / 2;
    for dst_x in 0..HUE_STRIP_WIDTH {
        let src_x = if HUE_STRIP_WIDTH <= 1 {
            0
        } else {
            dst_x * (width - 1) / (HUE_STRIP_WIDTH - 1)
        };
        let src_offset = ((sample_y * width + src_x) * 4) as usize;
        let dst_offset = (dst_x * 4) as usize;
        row[dst_offset..dst_offset + 4].copy_from_slice(&pixels[src_offset..src_offset + 4]);
    }
    Ok(row)
}

fn argb1555_to_rgba8888_exact(color16: u16) -> [u8; 4] {
    let r5 = ((color16 >> 10) & 0x1F) as u32;
    let g5 = ((color16 >> 5) & 0x1F) as u32;
    let b5 = (color16 & 0x1F) as u32;
    [
        ((r5 * 255) / 31) as u8,
        ((g5 * 255) / 31) as u8,
        ((b5 * 255) / 31) as u8,
        255,
    ]
}

fn blur_palette_row_in_place(row: &mut [u8], radius: usize) {
    if row.is_empty() {
        return;
    }

    let alpha: i32 = if radius < 1 {
        16
    } else {
        BLUR_ALPHA_TABLE[radius.min(BLUR_ALPHA_TABLE.len()) - 1] as i32
    };

    for channel in 0..4 {
        let mut accum = i32::from(row[channel]) << 4;
        let mut x = 4 + channel;
        while x < row.len() {
            let sample = i32::from(row[x]) << 4;
            accum += ((sample - accum) * alpha) / 16;
            row[x] = (accum >> 4) as u8;
            x += 4;
        }

        let last = row.len() - 4 + channel;
        let mut accum = i32::from(row[last]) << 4;
        let mut x = row.len() as isize - 8 + channel as isize;
        while x >= 0 {
            let idx = x as usize;
            let sample = i32::from(row[idx]) << 4;
            accum += ((sample - accum) * alpha) / 16;
            row[idx] = (accum >> 4) as u8;
            x -= 4;
        }
    }
}

fn blit_palette_row(texture_bytes: &mut [u8], x: u32, y: u32, row: &[u8]) -> eyre::Result<()> {
    if row.len() != HUE_STRIP_WIDTH as usize * 4 {
        eyre::bail!("palette row length {} does not match expected width", row.len());
    }
    if x + HUE_STRIP_WIDTH > HUES_TEXTURE_WIDTH || y >= HUES_TEXTURE_HEIGHT {
        eyre::bail!("palette row at {},{} is outside hue texture bounds", x, y);
    }

    let row_start = ((y * HUES_TEXTURE_WIDTH + x) * 4) as usize;
    let row_end = row_start + row.len();
    texture_bytes[row_start..row_end].copy_from_slice(row);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use udd_assets::hues::{parse_hues_csv, HuesPackage};
    use udd_container::UddpReader;

    fn hue_entry(id: u32, name: &str, colors: [u16; 32]) -> HueEntry {
        let mut name_bytes = [0u8; 20];
        let raw = name.as_bytes();
        let len = raw.len().min(name_bytes.len());
        name_bytes[..len].copy_from_slice(&raw[..len]);
        HueEntry {
            id,
            color_table: colors,
            table_start: 1,
            table_end: 31,
            name: name_bytes,
        }
    }

    #[test]
    fn texture_builder_packs_rows_into_fixed_layout() {
        let mut first_colors = [0u16; 32];
        first_colors[0] = 0x7C00;
        first_colors[31] = 0x03E0;

        let mut second_colors = [0u16; 32];
        second_colors[0] = 0x001F;
        second_colors[31] = 0x7FFF;

        let hues = vec![
            hue_entry(1, "first", first_colors),
            hue_entry(1024, "second", second_colors),
        ];

        let (records, texture) = build_hues_texture_and_records(&hues).expect("build hues texture");
        assert_eq!(records.len(), 2);
        assert_eq!(
            texture.len(),
            HUES_TEXTURE_WIDTH as usize * HUES_TEXTURE_HEIGHT as usize * 4
        );

        let first = &records[0];
        assert_eq!(first.hue_id, 1);
        assert_eq!(first.texture_column, 0);
        assert_eq!(first.texture_row, 1);

        let second = &records[1];
        assert_eq!(second.hue_id, 1024);
        assert_eq!(second.texture_column, 1);
        assert_eq!(second.texture_row, 0);

        let transparent_prefix = &texture[..HUE_STRIP_WIDTH as usize * 4];
        assert!(transparent_prefix.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn package_roundtrip_loads_csv_and_texture() {
        let hues = vec![hue_entry(1, "alpha,beta", [0x7FFF; 32])];
        let (records, texture) = build_hues_texture_and_records(&hues).expect("build texture");
        let csv = encode_hues_csv(&records).expect("encode csv");
        let parsed = parse_hues_csv(&csv).expect("parse csv");
        assert_eq!(parsed[1].as_ref().expect("hue 1").name, "alpha,beta");

        let mut builder = UddpBuilder::new(LookupMode::VirtualPathHash);
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Metadata as u8,
                compression: CompressionFlag::None,
                width: 0,
                height: 0,
                virtual_path: Some(HUES_METADATA_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &csv,
            })
            .expect("add csv");
        builder
            .add_file(AddFileRequest {
                data_type: DataType::Texture as u8,
                compression: CompressionFlag::None,
                width: HUES_TEXTURE_WIDTH,
                height: HUES_TEXTURE_HEIGHT,
                virtual_path: Some(HUES_TEXTURE_ENTRY_PATH),
                path_hash64: None,
                id: None,
                data: &texture,
            })
            .expect("add texture");

        let package = HuesPackage::from_uddp_package(
            UddpReader::open(builder.build().expect("build package")).expect("open package"),
        )
        .expect("load hues package");

        assert_eq!(package.hue_name(1), Some("alpha,beta"));
        assert_eq!(
            package.texture_coord_for_hue(1).expect("coord").row,
            1
        );
        assert_eq!(package.read_texture_bytes().expect("texture bytes"), texture);
    }

    #[test]
    fn hues_uddp_creation_expands_each_hue_to_256_pixel_strip() {
        let mut colors = [0u16; 32];
        colors.fill(0x7C00);
        let hues = vec![hue_entry(1, "red", colors)];

        let (records, texture) = build_hues_texture_and_records(&hues).expect("build texture");
        let coord = records
            .iter()
            .find(|record| record.hue_id == 1)
            .expect("hue 1 record");
        let row_start = ((coord.texture_row * HUES_TEXTURE_WIDTH
            + coord.texture_column * HUE_STRIP_WIDTH)
            * 4) as usize;
        let strip = &texture[row_start..row_start + HUE_STRIP_WIDTH as usize * 4];

        for pixel in strip.chunks_exact(4) {
            assert_eq!(pixel, &[255, 0, 0, 255]);
        }
    }

    #[test]
    fn ec_hue_bitmap_row_is_resampled_to_lookup_strip() {
        let pixels = vec![
            1, 2, 3, 4,
            5, 6, 7, 8,
        ];

        let row = build_palette_row_from_rgba_image(2, 1, &pixels).expect("build row");

        assert_eq!(&row[..4], &[1, 2, 3, 4]);
        assert_eq!(
            &row[(HUE_STRIP_WIDTH as usize - 1) * 4..HUE_STRIP_WIDTH as usize * 4],
            &[5, 6, 7, 8]
        );
    }
}
