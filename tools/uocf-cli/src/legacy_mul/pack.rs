use super::*;

pub(super) fn to_uop(
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

    let mut idx_entries = read_idx_entries(&mut reader, reader_idx.as_mut(), file_type)?;
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

    write_uop_header(&mut writer, file_type, file_count as u32)?;

    let table_count = (file_count as f64 / LegacyMulFileConverter::TABLE_SIZE as f64).ceil() as usize;
    let mut table_entries = vec![
        TableEntry {
            offset: 0,
            header_length: 0,
            size: 0,
            decompressed_size: 0,
            identifier: 0,
            hash: 0,
            compression_flag: 0,
        };
        LegacyMulFileConverter::TABLE_SIZE as usize
    ];

    let (hash_format_0, hash_format_1, _max_id) = get_hash_format(file_type, type_index);

    for i in 0..table_count {
        let this_table = writer.stream_position()?;
        let idx_start = i * LegacyMulFileConverter::TABLE_SIZE as usize;
        let idx_end = ((i + 1) * LegacyMulFileConverter::TABLE_SIZE as usize).min(file_count);

        writer.write_u32::<LittleEndian>((idx_end - idx_start) as u32)?;
        writer.write_u64::<LittleEndian>(0)?;
        writer.seek(SeekFrom::Current(34 * LegacyMulFileConverter::TABLE_SIZE as i64))?;

        for (table_idx, idx_entry) in idx_entries[idx_start..idx_end].iter().enumerate() {
            write_table_entry_data(
                &mut reader,
                &mut writer,
                &mut table_entries[table_idx],
                idx_entry,
                file_type,
                file_count,
                idx_start + table_idx,
                &hash_format_0,
                &hash_format_1,
                compression_flag,
            )?;
        }

        let next_table = writer.stream_position()?;

        if i < table_count - 1 {
            writer.seek(SeekFrom::Start(this_table + 4))?;
            writer.write_u64::<LittleEndian>(next_table)?;
        } else {
            writer.seek(SeekFrom::Start(this_table + 12))?;
        }

        write_table_entries(&mut writer, &table_entries, idx_end - idx_start)?;
        writer.seek(SeekFrom::Start(next_table))?;
    }

    Ok(())
}

fn read_idx_entries(
    reader: &mut BufReader<File>,
    mut reader_idx: Option<&mut BufReader<File>>,
    file_type: FileType,
) -> io::Result<Vec<IdxEntry>> {
    let mut idx_entries = Vec::new();

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

        return Ok(idx_entries);
    }

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

    Ok(idx_entries)
}

fn write_uop_header(
    writer: &mut BufWriter<File>,
    file_type: FileType,
    file_count: u32,
) -> io::Result<()> {
    writer.write_u32::<LittleEndian>(0x50594D)?;
    writer.write_u32::<LittleEndian>(match file_type {
        FileType::GumpartLegacyMul => 4,
        _ => 5,
    })?;
    writer.write_u32::<LittleEndian>(0xFD23EC43)?;
    writer.write_u64::<LittleEndian>(match file_type {
        FileType::GumpartLegacyMul => 0x28,
        _ => LegacyMulFileConverter::FIRST_TABLE,
    })?;
    writer.write_u32::<LittleEndian>(LegacyMulFileConverter::TABLE_SIZE)?;
    writer.write_u32::<LittleEndian>(file_count)?;
    writer.write_u32::<LittleEndian>(1)?;
    writer.write_u32::<LittleEndian>(1)?;
    writer.write_u32::<LittleEndian>(0)?;

    if file_type != FileType::GumpartLegacyMul {
        for _ in 0x28..LegacyMulFileConverter::FIRST_TABLE {
            writer.write_u8(0)?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_table_entry_data(
    reader: &mut BufReader<File>,
    writer: &mut BufWriter<File>,
    table_entry: &mut TableEntry,
    idx_entry: &IdxEntry,
    file_type: FileType,
    file_count: usize,
    entry_index: usize,
    hash_format_0: &str,
    hash_format_1: &str,
    compression_flag: CompressionFlag,
) -> io::Result<()> {
    if file_type == FileType::MultiCollection && entry_index == file_count - 1 {
        return write_housing_entry(writer, table_entry);
    }

    reader.seek(SeekFrom::Start(idx_entry.offset as u64))?;
    let mut data = vec![0; idx_entry.size as usize];
    reader.read_exact(&mut data)?;
    let size_decompressed = data.len();

    if file_type == FileType::GumpartLegacyMul {
        let width = (idx_entry.extra >> 16) & 0xFFFF;
        let height = idx_entry.extra & 0xFFFF;

        let mut gump_art_data = Vec::new();
        gump_art_data.write_u32::<LittleEndian>(width as u32)?;
        gump_art_data.write_u32::<LittleEndian>(height as u32)?;
        gump_art_data.write_all(&data)?;
        data = gump_art_data;
    }

    let identifier_str = if file_type == FileType::GumpartLegacyMul && idx_entry.id == 9834 {
        hash_format_1.replace("{0:0000000}", &format!("{:07}", idx_entry.id))
    } else {
        hash_format_0.replace("{0:00000000}", &format!("{:08}", idx_entry.id))
    };

    table_entry.identifier = hash_file_name_single(&identifier_str);

    let mut final_data = data;
    let mut compressed_size = size_decompressed;
    let current_compression_flag = compression_flag_for(file_type, compression_flag);

    match current_compression_flag {
        CompressionFlag::Zlib => {
            let mut encoder = ZlibEncoder::new(final_data.as_slice(), Compression::default());
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

    table_entry.offset = writer.stream_position()?;
    table_entry.size = compressed_size as u32;
    table_entry.decompressed_size = size_decompressed as u32;
    table_entry.compression_flag = current_compression_flag as i16;
    table_entry.hash = hash_data_block(&final_data)?;
    writer.write_all(&final_data)?;

    Ok(())
}

fn write_housing_entry(writer: &mut BufWriter<File>, table_entry: &mut TableEntry) -> io::Result<()> {
    table_entry.identifier = hash_file_name_single("build/multicollection/housing.bin");

    let housing_path = Path::new("housing.bin");
    if !housing_path.exists() {
        table_entry.hash = 0;
        table_entry.offset = 0;
        table_entry.size = 0;
        table_entry.decompressed_size = 0;
        table_entry.compression_flag = CompressionFlag::None as i16;
        return Ok(());
    }

    let data = std::fs::read(housing_path)?;
    table_entry.hash = hash_data_block(&data)?;
    table_entry.offset = writer.stream_position()?;
    table_entry.size = data.len() as u32;
    table_entry.decompressed_size = data.len() as u32;
    table_entry.compression_flag = CompressionFlag::None as i16;
    writer.write_all(&data)?;

    Ok(())
}

fn write_table_entries(
    writer: &mut BufWriter<File>,
    table_entries: &[TableEntry],
    used_entries: usize,
) -> io::Result<()> {
    for table_entry in &table_entries[..used_entries] {
        writer.write_u64::<LittleEndian>(table_entry.offset)?;
        writer.write_u32::<LittleEndian>(table_entry.header_length)?;
        writer.write_u32::<LittleEndian>(table_entry.size)?;
        writer.write_u32::<LittleEndian>(table_entry.decompressed_size)?;
        writer.write_u64::<LittleEndian>(table_entry.identifier)?;
        writer.write_u32::<LittleEndian>(table_entry.hash)?;
        writer.write_i16::<LittleEndian>(table_entry.compression_flag)?;
    }

    let empty_table_entry_bytes = [0u8; 8 + 4 + 4 + 4 + 8 + 4 + 2];
    for _ in used_entries..LegacyMulFileConverter::TABLE_SIZE as usize {
        writer.write_all(&empty_table_entry_bytes)?;
    }

    Ok(())
}
