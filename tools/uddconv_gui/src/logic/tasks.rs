use color_eyre::eyre;
use uddconv::{
    cc_art::{CcArtAtlasOptions, convert_art_mul_to_cc_art_uddp_from_sources, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    ec_art::{EcArtAtlasOptions, convert_ec_art_uop_to_ec_art_uddp_from_sources},
    ec_land::{EcLandAtlasOptions, convert_ec_land_uop_to_ec_land_uddp_from_sources},
    upscale::UpscaleFilter,
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources},
    cc_map::convert_map_mul_to_uddp_from_sources,
    cc_statics::convert_statics_mul_to_uddp_from_sources,
    cc_radar::{build_facet_radar_dds, RadarFormat, RadarBuildOptions},
    source_paths::gather_source_dirs,
    UddCompressionFlag,
};
use uddconv_cli::{
    package_info::get_package_info_string,
    extract::extract_package,
    tool_cli::{diff_paths, DiffKind},
};
use crate::app::UddConvApp;
use crate::models::{LogMessage, LogLevel, TextureOptimization};

impl UddConvApp {
    pub fn spawn_task<F>(&self, name: String, task: F)
    where
        F: FnOnce() -> eyre::Result<String> + Send + 'static,
    {
        let is_converting = self.is_converting.clone();
        let logs = self.logs.clone();

        *is_converting.lock().unwrap() = true;

        std::thread::spawn(move || {
            {
                let Ok(mut logs) = logs.lock() else {
                    return;
                };
                logs.push(LogMessage {
                    text: format!("[RUN] Starting: {}", name),
                    level: LogLevel::Info,
                });
            }

            let result = task();

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
            *is_converting.lock().unwrap() = false;
        });
    }

    pub fn convert_cc_art(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("cc_art.uddp");
        self.spawn_task("CC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_cc_art {
                TextureOptimization::None => UddCompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => UddCompressionFlag::None,
                TextureOptimization::JpegXl => UddCompressionFlag::JpegXl,
            };

            let summary = convert_art_mul_to_cc_art_uddp_from_sources(
                &sources, &output,
                &CcArtAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale: UpscaleFilter::default(),
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_ec_art(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("ec_art.uddp");
        self.spawn_task("EC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_ec_art {
                TextureOptimization::None => UddCompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => UddCompressionFlag::None,
                TextureOptimization::JpegXl => UddCompressionFlag::JpegXl,
            };

            let summary = convert_ec_art_uop_to_ec_art_uddp_from_sources(
                &sources, &output,
                &EcArtAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    crop_transparent_bounds: false,
                    compression,
                    upscale: UpscaleFilter::default(),
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_ec_land(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("ec_land.uddp");
        self.spawn_task("EC Land Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_ec_land {
                TextureOptimization::None => UddCompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => UddCompressionFlag::None,
                TextureOptimization::JpegXl => UddCompressionFlag::JpegXl,
            };

            let summary = convert_ec_land_uop_to_ec_land_uddp_from_sources(
                &sources, &output,
                &EcLandAtlasOptions {
                    atlas_width: DEFAULT_ATLAS_PAGE_WIDTH,
                    atlas_height: DEFAULT_ATLAS_PAGE_HEIGHT,
                    gutter: DEFAULT_ATLAS_GUTTER,
                    compression,
                    upscale_64: settings.upscale_ec_land_64,
                    upscale_128: settings.upscale_ec_land_128,
                    upscale_256: settings.upscale_ec_land_256,
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tilemeta(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tilemeta.uddp");
        self.spawn_task("Tilemeta Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }
            build_tilemeta_uddp_from_sources(&sources, &output, &TileMetaBuildOptions { adjust_ec_art_sampling: false, use_ec_radarcol: false })?;
            Ok(format!("Wrote tilemeta.uddp to {}", output.display()))
        });
    }

    pub fn convert_map(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("map{}.uddp", map_id));
        self.spawn_task(format!("Map {} Packing", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            let summary = convert_map_mul_to_uddp_from_sources(&sources, &output, map_id)?;
            Ok(format!("Wrote {} chunks to {}", summary.chunk_count, output.display()))
        });
    }

    pub fn convert_statics(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("statics{}.uddp", map_id));
        self.spawn_task(format!("Statics {} Packing", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            let summary = convert_statics_mul_to_uddp_from_sources(&sources, &output, map_id)?;
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
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if !tilemeta_path.exists() {
                eyre::bail!("tilemeta.uddp not found in input UDDP directory. Pack Tilemeta first!");
            }

            if settings.radar_format == RadarFormat::Bc7Ktx2 {
                uddconv_ktx2::build_facet_radar_ktx2(
                    &sources,
                    &tilemeta_path,
                    &output,
                    map_id,
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
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());

            // 1. Map
            let map_output = settings.output_uddp_dir.join(format!("map{}.uddp", map_id));
            convert_map_mul_to_uddp_from_sources(&sources, &map_output, map_id)?;

            // 2. Statics
            let statics_output = settings
                .output_uddp_dir
                .join(format!("statics{}.uddp", map_id));
            convert_statics_mul_to_uddp_from_sources(&sources, &statics_output, map_id)?;

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
                uddconv_ktx2::build_facet_radar_ktx2(
                    &sources,
                    &tilemeta_path,
                    &radar_output,
                    map_id,
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
        let file = self.tool_file_1.clone().unwrap();
        self.spawn_task(format!("Info: {}", file.file_name().unwrap().to_string_lossy()), move || {
            get_package_info_string(&file)
        });
    }

    pub fn tool_extract(&self) {
        let file = self.tool_file_1.clone().unwrap();
        self.spawn_task(format!("Extract: {}", file.file_name().unwrap().to_string_lossy()), move || {
            extract_package(&file, None)?;
            Ok(format!("Extracted to sidecar folder next to {}", file.display()))
        });
    }

    pub fn tool_diff(&self) {
        let left = self.tool_file_1.clone().unwrap();
        let right = self.tool_file_2.clone().unwrap();
        self.spawn_task("Diff Packages".to_string(), move || {
            diff_paths(&left, &right, DiffKind::Auto)?;
            Ok("Diff complete. Check console for output.".to_string())
        });
    }
}
