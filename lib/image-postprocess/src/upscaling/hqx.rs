//! HQx (High Quality upscaler) algorithms.
//!
//! Reference: https://github.com/brunexgeek/hqx
//! Original by Maxim Stepin.
//! Logic: Pattern matching in a 3x3 window with thresholded color distance.

pub fn apply_hqx(width: u32, height: u32, rgba: &[u8], scale: u32) -> (u32, u32, Vec<u8>) {
    use hqx::{hq2x, hq3x, hq4x};

    let target_width = width * scale;
    let target_height = height * scale;

    // Convert input RGBA to 0xAARRGGBB for the HQx crate
    let mut src_u32 = vec![0u32; (width * height) as usize];
    for (i, chunk) in rgba.chunks_exact(4).enumerate() {
        src_u32[i] = ((chunk[3] as u32) << 24)
            | ((chunk[0] as u32) << 16)
            | ((chunk[1] as u32) << 8)
            | (chunk[2] as u32);
    }

    let mut dst_u32 = vec![0u32; (target_width * target_height) as usize];

    match scale {
        2 => hq2x(&src_u32, &mut dst_u32, width as usize, height as usize),
        3 => hq3x(&src_u32, &mut dst_u32, width as usize, height as usize),
        4 => hq4x(&src_u32, &mut dst_u32, width as usize, height as usize),
        _ => {
            // Fallback for unsupported scales
            return (width, height, rgba.to_vec());
        }
    }

    // Convert output 0xAARRGGBB back to RGBA
    let mut out_rgba = vec![0u8; (target_width * target_height * 4) as usize];
    for (i, val) in dst_u32.into_iter().enumerate() {
        out_rgba[i * 4] = ((val >> 16) & 0xFF) as u8; // R
        out_rgba[i * 4 + 1] = ((val >> 8) & 0xFF) as u8; // G
        out_rgba[i * 4 + 2] = (val & 0xFF) as u8; // B
        out_rgba[i * 4 + 3] = ((val >> 24) & 0xFF) as u8; // A
    }

    (target_width, target_height, out_rgba)
}
