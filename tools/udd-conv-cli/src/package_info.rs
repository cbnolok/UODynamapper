use std::collections::BTreeMap;
use std::path::Path;

use color_eyre::eyre::{self, WrapErr};
use udd_assets::{
    HuesPackage, MobileAnimCcPackage, MobileAnimEcPackage, TexArtCcPackage, TexArtEcPackage,
    TexLandEcPackage, TileMetaPackage,
};
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

fn art_upscale_summary<'a>(
    slots: impl Iterator<Item = (&'a str, u16)>,
) -> String {
    let mut buckets = BTreeMap::<(&'a str, u16), u32>::new();
    for (algorithm, factor) in slots {
        *buckets.entry((algorithm, factor.max(1))).or_default() += 1;
    }
    if buckets.is_empty() {
        return "none".to_string();
    }
    buckets
        .into_iter()
        .map(|((algorithm, factor), count)| format!("{algorithm} {factor}x={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn mobile_anim_page_bucket_summary(
    pages: impl Iterator<Item = (u32, u32, u32)>,
) -> String {
    let mut buckets = BTreeMap::<(u32, u32), (u32, u32)>::new();
    for (width, height, frame_count) in pages {
        let entry = buckets.entry((width, height)).or_default();
        entry.0 += 1;
        entry.1 += frame_count;
    }
    if buckets.is_empty() {
        return "none".to_string();
    }
    buckets
        .into_iter()
        .map(|((width, height), (page_count, frame_count))| {
            format!("{width}x{height}: {page_count} pages / {frame_count} frames")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn print_known_package_summary(package: &UddpReader) -> eyre::Result<bool> {
    if let Ok(package) = TexArtCcPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        let first_page_format = package
            .pages()
            .first()
            .and_then(|page| package.read_page_bytes(page.page_index).ok().map(|data| (page, data.len())));

        println!("Recognized package: tex_art_cc");
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
        println!(
            "Upscale: {}",
            art_upscale_summary(package.slots().iter().filter(|slot| slot.is_present()).map(|slot| {
                (slot.upscale_algorithm_name(), slot.upscale_factor)
            }))
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

    if let Ok(package) = TexArtEcPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        println!("Recognized package: tex_art_ec");
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
        println!(
            "Upscale: {}",
            art_upscale_summary(package.slots().iter().filter(|slot| slot.is_present()).map(|slot| {
                (slot.upscale_algorithm_name(), slot.upscale_factor)
            }))
        );
        return Ok(true);
    }

    if let Ok(package) = MobileAnimCcPackage::from_uddp_package(package.clone()) {
        println!("Recognized package: mobile_anim_cc");
        println!("Known logical files: metadata=5, textures={}", package.pages().len());
        println!(
            "Atlas: max={}x{}, gutter={}, pages={}, buckets={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            mobile_anim_page_bucket_summary(package.pages().iter().map(|page| {
                (page.atlas_width, page.atlas_height, page.frame_count)
            }))
        );
        println!(
            "Animations: {}, frames={}, body_maps={}, body_types={}",
            package.animations().len(),
            package.frames().len(),
            package.body_resolve().len(),
            package.body_types().len()
        );
        return Ok(true);
    }

    if let Ok(package) = MobileAnimEcPackage::from_uddp_package(package.clone()) {
        println!("Recognized package: mobile_anim_ec");
        println!("Known logical files: metadata=5, textures={}", package.pages().len());
        println!(
            "Atlas: max={}x{}, gutter={}, pages={}, buckets={}",
            package.atlas_width(),
            package.atlas_height(),
            package.gutter(),
            package.pages().len(),
            mobile_anim_page_bucket_summary(package.pages().iter().map(|page| {
                (page.atlas_width, page.atlas_height, page.frame_count)
            }))
        );
        println!(
            "Animations: {}, frames={}, items={}, source_hints={}",
            package.animations().len(),
            package.frames().len(),
            package.items().len(),
            package.source_hints().len()
        );
        return Ok(true);
    }

    if let Ok(package) = TexLandEcPackage::from_uddp_package(package.clone()) {
        let populated_slots = package.slots().iter().filter(|slot| slot.is_present()).count();
        let override_entries = package.terrain_override_actions().len();
        let override_texture_refs = package
            .terrain_override_actions()
            .keys()
            .map(|material_id| package.resolve_override_texture_slots(*material_id).len())
            .sum::<usize>();
        let resolved_override_texture_refs = package
            .terrain_override_actions()
            .keys()
            .flat_map(|material_id| package.resolve_override_texture_slots(*material_id))
            .filter(|texture| texture.runtime_slot_id.is_some())
            .count();
        let effective_slot_changes = package.effective_override_slot_changes();
        let policy_count = package
            .terrain_override_details()
            .values()
            .map(|details| details.policies.len())
            .sum::<usize>();
        let liquid_count = package
            .terrain_override_details()
            .values()
            .filter(|details| details.liquid.is_some())
            .count();
        let ignore_count = package
            .terrain_override_details()
            .values()
            .filter(|details| details.ignore_code.is_some())
            .count();
        println!("Recognized package: tex_land_ec");
        println!("Known logical files: metadata>=3, textures={}", package.pages().len());
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
        println!(
            "Terrain override metadata: entries={}, policies={}, liquids={}, ignores={}, texture_refs={} (resolved={}), effective_slot_changes={}",
            override_entries,
            policy_count,
            liquid_count,
            ignore_count,
            override_texture_refs,
            resolved_override_texture_refs,
            effective_slot_changes.len()
        );
        let mut override_details = package
            .terrain_override_details()
            .values()
            .collect::<Vec<_>>();
        override_details.sort_by_key(|details| details.material_id);
        for details in override_details.into_iter().take(8) {
            if !details.policies.is_empty() || details.liquid.is_some() || details.ignore_code.is_some() {
                let policies = details
                    .policies
                    .iter()
                    .map(|policy| policy.policy.as_str())
                    .collect::<Vec<_>>()
                    .join("|");
                println!(
                    "  material {} override details: policies={} liquid={} ignore={}",
                    details.material_id,
                    if policies.is_empty() { "none" } else { policies.as_str() },
                    details.liquid.is_some(),
                    details.ignore_code.is_some()
                );
            }
        }
        for change in effective_slot_changes.iter().take(8) {
            println!(
                "  material {} query {}: runtime_slot={:?} effective_slot={:?}",
                change.material_id,
                change.query_tile_id,
                change.runtime_slot_id,
                change.effective_runtime_slot_id
            );
        }
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

    if let Ok(package) = HuesPackage::from_uddp_package(package.clone()) {
        let populated = package
            .slots()
            .iter()
            .filter_map(|slot| slot.as_ref())
            .filter(|slot| slot.is_present())
            .count();
        println!("Recognized package: hues");
        println!("Known logical files: metadata=1, textures=1");
        println!(
            "Hues: slots={}, populated={}, texture={}x{}",
            package.slots().len(),
            populated,
            udd_assets::hues::HUES_TEXTURE_WIDTH,
            udd_assets::hues::HUES_TEXTURE_HEIGHT
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
