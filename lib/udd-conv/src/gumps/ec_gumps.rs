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
    default_single_gump_compression, ensure_single_gump_format_for_package, is_paperdoll_body_gump_id, GumpPixelFormat,
    SingleGumpWriter,
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

/// The brightest channel a glow texel has ([`glow_texels`]).
const GLOW_DARK_MAX_CHANNEL: u8 = 32;
/// The fewest texels a glow region has ([`glow_texels`]).
const GLOW_MIN_TEXELS: usize = 1024;
/// `UOPaperdoll::BuildImage` (UOSA 0x0050c790) skips a gump of this width and height, so the bake leaves it as it is.
const ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE: u16 = 256;
const RGBA_BYTES: usize = 4;
/// A texel's R, G and B, all `UOPaperdoll::DrawGumpLayer` reads of it.
const RGB_BYTES: usize = 3;
const ALPHA_BYTE: usize = 3;
const PURE_BLACK: [u8; RGB_BYTES] = [0; RGB_BYTES];
const TRANSPARENT_BLACK: [u8; RGBA_BYTES] = [0; RGBA_BYTES];

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
            Ok(mut decoded) => {
                // Baked before the upscale passes, on the texels the client reads.
                if is_enhanced_paperdoll_gump_id(gump_id) {
                    bake_enhanced_paperdoll_rule(&mut decoded);
                }
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
            Ok(mut decoded) => {
                // Baked before the upscale passes, on the texels the client reads.
                if is_enhanced_paperdoll_gump_id(gump_id) {
                    bake_enhanced_paperdoll_rule(&mut decoded);
                }
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

/// The gumps the Enhanced paperdoll composes: the equipment gumps on atlas pages and the body gumps under them.
fn is_enhanced_paperdoll_gump_id(gump_id: u32) -> bool {
    is_paperdoll_equipment_gump_id(gump_id) || is_paperdoll_body_gump_id(gump_id)
}

/// Bakes the Enhanced paperdoll's drawn rule into a paperdoll gump's alpha at packing time (the user, 2026-11-06), so a
/// paperdoll composes its layers by plain alpha blending. `UOPaperdoll::DrawGumpLayer` (UOSA 0x00509b30) never reads
/// alpha: it draws each texel whose R, G, B differ from texel 0's (the key), and `UOPaperdoll::BuildImage` (0x0050c790)
/// then gives the canvas alpha 255 where its colour isn't pure black and 0 where it is
/// (`scripts/dev-helpers/ec-research/ghidra-paperdoll/results/uosa_paperdoll.md`, items 15 and 16).
///
/// A drawn texel, one off the key that isn't pure black, is stored opaque with its colour whatever alpha the art gave
/// it: the client draws the clear handle of gump 50669 and the clear trident of 60675, and body 665's soft texels
/// opaque. Every other texel is stored transparent black. Two departures from the client: a pure black texel leaves the
/// layers under it showing, where the client's canvas pass clears them too; and a glow region ([`glow_texels`]), which
/// the client draws as a dark patch, is left out. A 256x256 gump, which `BuildImage` skips, is left as it is.
fn bake_enhanced_paperdoll_rule(gump: &mut DecodedGump) {
    if gump.width == ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE && gump.height == ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE {
        return;
    }
    let Some(&[red, green, blue]) = gump.rgba.first_chunk::<RGB_BYTES>() else {
        return;
    };
    let key = [red, green, blue];
    let glow = glow_texels(&gump.rgba, usize::from(gump.width), usize::from(gump.height), key);
    for (texel, in_glow) in gump.rgba.chunks_exact_mut(RGBA_BYTES).zip(glow) {
        let rgb = &texel[..RGB_BYTES];
        if rgb != key && rgb != PURE_BLACK && !in_glow {
            texel[ALPHA_BYTE] = u8::MAX;
        } else {
            texel.copy_from_slice(&TRANSPARENT_BLACK);
        }
    }
}

/// Which texels of a `width` x `height` gump lie in a glow region (rule E of the 2026-11-06 Enhanced paperdoll
/// comparison): an 8-connected region of at least [`GLOW_MIN_TEXELS`] clear (alpha 0) texels off the `key` whose
/// brightest channel is at most [`GLOW_DARK_MAX_CHANNEL`]. The art leaves it clear and the client, reading no alpha,
/// draws it dark; of the Enhanced paperdoll gumps only 50322 and 60322 have one, of 10,844 texels each.
fn glow_texels(rgba: &[u8], width: usize, height: usize, key: [u8; RGB_BYTES]) -> Vec<bool> {
    let dark = rgba
        .chunks_exact(RGBA_BYTES)
        .map(|texel| {
            texel[ALPHA_BYTE] == 0
                && texel[..RGB_BYTES] != key
                && texel[..RGB_BYTES].iter().all(|&channel| channel <= GLOW_DARK_MAX_CHANNEL)
        })
        .collect::<Vec<_>>();
    let mut glow = vec![false; dark.len()];
    let mut seen = vec![false; dark.len()];
    let mut region = Vec::new();
    let mut pending = Vec::new();
    for start in 0..dark.len() {
        if !dark[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        pending.push(start);
        region.clear();
        while let Some(index) = pending.pop() {
            region.push(index);
            let (x, y) = (index % width, index / width);
            for neighbour_y in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                for neighbour_x in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let neighbour = neighbour_y * width + neighbour_x;
                    if dark[neighbour] && !seen[neighbour] {
                        seen[neighbour] = true;
                        pending.push(neighbour);
                    }
                }
            }
        }
        if region.len() >= GLOW_MIN_TEXELS {
            for &index in &region {
                glow[index] = true;
            }
        }
    }
    glow
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

    /// The key colour many Enhanced paperdoll gumps keep under their clear texels.
    const KEY: [u8; RGB_BYTES] = [152, 112, 104];
    const BAKE_SIDE: usize = 40;
    /// A 32x32 block of dark clear texels: exactly [`GLOW_MIN_TEXELS`].
    const GLOW_START: usize = 4;
    const GLOW_SIDE: usize = 32;

    fn texel_at(rgba: &[u8], x: usize, y: usize) -> [u8; RGBA_BYTES] {
        let start = (y * BAKE_SIDE + x) * RGBA_BYTES;
        rgba[start..start + RGBA_BYTES].try_into().unwrap()
    }

    fn set_texel(rgba: &mut [u8], x: usize, y: usize, texel: [u8; RGBA_BYTES]) {
        let start = (y * BAKE_SIDE + x) * RGBA_BYTES;
        rgba[start..start + RGBA_BYTES].copy_from_slice(&texel);
    }

    /// The bake stores what the Enhanced paperdoll draws: off the key and not pure black, opaque whatever the art's
    /// alpha; the key, pure black and a glow region transparent black, while a small dark clear region is drawn.
    #[test]
    fn bake_keeps_what_the_enhanced_paperdoll_draws() {
        const KEYED_CLEAR: [u8; RGBA_BYTES] = [KEY[0], KEY[1], KEY[2], 0];
        const KEYED_OPAQUE: [u8; RGBA_BYTES] = [KEY[0], KEY[1], KEY[2], u8::MAX];
        const OPAQUE: [u8; RGBA_BYTES] = [200, 150, 90, u8::MAX];
        const STALE_CLEAR: [u8; RGBA_BYTES] = [180, 160, 70, 0];
        const SOFT: [u8; RGBA_BYTES] = [100, 100, 100, 128];
        const BLACK_OPAQUE: [u8; RGBA_BYTES] = [0, 0, 0, u8::MAX];
        const DARK_CLEAR: [u8; RGBA_BYTES] = [10, 12, GLOW_DARK_MAX_CHANNEL, 0];
        let mut rgba = KEYED_CLEAR.repeat(BAKE_SIDE * BAKE_SIDE);
        for y in GLOW_START..GLOW_START + GLOW_SIDE {
            for x in GLOW_START..GLOW_START + GLOW_SIDE {
                set_texel(&mut rgba, x, y, DARK_CLEAR);
            }
        }
        let small_dark = [(0, 38), (1, 38), (0, 39), (1, 39)];
        for (x, y) in small_dark {
            set_texel(&mut rgba, x, y, DARK_CLEAR);
        }
        let singles = [
            ((39, 0), KEYED_OPAQUE, TRANSPARENT_BLACK),
            ((39, 2), OPAQUE, OPAQUE),
            ((39, 4), STALE_CLEAR, [180, 160, 70, u8::MAX]),
            ((39, 6), SOFT, [100, 100, 100, u8::MAX]),
            ((39, 8), BLACK_OPAQUE, TRANSPARENT_BLACK),
        ];
        for ((x, y), texel, _) in singles {
            set_texel(&mut rgba, x, y, texel);
        }
        let mut gump =
            DecodedGump { gump_id: 50_669, width: BAKE_SIDE as u16, height: BAKE_SIDE as u16, upscale_factor: 1, rgba };

        bake_enhanced_paperdoll_rule(&mut gump);

        for ((x, y), texel, baked) in singles {
            assert_eq!(texel_at(&gump.rgba, x, y), baked, "{texel:?} at {x},{y}");
        }
        assert_eq!(texel_at(&gump.rgba, 0, 0), TRANSPARENT_BLACK, "the key");
        for (x, y) in small_dark {
            assert_eq!(texel_at(&gump.rgba, x, y), [10, 12, GLOW_DARK_MAX_CHANNEL, u8::MAX], "small dark region");
        }
        for y in GLOW_START..GLOW_START + GLOW_SIDE {
            for x in GLOW_START..GLOW_START + GLOW_SIDE {
                assert_eq!(texel_at(&gump.rgba, x, y), TRANSPARENT_BLACK, "glow at {x},{y}");
            }
        }
    }

    /// A dark clear region one texel short of [`GLOW_MIN_TEXELS`] is drawn, and one a shade too bright for a glow is
    /// drawn whatever its size; on a black key, black isn't dark either.
    #[test]
    fn bake_leaves_small_or_bright_regions_drawn() {
        const BLACK_KEY_CLEAR: [u8; RGBA_BYTES] = [0, 0, 0, 0];
        const TOO_BRIGHT: u8 = GLOW_DARK_MAX_CHANNEL + 1;
        for (dark, region_len) in [([5, 5, 5, 0], GLOW_MIN_TEXELS - 1), ([TOO_BRIGHT, 0, 0, 0], GLOW_MIN_TEXELS)] {
            let mut rgba = BLACK_KEY_CLEAR.repeat(BAKE_SIDE * BAKE_SIDE);
            for index in 0..region_len {
                set_texel(&mut rgba, 1 + index % GLOW_SIDE, 1 + index / GLOW_SIDE, dark);
            }
            let mut gump =
                DecodedGump { gump_id: 60_322, width: BAKE_SIDE as u16, height: BAKE_SIDE as u16, upscale_factor: 1, rgba };

            bake_enhanced_paperdoll_rule(&mut gump);

            let drawn = gump.rgba.chunks_exact(RGBA_BYTES).filter(|texel| texel[ALPHA_BYTE] == u8::MAX).count();
            assert_eq!(drawn, region_len, "{dark:?}");
            assert_eq!(texel_at(&gump.rgba, 1, 1), [dark[0], dark[1], dark[2], u8::MAX]);
        }
    }

    /// The client skips a 256x256 gump, so the bake leaves it as the art has it.
    #[test]
    fn bake_leaves_256x256_gumps_alone() {
        let side = usize::from(ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE);
        let rgba = [KEY[0], KEY[1], KEY[2], 0].repeat(side * side);
        let mut gump = DecodedGump {
            gump_id: 50_001,
            width: ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE,
            height: ENHANCED_PAPERDOLL_SKIPPED_GUMP_SIZE,
            upscale_factor: 1,
            rgba: rgba.clone(),
        };
        bake_enhanced_paperdoll_rule(&mut gump);
        assert_eq!(gump.rgba, rgba);
    }

    #[test]
    fn enhanced_paperdoll_gumps_are_the_equipment_and_body_gumps() {
        for gump_id in [12, 13, 14, 15, 665, 666, 50_001, 51_253, 60_322, 69_999] {
            assert!(is_enhanced_paperdoll_gump_id(gump_id), "{gump_id}");
        }
        for gump_id in [0, 11, 16, 664, 667, 50_000, 70_000] {
            assert!(!is_enhanced_paperdoll_gump_id(gump_id), "{gump_id}");
        }
    }

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
