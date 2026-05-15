use color_eyre::eyre;
use udd_conv::{
    tex_art_cc::{TexArtCcAtlasOptions, convert_art_mul_to_tex_art_cc_uddp_from_sources, DEFAULT_ATLAS_GUTTER, DEFAULT_ATLAS_PAGE_WIDTH, DEFAULT_ATLAS_PAGE_HEIGHT},
    tex_art_ec::{TexArtEcAtlasOptions, convert_tex_art_ec_uop_to_tex_art_ec_uddp_from_sources},
    tex_land_ec::{TexLandEcAtlasOptions, convert_tex_land_ec_uop_to_tex_land_ec_uddp_from_sources},
    tilemeta::{TileMetaBuildOptions, build_tilemeta_uddp_from_sources},
    cc_map::convert_map_mul_to_uddp_from_sources,
    cc_statics::convert_statics_mul_to_uddp_from_sources,
    tex_land_cc::{TexLandCcAtlasOptions, convert_texmaps_mul_to_tex_land_cc_uddp},
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

    pub fn convert_tex_art_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_cc.uddp");
        self.spawn_task("CC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_art_cc {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_art_mul_to_tex_art_cc_uddp_from_sources(
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
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_cc(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_cc.uddp");
        self.spawn_task("CC Texmaps Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            if sources.is_empty() { eyre::bail!("No source dirs"); }

            let compression = match settings.opt_tex_land_cc {
                TextureOptimization::None => CompressionFlag::ZstdNoDict,
                TextureOptimization::Bc7 => CompressionFlag::None,
                TextureOptimization::Bc7Zstd => CompressionFlag::ZstdNoDict,
                TextureOptimization::JpegXl => CompressionFlag::JpegXl,
            };

            let summary = convert_texmaps_mul_to_tex_land_cc_uddp(
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
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_art_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_art_ec.uddp");
        self.spawn_task("EC Art Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
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
                },
            )?;
            Ok(format!("Wrote {} pages to {}", summary.page_count, output.display()))
        });
    }

    pub fn convert_tex_land_ec(&self) {
        let settings = self.settings.clone();
        let output = self.get_output_path("tex_land_ec.uddp");
        self.spawn_task("EC Land Packing".to_string(), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
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
            build_tilemeta_uddp_from_sources(&sources, &output, &TileMetaBuildOptions { adjust_tex_art_ec_sampling: false, use_ec_radarcol: false })?;
            Ok(format!("Wrote tilemeta.uddp to {}", output.display()))
        });
    }

    pub fn convert_map(&self, map_id: u32) {
        let settings = self.settings.clone();
        let output = self.get_output_path(&format!("map{}.uddp", map_id));
        self.spawn_task(format!("Map {} Packing", map_id), move || {
            let sources = gather_source_dirs(settings.cc_dir.as_ref(), settings.ec_dir.as_ref());
            let summary = convert_map_mul_to_uddp_from_sources(
                &sources, 
                &output, 
                map_id,
                settings.map_preferences[map_id as usize],
            )?;
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
                udd_conv_ktx2::build_facet_radar_ktx2(
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
            convert_map_mul_to_uddp_from_sources(
                &sources, 
                &map_output, 
                map_id,
                settings.map_preferences[map_id as usize],
            )?;

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
                udd_conv_ktx2::build_facet_radar_ktx2(
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
