// Credits:
// "Mythic" decompression from UOFiddler's MythicDecompress.cs.
// BWT decompression from ClassicUO.
// Most of the other code from LegacyMUL(CL)-N.

// TODO: use uocf::uop_container library instead of reinventing the wheel.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use flate2::{
    Compression,
    read::{ZlibDecoder, ZlibEncoder},
};
use uocf::uop_container::{
    compression::{mythic_decompress, zlib_bwt_codec},
    hash::{hash_data_block, hash_file_name_single},
};

mod pack;
mod unpack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    ArtLegacyMul,
    GumpartLegacyMul,
    MapLegacyMul,
    SoundLegacyMul,
    MultiCollection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFlag {
    None = 0,
    Zlib = 1,
    Mythic = 2,
    ZlibBwt = 3,
}

impl From<i16> for CompressionFlag {
    fn from(value: i16) -> Self {
        match value {
            0 => CompressionFlag::None,
            1 => CompressionFlag::Zlib,
            2 => CompressionFlag::Mythic,
            3 => CompressionFlag::ZlibBwt,
            _ => CompressionFlag::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct IdxEntry {
    id: i32,
    offset: i32,
    size: i32,
    extra: i32,
}

#[derive(Debug, Clone, Copy)]
struct TableEntry {
    offset: u64,
    header_length: u32,
    size: u32,
    decompressed_size: u32,
    identifier: u64,
    hash: u32,
    compression_flag: i16,
}

pub struct LegacyMulFileConverter;

impl LegacyMulFileConverter {
    const FIRST_TABLE: u64 = 0x200;
    const TABLE_SIZE: u32 = 0x64;

    pub fn to_uop(
        in_file: &Path,
        in_file_idx: Option<&Path>,
        out_file: &Path,
        file_type: FileType,
        type_index: i32,
        compression_flag: CompressionFlag,
    ) -> io::Result<()> {
        pack::to_uop(
            in_file,
            in_file_idx,
            out_file,
            file_type,
            type_index,
            compression_flag,
        )
    }

    pub fn from_uop(
        in_file: &Path,
        out_file: &Path,
        out_file_idx: Option<&Path>,
        file_type: FileType,
        type_index: i32,
        housing_bin_file: Option<&Path>,
    ) -> io::Result<()> {
        unpack::from_uop(
            in_file,
            out_file,
            out_file_idx,
            file_type,
            type_index,
            housing_bin_file,
        )
    }
}

fn compression_flag_for(file_type: FileType, compression_flag: CompressionFlag) -> CompressionFlag {
    if matches!(
        file_type,
        FileType::ArtLegacyMul
            | FileType::GumpartLegacyMul
            | FileType::MapLegacyMul
            | FileType::SoundLegacyMul
    ) {
        CompressionFlag::None
    } else {
        compression_flag
    }
}

fn read_and_decompress_entry(
    reader: &mut BufReader<File>,
    entry: &TableEntry,
) -> io::Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(entry.offset + entry.header_length as u64))?;
    let mut chunk_data = vec![0; entry.size as usize];
    reader.read_exact(&mut chunk_data)?;

    match CompressionFlag::from(entry.compression_flag) {
        CompressionFlag::Zlib => {
            let mut decoder = ZlibDecoder::new(chunk_data.as_slice());
            let mut decompressed = vec![0; entry.decompressed_size as usize];
            decoder.read_exact(&mut decompressed)?;
            Ok(decompressed)
        }
        CompressionFlag::Mythic => mythic_decompress::decompress_with_header(&chunk_data),
        CompressionFlag::ZlibBwt => zlib_bwt_codec::decompress(&chunk_data),
        CompressionFlag::None => Ok(chunk_data),
    }
}

fn expected_map_file_size(type_index: i32) -> Option<u64> {
    match type_index {
        0 => Some(89_915_392),
        1 => Some(89_915_392),
        2 => Some(11_289_600),
        3 => Some(16_056_320),
        4 => Some(6_421_156),
        5 => Some(16_056_320),
        _ => None,
    }
}

fn warn_if_non_standard_map_size(out_file: &Path, type_index: i32, current_size: u64) {
    match expected_map_file_size(type_index) {
        Some(expected_size) if current_size != expected_size => {
            eprintln!(
                "warning: map file '{}' has size {} bytes, expected {} bytes for standard map{}; keeping non-standard size as-is.",
                out_file.display(),
                current_size,
                expected_size,
                type_index,
            );
        }
        Some(_) => {}
        None => {
            eprintln!(
                "warning: map file '{}' uses unknown map type {}; generated size is {} bytes.",
                out_file.display(),
                type_index,
                current_size,
            );
        }
    }
}

fn get_hash_format(file_type: FileType, type_index: i32) -> (String, String, i32) {
    let mut max_id = 0x7FFFF;

    let (format_0, format_1) = match file_type {
        FileType::ArtLegacyMul => {
            max_id = 0x13FDC;
            (
                "build/artlegacymul/{0:00000000}.tga".to_string(),
                String::new(),
            )
        }
        FileType::GumpartLegacyMul => (
            "build/gumpartlegacymul/{0:00000000}.tga".to_string(),
            "build/gumpartlegacymul/{0:0000000}.tga".to_string(),
        ),
        FileType::MapLegacyMul => (
            format!("build/map{}legacymul/{{0:00000000}}.dat", type_index),
            String::new(),
        ),
        FileType::SoundLegacyMul => (
            "build/soundlegacymul/{0:00000000}.dat".to_string(),
            String::new(),
        ),
        FileType::MultiCollection => {
            max_id = 0x2200;
            (
                "build/multicollection/{0:000000}.bin".to_string(),
                String::new(),
            )
        }
    };

    (format_0, format_1, max_id)
}

fn write_multi_uop_entry_to_mul(
    mul_writer: &mut BufWriter<File>,
    chunk_data: &[u8],
) -> io::Result<()> {
    let mut span = chunk_data;
    let count = u32::from_le_bytes(read_exact_array::<4>(span, 4, "multi item count")?);
    span = &span[8..];

    for _ in 0..count {
        let item_id = u16::from_le_bytes(read_exact_array::<2>(span, 0, "multi item id")?);
        let x = i16::from_le_bytes(read_exact_array::<2>(span, 2, "multi item x")?);
        let y = i16::from_le_bytes(read_exact_array::<2>(span, 4, "multi item y")?);
        let z = i16::from_le_bytes(read_exact_array::<2>(span, 6, "multi item z")?);

        let flag_value = u16::from_le_bytes(read_exact_array::<2>(span, 8, "multi item flag")?);
        let clilocs_count =
            u32::from_le_bytes(read_exact_array::<4>(span, 10, "multi item cliloc count")?);

        let skip = (clilocs_count as usize).min(i32::MAX as usize) * 4;
        let next_offset = 14usize.checked_add(skip).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "multi item cliloc list is too large",
            )
        })?;
        span = span.get(next_offset..).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated multi item cliloc list",
            )
        })?;

        mul_writer.write_u16::<LittleEndian>(item_id)?;
        mul_writer.write_i16::<LittleEndian>(x)?;
        mul_writer.write_i16::<LittleEndian>(y)?;
        mul_writer.write_i16::<LittleEndian>(z)?;
        mul_writer.write_u32::<LittleEndian>(if flag_value != 0 { 0 } else { 1 })?;
        mul_writer.write_u32::<LittleEndian>(0)?;
    }

    Ok(())
}

fn read_exact_array<const N: usize>(
    data: &[u8],
    offset: usize,
    field_name: &'static str,
) -> io::Result<[u8; N]> {
    let end = offset.checked_add(N).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{field_name} offset overflow"),
        )
    })?;
    let bytes = data.get(offset..end).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("truncated {field_name}"),
        )
    })?;
    bytes.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid {field_name} byte width"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> io::Result<(File, std::path::PathBuf)> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "uocf-cli-{name}-{}-{unique}.tmp",
            std::process::id()
        ));
        Ok((File::create(&path)?, path))
    }

    #[test]
    fn truncated_multi_header_returns_error() -> io::Result<()> {
        let (file, path) = temp_file("truncated-multi-header")?;
        let mut writer = BufWriter::new(file);

        let result = write_multi_uop_entry_to_mul(&mut writer, &[0, 1, 2]);

        let _ = std::fs::remove_file(path);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        Ok(())
    }

    #[test]
    fn truncated_multi_item_returns_error() -> io::Result<()> {
        let (file, path) = temp_file("truncated-multi-item")?;
        let mut writer = BufWriter::new(file);
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0; 10]);

        let result = write_multi_uop_entry_to_mul(&mut writer, &data);

        let _ = std::fs::remove_file(path);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        Ok(())
    }
}
