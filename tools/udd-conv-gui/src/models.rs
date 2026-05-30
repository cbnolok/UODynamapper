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

#[derive(Deserialize, Serialize, Clone, PartialEq)]
pub struct AppSettings {
    pub cc_dir: Option<PathBuf>,
    pub ec_dir: Option<PathBuf>,
    #[serde(default)]
    pub dynamapper_routing_dir: Option<PathBuf>,
    pub input_uddp_dir: PathBuf,
    pub output_uddp_dir: PathBuf,
    pub link_uddp_dirs: bool,
    pub opt_tex_art_cc: TextureOptimization,
    pub opt_tex_land_cc: TextureOptimization,
    pub opt_tex_art_ec: TextureOptimization,
    pub opt_tex_land_ec: TextureOptimization,
    #[serde(default = "default_mobile_anim_optimization")]
    pub opt_mobile_anim_cc: TextureOptimization,
    #[serde(default = "default_mobile_anim_optimization")]
    pub opt_mobile_anim_ec: TextureOptimization,
    #[serde(default = "default_zstd_level")]
    pub zstd_tex_art_cc: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_tex_land_cc: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_tex_art_ec: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_tex_land_ec: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_tilemeta: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_mobile_anim_cc: i32,
    #[serde(default = "default_zstd_level")]
    pub zstd_mobile_anim_ec: i32,
    #[serde(default = "default_jxl_level")]
    pub jxl_tex_art_cc: u8,
    #[serde(default = "default_jxl_level")]
    pub jxl_tex_land_cc: u8,
    #[serde(default = "default_jxl_level")]
    pub jxl_tex_art_ec: u8,
    #[serde(default = "default_jxl_level")]
    pub jxl_tex_land_ec: u8,
    #[serde(default = "default_jxl_level")]
    pub jxl_mobile_anim_cc: u8,
    #[serde(default = "default_jxl_level")]
    pub jxl_mobile_anim_ec: u8,
    #[serde(default = "default_bc7_rdo_lambda")]
    pub bc7_rdo_lambda: f32,
    #[serde(default = "default_bc7_rdo_enabled")]
    pub bc7_rdo_enabled: bool,
    pub upscale_tex_art_cc: UpscaleFilter,
    pub upscale_tex_land_cc_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_cc_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_art_ec: UpscaleFilter,
    pub upscale_tex_land_ec_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_256: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_512: udd_conv::upscale::UpscaleConfig,
    #[serde(default)]
    pub upscale_mobile_anim_cc: UpscaleFilter,
    #[serde(default)]
    pub upscale_mobile_anim_ec: UpscaleFilter,
    pub radar_format: RadarFormat,
    pub radar_zstd: i32,
    pub map_preferences: [CcMapSourcePreference; 6],
    #[serde(default)]
    pub include_verdata: bool,
    #[serde(default)]
    pub include_map_difs: bool,
    #[serde(default)]
    pub include_static_difs: bool,
    #[serde(default)]
    pub ec_mobile_anim_allow_missing_kdl: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cc_dir: None,
            ec_dir: None,
            dynamapper_routing_dir: None,
            input_uddp_dir: PathBuf::from("packages"),
            output_uddp_dir: PathBuf::from("packages"),
            link_uddp_dirs: true,
            opt_tex_art_cc: TextureOptimization::None,
            opt_tex_land_cc: TextureOptimization::None,
            opt_tex_art_ec: TextureOptimization::None,
            opt_tex_land_ec: TextureOptimization::None,
            opt_mobile_anim_cc: default_mobile_anim_optimization(),
            opt_mobile_anim_ec: default_mobile_anim_optimization(),
            zstd_tex_art_cc: default_zstd_level(),
            zstd_tex_land_cc: default_zstd_level(),
            zstd_tex_art_ec: default_zstd_level(),
            zstd_tex_land_ec: default_zstd_level(),
            zstd_tilemeta: default_zstd_level(),
            zstd_mobile_anim_cc: default_zstd_level(),
            zstd_mobile_anim_ec: default_zstd_level(),
            jxl_tex_art_cc: default_jxl_level(),
            jxl_tex_land_cc: default_jxl_level(),
            jxl_tex_art_ec: default_jxl_level(),
            jxl_tex_land_ec: default_jxl_level(),
            jxl_mobile_anim_cc: default_jxl_level(),
            jxl_mobile_anim_ec: default_jxl_level(),
            bc7_rdo_lambda: default_bc7_rdo_lambda(),
            bc7_rdo_enabled: default_bc7_rdo_enabled(),
            upscale_tex_art_cc: UpscaleFilter::None,
            upscale_tex_land_cc_64: udd_conv::upscale::UpscaleConfig::default(),
            upscale_tex_land_cc_128: udd_conv::upscale::UpscaleConfig::default(),
            upscale_tex_art_ec: UpscaleFilter::None,
            upscale_tex_land_ec_64: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_128: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_256: udd_conv::upscale::UpscaleConfig { target_size: 256, filter: UpscaleFilter::FsrEasu2x },
            upscale_tex_land_ec_512: udd_conv::upscale::UpscaleConfig { target_size: 512, filter: UpscaleFilter::None },
            upscale_mobile_anim_cc: UpscaleFilter::None,
            upscale_mobile_anim_ec: UpscaleFilter::None,
            radar_format: RadarFormat::Bc7,
            radar_zstd: 3,
            map_preferences: [CcMapSourcePreference::Mul; 6],
            include_verdata: false,
            include_map_difs: false,
            include_static_difs: false,
            ec_mobile_anim_allow_missing_kdl: false,
        }
    }
}

fn default_bc7_rdo_lambda() -> f32 {
    udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA
}

fn default_bc7_rdo_enabled() -> bool {
    true
}

fn default_zstd_level() -> i32 {
    7
}

fn default_jxl_level() -> u8 {
    6
}

fn default_mobile_anim_optimization() -> TextureOptimization {
    TextureOptimization::Bc7
}

pub struct LogMessage {
    pub text: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetPackTask {
    TexArtCc,
    TexLandCc,
    TexArtEc,
    TexLandEc,
    MobileAnimCc,
    MobileAnimEc,
    TileMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetPackProgressState {
    Idle,
    Running,
    Succeeded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct AssetPackProgress {
    pub state: AssetPackProgressState,
    pub fraction: f32,
    pub text: String,
}

impl AssetPackProgress {
    pub fn idle() -> Self {
        Self {
            state: AssetPackProgressState::Idle,
            fraction: 0.0,
            text: "Ready".to_string(),
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpscalePreviewTarget {
    TexArtCc,
    TexLandCc64,
    TexLandCc128,
    TexArtEc,
    TexLandEc64,
    TexLandEc128,
    TexLandEc256,
    TexLandEc512,
}

pub struct UpscalePreviewState {
    pub target: UpscalePreviewTarget,
    pub id: u32,
    pub filter: UpscaleFilter,
    pub texture: Option<egui::TextureHandle>,
    pub upscaled_texture: Option<egui::TextureHandle>,
    pub upscaled_size: [u32; 2],
    pub original_size: [u32; 2],
    pub id_buffer: String,
    pub is_dirty: bool,
    pub zoom: f32,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Sources,
    Assets,
    World,
    Tools,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_settings_default_disables_optional_classic_patches() {
        let settings = AppSettings::default();

        assert!(!settings.include_verdata);
        assert!(!settings.include_map_difs);
        assert!(!settings.include_static_difs);
        assert!(!settings.ec_mobile_anim_allow_missing_kdl);
        assert_eq!(settings.dynamapper_routing_dir, None);
        assert!(settings.bc7_rdo_enabled);
        assert_eq!(settings.opt_mobile_anim_cc, TextureOptimization::Bc7);
        assert_eq!(settings.opt_mobile_anim_ec, TextureOptimization::Bc7);
    }

    #[test]
    fn app_settings_deserializes_old_config_without_patch_fields() {
        let serialized = toml::to_string(&AppSettings::default()).expect("serialize settings");
        let old_config = serialized
            .lines()
            .filter(|line| {
                !line.starts_with("include_") && !line.starts_with("bc7_rdo_enabled")
                    && !line.starts_with("dynamapper_routing_dir")
                    && !line.starts_with("ec_mobile_anim_allow_missing_kdl")
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\nignore_missing_tileart_for_tex_art_cc = true\n";

        let settings: AppSettings = toml::from_str(&old_config).expect("deserialize old settings");

        assert!(!settings.include_verdata);
        assert!(!settings.include_map_difs);
        assert!(!settings.include_static_difs);
        assert!(!settings.ec_mobile_anim_allow_missing_kdl);
        assert_eq!(settings.dynamapper_routing_dir, None);
        assert!(settings.bc7_rdo_enabled);
        assert_eq!(settings.opt_mobile_anim_cc, TextureOptimization::Bc7);
        assert_eq!(settings.opt_mobile_anim_ec, TextureOptimization::Bc7);
    }

    #[test]
    fn app_settings_roundtrips_enabled_patch_fields() {
        let mut settings = AppSettings::default();
        settings.include_verdata = true;
        settings.include_map_difs = true;
        settings.include_static_difs = true;
        settings.ec_mobile_anim_allow_missing_kdl = true;
        settings.dynamapper_routing_dir = Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"));

        let serialized = toml::to_string(&settings).expect("serialize settings");
        let restored: AppSettings = toml::from_str(&serialized).expect("deserialize settings");

        assert!(restored.include_verdata);
        assert!(restored.include_map_difs);
        assert!(restored.include_static_difs);
        assert!(restored.ec_mobile_anim_allow_missing_kdl);
        assert_eq!(
            restored.dynamapper_routing_dir,
            Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"))
        );
    }
}
