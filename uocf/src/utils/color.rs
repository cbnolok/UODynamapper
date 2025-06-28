#![allow(dead_code)]

#[inline(always)]
fn component_rgba_5bit_to_8bit(component: u8) -> u8 {
    assert!(component < 32);
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
        Rgba8888::new_from_components(
            component_rgba_5bit_to_8bit(self.r()),
            component_rgba_5bit_to_8bit(self.g()),
            component_rgba_5bit_to_8bit(self.b()),
            255 * self.a())
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
}
