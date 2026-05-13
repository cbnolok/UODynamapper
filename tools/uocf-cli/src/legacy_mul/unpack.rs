use super::*;

pub(super) fn from_uop(
    in_file: &Path,
    out_file: &Path,
    out_file_idx: Option<&Path>,
    file_type: FileType,
    type_index: i32,
    housing_bin_file: Option<&Path>,
) -> io::Result<()> {
    let (chunk_ids, chunk_ids2, max_id) = build_chunk_id_maps(file_type, type_index);
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

        let offsets = read_table_entries(&mut reader, entries)?;
        for entry in offsets {
            if entry.offset == 0 {
                continue;
            }

            if file_type == FileType::MultiCollection
                && entry.identifier == 0x126D1E99DDEDEE0A
                && housing_bin_file.is_some()
            {
                write_housing_bin(&mut reader, &entry, housing_bin_file.unwrap())?;
                continue;
            }

            let chunk_id = resolve_chunk_id(&chunk_ids, &chunk_ids2, entry.identifier)?;
            let decompressed_chunk_data = read_and_decompress_entry(&mut reader, &entry)?;

            if file_type == FileType::MapLegacyMul {
                mul_writer.seek(SeekFrom::Start(chunk_id as u64 * 0xC4000))?;
                mul_writer.write_all(&decompressed_chunk_data)?;
                continue;
            }

            write_non_map_chunk(
                &mut mul_writer,
                idx_writer.as_mut(),
                &decompressed_chunk_data,
                file_type,
                chunk_id,
            )?;
            used[chunk_id as usize] = true;
        }
    }

    if let Some(idx_writer) = idx_writer.as_mut() {
        write_unused_idx_entries(idx_writer, &used)?;
    }

    if file_type == FileType::MapLegacyMul {
        mul_writer.flush()?;
        let current_size = File::open(out_file)?.metadata()?.len();
        warn_if_non_standard_map_size(out_file, type_index, current_size);
    }

    Ok(())
}

fn build_chunk_id_maps(
    file_type: FileType,
    type_index: i32,
) -> (HashMap<u64, i32>, HashMap<u64, i32>, i32) {
    let mut chunk_ids = HashMap::new();
    let mut chunk_ids2 = HashMap::new();
    let (format_0, format_1, max_id) = get_hash_format(file_type, type_index);

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

    (chunk_ids, chunk_ids2, max_id)
}

fn read_table_entries(reader: &mut BufReader<File>, entries: u32) -> io::Result<Vec<TableEntry>> {
    let mut offsets = Vec::with_capacity(entries as usize);

    for _ in 0..entries {
        offsets.push(TableEntry {
            offset: reader.read_u64::<LittleEndian>()?,
            header_length: reader.read_u32::<LittleEndian>()?,
            size: reader.read_u32::<LittleEndian>()?,
            decompressed_size: reader.read_u32::<LittleEndian>()?,
            identifier: reader.read_u64::<LittleEndian>()?,
            hash: reader.read_u32::<LittleEndian>()?,
            compression_flag: reader.read_i16::<LittleEndian>()?,
        });
    }

    Ok(offsets)
}

fn resolve_chunk_id(
    chunk_ids: &HashMap<u64, i32>,
    chunk_ids2: &HashMap<u64, i32>,
    identifier: u64,
) -> io::Result<i32> {
    if let Some(&id) = chunk_ids.get(&identifier) {
        return Ok(id);
    }

    if let Some(&id) = chunk_ids2.get(&identifier) {
        return Ok(id);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Unknown identifier encountered ({:X})", identifier),
    ))
}

fn write_housing_bin(
    reader: &mut BufReader<File>,
    entry: &TableEntry,
    housing_path: &Path,
) -> io::Result<()> {
    let mut housing_writer = BufWriter::new(File::create(housing_path)?);
    let bin_data = read_and_decompress_entry(reader, entry)?;
    housing_writer.write_all(&bin_data)?;
    Ok(())
}

fn write_non_map_chunk(
    mul_writer: &mut BufWriter<File>,
    idx_writer: Option<&mut BufWriter<File>>,
    decompressed_chunk_data: &[u8],
    file_type: FileType,
    chunk_id: i32,
) -> io::Result<()> {
    let mut data_offset = 0;

    if let Some(idx_writer) = idx_writer {
        idx_writer.seek(SeekFrom::Start(chunk_id as u64 * 12))?;
        idx_writer.write_u32::<LittleEndian>(mul_writer.stream_position()? as u32)?;

        match file_type {
            FileType::GumpartLegacyMul => {
                let width = u32::from_le_bytes(decompressed_chunk_data[0..4].try_into().unwrap());
                let height = u32::from_le_bytes(decompressed_chunk_data[4..8].try_into().unwrap());
                idx_writer.write_u32::<LittleEndian>((decompressed_chunk_data.len() - 8) as u32)?;
                idx_writer.write_u32::<LittleEndian>((width << 16) | height)?;
                data_offset = 8;
            }
            FileType::SoundLegacyMul => {
                idx_writer.write_u32::<LittleEndian>(decompressed_chunk_data.len() as u32)?;
                idx_writer.write_u32::<LittleEndian>((chunk_id + 1) as u32)?;
            }
            FileType::MultiCollection => {
                let start_position = mul_writer.stream_position()?;
                write_multi_uop_entry_to_mul(mul_writer, decompressed_chunk_data)?;
                let end_position = mul_writer.stream_position()?;
                idx_writer.write_u32::<LittleEndian>((end_position - start_position) as u32)?;
                idx_writer.write_u32::<LittleEndian>(0)?;
            }
            _ => {
                idx_writer.write_u32::<LittleEndian>(decompressed_chunk_data.len() as u32)?;
                idx_writer.write_u32::<LittleEndian>(0)?;
            }
        }
    }

    if file_type != FileType::MultiCollection {
        mul_writer.write_all(&decompressed_chunk_data[data_offset..])?;
    }

    Ok(())
}

fn write_unused_idx_entries(idx_writer: &mut BufWriter<File>, used: &[bool]) -> io::Result<()> {
    for (i, is_used) in used.iter().enumerate() {
        if *is_used {
            continue;
        }

        idx_writer.seek(SeekFrom::Start(i as u64 * 12))?;
        idx_writer.write_i32::<LittleEndian>(-1)?;
        idx_writer.write_u64::<LittleEndian>(0)?;
    }

    Ok(())
}
