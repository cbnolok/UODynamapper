use color_eyre::eyre::{self, WrapErr};
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use udd_conv::{
    classic_patches::ClassicPatchOptions,
    tex_aux_ec::{pack_tex_aux_ec, TexAuxEcPackOptions},
    mobile_anim_cc::{
        convert_anim_mul_to_mobile_anim_cc_uddp_from_sources_with_progress,
        MobileAnimCcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as CC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    mobile_anim_ec::{
        convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources_with_progress,
        MobileAnimEcAtlasOptions,
        DEFAULT_ATLAS_GUTTER as EC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
        DEFAULT_ATLAS_PAGE_HEIGHT as EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
        DEFAULT_ATLAS_PAGE_WIDTH as EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
    },
    tex_art_cc::{TexArtCcAtlasOptions, convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches_and_progress, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    tex_art_ec::{TexArtEcAtlasOptions, convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources_with_progress},
    tex_land_ec::{TexLandEcAtlasOptions, convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources_with_progress},
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources_with_progress, build_tilemeta_uddp_from_split_sources_with_progress},
    cc_map::convert_map_mul_to_uddp_from_sources_with_patches,
    chunk_compression::ChunkCompression,
    cc_statics::convert_statics_mul_to_uddp_from_sources_with_patches,
    tex_land_cc::{
        CLASSIC_LAND_LOSSY_REFUSAL, TexLandCcAtlasOptions, convert_texmaps_mul_to_tex_land_cc_uddp_with_patches_and_progress,
    },
    cc_radar::{build_facet_radar_dds, RadarFormat, RadarBuildOptions},
    source_paths::gather_source_dirs,
    upscale::{UpscaleConfig, UpscaleFilter, UpscalePass},
    BuildProgress, BuildProgressPhase, CompressionFlag,
    PagePixelFormat,
};
use udd_conv::cc_gumps::{
    convert_gumps_to_uddp_from_sources_with_patches_options_and_progress, CcGumpsBuildSummary, CcGumpsOptions,
    CC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_CC_DEFAULT_OUTPUT,
};
use udd_conv::ec_gumps::{
    convert_ec_gumps_to_uddp_from_sources_with_progress, EcGumpsBuildSummary, EcGumpsOptions,
    EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT, EC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_EC_DEFAULT_OUTPUT,
};
use udd_conv::effects_ec::{pack_effects_ec, EffectsEcPackOptions};
use udd_conv::paperdoll_ec::{pack_paperdoll_ec, PaperdollEcPackOptions};
use udd_conv::hues::{convert_hues_to_hues_uddp_from_sources, HuesOptions};
use udd_conv::source_paths::find_first_existing_file;
use udd_conv::world_lights::{convert_client_lights_to_world_lights_uddp, WorldLightsOptions};
use uocf::enhanced::effect_texture::EFFECT_TEXTURE_UOP_NAME;
use uocf::enhanced::hues::HUES_UOP_NAME;
use uocf::enhanced::paperdoll::PAPERDOLL_UOP_NAME;
use udd_conv::page_compression::LOSSLESS_PAGE_ZSTD_LEVEL;
use udd_conv::single_gumps::{default_single_gump_compression, GumpPixelFormat};
use udd_conv::upscale_profile::UpscaleProfile;
use udd_conv::package_progress::{AssetTaskProgress, AssetTaskProgressStage};
use udd_conv_cli::{
    package_info::get_package_info_string,
    package_validate::{report_lines, validate_inputs, ReportLineKind, ValidateCounts},
    extract::extract_package,
    tool_cli::{diff_paths_report, export_csv_auto_report, hash_path_report, DiffKind},
    upscale_profile_config::load_upscale_profile as load_upscale_profile_config,
};
use udd_conv::conversion_tables::{EC_MOBILE_ANIMATIONS_KDL, EC_TERRAIN_OVERRIDES_KDL, TERRAIN_TRANSCODE_KDL};
use uocf::utils::bundled_files::{self, BundledFile};
use crate::app::UddConvApp;
use crate::models::{
    AppSettings, AssetPackProgress, AssetPackProgressState, AssetPackTask, GumpsEcSharing, LogLevel, LogMessage,
    TextureOptimization,
};

struct ConvertingFlagReset {
    is_converting: std::sync::Arc<std::sync::Mutex<bool>>,
}

#[derive(Debug)]
struct ConversionCancelled;

#[derive(Clone)]
struct AssetProgressReporter {
    task: AssetPackTask,
    progress: Arc<Mutex<HashMap<AssetPackTask, AssetPackProgress>>>,
    cancel: Arc<AtomicBool>,
}

impl AssetProgressReporter {
    fn set(&self, state: AssetPackProgressState, fraction: f32, text: impl Into<String>) {
        if let Ok(mut progress) = self.progress.lock() {
            let fraction = fraction.clamp(0.0, 1.0);
            if state == AssetPackProgressState::Running {
                if let Some(current) = progress.get(&self.task) {
                    if current.state == AssetPackProgressState::Running
                        && fraction < current.fraction
                    {
                        return;
                    }
                }
            }
            progress.insert(self.task, AssetPackProgress {
                state,
                fraction,
                text: text.into(),
            });
        }
    }

    /// A reporter of another task that shares this one's progress map and cancel flag, for a job that runs two tasks.
    fn for_task(&self, task: AssetPackTask) -> Self {
        Self {
            task,
            progress: self.progress.clone(),
            cancel: self.cancel.clone(),
        }
    }

    fn start(&self) {
        self.set(AssetPackProgressState::Running, 0.0, "Preparing");
    }

    fn finish(&self) {
        self.set(AssetPackProgressState::Succeeded, 1.0, "Complete");
    }

    fn fail(&self) {
        self.set(AssetPackProgressState::Failed, 1.0, "Failed");
    }

    fn cancelled(&self) {
        self.set(AssetPackProgressState::Cancelled, 1.0, "Cancelled");
    }

    fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn panic_if_cancelled(&self) {
        if self.cancel_requested() {
            panic::panic_any(ConversionCancelled);
        }
    }

    fn task_progress_with_extract_label(
        &self,
        progress: AssetTaskProgress,
        extract_label: &'static str,
    ) {
        self.panic_if_cancelled();
        let fraction = if progress.total == 0 {
            0.0
        } else {
            progress.completed.min(progress.total) as f32 / progress.total as f32
        };
        let (base, span, label) = match progress.stage {
            AssetTaskProgressStage::Extracting => (0.0, 0.20, extract_label),
            AssetTaskProgressStage::PackingAtlas => (0.20, 0.20, "Building atlas"),
            AssetTaskProgressStage::EncodingBc7 => (0.40, 0.15, "BC7 encoding"),
            AssetTaskProgressStage::ApplyingRdo => (0.55, 0.15, "Applying RDO"),
            AssetTaskProgressStage::RegisteringPages => (0.40, 0.30, "Registering pages"),
        };
        self.set(
            AssetPackProgressState::Running,
            base + fraction * span,
            format!("{label} {}/{}", progress.completed, progress.total),
        );
    }

    fn build_progress(&self, progress: BuildProgress) {
        // The package file is created as assembling starts: a stop honoured then would leave a partial package, so
        // the job finishes instead.
        if progress.phase != BuildProgressPhase::Assembling {
            self.panic_if_cancelled();
        }
        let phase_fraction = if progress.total == 0 {
            0.0
        } else {
            progress.completed.min(progress.total) as f32 / progress.total as f32
        };
        let (base, span, phase_label) = match progress.phase {
            BuildProgressPhase::TrainingDictionaries => (0.70, 0.05, "Training dictionaries"),
            BuildProgressPhase::CompressingFiles => (0.75, 0.18, "Compressing package"),
            BuildProgressPhase::Assembling => (0.93, 0.05, "Assembling package"),
        };
        let text = if progress.phase == BuildProgressPhase::CompressingFiles {
            if let Some(file) = progress.active_file {
                format!("Compressing file {}/{}", file.index + 1, file.total)
            } else {
                format!("{phase_label} {}/{}", progress.completed, progress.total)
            }
        } else {
            format!("{phase_label} {}/{}", progress.completed, progress.total)
        };
        self.set(
            AssetPackProgressState::Running,
            base + phase_fraction * span,
            text,
        );
    }
}

impl Drop for ConvertingFlagReset {
    fn drop(&mut self) {
        if let Ok(mut is_converting) = self.is_converting.lock() {
            *is_converting = false;
        }
    }
}

fn classic_patch_options(settings: &crate::models::AppSettings) -> ClassicPatchOptions {
    ClassicPatchOptions {
        verdata: settings.include_verdata,
        map_difs: settings.include_map_difs,
        static_difs: settings.include_static_difs,
        map_edition: settings.map_edition,
    }
}

fn gather_single_source_dir(
    dir: Option<&std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    dir.into_iter().cloned().collect()
}

fn gather_cc_source_dirs(
    cc_dir: Option<&std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    gather_single_source_dir(cc_dir)
}

fn gather_ec_source_dirs(
    ec_dir: Option<&std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    gather_single_source_dir(ec_dir)
}

fn ensure_output_parent(output: &Path) -> eyre::Result<()> {
    let Some(parent) = output.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}

fn texture_compression(
    optimization: TextureOptimization,
    zstd_level: i32,
) -> CompressionFlag {
    match optimization {
        TextureOptimization::None => CompressionFlag::ZstdNoDictLevel(zstd_level),
        TextureOptimization::Bc7 => CompressionFlag::None,
        TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDictLevel(zstd_level),
    }
}

/// The BC7 RDO lambda that turns RDO off.
const NO_BC7_RDO_LAMBDA: f32 = 0.0;

fn bc7_rdo_lambda(settings: &AppSettings, optimization: TextureOptimization) -> f32 {
    if settings.bc7_rdo_enabled && matches!(optimization, TextureOptimization::Bc7Zstd) {
        settings.bc7_rdo_lambda
    } else {
        NO_BC7_RDO_LAMBDA
    }
}

fn bc7_rdo_lookback_blocks(settings: &AppSettings) -> usize {
    udd_conv::bc7::normalize_bc7_rdo_lookback_blocks(settings.bc7_rdo_lookback_blocks)
}

fn upscale_filter_active(filter: UpscaleFilter) -> bool {
    !matches!(filter, UpscaleFilter::None)
}

fn upscale_config_active(config: UpscaleConfig) -> bool {
    upscale_filter_active(config.filter)
}

fn upscale_profile_active(settings: &AppSettings) -> bool {
    settings.upscale_profile_path.is_some()
}

fn load_upscale_profile(settings: &AppSettings) -> eyre::Result<Option<Arc<udd_conv::upscale_profile::UpscaleProfile>>> {
    settings
        .upscale_profile_path
        .as_deref()
        .map(|path| {
            load_upscale_profile_config(path)
                .map(Arc::new)
                .wrap_err_with(|| format!("load upscale profile {}", path.display()))
        })
        .transpose()
}

/// A conversion table: the one in the Dynamapper Routing Files folder when that is set, else the copy shipped with the
/// tools.
fn conversion_table(settings: &AppSettings, file: BundledFile) -> eyre::Result<PathBuf> {
    let given = settings.dynamapper_routing_dir.as_deref().map(|dir| file.in_dir(dir));
    bundled_files::resolve(given.as_deref(), file)
}

fn upscale_filter_passes(filter: UpscaleFilter) -> Vec<UpscalePass> {
    if upscale_filter_active(filter) {
        vec![UpscalePass::from(filter)]
    } else {
        Vec::new()
    }
}

/// The hues, world lights and Enhanced effects packers report no progress, so their bar shows this much while they
/// run.
const UNREPORTED_PACKING_PROGRESS: f32 = 0.1;
/// Where the bar of a packing job stands while the packages it wrote are validated.
const VALIDATING_PROGRESS: f32 = 0.99;

/// The extraction label of a job whose images an upscale filter or the upscale profile changes.
fn extract_label(upscaling: bool, settings: &AppSettings) -> &'static str {
    if upscaling || upscale_profile_active(settings) { "Extracting and upscaling" } else { "Extracting" }
}

/// The single gumps' pixel format, as `pack-gumps --bc1` and `pack-ec-gumps --bc7` pick it: the package's lossy format,
/// else lossless RGBA8888.
pub(crate) fn single_gump_pixel_format(lossy: bool, lossy_format: GumpPixelFormat) -> GumpPixelFormat {
    if lossy { lossy_format } else { GumpPixelFormat::Rgba8888 }
}

/// The single gumps' codec, as the gump packers' `--zstd=LEVEL` sets it: Zstd at the chosen level, else the package's
/// level for their pixel format.
fn single_gump_compression(zstd_level: Option<i32>, format: GumpPixelFormat) -> CompressionFlag {
    zstd_level
        .map(CompressionFlag::ZstdNoDictLevel)
        .unwrap_or_else(|| default_single_gump_compression(format))
}

/// The paperdoll atlas pages' codec, as the gump packers' `--zstd=LEVEL` sets it: the pages are lossless RGBA8888, with
/// Zstd at the chosen level, else at the lossless pages' level.
fn gump_atlas_compression(zstd_level: Option<i32>) -> CompressionFlag {
    CompressionFlag::ZstdNoDictLevel(zstd_level.unwrap_or(LOSSLESS_PAGE_ZSTD_LEVEL))
}

/// The folders the Classic gumps job reads, as `pack-gumps --ccdir --ecdir` does: the Classic one first.
pub(crate) fn cc_gumps_source_dirs(settings: &AppSettings) -> Vec<PathBuf> {
    gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref())
}

/// The folder the Enhanced gumps job reads, as `pack-ec-gumps --ecdir` does: the extracted gumpart folder when one is
/// set, else the Enhanced client folder.
pub(crate) fn ec_gumps_source_dirs(settings: &AppSettings) -> Vec<PathBuf> {
    gather_single_source_dir(settings.gumps_ec_gumpart_dir.as_ref().or(settings.ec_dir.as_ref()))
}

/// The Classic gumps job's options, mapped from the settings as `pack-gumps` maps its flags.
fn cc_gumps_options(settings: &AppSettings, upscale_profile: Option<Arc<UpscaleProfile>>) -> CcGumpsOptions {
    let single_pixel_format = single_gump_pixel_format(settings.gumps_cc_bc1, CC_LOSSY_SINGLE_GUMP_FORMAT);
    CcGumpsOptions {
        single_pixel_format,
        compression: single_gump_compression(settings.zstd_gumps_cc, single_pixel_format),
        atlas_compression: gump_atlas_compression(settings.zstd_gumps_cc),
        paperdoll_upscale_passes: upscale_filter_passes(settings.upscale_gumps_cc_paperdoll),
        single_upscale_passes: upscale_filter_passes(settings.upscale_gumps_cc_single),
        upscale_profile,
        source_preference: settings.gumps_cc_source,
    }
}

/// The Enhanced gumps job's options, mapped from the settings as `pack-ec-gumps` maps its flags.
fn ec_gumps_options(
    settings: &AppSettings,
    upscale_profile: Option<Arc<UpscaleProfile>>,
    share_with: Option<PathBuf>,
) -> EcGumpsOptions {
    let single_pixel_format = single_gump_pixel_format(settings.gumps_ec_bc7, EC_LOSSY_SINGLE_GUMP_FORMAT);
    EcGumpsOptions {
        max_id: settings.gumps_ec_max_id,
        single_pixel_format,
        compression: single_gump_compression(settings.zstd_gumps_ec, single_pixel_format),
        atlas_compression: gump_atlas_compression(settings.zstd_gumps_ec),
        paperdoll_upscale_passes: upscale_filter_passes(settings.upscale_gumps_ec_paperdoll),
        single_upscale_passes: upscale_filter_passes(settings.upscale_gumps_ec_single),
        upscale_profile,
        share_with,
    }
}

/// What the Enhanced gumps job shares the Classic twins with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EcGumpsShare {
    /// This gumps_cc.uddp (`pack-ec-gumps --share-with`).
    With(PathBuf),
    /// Nothing: packing standalone was chosen.
    Standalone,
    /// Nothing although sharing was chosen, since this gumps_cc.uddp isn't there: the package is standalone.
    MissingGumpsCc(PathBuf),
}

impl EcGumpsShare {
    fn share_with(&self) -> Option<PathBuf> {
        match self {
            Self::With(gumps_cc) => Some(gumps_cc.clone()),
            Self::Standalone | Self::MissingGumpsCc(_) => None,
        }
    }
}

/// What the Enhanced gumps share, decided as `uddconv.py` does: with sharing chosen, the output folder's gumps_cc.uddp
/// when it is there or the same job builds it first, else nothing.
pub(crate) fn ec_gumps_share(sharing: GumpsEcSharing, gumps_cc: &Path, gumps_cc_available: bool) -> EcGumpsShare {
    match sharing {
        GumpsEcSharing::Standalone => EcGumpsShare::Standalone,
        GumpsEcSharing::ShareWithGumpsCc if gumps_cc_available => EcGumpsShare::With(gumps_cc.to_path_buf()),
        GumpsEcSharing::ShareWithGumpsCc => EcGumpsShare::MissingGumpsCc(gumps_cc.to_path_buf()),
    }
}

/// Packs the Classic gumps as `pack-gumps` does, reporting to `progress`.
fn pack_gumps_cc(
    settings: &AppSettings,
    output: &Path,
    progress: &AssetProgressReporter,
) -> eyre::Result<CcGumpsBuildSummary> {
    let sources = cc_gumps_source_dirs(settings);
    if sources.is_empty() { eyre::bail!("No Classic or Enhanced source dir configured"); }
    ensure_output_parent(output)?;
    let options = cc_gumps_options(settings, load_upscale_profile(settings)?);
    let label = extract_label(
        upscale_filter_active(settings.upscale_gumps_cc_paperdoll) || upscale_filter_active(settings.upscale_gumps_cc_single),
        settings,
    );
    convert_gumps_to_uddp_from_sources_with_patches_options_and_progress(
        &sources,
        output,
        &classic_patch_options(settings),
        &options,
        |task_progress| progress.task_progress_with_extract_label(task_progress, label),
        |build_progress| progress.build_progress(build_progress),
    )
}

fn describe_gumps_cc(settings: &AppSettings, output: &Path, summary: &CcGumpsBuildSummary) -> String {
    let single_pixel_format = single_gump_pixel_format(settings.gumps_cc_bc1, CC_LOSSY_SINGLE_GUMP_FORMAT);
    format!(
        "Wrote {} Classic gumps to {} ({} single in {}, {} with soft alpha; {} atlas)",
        summary.gump_count,
        output.display(),
        summary.single_gump_count,
        single_pixel_format.label(),
        summary.single_soft_alpha_count,
        summary.atlas_gump_count
    )
}

/// Packs the Enhanced gumps as `pack-ec-gumps` does, sharing as `share` says and reporting to `progress`.
fn pack_gumps_ec(
    settings: &AppSettings,
    output: &Path,
    share: &EcGumpsShare,
    progress: &AssetProgressReporter,
) -> eyre::Result<EcGumpsBuildSummary> {
    let sources = ec_gumps_source_dirs(settings);
    if sources.is_empty() { eyre::bail!("No Enhanced source dir or extracted gumpart folder configured"); }
    ensure_output_parent(output)?;
    if let EcGumpsShare::MissingGumpsCc(gumps_cc) = share {
        log::warn!(
            "No {} to share the Classic gumps with: packing {} standalone.",
            gumps_cc.display(),
            output.display()
        );
    }
    let options = ec_gumps_options(settings, load_upscale_profile(settings)?, share.share_with());
    let label = extract_label(
        upscale_filter_active(settings.upscale_gumps_ec_paperdoll) || upscale_filter_active(settings.upscale_gumps_ec_single),
        settings,
    );
    convert_ec_gumps_to_uddp_from_sources_with_progress(
        &sources,
        output,
        &options,
        |task_progress| progress.task_progress_with_extract_label(task_progress, label),
        |build_progress| progress.build_progress(build_progress),
    )
}

fn describe_gumps_ec(settings: &AppSettings, output: &Path, summary: &EcGumpsBuildSummary) -> String {
    let single_pixel_format = single_gump_pixel_format(settings.gumps_ec_bc7, EC_LOSSY_SINGLE_GUMP_FORMAT);
    let single_formats = match summary.single_classic_colour_space_count {
        0 => single_pixel_format.label().to_string(),
        classic_count => format!(
            "{}, {classic_count} of them in the Classic colour space as {}",
            single_pixel_format.label(),
            EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT.label()
        ),
    };
    format!(
        "Wrote {} Enhanced gumps to {} ({} single in {}, {} with soft alpha; {} shared with gumps_cc; {} atlas; {} \
         skipped)",
        summary.gump_count,
        output.display(),
        summary.single_gump_count,
        single_formats,
        summary.single_soft_alpha_count,
        summary.single_shared_classic_count,
        summary.atlas_gump_count,
        summary.skipped_count
    )
}

/// Packs the Classic gumps, then the Enhanced gumps, sharing with the new gumps_cc when sharing is chosen. `progress`
/// reports the Enhanced gumps; the Classic gumps report on their own card.
fn pack_gumps_cc_and_ec(
    settings: &AppSettings,
    cc_output: &Path,
    ec_output: &Path,
    progress: &AssetProgressReporter,
) -> eyre::Result<(CcGumpsBuildSummary, EcGumpsBuildSummary)> {
    let cc_progress = progress.for_task(AssetPackTask::GumpsCc);
    progress.set(AssetPackProgressState::Running, 0.0, "Waiting for the Classic gumps");
    cc_progress.start();
    let cc_summary = match panic::catch_unwind(AssertUnwindSafe(|| pack_gumps_cc(settings, cc_output, &cc_progress))) {
        Ok(Ok(summary)) => {
            cc_progress.finish();
            summary
        }
        Ok(Err(error)) => {
            cc_progress.fail();
            return Err(error.wrap_err("pack the Classic gumps"));
        }
        // A stop while the Classic gumps pack ends both cards.
        Err(payload) => {
            if payload.is::<ConversionCancelled>() {
                cc_progress.cancelled();
            } else {
                cc_progress.fail();
            }
            panic::resume_unwind(payload);
        }
    };
    progress.panic_if_cancelled();
    let share = ec_gumps_share(settings.gumps_ec_sharing, cc_output, true);
    let ec_summary = pack_gumps_ec(settings, ec_output, &share, progress)?;
    Ok((cc_summary, ec_summary))
}

/// What `pack-hues`, `pack-lights`, `pack-ec-effects` and `pack-ec-paperdoll` write without `--output`.
pub(crate) const HUES_OUTPUT: &str = "hues.uddp";
pub(crate) const WORLD_LIGHTS_OUTPUT: &str = "world_lights.uddp";
pub(crate) const EFFECTS_EC_OUTPUT: &str = "effects_ec.uddp";
pub(crate) const PAPERDOLL_EC_OUTPUT: &str = "paperdoll_ec.uddp";
/// The Classic hues file the hues job reads first.
const HUES_MUL_NAME: &str = "hues.mul";
/// The other spelling of the Enhanced hues file `pack-hues` looks for.
const HUES_UOP_CAPITALIZED_NAME: &str = "Hues.uop";

/// The package codec of the hues and world lights jobs, as `pack-hues` and `pack-lights` map `--no-compression` and
/// `--zstd[=LEVEL]`: raw, else Zstd at the chosen level, else Zstd at the packer's level.
fn package_compression(no_compression: bool, zstd_level: Option<i32>) -> CompressionFlag {
    if no_compression {
        CompressionFlag::None
    } else {
        zstd_level.map(CompressionFlag::ZstdNoDictLevel).unwrap_or(CompressionFlag::ZstdNoDict)
    }
}

/// The folders the hues job reads, as `pack-hues --ccdir --ecdir` does: the Classic one first.
pub(crate) fn hues_source_dirs(settings: &AppSettings) -> Vec<PathBuf> {
    gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref())
}

/// The file the hues job reads, as `convert_hues_to_hues_uddp_from_sources` picks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HuesSource {
    NoFolder,
    /// A Classic hues.mul, from any source folder: read first, and spread over the palette rows by the hue ramp.
    Mul(PathBuf),
    /// The Enhanced Hues.uop, read when no folder holds a hues.mul: copied as it is.
    Uop(PathBuf),
    Missing,
}

pub(crate) fn hues_source(settings: &AppSettings) -> HuesSource {
    let sources = hues_source_dirs(settings);
    if sources.is_empty() {
        HuesSource::NoFolder
    } else if let Some(mul) = find_first_existing_file(&sources, &[HUES_MUL_NAME]) {
        HuesSource::Mul(mul)
    } else if let Some(uop) = find_first_existing_file(&sources, &[HUES_UOP_NAME, HUES_UOP_CAPITALIZED_NAME]) {
        HuesSource::Uop(uop)
    } else {
        HuesSource::Missing
    }
}

/// The hues job's options, mapped from the settings as `pack-hues` maps its flags.
fn hues_options(settings: &AppSettings) -> HuesOptions {
    HuesOptions {
        compression: package_compression(settings.hues_no_compression, settings.zstd_hues),
        ramp: settings.hues_ramp.into(),
    }
}

/// Packs the hues as `pack-hues` does.
fn pack_hues(settings: &AppSettings, output: &Path) -> eyre::Result<String> {
    let sources = hues_source_dirs(settings);
    if sources.is_empty() { eyre::bail!("No Classic or Enhanced source dir configured"); }
    ensure_output_parent(output)?;
    convert_hues_to_hues_uddp_from_sources(&sources, output, &hues_options(settings))?;
    Ok(format!("Wrote {HUES_OUTPUT} to {}", output.display()))
}

/// The world lights job's options, mapped from the settings as `pack-lights` maps its flags; of the Classic patch
/// files only verdata.mul patches the lights.
fn world_lights_options(settings: &AppSettings) -> WorldLightsOptions {
    WorldLightsOptions {
        compression: package_compression(settings.lights_no_compression, settings.zstd_lights),
        classic_patches: classic_patch_options(settings),
    }
}

/// Packs the world lights as `pack-lights` does: the Classic light.mul and the Enhanced TerrainTexture.uop lights.
fn pack_world_lights(settings: &AppSettings, output: &Path) -> eyre::Result<String> {
    if settings.cc_dir.is_none() && settings.ec_dir.is_none() {
        eyre::bail!("No Classic or Enhanced source dir configured");
    }
    ensure_output_parent(output)?;
    convert_client_lights_to_world_lights_uddp(
        settings.cc_dir.as_deref(),
        settings.ec_dir.as_deref(),
        output,
        &world_lights_options(settings),
    )?;
    Ok(format!("Wrote {WORLD_LIGHTS_OUTPUT} to {}", output.display()))
}

/// The EffectTexture.uop the Enhanced effects job reads, as `pack-ec-effects --ecdir` finds it.
pub(crate) fn effects_ec_source(settings: &AppSettings) -> Option<PathBuf> {
    find_first_existing_file(&gather_ec_source_dirs(settings.ec_dir.as_ref()), &[EFFECT_TEXTURE_UOP_NAME])
}

/// The Enhanced effects job's options, mapped from the settings as `pack-ec-effects` maps its flags.
fn effects_ec_options(settings: &AppSettings) -> EffectsEcPackOptions {
    EffectsEcPackOptions {
        bc7: settings.effects_ec_bc7,
        compression: settings
            .zstd_effects_ec
            .map(CompressionFlag::ZstdNoDictLevel)
            .unwrap_or(CompressionFlag::ZstdNoDict),
        strict: settings.effects_ec_strict,
        report: settings.effects_ec_report.clone(),
    }
}

/// Packs the Enhanced effects as `pack-ec-effects` does.
fn pack_effects_ec_job(settings: &AppSettings, output: &Path) -> eyre::Result<String> {
    let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
    if sources.is_empty() { eyre::bail!("No Enhanced source dir configured"); }
    ensure_output_parent(output)?;
    let options = effects_ec_options(settings);
    if let Some(report) = &options.report {
        ensure_output_parent(report)?;
    }
    let summary = pack_effects_ec(&sources, output, &options)?;
    Ok(format!("effects_ec: {summary}; wrote {}", output.display()))
}

/// The Paperdoll.uop and Hues.uop the Enhanced paperdoll job reads, as `pack-ec-paperdoll --ecdir` finds them.
pub(crate) fn paperdoll_ec_sources(settings: &AppSettings) -> (Option<PathBuf>, Option<PathBuf>) {
    let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
    (
        find_first_existing_file(&sources, &[PAPERDOLL_UOP_NAME]),
        find_first_existing_file(&sources, &[HUES_UOP_NAME]),
    )
}

/// The Enhanced paperdoll job's options, mapped from the settings as `pack-ec-paperdoll` maps `--zstd=LEVEL`.
fn paperdoll_ec_options(settings: &AppSettings) -> PaperdollEcPackOptions {
    match settings.zstd_paperdoll_ec {
        Some(level) => PaperdollEcPackOptions { compression: CompressionFlag::ZstdNoDictLevel(level) },
        None => PaperdollEcPackOptions::default(),
    }
}

/// Packs the Enhanced paperdoll frames as `pack-ec-paperdoll` does.
fn pack_paperdoll_ec_job(settings: &AppSettings, output: &Path) -> eyre::Result<String> {
    let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
    if sources.is_empty() { eyre::bail!("No Enhanced source dir configured"); }
    ensure_output_parent(output)?;
    let summary = pack_paperdoll_ec(&sources, output, &paperdoll_ec_options(settings))?;
    Ok(format!("paperdoll_ec: {summary}; wrote {}", output.display()))
}

/// The panel's colour of a `tool validate` report line.
pub(crate) fn report_line_level(kind: ReportLineKind) -> LogLevel {
    match kind {
        ReportLineKind::Passed => LogLevel::Success,
        ReportLineKind::Failed | ReportLineKind::Error => LogLevel::Error,
        ReportLineKind::Detail | ReportLineKind::Note => LogLevel::Info,
    }
}

fn push_panel_log(logs: &Arc<Mutex<Vec<LogMessage>>>, text: String, level: LogLevel) {
    if let Ok(mut logs) = logs.lock() {
        logs.push(LogMessage { text, level });
    }
}

/// Validates the packages `inputs` names or holds as `tool validate` does, logging each package's lines and the
/// summary to the panel. Fails when a package fails, or when there is none.
fn validate_packages_to_panel(
    inputs: &[PathBuf],
    logs: &Arc<Mutex<Vec<LogMessage>>>,
) -> eyre::Result<ValidateCounts> {
    let counts = validate_inputs(inputs, |report| {
        for (kind, line) in report_lines(report) {
            push_panel_log(logs, line, report_line_level(kind));
        }
    })?;
    let level = if counts.failed == 0 { LogLevel::Success } else { LogLevel::Error };
    push_panel_log(logs, counts.summary_line(), level);
    counts.ensure_passed()?;
    Ok(counts)
}

/// A packing job's message once the packages it wrote are validated: `message` with the validation's count, or the
/// validation's error.
fn validate_written_packages(
    message: String,
    packages: &[PathBuf],
    logs: &Arc<Mutex<Vec<LogMessage>>>,
) -> eyre::Result<String> {
    let counts = validate_packages_to_panel(packages, logs).wrap_err("validate the packages written")?;
    Ok(format!("{message}; {} validated", plural_packages(counts.packages)))
}

fn plural_packages(count: usize) -> String {
    if count == 1 { "1 package".to_string() } else { format!("{count} packages") }
}

impl UddConvApp {
    pub fn spawn_task<F>(&self, name: String, task: F)
    where
        F: FnOnce() -> eyre::Result<String> + Send + 'static,
    {
        let is_converting = self.is_converting.clone();
        let logs = self.logs.clone();
        let cancel = self.cancel_conversion.clone();

        match is_converting.lock() {
            Ok(mut busy) => {
                if *busy {
                    self.push_log(
                        format!("Cannot start {} while another task is running.", name),
                        LogLevel::Error,
                    );
                    return;
                }
                *busy = true;
                cancel.store(false, Ordering::Relaxed);
            }
            Err(_) => {
                self.push_log(
                    format!("Cannot start {} because the task state is unavailable.", name),
                    LogLevel::Error,
                );
                return;
            }
        }

        std::thread::spawn(move || {
            let _reset_busy = ConvertingFlagReset {
                is_converting: is_converting.clone(),
            };

            {
                let Ok(mut logs) = logs.lock() else {
                    return;
                };
                logs.push(LogMessage {
                    text: format!("[RUN] Starting: {}", name),
                    level: LogLevel::Info,
                });
            }

            let result = {
                let _stdout_capture = crate::logic::panel_logger::capture_stdout_to_panel(logs.clone());
                match panic::catch_unwind(AssertUnwindSafe(task)) {
                    Ok(result) => result,
                    Err(payload) if payload.is::<ConversionCancelled>() => {
                        Err(eyre::eyre!("conversion cancelled"))
                    }
                    Err(_) => Err(eyre::eyre!("task panicked")),
                }
            };

            {
                let Ok(mut logs) = logs.lock() else {
                    return;
                };
                let (text, level) = match result {
                    Ok(msg) => (format!("[DONE] {}: {}", name, msg), LogLevel::Success),
                    Err(e) => (format!("[ERR] {}: {}", name, e), LogLevel::Error),
                };
                logs.push(LogMessage { text, level });
            }
        });
    }

    /// [`Self::spawn_task`] for a packing job that writes `packages`, which it validates once it succeeds when
    /// Validate packages after packing is on.
    fn spawn_packing_task<F>(&self, name: String, packages: Vec<PathBuf>, task: F)
    where
        F: FnOnce() -> eyre::Result<String> + Send + 'static,
    {
        let validate = self.settings.validate_after_packing.then_some(packages);
        let logs = self.logs.clone();
        self.spawn_task(name, move || {
            let message = task()?;
            match &validate {
                Some(packages) => validate_written_packages(message, packages, &logs),
                None => Ok(message),
            }
        });
    }

    /// Runs an asset packing job on its card, which writes `packages` and validates them once it succeeds when
    /// Validate packages after packing is on.
    fn spawn_asset_task<F>(&self, task_id: AssetPackTask, name: String, packages: Vec<PathBuf>, task: F)
    where
        F: FnOnce(AssetProgressReporter) -> eyre::Result<String> + Send + 'static,
    {
        if self.is_busy() {
            self.push_log(
                format!("Cannot start {} while another task is running.", name),
                LogLevel::Error,
            );
            return;
        }

        let reporter = AssetProgressReporter {
            task: task_id,
            progress: self.asset_progress.clone(),
            cancel: self.cancel_conversion.clone(),
        };
        let reporter_for_task = reporter.clone();
        let validate = self.settings.validate_after_packing.then_some(packages);
        let logs = self.logs.clone();
        reporter.start();
        self.spawn_task(name, move || {
            let result = match panic::catch_unwind(AssertUnwindSafe(|| {
                task(reporter_for_task.clone())
            })) {
                Ok(result) => result,
                Err(payload) if payload.is::<ConversionCancelled>() => {
                    Err(eyre::eyre!("conversion cancelled"))
                }
                Err(payload) => panic::resume_unwind(payload),
            };
            let result = result.and_then(|message| match &validate {
                Some(packages) => {
                    reporter_for_task.set(AssetPackProgressState::Running, VALIDATING_PROGRESS, "Validating");
                    validate_written_packages(message, packages, &logs)
                }
                None => Ok(message),
            });
            match &result {
                Ok(_) => reporter_for_task.finish(),
                Err(_) if reporter_for_task.cancel_requested() => reporter_for_task.cancelled(),
                Err(_) => reporter_for_task.fail(),
            }
            result
        });
    }

    pub fn convert_tex_art_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_cc.uddp");
        self.spawn_asset_task(AssetPackTask::TexArtCc, "CC Art Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;

            let compression = texture_compression(
                settings.opt_tex_art_cc,
                settings.zstd_tex_art_cc,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_filter_active(settings.upscale_tex_art_cc)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches_and_progress(
                &sources, &output,
                &TexArtCcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale: settings.upscale_tex_art_cc,
                    upscale_passes: Vec::new(),
                    upscale_profile,
                    pixel_format: match settings.opt_tex_art_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: bc7_rdo_lambda(&settings, settings.opt_tex_art_cc),
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                    source_preference: udd_conv::classic_sources::SourceFormatPreference::Uop,
                },
                &classic_patch_options(&settings),
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_cc.uddp");
        self.spawn_asset_task(AssetPackTask::TexLandCc, "CC Texmaps Packing".to_string(), vec![output.clone()], move |progress| {
            if matches!(settings.opt_tex_land_cc, TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd) {
                eyre::bail!("{CLASSIC_LAND_LOSSY_REFUSAL}");
            }
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;

            let compression = texture_compression(
                settings.opt_tex_land_cc,
                settings.zstd_tex_land_cc,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_config_active(settings.upscale_tex_land_cc_64)
                || upscale_config_active(settings.upscale_tex_land_cc_128)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_texmaps_mul_to_tex_land_cc_uddp_with_patches_and_progress(
                &sources[0], &output,
                &TexLandCcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale_64: settings.upscale_tex_land_cc_64,
                    upscale_128: settings.upscale_tex_land_cc_128,
                    upscale_64_passes: Vec::new(),
                    upscale_128_passes: Vec::new(),
                    upscale_profile,
                    pixel_format: PagePixelFormat::Rgba8888,
                    bc7_rdo_lambda: NO_BC7_RDO_LAMBDA,
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                },
                &classic_patch_options(&settings),
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_art_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_ec.uddp");
        self.spawn_asset_task(AssetPackTask::TexArtEc, "EC Art Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;

            let compression = texture_compression(
                settings.opt_tex_art_ec,
                settings.zstd_tex_art_ec,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_filter_active(settings.upscale_tex_art_ec)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources_with_progress(
                &sources, &output,
                &TexArtEcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale: settings.upscale_tex_art_ec,
                    upscale_passes: Vec::new(),
                    upscale_profile,
                    pixel_format: match settings.opt_tex_art_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: bc7_rdo_lambda(&settings, settings.opt_tex_art_ec),
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                },
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_ec.uddp");
        self.spawn_asset_task(AssetPackTask::TexLandEc, "EC Land Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;
            let terrain_overrides_kdl = conversion_table(&settings, EC_TERRAIN_OVERRIDES_KDL)?;
            let transcode_kdl = conversion_table(&settings, TERRAIN_TRANSCODE_KDL)?;

            let compression = texture_compression(
                settings.opt_tex_land_ec,
                settings.zstd_tex_land_ec,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_config_active(settings.upscale_tex_land_ec_64)
                || upscale_config_active(settings.upscale_tex_land_ec_128)
                || upscale_config_active(settings.upscale_tex_land_ec_256)
                || upscale_config_active(settings.upscale_tex_land_ec_512)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources_with_progress(
                &sources, &output,
                &TexLandEcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale_64: settings.upscale_tex_land_ec_64,
                    upscale_128: settings.upscale_tex_land_ec_128,
                    upscale_256: settings.upscale_tex_land_ec_256,
                    upscale_512: settings.upscale_tex_land_ec_512,
                    upscale_64_passes: Vec::new(),
                    upscale_128_passes: Vec::new(),
                    upscale_256_passes: Vec::new(),
                    upscale_512_passes: Vec::new(),
                    upscale_profile,
                    pixel_format: match settings.opt_tex_land_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    // The lossy Enhanced land is BC7 without RDO.
                    bc7_rdo_lambda: NO_BC7_RDO_LAMBDA,
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                    transcode_kdl_path: Some(transcode_kdl),
                    terrain_overrides_kdl_path: Some(terrain_overrides_kdl),
                },
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tilemeta(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tilemeta.uddp");
        self.spawn_asset_task(AssetPackTask::TileMeta, "Tilemeta Packing".to_string(), vec![output.clone()], move |progress| {
            ensure_output_parent(&output)?;
            match (settings.cc_dir.as_ref(), settings.ec_dir.as_ref()) {
                (Some(cc_dir), Some(ec_dir)) => {
                    build_tilemeta_uddp_from_split_sources_with_progress(
                        cc_dir,
                        ec_dir,
                        &output,
                        &TileMetaBuildOptions {
                            use_ec_radarcol: false,
                            classic_patches: classic_patch_options(&settings),
                            package_compression: CompressionFlag::ZstdNoDictLevel(settings.zstd_tilemeta),
                        },
                        |build_progress| progress.build_progress(build_progress),
                    )?;
                }
                _ => {
                    let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
                    if sources.is_empty() { eyre::bail!("No source dirs"); }
                    build_tilemeta_uddp_from_sources_with_progress(
                        &sources,
                        &output,
                        &TileMetaBuildOptions {
                            use_ec_radarcol: false,
                            classic_patches: classic_patch_options(&settings),
                            package_compression: CompressionFlag::ZstdNoDictLevel(settings.zstd_tilemeta),
                        },
                        |build_progress| progress.build_progress(build_progress),
                    )?;
                }
            }
            Ok(format!("Wrote tilemeta.uddp to {}", output.display()))
        });
    }

    pub fn convert_mobile_anim_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("mobile_anim_cc.uddp");
        self.spawn_asset_task(AssetPackTask::MobileAnimCc, "CC Mobile Animation Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;

            let compression = texture_compression(
                settings.opt_mobile_anim_cc,
                settings.zstd_mobile_anim_cc,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_filter_active(settings.upscale_mobile_anim_cc)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_anim_mul_to_mobile_anim_cc_uddp_from_sources_with_progress(
                &sources,
                &output,
                &MobileAnimCcAtlasOptions {
                    atlas_width: CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: CC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: CC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
                    compression,
                    pixel_format: match settings.opt_mobile_anim_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: bc7_rdo_lambda(&settings, settings.opt_mobile_anim_cc),
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                    upscale_passes: upscale_filter_passes(settings.upscale_mobile_anim_cc),
                    upscale_profile,
                },
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!(
                "Wrote {} pages for {} packed frames out of {} total frames to {}",
                summary.page_count,
                summary.packed_frame_count,
                summary.frame_count,
                output.display()
            ))
        });
    }

    pub fn convert_mobile_anim_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("mobile_anim_ec.uddp");
        self.spawn_asset_task(AssetPackTask::MobileAnimEc, "EC Mobile Animation Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;
            // Without the table the packer writes empty item metadata, when the settings allow it.
            let metadata_path = match conversion_table(&settings, EC_MOBILE_ANIMATIONS_KDL) {
                Ok(path) => Some(path),
                Err(_) if settings.ec_mobile_anim_allow_missing_kdl => None,
                Err(error) => return Err(error),
            };

            let compression = texture_compression(
                settings.opt_mobile_anim_ec,
                settings.zstd_mobile_anim_ec,
            );
            let upscale_profile = load_upscale_profile(&settings)?;
            let extract_label = if upscale_filter_active(settings.upscale_mobile_anim_ec)
                || upscale_profile_active(&settings)
            {
                "Extracting and upscaling"
            } else {
                "Extracting"
            };

            let summary = convert_animationframe_uop_to_mobile_anim_ec_uddp_from_sources_with_progress(
                &sources,
                &output,
                &MobileAnimEcAtlasOptions {
                    atlas_width: EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: EC_MOBILE_ANIM_DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: EC_MOBILE_ANIM_DEFAULT_ATLAS_GUTTER,
                    compression,
                    pixel_format: match settings.opt_mobile_anim_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: bc7_rdo_lambda(&settings, settings.opt_mobile_anim_ec),
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                    upscale_passes: upscale_filter_passes(settings.upscale_mobile_anim_ec),
                    upscale_profile,
                    metadata_path,
                    allow_missing_metadata: settings.ec_mobile_anim_allow_missing_kdl,
                },
                |task_progress| progress.task_progress_with_extract_label(task_progress, extract_label),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!(
                "Wrote {} pages for {} packed frames out of {} total frames and {} animation records for {} bodies ({} without a sequence) to {}",
                summary.page_count,
                summary.packed_frame_count,
                summary.frame_count,
                summary.animation_count,
                summary.body_count,
                summary.layout.no_sequence_body_count,
                output.display()
            ))
        });
    }

    pub fn convert_tex_aux_ec(&self) {
        let settings = self.settings.clone();
        let tilemeta_path = self.get_input_uddp_path("tilemeta.uddp");
        let output = self.get_output_path("tex_aux_ec.uddp");
        self.spawn_asset_task(AssetPackTask::TexAuxEc, "EC Auxiliary Textures Packing".to_string(), vec![output.clone()], move |progress| {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No EC source dir configured"); }
            if !tilemeta_path.exists() {
                eyre::bail!("tilemeta.uddp not found in input UDDP directory. Pack Tilemeta first!");
            }
            ensure_output_parent(&output)?;
            let compression = texture_compression(
                settings.opt_tex_aux_ec,
                settings.zstd_tex_aux_ec,
            );
            progress.set(AssetPackProgressState::Running, 0.1, "Packing auxiliary EC textures");
            let summary = pack_tex_aux_ec(
                &sources,
                &tilemeta_path,
                &output,
                &TexAuxEcPackOptions {
                    compression,
                    pixel_format: match settings.opt_tex_aux_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: bc7_rdo_lambda(&settings, settings.opt_tex_aux_ec),
                    bc7_rdo_lookback_blocks: bc7_rdo_lookback_blocks(&settings),
                    upscale: settings.upscale_tex_aux_ec,
                    upscale_passes: Vec::new(),
                },
            )?;
            Ok(format!(
                "Packed {} auxiliary textures ({} refs considered) to {}",
                summary.included,
                summary.refs_considered,
                output.display()
            ))
        });
    }

    /// The gumps_cc.uddp of the output folder: what the Classic gumps job writes and the Enhanced one shares with.
    pub fn gumps_cc_output(&self) -> PathBuf {
        self.get_output_path(GUMPS_CC_DEFAULT_OUTPUT)
    }

    pub fn convert_gumps_cc(&self) {
        let settings = self.settings.clone();
        let output = self.gumps_cc_output();
        self.spawn_asset_task(AssetPackTask::GumpsCc, "Classic Gumps Packing".to_string(), vec![output.clone()], move |progress| {
            let summary = pack_gumps_cc(&settings, &output, &progress)?;
            Ok(describe_gumps_cc(&settings, &output, &summary))
        });
    }

    pub fn convert_gumps_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(GUMPS_EC_DEFAULT_OUTPUT);
        let gumps_cc = self.gumps_cc_output();
        self.spawn_asset_task(AssetPackTask::GumpsEc, "Enhanced Gumps Packing".to_string(), vec![output.clone()], move |progress| {
            let share = ec_gumps_share(settings.gumps_ec_sharing, &gumps_cc, gumps_cc.is_file());
            let summary = pack_gumps_ec(&settings, &output, &share, &progress)?;
            Ok(describe_gumps_ec(&settings, &output, &summary))
        });
    }

    /// Packs the Classic gumps, then the Enhanced gumps, which share with the new gumps_cc when sharing is chosen.
    pub fn convert_gumps_cc_and_ec(&self) {
        let settings = self.settings.clone();
        let cc_output = self.gumps_cc_output();
        let ec_output = self.get_output_path(GUMPS_EC_DEFAULT_OUTPUT);
        self.spawn_asset_task(
            AssetPackTask::GumpsEc,
            "Classic + Enhanced Gumps Packing".to_string(),
            vec![cc_output.clone(), ec_output.clone()],
            move |progress| {
                let (cc_summary, ec_summary) = pack_gumps_cc_and_ec(&settings, &cc_output, &ec_output, &progress)?;
                Ok(format!(
                    "{}. {}",
                    describe_gumps_cc(&settings, &cc_output, &cc_summary),
                    describe_gumps_ec(&settings, &ec_output, &ec_summary)
                ))
            },
        );
    }

    pub fn convert_hues(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(HUES_OUTPUT);
        self.spawn_asset_task(AssetPackTask::Hues, "Hues Packing".to_string(), vec![output.clone()], move |progress| {
            progress.set(AssetPackProgressState::Running, UNREPORTED_PACKING_PROGRESS, "Packing hues");
            pack_hues(&settings, &output)
        });
    }

    pub fn convert_world_lights(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(WORLD_LIGHTS_OUTPUT);
        self.spawn_asset_task(
            AssetPackTask::WorldLights,
            "World Lights Packing".to_string(),
            vec![output.clone()],
            move |progress| {
                progress.set(AssetPackProgressState::Running, UNREPORTED_PACKING_PROGRESS, "Packing world lights");
                pack_world_lights(&settings, &output)
            },
        );
    }

    pub fn convert_effects_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(EFFECTS_EC_OUTPUT);
        self.spawn_asset_task(
            AssetPackTask::EffectsEc,
            "Enhanced Effects Packing".to_string(),
            vec![output.clone()],
            move |progress| {
                progress.set(AssetPackProgressState::Running, UNREPORTED_PACKING_PROGRESS, "Packing Enhanced effects");
                pack_effects_ec_job(&settings, &output)
            },
        );
    }

    pub fn convert_paperdoll_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(PAPERDOLL_EC_OUTPUT);
        self.spawn_asset_task(
            AssetPackTask::PaperdollEc,
            "Enhanced Paperdoll Packing".to_string(),
            vec![output.clone()],
            move |progress| {
                progress.set(AssetPackProgressState::Running, UNREPORTED_PACKING_PROGRESS, "Packing Enhanced paperdoll");
                pack_paperdoll_ec_job(&settings, &output)
            },
        );
    }

    pub fn convert_map(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("map{}.uddp", map_id));
        self.spawn_packing_task(format!("Map {} Packing", map_id), vec![output.clone()], move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;
            let summary = convert_map_mul_to_uddp_from_sources_with_patches(
                &sources,
                &output,
                map_id,
                settings.map_preferences[map_id as usize],
                &classic_patch_options(&settings),
                ChunkCompression::default(),
            )?;
            Ok(format!("Wrote {} chunks to {}", summary.chunk_count, output.display()))
        });
    }

    pub fn convert_statics(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("statics{}.uddp", map_id));
        self.spawn_packing_task(format!("Statics {} Packing", map_id), vec![output.clone()], move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;
            let summary = convert_statics_mul_to_uddp_from_sources_with_patches(
                &sources,
                &output,
                map_id,
                &classic_patch_options(&settings),
                ChunkCompression::default(),
            )?;
            Ok(format!("Wrote {} chunks to {}", summary.chunk_count, output.display()))
        });
    }

    pub fn convert_radar(&self, map_id: u32) {
        let settings = self.settings.clone();
        let tilemeta_path = self.get_input_uddp_path("tilemeta.uddp");
        let output = self.get_output_path(&format!(
            "facet0{}.{}",
            map_id,
            settings.radar_format.extension()
        ));
        self.spawn_task(format!("RadarMap {} Generation", map_id), move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            if !tilemeta_path.exists() {
                eyre::bail!("tilemeta.uddp not found in input UDDP directory. Pack Tilemeta first!");
            }
            ensure_output_parent(&output)?;

            if settings.radar_format == RadarFormat::Bc7Ktx2 {
                let bc7_data = udd_conv::cc_radar::build_facet_radar_bc7_with_options(
                    &sources,
                    &tilemeta_path,
                    map_id,
                    settings.map_preferences[map_id as usize],
                    &classic_patch_options(&settings),
                )?;
                uot_texture_codecs::ktx2::write_ktx2_bc7_zstd(
                    bc7_data,
                    &output,
                    settings.radar_zstd,
                )?;
            } else {
                build_facet_radar_dds(
                    &sources,
                    &tilemeta_path,
                    &output,
                    map_id,
                    &RadarBuildOptions {
                        format: settings.radar_format,
                        zstd_level: settings.radar_zstd,
                        classic_patches: classic_patch_options(&settings),
                        map_source_preference: settings.map_preferences[map_id as usize],
                    },
                )?;
            }
            Ok(format!("Wrote radar texture to {}", output.display()))
        });
    }

    pub fn convert_all(&self, map_id: u32) {
        let settings = self.settings.clone();
        let tilemeta_path = self.get_input_uddp_path("tilemeta.uddp");
        let map_output = self.get_output_path(&format!("map{}.uddp", map_id));
        let statics_output = self.get_output_path(&format!("statics{}.uddp", map_id));
        // The radar texture is no package, so only the map and statics are validated.
        let packages = vec![map_output.clone(), statics_output.clone()];

        self.spawn_packing_task(format!("Full Map {} Batch", map_id), packages, move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            std::fs::create_dir_all(&settings.output_uddp_dir)?;

            // 1. Map
            convert_map_mul_to_uddp_from_sources_with_patches(
                &sources,
                &map_output,
                map_id,
                settings.map_preferences[map_id as usize],
                &classic_patch_options(&settings),
                ChunkCompression::default(),
            )?;

            // 2. Statics
            convert_statics_mul_to_uddp_from_sources_with_patches(
                &sources,
                &statics_output,
                map_id,
                &classic_patch_options(&settings),
                ChunkCompression::default(),
            )?;

            // 3. Radar
            if !tilemeta_path.exists() {
                return Ok(format!("Map {} and Statics {} complete, but RadarMap skipped (tilemeta.uddp missing in input dir).", map_id, map_id));
            }

            let radar_output = settings.output_uddp_dir.join(format!(
                "facet0{}.{}",
                map_id,
                settings.radar_format.extension()
            ));
            if settings.radar_format == RadarFormat::Bc7Ktx2 {
                let bc7_data = udd_conv::cc_radar::build_facet_radar_bc7_with_options(
                    &sources,
                    &tilemeta_path,
                    map_id,
                    settings.map_preferences[map_id as usize],
                    &classic_patch_options(&settings),
                )?;
                uot_texture_codecs::ktx2::write_ktx2_bc7_zstd(
                    bc7_data,
                    &radar_output,
                    settings.radar_zstd,
                )?;
            } else {
                build_facet_radar_dds(
                    &sources,
                    &tilemeta_path,
                    &radar_output,
                    map_id,
                    &RadarBuildOptions {
                        format: settings.radar_format,
                        zstd_level: settings.radar_zstd,
                        classic_patches: classic_patch_options(&settings),
                        map_source_preference: settings.map_preferences[map_id as usize],
                    },
                )?;
            }

            Ok(format!(
                "Map, Statics, and RadarMap for Map {} successfully generated.",
                map_id
            ))
        });
    }

    pub fn tool_info(&self) {
        let Some(file) = self.tool_file_1.clone() else {
            self.push_log("Select a package before running info.", LogLevel::Error);
            return;
        };
        let file_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        self.spawn_task(format!("Info: {}", file_name), move || {
            get_package_info_string(&file)
        });
    }

    pub fn tool_extract(&self) {
        let Some(file) = self.tool_file_1.clone() else {
            self.push_log("Select a package before extracting.", LogLevel::Error);
            return;
        };
        let file_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        self.spawn_task(format!("Extract: {}", file_name), move || {
            extract_package(&file, None)?;
            Ok(format!("Extracted to sidecar folder next to {}", file.display()))
        });
    }

    pub fn tool_export_csv(&self) {
        let Some(file) = self.tool_file_1.clone() else {
            self.push_log("Select a package before exporting CSV metadata.", LogLevel::Error);
            return;
        };
        let file_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        self.spawn_task(format!("Export CSV: {}", file_name), move || {
            export_csv_auto_report(&file, None)
        });
    }

    /// Validates the chosen packages and folders as `tool validate <PATHS>...` does, logging each package's result.
    pub fn tool_validate(&self) {
        if self.tool_validate_paths.is_empty() {
            self.push_log("Add packages or folders before validating.", LogLevel::Error);
            return;
        }
        let paths = self.tool_validate_paths.clone();
        let logs = self.logs.clone();
        self.spawn_task("Validate Packages".to_string(), move || {
            let counts = validate_packages_to_panel(&paths, &logs)?;
            Ok(format!("{} passed", plural_packages(counts.packages)))
        });
    }

    pub fn tool_hash_path(&self) {
        let value = self.tool_hash_value.trim();
        if value.is_empty() {
            self.push_log("Enter a virtual path before hashing.", LogLevel::Error);
            return;
        }
        self.push_log(hash_path_report(value), LogLevel::Info);
    }

    pub fn tool_diff(&self) {
        let Some(left) = self.tool_file_1.clone() else {
            self.push_log("Select package A before running diff.", LogLevel::Error);
            return;
        };
        let Some(right) = self.tool_file_2.clone() else {
            self.push_log("Select package B before running diff.", LogLevel::Error);
            return;
        };
        self.spawn_task("Diff Packages".to_string(), move || {
            diff_paths_report(&left, &right, DiffKind::Auto)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_bc7_does_not_apply_rdo() {
        let mut settings = AppSettings::default();
        settings.bc7_rdo_enabled = true;
        settings.bc7_rdo_lambda = 0.6;

        assert_eq!(bc7_rdo_lambda(&settings, TextureOptimization::Bc7), 0.0);
        assert_eq!(bc7_rdo_lambda(&settings, TextureOptimization::Bc7Zstd), 0.6);
    }

    #[test]
    fn disabled_bc7_rdo_overrides_supercompressed_bc7() {
        let mut settings = AppSettings::default();
        settings.bc7_rdo_enabled = false;
        settings.bc7_rdo_lambda = 0.6;

        assert_eq!(bc7_rdo_lambda(&settings, TextureOptimization::Bc7Zstd), 0.0);
    }

    /// The level `pack-gumps` and `pack-ec-gumps` give lossless single gumps without `--zstd=LEVEL`.
    const LOSSLESS_SINGLE_GUMP_ZSTD: CompressionFlag = CompressionFlag::ZstdNoDict;
    /// The level they give lossy single gumps and the atlas pages without `--zstd=LEVEL`.
    const DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD: CompressionFlag = CompressionFlag::ZstdNoDictLevel(19);
    const CHOSEN_GUMP_ZSTD_LEVEL: i32 = 12;

    #[test]
    fn default_gump_options_match_the_cli_without_flags() {
        let settings = AppSettings::default();

        let cc = cc_gumps_options(&settings, None);
        assert_eq!(cc.single_pixel_format, GumpPixelFormat::Rgba8888);
        assert_eq!(cc.compression, LOSSLESS_SINGLE_GUMP_ZSTD);
        assert_eq!(cc.atlas_compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);
        assert!(cc.paperdoll_upscale_passes.is_empty());
        assert!(cc.single_upscale_passes.is_empty());
        assert!(cc.upscale_profile.is_none());
        assert_eq!(cc.source_preference, udd_conv::classic_sources::SourceFormatPreference::Mul);

        let ec = ec_gumps_options(&settings, None, None);
        assert_eq!(ec.max_id, udd_conv::ec_gumps::EC_GUMP_DEFAULT_MAX_ID);
        assert_eq!(ec.single_pixel_format, GumpPixelFormat::Rgba8888);
        assert_eq!(ec.compression, LOSSLESS_SINGLE_GUMP_ZSTD);
        assert_eq!(ec.atlas_compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);
        assert!(ec.paperdoll_upscale_passes.is_empty());
        assert!(ec.single_upscale_passes.is_empty());
        assert_eq!(ec.share_with, None);
    }

    #[test]
    fn lossy_gumps_take_each_package_lossy_format_at_level_19() {
        let mut settings = AppSettings::default();
        settings.gumps_cc_bc1 = true;
        settings.gumps_ec_bc7 = true;

        let cc = cc_gumps_options(&settings, None);
        assert_eq!(cc.single_pixel_format, GumpPixelFormat::Bc1);
        assert_eq!(cc.compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);
        assert_eq!(cc.atlas_compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);

        let ec = ec_gumps_options(&settings, None, None);
        assert_eq!(ec.single_pixel_format, GumpPixelFormat::Bc7);
        assert_eq!(ec.compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);
        assert_eq!(ec.atlas_compression, DEFAULT_LOSSY_AND_ATLAS_GUMP_ZSTD);
    }

    #[test]
    fn a_chosen_gump_zstd_level_sets_single_gumps_and_atlas_pages() {
        let mut settings = AppSettings::default();
        settings.zstd_gumps_cc = Some(CHOSEN_GUMP_ZSTD_LEVEL);
        settings.zstd_gumps_ec = Some(CHOSEN_GUMP_ZSTD_LEVEL);
        settings.gumps_ec_bc7 = true;
        let chosen = CompressionFlag::ZstdNoDictLevel(CHOSEN_GUMP_ZSTD_LEVEL);

        let cc = cc_gumps_options(&settings, None);
        assert_eq!((cc.compression, cc.atlas_compression), (chosen, chosen));
        let ec = ec_gumps_options(&settings, None, None);
        assert_eq!((ec.compression, ec.atlas_compression), (chosen, chosen));
    }

    #[test]
    fn gump_upscale_filters_source_and_max_id_map_to_the_cli_options() {
        const MAX_ID: u32 = 70_000;
        let mut settings = AppSettings::default();
        settings.upscale_gumps_cc_paperdoll = UpscaleFilter::Xbrz2x;
        settings.upscale_gumps_cc_single = UpscaleFilter::Nearest2x;
        settings.upscale_gumps_ec_paperdoll = UpscaleFilter::Hq2xTrue;
        settings.upscale_gumps_ec_single = UpscaleFilter::Mmpx2x;
        settings.gumps_cc_source = udd_conv::classic_sources::SourceFormatPreference::Uop;
        settings.gumps_ec_max_id = MAX_ID;

        let cc = cc_gumps_options(&settings, None);
        assert_eq!(cc.paperdoll_upscale_passes, vec![UpscalePass::from(UpscaleFilter::Xbrz2x)]);
        assert_eq!(cc.single_upscale_passes, vec![UpscalePass::from(UpscaleFilter::Nearest2x)]);
        assert_eq!(cc.source_preference, udd_conv::classic_sources::SourceFormatPreference::Uop);

        let ec = ec_gumps_options(&settings, None, None);
        assert_eq!(ec.paperdoll_upscale_passes, vec![UpscalePass::from(UpscaleFilter::Hq2xTrue)]);
        assert_eq!(ec.single_upscale_passes, vec![UpscalePass::from(UpscaleFilter::Mmpx2x)]);
        assert_eq!(ec.max_id, MAX_ID);
    }

    #[test]
    fn enhanced_gumps_read_the_gumpart_folder_over_the_enhanced_client() {
        let mut settings = AppSettings::default();
        settings.cc_dir = Some(PathBuf::from("/uo/classic"));
        settings.ec_dir = Some(PathBuf::from("/uo/enhanced"));

        assert_eq!(cc_gumps_source_dirs(&settings), vec![PathBuf::from("/uo/classic"), PathBuf::from("/uo/enhanced")]);
        assert_eq!(ec_gumps_source_dirs(&settings), vec![PathBuf::from("/uo/enhanced")]);

        settings.gumps_ec_gumpart_dir = Some(PathBuf::from("/extracted/gumpart"));
        assert_eq!(ec_gumps_source_dirs(&settings), vec![PathBuf::from("/extracted/gumpart")]);
    }

    #[test]
    fn enhanced_gumps_share_only_when_chosen_and_a_gumps_cc_is_there() {
        let gumps_cc = Path::new("/out/gumps_cc.uddp");

        let shared = ec_gumps_share(GumpsEcSharing::ShareWithGumpsCc, gumps_cc, true);
        assert_eq!(shared, EcGumpsShare::With(gumps_cc.to_path_buf()));
        assert_eq!(shared.share_with(), Some(gumps_cc.to_path_buf()));

        let missing = ec_gumps_share(GumpsEcSharing::ShareWithGumpsCc, gumps_cc, false);
        assert_eq!(missing, EcGumpsShare::MissingGumpsCc(gumps_cc.to_path_buf()));
        assert_eq!(missing.share_with(), None);

        for available in [false, true] {
            let standalone = ec_gumps_share(GumpsEcSharing::Standalone, gumps_cc, available);
            assert_eq!(standalone, EcGumpsShare::Standalone);
            assert_eq!(standalone.share_with(), None);
        }
    }

    #[test]
    fn the_share_choice_reaches_the_enhanced_options() {
        let settings = AppSettings::default();
        let gumps_cc = PathBuf::from("/out/gumps_cc.uddp");

        let ec = ec_gumps_options(&settings, None, Some(gumps_cc.clone()));
        assert_eq!(ec.share_with, Some(gumps_cc));
    }

    const CHOSEN_PACKAGE_ZSTD_LEVEL: i32 = 15;

    #[test]
    fn default_hues_lights_and_effects_options_match_the_cli_without_flags() {
        let settings = AppSettings::default();

        let hues = hues_options(&settings);
        assert_eq!(hues.compression, CompressionFlag::ZstdNoDict);
        assert_eq!(hues.ramp, udd_conv::hues::HueRampMode::Linear);

        let lights = world_lights_options(&settings);
        assert_eq!(lights.compression, CompressionFlag::ZstdNoDict);
        assert!(!lights.classic_patches.verdata);

        let effects = effects_ec_options(&settings);
        assert!(!effects.bc7, "the Enhanced effects default to RGBA8, as pack-ec-effects without --bc7");
        assert_eq!(effects.compression, CompressionFlag::ZstdNoDict);
        assert!(!effects.strict);
        assert_eq!(effects.report, None);

        assert_eq!(paperdoll_ec_options(&settings).compression, PaperdollEcPackOptions::default().compression);
    }

    #[test]
    fn chosen_hues_lights_and_effects_options_map_to_the_cli_flags() {
        let mut settings = AppSettings::default();
        settings.hues_ramp = crate::models::HueRamp::Flat;
        settings.zstd_hues = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        settings.zstd_lights = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        settings.include_verdata = true;
        settings.effects_ec_bc7 = true;
        settings.zstd_effects_ec = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        settings.effects_ec_strict = true;
        settings.effects_ec_report = Some(PathBuf::from("/out/effects_ec_findings.txt"));
        let chosen = CompressionFlag::ZstdNoDictLevel(CHOSEN_PACKAGE_ZSTD_LEVEL);

        let hues = hues_options(&settings);
        assert_eq!((hues.compression, hues.ramp), (chosen, udd_conv::hues::HueRampMode::Flat));
        let lights = world_lights_options(&settings);
        assert_eq!(lights.compression, chosen);
        assert!(lights.classic_patches.verdata, "--use-verdata");
        let effects = effects_ec_options(&settings);
        assert!(effects.bc7 && effects.strict);
        assert_eq!(effects.compression, chosen);
        assert_eq!(effects.report, Some(PathBuf::from("/out/effects_ec_findings.txt")));

        settings.zstd_paperdoll_ec = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        assert_eq!(paperdoll_ec_options(&settings).compression, chosen);
    }

    /// `--no-compression` wins over `--zstd=LEVEL`, as in `pack-hues` and `pack-lights`.
    #[test]
    fn no_compression_stores_hues_and_lights_raw_whatever_the_level() {
        let mut settings = AppSettings::default();
        settings.hues_no_compression = true;
        settings.lights_no_compression = true;
        settings.zstd_hues = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);

        assert_eq!(hues_options(&settings).compression, CompressionFlag::None);
        assert_eq!(world_lights_options(&settings).compression, CompressionFlag::None);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("udd_conv_gui_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// The hues job reads a hues.mul from either folder first, else the Enhanced Hues.uop.
    #[test]
    fn hues_read_a_hues_mul_first_then_the_enhanced_hues_uop() {
        let root = scratch_dir("hues_source");
        let (classic, enhanced) = (root.join("classic"), root.join("enhanced"));
        std::fs::create_dir_all(&classic).expect("create classic dir");
        std::fs::create_dir_all(&enhanced).expect("create enhanced dir");
        let mut settings = AppSettings::default();
        assert_eq!(hues_source(&settings), HuesSource::NoFolder);

        settings.cc_dir = Some(classic.clone());
        settings.ec_dir = Some(enhanced.clone());
        assert_eq!(hues_source(&settings), HuesSource::Missing);
        std::fs::write(enhanced.join(HUES_UOP_CAPITALIZED_NAME), b"").expect("write Hues.uop");
        assert_eq!(hues_source(&settings), HuesSource::Uop(enhanced.join(HUES_UOP_CAPITALIZED_NAME)));
        std::fs::write(classic.join(HUES_MUL_NAME), b"").expect("write hues.mul");
        assert_eq!(hues_source(&settings), HuesSource::Mul(classic.join(HUES_MUL_NAME)));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn report_lines_take_the_panel_colours() {
        assert_eq!(report_line_level(ReportLineKind::Passed), LogLevel::Success);
        assert_eq!(report_line_level(ReportLineKind::Failed), LogLevel::Error);
        assert_eq!(report_line_level(ReportLineKind::Error), LogLevel::Error);
        assert_eq!(report_line_level(ReportLineKind::Detail), LogLevel::Info);
        assert_eq!(report_line_level(ReportLineKind::Note), LogLevel::Info);
    }

    /// A broken package logs a FAIL line and the summary in red and fails the validation; a folder with no package
    /// fails too.
    #[test]
    fn validation_logs_each_package_and_fails_on_a_broken_one() {
        const BROKEN_PACKAGE_BYTES: &[u8] = b"not a package";
        let dir = scratch_dir("validate");
        let broken = dir.join(HUES_OUTPUT);
        std::fs::write(&broken, BROKEN_PACKAGE_BYTES).expect("write broken package");
        let logs = Arc::new(Mutex::new(Vec::new()));

        let error = validate_written_packages("Wrote it".to_string(), &[broken.clone()], &logs)
            .expect_err("a broken package fails");

        assert!(format!("{error:#}").contains("1 of 1 packages failed validation"), "{error:#}");
        let logs = logs.lock().expect("logs");
        let first = logs.first().expect("a status line");
        assert!(first.text.starts_with("FAIL") && first.text.contains(&broken.display().to_string()), "{}", first.text);
        assert_eq!(first.level, LogLevel::Error);
        let last = logs.last().expect("a summary line");
        assert_eq!(last.text, "Validated 1 packages: 0 passed, 1 failed");
        assert_eq!(last.level, LogLevel::Error);
        drop(logs);

        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).expect("create empty dir");
        assert!(validate_packages_to_panel(&[empty], &Arc::new(Mutex::new(Vec::new()))).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn test_reporter(task: AssetPackTask) -> AssetProgressReporter {
        AssetProgressReporter {
            task,
            progress: Arc::new(Mutex::new(HashMap::new())),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn build_progress_at(phase: BuildProgressPhase) -> BuildProgress {
        BuildProgress { phase, completed: 0, total: 1, active_file: None }
    }

    /// A stop unwinds from the gump packers' callbacks while they extract, pack and compress, but not once the
    /// package file is being written.
    #[test]
    fn a_stop_unwinds_from_the_progress_callbacks_until_the_package_is_written() {
        let progress = test_reporter(AssetPackTask::GumpsEc);
        progress.cancel.store(true, Ordering::Relaxed);
        let unwinds = |callback: &dyn Fn()| {
            panic::catch_unwind(AssertUnwindSafe(callback)).err().is_some_and(|payload| payload.is::<ConversionCancelled>())
        };

        assert!(unwinds(&|| progress.task_progress_with_extract_label(
            AssetTaskProgress { stage: AssetTaskProgressStage::Extracting, completed: 1, total: 2 },
            "Extracting",
        )));
        for phase in [BuildProgressPhase::TrainingDictionaries, BuildProgressPhase::CompressingFiles] {
            assert!(unwinds(&|| progress.build_progress(build_progress_at(phase))), "{phase:?}");
        }
        assert!(!unwinds(&|| progress.build_progress(build_progress_at(BuildProgressPhase::Assembling))));
    }

    /// The gump extraction moves the bar.
    #[test]
    fn gump_extraction_moves_the_bar() {
        const HALF_EXTRACTED: f32 = 0.10;
        let progress = test_reporter(AssetPackTask::GumpsEc);

        progress.task_progress_with_extract_label(
            AssetTaskProgress { stage: AssetTaskProgressStage::Extracting, completed: 1, total: 2 },
            "Extracting",
        );

        let state = progress.progress.lock().expect("progress map")[&AssetPackTask::GumpsEc].clone();
        assert_eq!(state.state, AssetPackProgressState::Running);
        assert!((state.fraction - HALF_EXTRACTED).abs() < f32::EPSILON, "{}", state.fraction);
        assert_eq!(state.text, "Extracting 1/2");
    }

    const CC_CLIENT_DIR_ENV: &str = "UO_CC_CLIENT_DIR";
    const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
    /// The absolute folder the real-data gump test writes gumps_cc.uddp and gumps_ec.uddp to, outside the clients.
    const GUMPS_OUTPUT_DIR_ENV: &str = "UDD_CONV_GUI_GUMPS_OUTPUT_DIR";

    fn env_dir(name: &str) -> PathBuf {
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
    }

    /// The combined gumps job on the real clients, lossless and sharing: the Classic gumps, then the Enhanced gumps
    /// leaving out the Classic twins.
    #[test]
    #[ignore = "needs UO_CC_CLIENT_DIR, UO_EC_CLIENT_DIR and UDD_CONV_GUI_GUMPS_OUTPUT_DIR"]
    fn packs_real_classic_then_enhanced_gumps_sharing_the_classic_twins() {
        let cc_dir = env_dir(CC_CLIENT_DIR_ENV);
        let ec_dir = env_dir(EC_CLIENT_DIR_ENV);
        let output_dir = env_dir(GUMPS_OUTPUT_DIR_ENV);
        assert!(output_dir.is_absolute(), "{GUMPS_OUTPUT_DIR_ENV} must be absolute: {}", output_dir.display());
        for client_dir in [&cc_dir, &ec_dir] {
            assert!(!output_dir.starts_with(client_dir), "the output folder is inside {}", client_dir.display());
        }
        let mut settings = AppSettings::default();
        settings.cc_dir = Some(cc_dir);
        settings.ec_dir = Some(ec_dir);
        settings.output_uddp_dir = output_dir.clone();
        let cc_output = output_dir.join(GUMPS_CC_DEFAULT_OUTPUT);
        let ec_output = output_dir.join(GUMPS_EC_DEFAULT_OUTPUT);
        let progress = test_reporter(AssetPackTask::GumpsEc);

        let (cc_summary, ec_summary) =
            pack_gumps_cc_and_ec(&settings, &cc_output, &ec_output, &progress).expect("pack the gumps");

        assert!(cc_summary.single_gump_count > 0);
        assert!(ec_summary.single_gump_count > 0);
        assert!(ec_summary.single_shared_classic_count > 0, "the Enhanced gumps shared nothing");
        let states = progress.progress.lock().expect("progress map");
        assert_eq!(states[&AssetPackTask::GumpsCc].state, AssetPackProgressState::Succeeded);
    }

    /// The absolute folder the real-data hues, world lights, Enhanced effects and Enhanced paperdoll test writes to:
    /// the jobs' defaults in it, and the chosen options in its `options` folder.
    const PACKAGES_OUTPUT_DIR_ENV: &str = "UDD_CONV_GUI_PACKAGES_OUTPUT_DIR";
    /// The folder of the chosen options' packages, inside the output folder.
    const CHOSEN_OPTIONS_DIR: &str = "options";
    /// The Enhanced effects findings file the chosen options write, inside the chosen options' folder.
    const EFFECTS_EC_REPORT_NAME: &str = "effects_ec_findings.txt";
    /// The packages the real-data hues, world lights, Enhanced effects and Enhanced paperdoll test writes to each
    /// folder.
    const PACKAGES_PER_FOLDER: usize = 4;

    fn real_output_dir(env: &str, client_dirs: &[&PathBuf]) -> PathBuf {
        let output_dir = env_dir(env);
        assert!(output_dir.is_absolute(), "{env} must be absolute: {}", output_dir.display());
        for client_dir in client_dirs {
            assert!(!output_dir.starts_with(client_dir), "the output folder is inside {}", client_dir.display());
        }
        output_dir
    }

    /// The hues, world lights, Enhanced effects and Enhanced paperdoll jobs on the real clients, with the defaults and
    /// with chosen options, each through the GUI's job code; then every package validated.
    #[test]
    #[ignore = "needs UO_CC_CLIENT_DIR, UO_EC_CLIENT_DIR and UDD_CONV_GUI_PACKAGES_OUTPUT_DIR"]
    fn packs_real_hues_lights_and_effects() {
        // Shows the jobs' messages with --nocapture; another test may have installed it already.
        let _ = uot_logging::install_paris_logger();
        let cc_dir = env_dir(CC_CLIENT_DIR_ENV);
        let ec_dir = env_dir(EC_CLIENT_DIR_ENV);
        let output_dir = real_output_dir(PACKAGES_OUTPUT_DIR_ENV, &[&cc_dir, &ec_dir]);
        let mut settings = AppSettings::default();
        settings.cc_dir = Some(cc_dir);
        settings.ec_dir = Some(ec_dir);

        let chosen_dir = output_dir.join(CHOSEN_OPTIONS_DIR);
        let mut chosen = settings.clone();
        chosen.hues_ramp = crate::models::HueRamp::Flat;
        chosen.zstd_hues = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        chosen.lights_no_compression = true;
        chosen.include_verdata = true;
        chosen.effects_ec_bc7 = true;
        chosen.zstd_effects_ec = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);
        chosen.effects_ec_report = Some(chosen_dir.join(EFFECTS_EC_REPORT_NAME));
        chosen.zstd_paperdoll_ec = Some(CHOSEN_PACKAGE_ZSTD_LEVEL);

        for (settings, dir) in [(&settings, &output_dir), (&chosen, &chosen_dir)] {
            for message in [
                pack_hues(settings, &dir.join(HUES_OUTPUT)),
                pack_world_lights(settings, &dir.join(WORLD_LIGHTS_OUTPUT)),
                pack_effects_ec_job(settings, &dir.join(EFFECTS_EC_OUTPUT)),
                pack_paperdoll_ec_job(settings, &dir.join(PAPERDOLL_EC_OUTPUT)),
            ] {
                log::info!("{}", message.expect("pack"));
            }
            let logs = Arc::new(Mutex::new(Vec::new()));
            let counts = validate_packages_to_panel(&[dir.clone()], &logs).expect("validate");
            assert!(counts.packages >= PACKAGES_PER_FOLDER, "{counts:?}");
        }
        assert!(chosen_dir.join(EFFECTS_EC_REPORT_NAME).is_file());
    }

    /// The bar of a reporter's card.
    fn card_state(progress: &AssetProgressReporter) -> Option<AssetPackProgress> {
        progress.progress.lock().expect("progress map").get(&progress.task).cloned()
    }

    /// Stop pressed while the real Enhanced gumps are extracted: a thread watching the bar, as the Stop button does,
    /// requests it; the job ends with the bar moved and no package written.
    #[test]
    #[ignore = "needs UO_EC_CLIENT_DIR and UDD_CONV_GUI_GUMPS_OUTPUT_DIR"]
    fn a_stop_ends_the_real_enhanced_gumps_job_without_a_package() {
        // Shows the jobs' messages with --nocapture; another test may have installed it already.
        let _ = uot_logging::install_paris_logger();
        /// Where the bar stands when the watcher presses Stop: halfway through the extraction.
        const STOP_AT_FRACTION: f32 = 0.10;
        const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);
        let ec_dir = env_dir(EC_CLIENT_DIR_ENV);
        let output_dir = real_output_dir(GUMPS_OUTPUT_DIR_ENV, &[&ec_dir]);
        let mut settings = AppSettings::default();
        settings.ec_dir = Some(ec_dir);
        let output = output_dir.join(GUMPS_EC_DEFAULT_OUTPUT);
        assert!(!output.exists(), "{} is there before the job", output.display());

        let progress = test_reporter(AssetPackTask::GumpsEc);
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (progress, done) = (progress.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    if let Some(state) = card_state(&progress).filter(|state| state.fraction >= STOP_AT_FRACTION) {
                        progress.cancel.store(true, Ordering::Relaxed);
                        return Some(state);
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                None
            })
        };

        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            pack_gumps_ec(&settings, &output, &EcGumpsShare::Standalone, &progress)
        }));
        done.store(true, Ordering::Relaxed);

        let stopped_at = watcher.join().expect("watcher").expect("the bar reached the stop point");
        log::info!("Stop pressed at {:.3}: {}", stopped_at.fraction, stopped_at.text);
        assert!(result.err().is_some_and(|payload| payload.is::<ConversionCancelled>()), "the job did not stop");
        assert!(!output.exists(), "a stopped job left {}", output.display());

        // The package's compression reports in a burst at its end, too fast for a watcher: the build callback requests
        // the stop once half the package is compressed, before the reporter sees it.
        let progress = test_reporter(AssetPackTask::GumpsEc);
        let options = ec_gumps_options(&settings, None, None);
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            convert_ec_gumps_to_uddp_from_sources_with_progress(
                &ec_gumps_source_dirs(&settings),
                &output,
                &options,
                |task_progress| progress.task_progress_with_extract_label(task_progress, "Extracting"),
                |build_progress| {
                    if build_progress.phase == BuildProgressPhase::CompressingFiles
                        && build_progress.completed * 2 >= build_progress.total
                    {
                        progress.cancel.store(true, Ordering::Relaxed);
                    }
                    progress.build_progress(build_progress)
                },
            )
        }));

        let stopped_at = card_state(&progress).expect("the bar moved");
        log::info!("Stop requested mid-compression; the bar stands at {:.3}: {}", stopped_at.fraction, stopped_at.text);
        assert!(result.err().is_some_and(|payload| payload.is::<ConversionCancelled>()), "the job did not stop");
        assert!(stopped_at.text.starts_with("Compressing package"), "{}", stopped_at.text);
        assert!(!output.exists(), "a stopped job left {}", output.display());
    }
}
