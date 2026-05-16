//! MMPX (Multi-pixel Magnification) upscaling algorithm.
//!
//! Reference: https://github.com/pierogis/mmpx-rs
//! Native support for 2x magnification.

use image::RgbaImage;

pub fn apply_mmpx(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec()).expect("Failed to create RgbaImage for MMPX");
    
    match scale {
        2 => {
            let magnified = mmpx::magnify(&img);
            (magnified.width(), magnified.height(), magnified.into_raw())
        }
        4 => {
            let magnified1 = mmpx::magnify(&img);
            let magnified2 = mmpx::magnify(&magnified1);
            (magnified2.width(), magnified2.height(), magnified2.into_raw())
        }
        _ => {
            // MMPX natively supports 2x. For other scales, we return the original
            // or could potentially combine with other resizers, but for now we stick to native/nested.
            (width, height, rgba.to_vec())
        }
    }
}
