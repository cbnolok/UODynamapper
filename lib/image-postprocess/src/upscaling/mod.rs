//! Image upscaling filters for build-time asset processing.
//!
//! # Reference Implementations
//! - Kopf-Lischinski: https://github.com/vvanirudh/Pixel-Art
//! - HQx: https://github.com/brunexgeek/hqx
//! - xBRZ: https://docs.rs/xbrz-rs/latest/xbrz/
//! - Super-xBR: https://github.com/hansonw/super-xbr
//! - ScaleFX: https://github.com/Themaister/slang-shaders/tree/master/scalefx
//! - OmniScale: https://github.com/Themaister/slang-shaders/tree/master/omniscale
//! - Jinc2: /home/claudio/Scaricati/jinc2*.glsl
//! - Cheap Upscaling Triangulation: https://github.com/Swordfish90/cheap-upscaling-triangulation
//! - SaI/SuperSaI/SuperEagle: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/2xsai.c
//! - LQ2x: https://github.com/libretro/RetroArch/blob/master/gfx/video_filters/lq2x.c
//! - EPX/Scale2x: https://en.wikipedia.org/wiki/Pixel-art_scaling_algorithms#Scale2x
//! - NEDI: https://en.wikipedia.org/wiki/Edge-directed_interpolation

pub mod fsr;
pub mod depixelize;
pub mod hqx;
pub mod sai;
pub mod xbrz;
pub mod epx;
pub mod nedi;
pub mod lq;
pub mod mmpx;
pub mod super_xbr;
pub mod cut;
pub mod scalefx;
pub mod sharpen;
pub mod omniscale;
pub mod jinc2;

use image::imageops::{self, FilterType};
use image::{ImageBuffer, Rgba};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum UpscaleFilter {
    /// No upscaling. No parameters.
    #[default]
    None,
    /// Nearest-neighbor 2x upscale. Scale is fixed to 2.
    Nearest2x,
    /// Nearest-neighbor 3x upscale. Scale is fixed to 3.
    Nearest3x,
    /// Nearest-neighbor 4x upscale. Scale is fixed to 4.
    Nearest4x,
    /// Bilinear 2x upscale. Scale is fixed to 2.
    Bilinear2x,
    /// Bilinear 3x upscale. Scale is fixed to 3.
    Bilinear3x,
    /// Bilinear 4x upscale. Scale is fixed to 4.
    Bilinear4x,
    /// Catmull-Rom 2x upscale. Scale is fixed to 2.
    CatmullRom2x,
    /// Catmull-Rom 3x upscale. Scale is fixed to 3.
    CatmullRom3x,
    /// Catmull-Rom 4x upscale. Scale is fixed to 4.
    CatmullRom4x,
    /// Lanczos3 2x upscale. Scale is fixed to 2.
    Lanczos3_2x,
    /// Lanczos3 3x upscale. Scale is fixed to 3.
    Lanczos3_3x,
    /// Lanczos3 4x upscale. Scale is fixed to 4.
    Lanczos3_4x,
    /// SuperSaI 2x upscale. Scale is fixed to 2.
    SuperSai2x,
    /// FSR EASU 2x upscale. Scale is fixed to 2.
    FsrEasu2x,
    /// FSR EASU 3x upscale. Scale is fixed to 3.
    FsrEasu3x,
    /// FSR EASU 4x upscale. Scale is fixed to 4.
    FsrEasu4x,
    /// FSR EASU plus RCAS 2x upscale. Scale is fixed to 2.
    FsrEasuRcas2x,
    /// FSR EASU plus RCAS 3x upscale. Scale is fixed to 3.
    FsrEasuRcas3x,
    /// FSR EASU plus RCAS 4x upscale. Scale is fixed to 4.
    FsrEasuRcas4x,
    /// Kopf-Lischinski depixelization 2x upscale. Scale is fixed to 2.
    KLDepixelize2x,
    /// Kopf-Lischinski depixelization 3x upscale. Scale is fixed to 3.
    KLDepixelize3x,
    /// Kopf-Lischinski depixelization 4x upscale. Scale is fixed to 4.
    KLDepixelize4x,

    /// NEDI 2x upscale. Scale is fixed to 2.
    Nedi2x,
    /// 2xSaI upscale. Scale is fixed to 2.
    TwoSai2x,
    /// SuperEagle upscale. Scale is fixed to 2.
    SuperEagle2x,
    /// LQ 2x upscale. Scale is fixed to 2.
    Lq2x,
    /// LQ 3x upscale. Scale is fixed to 3.
    Lq3x,
    /// LQ 4x upscale. Scale is fixed to 4.
    Lq4x,
    /// Simple HQ 2x upscale. Scale is fixed to 2.
    Hq2xSimple,
    /// Simple HQ 3x upscale. Scale is fixed to 3.
    Hq3xSimple,
    /// Simple HQ 4x upscale. Scale is fixed to 4.
    Hq4xSimple,
    /// True HQ 2x upscale. Scale is fixed to 2.
    Hq2xTrue,
    /// True HQ 3x upscale. Scale is fixed to 3.
    Hq3xTrue,
    /// True HQ 4x upscale. Scale is fixed to 4.
    Hq4xTrue,
    /// EPX 2x upscale. Scale is fixed to 2.
    Epx2x,
    /// EPX 3x upscale. Scale is fixed to 3.
    Epx3x,
    /// EPX 4x upscale. Scale is fixed to 4.
    Epx4x,
    /// xBRZ 2x upscale. Scale is fixed to 2.
    Xbrz2x,
    /// xBRZ 3x upscale. Scale is fixed to 3.
    Xbrz3x,
    /// xBRZ 4x upscale. Scale is fixed to 4.
    Xbrz4x,
    /// Super-xBR 2x upscale. Scale is fixed to 2.
    SuperXbr2x,
    /// Cheap Upscaling Triangulation mode 1 at 2x. Scale is fixed to 2.
    Cut1_2x,
    /// Cheap Upscaling Triangulation mode 2 at 2x. Scale is fixed to 2.
    Cut2_2x,
    /// Cheap Upscaling Triangulation mode 3 at 2x. Scale is fixed to 2.
    Cut3_2x,
    /// ScaleFX 2x upscale. Scale is fixed to 2.
    ScaleFx2x,
    /// ScaleFX 3x upscale. Scale is fixed to 3.
    ScaleFx3x,
    /// ScaleFX 4x upscale. Scale is fixed to 4.
    ScaleFx4x,
    /// OmniScale 2x upscale. Scale is fixed to 2.
    OmniScale2x,
    /// OmniScale 3x upscale. Scale is fixed to 3.
    OmniScale3x,
    /// OmniScale 4x upscale. Scale is fixed to 4.
    OmniScale4x,
    /// Jinc2 2x upscale. Scale is fixed to 2.
    Jinc2_2x,
    /// Jinc2 3x upscale. Scale is fixed to 3.
    Jinc2_3x,
    /// Jinc2 4x upscale. Scale is fixed to 4.
    Jinc2_4x,
    /// Jinc2 sharp 2x upscale. Scale is fixed to 2.
    Jinc2Sharp2x,
    /// Jinc2 sharp 3x upscale. Scale is fixed to 3.
    Jinc2Sharp3x,
    /// Jinc2 sharp 4x upscale. Scale is fixed to 4.
    Jinc2Sharp4x,
    /// Jinc2 sharper 2x upscale. Scale is fixed to 2.
    Jinc2Sharper2x,
    /// Jinc2 sharper 3x upscale. Scale is fixed to 3.
    Jinc2Sharper3x,
    /// Jinc2 sharper 4x upscale. Scale is fixed to 4.
    Jinc2Sharper4x,
    /// Jinc2 sharpest 2x upscale. Scale is fixed to 2.
    Jinc2Sharpest2x,
    /// Jinc2 sharpest 3x upscale. Scale is fixed to 3.
    Jinc2Sharpest3x,
    /// Jinc2 sharpest 4x upscale. Scale is fixed to 4.
    Jinc2Sharpest4x,
    /// MMPX 2x upscale. Scale is fixed to 2.
    Mmpx2x,
    /// MMPX 4x upscale. Scale is fixed to 4.
    Mmpx4x,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EnhancementFilter {
    /// Perceptual vibrance boost. `factor` range: 0.0..=1.0.
    Vibrance { factor: f32 },
    /// Global saturation multiplier. `factor` range: 0.0..=2.0.
    Saturation { factor: f32 },
    /// Warm hue selective saturation boost. `factor` range: 0.0..=1.0.
    SelectiveWarm { factor: f32 },
    /// Green hue selective saturation boost. `factor` range: 0.0..=1.0.
    SelectiveGreen { factor: f32 },
    /// Local Laplacian clarity. `radius` range: 1..=16; `amount` range: 0.0..=1.0.
    LocalLaplacianClarity { radius: u32, amount: f32 },
    /// Local contrast enhancement. `intensity` range: 0.0..=1.0; `threshold` range: 0.0..=1.0; `blur_spread` range: 0.1..=16.0.
    ContrastEnhance { intensity: f32, threshold: f32, blur_spread: f32 },
    /// Adaptive logarithmic contrast. `radius` range: 0.1..=16.0; `gamma` range: 0.1..=4.0.
    AdaptiveLogContrast { radius: f32, gamma: f32 },
    /// Unsharp mask. `radius` range: 0.1..=16.0; `amount` range: 0.0..=2.0.
    UnsharpMask { radius: f32, amount: f32 },
    /// High-pass sharpening. `radius` range: 0.1..=16.0; `strength` range: 0.0..=2.0.
    HighPassSharpen { radius: f32, strength: f32 },
    /// ScaleFX-style smart deblur. `deblur_offset` range: 0.0..=2.0; `deblur_strength` range: 0.0..=2.0; `smart_deblur` range: 0.0..=1.0.
    ScaleFxSmartDeblur { deblur_offset: f32, deblur_strength: f32, smart_deblur: f32 },
    /// Guest.r-style deblur. No parameters.
    GuestrDeblur,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpscaleConfig {
    /// Target width/height for upscaling. 0 means use original size or scale factor.
    pub target_size: u32,
    pub filter: UpscaleFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum UpscalePass {
    None,
    Upscale(UpscaleFilter),
    Enhancement(EnhancementFilter),
}

impl UpscalePass {
    pub fn new(filter: UpscaleFilter) -> Self {
        Self::from(filter)
    }

    pub fn enhancement(filter: EnhancementFilter) -> Self {
        Self::from(filter)
    }

    pub fn filter(self) -> Option<UpscaleFilter> {
        match self {
            Self::None => Some(UpscaleFilter::None),
            Self::Upscale(filter) => Some(filter),
            Self::Enhancement(_) => None,
        }
    }

    pub fn scale_factor(self) -> u32 {
        self.filter().map_or(1, UpscaleFilter::scale_factor)
    }

    pub fn apply(self, width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
        match self {
            Self::None => (width, height, rgba.to_vec()),
            Self::Upscale(filter) => filter.apply(width, height, rgba),
            Self::Enhancement(filter) => (width, height, filter.apply(width, height, rgba)),
        }
    }

    pub fn apply_owned(self, width: u32, height: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
        match self {
            Self::None => (width, height, rgba),
            Self::Upscale(filter) => filter.apply(width, height, &rgba),
            Self::Enhancement(filter) => (width, height, filter.apply(width, height, &rgba)),
        }
    }
}

impl From<UpscaleFilter> for UpscalePass {
    fn from(filter: UpscaleFilter) -> Self {
        if matches!(filter, UpscaleFilter::None) {
            Self::None
        } else {
            Self::Upscale(filter)
        }
    }
}

impl From<EnhancementFilter> for UpscalePass {
    fn from(filter: EnhancementFilter) -> Self {
        Self::Enhancement(filter)
    }
}


impl UpscaleFilter {
    pub fn scale_factor(self) -> u32 {
        match self {
            Self::None => 1,
            Self::Nearest2x | Self::Bilinear2x | Self::CatmullRom2x | Self::Lanczos3_2x => 2,
            Self::Nearest3x | Self::Bilinear3x | Self::CatmullRom3x | Self::Lanczos3_3x => 3,
            Self::Nearest4x | Self::Bilinear4x | Self::CatmullRom4x | Self::Lanczos3_4x => 4,
            Self::Lq2x => 2,
            Self::Lq3x => 3,
            Self::Lq4x => 4,
            Self::SuperSai2x => 2,
            Self::TwoSai2x => 2,
            Self::SuperEagle2x => 2,
            Self::FsrEasu2x | Self::FsrEasuRcas2x => 2,
            Self::FsrEasu3x | Self::FsrEasuRcas3x => 3,
            Self::FsrEasu4x | Self::FsrEasuRcas4x => 4,
            Self::KLDepixelize2x => 2,
            Self::KLDepixelize3x => 3,
            Self::KLDepixelize4x => 4,
            Self::Hq2xSimple | Self::Hq2xTrue => 2,
            Self::Hq3xSimple | Self::Hq3xTrue => 3,
            Self::Hq4xSimple | Self::Hq4xTrue => 4,
            Self::Epx2x => 2,
            Self::Epx3x => 3,
            Self::Epx4x => 4,
            Self::Xbrz2x => 2,
            Self::Xbrz3x => 3,
            Self::Xbrz4x => 4,
            Self::SuperXbr2x => 2,
            Self::Cut1_2x => 2,
            Self::Cut2_2x => 2,
            Self::Cut3_2x => 2,
            Self::ScaleFx2x => 2,
            Self::ScaleFx3x => 3,
            Self::ScaleFx4x => 4,
            Self::OmniScale2x => 2,
            Self::OmniScale3x => 3,
            Self::OmniScale4x => 4,
            Self::Jinc2_2x | Self::Jinc2Sharp2x | Self::Jinc2Sharper2x | Self::Jinc2Sharpest2x => 2,
            Self::Jinc2_3x | Self::Jinc2Sharp3x | Self::Jinc2Sharper3x | Self::Jinc2Sharpest3x => 3,
            Self::Jinc2_4x | Self::Jinc2Sharp4x | Self::Jinc2Sharper4x | Self::Jinc2Sharpest4x => 4,
            Self::Nedi2x => 2,
            Self::Mmpx2x => 2,
            Self::Mmpx4x => 4,
        }
    }

    /// Whether the filter's output can stay in indexed storage: it adds no colour that isn't in the
    /// input and keeps alpha binary. Measured on 44 Classic animation frames (small, medium and large
    /// bodies and a worn item) and 20 Classic gumps: these filters only copy source pixels, and every
    /// other one blends, adding colours on all 64 samples. A new filter is classified the same way,
    /// by measuring it, and added to the test list below.
    pub fn keeps_palette(self) -> bool {
        match self {
            Self::None => true,
            Self::Nearest2x => true,
            Self::Nearest3x => true,
            Self::Nearest4x => true,
            Self::Bilinear2x => false,
            Self::Bilinear3x => false,
            Self::Bilinear4x => false,
            Self::CatmullRom2x => false,
            Self::CatmullRom3x => false,
            Self::CatmullRom4x => false,
            Self::Lanczos3_2x => false,
            Self::Lanczos3_3x => false,
            Self::Lanczos3_4x => false,
            Self::SuperSai2x => false,
            Self::FsrEasu2x => false,
            Self::FsrEasu3x => false,
            Self::FsrEasu4x => false,
            Self::FsrEasuRcas2x => false,
            Self::FsrEasuRcas3x => false,
            Self::FsrEasuRcas4x => false,
            Self::KLDepixelize2x => true,
            Self::KLDepixelize3x => true,
            Self::KLDepixelize4x => true,
            Self::Nedi2x => false,
            Self::TwoSai2x => false,
            Self::SuperEagle2x => false,
            Self::Lq2x => false,
            // Unlike LQ 2x and 4x, LQ 3x picks one of the four neighbours per output pixel, as Scale3x does.
            Self::Lq3x => true,
            Self::Lq4x => false,
            Self::Hq2xSimple => false,
            Self::Hq3xSimple => false,
            Self::Hq4xSimple => false,
            Self::Hq2xTrue => false,
            Self::Hq3xTrue => false,
            Self::Hq4xTrue => false,
            Self::Epx2x => true,
            Self::Epx3x => true,
            Self::Epx4x => true,
            Self::Xbrz2x => false,
            Self::Xbrz3x => false,
            Self::Xbrz4x => false,
            Self::SuperXbr2x => false,
            Self::Cut1_2x => false,
            Self::Cut2_2x => false,
            Self::Cut3_2x => false,
            Self::ScaleFx2x => true,
            Self::ScaleFx3x => true,
            Self::ScaleFx4x => true,
            Self::OmniScale2x => false,
            Self::OmniScale3x => false,
            Self::OmniScale4x => false,
            Self::Jinc2_2x => false,
            Self::Jinc2_3x => false,
            Self::Jinc2_4x => false,
            Self::Jinc2Sharp2x => false,
            Self::Jinc2Sharp3x => false,
            Self::Jinc2Sharp4x => false,
            Self::Jinc2Sharper2x => false,
            Self::Jinc2Sharper3x => false,
            Self::Jinc2Sharper4x => false,
            Self::Jinc2Sharpest2x => false,
            Self::Jinc2Sharpest3x => false,
            Self::Jinc2Sharpest4x => false,
            Self::Mmpx2x => true,
            Self::Mmpx4x => true,
        }
    }

    pub fn apply(&self, width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
        if matches!(self, Self::None) || rgba.is_empty() {
            return (width, height, rgba.to_vec());
        }

        let scale = self.scale_factor();
        let target_width = width * scale;
        let target_height = height * scale;

        let pixels = self.apply_to_size(width, height, rgba, target_width, target_height);
        (target_width, target_height, pixels)
    }

    pub fn apply_to_size(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
        target_width: u32,
        target_height: u32,
    ) -> Vec<u8> {
        if rgba.is_empty() {
            return rgba.to_vec();
        }

        if matches!(self, Self::None) {
            return rgba.to_vec();
        }

        if width == target_width && height == target_height {
            return rgba.to_vec();
        }

        match self {
            Self::None => rgba.to_vec(),
            Self::Nearest2x | Self::Nearest3x | Self::Nearest4x => {
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::Nearest);
                upscaled.into_raw()
            }
            Self::Bilinear2x | Self::Bilinear3x | Self::Bilinear4x => {
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::Triangle);
                upscaled.into_raw()
            }
            Self::CatmullRom2x | Self::CatmullRom3x | Self::CatmullRom4x => {
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::CatmullRom);
                upscaled.into_raw()
            }
            Self::Lanczos3_2x | Self::Lanczos3_3x | Self::Lanczos3_4x => {
                let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba).unwrap();
                let upscaled =
                    imageops::resize(&img, target_width, target_height, FilterType::Lanczos3);
                upscaled.into_raw()
            }
            Self::FsrEasu2x | Self::FsrEasu3x | Self::FsrEasu4x => fsr::apply_easu(width, height, rgba, target_width, target_height).2,
            Self::FsrEasuRcas2x | Self::FsrEasuRcas3x | Self::FsrEasuRcas4x => {
                let (_, _, easu_rgba) = fsr::apply_easu(width, height, rgba, target_width, target_height);
                fsr::apply_rcas(target_width, target_height, &easu_rgba, 0.0)
            }
            Self::KLDepixelize2x | Self::KLDepixelize3x | Self::KLDepixelize4x => {
                let scale = (target_width / width).max(1);
                depixelize::apply_depixelize(width, height, rgba, scale).2
            }
            Self::Lq2x | Self::Lq3x | Self::Lq4x => {
                let scale = (target_width / width).max(1);
                lq::apply_lq(width, height, rgba, scale).2
            }
            Self::Hq2xSimple | Self::Hq3xSimple | Self::Hq4xSimple => {
                let scale = (target_width / width).max(1);
                hqx::apply_hqx(width, height, rgba, scale).2
            }
            Self::Hq2xTrue | Self::Hq3xTrue | Self::Hq4xTrue => {
                let scale = (target_width / width).max(1);
                let input_pixels_cnt = (width * height) as usize;
                let output_pixels_cnt = (target_width * target_height) as usize;
                let mut output_pixels32: Vec<u32> = vec![0; output_pixels_cnt];
                let input_pixels32: &[u32] = unsafe { std::slice::from_raw_parts(rgba.as_ptr() as *const u32, input_pixels_cnt) };

                if scale == 2 {
                    ::hqx::hq2x(input_pixels32, &mut output_pixels32, width as usize, height as usize);
                } else if scale == 3 {
                    ::hqx::hq3x(input_pixels32, &mut output_pixels32, width as usize, height as usize);
                } else if scale == 4 {
                    ::hqx::hq4x(input_pixels32, &mut output_pixels32, width as usize, height as usize);
                }

                let byte_length = output_pixels32.len() * 4;
                let byte_slice: &[u8] = unsafe { std::slice::from_raw_parts(output_pixels32.as_ptr() as *const u8, byte_length) };
                byte_slice.to_vec()
            }
            Self::TwoSai2x | Self::SuperSai2x | Self::SuperEagle2x => {
                let scale = (target_width / width).max(1);
                sai::apply_sai(width, height, rgba, scale, *self).2
            }
            Self::Epx2x | Self::Epx3x | Self::Epx4x => {
                let scale = (target_width / width).max(1);
                epx::apply_epx(width, height, rgba, scale).2
            }
            Self::Xbrz2x | Self::Xbrz3x | Self::Xbrz4x => {
                let scale = (target_width / width).max(1);
                xbrz::apply_xbrz(width, height, rgba, scale).2
            }
            Self::SuperXbr2x => super_xbr::apply_super_xbr(width, height, rgba).2,
            Self::Cut1_2x => cut::apply_cut(width, height, rgba, cut::CutMode::Cut1).2,
            Self::Cut2_2x => cut::apply_cut(width, height, rgba, cut::CutMode::Cut2).2,
            Self::Cut3_2x => cut::apply_cut(width, height, rgba, cut::CutMode::Cut3).2,
            Self::ScaleFx2x => scalefx::apply_scalefx(width, height, rgba, scalefx::ScaleFxMode::Scale2x).2,
            Self::ScaleFx3x => scalefx::apply_scalefx(width, height, rgba, scalefx::ScaleFxMode::Scale3x).2,
            Self::ScaleFx4x => scalefx::apply_scalefx(width, height, rgba, scalefx::ScaleFxMode::Scale4x).2,
            Self::OmniScale2x => omniscale::apply_omniscale(width, height, rgba, omniscale::OmniScaleMode::Scale2x).2,
            Self::OmniScale3x => omniscale::apply_omniscale(width, height, rgba, omniscale::OmniScaleMode::Scale3x).2,
            Self::OmniScale4x => omniscale::apply_omniscale(width, height, rgba, omniscale::OmniScaleMode::Scale4x).2,
            Self::Jinc2_2x | Self::Jinc2_3x | Self::Jinc2_4x => {
                jinc2::apply_jinc2(width, height, rgba, target_width, target_height, jinc2::Jinc2Mode::Jinc2).2
            }
            Self::Jinc2Sharp2x | Self::Jinc2Sharp3x | Self::Jinc2Sharp4x => {
                jinc2::apply_jinc2(width, height, rgba, target_width, target_height, jinc2::Jinc2Mode::Sharp).2
            }
            Self::Jinc2Sharper2x | Self::Jinc2Sharper3x | Self::Jinc2Sharper4x => {
                jinc2::apply_jinc2(width, height, rgba, target_width, target_height, jinc2::Jinc2Mode::Sharper).2
            }
            Self::Jinc2Sharpest2x | Self::Jinc2Sharpest3x | Self::Jinc2Sharpest4x => {
                jinc2::apply_jinc2(width, height, rgba, target_width, target_height, jinc2::Jinc2Mode::Sharpest).2
            }
            Self::Nedi2x => nedi::apply_nedi(width, height, rgba).2,
            Self::Mmpx2x | Self::Mmpx4x => {
                let scale = (target_width / width).max(1);
                mmpx::apply_mmpx(width, height, rgba, scale).2
            }
        }
    }
}

impl EnhancementFilter {
    pub fn apply(self, width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        match self {
            Self::Vibrance { factor } => {
                let mut out = Vec::with_capacity(rgba.len());
                apply_vibrance(rgba, &mut out, factor);
                out
            }
            Self::Saturation { factor } => {
                let mut out = Vec::with_capacity(rgba.len());
                apply_saturation(rgba, &mut out, factor);
                out
            }
            Self::SelectiveWarm { factor } => {
                let mut out = Vec::with_capacity(rgba.len());
                apply_selective_hue_boost(rgba, &mut out, factor, HueBoostRange::Warm);
                out
            }
            Self::SelectiveGreen { factor } => {
                let mut out = Vec::with_capacity(rgba.len());
                apply_selective_hue_boost(rgba, &mut out, factor, HueBoostRange::Green);
                out
            }
            Self::LocalLaplacianClarity { radius, amount } => sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams { radius, amount },
            ).2,
            Self::ContrastEnhance { intensity, threshold, blur_spread } => sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams { intensity, threshold, blur_spread },
            ).2,
            Self::AdaptiveLogContrast { radius, gamma } => sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams { radius, gamma },
            ).2,
            Self::UnsharpMask { radius, amount } => sharpen::apply_unsharp_mask(
                width,
                height,
                rgba,
                sharpen::UnsharpMaskParams { radius, amount },
            ).2,
            Self::HighPassSharpen { radius, strength } => sharpen::apply_high_pass_sharpen(
                width,
                height,
                rgba,
                sharpen::HighPassSharpenParams { radius, strength },
            ).2,
            Self::ScaleFxSmartDeblur { deblur_offset, deblur_strength, smart_deblur } => sharpen::apply_scalefx_smart_deblur(
                width,
                height,
                rgba,
                sharpen::ScaleFxSmartDeblurParams {
                    deblur_offset,
                    deblur_strength,
                    smart_deblur,
                },
            ).2,
            Self::GuestrDeblur => sharpen::apply_guestr_deblur(
                width,
                height,
                rgba,
                sharpen::GuestrDeblurParams::default(),
            ).2,
        }
    }
}

#[derive(Clone, Copy)]
enum HueBoostRange {
    Warm,
    Green,
}

fn apply_vibrance(rgba: &[u8], out: &mut Vec<u8>, factor: f32) {
    for pixel in rgba.chunks_exact(4) {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        let gray = luma(r, g, b);
        let vibrance = 1.0 - r.max(g).max(b);
        let amount = 1.0 + vibrance.clamp(0.0, 1.0) * factor;
        push_rgb_with_alpha(out, gray + (r - gray) * amount, gray + (g - gray) * amount, gray + (b - gray) * amount, pixel[3]);
    }
}

fn apply_saturation(rgba: &[u8], out: &mut Vec<u8>, factor: f32) {
    for pixel in rgba.chunks_exact(4) {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        let gray = luma(r, g, b);
        push_rgb_with_alpha(out, gray + (r - gray) * factor, gray + (g - gray) * factor, gray + (b - gray) * factor, pixel[3]);
    }
}

fn apply_selective_hue_boost(rgba: &[u8], out: &mut Vec<u8>, factor: f32, range: HueBoostRange) {
    for pixel in rgba.chunks_exact(4) {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        let Some(hue) = hue_degrees(r, g, b) else {
            out.extend_from_slice(pixel);
            continue;
        };
        let weight = hue_range_weight(hue, range);
        if weight <= 0.0 {
            out.extend_from_slice(pixel);
            continue;
        }

        let gray = luma(r, g, b);
        let amount = 1.0 + factor * weight;
        push_rgb_with_alpha(out, gray + (r - gray) * amount, gray + (g - gray) * amount, gray + (b - gray) * amount, pixel[3]);
    }
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    r * 0.30 + g * 0.59 + b * 0.11
}

fn hue_degrees(r: f32, g: f32, b: f32) -> Option<f32> {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta <= f32::EPSILON {
        return None;
    }

    let hue = if (max - r).abs() <= f32::EPSILON {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() <= f32::EPSILON {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Some(hue)
}

fn hue_range_weight(hue: f32, range: HueBoostRange) -> f32 {
    match range {
        HueBoostRange::Warm => circular_range_weight(hue, 20.0, 55.0),
        HueBoostRange::Green => linear_range_weight(hue, 120.0, 55.0),
    }
}

fn circular_range_weight(hue: f32, center: f32, half_width: f32) -> f32 {
    let distance = (hue - center).abs().min(360.0 - (hue - center).abs());
    (1.0 - distance / half_width).clamp(0.0, 1.0)
}

fn linear_range_weight(hue: f32, center: f32, half_width: f32) -> f32 {
    let distance = (hue - center).abs();
    (1.0 - distance / half_width).clamp(0.0, 1.0)
}

fn push_rgb_with_alpha(out: &mut Vec<u8>, r: f32, g: f32, b: f32, a: u8) {
    out.push((r.clamp(0.0, 1.0) * 255.0).round() as u8);
    out.push((g.clamp(0.0, 1.0) * 255.0).round() as u8);
    out.push((b.clamp(0.0, 1.0) * 255.0).round() as u8);
    out.push(a);
}

pub fn apply_filter_passes(
    width: u32,
    height: u32,
    rgba: &[u8],
    passes: &[UpscaleFilter],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    let passes = passes.iter().copied().map(UpscalePass::from).collect::<Vec<_>>();
    apply_upscale_passes_owned(width, height, rgba.to_vec(), &passes)
}

pub fn apply_filter_passes_owned(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    passes: &[UpscaleFilter],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    let passes = passes.iter().copied().map(UpscalePass::from).collect::<Vec<_>>();
    apply_upscale_passes_owned(width, height, rgba, &passes)
}

pub fn apply_upscale_passes(
    width: u32,
    height: u32,
    rgba: &[u8],
    passes: &[UpscalePass],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    apply_upscale_passes_owned(width, height, rgba.to_vec(), passes)
}

pub fn apply_upscale_passes_owned(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    passes: &[UpscalePass],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    let mut width = width;
    let mut height = height;
    let mut rgba = rgba;
    let mut scale_factor = 1u32;
    let mut last_filter = UpscaleFilter::None;

    for pass in passes.iter().copied().filter(|pass| !matches!(pass, UpscalePass::None)) {
        let (next_width, next_height, next_rgba) = pass.apply_owned(width, height, rgba);
        width = next_width;
        height = next_height;
        rgba = next_rgba;
        scale_factor = scale_factor.saturating_mul(pass.scale_factor());
        if let Some(filter) = pass.filter() {
            last_filter = filter;
        }
    }

    (width, height, rgba, scale_factor, last_filter)
}

#[cfg(test)]
mod tests {
    use super::{apply_upscale_passes, EnhancementFilter, UpscaleFilter, UpscalePass};

    #[test]
    fn vibrance_boost_preserves_dimensions_and_alpha() {
        let rgba = [128, 96, 96, 77];
        let (width, height, pixels, scale, last_filter) =
            apply_upscale_passes(1, 1, &rgba, &[UpscalePass::from(EnhancementFilter::Vibrance { factor: 0.30 })]);

        assert_eq!((width, height), (1, 1));
        assert_eq!(scale, 1);
        assert_eq!(last_filter, UpscaleFilter::None);
        assert_eq!(pixels[3], 77);
        assert!(pixels[0] > rgba[0]);
        assert!(pixels[1] < rgba[1]);
        assert!(pixels[2] < rgba[2]);
    }

    #[test]
    fn saturation_boost_keeps_neutral_gray_neutral() {
        let rgba = [80, 80, 80, 255];
        let (_, _, pixels, _, _) =
            apply_upscale_passes(1, 1, &rgba, &[UpscalePass::from(EnhancementFilter::Saturation { factor: 1.25 })]);

        assert_eq!(pixels, rgba);
    }

    #[test]
    fn selective_warm_boost_leaves_blue_unchanged() {
        let rgba = [120, 80, 60, 255, 80, 80, 160, 128];
        let (_, _, pixels, scale, _) =
            apply_upscale_passes(2, 1, &rgba, &[UpscalePass::from(EnhancementFilter::SelectiveWarm { factor: 0.30 })]);

        assert_eq!(scale, 1);
        assert!(pixels[0] > rgba[0]);
        assert!(pixels[1] < rgba[1]);
        assert_eq!(&pixels[4..8], &rgba[4..8]);
    }

    #[test]
    fn selective_green_boost_targets_green_hues() {
        let rgba = [70, 130, 80, 255, 120, 80, 60, 255];
        let (_, _, pixels, _, _) =
            apply_upscale_passes(2, 1, &rgba, &[UpscalePass::from(EnhancementFilter::SelectiveGreen { factor: 0.30 })]);

        assert!(pixels[1] > rgba[1]);
        assert!(pixels[0] < rgba[0]);
        assert_eq!(&pixels[4..8], &rgba[4..8]);
    }

    #[test]
    fn local_contrast_passes_preserve_dimensions_and_alpha() {
        let rgba = [40, 40, 40, 11, 80, 80, 80, 22, 120, 120, 120, 33, 160, 160, 160, 44];
        for pass in [
            UpscalePass::from(EnhancementFilter::LocalLaplacianClarity { radius: 3, amount: 0.25 }),
            UpscalePass::from(EnhancementFilter::ContrastEnhance { intensity: 0.35, threshold: 0.08, blur_spread: 2.5 }),
            UpscalePass::from(EnhancementFilter::AdaptiveLogContrast { radius: 3.0, gamma: 0.80 }),
        ] {
            let (width, height, pixels, scale, last_filter) =
                apply_upscale_passes(4, 1, &rgba, &[pass]);

            assert_eq!((width, height), (4, 1));
            assert_eq!(scale, 1);
            assert_eq!(last_filter, UpscaleFilter::None);
            assert_eq!(pixels.len(), rgba.len());
            assert_eq!(pixels.chunks_exact(4).map(|pixel| pixel[3]).collect::<Vec<_>>(), vec![11, 22, 33, 44]);
        }
    }

    /// Every filter, so each one's palette classification is checked. A new filter goes here too.
    const ALL_FILTERS: [UpscaleFilter; 65] = [
        UpscaleFilter::None,
        UpscaleFilter::Nearest2x,
        UpscaleFilter::Nearest3x,
        UpscaleFilter::Nearest4x,
        UpscaleFilter::Bilinear2x,
        UpscaleFilter::Bilinear3x,
        UpscaleFilter::Bilinear4x,
        UpscaleFilter::CatmullRom2x,
        UpscaleFilter::CatmullRom3x,
        UpscaleFilter::CatmullRom4x,
        UpscaleFilter::Lanczos3_2x,
        UpscaleFilter::Lanczos3_3x,
        UpscaleFilter::Lanczos3_4x,
        UpscaleFilter::SuperSai2x,
        UpscaleFilter::FsrEasu2x,
        UpscaleFilter::FsrEasu3x,
        UpscaleFilter::FsrEasu4x,
        UpscaleFilter::FsrEasuRcas2x,
        UpscaleFilter::FsrEasuRcas3x,
        UpscaleFilter::FsrEasuRcas4x,
        UpscaleFilter::KLDepixelize2x,
        UpscaleFilter::KLDepixelize3x,
        UpscaleFilter::KLDepixelize4x,
        UpscaleFilter::Nedi2x,
        UpscaleFilter::TwoSai2x,
        UpscaleFilter::SuperEagle2x,
        UpscaleFilter::Lq2x,
        UpscaleFilter::Lq3x,
        UpscaleFilter::Lq4x,
        UpscaleFilter::Hq2xSimple,
        UpscaleFilter::Hq3xSimple,
        UpscaleFilter::Hq4xSimple,
        UpscaleFilter::Hq2xTrue,
        UpscaleFilter::Hq3xTrue,
        UpscaleFilter::Hq4xTrue,
        UpscaleFilter::Epx2x,
        UpscaleFilter::Epx3x,
        UpscaleFilter::Epx4x,
        UpscaleFilter::Xbrz2x,
        UpscaleFilter::Xbrz3x,
        UpscaleFilter::Xbrz4x,
        UpscaleFilter::SuperXbr2x,
        UpscaleFilter::Cut1_2x,
        UpscaleFilter::Cut2_2x,
        UpscaleFilter::Cut3_2x,
        UpscaleFilter::ScaleFx2x,
        UpscaleFilter::ScaleFx3x,
        UpscaleFilter::ScaleFx4x,
        UpscaleFilter::OmniScale2x,
        UpscaleFilter::OmniScale3x,
        UpscaleFilter::OmniScale4x,
        UpscaleFilter::Jinc2_2x,
        UpscaleFilter::Jinc2_3x,
        UpscaleFilter::Jinc2_4x,
        UpscaleFilter::Jinc2Sharp2x,
        UpscaleFilter::Jinc2Sharp3x,
        UpscaleFilter::Jinc2Sharp4x,
        UpscaleFilter::Jinc2Sharper2x,
        UpscaleFilter::Jinc2Sharper3x,
        UpscaleFilter::Jinc2Sharper4x,
        UpscaleFilter::Jinc2Sharpest2x,
        UpscaleFilter::Jinc2Sharpest3x,
        UpscaleFilter::Jinc2Sharpest4x,
        UpscaleFilter::Mmpx2x,
        UpscaleFilter::Mmpx4x,
    ];

    /// The opaque colours of the synthetic indexed frames. No opaque black, so transparent pixels
    /// bled into opaque ones show up as a new colour.
    const TEST_PALETTE: [[u8; 3]; 6] =
        [[200, 40, 30], [30, 160, 60], [40, 60, 210], [240, 220, 120], [90, 90, 90], [12, 8, 20]];
    const TRANSPARENT_PIXEL: [u8; 4] = [0, 0, 0, 0];
    const ALPHA_TRANSPARENT: u8 = 0;
    const ALPHA_OPAQUE: u8 = 255;
    const RGBA_BYTES: usize = 4;
    /// Sides of the synthetic frames: odd and unequal, so no filter only ever sees a power of two.
    const NOISE_WIDTH: u32 = 23;
    const NOISE_HEIGHT: u32 = 19;
    const SHAPES_WIDTH: u32 = 29;
    const SHAPES_HEIGHT: u32 = 25;
    /// Numerical Recipes' 32-bit LCG, for a reproducible noise frame; its low bits are weak, so
    /// draws use the high half.
    const LCG_MULTIPLIER: u32 = 1_664_525;
    const LCG_INCREMENT: u32 = 1_013_904_223;
    const LCG_SEED: u32 = 0x5EED;
    const LCG_HIGH_HALF_SHIFT: u32 = 16;
    /// One noise pixel in this many is transparent.
    const NOISE_TRANSPARENT_ONE_IN: u32 = 5;
    /// The shapes frame: a striped ellipse, two one-pixel lines and a grid of isolated dots.
    const ELLIPSE_RADIUS_SQUARED: i32 = 60;
    const ELLIPSE_Y_STRETCH: i32 = 2;
    const STRIPE_WIDTH: i32 = 3;
    const SHALLOW_LINE_RUN: i32 = 2;
    const DOT_PERIOD_X: i32 = 7;
    const DOT_PERIOD_Y: i32 = 5;
    const DOT_PHASE_X: i32 = 3;
    const DOT_PHASE_Y: i32 = 1;
    const LINE_COLOUR: usize = 0;
    const DOT_COLOUR: usize = 4;

    fn indexed_pixel(index: Option<usize>) -> [u8; 4] {
        index.map_or(TRANSPARENT_PIXEL, |index| {
            let [r, g, b] = TEST_PALETTE[index];
            [r, g, b, ALPHA_OPAQUE]
        })
    }

    fn noise_frame() -> Vec<u8> {
        let mut state = LCG_SEED;
        (0..NOISE_WIDTH * NOISE_HEIGHT)
            .flat_map(|_| {
                state = state.wrapping_mul(LCG_MULTIPLIER).wrapping_add(LCG_INCREMENT);
                let draw = state >> LCG_HIGH_HALF_SHIFT;
                let index = (draw % NOISE_TRANSPARENT_ONE_IN != 0)
                    .then(|| (draw / NOISE_TRANSPARENT_ONE_IN) as usize % TEST_PALETTE.len());
                indexed_pixel(index)
            })
            .collect()
    }

    fn shapes_frame() -> Vec<u8> {
        let (centre_x, centre_y) = (SHAPES_WIDTH as i32 / 2, SHAPES_HEIGHT as i32 / 2);
        (0..SHAPES_HEIGHT as i32)
            .flat_map(|y| (0..SHAPES_WIDTH as i32).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let (dx, dy) = (x - centre_x, y - centre_y);
                let index = if dx * dx + ELLIPSE_Y_STRETCH * dy * dy < ELLIPSE_RADIUS_SQUARED {
                    Some(((x + y) / STRIPE_WIDTH) as usize % TEST_PALETTE.len())
                } else if x == y || x == SHALLOW_LINE_RUN * y {
                    Some(LINE_COLOUR)
                } else if x % DOT_PERIOD_X == DOT_PHASE_X && y % DOT_PERIOD_Y == DOT_PHASE_Y {
                    Some(DOT_COLOUR)
                } else {
                    None
                };
                indexed_pixel(index)
            })
            .collect()
    }

    /// Whether `rgba` has a pixel indexed storage can't hold: partial alpha, or an opaque colour
    /// outside the test palette.
    fn leaves_palette(rgba: &[u8]) -> bool {
        rgba.chunks_exact(RGBA_BYTES).any(|pixel| match pixel[3] {
            ALPHA_TRANSPARENT => false,
            ALPHA_OPAQUE => !TEST_PALETTE.contains(&[pixel[0], pixel[1], pixel[2]]),
            _ => true,
        })
    }

    #[test]
    fn palette_keeping_filters_add_no_colour_and_keep_alpha_binary() {
        let frames = [(NOISE_WIDTH, NOISE_HEIGHT, noise_frame()), (SHAPES_WIDTH, SHAPES_HEIGHT, shapes_frame())];
        for filter in ALL_FILTERS {
            let left = frames.iter().any(|(width, height, rgba)| leaves_palette(&filter.apply(*width, *height, rgba).2));
            // Blending filters must trip too, or these frames couldn't catch a misclassified one.
            assert_eq!(left, !filter.keeps_palette(), "{filter:?}: keeps_palette() is {}", filter.keeps_palette());
        }
    }

    #[test]
    fn parameterized_pass_uses_custom_values() {
        let rgba = [128, 96, 96, 77];
        let pass = UpscalePass::from(EnhancementFilter::Vibrance { factor: 0.05 });
        let (_, _, custom, _, _) = apply_upscale_passes(1, 1, &rgba, &[pass]);
        let preset_pass = UpscalePass::from(EnhancementFilter::Vibrance { factor: 0.30 });
        let (_, _, preset, _, _) = apply_upscale_passes(1, 1, &rgba, &[preset_pass]);

        assert_eq!(custom[3], 77);
        assert_ne!(custom, preset);
    }
}
