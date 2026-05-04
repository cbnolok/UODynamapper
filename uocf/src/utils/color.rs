#![allow(dead_code)]

use std::sync::OnceLock;

#[cfg(not(debug_assertions))]
use wide::*;

#[inline(always)]
fn component_rgba_5bit_to_8bit(component: u8) -> u8 {
    debug_assert!(component < 32);
    component * 8
    //(((component as u32) * 255) / 31) as u8

// In ARGB16 each color is 5 bits, so a value between 0-31
// In ARGB32 each color is 8 bits, so a value between 0-255
// Convert them to 0-255 range:
// There is more than one way.

// More exact conversion
/*
// This ensures 31/31 converts to 255/255
//r = (uint8_t)(r * 255 / 31);        // R
//g = (uint8_t)(g * 255 / 31);        // G
//b = (uint8_t)(b * 255 / 31);        // B
*/

// Conversion from (AxisII)
// Image slightly darker (probably though it looks more like the img shown in the uo client)
/*
r *= 8;        // R
g *= 8;        // G
b *= 8;        // B
*/

// An alpha channel of 0 means maximum transparency.
}

pub fn components_from_rgba888(val: u32) -> [u8; 4] {
    [
        ( val & 0x000000FF) as u8,          // B
        ((val & 0x0000FF00) >> 8) as u8,    // G
        ((val & 0x00FF0000) >> 16) as u8,   // R
        ((val & 0xFF000000) >> 24) as u8,   // A
    ]
}

pub fn components_from_rgb888(val: u32) -> [u8; 3] {
    [
        ( val & 0x000000FF) as u8,          // B
        ((val & 0x0000FF00) >> 8) as u8,    // G
        ((val & 0x00FF0000) >> 16) as u8,   // R
    ]
}

pub(crate) static BGRA5551_RGBA8888_LUT: OnceLock<Box<[u32; 1 << 16]>> = OnceLock::new();

#[inline(always)]
pub(crate) fn rgba8888_word_from_bgra5551(raw_color: u16) -> u32 {
    let red = ((raw_color >> 10) & 0x1F) as u32;
    let green = ((raw_color >> 5) & 0x1F) as u32;
    let blue = (raw_color & 0x1F) as u32;

    (0xFF << 24) | ((blue << 3) << 16) | ((green << 3) << 8) | (red << 3)
}

pub(crate) fn color_lut() -> &'static [u32; 1 << 16] {
    BGRA5551_RGBA8888_LUT.get_or_init(|| {
        let mut table = Box::new([0u32; 1 << 16]);
        for (raw_color, rgba_word) in table.iter_mut().enumerate() {
            *rgba_word = rgba8888_word_from_bgra5551(raw_color as u16);
        }
        table
    })
}


pub struct Bgra5551 {
    value: u16
}
impl Bgra5551 {
    pub fn value(&self) -> u16 {
        self.value
    }

    pub fn new_from_val(value: u16) -> Self {
        Self {
            value
        }
    }

    pub fn new_from_components(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            value: ((a & 0x1) as u16) << 15 |
                ((r & 0x1F) as u16) << 10 |
                ((g & 0x1F) as u16) << 5 |
                (b & 0x1F) as u16
        }
    }

    #[inline(always)]
    fn b(&self) -> u8 {
        (self.value & 0x1F) as u8
    }
    #[inline(always)]
    fn g(&self) -> u8 {
        ((self.value >> 5) & 0x1F) as u8
    }
    #[inline(always)]
    fn r(&self) -> u8 {
        ((self.value >> 10) & 0x1F) as u8
    }
    #[inline(always)]
    fn a(&self) -> u8 {
        ((self.value >> 15) & 0x1F) as u8
    }

    pub fn set_a(&mut self, a: u8) -> &Self {
        self.value = (self.value & 0x7FFF) | ((a as u16 & 0x1) << 15);
        self
    }

    pub fn as_rgba8888(&self) -> Rgba8888 {
        let rgba = rgba8888_word_from_bgra5551(self.value);
        let alpha_mask = if self.a() == 0 { 0x00FF_FFFF } else { u32::MAX };
        Rgba8888::new_from_val(rgba & alpha_mask)
    }
}

pub struct Rgba8888 {
    value: u32
}
impl Rgba8888 {
    pub fn value(&self) -> u32 {
        self.value
    }
    pub fn new_from_components(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            value: (a as u32) << 24 |
                (b as u32) << 16 |
                (g as u32) << 8 |
                r as u32
        }
    }
    pub fn new_from_val(value: u32) -> Self {
        Self {
            value
        }
    }
    pub fn components(&self) -> (u8, u8, u8, u8) {
        (
            (self.value & 0xFF) as u8,
            ((self.value >> 8) & 0xFF) as u8,
            ((self.value >> 16) & 0xFF) as u8,
            ((self.value >> 24) & 0xFF) as u8,
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rgb555 {
    value: u16
}

impl Rgb555 {
    pub fn new_from_val(value: u16) -> Self {
        Self {
            value
        }
    }

    pub fn new_from_components(r: u8, g: u8, b: u8) -> Self {
        Self {
            value: ((r & 0x1F) as u16) << 10 |
                ((g & 0x1F) as u16) << 5 |
                (b & 0x1F) as u16
        }
    }

    #[inline(always)]
    pub fn r(&self) -> u8 {
        ((self.value >> 10) & 0x1F) as u8
    }
    #[inline(always)]
    pub fn g(&self) -> u8 {
        ((self.value >> 5) & 0x1F) as u8
    }
    #[inline(always)]
    pub fn b(&self) -> u8 {
        (self.value & 0x1F) as u8
    }

    pub fn as_rgba8888(&self) -> Rgba8888 {
        Rgba8888::new_from_components(
            component_rgba_5bit_to_8bit(self.r()),
            component_rgba_5bit_to_8bit(self.g()),
            component_rgba_5bit_to_8bit(self.b()),
            255)
    }
}

/// Utility function using AVX/SSE (via wide) to mass-convert Bgra5551 slices
/// into Rgba8888 streams natively.
#[inline(always)]
pub fn bulk_convert_bgra5551_to_rgba8888(raw_u16: &[u16], out_rgba: &mut [u8]) {
    // Assume slice matching lengths: `raw_u16.len() * 4 == out_rgba.len()`
    #[cfg(not(debug_assertions))]
    {
        let (pixel_data_u16_prefix, pixel_data_u16_suffix) = bytemuck::cast_slice::<_, u16>(raw_u16).as_chunks::<16>();

        let mut out_offset = 0;
        for &chunk_array in pixel_data_u16_prefix {
            let chunk = u16x16::new(chunk_array);

            let [lo, hi]: [u16x8; 2] = unsafe { std::mem::transmute(chunk) };

            let b_u16_lo: u32x8 = u32x8::from((lo & u16x8::splat(0x1F)) << 3);
            let g_u16_lo: u32x8 = u32x8::from(((lo >> 5) & u16x8::splat(0x1F)) << 3);
            let r_u16_lo: u32x8 = u32x8::from(((lo >> 10) & u16x8::splat(0x1F)) << 3);
            let a_u16_lo: u32x8 = u32x8::splat(0xFF); // Full opacity

            let b_u16_hi: u32x8 = u32x8::from((hi & u16x8::splat(0x1F)) << 3);
            let g_u16_hi: u32x8 = u32x8::from(((hi >> 5) & u16x8::splat(0x1F)) << 3);
            let r_u16_hi: u32x8 = u32x8::from(((hi >> 10) & u16x8::splat(0x1F)) << 3);
            let a_u16_hi: u32x8 = u32x8::splat(0xFF);

            #[allow(unused_mut)]
            let mut rgba_lo: u32x8 = (a_u16_lo << 24) | (b_u16_lo << 16) | (g_u16_lo << 8) | r_u16_lo;
            #[allow(unused_mut)]
            let mut rgba_hi: u32x8 = (a_u16_hi << 24) | (b_u16_hi << 16) | (g_u16_hi << 8) | r_u16_hi;

            #[cfg(target_endian = "big")]
            {
                rgba_lo = rgba_lo.swap_bytes();
                rgba_hi = rgba_hi.swap_bytes();
            }

            let arr_lo = rgba_lo.as_array();
            let arr_hi = rgba_hi.as_array();

            let slice_lo = bytemuck::cast_slice(arr_lo);
            let slice_hi = bytemuck::cast_slice(arr_hi);

            out_rgba[out_offset..out_offset+32].copy_from_slice(slice_lo);
            out_rgba[out_offset+32..out_offset+64].copy_from_slice(slice_hi);
            out_offset += 64;
        }

        let lut = color_lut();
        for &p in pixel_data_u16_suffix {
            out_rgba[out_offset..out_offset+4].copy_from_slice(&lut[p as usize].to_le_bytes());
            out_offset += 4;
        }
    }

    #[cfg(debug_assertions)]
    {
        let lut = color_lut();
        let mut out_offset = 0;
        for &p in raw_u16 {
            out_rgba[out_offset..out_offset+4].copy_from_slice(&lut[p as usize].to_le_bytes());
            out_offset += 4;
        }
    }
}
