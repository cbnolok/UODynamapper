use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use udd_assets::{CcArtPackage, EcArtPackage, EcLandPackage, TileMetaPackage};
use udd_container::{
    reconstruct_stored_size, unpack_codec, unpack_type, Codec, LookupMode, UDDP_MAGIC, UDPI_MAGIC,
    UddpReader,
};

fn lookup_mode_name(mode: LookupMode) -> &'static str {
    match mode {
        LookupMode::VirtualPathHash => "VirtualPathHash",
        LookupMode::DenseId => "DenseId",
        LookupMode::SparseId => "SparseId",
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::None => "None",
        Codec::ZstdNoDict => "ZstdNoDict",
        Codec::ZstdTypeDict => "ZstdTypeDict",
        Codec::JpegXl => "JpegXl",
    }
}

fn data_type_name(data_type: u8) -> &'static str {
    match data_type {
        0 => "Unknown",
        1 => "Art",
        2 => "Anim",
        3 => "Map",
        4 => "Gump",
        5 => "GumpArt",
        6 => "Sound",
        7 => "Music",
        8 => "Multi",
        9 => "Texture",
        10 => "Light",
        11 => "Metadata",
        12 => "Sector",
        13 => "Tile",
        14 => "Static",
        _ => "Custom",
    }
}


fn page_pixel_format_name(byte_len: usize, width: u32, height: u32) -> &'static str {
    let rgba_len = width as usize * height as usize * 4;
    if byte_len == rgba_len {
        "Rgba8888"
    } else {
        "Encoded"
    }
}

fn print_known_package_summary(package: &UddpReader) -> eyre::Result<bool> {
    if let Ok(package) = CcArtPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        let first_page_format = package
            .pages()
            .first()
            .and_then(|page| package.read_page_bytes(page.page_index).ok().map(|data| (page, data.len())));

        println!("Recognized package: cc_art");
        println!("Known logical files: metadata=2, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        if let Some((page, byte_len)) = first_page_format {
            println!(
                "Page payload format: {} (first stored page bytes={})",
                page_pixel_format_name(byte_len, page.used_width, page.used_height),
                byte_len
            );
        }
        return Ok(true);
    }

    if let Ok(package) = EcArtPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        println!("Recognized package: ec_art");
        println!("Known logical files: metadata=2, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        return Ok(true);
    }

    if let Ok(package) = EcLandPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        println!("Recognized package: ec_land");
        println!("Known logical files: metadata=3, textures={}", package.pages().len());
        println!(
            "Atlas: {}x{}, gutter={}, pages={}, slots={}, populated={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            package.slots().len(),
            populated_slots
        );
        println!("Terrain provenance rows: {}", package.terrain_provenance().len());
        return Ok(true);
    }

    if let Ok(package) = TileMetaPackage::from_uddp_package(package.clone()) {
        println!("Recognized package: tilemeta");
        println!("Known logical files: metadata=2");
        println!(
            "Tilemeta: land_tiles={}, item_tiles={}",
            package.land_tiles().len(),
            package.item_tiles().len()
        );
        return Ok(true);
    }

    Ok(false)
}

pub fn print_package_info(path: &Path) -> eyre::Result<()> {
    let package = UddpReader::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    let header = package.header();
    let kind = match header.magic {
        UDDP_MAGIC => "UDDP",
        UDPI_MAGIC => "UDDPI",
        _ => "Unknown",
    };

    let mut codec_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut type_counts: BTreeMap<u8, (u32, u64, u64)> = BTreeMap::new();
    let mut raw_total = 0u64;
    let mut stored_total = 0u64;

    for record in package.records() {
        let codec = unpack_codec(record.locator.meta32);
        let data_type = unpack_type(record.locator.meta32);
        let stored_size = reconstruct_stored_size(
            record.locator.raw_size,
            record.locator.meta32,
            record.locator.pos64,
        ) as u64;
        let raw_size = record.locator.raw_size as u64;

        *codec_counts.entry(codec_name(codec)).or_default() += 1;
        let entry = type_counts.entry(data_type).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += raw_size;
        entry.2 += stored_size;
        raw_total += raw_size;
        stored_total += stored_size;
    }

    println!("File: {}", path.display());
    println!("Kind: {}", kind);
    println!("Lookup mode: {}", lookup_mode_name(package.lookup_mode()));
    println!("Package bytes: {}", package.package_size_bytes());
    println!("Header file count: {}", header.file_count);
    println!("Stored package hash64: {:#018x}", package.stored_package_hash64());
    println!("Computed package hash64: {:#018x}", package.computed_package_hash64());
    match package.lookup_mode() {
        LookupMode::VirtualPathHash => println!("Logical key counts: path_hashes={}", header.file_count),
        LookupMode::DenseId | LookupMode::SparseId => println!("Logical key counts: ids={}", header.file_count),
    }

    let dictionaries = package.dictionary_records();
    println!("Dictionaries: {}", dictionaries.len());
    for (data_type, codec, size) in dictionaries {
        println!(
            "  {} ({}) : codec={}, bytes={}",
            data_type_name(data_type),
            data_type,
            codec_name(codec),
            size
        );
    }

    let recognized = print_known_package_summary(&package)?;
    if !recognized {
        println!(
            "Payload totals: raw={} stored={} saved={}",
            raw_total,
            stored_total,
            raw_total.saturating_sub(stored_total)
        );

        println!("Compression:");
        for (codec, count) in codec_counts {
            println!("  {}: {} files", codec, count);
        }

        println!("Data types:");
        for (data_type, (count, raw_size, stored_size)) in type_counts {
            println!(
                "  {} ({}) : {} files, raw={}, stored={}",
                data_type_name(data_type),
                data_type,
                count,
                raw_size,
                stored_size
            );
        }
    }

    Ok(())
}
pub fn get_package_info_string(path: &Path) -> eyre::Result<String> {
    use std::fmt::Write;
    let mut out = String::new();
    
    let package = UddpReader::load(path).wrap_err_with(|| format!("load {}", path.display()))?;
    let header = package.header();
    let kind = match header.magic {
        UDDP_MAGIC => "UDDP",
        UDPI_MAGIC => "UDDPI",
        _ => "Unknown",
    };

    let mut codec_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut type_counts: BTreeMap<u8, (u32, u64, u64)> = BTreeMap::new();
    let mut raw_total = 0u64;
    let mut stored_total = 0u64;

    for record in package.records() {
        let codec = unpack_codec(record.locator.meta32);
        let data_type = unpack_type(record.locator.meta32);
        let stored_size = reconstruct_stored_size(
            record.locator.raw_size,
            record.locator.meta32,
            record.locator.pos64,
        ) as u64;
        let raw_size = record.locator.raw_size as u64;

        *codec_counts.entry(codec_name(codec)).or_default() += 1;
        let entry = type_counts.entry(data_type).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += raw_size;
        entry.2 += stored_size;
        raw_total += raw_size;
        stored_total += stored_size;
    }

    writeln!(out, "File: {}", path.display())?;
    writeln!(out, "Kind: {}", kind)?;
    writeln!(out, "Lookup mode: {}", lookup_mode_name(package.lookup_mode()))?;
    writeln!(out, "Package bytes: {}", package.package_size_bytes())?;
    writeln!(out, "Header file count: {}", header.file_count)?;
    writeln!(out, "Stored package hash64: {:#018x}", package.stored_package_hash64())?;
    writeln!(out, "Computed package hash64: {:#018x}", package.computed_package_hash64())?;
    match package.lookup_mode() {
        LookupMode::VirtualPathHash => writeln!(out, "Logical key counts: path_hashes={}", header.file_count)?,
        LookupMode::DenseId | LookupMode::SparseId => writeln!(out, "Logical key counts: ids={}", header.file_count)?,
    }

    let dictionaries = package.dictionary_records();
    writeln!(out, "Dictionaries: {}", dictionaries.len())?;
    for (data_type, codec, size) in dictionaries {
        writeln!(
            out,
            "  {} ({}) : codec={}, bytes={}",
            data_type_name(data_type),
            data_type,
            codec_name(codec),
            size
        )?;
    }

    // Note: print_known_package_summary still prints to stdout, 
    // but for the GUI we mostly care about the generic info or 
    // we should refactor that too.
    
    writeln!(out, "\nPayload totals: raw={} stored={} saved={}",
        raw_total,
        stored_total,
        raw_total.saturating_sub(stored_total)
    )?;

    writeln!(out, "Compression:")?;
    for (codec, count) in codec_counts {
        writeln!(out, "  {}: {} files", codec, count)?;
    }

    writeln!(out, "Data types:")?;
    for (data_type, (count, raw_size, stored_size)) in type_counts {
        writeln!(
            out,
            "  {} ({}) : {} files, raw={}, stored={}",
            data_type_name(data_type),
            data_type,
            count,
            raw_size,
            stored_size
        )?;
    }

    Ok(out)
}
