use color_eyre::eyre;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex};
use udd_conv::{
    classic_patches::ClassicPatchOptions,
    tex_art_cc::{TexArtCcAtlasOptions, convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches_and_progress, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    tex_art_ec::{TexArtEcAtlasOptions, convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources_with_progress},
    tex_land_ec::{TexLandEcAtlasOptions, convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources_with_progress},
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources_with_progress, build_tilemeta_uddp_from_split_sources_with_progress},
    cc_map::convert_map_mul_to_uddp_from_sources_with_patches,
    cc_statics::convert_statics_mul_to_uddp_from_sources_with_patches,
    tex_land_cc::{TexLandCcAtlasOptions, convert_texmaps_mul_to_tex_land_cc_uddp_with_patches_and_progress},
    cc_radar::{build_facet_radar_dds, RadarFormat, RadarBuildOptions},
    source_paths::gather_source_dirs,
    BuildProgress, BuildProgressPhase, CompressionFlag,
    PagePixelFormat,
};
use udd_conv_cli::{
    package_info::get_package_info_string,
    extract::extract_package,
    tool_cli::{diff_paths_report, DiffKind},
};
use crate::app::UddConvApp;
use crate::models::{
    AssetPackProgress, AssetPackProgressState, AssetPackTask, LogLevel, LogMessage,
    TextureOptimization,
};

struct ConvertingFlagReset {
    is_converting: std::sync::Arc<std::sync::Mutex<bool>>,
}

#[derive(Clone)]
struct AssetProgressReporter {
    task: AssetPackTask,
    progress: Arc<Mutex<HashMap<AssetPackTask, AssetPackProgress>>>,
}

impl AssetProgressReporter {
    fn set(&self, state: AssetPackProgressState, fraction: f32, text: impl Into<String>) {
        if let Ok(mut progress) = self.progress.lock() {
            progress.insert(self.task, AssetPackProgress {
                state,
                fraction: fraction.clamp(0.0, 1.0),
                text: text.into(),
            });
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

    fn build_progress(&self, progress: BuildProgress) {
        let phase_fraction = if progress.total == 0 {
            0.0
        } else {
            progress.completed.min(progress.total) as f32 / progress.total as f32
        };
        let (base, span, phase_label) = match progress.phase {
            BuildProgressPhase::TrainingDictionaries => (0.0, 0.10, "Training dictionaries"),
            BuildProgressPhase::CompressingFiles => (0.10, 0.70, "Compressing package"),
            BuildProgressPhase::Assembling => (0.80, 0.18, "Assembling package"),
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
    jxl_level: u8,
) -> CompressionFlag {
    match optimization {
        TextureOptimization::None => CompressionFlag::ZstdNoDictLevel(zstd_level),
        TextureOptimization::Bc7 => CompressionFlag::None,
        TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDictLevel(zstd_level),
        TextureOptimization::JpegXl => CompressionFlag::JpegXlLevel(jxl_level),
    }
}

impl UddConvApp {
    pub fn spawn_task<F>(&self, name: String, task: F)
    where
        F: FnOnce() -> eyre::Result<String> + Send + 'static,
    {
        let is_converting = self.is_converting.clone();
        let logs = self.logs.clone();

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
        };
        let reporter_for_task = reporter.clone();
        reporter.start();
        self.spawn_task(name, move || {
            let result = task(reporter_for_task.clone());
            match &result {
                Ok(_) => reporter_for_task.finish(),
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
                settings.jxl_tex_art_cc,
            );

            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches_and_progress(
                &sources, &output,
                &TexArtCcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale: settings.upscale_tex_art_cc,
                    upscale_passes: Vec::new(),
                    pixel_format: match settings.opt_tex_art_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                    source_preference: udd_conv::classic_sources::SourceFormatPreference::Uop,
                },
                &classic_patch_options(&settings),
                |build_progress| progress.build_progress(build_progress),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_cc.uddp");
        self.spawn_asset_task(AssetPackTask::TexLandCc, "CC Texmaps Packing".to_string(), move |progress| {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            ensure_output_parent(&output)?;

            let compression = texture_compression(
                settings.opt_tex_land_cc,
                settings.zstd_tex_land_cc,
                settings.jxl_tex_land_cc,
            );

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
                    pixel_format: match settings.opt_tex_land_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                },
                &classic_patch_options(&settings),
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
                settings.jxl_tex_art_ec,
            );

            let summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources_with_progress(
                &sources, &output,
                &TexArtEcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    crop_transparent_bounds: false,
                    compression,
                    upscale: settings.upscale_tex_art_ec,
                    upscale_passes: Vec::new(),
                    pixel_format: match settings.opt_tex_art_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                },
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

            let compression = texture_compression(
                settings.opt_tex_land_ec,
                settings.zstd_tex_land_ec,
                settings.jxl_tex_land_ec,
            );

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
                    pixel_format: match settings.opt_tex_land_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                    transcode_kdl_path: None,
                },
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
                            adjust_tex_art_ec_sampling: false,
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
                            adjust_tex_art_ec_sampling: false,
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
                udd_image_codecs::ktx2::write_ktx2_bc7_zstd(
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
                udd_image_codecs::ktx2::write_ktx2_bc7_zstd(
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
