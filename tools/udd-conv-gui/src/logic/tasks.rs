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
    convert_gumps_to_uddp_from_sources_with_patches_and_options, CcGumpsBuildSummary, CcGumpsOptions,
    CC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_CC_DEFAULT_OUTPUT,
};
use udd_conv::ec_gumps::{
    convert_ec_gumps_to_uddp_from_sources, EcGumpsBuildSummary, EcGumpsOptions,
    EC_CLASSIC_COLOUR_SPACE_SINGLE_GUMP_FORMAT, EC_LOSSY_SINGLE_GUMP_FORMAT, GUMPS_EC_DEFAULT_OUTPUT,
};
use udd_conv::page_compression::LOSSLESS_PAGE_ZSTD_LEVEL;
use udd_conv::single_gumps::{default_single_gump_compression, GumpPixelFormat};
use udd_conv::upscale_profile::UpscaleProfile;
use udd_conv::package_progress::{AssetTaskProgress, AssetTaskProgressStage};
use udd_conv_cli::{
    package_info::get_package_info_string,
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
        self.panic_if_cancelled();
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

/// The gump packers report no progress, so their bar shows this much while they run.
const GUMPS_PACKING_PROGRESS: f32 = 0.1;

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

/// Packs the Classic gumps as `pack-gumps` does.
fn pack_gumps_cc(settings: &AppSettings, output: &Path) -> eyre::Result<CcGumpsBuildSummary> {
    let sources = cc_gumps_source_dirs(settings);
    if sources.is_empty() { eyre::bail!("No Classic or Enhanced source dir configured"); }
    ensure_output_parent(output)?;
    let options = cc_gumps_options(settings, load_upscale_profile(settings)?);
    convert_gumps_to_uddp_from_sources_with_patches_and_options(
        &sources,
        output,
        &classic_patch_options(settings),
        &options,
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

/// Packs the Enhanced gumps as `pack-ec-gumps` does, sharing as `share` says.
fn pack_gumps_ec(settings: &AppSettings, output: &Path, share: &EcGumpsShare) -> eyre::Result<EcGumpsBuildSummary> {
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
    convert_ec_gumps_to_uddp_from_sources(&sources, output, &options)
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
    cc_progress.set(AssetPackProgressState::Running, GUMPS_PACKING_PROGRESS, "Packing Classic gumps");
    let cc_summary = match pack_gumps_cc(settings, cc_output) {
        Ok(summary) => {
            cc_progress.finish();
            summary
        }
        Err(error) => {
            cc_progress.fail();
            return Err(error.wrap_err("pack the Classic gumps"));
        }
    };
    progress.panic_if_cancelled();
    progress.set(AssetPackProgressState::Running, GUMPS_PACKING_PROGRESS, "Packing Enhanced gumps");
    let share = ec_gumps_share(settings.gumps_ec_sharing, cc_output, true);
    let ec_summary = pack_gumps_ec(settings, ec_output, &share)?;
    Ok((cc_summary, ec_summary))
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

    fn spawn_asset_task<F>(&self, task_id: AssetPackTask, name: String, task: F)
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
        self.spawn_asset_task(AssetPackTask::TexArtCc, "CC Art Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::TexLandCc, "CC Texmaps Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::TexArtEc, "EC Art Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::TexLandEc, "EC Land Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::TileMeta, "Tilemeta Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::MobileAnimCc, "CC Mobile Animation Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::MobileAnimEc, "EC Mobile Animation Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::TexAuxEc, "EC Auxiliary Textures Packing".to_string(), move |progress| {
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
        self.spawn_asset_task(AssetPackTask::GumpsCc, "Classic Gumps Packing".to_string(), move |progress| {
            progress.set(AssetPackProgressState::Running, GUMPS_PACKING_PROGRESS, "Packing Classic gumps");
            let summary = pack_gumps_cc(&settings, &output)?;
            Ok(describe_gumps_cc(&settings, &output, &summary))
        });
    }

    pub fn convert_gumps_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path(GUMPS_EC_DEFAULT_OUTPUT);
        let gumps_cc = self.gumps_cc_output();
        self.spawn_asset_task(AssetPackTask::GumpsEc, "Enhanced Gumps Packing".to_string(), move |progress| {
            let share = ec_gumps_share(settings.gumps_ec_sharing, &gumps_cc, gumps_cc.is_file());
            progress.set(AssetPackProgressState::Running, GUMPS_PACKING_PROGRESS, "Packing Enhanced gumps");
            let summary = pack_gumps_ec(&settings, &output, &share)?;
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

    pub fn convert_map(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("map{}.uddp", map_id));
        self.spawn_task(format!("Map {} Packing", map_id), move || {
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
        self.spawn_task(format!("Statics {} Packing", map_id), move || {
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

        self.spawn_task(format!("Full Map {} Batch", map_id), move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            std::fs::create_dir_all(&settings.output_uddp_dir)?;

            // 1. Map
            let map_output = settings.output_uddp_dir.join(format!("map{}.uddp", map_id));
            convert_map_mul_to_uddp_from_sources_with_patches(
                &sources,
                &map_output,
                map_id,
                settings.map_preferences[map_id as usize],
                &classic_patch_options(&settings),
                ChunkCompression::default(),
            )?;

            // 2. Statics
            let statics_output = settings
                .output_uddp_dir
                .join(format!("statics{}.uddp", map_id));
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
        let progress = AssetProgressReporter {
            task: AssetPackTask::GumpsEc,
            progress: Arc::new(Mutex::new(HashMap::new())),
            cancel: Arc::new(AtomicBool::new(false)),
        };

        let (cc_summary, ec_summary) =
            pack_gumps_cc_and_ec(&settings, &cc_output, &ec_output, &progress).expect("pack the gumps");

        assert!(cc_summary.single_gump_count > 0);
        assert!(ec_summary.single_gump_count > 0);
        assert!(ec_summary.single_shared_classic_count > 0, "the Enhanced gumps shared nothing");
        let states = progress.progress.lock().expect("progress map");
        assert_eq!(states[&AssetPackTask::GumpsCc].state, AssetPackProgressState::Succeeded);
    }
}
