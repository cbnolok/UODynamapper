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
pub mod xbr;
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
    #[default]
    None,
    Nearest2x,
    Nearest3x,
    Nearest4x,
    Bilinear2x,
    Bilinear3x,
    Bilinear4x,
    CatmullRom2x,
    CatmullRom3x,
    CatmullRom4x,
    Lanczos3_2x,
    Lanczos3_3x,
    Lanczos3_4x,
    SuperSai2x,
    FsrEasu2x,
    FsrEasu3x,
    FsrEasu4x,
    FsrEasuRcas2x,
    FsrEasuRcas3x,
    FsrEasuRcas4x,
    /// Kopf-Lischinski Depixelization variants.
    KLDepixelize2x,
    KLDepixelize3x,
    KLDepixelize4x,
    
    // New algorithms
    Nedi2x,
    TwoSai2x,
    SuperEagle2x,
    Lq2x,
    Lq3x,
    Lq4x,
    Hq2xSimple,
    Hq3xSimple,
    Hq4xSimple,
    Hq2xTrue,
    Hq3xTrue,
    Hq4xTrue,
    Epx2x,
    Epx3x,
    Epx4x,
    Xbr2x,
    Xbr3x,
    Xbr4x,
    SuperXbr2x,
    Cut1_2x,
    Cut2_2x,
    Cut3_2x,
    ScaleFx2x,
    ScaleFx3x,
    ScaleFx4x,
    OmniScale2x,
    OmniScale3x,
    OmniScale4x,
    Jinc2_2x,
    Jinc2_3x,
    Jinc2_4x,
    Jinc2Sharp2x,
    Jinc2Sharp3x,
    Jinc2Sharp4x,
    Jinc2Sharper2x,
    Jinc2Sharper3x,
    Jinc2Sharper4x,
    Jinc2Sharpest2x,
    Jinc2Sharpest3x,
    Jinc2Sharpest4x,
    Mmpx2x,
    Mmpx4x,
    Vibrance20,
    Vibrance30,
    Vibrance40,
    Saturation115,
    Saturation125,
    Saturation130,
    SelectiveWarm20,
    SelectiveWarm30,
    SelectiveWarm40,
    SelectiveGreen20,
    SelectiveGreen30,
    SelectiveGreen40,
    LocalLaplacianClarity15,
    LocalLaplacianClarity25,
    LocalLaplacianClarity30,
    UnityContrastEnhance20,
    UnityContrastEnhance35,
    UnityContrastEnhance50,
    AdaptiveLogContrast75,
    AdaptiveLogContrast80,
    AdaptiveLogContrast90,
    ScaleFxSmartDeblur,
    UnsharpMaskSmall,
    HighPassSharpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct UpscaleConfig {
    /// Target width/height for upscaling. 0 means use original size or scale factor.
    pub target_size: u32,
    pub filter: UpscaleFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UpscalePass {
    pub filter: UpscaleFilter,
    pub params: UpscalePassParams,
}

impl UpscalePass {
    pub fn new(filter: UpscaleFilter) -> Self {
        Self {
            filter,
            params: UpscalePassParams::Default,
        }
    }

    pub fn scale_factor(self) -> u32 {
        self.filter.scale_factor()
    }

    pub fn apply(self, width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
        if let Some(pixels) = self.apply_custom(width, height, rgba) {
            return (width, height, pixels);
        }
        self.filter.apply(width, height, rgba)
    }

    pub fn apply_owned(self, width: u32, height: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
        if let Some(pixels) = self.apply_custom(width, height, &rgba) {
            return (width, height, pixels);
        }
        self.filter.apply(width, height, &rgba)
    }

    fn apply_custom(self, width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        match self.params {
            UpscalePassParams::Default => None,
            UpscalePassParams::ColorFactor { factor } => {
                let mut out = Vec::with_capacity(rgba.len());
                match self.filter {
                    UpscaleFilter::Vibrance20 | UpscaleFilter::Vibrance30 | UpscaleFilter::Vibrance40 => {
                        apply_vibrance(rgba, &mut out, factor);
                    }
                    UpscaleFilter::Saturation115 | UpscaleFilter::Saturation125 | UpscaleFilter::Saturation130 => {
                        apply_saturation(rgba, &mut out, factor);
                    }
                    UpscaleFilter::SelectiveWarm20 | UpscaleFilter::SelectiveWarm30 | UpscaleFilter::SelectiveWarm40 => {
                        apply_selective_hue_boost(rgba, &mut out, factor, HueBoostRange::Warm);
                    }
                    UpscaleFilter::SelectiveGreen20 | UpscaleFilter::SelectiveGreen30 | UpscaleFilter::SelectiveGreen40 => {
                        apply_selective_hue_boost(rgba, &mut out, factor, HueBoostRange::Green);
                    }
                    _ => return None,
                }
                Some(out)
            }
            UpscalePassParams::LocalLaplacianClarity { radius, amount } => {
                Some(sharpen::apply_local_laplacian_clarity(
                    width,
                    height,
                    rgba,
                    sharpen::LocalLaplacianClarityParams { radius, amount },
                ).2)
            }
            UpscalePassParams::ContrastEnhance { intensity, threshold, blur_spread } => {
                Some(sharpen::apply_contrast_enhance(
                    width,
                    height,
                    rgba,
                    sharpen::ContrastEnhanceParams { intensity, threshold, blur_spread },
                ).2)
            }
            UpscalePassParams::AdaptiveLogContrast { radius, gamma } => {
                Some(sharpen::apply_adaptive_log_contrast(
                    width,
                    height,
                    rgba,
                    sharpen::AdaptiveLogContrastParams { radius, gamma },
                ).2)
            }
            UpscalePassParams::UnsharpMask { radius, amount } => {
                Some(sharpen::apply_unsharp_mask(
                    width,
                    height,
                    rgba,
                    sharpen::UnsharpMaskParams { radius, amount },
                ).2)
            }
            UpscalePassParams::HighPassSharpen { radius, strength } => {
                Some(sharpen::apply_high_pass_sharpen(
                    width,
                    height,
                    rgba,
                    sharpen::HighPassSharpenParams { radius, strength },
                ).2)
            }
            UpscalePassParams::ScaleFxSmartDeblur { deblur_offset, deblur_strength, smart_deblur } => {
                Some(sharpen::apply_scalefx_smart_deblur(
                    width,
                    height,
                    rgba,
                    sharpen::ScaleFxSmartDeblurParams {
                        deblur_offset,
                        deblur_strength,
                        smart_deblur,
                    },
                ).2)
            }
        }
    }
}

impl From<UpscaleFilter> for UpscalePass {
    fn from(filter: UpscaleFilter) -> Self {
        Self::new(filter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum UpscalePassParams {
    Default,
    ColorFactor {
        factor: f32,
    },
    LocalLaplacianClarity {
        radius: u32,
        amount: f32,
    },
    ContrastEnhance {
        intensity: f32,
        threshold: f32,
        blur_spread: f32,
    },
    AdaptiveLogContrast {
        radius: f32,
        gamma: f32,
    },
    UnsharpMask {
        radius: f32,
        amount: f32,
    },
    HighPassSharpen {
        radius: f32,
        strength: f32,
    },
    ScaleFxSmartDeblur {
        deblur_offset: f32,
        deblur_strength: f32,
        smart_deblur: f32,
    },
}

impl Default for UpscalePassParams {
    fn default() -> Self {
        Self::Default
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
            Self::Xbr2x => 2,
            Self::Xbr3x => 3,
            Self::Xbr4x => 4,
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
            Self::Vibrance20
            | Self::Vibrance30
            | Self::Vibrance40
            | Self::Saturation115
            | Self::Saturation125
            | Self::Saturation130
            | Self::SelectiveWarm20
            | Self::SelectiveWarm30
            | Self::SelectiveWarm40
            | Self::SelectiveGreen20
            | Self::SelectiveGreen30
            | Self::SelectiveGreen40
            | Self::LocalLaplacianClarity15
            | Self::LocalLaplacianClarity25
            | Self::LocalLaplacianClarity30
            | Self::UnityContrastEnhance20
            | Self::UnityContrastEnhance35
            | Self::UnityContrastEnhance50
            | Self::AdaptiveLogContrast75
            | Self::AdaptiveLogContrast80
            | Self::AdaptiveLogContrast90
            | Self::ScaleFxSmartDeblur
            | Self::UnsharpMaskSmall
            | Self::HighPassSharpen => 1,
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

        if let Some(pixels) = self.apply_color_boost(rgba) {
            return pixels;
        }

        if let Some(pixels) = self.apply_post_upscale_sharpen(width, height, rgba) {
            return pixels;
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
            Self::Xbr2x | Self::Xbr3x | Self::Xbr4x => {
                let scale = (target_width / width).max(1);
                xbr::apply_xbr(width, height, rgba, scale).2
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
            Self::LocalLaplacianClarity15 => sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams { radius: 2, amount: 0.15 },
            ).2,
            Self::LocalLaplacianClarity25 => sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams::default(),
            ).2,
            Self::LocalLaplacianClarity30 => sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams { radius: 4, amount: 0.30 },
            ).2,
            Self::UnityContrastEnhance20 => sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams { intensity: 0.20, threshold: 0.05, blur_spread: 2.0 },
            ).2,
            Self::UnityContrastEnhance35 => sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams::default(),
            ).2,
            Self::UnityContrastEnhance50 => sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams { intensity: 0.50, threshold: 0.15, blur_spread: 3.0 },
            ).2,
            Self::AdaptiveLogContrast75 => sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams { radius: 3.0, gamma: 0.75 },
            ).2,
            Self::AdaptiveLogContrast80 => sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams::default(),
            ).2,
            Self::AdaptiveLogContrast90 => sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams { radius: 3.0, gamma: 0.90 },
            ).2,
            Self::ScaleFxSmartDeblur => sharpen::apply_scalefx_smart_deblur(
                width,
                height,
                rgba,
                sharpen::ScaleFxSmartDeblurParams::default(),
            ).2,
            Self::UnsharpMaskSmall => sharpen::apply_unsharp_mask(
                width,
                height,
                rgba,
                sharpen::UnsharpMaskParams::default(),
            ).2,
            Self::HighPassSharpen => sharpen::apply_high_pass_sharpen(
                width,
                height,
                rgba,
                sharpen::HighPassSharpenParams::default(),
            ).2,
            Self::Vibrance20
            | Self::Vibrance30
            | Self::Vibrance40
            | Self::Saturation115
            | Self::Saturation125
            | Self::Saturation130
            | Self::SelectiveWarm20
            | Self::SelectiveWarm30
            | Self::SelectiveWarm40
            | Self::SelectiveGreen20
            | Self::SelectiveGreen30
            | Self::SelectiveGreen40 => rgba.to_vec(),
        }
    }

    fn apply_color_boost(&self, rgba: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(rgba.len());
        match self {
            Self::Vibrance20 => apply_vibrance(rgba, &mut out, 0.20),
            Self::Vibrance30 => apply_vibrance(rgba, &mut out, 0.30),
            Self::Vibrance40 => apply_vibrance(rgba, &mut out, 0.40),
            Self::Saturation115 => apply_saturation(rgba, &mut out, 1.15),
            Self::Saturation125 => apply_saturation(rgba, &mut out, 1.25),
            Self::Saturation130 => apply_saturation(rgba, &mut out, 1.30),
            Self::SelectiveWarm20 => apply_selective_hue_boost(rgba, &mut out, 0.20, HueBoostRange::Warm),
            Self::SelectiveWarm30 => apply_selective_hue_boost(rgba, &mut out, 0.30, HueBoostRange::Warm),
            Self::SelectiveWarm40 => apply_selective_hue_boost(rgba, &mut out, 0.40, HueBoostRange::Warm),
            Self::SelectiveGreen20 => apply_selective_hue_boost(rgba, &mut out, 0.20, HueBoostRange::Green),
            Self::SelectiveGreen30 => apply_selective_hue_boost(rgba, &mut out, 0.30, HueBoostRange::Green),
            Self::SelectiveGreen40 => apply_selective_hue_boost(rgba, &mut out, 0.40, HueBoostRange::Green),
            _ => return None,
        }
        Some(out)
    }

    fn apply_post_upscale_sharpen(&self, width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        match self {
            Self::LocalLaplacianClarity15 => Some(sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams { radius: 2, amount: 0.15 },
            ).2),
            Self::LocalLaplacianClarity25 => Some(sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams::default(),
            ).2),
            Self::LocalLaplacianClarity30 => Some(sharpen::apply_local_laplacian_clarity(
                width,
                height,
                rgba,
                sharpen::LocalLaplacianClarityParams { radius: 4, amount: 0.30 },
            ).2),
            Self::UnityContrastEnhance20 => Some(sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams { intensity: 0.20, threshold: 0.05, blur_spread: 2.0 },
            ).2),
            Self::UnityContrastEnhance35 => Some(sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams::default(),
            ).2),
            Self::UnityContrastEnhance50 => Some(sharpen::apply_contrast_enhance(
                width,
                height,
                rgba,
                sharpen::ContrastEnhanceParams { intensity: 0.50, threshold: 0.15, blur_spread: 3.0 },
            ).2),
            Self::AdaptiveLogContrast75 => Some(sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams { radius: 3.0, gamma: 0.75 },
            ).2),
            Self::AdaptiveLogContrast80 => Some(sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams::default(),
            ).2),
            Self::AdaptiveLogContrast90 => Some(sharpen::apply_adaptive_log_contrast(
                width,
                height,
                rgba,
                sharpen::AdaptiveLogContrastParams { radius: 3.0, gamma: 0.90 },
            ).2),
            Self::ScaleFxSmartDeblur => Some(sharpen::apply_scalefx_smart_deblur(
                width,
                height,
                rgba,
                sharpen::ScaleFxSmartDeblurParams::default(),
            ).2),
            Self::UnsharpMaskSmall => Some(sharpen::apply_unsharp_mask(
                width,
                height,
                rgba,
                sharpen::UnsharpMaskParams::default(),
            ).2),
            Self::HighPassSharpen => Some(sharpen::apply_high_pass_sharpen(
                width,
                height,
                rgba,
                sharpen::HighPassSharpenParams::default(),
            ).2),
            _ => None,
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

    for pass in passes.iter().copied().filter(|pass| !matches!(pass.filter, UpscaleFilter::None)) {
        let (next_width, next_height, next_rgba) = pass.apply_owned(width, height, rgba);
        width = next_width;
        height = next_height;
        rgba = next_rgba;
        scale_factor = scale_factor.saturating_mul(pass.scale_factor());
        last_filter = pass.filter;
    }

    (width, height, rgba, scale_factor, last_filter)
}

#[cfg(test)]
mod tests {
    use super::{apply_filter_passes, apply_upscale_passes, UpscaleFilter, UpscalePass, UpscalePassParams};

    #[test]
    fn vibrance_boost_preserves_dimensions_and_alpha() {
        let rgba = [128, 96, 96, 77];
        let (width, height, pixels, scale, last_filter) =
            apply_filter_passes(1, 1, &rgba, &[UpscaleFilter::Vibrance30]);

        assert_eq!((width, height), (1, 1));
        assert_eq!(scale, 1);
        assert_eq!(last_filter, UpscaleFilter::Vibrance30);
        assert_eq!(pixels[3], 77);
        assert!(pixels[0] > rgba[0]);
        assert!(pixels[1] < rgba[1]);
        assert!(pixels[2] < rgba[2]);
    }

    #[test]
    fn saturation_boost_keeps_neutral_gray_neutral() {
        let rgba = [80, 80, 80, 255];
        let (_, _, pixels, _, _) =
            apply_filter_passes(1, 1, &rgba, &[UpscaleFilter::Saturation125]);

        assert_eq!(pixels, rgba);
    }

    #[test]
    fn selective_warm_boost_leaves_blue_unchanged() {
        let rgba = [120, 80, 60, 255, 80, 80, 160, 128];
        let (_, _, pixels, scale, _) =
            apply_filter_passes(2, 1, &rgba, &[UpscaleFilter::SelectiveWarm30]);

        assert_eq!(scale, 1);
        assert!(pixels[0] > rgba[0]);
        assert!(pixels[1] < rgba[1]);
        assert_eq!(&pixels[4..8], &rgba[4..8]);
    }

    #[test]
    fn selective_green_boost_targets_green_hues() {
        let rgba = [70, 130, 80, 255, 120, 80, 60, 255];
        let (_, _, pixels, _, _) =
            apply_filter_passes(2, 1, &rgba, &[UpscaleFilter::SelectiveGreen30]);

        assert!(pixels[1] > rgba[1]);
        assert!(pixels[0] < rgba[0]);
        assert_eq!(&pixels[4..8], &rgba[4..8]);
    }

    #[test]
    fn local_contrast_passes_preserve_dimensions_and_alpha() {
        let rgba = [40, 40, 40, 11, 80, 80, 80, 22, 120, 120, 120, 33, 160, 160, 160, 44];
        for filter in [
            UpscaleFilter::LocalLaplacianClarity25,
            UpscaleFilter::UnityContrastEnhance35,
            UpscaleFilter::AdaptiveLogContrast80,
        ] {
            let (width, height, pixels, scale, last_filter) =
                apply_filter_passes(4, 1, &rgba, &[filter]);

            assert_eq!((width, height), (4, 1));
            assert_eq!(scale, 1);
            assert_eq!(last_filter, filter);
            assert_eq!(pixels.len(), rgba.len());
            assert_eq!(pixels.chunks_exact(4).map(|pixel| pixel[3]).collect::<Vec<_>>(), vec![11, 22, 33, 44]);
        }
    }

    #[test]
    fn parameterized_pass_uses_custom_values() {
        let rgba = [128, 96, 96, 77];
        let pass = UpscalePass {
            filter: UpscaleFilter::Vibrance30,
            params: UpscalePassParams::ColorFactor { factor: 0.05 },
        };
        let (_, _, custom, _, _) = apply_upscale_passes(1, 1, &rgba, &[pass]);
        let (_, _, preset, _, _) = apply_filter_passes(1, 1, &rgba, &[UpscaleFilter::Vibrance30]);

        assert_eq!(custom[3], 77);
        assert_ne!(custom, preset);
    }
}
