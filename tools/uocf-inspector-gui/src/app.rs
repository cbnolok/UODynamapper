use crate::logic::{ClientData, Dictionary, UopCache};
use eframe::egui;
use image_postprocess::palette::{
    palette_safe_upscale, PaletteModel, PaletteUpscaleConfig, RgbaFilterScaler, SnapMode,
    TransparencyPolicy,
};
use image_postprocess::upscaling::UpscaleFilter;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Instant;
use udd_assets::GumpsPackage;
use uocf::classic::art::ArtMap;
pub use uocf::classic::art::ArtSource;
use uocf::classic::cliloc::Cliloc;
use uocf::classic::gump::GumpMap;
use uocf::classic::multimap_render::SourceRect;
use uocf::classic::multimap_rle::MultimapRleImage;
use uocf::classic::sound::SoundMap;
use uocf::enhanced::hues::EcHuePackage;
use uocf::enhanced::localized_strings::{LocalizedStringsPackage, LOCALIZED_STRINGS_UOP_NAME};
use uocf::enhanced::multis::MultiCollection;
use uocf::classic::tiledata::TileData;
use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::{TaeAnimationAppearance, TaeFlag, TaeSittingAnimation, TileArtEntry};
use uocf::enhanced::terrain_definition::TerrainDefinitionEntry;
use uocf::enhanced::textures::{ECImageFormat, TextureFile, TextureItem as RawTextureItem};
use uocf::uop_container::hash::hash_file_name_single;
use uocf::uop_container::package::{LoadMode, UopPackage};
use serde::{Deserialize, Serialize};

const TERRAIN_TEXTURE_GUESS_MAX_ID: u32 = 4096;
const TERRAIN_TEXTURE_GUESS_EXTENSIONS: [&str; 2] = ["dds", "tga"];

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ViewMode {
    Home,
    UopExplorer,
    TexArtCc,
    CcTileData,
    TileMetadata,
    Animations,
    AnimData,
    Gumps,
    Multis,
    Multimap,
    Hues,
    Clilocs,
    TerrainDefinition,
    StringDictionary,
    Sounds,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TileMetadataSource {
    CcTileData,
    EcTileArt,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum HuesSource {
    CcMul,
    EcUop,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum EcHueingMode {
    Cc,
    Ec,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MultisSource {
    ClassicMul,
    Uop,
    Multimap,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MultiCollectionSource {
    ClassicClient,
    EnhancedClient,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum LocalizedStringsSource {
    Cliloc,
    LocalizedStringsUop,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum GumpSource {
    Classic,
    Enhanced,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum GumpViewerTab {
    StandardGumps,
    Paperdoll,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MultimapPreviewSelection {
    Loaded,
    Plain,
    Artistic,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MultimapConversionSource {
    Ktx2,
    ClientRadar,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum MultimapMapSourcePreference {
    Mul,
    Uop,
}

impl MultimapMapSourcePreference {
    pub fn to_udd_conv(self) -> udd_conv::classic_sources::SourceFormatPreference {
        match self {
            Self::Mul => udd_conv::classic_sources::SourceFormatPreference::Mul,
            Self::Uop => udd_conv::classic_sources::SourceFormatPreference::Uop,
        }
    }
}

pub struct MultimapConverterState {
    pub source: MultimapConversionSource,
    pub ktx2_path: String,
    pub tilemeta_path: String,
    pub map_id: u32,
    pub map_source_preference: MultimapMapSourcePreference,
    pub use_ec_radarcol: bool,
    pub include_verdata: bool,
    pub include_map_difs: bool,
    pub include_static_difs: bool,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width_enabled: bool,
    pub source_height_enabled: bool,
    pub source_width: u32,
    pub source_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub edge_threshold: u16,
    pub line_radius: u32,
    pub status: String,
}

impl Default for MultimapConverterState {
    fn default() -> Self {
        Self {
            source: MultimapConversionSource::Ktx2,
            ktx2_path: String::new(),
            tilemeta_path: String::new(),
            map_id: 0,
            map_source_preference: MultimapMapSourcePreference::Mul,
            use_ec_radarcol: false,
            include_verdata: false,
            include_map_difs: false,
            include_static_difs: false,
            source_x: 0,
            source_y: 0,
            source_width_enabled: false,
            source_height_enabled: false,
            source_width: uocf::classic::multimap_rle::DEFAULT_WIDTH.saturating_mul(2),
            source_height: uocf::classic::multimap_rle::DEFAULT_HEIGHT.saturating_mul(2),
            output_width: uocf::classic::multimap_rle::DEFAULT_WIDTH,
            output_height: uocf::classic::multimap_rle::DEFAULT_HEIGHT,
            edge_threshold: 28,
            line_radius: 0,
            status: String::new(),
        }
    }
}

#[derive(Clone)]
pub struct GeneratedMultimapPreview {
    pub label: String,
    pub source_label: String,
    pub source_width: u32,
    pub source_height: u32,
    pub crop: SourceRect,
    pub image: MultimapRleImage,
}

pub struct MultimapConversionOutput {
    pub plain: GeneratedMultimapPreview,
    pub artistic: GeneratedMultimapPreview,
}

pub struct MultimapWorkerResult {
    pub result: Result<MultimapConversionOutput, String>,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum UpscalePreviewAlgorithm {
    None,
    Nearest,
    Bilinear,
    CatmullRom,
    Lanczos3,
    Lq,
    SuperSai,
    FsrEasu,
    FsrEasuRcas,
    KLDepixelize,
    Nedi,
    TwoSai,
    SuperEagle,
    HqSimple,
    HqTrue,
    Epx,
    Xbr,
    SuperXbr,
    Cut1,
    Cut2,
    Cut3,
    ScaleFx,
    OmniScale,
    Jinc2,
    Jinc2Sharp,
    Jinc2Sharper,
    Jinc2Sharpest,
    Mmpx,
    Vibrance,
    Saturation,
    SelectiveWarm,
    SelectiveGreen,
    LocalLaplacianClarity,
    UnityContrastEnhance,
    AdaptiveLogContrast,
    PaletteSnapStrict,
    PaletteSnapRampAware,
    PaletteSnapExpanded,
    ScaleFxSmartDeblur,
    UnsharpMaskSmall,
    HighPassSharpen,
}

impl Default for UpscalePreviewAlgorithm {
    fn default() -> Self {
        Self::FsrEasuRcas
    }
}

impl UpscalePreviewAlgorithm {
    pub fn all() -> &'static [Self] {
        &[
            Self::None,
            Self::Nearest,
            Self::Bilinear,
            Self::CatmullRom,
            Self::Lanczos3,
            Self::Lq,
            Self::SuperSai,
            Self::FsrEasu,
            Self::FsrEasuRcas,
            Self::KLDepixelize,
            Self::Nedi,
            Self::TwoSai,
            Self::SuperEagle,
            Self::HqSimple,
            Self::HqTrue,
            Self::Epx,
            Self::Xbr,
            Self::SuperXbr,
            Self::Cut1,
            Self::Cut2,
            Self::Cut3,
            Self::ScaleFx,
            Self::OmniScale,
            Self::Jinc2,
            Self::Jinc2Sharp,
            Self::Jinc2Sharper,
            Self::Jinc2Sharpest,
            Self::Mmpx,
            Self::Vibrance,
            Self::Saturation,
            Self::SelectiveWarm,
            Self::SelectiveGreen,
            Self::LocalLaplacianClarity,
            Self::UnityContrastEnhance,
            Self::AdaptiveLogContrast,
            Self::PaletteSnapStrict,
            Self::PaletteSnapRampAware,
            Self::PaletteSnapExpanded,
            Self::ScaleFxSmartDeblur,
            Self::UnsharpMaskSmall,
            Self::HighPassSharpen,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No Upscaling",
            Self::Nearest => "Nearest",
            Self::Bilinear => "Bilinear",
            Self::CatmullRom => "CatmullRom",
            Self::Lanczos3 => "Lanczos3",
            Self::Lq => "lqx",
            Self::SuperSai => "Super2xSaI",
            Self::FsrEasu => "FSR EASU",
            Self::FsrEasuRcas => "FSR EASU + RCAS",
            Self::KLDepixelize => "Kopf-Lischinski",
            Self::Nedi => "NEDI",
            Self::TwoSai => "2xSaI",
            Self::SuperEagle => "SuperEagle",
            Self::HqSimple => "hqx simple",
            Self::HqTrue => "hqx true",
            Self::Epx => "EPX",
            Self::Xbr => "xBR",
            Self::SuperXbr => "Super-xBR",
            Self::Cut1 => "CUT1",
            Self::Cut2 => "CUT2",
            Self::Cut3 => "CUT3",
            Self::ScaleFx => "ScaleFX",
            Self::OmniScale => "OmniScale",
            Self::Jinc2 => "Jinc2",
            Self::Jinc2Sharp => "Jinc2 Sharp",
            Self::Jinc2Sharper => "Jinc2 Sharper",
            Self::Jinc2Sharpest => "Jinc2 Sharpest",
            Self::Mmpx => "MMPX",
            Self::Vibrance => "Vibrance Boost",
            Self::Saturation => "Saturation Boost",
            Self::SelectiveWarm => "Selective Red/Orange Boost",
            Self::SelectiveGreen => "Selective Green Boost",
            Self::LocalLaplacianClarity => "Local Laplacian Clarity",
            Self::UnityContrastEnhance => "Unity Contrast Enhance",
            Self::AdaptiveLogContrast => "Adaptive Log Contrast",
            Self::PaletteSnapStrict => "Palette Snap Strict",
            Self::PaletteSnapRampAware => "Palette Snap Ramp-Aware",
            Self::PaletteSnapExpanded => "Palette Snap Expanded",
            Self::ScaleFxSmartDeblur => "ScaleFX Smart Deblur",
            Self::UnsharpMaskSmall => "Unsharp Mask Small",
            Self::HighPassSharpen => "High-pass Sharpen",
        }
    }

    pub fn scale_options(self) -> &'static [u32] {
        match self {
            Self::None => &[1],
            Self::SuperSai | Self::Nedi | Self::TwoSai | Self::SuperEagle => &[2],
            Self::SuperXbr | Self::Cut1 | Self::Cut2 | Self::Cut3 => &[2],
            Self::Mmpx => &[2, 4],
            Self::Vibrance => &[20, 30, 40],
            Self::Saturation => &[115, 125, 130],
            Self::SelectiveWarm | Self::SelectiveGreen => &[20, 30, 40],
            Self::LocalLaplacianClarity => &[15, 25, 30],
            Self::UnityContrastEnhance => &[20, 35, 50],
            Self::AdaptiveLogContrast => &[75, 80, 90],
            Self::PaletteSnapStrict | Self::PaletteSnapRampAware => &[1],
            Self::PaletteSnapExpanded => &[8, 16, 32],
            Self::ScaleFxSmartDeblur | Self::UnsharpMaskSmall | Self::HighPassSharpen => &[1],
            _ => &[2, 3, 4],
        }
    }

    pub fn scale_value_label(self, value: u32) -> String {
        match self {
            Self::Vibrance
            | Self::SelectiveWarm
            | Self::SelectiveGreen
            | Self::LocalLaplacianClarity
            | Self::UnityContrastEnhance => format!("{value}%"),
            Self::Saturation => format!("{:.2}x", value as f32 / 100.0),
            Self::AdaptiveLogContrast => format!("{:.2} gamma", value as f32 / 100.0),
            Self::PaletteSnapStrict => "strict".to_string(),
            Self::PaletteSnapRampAware => "ramp".to_string(),
            Self::PaletteSnapExpanded => format!("{value} colors"),
            _ => format!("{value}x"),
        }
    }

    pub fn is_palette_snap(self) -> bool {
        matches!(
            self,
            Self::PaletteSnapStrict | Self::PaletteSnapRampAware | Self::PaletteSnapExpanded
        )
    }

    pub fn palette_config(self, scale: u32) -> Option<PaletteUpscaleConfig> {
        let mut config = PaletteUpscaleConfig::default();
        config.snap_mode = match self {
            Self::PaletteSnapStrict => SnapMode::StrictSnap,
            Self::PaletteSnapRampAware => SnapMode::RampAwareSnap,
            Self::PaletteSnapExpanded => SnapMode::ExpandedPalette {
                max_derived_colors: scale as usize,
            },
            _ => return None,
        };
        Some(config)
    }

    pub fn to_filter(self, scale: u32) -> UpscaleFilter {
        let scale = if self.scale_options().contains(&scale) {
            scale
        } else {
            self.scale_options()[0]
        };

        match (self, scale) {
            (Self::None, _) => UpscaleFilter::None,
            (Self::Nearest, 2) => UpscaleFilter::Nearest2x,
            (Self::Nearest, 3) => UpscaleFilter::Nearest3x,
            (Self::Nearest, _) => UpscaleFilter::Nearest4x,
            (Self::Bilinear, 2) => UpscaleFilter::Bilinear2x,
            (Self::Bilinear, 3) => UpscaleFilter::Bilinear3x,
            (Self::Bilinear, _) => UpscaleFilter::Bilinear4x,
            (Self::CatmullRom, 2) => UpscaleFilter::CatmullRom2x,
            (Self::CatmullRom, 3) => UpscaleFilter::CatmullRom3x,
            (Self::CatmullRom, _) => UpscaleFilter::CatmullRom4x,
            (Self::Lanczos3, 2) => UpscaleFilter::Lanczos3_2x,
            (Self::Lanczos3, 3) => UpscaleFilter::Lanczos3_3x,
            (Self::Lanczos3, _) => UpscaleFilter::Lanczos3_4x,
            (Self::Lq, 2) => UpscaleFilter::Lq2x,
            (Self::Lq, 3) => UpscaleFilter::Lq3x,
            (Self::Lq, _) => UpscaleFilter::Lq4x,
            (Self::SuperSai, _) => UpscaleFilter::SuperSai2x,
            (Self::FsrEasu, 2) => UpscaleFilter::FsrEasu2x,
            (Self::FsrEasu, 3) => UpscaleFilter::FsrEasu3x,
            (Self::FsrEasu, _) => UpscaleFilter::FsrEasu4x,
            (Self::FsrEasuRcas, 2) => UpscaleFilter::FsrEasuRcas2x,
            (Self::FsrEasuRcas, 3) => UpscaleFilter::FsrEasuRcas3x,
            (Self::FsrEasuRcas, _) => UpscaleFilter::FsrEasuRcas4x,
            (Self::KLDepixelize, 2) => UpscaleFilter::KLDepixelize2x,
            (Self::KLDepixelize, 3) => UpscaleFilter::KLDepixelize3x,
            (Self::KLDepixelize, _) => UpscaleFilter::KLDepixelize4x,
            (Self::Nedi, _) => UpscaleFilter::Nedi2x,
            (Self::TwoSai, _) => UpscaleFilter::TwoSai2x,
            (Self::SuperEagle, _) => UpscaleFilter::SuperEagle2x,
            (Self::HqSimple, 2) => UpscaleFilter::Hq2xSimple,
            (Self::HqSimple, 3) => UpscaleFilter::Hq3xSimple,
            (Self::HqSimple, _) => UpscaleFilter::Hq4xSimple,
            (Self::HqTrue, 2) => UpscaleFilter::Hq2xTrue,
            (Self::HqTrue, 3) => UpscaleFilter::Hq3xTrue,
            (Self::HqTrue, _) => UpscaleFilter::Hq4xTrue,
            (Self::Epx, 2) => UpscaleFilter::Epx2x,
            (Self::Epx, 3) => UpscaleFilter::Epx3x,
            (Self::Epx, _) => UpscaleFilter::Epx4x,
            (Self::Xbr, 2) => UpscaleFilter::Xbr2x,
            (Self::Xbr, 3) => UpscaleFilter::Xbr3x,
            (Self::Xbr, _) => UpscaleFilter::Xbr4x,
            (Self::SuperXbr, _) => UpscaleFilter::SuperXbr2x,
            (Self::Cut1, _) => UpscaleFilter::Cut1_2x,
            (Self::Cut2, _) => UpscaleFilter::Cut2_2x,
            (Self::Cut3, _) => UpscaleFilter::Cut3_2x,
            (Self::ScaleFx, 2) => UpscaleFilter::ScaleFx2x,
            (Self::ScaleFx, 3) => UpscaleFilter::ScaleFx3x,
            (Self::ScaleFx, _) => UpscaleFilter::ScaleFx4x,
            (Self::OmniScale, 2) => UpscaleFilter::OmniScale2x,
            (Self::OmniScale, 3) => UpscaleFilter::OmniScale3x,
            (Self::OmniScale, _) => UpscaleFilter::OmniScale4x,
            (Self::Jinc2, 2) => UpscaleFilter::Jinc2_2x,
            (Self::Jinc2, 3) => UpscaleFilter::Jinc2_3x,
            (Self::Jinc2, _) => UpscaleFilter::Jinc2_4x,
            (Self::Jinc2Sharp, 2) => UpscaleFilter::Jinc2Sharp2x,
            (Self::Jinc2Sharp, 3) => UpscaleFilter::Jinc2Sharp3x,
            (Self::Jinc2Sharp, _) => UpscaleFilter::Jinc2Sharp4x,
            (Self::Jinc2Sharper, 2) => UpscaleFilter::Jinc2Sharper2x,
            (Self::Jinc2Sharper, 3) => UpscaleFilter::Jinc2Sharper3x,
            (Self::Jinc2Sharper, _) => UpscaleFilter::Jinc2Sharper4x,
            (Self::Jinc2Sharpest, 2) => UpscaleFilter::Jinc2Sharpest2x,
            (Self::Jinc2Sharpest, 3) => UpscaleFilter::Jinc2Sharpest3x,
            (Self::Jinc2Sharpest, _) => UpscaleFilter::Jinc2Sharpest4x,
            (Self::Mmpx, 2) => UpscaleFilter::Mmpx2x,
            (Self::Mmpx, _) => UpscaleFilter::Mmpx4x,
            (Self::Vibrance, 20) => UpscaleFilter::Vibrance20,
            (Self::Vibrance, 30) => UpscaleFilter::Vibrance30,
            (Self::Vibrance, _) => UpscaleFilter::Vibrance40,
            (Self::Saturation, 115) => UpscaleFilter::Saturation115,
            (Self::Saturation, 125) => UpscaleFilter::Saturation125,
            (Self::Saturation, _) => UpscaleFilter::Saturation130,
            (Self::SelectiveWarm, 20) => UpscaleFilter::SelectiveWarm20,
            (Self::SelectiveWarm, 30) => UpscaleFilter::SelectiveWarm30,
            (Self::SelectiveWarm, _) => UpscaleFilter::SelectiveWarm40,
            (Self::SelectiveGreen, 20) => UpscaleFilter::SelectiveGreen20,
            (Self::SelectiveGreen, 30) => UpscaleFilter::SelectiveGreen30,
            (Self::SelectiveGreen, _) => UpscaleFilter::SelectiveGreen40,
            (Self::LocalLaplacianClarity, 15) => UpscaleFilter::LocalLaplacianClarity15,
            (Self::LocalLaplacianClarity, 25) => UpscaleFilter::LocalLaplacianClarity25,
            (Self::LocalLaplacianClarity, _) => UpscaleFilter::LocalLaplacianClarity30,
            (Self::UnityContrastEnhance, 20) => UpscaleFilter::UnityContrastEnhance20,
            (Self::UnityContrastEnhance, 35) => UpscaleFilter::UnityContrastEnhance35,
            (Self::UnityContrastEnhance, _) => UpscaleFilter::UnityContrastEnhance50,
            (Self::AdaptiveLogContrast, 75) => UpscaleFilter::AdaptiveLogContrast75,
            (Self::AdaptiveLogContrast, 80) => UpscaleFilter::AdaptiveLogContrast80,
            (Self::AdaptiveLogContrast, _) => UpscaleFilter::AdaptiveLogContrast90,
            (
                Self::PaletteSnapStrict
                | Self::PaletteSnapRampAware
                | Self::PaletteSnapExpanded,
                _,
            ) => UpscaleFilter::None,
            (Self::ScaleFxSmartDeblur, _) => UpscaleFilter::ScaleFxSmartDeblur,
            (Self::UnsharpMaskSmall, _) => UpscaleFilter::UnsharpMaskSmall,
            (Self::HighPassSharpen, _) => UpscaleFilter::HighPassSharpen,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct UpscalePreviewPass {
    pub algorithm: UpscalePreviewAlgorithm,
    pub scale: u32,
}

impl Default for UpscalePreviewPass {
    fn default() -> Self {
        Self {
            algorithm: UpscalePreviewAlgorithm::default(),
            scale: 2,
        }
    }
}

impl UpscalePreviewPass {
    pub fn clamp_scale(&mut self) {
        if !self.algorithm.scale_options().contains(&self.scale) {
            self.scale = self.algorithm.scale_options()[0];
        }
    }

    pub fn filter(self) -> UpscaleFilter {
        self.algorithm.to_filter(self.scale)
    }

    pub fn palette_config(self) -> Option<PaletteUpscaleConfig> {
        self.algorithm.palette_config(self.scale)
    }

    pub fn display_value(self) -> String {
        if self.algorithm.is_palette_snap() {
            self.algorithm.scale_value_label(self.scale)
        } else {
            format!("{:?}", self.filter())
        }
    }
}

pub struct UpscalePreviewResult {
    pub source_key: u64,
    pub passes: Vec<UpscalePreviewPass>,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub elapsed_ms: u128,
    pub palette_status: String,
}

pub struct UopEntryLabel {
    pub hash: u64,
    pub display_name: String,
    pub search_name: String,
}

fn should_show_uop_entry(file: &uocf::uop_container::file::UopFile, hide_empty_entries: bool) -> bool {
    !hide_empty_entries || file.has_size()
}

pub fn guess_uop_image_format_from_payload(data: &[u8]) -> Option<(&'static str, ECImageFormat)> {
    if data.starts_with(b"DDS ") {
        return Some(("dds", ECImageFormat::DDS));
    }

    if data.len() >= 18 {
        let image_type = data[2];
        if (image_type == 2 || image_type == 10) && data[1] <= 1 {
            return Some(("tga", ECImageFormat::TGA));
        }
    }

    None
}

fn terrain_texture_guess_candidate(texture_id: u32, extension: &str) -> String {
    format!("build/terraintexture/{texture_id:08}.{extension}")
}

fn collect_terrain_texture_guess_names(package: &UopPackage) -> HashMap<u64, String> {
    let mut names = HashMap::new();
    for texture_id in 0..=TERRAIN_TEXTURE_GUESS_MAX_ID {
        for extension in TERRAIN_TEXTURE_GUESS_EXTENSIONS {
            let candidate = terrain_texture_guess_candidate(texture_id, extension);
            let hash = hash_file_name_single(&candidate);
            if package.get_file_by_hash(hash).is_some() {
                names.entry(hash).or_insert(candidate);
            }
        }
    }
    names
}

fn is_terrain_texture_uop_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("terraintexture.uop"))
        .unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct AnimationFrameUopEntry {
    pub package_index: usize,
    pub file_hash: u64,
    pub body_id: u32,
    pub action_id: Option<u16>,
    pub direction: Option<u8>,
    pub group_id: Option<u8>,
    pub source_index: u32,
    pub frame_count: usize,
}

pub fn upscale_filter_cli_value(filter: UpscaleFilter) -> &'static str {
    match filter {
        UpscaleFilter::None => "none",
        UpscaleFilter::Nearest2x => "nearest2x",
        UpscaleFilter::Nearest3x => "nearest3x",
        UpscaleFilter::Nearest4x => "nearest4x",
        UpscaleFilter::Bilinear2x => "bilinear2x",
        UpscaleFilter::Bilinear3x => "bilinear3x",
        UpscaleFilter::Bilinear4x => "bilinear4x",
        UpscaleFilter::CatmullRom2x => "catmull-rom2x",
        UpscaleFilter::CatmullRom3x => "catmull-rom3x",
        UpscaleFilter::CatmullRom4x => "catmull-rom4x",
        UpscaleFilter::Lanczos3_2x => "lanczos3-2x",
        UpscaleFilter::Lanczos3_3x => "lanczos3-3x",
        UpscaleFilter::Lanczos3_4x => "lanczos3-4x",
        UpscaleFilter::SuperSai2x => "super-sai2x",
        UpscaleFilter::FsrEasu2x => "fsr-easu2x",
        UpscaleFilter::FsrEasu3x => "fsr-easu3x",
        UpscaleFilter::FsrEasu4x => "fsr-easu4x",
        UpscaleFilter::FsrEasuRcas2x => "fsr-easu-rcas2x",
        UpscaleFilter::FsrEasuRcas3x => "fsr-easu-rcas3x",
        UpscaleFilter::FsrEasuRcas4x => "fsr-easu-rcas4x",
        UpscaleFilter::KLDepixelize2x => "kl-depixelize2x",
        UpscaleFilter::KLDepixelize3x => "kl-depixelize3x",
        UpscaleFilter::KLDepixelize4x => "kl-depixelize4x",
        UpscaleFilter::Nedi2x => "nedi2x",
        UpscaleFilter::TwoSai2x => "two-sai2x",
        UpscaleFilter::SuperEagle2x => "super-eagle2x",
        UpscaleFilter::Lq2x => "lq2x",
        UpscaleFilter::Lq3x => "lq3x",
        UpscaleFilter::Lq4x => "lq4x",
        UpscaleFilter::Hq2xSimple => "hq2x-simple",
        UpscaleFilter::Hq3xSimple => "hq3x-simple",
        UpscaleFilter::Hq4xSimple => "hq4x-simple",
        UpscaleFilter::Hq2xTrue => "hq2x-true",
        UpscaleFilter::Hq3xTrue => "hq3x-true",
        UpscaleFilter::Hq4xTrue => "hq4x-true",
        UpscaleFilter::Epx2x => "epx2x",
        UpscaleFilter::Epx3x => "epx3x",
        UpscaleFilter::Epx4x => "epx4x",
        UpscaleFilter::Xbr2x => "xbr2x",
        UpscaleFilter::Xbr3x => "xbr3x",
        UpscaleFilter::Xbr4x => "xbr4x",
        UpscaleFilter::SuperXbr2x => "super-xbr2x",
        UpscaleFilter::Cut1_2x => "cut1-2x",
        UpscaleFilter::Cut2_2x => "cut2-2x",
        UpscaleFilter::Cut3_2x => "cut3-2x",
        UpscaleFilter::ScaleFx2x => "scalefx2x",
        UpscaleFilter::ScaleFx3x => "scalefx3x",
        UpscaleFilter::ScaleFx4x => "scalefx4x",
        UpscaleFilter::OmniScale2x => "omniscale2x",
        UpscaleFilter::OmniScale3x => "omniscale3x",
        UpscaleFilter::OmniScale4x => "omniscale4x",
        UpscaleFilter::Jinc2_2x => "jinc2-2x",
        UpscaleFilter::Jinc2_3x => "jinc2-3x",
        UpscaleFilter::Jinc2_4x => "jinc2-4x",
        UpscaleFilter::Jinc2Sharp2x => "jinc2-sharp2x",
        UpscaleFilter::Jinc2Sharp3x => "jinc2-sharp3x",
        UpscaleFilter::Jinc2Sharp4x => "jinc2-sharp4x",
        UpscaleFilter::Jinc2Sharper2x => "jinc2-sharper2x",
        UpscaleFilter::Jinc2Sharper3x => "jinc2-sharper3x",
        UpscaleFilter::Jinc2Sharper4x => "jinc2-sharper4x",
        UpscaleFilter::Jinc2Sharpest2x => "jinc2-sharpest2x",
        UpscaleFilter::Jinc2Sharpest3x => "jinc2-sharpest3x",
        UpscaleFilter::Jinc2Sharpest4x => "jinc2-sharpest4x",
        UpscaleFilter::Mmpx2x => "mmpx2x",
        UpscaleFilter::Mmpx4x => "mmpx4x",
        UpscaleFilter::Vibrance20 => "vibrance20",
        UpscaleFilter::Vibrance30 => "vibrance30",
        UpscaleFilter::Vibrance40 => "vibrance40",
        UpscaleFilter::Saturation115 => "saturation115",
        UpscaleFilter::Saturation125 => "saturation125",
        UpscaleFilter::Saturation130 => "saturation130",
        UpscaleFilter::SelectiveWarm20 => "selective-warm20",
        UpscaleFilter::SelectiveWarm30 => "selective-warm30",
        UpscaleFilter::SelectiveWarm40 => "selective-warm40",
        UpscaleFilter::SelectiveGreen20 => "selective-green20",
        UpscaleFilter::SelectiveGreen30 => "selective-green30",
        UpscaleFilter::SelectiveGreen40 => "selective-green40",
        UpscaleFilter::ScaleFxSmartDeblur => "scalefx-smart-deblur",
        UpscaleFilter::UnsharpMaskSmall => "unsharp-mask-small",
        UpscaleFilter::HighPassSharpen => "high-pass-sharpen",
    }
}

pub fn upscale_pass_cli_value(pass: UpscalePreviewPass) -> String {
    if pass.algorithm.is_palette_snap() {
        match pass.algorithm {
            UpscalePreviewAlgorithm::PaletteSnapStrict => "palette-snap-strict".to_string(),
            UpscalePreviewAlgorithm::PaletteSnapRampAware => {
                "palette-snap-ramp-aware".to_string()
            }
            UpscalePreviewAlgorithm::PaletteSnapExpanded => {
                format!("palette-snap-expanded-{}", pass.scale)
            }
            _ => unreachable!(),
        }
    } else {
        upscale_filter_cli_value(pass.filter()).to_string()
    }
}

fn apply_upscale_preview_passes(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    passes: &[UpscalePreviewPass],
) -> (u32, u32, Vec<u8>, String) {
    let transparency = TransparencyPolicy::default();
    let source_palette = PaletteModel::from_rgba(width, height, &rgba, &transparency).ok();
    let mut width = width;
    let mut height = height;
    let mut rgba = rgba;
    let mut palette_status = String::new();
    let mut index = 0usize;

    while index < passes.len() {
        let pass = passes[index];
        if pass.algorithm.is_palette_snap() {
            if let Some(config) = pass.palette_config() {
                match apply_palette_snap_pass(
                    width,
                    height,
                    &rgba,
                    source_palette.as_ref(),
                    &config,
                    UpscaleFilter::None,
                ) {
                    Ok((next_width, next_height, next_rgba, status)) => {
                        width = next_width;
                        height = next_height;
                        rgba = next_rgba;
                        palette_status = status;
                    }
                    Err(status) => palette_status = status,
                }
            }
            index += 1;
            continue;
        }

        let filter = pass.filter();
        if let Some(next_pass) = passes.get(index + 1).copied() {
            if let Some(config) = next_pass.palette_config() {
                match apply_palette_snap_pass(
                    width,
                    height,
                    &rgba,
                    source_palette.as_ref(),
                    &config,
                    filter,
                ) {
                    Ok((next_width, next_height, next_rgba, status)) => {
                        width = next_width;
                        height = next_height;
                        rgba = next_rgba;
                        palette_status = status;
                    }
                    Err(status) => {
                        let (next_width, next_height, next_rgba) =
                            filter.apply(width, height, &rgba);
                        width = next_width;
                        height = next_height;
                        rgba = next_rgba;
                        palette_status = status;
                    }
                }
                index += 2;
                continue;
            }
        }

        let (next_width, next_height, next_rgba) = filter.apply(width, height, &rgba);
        width = next_width;
        height = next_height;
        rgba = next_rgba;
        index += 1;
    }

    (width, height, rgba, palette_status)
}

fn apply_palette_snap_pass(
    width: u32,
    height: u32,
    rgba: &[u8],
    source_palette: Option<&PaletteModel>,
    config: &PaletteUpscaleConfig,
    filter: UpscaleFilter,
) -> Result<(u32, u32, Vec<u8>, String), String> {
    let scaler = RgbaFilterScaler::new(filter);
    let result = palette_safe_upscale(width, height, rgba, source_palette, &scaler, config)
        .map_err(|error| format!("Palette snap failed: {error:?}"))?;
    let status = format!(
        "Palette snap: {} off-palette candidates, {} source colors, {} final colors",
        result.diagnostics.off_palette_candidates,
        result
            .diagnostics
            .stages
            .first()
            .map_or(0, |stage| stage.distinct_opaque_colors),
        result
            .diagnostics
            .stages
            .last()
            .map_or(0, |stage| stage.distinct_opaque_colors)
    );
    Ok((result.width, result.height, result.rgba, status))
}

#[derive(Clone, Debug)]
pub struct PaperdollEquipmentInput {
    pub slot: String,
    pub item_id: String,
    pub hue: String,
}

impl Default for PaperdollEquipmentInput {
    fn default() -> Self {
        Self {
            slot: String::new(),
            item_id: String::new(),
            hue: "0".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct TerrainDefinitionFileEntry {
    pub filename_hash: u64,
    pub byte_len: usize,
    pub raw_prefix: Vec<u8>,
    pub entry: TerrainDefinitionEntry,
}

#[derive(Clone)]
pub struct TileArtFileEntry {
    pub filename_hash: u64,
    pub entry: TileArtEntry,
}

#[derive(Clone)]
pub struct CcTileDataRow {
    pub art_id: u32,
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub flags_raw: String,
    pub flags_summary: String,
    pub texture_id: String,
    pub height: String,
    pub weight: String,
    pub quality: String,
    pub quantity: String,
    pub anim_id: String,
    pub hue_extra: String,
    pub stacking_offset: String,
    pub value: String,
    pub search_text: String,
}

#[derive(Clone)]
pub struct ArtViewerRow {
    pub art_id: u32,
    pub label: String,
    pub group: &'static str,
    pub search_text: String,
}

#[derive(Clone)]
pub struct TileArtDisplayRow {
    pub filename_hash: u64,
    pub tile_id: String,
    pub old_id: String,
    pub type_name: &'static str,
    pub properties_summary: String,
    pub flags_raw: String,
    pub flags_summary: String,
    pub ec_window: String,
    pub ec_offset: String,
    pub cc_window: String,
    pub cc_offset: String,
    pub texture_summary: String,
    pub sitting_summary: String,
    pub appearance_summary: String,
    pub search_text: String,
}

#[derive(Clone)]
pub struct LocalizedStringDisplayFile {
    pub filename_hash: u64,
    pub rows: Vec<LocalizedStringDisplayRow>,
}

#[derive(Clone)]
pub struct LocalizedStringDisplayRow {
    pub entry_index: usize,
    pub id: String,
    pub unk: String,
    pub search_text: String,
}

#[derive(Clone, Debug)]
pub struct SoundListEntry {
    pub slot_id: u32,
    pub name: String,
    pub pcm_bytes: usize,
    pub duration_seconds: f64,
}

#[derive(Clone)]
pub struct ClilocFileEntry {
    pub label: String,
    pub path: PathBuf,
    pub cliloc: Arc<Cliloc>,
}

pub struct SoundPlayer {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    sink: Option<rodio::Sink>,
}

#[derive(Clone)]
pub struct InspectorImagePreview {
    pub key: u64,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl SoundPlayer {
    pub fn new() -> color_eyre::eyre::Result<Self> {
        let (_stream, handle) = rodio::OutputStream::try_default()?;
        Ok(Self {
            _stream,
            handle,
            sink: None,
        })
    }

    pub fn play_pcm(&mut self, pcm_data: &[u8]) -> color_eyre::eyre::Result<()> {
        self.stop();
        let samples: Vec<i16> = pcm_data
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect();
        let source = rodio::buffer::SamplesBuffer::new(
            uocf::classic::sound::CHANNELS,
            uocf::classic::sound::SAMPLE_RATE,
            samples,
        );
        let sink = rodio::Sink::try_new(&self.handle)?;
        sink.append(source);
        self.sink = Some(sink);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }

    pub fn is_playing(&self) -> bool {
        self.sink.as_ref().map_or(false, |sink| !sink.empty())
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AppSettings {
    pub cc_path: Option<PathBuf>,
    pub ec_path: Option<PathBuf>,
    pub dict_path: Option<PathBuf>,
    pub last_view_mode: Option<ViewMode>,
}

fn collect_sound_entries(sounds: &SoundMap) -> Vec<SoundListEntry> {
    let mut entries = Vec::new();
    for slot_id in 0..sounds.slot_count() as u32 {
        if let Ok(Some(sound)) = sounds.read_slot(slot_id) {
            let pcm_bytes = sound.pcm_data.len();
            let duration_seconds = sound.duration_seconds();
            entries.push(SoundListEntry {
                slot_id,
                name: sound.name,
                pcm_bytes,
                duration_seconds,
            });
        }
    }
    entries
}

fn collect_cc_tiledata_rows(tiledata: &TileData) -> Vec<CcTileDataRow> {
    let mut rows = Vec::with_capacity(tiledata.land_tiles().len() + tiledata.item_tiles().len());

    for tile in tiledata.land_tiles() {
        let art_id = tile.tile_id as u32;
        let id = tile.tile_id.to_string();
        let name = tile.name_ascii().to_string();
        let flags_raw = format!("0x{:08X}", tile.flags.internal_flags);
        let flags_summary = classic_tile_flags_summary(tile.flags);
        let texture_id = tile.texture_id.to_string();
        let search_text = format!(
            "{} land {} {} {} {}",
            id,
            name.to_lowercase(),
            flags_raw.to_lowercase(),
            flags_summary.to_lowercase(),
            texture_id
        );
        rows.push(CcTileDataRow {
            art_id,
            id,
            kind: "Land",
            name,
            flags_raw,
            flags_summary,
            texture_id,
            height: String::new(),
            weight: String::new(),
            quality: String::new(),
            quantity: String::new(),
            anim_id: String::new(),
            hue_extra: String::new(),
            stacking_offset: String::new(),
            value: String::new(),
            search_text,
        });
    }

    for tile in tiledata.item_tiles() {
        let art_id = tile.tile_id as u32 + 0x4000;
        let id = tile.tile_id.to_string();
        let name = tile.name_ascii().to_string();
        let flags_raw = format!("0x{:08X}", tile.flags.internal_flags);
        let flags_summary = classic_tile_flags_summary(tile.flags);
        let height = tile.height_raw().to_string();
        let weight = tile.weight.to_string();
        let quality = tile.quality.to_string();
        let quantity = tile.quantity.to_string();
        let anim_id = tile.anim_id.to_string();
        let hue_extra = tile.hue_extra.to_string();
        let stacking_offset = tile.stacking_offset.to_string();
        let value = tile.value.to_string();
        let search_text = format!(
            "{} item {} {} {} height {} weight {} quality {} quantity {} anim {} hue {} stack {} value {}",
            id,
            name.to_lowercase(),
            flags_raw.to_lowercase(),
            flags_summary.to_lowercase(),
            height,
            weight,
            quality,
            quantity,
            anim_id,
            hue_extra,
            stacking_offset,
            value
        );
        rows.push(CcTileDataRow {
            art_id,
            id,
            kind: "Item",
            name,
            flags_raw,
            flags_summary,
            texture_id: String::new(),
            height,
            weight,
            quality,
            quantity,
            anim_id,
            hue_extra,
            stacking_offset,
            value,
            search_text,
        });
    }

    rows
}

fn collect_tileart_display_rows(
    entries: &[TileArtFileEntry],
    string_dictionary: Option<&UoStringDictionary>,
) -> Vec<TileArtDisplayRow> {
    entries
        .iter()
        .map(|file| {
            let entry = &file.entry;
            let tile_id = entry.tile_id.to_string();
            let old_id = entry.old_id.to_string();
            let type_name = tileart_type_name(entry.type_val);
            let properties_summary = tileart_properties_summary(entry);
            let flags_raw = format!("0x{:016X}", entry.flags1.bits());
            let flags_summary = tileart_flags_summary(entry.flags1);
            let ec_window = format!(
                "{},{} -> {},{}",
                entry.ec_img_offset.x_start,
                entry.ec_img_offset.y_start,
                entry.ec_img_offset.x_end,
                entry.ec_img_offset.y_end
            );
            let ec_offset = format!(
                "{},{}",
                entry.ec_img_offset.x_off,
                entry.ec_img_offset.y_off
            );
            let cc_window = format!(
                "{},{} -> {},{}",
                entry.cc_img_offset.x_start,
                entry.cc_img_offset.y_start,
                entry.cc_img_offset.x_end,
                entry.cc_img_offset.y_end
            );
            let cc_offset = format!(
                "{},{}",
                entry.cc_img_offset.x_off,
                entry.cc_img_offset.y_off
            );
            let texture_summary = tileart_texture_summary(entry, string_dictionary);
            let sitting_summary = tileart_sitting_summary(entry.sitting.as_ref());
            let appearance_summary = tileart_appearance_summary(&entry.appearance_vector);
            let search_text = format!(
                "{} {} {} {} {} {} {} {}",
                tile_id,
                old_id,
                type_name.to_lowercase(),
                properties_summary.to_lowercase(),
                flags_raw.to_lowercase(),
                flags_summary.to_lowercase(),
                texture_summary.to_lowercase(),
                appearance_summary.to_lowercase()
            );
            TileArtDisplayRow {
                filename_hash: file.filename_hash,
                tile_id,
                old_id,
                type_name,
                properties_summary,
                flags_raw,
                flags_summary,
                ec_window,
                ec_offset,
                cc_window,
                cc_offset,
                texture_summary,
                sitting_summary,
                appearance_summary,
                search_text,
            }
        })
        .collect()
}

fn classic_tile_flags_summary(flags: uocf::classic::tiledata::Flags) -> String {
    let labels = [
        (flags.background(), "Background"),
        (flags.weapon(), "Weapon"),
        (flags.transparent(), "Transparent"),
        (flags.translucent(), "Translucent"),
        (flags.wall(), "Wall"),
        (flags.damaging(), "Damaging"),
        (flags.impassable(), "Impassable"),
        (flags.wet(), "Wet"),
        (flags.internal_flags & 0x0000_0100 != 0, "Unknown08"),
        (flags.surface(), "Surface"),
        (flags.bridge(), "Bridge"),
        (flags.generic(), "Generic/Stackable"),
        (flags.window(), "Window"),
        (flags.noshoot(), "NoShoot"),
        (flags.prefixa(), "ArticleA"),
        (flags.prefixan(), "ArticleAn"),
        (flags.internal(), "Internal"),
        (flags.foliage(), "Foliage"),
        (flags.partialhue(), "PartialHue"),
        (flags.internal_flags & 0x0008_0000 != 0, "Unknown19"),
        (flags.map(), "Map"),
        (flags.container(), "Container"),
        (flags.wearable(), "Wearable"),
        (flags.lightsource(), "LightSource"),
        (flags.animated(), "Animated"),
        (flags.nodiagonal(), "NoDiagonal"),
        (flags.internal_flags & 0x0400_0000 != 0, "Unknown26"),
        (flags.armor(), "Armor"),
        (flags.roof(), "Roof"),
        (flags.door(), "Door"),
        (flags.stairback(), "StairBack"),
        (flags.stairright(), "StairRight"),
    ];

    let active = labels
        .iter()
        .filter_map(|(active, label)| active.then_some(*label))
        .collect::<Vec<_>>();
    if active.is_empty() {
        "none".to_string()
    } else {
        active.join(", ")
    }
}

pub(crate) fn tileart_type_name(type_val: i32) -> &'static str {
    match type_val {
        0 => "Static",
        1 => "Solid",
        2 => "Liquid",
        _ => "Unknown",
    }
}

pub(crate) fn tileart_property_name(id: u8) -> &'static str {
    match id {
        0 => "Weight",
        1 => "Quality",
        2 => "Quantity",
        3 => "Height",
        4 => "Value",
        5 => "AC/VC",
        6 => "Slot",
        7 => "OffC8",
        8 => "Appearance",
        9 => "Race",
        10 => "Gender",
        11 => "Paperdoll",
        _ => "Unknown",
    }
}

fn tileart_properties_summary(entry: &TileArtEntry) -> String {
    let mut parts = Vec::new();
    for prop in entry.prop_vector1.iter().chain(entry.prop_vector2.iter()) {
        parts.push(format!("{}={}", tileart_property_name(prop.id), prop.val));
    }

    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

pub(crate) fn tileart_flags_summary(flags: TaeFlag) -> String {
    let labels = [
        (TaeFlag::Background, "Background"),
        (TaeFlag::Weapon, "Weapon"),
        (TaeFlag::Transparent, "Transparent"),
        (TaeFlag::Translucent, "Translucent"),
        (TaeFlag::Wall, "Wall"),
        (TaeFlag::Damaging, "Damaging"),
        (TaeFlag::Impassable, "Impassable"),
        (TaeFlag::Wet, "Wet"),
        (TaeFlag::Ignored, "Ignored"),
        (TaeFlag::Surface, "Surface"),
        (TaeFlag::Bridge, "Bridge"),
        (TaeFlag::Generic, "Generic"),
        (TaeFlag::Window, "Window"),
        (TaeFlag::NoShoot, "NoShoot"),
        (TaeFlag::ArticleA, "ArticleA"),
        (TaeFlag::ArticleAn, "ArticleAn"),
        (TaeFlag::Mongen, "Mongen"),
        (TaeFlag::Foliage, "Foliage"),
        (TaeFlag::PartialHue, "PartialHue"),
        (TaeFlag::UseNewArt, "UseNewArt"),
        (TaeFlag::Map, "Map"),
        (TaeFlag::Container, "Container"),
        (TaeFlag::Wearable, "Wearable"),
        (TaeFlag::LightSource, "LightSource"),
        (TaeFlag::Animation, "Animation"),
        (TaeFlag::HoverOver, "HoverOver"),
        (TaeFlag::ArtUsed, "ArtUsed"),
        (TaeFlag::Armor, "Armor"),
        (TaeFlag::Roof, "Roof"),
        (TaeFlag::Door, "Door"),
        (TaeFlag::StairBack, "StairBack"),
        (TaeFlag::StairRight, "StairRight"),
        (TaeFlag::NoHouse, "NoHouse"),
        (TaeFlag::NoDraw, "NoDraw"),
        (TaeFlag::Unused1, "Unused1"),
        (TaeFlag::AlphaBlend, "AlphaBlend"),
        (TaeFlag::NoShadow, "NoShadow"),
        (TaeFlag::PixelBleed, "PixelBleed"),
        (TaeFlag::Unused2, "Unused2"),
        (TaeFlag::PlayAnimOnce, "PlayAnimOnce"),
        (TaeFlag::MultiMovable, "MultiMovable"),
    ];

    let active = labels
        .iter()
        .filter_map(|(flag, label)| flags.contains(*flag).then_some(*label))
        .collect::<Vec<_>>();
    if active.is_empty() {
        "none".to_string()
    } else {
        active.join(", ")
    }
}

fn tileart_texture_summary(
    entry: &TileArtEntry,
    string_dictionary: Option<&UoStringDictionary>,
) -> String {
    if let Some(dict) = string_dictionary {
        let art_data = entry.process(dict);
        let mut parts = Vec::new();
        for block in art_data.texture_items {
            for item in block {
                parts.push(format!("{}:{:?}:{}", item.id, item.texture_type, item.path));
            }
        }
        if !parts.is_empty() {
            return parts.join(" | ");
        }
    }

    let mut parts = Vec::new();
    for (block_index, block) in entry.texture_vector.iter().enumerate() {
        if block.has_texture == 1 {
            parts.push(format!("block {}: {} refs", block_index, block.texture_items_count));
        }
    }
    parts.join(" | ")
}

fn tileart_sitting_summary(sitting: Option<&TaeSittingAnimation>) -> String {
    if let Some(sitting) = sitting {
        format!(
            "yes ({}, {}, {}, {})",
            sitting.unk1, sitting.unk2, sitting.unk3, sitting.unk4
        )
    } else {
        "no".to_string()
    }
}

fn tileart_appearance_summary(appearance: &[TaeAnimationAppearance]) -> String {
    if appearance.is_empty() {
        return "none".to_string();
    }

    let mut counts = [0usize; 2];
    let mut other = 0usize;
    for item in appearance {
        match item.sub_type {
            0 => counts[0] += 1,
            1 => counts[1] += 1,
            _ => other += 1,
        }
    }

    format!(
        "{} records (type0 {}, type1 {}, other {})",
        appearance.len(),
        counts[0],
        counts[1],
        other
    )
}

fn collect_localized_string_rows(
    package: &LocalizedStringsPackage,
) -> Vec<LocalizedStringDisplayFile> {
    package
        .files
        .iter()
        .map(|file| LocalizedStringDisplayFile {
            filename_hash: file.filename_hash,
            rows: file
                .strings
                .entries
                .iter()
                .enumerate()
                .map(|(entry_index, entry)| LocalizedStringDisplayRow {
                    entry_index,
                    id: entry.id.to_string(),
                    unk: format!("0x{:02X}", entry.unk),
                    search_text: format!("{} {}", entry.id, entry.text.to_lowercase()),
                })
                .collect(),
        })
        .collect()
}

fn load_optional_gumps_package(
    base_path: &Path,
    file_name: &str,
    mut log: impl FnMut(String),
) -> Option<GumpsPackage> {
    let package_path = base_path.join(file_name);
    if !package_path.exists() {
        return None;
    }

    log(format!("Loading gump package from {}", package_path.display()));
    match GumpsPackage::load(&package_path) {
        Ok(package) => {
            log(format!("Successfully loaded {file_name}."));
            Some(package)
        }
        Err(error) => {
            log(format!("Failed to load {file_name}: {error}"));
            None
        }
    }
}

fn load_cliloc_files(base_path: &Path, mut log: impl FnMut(String)) -> Vec<ClilocFileEntry> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let Ok(entries) = std::fs::read_dir(base_path) else {
        return Vec::new();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_cliloc_file_name(name) {
            continue;
        }
        candidates.push(path);
    }

    candidates.sort_by_key(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default()
    });

    let mut clilocs = Vec::new();
    for path in candidates {
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.display().to_string());
        match Cliloc::load(&path) {
            Ok(cliloc) => clilocs.push(ClilocFileEntry {
                label,
                path,
                cliloc: Arc::new(cliloc),
            }),
            Err(error) => log(format!("Failed to load {label}: {error}")),
        }
    }

    clilocs
}

fn is_cliloc_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("cliloc.") && lower.len() > "cliloc.".len()
}

fn default_cliloc_file_index(clilocs: &[ClilocFileEntry]) -> Option<usize> {
    clilocs
        .iter()
        .position(|entry| entry.label.eq_ignore_ascii_case("cliloc.enu"))
        .or_else(|| clilocs.first().map(|_| 0))
}

fn find_localized_strings_uop(base_path: &Path) -> Option<PathBuf> {
    find_client_file_case_insensitive(base_path, LOCALIZED_STRINGS_UOP_NAME)
}

fn find_client_file_case_insensitive(base_path: &Path, file_name: &str) -> Option<PathBuf> {
    let direct_path = base_path.join(file_name);
    if direct_path.exists() {
        return Some(direct_path);
    }

    let entries = std::fs::read_dir(base_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(file_name))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }

    None
}

pub struct UopInspectorApp {
    pub settings: AppSettings,
    pub logs: Vec<String>,
    pub show_search_paths: bool,

    pub dictionary: Dictionary,
    pub uo_string_dictionary: Option<Arc<UoStringDictionary>>,
    pub cliloc: Option<Arc<Cliloc>>,
    pub cliloc_files: Vec<ClilocFileEntry>,
    pub localized_strings: Option<Arc<LocalizedStringsPackage>>,
    pub localized_string_rows: Option<Arc<Vec<LocalizedStringDisplayFile>>>,
    pub string_dictionary_raw_hash: Option<u64>,
    pub uop_cache: UopCache,
    pub client_data: Option<ClientData>,
    pub cc_tiledata: Option<Arc<TileData>>,
    pub cc_tiledata_rows: Option<Arc<Vec<CcTileDataRow>>>,
    pub art_viewer_rows: Option<Arc<Vec<ArtViewerRow>>>,
    pub art_viewer_rows_source: Option<ArtSource>,
    pub cc_gumps_package: Option<Arc<GumpsPackage>>,
    pub ec_gumps_package: Option<Arc<GumpsPackage>>,
    pub cc_gumps: Option<Arc<GumpMap>>,
    pub cc_sounds: Option<Arc<SoundMap>>,
    pub cc_sound_entries: Option<Arc<Vec<SoundListEntry>>>,
    pub cc_multimap: Option<Arc<MultimapRleImage>>,
    pub cc_multimap_path: Option<PathBuf>,
    pub generated_multimap_plain: Option<GeneratedMultimapPreview>,
    pub generated_multimap_artistic: Option<GeneratedMultimapPreview>,
    pub sound_player: Option<SoundPlayer>,

    pub selected_uop_idx: Option<usize>,
    pub selected_file_hash: Option<u64>,
    pub selected_tex_art_cc_id: Option<u32>,
    pub selected_terrain_def_hash: Option<u64>,
    pub selected_tileart_hash: Option<u64>,
    pub selected_ec_hue_hash: Option<u64>,
    pub selected_multi_uop_hash: Option<u64>,
    pub selected_localized_file_hash: Option<u64>,
    pub selected_cliloc_file_idx: Option<usize>,
    // pub selected_cc_tile_id: Option<u32>,
    pub selected_legacy_source: ArtSource,

    pub search_query: String,
    pub find_hash_query: String,
    pub hide_empty_uop_entries: bool,
    pub guess_terrain_texture_file_format: bool,
    pub status_message: String,
    pub view_mode: ViewMode,
    pub tile_metadata_source: TileMetadataSource,
    pub hues_source: HuesSource,
    pub multis_source: MultisSource,
    pub localized_strings_source: LocalizedStringsSource,
    pub multimap_preview_selection: MultimapPreviewSelection,
    pub multimap_converter: MultimapConverterState,
    pub multimap_worker_rx: Option<mpsc::Receiver<MultimapWorkerResult>>,
    pub multimap_worker_active: bool,

    pub texture_previews: HashMap<u64, egui::TextureHandle>,
    pub ec_texture_previews: HashMap<u32, egui::TextureHandle>,
    pub ec_texture_preview_source_keys: HashMap<u32, u64>,
    pub uop_entry_labels: HashMap<usize, Arc<Vec<UopEntryLabel>>>,
    pub uop_entry_payloads: HashMap<(usize, u64), Arc<[u8]>>,
    pub terrain_texture_guess_names: HashMap<usize, Arc<HashMap<u64, String>>>,
    pub animationframe_uop_entries: HashMap<(u8, usize), Arc<Vec<AnimationFrameUopEntry>>>,
    pub multimap_texture: Option<egui::TextureHandle>,
    pub image_preview_sources: HashMap<u64, InspectorImagePreview>,
    pub current_image_preview_key: Option<u64>,
    pub show_upscale_preview: bool,
    pub upscale_preview_passes: Vec<UpscalePreviewPass>,
    pub upscale_preview_zoom: f32,
    pub upscale_original_texture: Option<egui::TextureHandle>,
    pub upscale_original_texture_key: Option<u64>,
    pub upscale_preview_texture: Option<egui::TextureHandle>,
    pub upscale_preview_texture_key: Option<(u64, Vec<UpscalePreviewPass>)>,
    pub upscale_preview_worker_key: Option<(u64, Vec<UpscalePreviewPass>)>,
    pub upscale_preview_worker_rx: Option<mpsc::Receiver<UpscalePreviewResult>>,
    pub upscale_preview_size: [u32; 2],
    pub upscale_preview_elapsed_ms: Option<u128>,
    pub upscale_preview_status: String,

    // Gumps and paperdolls
    pub selected_gump_source: GumpSource,
    pub selected_gump_viewer_tab: GumpViewerTab,
    pub selected_gump_id: String,
    pub standard_gump_ids: Vec<u32>,
    pub paperdoll_gump_ids: Vec<u32>,
    pub gump_list_source: Option<GumpSource>,
    pub gump_list_profile: String,
    pub gump_list_status: String,
    pub selected_paperdoll_profile: String,
    pub paperdoll_body_id: String,
    pub paperdoll_body_hue: String,
    pub paperdoll_equipment: Vec<PaperdollEquipmentInput>,
    pub paperdoll_preview: Option<egui::TextureHandle>,

    // Animations
    pub selected_anim_id: u32,
    pub selected_animdata_id: u32,
    pub selected_animdata_art_source: ArtSource,
    pub animdata_frame_delay_ms: f32,
    pub selected_anim_file_idx: u8,
    pub current_frame_idx: usize,
    pub is_playing: bool,
    pub last_frame_time: f64,
    pub playback_speed: f32,
    pub loop_animation: bool,

    pub selected_anim_sequence: Option<uocf::animation_sequence::AnimationSequence>,
    pub selected_action_id: u16,
    pub selected_direction: u8,

    pub selected_multi_id: u32,
    pub selected_sound_slot: u32,
    pub selected_sound_id: u32,
    pub sound_search_query: String,
    pub selected_hue_id: u16,
    pub selected_ec_hue_id: u16,
    pub selected_ec_hueing_mode: EcHueingMode,
    pub selected_cliloc_number: i32,
    pub multimap_zoom: f32,

    pub terrain_def_package: Option<Arc<uocf::enhanced::terrain_definition::TerrainDefinitionPackage>>,
    pub terrain_def_files: Option<Arc<Vec<TerrainDefinitionFileEntry>>>,
    pub ec_tileart_entries: Option<Arc<Vec<TileArtFileEntry>>>,
    pub ec_tileart_rows: Option<Arc<Vec<TileArtDisplayRow>>>,
    pub ec_hues: Option<Arc<EcHuePackage>>,
    pub multi_collection: Option<Arc<MultiCollection>>,
    pub multi_collection_source: Option<MultiCollectionSource>,
    pub multi_collection_path: Option<PathBuf>,
}

impl UopInspectorApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        color_eyre::install().ok();

        let settings: AppSettings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "uocf_inspector_settings"))
            .unwrap_or_default();

        let mut app = Self {
            settings: settings.clone(),
            logs: Vec::new(),
            show_search_paths: false,

            dictionary: Dictionary::new(),
            uo_string_dictionary: None,
            cliloc: None,
            cliloc_files: Vec::new(),
            localized_strings: None,
            localized_string_rows: None,
            string_dictionary_raw_hash: None,
            uop_cache: UopCache::new(),
            client_data: None,
            cc_tiledata: None,
            cc_tiledata_rows: None,
            art_viewer_rows: None,
            art_viewer_rows_source: None,
            cc_gumps_package: None,
            ec_gumps_package: None,
            cc_gumps: None,
            cc_sounds: None,
            cc_sound_entries: None,
            cc_multimap: None,
            cc_multimap_path: None,
            generated_multimap_plain: None,
            generated_multimap_artistic: None,
            sound_player: None,
            selected_uop_idx: None,
            selected_file_hash: None,
            selected_tex_art_cc_id: None,
            selected_terrain_def_hash: None,
            selected_tileart_hash: None,
            selected_ec_hue_hash: None,
            selected_multi_uop_hash: None,
            selected_localized_file_hash: None,
            selected_cliloc_file_idx: None,
            // selected_cc_tile_id: None,
            selected_legacy_source: ArtSource::Any,
            search_query: String::new(),
            find_hash_query: String::new(),
            hide_empty_uop_entries: true,
            guess_terrain_texture_file_format: false,
            status_message: "Welcome to UOCF Inspector".to_string(),
            view_mode: settings.last_view_mode.unwrap_or(ViewMode::Home),
            tile_metadata_source: TileMetadataSource::CcTileData,
            hues_source: HuesSource::CcMul,
            multis_source: MultisSource::ClassicMul,
            localized_strings_source: LocalizedStringsSource::Cliloc,
            multimap_preview_selection: MultimapPreviewSelection::Loaded,
            multimap_converter: MultimapConverterState::default(),
            multimap_worker_rx: None,
            multimap_worker_active: false,
            terrain_def_package: None,
            terrain_def_files: None,
            ec_tileart_entries: None,
            ec_tileart_rows: None,
            ec_hues: None,
            multi_collection: None,
            multi_collection_source: None,
            multi_collection_path: None,
            texture_previews: HashMap::new(),
            ec_texture_previews: HashMap::new(),
            ec_texture_preview_source_keys: HashMap::new(),
            uop_entry_labels: HashMap::new(),
            uop_entry_payloads: HashMap::new(),
            terrain_texture_guess_names: HashMap::new(),
            animationframe_uop_entries: HashMap::new(),
            multimap_texture: None,
            image_preview_sources: HashMap::new(),
            current_image_preview_key: None,
            show_upscale_preview: false,
            upscale_preview_passes: vec![UpscalePreviewPass::default()],
            upscale_preview_zoom: 1.0,
            upscale_original_texture: None,
            upscale_original_texture_key: None,
            upscale_preview_texture: None,
            upscale_preview_texture_key: None,
            upscale_preview_worker_key: None,
            upscale_preview_worker_rx: None,
            upscale_preview_size: [0, 0],
            upscale_preview_elapsed_ms: None,
            upscale_preview_status: String::new(),
            selected_gump_source: GumpSource::Classic,
            selected_gump_viewer_tab: GumpViewerTab::StandardGumps,
            selected_gump_id: String::new(),
            standard_gump_ids: Vec::new(),
            paperdoll_gump_ids: Vec::new(),
            gump_list_source: None,
            gump_list_profile: String::new(),
            gump_list_status: String::new(),
            selected_paperdoll_profile: "human_male".to_string(),
            paperdoll_body_id: String::new(),
            paperdoll_body_hue: "0".to_string(),
            paperdoll_equipment: vec![PaperdollEquipmentInput::default(); 6],
            paperdoll_preview: None,

            selected_anim_id: 0,
            selected_animdata_id: 0,
            selected_animdata_art_source: ArtSource::CcUop,
            animdata_frame_delay_ms: 100.0,
            selected_anim_file_idx: 0,
            current_frame_idx: 0,
            is_playing: false,
            last_frame_time: 0.0,
            playback_speed: 1.0,
            loop_animation: true,
            selected_anim_sequence: None,
            selected_action_id: 0,
            selected_direction: 0,
            selected_multi_id: 0,
            selected_sound_slot: 0,
            selected_sound_id: 0,
            sound_search_query: String::new(),
            selected_hue_id: 0,
            selected_ec_hue_id: 1,
            selected_ec_hueing_mode: EcHueingMode::Cc,
            selected_cliloc_number: 0,
            multimap_zoom: 0.25,
        };

        app.log("UOCF Inspector starting...");
        app.log("Loading previous settings...");
        app.trigger_reload();

        app
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        log::info!("{}", msg);
        self.logs.push(msg);
    }

    pub fn select_cliloc_file(&mut self, index: usize) {
        if let Some(entry) = self.cliloc_files.get(index) {
            self.selected_cliloc_file_idx = Some(index);
            self.cliloc = Some(Arc::clone(&entry.cliloc));
        }
    }

    pub fn trigger_reload(&mut self) {
        self.log("Starting asset reload...");
        self.ec_hues = None;
        self.client_data = None;
        self.cc_tiledata = None;
        self.cc_tiledata_rows = None;
        self.art_viewer_rows = None;
        self.art_viewer_rows_source = None;
        self.ec_tileart_entries = None;
        self.ec_tileart_rows = None;
        self.selected_ec_hue_hash = None;
        self.cliloc = None;
        self.cliloc_files.clear();
        self.selected_cliloc_file_idx = None;
        self.localized_strings = None;
        self.localized_string_rows = None;
        self.multi_collection = None;
        self.multi_collection_source = None;
        self.multi_collection_path = None;
        self.selected_multi_uop_hash = None;
        self.selected_localized_file_hash = None;
        self.cc_sounds = None;
        self.cc_sound_entries = None;
        self.cc_gumps_package = None;
        self.ec_gumps_package = None;
        self.cc_gumps = None;
        self.uop_cache.loaded_uops.clear();
        self.selected_uop_idx = None;
        self.selected_file_hash = None;
        self.cc_multimap = None;
        self.cc_multimap_path = None;
        self.generated_multimap_plain = None;
        self.generated_multimap_artistic = None;
        self.multimap_preview_selection = MultimapPreviewSelection::Loaded;
        self.multimap_worker_rx = None;
        self.multimap_worker_active = false;
        self.texture_previews.clear();
        self.ec_texture_previews.clear();
        self.uop_entry_labels.clear();
        self.uop_entry_payloads.clear();
        self.terrain_texture_guess_names.clear();
        self.animationframe_uop_entries.clear();
        self.multimap_texture = None;
        self.paperdoll_preview = None;
        self.image_preview_sources.clear();
        self.ec_texture_preview_source_keys.clear();
        self.current_image_preview_key = None;
        self.upscale_original_texture = None;
        self.upscale_original_texture_key = None;
        self.upscale_preview_texture = None;
        self.upscale_preview_texture_key = None;
        self.upscale_preview_worker_key = None;
        self.upscale_preview_worker_rx = None;
        self.upscale_preview_size = [0, 0];
        self.upscale_preview_elapsed_ms = None;
        self.upscale_preview_status.clear();
        if let Some(player) = &mut self.sound_player {
            player.stop();
        }

        // 1. Try to load CC assets (mul and uop)
        if let Some(path) = self.settings.cc_path.clone() {
            self.log(format!("Trying to load CC assets from {}", path.display()));
            if let Some(package) = load_optional_gumps_package(&path, "gumps_cc.uddp", |msg| self.log(msg)) {
                self.cc_gumps_package = Some(Arc::new(package));
            }
            let art_res = ArtMap::load(&path);
            let td_res = TileData::load(path.join("tiledata.mul"));
            match GumpMap::load(&path) {
                Ok(gumps) => {
                    self.cc_gumps = Some(Arc::new(gumps));
                    self.log("Loading CC gump art... Success");
                }
                Err(e) => {
                    self.log(format!("Loading CC gump art... Failed: {}", e));
                }
            }
            let loaded_sounds = SoundMap::load(&path).ok().map(Arc::new);
            let loaded_sound_entries = loaded_sounds
                .as_ref()
                .map(|sounds| Arc::new(collect_sound_entries(sounds)));
            self.cc_sounds = loaded_sounds.clone();
            self.cc_sound_entries = loaded_sound_entries.clone();
            let multimap_path = path.join("multimap.rle");
            if multimap_path.exists() {
                match uocf::classic::multimap_rle::load_rle(&multimap_path) {
                    Ok(multimap) => {
                        self.cc_multimap = Some(Arc::new(multimap));
                        self.cc_multimap_path = Some(multimap_path.clone());
                        self.log(format!("Loaded multimap.rle from {}", multimap_path.display()));
                    }
                    Err(e) => {
                        self.log(format!("Failed to load multimap.rle: {}", e));
                    }
                }
            }
            let loaded_tiledata = match td_res {
                Ok(td) => {
                    let td = Arc::new(td);
                    self.cc_tiledata_rows = Some(Arc::new(collect_cc_tiledata_rows(&td)));
                    self.cc_tiledata = Some(Arc::clone(&td));
                    Some(td)
                }
                Err(e) => {
                    self.log(format!("tiledata.mul unavailable: {}", e));
                    self.cc_tiledata = None;
                    self.cc_tiledata_rows = None;
                    None
                }
            };

            match art_res {
                Ok(art) => {
                    let td = loaded_tiledata.unwrap_or_else(|| Arc::new(TileData::new_empty()));
                    let multis = uocf::classic::multi::MultiMap::load(&path)
                        .ok()
                        .map(Arc::new);
                    let cliloc_files = load_cliloc_files(&path, |msg| self.log(msg));
                    self.cliloc_files = cliloc_files;
                    if let Some(index) = default_cliloc_file_index(&self.cliloc_files) {
                        self.select_cliloc_file(index);
                    }
                    let hues = uocf::classic::hues::load_hues(&path.join("hues.mul"))
                        .ok()
                        .map(Arc::new);
                    let animdata = uocf::classic::animdata::AnimData::load(path.join("animdata.mul"))
                        .ok()
                        .map(Arc::new);
                    let anim_map = match uocf::classic::anim::AnimMap::load(&path) {
                        Ok(anim_map) => Some(Arc::new(anim_map)),
                        Err(e) => {
                            self.log(format!("Classic animation MUL sources unavailable: {}", e));
                            None
                        }
                    };
                    let mut anim_defs = None;
                    let anim_def_path = path.join("AnimationDefinition.uop");
                    if anim_def_path.exists() {
                        if let Ok(package) = UopPackage::load(&anim_def_path) {
                            let hash = uocf::uop_container::hash::hash_file_name_single(
                                "data/animationdefinition/animationdefinition.bin",
                            );
                            if let Some(file) = package.get_file_by_hash(hash) {
                                if let Ok(data) = file.unpack() {
                                    if let Ok(defs) =
                                        uocf::classic::anim::AnimationDefinition::parse(&data)
                                    {
                                        anim_defs = Some(Arc::new(defs));
                                    }
                                }
                            }
                        }
                    }

                    self.client_data = Some(ClientData {
                        path: path.clone(),
                        art: Arc::new(art),
                        tiledata: td,
                        multis,
                        _ec_multis: None,
                        hues,
                        animdata,
                        anim_map,
                        anim_defs,
                    });
                    self.log("Loading CC assets... Success");
                }
                Err(e) => {
                    self.log(format!("Loading CC assets... Failed: {}", e));
                }
            }

            for uop_name in ["artlegacymul.uop", "artLegacyMUL.uop"] {
                let uop_path = path.join(uop_name);
                if uop_path.exists() {
                    match UopPackage::load(&uop_path) {
                        Ok(package) => {
                            self.uop_cache.loaded_uops.push(Arc::new(crate::logic::uop_cache::LoadedUop {
                                path: uop_path,
                                package,
                            }));
                            self.log(format!("Loading {} into cache... Success", uop_name));
                        }
                        Err(e) => {
                            self.log(format!("Loading {} into cache... Failed: {}", uop_name, e));
                        }
                    }
                    break;
                }
            }

            for uop_name in ["MultiCollection.uop", "multicollection.uop"] {
                let uop_path = path.join(uop_name);
                if uop_path.exists() {
                    self.log(format!("Parsing {} from {}", uop_name, uop_path.display()));
                    match MultiCollection::load(&uop_path) {
                        Ok(collection) => {
                            let count = collection.items.len();
                            self.multi_collection = Some(Arc::new(collection));
                            self.multi_collection_source = Some(MultiCollectionSource::ClassicClient);
                            self.multi_collection_path = Some(uop_path.clone());
                            self.log(format!("Parsed {} MultiCollection.uop entries.", count));
                        }
                        Err(e) => self.log(format!("Failed to parse {}: {}", uop_name, e)),
                    }
                    match UopPackage::load(&uop_path) {
                        Ok(package) => {
                            self.uop_cache.add(uop_path, package);
                            self.log(format!("Loading {} into cache... Success", uop_name));
                        }
                        Err(e) => self.log(format!("Loading {} into cache... Failed: {}", uop_name, e)),
                    }
                    break;
                }
            }
        }

        // 2. Try to load EC assets (string dictionary and legacy texture)
        if let Some(ec_base_path) = self.settings.ec_path.clone() {
            self.log(format!(
                "Trying to load EC assets from {}",
                ec_base_path.display()
            ));
            if let Some(package) = load_optional_gumps_package(&ec_base_path, "gumps_ec.uddp", |msg| self.log(msg)) {
                self.ec_gumps_package = Some(Arc::new(package));
            }

            // Try load string dictionary
            let sd_path = ec_base_path.join("string_dictionary.uop");
            if sd_path.exists() {
                self.log(format!("Loading string dictionary from {}", sd_path.display()));
                match UoStringDictionary::load(&sd_path) {
                    Ok(dict) => {
                        self.uo_string_dictionary = Some(Arc::new(dict));
                        self.log("Successfully loaded EC string dictionary.");
                        if let Ok(package) = UopPackage::load(&sd_path) {
                            self.string_dictionary_raw_hash = package
                                .get_file_by_hash(0)
                                .map(|file| file.filename_hash())
                                .or_else(|| package.iter_files().find(|file| file.has_size()).map(|file| file.filename_hash()));
                        }
                    }
                    Err(e) => {
                        self.log(format!("Failed to load EC string dictionary: {}", e));
                    }
                }
            }

            // Try load LegacyTexture.uop and attach to CC art if available
            let lt_path = ec_base_path.join("LegacyTexture.uop");
            if lt_path.exists() {
                self.log(format!("Loading LegacyTexture.uop from {}", lt_path.display()));
                match UopPackage::load(&lt_path) {
                    Ok(package) => {
                        if let Some(client) = &mut self.client_data {
                            let mut art = (*client.art).clone();
                            art = art.with_uop(package.clone());
                            client.art = Arc::new(art);
                            self.log("Attached LegacyTexture.uop to CC art.");
                        } else {
                            // If no CC path, load it standalone
                            let art = ArtMap::load_standalone_uop(package.clone());
                            self.client_data = Some(ClientData {
                                path: ec_base_path.clone(),
                                art: Arc::new(art),
                                tiledata: Arc::new(TileData::new_empty()),
                                multis: None,
                                _ec_multis: None,
                                hues: None,
                                animdata: None,
                                anim_map: None,
                                anim_defs: None,
                            });
                            self.log("Loaded standalone Legacy Art UOP from EC folder.");
                        }
                    }
                    Err(e) => {
                        self.log(format!("Failed to load LegacyTexture.uop: {}", e));
                    }
                }
            }

            if let Some(texture_path) =
                find_client_file_case_insensitive(&ec_base_path, "Texture.uop")
            {
                self.log(format!("Loading Texture.uop from {}", texture_path.display()));
                match UopPackage::load(&texture_path) {
                    Ok(package) => {
                        if let Some(client) = &mut self.client_data {
                            let mut art = (*client.art).clone();
                            art = art.with_ec_kr_uop(package.clone());
                            client.art = Arc::new(art);
                            self.log("Attached Texture.uop to CC art as EC UOP KR.");
                        } else {
                            let art = ArtMap::load_standalone_ec_kr_uop(package.clone());
                            self.client_data = Some(ClientData {
                                path: ec_base_path.clone(),
                                art: Arc::new(art),
                                tiledata: Arc::new(TileData::new_empty()),
                                multis: None,
                                _ec_multis: None,
                                hues: None,
                                animdata: None,
                                anim_map: None,
                                anim_defs: None,
                            });
                            self.log("Loaded standalone Texture.uop from EC folder as EC UOP KR.");
                        }
                    }
                    Err(e) => {
                        self.log(format!("Failed to load Texture.uop: {}", e));
                    }
                }
            }

            if let Some(hues_path) =
                find_client_file_case_insensitive(&ec_base_path, "hues.uop")
            {
                self.log(format!("Parsing hues.uop from {}", hues_path.display()));
                match EcHuePackage::load(&hues_path) {
                    Ok(hues) => {
                        let bitmap_count = hues.bitmaps.len();
                        self.ec_hues = Some(Arc::new(hues));
                        self.log(format!("Parsed EC hues.uop with {} hue bitmaps.", bitmap_count));
                    }
                    Err(e) => {
                        self.log(format!("Failed to parse hues.uop: {}", e));
                    }
                }
            }

            for uop_name in ["MultiCollection.uop", "multicollection.uop"] {
                let uop_path = ec_base_path.join(uop_name);
                if uop_path.exists() {
                    self.log(format!("Parsing {} from {}", uop_name, uop_path.display()));
                    match MultiCollection::load(&uop_path) {
                        Ok(collection) => {
                            let count = collection.items.len();
                            self.multi_collection = Some(Arc::new(collection));
                            self.multi_collection_source = Some(MultiCollectionSource::EnhancedClient);
                            self.multi_collection_path = Some(uop_path.clone());
                            self.log(format!("Parsed {} MultiCollection.uop entries.", count));
                        }
                        Err(e) => self.log(format!("Failed to parse {}: {}", uop_name, e)),
                    }
                    break;
                }
            }

            if let Some(uop_path) = find_localized_strings_uop(&ec_base_path) {
                let uop_name = uop_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(LOCALIZED_STRINGS_UOP_NAME);
                self.log(format!("Parsing {} from {}", uop_name, uop_path.display()));
                match LocalizedStringsPackage::load(&uop_path) {
                    Ok(strings) => {
                        let count = strings.len();
                        let rows = collect_localized_string_rows(&strings);
                        self.localized_strings = Some(Arc::new(strings));
                        self.localized_string_rows = Some(Arc::new(rows));
                        self.log(format!("Parsed {} localized string entries.", count));
                    }
                    Err(e) => self.log(format!("Failed to parse {}: {}", uop_name, e)),
                }
            }

            // Load additional EC UOPs into the cache for exploration
            let ec_uops = [
                "string_dictionary.uop",
                "localizedstrings.uop",
                "LocalizedStrings.uop",
                "hues.uop",
                "MultiCollection.uop",
                "multicollection.uop",
                "tileart.uop",
                "terraindefinition.uop",
                "terraintexture.uop",
                "legacytexture.uop",
                "legacyterrain.uop",
                "texture.uop",
                "interface.uop",
                "waypoint.uop",
            ];
            for uop_name in ec_uops {
                if let Some(uop_path) =
                    find_client_file_case_insensitive(&ec_base_path, uop_name)
                {
                    if self
                        .uop_cache
                        .loaded_uops
                        .iter()
                        .any(|loaded| loaded.path.as_path() == uop_path.as_path())
                    {
                        continue;
                    }
                    let load_mode = if uop_name.eq_ignore_ascii_case("interface.uop") {
                        LoadMode::Lazy
                    } else {
                        LoadMode::Eager
                    };
                    match UopPackage::load_with_mode(&uop_path, load_mode) {
                        Ok(package) => {
                            self.uop_cache.loaded_uops.push(Arc::new(crate::logic::uop_cache::LoadedUop {
                                path: uop_path,
                                package,
                            }));
                            self.log(format!("Loading {} into cache... Success", uop_name));
                        }
                        Err(e) => {
                            self.log(format!("Loading {} into cache... Failed: {}", uop_name, e));
                        }
                    }
                }
            }

            let tileart_path = ec_base_path.join("tileart.uop");
            if tileart_path.exists() {
                self.log(format!("Parsing tileart.uop from {}", tileart_path.display()));
                match UopPackage::load(&tileart_path) {
                    Ok(package) => {
                        let mut entries = Vec::new();
                        let mut failed = 0usize;
                        for file in package.iter_files() {
                            match TileArtEntry::parse_raw(&file) {
                                Ok(entry) => entries.push(TileArtFileEntry {
                                    filename_hash: file.filename_hash(),
                                    entry,
                                }),
                                Err(_) => failed += 1,
                            }
                        }
                        entries.sort_by_key(|file| file.entry.tile_id);
                        let rows = collect_tileart_display_rows(
                            &entries,
                            self.uo_string_dictionary.as_deref(),
                        );
                        let count = entries.len();
                        self.ec_tileart_entries = Some(Arc::new(entries));
                        self.ec_tileart_rows = Some(Arc::new(rows));
                        self.log(format!(
                            "Parsed {} tileart.uop entries ({} skipped).",
                            count, failed
                        ));
                    }
                    Err(e) => {
                        self.log(format!("Failed to load tileart.uop: {}", e));
                    }
                }
            }

            // Also load TerrainDefinitionPackage for the ad-hoc viewer
            let td_path = ec_base_path.join("terraindefinition.uop");
            if td_path.exists() {
                self.log(format!(
                    "Parsing TerrainDefinitionPackage from {}",
                    td_path.display()
                ));
                match UopPackage::load(&td_path) {
                    Ok(package) => {
                        let dict_arc = self.uo_string_dictionary.clone();
                        let dict = dict_arc.as_deref();
                        match uocf::enhanced::terrain_definition::TerrainDefinitionPackage::from_package(
                            &package,
                            dict,
                        ) {
                            Ok(pkg) => {
                                self.terrain_def_package = Some(Arc::new(pkg));
                                self.log("Successfully parsed TerrainDefinitionPackage.");
                            }
                            Err(e) => {
                                self.log(format!("Failed to parse TerrainDefinitionPackage: {}", e));
                            }
                        }

                        let mut files = Vec::new();
                        let mut failed = 0usize;
                        for file in package.iter_files() {
                            if !file.has_size() {
                                continue;
                            }

                            let data = match file.unpack() {
                                Ok(data) => data,
                                Err(_) => {
                                    failed += 1;
                                    continue;
                                }
                            };
                            match uocf::enhanced::terrain_definition::parse_entry(&file, dict) {
                                Ok(entry) => {
                                    files.push(TerrainDefinitionFileEntry {
                                        filename_hash: file.filename_hash(),
                                        byte_len: data.len(),
                                        raw_prefix: data[..data.len().min(256)].to_vec(),
                                        entry,
                                    });
                                }
                                Err(_) => failed += 1,
                            }
                        }
                        files.sort_by_key(|file| file.entry.id);
                        let count = files.len();
                        self.terrain_def_files = Some(Arc::new(files));
                        self.log(format!(
                            "Parsed {} TerrainDefinition.uop files ({} skipped).",
                            count, failed
                        ));
                    }
                    Err(e) => {
                        self.log(format!("Failed to load TerrainDefinition.uop: {}", e));
                    }
                }
            }
        }

        // 3. Try to load .dic hash dictionary from settings path
        if let Some(path) = self.settings.dict_path.clone() {
            self.log(format!(
                "Trying to load dictionary from {}",
                path.display()
            ));
            match self.dictionary.load_dic(&path) {
                Ok(_) => {
                    self.log(format!(
                        "Loaded DIC dictionary with {} named entries",
                        self.dictionary.count()
                    ));
                }
                Err(e) => {
                    self.log(format!("Failed to load DIC dictionary: {}", e));
                }
            }
        }

        // 4. Always try to find .dic dicts in current folder as well
        self.load_local_dictionaries();
    }

    fn load_local_dictionaries(&mut self) {
        // Try current dir
        self.scan_dir_for_dicts(".");

        // Try exe dir
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                if dir != std::path::Path::new(".") {
                    self.scan_dir_for_dicts(dir);
                }
            }
        }
    }

    fn scan_dir_for_dicts(&mut self, dir: impl AsRef<std::path::Path>) {
        let dir = dir.as_ref();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("dic") {
                    self.log(format!("Found DIC dictionary candidate: {}", path.display()));
                    match self.dictionary.load_dic(&path) {
                        Ok(_) => {
                            self.log(format!(
                                "Successfully loaded DIC dictionary: {}",
                                path.display()
                            ));
                        }
                        Err(_) => {
                            // Silently ignore if it's not a valid dictionary.
                        }
                    }
                }
            }
        }
    }

    pub fn register_current_image_preview(
        &mut self,
        key: u64,
        label: impl Into<String>,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let expected_len = width as usize * height as usize * 4;
        if rgba.len() != expected_len {
            return;
        }

        if !self.image_preview_sources.contains_key(&key) {
            self.image_preview_sources.insert(
                key,
                InspectorImagePreview {
                    key,
                    label: label.into(),
                    width,
                    height,
                    rgba: Arc::<[u8]>::from(rgba),
                },
            );
        }
        self.select_image_preview(key);
    }

    pub fn select_image_preview(&mut self, key: u64) {
        if !self.image_preview_sources.contains_key(&key) {
            return;
        }
        if self.current_image_preview_key != Some(key) {
            self.current_image_preview_key = Some(key);
            self.upscale_original_texture_key = None;
            self.upscale_preview_texture_key = None;
            self.upscale_preview_worker_key = None;
            self.upscale_preview_worker_rx = None;
            self.upscale_preview_elapsed_ms = None;
            self.upscale_preview_status.clear();
        }
    }

    pub fn current_image_preview(&self) -> Option<&InspectorImagePreview> {
        self.current_image_preview_key
            .and_then(|key| self.image_preview_sources.get(&key))
    }

    pub fn current_upscale_preview_passes(&self) -> Vec<UpscalePreviewPass> {
        self.upscale_preview_passes.clone()
    }

    pub fn clamp_upscale_preview_passes(&mut self) {
        if self.upscale_preview_passes.is_empty() {
            self.upscale_preview_passes
                .push(UpscalePreviewPass::default());
        }
        for pass in &mut self.upscale_preview_passes {
            pass.clamp_scale();
        }
    }

    pub fn refresh_upscale_preview_textures(&mut self, ctx: &egui::Context) {
        self.clamp_upscale_preview_passes();
        self.poll_upscale_preview_worker(ctx);
        let Some(source) = self.current_image_preview().cloned() else {
            self.upscale_original_texture = None;
            self.upscale_original_texture_key = None;
            self.upscale_preview_texture = None;
            self.upscale_preview_texture_key = None;
            self.upscale_preview_worker_key = None;
            self.upscale_preview_worker_rx = None;
            self.upscale_preview_size = [0, 0];
            self.upscale_preview_elapsed_ms = None;
            self.upscale_preview_status.clear();
            return;
        };

        if self.upscale_original_texture_key != Some(source.key) {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [source.width as usize, source.height as usize],
                &source.rgba,
            );
            self.upscale_original_texture = Some(ctx.load_texture(
                format!("uocf_upscale_original_{:016X}", source.key),
                color_image,
                egui::TextureOptions::NEAREST,
            ));
            self.upscale_original_texture_key = Some(source.key);
        }

        let passes = self.current_upscale_preview_passes();
        let texture_key = (source.key, passes.clone());
        if self.upscale_preview_texture_key == Some(texture_key.clone())
            || self.upscale_preview_worker_key == Some(texture_key.clone())
        {
            return;
        }

        self.upscale_preview_texture = None;
        self.upscale_preview_texture_key = None;
        self.upscale_preview_size = [0, 0];
        self.upscale_preview_elapsed_ms = None;
        self.upscale_preview_status = "Computing preview...".to_string();

        let source_key = source.key;
        let source_width = source.width;
        let source_height = source.height;
        let source_rgba = Arc::clone(&source.rgba);
        let worker_passes = passes.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let started = Instant::now();
            let (width, height, rgba, palette_status) = apply_upscale_preview_passes(
                source_width,
                source_height,
                source_rgba.to_vec(),
                &worker_passes,
            );
            let _ = tx.send(UpscalePreviewResult {
                source_key,
                passes: worker_passes,
                width,
                height,
                rgba,
                elapsed_ms: started.elapsed().as_millis(),
                palette_status,
            });
        });

        self.upscale_preview_worker_key = Some(texture_key);
        self.upscale_preview_worker_rx = Some(rx);
        ctx.request_repaint();
    }

    fn poll_upscale_preview_worker(&mut self, ctx: &egui::Context) {
        let result = match self.upscale_preview_worker_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Empty)) | None => return,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.upscale_preview_worker_rx = None;
                self.upscale_preview_worker_key = None;
                self.upscale_preview_status = "Preview worker stopped.".to_string();
                return;
            }
        };

        let Some(result) = result else {
            return;
        };
        let texture_key = (result.source_key, result.passes.clone());
        self.upscale_preview_worker_rx = None;
        self.upscale_preview_worker_key = None;
        if self.current_image_preview_key == Some(result.source_key) {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [result.width as usize, result.height as usize],
                &result.rgba,
            );
            self.upscale_preview_texture = Some(ctx.load_texture(
                format!("uocf_upscale_preview_{:016X}_{:?}", result.source_key, result.passes),
                color_image,
                egui::TextureOptions::NEAREST,
            ));
            self.upscale_preview_texture_key = Some(texture_key);
            self.upscale_preview_size = [result.width, result.height];
            self.upscale_preview_elapsed_ms = Some(result.elapsed_ms);
            self.upscale_preview_status = result.palette_status;
            ctx.request_repaint();
        }
    }

    pub fn open_uop(&mut self) {
        if let Some(path) = crate::dialog::file_dialog()
            .add_filter("UOP Packages", &["uop"])
            .pick_file()
        {
            match UopPackage::load_with_mode(&path, LoadMode::Eager) {
                Ok(package) => {
                    self.uop_cache.add(path.clone(), package);
                    self.selected_uop_idx = Some(self.uop_cache.loaded_uops.len() - 1);
                    self.status_message = format!("Loaded {}", path.display());
                    self.view_mode = ViewMode::UopExplorer;
                }
                Err(e) => {
                    self.status_message = format!("Failed to load UOP: {}", e);
                }
            }
        }
    }

    pub fn get_uop_texture(
        &mut self,
        ctx: &egui::Context,
        hash: u64,
        data: &[u8],
        name: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.texture_previews.get(&hash).cloned() {
            self.select_image_preview(hash);
            return Some(handle);
        }

        let lower_name = name.to_lowercase();
        if lower_name.ends_with(".bmp") {
            let (width, height, pixels) = uocf::enhanced::hues::decode_hue_image_to_rgba(data).ok()?;
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &pixels,
            );
            let handle = ctx.load_texture(name, color_image, Default::default());
            self.texture_previews.insert(hash, handle.clone());
            self.register_current_image_preview(hash, name, width, height, &pixels);
            return Some(handle);
        }

        let format = if lower_name.ends_with(".dds") {
            ECImageFormat::DDS
        } else if lower_name.ends_with(".tga") {
            ECImageFormat::TGA
        } else if let Some((_, format)) = guess_uop_image_format_from_payload(data) {
            format
        } else {
            ECImageFormat::Unknown
        };

        let tex_file = TextureFile {
            metadata: RawTextureItem::absent(),
            is_ec: true,
            format,
            props: None,
            raw_data: data.into(),
            image_data_offset: 0,
        };

        match tex_file.decode_to_rgba() {
            Ok(img) => {
                let size = [img.width() as usize, img.height() as usize];
                let pixels = img.to_rgba8();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
                let handle = ctx.load_texture(name, color_image, Default::default());
                self.texture_previews.insert(hash, handle.clone());
                self.register_current_image_preview(
                    hash,
                    name,
                    size[0] as u32,
                    size[1] as u32,
                    pixels.as_raw(),
                );
                Some(handle)
            }
            Err(_) => None,
        }
    }

    pub fn selected_uop_is_terrain_texture(&self, uop_idx: usize) -> bool {
        self.uop_cache
            .loaded_uops
            .get(uop_idx)
            .map(|loaded| is_terrain_texture_uop_path(&loaded.path))
            .unwrap_or(false)
    }

    pub fn set_guess_terrain_texture_file_format(&mut self, enabled: bool) {
        if self.guess_terrain_texture_file_format != enabled {
            self.guess_terrain_texture_file_format = enabled;
            self.uop_entry_labels.clear();
            self.terrain_texture_guess_names.clear();
        }
    }

    pub fn set_hide_empty_uop_entries(&mut self, enabled: bool) {
        if self.hide_empty_uop_entries != enabled {
            self.hide_empty_uop_entries = enabled;
            self.uop_entry_labels.clear();
            if enabled {
                self.selected_file_hash = None;
            }
        }
    }

    fn get_terrain_texture_guess_names(&mut self, uop_idx: usize) -> Option<Arc<HashMap<u64, String>>> {
        if !self.guess_terrain_texture_file_format || !self.selected_uop_is_terrain_texture(uop_idx) {
            return None;
        }

        if let Some(names) = self.terrain_texture_guess_names.get(&uop_idx).cloned() {
            return Some(names);
        }

        let loaded = self.uop_cache.loaded_uops.get(uop_idx)?.clone();
        let names = Arc::new(collect_terrain_texture_guess_names(&loaded.package));
        self.terrain_texture_guess_names.insert(uop_idx, Arc::clone(&names));
        Some(names)
    }

    pub fn resolve_uop_entry_display_name(&mut self, uop_idx: usize, hash: u64) -> String {
        if let Some(name) = self.dictionary.resolve(hash) {
            return name.to_string();
        }

        if let Some(names) = self.get_terrain_texture_guess_names(uop_idx) {
            if let Some(name) = names.get(&hash) {
                return name.clone();
            }
        }

        format!("{:016X}", hash)
    }

    pub fn get_uop_entry_labels(&mut self, uop_idx: usize) -> Arc<Vec<UopEntryLabel>> {
        if let Some(labels) = self.uop_entry_labels.get(&uop_idx).cloned() {
            return labels;
        }

        let Some(loaded) = self.uop_cache.loaded_uops.get(uop_idx).cloned() else {
            return Arc::new(Vec::new());
        };
        let guessed_names = self.get_terrain_texture_guess_names(uop_idx);

        let labels = loaded
            .package
            .iter_files()
            .filter(|file| should_show_uop_entry(file, self.hide_empty_uop_entries))
            .map(|file| {
                let hash = file.filename_hash();
                let display_name = self
                    .dictionary
                    .resolve(hash)
                    .map(str::to_string)
                    .or_else(|| guessed_names.as_ref().and_then(|names| names.get(&hash).cloned()))
                    .unwrap_or_else(|| format!("{:016X}", hash));
                let search_name = display_name.to_lowercase();
                UopEntryLabel {
                    hash,
                    display_name,
                    search_name,
                }
            })
            .collect::<Vec<_>>();

        let labels = Arc::new(labels);
        self.uop_entry_labels.insert(uop_idx, Arc::clone(&labels));
        labels
    }

    pub fn get_uop_entry_payload(&mut self, uop_idx: usize, hash: u64) -> Option<Arc<[u8]>> {
        let key = (uop_idx, hash);
        if let Some(payload) = self.uop_entry_payloads.get(&key).cloned() {
            return Some(payload);
        }

        let loaded = self.uop_cache.loaded_uops.get(uop_idx)?.clone();
        let payload = loaded.package.unpack_file_arc_by_hash(hash).ok()??;
        self.uop_entry_payloads.insert(key, Arc::clone(&payload));
        Some(payload)
    }

    pub fn save_entry(&mut self, hash: u64, name: &str) {
        if let Some(uop_idx) = self.selected_uop_idx {
            let loaded = &self.uop_cache.loaded_uops[uop_idx];
            if loaded.package.get_file_by_hash(hash).is_some() {
                if let Some(path) = crate::dialog::file_dialog()
                    .set_file_name(name)
                    .set_title("Extract Entry")
                    .save_file()
                {
                    match loaded.package.unpack_file_by_hash(hash) {
                        Ok(Some(data)) => {
                            if let Err(e) = std::fs::write(&path, data) {
                                self.status_message = format!("Failed to save: {}", e);
                            } else {
                                self.status_message = format!("Extracted to: {}", path.display());
                            }
                        }
                        Ok(None) => {
                            self.status_message = format!("Entry {:016X} is not in package", hash);
                        }
                        Err(e) => {
                            self.status_message = format!("Failed to unpack: {}", e);
                        }
                    }
                }
            }
        }
    }

    pub fn get_ec_texture_by_id(
        &mut self,
        ctx: &egui::Context,
        texture_id: u32,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.ec_texture_previews.get(&texture_id).cloned() {
            if let Some(key) = self.ec_texture_preview_source_keys.get(&texture_id).copied() {
                self.select_image_preview(key);
            }
            return Some(handle);
        }

        let loaded_uops = self.uop_cache.loaded_uops.clone();
        for loaded in &loaded_uops {
            let candidates = [
                format!("build/worldart/{:08}.dds", texture_id),
                format!("build/worldart/{:08}.tga", texture_id),
                format!("build/tileartlegacy/{:08}.dds", texture_id),
                format!("build/tileartlegacy/{:08}.tga", texture_id),
                format!("build/tileartenhanced/{:08}.dds", texture_id),
                format!("build/tileartenhanced/{:08}.tga", texture_id),
            ];

            for path in candidates {
                let hash = uocf::uop_container::hash::hash_file_name_single(&path);
                if loaded.package.get_file_by_hash(hash).is_some() {
                    if let Ok(Some(data)) = loaded.package.unpack_file_by_hash(hash) {
                        if let Some(handle) = self.get_uop_texture(ctx, hash, &data, &path) {
                            self.ec_texture_previews.insert(texture_id, handle.clone());
                            self.ec_texture_preview_source_keys.insert(texture_id, hash);
                            return Some(handle);
                        }
                    }
                }
            }
        }
        None
    }

    pub fn get_cc_gump_texture(
        &mut self,
        ctx: &egui::Context,
        gump_id: u32,
    ) -> Option<egui::TextureHandle> {
        let key = 0x6D00000000000000 | gump_id as u64;
        if let Some(handle) = self.texture_previews.get(&key).cloned() {
            self.select_image_preview(key);
            return Some(handle);
        }

        let (width, height, pixels) = if let Some(package) = &self.cc_gumps_package {
            match package.read_gump_rgba(gump_id) {
                Ok(gump) => gump,
                Err(_) => {
                    let gumps = Arc::clone(self.cc_gumps.as_ref()?);
                    let mut scratch = Vec::new();
                    let (width, height, pixels) = gumps.decode_gump(gump_id, &mut scratch).ok()?;
                    (u32::from(width), u32::from(height), pixels)
                }
            }
        } else {
            let gumps = Arc::clone(self.cc_gumps.as_ref()?);
            let mut scratch = Vec::new();
            let (width, height, pixels) = gumps.decode_gump(gump_id, &mut scratch).ok()?;
            (u32::from(width), u32::from(height), pixels)
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &pixels,
        );
        let handle = ctx.load_texture(format!("cc_gump_{gump_id}"), image, Default::default());
        self.texture_previews.insert(key, handle.clone());
        self.register_current_image_preview(
            key,
            format!("CC gump {gump_id}"),
            width,
            height,
            &pixels,
        );
        Some(handle)
    }

    pub fn get_ec_gump_texture(
        &mut self,
        ctx: &egui::Context,
        gump_id: u32,
    ) -> Option<egui::TextureHandle> {
        if let Some(package) = &self.ec_gumps_package {
            if let Ok((width, height, pixels)) = package.read_gump_rgba(gump_id) {
                let key = 0x6E00000000000000 | gump_id as u64;
                if let Some(handle) = self.texture_previews.get(&key).cloned() {
                    self.select_image_preview(key);
                    return Some(handle);
                }
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                );
                let handle = ctx.load_texture(format!("ec_gump_{gump_id}"), image, Default::default());
                self.texture_previews.insert(key, handle.clone());
                self.register_current_image_preview(
                    key,
                    format!("EC gump {gump_id}"),
                    width,
                    height,
                    &pixels,
                );
                return Some(handle);
            }
        }

        let loaded_uops = self.uop_cache.loaded_uops.clone();
        for loaded in &loaded_uops {
            let is_interface = loaded
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("interface.uop"))
                .unwrap_or(false);
            if !is_interface {
                continue;
            }

            let candidates = [
                format!("data/interface/default/textures/gumpart/{:08}.tga", gump_id),
                format!("data/interface/default/textures/gumpart/{:08}.dds", gump_id),
            ];
            for path in candidates {
                let hash = hash_file_name_single(&path);
                if let Some(file) = loaded.package.get_file_by_hash(hash) {
                    if let Ok(data) = file.unpack() {
                        if let Some(handle) = self.get_uop_texture(ctx, hash, &data, &path) {
                            return Some(handle);
                        }
                    }
                }
            }
        }

        None
    }

    pub fn get_gump_texture(
        &mut self,
        ctx: &egui::Context,
        source: GumpSource,
        gump_id: u32,
    ) -> Option<egui::TextureHandle> {
        match source {
            GumpSource::Classic => self.get_cc_gump_texture(ctx, gump_id),
            GumpSource::Enhanced => self.get_ec_gump_texture(ctx, gump_id),
        }
    }

    fn get_uop_art_image_texture(
        &mut self,
        ctx: &egui::Context,
        key: u64,
        art_id: u32,
        source: ArtSource,
        scratch: &[u8],
    ) -> Option<egui::TextureHandle> {
        let format = if scratch.starts_with(b"DDS ") {
            ECImageFormat::DDS
        } else {
            ECImageFormat::TGA
        };
        let tex_file = TextureFile {
            metadata: RawTextureItem::absent(),
            is_ec: source.is_ec_uop(),
            format,
            props: None,
            raw_data: Arc::from(scratch),
            image_data_offset: 0,
        };
        let img = tex_file.decode_to_rgba().ok()?;
        let mut rgba = img.to_rgba8();
        if cc_hue_should_apply_to_art(art_id, source) && self.selected_hue_id > 0 {
            if let Some(hue) = self
                .client_data
                .as_ref()
                .and_then(|client| client.hues.as_ref())
                .and_then(|hues| hues.get((self.selected_hue_id as usize).saturating_sub(1)))
            {
                for pixel in rgba.as_mut().chunks_exact_mut(4) {
                    let r = pixel[0] as u32;
                    let g = pixel[1] as u32;
                    let b = pixel[2] as u32;
                    let a = pixel[3] as u32;
                    let color = (a << 24) | (r << 16) | (g << 8) | b;
                    let hued = hue.apply_to_color32(color, false);

                    pixel[0] = ((hued >> 16) & 0xFF) as u8;
                    pixel[1] = ((hued >> 8) & 0xFF) as u8;
                    pixel[2] = (hued & 0xFF) as u8;
                    pixel[3] = ((hued >> 24) & 0xFF) as u8;
                }
            }
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        let handle = ctx.load_texture(
            format!("uop_art_image_{}_{:?}_h{}", art_id, source, self.selected_hue_id),
            image,
            Default::default(),
        );
        self.texture_previews.insert(key, handle.clone());
        self.register_current_image_preview(
            key,
            format!("art {} {:?} hue {}", art_id, source, self.selected_hue_id),
            rgba.width(),
            rgba.height(),
            rgba.as_raw(),
        );
        Some(handle)
    }

    fn get_tex_art_cc_texture_from_source(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
        source: ArtSource,
    ) -> Option<egui::TextureHandle> {
        let hue_id = self.selected_hue_id;
        let key = 0xCC000000 | (source as u64) << 48 | (hue_id as u64) << 32 | art_id as u64;
        if let Some(handle) = self.texture_previews.get(&key).cloned() {
            self.select_image_preview(key);
            return Some(handle);
        }

        if let Some(client) = &self.client_data {
            let art = Arc::clone(&client.art);
            let hues = client.hues.clone();
            let mut scratch = Vec::new();
            if art_id < 0x4000 {
                if art
                    .get_raw_art_data_from_source(art_id, source, &mut scratch)
                    .is_ok()
                {
                    if scratch.starts_with(b"DDS ") {
                        return self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch);
                    }
                    if source != ArtSource::Mul {
                        if let Some(handle) =
                            self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch)
                        {
                            return Some(handle);
                        }
                    }

                    let mut pixels = [0u8; 44 * 44 * 4];
                    if uocf::classic::art::decode_land_tile_from_raw(&scratch, &mut pixels)
                        .is_ok()
                    {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied([44, 44], &pixels[..]);
                        let handle = ctx.load_texture(
                            format!("cc_land_{}_{:?}_h{}", art_id, source, hue_id),
                            image,
                            Default::default(),
                        );
                        self.texture_previews.insert(key, handle.clone());
                        self.register_current_image_preview(
                            key,
                            format!("land art {} {:?} hue {}", art_id, source, hue_id),
                            44,
                            44,
                            &pixels[..],
                        );
                        return Some(handle);
                    }
                }
            } else {
                if art
                    .get_raw_art_data_from_source(art_id, source, &mut scratch)
                    .is_ok()
                    && scratch.starts_with(b"DDS ")
                {
                    return self.get_uop_art_image_texture(ctx, key, art_id, source, &scratch);
                }

                if let Ok((w, h, mut pixels)) =
                    art.decode_static_tile_from_source(art_id, source, &mut scratch)
                {
                    // Apply hue if selected
                    if hue_id > 0 {
                        if let Some(hues) = &hues {
                            if let Some(hue) = hues.get((hue_id as usize).saturating_sub(1)) {
                                for i in (0..pixels.len()).step_by(4) {
                                    let r = pixels[i] as u32;
                                    let g = pixels[i + 1] as u32;
                                    let b = pixels[i + 2] as u32;
                                    let a = pixels[i + 3] as u32;
                                    let color = (a << 24) | (r << 16) | (g << 8) | b;

                                    let hued = hue.apply_to_color32(color, false);

                                    pixels[i] = ((hued >> 16) & 0xFF) as u8;
                                    pixels[i + 1] = ((hued >> 8) & 0xFF) as u8;
                                    pixels[i + 2] = (hued & 0xFF) as u8;
                                    pixels[i + 3] = ((hued >> 24) & 0xFF) as u8;
                                }
                            }
                        }
                    }

                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        &pixels[..],
                    );
                    let handle = ctx.load_texture(
                        format!("cc_static_{}_{:?}_h{}", art_id, source, hue_id),
                        image,
                        Default::default(),
                    );
                    self.texture_previews.insert(key, handle.clone());
                    self.register_current_image_preview(
                        key,
                        format!("static art {} {:?} hue {}", art_id, source, hue_id),
                        w as u32,
                        h as u32,
                        &pixels[..],
                    );
                    return Some(handle);
                }
            }
        }
        None
    }

    pub fn get_tex_art_cc_texture(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
    ) -> Option<egui::TextureHandle> {
        self.get_tex_art_cc_texture_from_source(ctx, art_id, self.selected_legacy_source)
    }

    pub fn get_tex_art_texture_from_source(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
        source: ArtSource,
    ) -> Option<egui::TextureHandle> {
        self.get_tex_art_cc_texture_from_source(ctx, art_id, source)
    }

    pub fn get_tex_art_texture_with_ec_hue_from_source(
        &mut self,
        ctx: &egui::Context,
        art_id: u32,
        source: ArtSource,
        hue_id: u16,
    ) -> Option<egui::TextureHandle> {
        let hueing_mode = self.selected_ec_hueing_mode;
        let key = 0xEC00_0000_0000_0000u64
            | ((hueing_mode as u64) << 56)
            | ((source as u64) << 48)
            | ((hue_id as u64) << 32)
            | art_id as u64;
        if let Some(handle) = self.texture_previews.get(&key).cloned() {
            self.select_image_preview(key);
            return Some(handle);
        }

        let hue_table = self.ec_hue_lookup_table(hue_id, hueing_mode)?;
        let (width, height, mut pixels) = self.decode_art_item_rgba_from_source(art_id, source)?;
        apply_ec_hue_table_to_rgba(&mut pixels, &hue_table, hueing_mode);

        let image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &pixels,
        );
        let handle = ctx.load_texture(
            format!("ec_hued_art_{}_{:?}_h{}", art_id, source, hue_id),
            image,
            egui::TextureOptions::NEAREST,
        );
        self.texture_previews.insert(key, handle.clone());
        self.register_current_image_preview(
            key,
            format!("art {} {:?} EC hue {} {:?}", art_id, source, hue_id, hueing_mode),
            width,
            height,
            &pixels,
        );
        Some(handle)
    }

    fn decode_art_item_rgba_from_source(
        &self,
        art_id: u32,
        source: ArtSource,
    ) -> Option<(u32, u32, Vec<u8>)> {
        if art_id < 0x4000 {
            return None;
        }

        let client = self.client_data.as_ref()?;
        let art = Arc::clone(&client.art);
        let mut scratch = Vec::new();
        if art
            .get_raw_art_data_from_source(art_id, source, &mut scratch)
            .is_ok()
        {
            let format = if scratch.starts_with(b"DDS ") {
                ECImageFormat::DDS
            } else {
                ECImageFormat::TGA
            };
            if source != ArtSource::Mul || scratch.starts_with(b"DDS ") {
                let tex_file = TextureFile {
                    metadata: RawTextureItem::absent(),
                    is_ec: source.is_ec_uop(),
                    format,
                    props: None,
                    raw_data: Arc::from(scratch.as_slice()),
                    image_data_offset: 0,
                };
                if let Ok(img) = tex_file.decode_to_rgba() {
                    let rgba = img.to_rgba8();
                    return Some((rgba.width(), rgba.height(), rgba.into_raw()));
                }
            }
        }

        scratch.clear();
        art.decode_static_tile_from_source(art_id, source, &mut scratch)
            .ok()
            .map(|(width, height, pixels)| (width as u32, height as u32, pixels))
    }

    fn ec_hue_lookup_table(&self, hue_id: u16, hueing_mode: EcHueingMode) -> Option<Vec<u8>> {
        let hash = uocf::enhanced::hues::hue_bitmap_hash(hue_id);
        let loaded_uops = self.uop_cache.loaded_uops.clone();
        for loaded in &loaded_uops {
            let is_hues = loaded
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(uocf::enhanced::hues::HUES_UOP_NAME))
                .unwrap_or(false);
            if !is_hues {
                continue;
            }
            let Ok(Some(data)) = loaded.package.unpack_file_by_hash(hash) else {
                continue;
            };
            let Ok((width, height, pixels)) = uocf::enhanced::hues::decode_hue_image_to_rgba(&data) else {
                continue;
            };
            return match hueing_mode {
                EcHueingMode::Cc => build_ec_hue_lookup_table(width, height, &pixels),
                EcHueingMode::Ec => build_ec_hue_lookup_strip(width, height, &pixels),
            };
        }
        None
    }

    pub fn get_multimap_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let key = 0x6F00000000000000;
        if let Some(handle) = self.multimap_texture.clone() {
            self.select_image_preview(key);
            return Some(handle);
        }

        let multimap = Arc::clone(self.cc_multimap.as_ref()?);
        let rgba = multimap.to_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [multimap.width as usize, multimap.height as usize],
            &rgba,
        );
        let handle = ctx.load_texture("cc_multimap_rle", image, Default::default());
        self.multimap_texture = Some(handle.clone());
        self.register_current_image_preview(
            key,
            "multimap.rle",
            multimap.width,
            multimap.height,
            &rgba,
        );
        Some(handle)
    }

    pub fn get_generated_multimap_texture(
        &mut self,
        ctx: &egui::Context,
        selection: MultimapPreviewSelection,
    ) -> Option<egui::TextureHandle> {
        let (key, texture_name, preview) = match selection {
            MultimapPreviewSelection::Plain => (
                0x6F00000000000001,
                "generated_multimap_plain",
                self.generated_multimap_plain.as_ref()?,
            ),
            MultimapPreviewSelection::Artistic => (
                0x6F00000000000002,
                "generated_multimap_artistic",
                self.generated_multimap_artistic.as_ref()?,
            ),
            MultimapPreviewSelection::Loaded => return self.get_multimap_texture(ctx),
        };
        if let Some(handle) = self.texture_previews.get(&key).cloned() {
            self.select_image_preview(key);
            return Some(handle);
        }

        let rgba = preview.image.to_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [preview.image.width as usize, preview.image.height as usize],
            &rgba,
        );
        let handle = ctx.load_texture(texture_name, image, Default::default());
        self.texture_previews.insert(key, handle.clone());
        self.register_current_image_preview(
            key,
            preview.label.clone(),
            preview.image.width,
            preview.image.height,
            &rgba,
        );
        Some(handle)
    }

    pub fn select_raw_uop_entry(&mut self, package_name: &str, file_hash: u64) -> bool {
        let package_name = package_name.to_ascii_lowercase();
        let Some(index) = self.uop_cache.loaded_uops.iter().position(|loaded| {
            loaded
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(&package_name))
                .unwrap_or(false)
        }) else {
            return false;
        };

        self.selected_uop_idx = Some(index);
        self.selected_file_hash = Some(file_hash);
        self.find_hash_query = format!("{:016X}", file_hash);
        self.view_mode = ViewMode::UopExplorer;
        true
    }

    pub fn select_raw_uop_entry_at_path(&mut self, package_path: &Path, file_hash: u64) -> bool {
        let Some(index) = self.uop_cache.loaded_uops.iter().position(|loaded| {
            loaded.path.as_path() == package_path
        }) else {
            return false;
        };

        self.selected_uop_idx = Some(index);
        self.selected_file_hash = Some(file_hash);
        self.find_hash_query = format!("{:016X}", file_hash);
        self.view_mode = ViewMode::UopExplorer;
        true
    }

    pub fn select_raw_ec_hue_bitmap(&mut self, hue_id: u16) -> bool {
        let hash = uocf::enhanced::hues::hue_bitmap_hash(hue_id);
        if self.select_raw_uop_entry(uocf::enhanced::hues::HUES_UOP_NAME, hash) {
            self.selected_ec_hue_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_ec_hues_atlas(&mut self) -> bool {
        let hash = uocf::enhanced::hues::hues_atlas_hash();
        if self.select_raw_uop_entry(uocf::enhanced::hues::HUES_UOP_NAME, hash) {
            self.selected_ec_hue_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_ec_huenames(&mut self) -> bool {
        let hash = uocf::enhanced::hues::huenames_hash();
        if self.select_raw_uop_entry(uocf::enhanced::hues::HUES_UOP_NAME, hash) {
            self.selected_ec_hue_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_ec_hue_palette(&mut self) -> bool {
        let hash = uocf::enhanced::hues::FIXED_PALETTE_HASH;
        if self.select_raw_uop_entry(uocf::enhanced::hues::HUES_UOP_NAME, hash) {
            self.selected_ec_hue_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_multi_collection_entry(&mut self, multi_id: u32) -> bool {
        let hash = uocf::enhanced::multis::multi_collection_hash(multi_id);
        if let Some(path) = self.multi_collection_path.clone() {
            if self.select_raw_uop_entry_at_path(&path, hash) {
                self.selected_multi_uop_hash = Some(hash);
                return true;
            }
        }

        if self.select_raw_uop_entry(uocf::enhanced::multis::MULTI_COLLECTION_UOP_NAME, hash)
            || self.select_raw_uop_entry("multicollection.uop", hash)
        {
            self.selected_multi_uop_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_multi_collection_housing(&mut self) -> bool {
        let hash = uocf::enhanced::multis::housing_hash();
        if let Some(path) = self.multi_collection_path.clone() {
            if self.select_raw_uop_entry_at_path(&path, hash) {
                self.selected_multi_uop_hash = Some(hash);
                return true;
            }
        }

        if self.select_raw_uop_entry(uocf::enhanced::multis::MULTI_COLLECTION_UOP_NAME, hash)
            || self.select_raw_uop_entry("multicollection.uop", hash)
        {
            self.selected_multi_uop_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn select_raw_localized_strings_file(&mut self, hash: u64) -> bool {
        if self.select_raw_uop_entry(
            uocf::enhanced::localized_strings::LOCALIZED_STRINGS_UOP_NAME,
            hash,
        ) || self.select_raw_uop_entry("LocalizedStrings.uop", hash)
        {
            self.selected_localized_file_hash = Some(hash);
            return true;
        }
        false
    }

    pub fn get_hues_uop_texture(
        &mut self,
        ctx: &egui::Context,
        hash: u64,
        name: &str,
    ) -> Option<egui::TextureHandle> {
        let preview_key = 0x4855_4553_0000_0000u64 ^ hash;
        if let Some(handle) = self.texture_previews.get(&preview_key).cloned() {
            self.select_image_preview(preview_key);
            return Some(handle);
        }

        let loaded_uops = self.uop_cache.loaded_uops.clone();
        for loaded in &loaded_uops {
            let is_hues = loaded
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case(uocf::enhanced::hues::HUES_UOP_NAME))
                .unwrap_or(false);
            if !is_hues {
                continue;
            }
            if loaded.package.get_file_by_hash(hash).is_some() {
                if let Ok(Some(data)) = loaded.package.unpack_file_by_hash(hash) {
                    let lower_name = name.to_lowercase();
                    if lower_name.ends_with(".bmp") {
                        let (width, height, pixels) =
                            uocf::enhanced::hues::decode_hue_image_to_rgba(&data).ok()?;
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &pixels,
                        );
                        let handle = ctx.load_texture(name, color_image, egui::TextureOptions::NEAREST);
                        self.texture_previews.insert(preview_key, handle.clone());
                        self.register_current_image_preview(preview_key, name, width, height, &pixels);
                        return Some(handle);
                    }

                    let format = if lower_name.ends_with(".dds") {
                        ECImageFormat::DDS
                    } else if lower_name.ends_with(".tga") {
                        ECImageFormat::TGA
                    } else {
                        ECImageFormat::Unknown
                    };

                    let tex_file = TextureFile {
                        metadata: RawTextureItem::absent(),
                        is_ec: true,
                        format,
                        props: None,
                        raw_data: data.into(),
                        image_data_offset: 0,
                    };

                    if let Ok(img) = tex_file.decode_to_rgba() {
                        let size = [img.width() as usize, img.height() as usize];
                        let pixels = img.to_rgba8();
                        let color_image =
                            egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
                        let handle =
                            ctx.load_texture(name, color_image, egui::TextureOptions::NEAREST);
                        self.texture_previews.insert(preview_key, handle.clone());
                        self.register_current_image_preview(
                            preview_key,
                            name,
                            size[0] as u32,
                            size[1] as u32,
                            pixels.as_raw(),
                        );
                        return Some(handle);
                    }
                }
            }
        }
        None
    }

    pub fn select_raw_art_entry(&mut self, art_id: u32, source: ArtSource) -> bool {
        let candidates: &[(&str, &str)] = match source {
            ArtSource::CcUop => &[("artlegacymul.uop", "build/artlegacymul/{id:08}.tga")],
            ArtSource::EcUop | ArtSource::EcUopLegacy => &[
                ("legacytexture.uop", "build/tileartlegacy/{id:08}.dds"),
                ("legacytexture.uop", "build/tileartlegacy/{id:08}.tga"),
                ("legacytexture.uop", "build/legacytexture/{id:08}.tga"),
            ],
            ArtSource::EcUopKr => &[
                ("texture.uop", "build/worldart/{id:08}.dds"),
                ("texture.uop", "build/worldart/{id:08}.tga"),
                ("texture.uop", "build/tileartenhanced/{id:08}.dds"),
                ("texture.uop", "build/tileartenhanced/{id:08}.tga"),
            ],
            ArtSource::Mul | ArtSource::Any => return false,
        };

        for (package_name, template) in candidates {
            let package_art_id = match source {
                ArtSource::EcUop => match uocf::classic::art::ec_legacy_texture_id_from_art_id(art_id) {
                    Some(id) => id,
                    None => continue,
                },
                ArtSource::EcUopLegacy => {
                    uocf::classic::art::ec_legacy_texture_id_from_art_id(art_id).unwrap_or(art_id)
                }
                _ => art_id,
            };
            let path = template.replace("{id:08}", &format!("{:08}", package_art_id));
            let hash = hash_file_name_single(&path);
            if self.select_raw_uop_entry(package_name, hash) {
                return true;
            }
        }

        false
    }
}

fn build_ec_hue_lookup_table(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || pixels.len() != width as usize * height as usize * 4 {
        return None;
    }

    let mut table = vec![0u8; 32 * 4];
    let sample_horizontally = width >= height;
    for color_index in 0..32usize {
        let (src_x, src_y) = if sample_horizontally {
            let x = if width <= 1 {
                0
            } else {
                color_index as u32 * (width - 1) / 31
            };
            (x, height / 2)
        } else {
            let y = if height <= 1 {
                0
            } else {
                color_index as u32 * (height - 1) / 31
            };
            (width / 2, y)
        };
        let src = ((src_y * width + src_x) as usize) * 4;
        table[color_index * 4..color_index * 4 + 4].copy_from_slice(&pixels[src..src + 4]);
    }
    Some(table)
}

fn build_ec_hue_lookup_strip(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || pixels.len() != width as usize * height as usize * 4 {
        return None;
    }

    let sample_horizontally = width >= height;
    let color_count = if sample_horizontally { width } else { height } as usize;
    let mut table = vec![0u8; color_count * 4];
    for color_index in 0..color_count {
        let (src_x, src_y) = if sample_horizontally {
            (color_index as u32, height / 2)
        } else {
            (width / 2, color_index as u32)
        };
        let src = ((src_y * width + src_x) as usize) * 4;
        table[color_index * 4..color_index * 4 + 4].copy_from_slice(&pixels[src..src + 4]);
    }
    Some(table)
}

fn apply_ec_hue_table_to_rgba(
    pixels: &mut [u8],
    hue_table: &[u8],
    hueing_mode: EcHueingMode,
) {
    if hue_table.len() < 4 || hue_table.len() % 4 != 0 {
        return;
    }

    let color_count = hue_table.len() / 4;
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            continue;
        }

        let source_luma = ec_hue_source_luma(pixel, hueing_mode);
        let color_index = ec_hue_color_index(pixel, hueing_mode, color_count);
        let src = color_index * 4;
        apply_ec_hue_color_to_pixel(pixel, &hue_table[src..src + 4], source_luma);
        pixel[3] = alpha;
    }
}

fn ec_hue_color_index(pixel: &[u8], hueing_mode: EcHueingMode, color_count: usize) -> usize {
    match hueing_mode {
        EcHueingMode::Cc => {
            let r5 = pixel[0] >> 3;
            let g5 = pixel[1] >> 3;
            let b5 = pixel[2] >> 3;
            ((r5 as u16 + g5 as u16 + b5 as u16) / 3).min(31) as usize
        }
        EcHueingMode::Ec => {
            ec_hue_source_luma(pixel, hueing_mode) as usize * (color_count - 1) / 255
        }
    }
}

fn ec_hue_source_luma(pixel: &[u8], hueing_mode: EcHueingMode) -> u8 {
    match hueing_mode {
        EcHueingMode::Cc => ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3) as u8,
        EcHueingMode::Ec => {
            ((pixel[0] as u32 * 54 + pixel[1] as u32 * 182 + pixel[2] as u32 * 18 + 127) / 255)
                as u8
        }
    }
}

fn apply_ec_hue_color_to_pixel(pixel: &mut [u8], hue_color: &[u8], source_luma: u8) {
    if source_luma == 0 {
        pixel[0] = hue_color[0];
        pixel[1] = hue_color[1];
        pixel[2] = hue_color[2];
        return;
    }

    let source_luma = source_luma as u32;
    for channel in 0..3 {
        let value = hue_color[channel] as u32 * pixel[channel] as u32 / source_luma;
        pixel[channel] = value.min(255) as u8;
    }
}

fn cc_hue_should_apply_to_art(art_id: u32, source: ArtSource) -> bool {
    art_id >= uocf::classic::art::STATIC_TILE_ID_BASE || source.is_ec_uop()
}

impl eframe::App for UopInspectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::ui::draw_ui(self, ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.last_view_mode = Some(self.view_mode);
        eframe::set_value(storage, "uocf_inspector_settings", &self.settings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cliloc_payload(text: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_i32.to_le_bytes());
        bytes.extend_from_slice(&2_i16.to_le_bytes());
        bytes.extend_from_slice(&100_i32.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&(text.len() as i16).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn test_app() -> UopInspectorApp {
        UopInspectorApp {
            settings: AppSettings::default(),
            logs: Vec::new(),
            show_search_paths: false,
            dictionary: Dictionary::new(),
            uo_string_dictionary: None,
            cliloc: None,
            cliloc_files: Vec::new(),
            localized_strings: None,
            localized_string_rows: None,
            string_dictionary_raw_hash: None,
            uop_cache: UopCache::new(),
            client_data: None,
            cc_tiledata: None,
            cc_tiledata_rows: None,
            art_viewer_rows: None,
            art_viewer_rows_source: None,
            cc_gumps_package: None,
            ec_gumps_package: None,
            cc_gumps: None,
            cc_sounds: None,
            cc_sound_entries: None,
            cc_multimap: None,
            cc_multimap_path: None,
            generated_multimap_plain: None,
            generated_multimap_artistic: None,
            sound_player: None,
            selected_uop_idx: None,
            selected_file_hash: None,
            selected_tex_art_cc_id: None,
            selected_terrain_def_hash: None,
            selected_tileart_hash: None,
            selected_ec_hue_hash: None,
            selected_multi_uop_hash: None,
            selected_localized_file_hash: None,
            selected_cliloc_file_idx: None,
            selected_legacy_source: ArtSource::Any,
            search_query: String::new(),
            find_hash_query: String::new(),
            hide_empty_uop_entries: true,
            guess_terrain_texture_file_format: false,
            status_message: String::new(),
            view_mode: ViewMode::Home,
            tile_metadata_source: TileMetadataSource::CcTileData,
            hues_source: HuesSource::CcMul,
            multis_source: MultisSource::ClassicMul,
            localized_strings_source: LocalizedStringsSource::Cliloc,
            multimap_preview_selection: MultimapPreviewSelection::Loaded,
            multimap_converter: MultimapConverterState::default(),
            multimap_worker_rx: None,
            multimap_worker_active: false,
            texture_previews: HashMap::new(),
            ec_texture_previews: HashMap::new(),
            ec_texture_preview_source_keys: HashMap::new(),
            uop_entry_labels: HashMap::new(),
            uop_entry_payloads: HashMap::new(),
            terrain_texture_guess_names: HashMap::new(),
            animationframe_uop_entries: HashMap::new(),
            multimap_texture: None,
            image_preview_sources: HashMap::new(),
            current_image_preview_key: None,
            show_upscale_preview: false,
            upscale_preview_passes: vec![UpscalePreviewPass::default()],
            upscale_preview_zoom: 1.0,
            upscale_original_texture: None,
            upscale_original_texture_key: None,
            upscale_preview_texture: None,
            upscale_preview_texture_key: None,
            upscale_preview_worker_key: None,
            upscale_preview_worker_rx: None,
            upscale_preview_size: [0, 0],
            upscale_preview_elapsed_ms: None,
            upscale_preview_status: String::new(),
            selected_gump_source: GumpSource::Classic,
            selected_gump_viewer_tab: GumpViewerTab::StandardGumps,
            selected_gump_id: String::new(),
            standard_gump_ids: Vec::new(),
            paperdoll_gump_ids: Vec::new(),
            gump_list_source: None,
            gump_list_profile: String::new(),
            gump_list_status: String::new(),
            selected_paperdoll_profile: "human_male".to_string(),
            paperdoll_body_id: String::new(),
            paperdoll_body_hue: "0".to_string(),
            paperdoll_equipment: vec![PaperdollEquipmentInput::default(); 6],
            paperdoll_preview: None,
            selected_anim_id: 0,
            selected_animdata_id: 0,
            selected_animdata_art_source: ArtSource::CcUop,
            animdata_frame_delay_ms: 100.0,
            selected_anim_file_idx: 0,
            current_frame_idx: 0,
            is_playing: false,
            last_frame_time: 0.0,
            playback_speed: 1.0,
            loop_animation: true,
            selected_anim_sequence: None,
            selected_action_id: 0,
            selected_direction: 0,
            selected_multi_id: 0,
            selected_sound_slot: 0,
            selected_sound_id: 0,
            sound_search_query: String::new(),
            selected_hue_id: 0,
            selected_ec_hueing_mode: EcHueingMode::Cc,
            selected_ec_hue_id: 1,
            selected_cliloc_number: 0,
            multimap_zoom: 0.25,
            terrain_def_package: None,
            terrain_def_files: None,
            ec_tileart_entries: None,
            ec_tileart_rows: None,
            ec_hues: None,
            multi_collection: None,
            multi_collection_source: None,
            multi_collection_path: None,
        }
    }

    #[test]
    fn test_uocf_inspector_app_manual_log() {
        let mut app = test_app();

        assert_eq!(app.logs.len(), 0);
        app.log("hello test");
        assert_eq!(app.logs.len(), 1);
        assert_eq!(app.logs[0], "hello test");
    }

    #[test]
    fn terrain_texture_guess_covers_common_image_extensions() {
        let dds = "build/terraintexture/00000042.dds";
        let tga = "build/terraintexture/00000042.tga";
        let mut package = UopPackage::new_default();
        package
            .add_file_from_memory(b"dds", dds, uocf::uop_container::file::CompressionFlag::None)
            .expect("add dds terrain texture");
        package
            .add_file_from_memory(b"tga", tga, uocf::uop_container::file::CompressionFlag::None)
            .expect("add tga terrain texture");

        let names = collect_terrain_texture_guess_names(&package);
        assert_eq!(
            names.get(&hash_file_name_single(dds)).map(String::as_str),
            Some(dds)
        );
        assert_eq!(
            names.get(&hash_file_name_single(tga)).map(String::as_str),
            Some(tga)
        );
    }

    #[test]
    fn uop_entry_filter_hides_empty_records_by_default() {
        let empty = uocf::uop_container::file::UopFile::new();
        let populated = uocf::uop_container::file::UopFile::new()
            .create_file_from_bytes(
                b"payload",
                hash_file_name_single("build/terraintexture/00000042.dds"),
                uocf::uop_container::file::CompressionFlag::None,
            )
            .expect("create populated UOP file");

        assert!(!should_show_uop_entry(&empty, true));
        assert!(should_show_uop_entry(&empty, false));
        assert!(should_show_uop_entry(&populated, true));
    }

    #[test]
    fn uop_image_format_guess_uses_payload_content() {
        let mut tga = vec![0; 18];
        tga[2] = 2;
        tga[12..14].copy_from_slice(&1_u16.to_le_bytes());
        tga[14..16].copy_from_slice(&1_u16.to_le_bytes());
        tga[16] = 32;

        assert_eq!(
            guess_uop_image_format_from_payload(b"DDS payload").map(|(extension, format)| (extension, format)),
            Some(("dds", ECImageFormat::DDS))
        );
        assert_eq!(
            guess_uop_image_format_from_payload(&tga).map(|(extension, format)| (extension, format)),
            Some(("tga", ECImageFormat::TGA))
        );
        assert_eq!(guess_uop_image_format_from_payload(b"not image"), None);
    }

    #[test]
    fn palette_snap_preview_pass_restricts_bilinear_output_to_source_colors() {
        let rgba = vec![
            255, 0, 0, 255, 0, 0, 255, 255,
            0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let passes = [
            UpscalePreviewPass {
                algorithm: UpscalePreviewAlgorithm::Bilinear,
                scale: 2,
            },
            UpscalePreviewPass {
                algorithm: UpscalePreviewAlgorithm::PaletteSnapStrict,
                scale: 1,
            },
        ];

        let (width, height, pixels, status) = apply_upscale_preview_passes(2, 2, rgba, &passes);

        assert_eq!((width, height), (4, 4));
        assert!(status.contains("Palette snap"));
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel == [255, 0, 0, 255] || pixel == [0, 0, 255, 255]);
        }
    }

    #[test]
    fn palette_snap_preview_pass_is_part_of_pass_identity() {
        let strict = UpscalePreviewPass {
            algorithm: UpscalePreviewAlgorithm::PaletteSnapStrict,
            scale: 1,
        };
        let expanded = UpscalePreviewPass {
            algorithm: UpscalePreviewAlgorithm::PaletteSnapExpanded,
            scale: 16,
        };

        assert_ne!(strict, expanded);
        assert_eq!(upscale_pass_cli_value(strict), "palette-snap-strict");
        assert_eq!(upscale_pass_cli_value(expanded), "palette-snap-expanded-16");
    }

    #[test]
    fn load_cliloc_files_discovers_translations_and_prefers_enu() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "uocf_inspector_cliloc_translations_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Cliloc.deu"), test_cliloc_payload("bank")).unwrap();
        std::fs::write(dir.join("Cliloc.enu"), test_cliloc_payload("bank")).unwrap();
        std::fs::write(dir.join("tiledata.mul"), []).unwrap();

        let mut logs = Vec::new();
        let clilocs = load_cliloc_files(&dir, |msg| logs.push(msg));

        assert!(logs.is_empty());
        assert_eq!(clilocs.len(), 2);
        assert_eq!(clilocs[0].label, "Cliloc.deu");
        assert_eq!(clilocs[1].label, "Cliloc.enu");
        assert_eq!(default_cliloc_file_index(&clilocs), Some(1));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn select_raw_uop_entry_selects_matching_package() {
        let mut app = test_app();
        app.uop_cache
            .add(PathBuf::from("tileart.uop"), UopPackage::new_default());

        assert!(app.select_raw_uop_entry("TileArt.uop", 0x1234));
        assert_eq!(app.selected_uop_idx, Some(0));
        assert_eq!(app.selected_file_hash, Some(0x1234));
        assert_eq!(app.find_hash_query, "0000000000001234");
        assert_eq!(app.view_mode, ViewMode::UopExplorer);
    }

    #[test]
    fn select_raw_uop_entry_rejects_missing_package() {
        let mut app = test_app();

        assert!(!app.select_raw_uop_entry("missing.uop", 0x1234));
        assert_eq!(app.selected_uop_idx, None);
        assert_eq!(app.selected_file_hash, None);
        assert_eq!(app.view_mode, ViewMode::Home);
    }

    #[test]
    fn select_raw_ec_hue_bitmap_uses_hues_package() {
        let mut app = test_app();
        app.uop_cache
            .add(PathBuf::from("hues.uop"), UopPackage::new_default());

        let expected = uocf::enhanced::hues::hue_bitmap_hash(42);
        assert!(app.select_raw_ec_hue_bitmap(42));
        assert_eq!(app.selected_uop_idx, Some(0));
        assert_eq!(app.selected_file_hash, Some(expected));
        assert_eq!(app.selected_ec_hue_hash, Some(expected));
        assert_eq!(app.find_hash_query, format!("{expected:016X}"));
    }

    #[test]
    fn select_raw_ec_hues_special_files_use_expected_hashes() {
        let mut app = test_app();
        app.uop_cache
            .add(PathBuf::from("hues.uop"), UopPackage::new_default());

        assert!(app.select_raw_ec_hues_atlas());
        assert_eq!(app.selected_file_hash, Some(uocf::enhanced::hues::hues_atlas_hash()));

        assert!(app.select_raw_ec_huenames());
        assert_eq!(app.selected_file_hash, Some(uocf::enhanced::hues::huenames_hash()));

        assert!(app.select_raw_ec_hue_palette());
        assert_eq!(app.selected_file_hash, Some(uocf::enhanced::hues::FIXED_PALETTE_HASH));
    }

    #[test]
    fn missing_hues_uop_does_not_select_raw_ec_hue() {
        let mut app = test_app();

        assert!(!app.select_raw_ec_hue_bitmap(1));
        assert_eq!(app.selected_uop_idx, None);
        assert_eq!(app.selected_file_hash, None);
        assert_eq!(app.view_mode, ViewMode::Home);
    }

    #[test]
    fn ec_hue_application_uses_cc_style_intensity_steps() {
        let mut hue_table = vec![0u8; 32 * 4];
        for color_index in 0..32usize {
            let offset = color_index * 4;
            hue_table[offset] = color_index as u8;
            hue_table[offset + 1] = color_index as u8 + 1;
            hue_table[offset + 2] = color_index as u8 + 2;
            hue_table[offset + 3] = 255;
        }

        let mut pixels = vec![
            0, 0, 0, 240,
            16, 16, 16, 200,
            255, 255, 255, 128,
        ];

        apply_ec_hue_table_to_rgba(&mut pixels, &hue_table, EcHueingMode::Cc);

        assert_eq!(&pixels[0..4], &[0, 1, 2, 240]);
        assert_eq!(&pixels[4..8], &[2, 3, 4, 200]);
        assert_eq!(&pixels[8..12], &[31, 32, 33, 128]);
    }

    #[test]
    fn ec_hue_application_can_use_full_strip_pixels() {
        let mut hue_table = vec![0u8; 256 * 4];
        for color_index in 0..256usize {
            let offset = color_index * 4;
            hue_table[offset] = color_index as u8;
            hue_table[offset + 1] = (color_index + 1).min(255) as u8;
            hue_table[offset + 2] = (color_index + 2).min(255) as u8;
            hue_table[offset + 3] = 255;
        }

        let mut pixels = vec![
            0, 0, 0, 240,
            127, 127, 127, 200,
            255, 255, 255, 128,
        ];

        apply_ec_hue_table_to_rgba(&mut pixels, &hue_table, EcHueingMode::Ec);

        assert_eq!(&pixels[0..4], &[0, 1, 2, 240]);
        assert_eq!(&pixels[4..8], &[127, 128, 129, 200]);
        assert_eq!(&pixels[8..12], &[255, 255, 255, 128]);
    }

    #[test]
    fn ec_hue_application_tints_colored_pixels_instead_of_flat_replacement() {
        for hueing_mode in [EcHueingMode::Cc, EcHueingMode::Ec] {
            let color_count = match hueing_mode {
                EcHueingMode::Cc => 32,
                EcHueingMode::Ec => 256,
            };
            let mut hue_table = vec![0u8; color_count * 4];
            for color in hue_table.chunks_exact_mut(4) {
                color.copy_from_slice(&[80, 40, 20, 255]);
            }
            let mut pixels = vec![
                96, 32, 32, 240,
                32, 96, 32, 240,
            ];

            apply_ec_hue_table_to_rgba(&mut pixels, &hue_table, hueing_mode);

            assert_ne!(&pixels[0..3], &[80, 40, 20]);
            assert_ne!(&pixels[4..7], &[80, 40, 20]);
            assert_ne!(&pixels[0..3], &pixels[4..7]);
            assert_eq!(pixels[3], 240);
            assert_eq!(pixels[7], 240);
        }
    }

    #[test]
    fn cc_hues_apply_to_ec_static_uop_art_ids() {
        assert!(cc_hue_should_apply_to_art(0x4000, ArtSource::CcUop));
        assert!(!cc_hue_should_apply_to_art(0x0001, ArtSource::CcUop));
        assert!(cc_hue_should_apply_to_art(0x0001, ArtSource::EcUopLegacy));
        assert!(cc_hue_should_apply_to_art(0x0001, ArtSource::EcUopKr));
    }

    #[test]
    fn find_client_file_case_insensitive_accepts_ec_hues_casing() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "uocf_inspector_hues_case_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("Hues.uop");
        std::fs::write(&path, []).unwrap();

        assert_eq!(
            find_client_file_case_insensitive(&dir, "hues.uop").as_deref(),
            Some(path.as_path())
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_localized_strings_uop_accepts_ec_casing() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "uocf_inspector_localized_strings_case_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("LocalizedStrings.UOP");
        std::fs::write(&path, []).unwrap();

        assert_eq!(find_localized_strings_uop(&dir).as_deref(), Some(path.as_path()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collect_localized_string_rows_caches_display_and_search_text() {
        let package = LocalizedStringsPackage {
            files: vec![uocf::enhanced::localized_strings::LocalizedStringsFile {
                filename_hash: 0x1234,
                byte_len: 0,
                strings: uocf::enhanced::localized_strings::LocalizedStringTable {
                    header1: 0,
                    header2: 0,
                    entries: vec![uocf::enhanced::localized_strings::LocalizedStringEntry {
                        id: 500052,
                        unk: 0xAB,
                        text: "Bank Balance".to_string(),
                    }],
                },
            }],
        };

        let rows = collect_localized_string_rows(&package);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename_hash, 0x1234);
        assert_eq!(rows[0].rows.len(), 1);
        assert_eq!(rows[0].rows[0].entry_index, 0);
        assert_eq!(rows[0].rows[0].id, "500052");
        assert_eq!(rows[0].rows[0].unk, "0xAB");
        assert!(rows[0].rows[0].search_text.contains("500052"));
        assert!(rows[0].rows[0].search_text.contains("bank balance"));
    }

    #[test]
    fn select_raw_multi_collection_entry_uses_expected_hash() {
        let mut app = test_app();
        app.uop_cache
            .add(PathBuf::from("MultiCollection.uop"), UopPackage::new_default());

        let expected = uocf::enhanced::multis::multi_collection_hash(7);
        assert!(app.select_raw_multi_collection_entry(7));
        assert_eq!(app.selected_file_hash, Some(expected));
        assert_eq!(app.selected_multi_uop_hash, Some(expected));
    }

    #[test]
    fn select_raw_multi_collection_entry_prefers_loaded_source_path() {
        let mut app = test_app();
        let cc_path = PathBuf::from("cc/MultiCollection.uop");
        let ec_path = PathBuf::from("ec/MultiCollection.uop");
        app.uop_cache.add(cc_path, UopPackage::new_default());
        app.uop_cache.add(ec_path.clone(), UopPackage::new_default());
        app.multi_collection_path = Some(ec_path);

        let expected = uocf::enhanced::multis::multi_collection_hash(7);
        assert!(app.select_raw_multi_collection_entry(7));
        assert_eq!(app.selected_uop_idx, Some(1));
        assert_eq!(app.selected_file_hash, Some(expected));
        assert_eq!(app.selected_multi_uop_hash, Some(expected));
    }

    #[test]
    fn select_raw_localized_strings_file_uses_expected_hash() {
        let mut app = test_app();
        app.uop_cache
            .add(PathBuf::from("localizedstrings.uop"), UopPackage::new_default());

        assert!(app.select_raw_localized_strings_file(0x1234));
        assert_eq!(app.selected_file_hash, Some(0x1234));
        assert_eq!(app.selected_localized_file_hash, Some(0x1234));
    }
}
