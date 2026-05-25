//! Image upscaling filters for build-time asset processing.
//!
//! # Reference Implementations
//! - Kopf-Lischinski: https://github.com/vvanirudh/Pixel-Art
//! - HQx: https://github.com/brunexgeek/hqx
//! - xBRZ: https://docs.rs/xbrz-rs/latest/xbrz/
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
    Mmpx2x,
    Mmpx4x,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct UpscaleConfig {
    /// Target width/height for upscaling. 0 means use original size or scale factor.
    pub target_size: u32,
    pub filter: UpscaleFilter,
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
            Self::Nedi2x => 2,
            Self::Mmpx2x => 2,
            Self::Mmpx4x => 4,
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
        if matches!(self, Self::None) || rgba.is_empty() {
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
            Self::Nedi2x => nedi::apply_nedi(width, height, rgba).2,
            Self::Mmpx2x | Self::Mmpx4x => {
                let scale = (target_width / width).max(1);
                mmpx::apply_mmpx(width, height, rgba, scale).2
            }
        }
    }
}

pub fn apply_filter_passes(
    width: u32,
    height: u32,
    rgba: &[u8],
    passes: &[UpscaleFilter],
) -> (u32, u32, Vec<u8>, u32, UpscaleFilter) {
    let mut width = width;
    let mut height = height;
    let mut rgba = rgba.to_vec();
    let mut scale_factor = 1u32;
    let mut last_filter = UpscaleFilter::None;

    for filter in passes.iter().copied().filter(|filter| !matches!(filter, UpscaleFilter::None)) {
        let (next_width, next_height, next_rgba) = filter.apply(width, height, &rgba);
        width = next_width;
        height = next_height;
        rgba = next_rgba;
        scale_factor = scale_factor.saturating_mul(filter.scale_factor());
        last_filter = filter;
    }

    (width, height, rgba, scale_factor, last_filter)
}
