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

use crate::gump_atlas::{
    add_gump_atlas_files, is_paperdoll_equipment_gump_id, DecodedGump, GumpAtlasOptions,
};
use crate::package_progress::build_and_write_package;

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
    pub single_gump_count: u32,
    pub atlas_gump_count: u32,
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
    if let Some(interface_uop) = find_interface_uop(source_dirs) {
        convert_ec_gumps_from_interface_uop(&interface_uop, output_path, options)
    } else if let Some(gumpart_dir) = find_ec_gumpart_dir(source_dirs) {
        convert_ec_gumps_from_extracted_dir(&gumpart_dir, output_path, options)
    } else {
        eyre::bail!(
            "missing EC gump source: expected Interface.uop/interface.uop, or an extracted data/interface/default/textures/gumpart directory"
        );
    }
}

fn find_interface_uop(source_dirs: &[PathBuf]) -> Option<PathBuf> {
    source_dirs.iter().find_map(|dir| {
        ["Interface.uop", "interface.uop", "INTERFACE.UOP"]
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
            .or_else(|| {
                fs::read_dir(dir)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| name.eq_ignore_ascii_case("interface.uop"))
                            .unwrap_or(false)
                            && path.is_file()
                    })
            })
    })
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
    let mut single_gump_count = 0u32;
    let mut atlas_gumps = Vec::new();
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

        match decode_ec_gump_payload(gump_id, format, &payload) {
            Ok(decoded) => {
                if is_paperdoll_equipment_gump_id(gump_id) {
                    atlas_gumps.push(decoded);
                } else {
                    add_single_decoded_ec_gump(&mut builder, &decoded, options.compression)?;
                    single_gump_count += 1;
                }
                gump_count += 1;
            }
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    let atlas_gump_count =
        add_gump_atlas_files(&mut builder, atlas_gumps, &GumpAtlasOptions::default())?;
    pb.finish_with_message("EC gumps packed");
    build_and_write_package(&mut builder, output_path)?;

    Ok(EcGumpsBuildSummary {
        gump_count,
        single_gump_count,
        atlas_gump_count,
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
    let mut single_gump_count = 0u32;
    let mut atlas_gumps = Vec::new();
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

        let format = infer_ec_image_format(&payload, source.format);
        match decode_ec_gump_payload(gump_id, format, &payload) {
            Ok(decoded) => {
                if is_paperdoll_equipment_gump_id(gump_id) {
                    atlas_gumps.push(decoded);
                } else {
                    add_single_decoded_ec_gump(&mut builder, &decoded, options.compression)?;
                    single_gump_count += 1;
                }
                gump_count += 1;
            }
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    let atlas_gump_count =
        add_gump_atlas_files(&mut builder, atlas_gumps, &GumpAtlasOptions::default())?;
    pb.finish_with_message("EC gumps packed");
    build_and_write_package(&mut builder, output_path)?;

    Ok(EcGumpsBuildSummary {
        gump_count,
        single_gump_count,
        atlas_gump_count,
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
            return Ok(Some((infer_ec_image_format(&payload, format), payload)));
        }
    }
    Ok(None)
}

fn add_single_decoded_ec_gump(
    builder: &mut UddpBuilder,
    decoded: &DecodedGump,
    compression: CompressionFlag,
) -> eyre::Result<()> {
    let mut payload = Vec::with_capacity(8 + decoded.rgba.len());
    payload.write_u32::<LittleEndian>(u32::from(decoded.width))?;
    payload.write_u32::<LittleEndian>(u32::from(decoded.height))?;
    payload.extend_from_slice(&decoded.rgba);

    builder.add_file(AddFileRequest {
        data_type: DataType::Gump as u8,
        compression,
        width: u32::from(decoded.width),
        height: u32::from(decoded.height),
        virtual_path: None,
        path_hash64: None,
        id: Some(decoded.gump_id),
        data: &payload,
    })?;
    Ok(())
}

fn decode_ec_gump_payload(
    gump_id: u32,
    format: ECImageFormat,
    source_payload: &[u8],
) -> eyre::Result<DecodedGump> {
    let tex_file = TextureFile {
        metadata: TextureItem::absent(),
        is_ec: true,
        format,
        props: None,
        raw_data: Arc::from(source_payload),
    };
    let image = tex_file.decode_to_rgba()?;
    let rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width > u16::MAX as u32 || height > u16::MAX as u32 {
        eyre::bail!("EC gump {gump_id} dimensions exceed u16 metadata bounds: {width}x{height}");
    }
    Ok(DecodedGump {
        gump_id,
        width: width as u16,
        height: height as u16,
        rgba: rgba.into_raw(),
    })
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

fn infer_ec_image_format(payload: &[u8], fallback: ECImageFormat) -> ECImageFormat {
    if payload.starts_with(b"DDS ") {
        ECImageFormat::DDS
    } else if is_probably_tga(payload) {
        ECImageFormat::TGA
    } else {
        fallback
    }
}

fn is_probably_tga(payload: &[u8]) -> bool {
    if payload.len() < 18 {
        return false;
    }

    let image_type = payload[2];
    (image_type == 2 || image_type == 10) && payload[1] <= 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn infer_ec_image_format_prefers_payload_magic_over_extension() {
        assert_eq!(infer_ec_image_format(b"DDS payload", ECImageFormat::TGA), ECImageFormat::DDS);
        let mut tga = vec![0u8; 18];
        tga[2] = 2;
        assert_eq!(infer_ec_image_format(&tga, ECImageFormat::DDS), ECImageFormat::TGA);
    }

    #[test]
    fn find_interface_uop_accepts_client_casing() {
        let root = unique_temp_dir("ec-interface-uop-case");
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(root.join("Interface.uop"), b"not a real uop").expect("write marker");

        let found = find_interface_uop(&[root.clone()]).expect("find Interface.uop");

        assert_eq!(found.file_name().and_then(|name| name.to_str()), Some("Interface.uop"));
        fs::remove_dir_all(root).ok();
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{}-{nanos}", std::process::id()))
    }
}
