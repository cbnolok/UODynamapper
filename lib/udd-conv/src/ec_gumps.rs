//! Build-time support for `gumps_ec.uddp`.

use byteorder::{LittleEndian, WriteBytesExt};
use color_eyre::eyre::{self, WrapErr};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use udd_container::{AddFileRequest, CompressionFlag, DataType, LookupMode, UddpBuilder};
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::package_progress::build_and_write_package;
use crate::source_paths::find_first_existing_file;

pub const GUMPS_EC_DEFAULT_OUTPUT: &str = "gumps_ec.uddp";
pub const EC_GUMP_DEFAULT_MAX_ID: u32 = 99_999;

pub struct EcGumpsOptions {
    pub max_id: u32,
    pub compression: CompressionFlag,
}

impl Default for EcGumpsOptions {
    fn default() -> Self {
        Self {
            max_id: EC_GUMP_DEFAULT_MAX_ID,
            compression: CompressionFlag::ZstdNoDict,
        }
    }
}

pub struct EcGumpsBuildSummary {
    pub gump_count: u32,
    pub skipped_count: u32,
}

#[derive(Clone)]
struct ExtractedGumpSource {
    path: PathBuf,
    format: ECImageFormat,
}

pub fn convert_ec_gumps_to_uddp_from_sources(
    source_dirs: &[PathBuf],
    output_path: &Path,
    options: &EcGumpsOptions,
) -> eyre::Result<EcGumpsBuildSummary> {
    if let Some(interface_uop) = find_first_existing_file(source_dirs, &["interface.uop"]) {
        convert_ec_gumps_from_interface_uop(&interface_uop, output_path, options)
    } else if let Some(gumpart_dir) = find_ec_gumpart_dir(source_dirs) {
        convert_ec_gumps_from_extracted_dir(&gumpart_dir, output_path, options)
    } else {
        eyre::bail!(
            "missing EC gump source: expected interface.uop or data/interface/default/textures/gumpart"
        );
    }
}

fn convert_ec_gumps_from_interface_uop(
    interface_uop: &Path,
    output_path: &Path,
    options: &EcGumpsOptions,
) -> eyre::Result<EcGumpsBuildSummary> {
    let package = UopPackage::load_with_mode(interface_uop, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", interface_uop.display()))?;
    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut gump_count = 0u32;
    let mut skipped_count = 0u32;

    let pb = ProgressBar::new(u64::from(options.max_id) + 1);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} EC gump ids ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    for gump_id in 0..=options.max_id {
        pb.inc(1);
        let Some((format, payload)) = unpack_ec_gump_payload(&package, gump_id)? else {
            continue;
        };

        match add_decoded_ec_gump(&mut builder, gump_id, format, &payload, options.compression) {
            Ok(()) => gump_count += 1,
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    pb.finish_with_message("EC gumps packed");
    build_and_write_package(&mut builder, output_path)?;

    Ok(EcGumpsBuildSummary {
        gump_count,
        skipped_count,
    })
}

fn convert_ec_gumps_from_extracted_dir(
    gumpart_dir: &Path,
    output_path: &Path,
    options: &EcGumpsOptions,
) -> eyre::Result<EcGumpsBuildSummary> {
    let sources = collect_extracted_gumps(gumpart_dir)?;
    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut gump_count = 0u32;
    let mut skipped_count = 0u32;

    let pb = ProgressBar::new(sources.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} EC gumps ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );

    for (gump_id, source) in sources {
        pb.inc(1);
        let payload = match fs::read(&source.path) {
            Ok(payload) => payload,
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
                continue;
            }
        };

        match add_decoded_ec_gump(
            &mut builder,
            gump_id,
            source.format,
            &payload,
            options.compression,
        ) {
            Ok(()) => gump_count += 1,
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    pb.finish_with_message("EC gumps packed");
    build_and_write_package(&mut builder, output_path)?;

    Ok(EcGumpsBuildSummary {
        gump_count,
        skipped_count,
    })
}

fn unpack_ec_gump_payload(
    package: &UopPackage,
    gump_id: u32,
) -> eyre::Result<Option<(ECImageFormat, Vec<u8>)>> {
    for (path, format) in ec_gump_candidates(gump_id) {
        let hash = hash_file_name_single(&path);
        if package.get_file_by_hash(hash).is_some() {
            let payload = package
                .unpack_file_by_hash(hash)?
                .ok_or_else(|| eyre::eyre!("EC gump entry disappeared from interface.uop"))?;
            return Ok(Some((format, payload)));
        }
    }
    Ok(None)
}

fn add_decoded_ec_gump(
    builder: &mut UddpBuilder,
    gump_id: u32,
    format: ECImageFormat,
    source_payload: &[u8],
    compression: CompressionFlag,
) -> eyre::Result<()> {
    let (width, height, rgba) = decode_ec_gump_payload(format, source_payload)?;
    let mut payload = Vec::with_capacity(8 + rgba.len());
    payload.write_u32::<LittleEndian>(width)?;
    payload.write_u32::<LittleEndian>(height)?;
    payload.extend_from_slice(&rgba);

    builder.add_file(AddFileRequest {
        data_type: DataType::Gump as u8,
        compression,
        width,
        height,
        virtual_path: None,
        path_hash64: None,
        id: Some(gump_id),
        data: &payload,
    })?;
    Ok(())
}

fn decode_ec_gump_payload(
    format: ECImageFormat,
    source_payload: &[u8],
) -> eyre::Result<(u32, u32, Vec<u8>)> {
    let tex_file = TextureFile {
        metadata: TextureItem::absent(),
        is_ec: true,
        format,
        props: None,
        raw_data: Arc::from(source_payload),
    };
    let image = tex_file.decode_to_rgba()?;
    let rgba = image.to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn find_ec_gumpart_dir(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        let nested = dir.join("data/interface/default/textures/gumpart");
        if nested.is_dir() {
            return Some(nested);
        }
        dir.is_dir()
            .then(|| dir.clone())
            .filter(|candidate| directory_contains_ec_gump(candidate))
    })
}

fn directory_contains_ec_gump(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .any(|entry| {
            let path = entry.path();
            ec_gump_id_from_path(&path).is_some() && ec_image_format_from_path(&path).is_some()
        })
}

fn collect_extracted_gumps(gumpart_dir: &Path) -> eyre::Result<BTreeMap<u32, ExtractedGumpSource>> {
    let mut sources: BTreeMap<u32, ExtractedGumpSource> = BTreeMap::new();
    for entry in fs::read_dir(gumpart_dir)
        .wrap_err_with(|| format!("read {}", gumpart_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(gump_id) = ec_gump_id_from_path(&path) else {
            continue;
        };
        let Some(format) = ec_image_format_from_path(&path) else {
            continue;
        };

        match sources.get(&gump_id) {
            Some(existing) if existing.format == ECImageFormat::TGA => {}
            _ => {
                sources.insert(gump_id, ExtractedGumpSource { path, format });
            }
        }
    }
    Ok(sources)
}

fn ec_gump_candidates(gump_id: u32) -> [(String, ECImageFormat); 2] {
    [
        (
            format!("data/interface/default/textures/gumpart/{gump_id:08}.tga"),
            ECImageFormat::TGA,
        ),
        (
            format!("data/interface/default/textures/gumpart/{gump_id:08}.dds"),
            ECImageFormat::DDS,
        ),
    ]
}

fn ec_gump_id_from_path(path: &Path) -> Option<u32> {
    let stem = path.file_stem()?.to_str()?;
    stem.parse::<u32>().ok()
}

fn ec_image_format_from_path(path: &Path) -> Option<ECImageFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "tga" => Some(ECImageFormat::TGA),
        "dds" => Some(ECImageFormat::DDS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ec_gump_candidates_use_interface_gumpart_paths() {
        assert_eq!(
            ec_gump_candidates(9504),
            [
                (
                    "data/interface/default/textures/gumpart/00009504.tga".to_string(),
                    ECImageFormat::TGA,
                ),
                (
                    "data/interface/default/textures/gumpart/00009504.dds".to_string(),
                    ECImageFormat::DDS,
                ),
            ]
        );
    }

    #[test]
    fn ec_gump_id_from_path_accepts_numeric_stems() {
        assert_eq!(ec_gump_id_from_path(Path::new("00009504.tga")), Some(9504));
        assert_eq!(ec_gump_id_from_path(Path::new("bad.tga")), None);
    }

    #[test]
    fn ec_image_format_from_path_is_case_insensitive() {
        assert_eq!(
            ec_image_format_from_path(Path::new("00009504.TGA")),
            Some(ECImageFormat::TGA)
        );
        assert_eq!(
            ec_image_format_from_path(Path::new("00009504.DDS")),
            Some(ECImageFormat::DDS)
        );
        assert_eq!(ec_image_format_from_path(Path::new("00009504.png")), None);
    }
}
