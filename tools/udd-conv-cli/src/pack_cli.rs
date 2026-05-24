use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ec_material_audit::{
    audit_ec_material_refs, audit_ec_surface_redirection, audit_ec_terrain_definition_kdl,
    audit_ec_terrain_overrides, audit_ec_terrain_primary_selection, inventory_ec_support_textures,
    write_ec_material_baseline_report, write_ec_terrain_override_candidates,
    write_ec_terrain_practical_review,
};
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use color_eyre::eyre;
use serde::Serialize;
use udd_conv::{
    classic_patches::ClassicPatchOptions,
    cc_gumps::{convert_gumps_to_uddp_from_sources_with_patches, GUMPS_CC_DEFAULT_OUTPUT},
    cc_map::{convert_map_mul_to_uddp_from_sources_with_patches, CcMapSourcePreference},
    cc_statics::convert_statics_mul_to_uddp_from_sources_with_patches,
    ec_gumps::{
        convert_ec_gumps_to_uddp_from_sources, EcGumpsOptions, EC_GUMP_DEFAULT_MAX_ID,
        GUMPS_EC_DEFAULT_OUTPUT,
    },
    hues::{convert_hues_mul_to_hues_uddp_from_sources, HuesOptions},
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
    label: &'static str,
}

fn resolve_texture_output_format(
    raw: bool,
    bc7: bool,
    bc7_rdo: bool,
    bc7_rdo_lambda: f32,
) -> eyre::Result<TextureOutputFormat> {
    match (raw, bc7, bc7_rdo) {
        (true, false, false) => Ok(TextureOutputFormat {
            compression: CompressionFlag::ZstdNoDict,
            pixel_format: PagePixelFormat::Rgba8888,
            bc7_rdo_lambda: 0.0,
            label: "RGBA8888",
        }),
        (false, true, false) => Ok(TextureOutputFormat {
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda: 0.0,
            label: "BC7",
        }),
        (false, false, true) => Ok(TextureOutputFormat {
            compression: CompressionFlag::None,
            pixel_format: PagePixelFormat::Bc7,
            bc7_rdo_lambda,
            label: "BC7 RDO",
        }),
        _ => eyre::bail!("select exactly one output format: --raw, --bc7, or --bc7-rdo"),
    }
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
    #[command(group(ArgGroup::new("output_format").required(true).args(["raw", "bc7", "bc7_rdo"])))]
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
        #[arg(long, help = "Write RGBA8888 atlas pages with package compression.")]
        raw: bool,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Atlas placement policy.")]
        packing_mode: CliAtlasPackingMode,
        #[arg(long, default_value_t = false, help = "Extrude slot edge pixels into atlas gutters for linear/bilinear filtering.")]
        filtering_ready: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
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
    #[command(group(ArgGroup::new("output_format").required(true).args(["raw", "bc7", "bc7_rdo"])))]
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
        #[arg(long, help = "Write RGBA8888 atlas pages with package compression.")]
        raw: bool,
        #[arg(long, help = "Write BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Atlas placement policy.")]
        packing_mode: CliAtlasPackingMode,
        #[arg(long, default_value_t = false, help = "Extrude tile edge pixels into atlas gutters for linear/bilinear filtering.")]
        filtering_ready: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
        #[arg(long, value_enum, default_value_t = CliUpscaleFilter::None)]
        upscale: CliUpscaleFilter,
    },
    /// Packs classic anim*.mul/anim*.idx mobile animations into mobile_anim_cc.uddp atlas pages.
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
    },
    /// Packs EC AnimationFrame.uop mobile animations into mobile_anim_ec.uddp atlas pages.
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
    },
    /// Packs EC art and land in one shared source pass into tex_art_ec.uddp and tex_land_ec.uddp.
    #[command(group(ArgGroup::new("output_format").required(true).args(["raw", "bc7", "bc7_rdo"])))]
    PackEcTextures {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "tex_art_ec.uddp")]
        art_output: PathBuf,
        #[arg(long, default_value = "tex_land_ec.uddp")]
        land_output: PathBuf,
        #[arg(long, help = "Embed this terrain routing KDL in tex_land_ec.uddp instead of auto-discovered TerrainTranscode.kdl.")]
        land_transcode_kdl: Option<PathBuf>,
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
        #[arg(long, help = "Write EC land RGBA8888 atlas pages with package compression.")]
        raw: bool,
        #[arg(long, help = "Write EC land BC7 atlas pages without BC7 RDO.")]
        bc7: bool,
        #[arg(long = "bc7-rdo", help = "Write EC land BC7 atlas pages with BC7 RDO.")]
        bc7_rdo: bool,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Art atlas placement policy.")]
        art_packing_mode: CliAtlasPackingMode,
        #[arg(long, value_enum, default_value_t = CliAtlasPackingMode::MaximumPacking, help = "Land atlas placement policy.")]
        land_packing_mode: CliAtlasPackingMode,
        #[arg(long, default_value_t = false, help = "Extrude EC art edge pixels into atlas gutters for linear/bilinear filtering.")]
        art_filtering_ready: bool,
        #[arg(long, default_value_t = false, help = "Extrude EC land edge pixels into atlas gutters for linear/bilinear filtering.")]
        land_filtering_ready: bool,
        #[arg(long, default_value_t = udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA, help = "BC7 RDO lambda for EC texture pages. Use 0 to disable RDO.")]
        bc7_rdo_lambda: f32,
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
    },
    /// Packs Classic hues.mul into hues.uddp.
    PackHues {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = "hues.uddp")]
        output: PathBuf,
        #[arg(long, default_value_t = false, help = "Do not use compression.")]
        no_compression: bool,
    },
    /// Packs Classic gumpidx.mul/gumpart.mul or gumpartLegacyMUL.uop into gumps_cc.uddp.
    PackGumps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[command(flatten)]
        classic_patches: ClassicPatchArgs,
        #[arg(long, default_value = GUMPS_CC_DEFAULT_OUTPUT)]
        output: PathBuf,
    },
    /// Packs EC interface.uop gumpart into gumps_ec.uddp.
    PackEcGumps {
        #[command(flatten)]
        source_dirs: SourceDirArgs,
        #[arg(long, default_value = GUMPS_EC_DEFAULT_OUTPUT)]
        output: PathBuf,
        #[arg(long, default_value_t = EC_GUMP_DEFAULT_MAX_ID, help = "Highest numeric gump id to probe when reading interface.uop.")]
        max_id: u32,
    },
}

pub fn run() -> eyre::Result<()> {
    color_eyre::install()?;
    env_logger::init();

    match Cli::parse().command {
        Commands::PackArt {
            source_dirs: source_dir_args,
            classic_patches,
            output,
            atlas_width,
            atlas_height,
            gutter,
            raw,
            bc7,
            bc7_rdo,
            packing_mode,
            filtering_ready,
            bc7_rdo_lambda,
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
            let output_format = resolve_texture_output_format(raw, bc7, bc7_rdo, bc7_rdo_lambda)?;
            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                &TexArtCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression: output_format.compression,
                    upscale: upscale.into(),
                    pixel_format: output_format.pixel_format,
                    packing_mode: packing_mode.into(),
                    filtering_ready,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
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
            bc7,
            bc7_rdo,
            packing_mode,
            filtering_ready,
            bc7_rdo_lambda,
            upscale,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let output_format = resolve_texture_output_format(raw, bc7, bc7_rdo, bc7_rdo_lambda)?;
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
                    pixel_format: output_format.pixel_format,
                    packing_mode: packing_mode.into(),
                    filtering_ready,
                    bc7_rdo_lambda: output_format.bc7_rdo_lambda,
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
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_anim_mul_to_mobile_anim_cc_uddp_from_sources(
                &paths,
                &out_file,
                &MobileAnimCcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression: CompressionFlag::ZstdNoDict,
                    pixel_format: PagePixelFormat::Rgba8888,
                },
            )?;
            println!(
                "Wrote {} pages (RGBA8888) for {} packed frames out of {} total frames across {} animations to '{}'.",
                summary.page_count,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                out_file.display()
            );
        }
        Commands::PackEcMobileAnims {
            source_dirs: source_dir_args,
            output,
            atlas_width,
            atlas_height,
            gutter,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources(
                &paths,
                &out_file,
                &MobileAnimEcAtlasOptions {
                    atlas_width,
                    atlas_height,
                    gutter,
                    compression: CompressionFlag::ZstdNoDict,
                    pixel_format: PagePixelFormat::Rgba8888,
                },
            )?;
            println!(
                "Wrote {} pages (RGBA8888) from {} EC animation UOPs for {} packed frames out of {} logical frames across {} EC animations from {} bodies with {} metadata items and {} source hints to '{}'.",
                summary.page_count,
                summary.source_uop_count,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                summary.body_count,
                summary.item_metadata_count,
                summary.source_hint_count,
                out_file.display()
            );
        }
        Commands::PackEcTextures {
            source_dirs: source_dir_args,
            art_output,
            land_output,
            land_transcode_kdl,
            art_atlas_width,
            art_atlas_height,
            art_gutter,
            land_atlas_width,
            land_atlas_height,
            land_gutter,
            raw,
            bc7,
            bc7_rdo,
            art_packing_mode,
            land_packing_mode,
            art_filtering_ready,
            land_filtering_ready,
            bc7_rdo_lambda,
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
            let land_output_format =
                resolve_texture_output_format(raw, bc7, bc7_rdo, bc7_rdo_lambda)?;
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
                    filtering_ready: art_filtering_ready,
                    bc7_rdo_lambda,
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
                    pixel_format: land_output_format.pixel_format,
                    packing_mode: land_packing_mode.into(),
                    filtering_ready: land_filtering_ready,
                    bc7_rdo_lambda: land_output_format.bc7_rdo_lambda,
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
            ec_art_cropped,
            use_ec_radarcol,
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let options = TileMetaBuildOptions {
                adjust_tex_art_ec_sampling: ec_art_cropped,
                use_ec_radarcol,
                classic_patches: classic_patches.into(),
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
        Commands::PackRadar {
            source_dirs: source_dir_args,
            classic_patches,
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
                let bc7_data = udd_conv::cc_radar::build_facet_radar_bc7_with_patches(
                    &paths,
                    &tilemeta_path,
                    map_id,
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
                    },
                )?;
            }
        }
        Commands::PackLights {
            source_dirs: source_dir_args,
            classic_patches,
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
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let compression = if no_compression {
                CompressionFlag::None
            } else {
                CompressionFlag::ZstdNoDict
            };
            convert_hues_mul_to_hues_uddp_from_sources(
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
        } => {
            let paths = collect_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_gumps_to_uddp_from_sources_with_patches(
                &paths,
                &out_file,
                &classic_patches.into(),
            )?;
            println!(
                "Wrote {} gumps to '{}'.",
                summary.gump_count,
                out_file.display()
            );
        }
        Commands::PackEcGumps {
            source_dirs: source_dir_args,
            output,
            max_id,
        } => {
            let paths = collect_ec_source_dirs(&source_dir_args)?;
            let out_file = resolve_output_path(&paths, &output);
            let summary = convert_ec_gumps_to_uddp_from_sources(
                &paths,
                &out_file,
                &EcGumpsOptions {
                    max_id,
                    compression: CompressionFlag::ZstdNoDict,
                },
            )?;
            println!(
                "Wrote {} EC gumps to '{}' ({} skipped).",
                summary.gump_count,
                out_file.display(),
                summary.skipped_count
            );
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
                classic_patches,
                output,
                ec_art_cropped,
                use_ec_radarcol,
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
                assert!(bc7);
                assert!(!bc7_rdo);
                assert!(!raw);
            }
            _ => panic!("unexpected command parsed"),
        }
    }

    #[test]
    fn cli_requires_texture_output_format() {
        let error = match Cli::try_parse_from([
            "uddpack",
            "pack-art",
            "--ccdir",
            "/cc",
            "--output",
            "tex_art_cc.uddp",
        ]) {
            Ok(_) => panic!("texture pack commands must require an output format"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("--raw"));
        assert!(error.to_string().contains("--bc7"));
        assert!(error.to_string().contains("--bc7-rdo"));
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
            } => {
                assert_eq!(source_dirs.ecdir, Some(PathBuf::from("/ec")));
                assert_eq!(output, PathBuf::from("gumps_ec.uddp"));
                assert_eq!(max_id, 12345);
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
