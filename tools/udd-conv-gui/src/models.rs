use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use udd_conv::classic_sources::SourceFormatPreference;
use udd_conv::cc_radar::RadarFormat;
use udd_conv::ec_gumps::EC_GUMP_DEFAULT_MAX_ID;
use udd_conv::hues::HueRampMode;
use udd_conv::page_compression::LOSSLESS_PAGE_ZSTD_LEVEL;
use udd_conv::upscale::UpscaleFilter;
use uocf::map_edition::MapEdition;

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureOptimization {
    None,
    Bc7,
    Bc7Zstd,
}

/// What the Enhanced gumps job does with the Classic gumps Interface.uop repeats.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GumpsEcSharing {
    /// Leave out each single gump the gumps_cc.uddp of the output folder draws the same (`pack-ec-gumps
    /// --share-with`), packing standalone when there is none.
    ShareWithGumpsCc,
    /// Keep every gump, so the package needs no gumps_cc.uddp.
    Standalone,
}

/// How the hues job spreads the 32 Classic `hues.mul` colours over a palette row (`pack-hues --hue-ramp`).
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum HueRamp {
    /// Colour i at x = 8i, ramping linearly to colour i + 1, as EA's Hues.uop does.
    Linear,
    /// Each colour repeated over 8 texels.
    Flat,
}

impl From<HueRamp> for HueRampMode {
    fn from(ramp: HueRamp) -> Self {
        match ramp {
            HueRamp::Linear => HueRampMode::Linear,
            HueRamp::Flat => HueRampMode::Flat,
        }
    }
}

/// The Zstd level of `--zstd` without a level, and of the packers' default Zstd (`CompressionFlag::ZstdNoDict`).
pub const DEFAULT_ZSTD_LEVEL: i32 = 7;

/// The GUI writes this file itself (on every change and on exit), so it is read leniently: keys missing from an
/// older file take their `Default` value and unknown keys are dropped, each with a warning
/// (see `logic::settings::load_settings_from_path`).
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    pub cc_dir: Option<PathBuf>,
    /// The Classic client executable the world tab's dif check reads instead of the folder's `client.exe`; cleared
    /// when the Classic folder changes.
    pub classic_client_exe: Option<PathBuf>,
    pub ec_dir: Option<PathBuf>,
    pub dynamapper_routing_dir: Option<PathBuf>,
    pub upscale_profile_path: Option<PathBuf>,
    pub input_uddp_dir: PathBuf,
    pub output_uddp_dir: PathBuf,
    pub link_uddp_dirs: bool,
    pub opt_tex_art_cc: TextureOptimization,
    pub opt_tex_land_cc: TextureOptimization,
    pub opt_tex_art_ec: TextureOptimization,
    pub opt_tex_land_ec: TextureOptimization,
    pub opt_mobile_anim_cc: TextureOptimization,
    pub opt_mobile_anim_ec: TextureOptimization,
    pub opt_tex_aux_ec: TextureOptimization,
    pub zstd_tex_art_cc: i32,
    pub zstd_tex_land_cc: i32,
    pub zstd_tex_art_ec: i32,
    pub zstd_tex_land_ec: i32,
    pub zstd_tilemeta: i32,
    pub zstd_mobile_anim_cc: i32,
    pub zstd_mobile_anim_ec: i32,
    pub zstd_tex_aux_ec: i32,
    pub bc7_rdo_lambda: f32,
    pub bc7_rdo_lookback_blocks: usize,
    pub bc7_rdo_enabled: bool,
    pub upscale_tex_art_cc: UpscaleFilter,
    pub upscale_tex_land_cc_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_cc_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_art_ec: UpscaleFilter,
    pub upscale_tex_land_ec_64: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_128: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_256: udd_conv::upscale::UpscaleConfig,
    pub upscale_tex_land_ec_512: udd_conv::upscale::UpscaleConfig,
    pub upscale_mobile_anim_cc: UpscaleFilter,
    pub upscale_mobile_anim_ec: UpscaleFilter,
    pub upscale_tex_aux_ec: UpscaleFilter,
    pub radar_format: RadarFormat,
    pub radar_zstd: i32,
    pub map_preferences: [SourceFormatPreference; 6],
    pub include_verdata: bool,
    pub include_map_difs: bool,
    pub include_static_difs: bool,
    /// The edition of the Classic facet files the map, statics and radar conversions read.
    pub map_edition: MapEdition,
    pub ec_mobile_anim_allow_missing_kdl: bool,
    /// The Classic single gumps as BC1 (`pack-gumps --bc1`), else lossless RGBA8888.
    pub gumps_cc_bc1: bool,
    /// The Zstd level of the Classic single gumps and atlas pages (`pack-gumps --zstd=LEVEL`); `None` keeps the
    /// package's levels.
    pub zstd_gumps_cc: Option<i32>,
    pub upscale_gumps_cc_paperdoll: UpscaleFilter,
    pub upscale_gumps_cc_single: UpscaleFilter,
    /// The Classic gump files read first (`pack-gumps --uop` for UOP); the other format is the fallback.
    pub gumps_cc_source: SourceFormatPreference,
    /// The Enhanced single gumps lossy (`pack-ec-gumps --bc7`): BC7, the ones in the Classic colour space BC1.
    pub gumps_ec_bc7: bool,
    /// The Zstd level of the Enhanced single gumps and atlas pages (`pack-ec-gumps --zstd=LEVEL`); `None` keeps the
    /// package's levels.
    pub zstd_gumps_ec: Option<i32>,
    pub upscale_gumps_ec_paperdoll: UpscaleFilter,
    pub upscale_gumps_ec_single: UpscaleFilter,
    /// The highest gump id read from Interface.uop (`pack-ec-gumps --max-id`).
    pub gumps_ec_max_id: u32,
    /// An extracted gumpart folder the Enhanced gumps are read from instead of the Enhanced client folder.
    pub gumps_ec_gumpart_dir: Option<PathBuf>,
    pub gumps_ec_sharing: GumpsEcSharing,
    /// How hues.mul colours fill a palette row (`pack-hues --hue-ramp`); an Enhanced Hues.uop is copied as it is.
    pub hues_ramp: HueRamp,
    /// The hues package stored raw (`pack-hues --no-compression`).
    pub hues_no_compression: bool,
    /// The Zstd level of the hues package (`pack-hues --zstd=LEVEL`); `None` keeps the packer's level.
    pub zstd_hues: Option<i32>,
    /// The world lights package stored raw (`pack-lights --no-compression`).
    pub lights_no_compression: bool,
    /// The Zstd level of the world lights package (`pack-lights --zstd=LEVEL`); `None` keeps the packer's level.
    pub zstd_lights: Option<i32>,
    /// The uncompressed Enhanced effect textures whose sides are multiples of 4 as BC7 (`pack-ec-effects --bc7`), else
    /// RGBA8.
    pub effects_ec_bc7: bool,
    /// The Zstd level of the Enhanced effects' manifests and textures (`pack-ec-effects --zstd=LEVEL`); `None` keeps
    /// the packer's level.
    pub zstd_effects_ec: Option<i32>,
    /// Fail on error findings instead of writing flagged records (`pack-ec-effects --strict`).
    pub effects_ec_strict: bool,
    /// The file the Enhanced effects packer writes its findings to (`pack-ec-effects --report`).
    pub effects_ec_report: Option<PathBuf>,
    /// The Zstd level of the Enhanced paperdoll frames and manifests (`pack-ec-paperdoll --zstd=LEVEL`); `None` keeps
    /// the packer's level.
    pub zstd_paperdoll_ec: Option<i32>,
    /// The file the Enhanced paperdoll packer writes its dress-layer findings to (`pack-ec-paperdoll --report`).
    pub paperdoll_ec_report: Option<PathBuf>,
    /// Validate each package a packing job writes, as `tool validate` does, once the job succeeds.
    pub validate_after_packing: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            cc_dir: None,
            classic_client_exe: None,
            ec_dir: None,
            dynamapper_routing_dir: None,
            upscale_profile_path: None,
            input_uddp_dir: PathBuf::from("packages"),
            output_uddp_dir: PathBuf::from("packages"),
            link_uddp_dirs: true,
            opt_tex_art_cc: TextureOptimization::None,
            opt_tex_land_cc: TextureOptimization::None,
            opt_tex_art_ec: TextureOptimization::None,
            opt_tex_land_ec: TextureOptimization::None,
            opt_mobile_anim_cc: default_mobile_anim_optimization(),
            opt_mobile_anim_ec: default_mobile_anim_optimization(),
            opt_tex_aux_ec: TextureOptimization::None,
            zstd_tex_art_cc: LOSSLESS_PAGE_ZSTD_LEVEL,
            zstd_tex_land_cc: LOSSLESS_PAGE_ZSTD_LEVEL,
            zstd_tex_art_ec: LOSSLESS_PAGE_ZSTD_LEVEL,
            zstd_tex_land_ec: LOSSLESS_PAGE_ZSTD_LEVEL,
            zstd_tilemeta: default_zstd_level(),
            zstd_mobile_anim_cc: default_zstd_level(),
            zstd_mobile_anim_ec: default_zstd_level(),
            zstd_tex_aux_ec: LOSSLESS_PAGE_ZSTD_LEVEL,
            bc7_rdo_lambda: default_bc7_rdo_lambda(),
            bc7_rdo_lookback_blocks: default_bc7_rdo_lookback_blocks(),
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
            upscale_tex_aux_ec: UpscaleFilter::None,
            radar_format: RadarFormat::Bc7,
            radar_zstd: 3,
            map_preferences: [SourceFormatPreference::Mul; 6],
            include_verdata: false,
            include_map_difs: false,
            include_static_difs: false,
            map_edition: MapEdition::Standard,
            ec_mobile_anim_allow_missing_kdl: false,
            gumps_cc_bc1: false,
            zstd_gumps_cc: None,
            upscale_gumps_cc_paperdoll: UpscaleFilter::None,
            upscale_gumps_cc_single: UpscaleFilter::None,
            gumps_cc_source: SourceFormatPreference::Mul,
            gumps_ec_bc7: false,
            zstd_gumps_ec: None,
            upscale_gumps_ec_paperdoll: UpscaleFilter::None,
            upscale_gumps_ec_single: UpscaleFilter::None,
            gumps_ec_max_id: EC_GUMP_DEFAULT_MAX_ID,
            gumps_ec_gumpart_dir: None,
            gumps_ec_sharing: GumpsEcSharing::ShareWithGumpsCc,
            hues_ramp: HueRamp::Linear,
            hues_no_compression: false,
            zstd_hues: None,
            lights_no_compression: false,
            zstd_lights: None,
            effects_ec_bc7: false,
            zstd_effects_ec: None,
            effects_ec_strict: false,
            effects_ec_report: None,
            zstd_paperdoll_ec: None,
            paperdoll_ec_report: None,
            validate_after_packing: false,
        }
    }
}

fn default_bc7_rdo_lambda() -> f32 {
    udd_conv::bc7::DEFAULT_BC7_RDO_LAMBDA
}

fn default_bc7_rdo_lookback_blocks() -> usize {
    udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS
}

fn default_bc7_rdo_enabled() -> bool {
    true
}

fn default_zstd_level() -> i32 {
    DEFAULT_ZSTD_LEVEL
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
    TexAuxEc,
    MobileAnimCc,
    MobileAnimEc,
    TileMeta,
    GumpsCc,
    GumpsEc,
    Hues,
    WorldLights,
    EffectsEc,
    PaperdollEc,
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

#[derive(Debug, PartialEq, Clone, Copy)]
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
        assert_eq!(settings.map_edition, MapEdition::Standard);
        assert!(!settings.ec_mobile_anim_allow_missing_kdl);
        assert_eq!(settings.dynamapper_routing_dir, None);
        assert_eq!(settings.upscale_profile_path, None);
        assert_eq!(settings.classic_client_exe, None);
        assert!(settings.bc7_rdo_enabled);
        assert_eq!(
            settings.bc7_rdo_lookback_blocks,
            udd_conv::bc7::DEFAULT_BC7_RDO_LOOKBACK_BLOCKS
        );
        assert_eq!(settings.opt_mobile_anim_cc, TextureOptimization::Bc7);
        assert_eq!(settings.opt_mobile_anim_ec, TextureOptimization::Bc7);
    }

    #[test]
    fn app_settings_default_packs_land_as_lossless_zstd() {
        let settings = AppSettings::default();

        assert_eq!(settings.opt_tex_land_cc, TextureOptimization::None);
        assert_eq!(settings.opt_tex_land_ec, TextureOptimization::None);
        assert_eq!(settings.zstd_tex_land_cc, LOSSLESS_PAGE_ZSTD_LEVEL);
        assert_eq!(settings.zstd_tex_land_ec, LOSSLESS_PAGE_ZSTD_LEVEL);
    }

    #[test]
    fn app_settings_default_packs_art_and_aux_as_lossless_zstd() {
        let settings = AppSettings::default();

        assert_eq!(settings.opt_tex_art_cc, TextureOptimization::None);
        assert_eq!(settings.opt_tex_art_ec, TextureOptimization::None);
        assert_eq!(settings.opt_tex_aux_ec, TextureOptimization::None);
        assert_eq!(settings.zstd_tex_art_cc, LOSSLESS_PAGE_ZSTD_LEVEL);
        assert_eq!(settings.zstd_tex_art_ec, LOSSLESS_PAGE_ZSTD_LEVEL);
        assert_eq!(settings.zstd_tex_aux_ec, LOSSLESS_PAGE_ZSTD_LEVEL);
    }

    #[test]
    fn app_settings_default_packs_gumps_lossless_and_shares_the_classic_twins() {
        let settings = AppSettings::default();

        assert!(!settings.gumps_cc_bc1);
        assert!(!settings.gumps_ec_bc7);
        assert_eq!(settings.zstd_gumps_cc, None);
        assert_eq!(settings.zstd_gumps_ec, None);
        assert_eq!(settings.gumps_cc_source, SourceFormatPreference::Mul);
        assert_eq!(settings.gumps_ec_max_id, EC_GUMP_DEFAULT_MAX_ID);
        assert_eq!(settings.gumps_ec_gumpart_dir, None);
        assert_eq!(settings.gumps_ec_sharing, GumpsEcSharing::ShareWithGumpsCc);
    }

    /// The hues, world lights and Enhanced effects jobs default to the CLI without flags, and no job validates what
    /// it writes until asked.
    #[test]
    fn app_settings_default_packs_hues_lights_and_effects_as_the_cli_does() {
        let settings = AppSettings::default();

        assert_eq!(settings.hues_ramp, HueRamp::Linear);
        assert!(!settings.hues_no_compression);
        assert_eq!(settings.zstd_hues, None);
        assert!(!settings.lights_no_compression);
        assert_eq!(settings.zstd_lights, None);
        assert!(!settings.effects_ec_bc7);
        assert_eq!(settings.zstd_effects_ec, None);
        assert!(!settings.effects_ec_strict);
        assert_eq!(settings.effects_ec_report, None);
        assert_eq!(settings.zstd_paperdoll_ec, None);
        assert_eq!(settings.paperdoll_ec_report, None);
        assert!(!settings.validate_after_packing);
    }

    #[test]
    fn hue_ramps_map_to_the_packer_modes() {
        assert_eq!(HueRampMode::from(HueRamp::Linear), HueRampMode::Linear);
        assert_eq!(HueRampMode::from(HueRamp::Flat), HueRampMode::Flat);
    }

    #[test]
    fn app_settings_roundtrips_hues_lights_effects_and_validate_fields() {
        const ZSTD_LEVEL: i32 = 15;
        let mut settings = AppSettings::default();
        settings.hues_ramp = HueRamp::Flat;
        settings.hues_no_compression = true;
        settings.zstd_hues = Some(ZSTD_LEVEL);
        settings.lights_no_compression = true;
        settings.zstd_lights = Some(ZSTD_LEVEL);
        settings.effects_ec_bc7 = true;
        settings.zstd_effects_ec = Some(ZSTD_LEVEL);
        settings.effects_ec_strict = true;
        settings.effects_ec_report = Some(PathBuf::from("reports/effects_ec.txt"));
        settings.zstd_paperdoll_ec = Some(ZSTD_LEVEL);
        settings.paperdoll_ec_report = Some(PathBuf::from("reports/paperdoll_ec.txt"));
        settings.validate_after_packing = true;

        let serialized = toml::to_string(&settings).expect("serialize settings");
        let restored: AppSettings = toml::from_str(&serialized).expect("deserialize settings");

        assert!(restored == settings);
    }

    #[test]
    fn app_settings_roundtrips_gump_fields() {
        const ZSTD_LEVEL: i32 = 12;
        const MAX_ID: u32 = 70_000;
        let mut settings = AppSettings::default();
        settings.gumps_cc_bc1 = true;
        settings.gumps_ec_bc7 = true;
        settings.zstd_gumps_cc = Some(ZSTD_LEVEL);
        settings.zstd_gumps_ec = Some(ZSTD_LEVEL);
        settings.upscale_gumps_cc_single = UpscaleFilter::Nearest2x;
        settings.upscale_gumps_ec_paperdoll = UpscaleFilter::Xbrz2x;
        settings.gumps_cc_source = SourceFormatPreference::Uop;
        settings.gumps_ec_max_id = MAX_ID;
        settings.gumps_ec_gumpart_dir = Some(PathBuf::from("extracted/gumpart"));
        settings.gumps_ec_sharing = GumpsEcSharing::Standalone;

        let serialized = toml::to_string(&settings).expect("serialize settings");
        let restored: AppSettings = toml::from_str(&serialized).expect("deserialize settings");

        assert!(restored == settings);
    }

    #[test]
    fn app_settings_roundtrips_enabled_patch_fields() {
        let mut settings = AppSettings::default();
        settings.include_verdata = true;
        settings.include_map_difs = true;
        settings.include_static_difs = true;
        settings.map_edition = MapEdition::Variant;
        settings.ec_mobile_anim_allow_missing_kdl = true;
        settings.dynamapper_routing_dir = Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"));
        settings.upscale_profile_path = Some(PathBuf::from("profiles/mobile-upscale.toml"));
        settings.classic_client_exe = Some(PathBuf::from("/uo/classic/client_tc.exe"));
        settings.bc7_rdo_lookback_blocks = 128;

        let serialized = toml::to_string(&settings).expect("serialize settings");
        let restored: AppSettings = toml::from_str(&serialized).expect("deserialize settings");

        assert!(restored.include_verdata);
        assert!(restored.include_map_difs);
        assert!(restored.include_static_difs);
        assert_eq!(restored.map_edition, MapEdition::Variant);
        assert!(restored.ec_mobile_anim_allow_missing_kdl);
        assert_eq!(
            restored.dynamapper_routing_dir,
            Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"))
        );
        assert_eq!(
            restored.upscale_profile_path,
            Some(PathBuf::from("profiles/mobile-upscale.toml"))
        );
        assert_eq!(restored.classic_client_exe, Some(PathBuf::from("/uo/classic/client_tc.exe")));
        assert_eq!(restored.bc7_rdo_lookback_blocks, 128);
    }
}
