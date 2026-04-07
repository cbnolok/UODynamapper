// cc_uop_converter.rs

// Credits:
// "Mythic" decompression from UOFiddler's MythicDecompress.cs.
// BWT decompression from ClassicUO.
// Most of the other code from LegacyMUL(CL)-N.

// TODO: use uocf::uop library instead of reinventing the wheel.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use flate2::{
    Compression,
    read::{ZlibDecoder, ZlibEncoder},
};

use clap::{Parser, Subcommand};
use color_eyre::eyre;

use uocf_tools::art_uddp::{
    CcArtAtlasOptions, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_HEIGHT,
    DEFAULT_ATLAS_PAGE_WIDTH, convert_art_mul_to_cc_art_uddp,
};
use uocf::uop::{
    compression::{mythic_decompress, zlib_bwt_codec},
    hash::{hash_data_block, hash_file_name_single},
};

// This is the FileType enum provided by the user
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
    Mythic = 2,  // Re-added Mythic
    ZlibBwt = 3, // ZlibBwt
}

impl From<i16> for CompressionFlag {
    fn from(value: i16) -> Self {
        match value {
            0 => CompressionFlag::None,
            1 => CompressionFlag::Zlib,
            2 => CompressionFlag::Mythic,
            3 => CompressionFlag::ZlibBwt,
            _ => CompressionFlag::None, // Default or error handling
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
    compressed: bool,
}

pub struct LegacyMulFileConverter;

impl LegacyMulFileConverter {
    const FIRST_TABLE: u64 = 0x200;
    const TABLE_SIZE: u32 = 0x64; // 100 entries

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
            reader.seek(SeekFrom::Start(0))?; // Reset reader position

            let mut position = 0;
            let mut id = 0;
            while position < length {
                idx_entries.push(IdxEntry {
                    id,
                    offset: position,
                    size: 0xC4000, // 800 KB chunks
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
            reader_idx.seek(SeekFrom::Start(0))?; // Reset reader_idx position

            for i in 0..idx_entry_count {
                let offset = reader_idx.read_i32::<LittleEndian>()?;
                if offset < 0 {
                    reader_idx.seek(SeekFrom::Current(8))?; // skip
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
            file_count += 1; // for "housing.bin"
            idx_entries.push(IdxEntry {
                // Add a dummy entry for housing.bin
                id: -1, // This ID won't be used for reading from MUL/IDX
                offset: 0,
                size: 0,
                extra: 0,
            });
        }

        // File header
        writer.write_u32::<LittleEndian>(0x50594D)?; // MYP
        writer.write_u32::<LittleEndian>(match file_type {
            FileType::GumpartLegacyMul => 4,
            _ => 5,
        })?; // version
        writer.write_u32::<LittleEndian>(0xFD23EC43)?; // format timestamp?
        writer.write_u64::<LittleEndian>(match file_type {
            FileType::GumpartLegacyMul => 0x28,
            _ => Self::FIRST_TABLE,
        })?; // first table
        writer.write_u32::<LittleEndian>(Self::TABLE_SIZE)?; // table size
        writer.write_u32::<LittleEndian>(file_count as u32)?; // file count (CHANGED)
        writer.write_u32::<LittleEndian>(1)?; // modified count? (CHANGED from 0)
        writer.write_u32::<LittleEndian>(1)?; // ? (CHANGED from 0)
        writer.write_u32::<LittleEndian>(0)?; // ?

        // Padding
        if file_type != FileType::GumpartLegacyMul {
            for _ in 0x28..Self::FIRST_TABLE {
                writer.write_u8(0)?;
            }
        }

        let table_count = (file_count as f64 / Self::TABLE_SIZE as f64).ceil() as usize; // Use file_count
        let mut table_entries: Vec<TableEntry> = vec![
            TableEntry {
                offset: 0,
                header_length: 0,
                size: 0,
                decompressed_size: 0,
                identifier: 0,
                hash: 0,
                compression_flag: 0,
                compressed: false,
            };
            Self::TABLE_SIZE as usize
        ];

        let (hash_format_0, hash_format_1, _max_id) = Self::get_hash_format(file_type, type_index);

        for i in 0..table_count {
            let this_table = writer.stream_position()?;

            let idx_start = i * Self::TABLE_SIZE as usize;
            let idx_end = ((i + 1) * Self::TABLE_SIZE as usize).min(file_count);

            // Table header
            writer.write_u32::<LittleEndian>((idx_end - idx_start) as u32)?;
            writer.write_u64::<LittleEndian>(0)?; // next table, filled in later
            writer.seek(SeekFrom::Current(34 * Self::TABLE_SIZE as i64))?; // table entries, filled in later

            // Data
            let mut table_idx = 0;
            for j in idx_start..idx_end {
                let mut data: Vec<u8>;
                let mut size_decompressed: usize;
                let current_compression_flag = if matches!(
                    file_type,
                    FileType::ArtLegacyMul
                        | FileType::GumpartLegacyMul
                        | FileType::MapLegacyMul
                        | FileType::SoundLegacyMul
                ) {
                    CompressionFlag::None // Force no compression for these types
                } else {
                    compression_flag
                };

                if file_type == FileType::MultiCollection && j == file_count - 1 {
                    // This is the housing.bin entry
                    let housing_path = Path::new("housing.bin"); // Assumes housing.bin is in the current directory
                    if housing_path.exists() {
                        data = std::fs::read(housing_path)?;
                        size_decompressed = data.len();

                        table_entries[table_idx].identifier =
                            hash_file_name_single("build/multicollection/housing.bin");
                        table_entries[table_idx].hash = hash_data_block(&data)?;
                        table_entries[table_idx].offset = writer.stream_position()?;
                        table_entries[table_idx].size = data.len() as u32;
                        table_entries[table_idx].decompressed_size = size_decompressed as u32;
                        table_entries[table_idx].compression_flag = CompressionFlag::None as i16; // housing.bin is not compressed in UOP
                        table_entries[table_idx].compressed = false;
                        writer.write_all(&data)?;
                    } else {
                        // If housing.bin doesn't exist, write an empty entry
                        table_entries[table_idx].identifier =
                            hash_file_name_single("build/multicollection/housing.bin");
                        table_entries[table_idx].hash = 0;
                        table_entries[table_idx].offset = 0;
                        table_entries[table_idx].size = 0;
                        table_entries[table_idx].decompressed_size = 0;
                        table_entries[table_idx].compression_flag = CompressionFlag::None as i16;
                        table_entries[table_idx].compressed = false;
                    }
                } else {
                    // Normal MUL entry
                    reader.seek(SeekFrom::Start(idx_entries[j].offset as u64))?;
                    data = vec![0; idx_entries[j].size as usize];
                    reader.read_exact(&mut data)?;
                    size_decompressed = data.len();

                    if file_type == FileType::GumpartLegacyMul {
                        /*
                        let width = u32::from_le_bytes(
                                decompressed_chunk_data[0..4].try_into().unwrap(),
                            );
                        let height = u32::from_le_bytes(
                                decompressed_chunk_data[4..8].try_into().unwrap(),
                            );
                        */
                        let width = (idx_entries[j].extra >> 16) & 0xFFFF;
                        let height = (idx_entries[j].extra) & 0xFFFF;

                        let mut gump_art_data = Vec::new();
                        gump_art_data.write_u32::<LittleEndian>(width as u32)?;
                        gump_art_data.write_u32::<LittleEndian>(height as u32)?;
                        gump_art_data.write_all(&data)?;
                        data = gump_art_data;
                        size_decompressed = data.len();
                    }

                    let identifier_str =
                        if file_type == FileType::GumpartLegacyMul && idx_entries[j].id == 9834 {
                            format!(
                                "{}",
                                hash_format_1
                                    .replace("{0:0000000}", &format!("{:07}", idx_entries[j].id))
                            )
                        } else {
                            format!(
                                "{}",
                                hash_format_0
                                    .replace("{0:00000000}", &format!("{:08}", idx_entries[j].id))
                            )
                        };
                    table_entries[table_idx].identifier = hash_file_name_single(&identifier_str);

                    let mut final_data = data;
                    let mut compressed_size = size_decompressed;
                    let mut is_compressed = false;

                    match current_compression_flag {
                        CompressionFlag::Zlib => {
                            let mut encoder =
                                ZlibEncoder::new(final_data.as_slice(), Compression::default());
                            let mut compressed_data = Vec::new();
                            encoder.read_to_end(&mut compressed_data)?;
                            final_data = compressed_data;
                            compressed_size = final_data.len();
                            is_compressed = true;
                        }
                        CompressionFlag::Mythic => {
                            // Handle Mythic compression
                            let original_len = final_data.len() as u32;
                            let transformed_data = mythic_decompress::transform(&final_data);
                            let mut temp_data = Vec::new();
                            temp_data.write_u32::<LittleEndian>(original_len ^ 0x8E2C9A3D)?;
                            temp_data.write_all(&transformed_data)?;
                            final_data = temp_data;
                            compressed_size = final_data.len();
                            is_compressed = true;
                        }
                        CompressionFlag::ZlibBwt => {
                            // Handle ZlibBwt compression (unsupported)
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                "ZlibBwt compression is not supported.",
                            ));
                        }
                        CompressionFlag::None => { /* No compression */ }
                    }

                    table_entries[table_idx].offset = writer.stream_position()?;
                    table_entries[table_idx].size = compressed_size as u32;
                    table_entries[table_idx].decompressed_size = size_decompressed as u32;
                    table_entries[table_idx].compression_flag = current_compression_flag as i16;
                    table_entries[table_idx].compressed = is_compressed;
                    table_entries[table_idx].hash = hash_data_block(&final_data)?;
                    writer.write_all(&final_data)?;
                }
                table_idx += 1;
            }

            let next_table = writer.stream_position()?;

            // Go back and fix table header
            if i < table_count - 1 {
                writer.seek(SeekFrom::Start(this_table + 4))?;
                writer.write_u64::<LittleEndian>(next_table)?;
            } else {
                writer.seek(SeekFrom::Start(this_table + 12))?;
                // No need to fix the next table address, it's the last
            }

            // Table entries
            table_idx = 0;
            for _j in idx_start..idx_end {
                writer.write_u64::<LittleEndian>(table_entries[table_idx].offset)?;
                writer.write_u32::<LittleEndian>(0)?; // header length
                writer.write_u32::<LittleEndian>(table_entries[table_idx].size)?; // compressed size
                writer.write_u32::<LittleEndian>(table_entries[table_idx].decompressed_size)?; // decompressed size
                writer.write_u64::<LittleEndian>(table_entries[table_idx].identifier)?;
                writer.write_u32::<LittleEndian>(table_entries[table_idx].hash)?;
                writer.write_i16::<LittleEndian>(table_entries[table_idx].compression_flag)?; // compression method
                table_idx += 1;
            }

            // Fill remainder with empty entries
            let empty_table_entry_bytes = [0u8; 8 + 4 + 4 + 4 + 8 + 4 + 2]; // Corresponds to C# _emptyTableEntry
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
            let identifier_str =
                format!("{}", format_0.replace("{0:00000000}", &format!("{:08}", i)));
            chunk_ids.insert(hash_file_name_single(&identifier_str), i);
        }

        if !format_1.is_empty() {
            for i in 0..max_id {
                let identifier_str =
                    format!("{}", format_1.replace("{0:0000000}", &format!("{:07}", i)));
                chunk_ids2.insert(hash_file_name_single(&identifier_str), i);
            }
        }

        let mut used = vec![false; max_id as usize];

        // TODO: do not reimplement UOP file parsing, but use our uop library.
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

        reader.read_u32::<LittleEndian>()?; // version ?
        reader.read_u32::<LittleEndian>()?; // format timestamp? 0xFD23EC43

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
                let compressed = compression_flag_val != 0;

                offsets.push(TableEntry {
                    offset,
                    header_length,
                    size,
                    decompressed_size,
                    identifier,
                    hash,
                    compression_flag: compression_flag_val,
                    compressed,
                });
            }

            for entry in offsets {
                if entry.offset == 0 {
                    continue; // skip empty entry
                }

                // extract housing.bin file
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
                            // Handle Mythic decompression
                            mythic_decompress::decompress_with_header(&bin_data)?
                        }
                        CompressionFlag::ZlibBwt => {
                            // Handle ZlibBwt decompression
                            zlib_bwt_codec::decompress(&bin_data)?
                        }
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
                        // Handle Mythic decompression
                        mythic_decompress::decompress_with_header(&chunk_data)?
                    }
                    CompressionFlag::ZlibBwt => {
                        // Handle ZlibBwt decompression
                        zlib_bwt_codec::decompress(&chunk_data)?
                    }
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
                            .write_u32::<LittleEndian>(mul_writer.stream_position()? as u32)?; // Position

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
                                )?; // Size
                                idx_writer.write_u32::<LittleEndian>(0)?; // Extra
                            }
                            _ => {
                                idx_writer.write_u32::<LittleEndian>(
                                    decompressed_chunk_data.len() as u32,
                                )?; // Size
                                idx_writer.write_u32::<LittleEndian>(0)?; // Extra
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

        // Fix index
        if let Some(idx_writer) = idx_writer.as_mut() {
            for i in 0..used.len() {
                if used[i] {
                    continue;
                }
                idx_writer.seek(SeekFrom::Start(i as u64 * 12))?;
                idx_writer.write_i32::<LittleEndian>(-1)?; // Position (lookup)
                idx_writer.write_u64::<LittleEndian>(0)?; // Size + Extra
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
        let mut max_id = 0x7FFFF; // Default

        let (format_0, format_1) = match file_type {
            FileType::ArtLegacyMul => {
                max_id = 0x13FDC;
                (
                    "build/artlegacymul/{0:00000000}.tga".to_string(),
                    String::new(),
                )
            }
            FileType::GumpartLegacyMul => {
                // maxId = 0xEF3C on 7.0.8.2
                (
                    "build/gumpartlegacymul/{0:00000000}.tga".to_string(),
                    "build/gumpartlegacymul/{0:0000000}.tga".to_string(),
                )
            }
            FileType::MapLegacyMul => {
                // maxId = 0x71 on 7.0.8.2 for Fel/Tram
                (
                    format!("build/map{}legacymul/{{0:00000000}}.dat", type_index),
                    String::new(),
                )
            }
            FileType::SoundLegacyMul => {
                // maxId = 0x1000 on 7.0.8.2
                (
                    "build/soundlegacymul/{0:00000000}.dat".to_string(),
                    String::new(),
                )
            }
            FileType::MultiCollection => {
                max_id = 0x2200; // seems like this is reasonable limit for multis
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

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extracts known UOP files into MUL format.
    Extract {
        #[arg(name = "path")]
        path: PathBuf,
    },
    /// Packs known MUL files into UOP format.
    Pack {
        #[arg(name = "path")]
        path: PathBuf,
    },
    /// Packs classic art.mul/artidx.mul into cc_art.uddp atlas pages.
    PackArtUddp {
        #[arg(name = "path")]
        path: PathBuf,
        #[arg(long, default_value = "cc_art.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
    },
}

fn print_results(success: u32, total: u32) {
    println!();
    if success < total {
        println!("Errors: {}", total - success);
    } else {
        println!("All actions completed successfully.");
    }
}

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let mut success_count = 0;
    let mut total_count = 0;

    match &cli.command {
        Commands::Extract { path } => {
            println!("Mode: Extract from UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                eprintln!("Directory '{}' does not exist!", path.display());
                return Ok(());
            }

            let uop_dir = path;

            // Extract artLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("artLegacyMUL.uop"),
                &uop_dir.join("art.mul"),
                Some(&uop_dir.join("artidx.mul")),
                FileType::ArtLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting artLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract gumpartLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("gumpartLegacyMUL.uop"),
                &uop_dir.join("gumpart.mul"),
                Some(&uop_dir.join("gumpidx.mul")),
                FileType::GumpartLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting gumpartLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract MultiCollection.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("MultiCollection.uop"),
                &uop_dir.join("multi.mul"),
                Some(&uop_dir.join("multiidx.mul")),
                FileType::MultiCollection,
                0,
                Some(&uop_dir.join("housing.bin")),
            ) {
                eprintln!("Error extracting MultiCollection.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract soundLegacyMUL.uop
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::from_uop(
                &uop_dir.join("soundLegacyMUL.uop"),
                &uop_dir.join("sound.mul"),
                Some(&uop_dir.join("soundidx.mul")),
                FileType::SoundLegacyMul,
                0,
                None,
            ) {
                eprintln!("Error extracting soundLegacyMUL.uop: {}", e);
            } else {
                success_count += 1;
            }

            // Extract maps
            for i in 0..=5 {
                total_count += 1;
                let map_name = format!("map{}", i);
                if let Err(e) = LegacyMulFileConverter::from_uop(
                    &uop_dir.join(format!("{}LegacyMUL.uop", map_name)),
                    &uop_dir.join(format!("{}.mul", map_name)),
                    None,
                    FileType::MapLegacyMul,
                    i,
                    None,
                ) {
                    eprintln!("Error extracting {}LegacyMUL.uop: {}", map_name, e);
                } else {
                    success_count += 1;
                }

                total_count += 1;
                let map_x_name = format!("map{}x", i);
                if let Err(e) = LegacyMulFileConverter::from_uop(
                    &uop_dir.join(format!("{}LegacyMUL.uop", map_x_name)),
                    &uop_dir.join(format!("{}.mul", map_x_name)),
                    None,
                    FileType::MapLegacyMul,
                    i,
                    None,
                ) {
                    eprintln!("Error extracting {}LegacyMUL.uop: {}", map_x_name, e);
                } else {
                    success_count += 1;
                }
            }
        }
        Commands::Pack { path } => {
            println!("Mode: Pack to UOP.");
            println!();

            if !path.exists() || !path.is_dir() {
                eprintln!("Directory '{}' does not exist!", path.display());
                return Ok(());
            }

            let mul_dir = path;

            // Pack art.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("art.mul"),
                Some(&mul_dir.join("artidx.mul")),
                &mul_dir.join("artLegacyMUL.uop"),
                FileType::ArtLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing art.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack gumpart.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("gumpart.mul"),
                Some(&mul_dir.join("gumpidx.mul")),
                &mul_dir.join("gumpartLegacyMUL.uop"),
                FileType::GumpartLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing gumpart.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack multi.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("multi.mul"),
                Some(&mul_dir.join("multiidx.mul")),
                &mul_dir.join("MultiCollection.uop"),
                FileType::MultiCollection,
                0,
                CompressionFlag::None, // housing.bin is not compressed
            ) {
                eprintln!("Error packing multi.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack sound.mul
            total_count += 1;
            if let Err(e) = LegacyMulFileConverter::to_uop(
                &mul_dir.join("sound.mul"),
                Some(&mul_dir.join("soundidx.mul")),
                &mul_dir.join("soundLegacyMUL.uop"),
                FileType::SoundLegacyMul,
                0,
                CompressionFlag::Zlib,
            ) {
                eprintln!("Error packing sound.mul: {}", e);
            } else {
                success_count += 1;
            }

            // Pack maps
            for i in 0..=5 {
                total_count += 1;
                let map_name = format!("map{}", i);
                if let Err(e) = LegacyMulFileConverter::to_uop(
                    &mul_dir.join(format!("{}.mul", map_name)),
                    None,
                    &mul_dir.join(format!("{}LegacyMUL.uop", map_name)),
                    FileType::MapLegacyMul,
                    i,
                    CompressionFlag::None,
                ) {
                    eprintln!("Error packing {}.mul: {}", map_name, e);
                } else {
                    success_count += 1;
                }

                total_count += 1;
                let map_x_name = format!("map{}x", i);
                if let Err(e) = LegacyMulFileConverter::to_uop(
                    &mul_dir.join(format!("{}.mul", map_x_name)),
                    None,
                    &mul_dir.join(format!("{}LegacyMUL.uop", map_x_name)),
                    FileType::MapLegacyMul,
                    i,
                    CompressionFlag::None,
                ) {
                    eprintln!("Error packing {}.mul: {}", map_x_name, e);
                } else {
                    success_count += 1;
                }
            }
        }
        Commands::PackArtUddp {
            path,
            output,
            atlas_width,
            atlas_height,
            gutter,
        } => {
            println!("Mode: Pack art.mul to cc_art.uddp.");
            println!();

            if !path.exists() || !path.is_dir() {
                eprintln!("Directory '{}' does not exist!", path.display());
                return Ok(());
            }

            total_count += 1;
            let out_file = if output.is_absolute() {
                output.clone()
            } else {
                path.join(output)
            };
            let options = CcArtAtlasOptions {
                atlas_width: *atlas_width,
                atlas_height: *atlas_height,
                gutter: *gutter,
            };

            match convert_art_mul_to_cc_art_uddp(path, &out_file, &options) {
                Ok(summary) => {
                    println!(
                        "Wrote {} pages for {} populated slots out of {} total slots to '{}'.",
                        summary.page_count,
                        summary.populated_slot_count,
                        summary.slot_count,
                        out_file.display()
                    );
                    success_count += 1;
                }
                Err(error) => {
                    eprintln!("Error packing cc_art.uddp: {error}");
                }
            }
        }
    }

    print_results(success_count, total_count);

    Ok(())
}
