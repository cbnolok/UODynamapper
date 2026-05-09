//! xBRZ upscaling algorithm.
//!
//! Reference: https://docs.rs/xbrz-rs/latest/xbrz/
//! Original by Zenju: https://sourceforge.net/projects/xbrz/

pub fn apply_xbr(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    // xbrz-rs provides a simple scale_rgba function.
    // Factor must be between 1 and 6.
    let factor = scale.clamp(1, 6) as usize;
    let out_rgba = xbrz::scale_rgba(rgba, width as usize, height as usize, factor);
    
    let target_width = width * factor as u32;
    let target_height = height * factor as u32;

    (target_width, target_height, out_rgba)
}
