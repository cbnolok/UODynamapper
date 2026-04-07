// Credits:
// "Mythic" decompression from UOFiddler's MythicDecompress.cs.
// BWT decompression from ClassicUO.
// Most of the other code from LegacyMUL(CL)-N.

// TODO: use uocf::uop library instead of reinventing the wheel.

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
use uocf::uop::{
    compression::{mythic_decompress, zlib_bwt_codec},
    hash::{hash_data_block, hash_file_name_single},
};

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
        let mut reader = BufReader::new(File::open(in_file)?);
        let mut reader_idx = if let Some(path) = in_file_idx {
            Some(BufReader::new(File::open(path)?))
        } else {
            None
        };
        let mut writer = BufWriter::new(File::create(out_file)?);

        let mut idx_entries: Vec<IdxEntry> = Vec::new();

        if file_type == FileType::MapLegacyMul {
            let length = reader.seek(SeekFrom::End(0))? as i32;
            reader.seek(SeekFrom::Start(0))?;

            let mut position = 0;
            let mut id = 0;
            while position < length {
                idx_entries.push(IdxEntry {
                    id,
                    offset: position,
                    size: 0xC4000,
                    extra: 0,
                });
                position += 0xC4000;
                id += 1;
            }
        } else {
            let reader_idx = reader_idx.as_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Index file is required for this file type",
                )
            })?;
            let idx_entry_count = (reader_idx.seek(SeekFrom::End(0))? / 12) as usize;
            reader_idx.seek(SeekFrom::Start(0))?;

            for i in 0..idx_entry_count {
                let offset = reader_idx.read_i32::<LittleEndian>()?;
                if offset < 0 {
                    reader_idx.seek(SeekFrom::Current(8))?;
                    continue;
                }
                idx_entries.push(IdxEntry {
                    id: i as i32,
                    offset,
                    size: reader_idx.read_i32::<LittleEndian>()?,
                    extra: reader_idx.read_i32::<LittleEndian>()?,
                });
            }
        }

        let mut file_count = idx_entries.len();
        if file_type == FileType::MultiCollection {
            file_count += 1;
            idx_entries.push(IdxEntry {
                id: -1,
                offset: 0,
                size: 0,
                extra: 0,
            });
        }

        writer.write_u32::<LittleEndian>(0x50594D)?;
        writer.write_u32::<LittleEndian>(match file_type {
            FileType::GumpartLegacyMul => 4,
            _ => 5,
        })?;
        writer.write_u32::<LittleEndian>(0xFD23EC43)?;
        writer.write_u64::<LittleEndian>(match file_type {
            FileType::GumpartLegacyMul => 0x28,
            _ => Self::FIRST_TABLE,
        })?;
        writer.write_u32::<LittleEndian>(Self::TABLE_SIZE)?;
        writer.write_u32::<LittleEndian>(file_count as u32)?;
        writer.write_u32::<LittleEndian>(1)?;
        writer.write_u32::<LittleEndian>(1)?;
        writer.write_u32::<LittleEndian>(0)?;

        if file_type != FileType::GumpartLegacyMul {
            for _ in 0x28..Self::FIRST_TABLE {
                writer.write_u8(0)?;
            }
        }

        let table_count = (file_count as f64 / Self::TABLE_SIZE as f64).ceil() as usize;
        let mut table_entries: Vec<TableEntry> = vec![
            TableEntry {
                offset: 0,
                header_length: 0,
                size: 0,
                decompressed_size: 0,
                identifier: 0,
                hash: 0,
                compression_flag: 0,
            };
            Self::TABLE_SIZE as usize
        ];

        let (hash_format_0, hash_format_1, _max_id) = Self::get_hash_format(file_type, type_index);

        for i in 0..table_count {
            let this_table = writer.stream_position()?;

            let idx_start = i * Self::TABLE_SIZE as usize;
            let idx_end = ((i + 1) * Self::TABLE_SIZE as usize).min(file_count);

            writer.write_u32::<LittleEndian>((idx_end - idx_start) as u32)?;
            writer.write_u64::<LittleEndian>(0)?;
            writer.seek(SeekFrom::Current(34 * Self::TABLE_SIZE as i64))?;

            let mut table_idx = 0;
            for j in idx_start..idx_end {
                let mut data: Vec<u8>;
                let size_decompressed: usize;
                let current_compression_flag = if matches!(
                    file_type,
                    FileType::ArtLegacyMul
                        | FileType::GumpartLegacyMul
                        | FileType::MapLegacyMul
                        | FileType::SoundLegacyMul
                ) {
                    CompressionFlag::None
                } else {
                    compression_flag
                };

                if file_type == FileType::MultiCollection && j == file_count - 1 {
                    let housing_path = Path::new("housing.bin");
                    if housing_path.exists() {
                        data = std::fs::read(housing_path)?;
                        size_decompressed = data.len();

                        table_entries[table_idx].identifier =
                            hash_file_name_single("build/multicollection/housing.bin");
                        table_entries[table_idx].hash = hash_data_block(&data)?;
                        table_entries[table_idx].offset = writer.stream_position()?;
                        table_entries[table_idx].size = data.len() as u32;
                        table_entries[table_idx].decompressed_size = size_decompressed as u32;
                        table_entries[table_idx].compression_flag = CompressionFlag::None as i16;
                        writer.write_all(&data)?;
                    } else {
                        table_entries[table_idx].identifier =
                            hash_file_name_single("build/multicollection/housing.bin");
                        table_entries[table_idx].hash = 0;
                        table_entries[table_idx].offset = 0;
                        table_entries[table_idx].size = 0;
                        table_entries[table_idx].decompressed_size = 0;
                        table_entries[table_idx].compression_flag = CompressionFlag::None as i16;
                    }
                } else {
                    reader.seek(SeekFrom::Start(idx_entries[j].offset as u64))?;
                    data = vec![0; idx_entries[j].size as usize];
                    reader.read_exact(&mut data)?;
                    size_decompressed = data.len();

                    if file_type == FileType::GumpartLegacyMul {
                        let width = (idx_entries[j].extra >> 16) & 0xFFFF;
                        let height = idx_entries[j].extra & 0xFFFF;

                        let mut gump_art_data = Vec::new();
                        gump_art_data.write_u32::<LittleEndian>(width as u32)?;
                        gump_art_data.write_u32::<LittleEndian>(height as u32)?;
                        gump_art_data.write_all(&data)?;
                        data = gump_art_data;
                    }

                    let identifier_str = if file_type == FileType::GumpartLegacyMul
                        && idx_entries[j].id == 9834
                    {
                        hash_format_1.replace("{0:0000000}", &format!("{:07}", idx_entries[j].id))
                    } else {
                        hash_format_0
                            .replace("{0:00000000}", &format!("{:08}", idx_entries[j].id))
                    };
                    table_entries[table_idx].identifier = hash_file_name_single(&identifier_str);

                    let mut final_data = data;
                    let mut compressed_size = size_decompressed;

                    match current_compression_flag {
                        CompressionFlag::Zlib => {
                            let mut encoder =
                                ZlibEncoder::new(final_data.as_slice(), Compression::default());
                            let mut compressed_data = Vec::new();
                            encoder.read_to_end(&mut compressed_data)?;
                            final_data = compressed_data;
                            compressed_size = final_data.len();
                        }
                        CompressionFlag::Mythic => {
                            let original_len = final_data.len() as u32;
                            let transformed_data = mythic_decompress::transform(&final_data);
                            let mut temp_data = Vec::new();
                            temp_data.write_u32::<LittleEndian>(original_len ^ 0x8E2C9A3D)?;
                            temp_data.write_all(&transformed_data)?;
                            final_data = temp_data;
                            compressed_size = final_data.len();
                        }
                        CompressionFlag::ZlibBwt => {
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                "ZlibBwt compression is not supported.",
                            ));
                        }
                        CompressionFlag::None => {}
                    }

                    table_entries[table_idx].offset = writer.stream_position()?;
                    table_entries[table_idx].size = compressed_size as u32;
                    table_entries[table_idx].decompressed_size = size_decompressed as u32;
                    table_entries[table_idx].compression_flag = current_compression_flag as i16;
                    table_entries[table_idx].hash = hash_data_block(&final_data)?;
                    writer.write_all(&final_data)?;
                }
                table_idx += 1;
            }

            let next_table = writer.stream_position()?;

            if i < table_count - 1 {
                writer.seek(SeekFrom::Start(this_table + 4))?;
                writer.write_u64::<LittleEndian>(next_table)?;
            } else {
                writer.seek(SeekFrom::Start(this_table + 12))?;
            }

            table_idx = 0;
            for _j in idx_start..idx_end {
                writer.write_u64::<LittleEndian>(table_entries[table_idx].offset)?;
                writer.write_u32::<LittleEndian>(table_entries[table_idx].header_length)?;
                writer.write_u32::<LittleEndian>(table_entries[table_idx].size)?;
                writer.write_u32::<LittleEndian>(table_entries[table_idx].decompressed_size)?;
                writer.write_u64::<LittleEndian>(table_entries[table_idx].identifier)?;
                writer.write_u32::<LittleEndian>(table_entries[table_idx].hash)?;
                writer.write_i16::<LittleEndian>(table_entries[table_idx].compression_flag)?;
                table_idx += 1;
            }

            let empty_table_entry_bytes = [0u8; 8 + 4 + 4 + 4 + 8 + 4 + 2];
            for _ in table_idx..Self::TABLE_SIZE as usize {
                writer.write_all(&empty_table_entry_bytes)?;
            }

            writer.seek(SeekFrom::Start(next_table))?;
        }

        Ok(())
    }

    pub fn from_uop(
        in_file: &Path,
        out_file: &Path,
        out_file_idx: Option<&Path>,
        file_type: FileType,
        type_index: i32,
        housing_bin_file: Option<&Path>,
    ) -> io::Result<()> {
        let mut chunk_ids: HashMap<u64, i32> = HashMap::new();
        let mut chunk_ids2: HashMap<u64, i32> = HashMap::new();

        let (format_0, format_1, max_id) = Self::get_hash_format(file_type, type_index);

        for i in 0..max_id {
            let identifier_str = format_0.replace("{0:00000000}", &format!("{:08}", i));
            chunk_ids.insert(hash_file_name_single(&identifier_str), i);
        }

        if !format_1.is_empty() {
            for i in 0..max_id {
                let identifier_str = format_1.replace("{0:0000000}", &format!("{:07}", i));
                chunk_ids2.insert(hash_file_name_single(&identifier_str), i);
            }
        }

        let mut used = vec![false; max_id as usize];

        let mut reader = BufReader::new(File::open(in_file)?);

        let mut mul_writer = BufWriter::new(File::create(out_file)?);
        let mut idx_writer = if let Some(path) = out_file_idx {
            Some(BufWriter::new(File::create(path)?))
        } else {
            None
        };

        if reader.read_u32::<LittleEndian>()? != 0x50594D {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "inFile is not a UOP file.",
            ));
        }

        reader.read_u32::<LittleEndian>()?;
        reader.read_u32::<LittleEndian>()?;

        let mut next_table = reader.read_u64::<LittleEndian>()?;

        while next_table != 0 {
            reader.seek(SeekFrom::Start(next_table))?;
            let entries = reader.read_u32::<LittleEndian>()?;
            next_table = reader.read_u64::<LittleEndian>()?;

            let mut offsets: Vec<TableEntry> = Vec::with_capacity(entries as usize);
            for _ in 0..entries {
                let offset = reader.read_u64::<LittleEndian>()?;
                let header_length = reader.read_u32::<LittleEndian>()?;
                let size = reader.read_u32::<LittleEndian>()?;
                let decompressed_size = reader.read_u32::<LittleEndian>()?;
                let identifier = reader.read_u64::<LittleEndian>()?;
                let hash = reader.read_u32::<LittleEndian>()?;
                let compression_flag_val = reader.read_i16::<LittleEndian>()?;

                offsets.push(TableEntry {
                    offset,
                    header_length,
                    size,
                    decompressed_size,
                    identifier,
                    hash,
                    compression_flag: compression_flag_val,
                });
            }

            for entry in offsets {
                if entry.offset == 0 {
                    continue;
                }

                if file_type == FileType::MultiCollection
                    && entry.identifier == 0x126D1E99DDEDEE0A
                    && housing_bin_file.is_some()
                {
                    let mut housing_writer =
                        BufWriter::new(File::create(housing_bin_file.unwrap())?);
                    reader.seek(SeekFrom::Start(entry.offset + entry.header_length as u64))?;
                    let mut bin_data = vec![0; entry.size as usize];
                    reader.read_exact(&mut bin_data)?;

                    let bin_data_to_write = match CompressionFlag::from(entry.compression_flag) {
                        CompressionFlag::Zlib => {
                            let mut decoder = ZlibDecoder::new(bin_data.as_slice());
                            let mut decompressed = vec![0; entry.decompressed_size as usize];
                            decoder.read_exact(&mut decompressed)?;
                            decompressed
                        }
                        CompressionFlag::Mythic => {
                            mythic_decompress::decompress_with_header(&bin_data)?
                        }
                        CompressionFlag::ZlibBwt => zlib_bwt_codec::decompress(&bin_data)?,
                        CompressionFlag::None => bin_data,
                    };
                    housing_writer.write_all(&bin_data_to_write)?;
                    continue;
                }

                let chunk_id = if let Some(&id) = chunk_ids.get(&entry.identifier) {
                    id
                } else if let Some(&id) = chunk_ids2.get(&entry.identifier) {
                    id
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Unknown identifier encountered ({:X})", entry.identifier),
                    ));
                };

                reader.seek(SeekFrom::Start(entry.offset + entry.header_length as u64))?;
                let mut chunk_data = vec![0; entry.size as usize];
                reader.read_exact(&mut chunk_data)?;

                let decompressed_chunk_data = match CompressionFlag::from(entry.compression_flag) {
                    CompressionFlag::Zlib => {
                        let mut decoder = ZlibDecoder::new(chunk_data.as_slice());
                        let mut decompressed = vec![0; entry.decompressed_size as usize];
                        decoder.read_exact(&mut decompressed)?;
                        decompressed
                    }
                    CompressionFlag::Mythic => {
                        mythic_decompress::decompress_with_header(&chunk_data)?
                    }
                    CompressionFlag::ZlibBwt => zlib_bwt_codec::decompress(&chunk_data)?,
                    CompressionFlag::None => chunk_data,
                };

                if file_type == FileType::MapLegacyMul {
                    mul_writer.seek(SeekFrom::Start(chunk_id as u64 * 0xC4000))?;
                    mul_writer.write_all(&decompressed_chunk_data)?;
                } else {
                    let mut data_offset = 0;
                    if let Some(idx_writer) = idx_writer.as_mut() {
                        idx_writer.seek(SeekFrom::Start(chunk_id as u64 * 12))?;
                        idx_writer
                            .write_u32::<LittleEndian>(mul_writer.stream_position()? as u32)?;

                        match file_type {
                            FileType::GumpartLegacyMul => {
                                let width = u32::from_le_bytes(
                                    decompressed_chunk_data[0..4].try_into().unwrap(),
                                );
                                let height = u32::from_le_bytes(
                                    decompressed_chunk_data[4..8].try_into().unwrap(),
                                );
                                idx_writer.write_u32::<LittleEndian>(
                                    (decompressed_chunk_data.len() - 8) as u32,
                                )?;
                                idx_writer.write_u32::<LittleEndian>((width << 16) | height)?;
                                data_offset = 8;
                            }
                            FileType::SoundLegacyMul => {
                                idx_writer.write_u32::<LittleEndian>(
                                    decompressed_chunk_data.len() as u32,
                                )?;
                                idx_writer.write_u32::<LittleEndian>((chunk_id + 1) as u32)?;
                            }
                            FileType::MultiCollection => {
                                let start_position = mul_writer.stream_position()?;
                                Self::write_multi_uop_entry_to_mul(
                                    &mut mul_writer,
                                    &decompressed_chunk_data,
                                )?;
                                let end_position = mul_writer.stream_position()?;
                                idx_writer.write_u32::<LittleEndian>(
                                    (end_position - start_position) as u32,
                                )?;
                                idx_writer.write_u32::<LittleEndian>(0)?;
                            }
                            _ => {
                                idx_writer.write_u32::<LittleEndian>(
                                    decompressed_chunk_data.len() as u32,
                                )?;
                                idx_writer.write_u32::<LittleEndian>(0)?;
                            }
                        }
                        used[chunk_id as usize] = true;
                    }

                    if file_type != FileType::MultiCollection {
                        mul_writer.write_all(&decompressed_chunk_data[data_offset..])?;
                    }
                }
            }
        }

        if let Some(idx_writer) = idx_writer.as_mut() {
            for (i, is_used) in used.iter().enumerate() {
                if *is_used {
                    continue;
                }
                idx_writer.seek(SeekFrom::Start(i as u64 * 12))?;
                idx_writer.write_i32::<LittleEndian>(-1)?;
                idx_writer.write_u64::<LittleEndian>(0)?;
            }
        }

        Self::check_and_fix_map_files(out_file, file_type, type_index)?;

        Ok(())
    }

    fn check_and_fix_map_files(
        out_file: &Path,
        file_type: FileType,
        type_index: i32,
    ) -> io::Result<()> {
        if file_type != FileType::MapLegacyMul {
            return Ok(());
        }

        let expected_size = Self::get_expected_map_file_size(type_index);

        if expected_size == 0 {
            return Ok(());
        }

        let map_file = File::options().read(true).write(true).open(out_file)?;
        let current_size = map_file.metadata()?.len();
        let size_diff = current_size as i64 - expected_size as i64;

        if size_diff > 0 {
            map_file.set_len(current_size - size_diff as u64)?;
        }

        Ok(())
    }

    fn get_expected_map_file_size(type_index: i32) -> u64 {
        match type_index {
            0 => 89_915_392,
            1 => 89_915_392,
            2 => 11_289_600,
            3 => 16_056_320,
            4 => 6_421_156,
            5 => 16_056_320,
            _ => 0,
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
        let count = u32::from_le_bytes(span[4..8].try_into().unwrap());
        span = &span[8..];

        for _ in 0..count {
            let item_id = u16::from_le_bytes(span[0..2].try_into().unwrap());
            let x = i16::from_le_bytes(span[2..4].try_into().unwrap());
            let y = i16::from_le_bytes(span[4..6].try_into().unwrap());
            let z = i16::from_le_bytes(span[6..8].try_into().unwrap());

            let flag_value = u16::from_le_bytes(span[8..10].try_into().unwrap());
            let clilocs_count = u32::from_le_bytes(span[10..14].try_into().unwrap());

            let skip = (clilocs_count as usize).min(i32::MAX as usize) * 4;
            span = &span[(14 + skip)..];

            mul_writer.write_u16::<LittleEndian>(item_id)?;
            mul_writer.write_i16::<LittleEndian>(x)?;
            mul_writer.write_i16::<LittleEndian>(y)?;
            mul_writer.write_i16::<LittleEndian>(z)?;
            mul_writer.write_u32::<LittleEndian>(if flag_value != 0 { 0 } else { 1 })?;
            mul_writer.write_u32::<LittleEndian>(0)?;
        }
        Ok(())
    }
}
