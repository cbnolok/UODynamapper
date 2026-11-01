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
use log::warn;
use flate2::read::ZlibDecoder;
use uocf::uop_container::{
    compression::{mythic_decompress, zlib_bwt_codec},
    encode_payload,
    hash::{hash_data_block, hash_file_name_single},
    UopCompression,
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
            warn!(
                "warning: map file '{}' has size {} bytes, expected {} bytes for standard map{}; keeping non-standard size as-is.",
                out_file.display(),
                current_size,
                expected_size,
                type_index,
            );
        }
        Some(_) => {}
        None => {
            warn!(
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

    #[test]
    fn sound_mul_uop_roundtrip_preserves_payload() -> io::Result<()> {
        let temp = std::env::temp_dir().join(format!(
            "uocf-cli-sound-roundtrip-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp)?;

        let sound_mul = temp.join("sound.mul");
        let sound_idx = temp.join("soundidx.mul");
        let sound_uop = temp.join("soundLegacyMUL.uop");
        let out_mul = temp.join("out_sound.mul");
        let out_idx = temp.join("out_soundidx.mul");
        let payload = b"RIFFdemo-wave-payload";

        std::fs::write(&sound_mul, payload)?;
        {
            let mut idx = BufWriter::new(File::create(&sound_idx)?);
            idx.write_i32::<LittleEndian>(0)?;
            idx.write_i32::<LittleEndian>(payload.len() as i32)?;
            idx.write_i32::<LittleEndian>(1)?;
        }

        LegacyMulFileConverter::to_uop(
            &sound_mul,
            Some(&sound_idx),
            &sound_uop,
            FileType::SoundLegacyMul,
            0,
            CompressionFlag::Zlib,
        )?;
        LegacyMulFileConverter::from_uop(
            &sound_uop,
            &out_mul,
            Some(&out_idx),
            FileType::SoundLegacyMul,
            0,
            None,
        )?;

        assert_eq!(std::fs::read(&out_mul)?, payload);
        let mut idx = BufReader::new(File::open(&out_idx)?);
        assert_eq!(idx.read_i32::<LittleEndian>()?, 0);
        assert_eq!(idx.read_i32::<LittleEndian>()?, payload.len() as i32);
        assert_eq!(idx.read_i32::<LittleEndian>()?, 1);

        let _ = std::fs::remove_dir_all(temp);
        Ok(())
    }

    #[test]
    fn art_mul_uop_roundtrip_preserves_payload_and_index() -> io::Result<()> {
        let temp = temp_dir("art-roundtrip")?;
        let art_mul = temp.join("art.mul");
        let art_idx = temp.join("artidx.mul");
        let art_uop = temp.join("artLegacyMUL.uop");
        let out_mul = temp.join("out_art.mul");
        let out_idx = temp.join("out_artidx.mul");
        let payload = b"art-payload";

        std::fs::write(&art_mul, payload)?;
        write_idx_entry(&art_idx, 0, payload.len() as i32, 0)?;

        LegacyMulFileConverter::to_uop(
            &art_mul,
            Some(&art_idx),
            &art_uop,
            FileType::ArtLegacyMul,
            0,
            CompressionFlag::Zlib,
        )?;
        LegacyMulFileConverter::from_uop(
            &art_uop,
            &out_mul,
            Some(&out_idx),
            FileType::ArtLegacyMul,
            0,
            None,
        )?;

        assert_eq!(std::fs::read(&out_mul)?, payload);
        assert_idx_entry(&out_idx, 0, payload.len() as i32, 0)?;

        let _ = std::fs::remove_dir_all(temp);
        Ok(())
    }

    #[test]
    fn gump_mul_uop_roundtrip_preserves_payload_and_dimensions() -> io::Result<()> {
        let temp = temp_dir("gump-roundtrip")?;
        let gump_mul = temp.join("gumpart.mul");
        let gump_idx = temp.join("gumpidx.mul");
        let gump_uop = temp.join("gumpartLegacyMUL.uop");
        let out_mul = temp.join("out_gumpart.mul");
        let out_idx = temp.join("out_gumpidx.mul");
        let payload = b"gump-payload";
        let extra = (2i32 << 16) | 3;

        std::fs::write(&gump_mul, payload)?;
        write_idx_entry(&gump_idx, 0, payload.len() as i32, extra)?;

        LegacyMulFileConverter::to_uop(
            &gump_mul,
            Some(&gump_idx),
            &gump_uop,
            FileType::GumpartLegacyMul,
            0,
            CompressionFlag::Zlib,
        )?;
        LegacyMulFileConverter::from_uop(
            &gump_uop,
            &out_mul,
            Some(&out_idx),
            FileType::GumpartLegacyMul,
            0,
            None,
        )?;

        assert_eq!(std::fs::read(&out_mul)?, payload);
        assert_idx_entry(&out_idx, 0, payload.len() as i32, extra)?;

        let _ = std::fs::remove_dir_all(temp);
        Ok(())
    }

    #[test]
    fn one_block_map_mul_uop_roundtrip_preserves_payload() -> io::Result<()> {
        let temp = temp_dir("map-roundtrip")?;
        let map_mul = temp.join("map0.mul");
        let map_uop = temp.join("map0LegacyMUL.uop");
        let out_mul = temp.join("out_map0.mul");
        let mut payload = vec![0u8; 0xC4000];
        payload[0] = 7;
        payload[0xC4000 - 1] = 9;

        std::fs::write(&map_mul, &payload)?;

        LegacyMulFileConverter::to_uop(
            &map_mul,
            None,
            &map_uop,
            FileType::MapLegacyMul,
            0,
            CompressionFlag::None,
        )?;
        LegacyMulFileConverter::from_uop(
            &map_uop,
            &out_mul,
            None,
            FileType::MapLegacyMul,
            0,
            None,
        )?;

        assert_eq!(std::fs::read(&out_mul)?, payload);

        let _ = std::fs::remove_dir_all(temp);
        Ok(())
    }

    #[test]
    fn multi_uop_entry_unpack_writes_legacy_multi_record() -> io::Result<()> {
        let (file, path) = temp_file("valid-multi-entry")?;
        let mut writer = BufWriter::new(file);
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&123u16.to_le_bytes());
        data.extend_from_slice(&(-2i16).to_le_bytes());
        data.extend_from_slice(&3i16.to_le_bytes());
        data.extend_from_slice(&4i16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        write_multi_uop_entry_to_mul(&mut writer, &data)?;
        writer.flush()?;

        let bytes = std::fs::read(&path)?;
        let _ = std::fs::remove_file(path);
        assert_eq!(bytes.len(), 16);
        assert_eq!(u16::from_le_bytes(bytes[0..2].try_into().unwrap()), 123);
        assert_eq!(i16::from_le_bytes(bytes[2..4].try_into().unwrap()), -2);
        assert_eq!(i16::from_le_bytes(bytes[4..6].try_into().unwrap()), 3);
        assert_eq!(i16::from_le_bytes(bytes[6..8].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
        Ok(())
    }

    fn temp_dir(name: &str) -> io::Result<std::path::PathBuf> {
        let path = std::env::temp_dir().join(format!(
            "uocf-cli-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn write_idx_entry(path: &std::path::Path, offset: i32, size: i32, extra: i32) -> io::Result<()> {
        let mut idx = BufWriter::new(File::create(path)?);
        idx.write_i32::<LittleEndian>(offset)?;
        idx.write_i32::<LittleEndian>(size)?;
        idx.write_i32::<LittleEndian>(extra)?;
        Ok(())
    }

    /// Every payload of an unpacked real package, by name hash (empty slots left out).
    fn payloads(path: &std::path::Path) -> io::Result<std::collections::BTreeMap<u64, Vec<u8>>> {
        let package = uocf::uop_container::package::UopPackage::load(path)?;
        let hashes: Vec<u64> =
            package.iter_files().filter(|file| !file.is_empty_slot()).map(|file| file.filename_hash()).collect();
        hashes.into_iter().map(|hash| Ok((hash, package.unpack_file_by_hash(hash)?.unwrap_or_default()))).collect()
    }

    /// The records of a MUL and IDX pair by index: `(extra, payload)`, `None` for an empty slot. The MUL alone (a
    /// map) is one record.
    fn mul_records(mul: &std::path::Path, idx: Option<&std::path::Path>) -> io::Result<Vec<Option<(i32, Vec<u8>)>>> {
        const IDX_ENTRY_BYTES: usize = 12;
        const FIELD_BYTES: usize = 4;
        let data = std::fs::read(mul)?;
        let Some(idx) = idx else { return Ok(vec![Some((0, data))]) };
        let field = |entry: &[u8], index: usize| {
            i32::from_le_bytes(entry[index * FIELD_BYTES..(index + 1) * FIELD_BYTES].try_into().unwrap())
        };
        Ok(std::fs::read(idx)?
            .chunks_exact(IDX_ENTRY_BYTES)
            .map(|entry| {
                let (offset, size, extra) = (field(entry, 0), field(entry, 1), field(entry, 2));
                (offset >= 0 && size > 0).then(|| (extra, data[offset as usize..(offset + size) as usize].to_vec()))
            })
            .collect())
    }

    /// `mul-convert` on the real Classic packages: each is extracted to MUL (and IDX), packed back to a UOP holding
    /// the same payloads under the same names, and that UOP extracts to the same records (the MUL's record order
    /// follows the package's entry order, which the pack does not keep).
    #[test]
    #[ignore = "needs a real Classic client: set UO_CC_CLIENT_DIR and run with --ignored --release"]
    fn real_classic_packages_extract_and_pack_back_to_their_payloads() -> io::Result<()> {
        const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
        /// The map index of the facet package checked.
        const MAP_INDEX: i32 = 0;
        let dir = std::path::PathBuf::from(
            std::env::var_os(CC_CLIENT_DIR_ENV).unwrap_or_else(|| panic!("set {CC_CLIENT_DIR_ENV}")),
        );
        let cases = [
            ("artLegacyMUL.uop", FileType::ArtLegacyMul, true, 0),
            ("gumpartLegacyMUL.uop", FileType::GumpartLegacyMul, true, 0),
            ("map0LegacyMUL.uop", FileType::MapLegacyMul, false, MAP_INDEX),
            ("soundLegacyMUL.uop", FileType::SoundLegacyMul, true, 0),
        ];
        for (name, file_type, has_idx, type_index) in cases {
            let source = dir.join(name);
            let temp = temp_dir("real-repack")?;
            let (mul, idx) = (temp.join("out.mul"), temp.join("out.idx"));
            let (packed, mul_again, idx_again) = (temp.join("packed.uop"), temp.join("again.mul"), temp.join("again.idx"));
            let idx_arg = |path: &std::path::Path| has_idx.then(|| path.to_path_buf());
            LegacyMulFileConverter::from_uop(&source, &mul, idx_arg(&idx).as_deref(), file_type, type_index, None)?;
            LegacyMulFileConverter::to_uop(&mul, idx_arg(&idx).as_deref(), &packed, file_type, type_index, CompressionFlag::None)?;
            let (original, repacked) = (payloads(&source)?, payloads(&packed)?);
            let missing = original.keys().filter(|hash| !repacked.contains_key(hash)).count();
            let differing = original.iter().filter(|(hash, bytes)| repacked.get(hash).is_some_and(|other| other != *bytes)).count();
            let extra = repacked.len().saturating_sub(original.len() - missing);
            println!("{name}: {} entries; repacked {}, missing {missing}, differing {differing}, extra {extra}", original.len(), repacked.len());
            LegacyMulFileConverter::from_uop(&packed, &mul_again, idx_arg(&idx_again).as_deref(), file_type, type_index, None)?;
            let records = mul_records(&mul, idx_arg(&idx).as_deref())?;
            let same_records = records == mul_records(&mul_again, idx_arg(&idx_again).as_deref())?;
            let same_bytes = std::fs::read(&mul)? == std::fs::read(&mul_again)?;
            println!(
                "{name}: {} records extracted again identical {same_records} (MUL bytes identical {same_bytes})",
                records.iter().flatten().count()
            );
            std::fs::remove_dir_all(&temp)?;
            assert_eq!((missing, differing, extra), (0, 0, 0), "{name}: payloads");
            assert!(same_records, "{name}: the repacked package extracts differently");
        }
        Ok(())
    }

    fn assert_idx_entry(
        path: &std::path::Path,
        expected_offset: i32,
        expected_size: i32,
        expected_extra: i32,
    ) -> io::Result<()> {
        let mut idx = BufReader::new(File::open(path)?);
        assert_eq!(idx.read_i32::<LittleEndian>()?, expected_offset);
        assert_eq!(idx.read_i32::<LittleEndian>()?, expected_size);
        assert_eq!(idx.read_i32::<LittleEndian>()?, expected_extra);
        Ok(())
    }
}
