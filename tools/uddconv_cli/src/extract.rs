use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, WrapErr};
use image::{ColorType, ImageFormat};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use uddconv::bc7::{ImageExtent, decode_bc7_to_rgba8888};
use uddconv::cc_art::{CcArtPackage, PagePixelFormat};
use uddconv::ec_art::EcArtPackage;
use uddconv::ec_land::{EcLandPackage, MISSING_SLOT_ID, MISSING_TEXTURE_ID};
use uddconv::tilemeta::TileMetaPackage;
use uocf::udd::{Codec, LookupMode, UddpReader};
use uocf::udd::uddp::FileKey;

pub fn extract_package(file: &Path, output: Option<&Path>) -> eyre::Result<()> {
    let out_dir = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output_dir(file));
    std::fs::create_dir_all(&out_dir)
        .wrap_err_with(|| format!("create {}", out_dir.display()))?;

    let package = UddpReader::load(file).wrap_err_with(|| format!("load {}", file.display()))?;

    if extract_cc_art(&package, &out_dir)? {
        println!("Extracted cc_art package to '{}'", out_dir.display());
        return Ok(());
    }
    if extract_ec_art(&package, &out_dir)? {
        println!("Extracted ec_art package to '{}'", out_dir.display());
        return Ok(());
    }
    if extract_ec_land(&package, &out_dir)? {
        println!("Extracted ec_land package to '{}'", out_dir.display());
        return Ok(());
    }
    if extract_tilemeta(&package, &out_dir)? {
        println!("Extracted tilemeta package to '{}'", out_dir.display());
        return Ok(());
    }

    extract_generic(&package, &out_dir)?;
    println!("Extracted generic package to '{}'", out_dir.display());
    Ok(())
}

fn default_output_dir(file: &Path) -> PathBuf {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
    file.with_file_name(format!("{stem}.extract"))
}

fn extract_cc_art(package: &UddpReader, out_dir: &Path) -> eyre::Result<bool> {
    let Ok(package) = CcArtPackage::from_uddp_package(package.clone()) else {
        return Ok(false);
    };

    extract_atlas_pages(
        out_dir,
        "cc_art",
        package.atlas_width(),
        package.atlas_height(),
        package.gutter(),
        package.pages().iter().map(|page| AtlasPageRow {
            page_index: page.page_index,
            tile_count: page.tile_count,
            used_width: page.used_width,
            used_height: page.used_height,
            pixel_format: page.pixel_format,
        }),
        |page_index| package.read_page_bytes(page_index),
    )?;

    let mut slots_csv = String::from("art_id,kind,page_index,page_tile_index,x,y,width,height\n");
    for slot in package.slots().iter().filter(|slot| slot.is_present()) {
        let kind = if slot.is_land() { "land" } else { "static" };
        writeln!(
            slots_csv,
            "{},{},{},{},{},{},{},{}",
            slot.art_id,
            kind,
            slot.page_index,
            slot.page_tile_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height
        )
        .unwrap();
    }
    write_text_file(&out_dir.join("metadata/present_slots.csv"), &slots_csv)?;
    Ok(true)
}

fn extract_ec_art(package: &UddpReader, out_dir: &Path) -> eyre::Result<bool> {
    let Ok(package) = EcArtPackage::from_uddp_package(package.clone()) else {
        return Ok(false);
    };

    extract_atlas_pages(
        out_dir,
        "ec_art",
        package.atlas_width(),
        package.atlas_height(),
        package.gutter(),
        package.pages().iter().map(|page| AtlasPageRow {
            page_index: page.page_index,
            tile_count: page.tile_count,
            used_width: page.used_width,
            used_height: page.used_height,
            pixel_format: page.pixel_format,
        }),
        |page_index| package.read_page_bytes(page_index),
    )?;

    let mut slots_csv = String::from("art_id,kind,page_index,page_tile_index,x,y,width,height\n");
    for slot in package.slots().iter().filter(|slot| slot.is_present()) {
        let kind = if slot.is_land() { "land" } else { "static" };
        writeln!(
            slots_csv,
            "{},{},{},{},{},{},{},{}",
            slot.art_id,
            kind,
            slot.page_index,
            slot.page_tile_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height
        )
        .unwrap();
    }
    write_text_file(&out_dir.join("metadata/present_slots.csv"), &slots_csv)?;
    Ok(true)
}

fn extract_ec_land(package: &UddpReader, out_dir: &Path) -> eyre::Result<bool> {
    let Ok(package) = EcLandPackage::from_uddp_package(package.clone()) else {
        return Ok(false);
    };

    extract_atlas_pages(
        out_dir,
        "ec_land",
        package.atlas_width(),
        package.atlas_height(),
        package.gutter(),
        package.pages().iter().map(|page| AtlasPageRow {
            page_index: page.page_index,
            tile_count: page.tile_count,
            used_width: page.used_width,
            used_height: page.used_height,
            pixel_format: page.pixel_format,
        }),
        |page_index| package.read_page_bytes(page_index),
    )?;

    let metadata_dir = out_dir.join("metadata");
    let summary = format!(
        "package=ec_land\natlas_width={}\natlas_height={}\ngutter={}\npresent_slots={}\nterrain_provenance_rows={}\n",
        package.atlas_width(),
        package.atlas_height(),
        package.gutter(),
        package.slots().iter().filter(|slot| slot.is_present()).count(),
        package.terrain_provenance().len(),
    );
    write_text_file(&metadata_dir.join("summary.txt"), &summary)?;

    let mut slots_csv = String::from("art_id,kind,page_index,page_tile_index,x,y,width,height\n");
    for slot in package.slots().iter().filter(|slot| slot.is_present()) {
        let kind = if slot.is_land() { "land" } else { "static" };
        writeln!(
            slots_csv,
            "{},{},{},{},{},{},{},{}",
            slot.art_id,
            kind,
            slot.page_index,
            slot.page_tile_index,
            slot.x,
            slot.y,
            slot.width,
            slot.height
        )
        .unwrap();
    }
    write_text_file(&out_dir.join("metadata/present_slots.csv"), &slots_csv)?;

    let mut provenance_csv = String::from(
        "material_id,material_name_id,alias_count_index,alias_slot_id,alias_tile_flags,selected_texture_id,canonical_slot_id\n",
    );
    for record in package.terrain_provenance() {
        writeln!(
            provenance_csv,
            "{},{},{},{},{},{},{}",
            record.material_id,
            record.material_name_id,
            record.alias_count_index,
            record.alias_slot_id,
            record.alias_tile_flags,
            optional_u32_csv(record.selected_texture_id, MISSING_TEXTURE_ID),
            optional_u32_csv(record.canonical_slot_id, MISSING_SLOT_ID)
        )
        .unwrap();
    }
    write_text_file(&out_dir.join("metadata/terrain_provenance.csv"), &provenance_csv)?;
    Ok(true)
}

fn extract_tilemeta(package: &UddpReader, out_dir: &Path) -> eyre::Result<bool> {
    let Ok(package) = TileMetaPackage::from_uddp_package(package.clone()) else {
        return Ok(false);
    };
    let metadata_dir = out_dir.join("metadata");
    std::fs::create_dir_all(&metadata_dir)
        .wrap_err_with(|| format!("create {}", metadata_dir.display()))?;

    let summary = format!(
        "package=tilemeta\nland_tiles={}\nitem_tiles={}\n",
        package.land_tiles().len(),
        package.item_tiles().len(),
    );
    write_text_file(&metadata_dir.join("summary.txt"), &summary)?;

    let mut land_csv = String::from("tile_id,texture_id,tile_type,flags,radar_r,radar_g,radar_b,radar_a,name\n");
    let pb = progress_bar(
        (package.land_tiles().len() + package.item_tiles().len()) as u64,
        "extracting tilemeta",
    );
    for tile in package.land_tiles() {
        pb.inc(1);
        writeln!(
            land_csv,
            "{},{},{},{},{},{},{},{},{}",
            tile.tile_id,
            tile.texture_id,
            tile.tile_type,
            tile.flags,
            tile.radar_color[0],
            tile.radar_color[1],
            tile.radar_color[2],
            tile.radar_color[3],
            csv_escape(tile.name_ascii())
        )
        .unwrap();
    }
    write_text_file(&out_dir.join("land_tiles.csv"), &land_csv)?;

    let mut item_csv = String::from("tile_id,weight,quality,quantity,hue_extra,flags,anim_id,stacking_offset,value,height,radar_r,radar_g,radar_b,radar_a,name,ec_texture_id,ec_start_x,ec_start_y,ec_offset_x,ec_offset_y,cc_texture_id,cc_start_x,cc_start_y,cc_offset_x,cc_offset_y\n");
    for tile in package.item_tiles() {
        pb.inc(1);
        writeln!(
            item_csv,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            tile.tile_id,
            tile.weight,
            tile.quality,
            tile.quantity,
            tile.hue_extra,
            tile.flags,
            tile.anim_id,
            tile.stacking_offset,
            tile.value,
            tile.height,
            tile.radar_color[0],
            tile.radar_color[1],
            tile.radar_color[2],
            tile.radar_color[3],
            csv_escape(tile.name_ascii()),
            tile.ec_texture_id,
            tile.ec_start_x,
            tile.ec_start_y,
            tile.ec_offset_x,
            tile.ec_offset_y,
            tile.cc_texture_id,
            tile.cc_start_x,
            tile.cc_start_y,
            tile.cc_offset_x,
            tile.cc_offset_y
        )
        .unwrap();
    }
    pb.finish_with_message("Tilemeta extracted");
    write_text_file(&out_dir.join("item_tiles.csv"), &item_csv)?;
    Ok(true)
}

fn extract_generic(package: &UddpReader, out_dir: &Path) -> eyre::Result<()> {
    let payload_dir = out_dir.join("payloads");
    std::fs::create_dir_all(&payload_dir)
        .wrap_err_with(|| format!("create {}", payload_dir.display()))?;

    let records = package.records();
    let pb = progress_bar(records.len() as u64, "extracting payloads");
    let extracted_records = records
        .par_iter()
        .map(|record| -> eyre::Result<String> {
            let (key_name, data) = match record.key {
                FileKey::PathHash(path_hash) => (
                    format!("path_hash_{path_hash:016x}"),
                    package.read_file_by_path_hash(path_hash)?,
                ),
                FileKey::Id(id) => match package.lookup_mode() {
                    LookupMode::DenseId => (format!("id_{id:08}"), package.read_file_by_dense_id(id)?),
                    LookupMode::SparseId => (format!("id_{id:08}"), package.read_file_by_sparse_id(id)?),
                    LookupMode::VirtualPathHash => {
                        unreachable!("path hash packages should not expose id keys")
                    }
                },
            };

            std::fs::write(payload_dir.join(format!("{key_name}.bin")), data)
                .wrap_err_with(|| format!("write extracted payload {key_name}"))?;

            Ok(format!(
                "{},{},{},{},{}\n",
                key_name,
                record.locator.meta32 & 0x3F,
                codec_name(unpack_codec(record.locator.meta32)),
                record.locator.raw_size,
                reconstruct_stored_size(record.locator.raw_size, record.locator.meta32, record.locator.pos64)
            ))
        })
        .collect::<Vec<_>>();

    let mut records_csv = String::from("key,data_type,codec,raw_size,stored_size\n");
    for extracted_record in extracted_records {
        pb.inc(1);
        records_csv.push_str(&extracted_record?);
    }
    pb.finish_with_message("Payloads extracted");
    write_text_file(&out_dir.join("records.csv"), &records_csv)?;
    Ok(())
}

struct AtlasPageRow {
    page_index: u32,
    tile_count: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
}

struct ExtractedAtlasPage {
    csv_row: String,
}

fn extract_atlas_pages<I, F>(
    out_dir: &Path,
    package_name: &str,
    atlas_width: u32,
    atlas_height: u32,
    gutter: u16,
    pages: I,
    read_page_bytes: F,
) -> eyre::Result<()>
where
    I: IntoIterator<Item = AtlasPageRow>,
    F: Fn(u32) -> eyre::Result<Vec<u8>> + Sync,
{
    let pages_dir = out_dir.join("pages");
    let metadata_dir = out_dir.join("metadata");
    std::fs::create_dir_all(&pages_dir)
        .wrap_err_with(|| format!("create {}", pages_dir.display()))?;
    std::fs::create_dir_all(&metadata_dir)
        .wrap_err_with(|| format!("create {}", metadata_dir.display()))?;

    let summary = format!(
        "package={package_name}\natlas_width={atlas_width}\natlas_height={atlas_height}\ngutter={gutter}\n"
    );
    write_text_file(&metadata_dir.join("summary.txt"), &summary)?;

    let pages = pages.into_iter().collect::<Vec<_>>();
    let pb = progress_bar(pages.len() as u64, &format!("extracting {package_name} pages"));
    let extracted_pages = pages
        .par_iter()
        .map(|page| -> eyre::Result<ExtractedAtlasPage> {
            let encoded = read_page_bytes(page.page_index)?;
            let rgba =
                decode_page_to_rgba(&encoded, page.pixel_format, page.used_width, page.used_height)?;
            let png_name = format!("page_{:05}.png", page.page_index);
            write_rgba_png(&pages_dir.join(&png_name), page.used_width, page.used_height, &rgba)?;
            Ok(ExtractedAtlasPage {
                csv_row: format!(
                    "{},{},{},{},{},{}\n",
                    page.page_index,
                    page.tile_count,
                    page.used_width,
                    page.used_height,
                    page.pixel_format.extension(),
                    png_name
                ),
            })
        })
        .collect::<Vec<_>>();

    let mut pages_csv = String::from("page_index,tile_count,used_width,used_height,pixel_format,png\n");
    for extracted_page in extracted_pages {
        pb.inc(1);
        pages_csv.push_str(&extracted_page?.csv_row);
    }
    pb.finish_with_message(format!("{package_name} pages extracted"));
    write_text_file(&metadata_dir.join("pages.csv"), &pages_csv)?;
    Ok(())
}

fn progress_bar(len: u64, message: &str) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message(message.to_string());
    pb
}

fn decode_page_to_rgba(
    encoded: &[u8],
    pixel_format: PagePixelFormat,
    width: u32,
    height: u32,
) -> eyre::Result<Vec<u8>> {
    match pixel_format {
        PagePixelFormat::Rgba8888 => Ok(encoded.to_vec()),
        PagePixelFormat::Bc7 => {
            let extent = ImageExtent::new(width, height).map_err(|e| eyre::eyre!(e.to_string()))?;
            decode_bc7_to_rgba8888(encoded, extent).map_err(|e| eyre::eyre!(e.to_string()))
        }
    }
}

fn write_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> eyre::Result<()> {
    image::save_buffer_with_format(path, rgba, width, height, ColorType::Rgba8, ImageFormat::Png)
        .wrap_err_with(|| format!("write {}", path.display()))
}

fn write_text_file(path: &Path, text: &str) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(path, text).wrap_err_with(|| format!("write {}", path.display()))
}

fn csv_escape(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn optional_u32_csv(value: u32, missing: u32) -> String {
    if value == missing {
        String::new()
    } else {
        value.to_string()
    }
}

fn unpack_codec(meta32: u32) -> Codec {
    match ((meta32 >> 6) & 0x03) as u8 {
        0 => Codec::None,
        1 => Codec::ZstdNoDict,
        2 => Codec::ZstdTypeDict,
        _ => Codec::JpegXl,
    }
}

fn reconstruct_stored_size(raw_size: u32, meta32: u32, pos64: u64) -> u32 {
    match unpack_codec(meta32) {
        Codec::None => raw_size,
        Codec::ZstdNoDict | Codec::ZstdTypeDict | Codec::JpegXl => {
            raw_size.saturating_sub(((meta32 >> 8) & 0xFF) << 24 | ((pos64 >> 40) as u32 & 0x00FF_FFFF))
        }
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
