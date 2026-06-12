use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ec_material_audit::{
    audit_ec_material_refs, audit_ec_surface_redirection, audit_ec_terrain_definition_kdl,
    audit_ec_terrain_overrides, audit_ec_terrain_primary_selection, inventory_ec_support_textures,
    write_ec_material_baseline_report, write_ec_terrain_override_candidates,
    write_ec_terrain_practical_review,
};
use crate::upscale_profile_config::load_upscale_profile;
use clap::{ArgGroup, Args, Parser, Subcommand};
use color_eyre::eyre;
use serde::Serialize;
use udd_conv::{
    classic_patches::ClassicPatchOptions,
    classic_sources::SourceFormatPreference,
    cc_gumps::{
        convert_gumps_to_uddp_from_sources_with_patches_and_options, CcGumpsOptions,
        GUMPS_CC_DEFAULT_OUTPUT,
    },
    cc_map::{convert_map_mul_to_uddp_from_sources_with_patches, CcMapSourcePreference},
    cc_statics::{
        convert_statics_mul_to_uddp_from_sources_with_dimensions_and_patches,
        convert_statics_mul_to_uddp_from_sources_with_patches,
    },
    ec_gumps::{
        convert_ec_gumps_to_uddp_from_sources, EcGumpsOptions, EC_GUMP_DEFAULT_MAX_ID,
        GUMPS_EC_DEFAULT_OUTPUT,
    },
    hues::{convert_hues_to_hues_uddp_from_sources, HuesOptions},
    mobile_anim_cc::{
        convert_anim_mul_to_mobile_anim_cc_uddp_from_sources, MobileAnimCcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as CC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    mobile_anim_ec::{
        convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources, MobileAnimEcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as EC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    source_paths::{gather_source_dirs, resolve_output_path},
    tex_art_cc::{
        convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches, TexArtCcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as CC_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tex_art_ec::{
        convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources, load_tex_art_ec_sources,
        TexArtEcAtlasOptions, DEFAULT_ATLAS_GUTTER as EC_ART_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_ART_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tex_land_cc::{
        convert_texmaps_mul_to_tex_land_cc_uddp_with_patches, TexLandCcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as CC_TEXMAPS_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_TEXMAPS_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_TEXMAPS_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tex_land_ec::{
        convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_loaded_sources, TexLandEcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as EC_LAND_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tilemeta::{
        build_tilemeta_uddp_from_sources, build_tilemeta_uddp_from_split_sources,
        build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources, TileMetaBuildOptions,
    },
    upscale::{EnhancementFilter, UpscaleFilter, UpscalePass},
    world_lights::{convert_client_lights_to_world_lights_uddp, WorldLightsOptions},
    CompressionFlag, PagePixelFormat,
};

#[cfg(test)]
const DEFAULT_ZSTD_LEVEL: i32 = 7;
const DEFAULT_ZSTD_LEVEL_VALUE: &str = "7";
const DEFAULT_JXL_LEVEL: u8 = 6;

/// Pack UODynamapper runtime packages from Classic and Enhanced Client assets.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Clone)]
struct SourceDirArgs {
    #[arg(long)]
    ccdir: Option<PathBuf>,
    #[arg(long)]
    ecdir: Option<PathBuf>,
}

#[derive(Args, Clone, Copy, Debug, Default)]
struct ClassicPatchArgs {
    #[arg(long = "include-verdata", default_value_t = false, help = "Include verdata.mul patches when present.")]
    include_verdata: bool,
    #[arg(long = "include-map-difs", default_value_t = false, help = "Include mapdif/mapdifl patch files when present.")]
    include_map_difs: bool,
    #[arg(long = "include-static-difs", default_value_t = false, help = "Include stadif/stadifi/stadifl patch files when present.")]
    include_static_difs: bool,
}

impl From<ClassicPatchArgs> for ClassicPatchOptions {
    fn from(value: ClassicPatchArgs) -> Self {
        Self {
            verdata: value.include_verdata,
            map_difs: value.include_map_difs,
            static_difs: value.include_static_difs,
        }
    }
}

#[derive(Clone, Copy)]
struct TextureOutputFormat {
    compression: CompressionFlag,
    pixel_format: PagePixelFormat,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
    label: &'static str,
}

#[derive(Clone, Copy, Debug, Default)]
struct TextureOutputFormatArgs {
    raw: bool,
    jxl: bool,
    jxl_level: Option<u8>,
    zstd: Option<i32>,
    bc7: bool,
    bc7_rdo: bool,
}

fn zstd_compression(level: i32) -> CompressionFlag {
    CompressionFlag::ZstdNoDictLevel(level)
}

fn parse_bc7_rdo_lookback_blocks(value: &str) -> Result<usize, String> {
    let blocks = value
        .parse::<usize>()
        .map_err(|_| format!("expected one of 64, 128, or 256, got {value}"))?;
    if udd_conv::bc7::BC7_RDO_LOOKBACK_BLOCK_PRESETS.contains(&blocks) {
        Ok(blocks)
    } else {
        Err(format!("expected one of 64, 128, or 256, got {value}"))
    }
}

fn jxl_compression_level(level: Option<u8>) -> CompressionFlag {
    CompressionFlag::JpegXlLevel(level.unwrap_or(DEFAULT_JXL_LEVEL))
}

fn jxl_zstd_compression_levels(jxl_level: Option<u8>, zstd_level: i32) -> CompressionFlag {
    CompressionFlag::JpegXlZstdLevels {
        jxl_level: jxl_level.unwrap_or(DEFAULT_JXL_LEVEL),
        zstd_level,
    }
}

fn resolve_texture_output_format(
    raw: bool,
    jxl: bool,
    jxl_level: Option<u8>,
    zstd: Option<i32>,
    bc7: bool,
    bc7_rdo: bool,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> eyre::Result<TextureOutputFormat> {
    match (raw, jxl, bc7, bc7_rdo) {
        (false, false, false, false) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                jxl_zstd_compression_levels(jxl_level, level)
            } else {
                jxl_compression_level(jxl_level)
            },
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() {
                "RGBA8888 JXL + Zstd"
            } else {
                "RGBA8888 JXL"
            },
        }),
        (true, false, false, false) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::None
            },
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() { "RGBA8888 Zstd" } else { "RGBA8888 raw" },
        }),
        (false, true, false, false) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                jxl_zstd_compression_levels(jxl_level, level)
            } else {
                jxl_compression_level(jxl_level)
            },
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() {
                "RGBA8888 JXL + Zstd"
            } else {
                "RGBA8888 JXL"
            },
        }),
        (false, false, true, false) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::None
            },
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() { "BC7 + Zstd" } else { "BC7" },
        }),
        (false, false, false, true) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::None
            },
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() { "BC7 RDO + Zstd" } else { "BC7 RDO" },
        }),
        _ => eyre::bail!("select at most one output format: --raw, --jxl, --bc7, or --bc7-rdo"),
    }
}

fn resolve_texture_output_format_args(
    args: TextureOutputFormatArgs,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> eyre::Result<TextureOutputFormat> {
    resolve_texture_output_format(
        args.raw,
        args.jxl,
        args.jxl_level,
        args.zstd,
        args.bc7,
        args.bc7_rdo,
        bc7_rdo_lambda,
        bc7_rdo_lookback_blocks,
    )
}

fn resolve_mobile_anim_output_format(
    raw: bool,
    jxl: bool,
    jxl_level: Option<u8>,
    zstd: Option<i32>,
    bc7: bool,
    bc7_rdo: bool,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> eyre::Result<TextureOutputFormat> {
    match (raw, jxl, bc7, bc7_rdo) {
        (false, false, false, false) => Ok(TextureOutputFormat {
            compression: if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::None
            },
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: if zstd.is_some() { "BC7 + Zstd" } else { "BC7" },
        }),
        _ => resolve_texture_output_format(
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
        ),
    }
}

fn percent_string(part: u64, total: u64) -> String {
    if total == 0 {
        "0.00%".to_string()
    } else {
        format!("{:.2}%", part as f64 * 100.0 / total as f64)
    }
}

fn estimated_bc7_bytes(pixel_count: u64, pixel_format: PagePixelFormat) -> u64 {
    match pixel_format {
        PagePixelFormat::Bc7 => pixel_count,
        PagePixelFormat::Rgba8888 => pixel_count * 4,
    }
}

fn collect_source_dirs(args: &SourceDirArgs) -> eyre::Result<Vec<PathBuf>> {
    validate_optional_source_dir(args.ccdir.as_ref(), "--ccdir")?;
    validate_optional_source_dir(args.ecdir.as_ref(), "--ecdir")?;

    let dirs = gather_source_dirs(args.ccdir.as_ref(), args.ecdir.as_ref());
    if dirs.is_empty() {
        eyre::bail!("at least one source root must be provided via --ccdir or --ecdir");
    }
    Ok(dirs)
}

fn collect_ec_source_dirs(args: &SourceDirArgs) -> eyre::Result<Vec<PathBuf>> {
    let ecdir = args
        .ecdir
        .as_ref()
        .ok_or_else(|| eyre::eyre!("--ecdir is required for EC source files"))?;
    validate_source_dir(ecdir, "--ecdir")?;
    Ok(vec![ecdir.clone()])
}

fn validate_optional_source_dir(path: Option<&PathBuf>, arg_name: &str) -> eyre::Result<()> {
    if let Some(path) = path {
        validate_source_dir(path, arg_name)?;
    }
    Ok(())
}

fn validate_source_dir(path: &Path, arg_name: &str) -> eyre::Result<()> {
    if !path.exists() {
        eyre::bail!("{arg_name} does not exist: {}", path.display());
    }
    if !path.is_dir() {
        eyre::bail!("{arg_name} is not a directory: {}", path.display());
    }
    Ok(())
}

#[derive(Serialize)]
struct LandRoutingAuditReport {
    routing_path: String,
    tex_land_ec_path: String,
    route_rows: usize,
    route_count: usize,
    unique_source_count: usize,
    target_material_count: usize,
    duplicate_source_count: usize,
    duplicate_sources: Vec<LandRoutingDuplicateSource>,
    missing_target_provenance_count: usize,
    missing_target_provenance: Vec<u32>,
    unresolved_route_count: usize,
    unresolved_routes: Vec<LandRoutingUnresolvedRoute>,
}

#[derive(Serialize)]
struct LandRoutingDuplicateSource {
    cc_id: u32,
    previous_target: u32,
    previous_row: usize,
    replacement_target: u32,
    replacement_row: usize,
}

#[derive(Serialize)]
struct LandRoutingUnresolvedRoute {
    cc_id: u32,
    target_material_id: u32,
    resolved_runtime_slot_id: Option<u32>,
}

fn audit_land_routing_kdl(
    routing_path: &Path,
    tex_land_ec_path: &Path,
    output: &Path,
) -> eyre::Result<()> {
    let routing = udd_assets::eckr_terrain_kdl::EckrTerrainRouting::load(routing_path)?;
    let route_count = routing
        .entries
        .iter()
        .map(|entry| entry.old_ids.len())
        .sum::<usize>();

    let mut route_map = HashMap::new();
    let mut first_seen: HashMap<u32, (u32, usize)> = HashMap::new();
    let mut duplicates = Vec::new();
    for (row_index, entry) in routing.entries.iter().enumerate() {
        let row_index = row_index + 1;
        for &cc_id in &entry.old_ids {
            if let Some((previous_target, previous_row)) =
                first_seen.insert(cc_id, (entry.new_id, row_index))
            {
                duplicates.push(LandRoutingDuplicateSource {
                    cc_id,
                    previous_target,
                    previous_row,
                    replacement_target: entry.new_id,
                    replacement_row: row_index,
                });
            }
            route_map.insert(cc_id, entry.new_id);
        }
    }

    let target_materials = route_map.values().copied().collect::<BTreeSet<_>>();
    let mut provenance_by_material: BTreeMap<u32, usize> = BTreeMap::new();
    let mut package = udd_assets::TexLandEcPackage::load(tex_land_ec_path)?;
    for record in package.terrain_provenance() {
        *provenance_by_material.entry(record.material_id).or_default() += 1;
    }
    package.set_transcode(route_map.clone());

    let missing_target_provenance = target_materials
        .iter()
        .copied()
        .filter(|target| !provenance_by_material.contains_key(target))
        .collect::<Vec<_>>();

    let mut unresolved_routes = Vec::new();
    for (&cc_id, &target_material_id) in &route_map {
        let resolved_runtime_slot_id = package.resolve_runtime_slot_id(cc_id);
        if resolved_runtime_slot_id
            .and_then(|slot_id| package.present_slot(slot_id).map(|_| slot_id))
            .is_none()
        {
            unresolved_routes.push(LandRoutingUnresolvedRoute {
                cc_id,
                target_material_id,
                resolved_runtime_slot_id,
            });
        }
    }
    unresolved_routes.sort_by_key(|route| (route.target_material_id, route.cc_id));

    let report = LandRoutingAuditReport {
        routing_path: routing_path.display().to_string(),
        tex_land_ec_path: tex_land_ec_path.display().to_string(),
        route_rows: routing.entries.len(),
        route_count,
        unique_source_count: route_map.len(),
        target_material_count: target_materials.len(),
        duplicate_source_count: duplicates.len(),
        duplicate_sources: duplicates,
        missing_target_provenance_count: missing_target_provenance.len(),
        missing_target_provenance,
        unresolved_route_count: unresolved_routes.len(),
        unresolved_routes,
    };

    let bytes = serde_json::to_vec_pretty(&report)?;
    fs::write(output, bytes)?;
    println!(
        "Wrote land routing audit with {} routes and {} unresolved routes to '{}'.",
        report.unique_source_count,
        report.unresolved_route_count,
        output.display()
    );
    Ok(())
}

fn find_raw_tilemeta_package(uddp_dir: &Path) -> eyre::Result<PathBuf> {
    ["tilemeta.uddp"]
        .into_iter()
        .map(|file_name| uddp_dir.join(file_name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            eyre::eyre!(
                "missing raw tilemeta package in '{}': expected tilemeta.uddp",
                uddp_dir.display()
            )
        })
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CliUpscaleFilter {
    #[default]
    None,
    #[value(name = "palette-snap-strict")]
    PaletteSnapStrict,
    #[value(name = "palette-snap-ramp-aware")]
    PaletteSnapRampAware,
    #[value(name = "palette-snap-expanded-8")]
    PaletteSnapExpanded8,
    #[value(name = "palette-snap-expanded-16")]
    PaletteSnapExpanded16,
    #[value(name = "palette-snap-expanded-32")]
    PaletteSnapExpanded32,
    #[value(name = "palette-dither-reinsert-checkerboard")]
    PaletteDitherReinsertCheckerboard,
    Nearest2x,
    Nearest3x,
    Nearest4x,
    Bilinear2x,
    Bilinear3x,
    Bilinear4x,
    CatmullRom2x,
    CatmullRom3x,
    CatmullRom4x,
    Lanczos3_2x,
    Lanczos3_3x,
    Lanczos3_4x,
    Lq2x,
    Lq3x,
    Lq4x,
    SuperSai2x,
    FsrEasu2x,
    FsrEasu3x,
    FsrEasu4x,
    FsrEasuRcas2x,
    FsrEasuRcas3x,
    FsrEasuRcas4x,
    KLDepixelize2x,
    KLDepixelize3x,
    KLDepixelize4x,
    Nedi2x,
    TwoSai2x,
    SuperEagle2x,
    Hq2xSimple,
    Hq3xSimple,
    Hq4xSimple,
    Hq2xTrue,
    Hq3xTrue,
    Hq4xTrue,
    Epx2x,
    Epx3x,
    Epx4x,
    Xbr2x,
    Xbr3x,
    Xbr4x,
    SuperXbr2x,
    Cut1_2x,
    Cut2_2x,
    Cut3_2x,
    ScaleFx2x,
    ScaleFx3x,
    ScaleFx4x,
    OmniScale2x,
    OmniScale3x,
    OmniScale4x,
    Jinc2_2x,
    Jinc2_3x,
    Jinc2_4x,
    Jinc2Sharp2x,
    Jinc2Sharp3x,
    Jinc2Sharp4x,
    Jinc2Sharper2x,
    Jinc2Sharper3x,
    Jinc2Sharper4x,
    Jinc2Sharpest2x,
    Jinc2Sharpest3x,
    Jinc2Sharpest4x,
    Mmpx2x,
    Mmpx4x,
    Vibrance,
    Saturation,
    SelectiveWarm,
    SelectiveGreen,
    LocalLaplacianClarity,
    ContrastEnhance,
    AdaptiveLogContrast,
    ScaleFxSmartDeblur,
    GuestrDeblur,
    UnsharpMask,
    HighPassSharpen,
}

impl From<CliUpscaleFilter> for UpscaleFilter {
    fn from(val: CliUpscaleFilter) -> Self {
        match val {
            CliUpscaleFilter::None => UpscaleFilter::None,
            CliUpscaleFilter::PaletteSnapStrict
            | CliUpscaleFilter::PaletteSnapRampAware
            | CliUpscaleFilter::PaletteSnapExpanded8
            | CliUpscaleFilter::PaletteSnapExpanded16
            | CliUpscaleFilter::PaletteSnapExpanded32
            | CliUpscaleFilter::PaletteDitherReinsertCheckerboard => UpscaleFilter::None,
            CliUpscaleFilter::Nearest2x => UpscaleFilter::Nearest2x,
            CliUpscaleFilter::Nearest3x => UpscaleFilter::Nearest3x,
            CliUpscaleFilter::Nearest4x => UpscaleFilter::Nearest4x,
            CliUpscaleFilter::Bilinear2x => UpscaleFilter::Bilinear2x,
            CliUpscaleFilter::Bilinear3x => UpscaleFilter::Bilinear3x,
            CliUpscaleFilter::Bilinear4x => UpscaleFilter::Bilinear4x,
            CliUpscaleFilter::CatmullRom2x => UpscaleFilter::CatmullRom2x,
            CliUpscaleFilter::CatmullRom3x => UpscaleFilter::CatmullRom3x,
            CliUpscaleFilter::CatmullRom4x => UpscaleFilter::CatmullRom4x,
            CliUpscaleFilter::Lanczos3_2x => UpscaleFilter::Lanczos3_2x,
            CliUpscaleFilter::Lanczos3_3x => UpscaleFilter::Lanczos3_3x,
            CliUpscaleFilter::Lanczos3_4x => UpscaleFilter::Lanczos3_4x,
            CliUpscaleFilter::Lq2x => UpscaleFilter::Lq2x,
            CliUpscaleFilter::Lq3x => UpscaleFilter::Lq3x,
            CliUpscaleFilter::Lq4x => UpscaleFilter::Lq4x,
            CliUpscaleFilter::SuperSai2x => UpscaleFilter::SuperSai2x,
            CliUpscaleFilter::FsrEasu2x => UpscaleFilter::FsrEasu2x,
            CliUpscaleFilter::FsrEasu3x => UpscaleFilter::FsrEasu3x,
            CliUpscaleFilter::FsrEasu4x => UpscaleFilter::FsrEasu4x,
            CliUpscaleFilter::FsrEasuRcas2x => UpscaleFilter::FsrEasuRcas2x,
            CliUpscaleFilter::FsrEasuRcas3x => UpscaleFilter::FsrEasuRcas3x,
            CliUpscaleFilter::FsrEasuRcas4x => UpscaleFilter::FsrEasuRcas4x,
            CliUpscaleFilter::KLDepixelize2x => UpscaleFilter::KLDepixelize2x,
            CliUpscaleFilter::KLDepixelize3x => UpscaleFilter::KLDepixelize3x,
            CliUpscaleFilter::KLDepixelize4x => UpscaleFilter::KLDepixelize4x,
            CliUpscaleFilter::Nedi2x => UpscaleFilter::Nedi2x,
            CliUpscaleFilter::TwoSai2x => UpscaleFilter::TwoSai2x,
            CliUpscaleFilter::SuperEagle2x => UpscaleFilter::SuperEagle2x,
            CliUpscaleFilter::Hq2xSimple => UpscaleFilter::Hq2xSimple,
            CliUpscaleFilter::Hq3xSimple => UpscaleFilter::Hq3xSimple,
            CliUpscaleFilter::Hq4xSimple => UpscaleFilter::Hq4xSimple,
            CliUpscaleFilter::Hq2xTrue => UpscaleFilter::Hq2xTrue,
            CliUpscaleFilter::Hq3xTrue => UpscaleFilter::Hq3xTrue,
            CliUpscaleFilter::Hq4xTrue => UpscaleFilter::Hq4xTrue,
            CliUpscaleFilter::Epx2x => UpscaleFilter::Epx2x,
            CliUpscaleFilter::Epx3x => UpscaleFilter::Epx3x,
            CliUpscaleFilter::Epx4x => UpscaleFilter::Epx4x,
            CliUpscaleFilter::Xbr2x => UpscaleFilter::Xbrz2x,
            CliUpscaleFilter::Xbr3x => UpscaleFilter::Xbrz3x,
            CliUpscaleFilter::Xbr4x => UpscaleFilter::Xbrz4x,
            CliUpscaleFilter::SuperXbr2x => UpscaleFilter::SuperXbr2x,
            CliUpscaleFilter::Cut1_2x => UpscaleFilter::Cut1_2x,
            CliUpscaleFilter::Cut2_2x => UpscaleFilter::Cut2_2x,
            CliUpscaleFilter::Cut3_2x => UpscaleFilter::Cut3_2x,
            CliUpscaleFilter::ScaleFx2x => UpscaleFilter::ScaleFx2x,
            CliUpscaleFilter::ScaleFx3x => UpscaleFilter::ScaleFx3x,
            CliUpscaleFilter::ScaleFx4x => UpscaleFilter::ScaleFx4x,
            CliUpscaleFilter::OmniScale2x => UpscaleFilter::OmniScale2x,
            CliUpscaleFilter::OmniScale3x => UpscaleFilter::OmniScale3x,
            CliUpscaleFilter::OmniScale4x => UpscaleFilter::OmniScale4x,
            CliUpscaleFilter::Jinc2_2x => UpscaleFilter::Jinc2_2x,
            CliUpscaleFilter::Jinc2_3x => UpscaleFilter::Jinc2_3x,
            CliUpscaleFilter::Jinc2_4x => UpscaleFilter::Jinc2_4x,
            CliUpscaleFilter::Jinc2Sharp2x => UpscaleFilter::Jinc2Sharp2x,
            CliUpscaleFilter::Jinc2Sharp3x => UpscaleFilter::Jinc2Sharp3x,
            CliUpscaleFilter::Jinc2Sharp4x => UpscaleFilter::Jinc2Sharp4x,
            CliUpscaleFilter::Jinc2Sharper2x => UpscaleFilter::Jinc2Sharper2x,
            CliUpscaleFilter::Jinc2Sharper3x => UpscaleFilter::Jinc2Sharper3x,
            CliUpscaleFilter::Jinc2Sharper4x => UpscaleFilter::Jinc2Sharper4x,
            CliUpscaleFilter::Jinc2Sharpest2x => UpscaleFilter::Jinc2Sharpest2x,
            CliUpscaleFilter::Jinc2Sharpest3x => UpscaleFilter::Jinc2Sharpest3x,
            CliUpscaleFilter::Jinc2Sharpest4x => UpscaleFilter::Jinc2Sharpest4x,
            CliUpscaleFilter::Mmpx2x => UpscaleFilter::Mmpx2x,
            CliUpscaleFilter::Mmpx4x => UpscaleFilter::Mmpx4x,
            CliUpscaleFilter::Vibrance
            | CliUpscaleFilter::Saturation
            | CliUpscaleFilter::SelectiveWarm
            | CliUpscaleFilter::SelectiveGreen
            | CliUpscaleFilter::LocalLaplacianClarity
            | CliUpscaleFilter::ContrastEnhance
            | CliUpscaleFilter::AdaptiveLogContrast
            | CliUpscaleFilter::ScaleFxSmartDeblur
            | CliUpscaleFilter::GuestrDeblur
            | CliUpscaleFilter::UnsharpMask
            | CliUpscaleFilter::HighPassSharpen => UpscaleFilter::None,
        }
    }
}

impl CliUpscaleFilter {
    pub(crate) fn into_pass(self) -> UpscalePass {
        match self {
            Self::PaletteSnapStrict => UpscalePass::PaletteSnapStrict,
            Self::PaletteSnapRampAware => UpscalePass::PaletteSnapRampAware,
            Self::PaletteSnapExpanded8 => UpscalePass::PaletteSnapExpanded {
                max_derived_colors: 8,
            },
            Self::PaletteSnapExpanded16 => UpscalePass::PaletteSnapExpanded {
                max_derived_colors: 16,
            },
            Self::PaletteSnapExpanded32 => UpscalePass::PaletteSnapExpanded {
                max_derived_colors: 32,
            },
            Self::PaletteDitherReinsertCheckerboard => UpscalePass::PaletteDitherReinsertCheckerboard,
            Self::Vibrance => UpscalePass::from(EnhancementFilter::Vibrance { factor: 0.30 }),
            Self::Saturation => UpscalePass::from(EnhancementFilter::Saturation { factor: 1.25 }),
            Self::SelectiveWarm => UpscalePass::from(EnhancementFilter::SelectiveWarm { factor: 0.30 }),
            Self::SelectiveGreen => UpscalePass::from(EnhancementFilter::SelectiveGreen { factor: 0.30 }),
            Self::LocalLaplacianClarity => {
                UpscalePass::from(EnhancementFilter::LocalLaplacianClarity { radius: 3, amount: 0.25 })
            }
            Self::ContrastEnhance => {
                UpscalePass::from(EnhancementFilter::ContrastEnhance { intensity: 0.35, threshold: 0.08, blur_spread: 2.5 })
            }
            Self::AdaptiveLogContrast => {
                UpscalePass::from(EnhancementFilter::AdaptiveLogContrast { radius: 3.0, gamma: 0.80 })
            }
            Self::ScaleFxSmartDeblur => {
                UpscalePass::from(EnhancementFilter::ScaleFxSmartDeblur {
                    deblur_offset: 0.6,
                    deblur_strength: 0.55,
                    smart_deblur: 0.4,
                })
            }
            Self::GuestrDeblur => UpscalePass::from(EnhancementFilter::GuestrDeblur),
            Self::UnsharpMask => UpscalePass::from(EnhancementFilter::UnsharpMask { radius: 1.0, amount: 0.35 }),
            Self::HighPassSharpen => UpscalePass::from(EnhancementFilter::HighPassSharpen { radius: 2.0, strength: 0.18 }),
            filter => UpscalePass::from(UpscaleFilter::from(filter)),
        }
    }
}

fn convert_upscale_passes(passes: Vec<CliUpscaleFilter>) -> Vec<UpscalePass> {
    passes.into_iter().map(CliUpscaleFilter::into_pass).collect()
}

fn load_optional_upscale_profile(path: Option<PathBuf>) -> eyre::Result<Option<Arc<udd_conv::upscale_profile::UpscaleProfile>>> {
    path.map(|path| load_upscale_profile(&path).map(Arc::new))
        .transpose()
}

#[derive(Subcommand)]
enum Commands {
    /// Packs classic art.mul/artidx.mul into tex_art_cc.uddp atlas pages.
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    PackArt {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = "tex_art_cc.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(long, help = "Write uncompressed RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl or default JXL output.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass after the selected output encoding. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, help = "BC7 RDO lookback preset in previous BC7 blocks.")]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long, default_value_t = 256)]
        upscale_64_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale_64_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = 256)]
        upscale_128_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale_128_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = 256)]
        upscale_256_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale_256_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = false, help = "Prefer art.mul/artidx.mul over artLegacyMUL.uop.")]
        mul: bool,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale: CliUpscaleFilter,
        #[arg(long = "upscale-pass", value_enum, help = "Add an upscale pass before atlas encoding. Repeat to chain filters.")]
        upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs Classic texmaps.mul into tex_land_cc.uddp atlas pages.
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    PackTexmaps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = "tex_land_cc.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(long, help = "Write uncompressed RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl or default JXL output.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass after the selected output encoding. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, help = "BC7 RDO lookback preset in previous BC7 blocks.")]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale: CliUpscaleFilter,
        #[arg(long = "upscale-64-pass", value_enum, help = "Add a 64x64 texmap upscale pass. Repeat to chain filters.")]
        upscale_64_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-128-pass", value_enum, help = "Add a 128x128 texmap upscale pass. Repeat to chain filters.")]
        upscale_128_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs classic anim*.mul/anim*.idx mobile animations into mobile_anim_cc.uddp atlas pages.
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    PackMobileAnims {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "mobile_anim_cc.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(long = "crop-transparent-bounds", default_value_t = false, help = "Trim transparent mobile animation frame borders before atlas packing and adjust frame centers.")]
        crop_transparent_bounds: bool,
        #[arg(long, help = "Write uncompressed RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass after the selected output encoding. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO. This is the default.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, help = "BC7 RDO lookback preset in previous BC7 blocks.")]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long = "upscale-pass", value_enum, help = "Add an upscale pass before atlas encoding. Repeat to chain filters.")]
        upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs EC AnimationFrame.uop mobile animations into mobile_anim_ec.uddp atlas pages.
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    PackEcMobileAnims {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "mobile_anim_ec.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = EC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(long = "crop-transparent-bounds", default_value_t = false, help = "Trim transparent EC mobile animation frame borders before atlas packing and adjust frame centers.")]
        crop_transparent_bounds: bool,
        #[arg(long, help = "Write uncompressed RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass after the selected output encoding. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO. This is the default.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, help = "BC7 RDO lookback preset in previous BC7 blocks.")]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long, help = "Directory containing EcMobileAnimations.kdl, or the KDL file itself.")]
        tables: Option<PathBuf>,
        #[arg(long = "ec-mobile-animations-kdl", help = "Exact EcMobileAnimations.kdl path.")]
        ec_mobile_animations_kdl: Option<PathBuf>,
        #[arg(long = "upscale-pass", value_enum, help = "Add an upscale pass before atlas encoding. Repeat to chain filters.")]
        upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs EC art and land in one shared source pass into tex_art_ec.uddp and tex_land_ec.uddp.
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    #[command(group(ArgGroup::new("art_output_format").args(["art_raw", "art_jxl", "art_bc7", "art_bc7_rdo"])))]
    #[command(group(ArgGroup::new("land_output_format").args(["land_raw", "land_jxl", "land_bc7", "land_bc7_rdo"])))]
    PackEcTextures {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = "tex_art_ec.uddp")]
        art_output: PathBuf,
        #[arg(long, default_value = "tex_land_ec.uddp")]
        land_output: PathBuf,
        #[arg(long, help = "Also build tilemeta.uddp from the same loaded EC tileart/TerrainDefinition sources. Requires --ccdir.")]
        tilemeta_output: Option<PathBuf>,
        #[arg(long = "tilemeta-ec-art-cropped", default_value_t = false)]
        tilemeta_ec_art_cropped: bool,
        #[arg(long = "tilemeta-use-ec-radarcol", default_value_t = false)]
        tilemeta_use_ec_radarcol: bool,
        #[arg(long, help = "Embed this terrain routing KDL in tex_land_ec.uddp instead of auto-discovered TerrainTranscode.kdl.")]
        land_transcode_kdl: Option<PathBuf>,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_WIDTH)]
        art_atlas_width: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT)]
        art_atlas_height: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_GUTTER)]
        art_gutter: u16,
        #[arg(long = "art-crop-transparent-bounds", default_value_t = false, help = "Trim transparent EC art borders inside tileart sampling windows before atlas packing.")]
        art_crop_transparent_bounds: bool,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH)]
        land_atlas_width: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT)]
        land_atlas_height: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_GUTTER)]
        land_gutter: u16,
        #[arg(long, help = "Write uncompressed EC RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write EC RGBA8888 atlas pages with lossless JPEG XL payload compression.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for shared EC output, 1 fastest through 10 strongest. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass to EC art and land after the selected output encoding. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write EC BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write EC BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, help = "Write EC art atlas pages as uncompressed RGBA8888, overriding the shared format flags.")]
        art_raw: bool,
        #[arg(long, help = "Write EC art atlas pages as RGBA8888 with lossless JPEG XL payload compression, overriding the shared format flags.")]
        art_jxl: bool,
        #[arg(long = "art-jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for EC art output. Default 6.")]
        art_jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass to EC art after the selected art output encoding. Optionally pass --art-zstd=LEVEL.")]
        art_zstd: Option<i32>,
        #[arg(long, help = "Write EC art atlas pages as BC7 without BC7 RDO, overriding the shared format flags.")]
        art_bc7: bool,
        #[arg(long = "art-bc7-rdo", help = "Write EC art atlas pages as BC7 with BC7 RDO, overriding the shared format flags.")]
        art_bc7_rdo: bool,
        #[arg(long, help = "Write EC land atlas pages as uncompressed RGBA8888, overriding the shared format flags.")]
        land_raw: bool,
        #[arg(long, help = "Write EC land atlas pages as RGBA8888 with lossless JPEG XL payload compression, overriding the shared format flags.")]
        land_jxl: bool,
        #[arg(long = "land-jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for EC land output. Default 6.")]
        land_jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Apply a file-level Zstd compression pass to EC land after the selected land output encoding. Optionally pass --land-zstd=LEVEL.")]
        land_zstd: Option<i32>,
        #[arg(long, help = "Write EC land atlas pages as BC7 without BC7 RDO, overriding the shared format flags.")]
        land_bc7: bool,
        #[arg(long = "land-bc7-rdo", help = "Write EC land atlas pages as BC7 with BC7 RDO, overriding the shared format flags.")]
        land_bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda for EC texture pages. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS, help = "BC7 RDO lookback preset in previous BC7 blocks.")]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long, default_value_t = 256)]
        upscale_64_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::FsrEasu2x)]
        upscale_64_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = 256)]
        upscale_128_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::FsrEasu2x)]
        upscale_128_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = 256)]
        upscale_256_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale_256_algo: CliUpscaleFilter,
        #[arg(long, default_value_t = 512)]
        upscale_512_size: u32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale_512_algo: CliUpscaleFilter,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None, help = "Legacy global upscale filter for art.")]
        upscale: CliUpscaleFilter,
        #[arg(long = "art-upscale-pass", value_enum, help = "Add an EC art upscale pass. Repeat to chain filters.")]
        art_upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-64-pass", value_enum, help = "Add a 64x64 EC land upscale pass. Repeat to chain filters.")]
        upscale_64_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-128-pass", value_enum, help = "Add a 128x128 EC land upscale pass. Repeat to chain filters.")]
        upscale_128_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-256-pass", value_enum, help = "Add a 256x256 EC land upscale pass. Repeat to chain filters.")]
        upscale_256_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-512-pass", value_enum, help = "Add a 512x512 EC land upscale pass. Repeat to chain filters.")]
        upscale_512_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Audits direct EC material texture references from tileart.uop and TerrainDefinition.uop.
    AuditEcMaterialRefs {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_material_owner_refs.csv")]
        output: PathBuf,
    },
    /// Inventories TerrainTexture.uop and EffectTexture.uop support resources.
    InventoryEcSupportTextures {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "terraintexture_inventory.csv")]
        terrain_output: PathBuf,
        #[arg(long, default_value = "effecttexture_inventory.csv")]
        effect_output: PathBuf,
    },
    /// Writes a developer baseline report for current EC material heuristics and metadata axes.
    ReportEcMaterialBaseline {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_material_baseline.md")]
        output: PathBuf,
    },
    /// Writes a JSON audit of TerrainDefinition primary texture selection and layer reasoning.
    AuditEcTerrainPrimarySelection {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(
            long = "terrain-overrides",
            default_value = "dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl"
        )]
        terrain_overrides: PathBuf,
        #[arg(long = "tex-land-ec")]
        tex_land_ec: Option<PathBuf>,
        #[arg(long, default_value = "ec_terrain_primary_selection.json")]
        output: PathBuf,
    },
    /// Writes a short JSON queue of terrain material decisions needed before runtime use.
    ReportEcTerrainPracticalReview {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(
            long = "terrain-overrides",
            default_value = "dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl"
        )]
        terrain_overrides: PathBuf,
        #[arg(long = "tex-land-ec")]
        tex_land_ec: Option<PathBuf>,
        #[arg(long, default_value = "ec_terrain_practical_review.json")]
        output: PathBuf,
    },
    /// Compares TerrainDefinition.kdl against TerrainDefinition.uop and reports manual-only fields.
    AuditEcTerrainDefinitionKdl {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(
            long = "terrain-definition-kdl",
            default_value = "dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl"
        )]
        terrain_definition_kdl: PathBuf,
        #[arg(
            long = "terrain-overrides",
            default_value = "dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl"
        )]
        terrain_overrides: PathBuf,
        #[arg(long, default_value = "ec_terrain_definition_kdl_audit.json")]
        output: PathBuf,
    },
    /// Writes a KDL review skeleton for TerrainDefinition manual integration candidates.
    ReportEcTerrainOverrideCandidates {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(
            long = "terrain-definition-kdl",
            default_value = "dynamapper/assets/cc_ec_convtables/TerrainDefinition.kdl"
        )]
        terrain_definition_kdl: PathBuf,
        #[arg(long, default_value = "EcTerrainOverrides.review.kdl")]
        output: PathBuf,
    },
    /// Validates EcTerrainOverrides.kdl against source-derived TerrainDefinition evidence.
    AuditEcTerrainOverrides {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(
            long = "terrain-overrides",
            default_value = "dynamapper/assets/cc_ec_convtables/EcTerrainOverrides.kdl"
        )]
        terrain_overrides: PathBuf,
        #[arg(long, default_value = "ec_terrain_overrides_audit.json")]
        output: PathBuf,
    },
    /// Validates a terrain routing KDL against a tex_land_ec.uddp package.
    AuditLandRoutingKdl {
        #[arg(long)]
        routing: PathBuf,
        #[arg(long = "tex-land-ec")]
        tex_land_ec: PathBuf,
        #[arg(long, default_value = "land_routing_audit.json")]
        output: PathBuf,
    },
    /// Writes a JSON audit of surface-like art redirection through tilemeta and tex_land_ec provenance.
    AuditEcSurfaceRedirection {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        tilemeta: PathBuf,
        #[arg(long = "ec-art")]
        tex_art_ec: Option<PathBuf>,
        #[arg(long = "ec-land")]
        tex_land_ec: PathBuf,
        #[arg(
            long = "surface-overrides",
            default_value = "dynamapper/assets/cc_ec_convtables/EcSurfaceOverrides.kdl"
        )]
        surface_overrides: PathBuf,
        #[arg(long, default_value = "ec_surface_redirection.json")]
        output: PathBuf,
    },
    /// Packs CC tiledata and EC tileart into tilemeta.uddp.
    #[command(name = "pack-tilemeta")]
    PackTilemeta {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = "tilemeta.uddp")]
        output: PathBuf,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Accepted for parity with package helper scripts; tilemeta currently keeps its internal package compression policy.")]
        zstd: Option<i32>,
        #[arg(long, default_value_t = false)]
        ec_art_cropped: bool,
        #[arg(long, default_value_t = false)]
        use_ec_radarcol: bool,
    },
    /// Generates a radar map (facet0X.dds) from Classic map and statics.
    PackRadar {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long = "uddpdir")]
        uddp_dir: Option<PathBuf>,
        #[arg(long = "outdir")]
        outdir: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        /// Output format: rgba8, bc7, bc7ktx2
        #[arg(long, default_value = "bc7")]
        format: String,
        #[arg(long, default_value_t = false)]
        uop: bool,
        /// Zstd compression level for KTX2 (1-22)
        #[arg(long, default_value_t = 3)]
        zstd_level: i32,
    },
    /// Packs Classic mapX.mul into mapX.uddp blocks.
    PackMap {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        uop: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Accepted for parity with package helper scripts; map packing currently keeps its internal package compression policy.")]
        zstd: Option<i32>,
    },
    /// Packs Classic staticsX.mul into staticsX.uddp blocks.
    PackStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Accepted for parity with package helper scripts; static packing currently keeps its internal package compression policy.")]
        zstd: Option<i32>,
    },
    /// Packs Classic mapX.uddp and staticsX.uddp together, reusing the map dimensions for statics.
    PackMapStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        map_output: Option<PathBuf>,
        #[arg(long)]
        statics_output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        uop: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Accepted for parity with package helper scripts; map/statics packing currently keeps its internal package compression policy.")]
        zstd: Option<i32>,
    },
    /// Packs CC and EC lighting textures into world_lights.uddp.
    PackLights {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = "world_lights.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false, help = "Do not use compression.")]
        no_compression: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Use Zstd package compression. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
    },
    /// Packs Classic hues.mul or EC hues.uop into hues.uddp.
    PackHues {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "hues.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false, help = "Do not use compression.")]
        no_compression: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Use Zstd package compression. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
    },
    /// Packs Classic gumpidx.mul/gumpart.mul or gumpartLegacyMUL.uop into gumps_cc.uddp.
    PackGumps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = GUMPS_CC_DEFAULT_OUTPUT)]
        output: PathBuf,
        #[arg(long, help = "Store decoded RGBA8888 gump pixels. This is the only gump payload format currently written.")]
        raw: bool,
        #[arg(long, help = "Write paperdoll equipment gump atlas pages with lossless JPEG XL payload compression. Single gump payloads remain raw RGBA8888.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for gump atlas pages. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Use Zstd package compression. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, default_value_t = false)]
        uop: bool,
        #[arg(long = "paperdoll-upscale-pass", value_enum, help = "Add a paperdoll equipment gump upscale pass. Repeat to chain filters.")]
        paperdoll_upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "single-upscale-pass", value_enum, help = "Add a non-paperdoll gump upscale pass. Repeat to chain filters.")]
        single_upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs EC interface.uop gumpart into gumps_ec.uddp.
    PackEcGumps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = GUMPS_EC_DEFAULT_OUTPUT)]
        output: PathBuf,
        #[arg(long, default_value_t = EC_GUMP_DEFAULT_MAX_ID, help = "Highest numeric gump id to probe when reading interface.uop.")]
        max_id: u32,
        #[arg(long, help = "Store decoded RGBA8888 gump pixels. This is the only gump payload format currently written.")]
        raw: bool,
        #[arg(long, help = "Write paperdoll equipment gump atlas pages with lossless JPEG XL payload compression. Single gump payloads remain raw RGBA8888.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for gump atlas pages. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Use Zstd package compression. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long = "paperdoll-upscale-pass", value_enum, help = "Add a paperdoll equipment gump upscale pass. Repeat to chain filters.")]
        paperdoll_upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "single-upscale-pass", value_enum, help = "Add a non-paperdoll gump upscale pass. Repeat to chain filters.")]
        single_upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
}

pub fn run() -> eyre::Result<()> {
    color_eyre::install()?;
    let _ = udd_logging::install_tracing_indicatif_logger();

    match Cli::parse().command {
        Commands::PackArt {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            atlas_width,
            atlas_height,
            gutter,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            upscale_64_size: _, // Land upscaling not currently applied to CC Art
            upscale_64_algo: _,
            upscale_128_size: _,
            upscale_128_algo: _,
            upscale_256_size: _,
            upscale_256_algo: _,
            mul,
            upscale,
            upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let output_format = resolve_texture_output_format(
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
            )?;
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                &TexArtCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression: output_format.compression,
                    upscale: upscale.into(),
                    upscale_passes: convert_upscale_passes(upscale_passes),
                    upscale_profile,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    source_preference: if mul {
                        SourceFormatPreference::Mul
                    } else {
                        SourceFormatPreference::Uop
                    },
                },
                &classic_patches.into(),
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                output_format.label,
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackTexmaps {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            atlas_width,
            atlas_height,
            gutter,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            upscale,
            upscale_64_passes,
            upscale_128_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let output_format = resolve_texture_output_format(
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
            )?;
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_texmaps_mul_to_tex_land_cc_uddp_with_patches(
                &paths[0], // Use the first source dir (usually ccdir)
                &out_file,
                &TexLandCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression: output_format.compression,
                    upscale_64: udd_conv::upscale::UpscaleConfig {
                        target_size: 256,
                        filter: upscale.into(),
                    },
                    upscale_128: udd_conv::upscale::UpscaleConfig {
                        target_size: 256,
                        filter: upscale.into(),
                    },
                    upscale_64_passes: convert_upscale_passes(upscale_64_passes),
                    upscale_128_passes: convert_upscale_passes(upscale_128_passes),
                    upscale_profile,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                },
                &classic_patches.into(),
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                output_format.label,
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackMobileAnims {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            crop_transparent_bounds,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let output_format =
                resolve_mobile_anim_output_format(
                    raw,
                    jxl,
                    jxl_level,
                    zstd,
                    bc7,
                    bc7_rdo,
                    bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks,
                )?;
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_anim_mul_to_mobile_anim_cc_uddp_from_sources(
                &paths,
                &out_file,
                &MobileAnimCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    crop_transparent_bounds,
                    compression: output_format.compression,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    upscale_passes: convert_upscale_passes(upscale_passes),
                    upscale_profile,
                },
            )?;
            println!(
                "Wrote {} pages ({}) for {} packed frames out of {} total frames across {} animations to '{}'.",
                summary.page_count,
                output_format.label,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                out_file.display()
            );
            println!(
                "Mobile animation atlas usage: {} used pixels, {} filled ({}) and {} empty ({}); estimated uncompressed page payload {} bytes.",
                summary.used_page_pixel_count,
                summary.filled_pixel_count,
                percent_string(summary.filled_pixel_count, summary.used_page_pixel_count),
                summary.empty_pixel_count,
                percent_string(summary.empty_pixel_count, summary.used_page_pixel_count),
                estimated_bc7_bytes(summary.used_page_pixel_count, output_format.pixel_format),
            );
        }
        Commands::PackEcMobileAnims {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            crop_transparent_bounds,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            tables,
            ec_mobile_animations_kdl,
            upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let output_format =
                resolve_mobile_anim_output_format(
                    raw,
                    jxl,
                    jxl_level,
                    zstd,
                    bc7,
                    bc7_rdo,
                    bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks,
                )?;
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources(
                &paths,
                &out_file,
                &MobileAnimEcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    crop_transparent_bounds,
                    compression: output_format.compression,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    upscale_passes: convert_upscale_passes(upscale_passes),
                    upscale_profile,
                    metadata_path: ec_mobile_animations_kdl,
                    tables_dir: tables,
                    allow_missing_metadata: false,
                },
            )?;
            println!(
                "Wrote {} pages ({}) from {} EC animation UOPs for {} packed frames out of {} logical frames across {} EC animations from {} bodies with {} metadata items and {} source hints to '{}'.",
                summary.page_count,
                output_format.label,
                summary.source_uop_count,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                summary.body_count,
                summary.item_metadata_count,
                summary.source_hint_count,
                out_file.display()
            );
            println!(
                "EC mobile animation atlas usage: {} used pixels, {} filled ({}) and {} empty ({}); estimated uncompressed page payload {} bytes.",
                summary.used_page_pixel_count,
                summary.filled_pixel_count,
                percent_string(summary.filled_pixel_count, summary.used_page_pixel_count),
                summary.empty_pixel_count,
                percent_string(summary.empty_pixel_count, summary.used_page_pixel_count),
                estimated_bc7_bytes(summary.used_page_pixel_count, output_format.pixel_format),
            );
        }
        Commands::PackEcTextures {
            source_dirs: source_dir_args,
            classic_patches,
            art_output,
            land_output,
            tilemeta_output,
            tilemeta_ec_art_cropped,
            tilemeta_use_ec_radarcol,
            land_transcode_kdl,
            art_atlas_width,
            art_atlas_height,
            art_gutter,
            art_crop_transparent_bounds,
            land_atlas_width,
            land_atlas_height,
            land_gutter,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            art_raw,
            art_jxl,
            art_jxl_level,
            art_zstd,
            art_bc7,
            art_bc7_rdo,
            land_raw,
            land_jxl,
            land_jxl_level,
            land_zstd,
            land_bc7,
            land_bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            upscale_64_size,
            upscale_64_algo,
            upscale_128_size,
            upscale_128_algo,
            upscale_256_size,
            upscale_256_algo,
            upscale_512_size,
            upscale_512_algo,
            upscale,
            art_upscale_passes,
            upscale_64_passes,
            upscale_128_passes,
            upscale_256_passes,
            upscale_512_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let ec_paths = collect_ec_source_dirs(&source_dir_args)?;
            let art_out_file = resolve_output_path(&ec_paths, &art_output);
            let land_out_file = resolve_output_path(&ec_paths, &land_output);
            let tilemeta_out_file = tilemeta_output
                .as_ref()
                .map(|output| resolve_output_path(&paths, output));
            if tilemeta_out_file.is_some() && source_dir_args.ccdir.is_none() {
                eyre::bail!("--tilemeta-output requires --ccdir for tiledata.mul");
            }
            let shared_sources = load_tex_art_ec_sources(&ec_paths)?;
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let upscale_filter = UpscaleFilter::from(upscale);
            let shared_output_format = TextureOutputFormatArgs {
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
            };
            let art_output_format = resolve_texture_output_format_args(
                TextureOutputFormatArgs {
                    raw: art_raw || shared_output_format.raw,
                    jxl: art_jxl || shared_output_format.jxl,
                    jxl_level: art_jxl_level.or(shared_output_format.jxl_level),
                    zstd: art_zstd.or(shared_output_format.zstd),
                    bc7: art_bc7 || shared_output_format.bc7,
                    bc7_rdo: art_bc7_rdo || shared_output_format.bc7_rdo,
                },
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
            )?;
            let land_output_format = resolve_texture_output_format_args(
                TextureOutputFormatArgs {
                    raw: land_raw || shared_output_format.raw,
                    jxl: land_jxl || shared_output_format.jxl,
                    jxl_level: land_jxl_level.or(shared_output_format.jxl_level),
                    zstd: land_zstd.or(shared_output_format.zstd),
                    bc7: land_bc7 || shared_output_format.bc7,
                    bc7_rdo: land_bc7_rdo || shared_output_format.bc7_rdo,
                },
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
            )?;
            let art_summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(
                &shared_sources,
                &art_out_file,
                &TexArtEcAtlasOptions {
                    atlas_width: art_atlas_width,
                    atlas_height: art_atlas_height,
                    gutter: art_gutter,
                    crop_transparent_bounds: art_crop_transparent_bounds,
                    compression: art_output_format.compression,
                    upscale: upscale_filter,
                    upscale_passes: convert_upscale_passes(art_upscale_passes),
                    upscale_profile: upscale_profile.clone(),
                    pixel_format: art_output_format.pixel_format,
                    bc7_rdo_lambda: art_output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: art_output_format.bc7_rdo_lookback_blocks,
                },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated art slots out of {} total slots to '{}'.",
                art_summary.page_count,
                art_output_format.label,
                art_summary.populated_slot_count,
                art_summary.slot_count,
                art_out_file.display()
            );

            let land_summary = convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_loaded_sources(
                &paths,
                &shared_sources.terrain_definition_path,
                shared_sources.texture_uop_path.as_deref(),
                shared_sources.legacy_texture_uop_path.as_deref(),
                shared_sources.terrain_definition(),
                shared_sources.world_textures.as_ref(),
                shared_sources.legacy_textures.as_ref(),
                &land_out_file,
                &TexLandEcAtlasOptions {
                    atlas_width: land_atlas_width,
                    atlas_height: land_atlas_height,
                    gutter: land_gutter,
                    compression: land_output_format.compression,
                    upscale_64: udd_conv::upscale::UpscaleConfig {
                        target_size: upscale_64_size,
                        filter: upscale_64_algo.into(),
                    },
                    upscale_128: udd_conv::upscale::UpscaleConfig {
                        target_size: upscale_128_size,
                        filter: upscale_128_algo.into(),
                    },
                    upscale_256: udd_conv::upscale::UpscaleConfig {
                        target_size: upscale_256_size,
                        filter: upscale_256_algo.into(),
                    },
                    upscale_512: udd_conv::upscale::UpscaleConfig {
                        target_size: upscale_512_size,
                        filter: upscale_512_algo.into(),
                    },
                    upscale_64_passes: convert_upscale_passes(upscale_64_passes),
                    upscale_128_passes: convert_upscale_passes(upscale_128_passes),
                    upscale_256_passes: convert_upscale_passes(upscale_256_passes),
                    upscale_512_passes: convert_upscale_passes(upscale_512_passes),
                    upscale_profile,
                    pixel_format: land_output_format.pixel_format,
                    bc7_rdo_lambda: land_output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: land_output_format.bc7_rdo_lookback_blocks,
                    transcode_kdl_path: land_transcode_kdl,
                },
            )?;
            println!(
                "TerrainDefinition raw texture packing: {} entries, {} alias refs, {} unique alias slots, {} source textures, {} selected textures, {} unique packed textures.",
                land_summary.terrain_entry_count,
                land_summary.terrain_alias_ref_count,
                land_summary.unique_alias_slot_count,
                land_summary.unique_source_texture_count,
                land_summary.unique_texture_selection_count,
                land_summary.unique_packed_texture_count,
            );
            if let Some(path) = land_summary.terrain_override_source.as_ref() {
                println!(
                    "Embedded {} reviewed terrain override entries from '{}'.",
                    land_summary.terrain_override_entry_count,
                    path.display()
                );
            }
            println!(
                "Wrote {} pages ({}) for {} populated land slots out of {} total slots to '{}'.",
                land_summary.page_count,
                land_output_format.label,
                land_summary.populated_slot_count,
                land_summary.slot_count,
                land_out_file.display()
            );

            if let Some(tilemeta_out_file) = tilemeta_out_file.as_ref() {
                let ccdir = source_dir_args
                    .ccdir
                    .as_ref()
                    .expect("--ccdir validated for tilemeta output");
                let ecdir = source_dir_args
                    .ecdir
                    .as_ref()
                    .expect("--ecdir validated for EC texture output");
                build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
                    ccdir,
                    ecdir,
                    &shared_sources,
                    tilemeta_out_file,
                    &TileMetaBuildOptions {
                        adjust_tex_art_ec_sampling: tilemeta_ec_art_cropped,
                        use_ec_radarcol: tilemeta_use_ec_radarcol,
                        classic_patches: classic_patches.into(),
                        package_compression: CompressionFlag::ZstdNoDict,
                    },
                )?;
                println!(
                    "Wrote tilemeta.uddp from shared EC sources to '{}'.",
                    tilemeta_out_file.display()
                );
            }
        }
        Commands::AuditEcMaterialRefs {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            audit_ec_material_refs(&paths, &out_file)?;
        }
        Commands::InventoryEcSupportTextures {
            source_dirs: source_dir_args,
            terrain_output,
            effect_output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let terrain_out_file = resolve_output_path(&paths, &terrain_output);
            let effect_out_file = resolve_output_path(&paths, &effect_output);
            inventory_ec_support_textures(&paths, &terrain_out_file, &effect_out_file)?;
        }
        Commands::ReportEcMaterialBaseline {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            write_ec_material_baseline_report(&out_file)?;
        }
        Commands::AuditEcTerrainPrimarySelection {
            source_dirs: source_dir_args,
            terrain_overrides,
            tex_land_ec,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let terrain_overrides = terrain_overrides.exists().then_some(terrain_overrides);
            audit_ec_terrain_primary_selection(
                &paths,
                terrain_overrides.as_deref(),
                tex_land_ec.as_deref(),
                &out_file,
            )?;
        }
        Commands::ReportEcTerrainPracticalReview {
            source_dirs: source_dir_args,
            terrain_overrides,
            tex_land_ec,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let terrain_overrides = terrain_overrides.exists().then_some(terrain_overrides);
            write_ec_terrain_practical_review(
                &paths,
                terrain_overrides.as_deref(),
                tex_land_ec.as_deref(),
                &out_file,
            )?;
        }
        Commands::AuditEcTerrainDefinitionKdl {
            source_dirs: source_dir_args,
            terrain_definition_kdl,
            terrain_overrides,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let terrain_overrides = terrain_overrides.exists().then_some(terrain_overrides);
            audit_ec_terrain_definition_kdl(
                &paths,
                &terrain_definition_kdl,
                terrain_overrides.as_deref(),
                &out_file,
            )?;
        }
        Commands::ReportEcTerrainOverrideCandidates {
            source_dirs: source_dir_args,
            terrain_definition_kdl,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            write_ec_terrain_override_candidates(&paths, &terrain_definition_kdl, &out_file)?;
        }
        Commands::AuditEcTerrainOverrides {
            source_dirs: source_dir_args,
            terrain_overrides,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            audit_ec_terrain_overrides(&paths, &terrain_overrides, &out_file)?;
        }
        Commands::AuditLandRoutingKdl {
            routing,
            tex_land_ec,
            output,
        } => {
            audit_land_routing_kdl(&routing, &tex_land_ec, &output)?;
        }
        Commands::AuditEcSurfaceRedirection {
            source_dirs: source_dir_args,
            tilemeta,
            tex_art_ec,
            tex_land_ec,
            surface_overrides,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let surface_overrides = surface_overrides.exists().then_some(surface_overrides);
            audit_ec_surface_redirection(
                &paths,
                &tilemeta,
                tex_art_ec.as_deref(),
                &tex_land_ec,
                surface_overrides.as_deref(),
                &out_file,
            )?;
        }
        Commands::PackTilemeta {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            zstd,
            ec_art_cropped,
            use_ec_radarcol,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let options = TileMetaBuildOptions {
                adjust_tex_art_ec_sampling: ec_art_cropped,
                use_ec_radarcol,
                classic_patches: classic_patches.into(),
                package_compression: zstd.map(zstd_compression).unwrap_or(CompressionFlag::ZstdNoDict),
            };
            match (source_dir_args.ccdir.as_ref(), source_dir_args.ecdir.as_ref()) {
                (Some(ccdir), Some(ecdir)) => {
                    build_tilemeta_uddp_from_split_sources(ccdir, ecdir, &out_file, &options)?;
                }
                _ => {
                    build_tilemeta_uddp_from_sources(&paths, &out_file, &options)?;
                }
            }
            println!("Wrote tilemeta.uddp to '{}'.", out_file.display());
        }
        Commands::PackMap {
            source_dirs: source_dir_args,
            classic_patches,
            map_id,
            output,
            uop,
            zstd: _,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("map{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_map_mul_to_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                map_id,
                if uop {
                    CcMapSourcePreference::Uop
                } else {
                    CcMapSourcePreference::Mul
                },
                &classic_patches.into(),
            )?;
            println!(
                "Wrote {} chunks for map {} to '{}' ({}x{} package chunks).",
                summary.chunk_count,
                summary.map_id,
                out_file.display(),
                summary.width_chunks,
                summary.height_chunks,
            );
        }
        Commands::PackStatics {
            source_dirs: source_dir_args,
            classic_patches,
            map_id,
            output,
            zstd: _,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_statics_mul_to_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                map_id,
                &classic_patches.into(),
            )?;
            println!(
                "Wrote {} chunks with {} total statics for map {} to '{}'.",
                summary.chunk_count,
                summary.total_statics,
                summary.map_id,
                out_file.display(),
            );
        }
        Commands::PackMapStatics {
            source_dirs: source_dir_args,
            classic_patches,
            map_id,
            map_output,
            statics_output,
            uop,
            zstd: _,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_map_output = PathBuf::from(format!("map{}.uddp", map_id));
            let default_statics_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let map_out_file =
                resolve_output_path(&paths, map_output.as_ref().unwrap_or(&default_map_output));
            let statics_out_file = resolve_output_path(
                &paths,
                statics_output.as_ref().unwrap_or(&default_statics_output),
            );
            let patch_options: ClassicPatchOptions = classic_patches.into();
            let map_summary = convert_map_mul_to_uddp_from_sources_with_patches(
                &paths,
                &map_out_file,
                map_id,
                if uop {
                    CcMapSourcePreference::Uop
                } else {
                    CcMapSourcePreference::Mul
                },
                &patch_options,
            )?;
            println!(
                "Wrote {} chunks for map {} to '{}' ({}x{} package chunks).",
                map_summary.chunk_count,
                map_summary.map_id,
                map_out_file.display(),
                map_summary.width_chunks,
                map_summary.height_chunks,
            );

            let statics_summary =
                convert_statics_mul_to_uddp_from_sources_with_dimensions_and_patches(
                    &paths,
                    &statics_out_file,
                    map_id,
                    map_summary.width_blocks,
                    map_summary.height_blocks,
                    &patch_options,
                )?;
            println!(
                "Wrote {} chunks with {} total statics for map {} to '{}' using map dimensions from the map pass.",
                statics_summary.chunk_count,
                statics_summary.total_statics,
                statics_summary.map_id,
                statics_out_file.display(),
            );
        }
        Commands::PackRadar {
            source_dirs: source_dir_args,
            classic_patches,
            map_id,
            uddp_dir,
            outdir,
            output,
            format,
            uop,
            zstd_level,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;

            let radar_format = match format.to_lowercase().as_str() {
                "rgba8" => udd_conv::cc_radar::RadarFormat::Rgba8,
                "bc7" => udd_conv::cc_radar::RadarFormat::Bc7,
                "bc7ktx2" => udd_conv::cc_radar::RadarFormat::Bc7Ktx2,
                _ => eyre::bail!("Invalid format: {}. Valid: rgba8, bc7, bc7ktx2", format),
            };

            let default_output =
                PathBuf::from(format!("facet0{}.{}", map_id, radar_format.extension()));

            let output_path_to_use = output.as_ref().unwrap_or(&default_output);
            let out_file = if let Some(dir) = &outdir {
                dir.join(output_path_to_use)
            } else {
                resolve_output_path(&paths, output_path_to_use)
            };

            let uddp_dir = uddp_dir.unwrap_or_else(|| PathBuf::from("."));
            let tilemeta_path = find_raw_tilemeta_package(&uddp_dir)?;
            let map_source_preference = if uop {
                CcMapSourcePreference::Uop
            } else {
                CcMapSourcePreference::Mul
            };

            if radar_format == udd_conv::cc_radar::RadarFormat::Bc7Ktx2 {
                let bc7_data = udd_conv::cc_radar::build_facet_radar_bc7_with_options(
                    &paths,
                    &tilemeta_path,
                    map_id,
                    map_source_preference,
                    &classic_patches.into(),
                )?;
                udd_image_codecs::ktx2::write_ktx2_bc7_zstd(bc7_data, &out_file, zstd_level)?;
                println!("Successfully created facet0{}.ktx2 (BC7 + Zstd)", map_id);
            } else {
                udd_conv::cc_radar::build_facet_radar_dds(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    &udd_conv::cc_radar::RadarBuildOptions {
                        format: radar_format,
                        zstd_level,
                        classic_patches: classic_patches.into(),
                        map_source_preference,
                    },
                )?;
            }
        }
        Commands::PackLights {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            no_compression,
            zstd,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if no_compression {
                CompressionFlag::None
            } else if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::ZstdNoDict
            };
            convert_client_lights_to_world_lights_uddp(
                source_dir_args.ccdir.as_deref(),
                source_dir_args.ecdir.as_deref(),
                &out_file,
                &WorldLightsOptions {
                    compression,
                    classic_patches: classic_patches.into(),
                },
            )?;
            println!("Wrote world_lights.uddp to '{}'.", out_file.display());
        }
        Commands::PackHues {
            source_dirs: source_dir_args,
            output,
            no_compression,
            zstd,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if no_compression {
                CompressionFlag::None
            } else if let Some(level) = zstd {
                zstd_compression(level)
            } else {
                CompressionFlag::ZstdNoDict
            };
            convert_hues_to_hues_uddp_from_sources(
                &paths,
                &out_file,
                &HuesOptions { compression },
            )?;
            println!("Wrote hues.uddp to '{}'.", out_file.display());
        }
        Commands::PackGumps {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            raw: _,
            jxl,
            jxl_level,
            zstd,
            uop,
            paperdoll_upscale_passes,
            single_upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_gumps_to_uddp_from_sources_with_patches_and_options(
                &paths,
                &out_file,
                &classic_patches.into(),
                &CcGumpsOptions {
                    compression: if let Some(level) = zstd {
                        zstd_compression(level)
                    } else if jxl {
                        CompressionFlag::None
                    } else {
                        CompressionFlag::ZstdNoDict
                    },
                    atlas_compression: if jxl {
                        zstd.map(|level| jxl_zstd_compression_levels(jxl_level, level))
                            .unwrap_or_else(|| jxl_compression_level(jxl_level))
                    } else {
                        zstd.map(zstd_compression).unwrap_or(CompressionFlag::ZstdNoDict)
                    },
                    paperdoll_upscale_passes: convert_upscale_passes(paperdoll_upscale_passes),
                    single_upscale_passes: convert_upscale_passes(single_upscale_passes),
                    upscale_profile,
                    source_preference: if uop {
                        SourceFormatPreference::Uop
                    } else {
                        SourceFormatPreference::Mul
                    },
                },
            )?;
            println!(
                "Wrote {} Classic gumps to '{}' ({} single, {} atlas).",
                summary.gump_count,
                out_file.display(),
                summary.single_gump_count,
                summary.atlas_gump_count
            );
        }
        Commands::PackEcGumps {
            source_dirs: source_dir_args,
            output,
            max_id,
            raw: _,
            jxl,
            jxl_level,
            zstd,
            paperdoll_upscale_passes,
            single_upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let summary = convert_ec_gumps_to_uddp_from_sources(
                &paths,
                &out_file,
                &EcGumpsOptions {
                    max_id,
                    compression: if let Some(level) = zstd {
                        zstd_compression(level)
                    } else if jxl {
                        CompressionFlag::None
                    } else {
                        CompressionFlag::ZstdNoDict
                    },
                    atlas_compression: if jxl {
                        zstd.map(|level| jxl_zstd_compression_levels(jxl_level, level))
                            .unwrap_or_else(|| jxl_compression_level(jxl_level))
                    } else {
                        zstd.map(zstd_compression).unwrap_or(CompressionFlag::ZstdNoDict)
                    },
                    paperdoll_upscale_passes: convert_upscale_passes(paperdoll_upscale_passes),
                    single_upscale_passes: convert_upscale_passes(single_upscale_passes),
                    upscale_profile,
                },
            )?;
            println!(
                "Wrote {} EC gumps to '{}' ({} single, {} atlas, {} skipped).",
                summary.gump_count,
                out_file.display(),
                summary.single_gump_count,
                summary.atlas_gump_count,
                summary.skipped_count
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(test_name: &str) -> PathBuf {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("udd_pack_cli_{test_name}_{timestamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn collect_source_dirs_rejects_missing_roots() {
        let error = collect_source_dirs(&SourceDirArgs {
            ccdir: None,
            ecdir: None,
        })
        .expect_err("missing source dirs must fail");

        assert!(error
            .to_string()
            .contains("at least one source root must be provided"));
    }

    #[test]
    fn collect_source_dirs_deduplicates_same_root() {
        let shared = temp_dir("shared_root");
        let dirs = collect_source_dirs(&SourceDirArgs {
            ccdir: Some(shared.clone()),
            ecdir: Some(shared.clone()),
        })
        .expect("collect source dirs");

        assert_eq!(dirs, vec![shared.clone()]);

        let _ = fs::remove_dir_all(shared);
    }

    #[test]
    fn collect_ec_source_dirs_uses_only_ec_root() {
        let ccdir = temp_dir("cc_root");
        let ecdir = temp_dir("ec_root");
        let dirs = collect_ec_source_dirs(&SourceDirArgs {
            ccdir: Some(ccdir.clone()),
            ecdir: Some(ecdir.clone()),
        })
        .expect("collect ec dirs");

        assert_eq!(dirs, vec![ecdir.clone()]);

        let _ = fs::remove_dir_all(ccdir);
        let _ = fs::remove_dir_all(ecdir);
    }

    #[test]
    fn collect_ec_source_dirs_rejects_cc_only_root() {
        let error = collect_ec_source_dirs(&SourceDirArgs {
            ccdir: Some(PathBuf::from("/cc")),
            ecdir: None,
        })
        .expect_err("cc-only roots must not satisfy EC source loading");

        assert!(error.to_string().contains("--ecdir is required"));
    }

    #[test]
    fn cli_parses_pack_tilemeta_alias_with_both_roots() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-tilemeta",
            "--ccdir",
            "/cc",
            "--ecdir",
            "/ec",
            "--output",
            "tilemeta.uddp",
        ])
        .expect("parse cli args");

        match cli.command {
            Commands::PackTilemeta {
                source_dirs,
                classic_patches,
                output,
                ec_art_cropped,
                use_ec_radarcol,
                ..
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert!(!classic_patches.include_verdata);
                assert!(!classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
                assert_eq!(output, PathBuf::from("tilemeta.uddp"));
                assert!(!ec_art_cropped);
                assert!(!use_ec_radarcol);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_textures() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-textures",
            "--ecdir",
            "/ec",
            "--art-output",
            "tex_art_ec.uddp",
            "--land-output",
            "tex_land_ec.uddp",
            "--land-transcode-kdl",
            "dynamapper/assets/cc_ec_convtables/KrTerrainRouting.generated.kdl",
            "--art-crop-transparent-bounds",
            "--bc7",
        ])
        .expect("parse unified ec texture args");

        match cli.command {
            Commands::PackEcTextures {
                source_dirs,
                art_output,
                land_output,
                land_transcode_kdl,
                art_crop_transparent_bounds,
                bc7,
                bc7_rdo,
                raw,
                jxl,
                zstd,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(art_output, PathBuf::from("tex_art_ec.uddp"));
                assert_eq!(land_output, PathBuf::from("tex_land_ec.uddp"));
                assert_eq!(
                    land_transcode_kdl,
                    Some(PathBuf::from(
                        "dynamapper/assets/cc_ec_convtables/KrTerrainRouting.generated.kdl"
                    ))
                );
                assert!(art_crop_transparent_bounds);
                assert!(bc7);
                assert!(!bc7_rdo);
                assert!(!raw);
                assert!(!jxl);
                assert!(zstd.is_none());
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_textures_with_separate_output_formats() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-textures",
            "--ecdir",
            "/ec",
            "--art-jxl",
            "--land-bc7-rdo",
        ])
        .expect("parse separate ec texture output args");

        match cli.command {
            Commands::PackEcTextures {
                art_jxl,
                land_bc7_rdo,
                raw,
                jxl,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(art_jxl);
                assert!(land_bc7_rdo);
                assert!(!raw);
                assert!(!jxl);
                assert!(!bc7);
                assert!(!bc7_rdo);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_textures_with_shared_tilemeta_output() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-textures",
            "--ccdir",
            "/cc",
            "--ecdir",
            "/ec",
            "--tilemeta-output",
            "tilemeta.uddp",
            "--tilemeta-use-ec-radarcol",
            "--include-verdata",
        ])
        .expect("parse EC texture group with tilemeta output");

        match cli.command {
            Commands::PackEcTextures {
                source_dirs,
                classic_patches,
                tilemeta_output,
                tilemeta_use_ec_radarcol,
                ..
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(tilemeta_output, Some(PathBuf::from("tilemeta.uddp")));
                assert!(tilemeta_use_ec_radarcol);
                assert!(classic_patches.include_verdata);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_map_statics_group() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-map-statics",
            "--ccdir",
            "/cc",
            "--map-id",
            "0",
            "--map-output",
            "map0.uddp",
            "--statics-output",
            "statics0.uddp",
            "--include-static-difs",
        ])
        .expect("parse grouped map/statics command");

        match cli.command {
            Commands::PackMapStatics {
                source_dirs,
                classic_patches,
                map_id,
                map_output,
                statics_output,
                uop,
                ..
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert!(classic_patches.include_static_difs);
                assert_eq!(map_id, 0);
                assert_eq!(map_output, Some(PathBuf::from("map0.uddp")));
                assert_eq!(statics_output, Some(PathBuf::from("statics0.uddp")));
                assert!(!uop);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_defaults_texture_output_format_to_jxl() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
        ])
        .expect("texture pack commands default to JXL output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(!raw);
                assert!(!jxl);
                assert!(zstd.is_none());
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::JpegXlLevel(DEFAULT_JXL_LEVEL)
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_defaults_mobile_animation_output_format_to_bc7() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--output",
            "mobile_anim_cc.uddp",
            "--crop-transparent-bounds",
        ])
        .expect("mobile animation pack commands default to BC7 output");

        match cli.command {
            Commands::PackMobileAnims {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                crop_transparent_bounds,
                ..
            } => {
                assert!(!raw);
                assert!(!jxl);
                assert!(zstd.is_none());
                assert!(!bc7);
                assert!(!bc7_rdo);
                assert!(crop_transparent_bounds);
                let output_format = resolve_mobile_anim_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Bc7);
                assert_eq!(output_format.compression, CompressionFlag::None);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_repeated_upscale_passes() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--upscale-pass",
            "hq3x-true",
            "--upscale-pass",
            "fsr-easu-rcas2x",
        ])
        .expect("parse repeated upscale passes");

        match cli.command {
            Commands::PackMobileAnims { upscale_passes, .. } => {
                assert_eq!(
                    upscale_passes,
                    vec![CliUpscaleFilter::Hq3xTrue, CliUpscaleFilter::FsrEasuRcas2x]
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_palette_snap_upscale_passes() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--upscale-pass",
            "bilinear2x",
            "--upscale-pass",
            "palette-snap-strict",
            "--upscale-pass",
            "palette-snap-expanded-16",
            "--upscale-pass",
            "palette-dither-reinsert-checkerboard",
        ])
        .expect("parse palette snap upscale passes");

        match cli.command {
            Commands::PackMobileAnims { upscale_passes, .. } => {
                assert_eq!(
                    upscale_passes,
                    vec![
                        CliUpscaleFilter::Bilinear2x,
                        CliUpscaleFilter::PaletteSnapStrict,
                        CliUpscaleFilter::PaletteSnapExpanded16,
                        CliUpscaleFilter::PaletteDitherReinsertCheckerboard,
                    ]
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_converts_palette_snap_upscale_passes() {
        let passes = convert_upscale_passes(vec![
            CliUpscaleFilter::Bilinear2x,
            CliUpscaleFilter::PaletteSnapRampAware,
            CliUpscaleFilter::PaletteSnapExpanded16,
            CliUpscaleFilter::PaletteDitherReinsertCheckerboard,
        ]);

        assert_eq!(passes.len(), 4);
        assert_eq!(passes[0].filter(), Some(UpscaleFilter::Bilinear2x));
        assert!(matches!(passes[1], UpscalePass::PaletteSnapRampAware));
        assert!(matches!(
            passes[2],
            UpscalePass::PaletteSnapExpanded {
                max_derived_colors: 16
            }
        ));
        assert!(matches!(passes[3], UpscalePass::PaletteDitherReinsertCheckerboard));
    }

    #[test]
    fn cli_parses_color_boost_upscale_passes() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--upscale-pass",
            "vibrance",
            "--upscale-pass",
            "selective-warm",
        ])
        .expect("parse color boost upscale passes");

        match cli.command {
            Commands::PackMobileAnims { upscale_passes, .. } => {
                assert_eq!(
                    upscale_passes,
                    vec![CliUpscaleFilter::Vibrance, CliUpscaleFilter::SelectiveWarm]
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_local_contrast_upscale_passes() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--upscale-pass",
            "local-laplacian-clarity",
            "--upscale-pass",
            "contrast-enhance",
            "--upscale-pass",
            "adaptive-log-contrast",
        ])
        .expect("parse local contrast upscale passes");

        match cli.command {
            Commands::PackMobileAnims { upscale_passes, .. } => {
                assert_eq!(
                    upscale_passes,
                    vec![
                        CliUpscaleFilter::LocalLaplacianClarity,
                        CliUpscaleFilter::ContrastEnhance,
                        CliUpscaleFilter::AdaptiveLogContrast,
                    ]
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_upscale_profile_path() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-mobile-anims",
            "--ccdir",
            "/cc",
            "--upscale-profile",
            "/tmp/upscale-profile.toml",
        ])
        .expect("parse upscale profile path");

        match cli.command {
            Commands::PackMobileAnims { upscale_profile, .. } => {
                assert_eq!(upscale_profile, Some(PathBuf::from("/tmp/upscale-profile.toml")));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_gump_upscale_families() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-gumps",
            "--ccdir",
            "/cc",
            "--jxl",
            "--paperdoll-upscale-pass",
            "nedi2x",
            "--single-upscale-pass",
            "fsr-easu3x",
        ])
        .expect("parse gump upscale passes");

        match cli.command {
            Commands::PackGumps {
                jxl,
                paperdoll_upscale_passes,
                single_upscale_passes,
                ..
            } => {
                assert!(jxl);
                assert_eq!(paperdoll_upscale_passes, vec![CliUpscaleFilter::Nedi2x]);
                assert_eq!(single_upscale_passes, vec![CliUpscaleFilter::FsrEasu3x]);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_selects_raw_as_uncompressed_rgba_payload() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--raw",
        ])
        .expect("parse raw texture output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(raw);
                assert!(!jxl);
                assert!(zstd.is_none());
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(output_format.compression, CompressionFlag::None);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_selects_jxl_as_raw_rgba_payload_compression() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--jxl",
        ])
        .expect("parse jxl texture output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(!raw);
                assert!(jxl);
                assert!(zstd.is_none());
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::JpegXlLevel(DEFAULT_JXL_LEVEL)
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_accepts_explicit_jxl_level() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--jxl",
            "--jxl-level",
            "4",
        ])
        .expect("parse explicit jxl level");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(!raw);
                assert!(jxl);
                assert_eq!(jxl_level, Some(4));
                assert!(zstd.is_none());
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    jxl_level,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(output_format.compression, CompressionFlag::JpegXlLevel(4));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_allows_zstd_with_raw_rgba_payload() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--raw",
            "--zstd",
        ])
        .expect("parse raw zstd texture output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(raw);
                assert!(!jxl);
                assert_eq!(zstd, Some(DEFAULT_ZSTD_LEVEL));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::ZstdNoDictLevel(DEFAULT_ZSTD_LEVEL)
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_allows_zstd_with_jxl_payload() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--jxl",
            "--zstd",
        ])
        .expect("parse jxl zstd texture output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(!raw);
                assert!(jxl);
                assert_eq!(zstd, Some(DEFAULT_ZSTD_LEVEL));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::JpegXlZstdLevels {
                        jxl_level: DEFAULT_JXL_LEVEL,
                        zstd_level: DEFAULT_ZSTD_LEVEL,
                    }
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_accepts_explicit_zstd_level() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--raw",
            "--zstd=5",
        ])
        .expect("parse explicit zstd level");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                assert!(raw);
                assert!(!jxl);
                assert_eq!(zstd, Some(5));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    1.0,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(output_format.compression, CompressionFlag::ZstdNoDictLevel(5));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_allows_zstd_with_bc7_rdo_payload() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--bc7-rdo",
            "--bc7-rdo-lambda",
            "2.5",
            "--bc7-rdo-lookback-blocks",
            "128",
            "--zstd",
        ])
        .expect("parse bc7 rdo zstd texture output");

        match cli.command {
            Commands::PackArt {
                raw,
                jxl,
                zstd,
                bc7,
                bc7_rdo,
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
                ..
            } => {
                assert!(!raw);
                assert!(!jxl);
                assert_eq!(zstd, Some(DEFAULT_ZSTD_LEVEL));
                assert!(!bc7);
                assert!(bc7_rdo);
                let output_format = resolve_texture_output_format(
                    raw,
                    jxl,
                    None,
                    zstd,
                    bc7,
                    bc7_rdo,
                    bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Bc7);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::ZstdNoDictLevel(DEFAULT_ZSTD_LEVEL)
                );
                assert_eq!(output_format.bc7_rdo_lambda, 2.5);
                assert_eq!(output_format.bc7_rdo_lookback_blocks, 128);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_rejects_non_preset_bc7_rdo_lookback_blocks() {
        let error = match Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
            "--bc7-rdo",
            "--bc7-rdo-lookback-blocks",
            "96",
        ]) {
            Ok(_) => panic!("bc7 rdo lookback must be a supported preset"),
            Err(error) => error,
        };

        assert!(
            error.to_string().contains("expected one of 64, 128, or 256"),
            "{error}"
        );
    }

    #[test]
    fn cli_rejects_multiple_texture_output_formats() {
        let error = match Cli::try_parse_from([
            "uddpack",
            "pack-texmaps",
            "--ccdir",
            "/cc",
            "--output",
            "tex_land_cc.uddp",
            "--raw",
            "--bc7",
        ]) {
            Ok(_) => panic!("texture output formats are mutually exclusive"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("cannot be used with"));
    }

    #[test]
    fn cli_parses_audit_land_routing_kdl() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "audit-land-routing-kdl",
            "--routing",
            "dynamapper/assets/cc_ec_convtables/KrTerrainRouting.generated.kdl",
            "--tex-land-ec",
            "/tmp/tex_land_ec.uddp",
            "--output",
            "/tmp/land_routing_audit.json",
        ])
        .expect("parse land routing audit args");

        match cli.command {
            Commands::AuditLandRoutingKdl {
                routing,
                tex_land_ec,
                output,
            } => {
                assert_eq!(
                    routing,
                    PathBuf::from("dynamapper/assets/cc_ec_convtables/KrTerrainRouting.generated.kdl")
                );
                assert_eq!(tex_land_ec, PathBuf::from("/tmp/tex_land_ec.uddp"));
                assert_eq!(output, PathBuf::from("/tmp/land_routing_audit.json"));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_terrain_practical_review() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "report-ec-terrain-practical-review",
            "--ecdir",
            "/ec",
            "--tex-land-ec",
            "/tmp/tex_land_ec.uddp",
            "--output",
            "/tmp/review.json",
        ])
        .expect("parse terrain practical review args");

        match cli.command {
            Commands::ReportEcTerrainPracticalReview {
                source_dirs,
                tex_land_ec,
                output,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(tex_land_ec, Some(PathBuf::from("/tmp/tex_land_ec.uddp")));
                assert_eq!(output, PathBuf::from("/tmp/review.json"));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_hues() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-hues",
            "--ccdir",
            "/cc",
            "--output",
            "hues.uddp",
        ])
        .expect("parse hues args");

        match cli.command {
            Commands::PackHues { source_dirs, output, .. } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(output, PathBuf::from("hues.uddp"));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_gumps() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-gumps",
            "--ecdir",
            "/ec",
            "--output",
            "gumps_ec.uddp",
            "--max-id",
            "12345",
        ])
        .expect("parse ec gump args");

        match cli.command {
            Commands::PackEcGumps {
                source_dirs,
                output,
                max_id,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("gumps_ec.uddp"));
                assert_eq!(max_id, 12345);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_ec_mobile_animation_tables_path() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-mobile-anims",
            "--ecdir",
            "/ec",
            "--tables",
            "/tables",
            "--ec-mobile-animations-kdl",
            "/tables/EcMobileAnimations.kdl",
            "--crop-transparent-bounds",
        ])
        .expect("parse ec mobile animation metadata paths");

        match cli.command {
            Commands::PackEcMobileAnims {
                tables,
                ec_mobile_animations_kdl,
                crop_transparent_bounds,
                ..
            } => {
                assert_eq!(tables, Some(PathBuf::from("/tables")));
                assert_eq!(
                    ec_mobile_animations_kdl,
                    Some(PathBuf::from("/tables/EcMobileAnimations.kdl"))
                );
                assert!(crop_transparent_bounds);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn classic_patch_args_convert_to_shared_options() {
        let options: ClassicPatchOptions = ClassicPatchArgs {
            include_verdata: true,
            include_map_difs: false,
            include_static_difs: true,
        }
        .into();

        assert_eq!(
            options,
            ClassicPatchOptions {
                verdata: true,
                map_difs: false,
                static_difs: true,
            }
        );
    }

    #[test]
    fn collect_source_dirs_rejects_missing_ccdir() {
        let args = SourceDirArgs {
            ccdir: Some(PathBuf::from("/definitely/missing/uo/client")),
            ecdir: None,
        };
        let error = collect_source_dirs(&args).expect_err("missing ccdir should fail");

        assert!(error.to_string().contains("--ccdir does not exist"));
    }

    #[test]
    fn collect_ec_source_dirs_rejects_missing_ecdir() {
        let args = SourceDirArgs {
            ccdir: None,
            ecdir: Some(PathBuf::from("/definitely/missing/ec/client")),
        };
        let error = collect_ec_source_dirs(&args).expect_err("missing ecdir should fail");

        assert!(error.to_string().contains("--ecdir does not exist"));
    }

    #[test]
    fn cli_parses_pack_map_patch_flags() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-map",
            "--ccdir",
            "/cc",
            "--map-id",
            "2",
            "--include-verdata",
            "--include-map-difs",
            "--include-static-difs",
        ])
        .expect("parse map patch args");

        match cli.command {
            Commands::PackMap {
                classic_patches,
                map_id,
                ..
            } => {
                assert_eq!(map_id, 2);
                assert!(classic_patches.include_verdata);
                assert!(classic_patches.include_map_difs);
                assert!(classic_patches.include_static_difs);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_radar_patch_flags_independently() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-radar",
            "--ccdir",
            "/cc",
            "--map-id",
            "1",
            "--uddpdir",
            "/packages",
            "--include-map-difs",
        ])
        .expect("parse radar patch args");

        match cli.command {
            Commands::PackRadar {
                classic_patches,
                map_id,
                ..
            } => {
                assert_eq!(map_id, 1);
                assert!(!classic_patches.include_verdata);
                assert!(classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_lights_verdata_without_diff_flags() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-lights",
            "--ccdir",
            "/cc",
            "--include-verdata",
        ])
        .expect("parse lights patch args");

        match cli.command {
            Commands::PackLights {
                classic_patches, ..
            } => {
                assert!(classic_patches.include_verdata);
                assert!(!classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
            }
            _ => panic!("unexpected command parsed"),
        }
    }
}
