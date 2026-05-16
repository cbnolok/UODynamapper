use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use udd_conv::cc_map::CcMapSourcePreference;
use udd_conv::cc_radar::RadarFormat;
use udd_conv::upscale::UpscaleFilter;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureOptimization {
    None,
    Bc7,
    Bc7Zstd,
    JpegXl,
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AtlasPackingModeSetting {
    #[default]
    MaximumPacking,
    Bc7Oriented,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AppSettings {
    pub cc_dir: Option<PathBuf>,
    pub ec_dir: Option<PathBuf>,
    pub input_uddp_dir: PathBuf,
    pub output_uddp_dir: PathBuf,
    pub link_uddp_dirs: bool,
    pub opt_tex_art_cc: TextureOptimization,
    pub opt_tex_land_cc: TextureOptimization,
    pub opt_tex_art_ec: TextureOptimization,
    pub opt_tex_land_ec: TextureOptimization,
    pub packing_tex_art_cc: AtlasPackingModeSetting,
    pub packing_tex_land_cc: AtlasPackingModeSetting,
    pub packing_tex_art_ec: AtlasPackingModeSetting,
    pub packing_tex_land_ec: AtlasPackingModeSetting,
    pub upscale_tex_art_cc: UpscaleFilter,
    pub upscale_tex_land_cc_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_cc_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_art_ec: UpscaleFilter,
    pub upscale_tex_land_ec_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_256: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_512: udd_conv::upscale::UpscaleConfig,
    pub radar_format: RadarFormat,
    pub radar_zstd: i32,
    pub map_preferences: [CcMapSourcePreference; 6],
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cc_dir: None,
            ec_dir: None,
            input_uddp_dir: PathBuf::from("packages"),
            output_uddp_dir: PathBuf::from("packages"),
            link_uddp_dirs: true,
            opt_tex_art_cc: TextureOptimization::None,
            opt_tex_land_cc: TextureOptimization::None,
            opt_tex_art_ec: TextureOptimization::None,
            opt_tex_land_ec: TextureOptimization::None,
            packing_tex_art_cc: AtlasPackingModeSetting::MaximumPacking,
            packing_tex_land_cc: AtlasPackingModeSetting::MaximumPacking,
            packing_tex_art_ec: AtlasPackingModeSetting::MaximumPacking,
            packing_tex_land_ec: AtlasPackingModeSetting::MaximumPacking,
            upscale_tex_art_cc: UpscaleFilter::None,
            upscale_tex_land_cc_64: udd_conv::upscale::UpscaleConfig::default(),
            upscale_tex_land_cc_128: udd_conv::upscale::UpscaleConfig::default(),
            upscale_tex_art_ec: UpscaleFilter::None,
            upscale_tex_land_ec_64: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_128: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_256: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_512: udd_conv::upscale::UpscaleConfig { target_size: 512, filter: UpscaleFilter::None },
            radar_format: RadarFormat::Bc7,
            radar_zstd: 3,
            map_preferences: [CcMapSourcePreference::Mul; 6],
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
