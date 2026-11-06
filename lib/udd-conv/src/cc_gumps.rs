//! Build-time support for `gumps_cc.uddp`.

use color_eyre::eyre::{self, WrapErr};
use crate::progress::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uocf::classic::gump::GumpMap;
use udd_container::{CompressionFlag, LookupMode, UddpBuilder};

use crate::classic_patches::{load_verdata_if_enabled, ClassicPatchOptions};
use crate::classic_sources::{resolve_classic_gump_source, SourceFormat, SourceFormatPreference};
use crate::gump_atlas::{
    add_gump_atlas_files, is_paperdoll_equipment_gump_id, DecodedGump, GumpAtlasOptions,
};
use crate::package_progress::build_and_write_package;
use crate::page_compression::LOSSLESS_PAGE_COMPRESSION;
use crate::single_gumps::{default_single_gump_compression, GumpPixelFormat, SingleGumpWriter};
use crate::source_paths::source_path_label;
use crate::upscale::{apply_upscale_passes, UpscalePass};
use crate::upscale_profile::{UpscaleImageType, UpscaleProfile, UpscaleTarget};

pub const GUMPS_CC_DEFAULT_OUTPUT: &str = "gumps_cc.uddp";

pub struct CcGumpsBuildSummary {
    pub gump_count: u32,
    pub single_gump_count: u32,
    /// Single gumps with alpha neither 0 nor 255, which BC1 rounds.
    pub single_soft_alpha_count: u32,
    pub atlas_gump_count: u32,
}

pub struct CcGumpsOptions {
    /// How single gumps are stored: lossless RGBA8888, or BC1 when packing lossy. Paperdoll gumps stay lossless.
    pub single_pixel_format: GumpPixelFormat,
    /// The builder codec of single gumps.
    pub compression: CompressionFlag,
    pub atlas_compression: CompressionFlag,
    pub paperdoll_upscale_passes: Vec<UpscalePass>,
    pub single_upscale_passes: Vec<UpscalePass>,
    pub upscale_profile: Option<Arc<UpscaleProfile>>,
    pub source_preference: SourceFormatPreference,
}

impl Default for CcGumpsOptions {
    fn default() -> Self {
        Self {
            single_pixel_format: GumpPixelFormat::Rgba8888,
            compression: default_single_gump_compression(GumpPixelFormat::Rgba8888),
            atlas_compression: LOSSLESS_PAGE_COMPRESSION,
            paperdoll_upscale_passes: Vec::new(),
            single_upscale_passes: Vec::new(),
            upscale_profile: None,
            source_preference: SourceFormatPreference::Mul,
        }
    }
}

fn gump_upscale_passes_for(
    options: &CcGumpsOptions,
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

pub fn convert_gumps_to_uddp_from_sources_with_patches(
    source_dirs: &[PathBuf],
    output_path: &Path,
    patch_options: &ClassicPatchOptions,
) -> eyre::Result<CcGumpsBuildSummary> {
    convert_gumps_to_uddp_from_sources_with_patches_and_options(
        source_dirs,
        output_path,
        patch_options,
        &CcGumpsOptions::default(),
    )
}

pub fn convert_gumps_to_uddp_from_sources_with_patches_and_options(
    source_dirs: &[PathBuf],
    output_path: &Path,
    patch_options: &ClassicPatchOptions,
    options: &CcGumpsOptions,
) -> eyre::Result<CcGumpsBuildSummary> {
    let gump_source = resolve_classic_gump_source(source_dirs, options.source_preference)?;
    println!(
        "Using CC gump source file ({}): {}",
        gump_source.format.label(),
        source_path_label(&gump_source.root, &gump_source.primary_path)
    );

    let mut gumps = gump_source
        .load_gump_map()
        .wrap_err_with(|| format!("load Classic gumps from {}", gump_source.root.display()))?;
    if gump_source.format == SourceFormat::Mul {
        if let Some(verdata) = load_verdata_if_enabled(source_dirs, patch_options)? {
            gumps = gumps.with_verdata(verdata);
        }
    } else if patch_options.any() {
        log::warn!("Classic gump patch files are ignored when converting gumps from UOP.");
    }

    let max_id = gumps.max_id();
    let pb = ProgressBar::new(u64::from(max_id) + 1);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg} ({eta})")
            .unwrap()
            .progress_chars("#>-"),
    );
    pb.set_message("extracting CC gumps");

    let mut builder = UddpBuilder::new(LookupMode::SparseId);
    let mut scratch = Vec::new();
    let mut gump_count = 0u32;
    let mut single_gumps = SingleGumpWriter::new(options.single_pixel_format, options.compression);
    let mut atlas_gumps = Vec::new();

    for gump_id in 0..=max_id {
        pb.inc(1);
        if !gumps.has_id(gump_id) {
            continue;
        }

        let (width, height, rgba) = match gumps.decode_gump(gump_id, &mut scratch) {
            Ok(decoded) => decoded,
            Err(error) => {
                log::warn!("Skipping gump {gump_id}: {error}");
                continue;
            }
        };

        if is_paperdoll_equipment_gump_id(gump_id) {
            let passes = gump_upscale_passes_for(
                options,
                UpscaleImageType::GumpsEquip,
                gump_id,
                &options.paperdoll_upscale_passes,
            );
            let (width, height, rgba, upscale_factor, _) = apply_upscale_passes(
                u32::from(width),
                u32::from(height),
                &rgba,
                &passes,
            );
            atlas_gumps.push(DecodedGump {
                gump_id,
                width: width as u16,
                height: height as u16,
                upscale_factor: upscale_factor as u16,
                rgba,
            });
        } else {
            let passes = gump_upscale_passes_for(
                options,
                UpscaleImageType::GumpsNonEquip,
                gump_id,
                &options.single_upscale_passes,
            );
            let (width, height, rgba, _, _) = apply_upscale_passes(
                u32::from(width),
                u32::from(height),
                &rgba,
                &passes,
            );
            single_gumps.push(&mut builder, gump_id, width, height, rgba)?;
        }
        gump_count += 1;
    }

    let single_summary = single_gumps.finish(&mut builder)?;
    let atlas_gump_count = add_gump_atlas_files(
        &mut builder,
        atlas_gumps,
        &GumpAtlasOptions {
            compression: options.atlas_compression,
            ..GumpAtlasOptions::default()
        },
    )?;
    pb.finish_with_message("CC gumps extracted");
    build_and_write_package(builder, output_path)?;

    Ok(CcGumpsBuildSummary {
        gump_count,
        single_gump_count: single_summary.count,
        single_soft_alpha_count: single_summary.soft_alpha_count,
        atlas_gump_count,
    })
}
