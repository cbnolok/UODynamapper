use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use uddconv::cc_radar::RadarFormat;
use uddconv::upscale::{UpscaleConfig, UpscaleFilter};

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
pub enum TextureOptimization {
    None,
    Bc7,
    JpegXl,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AppSettings {
    pub cc_dir: Option<PathBuf>,
    pub ec_dir: Option<PathBuf>,
    pub input_uddp_dir: PathBuf,
    pub output_uddp_dir: PathBuf,
    pub link_uddp_dirs: bool,
    pub opt_cc_art: TextureOptimization,
    pub opt_ec_art: TextureOptimization,
    pub opt_ec_land: TextureOptimization,
    pub upscale_ec_land_64: UpscaleConfig,
    pub upscale_ec_land_128: UpscaleConfig,
    pub upscale_ec_land_256: UpscaleConfig,
    pub radar_format: RadarFormat,
    pub radar_zstd: i32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cc_dir: None,
            ec_dir: None,
            input_uddp_dir: PathBuf::from("packages"),
            output_uddp_dir: PathBuf::from("packages"),
            link_uddp_dirs: true,
            opt_cc_art: TextureOptimization::None,
            opt_ec_art: TextureOptimization::None,
            opt_ec_land: TextureOptimization::None,
            upscale_ec_land_64: UpscaleConfig {
                target_size: 256,
                filter: UpscaleFilter::FsrEasu,
            },
            upscale_ec_land_128: UpscaleConfig {
                target_size: 256,
                filter: UpscaleFilter::FsrEasu,
            },
            upscale_ec_land_256: UpscaleConfig {
                target_size: 256,
                filter: UpscaleFilter::None,
            },
            radar_format: RadarFormat::Bc7,
            radar_zstd: 3,
        }
    }
}

pub struct LogMessage {
    pub text: String,
    pub level: LogLevel,
}

#[derive(PartialEq, Clone, Copy)]
pub enum LogLevel {
    Info,
    Success,
    // Warning,
    Error,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Sources,
    Assets,
    World,
    Tools,
}
