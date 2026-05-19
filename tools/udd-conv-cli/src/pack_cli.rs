use std::path::{Path, PathBuf};

use crate::ec_material_audit::{
    audit_ec_material_refs, audit_ec_surface_redirection, audit_ec_terrain_primary_selection,
    inventory_ec_support_textures, write_ec_material_baseline_report,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre;
use udd_conv::{
    cc_map::{convert_map_mul_to_uddp_from_sources, CcMapSourcePreference},
    cc_statics::convert_statics_mul_to_uddp_from_sources,
    source_paths::{gather_source_dirs, resolve_output_path},
    tex_art_cc::{
        convert_art_mul_to_tex_art_cc_uddp_from_sources, TexArtCcAtlasOptions,
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
        convert_texmaps_mul_to_tex_land_cc_uddp, TexLandCcAtlasOptions,
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
        TileMetaBuildOptions,
    },
    upscale::UpscaleFilter,
    world_lights::{convert_client_lights_to_world_lights_uddp, WorldLightsOptions},
    AtlasPackingMode, CompressionFlag, PagePixelFormat,
};

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

fn collect_source_dirs(args: &SourceDirArgs) -> eyre::Result<Vec<PathBuf>> {
    let dirs = gather_source_dirs(args.ccdir.as_ref(), args.ecdir.as_ref());
    if dirs.is_empty() {
        eyre::bail!("at least one source root must be provided via --ccdir or --ecdir");
    }
    Ok(dirs)
}

fn collect_ec_source_dirs(args: &SourceDirArgs) -> eyre::Result<Vec<PathBuf>> {
    args.ecdir
        .as_ref()
        .map(|path| vec![path.clone()])
        .ok_or_else(|| eyre::eyre!("--ecdir is required for EC source files"))
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

#[derive(clap::ValueEnum, Clone, Copy, Debug, Default)]
pub enum CliUpscaleFilter {
    #[default]
    None,
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
    Depixelize2x,
    Depixelize3x,
    Depixelize4x,
    Nedi2x,
    TwoSai2x,
    SuperEagle2x,
    Hq2x,
    Hq3x,
    Hq4x,
    Epx2x,
    Epx3x,
    Epx4x,
    Xbr2x,
    Xbr3x,
    Xbr4x,
}

#[derive(ValueEnum, Clone, Copy, Debug, Default)]
pub enum CliAtlasPackingMode {
    #[default]
    MaximumPacking,
    Bc7Oriented,
}

impl From<CliAtlasPackingMode> for AtlasPackingMode {
    fn from(value: CliAtlasPackingMode) -> Self {
        match value {
            CliAtlasPackingMode::MaximumPacking => AtlasPackingMode::MaximumPacking,
            CliAtlasPackingMode::Bc7Oriented => AtlasPackingMode::Bc7Oriented,
        }
    }
}

impl From<CliUpscaleFilter> for UpscaleFilter {
    fn from(val: CliUpscaleFilter) -> Self {
        match val {
            CliUpscaleFilter::None => UpscaleFilter::None,
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
            CliUpscaleFilter::Depixelize2x => UpscaleFilter::Depixelize2x,
            CliUpscaleFilter::Depixelize3x => UpscaleFilter::Depixelize3x,
            CliUpscaleFilter::Depixelize4x => UpscaleFilter::Depixelize4x,
            CliUpscaleFilter::Nedi2x => UpscaleFilter::Nedi2x,
            CliUpscaleFilter::TwoSai2x => UpscaleFilter::TwoSai2x,
            CliUpscaleFilter::SuperEagle2x => UpscaleFilter::SuperEagle2x,
            CliUpscaleFilter::Hq2x => UpscaleFilter::Hq2x,
            CliUpscaleFilter::Hq3x => UpscaleFilter::Hq3x,
            CliUpscaleFilter::Hq4x => UpscaleFilter::Hq4x,
            CliUpscaleFilter::Epx2x => UpscaleFilter::Epx2x,
            CliUpscaleFilter::Epx3x => UpscaleFilter::Epx3x,
            CliUpscaleFilter::Epx4x => UpscaleFilter::Epx4x,
            CliUpscaleFilter::Xbr2x => UpscaleFilter::Xbr2x,
            CliUpscaleFilter::Xbr3x => UpscaleFilter::Xbr3x,
            CliUpscaleFilter::Xbr4x => UpscaleFilter::Xbr4x,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Packs classic art.mul/artidx.mul into tex_art_cc.uddp atlas pages.
    PackArt {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tex_art_cc.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(
            long,
            default_value_t = false,
            help = "Use BC7 compression (VRAM optimization)."
        )]
        bc7: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Atlas placement policy.")]
        packing_mode: CliAtlasPackingMode,
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
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale: CliUpscaleFilter,
    },
    /// Packs Classic texmaps.mul into tex_land_cc.uddp atlas pages.
    PackTexmaps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tex_land_cc.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_PAGE_WIDTH)]
        atlas_width: u32,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_PAGE_HEIGHT)]
        atlas_height: u32,
        #[arg(long, default_value_t = CC_TEXMAPS_DEFAULT_ATLAS_GUTTER)]
        gutter: u16,
        #[arg(
            long,
            default_value_t = false,
            help = "Use BC7 compression (VRAM optimization)."
        )]
        bc7: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Atlas placement policy.")]
        packing_mode: CliAtlasPackingMode,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale: CliUpscaleFilter,
    },
    /// Packs EC art and land in one shared source pass into tex_art_ec.uddp and tex_land_ec.uddp.
    PackEcTextures {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tex_art_ec.uddp")]
        art_output: PathBuf,
        #[arg(long, default_value = "tex_land_ec.uddp")]
        land_output: PathBuf,
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
        #[arg(
            long,
            default_value_t = false,
            help = "Use BC7 compression for land (VRAM optimization)."
        )]
        land_bc7: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Art atlas placement policy.")]
        art_packing_mode: CliAtlasPackingMode,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Land atlas placement policy.")]
        land_packing_mode: CliAtlasPackingMode,
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
        #[arg(long, default_value = "ec_terrain_primary_selection.json")]
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
        #[arg(long, default_value = "ec_surface_redirection.json")]
        output: PathBuf,
    },
    /// Packs CC tiledata and EC tileart into tilemeta.uddp.
    #[command(name = "pack-tilemeta")]
    PackTilemeta {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tilemeta.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false)]
        ec_art_cropped: bool,
        #[arg(long, default_value_t = false)]
        use_ec_radarcol: bool,
    },
    /// Generates a radar map (facet0X.dds) from Classic map and statics.
    PackRadar {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
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
        /// Zstd compression level for KTX2 (1-22)
        #[arg(long, default_value_t = 3)]
        zstd_level: i32,
    },
    /// Packs Classic mapX.mul into mapX.uddp blocks.
    PackMap {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        uop: bool,
    },
    /// Packs Classic staticsX.mul into staticsX.uddp blocks.
    PackStatics {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long)]
        map_id: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Packs CC and EC lighting textures into world_lights.uddp.
    PackLights {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "world_lights.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false, help = "Do not use compression.")]
        no_compression: bool,
    },
}

pub fn run() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    match Cli::parse().command {
        Commands::PackArt {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
            packing_mode,
            upscale_64_size: _, // Land upscaling not currently applied to CC Art
            upscale_64_algo: _,
            upscale_128_size: _,
            upscale_128_algo: _,
            upscale_256_size: _,
            upscale_256_algo: _,
            upscale,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if bc7 {
                CompressionFlag::None
            } else {
                CompressionFlag::ZstdNoDict
            };
            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources(
                &paths,
                &out_file,
                &TexArtCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression,
                    upscale: upscale.into(),
                    pixel_format: if bc7 {
                        PagePixelFormat::Bc7
                    } else {
                        PagePixelFormat::Rgba8888
                    },
                    packing_mode: packing_mode.into(),
                },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackTexmaps {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
            bc7,
            packing_mode,
            upscale,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if bc7 {
                CompressionFlag::None
            } else {
                CompressionFlag::ZstdNoDict
            };
            let summary = convert_texmaps_mul_to_tex_land_cc_uddp(
                &paths[0], // Use the first source dir (usually ccdir)
                &out_file,
                &TexLandCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression,
                    upscale_64: udd_conv::upscale::UpscaleConfig {
                        target_size: 256,
                        filter: upscale.into(),
                    },
                    upscale_128: udd_conv::upscale::UpscaleConfig {
                        target_size: 256,
                        filter: upscale.into(),
                    },
                    pixel_format: if bc7 {
                        PagePixelFormat::Bc7
                    } else {
                        PagePixelFormat::Rgba8888
                    },
                    packing_mode: packing_mode.into(),
                },
            )?;
            println!(
                "Wrote {} pages ({}) for {} populated slots out of {} total slots to '{}'.",
                summary.page_count,
                if bc7 { "BC7" } else { "RGBA8888" },
                summary.populated_slot_count,
                summary.slot_count,
                out_file.display()
            );
        }
        Commands::PackEcTextures {
            source_dirs: source_dir_args,
            art_output,
            land_output,
            art_atlas_width,
            art_atlas_height,
            art_gutter,
            land_atlas_width,
            land_atlas_height,
            land_gutter,
            land_bc7,
            art_packing_mode,
            land_packing_mode,
            upscale_64_size,
            upscale_64_algo,
            upscale_128_size,
            upscale_128_algo,
            upscale_256_size,
            upscale_256_algo,
            upscale_512_size,
            upscale_512_algo,
            upscale,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let ec_paths = collect_ec_source_dirs(&source_dir_args)?;
            let art_out_file = resolve_output_path(&ec_paths, &art_output);
            let land_out_file = resolve_output_path(&ec_paths, &land_output);
            let shared_sources = load_tex_art_ec_sources(&ec_paths)?;
            let upscale_filter = UpscaleFilter::from(upscale);
            let art_summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_loaded_sources(
                &shared_sources,
                &art_out_file,
                &TexArtEcAtlasOptions {
                    atlas_width: art_atlas_width,
                    atlas_height: art_atlas_height,
                    gutter: art_gutter,
                    crop_transparent_bounds: false,
                    compression: CompressionFlag::ZstdNoDict, // EC Art always uses Zstd here
                    upscale: upscale_filter,
                    pixel_format: PagePixelFormat::Rgba8888,
                    packing_mode: art_packing_mode.into(),
                },
            )?;
            println!(
                "Wrote {} pages (RGBA8888) for {} populated art slots out of {} total slots to '{}'.",
                art_summary.page_count,
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
                    compression: if land_bc7 {
                        CompressionFlag::None
                    } else {
                        CompressionFlag::ZstdNoDict
                    },
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
                    pixel_format: if land_bc7 {
                        PagePixelFormat::Bc7
                    } else {
                        PagePixelFormat::Rgba8888
                    },
                    packing_mode: land_packing_mode.into(),
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
            println!(
                "Wrote {} pages ({}) for {} populated land slots out of {} total slots to '{}'.",
                land_summary.page_count,
                if land_bc7 { "BC7" } else { "RGBA8888" },
                land_summary.populated_slot_count,
                land_summary.slot_count,
                land_out_file.display()
            );
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
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            audit_ec_terrain_primary_selection(&paths, &out_file)?;
        }
        Commands::AuditEcSurfaceRedirection {
            source_dirs: source_dir_args,
            tilemeta,
            tex_art_ec,
            tex_land_ec,
            output,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            audit_ec_surface_redirection(
                &paths,
                &tilemeta,
                tex_art_ec.as_deref(),
                &tex_land_ec,
                &out_file,
            )?;
        }
        Commands::PackTilemeta {
            source_dirs: source_dir_args,
            output,
            ec_art_cropped,
            use_ec_radarcol,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let options = TileMetaBuildOptions {
                adjust_tex_art_ec_sampling: ec_art_cropped,
                use_ec_radarcol,
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
            map_id,
            output,
            uop,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("map{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_map_mul_to_uddp_from_sources(
                &paths,
                &out_file,
                map_id,
                if uop {
                    CcMapSourcePreference::Uop
                } else {
                    CcMapSourcePreference::Mul
                },
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
            map_id,
            output,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let default_output = PathBuf::from(format!("statics{}.uddp", map_id));
            let out_file = resolve_output_path(&paths, output.as_ref().unwrap_or(&default_output));
            let summary = convert_statics_mul_to_uddp_from_sources(&paths, &out_file, map_id)?;
            println!(
                "Wrote {} chunks with {} total statics for map {} to '{}'.",
                summary.chunk_count,
                summary.total_statics,
                summary.map_id,
                out_file.display(),
            );
        }
        Commands::PackRadar {
            source_dirs: source_dir_args,
            map_id,
            uddp_dir,
            outdir,
            output,
            format,
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

            if radar_format == udd_conv::cc_radar::RadarFormat::Bc7Ktx2 {
                udd_conv_ktx2::build_facet_radar_ktx2(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    zstd_level,
                )?;
            } else {
                udd_conv::cc_radar::build_facet_radar_dds(
                    &paths,
                    &tilemeta_path,
                    &out_file,
                    map_id,
                    &udd_conv::cc_radar::RadarBuildOptions {
                        format: radar_format,
                        zstd_level,
                    },
                )?;
            }
        }
        Commands::PackLights {
            source_dirs: source_dir_args,
            output,
            no_compression,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if no_compression {
                CompressionFlag::None
            } else {
                CompressionFlag::ZstdNoDict
            };
            convert_client_lights_to_world_lights_uddp(
                source_dir_args.ccdir.as_deref(),
                source_dir_args.ecdir.as_deref(),
                &out_file,
                &WorldLightsOptions { compression },
            )?;
            println!("Wrote world_lights.uddp to '{}'.", out_file.display());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let shared = PathBuf::from("/tmp/uo-client");
        let dirs = collect_source_dirs(&SourceDirArgs {
            ccdir: Some(shared.clone()),
            ecdir: Some(shared),
        })
        .expect("collect source dirs");

        assert_eq!(dirs, vec![PathBuf::from("/tmp/uo-client")]);
    }

    #[test]
    fn collect_ec_source_dirs_uses_only_ec_root() {
        let dirs = collect_ec_source_dirs(&SourceDirArgs {
            ccdir: Some(PathBuf::from("/cc")),
            ecdir: Some(PathBuf::from("/ec")),
        })
        .expect("collect ec dirs");

        assert_eq!(dirs, vec![PathBuf::from("/ec")]);
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
                output,
                ec_art_cropped,
                use_ec_radarcol,
            } => {
                assert_eq!(source_dirs.ccdir, Some(PathBuf::from("/cc")));
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
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
            "--land-bc7",
        ])
        .expect("parse unified ec texture args");

        match cli.command {
            Commands::PackEcTextures {
                source_dirs,
                art_output,
                land_output,
                land_bc7,
                ..
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(art_output, PathBuf::from("tex_art_ec.uddp"));
                assert_eq!(land_output, PathBuf::from("tex_land_ec.uddp"));
                assert!(land_bc7);
            }
            _ => panic!("unexpected command parsed"),
        }
    }
}
