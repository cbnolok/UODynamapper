use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::info;

use crate::ec_material_audit::{
    audit_ec_material_refs, audit_ec_surface_redirection,
    audit_ec_terrain_overrides, audit_ec_terrain_primary_selection, inventory_ec_support_textures,
    write_ec_abnormality_report, write_ec_effect_meta_manifest, write_ec_material_baseline_report,
    write_ec_terrain_practical_review,
    write_terrain_definition_owner_audit, write_tileart_owner_audit,
    write_terrain_definition_review, write_tileart_liquid_surface_report,
};
use crate::upscale_profile_config::load_upscale_profile;
use clap::{ArgGroup, Args, Parser, Subcommand};
use color_eyre::eyre;
use serde::Serialize;
use uocf::map_edition::MapEdition;
use uocf::utils::bundled_files;
use udd_conv::conversion_tables::{
    EC_MOBILE_ANIMATIONS_KDL, EC_SURFACE_OVERRIDES_KDL, EC_TERRAIN_OVERRIDES_KDL, TERRAIN_TRANSCODE_KDL,
};
use udd_conv::{
    classic_patches::ClassicPatchOptions,
    tex_aux_ec::pack_tex_aux_ec,
    classic_sources::SourceFormatPreference,
    cc_gumps::{
        convert_gumps_to_uddp_from_sources_with_patches_and_options, CcGumpsOptions,
        CC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_CC_DEFAULT_OUTPUT,
    },
    cc_map::convert_map_mul_to_uddp_from_sources_with_patches,
    chunk_compression::ChunkCompression,
    page_compression::LOSSLESS_PAGE_ZSTD_LEVEL,
    single_gumps::{default_single_gump_compression, GumpPixelFormat},
    cc_statics::{
        convert_statics_mul_to_uddp_from_sources_with_dimensions_and_patches,
        convert_statics_mul_to_uddp_from_sources_with_patches,
    },
    effects_ec::{pack_effects_ec, EffectsEcPackOptions},
    ec_gumps::{
        convert_ec_gumps_to_uddp_from_sources, EcGumpsOptions, EC_GUMP_DEFAULT_MAX_ID,
        EC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_EC_DEFAULT_OUTPUT,
    },
    hues::{convert_hues_to_hues_uddp_from_sources, HueRampMode, HuesOptions},
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
        convert_texmaps_mul_to_tex_land_cc_uddp_with_patches, TexLandCcAtlasOptions, CLASSIC_LAND_LOSSY_REFUSAL,
        DEFAULT_ATLAS_GUTTER as CC_TEXMAPS_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_TEXMAPS_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_TEXMAPS_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tex_land_ec::{
        convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_loaded_sources, TexLandEcAtlasOptions, BC7_LAND_PAGE_ZSTD_LEVEL,
        DEFAULT_ATLAS_GUTTER as EC_LAND_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    terrain_meta::write_terrain_meta,
    tilemeta::{
        build_tilemeta_uddp_from_sources, build_tilemeta_uddp_from_split_sources,
        build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources, TileMetaBuildOptions,
    },
    upscale::{EnhancementFilter, UpscaleFilter, UpscalePass},
    world_lights::{convert_client_lights_to_world_lights_uddp, WorldLightsOptions},
    CompressionFlag, PagePixelFormat,
};

/// The level of a bare `--zstd` where it adds a pass over another encoding (JPEG XL, BC7, raw texture pages).
const DEFAULT_ZSTD_LEVEL: i32 = 7;
const DEFAULT_ZSTD_LEVEL_VALUE: &str = "7";
/// The BC7 RDO lambda that turns RDO off.
const NO_BC7_RDO_LAMBDA: f32 = 0.0;
const CHUNK_ZSTD_HELP: &str = "Zstd level of the chunks (--zstd=LEVEL). Chunks are compressed with plain Zstd unless --stored; without a level (or without the flag) map chunks use level 19 and statics chunks level 7.";
const CHUNK_STORED_HELP: &str = "Store the chunks uncompressed: the fastest runtime read, for a package about 6.6x (map) or 2.8x (statics) larger.";
const DEFAULT_JXL_LEVEL: u8 = 6;
const LAND_ZSTD_HELP: &str = "Zstd level of the land pages (--zstd=LEVEL), which without a format flag are lossless RGBA8888 with Zstd 19; with --raw or --jxl, a file-level Zstd pass after that encoding. A bare --zstd keeps level 19.";
const CLASSIC_LAND_BC7_HELP: &str = "Refused, with the reason: Classic land stays lossless.";
const GUMP_ZSTD_HELP: &str = "Zstd level of the single gumps and, unless --jxl, of the atlas pages (--zstd=LEVEL). Without it single gumps use level 7 (19 with --bc1 or --bc7) and atlas pages, lossless RGBA8888, level 19; with --jxl a Zstd pass over the JPEG XL pages. A bare --zstd keeps these levels, and gives the JPEG XL pages a level 7 pass.";
const ART_ZSTD_HELP: &str = "Zstd level of the atlas pages (--zstd=LEVEL), which without a format flag are lossless RGBA8888 with Zstd 19; with --raw, --jxl, --bc7 or --bc7-rdo, a file-level Zstd pass after that encoding. A bare --zstd keeps level 19 without a format flag, and is a level 7 pass after one.";
const TEX_AUX_ZSTD_HELP: &str = "Zstd level of the textures (--zstd=LEVEL), which without a format flag are lossless RGBA8888 with Zstd 19; with --raw, --jxl, --bc7 or --bc7-rdo, a file-level Zstd pass after that encoding. A bare --zstd keeps level 19 without a format flag, and is a level 7 pass after one.";
const GUMP_RAW_HELP: &str = "Store the single gumps as lossless RGBA8888: the default.";
const GUMP_BC1_HELP: &str = "Store every single (non-paperdoll) Classic gump as BC1: lossy, with one bit of alpha (colour 0 is its transparent texel), and Zstd 19 unless --zstd=LEVEL. Paperdoll gumps (50001-69999) stay lossless on their atlas pages, and the paperdoll body gumps (12-15, 665, 666) stay RGBA8888: Classic partial hue needs exact greys.";
const GUMP_BC7_HELP: &str = "Store every single (non-paperdoll) Enhanced gump as BC7 of premultiplied colour: lossy, soft alpha kept, and Zstd 19 unless --zstd=LEVEL. Paperdoll gumps (50001-69999) stay lossless on their atlas pages, and the paperdoll body gumps (12-15, 665, 666) stay RGBA8888: the Enhanced client hues exact greys only.";
const OUTPUT_PATH_HELP: &str = "Output paths (--output, --art-output, --land-output, ...) and their default \
file names resolve against the current working directory; absolute paths are used as given. \
Nothing is written into --ccdir or --ecdir unless an output path points there.";

/// Pack UODynamapper runtime packages from Classic and Enhanced Client assets.
#[derive(Parser)]
#[command(author, version, about, long_about = None, after_help = OUTPUT_PATH_HELP)]
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

const USE_VERDATA_HELP: &str = "Apply verdata.mul, the patch file only clients of that era ship. It patches only the \
standard map0.mul, staidx0.mul and statics0.mul and the other files whose ids it lists (art, texmaps, gumps, anims, \
sounds, multis, lights, tiledata, animdata), never a UOP map or the x variant files. Ignored by default; when \
requested but absent, a warning is printed and the sources are read without it.";

#[derive(Args, Clone, Copy, Debug, Default)]
struct ClassicPatchArgs {
    #[arg(long = "use-verdata", default_value_t = false, help = USE_VERDATA_HELP)]
    use_verdata: bool,
    #[arg(long = "include-map-difs", default_value_t = false, help = "Include each facet's mapdifN/mapdiflN patch files when present (pack-map, pack-map-statics, pack-radar). A facet 1 with no map1 file is always built from map0 with its facet-1 difs.")]
    include_map_difs: bool,
    #[arg(long = "include-static-difs", default_value_t = false, help = "Include each facet's stadifN/stadifiN/stadiflN patch files when present (pack-statics, pack-map-statics, pack-radar).")]
    include_static_difs: bool,
}

impl From<ClassicPatchArgs> for ClassicPatchOptions {
    fn from(value: ClassicPatchArgs) -> Self {
        Self {
            verdata: value.use_verdata,
            map_difs: value.include_map_difs,
            static_difs: value.include_static_difs,
            ..ClassicPatchOptions::NONE
        }
    }
}

/// `--map-edition` of the map, statics and radar commands.
#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CliMapEdition {
    /// The standard facet files (mapN, staidxN, staticsN).
    #[default]
    Standard,
    /// The x files (mapNxLegacyMUL.uop or mapNx.mul, staidxNx, staticsNx) the server selects with the 0xA9 flag
    /// 0x8000. Never falls back to the standard files.
    Variant,
}

impl From<CliMapEdition> for MapEdition {
    fn from(edition: CliMapEdition) -> Self {
        match edition {
            CliMapEdition::Standard => MapEdition::Standard,
            CliMapEdition::Variant => MapEdition::Variant,
        }
    }
}

const MAP_EDITION_HELP: &str = "Map edition to read: standard, or variant (the x files mapNx / staidxNx / staticsNx \
the server selects with the 0xA9 flag 0x8000; facets 0, 1, 2 and 5 only, the others read their standard files). The \
variant never falls back to the standard files. The dif options apply to whichever edition is selected.";

const TERRAIN_OVERRIDES_HELP: &str = "EcTerrainOverrides.kdl to read. Default: the copy shipped with the tools.";
const SURFACE_OVERRIDES_HELP: &str = "EcSurfaceOverrides.kdl to read. Default: the copy shipped with the tools.";

/// The EcMobileAnimations.kdl of `--ec-mobile-animations-kdl`, else of `--tables` (the file, or the folder holding it),
/// else the copy shipped with the tools.
fn resolve_ec_mobile_animations_kdl(exact: Option<PathBuf>, tables: Option<PathBuf>) -> eyre::Result<PathBuf> {
    let given = exact.or_else(|| tables.map(|tables| {
        if tables.is_dir() { EC_MOBILE_ANIMATIONS_KDL.in_dir(&tables) } else { tables }
    }));
    bundled_files::resolve(given.as_deref(), EC_MOBILE_ANIMATIONS_KDL)
}

/// The Classic options of a map, statics or radar command.
fn facet_patch_options(classic_patches: ClassicPatchArgs, map_edition: CliMapEdition) -> ClassicPatchOptions {
    ClassicPatchOptions { map_edition: map_edition.into(), ..classic_patches.into() }
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
            compression: zstd_compression(zstd.unwrap_or(LOSSLESS_PAGE_ZSTD_LEVEL)),
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            bc7_rdo_lookback_blocks,
            label: "RGBA8888 Zstd",
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

/// A `--zstd` that adds a pass over another encoding: a bare one is [`DEFAULT_ZSTD_LEVEL`].
fn pass_zstd_level(zstd: Option<Option<i32>>) -> Option<i32> {
    zstd.map(|level| level.unwrap_or(DEFAULT_ZSTD_LEVEL))
}

/// The art and auxiliary texture pages of `pack-art`, `pack-ec-textures` and `pack-ec-tex-aux`: without a format flag,
/// lossless RGBA8888 with Zstd at `--zstd=LEVEL` or [`LOSSLESS_PAGE_ZSTD_LEVEL`] (a bare `--zstd` too), as for gump atlas
/// pages; after `--raw`, `--jxl`, `--bc7` or `--bc7-rdo` a bare `--zstd` is a [`DEFAULT_ZSTD_LEVEL`] pass over that
/// encoding.
fn resolve_art_output_format(
    raw: bool,
    jxl: bool,
    jxl_level: Option<u8>,
    zstd: Option<Option<i32>>,
    bc7: bool,
    bc7_rdo: bool,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> eyre::Result<TextureOutputFormat> {
    let zstd = if raw || jxl || bc7 || bc7_rdo { pass_zstd_level(zstd) } else { zstd.flatten() };
    resolve_texture_output_format(raw, jxl, jxl_level, zstd, bc7, bc7_rdo, bc7_rdo_lambda, bc7_rdo_lookback_blocks)
}

/// A land `--zstd`: a bare one keeps the land pages' own [`LOSSLESS_PAGE_ZSTD_LEVEL`], as for map chunks.
fn land_zstd_level(zstd: Option<Option<i32>>) -> Option<i32> {
    zstd.map(|level| level.unwrap_or(LOSSLESS_PAGE_ZSTD_LEVEL))
}

/// The land pages of `pack-texmaps` and `pack-ec-textures`: without a format flag, lossless RGBA8888 with Zstd at
/// `--zstd=LEVEL` or [`LOSSLESS_PAGE_ZSTD_LEVEL`]; BC7 blocks with Zstd at `--zstd=LEVEL` or [`BC7_LAND_PAGE_ZSTD_LEVEL`];
/// `--raw` and `--jxl` resolve as for any other texture.
fn resolve_land_output_format(
    args: TextureOutputFormatArgs,
    bc7_rdo_lambda: f32,
    bc7_rdo_lookback_blocks: usize,
) -> eyre::Result<TextureOutputFormat> {
    if args.bc7 || args.bc7_rdo {
        return resolve_texture_output_format_args(
            TextureOutputFormatArgs { zstd: Some(args.zstd.unwrap_or(BC7_LAND_PAGE_ZSTD_LEVEL)), ..args },
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
        );
    }
    if args.raw || args.jxl {
        return resolve_texture_output_format_args(args, bc7_rdo_lambda, bc7_rdo_lookback_blocks);
    }
    Ok(TextureOutputFormat {
        compression: zstd_compression(args.zstd.unwrap_or(LOSSLESS_PAGE_ZSTD_LEVEL)),
        pixel_format: PagePixelFormat::Rgba8888,
        bc7_rdo_lambda: 0.0,
        bc7_rdo_lookback_blocks,
        label: "RGBA8888 Zstd",
    })
}

/// `pack-texmaps` refuses BC7, with the reason.
fn refuse_classic_land_bc7(bc7: bool, bc7_rdo: bool) -> eyre::Result<()> {
    if bc7 || bc7_rdo {
        eyre::bail!("pack-texmaps: {CLASSIC_LAND_LOSSY_REFUSAL}");
    }
    Ok(())
}

/// The gump atlas pages: lossless RGBA8888 with Zstd at `--zstd=LEVEL` or [`LOSSLESS_PAGE_ZSTD_LEVEL`] (a bare `--zstd`
/// too), or JPEG XL with `--jxl`, followed by a Zstd pass when `--zstd` is given as well.
fn gump_atlas_compression(jxl: bool, jxl_level: Option<u8>, zstd: Option<Option<i32>>) -> CompressionFlag {
    if jxl {
        pass_zstd_level(zstd)
            .map(|level| jxl_zstd_compression_levels(jxl_level, level))
            .unwrap_or_else(|| jxl_compression_level(jxl_level))
    } else {
        zstd_compression(zstd.flatten().unwrap_or(LOSSLESS_PAGE_ZSTD_LEVEL))
    }
}

/// The single gumps' pixel format: the package's lossy format with `--bc1` (Classic) or `--bc7` (Enhanced), lossless
/// RGBA8888 otherwise.
fn single_gump_pixel_format(lossy: bool, lossy_format: GumpPixelFormat) -> GumpPixelFormat {
    if lossy { lossy_format } else { GumpPixelFormat::Rgba8888 }
}

/// The single gumps: Zstd at `--zstd=LEVEL`, or the package's default level for their format (a bare `--zstd` too).
fn single_gump_compression(zstd: Option<Option<i32>>, format: GumpPixelFormat) -> CompressionFlag {
    zstd.flatten().map(zstd_compression).unwrap_or(default_single_gump_compression(format))
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
    info!(
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

/// How `pack-hues` spreads the 32 Classic `hues.mul` colours over a 256-texel palette row.
#[derive(clap::ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CliHueRamp {
    /// Colour i at x = 8i, ramping linearly to colour i + 1 (EA's Hues.uop).
    #[default]
    Linear,
    /// Each colour repeated over 8 texels.
    Flat,
}

impl From<CliHueRamp> for HueRampMode {
    fn from(ramp: CliHueRamp) -> Self {
        match ramp {
            CliHueRamp::Linear => HueRampMode::Linear,
            CliHueRamp::Flat => HueRampMode::Flat,
        }
    }
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
    /// Packs classic art.mul/artidx.mul into tex_art_cc.uddp atlas pages, lossless RGBA8888 with Zstd 19 unless a format is given.
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
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression instead of the default Zstd 19: a few percent larger, and many times slower to decode.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = ART_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
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
    /// Packs Classic texmaps.mul into tex_land_cc.uddp atlas pages, lossless RGBA8888 with Zstd 19 unless a format is given.
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
        #[arg(long, help = "Write RGBA8888 atlas pages with lossless JPEG XL payload compression: about the size of the default Zstd 19, but many times slower to decode.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = LAND_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
        #[arg(long, help = CLASSIC_LAND_BC7_HELP)]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = CLASSIC_LAND_BC7_HELP)]
        bc7_rdo: bool,
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
        #[arg(long, help = "Directory containing EcMobileAnimations.kdl, or the KDL file itself. Default: the copy shipped with the tools.")]
        tables: Option<PathBuf>,
        #[arg(long = "ec-mobile-animations-kdl", help = "Exact EcMobileAnimations.kdl path. Default: the copy shipped with the tools.")]
        ec_mobile_animations_kdl: Option<PathBuf>,
        #[arg(long = "upscale-pass", value_enum, help = "Add an upscale pass before atlas encoding. Repeat to chain filters.")]
        upscale_passes: Vec<CliUpscaleFilter>,
        #[arg(long = "upscale-profile", help = "TOML or KDL file with per-image-type upscale passes and per-id overrides.")]
        upscale_profile: Option<PathBuf>,
    },
    /// Packs EC art and land in one shared source pass into tex_art_ec.uddp and tex_land_ec.uddp. Without a format flag
    /// art and land pages are lossless RGBA8888 with Zstd 19; BC7 land blocks take Zstd 19 too.
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
        #[arg(long = "tilemeta-use-ec-radarcol", default_value_t = false)]
        tilemeta_use_ec_radarcol: bool,
        #[arg(long, help = "Embed this TerrainTranscode.kdl (the terrain routing table) in tex_land_ec.uddp. Default: the copy shipped with the tools.")]
        land_transcode_kdl: Option<PathBuf>,
        #[arg(long, help = "Embed this EcTerrainOverrides.kdl (the reviewed terrain layer substitutions) in tex_land_ec.uddp. Default: the copy shipped with the tools.")]
        land_terrain_overrides_kdl: Option<PathBuf>,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_WIDTH)]
        art_atlas_width: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_PAGE_HEIGHT)]
        art_atlas_height: u32,
        #[arg(long, default_value_t = EC_ART_DEFAULT_ATLAS_GUTTER)]
        art_gutter: u16,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_WIDTH)]
        land_atlas_width: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_PAGE_HEIGHT)]
        land_atlas_height: u32,
        #[arg(long, default_value_t = EC_LAND_DEFAULT_ATLAS_GUTTER)]
        land_gutter: u16,
        #[arg(long, help = "Write uncompressed EC RGBA8888 atlas pages.")]
        raw: bool,
        #[arg(long, help = "Write EC RGBA8888 atlas pages with lossless JPEG XL payload compression instead of the default Zstd 19.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for shared EC output, 1 fastest through 10 strongest. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = "Zstd level of the EC art and land pages (--zstd=LEVEL), which without a format flag are lossless RGBA8888 with Zstd 19, as are BC7 land blocks; with another format, a file-level Zstd pass after that encoding. A bare --zstd keeps level 19, except as a pass over another art format, where it is level 7.")]
        zstd: Option<Option<i32>>,
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
        #[arg(long, num_args = 0..=1, require_equals = true, help = "Zstd level of the EC art pages, overriding --zstd: without an art format flag, the level of their lossless RGBA8888 pages' Zstd (default 19); with one, a file-level Zstd pass after that encoding. Optionally pass --art-zstd=LEVEL; a bare --art-zstd keeps level 19, and is a level 7 pass after a format flag.")]
        art_zstd: Option<Option<i32>>,
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
        #[arg(long, num_args = 0..=1, require_equals = true, help = "Apply a file-level Zstd compression pass to EC land after the selected land output encoding; without a land format flag or in BC7, the level of the land pages' Zstd (default 19). Optionally pass --land-zstd=LEVEL; a bare --land-zstd keeps level 19.")]
        land_zstd: Option<Option<i32>>,
        #[arg(long, help = "Write EC land atlas pages as BC7 without BC7 RDO, overriding the shared format flags: the lossy Enhanced land, with Zstd 19 on the blocks unless --land-zstd=LEVEL.")]
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
        #[arg(long, default_value = "ec_support_texture_summary.md")]
        summary_output: PathBuf,
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
        #[arg(long = "terrain-overrides", help = TERRAIN_OVERRIDES_HELP)]
        terrain_overrides: Option<PathBuf>,
        #[arg(long = "tex-land-ec")]
        tex_land_ec: Option<PathBuf>,
        #[arg(long, default_value = "ec_terrain_primary_selection.json")]
        output: PathBuf,
    },
    /// Writes a short JSON queue of terrain material decisions needed before runtime use.
    ReportEcTerrainPracticalReview {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long = "terrain-overrides", help = TERRAIN_OVERRIDES_HELP)]
        terrain_overrides: Option<PathBuf>,
        #[arg(long = "tex-land-ec")]
        tex_land_ec: Option<PathBuf>,
        #[arg(long, default_value = "ec_terrain_practical_review.json")]
        output: PathBuf,
    },
    /// Validates EcTerrainOverrides.kdl against source-derived TerrainDefinition evidence.
    AuditEcTerrainOverrides {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long = "terrain-overrides", help = TERRAIN_OVERRIDES_HELP)]
        terrain_overrides: Option<PathBuf>,
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
        #[arg(long = "surface-overrides", help = SURFACE_OVERRIDES_HELP)]
        surface_overrides: Option<PathBuf>,
        #[arg(long, default_value = "ec_surface_redirection.json")]
        output: PathBuf,
    },
    /// Builds terrain-layer sidecar binary from TerrainDefinition.uop.
    #[command(name = "pack-terrain-meta")]
    PackTerrainMeta {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "terrain_meta.bin")]
        output: PathBuf,
    },
    /// Writes a JSON manifest of all EffectTexture.uop resources with direct owner references.
    #[command(name = "write-ec-effect-meta-manifest")]
    WriteEcEffectMetaManifest {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "ec_effect_meta_manifest.json")]
        output: PathBuf,
    },
    /// Writes a CSV abnormality report for EC item tile base-texture selections, plus a markdown summary.
    #[command(name = "write-ec-abnormality-report")]
    WriteEcAbnormalityReport {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        tilemeta: PathBuf,
        #[arg(long, default_value = "ec_abnormality_report.csv")]
        output: PathBuf,
    },
    /// Writes a filtered CSV of tileart owner references (owner_kind=tileart subset of the Phase 0 census).
    #[command(name = "write-tileart-owner-audit")]
    WriteTileartOwnerAudit {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tileart_owner_audit.csv")]
        output: PathBuf,
    },
    /// Writes a filtered CSV of TerrainDefinition owner references (owner_kind=terrain_definition subset of the Phase 0 census).
    #[command(name = "write-terrain-definition-owner-audit")]
    WriteTerrainDefinitionOwnerAudit {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "terrain_definition_owner_audit.csv")]
        output: PathBuf,
    },
    /// Writes a JSON review artifact for all TerrainDefinition material entries with layer classification.
    #[command(name = "write-terrain-definition-review")]
    WriteTerrainDefinitionReview {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "terrain_definition_review.json")]
        output: PathBuf,
    },
    /// Writes a JSON report of tileart entries with Liquid type or Wet/Surface flags.
    #[command(name = "write-tileart-liquid-surface-report")]
    WriteTileartLiquidSurfaceReport {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        /// Optional path to tilemeta.uddp for role-aware base selection.
        #[arg(long)]
        tilemeta: Option<PathBuf>,
        #[arg(long, default_value = "tileart_liquid_surface_report.json")]
        output: PathBuf,
    },
    /// Packs non-base WorldArt auxiliary EC texture refs from tilemeta.uddp into tex_aux_ec.uddp, plus the
    /// client's missing-texture fallback (SystemTextures.uop), aliased by every absent source id. Textures are lossless
    /// RGBA8888 with Zstd 19 unless a format is given.
    #[command(name = "pack-ec-tex-aux")]
    #[command(group(ArgGroup::new("output_format").args(["raw", "jxl", "bc7", "bc7_rdo"])))]
    PackEcTexAux {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        tilemeta: PathBuf,
        #[arg(long, default_value = "tex_aux_ec.uddp")]
        output: PathBuf,
        #[arg(long, help = "Write uncompressed RGBA8888 textures.")]
        raw: bool,
        #[arg(long, help = "Write RGBA8888 textures with lossless JPEG XL payload compression instead of the default Zstd 19: a few percent larger, and many times slower to decode.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level, 1 fastest through 10 strongest. Default 6. Applies only with --jxl.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = TEX_AUX_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
        #[arg(long, help = "Write BC7 textures without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 textures with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA)]
        bc7_rdo_lambda: f32,
        #[arg(long, value_parser = parse_bc7_rdo_lookback_blocks, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS)]
        bc7_rdo_lookback_blocks: usize,
        #[arg(long = "upscale-pass", value_enum, help = "Add an upscale pass. Repeat to chain filters.")]
        upscale_passes: Vec<CliUpscaleFilter>,
    },
    /// Packs the EC particle effects (definitions, compiled Chaos and NIF systems, textures) into effects_ec.uddp.
    #[command(name = "pack-ec-effects")]
    PackEcEffects {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "effects_ec.uddp")]
        output: PathBuf,
        #[arg(long, help = "Encode uncompressed textures whose size is a multiple of 4 to BC7 (default: RGBA8). DXT1/DXT5 sources stay BC1/BC3.")]
        bc7: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = DEFAULT_ZSTD_LEVEL_VALUE, help = "Zstd level of the manifests and textures. Optionally pass --zstd=LEVEL.")]
        zstd: Option<i32>,
        #[arg(long, help = "Write every finding of the packer to this file, one line each.")]
        report: Option<PathBuf>,
        #[arg(long, help = "Fail on error findings instead of writing flagged records (particle files missing from the client are warnings).")]
        strict: bool,
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
        use_ec_radarcol: bool,
    },
    /// Generates a radar map (facet0X.dds) from Classic map and statics.
    PackRadar {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard, help = MAP_EDITION_HELP)]
        map_edition: CliMapEdition,
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
        #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard, help = MAP_EDITION_HELP)]
        map_edition: CliMapEdition,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        uop: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, conflicts_with = "stored", help = CHUNK_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
        #[arg(long, default_value_t = false, help = CHUNK_STORED_HELP)]
        stored: bool,
    },
    /// Packs Classic staticsX.mul into staticsX.uddp blocks.
    PackStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard, help = MAP_EDITION_HELP)]
        map_edition: CliMapEdition,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, num_args = 0..=1, require_equals = true, conflicts_with = "stored", help = CHUNK_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
        #[arg(long, default_value_t = false, help = CHUNK_STORED_HELP)]
        stored: bool,
    },
    /// Packs Classic mapX.uddp and staticsX.uddp together, reusing the map dimensions for statics.
    PackMapStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long = "map-edition", value_enum, default_value_t = CliMapEdition::Standard, help = MAP_EDITION_HELP)]
        map_edition: CliMapEdition,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        map_output: Option<PathBuf>,
        #[arg(long)]
        statics_output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        uop: bool,
        #[arg(long, num_args = 0..=1, require_equals = true, conflicts_with = "stored", help = CHUNK_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
        #[arg(long, default_value_t = false, help = CHUNK_STORED_HELP)]
        stored: bool,
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
        #[arg(long, value_enum, default_value_t = CliHueRamp::Linear, help = "How hues.mul colours fill a palette row: linear ramp between colours (EA's Hues.uop) or flat 8-texel blocks. EC Hues.uop sources are copied as they are.")]
        hue_ramp: CliHueRamp,
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
        #[arg(long, conflicts_with = "bc1", help = GUMP_RAW_HELP)]
        raw: bool,
        #[arg(long, help = GUMP_BC1_HELP)]
        bc1: bool,
        #[arg(long, help = "Write paperdoll equipment gump atlas pages with lossless JPEG XL payload compression instead of Zstd 19: pages about a fifth smaller, but many times slower to decode. Single gumps aren't affected.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for gump atlas pages. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = GUMP_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
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
        #[arg(long, conflicts_with = "bc7", help = GUMP_RAW_HELP)]
        raw: bool,
        #[arg(long, help = GUMP_BC7_HELP)]
        bc7: bool,
        #[arg(long, help = "Write paperdoll equipment gump atlas pages with lossless JPEG XL payload compression instead of Zstd 19: pages about a fifth smaller, but many times slower to decode. Single gumps aren't affected.")]
        jxl: bool,
        #[arg(long = "jxl-level", value_parser = clap::value_parser!(u8).range(1..=10), help = "JPEG XL encoder level for gump atlas pages. Default 6.")]
        jxl_level: Option<u8>,
        #[arg(long, num_args = 0..=1, require_equals = true, help = GUMP_ZSTD_HELP)]
        zstd: Option<Option<i32>>,
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
    run_with_args(std::env::args_os())
}

pub fn run_with_args<I, T>(args: I) -> eyre::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    match Cli::parse_from(args).command {
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
            let out_file = resolve_output_path(&output);
            let output_format = resolve_art_output_format(
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
            info!(
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
            upscale,
            upscale_64_passes,
            upscale_128_passes,
            upscale_profile,
        } => {
            refuse_classic_land_bc7(bc7, bc7_rdo)?;
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let output_format = resolve_land_output_format(
                TextureOutputFormatArgs { raw, jxl, jxl_level, zstd: land_zstd_level(zstd), ..Default::default() },
                NO_BC7_RDO_LAMBDA,
                udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
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
            info!(
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
            let out_file = resolve_output_path(&output);
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
                    compression: output_format.compression,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    upscale_passes: convert_upscale_passes(upscale_passes),
                    upscale_profile,
                },
            )?;
            info!(
                "Wrote {} pages ({}) for {} packed frames out of {} total frames across {} animations to '{}'.",
                summary.page_count,
                output_format.label,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                out_file.display()
            );
            if summary.skipped_animation_count != 0 {
                log::warn!(
                    "{} animation entries were left out because they couldn't be read; each is named in a warning above",
                    summary.skipped_animation_count
                );
            }
            info!(
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
            let out_file = resolve_output_path(&output);
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
                    compression: output_format.compression,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    upscale_passes: convert_upscale_passes(upscale_passes),
                    upscale_profile,
                    metadata_path: Some(resolve_ec_mobile_animations_kdl(ec_mobile_animations_kdl, tables)?),
                    allow_missing_metadata: false,
                },
            )?;
            info!(
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
            info!(
                "EC mobile animation atlas usage: {} used pixels, {} filled ({}) and {} empty ({}); estimated uncompressed page payload {} bytes.",
                summary.used_page_pixel_count,
                summary.filled_pixel_count,
                percent_string(summary.filled_pixel_count, summary.used_page_pixel_count),
                summary.empty_pixel_count,
                percent_string(summary.empty_pixel_count, summary.used_page_pixel_count),
                estimated_bc7_bytes(summary.used_page_pixel_count, output_format.pixel_format),
            );
            let layout = summary.layout;
            info!(
                "EC mobile animation records: {} sequence bodies ({} plain actions, {} aliases, {} unresolved, {} plain actions without frames, {} frame count mismatches), {} unmapped sequences without frames, {} sequence parse errors; {} no-sequence bodies ({} directional and {} undirected actions); {} mirrored records; {} missing frame placeholders; {} unreferenced source frames; {} dropped skipped frames.",
                layout.sequence_body_count,
                layout.plain_action_count,
                layout.alias_action_count,
                layout.unresolved_action_count,
                layout.plain_action_without_frames_count,
                layout.frame_count_mismatch_action_count,
                layout.sequence_without_frames_count,
                layout.sequence_parse_error_count,
                layout.no_sequence_body_count,
                layout.fallback_directional_action_count,
                layout.fallback_undirected_action_count,
                layout.mirrored_record_count,
                layout.missing_frame_count,
                layout.unreferenced_frame_count,
                layout.dropped_skipped_frame_count,
            );
            info!(
                "EC mobile body mapping: {} tileart placeholders ({} falling back to body 0 frames, {} without frames, {} ghosts), {} bodies sharing another body's animation set, {} bodies without placeholder.",
                layout.placeholder_body_count,
                layout.frames_fallback_body_count,
                layout.body_without_frames_count,
                layout.ghost_body_count,
                layout.shared_animation_set_body_count,
                layout.unmapped_body_count,
            );
        }
        Commands::PackEcTextures {
            source_dirs: source_dir_args,
            classic_patches,
            art_output,
            land_output,
            tilemeta_output,
            tilemeta_use_ec_radarcol,
            land_transcode_kdl,
            land_terrain_overrides_kdl,
            art_atlas_width,
            art_atlas_height,
            art_gutter,
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
            // Before the art pass, so a missing table fails the command at once.
            let land_terrain_overrides_kdl =
                bundled_files::resolve(land_terrain_overrides_kdl.as_deref(), EC_TERRAIN_OVERRIDES_KDL)?;
            let land_transcode_kdl = bundled_files::resolve(land_transcode_kdl.as_deref(), TERRAIN_TRANSCODE_KDL)?;
            let art_out_file = resolve_output_path(&art_output);
            let land_out_file = resolve_output_path(&land_output);
            let tilemeta_out_file = tilemeta_output
                .as_ref()
                .map(|output| resolve_output_path(output));
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
                bc7,
                bc7_rdo,
                ..Default::default()
            };
            let art_output_format = resolve_art_output_format(
                art_raw || shared_output_format.raw,
                art_jxl || shared_output_format.jxl,
                art_jxl_level.or(shared_output_format.jxl_level),
                art_zstd.or(zstd),
                art_bc7 || shared_output_format.bc7,
                art_bc7_rdo || shared_output_format.bc7_rdo,
                bc7_rdo_lambda,
                bc7_rdo_lookback_blocks,
            )?;
            let land_output_format = resolve_land_output_format(
                TextureOutputFormatArgs {
                    raw: land_raw || shared_output_format.raw,
                    jxl: land_jxl || shared_output_format.jxl,
                    jxl_level: land_jxl_level.or(shared_output_format.jxl_level),
                    zstd: land_zstd_level(land_zstd.or(zstd)),
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
                    compression: art_output_format.compression,
                    upscale: upscale_filter,
                    upscale_passes: convert_upscale_passes(art_upscale_passes),
                    upscale_profile: upscale_profile.clone(),
                    pixel_format: art_output_format.pixel_format,
                    bc7_rdo_lambda: art_output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: art_output_format.bc7_rdo_lookback_blocks,
                },
            )?;
            info!(
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
                Some(shared_sources.texture_uop_path.as_path()),
                shared_sources.terrain_definition(),
                Some(&shared_sources.world_textures),
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
                    transcode_kdl_path: Some(land_transcode_kdl),
                    terrain_overrides_kdl_path: Some(land_terrain_overrides_kdl),
                },
            )?;
            info!(
                "TerrainDefinition raw texture packing: {} entries, {} alias refs, {} unique alias slots, {} source textures, {} selected textures, {} unique packed textures.",
                land_summary.terrain_entry_count,
                land_summary.terrain_alias_ref_count,
                land_summary.unique_alias_slot_count,
                land_summary.unique_source_texture_count,
                land_summary.unique_texture_selection_count,
                land_summary.unique_packed_texture_count,
            );
            info!(
                "Embedded {} reviewed terrain override entries from '{}'.",
                land_summary.terrain_override_entry_count,
                land_summary.terrain_override_source.display()
            );
            info!(
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
                build_tilemeta_uddp_from_split_sources_with_loaded_ec_sources(
                    ccdir,
                    &shared_sources,
                    &art_summary.ec_crop_adjustments,
                    tilemeta_out_file,
                    &TileMetaBuildOptions {
                        use_ec_radarcol: tilemeta_use_ec_radarcol,
                        classic_patches: classic_patches.into(),
                        package_compression: CompressionFlag::ZstdNoDict,
                    },
                )?;
                info!(
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
            let out_file = resolve_output_path(&output);
            audit_ec_material_refs(&paths, &out_file)?;
        }
        Commands::InventoryEcSupportTextures {
            source_dirs: source_dir_args,
            terrain_output,
            effect_output,
            summary_output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let terrain_out_file = resolve_output_path(&terrain_output);
            let effect_out_file = resolve_output_path(&effect_output);
            let summary_out_file = resolve_output_path(&summary_output);
            inventory_ec_support_textures(&paths, &terrain_out_file, &effect_out_file, &summary_out_file)?;
        }
        Commands::ReportEcMaterialBaseline {
            source_dirs: source_dir_args,
            output,
        } => {
            collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_ec_material_baseline_report(&out_file)?;
        }
        Commands::AuditEcTerrainPrimarySelection {
            source_dirs: source_dir_args,
            terrain_overrides,
            tex_land_ec,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let terrain_overrides = bundled_files::resolve(terrain_overrides.as_deref(), EC_TERRAIN_OVERRIDES_KDL)?;
            audit_ec_terrain_primary_selection(
                &paths,
                Some(&terrain_overrides),
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
            let out_file = resolve_output_path(&output);
            let terrain_overrides = bundled_files::resolve(terrain_overrides.as_deref(), EC_TERRAIN_OVERRIDES_KDL)?;
            write_ec_terrain_practical_review(
                &paths,
                Some(&terrain_overrides),
                tex_land_ec.as_deref(),
                &out_file,
            )?;
        }
        Commands::AuditEcTerrainOverrides {
            source_dirs: source_dir_args,
            terrain_overrides,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let terrain_overrides = bundled_files::resolve(terrain_overrides.as_deref(), EC_TERRAIN_OVERRIDES_KDL)?;
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
            let out_file = resolve_output_path(&output);
            let surface_overrides = bundled_files::resolve(surface_overrides.as_deref(), EC_SURFACE_OVERRIDES_KDL)?;
            audit_ec_surface_redirection(
                &paths,
                &tilemeta,
                tex_art_ec.as_deref(),
                &tex_land_ec,
                Some(&surface_overrides),
                &out_file,
            )?;
        }
        Commands::PackTerrainMeta {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let terrain_definition_path =
                udd_conv::source_paths::find_first_existing_file(&paths, &["TerrainDefinition.uop"])
                    .ok_or_else(|| eyre::eyre!("missing required file: TerrainDefinition.uop"))?;
            let terrain_definition =
                uocf::enhanced::terrain_definition::TerrainDefinitionPackage::load(
                    &terrain_definition_path,
                )
                .map_err(|e| eyre::eyre!("load TerrainDefinition.uop: {e}"))?;
            write_terrain_meta(&terrain_definition, &out_file)?;
        }
        Commands::WriteEcEffectMetaManifest {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_ec_effect_meta_manifest(&paths, &out_file)?;
        }
        Commands::WriteEcAbnormalityReport {
            source_dirs: source_dir_args,
            tilemeta,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_ec_abnormality_report(&paths, &tilemeta, &out_file)?;
        }
        Commands::WriteTileartOwnerAudit {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_tileart_owner_audit(&paths, &out_file)?;
        }
        Commands::WriteTerrainDefinitionOwnerAudit {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_terrain_definition_owner_audit(&paths, &out_file)?;
        }
        Commands::WriteTerrainDefinitionReview {
            source_dirs: source_dir_args,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_terrain_definition_review(&paths, &out_file)?;
        }
        Commands::WriteTileartLiquidSurfaceReport {
            source_dirs: source_dir_args,
            tilemeta,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            write_tileart_liquid_surface_report(&paths, tilemeta.as_deref(), &out_file)?;
        }
        Commands::PackEcTexAux {
            source_dirs: source_dir_args,
            tilemeta,
            output,
            raw,
            jxl,
            jxl_level,
            zstd,
            bc7,
            bc7_rdo,
            bc7_rdo_lambda,
            bc7_rdo_lookback_blocks,
            upscale_passes,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let output_format = resolve_art_output_format(
                raw, jxl, jxl_level, zstd, bc7, bc7_rdo, bc7_rdo_lambda, bc7_rdo_lookback_blocks,
            )?;
            let summary = pack_tex_aux_ec(
                &paths,
                &tilemeta,
                &out_file,
                &udd_conv::tex_aux_ec::TexAuxEcPackOptions {
                    compression: output_format.compression,
                    pixel_format: output_format.pixel_format,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
                    bc7_rdo_lookback_blocks: output_format.bc7_rdo_lookback_blocks,
                    upscale: UpscaleFilter::None,
                    upscale_passes: convert_upscale_passes(upscale_passes),
                },
            )?;
            info!(
                "tex_aux_ec: packed {} auxiliary textures plus the missing-texture fallback ({} refs considered, {} primary excluded, {} unread-family (legacy) excluded, {} no-path excluded, {} source-absent ids aliased to the missing texture, {} decode errors, {} duplicates) to '{}'.",
                summary.included,
                summary.refs_considered,
                summary.excluded_primary,
                summary.excluded_unread_family,
                summary.excluded_no_path,
                summary.excluded_source_absent,
                summary.excluded_decode_error,
                summary.excluded_duplicate_id,
                out_file.display()
            );
        }
        Commands::PackEcEffects { source_dirs: source_dir_args, output, bc7, zstd, report, strict } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let summary = pack_effects_ec(
                &paths,
                &out_file,
                &EffectsEcPackOptions {
                    bc7,
                    compression: zstd.map(zstd_compression).unwrap_or(CompressionFlag::ZstdNoDict),
                    strict,
                    report,
                },
            )?;
            info!("effects_ec: {summary}; wrote '{}'.", out_file.display());
        }
        Commands::PackTilemeta {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            zstd,
            use_ec_radarcol,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let options = TileMetaBuildOptions {
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
            info!("Wrote tilemeta.uddp to '{}'.", out_file.display());
        }
        Commands::PackMap {
            source_dirs: source_dir_args,
            classic_patches,
            map_edition,
            map_id,
            output,
            uop,
            zstd,
            stored,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("map{}.uddp", map_id));
            let out_file = resolve_output_path(output.as_ref().unwrap_or(&default_output));
            let summary = convert_map_mul_to_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                map_id,
                if uop {
                    SourceFormatPreference::Uop
                } else {
                    SourceFormatPreference::Mul
                },
                &facet_patch_options(classic_patches, map_edition),
                ChunkCompression::from_cli(stored, zstd),
            )?;
            info!(
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
            map_edition,
            map_id,
            output,
            zstd,
            stored,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let out_file = resolve_output_path(output.as_ref().unwrap_or(&default_output));
            let summary = convert_statics_mul_to_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                map_id,
                &facet_patch_options(classic_patches, map_edition),
                ChunkCompression::from_cli(stored, zstd),
            )?;
            info!(
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
            map_edition,
            map_id,
            map_output,
            statics_output,
            uop,
            zstd,
            stored,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_map_output = PathBuf::from(format!("map{}.uddp", map_id));
            let default_statics_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let map_out_file =
                resolve_output_path(map_output.as_ref().unwrap_or(&default_map_output));
            let statics_out_file = resolve_output_path(
                statics_output.as_ref().unwrap_or(&default_statics_output),
            );
            let patch_options = facet_patch_options(classic_patches, map_edition);
            let chunk_compression = ChunkCompression::from_cli(stored, zstd);
            let map_summary = convert_map_mul_to_uddp_from_sources_with_patches(
                &paths,
                &map_out_file,
                map_id,
                if uop {
                    SourceFormatPreference::Uop
                } else {
                    SourceFormatPreference::Mul
                },
                &patch_options,
                chunk_compression,
            )?;
            info!(
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
                    chunk_compression,
                )?;
            info!(
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
            map_edition,
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
                resolve_output_path(output_path_to_use)
            };

            let uddp_dir = uddp_dir.unwrap_or_else(|| PathBuf::from("."));
            let tilemeta_path = find_raw_tilemeta_package(&uddp_dir)?;
            let map_source_preference = if uop {
                SourceFormatPreference::Uop
            } else {
                SourceFormatPreference::Mul
            };

            if radar_format == udd_conv::cc_radar::RadarFormat::Bc7Ktx2 {
                let bc7_data = udd_conv::cc_radar::build_facet_radar_bc7_with_options(
                    &paths,
                    &tilemeta_path,
                    map_id,
                    map_source_preference,
                    &facet_patch_options(classic_patches, map_edition),
                )?;
                udd_image_codecs::ktx2::write_ktx2_bc7_zstd(bc7_data, &out_file, zstd_level)?;
                info!("Successfully created facet0{}.ktx2 (BC7 + Zstd)", map_id);
            } else {
                udd_conv::cc_radar::build_facet_radar_dds(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    &udd_conv::cc_radar::RadarBuildOptions {
                        format: radar_format,
                        zstd_level,
                        classic_patches: facet_patch_options(classic_patches, map_edition),
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
            collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
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
            info!("Wrote world_lights.uddp to '{}'.", out_file.display());
        }
        Commands::PackHues {
            source_dirs: source_dir_args,
            output,
            hue_ramp,
            no_compression,
            zstd,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
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
                &HuesOptions { compression, ramp: hue_ramp.into() },
            )?;
            info!("Wrote hues.uddp to '{}'.", out_file.display());
        }
        Commands::PackGumps {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            raw: _,
            bc1,
            jxl,
            jxl_level,
            zstd,
            uop,
            paperdoll_upscale_passes,
            single_upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let single_pixel_format = single_gump_pixel_format(bc1, CC_LOSSY_SINGLE_GUMP_FORMAT);
            let summary = convert_gumps_to_uddp_from_sources_with_patches_and_options(
                &paths,
                &out_file,
                &classic_patches.into(),
                &CcGumpsOptions {
                    single_pixel_format,
                    // Single gumps carry a size header, so they can't go through the image codec.
                    compression: single_gump_compression(zstd, single_pixel_format),
                    atlas_compression: gump_atlas_compression(jxl, jxl_level, zstd),
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
            info!(
                "Wrote {} Classic gumps to '{}' ({} single in {}, {} with soft alpha; {} atlas).",
                summary.gump_count,
                out_file.display(),
                summary.single_gump_count,
                single_pixel_format.label(),
                summary.single_soft_alpha_count,
                summary.atlas_gump_count
            );
        }
        Commands::PackEcGumps {
            source_dirs: source_dir_args,
            output,
            max_id,
            raw: _,
            bc7,
            jxl,
            jxl_level,
            zstd,
            paperdoll_upscale_passes,
            single_upscale_passes,
            upscale_profile,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&output);
            let upscale_profile = load_optional_upscale_profile(upscale_profile)?;
            let single_pixel_format = single_gump_pixel_format(bc7, EC_LOSSY_SINGLE_GUMP_FORMAT);
            let summary = convert_ec_gumps_to_uddp_from_sources(
                &paths,
                &out_file,
                &EcGumpsOptions {
                    max_id,
                    single_pixel_format,
                    // Single gumps carry a size header, so they can't go through the image codec.
                    compression: single_gump_compression(zstd, single_pixel_format),
                    atlas_compression: gump_atlas_compression(jxl, jxl_level, zstd),
                    paperdoll_upscale_passes: convert_upscale_passes(paperdoll_upscale_passes),
                    single_upscale_passes: convert_upscale_passes(single_upscale_passes),
                    upscale_profile,
                },
            )?;
            info!(
                "Wrote {} EC gumps to '{}' ({} single in {}, {} with soft alpha; {} atlas; {} skipped).",
                summary.gump_count,
                out_file.display(),
                summary.single_gump_count,
                single_pixel_format.label(),
                summary.single_soft_alpha_count,
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
                use_ec_radarcol,
                ..
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert!(!classic_patches.use_verdata);
                assert!(!classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
                assert_eq!(output, PathBuf::from("tilemeta.uddp"));
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
            "dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl",
            "--bc7",
        ])
        .expect("parse unified ec texture args");

        match cli.command {
            Commands::PackEcTextures {
                source_dirs,
                art_output,
                land_output,
                land_transcode_kdl,
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
                        "dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl"
                    ))
                );
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
            "--use-verdata",
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
                assert!(classic_patches.use_verdata);
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

    /// The art format `pack-art`, `pack-ec-textures` and `pack-ec-tex-aux` resolve, as their command arms do.
    fn parsed_art_output_format(args: &[&str]) -> TextureOutputFormat {
        let format = match Cli::try_parse_from(args).expect("parse art args").command {
            Commands::PackArt { raw, jxl, jxl_level, zstd, bc7, bc7_rdo, .. }
            | Commands::PackEcTexAux { raw, jxl, jxl_level, zstd, bc7, bc7_rdo, .. } => resolve_art_output_format(
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
                NO_BC7_RDO_LAMBDA,
                udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            ),
            Commands::PackEcTextures {
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
                ..
            } => resolve_art_output_format(
                art_raw || raw,
                art_jxl || jxl,
                art_jxl_level.or(jxl_level),
                art_zstd.or(zstd),
                art_bc7 || bc7,
                art_bc7_rdo || bc7_rdo,
                NO_BC7_RDO_LAMBDA,
                udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
            ),
            _ => panic!("unexpected command parsed"),
        };
        format.expect("resolve art output format")
    }

    /// Without a format flag the art and auxiliary texture pages are lossless RGBA8888 with Zstd 19 (the user,
    /// 2026-11-06), and a bare `--zstd` keeps that level; `--jxl` still writes JPEG XL, with a level 7 pass (`JXZ1`) under
    /// a bare `--zstd`.
    #[test]
    fn art_and_aux_pages_default_to_lossless_zstd_19() {
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;
        const EXPLICIT_JXL_LEVEL: u8 = 4;
        let lossless_zstd = CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL);
        for (command, flag_prefix) in [
            (["uddpack", "pack-art", "--ccdir", "/cc"].as_slice(), ""),
            (["uddpack", "pack-ec-tex-aux", "--ecdir", "/ec", "--tilemeta", "tilemeta.uddp"].as_slice(), ""),
            (["uddpack", "pack-ec-textures", "--ecdir", "/ec"].as_slice(), ""),
            (["uddpack", "pack-ec-textures", "--ecdir", "/ec"].as_slice(), "art-"),
        ] {
            let format = |flags: &[&str]| {
                let flags = flags.iter().map(|flag| format!("--{flag_prefix}{flag}")).collect::<Vec<_>>();
                let args = command.iter().copied().chain(flags.iter().map(String::as_str)).collect::<Vec<_>>();
                parsed_art_output_format(&args)
            };
            let default = format(&[]);
            assert_eq!(default.pixel_format, PagePixelFormat::Rgba8888, "{command:?} {flag_prefix}");
            assert_eq!(default.compression, lossless_zstd, "{command:?} {flag_prefix}");
            assert_eq!(format(&["zstd"]).compression, lossless_zstd, "{command:?} {flag_prefix}");
            assert_eq!(
                format(&["zstd=5"]).compression,
                CompressionFlag::ZstdNoDictLevel(EXPLICIT_ZSTD_LEVEL),
                "{command:?} {flag_prefix}"
            );
            let jxl = format(&["jxl"]);
            assert_eq!(jxl.pixel_format, PagePixelFormat::Rgba8888, "{command:?} {flag_prefix}");
            assert_eq!(jxl.compression, CompressionFlag::JpegXlLevel(DEFAULT_JXL_LEVEL), "{command:?} {flag_prefix}");
            assert_eq!(
                format(&["jxl", "jxl-level=4"]).compression,
                CompressionFlag::JpegXlLevel(EXPLICIT_JXL_LEVEL),
                "{command:?} {flag_prefix}"
            );
            assert_eq!(
                format(&["jxl", "zstd"]).compression,
                CompressionFlag::JpegXlZstdLevels { jxl_level: DEFAULT_JXL_LEVEL, zstd_level: DEFAULT_ZSTD_LEVEL },
                "{command:?} {flag_prefix}"
            );
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
        ])
        .expect("mobile animation pack commands default to BC7 output");

        match cli.command {
            Commands::PackMobileAnims {
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
    fn cli_defaults_land_output_format_to_lossless_zstd() {
        let cli = Cli::try_parse_from(["uddpack", "pack-texmaps", "--ccdir", "/cc"])
            .expect("parse default land texture args");

        match cli.command {
            Commands::PackTexmaps {
                raw,
                jxl,
                jxl_level,
                zstd,
                bc7,
                bc7_rdo,
                ..
            } => {
                refuse_classic_land_bc7(bc7, bc7_rdo).unwrap();
                let output_format = resolve_land_output_format(
                    TextureOutputFormatArgs { raw, jxl, jxl_level, zstd: land_zstd_level(zstd), bc7, bc7_rdo },
                    NO_BC7_RDO_LAMBDA,
                    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS,
                )
                .unwrap();
                assert_eq!(output_format.pixel_format, PagePixelFormat::Rgba8888);
                assert_eq!(
                    output_format.compression,
                    CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL)
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    /// `pack-texmaps` refuses BC7 with the reason, and still parses the flags to say so.
    #[test]
    fn pack_texmaps_refuses_bc7_with_the_reason() {
        for flag in ["--bc7", "--bc7-rdo"] {
            let cli = Cli::try_parse_from(["uddpack", "pack-texmaps", "--ccdir", "/cc", flag]).expect("parse BC7 flag");
            let Commands::PackTexmaps { bc7, bc7_rdo, .. } = cli.command else {
                panic!("unexpected command parsed");
            };
            let error = refuse_classic_land_bc7(bc7, bc7_rdo).unwrap_err().to_string();
            assert!(error.contains(CLASSIC_LAND_LOSSY_REFUSAL), "{flag}: {error}");
        }
    }

    /// BC7 land blocks take Zstd 19 unless a level is given, and `--land-bc7` has no RDO.
    #[test]
    fn bc7_land_blocks_take_zstd_19_without_rdo() {
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;
        let format = |args: TextureOutputFormatArgs| {
            resolve_land_output_format(args, udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS)
                .unwrap()
        };
        let bc7 = format(TextureOutputFormatArgs { bc7: true, ..Default::default() });
        assert_eq!(bc7.pixel_format, PagePixelFormat::Bc7);
        assert_eq!(bc7.compression, CompressionFlag::ZstdNoDictLevel(BC7_LAND_PAGE_ZSTD_LEVEL));
        assert_eq!(bc7.bc7_rdo_lambda, NO_BC7_RDO_LAMBDA);
        assert_eq!(
            format(TextureOutputFormatArgs { bc7: true, zstd: Some(EXPLICIT_ZSTD_LEVEL), ..Default::default() }).compression,
            CompressionFlag::ZstdNoDictLevel(EXPLICIT_ZSTD_LEVEL)
        );
        assert_eq!(
            format(TextureOutputFormatArgs { bc7_rdo: true, ..Default::default() }).compression,
            CompressionFlag::ZstdNoDictLevel(BC7_LAND_PAGE_ZSTD_LEVEL)
        );
    }

    #[test]
    fn land_output_format_keeps_jxl_raw_and_zstd_levels_as_explicit_choices() {
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;
        let compression = |args: TextureOutputFormatArgs| {
            resolve_land_output_format(args, 0.0, udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS)
                .unwrap()
                .compression
        };

        assert_eq!(
            compression(TextureOutputFormatArgs { zstd: Some(EXPLICIT_ZSTD_LEVEL), ..Default::default() }),
            CompressionFlag::ZstdNoDictLevel(EXPLICIT_ZSTD_LEVEL)
        );
        assert_eq!(
            compression(TextureOutputFormatArgs { jxl: true, ..Default::default() }),
            CompressionFlag::JpegXlLevel(DEFAULT_JXL_LEVEL)
        );
        assert_eq!(
            compression(TextureOutputFormatArgs { raw: true, ..Default::default() }),
            CompressionFlag::None
        );
    }

    #[test]
    fn gump_atlas_pages_default_to_lossless_zstd() {
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;

        assert_eq!(
            gump_atlas_compression(false, None, None),
            CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL)
        );
        assert_eq!(
            gump_atlas_compression(false, None, Some(Some(EXPLICIT_ZSTD_LEVEL))),
            CompressionFlag::ZstdNoDictLevel(EXPLICIT_ZSTD_LEVEL)
        );
        assert_eq!(
            gump_atlas_compression(true, None, None),
            CompressionFlag::JpegXlLevel(DEFAULT_JXL_LEVEL)
        );
    }

    /// A bare `--zstd` keeps each package's own levels, as for map chunks: gump atlas pages and land pages 19, single
    /// gumps the package default; only a pass over JPEG XL pages takes the plain default.
    /// `--bc1` stores Classic single gumps as BC1 and `--bc7` Enhanced ones as BC7, each with Zstd 19 unless a level is
    /// given; `--raw` is the lossless default, and neither command takes the other's lossy flag.
    #[test]
    fn lossy_flags_select_each_package_lossy_single_gumps() {
        use udd_conv::single_gumps::{BC1_SINGLE_GUMP_ZSTD_LEVEL, BC7_SINGLE_GUMP_ZSTD_LEVEL};
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;
        let parse = |args: &[&str]| match Cli::try_parse_from(args).expect("parse gump args").command {
            Commands::PackGumps { bc1, zstd, .. } => {
                let format = single_gump_pixel_format(bc1, CC_LOSSY_SINGLE_GUMP_FORMAT);
                (format, single_gump_compression(zstd, format))
            }
            Commands::PackEcGumps { bc7, zstd, .. } => {
                let format = single_gump_pixel_format(bc7, EC_LOSSY_SINGLE_GUMP_FORMAT);
                (format, single_gump_compression(zstd, format))
            }
            _ => panic!("unexpected command parsed"),
        };
        for (command, flag, other_flag, format, level) in [
            ("pack-gumps", "--bc1", "--bc7", GumpPixelFormat::Bc1, BC1_SINGLE_GUMP_ZSTD_LEVEL),
            ("pack-ec-gumps", "--bc7", "--bc1", GumpPixelFormat::Bc7, BC7_SINGLE_GUMP_ZSTD_LEVEL),
        ] {
            assert_eq!(
                parse(&["uddpack", command]),
                (GumpPixelFormat::Rgba8888, CompressionFlag::ZstdNoDict)
            );
            assert_eq!(parse(&["uddpack", command, flag]), (format, CompressionFlag::ZstdNoDictLevel(level)));
            assert_eq!(
                parse(&["uddpack", command, flag, "--zstd"]),
                parse(&["uddpack", command, flag])
            );
            assert_eq!(
                parse(&["uddpack", command, flag, "--zstd=5"]),
                (format, CompressionFlag::ZstdNoDictLevel(EXPLICIT_ZSTD_LEVEL))
            );
            assert!(Cli::try_parse_from(["uddpack", command, flag, "--raw"]).is_err());
            assert!(Cli::try_parse_from(["uddpack", command, other_flag]).is_err());
        }
    }

    #[test]
    fn bare_zstd_keeps_the_gump_and_land_page_levels() {
        const EXPLICIT_ZSTD_LEVEL: i32 = 5;
        let gump_zstd = |args: &[&str]| match Cli::try_parse_from(args).expect("parse gump args").command {
            Commands::PackGumps { zstd, .. } | Commands::PackEcGumps { zstd, .. } => zstd,
            _ => panic!("unexpected command parsed"),
        };
        for command in ["pack-gumps", "pack-ec-gumps"] {
            let zstd = gump_zstd(&["uddpack", command, "--zstd"]);
            assert_eq!(zstd, Some(None));
            assert_eq!(
                gump_atlas_compression(false, None, zstd),
                CompressionFlag::ZstdNoDictLevel(LOSSLESS_PAGE_ZSTD_LEVEL)
            );
            assert_eq!(
                single_gump_compression(zstd, GumpPixelFormat::Rgba8888),
                single_gump_compression(None, GumpPixelFormat::Rgba8888)
            );
            assert_eq!(
                gump_atlas_compression(true, None, zstd),
                CompressionFlag::JpegXlZstdLevels { jxl_level: DEFAULT_JXL_LEVEL, zstd_level: DEFAULT_ZSTD_LEVEL }
            );
        }

        let land_zstd = |args: &[&str]| -> Option<i32> {
            match Cli::try_parse_from(args).expect("parse land args").command {
                Commands::PackTexmaps { zstd, .. } => land_zstd_level(zstd),
                Commands::PackEcTextures { zstd, land_zstd, .. } => land_zstd_level(land_zstd.or(zstd)),
                _ => panic!("unexpected command parsed"),
            }
        };
        assert_eq!(land_zstd(&["uddpack", "pack-texmaps", "--zstd"]), Some(LOSSLESS_PAGE_ZSTD_LEVEL));
        assert_eq!(land_zstd(&["uddpack", "pack-ec-textures", "--zstd"]), Some(LOSSLESS_PAGE_ZSTD_LEVEL));
        assert_eq!(land_zstd(&["uddpack", "pack-ec-textures", "--land-zstd"]), Some(LOSSLESS_PAGE_ZSTD_LEVEL));
        assert_eq!(
            land_zstd(&["uddpack", "pack-ec-textures", "--zstd", "--land-zstd=5"]),
            Some(EXPLICIT_ZSTD_LEVEL)
        );
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
                let output_format = resolve_art_output_format(
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
                let output_format = resolve_art_output_format(
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
                let output_format = resolve_art_output_format(
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
                assert_eq!(zstd, Some(None));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_art_output_format(
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
                assert_eq!(zstd, Some(None));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_art_output_format(
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
                assert_eq!(zstd, Some(Some(5)));
                assert!(!bc7);
                assert!(!bc7_rdo);
                let output_format = resolve_art_output_format(
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
                assert_eq!(zstd, Some(None));
                assert!(!bc7);
                assert!(bc7_rdo);
                let output_format = resolve_art_output_format(
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
            "dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl",
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
                    PathBuf::from("dynamapper/assets/cc_ec_convtables/TerrainTranscode.kdl")
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
            Commands::PackHues { source_dirs, output, hue_ramp, .. } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(output, PathBuf::from("hues.uddp"));
                assert_eq!(hue_ramp, CliHueRamp::Linear);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_hues_flat_ramp() {
        let cli = Cli::try_parse_from(["uddpack", "pack-hues", "--ccdir", "/cc", "--hue-ramp", "flat"])
            .expect("parse hues args");

        match cli.command {
            Commands::PackHues { hue_ramp, .. } => {
                assert_eq!(HueRampMode::from(hue_ramp), HueRampMode::Flat);
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
        ])
        .expect("parse ec mobile animation metadata paths");

        match cli.command {
            Commands::PackEcMobileAnims {
                tables,
                ec_mobile_animations_kdl,
                ..
            } => {
                assert_eq!(tables, Some(PathBuf::from("/tables")));
                assert_eq!(
                    ec_mobile_animations_kdl,
                    Some(PathBuf::from("/tables/EcMobileAnimations.kdl"))
                );
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_parses_pack_ec_effects() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-ec-effects",
            "--ecdir",
            "/ec",
            "--bc7",
            "--zstd=12",
            "--report",
            "/out/effects_report.txt",
            "--strict",
        ])
        .expect("parse pack-ec-effects");

        match cli.command {
            Commands::PackEcEffects { source_dirs, output, bc7, zstd, report, strict } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("effects_ec.uddp"));
                assert!(bc7 && strict);
                assert_eq!(zstd, Some(12));
                assert_eq!(report, Some(PathBuf::from("/out/effects_report.txt")));
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn classic_patch_args_convert_to_shared_options() {
        let options: ClassicPatchOptions = ClassicPatchArgs {
            use_verdata: true,
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
                map_edition: MapEdition::Standard,
            }
        );
    }

    #[test]
    fn cli_parses_the_map_edition_of_map_statics_and_radar_commands() {
        let parse = |command: &str, edition: Option<&str>| {
            let mut args = vec!["uddpack", command, "--ccdir", "/cc", "--map-id", "1", "--include-static-difs"];
            if let Some(edition) = edition {
                args.extend(["--map-edition", edition]);
            }
            Cli::try_parse_from(args).expect("parse map edition args").command
        };
        for command in ["pack-map", "pack-statics", "pack-map-statics", "pack-radar"] {
            for (edition, expected) in [(None, CliMapEdition::Standard), (Some("variant"), CliMapEdition::Variant)] {
                let (classic_patches, map_edition) = match parse(command, edition) {
                    Commands::PackMap { classic_patches, map_edition, .. }
                    | Commands::PackStatics { classic_patches, map_edition, .. }
                    | Commands::PackMapStatics { classic_patches, map_edition, .. }
                    | Commands::PackRadar { classic_patches, map_edition, .. } => (classic_patches, map_edition),
                    _ => panic!("unexpected command parsed"),
                };
                assert_eq!(map_edition, expected, "{command}");
                // The edition and the difs are independent choices.
                let options = facet_patch_options(classic_patches, map_edition);
                assert_eq!(options.map_edition, MapEdition::from(expected), "{command}");
                assert!(options.static_difs && !options.map_difs, "{command}");
            }
        }
        assert!(Cli::try_parse_from(["uddpack", "pack-art", "--ccdir", "/cc", "--map-edition", "variant"]).is_err());
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
            "--use-verdata",
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
                assert!(classic_patches.use_verdata);
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
                assert!(!classic_patches.use_verdata);
                assert!(classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn map_and_statics_chunks_are_compressed_unless_stored_is_asked() {
        const ASKED_LEVEL: i32 = 3;
        let level_arg = format!("--zstd={ASKED_LEVEL}");
        for command in ["pack-map", "pack-statics", "pack-map-statics"] {
            for (extra, expected) in [
                (vec![], ChunkCompression::PackageDefault),
                (vec!["--zstd"], ChunkCompression::PackageDefault),
                (vec![level_arg.as_str()], ChunkCompression::Zstd(ASKED_LEVEL)),
                (vec!["--stored"], ChunkCompression::Stored),
            ] {
                let mut args = vec!["uddpack", command, "--ccdir", "/cc", "--map-id", "0"];
                args.extend(extra.iter().copied());
                let (zstd, stored) = match Cli::try_parse_from(args).expect("parse map args").command {
                    Commands::PackMap { zstd, stored, .. }
                    | Commands::PackStatics { zstd, stored, .. }
                    | Commands::PackMapStatics { zstd, stored, .. } => (zstd, stored),
                    _ => panic!("unexpected command parsed"),
                };
                assert_eq!(ChunkCompression::from_cli(stored, zstd), expected, "{command} {extra:?}");
            }
            let both = ["uddpack", command, "--ccdir", "/cc", "--map-id", "0", "--stored", "--zstd"];
            assert!(Cli::try_parse_from(both).is_err(), "{command}: --stored conflicts with --zstd");
        }
    }

    #[test]
    fn verdata_is_off_by_default_and_has_no_old_name() {
        for command in ["pack-map", "pack-statics", "pack-map-statics"] {
            let cli = Cli::try_parse_from(["uddpack", command, "--ccdir", "/cc", "--map-id", "0"])
                .expect("parse map args");
            let classic_patches = match cli.command {
                Commands::PackMap { classic_patches, .. }
                | Commands::PackStatics { classic_patches, .. }
                | Commands::PackMapStatics { classic_patches, .. } => classic_patches,
                _ => panic!("unexpected command parsed"),
            };
            assert!(!classic_patches.use_verdata, "{command}");
            assert!(
                Cli::try_parse_from(["uddpack", command, "--ccdir", "/cc", "--map-id", "0", "--include-verdata"])
                    .is_err(),
                "{command}"
            );
        }
    }

    #[test]
    fn cli_parses_pack_lights_verdata_without_diff_flags() {
        let cli = Cli::try_parse_from([
            "uddpack",
            "pack-lights",
            "--ccdir",
            "/cc",
            "--use-verdata",
        ])
        .expect("parse lights patch args");

        match cli.command {
            Commands::PackLights {
                classic_patches, ..
            } => {
                assert!(classic_patches.use_verdata);
                assert!(!classic_patches.include_map_difs);
                assert!(!classic_patches.include_static_difs);
            }
            _ => panic!("unexpected command parsed"),
        }
    }
}
