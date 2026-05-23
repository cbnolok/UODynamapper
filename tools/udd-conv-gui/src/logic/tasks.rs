use color_eyre::eyre;
use std::panic::{self, AssertUnwindSafe};
use udd_conv::{
    AtlasPackingMode,
    classic_patches::ClassicPatchOptions,
    tex_art_cc::{TexArtCcAtlasOptions, convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    tex_art_ec::{TexArtEcAtlasOptions, convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources},
    tex_land_ec::{TexLandEcAtlasOptions, convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources},
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources, build_tilemeta_uddp_from_split_sources},
    cc_map::convert_map_mul_to_uddp_from_sources_with_patches,
    cc_statics::convert_statics_mul_to_uddp_from_sources_with_patches,
    tex_land_cc::{TexLandCcAtlasOptions, convert_texmaps_mul_to_tex_land_cc_uddp_with_patches},
    cc_radar::{build_facet_radar_dds, RadarFormat, RadarBuildOptions},
    source_paths::gather_source_dirs,
    CompressionFlag,
    PagePixelFormat,
};
use udd_conv_cli::{
    package_info::get_package_info_string,
    extract::extract_package,
    tool_cli::{diff_paths, DiffKind},
};
use crate::app::UddConvApp;
use crate::models::{AtlasPackingModeSetting, LogLevel, LogMessage, TextureOptimization};

struct ConvertingFlagReset {
    is_converting: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl Drop for ConvertingFlagReset {
    fn drop(&mut self) {
        if let Ok(mut is_converting) = self.is_converting.lock() {
            *is_converting = false;
        }
    }
}

fn atlas_packing_mode(setting: AtlasPackingModeSetting) -> AtlasPackingMode {
    match setting {
        AtlasPackingModeSetting::MaximumPacking => AtlasPackingMode::MaximumPacking,
        AtlasPackingModeSetting::Bc7Oriented => AtlasPackingMode::Bc7Oriented,
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

            let result = match panic::catch_unwind(AssertUnwindSafe(task)) {
                Ok(result) => result,
                Err(_) => Err(eyre::eyre!("task panicked")),
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

    pub fn convert_tex_art_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_cc.uddp");
        self.spawn_task("CC Art Packing".to_string(), move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_art_cc {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources_with_patches(
                &sources, &output,
                &TexArtCcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale: settings.upscale_tex_art_cc,
                    pixel_format: match settings.opt_tex_art_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    packing_mode: atlas_packing_mode(settings.packing_tex_art_cc),
                    filtering_ready: settings.filtering_ready_tex_art_cc,
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                },
                &classic_patch_options(&settings),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_cc.uddp");
        self.spawn_task("CC Texmaps Packing".to_string(), move || {
            let sources = gather_cc_source_dirs(settings.cc_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_land_cc {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_texmaps_mul_to_tex_land_cc_uddp_with_patches(
                &sources[0], &output,
                &TexLandCcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale_64: settings.upscale_tex_land_cc_64,
                    upscale_128: settings.upscale_tex_land_cc_128,
                    pixel_format: match settings.opt_tex_land_cc {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    packing_mode: atlas_packing_mode(settings.packing_tex_land_cc),
                    filtering_ready: settings.filtering_ready_tex_land_cc,
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                },
                &classic_patch_options(&settings),
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_art_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_ec.uddp");
        self.spawn_task("EC Art Packing".to_string(), move || {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_art_ec {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources(
                &sources, &output,
                &TexArtEcAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    crop_transparent_bounds: false,
                    compression,
                    upscale: settings.upscale_tex_art_ec,
                    pixel_format: match settings.opt_tex_art_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    packing_mode: atlas_packing_mode(settings.packing_tex_art_ec),
                    filtering_ready: settings.filtering_ready_tex_art_ec,
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_ec.uddp");
        self.spawn_task("EC Land Packing".to_string(), move || {
            let sources = gather_ec_source_dirs(settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_land_ec {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources(
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
                    pixel_format: match settings.opt_tex_land_ec {
                        TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd => PagePixelFormat::Bc7,
                        _ => PagePixelFormat::Rgba8888,
                    },
                    packing_mode: atlas_packing_mode(settings.packing_tex_land_ec),
                    filtering_ready: settings.filtering_ready_tex_land_ec,
                    bc7_rdo_lambda: settings.bc7_rdo_lambda,
                    transcode_kdl_path: None,
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tilemeta(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tilemeta.uddp");
        self.spawn_task("Tilemeta Packing".to_string(), move || {
            match (settings.cc_dir.as_ref(), settings.ec_dir.as_ref()) {
                (Some(cc_dir), Some(ec_dir)) => {
                    build_tilemeta_uddp_from_split_sources(
                        cc_dir,
                        ec_dir,
                        &output,
                        &TileMetaBuildOptions {
                            adjust_tex_art_ec_sampling: false,
                            use_ec_radarcol: false,
                            classic_patches: classic_patch_options(&settings),
                        },
                    )?;
                }
                _ => {
                    let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
                    if sources.is_empty() { eyre::bail!("No source dirs"); }
                    build_tilemeta_uddp_from_sources(
                        &sources,
                        &output,
                        &TileMetaBuildOptions {
                            adjust_tex_art_ec_sampling: false,
                            use_ec_radarcol: false,
                            classic_patches: classic_patch_options(&settings),
                        },
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

            if settings.radar_format == RadarFormat::Bc7Ktx2 {
                udd_conv_ktx2::build_facet_radar_ktx2_with_patches(
                    &sources,
                    &tilemeta_path,
                    &output,
                    map_id,
                    settings.radar_zstd,
                    &classic_patch_options(&settings),
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
                udd_conv_ktx2::build_facet_radar_ktx2_with_patches(
                    &sources,
                    &tilemeta_path,
                    &radar_output,
                    map_id,
                    settings.radar_zstd,
                    &classic_patch_options(&settings),
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
            diff_paths(&left, &right, DiffKind::Auto)?;
            Ok("Diff complete. Check console for output.".to_string())
        });
    }
}
