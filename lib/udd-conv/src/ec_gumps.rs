//! Build-time support for `gumps_ec.uddp`.

use color_eyre::eyre::{self, WrapErr};
use crate::progress::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use udd_assets::gumps::GumpsPackage;
use udd_container::{CompressionFlag, LookupMode, UddpBuilder};
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem};
use uocf::uop_container::package::{LoadMode, UopPackage};

use crate::gump_atlas::{
    add_gump_atlas_files_with_progress, is_paperdoll_equipment_gump_id, DecodedGump, GumpAtlasOptions,
};
use crate::package_progress::{
    build_and_write_package_with_progress, AssetProgressReporter, AssetTaskProgress, AssetTaskProgressStage,
};
use crate::page_compression::LOSSLESS_PAGE_COMPRESSION;
use crate::single_gumps::{
    default_single_gump_compression, ensure_single_gump_format_for_package, GumpPixelFormat, SingleGumpWriter,
};
use crate::source_paths::source_path_label_from_dirs;
use crate::upscale::{apply_upscale_passes, UpscalePass};
use crate::upscale_profile::{UpscaleImageType, UpscaleProfile, UpscaleTarget};

pub const GUMPS_EC_DEFAULT_OUTPUT: &str = "gumps_ec.uddp";
pub const EC_GUMP_DEFAULT_MAX_ID: u32 = 99_999;

/// The lossy format of Enhanced single gumps (the user, 2026-11-06): BC7, which keeps soft alpha.
pub const EC_LOSSY_SINGLE_GUMP_FORMAT: GumpPixelFormat = GumpPixelFormat::Bc7;
/// The lossy format of the Enhanced single gumps in the Classic colour space
/// ([`crate::single_gumps::is_in_classic_colour_space`]), the Classic gumps Interface.uop repeats (the user,
/// 2026-11-06): BC1, as `gumps_cc` stores them, since they have no soft alpha for BC7 to keep.
pub const EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT: GumpPixelFormat = GumpPixelFormat::Bc1;

pub struct EcGumpsOptions {
    pub max_id: u32,
    /// How single gumps are stored: lossless RGBA8888, or BC7 when packing lossy, with the gumps in the Classic colour
    /// space as BC1. Paperdoll gumps stay lossless: the Enhanced client hues exact greys only
    /// (`UOPaperdoll::DrawGumpLayer`).
    pub single_pixel_format: GumpPixelFormat,
    /// The builder codec of single gumps.
    pub compression: CompressionFlag,
    pub atlas_compression: CompressionFlag,
    pub paperdoll_upscale_passes: Vec<UpscalePass>,
    pub single_upscale_passes: Vec<UpscalePass>,
    pub upscale_profile: Option<Arc<UpscaleProfile>>,
    /// The `gumps_cc.uddp` to share the Classic twins with (`--share-with`): each single gump it draws the same is left
    /// out and listed in the shared Classic gump table, for Dynamapper to draw from the Classic source. `None` packs
    /// standalone.
    pub share_with: Option<PathBuf>,
}

impl Default for EcGumpsOptions {
    fn default() -> Self {
        Self {
            max_id: EC_GUMP_DEFAULT_MAX_ID,
            single_pixel_format: GumpPixelFormat::Rgba8888,
            compression: default_single_gump_compression(GumpPixelFormat::Rgba8888),
            atlas_compression: LOSSLESS_PAGE_COMPRESSION,
            paperdoll_upscale_passes: Vec::new(),
            single_upscale_passes: Vec::new(),
            upscale_profile: None,
            share_with: None,
        }
    }
}

fn gump_upscale_passes_for(
    options: &EcGumpsOptions,
    image_type: UpscaleImageType,
    gump_id: u32,
    fallback: &[UpscalePass],
) -> Vec<UpscalePass> {
    options
        .upscale_profile
        .as_ref()
        .map(|profile| profile.passes_for(UpscaleTarget::new(image_type, gump_id), fallback))
        .unwrap_or_else(|| fallback.to_vec())
}

/// The single gump writer of `options`. Packing lossy, the gumps in the Classic colour space go
/// [`EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT`] and the rest [`EC_LOSSY_SINGLE_GUMP_FORMAT`]. The writer tests the
/// pixels it is given, after the upscale passes, so an interpolating upscaler, whose blended texels leave the Classic
/// colour space, sends a gump to BC7. That is intended: blending makes new colours and soft alpha edges, which BC7 keeps
/// and BC1 would round. An upscaler that only copies texels, such as nearest neighbour, keeps a gump in the Classic
/// colour space.
fn single_gump_writer(options: &EcGumpsOptions, shared_with: Option<&Arc<GumpsPackage>>) -> SingleGumpWriter {
    let writer = SingleGumpWriter::new(options.single_pixel_format, options.compression);
    let writer = if options.single_pixel_format == EC_LOSSY_SINGLE_GUMP_FORMAT {
        writer.with_classic_colour_space_format(EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT)
    } else {
        writer
    };
    match shared_with {
        Some(classic) => writer.with_shared_classic_gumps(classic.clone()),
        None => writer,
    }
}

/// Opens the `--share-with` package, refusing the output package itself and a `gumps_ec` that shares gumps.
fn load_share_source(share_with: &Path, output_path: &Path) -> eyre::Result<Arc<GumpsPackage>> {
    if let (Ok(source), Ok(output)) = (fs::canonicalize(share_with), fs::canonicalize(output_path)) {
        eyre::ensure!(source != output, "--share-with names the output package {}", output_path.display());
    }
    let classic = GumpsPackage::load(share_with)
        .wrap_err_with(|| format!("load the gumps_cc package to share with, {}", share_with.display()))?;
    eyre::ensure!(
        classic.shared_classic_gump_count() == 0,
        "{} leaves {} gumps out for another package to draw, so it is a shared gumps_ec, not a gumps_cc",
        share_with.display(),
        classic.shared_classic_gump_count()
    );
    Ok(Arc::new(classic))
}

pub struct EcGumpsBuildSummary {
    pub gump_count: u32,
    pub single_gump_count: u32,
    /// Single gumps with alpha neither 0 nor 255, which BC7 keeps.
    pub single_soft_alpha_count: u32,
    /// Single gumps in the Classic colour space stored as [`EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT`] in a lossy pack.
    pub single_classic_colour_space_count: u32,
    /// Single gumps left out because the `--share-with` package draws them the same; `single_gump_count` leaves them out.
    pub single_shared_classic_count: u32,
    /// Single gumps stored although the `--share-with` package holds them, because its copy is lossier.
    pub single_kept_classic_lossier_count: u32,
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
    convert_ec_gumps_to_uddp_from_sources_with_progress(source_dirs, output_path, options, |_| {}, |_| {})
}

/// [`convert_ec_gumps_to_uddp_from_sources`], reporting the gump extraction, the paperdoll atlas packing and the
/// package build.
pub fn convert_ec_gumps_to_uddp_from_sources_with_progress(
    source_dirs: &[PathBuf],
    output_path: &Path,
    options: &EcGumpsOptions,
    payload_progress: impl Fn(AssetTaskProgress) + Sync,
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<EcGumpsBuildSummary> {
    let payload_progress_reporter = AssetProgressReporter::new(payload_progress);
    let payload_progress = |progress| payload_progress_reporter.report(progress);
    ensure_single_gump_format_for_package(options.single_pixel_format, EC_LOSSY_SINGLE_GUMP_FORMAT, "gumps_ec")?;
    let shared_with = options
        .share_with
        .as_deref()
        .map(|share_with| load_share_source(share_with, output_path))
        .transpose()?;
    if let Some(share_with) = &options.share_with {
        log::info!("Leaving out the single gumps this gumps_cc draws the same: {}", share_with.display());
    }
    if let Some(interface_uop) = find_interface_uop(source_dirs) {
        println!(
            "Using EC gump source file (UOP): {}",
            source_path_label_from_dirs(source_dirs, &interface_uop)
        );
        convert_ec_gumps_from_interface_uop(
            &interface_uop,
            output_path,
            options,
            shared_with.as_ref(),
            &payload_progress,
            package_progress,
        )
    } else if let Some(gumpart_dir) = find_ec_gumpart_dir(source_dirs) {
        println!(
            "Using EC gump extracted source directory: {}",
            source_path_label_from_dirs(source_dirs, &gumpart_dir)
        );
        convert_ec_gumps_from_extracted_dir(
            &gumpart_dir,
            output_path,
            options,
            shared_with.as_ref(),
            &payload_progress,
            package_progress,
        )
    } else {
        eyre::bail!(
            "missing EC gump source: expected Interface.uop/interface.uop, or an extracted data/interface/default/textures/gumpart directory"
        );
    }
}

/// The Interface.uop of the first source folder that holds one, in any casing: the Enhanced gump source read first.
pub fn find_interface_uop(source_dirs: &[PathBuf]) -> Option<PathBuf> {
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
    shared_with: Option<&Arc<GumpsPackage>>,
    task_progress: &(impl Fn(AssetTaskProgress) + Sync),
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<EcGumpsBuildSummary> {
    let package = UopPackage::load_with_mode(interface_uop, LoadMode::Lazy)
        .wrap_err_with(|| format!("load {}", interface_uop.display()))?;
    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut gump_count = 0u32;
    let mut single_gumps = single_gump_writer(options, shared_with);
    let mut atlas_gumps = Vec::new();
    let mut skipped_count = 0u32;

    let pb = ProgressBar::new(u64::from(options.max_id) + 1);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message("extracting EC gumps from Interface.uop");
    let gump_id_count = u64::from(options.max_id) + 1;
    task_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 0,
        total: gump_id_count,
    });

    for gump_id in 0..=options.max_id {
        pb.inc(1);
        task_progress(AssetTaskProgress {
            stage: AssetTaskProgressStage::Extracting,
            completed: u64::from(gump_id) + 1,
            total: gump_id_count,
        });
        let Some((format, payload)) = unpack_ec_gump_payload(&package, gump_id)? else {
            continue;
        };

        match decode_ec_gump_payload(gump_id, format, &payload) {
            Ok(decoded) => {
                if is_paperdoll_equipment_gump_id(gump_id) {
                    let passes = gump_upscale_passes_for(
                        options,
                        UpscaleImageType::GumpsEquip,
                        gump_id,
                        &options.paperdoll_upscale_passes,
                    );
                    atlas_gumps.push(upscale_decoded_gump(decoded, &passes));
                } else {
                    let passes = gump_upscale_passes_for(
                        options,
                        UpscaleImageType::GumpsNonEquip,
                        gump_id,
                        &options.single_upscale_passes,
                    );
                    let decoded = upscale_decoded_gump(decoded, &passes);
                    single_gumps.push(
                        &mut builder,
                        decoded.gump_id,
                        u32::from(decoded.width),
                        u32::from(decoded.height),
                        decoded.rgba,
                    )?;
                }
                gump_count += 1;
            }
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    let single_summary = single_gumps.finish(&mut builder)?;
    let atlas_gump_count = add_gump_atlas_files_with_progress(
        &mut builder,
        atlas_gumps,
        &GumpAtlasOptions {
            compression: options.atlas_compression,
            ..GumpAtlasOptions::default()
        },
        task_progress,
    )?;
    pb.finish_with_message("EC gumps extracted from Interface.uop");
    build_and_write_package_with_progress(builder, output_path, package_progress)?;

    Ok(EcGumpsBuildSummary {
        // A shared gump is left out of the package.
        gump_count: gump_count - single_summary.shared_classic_count,
        single_gump_count: single_summary.count,
        single_soft_alpha_count: single_summary.soft_alpha_count,
        single_classic_colour_space_count: single_summary.classic_colour_space_count,
        single_shared_classic_count: single_summary.shared_classic_count,
        single_kept_classic_lossier_count: single_summary.kept_classic_lossier_count,
        atlas_gump_count,
        skipped_count,
    })
}

fn convert_ec_gumps_from_extracted_dir(
    gumpart_dir: &Path,
    output_path: &Path,
    options: &EcGumpsOptions,
    shared_with: Option<&Arc<GumpsPackage>>,
    task_progress: &(impl Fn(AssetTaskProgress) + Sync),
    package_progress: impl FnMut(udd_container::BuildProgress),
) -> eyre::Result<EcGumpsBuildSummary> {
    let sources = collect_extracted_gumps(gumpart_dir)?;
    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut gump_count = 0u32;
    let mut single_gumps = single_gump_writer(options, shared_with);
    let mut atlas_gumps = Vec::new();
    let mut skipped_count = 0u32;

    let pb = ProgressBar::new(sources.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message("extracting EC gumps from extracted files");
    let source_count = sources.len() as u64;
    task_progress(AssetTaskProgress {
        stage: AssetTaskProgressStage::Extracting,
        completed: 0,
        total: source_count,
    });

    for (source_index, (gump_id, source)) in sources.into_iter().enumerate() {
        pb.inc(1);
        task_progress(AssetTaskProgress {
            stage: AssetTaskProgressStage::Extracting,
            completed: source_index as u64 + 1,
            total: source_count,
        });
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
                    let passes = gump_upscale_passes_for(
                        options,
                        UpscaleImageType::GumpsEquip,
                        gump_id,
                        &options.paperdoll_upscale_passes,
                    );
                    atlas_gumps.push(upscale_decoded_gump(decoded, &passes));
                } else {
                    let passes = gump_upscale_passes_for(
                        options,
                        UpscaleImageType::GumpsNonEquip,
                        gump_id,
                        &options.single_upscale_passes,
                    );
                    let decoded = upscale_decoded_gump(decoded, &passes);
                    single_gumps.push(
                        &mut builder,
                        decoded.gump_id,
                        u32::from(decoded.width),
                        u32::from(decoded.height),
                        decoded.rgba,
                    )?;
                }
                gump_count += 1;
            }
            Err(error) => {
                skipped_count += 1;
                log::warn!("Skipping EC gump {gump_id}: {error}");
            }
        }
    }

    let single_summary = single_gumps.finish(&mut builder)?;
    let atlas_gump_count = add_gump_atlas_files_with_progress(
        &mut builder,
        atlas_gumps,
        &GumpAtlasOptions {
            compression: options.atlas_compression,
            ..GumpAtlasOptions::default()
        },
        task_progress,
    )?;
    pb.finish_with_message("EC gumps extracted from extracted files");
    build_and_write_package_with_progress(builder, output_path, package_progress)?;

    Ok(EcGumpsBuildSummary {
        // A shared gump is left out of the package.
        gump_count: gump_count - single_summary.shared_classic_count,
        single_gump_count: single_summary.count,
        single_soft_alpha_count: single_summary.soft_alpha_count,
        single_classic_colour_space_count: single_summary.classic_colour_space_count,
        single_shared_classic_count: single_summary.shared_classic_count,
        single_kept_classic_lossier_count: single_summary.kept_classic_lossier_count,
        atlas_gump_count,
        skipped_count,
    })
}

fn unpack_ec_gump_payload(
    package: &UopPackage,
    gump_id: u32,
) -> eyre::Result<Option<(ECImageFormat, Vec<u8>)>> {
    let candidates = ec_gump_candidates(gump_id);
    let candidate_refs = [candidates[0].0.as_str(), candidates[1].0.as_str()];
    let mut scratch_hashes = Vec::new();
    if let Some((index, hash, _file)) =
        package.get_file_by_name_batch(&candidate_refs, &mut scratch_hashes)
    {
        let format = candidates[index].1;
        let payload = package
            .unpack_file_by_hash(hash)?
            .ok_or_else(|| eyre::eyre!("EC gump entry disappeared from interface.uop"))?;
        return Ok(Some((infer_ec_image_format(&payload, format), payload)));
    }
    Ok(None)
}

fn upscale_decoded_gump(mut decoded: DecodedGump, passes: &[UpscalePass]) -> DecodedGump {
    let (width, height, rgba, upscale_factor, _) = apply_upscale_passes(
        u32::from(decoded.width),
        u32::from(decoded.height),
        &decoded.rgba,
        passes,
    );
    decoded.width = width as u16;
    decoded.height = height as u16;
    decoded.upscale_factor = upscale_factor as u16;
    decoded.rgba = rgba;
    decoded
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
        image_data_offset: 0,
    };
    let rgba = tex_file.decode_to_rgba8()?;
    let width = rgba.width();
    let height = rgba.height();
    if width > u16::MAX as u32 || height > u16::MAX as u32 {
        eyre::bail!("EC gump {gump_id} dimensions exceed u16 metadata bounds: {width}x{height}");
    }
    Ok(DecodedGump {
        gump_id,
        width: width as u16,
        height: height as u16,
        upscale_factor: 1,
        rgba: rgba.into_raw(),
    })
}

/// The extracted gumpart folder the Enhanced gumps are read from when no source folder holds an Interface.uop: a
/// source folder's `data/interface/default/textures/gumpart`, or the source folder itself when it holds Enhanced gumps.
pub fn find_ec_gumpart_dir(source_dirs: &[PathBuf]) -> Option<PathBuf> {
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

        let found = find_interface_uop(std::slice::from_ref(&root)).expect("find Interface.uop");

        assert_eq!(found.file_name().and_then(|name| name.to_str()), Some("Interface.uop"));
        fs::remove_dir_all(root).ok();
    }

    /// Packing lossy, a gump in the Classic colour space goes BC1 and another BC7; lossless, both stay RGBA8888.
    #[test]
    fn lossy_pack_stores_classic_colour_space_gumps_as_bc1() {
        use crate::single_gumps::default_single_gump_compression;
        use udd_assets::gumps::GumpsPackage;
        use udd_container::UddpReader;
        const SIDE: u32 = 4;
        const CLASSIC_GUMP_ID: u32 = 2000;
        const ENHANCED_GUMP_ID: u32 = 2001;
        const CLASSIC_OPAQUE: [u8; 4] = [248, 0, 120, 255];
        const ENHANCED_OPAQUE: [u8; 4] = [247, 0, 120, 255];
        let gumps = [(CLASSIC_GUMP_ID, CLASSIC_OPAQUE), (ENHANCED_GUMP_ID, ENHANCED_OPAQUE)];
        let lossy = EcGumpsOptions {
            single_pixel_format: EC_LOSSY_SINGLE_GUMP_FORMAT,
            compression: default_single_gump_compression(EC_LOSSY_SINGLE_GUMP_FORMAT),
            ..EcGumpsOptions::default()
        };

        for (options, classic_format, enhanced_format) in [
            (&lossy, EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT, EC_LOSSY_SINGLE_GUMP_FORMAT),
            (&EcGumpsOptions::default(), GumpPixelFormat::Rgba8888, GumpPixelFormat::Rgba8888),
        ] {
            let mut builder = UddpBuilder::new(LookupMode::SparseId);
            let mut writer = single_gump_writer(options, None);
            for (gump_id, pixel) in gumps {
                writer.push(&mut builder, gump_id, SIDE, SIDE, pixel.repeat((SIDE * SIDE) as usize)).unwrap();
            }
            writer.finish(&mut builder).unwrap();
            crate::gump_atlas::add_gump_atlas_files(&mut builder, Vec::new(), &GumpAtlasOptions::default()).unwrap();

            let package = GumpsPackage::from_uddp_package(UddpReader::open(builder.build().unwrap()).unwrap()).unwrap();
            assert_eq!(package.single_gump_pixel_format(CLASSIC_GUMP_ID).unwrap(), Some(classic_format));
            assert_eq!(package.single_gump_pixel_format(ENHANCED_GUMP_ID).unwrap(), Some(enhanced_format));
        }
    }

    /// Side of the square synthetic gumps the progress tests pack.
    const SYNTHETIC_GUMP_SIDE: u16 = 2;
    /// Gump ids of the synthetic extracted gumpart folder.
    const SYNTHETIC_GUMP_IDS: [u32; 3] = [1000, 1001, 1002];
    const TGA_HEADER_LEN: usize = 18;
    const TGA_IMAGE_TYPE_OFFSET: usize = 2;
    const TGA_UNCOMPRESSED_TRUE_COLOUR: u8 = 2;
    const TGA_WIDTH_OFFSET: usize = 12;
    const TGA_HEIGHT_OFFSET: usize = 14;
    const TGA_BITS_PER_PIXEL_OFFSET: usize = 16;
    const TGA_BITS_PER_PIXEL: u8 = 32;
    const TGA_DESCRIPTOR_OFFSET: usize = 17;
    /// Eight alpha bits, rows stored top to bottom.
    const TGA_DESCRIPTOR_TOP_LEFT_ALPHA8: u8 = 0x28;
    const OPAQUE_BGRA: [u8; 4] = [10, 20, 30, 255];

    /// An uncompressed 32-bit TGA of `side` x `side` opaque pixels.
    fn solid_tga(side: u16) -> Vec<u8> {
        let mut tga = vec![0u8; TGA_HEADER_LEN];
        tga[TGA_IMAGE_TYPE_OFFSET] = TGA_UNCOMPRESSED_TRUE_COLOUR;
        tga[TGA_WIDTH_OFFSET..TGA_WIDTH_OFFSET + 2].copy_from_slice(&side.to_le_bytes());
        tga[TGA_HEIGHT_OFFSET..TGA_HEIGHT_OFFSET + 2].copy_from_slice(&side.to_le_bytes());
        tga[TGA_BITS_PER_PIXEL_OFFSET] = TGA_BITS_PER_PIXEL;
        tga[TGA_DESCRIPTOR_OFFSET] = TGA_DESCRIPTOR_TOP_LEFT_ALPHA8;
        tga.extend(OPAQUE_BGRA.repeat(usize::from(side) * usize::from(side)));
        tga
    }

    /// A temporary folder of extracted gumpart TGAs, one per [`SYNTHETIC_GUMP_IDS`].
    fn synthetic_gumpart_dir(label: &str) -> PathBuf {
        let dir = unique_temp_dir(label);
        fs::create_dir_all(&dir).expect("create gumpart dir");
        for gump_id in SYNTHETIC_GUMP_IDS {
            fs::write(dir.join(format!("{gump_id:08}.tga")), solid_tga(SYNTHETIC_GUMP_SIDE)).expect("write tga");
        }
        dir
    }

    /// The payload the progress callback panics with to cancel the job.
    struct Cancelled;

    #[test]
    fn progress_reports_every_extracted_gump_then_the_package_build() {
        let dir = synthetic_gumpart_dir("udd-conv-ec-gumps-progress");
        let output = dir.join(GUMPS_EC_DEFAULT_OUTPUT);
        let payload = std::sync::Mutex::new(Vec::new());
        let mut phases = Vec::new();

        let summary = convert_ec_gumps_to_uddp_from_sources_with_progress(
            &[dir.clone()],
            &output,
            &EcGumpsOptions::default(),
            |progress| payload.lock().unwrap().push(progress),
            |progress: udd_container::BuildProgress| phases.push(progress.phase),
        )
        .expect("pack synthetic gumps");

        let total = SYNTHETIC_GUMP_IDS.len() as u64;
        let payload = payload.into_inner().unwrap();
        assert_eq!(summary.gump_count, SYNTHETIC_GUMP_IDS.len() as u32);
        assert_eq!(
            payload.iter().filter(|progress| progress.stage == AssetTaskProgressStage::Extracting).last(),
            Some(&AssetTaskProgress { stage: AssetTaskProgressStage::Extracting, completed: total, total })
        );
        assert!(payload.iter().any(|progress| progress.stage == AssetTaskProgressStage::PackingAtlas));
        assert!(phases.contains(&udd_container::BuildProgressPhase::Assembling));
        assert!(output.is_file());
        fs::remove_dir_all(&dir).expect("remove gumpart dir");
    }

    /// A job cancelled from its progress callback mid-extraction unwinds before the package file is created.
    #[test]
    fn cancelling_from_the_progress_callback_leaves_no_package() {
        const CANCEL_AFTER_GUMPS: u64 = 2;
        let dir = synthetic_gumpart_dir("udd-conv-ec-gumps-cancel");
        let output = dir.join(GUMPS_EC_DEFAULT_OUTPUT);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            convert_ec_gumps_to_uddp_from_sources_with_progress(
                &[dir.clone()],
                &output,
                &EcGumpsOptions::default(),
                |progress| {
                    if progress.stage == AssetTaskProgressStage::Extracting && progress.completed >= CANCEL_AFTER_GUMPS {
                        std::panic::panic_any(Cancelled);
                    }
                },
                |_| {},
            )
        }));

        let payload = result.err().expect("the job unwinds");
        assert!(payload.is::<Cancelled>());
        assert!(!output.exists());
        fs::remove_dir_all(&dir).expect("remove gumpart dir");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{}-{nanos}", std::process::id()))
    }
}
